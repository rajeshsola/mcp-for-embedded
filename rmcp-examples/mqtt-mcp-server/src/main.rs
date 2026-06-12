use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use rumqttc::{AsyncClient, Event, MqttOptions, Outgoing, Packet, QoS};
use std::time::Duration;

// ── parameter structs ────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct PublishParams {
    #[schemars(description = "MQTT broker hostname or IP address (e.g. localhost, 192.168.1.1)")]
    host: String,
    #[schemars(description = "MQTT broker port (default: 1883)")]
    port: Option<u16>,
    #[schemars(description = "MQTT client ID; auto-generated when omitted")]
    client_id: Option<String>,
    #[schemars(description = "Username for broker authentication")]
    username: Option<String>,
    #[schemars(description = "Password for broker authentication")]
    password: Option<String>,
    #[schemars(description = "Topic to publish to (e.g. sensors/temperature)")]
    topic: String,
    #[schemars(description = "Message payload as a UTF-8 string")]
    payload: String,
    #[schemars(
        description = "QoS level: 0 = at-most-once, 1 = at-least-once, 2 = exactly-once (default: 0)"
    )]
    qos: Option<u8>,
    #[schemars(description = "Retain the message on the broker (default: false)")]
    retain: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SubscribeParams {
    #[schemars(description = "MQTT broker hostname or IP address")]
    host: String,
    #[schemars(description = "MQTT broker port (default: 1883)")]
    port: Option<u16>,
    #[schemars(description = "MQTT client ID; auto-generated when omitted")]
    client_id: Option<String>,
    #[schemars(description = "Username for broker authentication")]
    username: Option<String>,
    #[schemars(description = "Password for broker authentication")]
    password: Option<String>,
    #[schemars(
        description = "Topic filter to subscribe to; supports wildcards + and # (e.g. sensors/#)"
    )]
    topic: String,
    #[schemars(description = "QoS level for the subscription: 0, 1, or 2 (default: 0)")]
    qos: Option<u8>,
    #[schemars(
        description = "Stop after collecting this many messages (default: 10)"
    )]
    max_messages: Option<usize>,
    #[schemars(description = "Total time to wait for messages in milliseconds (default: 5000)")]
    timeout_ms: Option<u64>,
}

// ── server struct ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct MqttMcpServer {
    tool_router: ToolRouter<Self>,
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn qos_from_u8(q: u8) -> Result<QoS, McpError> {
    match q {
        0 => Ok(QoS::AtMostOnce),
        1 => Ok(QoS::AtLeastOnce),
        2 => Ok(QoS::ExactlyOnce),
        _ => Err(McpError::invalid_params(
            format!("invalid QoS {}: must be 0, 1, or 2", q),
            None,
        )),
    }
}

fn unique_client_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{}-{}-{}", prefix, std::process::id(), nanos)
}

fn build_opts(
    client_id: String,
    host: &str,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    keep_alive: Duration,
) -> MqttOptions {
    let mut opts = MqttOptions::new(client_id, host, port);
    opts.set_keep_alive(keep_alive);
    if let (Some(u), Some(p)) = (username, password) {
        opts.set_credentials(u, p);
    }
    opts
}

// ── tool implementations ─────────────────────────────────────────────────────

