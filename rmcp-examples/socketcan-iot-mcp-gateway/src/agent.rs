//! LLM-powered CAN → MQTT agent.
//!
//! Each decoded CAN frame is sent to an Anthropic Claude model via the RIG
//! framework.  The LLM reasons about the telemetry — determining whether to
//! publish, which fields carry meaningful data, and what alerts to raise —
//! instead of relying on the hard-coded thresholds used in gateway.rs.
//!
//! Data flow:
//!   CAN bus
//!     └─▶ receive_frame  (MCP tool in socketcan-mcp)
//!               └─▶ Claude via RIG extractor  ← reasoning happens here
//!                         └─▶ publish_telemetry  (MCP tool in socketcan-mcp)
//!                                   └─▶ MQTT broker
//!
//! Required env:
//!   ANTHROPIC_API_KEY   — Anthropic API key
//!   LLM_MODEL           — optional model override
//!                         (default: claude-sonnet-4-6)

use anyhow::{bail, Context, Result};
use rig_core::prelude::*;
use rig_core::providers::anthropic::{self, completion::CLAUDE_SONNET_4_6};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

// ── LLM output schema ─────────────────────────────────────────────────────────

/// Structured routing decision the LLM must return for every CAN frame.
/// All fields are interpreted by the gateway to decide topic and payload.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct RoutingDecision {
    /// Publish this frame to MQTT. Set false to drop the frame silently.
    publish: bool,

    /// Vehicle speed in km/h — include only when a valid value is present.
    speed: Option<u16>,

    /// Engine RPM — include only when a valid value is present.
    rpm: Option<u16>,

    /// Fuel level 0–255 — include only when a valid value is present.
    fuel_level: Option<u8>,

    /// Driving mode: 0 = Normal, 1 = Sport, 2 = Eco — include when present.
    driving_mode: Option<u8>,

    /// Alert codes raised for this frame. Examples:
    /// OVERSPEED, HIGH_RPM, LOW_FUEL, CRITICAL_FUEL, UNKNOWN_FRAME
    alerts: Vec<String>,

    /// One-sentence explanation of the routing decision.
    reasoning: String,
}

const SYSTEM_PROMPT: &str = "\
You are an expert IoT vehicle-telemetry routing agent embedded in a CAN-bus \
gateway. Your job is to analyse each decoded CAN frame and decide how to route it.\n\
\n\
Guidelines:\n\
• Set publish=true for any frame that carries meaningful vehicle telemetry.\n\
• Only include telemetry fields that have a valid, non-null value.\n\
• Raise alerts when sensor readings exceed safe thresholds:\n\
    - OVERSPEED      : speed > 200 km/h\n\
    - HIGH_RPM       : rpm > 6000\n\
    - LOW_FUEL       : fuel_level < 20\n\
    - CRITICAL_FUEL  : fuel_level < 10 (raise in addition to LOW_FUEL)\n\
• Set publish=false only when no recognised telemetry is present (e.g.\n\
  diagnostics-only or fully unknown frames).\n\
• Provide a concise one-sentence reasoning that names the frame type and\n\
  any notable conditions.\n\
• Respond ONLY by calling the submit function — never include free-form text.";

// ── MCP / JSON-RPC 2.0 client ─────────────────────────────────────────────────

struct McpClient {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    _child: Child,
    next_id: u64,
}

impl McpClient {
    /// Spawn `cmd` as the MCP server and complete the MCP initialise handshake.
    async fn spawn(cmd: &mut Command) -> Result<Self> {
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut child = cmd.spawn().context("failed to spawn MCP server process")?;
        let stdin = child.stdin.take().expect("piped stdin configured above");
        let stdout = child.stdout.take().expect("piped stdout configured above");
        let mut c = Self {
            stdin,
            stdout: BufReader::new(stdout),
            _child: child,
            next_id: 1,
        };
        c.handshake().await.context("MCP handshake failed")?;
        Ok(c)
    }

    async fn send(&mut self, msg: &serde_json::Value) -> Result<()> {
        let mut line = serde_json::to_vec(msg).context("JSON serialise")?;
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .context("write to MCP stdin")?;
        Ok(())
    }

