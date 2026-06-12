use tonic::{transport::Server, Request, Response, Status};

// Generated code from proto/examples.proto
pub mod examples {
    tonic::include_proto!("examples");
}

use examples::{
    HelloReply, HelloRequest, MathReply, MathRequest,
    examples_server::{Examples, ExamplesServer},
};

// ── service implementation ────────────────────────────────────────────────────

#[derive(Default)]
struct ExamplesService;

#[tonic::async_trait]
impl Examples for ExamplesService {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let name = request.into_inner().name;
        println!("[server] say_hello: name={name}");
        Ok(Response::new(HelloReply {
            message: format!("Hello, {name}!"),
        }))
    }

    async fn add(
        &self,
        request: Request<MathRequest>,
    ) -> Result<Response<MathReply>, Status> {
        let MathRequest { a, b } = request.into_inner();
        let result = a + b;
        println!("[server] add: {a} + {b} = {result}");
        Ok(Response::new(MathReply { result }))
    }

    async fn multiply(
        &self,
        request: Request<MathRequest>,
    ) -> Result<Response<MathReply>, Status> {
        let MathRequest { a, b } = request.into_inner();
        let result = a * b;
        println!("[server] multiply: {a} × {b} = {result}");
        Ok(Response::new(MathReply { result }))
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    println!("gRPC server listening on {addr}");

    Server::builder()
        .add_service(ExamplesServer::new(ExamplesService::default()))
        .serve(addr)
        .await?;

    Ok(())
}