#[tool_router]
impl MqttMcpServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Connect to an MQTT broker and publish a single message to a topic. \
                       Returns when the broker has confirmed delivery (QoS 1/2) or the message \
                       has been written to the network (QoS 0)."
    )]
    async fn mqtt_publish(
        &self,
        Parameters(params): Parameters<PublishParams>,
    ) -> Result<CallToolResult, McpError> {
        let port = params.port.unwrap_or(1883);
        let client_id = params.client_id.unwrap_or_else(|| unique_client_id("mcp-pub"));
        let qos_level = qos_from_u8(params.qos.unwrap_or(0))?;
        let retain = params.retain.unwrap_or(false);

        let opts = build_opts(
            client_id,
            &params.host,
            port,
            params.username,
            params.password,
            Duration::from_secs(10),
        );
        let (client, mut eventloop) = AsyncClient::new(opts, 10);

        client
            .publish(&params.topic, qos_level, retain, params.payload.as_bytes())
            .await
            .map_err(|e| {
                McpError::internal_error(format!("failed to queue publish: {}", e), None)
            })?;

        let result = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match eventloop.poll().await {
                    // QoS 0: message left the outbound buffer → done
                    Ok(Event::Outgoing(Outgoing::Publish(_)))
                        if qos_level == QoS::AtMostOnce =>
                    {
                        return Ok(());
                    }
                    // QoS 1: broker acknowledged
                    Ok(Event::Incoming(Packet::PubAck(_)))
                        if qos_level == QoS::AtLeastOnce =>
                    {
                        return Ok(());
                    }
                    // QoS 2: broker completed the exchange
                    Ok(Event::Incoming(Packet::PubComp(_)))
                        if qos_level == QoS::ExactlyOnce =>
                    {
                        return Ok(());
                    }
                    Ok(_) => {}
                    Err(e) => return Err(format!("broker connection error: {}", e)),
                }
            }
        })
        .await;

        drop(client);

        match result {
            Ok(Ok(())) => Ok(CallToolResult::success(vec![Content::text(format!(
                "published to '{}' on {}:{} [qos={} retain={}]",
                params.topic,
                params.host,
                port,
                params.qos.unwrap_or(0),
                retain,
            ))])),
            Ok(Err(e)) => Err(McpError::internal_error(e, None)),
            Err(_) => Err(McpError::internal_error(
                "timed out waiting for broker confirmation (10 s); check host/port/credentials",
                None,
            )),
        }
    }

    #[tool(
        description = "Subscribe to an MQTT topic (wildcards + and # supported) and collect \
                       incoming messages until the timeout or message limit is reached. \
                       Returns all collected messages with their topics."
    )]
    async fn mqtt_subscribe(
        &self,
        Parameters(params): Parameters<SubscribeParams>,
    ) -> Result<CallToolResult, McpError> {
        let port = params.port.unwrap_or(1883);
        let client_id = params.client_id.unwrap_or_else(|| unique_client_id("mcp-sub"));
        let qos_level = qos_from_u8(params.qos.unwrap_or(0))?;
        let max_msgs = params.max_messages.unwrap_or(10);
        let timeout_ms = params.timeout_ms.unwrap_or(5000);
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(timeout_ms);

        let opts = build_opts(
            client_id,
            &params.host,
            port,
            params.username,
            params.password,
            Duration::from_secs(30),
        );
        let (client, mut eventloop) = AsyncClient::new(opts, 10);

        client
            .subscribe(&params.topic, qos_level)
            .await
            .map_err(|e| {
                McpError::internal_error(format!("failed to queue subscribe: {}", e), None)
            })?;

        let mut messages: Vec<String> = Vec::new();

        loop {
            let remaining =
                deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, eventloop.poll()).await {
                Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                    let payload = String::from_utf8_lossy(&p.payload).to_string();
                    messages.push(format!("[{}] {}", p.topic, payload));
                    if messages.len() >= max_msgs {
                        break;
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    return Err(McpError::internal_error(
                        format!("broker connection error: {}", e),
                        None,
                    ));
                }
                Err(_) => break, // overall deadline elapsed
            }
        }

        drop(client);

        if messages.is_empty() {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "subscribed to '{}' on {}:{} — no messages received within {} ms",
                params.topic, params.host, port, timeout_ms,
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "received {} message(s) from '{}' on {}:{}:\n{}",
                messages.len(),
                params.topic,
                params.host,
                port,
                messages.join("\n"),
            ))]))
        }
    }
}

// ── server metadata ──────────────────────────────────────────────────────────

#[tool_handler]
impl ServerHandler for MqttMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "MCP server for MQTT publish/subscribe over any MQTT 3.1.1 broker. \
                 Use mqtt_publish to send a message to a topic and mqtt_subscribe to \
                 collect messages from a topic or wildcard filter. Both tools accept \
                 optional credentials for authenticated brokers."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = MqttMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
