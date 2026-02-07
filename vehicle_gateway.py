# server.py
from fastmcp import FastMCP
from typing import Optional, Literal
from pydantic import BaseModel
import datetime
import can
import json

app = FastMCP("iot-gateway-mcp")

# ---------------------------------------
# Common Sensor Data Model
# ---------------------------------------
class SensorData(BaseModel):
    speed: Optional[float] = None
    rpm: Optional[float] = None
    fuel: Optional[float] = None
    latitude: Optional[float] = None
    longitude: Optional[float] = None
    timestamp: Optional[str] = None


# ---------------------------------------
# Unified IoT publish request
# ---------------------------------------
class IoTRequest(BaseModel):
    platform: Literal[
        "thingsboard",
        "thingspeak",
        "custom_rest",
        "custom_mqtt"
    ]

    target: str              # URL, broker:port, channelid, etc.
    api_key: Optional[str]   # tokens, keys if needed
    sensor_data: SensorData
    
class Config(BaseModel):
    mqtt_broker : Optional[str]
    mqtt_topic : Optional[str]
    http_rest_end_point : Optional[str]
    api_key: Optional[str]
    
CAN_INTERFACE = "vcan0"

def init_can():
    return can.interface.Bus(channel=CAN_INTERFACE, interface="socketcan")
    
can_bus = init_can()
    

@app.tool
def read_sensor_data(timeout_ms: int = 10000) -> str:  
    msg = can_bus.recv(timeout_ms / 1000.0)
    if msg is None:
        return {"status": "timeout"}
    can_id = msg.arbitration_id
    """
    return {
        "can_id": msg.arbitration_id,
        "timestamp": msg.timestamp,
        "data": list(msg.data),
        "dlc": msg.dlc,
        "is_extended": msg.is_extended_id,
    }
    """
    if can_id == 0x101:
        return "Speed is {}".format(msg.data[0])
    elif can_id == 0x102:
        return "RPM is {}".format(msg.data[0])
    elif can_id == 0x103:
        return "Fuel level is {}".format(msg.data[0])
    elif can_id == 0x100:
        reading = { "speed" : msg.data[0], "rpm" : msg.data[1], "fuel" : msg.data[2] }
        return json.dumps(reading)
    else:
        return "Invalid CAN Frame"
        
@app.tool
async def can_read_callback() -> str:
    return await anyio.to_thread.run_sync(read_sensor_data, 2000)

@app.tool()
def publish_telemetry(protocol:str, platform:str, telemetry_data : str, cfg : Config  ) -> str:
    if protocol == "mqtt":
        if platform == "thingspeak" :
            resp = "Sending {} to thingspeak using mqtt".format(telemetry_data)
        elif platform == "thingsboard" :
            resp = "Sending {} to thingsboard using mqtt".format(telemetry_data) 
        else :
            resp = "Sending {} to custom mqtt broker ".format(telemetry_data)
    if protocol == "http":
        if platform == "thingspeak" :
            resp = "Sending {} to thingspeak using http".format(telemetry_data)  # retrieve response and return
        elif platform == "thingsboard" :
            resp = "Sending {} to thingspeak using http".format(telemetry_data)  # retrieve response and return
        else :
            resp = "Sending {} to thingspeak using http".format(telemetry_data)  # retrieve response and return
    return resp
    
@app.tool()
def read_from_cloud(protocol:str, platform:str, cfg : Config) -> str :
    if protocol == "mqtt":
        "subscribe to MQTT broker"
        pass
    else:
        "unknown protocol"
        pass
        
# ---------------------------------------
# SINGLE Unified Tool
# ---------------------------------------
@app.tool()
def publish_sensor_data(request: IoTRequest) -> str:
    """
    Publish environmental sensor data to any IoT platform.
    LLM decides how to fill the protocol/platform details.

    Example natural language mapping:
    "Send this data to ThingsBoard" =>
        platform="thingsboard", target="http://.../token"
    """

    data = request.sensor_data.dict(exclude_none=True)

    # Dynamic dispatch to internal handlers
    if request.platform == "thingsboard":
        return send_to_thingsboard(request.target, request.api_key, data)

    if request.platform == "thingspeak":
        return send_to_thingspeak(request.target, request.api_key, data)

    if request.platform == "custom_rest":
        return send_to_rest(request.target, request.api_key, data)

    if request.platform == "custom_mqtt":
        return send_to_mqtt(request.target, data)

    return "Unsupported platform"


# ---------------------------------------
# Internal protocol handlers (optional)
# You can fill in actual sending logic later.
# ---------------------------------------

def send_to_thingsboard(endpoint, token, data):
    return f"[Mock] Sent to ThingsBoard at {endpoint} with token={token}: {data}"


def send_to_thingspeak(channel_id, api_key, data):
    return f"[Mock] Sent to ThingSpeak channel {channel_id} with key={api_key}: {data}"


def send_to_rest(url, api_key, data):
    return f"[Mock] Sent REST POST to {url} api_key={api_key}: {data}"


def send_to_mqtt(broker, data):
    return f"[Mock] Published MQTT to {broker}: {data}"


# ---------------------------------------
# Run MCP server
# ---------------------------------------
if __name__ == "__main__":
    app.run()


# /home/rajeshsola/Public/MCP/Python/socketcan-mcp-server/bin/python3 iot_publish.py
# npx @modelcontextprotocol/inspector cargo run -p mcp-server-examples --example  
