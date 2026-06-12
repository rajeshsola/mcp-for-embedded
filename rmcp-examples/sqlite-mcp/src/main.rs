use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use rusqlite::{Connection, params_from_iter, types::ValueRef};

// ── parameter structs ────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct QueryParams {
    #[schemars(description = "Path to the SQLite database file; created if it does not exist")]
    db_path: String,
    #[schemars(description = "SELECT SQL statement to execute")]
    query: String,
    #[schemars(description = "Positional values bound to ? placeholders in order (all treated as text)")]
    params: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ExecuteParams {
    #[schemars(description = "Path to the SQLite database file")]
    db_path: String,
    #[schemars(
        description = "Single SQL statement to execute: INSERT, UPDATE, DELETE, CREATE TABLE, DROP TABLE, ALTER TABLE, etc."
    )]
    statement: String,
    #[schemars(description = "Positional values bound to ? placeholders in order")]
    params: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct BatchParams {
    #[schemars(description = "Path to the SQLite database file")]
    db_path: String,
    #[schemars(
        description = "One or more semicolon-separated SQL statements executed in a single transaction; no ? placeholders supported"
    )]
    sql: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct DbParams {
    #[schemars(description = "Path to the SQLite database file")]
    db_path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct TableParams {
    #[schemars(description = "Path to the SQLite database file")]
    db_path: String,
    #[schemars(description = "Name of the table to inspect")]
    table: String,
}

// ── server struct ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SqliteMcpServer {
    tool_router: ToolRouter<Self>,
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn open_conn(db_path: &str) -> Result<Connection, McpError> {
    Connection::open(db_path)
        .map_err(|e| McpError::internal_error(format!("cannot open '{}': {}", db_path, e), None))
}

fn value_to_string(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(b) => String::from_utf8_lossy(b).into_owned(),
        ValueRef::Blob(b) => format!("<blob {} bytes>", b.len()),
    }
}

fn truncate_cell(s: &str) -> String {
    const MAX: usize = 80;
    if s.len() <= MAX {
        return s.to_owned();
    }
    // Walk back to a valid UTF-8 boundary
    let end = (0..=MAX - 3).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
    format!("{}...", &s[..end])
}

fn format_table(columns: &[String], rows: &[Vec<String>]) -> String {
    if columns.is_empty() {
        return "(no columns returned)".to_string();
    }
    // Compute display widths from headers + truncated cell values
    let mut widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
    let display: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(i, v)| {
                    let cell = truncate_cell(v);
                    if let Some(w) = widths.get_mut(i) {
                        *w = (*w).max(cell.len());
                    }
                    cell
                })
                .collect()
        })
        .collect();

    let sep: String = widths
        .iter()
        .map(|&w| "-".repeat(w + 2))
        .collect::<Vec<_>>()
        .join("+");
    let border = format!("+{sep}+");

    let header: String = columns
        .iter()
        .zip(&widths)
        .map(|(c, &w)| format!(" {c:w$} "))
        .collect::<Vec<_>>()
        .join("|");

    let mut out = format!("{border}\n|{header}|\n{border}\n");
    for row in &display {
        let line: String = row
            .iter()
            .zip(&widths)
            .map(|(v, &w)| format!(" {v:w$} "))
            .collect::<Vec<_>>()
            .join("|");
        out.push_str(&format!("|{line}|\n"));
    }
    out.push_str(&format!("{border}\n{} row(s)", rows.len()));
    out
}

// ── tools ────────────────────────────────────────────────────────────────────

