/// gRPC MCP server
///
/// Provides three tools:
///   grpc_list_services – discover services via gRPC server reflection
///   grpc_describe      – describe a service or method (fields, streaming flags)
///   grpc_call          – make a unary gRPC call with a JSON request, get a JSON response
///
/// Encoding/decoding uses gRPC Server Reflection (v1alpha) + prost-reflect dynamic messages.
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};

use bytes::{Buf, BufMut, Bytes};
use futures::stream;
use http::uri::PathAndQuery;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, FieldDescriptor, Kind};
use serde::de::DeserializeSeed;
use std::collections::{HashMap, HashSet};
use tonic::{
    codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder},
    transport::Channel,
};

// ── inline gRPC reflection proto types (v1alpha) ─────────────────────────────

#[derive(Clone, PartialEq, prost::Message)]
struct ServerReflectionRequest {
    #[prost(string, tag = "1")]
    pub host: String,
    #[prost(oneof = "reflect_request::MessageRequest", tags = "3, 4, 7")]
    pub message_request: Option<reflect_request::MessageRequest>,
}

mod reflect_request {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum MessageRequest {
        #[prost(string, tag = "3")]
        FileByFilename(String),
        #[prost(string, tag = "4")]
        FileContainingSymbol(String),
        #[prost(string, tag = "7")]
        ListServices(String),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct ServerReflectionResponse {
    #[prost(string, tag = "1")]
    pub valid_host: String,
    #[prost(oneof = "reflect_response::MessageResponse", tags = "4, 6, 7")]
    pub message_response: Option<reflect_response::MessageResponse>,
}

mod reflect_response {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum MessageResponse {
        #[prost(message, tag = "4")]
        FileDescriptorResponse(super::FileDescriptorResponse),
        #[prost(message, tag = "6")]
        ListServicesResponse(super::ListServiceResponse),
        #[prost(message, tag = "7")]
        ErrorResponse(super::ReflectionError),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct FileDescriptorResponse {
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub file_descriptor_proto: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ListServiceResponse {
    #[prost(message, repeated, tag = "1")]
    pub service: Vec<ServiceResponse>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ServiceResponse {
    #[prost(string, tag = "1")]
    pub name: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ReflectionError {
    #[prost(int32, tag = "1")]
    pub error_code: i32,
    #[prost(string, tag = "2")]
    pub error_message: String,
}

// ── raw bytes codec for dynamic gRPC calls ────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct RawBytesCodec;

#[derive(Debug, Clone, Default)]
struct RawBytesEncoder;

#[derive(Debug, Clone, Default)]
struct RawBytesDecoder;

impl Encoder for RawBytesEncoder {
    type Item = Bytes;
    type Error = tonic::Status;
    fn encode(&mut self, item: Bytes, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        dst.put(item);
        Ok(())
    }
}

impl Decoder for RawBytesDecoder {
    type Item = Bytes;
    type Error = tonic::Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Bytes>, Self::Error> {
        let remaining = src.remaining();
        if remaining == 0 {
            return Ok(None);
        }
        Ok(Some(src.copy_to_bytes(remaining)))
    }
}

impl Codec for RawBytesCodec {
    type Encode = Bytes;
    type Decode = Bytes;
    type Encoder = RawBytesEncoder;
    type Decoder = RawBytesDecoder;
    fn encoder(&mut self) -> RawBytesEncoder { RawBytesEncoder }
    fn decoder(&mut self) -> RawBytesDecoder { RawBytesDecoder }
}

// ── prost codec for the reflection protocol ──────────────────────────────────

#[derive(Debug, Clone, Default)]
struct ReflectionCodec;
#[derive(Debug, Clone, Default)]
struct ReflectionEncoder;
#[derive(Debug, Clone, Default)]
struct ReflectionDecoder;

impl Encoder for ReflectionEncoder {
    type Item = ServerReflectionRequest;
    type Error = tonic::Status;
    fn encode(&mut self, item: ServerReflectionRequest, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst).map_err(|e| tonic::Status::internal(e.to_string()))
    }
}

impl Decoder for ReflectionDecoder {
    type Item = ServerReflectionResponse;
    type Error = tonic::Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<ServerReflectionResponse>, Self::Error> {
        let remaining = src.remaining();
        if remaining == 0 { return Ok(None); }
        let bytes = src.copy_to_bytes(remaining);
        ServerReflectionResponse::decode(bytes)
            .map(Some)
            .map_err(|e| tonic::Status::internal(format!("decode: {}", e)))
    }
}

impl Codec for ReflectionCodec {
    type Encode = ServerReflectionRequest;
    type Decode = ServerReflectionResponse;
    type Encoder = ReflectionEncoder;
    type Decoder = ReflectionDecoder;
    fn encoder(&mut self) -> ReflectionEncoder { ReflectionEncoder }
    fn decoder(&mut self) -> ReflectionDecoder { ReflectionDecoder }
}

// ── gRPC / reflection helpers ─────────────────────────────────────────────────

const REFLECTION_PATH: &str =
    "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo";

fn mcp_err(msg: impl std::fmt::Display) -> McpError {
    McpError::internal_error(msg.to_string(), None)
}

async fn connect(endpoint: &str) -> Result<Channel, McpError> {
    Channel::from_shared(endpoint.to_string())
        .map_err(|e| McpError::invalid_params(format!("invalid endpoint '{}': {}", endpoint, e), None))?
        .connect()
        .await
        .map_err(|e| mcp_err(format!("cannot connect to '{}': {}", endpoint, e)))
}

/// Single reflection request → single reflection response.
async fn reflect(
    channel: Channel,
    request: ServerReflectionRequest,
) -> Result<ServerReflectionResponse, McpError> {
    let mut client = tonic::client::Grpc::new(channel);
    client.ready().await.map_err(|e| mcp_err(format!("gRPC not ready: {}", e)))?;

    let path: PathAndQuery = REFLECTION_PATH.parse().unwrap();
    let req_stream = stream::once(futures::future::ready(request));
    let response: tonic::Response<tonic::Streaming<ServerReflectionResponse>> = client
        .streaming(
            tonic::Request::new(req_stream),
            path,
            ReflectionCodec,
        )
        .await
        .map_err(|e| mcp_err(format!("reflection RPC failed: {}", e)))?;

    response
        .into_inner()
        .message()
        .await
        .map_err(|e| mcp_err(format!("reflection read error: {}", e)))?
        .ok_or_else(|| mcp_err("empty reflection response — is reflection enabled on the server?"))
}

/// Fetch FileDescriptorProtos for a symbol, then recursively resolve all imports.
async fn descriptor_pool_for_symbol(
    endpoint: &str,
    symbol: &str,
) -> Result<DescriptorPool, McpError> {
    let mut raw_fds: Vec<Vec<u8>> = Vec::new();
    let mut fetched_files: HashSet<String> = HashSet::new();
    let mut pending_files: Vec<String> = Vec::new();

    // First: file(s) containing the requested symbol
    let ch = connect(endpoint).await?;
    let resp = reflect(
        ch,
        ServerReflectionRequest {
            host: String::new(),
            message_request: Some(reflect_request::MessageRequest::FileContainingSymbol(
                symbol.to_string(),
            )),
        },
    )
    .await?;

    match resp.message_response {
        Some(reflect_response::MessageResponse::FileDescriptorResponse(fdr)) => {
            for bytes in fdr.file_descriptor_proto {
                // Collect import names that need to be resolved
                if let Ok(fd) = prost_types::FileDescriptorProto::decode(bytes.as_slice()) {
                    let name = fd.name().to_string();
                    for dep in &fd.dependency {
                        if !fetched_files.contains(dep) {
                            pending_files.push(dep.clone());
                        }
                    }
                    fetched_files.insert(name);
                }
                raw_fds.push(bytes);
            }
        }
        Some(reflect_response::MessageResponse::ErrorResponse(e)) => {
            return Err(McpError::invalid_params(
                format!("reflection error {}: {}", e.error_code, e.error_message),
                None,
            ));
        }
        _ => return Err(mcp_err("unexpected reflection response")),
    }

    // Recursively fetch imports by filename
    while let Some(filename) = pending_files.pop() {
        if fetched_files.contains(&filename) {
            continue;
        }
        fetched_files.insert(filename.clone());

        let ch2 = connect(endpoint).await?;
        let resp2 = reflect(
            ch2,
            ServerReflectionRequest {
                host: String::new(),
                message_request: Some(reflect_request::MessageRequest::FileByFilename(
                    filename.clone(),
                )),
            },
        )
        .await?;

        if let Some(reflect_response::MessageResponse::FileDescriptorResponse(fdr)) =
            resp2.message_response
        {
            for bytes in fdr.file_descriptor_proto {
                if let Ok(fd) = prost_types::FileDescriptorProto::decode(bytes.as_slice()) {
                    for dep in &fd.dependency {
                        if !fetched_files.contains(dep) {
                            pending_files.push(dep.clone());
                        }
                    }
                }
                raw_fds.push(bytes);
            }
        }
    }

    // Build DescriptorPool; retry in case of ordering issues
    let mut pool = DescriptorPool::new();
    let mut fds: Vec<prost_types::FileDescriptorProto> = raw_fds
        .into_iter()
        .filter_map(|b| prost_types::FileDescriptorProto::decode(b.as_slice()).ok())
        .collect();

    let max_passes = fds.len() + 1;
    for _ in 0..max_passes {
        fds.retain(|fd| pool.add_file_descriptor_proto(fd.clone()).is_err());
        if fds.is_empty() {
            break;
        }
    }

    Ok(pool)
}

/// Make a raw unary gRPC call and return the response bytes.
async fn raw_unary_call(
    endpoint: &str,
    service: &str,
    method: &str,
    request_bytes: Bytes,
    metadata: Option<HashMap<String, String>>,
) -> Result<Bytes, McpError> {
    let channel = connect(endpoint).await?;
    let mut client = tonic::client::Grpc::new(channel);
    client.ready().await.map_err(|e| mcp_err(format!("gRPC not ready: {}", e)))?;

    let mut request = tonic::Request::new(request_bytes);
    if let Some(meta) = metadata {
        for (k, v) in meta {
            use tonic::metadata::{MetadataKey, MetadataValue};
            use std::str::FromStr;
            if let (Ok(key), Ok(val)) = (
                MetadataKey::from_bytes(k.as_bytes()),
                MetadataValue::from_str(&v),
            ) {
                request.metadata_mut().insert(key, val);
            }
        }
    }

    let path: PathAndQuery = format!("/{}/{}", service, method)
        .parse()
        .map_err(|e| McpError::invalid_params(format!("invalid path: {}", e), None))?;

    client
        .unary(request, path, RawBytesCodec)
        .await
        .map(|r| r.into_inner())
        .map_err(|s| mcp_err(format!("gRPC call failed: {} — {}", s.code(), s.message())))
}

fn format_field(f: &FieldDescriptor) -> String {
    let kind = match f.kind() {
        Kind::Message(m) => m.full_name().to_string(),
        Kind::Enum(e) => e.full_name().to_string(),
        k => format!("{:?}", k).to_lowercase(),
    };
    let label = if f.is_list() {
        "repeated "
    } else if f.is_map() {
        "map "
    } else {
        ""
    };
    format!("  {} {}{}  = {}", f.name(), label, kind, f.number())
}

// ── parameter structs ─────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ListServicesParams {
    #[schemars(description = "gRPC server endpoint, e.g. http://localhost:50051")]
    endpoint: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct DescribeParams {
    #[schemars(description = "gRPC server endpoint")]
    endpoint: String,
    #[schemars(
        description = "Fully-qualified service name (e.g. helloworld.Greeter) \
                       or method name (e.g. helloworld.Greeter.SayHello)"
    )]
    symbol: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct CallParams {
    #[schemars(description = "gRPC server endpoint, e.g. http://localhost:50051")]
    endpoint: String,
    #[schemars(
        description = "Fully-qualified service name, e.g. helloworld.Greeter"
    )]
    service: String,
    #[schemars(description = "Method name (unqualified), e.g. SayHello")]
    method: String,
    #[schemars(
        description = "Request message as a JSON object matching the method's input type. \
                       Field names use snake_case or proto camelCase."
    )]
    request_json: String,
    #[schemars(
        description = "Optional gRPC metadata headers (e.g. {\"authorization\": \"Bearer token\"})"
    )]
    metadata: Option<HashMap<String, String>>,
}

