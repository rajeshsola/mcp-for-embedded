"""MCP client for socketcan_mcp.py — send and receive CAN frames.

Prerequisites:
    - A virtual CAN interface must exist:
        sudo modprobe vcan
        sudo ip link add dev vcan0 type vcan
        sudo ip link set up vcan0
    - python-can installed in the same environment as the server.

Usage:
    python socketcan_client.py [interface]       (default: vcan0)
"""
import asyncio
import sys
from mcp_client_base import McpClient, banner, show

INTERFACE = sys.argv[1] if len(sys.argv) > 1 else "vcan0"


async def main() -> None:
    async with McpClient("socketcan_mcp.py") as client:
        banner(f"SocketCAN MCP Client  [{INTERFACE}]")

        tools = await client.list_tools()
        print(f"Registered tools: {[t.name for t in tools]}")

        # ── send a standard 11-bit frame ──────────────────────────────────────
        show(
            "send_frame  ID=0x123 data=DEADBEEF",
            await client.call(
                "send_frame",
                interface=INTERFACE,
                can_id=0x123,          # 291 decimal — standard ID (≤ 0x7FF)
                data="DEADBEEF",
            ),
        )

        # ── send an extended 29-bit frame ─────────────────────────────────────
        show(
            "send_frame  ID=0x1FFFF data=0102030405060708 (extended)",
            await client.call(
                "send_frame",
                interface=INTERFACE,
                can_id=0x1FFFF,
                data="0102030405060708",
                extended=True,
            ),
        )

        # ── receive the frame we just sent ────────────────────────────────────
        # On a vcan loopback interface, transmitted frames are also received.
        show(
            "receive_frame  timeout=500 ms",
            await client.call(
                "receive_frame",
                interface=INTERFACE,
                timeout_ms=500,
            ),
        )

        # ── send a vehicle telemetry frame (CAN ID 0x100) ────────────────────
        # speed=120 km/h (0x0078), rpm=3000 (0x0BB8), fuel=80, mode=1
        show(
            "send_frame  ID=0x100 vehicle telemetry",
            await client.call(
                "send_frame",
                interface=INTERFACE,
                can_id=0x100,
                data="00780BB8501",   # speed | rpm | fuel | mode (partial, valid hex even length)
            ),
        )

        # ── error path: data too long ─────────────────────────────────────────
        print("\n[send_frame with 9-byte payload — expected error]")
        try:
            await client.call(
                "send_frame",
                interface=INTERFACE,
                can_id=0x1,
                data="AABBCCDDEE112233FF",   # 9 bytes
            )
        except RuntimeError as e:
            print(f"  got expected error: {e}")


if __name__ == "__main__":
    asyncio.run(main())
