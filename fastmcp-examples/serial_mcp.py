"""Serial port MCP server — read, write, and configure UART/RS-232/USB-serial ports.

Requires: pyserial
    pip install pyserial

Tools exposed
─────────────
  serial_list_ports   — list all ports visible to the OS
  serial_open         — open a port with full UART configuration
  serial_write        — write text or hex bytes to an open port
  serial_read         — read bytes from an open port
  serial_write_read   — write then read (request-response pattern)
  serial_flush        — flush input/output buffers
  serial_get_status   — report current port config and signal lines
  serial_close        — close an open port

Port registry
─────────────
The server keeps a registry of open ports for the lifetime of the process.
Use serial_open to create an entry and serial_close to remove it.
The port key is the device path (e.g. /dev/ttyUSB0) or a pyserial URL
such as loop:// (software loopback, no hardware needed).

Loopback URLs (for testing without hardware)
────────────────────────────────────────────
  loop://           in-process software loopback (write comes back as read)
  /dev/pts/N        virtual port created by socat or tty0tty
"""

import asyncio
from typing import Optional

import serial
import serial.tools.list_ports
from fastmcp import FastMCP

mcp = FastMCP(
    "serial",
    instructions=(
        "MCP server for serial port (UART/RS-232/USB-serial) operations. "
        "Workflow: serial_list_ports → serial_open → serial_write / serial_read "
        "/ serial_write_read → serial_close. "
        "Use port='loop://' for software loopback testing without hardware."
    ),
)

# ── port registry ─────────────────────────────────────────────────────────────
# Maps port name → open serial.Serial instance.
# Protected by _lock for thread-safe access from run_in_executor.

_ports: dict[str, serial.Serial] = {}
_lock = asyncio.Lock()

# ── parity / stop-bit maps ────────────────────────────────────────────────────

_PARITY = {
    "N": serial.PARITY_NONE,
    "E": serial.PARITY_EVEN,
    "O": serial.PARITY_ODD,
    "M": serial.PARITY_MARK,
    "S": serial.PARITY_SPACE,
}

_STOPBITS = {
    1:   serial.STOPBITS_ONE,
    1.5: serial.STOPBITS_ONE_POINT_FIVE,
    2:   serial.STOPBITS_TWO,
}

_DATABITS = {
    5: serial.FIVEBITS,
    6: serial.SIXBITS,
    7: serial.SEVENBITS,
    8: serial.EIGHTBITS,
}


# ── helpers ───────────────────────────────────────────────────────────────────

def _get_port(port: str) -> serial.Serial:
    ser = _ports.get(port)
    if ser is None:
        raise ValueError(
            f"port '{port}' is not open. Call serial_open first."
        )
    if not ser.is_open:
        raise ValueError(f"port '{port}' was opened but is now closed.")
    return ser


def _encode_write_data(data: str, hex_mode: bool, encoding: str) -> bytes:
    if hex_mode:
        cleaned = data.replace(" ", "").replace(":", "").replace("-", "")
        return bytes.fromhex(cleaned)
    return data.encode(encoding)


def _decode_read_data(raw: bytes, hex_mode: bool, encoding: str) -> str:
    if hex_mode:
        return " ".join(f"{b:02X}" for b in raw)
    try:
        return raw.decode(encoding, errors="replace")
    except Exception:
        return " ".join(f"{b:02X}" for b in raw)


# ── tools ─────────────────────────────────────────────────────────────────────

@mcp.tool()
def serial_list_ports() -> str:
    """List all serial ports visible to the operating system.

    Returns each port's device path, description, and hardware ID.
    """
    ports = list(serial.tools.list_ports.comports())
    if not ports:
        return "no serial ports found"
    lines = [f"{len(ports)} port(s) found:"]
    for p in sorted(ports, key=lambda x: x.device):
        hwid = f"  hwid={p.hwid}" if p.hwid and p.hwid != "n/a" else ""
        lines.append(f"  {p.device:<20} {p.description}{hwid}")
    return "\n".join(lines)


