"""MCP client for serial_mcp.py — demonstrates all serial port tools.

Runs all tests against pyserial's built-in  loop://  URL — no hardware,
no socat, no kernel modules needed.

Usage:
    cd python-mcp
    python serial_client.py
"""
import asyncio
from mcp_client_base import McpClient, banner, show


async def main() -> None:
    async with McpClient("serial_mcp.py") as client:
        banner("Serial MCP Client")

        # ── list tools ────────────────────────────────────────────────────────
        tools = await client.list_tools()
        print(f"Registered tools ({len(tools)}):")
        for t in tools:
            print(f"  {t.name}")

        # ── list physical ports ───────────────────────────────────────────────
        banner("serial_list_ports()")
        show("list_ports", await client.call("serial_list_ports"))

        # ── open loop:// — software loopback, no hardware required ────────────
        banner("serial_open(port='loop://')")
        show("open loop://", await client.call(
            "serial_open",
            port="loop://",
            baud_rate=115200,
            timeout_ms=500,
        ))

        # ── status before any traffic ─────────────────────────────────────────
        banner("serial_get_status()")
        show("status", await client.call("serial_get_status", port="loop://"))

        # ── write text, read it back ──────────────────────────────────────────
        banner("serial_write + serial_read (text mode)")
        msg = "Hello, serial world!\r\n"
        show("write", await client.call(
            "serial_write",
            port="loop://",
            data=msg,
        ))
        # loop:// reflects bytes immediately — read exactly what was written
        show("read", await client.call(
            "serial_read",
            port="loop://",
            num_bytes=len(msg.encode()),
        ))

        # ── write hex, read hex ───────────────────────────────────────────────
        banner("serial_write + serial_read (hex mode)")
        show("write hex", await client.call(
            "serial_write",
            port="loop://",
            data="DE AD BE EF CA FE",
            hex_mode=True,
        ))
        show("read hex", await client.call(
            "serial_read",
            port="loop://",
            num_bytes=6,
            hex_mode=True,
        ))

        # ── write-read round-trip ─────────────────────────────────────────────
        banner("serial_write_read (AT-command style)")
        show("write_read", await client.call(
            "serial_write_read",
            port="loop://",
            data="AT+INFO\r\n",
            read_bytes=64,
            delay_ms=10,
        ))

        # ── flush ─────────────────────────────────────────────────────────────
        banner("serial_flush()")
        show("flush", await client.call("serial_flush", port="loop://"))

        # ── timeout path: read when buffer is empty ───────────────────────────
        banner("serial_read — empty buffer (timeout expected)")
        show("read (empty)", await client.call(
            "serial_read",
            port="loop://",
            num_bytes=16,
            timeout_ms=100,
        ))

        # ── duplicate-open guard ──────────────────────────────────────────────
        banner("serial_open — already-open guard")
        show("open again", await client.call(
            "serial_open",
            port="loop://",
        ))

        # ── close ─────────────────────────────────────────────────────────────
        banner("serial_close()")
        show("close", await client.call("serial_close", port="loop://"))

        # ── close again — idempotent ──────────────────────────────────────────
        show("close (again)", await client.call("serial_close", port="loop://"))

        # ── error path: write to closed port ─────────────────────────────────
        banner("Error paths")
        print("[write to closed port — expected error]")
        try:
            await client.call("serial_write", port="loop://", data="x")
        except RuntimeError as e:
            print(f"  got expected error: {e}")

        print("\n[read from closed port — expected error]")
        try:
            await client.call("serial_read", port="loop://", num_bytes=4)
        except RuntimeError as e:
            print(f"  got expected error: {e}")

        print("\n[open with invalid parity — expected error]")
        try:
            await client.call("serial_open", port="loop://", parity="Z")
        except RuntimeError as e:
            print(f"  got expected error: {e}")

        print("\n[open with invalid baud + data test — success]")
        show("open 7-bit", await client.call(
            "serial_open",
            port="loop://",
            baud_rate=9600,
            data_bits=7,
            parity="E",
            stop_bits=1,
        ))
        show("write 7-bit text", await client.call(
            "serial_write",
            port="loop://",
            data="test",
        ))
        show("read 7-bit text", await client.call(
            "serial_read",
            port="loop://",
            num_bytes=4,
        ))
        show("close 7-bit", await client.call("serial_close", port="loop://"))


if __name__ == "__main__":
    asyncio.run(main())
