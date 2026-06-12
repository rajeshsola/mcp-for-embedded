use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use socketcan::{CanDataFrame, CanFrame, CanSocket, EmbeddedFrame, ExtendedId, Id, Socket, StandardId};
use std::time::Duration;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SendFrameParams {
    #[schemars(description = "CAN frame ID as a decimal integer (e.g. 291 for 0x123); IDs above 0x7FF are treated as extended automatically")]
    can_id: u32,
    #[schemars(description = "Frame payload as a hex string with no separators (e.g. DEADBEEF01020304); maximum 8 bytes")]
    data: String,
    #[schemars(description = "Force 29-bit extended CAN ID; auto-detected when can_id > 0x7FF")]
    extended: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ReceiveFrameParams {
    #[schemars(description = "How long to wait for a frame in milliseconds (default: 1000)")]
    timeout_ms: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct PublishTelemetryParams {
    #[schemars(description = "MQTT broker hostname or IP address (default: localhost)")]
    broker_host: Option<String>,
    #[schemars(description = "MQTT broker port (default: 1883)")]
    broker_port: Option<u16>,
    #[schemars(description = "Vehicle speed in km/h")]
    speed: Option<u16>,
    #[schemars(description = "Engine RPM")]
    rpm: Option<u16>,
    #[schemars(description = "Fuel level (0-255)")]
    fuel_level: Option<u8>,
    #[schemars(description = "Driving mode: 0=Normal, 1=Sport, 2=Eco")]
    driving_mode: Option<u8>,
}

#[derive(Clone)]
struct CanMcpServer {
    interface: String,
    tool_router: ToolRouter<Self>,
}

fn parse_hex_data(s: &str) -> Result<Vec<u8>, String> {
    let s: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':' && *c != '-')
        .collect();
    if s.len() % 2 != 0 {
        return Err("hex string must have an even number of nibbles".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| format!("invalid byte at position {}: {}", i / 2, e))
        })
        .collect()
}

