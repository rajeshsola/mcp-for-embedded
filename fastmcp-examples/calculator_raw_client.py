"""Raw stdio MCP client for calculator_mcp.py.

NO fastmcp / mcp SDK used — only stdlib: subprocess, json, sys.

Illustrates the exact JSON-RPC 2.0 messages that flow over stdin/stdout
between an MCP client and server.

Wire protocol overview
──────────────────────
Every message is a JSON object terminated by a single newline (\n).

  CLIENT  →  SERVER  (requests)
  ───────────────────────────────────────────────────────────────────
  { "jsonrpc": "2.0",
    "id": <int>,              ← present on requests that expect a reply
    "method": "<method>",
    "params": { ... } }

  CLIENT  →  SERVER  (notifications — no reply expected, no "id")
  ───────────────────────────────────────────────────────────────────
  { "jsonrpc": "2.0",
    "method": "<method>",
    "params": { ... } }

  SERVER  →  CLIENT  (responses)
  ───────────────────────────────────────────────────────────────────
  { "jsonrpc": "2.0",
    "id": <int>,              ← mirrors the request id
    "result": { ... } }       ← OR "error": {"code":..,"message":..}

MCP handshake sequence
──────────────────────
  1. client  →  initialize        (request)
  2. server  →  initialize result (response)
  3. client  →  notifications/initialized  (notification, no id)
     ── server is now ready ──

Tool call sequence
──────────────────
  4. client  →  tools/list        (optional: discover available tools)
  5. server  →  tools/list result
  6. client  →  tools/call        (invoke a tool)
  7. server  →  tools/call result
"""

import json
import subprocess
import sys
from pathlib import Path

SERVER_SCRIPT = str(Path(__file__).parent / "calculator_mcp.py")

# ── global request-ID counter ─────────────────────────────────────────────────

_next_id = 0


def alloc_id() -> int:
    global _next_id
    _next_id += 1
    return _next_id


# ── raw JSON-RPC I/O ──────────────────────────────────────────────────────────

def send(proc: subprocess.Popen, msg: dict) -> None:
    """Serialise *msg* as compact JSON + newline and write to the server's stdin."""
    line = json.dumps(msg, separators=(",", ":")) + "\n"
    # Pretty-print so we can read it in the console
    print(f"\n  CLIENT → SERVER\n  {line.strip()}")
    proc.stdin.write(line)
    proc.stdin.flush()


def recv(proc: subprocess.Popen) -> dict:
    """Read the next JSON-RPC *response* from the server's stdout.

    MCP servers may send *notifications* at any time — these have a "method"
    key but NO "id".  We skip them silently (they are not replies to our
    requests).
    """
    while True:
        raw = proc.stdout.readline()
        if not raw:
            raise EOFError("server closed stdout — process may have crashed")
        raw = raw.strip()
        if not raw:
            continue

        msg = json.loads(raw)

        # Notification: has "method" but no "id" — not a reply, skip it
        if "method" in msg and "id" not in msg:
            print(f"  SERVER → CLIENT  [notification]\n  {raw}")
            continue

        # Response to one of our requests
        print(f"\n  SERVER → CLIENT\n  {raw}")
        return msg


# ── MCP protocol steps ────────────────────────────────────────────────────────

def mcp_initialize(proc: subprocess.Popen) -> dict:
    """
    Step 1 & 2 — MCP handshake (initialize).

    Request:
      { "jsonrpc":"2.0", "id":1, "method":"initialize",
        "params": {
          "protocolVersion": "2024-11-05",
          "capabilities": {},
          "clientInfo": {"name":"...", "version":"..."} } }

    Response:
      { "jsonrpc":"2.0", "id":1,
        "result": {
          "protocolVersion": "2024-11-05",
          "capabilities": {"tools":{}},
          "serverInfo": {"name":"calculator","version":"..."} } }
    """
    print("\n" + "=" * 60)
    print("STEP 1 — initialize (client → server)")
    print("=" * 60)

    send(proc, {
        "jsonrpc": "2.0",
        "id": alloc_id(),
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "raw-stdio-client", "version": "0.1.0"},
        },
    })

    resp = recv(proc)
    if "error" in resp:
        raise RuntimeError(f"initialize failed: {resp['error']}")

    server_info = resp["result"].get("serverInfo", {})
    proto = resp["result"].get("protocolVersion", "?")
    print(f"\n  ✓ server: {server_info}  protocol: {proto}")
    return resp["result"]


def mcp_initialized(proc: subprocess.Popen) -> None:
    """
    Step 3 — notifications/initialized.

    After the client receives the initialize *response*, it MUST send this
    notification to signal it is ready.  No reply is expected (no "id").

    Notification:
      { "jsonrpc":"2.0", "method":"notifications/initialized", "params":{} }
    """
    print("\n" + "=" * 60)
    print("STEP 2 — notifications/initialized (client → server, no reply)")
    print("=" * 60)

    send(proc, {
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {},
    })
    print("  ✓ handshake complete — server is now ready for tool calls")


