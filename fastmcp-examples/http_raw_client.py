"""Raw HTTP MCP client for http_mcp_server.py.

NO fastmcp / mcp SDK used — only httpx (HTTP) + json + sys (stdlib).

Shows every HTTP request and response so you can see exactly how MCP
works over the Streamable HTTP transport (spec 2025-03-26).

How MCP over HTTP differs from stdio
──────────────────────────────────────────────────────────────────────
  stdio:  messages are newline-delimited JSON lines on a subprocess pipe
  HTTP:   messages are POST requests to a single endpoint  POST /mcp

Both transports use the same JSON-RPC 2.0 message structure inside.

HTTP wire protocol summary
──────────────────────────────────────────────────────────────────────
Every request from client to server:
    POST /mcp  HTTP/1.1
    Content-Type: application/json
    Accept: application/json, text/event-stream    ← REQUIRED by spec
    Mcp-Session-Id: <token>                        ← required after init

Responses from server:
    ┌──────────────────────────────────────────────────────────────┐
    │ Message type     │ HTTP status │ Content-Type               │
    ├──────────────────┼─────────────┼────────────────────────────┤
    │ initialize resp  │ 200 OK      │ text/event-stream          │
    │ notification ack │ 202 Accept  │ (empty body)               │
    │ tools/list resp  │ 200 OK      │ text/event-stream          │
    │ tools/call resp  │ 200 OK      │ text/event-stream          │
    └──────────────────┴─────────────┴────────────────────────────┘

SSE response body format (text/event-stream):
    event: message\n
    data: {"jsonrpc":"2.0","id":N,"result":{...}}\n
    \n                      ← blank line terminates the event

Session ID
──────────────────────────────────────────────────────────────────────
The server returns  Mcp-Session-Id  in the  initialize  response header.
The client MUST echo it back in every subsequent request header.
This lets the server correlate all requests to the same logical session
(important when many clients share one HTTP server).

Usage:
    # Terminal 1 — start the server
    python http_mcp_server.py

    # Terminal 2 — run this client
    python http_raw_client.py
"""

import json
import sys
import httpx

SERVER_ENDPOINT = "http://127.0.0.1:8000/mcp"

# ── ID counter ────────────────────────────────────────────────────────────────

_next_id = 0

def alloc_id() -> int:
    global _next_id
    _next_id += 1
    return _next_id


# ── SSE parser ────────────────────────────────────────────────────────────────

def parse_sse_body(text: str) -> dict | None:
    """Extract the JSON payload from a single SSE event.

    SSE body looks like:
        event: message\\n
        data: {"jsonrpc":"2.0","id":1,"result":{...}}\\n
        \\n

    We find the line starting with  'data: '  and JSON-parse it.
    """
    for line in text.splitlines():
        if line.startswith("data: "):
            return json.loads(line[6:])
    return None


def parse_response(resp: httpx.Response) -> dict | None:
    """Return the JSON-RPC object from either a JSON or SSE response."""
    ct = resp.headers.get("content-type", "")
    if not resp.text:
        return None
    if "text/event-stream" in ct:
        return parse_sse_body(resp.text)
    return json.loads(resp.text)   # plain application/json fallback


# ── pretty-print helpers ──────────────────────────────────────────────────────

SEP = "─" * 62

def _print_request(method: str, url: str, headers: dict, body: dict | None) -> None:
    print(f"\n{'─'*62}")
    print(f"  REQUEST")
    print(f"{'─'*62}")
    print(f"  {method} {url}  HTTP/1.1")
    for k, v in headers.items():
        print(f"  {k}: {v}")
    if body is not None:
        print(f"  Body: {json.dumps(body, separators=(',', ':'))}")