@mcp.tool()
async def serial_open(
    port: str,
    baud_rate: int = 9600,
    data_bits: int = 8,
    parity: str = "N",
    stop_bits: float = 1.0,
    timeout_ms: int = 1000,
    write_timeout_ms: Optional[int] = None,
    xonxoff: bool = False,
    rtscts: bool = False,
    dsrdtr: bool = False,
) -> str:
    """Open a serial port with the specified UART configuration.

    Args:
        port: Device path (e.g. /dev/ttyUSB0, /dev/ttyS0, COM3)
              or a pyserial URL (loop:// for software loopback).
        baud_rate: Baud rate — common values: 9600, 19200, 38400, 57600,
                   115200, 230400, 460800, 921600. Default: 9600.
        data_bits: Number of data bits: 5, 6, 7, or 8. Default: 8.
        parity: Parity: N=None, E=Even, O=Odd, M=Mark, S=Space. Default: N.
        stop_bits: Stop bits: 1, 1.5, or 2. Default: 1.
        timeout_ms: Read timeout in milliseconds. Default: 1000 ms.
        write_timeout_ms: Write timeout in ms. None = blocking. Default: None.
        xonxoff: Enable software flow control (XON/XOFF). Default: False.
        rtscts: Enable RTS/CTS hardware flow control. Default: False.
        dsrdtr: Enable DSR/DTR hardware flow control. Default: False.
    """
    if parity.upper() not in _PARITY:
        raise ValueError(f"invalid parity '{parity}'; must be N/E/O/M/S")
    if data_bits not in _DATABITS:
        raise ValueError(f"invalid data_bits {data_bits}; must be 5/6/7/8")
    if stop_bits not in _STOPBITS:
        raise ValueError(f"invalid stop_bits {stop_bits}; must be 1/1.5/2")

    async with _lock:
        if port in _ports and _ports[port].is_open:
            return f"port '{port}' is already open — call serial_close first to reconfigure"

    def _do_open():
        ser = serial.serial_for_url(
            port,
            baudrate=baud_rate,
            bytesize=_DATABITS[data_bits],
            parity=_PARITY[parity.upper()],
            stopbits=_STOPBITS[stop_bits],
            timeout=timeout_ms / 1000.0,
            write_timeout=write_timeout_ms / 1000.0 if write_timeout_ms is not None else None,
            xonxoff=xonxoff,
            rtscts=rtscts,
            dsrdtr=dsrdtr,
        )
        return ser

    loop = asyncio.get_event_loop()
    ser = await loop.run_in_executor(None, _do_open)

    async with _lock:
        _ports[port] = ser

    return (
        f"opened '{port}' — "
        f"{baud_rate} {data_bits}{parity.upper()}{int(stop_bits)} "
        f"timeout={timeout_ms} ms  "
        f"flow={'XON/XOFF' if xonxoff else 'RTS/CTS' if rtscts else 'none'}"
    )


@mcp.tool()
async def serial_write(
    port: str,
    data: str,
    encoding: str = "utf-8",
    hex_mode: bool = False,
) -> str:
    """Write data to an open serial port.

    Args:
        port: Device path of the open port.
        data: Data to send. In hex_mode, provide hex bytes with optional
              separators (e.g. 'DE AD BE EF' or 'DEADBEEF').
              Otherwise provide a text string (e.g. 'AT\\r\\n').
        encoding: Text encoding when hex_mode is False. Default: utf-8.
        hex_mode: If True, parse data as hex bytes. Default: False.
    """
    async with _lock:
        ser = _get_port(port)
        raw = _encode_write_data(data, hex_mode, encoding)

        def _do_write():
            n = ser.write(raw)
            ser.flush()
            return n

        loop = asyncio.get_event_loop()
        n = await loop.run_in_executor(None, _do_write)

    hex_str = " ".join(f"{b:02X}" for b in raw)
    return f"wrote {n} byte(s) to '{port}': [{hex_str}]"


@mcp.tool()
async def serial_read(
    port: str,
    num_bytes: int = 256,
    timeout_ms: Optional[int] = None,
    hex_mode: bool = False,
    encoding: str = "utf-8",
) -> str:
    """Read bytes from an open serial port.

    Returns when num_bytes have been received, the timeout expires, or
    no data arrives before the port's configured read timeout.

    Args:
        port: Device path of the open port.
        num_bytes: Maximum number of bytes to read. Default: 256.
        timeout_ms: Override the port's read timeout for this call (ms).
                    None keeps the port's configured timeout.
        hex_mode: If True, return data as hex-encoded string. Default: False.
        encoding: Text decoding when hex_mode is False. Default: utf-8.
    """
    async with _lock:
        ser = _get_port(port)
        old_timeout = ser.timeout
        if timeout_ms is not None:
            ser.timeout = timeout_ms / 1000.0

        def _do_read():
            data = ser.read(num_bytes)
            return data

        loop = asyncio.get_event_loop()
        raw = await loop.run_in_executor(None, _do_read)

        if timeout_ms is not None:
            ser.timeout = old_timeout

    if not raw:
        return f"read 0 bytes from '{port}' (timeout)"

    decoded = _decode_read_data(raw, hex_mode, encoding)
    hex_str = " ".join(f"{b:02X}" for b in raw)
    return (
        f"read {len(raw)} byte(s) from '{port}':\n"
        f"  text : {repr(decoded)}\n"
        f"  hex  : {hex_str}"
    )


