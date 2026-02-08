# unified_server.py
from fastmcp import FastMCP
from pydantic import BaseModel
from typing import Optional, List, Dict
import datetime
import socket
import requests

app = FastMCP("unified-iot-mcp-server")


# ----------------------------------------------------
# MODELS
# ----------------------------------------------------
class SensorData(BaseModel):
    temperature: Optional[float] = None
    humidity: Optional[float] = None
    pressure: Optional[float] = None
    latitude: Optional[float] = None
    longitude: Optional[float] = None
    timestamp: Optional[str] = None


class IoTRequest(BaseModel):
    platform: str
    target: str
    api_key: Optional[str]
    sensor_data: SensorData


# ----------------------------------------------------
# SAMPLING (IN-MEMORY)
# ----------------------------------------------------
SAMPLE_BUFFER = []


@app.tool()
def sample_sensor_data(temperature: float, humidity: float) -> Dict:
    """Collect one sensor sample."""
    entry = {
        "temperature": temperature,
        "humidity": humidity,
        "timestamp": datetime.datetime.now().isoformat()
    }
    SAMPLE_BUFFER.append(entry)
    return {"status": "sample_recorded", "data": entry}


@app.tool()
def get_samples() -> List[dict]:
    """Return all samples."""
    return SAMPLE_BUFFER


@app.tool()
def clear_samples() -> str:
    SAMPLE_BUFFER.clear()
    return "Sample buffer cleared."


@app.tool()
def summarize_samples() -> dict:
    """Return statistical summary."""
    if not SAMPLE_BUFFER:
        return {"error": "No samples available"}

    temps = [s["temperature"] for s in SAMPLE_BUFFER]
    hums = [s["humidity"] for s in SAMPLE_BUFFER]

    return {
        "count": len(SAMPLE_BUFFER),
        "avg_temperature": sum(temps) / len(temps),
        "avg_humidity": sum(hums) / len(hums),
        "min_temperature": min(temps),
        "max_temperature": max(temps),
    }


# ----------------------------------------------------
# ELlCITATION TOOLS
# ----------------------------------------------------
@app.tool()
def elicit_parameters(task: str, params: dict) -> dict:
    """Return missing parameters for a task."""
    missing = []

    if task == "publish_sensor_data":
        if "platform" not in params:
            missing.append("Which IoT platform should be used?")
        if "target" not in params:
            missing.append("Provide the endpoint/broker/channel ID.")
        if "sensor_data" not in params:
            missing.append("Provide sensor_data (temperature, humidity, etc.)")

    return {
        "task": task,
        "missing_parameters": missing,
        "complete": len(missing) == 0
    }


# ----------------------------------------------------
# PROTOCOL DISCOVERY
# ----------------------------------------------------
@app.tool()
def discover_protocols(endpoint: str) -> Dict:
    """Lightweight IoT protocol discovery."""
    supported = []

    # HTTP probe
    try:
        requests.options(endpoint, timeout=3)
        supported.append("http-rest")
    except:
        pass

    # OpenAPI/Swagger
    for path in ["/swagger.json", "/openapi.json", "/api", "/api/docs"]:
        try:
            r = requests.get(endpoint.rstrip("/") + path, timeout=3)
            if r.status_code == 200:
                supported.append("openapi-rest")
        except:
            pass

    # MQTT probe
    hostname = endpoint.replace("http://", "").replace("https://", "").split("/")[0]

    def port_open(host, port):
        try:
            s = socket.create_connection((host, port), timeout=2)
            s.close()
            return True
        except:
            return False

    if port_open(hostname, 1883):
        supported.append("mqtt")
    if port_open(hostname, 8883):
        supported.append("mqtts")

    # CoAP probe
    if port_open(hostname, 5683):
        supported.append("coap")

    return {
        "endpoint": endpoint,
        "protocols": list(set(supported))
    }


