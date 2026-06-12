"""MCP client for grpc_mcp.py — list services, describe, and call unary methods.

Targets the grpc-examples server from this repo (grpc-examples/src/server.rs).

Prerequisites:
    - grpcio, grpcio-reflection, protobuf installed.
    - The gRPC examples server must be running with reflection enabled:
        cargo run --bin server           (inside grpc-examples/)
      or any other gRPC server at the endpoint below.

Usage:
    python grpc_client.py [endpoint]    (default: http://localhost:50051)
"""
import asyncio
import json
import sys
from mcp_client_base import McpClient, banner, show

# The grpc-examples server listens on [::1]:50051; map to localhost for clarity.
ENDPOINT = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:50051"

# Service defined in grpc-examples/proto/examples.proto
SERVICE = "examples.Examples"


async def main() -> None:
    async with McpClient("grpc_mcp.py") as client:
        banner(f"gRPC MCP Client  [{ENDPOINT}]")

        tools = await client.list_tools()
        print(f"Registered tools: {[t.name for t in tools]}")

        # ── 1. Discover available services ────────────────────────────────────
        show(
            "grpc_list_services",
            await client.call("grpc_list_services", endpoint=ENDPOINT),
        )

        # ── 2. Describe the Examples service ─────────────────────────────────
        show(
            f"grpc_describe  {SERVICE}",
            await client.call(
                "grpc_describe",
                endpoint=ENDPOINT,
                symbol=SERVICE,
            ),
        )

        # ── 3. Describe a single method ───────────────────────────────────────
        show(
            f"grpc_describe  {SERVICE}.SayHello",
            await client.call(
                "grpc_describe",
                endpoint=ENDPOINT,
                symbol=f"{SERVICE}.SayHello",
            ),
        )

        # ── 4. Call SayHello ─────────────────────────────────────────────────
        hello_req = json.dumps({"name": "FastMCP"})
        show(
            f"grpc_call  {SERVICE}/SayHello  {{name: FastMCP}}",
            await client.call(
                "grpc_call",
                endpoint=ENDPOINT,
                service=SERVICE,
                method="SayHello",
                request_json=hello_req,
            ),
        )

        # ── 5. Call Add ───────────────────────────────────────────────────────
        add_req = json.dumps({"a": 3.0, "b": 4.0})
        show(
            f"grpc_call  {SERVICE}/Add  {{a: 3, b: 4}}",
            await client.call(
                "grpc_call",
                endpoint=ENDPOINT,
                service=SERVICE,
                method="Add",
                request_json=add_req,
            ),
        )

        # ── 6. Call Multiply ──────────────────────────────────────────────────
        mul_req = json.dumps({"a": 6.0, "b": 7.0})
        show(
            f"grpc_call  {SERVICE}/Multiply  {{a: 6, b: 7}}",
            await client.call(
                "grpc_call",
                endpoint=ENDPOINT,
                service=SERVICE,
                method="Multiply",
                request_json=mul_req,
            ),
        )

        # ── 7. Call with metadata (e.g. auth token) ───────────────────────────
        show(
            f"grpc_call  {SERVICE}/SayHello  with metadata",
            await client.call(
                "grpc_call",
                endpoint=ENDPOINT,
                service=SERVICE,
                method="SayHello",
                request_json=json.dumps({"name": "MCP with metadata"}),
                metadata={"x-request-id": "mcp-demo-001"},
            ),
        )

        # ── Error path: unknown service ───────────────────────────────────────
        print("\n[grpc_describe  unknown.Service — expected error]")
        try:
            await client.call(
                "grpc_describe",
                endpoint=ENDPOINT,
                symbol="unknown.Service",
            )
        except RuntimeError as e:
            print(f"  got expected error: {e}")


if __name__ == "__main__":
    asyncio.run(main())
