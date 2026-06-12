use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use tokio::process::Command;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server_path = "../hello-mcp-server/target/debug/hello-mcp-server";

    let service = ().serve(TokioChildProcess::new(
        Command::new(server_path)
            .configure(|cmd| {
                cmd.current_dir("../hello-mcp-server");
            }),
    )?).await?;

    let tools = service.list_all_tools().await?;
    println!("Available tools: {}", tools.len());
    for tool in tools {
        println!("- {}", tool.name);
    }

    let add_result = service.call_tool(CallToolRequestParams {
        name: "add".into(),
        arguments: Some(serde_json::json!({ "a": 3.0, "b": 5.0 }).as_object().unwrap().clone()),
        meta: None,
        task: None,
    }).await?;
    println!("add result: {:?}", add_result);

    let multiply_result = service.call_tool(CallToolRequestParams {
        name: "multiply".into(),
        arguments: Some(serde_json::json!({ "a": 4.0, "b": 7.0 }).as_object().unwrap().clone()),
        meta: None,
        task: None,
    }).await?;
    println!("multiply result: {:?}", multiply_result);

    Ok(())
}
