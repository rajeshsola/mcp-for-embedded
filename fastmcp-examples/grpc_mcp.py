"""gRPC MCP server — dynamic calls via gRPC Server Reflection.

Requires: grpcio, grpcio-reflection, protobuf
    pip install grpcio grpcio-reflection protobuf

The target gRPC server must have Server Reflection enabled
(grpc.reflection.v1alpha).

Workflow:
    1) grpc_list_services — discover available services
    2) grpc_describe      — inspect a service or method (fields, types)
    3) grpc_call          — invoke a unary method with a JSON request body
"""
import json
from typing import Optional

import grpc
from grpc_reflection.v1alpha import reflection_pb2, reflection_pb2_grpc
from google.protobuf import descriptor, descriptor_pb2, json_format
from google.protobuf import descriptor_pool as dp_module
from fastmcp import FastMCP

mcp = FastMCP(
    "grpc",
    instructions=(
        "MCP server for interacting with gRPC services via Server Reflection. "
        "Workflow: 1) grpc_list_services; 2) grpc_describe; 3) grpc_call."
    ),
)

# ── field type name map ───────────────────────────────────────────────────────

_FIELD_TYPE_NAMES: dict[int, str] = {
    v: k.lower().replace("type_", "")
    for k, v in vars(descriptor.FieldDescriptor).items()
    if k.startswith("TYPE_")
}
_LABEL_REPEATED = descriptor.FieldDescriptor.LABEL_REPEATED


# ── reflection helpers ────────────────────────────────────────────────────────

def _reflect_one(
    stub: reflection_pb2_grpc.ServerReflectionStub,
    request: reflection_pb2.ServerReflectionRequest,
) -> reflection_pb2.ServerReflectionResponse:
    for resp in stub.ServerReflectionInfo(iter([request])):
        return resp
    raise RuntimeError("empty reflection response — is reflection enabled on the server?")


def _fetch_all_fds(
    stub: reflection_pb2_grpc.ServerReflectionStub,
    initial_req: reflection_pb2.ServerReflectionRequest,
) -> list[descriptor_pb2.FileDescriptorProto]:
    """Recursively fetch FileDescriptorProtos for a symbol or filename."""
    seen: set[str] = set()
    fds: list[descriptor_pb2.FileDescriptorProto] = []
    queue: list[reflection_pb2.ServerReflectionRequest] = [initial_req]

    while queue:
        req = queue.pop(0)
        resp = _reflect_one(stub, req)

        if resp.HasField("error_response"):
            raise ValueError(
                f"reflection error {resp.error_response.error_code}: "
                f"{resp.error_response.error_message}"
            )
        if not resp.HasField("file_descriptor_response"):
            continue

        for fd_bytes in resp.file_descriptor_response.file_descriptor_proto:
            fd = descriptor_pb2.FileDescriptorProto()
            fd.ParseFromString(fd_bytes)
            if fd.name not in seen:
                seen.add(fd.name)
                fds.append(fd)
                for dep in fd.dependency:
                    if dep not in seen:
                        queue.append(
                            reflection_pb2.ServerReflectionRequest(
                                file_by_filename=dep
                            )
                        )
    return fds


def _build_pool(fds: list[descriptor_pb2.FileDescriptorProto]) -> dp_module.DescriptorPool:
    """Add FileDescriptorProtos to a new pool in dependency order."""
    pool = dp_module.DescriptorPool()
    fd_by_name = {fd.name: fd for fd in fds}
    added: set[str] = set()

    def _add(fd: descriptor_pb2.FileDescriptorProto) -> None:
        if fd.name in added:
            return
        for dep_name in fd.dependency:
            if dep_name in fd_by_name:
                _add(fd_by_name[dep_name])
        try:
            pool.Add(fd)
        except TypeError:
            pass  # already in the default pool
        added.add(fd.name)

    for fd in fds:
        _add(fd)

    return pool


