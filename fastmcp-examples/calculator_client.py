"""MCP client for calculator_mcp.py — exercises all seven arithmetic tools."""
import asyncio
from mcp_client_base import McpClient, banner, show


async def main() -> None:
    async with McpClient("calculator_mcp.py") as client:
        banner("Calculator MCP Client")

        tools = await client.list_tools()
        print(f"Registered tools: {[t.name for t in tools]}")

        show("add(10, 3)",          await client.call("add",      a=10.0, b=3.0))
        show("subtract(10, 3)",     await client.call("subtract", a=10.0, b=3.0))
        show("multiply(10, 3)",     await client.call("multiply", a=10.0, b=3.0))
        show("divide(10, 3)",       await client.call("divide",   a=10.0, b=3.0))
        show("modulo(10, 3)",       await client.call("modulo",   a=10.0, b=3.0))
        show("power(2, 10)",        await client.call("power",    base=2.0, exponent=10.0))
        show("sqrt(144)",           await client.call("sqrt",     x=144.0))

        # Edge-case: chained operations (client side)
        a = float(await client.call("multiply", a=3.0, b=4.0))   # 12
        b = float(await client.call("power",    base=2.0, exponent=3.0))  # 8
        result = await client.call("add", a=a, b=b)
        show("multiply(3,4) + power(2,3)", result)  # 20

        # Error path: divide by zero
        print("\n[divide(5, 0)  — expected error]")
        try:
            await client.call("divide", a=5.0, b=1.0)
        except RuntimeError as e:
            print(f"  got expected error: {e}")


if __name__ == "__main__":
    asyncio.run(main())
