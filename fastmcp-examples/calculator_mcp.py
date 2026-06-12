"""Simple calculator MCP server."""
import math
from fastmcp import FastMCP

mcp = FastMCP(
    "calculator",
    instructions=(
        "MCP server for basic arithmetic. "
        "Tools: add, subtract, multiply, divide, modulo, power, sqrt."
    ),
)


@mcp.tool()
def add(a: float, b: float) -> str:
    """Add two numbers and return the result."""
    return str(a + b)


@mcp.tool()
def subtract(a: float, b: float) -> str:
    """Subtract b from a and return the result."""
    return str(a - b)


@mcp.tool()
def multiply(a: float, b: float) -> str:
    """Multiply two numbers and return the result."""
    return str(a * b)


@mcp.tool()
def divide(a: float, b: float) -> str:
    """Divide a by b and return the result. Raises an error on division by zero."""
    if b == 0:
        raise ValueError("division by zero")
    return str(a / b)


@mcp.tool()
def modulo(a: float, b: float) -> str:
    """Compute a modulo b (remainder of a / b)."""
    if b == 0:
        raise ValueError("modulo by zero")
    return str(a % b)


@mcp.tool()
def power(base: float, exponent: float) -> str:
    """Raise base to the power of exponent."""
    return str(base ** exponent)


@mcp.tool()
def sqrt(x: float) -> str:
    """Compute the square root of x. Raises an error for negative inputs."""
    if x < 0:
        raise ValueError("cannot take square root of a negative number")
    return str(math.sqrt(x))


if __name__ == "__main__":
    mcp.run()