def _get_message_classes(
    fds: list[descriptor_pb2.FileDescriptorProto],
    pool: dp_module.DescriptorPool,
) -> dict[str, type]:
    try:
        from google.protobuf.message_factory import GetMessages  # type: ignore[attr-defined]
        try:
            return GetMessages(fds, pool=pool)
        except TypeError:
            return GetMessages(fds)
    except ImportError:
        pass

    from google.protobuf.message_factory import MessageFactory  # type: ignore[attr-defined]
    factory = MessageFactory(pool=pool)
    result: dict[str, type] = {}
    for fd in fds:
        prefix = f"{fd.package}." if fd.package else ""
        for msg_type in fd.message_type:
            full_name = prefix + msg_type.name
            try:
                desc = pool.FindMessageTypeByName(full_name)
                result[full_name] = factory.GetPrototype(desc)
            except Exception:
                pass
    return result


def _format_field(f: descriptor.FieldDescriptor) -> str:
    if f.type == descriptor.FieldDescriptor.TYPE_MESSAGE:
        type_name = f.message_type.full_name
    elif f.type == descriptor.FieldDescriptor.TYPE_ENUM:
        type_name = f.enum_type.full_name
    else:
        type_name = _FIELD_TYPE_NAMES.get(f.type, str(f.type))
    label = "repeated " if f.label == _LABEL_REPEATED else ""
    return f"  {f.name} {label}{type_name} = {f.number}"


def _open_channel(endpoint: str) -> grpc.Channel:
    if endpoint.startswith("https://") or endpoint.startswith("grpcs://"):
        stripped = endpoint.replace("https://", "").replace("grpcs://", "")
        return grpc.secure_channel(stripped, grpc.ssl_channel_credentials())
    stripped = endpoint.replace("http://", "").replace("grpc://", "")
    return grpc.insecure_channel(stripped)


# ── tools ─────────────────────────────────────────────────────────────────────

@mcp.tool()
def grpc_list_services(endpoint: str) -> str:
    """List all gRPC services available on a server via gRPC Server Reflection.

    Args:
        endpoint: gRPC server endpoint, e.g. http://localhost:50051.
    """
    channel = _open_channel(endpoint)
    stub = reflection_pb2_grpc.ServerReflectionStub(channel)
    resp = _reflect_one(
        stub,
        reflection_pb2.ServerReflectionRequest(list_services=""),
    )
    channel.close()

    if not resp.HasField("list_services_response"):
        return "no services found (unexpected reflection response)"

    names = [s.name for s in resp.list_services_response.service]
    if not names:
        return "no services found"
    return f"{len(names)} service(s):\n" + "\n".join(names)


@mcp.tool()
def grpc_describe(endpoint: str, symbol: str) -> str:
    """Describe a gRPC service or method using server reflection.

    Pass a fully-qualified service name (e.g. helloworld.Greeter) to list all
    methods, or a method name (e.g. helloworld.Greeter.SayHello) to see
    input/output message fields.

    Args:
        endpoint: gRPC server endpoint.
        symbol: Fully-qualified service or method name.
    """
    channel = _open_channel(endpoint)
    stub = reflection_pb2_grpc.ServerReflectionStub(channel)

    fds = _fetch_all_fds(
        stub,
        reflection_pb2.ServerReflectionRequest(file_containing_symbol=symbol),
    )
    channel.close()

    pool = _build_pool(fds)

    # Try as a service name
    try:
        svc = pool.FindServiceByName(symbol)
        lines = [f"service {svc.full_name}"]
        for m in svc.methods:
            flags = {
                (False, False): "",
                (True, False): " [client-streaming]",
                (False, True): " [server-streaming]",
                (True, True): " [bidi-streaming]",
            }[(m.client_streaming, m.server_streaming)]
            lines.append(
                f"\n  rpc {m.name}({m.input_type.full_name}) "
                f"returns ({m.output_type.full_name}){flags}"
            )
            lines.append("    Input fields:")
            for f in m.input_type.fields:
                lines.append(f"    {_format_field(f)}")
            lines.append("    Output fields:")
            for f in m.output_type.fields:
                lines.append(f"    {_format_field(f)}")
        return "\n".join(lines)
    except KeyError:
        pass

    # Try as a method name (package.Service.Method)
    if "." in symbol:
        last_dot = symbol.rfind(".")
        svc_name, method_name = symbol[:last_dot], symbol[last_dot + 1:]
        try:
            svc = pool.FindServiceByName(svc_name)
            m = svc.methods_by_name[method_name]
            flags = {
                (False, False): "unary",
                (True, False): "client-streaming",
                (False, True): "server-streaming",
                (True, True): "bidi-streaming",
            }[(m.client_streaming, m.server_streaming)]
            lines = [f"method {m.full_name} [{flags}]"]
            lines.append(f"\nInput: {m.input_type.full_name}")
            for f in m.input_type.fields:
                lines.append(_format_field(f))
            lines.append(f"\nOutput: {m.output_type.full_name}")
            for f in m.output_type.fields:
                lines.append(_format_field(f))
            return "\n".join(lines)
        except (KeyError, ValueError):
            pass

    raise ValueError(f"'{symbol}' not found as a service or method name")