// ── server ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct GrpcMcpServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl GrpcMcpServer {
    fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }

    #[tool(
        description = "List all gRPC services available on a server via gRPC Server Reflection. \
                       The server must have reflection enabled (grpc.reflection.v1alpha)."
    )]
    async fn grpc_list_services(
        &self,
        Parameters(p): Parameters<ListServicesParams>,
    ) -> Result<CallToolResult, McpError> {
        let channel = connect(&p.endpoint).await?;
        let resp = reflect(
            channel,
            ServerReflectionRequest {
                host: String::new(),
                message_request: Some(reflect_request::MessageRequest::ListServices(
                    String::new(),
                )),
            },
        )
        .await?;

        match resp.message_response {
            Some(reflect_response::MessageResponse::ListServicesResponse(list)) => {
                let names: Vec<String> = list.service.into_iter().map(|s| s.name).collect();
                if names.is_empty() {
                    Ok(CallToolResult::success(vec![Content::text("no services found")]))
                } else {
                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "{} service(s):\n{}",
                        names.len(),
                        names.join("\n")
                    ))]))
                }
            }
            Some(reflect_response::MessageResponse::ErrorResponse(e)) => {
                Err(McpError::internal_error(
                    format!("reflection error {}: {}", e.error_code, e.error_message),
                    None,
                ))
            }
            _ => Err(mcp_err("unexpected reflection response")),
        }
    }

    #[tool(
        description = "Describe a gRPC service or method using server reflection. \
                       Pass a fully-qualified service name (e.g. helloworld.Greeter) to list all \
                       methods, or a method name (e.g. helloworld.Greeter.SayHello) to see \
                       input/output message fields."
    )]
    async fn grpc_describe(
        &self,
        Parameters(p): Parameters<DescribeParams>,
    ) -> Result<CallToolResult, McpError> {
        // Determine if this is a method reference (service.Method) or a service name
        // A method has the form  pkg.Service.Method  where Method has no dot after it
        let pool = descriptor_pool_for_symbol(&p.endpoint, &p.symbol).await?;

        // Try as a service name first
        if let Some(svc) = pool.get_service_by_name(&p.symbol) {
            let mut out = format!("service {}\n", svc.full_name());
            for method in svc.methods() {
                let flags = match (method.is_client_streaming(), method.is_server_streaming()) {
                    (true, true) => " [bidi-streaming]",
                    (true, false) => " [client-streaming]",
                    (false, true) => " [server-streaming]",
                    (false, false) => "",
                };
                out.push_str(&format!(
                    "\n  rpc {}({}) returns ({}){}\n",
                    method.name(),
                    method.input().full_name(),
                    method.output().full_name(),
                    flags,
                ));
                out.push_str("    Input fields:\n");
                for f in method.input().fields() {
                    out.push_str(&format!("    {}\n", format_field(&f)));
                }
                out.push_str("    Output fields:\n");
                for f in method.output().fields() {
                    out.push_str(&format!("    {}\n", format_field(&f)));
                }
            }
            return Ok(CallToolResult::success(vec![Content::text(out)]));
        }

        // Try as a method full name:  pkg.Service.Method
        if let Some(dot) = p.symbol.rfind('.') {
            let svc_name = &p.symbol[..dot];
            let method_name = &p.symbol[dot + 1..];
            if let Some(svc) = pool.get_service_by_name(svc_name) {
                if let Some(method) = svc.methods().find(|m| m.name() == method_name) {
                    let flags = match (method.is_client_streaming(), method.is_server_streaming()) {
                        (true, true) => "bidi-streaming",
                        (true, false) => "client-streaming",
                        (false, true) => "server-streaming",
                        (false, false) => "unary",
                    };
                    let mut out = format!(
                        "method {} [{}]\n\nInput: {}\n",
                        method.full_name(),
                        flags,
                        method.input().full_name(),
                    );
                    for f in method.input().fields() {
                        out.push_str(&format!("  {}\n", format_field(&f)));
                    }
                    out.push_str(&format!("\nOutput: {}\n", method.output().full_name()));
                    for f in method.output().fields() {
                        out.push_str(&format!("  {}\n", format_field(&f)));
                    }
                    return Ok(CallToolResult::success(vec![Content::text(out)]));
                }
            }
        }

        Err(McpError::invalid_params(
            format!("'{}' not found as a service or method name", p.symbol),
            None,
        ))
    }

    #[tool(
        description = "Make a unary gRPC call. Provide the service name, method name, and request \
                       as a JSON object. The server must support gRPC Server Reflection for \
                       message encoding. Returns the response as a JSON object."
    )]
    async fn grpc_call(
        &self,
        Parameters(p): Parameters<CallParams>,
    ) -> Result<CallToolResult, McpError> {
        // Build the fully-qualified method symbol for reflection lookup
        let method_symbol = format!("{}.{}", p.service, p.method);
        let pool = descriptor_pool_for_symbol(&p.endpoint, &method_symbol).await?;

        let svc = pool.get_service_by_name(&p.service).ok_or_else(|| {
            McpError::invalid_params(format!("service '{}' not found", p.service), None)
        })?;
        let method_desc = svc
            .methods()
            .find(|m| m.name() == p.method)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("method '{}' not found in service '{}'", p.method, p.service),
                    None,
                )
            })?;

        if method_desc.is_client_streaming() || method_desc.is_server_streaming() {
            return Err(McpError::invalid_params(
                "grpc_call only supports unary methods; streaming methods are not yet supported",
                None,
            ));
        }

        // Decode JSON request → DynamicMessage → protobuf bytes
        let input_desc = method_desc.input();
        let request_msg: DynamicMessage = input_desc
            .deserialize(&mut serde_json::Deserializer::from_str(&p.request_json))
            .map_err(|e| {
                McpError::invalid_params(format!("failed to parse request_json: {}", e), None)
            })?;
        let request_bytes = Bytes::from(request_msg.encode_to_vec());

        // Make the gRPC call
        let response_bytes =
            raw_unary_call(&p.endpoint, &p.service, &p.method, request_bytes, p.metadata).await?;

        // Decode protobuf response bytes → DynamicMessage → JSON
        let output_desc = method_desc.output();
        let response_msg = DynamicMessage::decode(output_desc, response_bytes.as_ref())
            .map_err(|e| mcp_err(format!("failed to decode response: {}", e)))?;
        let response_json = serde_json::to_string_pretty(&response_msg)
            .map_err(|e| mcp_err(format!("failed to serialize response: {}", e)))?;

        Ok(CallToolResult::success(vec![Content::text(response_json)]))
    }
}

// ── server metadata ───────────────────────────────────────────────────────────

#[tool_handler]
impl ServerHandler for GrpcMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "MCP server for interacting with gRPC services. \
                 The target gRPC server must have Server Reflection enabled \
                 (grpc.reflection.v1alpha). Workflow: \
                 1) grpc_list_services to see available services; \
                 2) grpc_describe to inspect a service or method and its fields; \
                 3) grpc_call to invoke a unary method with a JSON request body."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = GrpcMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
