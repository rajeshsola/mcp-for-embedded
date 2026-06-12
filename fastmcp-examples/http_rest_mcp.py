"""HTTP REST MCP server — GET and write (POST/PUT/PATCH/DELETE) operations.

Requires: httpx
    pip install httpx
"""
import json
from typing import Optional
import httpx
from fastmcp import FastMCP

mcp = FastMCP(
    "http-rest",
    instructions=(
        "MCP server for HTTP REST operations. "
        "Tools: http_get for read operations; "
        "http_post, http_put, http_patch for write operations; "
        "http_delete for delete operations."
    ),
)

_DEFAULT_TIMEOUT = 10.0


def _format_response(resp: httpx.Response) -> str:
    body = resp.text
    try:
        parsed = json.loads(body)
        body = json.dumps(parsed, indent=2)
    except (json.JSONDecodeError, ValueError):
        pass
    return (
        f"HTTP {resp.status_code} {resp.reason_phrase}\n"
        f"Content-Type: {resp.headers.get('content-type', '')}\n\n"
        f"{body}"
    )


@mcp.tool()
async def http_get(
    url: str,
    headers: Optional[dict] = None,
    params: Optional[dict] = None,
    timeout_s: float = _DEFAULT_TIMEOUT,
) -> str:
    """Perform an HTTP GET request and return the response body with status code.

    Args:
        url: Full URL to request (e.g. https://api.example.com/items).
        headers: Optional HTTP request headers as a JSON object.
        params: Optional URL query parameters as a JSON object.
        timeout_s: Request timeout in seconds (default: 10).
    """
    async with httpx.AsyncClient() as client:
        resp = await client.get(
            url,
            headers=headers or {},
            params=params or {},
            timeout=timeout_s,
            follow_redirects=True,
        )
    return _format_response(resp)


@mcp.tool()
async def http_post(
    url: str,
    body: str,
    content_type: str = "application/json",
    headers: Optional[dict] = None,
    timeout_s: float = _DEFAULT_TIMEOUT,
) -> str:
    """Perform an HTTP POST request with a body and return the response.

    Args:
        url: Full URL to post to.
        body: Request body as a string (use JSON string for application/json).
        content_type: Content-Type header value (default: application/json).
        headers: Optional additional HTTP request headers.
        timeout_s: Request timeout in seconds (default: 10).
    """
    merged_headers = {"Content-Type": content_type, **(headers or {})}
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            url,
            content=body.encode(),
            headers=merged_headers,
            timeout=timeout_s,
            follow_redirects=True,
        )
    return _format_response(resp)


@mcp.tool()
async def http_put(
    url: str,
    body: str,
    content_type: str = "application/json",
    headers: Optional[dict] = None,
    timeout_s: float = _DEFAULT_TIMEOUT,
) -> str:
    """Perform an HTTP PUT request (full resource replacement) and return the response.

    Args:
        url: Full URL of the resource to replace.
        body: Request body as a string.
        content_type: Content-Type header value (default: application/json).
        headers: Optional additional HTTP request headers.
        timeout_s: Request timeout in seconds (default: 10).
    """
    merged_headers = {"Content-Type": content_type, **(headers or {})}
    async with httpx.AsyncClient() as client:
        resp = await client.put(
            url,
            content=body.encode(),
            headers=merged_headers,
            timeout=timeout_s,
            follow_redirects=True,
        )
    return _format_response(resp)


@mcp.tool()
async def http_patch(
    url: str,
    body: str,
    content_type: str = "application/json",
    headers: Optional[dict] = None,
    timeout_s: float = _DEFAULT_TIMEOUT,
) -> str:
    """Perform an HTTP PATCH request (partial resource update) and return the response.

    Args:
        url: Full URL of the resource to patch.
        body: Partial update body as a string.
        content_type: Content-Type header value (default: application/json).
        headers: Optional additional HTTP request headers.
        timeout_s: Request timeout in seconds (default: 10).
    """
    merged_headers = {"Content-Type": content_type, **(headers or {})}
    async with httpx.AsyncClient() as client:
        resp = await client.patch(
            url,
            content=body.encode(),
            headers=merged_headers,
            timeout=timeout_s,
            follow_redirects=True,
        )
    return _format_response(resp)


@mcp.tool()
async def http_delete(
    url: str,
    headers: Optional[dict] = None,
    timeout_s: float = _DEFAULT_TIMEOUT,
) -> str:
    """Perform an HTTP DELETE request and return the response status.

    Args:
        url: Full URL of the resource to delete.
        headers: Optional HTTP request headers.
        timeout_s: Request timeout in seconds (default: 10).
    """
    async with httpx.AsyncClient() as client:
        resp = await client.delete(
            url,
            headers=headers or {},
            timeout=timeout_s,
            follow_redirects=True,
        )
    return _format_response(resp)


if __name__ == "__main__":
    mcp.run()