@mcp.tool()
def grpc_call(
    endpoint: str,
    service: str,
    method: str,
    request_json: str,
    metadata: Optional[dict] = None,
) -> str:
    """Make a unary gRPC call. Encode the request from JSON and return the
    response as a JSON object. The server must support gRPC Server Reflection.

    Args:
        endpoint: gRPC server endpoint, e.g. http://localhost:50051.
        service: Fully-qualified service name, e.g. helloworld.Greeter.
        method: Method name (unqualified), e.g. SayHello.
        request_json: Request message as a JSON object matching the method's
                      input type. Field names use snake_case or proto camelCase.
        metadata: Optional gRPC metadata headers as a JSON object,
                  e.g. {"authorization": "Bearer token"}.
    """
    channel = _open_channel(endpoint)
    stub = reflection_pb2_grpc.ServerReflectionStub(channel)

    symbol = f"{service}.{method}"
    fds = _fetch_all_fds(
        stub,
        reflection_pb2.ServerReflectionRequest(file_containing_symbol=symbol),
    )

    pool = _build_pool(fds)
    msg_classes = _get_message_classes(fds, pool)

    try:
        svc_desc = pool.FindServiceByName(service)
    except KeyError:
        channel.close()
        raise ValueError(f"service '{service}' not found")

    try:
        method_desc = svc_desc.methods_by_name[method]
    except KeyError:
        channel.close()
        raise ValueError(f"method '{method}' not found in service '{service}'")

    if method_desc.client_streaming or method_desc.server_streaming:
        channel.close()
        raise ValueError(
            "grpc_call only supports unary methods; "
            "streaming methods are not supported"
        )

    input_cls = msg_classes.get(method_desc.input_type.full_name)
    output_cls = msg_classes.get(method_desc.output_type.full_name)

    if input_cls is None or output_cls is None:
        channel.close()
        raise RuntimeError(
            f"could not resolve message classes for "
            f"'{method_desc.input_type.full_name}' / '{method_desc.output_type.full_name}'"
        )

    request_msg = json_format.Parse(request_json, input_cls())

    # Raw bytes channel call — avoids needing a generated stub
    raw_method = channel.unary_unary(
        f"/{service}/{method}",
        request_serializer=lambda m: m.SerializeToString(),
        response_deserializer=lambda b: b,
    )

    meta = list((metadata or {}).items())
    response_bytes: bytes = raw_method(request_msg, metadata=meta)
    channel.close()

    response_msg = output_cls()
    response_msg.ParseFromString(response_bytes)
    return json_format.MessageToJson(response_msg, indent=2)


if __name__ == "__main__":
    mcp.run()
