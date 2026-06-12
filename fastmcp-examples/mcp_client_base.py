"""Shared async MCP client helper.

Wraps the mcp SDK's stdio transport into a simple context manager so each
client script can focus on tool calls without repeating the connection setup.

Usage:
    async with McpClient("calculator_mcp.py") as client:
        result = await client.call("add", a=3.0, b=4.0)
        tools  = await client.list_tools()
"""
import sys
from pathlib import Path
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

SERVERS_DIR = Path(__file__).parent


class McpClient:
    """Async context-manager that spawns an MCP server subprocess over stdio."""

    def __init__(self, server_file: str, *server_args: str):
        self._params = StdioServerParameters(
            command=sys.executable,
            args=[str(SERVERS_DIR / server_file), *server_args],
        )
        self._streams_ctx = None
        self._session_ctx = None
        self.session: ClientSession | None = None

    async def __aenter__(self) -> "McpClient":
        self._streams_ctx = stdio_client(self._params)
        read, write = await self._streams_ctx.__aenter__()
        self._session_ctx = ClientSession(read, write)
        self.session = await self._session_ctx.__aenter__()
        await self.session.initialize()
        return self

    async def __aexit__(self, *exc) -> None:
        if self._session_ctx:
            await self._session_ctx.__aexit__(*exc)
        if self._streams_ctx:
            await self._streams_ctx.__aexit__(*exc)

    async def call(self, tool_name: str, **kwargs) -> str:
        """Call a named tool and return its concatenated text content.

        Raises RuntimeError if the server returns an isError response.
        """
        result = await self.session.call_tool(tool_name, kwargs)
        texts = [c.text for c in result.content if hasattr(c, "text")]
        if result.isError:
            raise RuntimeError(f"tool error from '{tool_name}': {'; '.join(texts)}")
        return "\n".join(texts) if texts else "(no text content)"

    async def list_tools(self) -> list:
        """Return the list of Tool objects advertised by the server."""
        result = await self.session.list_tools()
        return result.tools


def banner(title: str) -> None:
    width = 60
    print(f"\n{'=' * width}")
    print(f"  {title}")
    print(f"{'=' * width}")


def show(label: str, value: str) -> None:
    print(f"\n[{label}]\n{value}")