def _print_response(resp: httpx.Response) -> None:
    print(f"\n  RESPONSE")
    print(f"{'─'*62}")
    print(f"  HTTP/1.1 {resp.status_code} {resp.reason_phrase}")
    important = {"content-type", "mcp-session-id", "content-length"}
    for k, v in resp.headers.items():
        if k.lower() in important:
            print(f"  {k}: {v}")
    if resp.text:
        print(f"  Body:")
        # Indent each line of the body for readability
        for line in resp.text.strip().splitlines():
            print(f"    {line}")


# ── MCP over HTTP steps ───────────────────────────────────────────────────────

def http_post(
    client: httpx.Client,
    msg: dict,
    session_id: str | None = None,
) -> httpx.Response:
    """Send one JSON-RPC message via HTTP POST.

    All MCP requests MUST include:
        Accept: application/json, text/event-stream
    Without this the server returns 406 Not Acceptable.

    If a session_id is known it is attached via the Mcp-Session-Id header.
    """
    headers = {
        "Content-Type": "application/json",
        # Server enforces this — returns 406 if absent.
        # It signals the client can handle either a direct JSON reply
        # or a streamed SSE response.
        "Accept": "application/json, text/event-stream",
    }
    if session_id:
        headers["Mcp-Session-Id"] = session_id

    _print_request("POST", SERVER_ENDPOINT, headers, msg)

    resp = client.post(
        SERVER_ENDPOINT,
        content=json.dumps(msg, separators=(",", ":")).encode(),
        headers=headers,
    )

    _print_response(resp)
    return resp


# ── protocol steps ────────────────────────────────────────────────────────────

def step_initialize(client: httpx.Client) -> tuple[dict, str]:
    """
    STEP 1 — initialize  (request → response)
    ──────────────────────────────────────────
    Identical JSON-RPC structure as stdio, but sent as HTTP POST.

    New in HTTP transport:
    • Server responds with  Mcp-Session-Id  header — a UUID that ties
      all future requests to this logical session.
    • Response body is SSE, not plain JSON.
    """
    print(f"\n{'═'*62}")
    print("  STEP 1 — initialize")
    print(f"{'═'*62}")

    resp = http_post(client, {
        "jsonrpc": "2.0",
        "id": alloc_id(),
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "raw-http-client", "version": "0.1.0"},
        },
    })

    if resp.status_code != 200:
        sys.exit(f"initialize failed: HTTP {resp.status_code}\n{resp.text}")

    result = parse_response(resp)
    session_id = resp.headers.get("mcp-session-id", "")

    print(f"\n  ✓  session_id  : {session_id}")
    print(f"  ✓  server      : {result['result']['serverInfo']}")
    print(f"  ✓  protocol    : {result['result']['protocolVersion']}")

    return result["result"], session_id


def step_initialized(client: httpx.Client, session_id: str) -> None:
    """
    STEP 2 — notifications/initialized  (notification, no reply expected)
    ──────────────────────────────────────────────────────────────────────
    A *notification* has no "id" field → the server MUST NOT send a JSON-RPC
    response body.

    Over HTTP: server returns  202 Accepted  with an empty body.
    Over stdio: server simply stays silent.

    Purpose: tells the server the client received the initialize result and
    is now ready for tool calls.
    """
    print(f"\n{'═'*62}")
    print("  STEP 2 — notifications/initialized  (notification, no id)")
    print(f"{'═'*62}")

    resp = http_post(client, {
        "jsonrpc": "2.0",
        # NOTE: no "id" key — this is a notification, not a request
        "method": "notifications/initialized",
        "params": {},
    }, session_id)

    if resp.status_code == 202:
        print("  ✓  202 Accepted — handshake complete, server is ready")
    else:
        sys.exit(f"notifications/initialized failed: HTTP {resp.status_code}")