    /// Read the next JSON-RPC response, skipping server-initiated notifications.
    async fn recv_response(&mut self) -> Result<serde_json::Value> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .context("read from MCP stdout")?;
            if n == 0 {
                bail!("MCP server closed stdout unexpectedly");
            }
            let s = line.trim();
            if s.is_empty() {
                continue;
            }
            let msg: serde_json::Value =
                serde_json::from_str(s).context("MCP server sent invalid JSON")?;
            // Notifications carry "method" but no "id" — skip them.
            if msg.get("method").is_some() && msg.get("id").is_none() {
                continue;
            }
            return Ok(msg);
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    async fn handshake(&mut self) -> Result<()> {
        let id = self.alloc_id();
        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "can-llm-agent", "version": "0.1.0" }
            }
        }))
        .await?;
        let resp = self.recv_response().await?;
        if let Some(err) = resp.get("error") {
            bail!("MCP initialize error: {err}");
        }
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }))
        .await?;
        Ok(())
    }

    async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.alloc_id();
        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .await?;
        let resp = self.recv_response().await?;
        if let Some(err) = resp.get("error") {
            bail!("tool '{name}' error: {err}");
        }
        Ok(resp["result"].clone())
    }

    fn is_error(result: &serde_json::Value) -> bool {
        result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn text_content(result: &serde_json::Value) -> Option<&str> {
        result["content"].as_array()?.iter().find_map(|c| {
            if c["type"] == "text" {
                c["text"].as_str()
            } else {
                None
            }
        })
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli_args = std::env::args().skip(1);

    let can_iface = cli_args.next().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: can-llm-agent <can_interface> [mqtt_host] [mqtt_port] [mcp_server_bin]\n\
             env:   ANTHROPIC_API_KEY  — Anthropic API key (required)\n\
                    LLM_MODEL          — model override (default: {CLAUDE_SONNET_4_6})"
        )
    })?;
    let mqtt_host = cli_args.next().unwrap_or_else(|| "localhost".to_string());
    let mqtt_port = cli_args
        .next()
        .map(|s| s.parse::<u16>().context("invalid MQTT port"))
        .transpose()?
        .unwrap_or(1883);
    let mcp_bin = cli_args
        .next()
        .unwrap_or_else(|| "socketcan-mcp".to_string());

    // ── RIG / LLM setup ───────────────────────────────────────────────────────
    let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| CLAUDE_SONNET_4_6.to_string());

    // ProviderClient::from_env reads ANTHROPIC_API_KEY
    let llm_client = anthropic::Client::from_env()
        .context("failed to create Anthropic client — is ANTHROPIC_API_KEY set?")?;

    // CompletionClient::extractor injects a 'submit' tool that forces the LLM
    // to return a RoutingDecision as structured JSON on every call.
    let extractor = llm_client
        .extractor::<RoutingDecision>(&model)
        .preamble(SYSTEM_PROMPT)
        .build();

    // ── MCP server ────────────────────────────────────────────────────────────
    let mut cmd = Command::new(&mcp_bin);
    cmd.arg(&can_iface);
    let mut mcp = McpClient::spawn(&mut cmd)
        .await
        .with_context(|| format!("cannot start MCP server '{mcp_bin}'"))?;

    eprintln!(
        "[agent] ready — CAN:{can_iface} → LLM({model}) → MQTT:{mqtt_host}:{mqtt_port}"
    );

    loop {
        // ── 1. Receive next CAN frame via MCP tool ────────────────────────────
        let recv = match mcp
            .call_tool("receive_frame", serde_json::json!({"timeout_ms": 2000}))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[agent] receive_frame RPC error: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };

        if McpClient::is_error(&recv) {
            continue; // poll timeout — no frame arrived; normal
        }

        let raw = match McpClient::text_content(&recv) {
            Some(t) => t,
            None => continue,
        };

        // ── 2. Parse decoded telemetry from tool output ───────────────────────
        let telemetry: serde_json::Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(_) => {
                // Remote/error frames come back as plain text — log and skip.
                eprintln!("[agent] non-telemetry frame: {raw}");
                continue;
            }
        };

        // ── 3. LLM reasoning via RIG extractor ───────────────────────────────
        let prompt = format!(
            "Decoded CAN frame telemetry:\n{}\n\nAnalyse and decide routing.",
            serde_json::to_string_pretty(&telemetry).unwrap_or_default()
        );

        let decision: RoutingDecision = match extractor.extract(prompt.as_str()).await {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[agent] LLM extraction error: {e}");
                continue;
            }
        };

        eprintln!(
            "[agent] LLM → publish={} alerts={:?}  \"{}\"",
            decision.publish, decision.alerts, decision.reasoning
        );

        if !decision.publish {
            eprintln!("[agent] frame dropped per LLM reasoning");
            continue;
        }

        // ── 4. Build publish_telemetry arguments from LLM decision ───────────
        let mut pub_args = serde_json::json!({
            "broker_host": mqtt_host,
            "broker_port": mqtt_port,
        });
        let obj = pub_args.as_object_mut().unwrap();

        if let Some(v) = decision.speed {
            obj.insert("speed".into(), v.into());
        }
        if let Some(v) = decision.rpm {
            obj.insert("rpm".into(), v.into());
        }
        if let Some(v) = decision.fuel_level {
            obj.insert("fuel_level".into(), v.into());
        }
        if let Some(v) = decision.driving_mode {
            obj.insert("driving_mode".into(), v.into());
        }

        // obj has broker_host + broker_port as baseline — need at least one telemetry field
        if obj.len() == 2 {
            eprintln!("[agent] LLM said publish=true but included no telemetry fields — skipping");
            continue;
        }

        // ── 5. Publish via MCP tool ───────────────────────────────────────────
        match mcp.call_tool("publish_telemetry", pub_args).await {
            Ok(r) if !McpClient::is_error(&r) => {
                let msg = McpClient::text_content(&r).unwrap_or("published");
                eprintln!("[agent] ✓ {msg}");
                if !decision.alerts.is_empty() {
                    eprintln!("[agent] ⚠  alerts: {:?}", decision.alerts);
                }
            }
            Ok(r) => {
                let detail = McpClient::text_content(&r).unwrap_or("(no detail)");
                eprintln!("[agent] publish_telemetry error: {detail}");
            }
            Err(e) => eprintln!("[agent] publish_telemetry RPC error: {e}"),
        }
    }
}
