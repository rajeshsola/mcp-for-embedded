"""MCP client for sqlite_mcp.py — create tables, insert data, query, describe.

Uses a temporary database file that is cleaned up at the end of the run.

Prerequisites:
    - No extra packages; sqlite3 is in the Python standard library.

Usage:
    python sqlite_client.py [db_path]   (default: /tmp/mcp_demo.db)
"""
import asyncio
import sys
import os
from mcp_client_base import McpClient, banner, show

DB_PATH = sys.argv[1] if len(sys.argv) > 1 else "/tmp/mcp_demo.db"


async def main() -> None:
    # Remove any leftover demo db from a previous run
    if os.path.exists(DB_PATH):
        os.remove(DB_PATH)

    async with McpClient("sqlite_mcp.py") as client:
        banner(f"SQLite MCP Client  [{DB_PATH}]")

        tools = await client.list_tools()
        print(f"Registered tools: {[t.name for t in tools]}")

        # ── 1. Create schema with sql_batch ───────────────────────────────────
        schema = """
        CREATE TABLE IF NOT EXISTS vehicles (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            vin         TEXT    NOT NULL UNIQUE,
            make        TEXT    NOT NULL,
            model       TEXT    NOT NULL,
            year        INTEGER NOT NULL,
            mileage_km  REAL    DEFAULT 0.0
        );
        CREATE TABLE IF NOT EXISTS telemetry (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            vehicle_id  INTEGER NOT NULL REFERENCES vehicles(id),
            ts          TEXT    NOT NULL,
            speed_kmh   INTEGER,
            rpm         INTEGER,
            fuel_pct    REAL
        );
        CREATE INDEX IF NOT EXISTS idx_telemetry_vehicle
            ON telemetry(vehicle_id);
        """
        show("sql_batch  create schema", await client.call("sql_batch", db_path=DB_PATH, sql=schema))

        # ── 2. List tables ────────────────────────────────────────────────────
        show("list_tables", await client.call("list_tables", db_path=DB_PATH))

        # ── 3. Describe vehicles table ────────────────────────────────────────
        show("describe_table  vehicles",
             await client.call("describe_table", db_path=DB_PATH, table="vehicles"))

        # ── 4. Insert rows with sql_execute ───────────────────────────────────
        show(
            "sql_execute  INSERT vehicle 1",
            await client.call(
                "sql_execute",
                db_path=DB_PATH,
                statement="INSERT INTO vehicles (vin, make, model, year, mileage_km) VALUES (?, ?, ?, ?, ?)",
                params=["1HGCM82633A123456", "Honda", "Accord", "2020", "45231.5"],
            ),
        )
        show(
            "sql_execute  INSERT vehicle 2",
            await client.call(
                "sql_execute",
                db_path=DB_PATH,
                statement="INSERT INTO vehicles (vin, make, model, year, mileage_km) VALUES (?, ?, ?, ?, ?)",
                params=["2T1BURHE0JC123789", "Toyota", "Corolla", "2021", "22100.0"],
            ),
        )

        # ── 5. Insert telemetry rows ──────────────────────────────────────────
        telemetry_batch = """
        INSERT INTO telemetry (vehicle_id, ts, speed_kmh, rpm, fuel_pct)
        VALUES (1, '2024-01-15T08:00:00', 95, 2800, 72.5);
        INSERT INTO telemetry (vehicle_id, ts, speed_kmh, rpm, fuel_pct)
        VALUES (1, '2024-01-15T08:05:00', 110, 3200, 71.0);
        INSERT INTO telemetry (vehicle_id, ts, speed_kmh, rpm, fuel_pct)
        VALUES (2, '2024-01-15T09:00:00', 60, 1800, 88.0);
        """
        show("sql_batch  insert telemetry", await client.call("sql_batch", db_path=DB_PATH, sql=telemetry_batch))

        # ── 6. SELECT with JOIN ───────────────────────────────────────────────
        show(
            "sql_query  JOIN vehicles + telemetry",
            await client.call(
                "sql_query",
                db_path=DB_PATH,
                query="""
                    SELECT v.make, v.model, v.year,
                           t.ts, t.speed_kmh, t.rpm, t.fuel_pct
                    FROM telemetry t
                    JOIN vehicles v ON v.id = t.vehicle_id
                    ORDER BY t.ts
                """,
            ),
        )

        # ── 7. Parameterised SELECT ───────────────────────────────────────────
        show(
            "sql_query  WHERE make=? (Honda)",
            await client.call(
                "sql_query",
                db_path=DB_PATH,
                query="SELECT id, vin, make, model, mileage_km FROM vehicles WHERE make = ?",
                params=["Honda"],
            ),
        )

        # ── 8. Aggregate query ────────────────────────────────────────────────
        show(
            "sql_query  AVG speed per vehicle",
            await client.call(
                "sql_query",
                db_path=DB_PATH,
                query="""
                    SELECT v.make || ' ' || v.model AS vehicle,
                           ROUND(AVG(t.speed_kmh), 1) AS avg_speed_kmh,
                           ROUND(AVG(t.fuel_pct), 1)  AS avg_fuel_pct
                    FROM telemetry t
                    JOIN vehicles v ON v.id = t.vehicle_id
                    GROUP BY v.id
                """,
            ),
        )

        # ── 9. UPDATE mileage ─────────────────────────────────────────────────
        show(
            "sql_execute  UPDATE mileage",
            await client.call(
                "sql_execute",
                db_path=DB_PATH,
                statement="UPDATE vehicles SET mileage_km = mileage_km + ? WHERE id = ?",
                params=["150.0", "1"],
            ),
        )

        # ── 10. Verify update ─────────────────────────────────────────────────
        show(
            "sql_query  SELECT updated mileage",
            await client.call(
                "sql_query",
                db_path=DB_PATH,
                query="SELECT id, make, model, mileage_km FROM vehicles",
            ),
        )

        # ── 11. Error path: table not found ───────────────────────────────────
        print("\n[describe_table  nonexistent — expected error]")
        try:
            await client.call("describe_table", db_path=DB_PATH, table="nonexistent")
        except RuntimeError as e:
            print(f"  got expected error: {e}")

    # Clean up the temp database
    if DB_PATH == "/tmp/mcp_demo.db" and os.path.exists(DB_PATH):
        os.remove(DB_PATH)
        print(f"\nCleaned up {DB_PATH}")


if __name__ == "__main__":
    asyncio.run(main())