# ----------------------------------------------------
# UNIFIED IOT PUBLISH TOOL
# ----------------------------------------------------
@app.tool()
def publish_sensor_data(request: IoTRequest) -> dict:
    """Unified publishing tool with elicitation behavior."""
    required = ["platform", "target", "sensor_data"]
    missing = [p for p in required if getattr(request, p) is None]

    if missing:
        return {
            "status": "needs_elicitation",
            "missing_parameters": missing
        }

    # Mock publish (replace with real logic)
    return {
        "status": "success",
        "message": f"Published to {request.platform}",
        "data": request.sensor_data.dict()
    }


# ----------------------------------------------------
# SENSOR ANALYSIS TOOL
# ----------------------------------------------------
@app.tool()
def analyze_sensor_data(sensor: SensorData) -> str:
    """Return diagnostics."""
    issues = []
    if sensor.temperature and sensor.temperature > 35:
        issues.append("High temperature")
    if sensor.humidity and sensor.humidity > 80:
        issues.append("High humidity")

    if not issues:
        return "Sensor values are normal."

    return "Issues detected: " + ", ".join(issues)


# ----------------------------------------------------
# RESOURCES
# ----------------------------------------------------
@app.resource("resource://metadata")
def server_info():
    return {
        "name": "Unified IoT MCP Server",
        "version": "1.0",
        "features": [
            "sampling",
            "elicitation",
            "protocol discovery",
            "iot publish",
            "sensor analysis"
        ]
    }


@app.resource("resource://sampling")
def sensor_samples_resource():
    return SAMPLE_BUFFER


@app.resource("resource://elicit")
def elicitation_guidelines():
    return {
        "rules": [
            "Always clarify missing fields before calling publish tool.",
            "Do not assume IoT endpoints.",
            "Ask for sensor values explicitly.",
        ]
    }

@app.resource("resource://elicit")
def elicitation_guidelines():
    return {
        "rules": [
            "Always clarify missing fields before calling publish tool.",
            "Do not assume IoT endpoints.",
            "Ask for sensor values explicitly.",
        ]
    }

# ----------------------------------------------------
# PROMPTS
# ----------------------------------------------------
@app.prompt()
def explain_sensor_data(sensor: SensorData) -> str:
    return f"""
Explain this sensor reading:
Temperature: {sensor.temperature}
Humidity: {sensor.humidity}
"""


@app.prompt()
def summarize_history(history: List[SensorData]) -> str:
    lines = [f"{h.timestamp}: T={h.temperature} H={h.humidity}" for h in history]
    return "Summarize these readings:\n" + "\n"
    
@app.prompt()
def explain_sampling(samples: list) -> str:
    lines = [f"{s['timestamp']}: T={s['temperature']} H={s['humidity']}" for s in samples]
    return "Explain the following sampled data:\n" + "\n".join(lines)


@app.prompt()
def elicit_missing_info(goal: str, provided: dict) -> str:
    return f"""
You must ask the user for missing information.

Goal: {goal}
Given: {provided}

Ask only the missing details.
"""

"""
# ----------------------------------------------------
# ROOTS (Top-level metadata)
# ----------------------------------------------------
@root()
def server_root():
    return {
        "name": "Unified IoT MCP Server",
        "description": "Server providing IoT sampling, elicitation, protocol discovery and sensor analysis.",
        "version": "1.0"
    }


@root()
def tools_root():
    return {
        "tools": [
            "sample_sensor_data",
            "get_samples",
            "clear_samples",
            "summarize_samples",
            "publish_sensor_data",
            "discover_protocols",
            "analyze_sensor_data",
            "elicit_parameters"
        ]
    }


@root()
def elicitation_root():
    return {
        "elicitation_policy": [
            "Ask user for unclear sensor values.",
            "Ask for IoT platform if missing.",
            "Ask for endpoint if not provided.",
        ]
    }


@root()
def sampling_root():
    return {
        "sampling_capabilities": {
            "collect": "sample_sensor_data",
            "view": "get_samples",
            "clear": "clear_samples",
            "summarize": "summarize_samples"
        }
    }
"""

# ----------------------------------------------------
# RUN SERVER
# ----------------------------------------------------
if __name__ == "__main__":
    app.run()
