//! CAN → MQTT gateway that orchestrates through the `socketcan-mcp` MCP server.
//!
//! Flow:
//!   [CAN bus]
//!      └─▶ receive_frame (MCP tool)
//!              └─▶ routing decision (field count / type)
//!                      └─▶ publish_telemetry (MCP tool)
//!                                └─▶ [MQTT broker]
//!
//! The gateway speaks the MCP JSON-RPC 2.0 protocol over the server process's
//! stdin/stdout so it exercises the real tool interface in main.rs rather than
//! bypassing it.

use anyhow::{bail, Context, Result};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const TELEMETRY_FIELDS: &[&str] = &["speed", "rpm", "fuel_level", "driving_mode"];

// ── Minimal MCP / JSON-RPC 2.0 client ────────────────────────────────────────

struct McpClient {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    _child: Child,
    next_id: u64,
}

impl McpClient {
    /// Spawn `cmd` as the MCP server subprocess and complete the MCP handshake.
    async fn spawn(cmd: &mut Command) -> Result<Self> {
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());

        let mut child = cmd.spawn().context("failed to spawn MCP server process")?;
        let stdin = child.stdin.take().expect("configured above");
        let stdout = child.stdout.take().expect("configured above");

        let mut client = Self {
            stdin,
            stdout: BufReader::new(stdout),
            _child: child,
            next_id: 1,
        };