fn id_to_u32(id: Id) -> u32 {
    match id {
        Id::Standard(s) => s.as_raw() as u32,
        Id::Extended(e) => e.as_raw(),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_vehicle_frame(frame_id: u32, data: &[u8]) -> serde_json::Value {
    match frame_id {
        0x100 => serde_json::json!({
            "speed":        (data.len() >= 2).then(|| u16::from_be_bytes([data[0], data[1]])),
            "rpm":          (data.len() >= 4).then(|| u16::from_be_bytes([data[2], data[3]])),
            "fuel_level":   data.get(4).copied(),
            "driving_mode": data.get(5).copied(),
        }),
        0x101 => serde_json::json!({
            "speed": (data.len() >= 2).then(|| u16::from_be_bytes([data[0], data[1]])),
        }),
        0x102 => serde_json::json!({
            "rpm": (data.len() >= 2).then(|| u16::from_be_bytes([data[0], data[1]])),
        }),
        0x103 => serde_json::json!({
            "fuel_level": data.first().copied(),
        }),
        0x104 => serde_json::json!({
            "driving_mode": data.first().copied(),
        }),
        _ => serde_json::json!({
            "frame_id": format!("0x{:X}", frame_id),
            "raw_data": hex_encode(data),
        }),
    }
}

#[tool_router]
impl CanMcpServer {
    fn new(interface: String) -> Self {
        Self {
            interface,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Send a CAN data frame on the configured SocketCAN interface")]
    async fn send_frame(
        &self,
        Parameters(SendFrameParams {
            can_id,
            data,
            extended,
        }): Parameters<SendFrameParams>,
    ) -> Result<CallToolResult, McpError> {
        let interface = self.interface.clone();
        let data_bytes = parse_hex_data(&data)
            .map_err(|e| McpError::invalid_params(format!("invalid hex data: {}", e), None))?;

        if data_bytes.len() > 8 {
            return Err(McpError::invalid_params(
                format!(
                    "CAN data length {} exceeds 8-byte maximum",
                    data_bytes.len()
                ),
                None,
            ));
        }

        let use_extended = extended.unwrap_or(can_id > 0x7FF);
        let id: Id = if use_extended {
            ExtendedId::new(can_id)
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("invalid extended CAN ID 0x{:X} (max 0x1FFFFFFF)", can_id),
                        None,
                    )
                })?
                .into()
        } else {
            StandardId::new(can_id as u16)
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("invalid standard CAN ID 0x{:X} (max 0x7FF)", can_id),
                        None,
                    )
                })?
                .into()
        };

        let frame = CanDataFrame::new(id, &data_bytes)
            .ok_or_else(|| McpError::invalid_params("failed to construct CAN frame", None))?;

        tokio::task::spawn_blocking(move || {
            let socket = CanSocket::open(&interface).map_err(|e| {
                McpError::internal_error(
                    format!("failed to open interface {}: {}", interface, e),
                    None,
                )
            })?;
            socket.write_frame(&frame).map_err(|e| {
                McpError::internal_error(format!("failed to send CAN frame: {}", e), None)
            })
        })
        .await
        .map_err(|e| McpError::internal_error(format!("task join error: {}", e), None))??;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "sent: ID=0x{:X} ({}) len={} data=[{}]",
            can_id,
            if use_extended { "extended" } else { "standard" },
            data_bytes.len(),
            hex_encode(&data_bytes),
        ))]))
    }

    #[tool(description = "Receive a single CAN frame from the configured SocketCAN interface")]
    async fn receive_frame(
        &self,
        Parameters(ReceiveFrameParams { timeout_ms }): Parameters<ReceiveFrameParams>,
    ) -> Result<CallToolResult, McpError> {
        let interface = self.interface.clone();
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(1000));

        let frame = tokio::task::spawn_blocking(move || {
            let socket = CanSocket::open(&interface).map_err(|e| {
                McpError::internal_error(
                    format!("failed to open interface {}: {}", interface, e),
                    None,
                )
            })?;
            socket.set_read_timeout(timeout).map_err(|e| {
                McpError::internal_error(format!("failed to set read timeout: {}", e), None)
            })?;
            socket.read_frame().map_err(|e| {
                McpError::internal_error(format!("failed to receive CAN frame: {}", e), None)
            })
        })
        .await
        .map_err(|e| McpError::internal_error(format!("task join error: {}", e), None))??;

        let msg = match frame {
            CanFrame::Data(f) => {
                let payload = decode_vehicle_frame(id_to_u32(f.id()), f.data());
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
            }
            CanFrame::Remote(f) => format!(
                "remote frame: ID=0x{:X} ({}) dlc={}",
                id_to_u32(f.id()),
                if f.is_extended() { "extended" } else { "standard" },
                f.dlc(),
            ),
            CanFrame::Error(_) => "error frame received".to_string(),
        };

        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    #[tool(description = "Publish vehicle telemetry readings to an MQTT broker. \
        Sends to 'vehicletelemetry/all' when multiple readings are provided; \
        sends to the matching sub-topic (vehicletelemetry/speed, /rpm, /fuel, /driving_mode) \
        when only one reading is provided.")]
    async fn publish_telemetry(
        &self,
        Parameters(PublishTelemetryParams {
            broker_host,
            broker_port,
            speed,
            rpm,
            fuel_level,
            driving_mode,
        }): Parameters<PublishTelemetryParams>,
    ) -> Result<CallToolResult, McpError> {
        let host = broker_host.unwrap_or_else(|| "localhost".to_string());
        let port = broker_port.unwrap_or(1883);

        let mut payload = serde_json::Map::new();
        let mut field_count: u8 = 0;

        if let Some(v) = speed       { payload.insert("speed".to_string(),        v.into()); field_count += 1; }
        if let Some(v) = rpm         { payload.insert("rpm".to_string(),           v.into()); field_count += 1; }
        if let Some(v) = fuel_level  { payload.insert("fuel_level".to_string(),    v.into()); field_count += 1; }
        if let Some(v) = driving_mode{ payload.insert("driving_mode".to_string(),  v.into()); field_count += 1; }

        if field_count == 0 {
            return Err(McpError::invalid_params(
                "at least one telemetry reading (speed, rpm, fuel_level, driving_mode) must be provided",
                None,
            ));
        }

        let topic = if field_count > 1 {
            "vehicletelemetry/all".to_string()
        } else if speed.is_some() {
            "vehicletelemetry/speed".to_string()
        } else if rpm.is_some() {
            "vehicletelemetry/rpm".to_string()
        } else if fuel_level.is_some() {
            "vehicletelemetry/fuel".to_string()
        } else {
            "vehicletelemetry/driving_mode".to_string()
        };

        let payload_str = serde_json::to_string(&payload)
            .map_err(|e| McpError::internal_error(format!("serialization error: {}", e), None))?;

        let mut opts = MqttOptions::new("socketcan-mcp-gw", &host, port);
        opts.set_keep_alive(Duration::from_secs(5));

        let (client, mut eventloop) = AsyncClient::new(opts, 10);

        client
            .publish(&topic, QoS::AtLeastOnce, false, payload_str.as_bytes())
            .await
            .map_err(|e| McpError::internal_error(format!("MQTT publish error: {}", e), None))?;

        // Drive the event loop until the broker acknowledges the publish (QoS 1 PubAck).
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Packet::PubAck(_))) => return Ok(()),
                    Ok(_) => {}
                    Err(e) => return Err(format!("{}", e)),
                }
            }
        })
        .await
        .map_err(|_| McpError::internal_error("MQTT publish timed out after 5 s", None))?
        .map_err(|e| McpError::internal_error(format!("MQTT event loop error: {}", e), None))?;

        let _ = client.disconnect().await;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "published to '{}': {}",
            topic, payload_str
        ))]))
    }
}

#[tool_handler]
impl ServerHandler for CanMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "MCP server for sending and receiving CAN frames via Linux SocketCAN. \
                 Requires a configured SocketCAN interface (e.g. vcan0 or can0)."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let interface = std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!("usage: socketcan-mcp <interface>  (e.g. vcan0, can0)")
    })?;
    let service = CanMcpServer::new(interface).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
