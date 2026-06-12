"""MCP client for http_rest_mcp.py — exercises GET, POST, PUT, PATCH, DELETE.

Uses the public JSONPlaceholder test API (https://jsonplaceholder.typicode.com).
No account or API key required.

Prerequisites:
    - httpx installed in the same environment as the server.
    - Network access to jsonplaceholder.typicode.com.
"""
import asyncio
import json
from mcp_client_base import McpClient, banner, show

BASE_URL = "https://jsonplaceholder.typicode.com"


async def main() -> None:
    async with McpClient("http_rest_mcp.py") as client:
        banner("HTTP REST MCP Client")

        tools = await client.list_tools()
        print(f"Registered tools: {[t.name for t in tools]}")

        # ── GET list of posts ─────────────────────────────────────────────────
        show(
            "http_get  /posts?_limit=3",
            await client.call(
                "http_get",
                url=f"{BASE_URL}/posts",
                params={"_limit": "3"},
            ),
        )

        # ── GET single resource ───────────────────────────────────────────────
        show(
            "http_get  /posts/1",
            await client.call(
                "http_get",
                url=f"{BASE_URL}/posts/1",
            ),
        )

        # ── GET with custom headers ───────────────────────────────────────────
        show(
            "http_get  /todos/1  with Accept header",
            await client.call(
                "http_get",
                url=f"{BASE_URL}/todos/1",
                headers={"Accept": "application/json"},
            ),
        )

        # ── POST — create a new resource ─────────────────────────────────────
        new_post = json.dumps({
            "title": "FastMCP HTTP REST client",
            "body": "Testing http_post via MCP tool",
            "userId": 1,
        })
        show(
            "http_post  /posts",
            await client.call(
                "http_post",
                url=f"{BASE_URL}/posts",
                body=new_post,
                content_type="application/json; charset=UTF-8",
            ),
        )

        # ── PUT — full replacement of post 1 ─────────────────────────────────
        updated_post = json.dumps({
            "id": 1,
            "title": "Updated via http_put",
            "body": "Full replacement body",
            "userId": 1,
        })
        show(
            "http_put  /posts/1",
            await client.call(
                "http_put",
                url=f"{BASE_URL}/posts/1",
                body=updated_post,
            ),
        )

        # ── PATCH — partial update ────────────────────────────────────────────
        patch_body = json.dumps({"title": "Patched title"})
        show(
            "http_patch  /posts/1",
            await client.call(
                "http_patch",
                url=f"{BASE_URL}/posts/1",
                body=patch_body,
            ),
        )

        # ── DELETE ────────────────────────────────────────────────────────────
        show(
            "http_delete  /posts/1",
            await client.call(
                "http_delete",
                url=f"{BASE_URL}/posts/1",
            ),
        )

        # ── Error path: bad host (connection error surfaces as tool error) ─────
        print("\n[http_get with invalid host — expected error]")
        try:
            await client.call(
                "http_get",
                url="http://this-host-does-not-exist.invalid/",
                timeout_s=3.0,
            )
        except RuntimeError as e:
            print(f"  got expected error: {e}")


if __name__ == "__main__":
    asyncio.run(main())