        client.handshake().await.context("MCP handshake failed")?;
        Ok(client)
    }

    // ── Low-level I/O ─────────────────────────────────────────────────────────

    async fn send(&mut self, msg: &serde_json::Value) -> Result<()> {
        let mut line = serde_json::to_vec(msg).context("failed to serialise JSON-RPC message")?;
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .context("failed to write to MCP server stdin")?;
        Ok(())
    }

    /// Read the next JSON-RPC *response* (skips server-side notifications).
    async fn recv_response(&mut self) -> Result<serde_json::Value> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .context("failed to read from MCP server stdout")?;
            if n == 0 {
                bail!("MCP server closed stdout — process may have crashed");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg: serde_json::Value =
                serde_json::from_str(trimmed).context("MCP server sent invalid JSON")?;
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

    // ── MCP protocol ──────────────────────────────────────────────────────────

    /// Perform the MCP `initialize` / `notifications/initialized` handshake.
    async fn handshake(&mut self) -> Result<()> {
        let id = self.alloc_id();
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "can-mqtt-gateway", "version": "0.1.0" }
            }
        }))
        .await?;

        let resp = self.recv_response().await?;
        if let Some(err) = resp.get("error") {
            bail!("initialize returned error: {err}");
        }

        // Complete the handshake — server is now ready for tool calls.
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }))
        .await?;

        Ok(())
    }

    /// Call a named MCP tool and return its raw `result` object.
    async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.alloc_id();
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .await?;

        let resp = self.recv_response().await?;
        if let Some(err) = resp.get("error") {
            bail!("tool '{name}' JSON-RPC error: {err}");
        }
        Ok(resp["result"].clone())
    }

    // ── Result helpers ────────────────────────────────────────────────────────

    fn is_error(result: &serde_json::Value) -> bool {
        result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// First `type=text` content string from a tool result, if any.
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

// ── Routing logic ─────────────────────────────────────────────────────────────

/// Inspect the decoded telemetry object from `receive_frame` and decide which
/// fields to forward and to which sub-topic.
///
/// Returns `None` when no recognised telemetry fields are present (e.g. an
/// unknown or remote CAN frame) so the gateway can skip it.
fn route(
    telemetry: &serde_json::Value,
    mqtt_host: &str,
    mqtt_port: u16,
) -> Option<serde_json::Value> {
    let obj = telemetry.as_object()?;

    let mut args = serde_json::json!({
        "broker_host": mqtt_host,
        "broker_port": mqtt_port,
    });
    let dest = args.as_object_mut().unwrap();

    for &field in TELEMETRY_FIELDS {
        if let Some(v) = obj.get(field).filter(|v| !v.is_null()) {
            dest.insert(field.to_string(), v.clone());
        }
    }

    // Sub-topic is chosen inside `publish_telemetry` based on field count:
    //   > 1 field → vehicletelemetry/all
    //   1 field  → vehicletelemetry/<field>
    // We only need to decide *whether* to forward.
    let n_fields = dest.len().saturating_sub(2); // minus broker_host / broker_port
    if n_fields == 0 {
        return None;
    }

    let topic_hint = if n_fields > 1 {
        "vehicletelemetry/all"
    } else {
        match dest.keys().find(|k| TELEMETRY_FIELDS.contains(&k.as_str())) {
            Some(k) if k == "fuel_level" => "vehicletelemetry/fuel",
            Some(k) => Box::leak(format!("vehicletelemetry/{k}").into_boxed_str()),
            None => "vehicletelemetry/unknown",
        }
    };

    eprintln!("[gateway] route → {topic_hint}  ({n_fields} field(s))");
    Some(args)
}

// ── Gateway ───────────────────────────────────────────────────────────────────

struct Gateway {
    can_interface: String,
    mcp_bin: String,
    mqtt_host: String,
    mqtt_port: u16,
}

impl Gateway {
    async fn run(self) -> Result<()> {
        let mut cmd = Command::new(&self.mcp_bin);
        cmd.arg(&self.can_interface);

        let mut mcp = McpClient::spawn(&mut cmd)
            .await
            .with_context(|| format!("cannot start MCP server '{}'", self.mcp_bin))?;

        eprintln!(
            "[gateway] ready — CAN:{} → MCP({}) → MQTT:{}:{}",
            self.can_interface, self.mcp_bin, self.mqtt_host, self.mqtt_port
        );

        loop {
            // ── 1. Pull next CAN frame via MCP ────────────────────────────────
            let recv = mcp
                .call_tool("receive_frame", serde_json::json!({"timeout_ms": 2000}))
                .await;

            let recv_result = match recv {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[gateway] receive_frame RPC error: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
            };

            if McpClient::is_error(&recv_result) {
                // Tool-level error: timeout waiting for a frame — normal, retry.
                continue;
            }

            let raw = match McpClient::text_content(&recv_result) {
                Some(t) => t,
                None => continue,
            };

            // ── 2. Parse the decoded telemetry JSON ───────────────────────────
            let telemetry: serde_json::Value = match serde_json::from_str(raw) {
                Ok(v) => v,
                Err(_) => {
                    // Remote / error frames come back as plain text — skip.
                    eprintln!("[gateway] non-telemetry frame: {raw}");
                    continue;
                }
            };

            // ── 3. Routing decision ───────────────────────────────────────────
            let Some(pub_args) = route(&telemetry, &self.mqtt_host, self.mqtt_port) else {
                eprintln!("[gateway] no recognised telemetry fields — skipping");
                continue;
            };

            // ── 4. Publish via MCP ────────────────────────────────────────────
            match mcp.call_tool("publish_telemetry", pub_args).await {
                Ok(r) if !McpClient::is_error(&r) => {
                    let msg = McpClient::text_content(&r).unwrap_or("published");
                    eprintln!("[gateway] ✓ {msg}");
                }
                Ok(r) => {
                    let detail = McpClient::text_content(&r).unwrap_or("(no detail)");
                    eprintln!("[gateway] publish_telemetry error: {detail}");
                }
                Err(e) => eprintln!("[gateway] publish_telemetry RPC error: {e}"),
            }
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);

    let can_iface = args.next().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: can-mqtt-gateway <can_interface> [mqtt_host] [mqtt_port] [mcp_server_bin]\n\
             example: can-mqtt-gateway vcan0 localhost 1883 socketcan-mcp"
        )
    })?;

    let mqtt_host = args.next().unwrap_or_else(|| "localhost".to_string());
    let mqtt_port = args
        .next()
        .map(|s| s.parse::<u16>().context("MQTT port must be 1–65535"))
        .transpose()?
        .unwrap_or(1883);
    let mcp_bin = args.next().unwrap_or_else(|| "socketcan-mcp".to_string());

    Gateway {
        can_interface: can_iface,
        mcp_bin,
        mqtt_host,
        mqtt_port,
    }
    .run()
    .await
}
