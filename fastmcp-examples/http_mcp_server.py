"""MCP server over Streamable HTTP transport (spec 2025-03-26).

FastMCP launches a Starlette/uvicorn ASGI app.
The MCP endpoint is reachable at:

    POST http://127.0.0.1:8000/mcp

Usage:
    python http_mcp_server.py
"""
from fastmcp import FastMCP

mcp = FastMCP(
    "http-demo",
    instructions="HTTP MCP demo server: say_hello and square tools.",
)


@mcp.tool()
def say_hello(name: str) -> str:
    """Greet someone by name and return a friendly message."""
    return f"Hello, {name}! Greetings from MCP over HTTP."


@mcp.tool()
def square(x: float) -> str:
    """Return the square of a number (x * x)."""
    return str(x * x)


if __name__ == "__main__":
    # transport="streamable-http"  →  POST /mcp  (JSON-RPC 2.0 over HTTP)
    # host / port match FastMCP defaults so the raw client can find the server.
    mcp.run(
        transport="streamable-http",
        host="127.0.0.1",
        port=8000,
        show_banner=False,   # suppress the ASCII art banner
    )
