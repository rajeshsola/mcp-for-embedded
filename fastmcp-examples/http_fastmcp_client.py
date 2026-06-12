"""FastMCP HTTP client for http_mcp_server.py.

Uses the FastMCP Client with Streamable HTTP transport.
All session management, SSE parsing, JSON-RPC framing, and the
Mcp-Session-Id header are handled automatically by the library.

Transport selection
───────────────────
FastMCP infers the transport from what you pass to Client():

    Client("http://...")          →  StreamableHttpTransport  (auto-inferred)
    Client("path/to/server.py")   →  PythonStdioTransport    (auto-inferred)
    Client(FastMCP_instance)      →  FastMCPTransport         (in-process)
    Client(StreamableHttpTransport("http://..."))  →  explicit

Both styles are shown below.

Usage:
    # Terminal 1 — start the server
    python http_mcp_server.py

    # Terminal 2 — run this client
    python http_fastmcp_client.py
"""

import asyncio
from fastmcp import Client
from fastmcp.client.transports import StreamableHttpTransport
from fastmcp.exceptions import ToolError

SERVER_URL = "http://127.0.0.1:8000/mcp"

# ── helper ────────────────────────────────────────────────────────────────────

def text_of(result) -> str:
    """Extract concatenated text from a CallToolResult."""
    return "\n".join(
        c.text for c in result.content if hasattr(c, "text")
    )


def banner(title: str) -> None:
    print(f"\n{'═' * 56}")
    print(f"  {title}")
    print(f"{'═' * 56}")


# ── demo functions ────────────────────────────────────────────────────────────

async def demo_list_tools(client: Client) -> None:
    """Show every tool the server advertises with its input schema."""
    banner("list_tools()")

    tools = await client.list_tools()
    print(f"  {len(tools)} tool(s) registered on the server:\n")

    for tool in tools:
        schema = tool.inputSchema or {}
        props  = schema.get("properties", {})
        req    = schema.get("required", [])
        params = ", ".join(
            f"{k}: {v.get('type','?')}"
            + (" (required)" if k in req else "")
            for k, v in props.items()
        )
        print(f"  ┌ {tool.name}({params})")
        print(f"  │ {tool.description}")
        print()


async def demo_say_hello(client: Client) -> None:
    """Call the say_hello tool with different names."""
    banner("call_tool('say_hello', ...)")

    for name in ["World", "FastMCP", "HTTP Transport"]:
        result = await client.call_tool("say_hello", {"name": name})
        print(f"  say_hello({name!r:20s}) → {text_of(result)}")


async def demo_square(client: Client) -> None:
    """Call the square tool with several inputs."""
    banner("call_tool('square', ...)")

    for x in [0, 5, 9, 12.5, -4]:
        result = await client.call_tool("square", {"x": x})
        print(f"  square({x:>6})              → {text_of(result)}")


async def demo_raise_on_error(client: Client) -> None:
    """Demonstrate raise_on_error=True (default) — ToolError is raised."""
    banner("Error handling — raise_on_error=True (default)")

    print("  Calling say_hello() with missing required argument 'name'...")
    try:
        await client.call_tool("say_hello", {})        # 'name' is required
    except ToolError as e:
        print(f"  ToolError caught: {e}")

    print("\n  Calling unknown_tool() ...")
    try:
        await client.call_tool("unknown_tool", {})
    except ToolError as e:
        print(f"  ToolError caught: {e}")


async def demo_no_raise(client: Client) -> None:
    """Demonstrate raise_on_error=False — inspect isError manually."""
    banner("Error handling — raise_on_error=False (manual check)")

    # call_tool_mcp returns the raw mcp.types.CallToolResult — isError (camelCase)
    # call_tool with raise_on_error=False returns FastMCP's CallToolResult  — is_error (snake_case)
    result = await client.call_tool(
        "say_hello", {},
        raise_on_error=False,              # suppress the automatic ToolError
    )
    print(f"  is_error : {result.is_error}")
    print(f"  content  : {text_of(result)}")


async def demo_structured_content(client: Client) -> None:
    """FastMCP servers also return structuredContent alongside text content."""
    banner("structuredContent (FastMCP extension)")

    # call_tool_mcp returns raw mcp.types.CallToolResult with camelCase fields
    result = await client.call_tool_mcp("square", {"x": 7})
    print(f"  isError          : {result.isError}")        # camelCase — raw protocol field
    print(f"  content[0].text  : {result.content[0].text}")
    if result.structuredContent:
        print(f"  structuredContent: {result.structuredContent}")


# ── two ways to create an HTTP client ────────────────────────────────────────

async def run_with_url_string() -> None:
    """
    Style 1 — pass the URL string directly.

    FastMCP calls infer_transport(url) internally and creates a
    StreamableHttpTransport automatically.  This is the simplest form.
    """
    banner("Style 1: Client(url_string)")
    print(f"  Client({SERVER_URL!r})\n")

    async with Client(SERVER_URL) as client:
        init = client.initialize_result          # set automatically by __aenter__
        print(f"  transport type : {type(client.transport).__name__}")
        print(f"  server name    : {init.serverInfo.name}  v{init.serverInfo.version}")
        print(f"  protocol       : {init.protocolVersion}")
        print(f"  instructions   : {init.instructions}")

        result = await client.call_tool("square", {"x": 3})
        print(f"  square(3)      = {text_of(result)}")


async def run_with_explicit_transport() -> None:
    """
    Style 2 — pass an explicit StreamableHttpTransport instance.

    Useful when you need to customise headers, auth, or TLS settings.
    """
    banner("Style 2: Client(StreamableHttpTransport(url))")

    transport = StreamableHttpTransport(
        url=SERVER_URL,
        # headers={"X-Api-Key": "secret"},   # optional: custom request headers
        # auth="bearer-token-string",         # optional: bearer token auth
    )
    print(f"  Client(StreamableHttpTransport({SERVER_URL!r}))\n")

    async with Client(transport) as client:
        init = client.initialize_result
        print(f"  transport type : {type(client.transport).__name__}")
        print(f"  server name    : {init.serverInfo.name}  v{init.serverInfo.version}")

        result = await client.call_tool("say_hello", {"name": "explicit transport"})
        print(f"  say_hello      = {text_of(result)}")


# ── full demo using Style 1 ───────────────────────────────────────────────────

async def full_demo() -> None:
    """Run all demos inside a single connected session."""
    banner("Full Demo — FastMCP HTTP Client")
    print(f"  Connecting to {SERVER_URL}\n")

    # The Client is an async context manager.
    # Entering it sends  initialize + notifications/initialized  automatically.
    # Exiting closes the session and releases the HTTP connection pool.
    async with Client(SERVER_URL) as client:
        await demo_list_tools(client)
        await demo_say_hello(client)
        await demo_square(client)
        await demo_raise_on_error(client)
        await demo_no_raise(client)
        await demo_structured_content(client)

    print(f"\n{'═' * 56}")
    print("  Session closed.")


# ── entry point ───────────────────────────────────────────────────────────────

async def main() -> None:
    await run_with_url_string()
    await run_with_explicit_transport()
    await full_demo()


if __name__ == "__main__":
    asyncio.run(main())
