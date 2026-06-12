"""SocketCAN MCP server — send and receive CAN frames on Linux SocketCAN interfaces.

Requires: python-can
    pip install python-can
"""
import asyncio
from typing import Optional
import can
from fastmcp import FastMCP

mcp = FastMCP(
    "socketcan",
    instructions=(
        "MCP server for sending and receiving CAN frames via Linux SocketCAN. "
        "Requires a configured SocketCAN interface (e.g. vcan0 or can0). "
        "Use send_frame to transmit a frame and receive_frame to read one."
    ),
)


def _parse_hex_data(s: str) -> bytes:
    """Strip separators and decode a hex string to bytes."""
    s = s.replace(" ", "").replace(":", "").replace("-", "")
    if len(s) % 2 != 0:
        raise ValueError("hex string must have an even number of nibbles")
    return bytes.fromhex(s)


def _hex_encode(data: bytes) -> str:
    return " ".join(f"{b:02X}" for b in data)


@mcp.tool()
async def send_frame(
    interface: str,
    can_id: int,
    data: str,
    extended: Optional[bool] = None,
) -> str:
    """Send a CAN data frame on a Linux SocketCAN interface.

    Args:
        interface: SocketCAN interface name (e.g. vcan0, can0).
        can_id: CAN frame ID as a decimal integer (e.g. 291 for 0x123).
                IDs above 0x7FF are treated as extended automatically.
        data: Frame payload as a hex string with no separators
              (e.g. DEADBEEF01020304); maximum 8 bytes.
        extended: Force 29-bit extended CAN ID; auto-detected when can_id > 0x7FF.
    """
    data_bytes = _parse_hex_data(data)
    if len(data_bytes) > 8:
        raise ValueError(f"CAN data length {len(data_bytes)} exceeds 8-byte maximum")

    use_extended = extended if extended is not None else (can_id > 0x7FF)

    if use_extended and can_id > 0x1FFF_FFFF:
        raise ValueError(f"invalid extended CAN ID 0x{can_id:X} (max 0x1FFFFFFF)")
    if not use_extended and can_id > 0x7FF:
        raise ValueError(f"invalid standard CAN ID 0x{can_id:X} (max 0x7FF)")

    def _send() -> None:
        bus = can.interface.Bus(channel=interface, bustype="socketcan")
        try:
            msg = can.Message(
                arbitration_id=can_id,
                data=data_bytes,
                is_extended_id=use_extended,
            )
            bus.send(msg)
        finally:
            bus.shutdown()

    loop = asyncio.get_event_loop()
    await loop.run_in_executor(None, _send)

    id_type = "extended" if use_extended else "standard"
    return (
        f"sent: ID=0x{can_id:X} ({id_type}) "
        f"len={len(data_bytes)} data=[{_hex_encode(data_bytes)}]"
    )


@mcp.tool()
async def receive_frame(
    interface: str,
    timeout_ms: Optional[int] = 1000,
) -> str:
    """Receive a single CAN frame from a Linux SocketCAN interface.

    Args:
        interface: SocketCAN interface name (e.g. vcan0, can0).
        timeout_ms: How long to wait for a frame in milliseconds (default: 1000).
    """
    timeout_s = (timeout_ms if timeout_ms is not None else 1000) / 1000.0

    def _receive() -> can.Message:
        bus = can.interface.Bus(channel=interface, bustype="socketcan")
        try:
            msg = bus.recv(timeout=timeout_s)
            if msg is None:
                raise TimeoutError(f"no frame received within {timeout_ms} ms")
            return msg
        finally:
            bus.shutdown()

    loop = asyncio.get_event_loop()
    msg: can.Message = await loop.run_in_executor(None, _receive)

    id_type = "extended" if msg.is_extended_id else "standard"

    if msg.is_error_frame:
        return "error frame received"
    if msg.is_remote_frame:
        return (
            f"remote frame: ID=0x{msg.arbitration_id:X} ({id_type}) dlc={msg.dlc}"
        )
    hex_data = _hex_encode(bytes(msg.data))
    return (
        f"data frame: ID=0x{msg.arbitration_id:X} ({id_type}) "
        f"len={msg.dlc} data=[{hex_data}]"
    )


if __name__ == "__main__":
    mcp.run()
