use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use socketcan::{CanDataFrame, CanFrame, CanSocket, EmbeddedFrame, ExtendedId, Id, Socket, StandardId};
use std::time::Duration;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SendFrameParams {
    #[schemars(description = "SocketCAN interface name (e.g. vcan0, can0)")]
    interface: String,
    #[schemars(description = "CAN frame ID as a decimal integer (e.g. 291 for 0x123); IDs above 0x7FF are treated as extended automatically")]
    can_id: u32,
    #[schemars(description = "Frame payload as a hex string with no separators (e.g. DEADBEEF01020304); maximum 8 bytes")]
    data: String,
    #[schemars(description = "Force 29-bit extended CAN ID; auto-detected when can_id > 0x7FF")]
    extended: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ReceiveFrameParams {
    #[schemars(description = "SocketCAN interface name (e.g. vcan0, can0)")]
    interface: String,
    #[schemars(description = "How long to wait for a frame in milliseconds (default: 1000)")]
    timeout_ms: Option<u64>,
}

#[derive(Clone)]
struct CanMcpServer {
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

#[tool_router]
impl CanMcpServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Send a CAN data frame on a Linux SocketCAN interface")]
    async fn send_frame(
        &self,
        Parameters(SendFrameParams {
            interface,
            can_id,
            data,
            extended,
        }): Parameters<SendFrameParams>,
    ) -> Result<CallToolResult, McpError> {
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

    #[tool(description = "Receive a single CAN frame from a Linux SocketCAN interface")]
    async fn receive_frame(
        &self,
        Parameters(ReceiveFrameParams {
            interface,
            timeout_ms,
        }): Parameters<ReceiveFrameParams>,
    ) -> Result<CallToolResult, McpError> {
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
            CanFrame::Data(f) => format!(
                "data frame: ID=0x{:X} ({}) len={} data=[{}]",
                id_to_u32(f.id()),
                if f.is_extended() { "extended" } else { "standard" },
                f.dlc(),
                hex_encode(f.data()),
            ),
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
    let service = CanMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
