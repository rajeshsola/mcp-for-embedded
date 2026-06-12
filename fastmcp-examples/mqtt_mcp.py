"""MQTT MCP server — publish and subscribe using paho-mqtt.

Requires: paho-mqtt
    pip install paho-mqtt
"""
import os
import threading
import time
from typing import Optional
import paho.mqtt.client as mqtt
from fastmcp import FastMCP

mcp = FastMCP(
    "mqtt",
    instructions=(
        "MCP server for MQTT publish/subscribe over any MQTT 3.1.1 broker. "
        "Use mqtt_publish to send a message to a topic and mqtt_subscribe to "
        "collect messages from a topic or wildcard filter. "
        "Both tools accept optional credentials for authenticated brokers."
    ),
)


def _unique_client_id(prefix: str) -> str:
    return f"{prefix}-{os.getpid()}-{time.time_ns() % 1_000_000}"


def _make_client(
    client_id: Optional[str],
    prefix: str,
    username: Optional[str],
    password: Optional[str],
) -> mqtt.Client:
    cid = client_id or _unique_client_id(prefix)
    client = mqtt.Client(client_id=cid)
    if username and password:
        client.username_pw_set(username, password)
    return client


@mcp.tool()
def mqtt_publish(
    host: str,
    topic: str,
    payload: str,
    port: int = 1883,
    qos: int = 0,
    retain: bool = False,
    client_id: Optional[str] = None,
    username: Optional[str] = None,
    password: Optional[str] = None,
) -> str:
    """Connect to an MQTT broker and publish a single message to a topic.

    Returns when the broker has confirmed delivery (QoS 1/2) or the message
    has been written to the network (QoS 0).

    Args:
        host: MQTT broker hostname or IP address (e.g. localhost, 192.168.1.1).
        topic: Topic to publish to (e.g. sensors/temperature).
        payload: Message payload as a UTF-8 string.
        port: MQTT broker port (default: 1883).
        qos: QoS level: 0=at-most-once, 1=at-least-once, 2=exactly-once (default: 0).
        retain: Retain the message on the broker (default: False).
        client_id: MQTT client ID; auto-generated when omitted.
        username: Username for broker authentication.
        password: Password for broker authentication.
    """
    if qos not in (0, 1, 2):
        raise ValueError(f"invalid QoS {qos}: must be 0, 1, or 2")

    done = threading.Event()
    error: list[str] = []

    client = _make_client(client_id, "mcp-pub", username, password)

    def on_connect(c, userdata, flags, rc):
        if rc != 0:
            error.append(f"connection refused: rc={rc}")
            done.set()
            return
        c.publish(topic, payload, qos=qos, retain=retain)
        if qos == 0:
            # No puback for QoS 0 — signal after queuing
            done.set()

    def on_publish(c, userdata, mid):
        done.set()

    client.on_connect = on_connect
    client.on_publish = on_publish
    client.connect(host, port, keepalive=10)
    client.loop_start()

    if not done.wait(timeout=10):
        client.loop_stop()
        client.disconnect()
        raise TimeoutError(
            "timed out waiting for broker confirmation (10 s); "
            "check host/port/credentials"
        )

    client.loop_stop()
    client.disconnect()

    if error:
        raise RuntimeError(error[0])

    return f"published to '{topic}' on {host}:{port} [qos={qos} retain={retain}]"


@mcp.tool()
def mqtt_subscribe(
    host: str,
    topic: str,
    port: int = 1883,
    qos: int = 0,
    max_messages: int = 10,
    timeout_ms: int = 5000,
    client_id: Optional[str] = None,
    username: Optional[str] = None,
    password: Optional[str] = None,
) -> str:
    """Subscribe to an MQTT topic and collect incoming messages until the timeout
    or message limit is reached. Wildcards + and # are supported.

    Args:
        host: MQTT broker hostname or IP address.
        topic: Topic filter to subscribe to; supports wildcards + and # (e.g. sensors/#).
        port: MQTT broker port (default: 1883).
        qos: QoS level for the subscription: 0, 1, or 2 (default: 0).
        max_messages: Stop after collecting this many messages (default: 10).
        timeout_ms: Total time to wait for messages in milliseconds (default: 5000).
        client_id: MQTT client ID; auto-generated when omitted.
        username: Username for broker authentication.
        password: Password for broker authentication.
    """
    if qos not in (0, 1, 2):
        raise ValueError(f"invalid QoS {qos}: must be 0, 1, or 2")

    messages: list[str] = []
    done = threading.Event()
    error: list[str] = []

    client = _make_client(client_id, "mcp-sub", username, password)

    def on_connect(c, userdata, flags, rc):
        if rc != 0:
            error.append(f"connection refused: rc={rc}")
            done.set()
            return
        c.subscribe(topic, qos)

    def on_message(c, userdata, msg):
        try:
            text = msg.payload.decode("utf-8", errors="replace")
        except Exception:
            text = repr(msg.payload)
        messages.append(f"[{msg.topic}] {text}")
        if len(messages) >= max_messages:
            done.set()

    client.on_connect = on_connect
    client.on_message = on_message
    client.connect(host, port, keepalive=30)
    client.loop_start()
    done.wait(timeout=timeout_ms / 1000.0)
    client.loop_stop()
    client.disconnect()

    if error:
        raise RuntimeError(error[0])

    if not messages:
        return (
            f"subscribed to '{topic}' on {host}:{port} — "
            f"no messages received within {timeout_ms} ms"
        )
    return (
        f"received {len(messages)} message(s) from '{topic}' on {host}:{port}:\n"
        + "\n".join(messages)
    )


if __name__ == "__main__":
    mcp.run()
