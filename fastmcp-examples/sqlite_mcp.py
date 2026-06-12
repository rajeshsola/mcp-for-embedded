"""SQLite MCP server — query, execute, and inspect SQLite databases.

Uses the Python standard library sqlite3 module; no extra packages required.
"""
import sqlite3
from typing import Optional
from fastmcp import FastMCP

mcp = FastMCP(
    "sqlite",
    instructions=(
        "MCP server for SQLite databases. "
        "Tools: sql_query – run SELECT queries; "
        "sql_execute – run a single INSERT/UPDATE/DELETE/DDL statement; "
        "sql_batch – run multiple semicolon-separated statements (schema scripts); "
        "list_tables – list all user tables in a database; "
        "describe_table – show column definitions and indexes for a table. "
        "All tools take db_path as the first argument; the file is created if absent."
    ),
)

_CELL_MAX = 80


def _open(db_path: str) -> sqlite3.Connection:
    try:
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        return conn
    except Exception as e:
        raise RuntimeError(f"cannot open '{db_path}': {e}") from e


def _cell(v: object) -> str:
    if v is None:
        return "NULL"
    s = str(v)
    if len(s) <= _CELL_MAX:
        return s
    return s[: _CELL_MAX - 3] + "..."


def _format_table(columns: list[str], rows: list[list[str]]) -> str:
    if not columns:
        return "(no columns returned)"

    widths = [len(c) for c in columns]
    display = []
    for row in rows:
        cells = [_cell(v) for v in row]
        for i, c in enumerate(cells):
            if i < len(widths):
                widths[i] = max(widths[i], len(c))
        display.append(cells)

    sep = "+".join("-" * (w + 2) for w in widths)
    border = f"+{sep}+"
    header = "|".join(f" {c:<{w}} " for c, w in zip(columns, widths))
    lines = [border, f"|{header}|", border]
    for row in display:
        line = "|".join(f" {v:<{w}} " for v, w in zip(row, widths))
        lines.append(f"|{line}|")
    lines.append(border)
    lines.append(f"{len(rows)} row(s)")
    return "\n".join(lines)


@mcp.tool()
def sql_query(
    db_path: str,
    query: str,
    params: Optional[list[str]] = None,
) -> str:
    """Execute a SELECT query and return results as a formatted table.

    Use ? placeholders in the query and supply values via params for safety.

    Args:
        db_path: Path to the SQLite database file; created if it does not exist.
        query: SELECT SQL statement to execute.
        params: Positional values bound to ? placeholders in order.
    """
    with _open(db_path) as conn:
        try:
            cursor = conn.execute(query, params or [])
        except sqlite3.Error as e:
            raise RuntimeError(f"query error: {e}") from e
        columns = [d[0] for d in cursor.description or []]
        rows = [list(row) for row in cursor.fetchall()]
    return _format_table(columns, rows)


@mcp.tool()
def sql_execute(
    db_path: str,
    statement: str,
    params: Optional[list[str]] = None,
) -> str:
    """Execute a single DML or DDL statement: INSERT, UPDATE, DELETE,
    CREATE TABLE, DROP TABLE, ALTER TABLE, CREATE INDEX, etc.

    Returns the number of rows affected and the last inserted row ID.

    Args:
        db_path: Path to the SQLite database file.
        statement: Single SQL statement to execute.
        params: Positional values bound to ? placeholders in order.
    """
    with _open(db_path) as conn:
        try:
            cursor = conn.execute(statement, params or [])
            conn.commit()
        except sqlite3.Error as e:
            raise RuntimeError(f"execute error: {e}") from e
        return (
            f"ok — {cursor.rowcount} row(s) affected, "
            f"last_insert_rowid = {cursor.lastrowid}"
        )


@mcp.tool()
def sql_batch(db_path: str, sql: str) -> str:
    """Execute multiple semicolon-separated SQL statements in one transaction.

    Useful for schema creation scripts. Does not support ? placeholders.

    Args:
        db_path: Path to the SQLite database file.
        sql: One or more semicolon-separated SQL statements.
    """
    with _open(db_path) as conn:
        try:
            conn.executescript(sql)
            conn.commit()
        except sqlite3.Error as e:
            raise RuntimeError(f"batch error: {e}") from e
    return "ok — batch executed"


@mcp.tool()
def list_tables(db_path: str) -> str:
    """List all user-defined tables in a SQLite database.

    Args:
        db_path: Path to the SQLite database file.
    """
    with _open(db_path) as conn:
        cursor = conn.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type = 'table' AND name NOT LIKE 'sqlite_%' "
            "ORDER BY name"
        )
        tables = [row[0] for row in cursor.fetchall()]

    if not tables:
        return "no tables found"
    return f"{len(tables)} table(s):\n" + "\n".join(tables)


@mcp.tool()
def describe_table(db_path: str, table: str) -> str:
    """Show the full schema of a table: column definitions (name, type,
    constraints) and any indexes defined on it.

    Args:
        db_path: Path to the SQLite database file.
        table: Name of the table to inspect.
    """
    with _open(db_path) as conn:
        row = conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?", [table]
        ).fetchone()
        if row is None:
            raise ValueError(f"table '{table}' not found in '{db_path}'")
        create_sql = row[0]

        # PRAGMA table_info: cid | name | type | notnull | dflt_value | pk
        safe = table.replace('"', '""')
        col_cursor = conn.execute(f'PRAGMA table_info("{safe}")')
        col_headers = ["cid", "name", "type", "notnull", "dflt_value", "pk"]
        col_rows = [list(r) for r in col_cursor.fetchall()]

        # PRAGMA index_list: seq | name | unique | origin | partial
        idx_cursor = conn.execute(f'PRAGMA index_list("{safe}")')
        idx_headers = ["seq", "name", "unique", "origin", "partial"]
        idx_rows = [list(r) for r in idx_cursor.fetchall()]

    out = (
        f"-- CREATE statement\n{create_sql}\n\n"
        f"-- Columns\n{_format_table(col_headers, col_rows)}"
    )
    if idx_rows:
        out += f"\n\n-- Indexes\n{_format_table(idx_headers, idx_rows)}"
    return out


if __name__ == "__main__":
    mcp.run()
