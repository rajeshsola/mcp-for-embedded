use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

// ── parameter structs ────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct GetParams {
    #[schemars(description = "Full URL to request (e.g. https://api.example.com/users)")]
    url: String,
    #[schemars(description = "Additional HTTP headers as key-value pairs")]
    headers: Option<HashMap<String, String>>,
    #[schemars(description = "Request timeout in milliseconds (default: 30000)")]
    timeout_ms: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct PostParams {
    #[schemars(description = "Full URL to POST to")]
    url: String,
    #[schemars(description = "Request body as a string (JSON, form data, plain text, etc.)")]
    body: String,
    #[schemars(
        description = "Content-Type header value (default: application/json)"
    )]
    content_type: Option<String>,
    #[schemars(description = "Additional HTTP headers as key-value pairs")]
    headers: Option<HashMap<String, String>>,
    #[schemars(description = "Request timeout in milliseconds (default: 30000)")]
    timeout_ms: Option<u64>,
}

// ── server struct ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct HttpMcpServer {
    client: reqwest::Client,
    tool_router: ToolRouter<Self>,
}

// ── helpers ──────────────────────────────────────────────────────────────────

const MAX_BODY_BYTES: usize = 50_000;

fn build_header_map(extra: Option<HashMap<String, String>>) -> Result<HeaderMap, McpError> {
    let mut map = HeaderMap::new();
    for (k, v) in extra.unwrap_or_default() {
        let name = HeaderName::from_str(&k).map_err(|e| {
            McpError::invalid_params(format!("invalid header name '{}': {}", k, e), None)
        })?;
        let val = HeaderValue::from_str(&v).map_err(|e| {
            McpError::invalid_params(format!("invalid value for header '{}': {}", k, e), None)
        })?;
        map.insert(name, val);
    }
    Ok(map)
}

fn format_response(status: reqwest::StatusCode, content_type: &str, body: &str) -> String {
    let (body_str, note) = if body.len() > MAX_BODY_BYTES {
        (
            &body[..MAX_BODY_BYTES],
            format!("\n\n[response body truncated at {} bytes]", MAX_BODY_BYTES),
        )
    } else {
        (body, String::new())
    };
    format!(
        "status: {} {}\ncontent-type: {}\n\n{}{}",
        status.as_u16(),
        status.canonical_reason().unwrap_or(""),
        content_type,
        body_str,
        note,
    )
}

// ── tool implementations ─────────────────────────────────────────────────────

#[tool_router]
impl HttpMcpServer {
    fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Send an HTTP GET request to a URL and return the status code and response body. \
                       Optionally supply extra headers. Response body is capped at 50 KB."
    )]
    async fn http_get(
        &self,
        Parameters(params): Parameters<GetParams>,
    ) -> Result<CallToolResult, McpError> {
        let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(30_000));
        let headers = build_header_map(params.headers)?;

        let resp = self
            .client
            .get(&params.url)
            .headers(headers)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| McpError::internal_error(format!("GET '{}' failed: {}", params.url, e), None))?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("(none)")
            .to_string();
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::internal_error(format!("failed to read response body: {}", e), None))?;

        Ok(CallToolResult::success(vec![Content::text(
            format_response(status, &content_type, &body),
        )]))
    }

    #[tool(
        description = "Send an HTTP POST request with a body to a URL and return the status code and response body. \
                       Defaults to Content-Type: application/json. Response body is capped at 50 KB."
    )]
    async fn http_post(
        &self,
        Parameters(params): Parameters<PostParams>,
    ) -> Result<CallToolResult, McpError> {
        let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(30_000));
        let content_type = params
            .content_type
            .unwrap_or_else(|| "application/json".to_string());

        let mut headers = build_header_map(params.headers)?;
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_str(&content_type).map_err(|e| {
                McpError::invalid_params(
                    format!("invalid content-type '{}': {}", content_type, e),
                    None,
                )
            })?,
        );

        let resp = self
            .client
            .post(&params.url)
            .headers(headers)
            .body(params.body)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| {
                McpError::internal_error(format!("POST '{}' failed: {}", params.url, e), None)
            })?;

        let status = resp.status();
        let resp_content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("(none)")
            .to_string();
        let body = resp
            .text()
            .await
            .map_err(|e| {
                McpError::internal_error(format!("failed to read response body: {}", e), None)
            })?;

        Ok(CallToolResult::success(vec![Content::text(
            format_response(status, &resp_content_type, &body),
        )]))
    }
}

// ── server metadata ──────────────────────────────────────────────────────────

#[tool_handler]
impl ServerHandler for HttpMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "MCP server for making HTTP REST requests. \
                 Use http_get to retrieve a resource and http_post to submit data. \
                 Both tools return the HTTP status code, response content-type, and body \
                 (truncated at 50 KB for large responses). \
                 Custom headers and per-request timeouts are supported."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = HttpMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
