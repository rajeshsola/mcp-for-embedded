"""MCP client for mqtt_mcp.py — publish and subscribe via MQTT.

Prerequisites:
    - An MQTT broker must be running and reachable.
    - paho-mqtt installed in the same environment as the server.

Usage:
    python mqtt_client.py [broker_host] [broker_port]   (defaults: localhost 1883)
"""
import asyncio
import json
import sys
from mcp_client_base import McpClient, banner, show

BROKER_HOST = sys.argv[1] if len(sys.argv) > 1 else "localhost"
BROKER_PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 1883


async def main() -> None:
    async with McpClient("mqtt_mcp.py") as client:
        banner(f"MQTT MCP Client  [{BROKER_HOST}:{BROKER_PORT}]")

        tools = await client.list_tools()
        print(f"Registered tools: {[t.name for t in tools]}")

        # ── QoS 0 publish ─────────────────────────────────────────────────────
        show(
            "mqtt_publish  test/hello  QoS 0",
            await client.call(
                "mqtt_publish",
                host=BROKER_HOST,
                port=BROKER_PORT,
                topic="test/hello",
                payload="Hello from FastMCP!",
                qos=0,
            ),
        )

        # ── QoS 1 publish (JSON payload, retained) ────────────────────────────
        telemetry = json.dumps({"speed": 120, "rpm": 3000, "fuel_level": 80})
        show(
            "mqtt_publish  vehicletelemetry/all  QoS 1  retained",
            await client.call(
                "mqtt_publish",
                host=BROKER_HOST,
                port=BROKER_PORT,
                topic="vehicletelemetry/all",
                payload=telemetry,
                qos=1,
                retain=True,
            ),
        )

        # ── publish speed only to sub-topic ──────────────────────────────────
        show(
            "mqtt_publish  vehicletelemetry/speed  QoS 1",
            await client.call(
                "mqtt_publish",
                host=BROKER_HOST,
                port=BROKER_PORT,
                topic="vehicletelemetry/speed",
                payload=json.dumps({"speed": 95}),
                qos=1,
            ),
        )

        # ── subscribe with wildcard to collect published messages ─────────────
        # The broker should deliver the retained vehicletelemetry/all message
        # and any new messages on vehicletelemetry/#.
        show(
            "mqtt_subscribe  vehicletelemetry/#  max=5  timeout=3000 ms",
            await client.call(
                "mqtt_subscribe",
                host=BROKER_HOST,
                port=BROKER_PORT,
                topic="vehicletelemetry/#",
                qos=1,
                max_messages=5,
                timeout_ms=3000,
            ),
        )

        # ── subscribe test/# — should get at least the hello message if retained
        show(
            "mqtt_subscribe  test/#  max=3  timeout=2000 ms",
            await client.call(
                "mqtt_subscribe",
                host=BROKER_HOST,
                port=BROKER_PORT,
                topic="test/#",
                max_messages=3,
                timeout_ms=2000,
            ),
        )

        # ── error path: invalid QoS ───────────────────────────────────────────
        print("\n[mqtt_publish with qos=5 — expected error]")
        try:
            await client.call(
                "mqtt_publish",
                host=BROKER_HOST,
                port=BROKER_PORT,
                topic="test/err",
                payload="bad qos",
                qos=5,
            )
        except RuntimeError as e:
            print(f"  got expected error: {e}")


if __name__ == "__main__":
    asyncio.run(main())
