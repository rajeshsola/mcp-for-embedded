use std::path::PathBuf;

use rig::{
    client::{CompletionClient, ProviderClient},
    completion::Prompt,
    providers::openai,
    tool::server::ToolServer,
};
use rig::tool::rmcp::McpClientHandler;
use rmcp::{
    model::ClientInfo,
    transport::TokioChildProcess,
};
use tokio::process::Command;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = PathBuf::from("/home/rajeshsola/Public/AgenticAI/RMCP/simple-calculator/simple-mcp-server/Cargo.toml");

    let tool_server_handle = ToolServer::new().run();
    let handler = McpClientHandler::new(ClientInfo::default(), tool_server_handle.clone());

    let mut command = Command::new("cargo");
    command.current_dir("/home/rajeshsola/Public/AgenticAI/RMCP/simple-calculator/simple-mcp-server");
    command.args(["run", "--quiet", "--manifest-path", manifest_path.to_str().unwrap()]);

    let transport = TokioChildProcess::new(command)?;

    let _mcp_service = handler.connect(transport).await?;

    let client = openai::Client::from_env();
    let agent = client
        .agent(openai::GPT_4O_MINI)
        .preamble("You are a calculator assistant. Use the MCP tools whenever the task requires arithmetic.")
        .tool_server_handle(tool_server_handle)
        .build();

    let response = agent
        .prompt("Use the available MCP tools to compute 12 * 7 and then say the result in one short sentence.")
        .await?;

    println!("{response}");

    Ok(())
}
