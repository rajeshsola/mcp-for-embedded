use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};

// ── parameter structs ────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ListPortsParams {}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct OpenParams {
    #[schemars(
        description = "Device path (e.g. /dev/ttyUSB0, /dev/ttyS0, COM3). \
                       Use a real device path — no in-process loopback URL like loop:// is supported in Rust serialport."
    )]
    port: String,

    #[schemars(description = "Baud rate (e.g. 9600, 115200, 230400). Default: 9600.")]
    baud_rate: Option<u32>,

    #[schemars(description = "Data bits per frame: 5, 6, 7, or 8. Default: 8.")]
    data_bits: Option<u8>,

    #[schemars(description = "Parity: N=None, E=Even, O=Odd. Default: N.")]
    parity: Option<String>,

    #[schemars(description = "Stop bits: 1 or 2. Default: 1.")]
    stop_bits: Option<u8>,

    #[schemars(description = "Read timeout in milliseconds. Default: 1000.")]
    timeout_ms: Option<u64>,

    #[schemars(description = "Flow control: none, software (XON/XOFF), hardware (RTS/CTS). Default: none.")]
    flow_control: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct WriteParams {
    #[schemars(description = "Device path of the open port.")]
    port: String,

    #[schemars(
        description = "Data to write. In hex mode provide hex bytes with optional separators \
                       (e.g. 'DE AD BE EF'). Otherwise provide a text string (e.g. 'AT\\r\\n')."
    )]
    data: String,

    #[schemars(description = "If true, parse data as hex bytes. Default: false.")]
    hex_mode: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ReadParams {
    #[schemars(description = "Device path of the open port.")]
    port: String,

    #[schemars(description = "Maximum number of bytes to read. Default: 256.")]
    num_bytes: Option<usize>,

    #[schemars(
        description = "Override the port read timeout for this call (ms). \
                       None uses the port's configured timeout."
    )]
    timeout_ms: Option<u64>,

    #[schemars(description = "If true, return data as hex-encoded string. Default: false.")]
    hex_mode: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct WriteReadParams {
    #[schemars(description = "Device path of the open port.")]
    port: String,

    #[schemars(description = "Data to write (text or hex depending on hex_mode).")]
    data: String,

    #[schemars(description = "Maximum response bytes to read. Default: 256.")]
    read_bytes: Option<usize>,

    #[schemars(description = "Treat data as hex; return hex. Default: false.")]
    hex_mode: Option<bool>,

    #[schemars(description = "Wait this many ms after writing before reading. Default: 50.")]
    delay_ms: Option<u64>,

    #[schemars(description = "Read timeout for the response in ms. Default: 1000.")]
    timeout_ms: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct PortParam {
    #[schemars(description = "Device path of the open port.")]
    port: String,
}

// ── port discovery (no libudev required) ─────────────────────────────────────

fn linux_list_ports() -> Vec<(String, String)> {
    use std::fs;
    use std::path::Path;

    // Walk /sys/class/tty; each entry is a potential TTY driver.
    // Keep only entries that have a "device" symlink (real hardware) and
    // whose driver name is not "tty" (the raw virtual console driver).
    let sys_tty = Path::new("/sys/class/tty");
    let mut results = Vec::new();

    let Ok(entries) = fs::read_dir(sys_tty) else {
        return results;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Quick filter: skip virtual terminals (tty0..ttyN without USB/S prefix variety)
        // Keep: ttyUSB*, ttyACM*, ttyS*, ttyAMA*, rfcomm*, ttyXR*, ttyO*
        let keep = name_str.starts_with("ttyUSB")
            || name_str.starts_with("ttyACM")
            || name_str.starts_with("ttyAMA")
            || name_str.starts_with("ttyXR")
            || name_str.starts_with("ttyO")
            || name_str.starts_with("rfcomm")
            || (name_str.starts_with("ttyS") && name_str[4..].parse::<u32>().is_ok());

        if !keep {
            continue;
        }

        let dev_path = format!("/dev/{}", name_str);
        if !Path::new(&dev_path).exists() {
            continue;
        }

        // Determine port type from driver symlink
        let driver_path = entry.path().join("device/driver");
        let kind = if let Ok(target) = fs::read_link(&driver_path) {
            let driver_name = target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if driver_name.contains("usb") || name_str.starts_with("ttyUSB") || name_str.starts_with("ttyACM") {
                format!("USB (driver={})", driver_name)
            } else {
                format!("serial (driver={})", driver_name)
            }
        } else if name_str.starts_with("ttyUSB") || name_str.starts_with("ttyACM") {
            "USB".to_string()
        } else {
            "unknown".to_string()
        };

        results.push((dev_path, kind));
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

// ── port registry type ────────────────────────────────────────────────────────

type PortHandle = Arc<Mutex<Box<dyn SerialPort>>>;
type PortMap = Arc<Mutex<HashMap<String, PortHandle>>>;

// ── server struct ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SerialMcpServer {
    tool_router: ToolRouter<Self>,
    ports: PortMap,
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn parse_hex(s: &str) -> Result<Vec<u8>, McpError> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':' && *c != '-')
        .collect();
    if cleaned.len() % 2 != 0 {
        return Err(McpError::invalid_params(
            "hex string must have an even number of nibbles",
            None,
        ));
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&cleaned[i..i + 2], 16)
                .map_err(|e| McpError::invalid_params(format!("invalid hex at byte {}: {}", i / 2, e), None))
        })
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_parity(s: &str) -> Result<Parity, McpError> {
    match s.to_uppercase().as_str() {
        "N" | "NONE" => Ok(Parity::None),
        "E" | "EVEN" => Ok(Parity::Even),
        "O" | "ODD" => Ok(Parity::Odd),
        other => Err(McpError::invalid_params(
            format!("invalid parity '{}'; must be N, E, or O", other),
            None,
        )),
    }
}