def mcp_list_tools(proc: subprocess.Popen) -> list[dict]:
    """
    Step 4 & 5 — tools/list (optional discovery step).

    Request:
      { "jsonrpc":"2.0", "id":2, "method":"tools/list", "params":{} }

    Response:
      { "jsonrpc":"2.0", "id":2,
        "result": {
          "tools": [
            { "name": "add",
              "description": "Add two numbers and return the result.",
              "inputSchema": {
                "type": "object",
                "properties": {
                  "a": {"type":"number"},
                  "b": {"type":"number"}
                },
                "required": ["a","b"] } },
            ... ] } }
    """
    print("\n" + "=" * 60)
    print("STEP 3 — tools/list (discover available tools)")
    print("=" * 60)

    send(proc, {
        "jsonrpc": "2.0",
        "id": alloc_id(),
        "method": "tools/list",
        "params": {},
    })

    resp = recv(proc)
    if "error" in resp:
        raise RuntimeError(f"tools/list failed: {resp['error']}")

    tools = resp["result"].get("tools", [])
    print(f"\n  ✓ {len(tools)} tool(s) available: {[t['name'] for t in tools]}")
    return tools


def mcp_call_tool(proc: subprocess.Popen, name: str, arguments: dict) -> str:
    """
    Step 6 & 7 — tools/call.

    Request:
      { "jsonrpc":"2.0", "id":N, "method":"tools/call",
        "params": {"name":"add", "arguments":{"a":10,"b":3}} }

    Successful response:
      { "jsonrpc":"2.0", "id":N,
        "result": {
          "isError": false,
          "content": [ {"type":"text","text":"13.0"} ] } }

    Error response (tool raised an exception):
      { "jsonrpc":"2.0", "id":N,
        "result": {
          "isError": true,
          "content": [ {"type":"text","text":"division by zero"} ] } }

    Note: isError=true is still a *successful* JSON-RPC response.
    A true JSON-RPC "error" key means the server itself failed (e.g. unknown
    method), not that the tool returned an error.
    """
    req_id = alloc_id()
    print("\n" + "-" * 60)
    print(f"TOOL CALL — {name}({arguments})")
    print("-" * 60)

    send(proc, {
        "jsonrpc": "2.0",
        "id": req_id,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })

    resp = recv(proc)

    # JSON-RPC level error — protocol problem (unknown method, bad params, etc.)
    if "error" in resp:
        raise RuntimeError(
            f"JSON-RPC error [{resp['error'].get('code')}]: "
            f"{resp['error'].get('message')}"
        )

    result = resp.get("result", {})

    # MCP tool-level error — the tool itself raised an exception
    if result.get("isError"):
        texts = [
            c["text"]
            for c in result.get("content", [])
            if c.get("type") == "text"
        ]
        raise RuntimeError(f"tool error: {'; '.join(texts)}")

    # Success — collect text content items
    texts = [
        c["text"]
        for c in result.get("content", [])
        if c.get("type") == "text"
    ]
    return "\n".join(texts) if texts else "(no text content)"


# ── main ──────────────────────────────────────────────────────────────────────

def main() -> None:
    print(f"Spawning MCP server: {SERVER_SCRIPT}")

    # stderr=DEVNULL suppresses the FastMCP startup banner / log lines.
    # Change to stderr=sys.stderr to see server-side diagnostics.
    proc = subprocess.Popen(
        [sys.executable, SERVER_SCRIPT],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        bufsize=1,          # line-buffered — each flush() makes data readable
    )

    try:
        # ── MCP handshake ─────────────────────────────────────────────────────
        mcp_initialize(proc)
        mcp_initialized(proc)

        # ── discover tools ────────────────────────────────────────────────────
        mcp_list_tools(proc)

        # ── call add(10, 3) ───────────────────────────────────────────────────
        result = mcp_call_tool(proc, "add", {"a": 10.0, "b": 3.0})
        print(f"\n  ✓ add(10, 3) = {result}")

        # ── call multiply(6, 7) ───────────────────────────────────────────────
        result = mcp_call_tool(proc, "multiply", {"a": 6.0, "b": 7.0})
        print(f"\n  ✓ multiply(6, 7) = {result}")

        # ── error path: divide by zero ────────────────────────────────────────
        print("\n" + "-" * 60)
        print("TOOL CALL — divide(5, 0)  [expected tool error]")
        print("-" * 60)
        try:
            mcp_call_tool(proc, "divide", {"a": 5.0, "b": 0.0})
        except RuntimeError as e:
            print(f"\n  ✓ caught expected error: {e}")

        # ── error path: unknown tool ──────────────────────────────────────────
        print("\n" + "-" * 60)
        print("TOOL CALL — unknown_tool()  [expected JSON-RPC error]")
        print("-" * 60)
        try:
            mcp_call_tool(proc, "unknown_tool", {})
        except RuntimeError as e:
            print(f"\n  ✓ caught expected error: {e}")

    finally:
        proc.stdin.close()
        proc.wait()
        print("\n" + "=" * 60)
        print("Server process terminated.")


if __name__ == "__main__":
    main()