def step_list_tools(client: httpx.Client, session_id: str) -> list[dict]:
    """
    STEP 3 — tools/list  (optional discovery)
    ─────────────────────────────────────────
    Returns the JSON Schema for every tool:
    name, description, inputSchema (parameter types + required list).
    """
    print(f"\n{'═'*62}")
    print("  STEP 3 — tools/list")
    print(f"{'═'*62}")

    resp = http_post(client, {
        "jsonrpc": "2.0",
        "id": alloc_id(),
        "method": "tools/list",
        "params": {},
    }, session_id)

    result = parse_response(resp)
    tools = result["result"]["tools"]
    print(f"\n  ✓  {len(tools)} tool(s) available:")
    for t in tools:
        req = t["inputSchema"].get("required", [])
        print(f"       {t['name']}({', '.join(req)})  — {t['description']}")
    return tools


def step_call_tool(
    client: httpx.Client,
    session_id: str,
    tool_name: str,
    arguments: dict,
) -> str:
    """
    STEP 4 — tools/call
    ────────────────────
    Request:
        { "method": "tools/call",
          "params": { "name": "<tool>", "arguments": { ... } } }

    Response (inside SSE data field):
        { "result": {
            "isError": false,
            "content": [ {"type":"text","text":"<value>"} ],
            "structuredContent": { "result": "<value>" }
          } }

    Two error levels to distinguish:
    • HTTP non-200       — transport-level problem (wrong path, bad session, etc.)
    • result.isError=true — the tool function raised a Python exception
    """
    print(f"\n{'═'*62}")
    print(f"  TOOL CALL — {tool_name}({arguments})")
    print(f"{'═'*62}")

    resp = http_post(client, {
        "jsonrpc": "2.0",
        "id": alloc_id(),
        "method": "tools/call",
        "params": {"name": tool_name, "arguments": arguments},
    }, session_id)

    if resp.status_code != 200:
        raise RuntimeError(f"HTTP {resp.status_code}: {resp.text}")

    result = parse_response(resp)

    if result["result"].get("isError"):
        texts = [
            c["text"]
            for c in result["result"].get("content", [])
            if c.get("type") == "text"
        ]
        raise RuntimeError(f"tool error: {'; '.join(texts)}")

    texts = [
        c["text"]
        for c in result["result"].get("content", [])
        if c.get("type") == "text"
    ]
    value = "\n".join(texts)
    print(f"\n  ✓  result: {value}")
    return value


# ── main ──────────────────────────────────────────────────────────────────────

def main() -> None:
    print(f"MCP over HTTP — connecting to {SERVER_ENDPOINT}")

    with httpx.Client(timeout=10.0) as client:
        # ── handshake ─────────────────────────────────────────────────────────
        _, session_id = step_initialize(client)
        step_initialized(client, session_id)

        # ── discover tools ────────────────────────────────────────────────────
        step_list_tools(client, session_id)

        # ── say_hello ─────────────────────────────────────────────────────────
        step_call_tool(client, session_id, "say_hello", {"name": "World"})
        step_call_tool(client, session_id, "say_hello", {"name": "MCP over HTTP"})

        # ── square ────────────────────────────────────────────────────────────
        step_call_tool(client, session_id, "square", {"x": 9.0})
        step_call_tool(client, session_id, "square", {"x": 12.5})

        # ── error path: missing required argument ─────────────────────────────
        print(f"\n{'═'*62}")
        print("  ERROR PATH — say_hello() with missing 'name' argument")
        print(f"{'═'*62}")
        try:
            step_call_tool(client, session_id, "say_hello", {})
        except RuntimeError as e:
            print(f"  ✓  caught: {e}")

        # ── error path: unknown tool ──────────────────────────────────────────
        print(f"\n{'═'*62}")
        print("  ERROR PATH — unknown_tool()")
        print(f"{'═'*62}")
        try:
            step_call_tool(client, session_id, "unknown_tool", {})
        except RuntimeError as e:
            print(f"  ✓  caught: {e}")

    print(f"\n{'═'*62}")
    print("  Done — session closed (HTTP connection pool released)")


if __name__ == "__main__":
    main()