fn parse_data_bits(n: u8) -> Result<DataBits, McpError> {
    match n {
        5 => Ok(DataBits::Five),
        6 => Ok(DataBits::Six),
        7 => Ok(DataBits::Seven),
        8 => Ok(DataBits::Eight),
        other => Err(McpError::invalid_params(
            format!("invalid data_bits {}; must be 5, 6, 7, or 8", other),
            None,
        )),
    }
}

fn parse_stop_bits(n: u8) -> Result<StopBits, McpError> {
    match n {
        1 => Ok(StopBits::One),
        2 => Ok(StopBits::Two),
        other => Err(McpError::invalid_params(
            format!("invalid stop_bits {}; must be 1 or 2", other),
            None,
        )),
    }
}

fn parse_flow(s: &str) -> Result<FlowControl, McpError> {
    match s.to_lowercase().as_str() {
        "none" => Ok(FlowControl::None),
        "software" | "xonxoff" => Ok(FlowControl::Software),
        "hardware" | "rtscts" => Ok(FlowControl::Hardware),
        other => Err(McpError::invalid_params(
            format!("invalid flow_control '{}'; must be none, software, or hardware", other),
            None,
        )),
    }
}

fn get_port_handle(ports: &PortMap, port: &str) -> Result<PortHandle, McpError> {
    let map = ports.lock().unwrap();
    map.get(port).cloned().ok_or_else(|| {
        McpError::invalid_params(
            format!("port '{}' is not open; call serial_open first", port),
            None,
        )
    })
}

// ── tools ─────────────────────────────────────────────────────────────────────