@mcp.tool()
async def serial_write_read(
    port: str,
    data: str,
    read_bytes: int = 256,
    encoding: str = "utf-8",
    hex_mode: bool = False,
    delay_ms: int = 50,
    timeout_ms: int = 1000,
) -> str:
    """Write data then read the response — the standard request-response pattern.

    Writes data, waits delay_ms for the device to respond, then reads
    up to read_bytes. Typical use: AT commands, Modbus RTU, NMEA queries.

    Args:
        port: Device path of the open port.
        data: Data to send (text or hex depending on hex_mode).
        read_bytes: Maximum response bytes to read. Default: 256.
        encoding: Text encoding/decoding. Default: utf-8.
        hex_mode: Treat data as hex; return hex. Default: False.
        delay_ms: Wait this long after writing before reading (ms). Default: 50.
        timeout_ms: Read timeout for the response (ms). Default: 1000.
    """
    async with _lock:
        ser = _get_port(port)
        raw_write = _encode_write_data(data, hex_mode, encoding)
        old_timeout = ser.timeout
        ser.timeout = timeout_ms / 1000.0

        def _do_write_read():
            ser.reset_input_buffer()
            n = ser.write(raw_write)
            ser.flush()
            return n

        loop = asyncio.get_event_loop()
        n_written = await loop.run_in_executor(None, _do_write_read)

    # Give the device time to respond
    await asyncio.sleep(delay_ms / 1000.0)

    async with _lock:
        ser = _get_port(port)

        def _do_read():
            return ser.read(read_bytes)

        raw_read = await loop.run_in_executor(None, _do_read)
        ser.timeout = old_timeout

    write_hex = " ".join(f"{b:02X}" for b in raw_write)
    if not raw_read:
        return (
            f"wrote {n_written} byte(s) to '{port}': [{write_hex}]\n"
            f"read  0 bytes (timeout — no response within {timeout_ms} ms)"
        )

    decoded = _decode_read_data(raw_read, hex_mode, encoding)
    read_hex = " ".join(f"{b:02X}" for b in raw_read)
    return (
        f"wrote {n_written} byte(s) to '{port}': [{write_hex}]\n"
        f"read  {len(raw_read)} byte(s) from '{port}':\n"
        f"  text : {repr(decoded)}\n"
        f"  hex  : {read_hex}"
    )


@mcp.tool()
async def serial_flush(port: str) -> str:
    """Flush input and output buffers of an open serial port.

    Discards any unread data in the input buffer and waits for all
    queued output to be transmitted.

    Args:
        port: Device path of the open port.
    """
    async with _lock:
        ser = _get_port(port)

        def _do_flush():
            ser.reset_input_buffer()
            ser.reset_output_buffer()
            ser.flush()

        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, _do_flush)

    return f"flushed input/output buffers of '{port}'"


@mcp.tool()
async def serial_get_status(port: str) -> str:
    """Return the current configuration and hardware signal line status.

    Shows baud rate, framing, timeouts, flow-control, and the state
    of CTS, DSR, CD, and RI signal lines (where supported by the hardware).

    Args:
        port: Device path of the open port.
    """
    async with _lock:
        ser = _get_port(port)

        def _do_status():
            try:
                cts = ser.cts
                dsr = ser.dsr
                cd  = ser.cd
                ri  = ser.ri
                signals = f"CTS={cts} DSR={dsr} CD={cd} RI={ri}"
            except Exception:
                signals = "signal lines: not available on this interface"

            in_waiting  = ser.in_waiting
            out_waiting = ser.out_waiting
            return signals, in_waiting, out_waiting

        loop = asyncio.get_event_loop()
        signals, in_w, out_w = await loop.run_in_executor(None, _do_status)

    stopbits_map = {
        serial.STOPBITS_ONE: "1",
        serial.STOPBITS_ONE_POINT_FIVE: "1.5",
        serial.STOPBITS_TWO: "2",
    }
    parity_map = {v: k for k, v in _PARITY.items()}

    lines = [
        f"port           : {port}",
        f"is_open        : {ser.is_open}",
        f"baud_rate      : {ser.baudrate}",
        f"data_bits      : {ser.bytesize}",
        f"parity         : {parity_map.get(ser.parity, ser.parity)}",
        f"stop_bits      : {stopbits_map.get(ser.stopbits, ser.stopbits)}",
        f"read_timeout   : {ser.timeout} s",
        f"write_timeout  : {ser.write_timeout} s",
        f"xonxoff        : {ser.xonxoff}",
        f"rtscts         : {ser.rtscts}",
        f"dsrdtr         : {ser.dsrdtr}",
        f"in_waiting     : {in_w} byte(s)",
        f"out_waiting    : {out_w} byte(s)",
        f"{signals}",
    ]
    return "\n".join(lines)


@mcp.tool()
async def serial_close(port: str) -> str:
    """Close an open serial port and remove it from the registry.

    Args:
        port: Device path of the open port.
    """
    async with _lock:
        ser = _ports.pop(port, None)
        if ser is None:
            return f"port '{port}' was not open"

        def _do_close():
            ser.close()

        loop = asyncio.get_event_loop()
        await loop.run_in_executor(None, _do_close)

    return f"closed '{port}'"


if __name__ == "__main__":
    mcp.run()