#[tool_router]
impl SqliteMcpServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Execute a SELECT query and return results as a formatted table. \
                       Use ? placeholders in the query and supply values via params for safety."
    )]
    async fn sql_query(
        &self,
        Parameters(p): Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let conn = open_conn(&p.db_path)?;
            let mut stmt = conn
                .prepare(&p.query)
                .map_err(|e| McpError::internal_error(format!("prepare error: {}", e), None))?;

            let columns: Vec<String> = stmt
                .column_names()
                .into_iter()
                .map(String::from)
                .collect();

            let sql_params: Vec<String> = p.params.unwrap_or_default();
            let rows: Vec<Vec<String>> = stmt
                .query_map(params_from_iter(sql_params.iter()), |row| {
                    let n = row.as_ref().column_count();
                    Ok((0..n)
                        .map(|i| {
                            row.get_ref(i)
                                .map(value_to_string)
                                .unwrap_or_else(|_| "ERR".to_string())
                        })
                        .collect())
                })
                .map_err(|e| McpError::internal_error(format!("query error: {}", e), None))?
                .filter_map(|r| r.ok())
                .collect();

            Ok(CallToolResult::success(vec![Content::text(
                format_table(&columns, &rows),
            )]))
        })
        .await
        .map_err(|e| McpError::internal_error(format!("join error: {}", e), None))?
    }

    #[tool(
        description = "Execute a single DML or DDL statement: INSERT, UPDATE, DELETE, \
                       CREATE TABLE, DROP TABLE, ALTER TABLE, CREATE INDEX, etc. \
                       Returns the number of rows affected and the last inserted row ID."
    )]
    async fn sql_execute(
        &self,
        Parameters(p): Parameters<ExecuteParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let conn = open_conn(&p.db_path)?;
            let sql_params: Vec<String> = p.params.unwrap_or_default();
            let rows_affected = conn
                .execute(&p.statement, params_from_iter(sql_params.iter()))
                .map_err(|e| McpError::internal_error(format!("execute error: {}", e), None))?;
            let last_rowid = conn.last_insert_rowid();
            Ok(CallToolResult::success(vec![Content::text(format!(
                "ok — {} row(s) affected, last_insert_rowid = {}",
                rows_affected, last_rowid
            ))]))
        })
        .await
        .map_err(|e| McpError::internal_error(format!("join error: {}", e), None))?
    }

    #[tool(
        description = "Execute multiple semicolon-separated SQL statements in one transaction. \
                       Useful for schema creation scripts. Does not support ? placeholders."
    )]
    async fn sql_batch(
        &self,
        Parameters(p): Parameters<BatchParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let conn = open_conn(&p.db_path)?;
            conn.execute_batch(&p.sql)
                .map_err(|e| McpError::internal_error(format!("batch error: {}", e), None))?;
            Ok(CallToolResult::success(vec![Content::text("ok — batch executed")]))
        })
        .await
        .map_err(|e| McpError::internal_error(format!("join error: {}", e), None))?
    }

    #[tool(description = "List all user-defined tables in a SQLite database.")]
    async fn list_tables(
        &self,
        Parameters(p): Parameters<DbParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let conn = open_conn(&p.db_path)?;
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                     WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                     ORDER BY name",
                )
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            let tables: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| McpError::internal_error(e.to_string(), None))?
                .filter_map(|r| r.ok())
                .collect();

            let text = if tables.is_empty() {
                "no tables found".to_string()
            } else {
                format!("{} table(s):\n{}", tables.len(), tables.join("\n"))
            };
            Ok(CallToolResult::success(vec![Content::text(text)]))
        })
        .await
        .map_err(|e| McpError::internal_error(format!("join error: {}", e), None))?
    }

    #[tool(
        description = "Show the full schema of a table: column definitions (name, type, \
                       constraints) and any indexes defined on it."
    )]
    async fn describe_table(
        &self,
        Parameters(p): Parameters<TableParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let conn = open_conn(&p.db_path)?;

            // CREATE statement
            let create_sql: Option<String> = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                    [&p.table],
                    |row| row.get(0),
                )
                .ok();

            if create_sql.is_none() {
                return Err(McpError::invalid_params(
                    format!("table '{}' not found in '{}'", p.table, p.db_path),
                    None,
                ));
            }

            // Column info via PRAGMA table_info
            let safe = p.table.replace('"', "\"\"");
            let pragma = format!("PRAGMA table_info(\"{safe}\")");
            let mut stmt = conn
                .prepare(&pragma)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            // PRAGMA table_info columns: cid | name | type | notnull | dflt_value | pk
            let col_rows: Vec<Vec<String>> = stmt
                .query_map([], |row| {
                    Ok((0..6_usize)
                        .map(|i| {
                            row.get_ref(i)
                                .map(value_to_string)
                                .unwrap_or_else(|_| String::new())
                        })
                        .collect())
                })
                .map_err(|e| McpError::internal_error(e.to_string(), None))?
                .filter_map(|r| r.ok())
                .collect();

            let headers = ["cid", "name", "type", "notnull", "dflt_value", "pk"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            // Indexes
            let idx_pragma = format!("PRAGMA index_list(\"{safe}\")");
            let mut idx_stmt = conn
                .prepare(&idx_pragma)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            // index_list columns: seq | name | unique | origin | partial
            let idx_rows: Vec<Vec<String>> = idx_stmt
                .query_map([], |row| {
                    Ok((0..5_usize)
                        .map(|i| {
                            row.get_ref(i)
                                .map(value_to_string)
                                .unwrap_or_else(|_| String::new())
                        })
                        .collect())
                })
                .map_err(|e| McpError::internal_error(e.to_string(), None))?
                .filter_map(|r| r.ok())
                .collect();

            let idx_headers = ["seq", "name", "unique", "origin", "partial"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            let mut out = format!(
                "-- CREATE statement\n{}\n\n-- Columns\n{}",
                create_sql.unwrap_or_default(),
                format_table(&headers, &col_rows),
            );

            if !idx_rows.is_empty() {
                out.push_str(&format!(
                    "\n\n-- Indexes\n{}",
                    format_table(&idx_headers, &idx_rows)
                ));
            }

            Ok(CallToolResult::success(vec![Content::text(out)]))
        })
        .await
        .map_err(|e| McpError::internal_error(format!("join error: {}", e), None))?
    }
}

// ── server metadata ──────────────────────────────────────────────────────────

#[tool_handler]
impl ServerHandler for SqliteMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "MCP server for SQLite databases. \
                 Tools: \
                 sql_query – run SELECT queries and get results as a table; \
                 sql_execute – run a single INSERT/UPDATE/DELETE/DDL statement; \
                 sql_batch – run multiple semicolon-separated statements (schema scripts); \
                 list_tables – list all user tables in a database; \
                 describe_table – show column definitions and indexes for a table. \
                 All tools take db_path as the first argument; the file is created if absent."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = SqliteMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