#[tool_router]
impl SerialMcpServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            ports: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tool(description = "List all serial ports visible to the operating system. \
                          Returns each port's device path and type (USB/PCI/unknown). \
                          Scans /sys/class/tty on Linux without requiring libudev.")]
    async fn serial_list_ports(
        &self,
        Parameters(_): Parameters<ListPortsParams>,
    ) -> Result<CallToolResult, McpError> {
        // Read /sys/class/tty and filter to real serial devices (not virtual terminals).
        // This avoids requiring libudev while still finding USB-serial, RS-232, and ACM ports.
        let ports = tokio::task::spawn_blocking(linux_list_ports)
            .await
            .map_err(|e| McpError::internal_error(format!("task error: {}", e), None))?;

        if ports.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "no serial ports found (checked /sys/class/tty)",
            )]));
        }

        let mut lines = vec![format!("{} port(s) found:", ports.len())];
        for (dev, kind) in &ports {
            lines.push(format!("  {:<24} {}", dev, kind));
        }
        Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
    }

    #[tool(description = "Open a serial port and register it for subsequent read/write operations. \
                          Call serial_close when finished. \
                          For loopback testing without hardware use socat to create a PTY pair \
                          (e.g. socat PTY,link=/tmp/ttyA PTY,link=/tmp/ttyB).")]
    async fn serial_open(
        &self,
        Parameters(p): Parameters<OpenParams>,
    ) -> Result<CallToolResult, McpError> {
        let baud_rate = p.baud_rate.unwrap_or(9600);
        let data_bits = parse_data_bits(p.data_bits.unwrap_or(8))?;
        let parity = parse_parity(p.parity.as_deref().unwrap_or("N"))?;
        let stop_bits = parse_stop_bits(p.stop_bits.unwrap_or(1))?;
        let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(1000));
        let flow = parse_flow(p.flow_control.as_deref().unwrap_or("none"))?;

        // Guard against re-opening without closing first
        {
            let map = self.ports.lock().unwrap();
            if map.contains_key(&p.port) {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "port '{}' is already open — call serial_close first to reconfigure",
                    p.port
                ))]));
            }
        }

        let port_name = p.port.clone();
        let ser = tokio::task::spawn_blocking(move || {
            serialport::new(&port_name, baud_rate)
                .data_bits(data_bits)
                .parity(parity)
                .stop_bits(stop_bits)
                .timeout(timeout)
                .flow_control(flow)
                .open()
        })
        .await
        .map_err(|e| McpError::internal_error(format!("task error: {}", e), None))?
        .map_err(|e| McpError::internal_error(format!("failed to open '{}': {}", p.port, e), None))?;

        self.ports
            .lock()
            .unwrap()
            .insert(p.port.clone(), Arc::new(Mutex::new(ser)));

        let parity_str = match parity {
            Parity::None => "N",
            Parity::Even => "E",
            Parity::Odd => "O",
        };
        let flow_str = match flow {
            FlowControl::None => "none",
            FlowControl::Software => "XON/XOFF",
            FlowControl::Hardware => "RTS/CTS",
        };

        Ok(CallToolResult::success(vec![Content::text(format!(
            "opened '{}' — {} {}{}{}  timeout={} ms  flow={}",
            p.port,
            baud_rate,
            p.data_bits.unwrap_or(8),
            parity_str,
            p.stop_bits.unwrap_or(1),
            p.timeout_ms.unwrap_or(1000),
            flow_str,
        ))]))
    }

    #[tool(description = "Write data to an open serial port. \
                          Supports plain text or hex byte strings (e.g. 'DE AD BE EF').")]
    async fn serial_write(
        &self,
        Parameters(p): Parameters<WriteParams>,
    ) -> Result<CallToolResult, McpError> {
        let raw: Vec<u8> = if p.hex_mode.unwrap_or(false) {
            parse_hex(&p.data)?
        } else {
            p.data.as_bytes().to_vec()
        };

        let port_handle = get_port_handle(&self.ports, &p.port)?;
        let raw_clone = raw.clone();
        let port_name = p.port.clone();

        let n = tokio::task::spawn_blocking(move || {
            let mut port = port_handle.lock().unwrap();
            port.write_all(&raw_clone)
                .map_err(|e| McpError::internal_error(format!("write error on '{}': {}", port_name, e), None))?;
            port.flush()
                .map_err(|e| McpError::internal_error(format!("flush error on '{}': {}", port_name, e), None))?;
            Ok::<usize, McpError>(raw_clone.len())
        })
        .await
        .map_err(|e| McpError::internal_error(format!("task error: {}", e), None))??;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "wrote {} byte(s) to '{}': [{}]",
            n,
            p.port,
            hex_encode(&raw),
        ))]))
    }

    #[tool(description = "Read bytes from an open serial port. \
                          Returns when num_bytes received or the read timeout expires.")]
    async fn serial_read(
        &self,
        Parameters(p): Parameters<ReadParams>,
    ) -> Result<CallToolResult, McpError> {
        let num_bytes = p.num_bytes.unwrap_or(256);
        let hex_mode = p.hex_mode.unwrap_or(false);
        let port_handle = get_port_handle(&self.ports, &p.port)?;
        let port_name = p.port.clone();
        let timeout_override = p.timeout_ms;

        let raw = tokio::task::spawn_blocking(move || {
            let mut port = port_handle.lock().unwrap();
            if let Some(ms) = timeout_override {
                port.set_timeout(Duration::from_millis(ms))
                    .map_err(|e| McpError::internal_error(format!("set_timeout error: {}", e), None))?;
            }
            let mut buf = vec![0u8; num_bytes];
            let n = port.read(&mut buf).unwrap_or(0);
            Ok::<Vec<u8>, McpError>(buf[..n].to_vec())
        })
        .await
        .map_err(|e| McpError::internal_error(format!("task error: {}", e), None))??;

        if raw.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "read 0 bytes from '{}' (timeout)",
                p.port
            ))]));
        }

        let text = String::from_utf8_lossy(&raw);
        let output = if hex_mode {
            format!(
                "read {} byte(s) from '{}':\n  hex  : {}",
                raw.len(),
                port_name,
                hex_encode(&raw)
            )
        } else {
            format!(
                "read {} byte(s) from '{}':\n  text : {:?}\n  hex  : {}",
                raw.len(),
                port_name,
                text.as_ref(),
                hex_encode(&raw)
            )
        };

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(description = "Write data then read the response — standard request-response pattern. \
                          Typical use: AT commands, Modbus RTU, NMEA queries. \
                          Writes data, waits delay_ms, then reads up to read_bytes.")]
    async fn serial_write_read(
        &self,
        Parameters(p): Parameters<WriteReadParams>,
    ) -> Result<CallToolResult, McpError> {
        let raw_write: Vec<u8> = if p.hex_mode.unwrap_or(false) {
            parse_hex(&p.data)?
        } else {
            p.data.as_bytes().to_vec()
        };
        let read_bytes = p.read_bytes.unwrap_or(256);
        let delay_ms = p.delay_ms.unwrap_or(50);
        let timeout_ms = p.timeout_ms.unwrap_or(1000);
        let hex_mode = p.hex_mode.unwrap_or(false);

        let port_handle = get_port_handle(&self.ports, &p.port)?;
        let port_name = p.port.clone();
        let raw_write_clone = raw_write.clone();

        let raw_read = tokio::task::spawn_blocking(move || {
            let mut port = port_handle.lock().unwrap();

            // Flush input buffer before sending the request
            port.clear(serialport::ClearBuffer::Input)
                .map_err(|e| McpError::internal_error(format!("clear error: {}", e), None))?;

            // Write the request
            port.write_all(&raw_write_clone)
                .map_err(|e| McpError::internal_error(format!("write error on '{}': {}", port_name, e), None))?;
            port.flush()
                .map_err(|e| McpError::internal_error(format!("flush error: {}", e), None))?;

            // Wait for device to respond
            std::thread::sleep(Duration::from_millis(delay_ms));

            // Read the response with the specified timeout
            port.set_timeout(Duration::from_millis(timeout_ms))
                .map_err(|e| McpError::internal_error(format!("set_timeout error: {}", e), None))?;

            let mut buf = vec![0u8; read_bytes];
            let n = port.read(&mut buf).unwrap_or(0);
            Ok::<Vec<u8>, McpError>(buf[..n].to_vec())
        })
        .await
        .map_err(|e| McpError::internal_error(format!("task error: {}", e), None))??;

        let write_hex = hex_encode(&raw_write);
        if raw_read.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "wrote {} byte(s) to '{}': [{}]\nread  0 bytes (timeout — no response within {} ms)",
                raw_write.len(),
                p.port,
                write_hex,
                timeout_ms,
            ))]));
        }

        let text = String::from_utf8_lossy(&raw_read);
        let output = if hex_mode {
            format!(
                "wrote {} byte(s) to '{}': [{}]\nread  {} byte(s):\n  hex  : {}",
                raw_write.len(),
                p.port,
                write_hex,
                raw_read.len(),
                hex_encode(&raw_read),
            )
        } else {
            format!(
                "wrote {} byte(s) to '{}': [{}]\nread  {} byte(s):\n  text : {:?}\n  hex  : {}",
                raw_write.len(),
                p.port,
                write_hex,
                raw_read.len(),
                text.as_ref(),
                hex_encode(&raw_read),
            )
        };

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(description = "Flush the input and output buffers of an open serial port. \
                          Discards unread input and waits for queued output to transmit.")]
    async fn serial_flush(
        &self,
        Parameters(p): Parameters<PortParam>,
    ) -> Result<CallToolResult, McpError> {
        let port_handle = get_port_handle(&self.ports, &p.port)?;
        let port_name = p.port.clone();

        tokio::task::spawn_blocking(move || {
            let mut port = port_handle.lock().unwrap();
            port.clear(serialport::ClearBuffer::All)
                .map_err(|e| McpError::internal_error(format!("clear error on '{}': {}", port_name, e), None))?;
            port.flush()
                .map_err(|e| McpError::internal_error(format!("flush error on '{}': {}", port_name, e), None))?;
            Ok::<(), McpError>(())
        })
        .await
        .map_err(|e| McpError::internal_error(format!("task error: {}", e), None))??;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "flushed input/output buffers of '{}'",
            p.port
        ))]))
    }

    #[tool(description = "Return the current UART configuration and byte-count status of an open port. \
                          Shows baud rate, framing (data/parity/stop), timeout, flow control, \
                          and how many bytes are waiting in the OS RX/TX buffers.")]
    async fn serial_get_status(
        &self,
        Parameters(p): Parameters<PortParam>,
    ) -> Result<CallToolResult, McpError> {
        let port_handle = get_port_handle(&self.ports, &p.port)?;
        let port_name = p.port.clone();

        let lines = tokio::task::spawn_blocking(move || {
            let port = port_handle.lock().unwrap();

            let baud    = port.baud_rate().unwrap_or(0);
            let dbits   = format!("{:?}", port.data_bits().unwrap_or(DataBits::Eight));
            let parity  = format!("{:?}", port.parity().unwrap_or(Parity::None));
            let sbits   = format!("{:?}", port.stop_bits().unwrap_or(StopBits::One));
            let flow    = format!("{:?}", port.flow_control().unwrap_or(FlowControl::None));
            let timeout = port.timeout();
            let in_buf  = port.bytes_to_read().unwrap_or(0);
            let out_buf = port.bytes_to_write().unwrap_or(0);

            Ok::<Vec<String>, McpError>(vec![
                format!("port           : {}", port_name),
                format!("baud_rate      : {}", baud),
                format!("data_bits      : {}", dbits),
                format!("parity         : {}", parity),
                format!("stop_bits      : {}", sbits),
                format!("flow_control   : {}", flow),
                format!("read_timeout   : {} ms", timeout.as_millis()),
                format!("in_waiting     : {} byte(s)", in_buf),
                format!("out_waiting    : {} byte(s)", out_buf),
            ])
        })
        .await
        .map_err(|e| McpError::internal_error(format!("task error: {}", e), None))??;

        Ok(CallToolResult::success(vec![Content::text(lines.join("\n"))]))
    }

    #[tool(description = "Close an open serial port and remove it from the registry.")]
    async fn serial_close(
        &self,
        Parameters(p): Parameters<PortParam>,
    ) -> Result<CallToolResult, McpError> {
        let removed = self.ports.lock().unwrap().remove(&p.port);
        match removed {
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "port '{}' was not open",
                p.port
            ))])),
            Some(handle) => {
                // Drop outside the map lock — the Box<dyn SerialPort> destructor closes the fd
                tokio::task::spawn_blocking(move || drop(handle))
                    .await
                    .map_err(|e| McpError::internal_error(format!("task error: {}", e), None))?;

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "closed '{}'",
                    p.port
                ))]))
            }
        }
    }
}

// ── server handler ────────────────────────────────────────────────────────────

#[tool_handler]
impl ServerHandler for SerialMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "MCP server for serial port (UART/RS-232/USB-serial) operations. \
                 Workflow: serial_list_ports → serial_open → \
                 serial_write / serial_read / serial_write_read → serial_close. \
                 For loopback testing without hardware, create a PTY pair with socat: \
                 socat PTY,link=/tmp/ttyA,raw,echo=0 PTY,link=/tmp/ttyB,raw,echo=0 \
                 then open both /tmp/ttyA and /tmp/ttyB."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = SerialMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
