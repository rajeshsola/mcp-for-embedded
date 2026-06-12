// Generated code from proto/examples.proto
pub mod examples {
    tonic::include_proto!("examples");
}

use examples::{
    HelloRequest, MathRequest,
    examples_client::ExamplesClient,
};

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server_addr = "http://[::1]:50051";
    println!("Connecting to {server_addr} ...\n");

    let mut client = ExamplesClient::connect(server_addr).await?;

    // ── SayHello ──────────────────────────────────────────────────────────────
    let reply = client
        .say_hello(HelloRequest { name: "World".to_string() })
        .await?
        .into_inner();
    println!("SayHello      → {}", reply.message);

    // ── Add ───────────────────────────────────────────────────────────────────
    let reply = client
        .add(MathRequest { a: 3.0, b: 4.0 })
        .await?
        .into_inner();
    println!("Add(3, 4)     → {}", reply.result);

    // ── Multiply ──────────────────────────────────────────────────────────────
    let reply = client
        .multiply(MathRequest { a: 6.0, b: 7.0 })
        .await?
        .into_inner();
    println!("Multiply(6, 7) → {}", reply.result);

    Ok(())
}
