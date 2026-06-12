use rig::{
    client::{CompletionClient, ProviderClient},
    completion::Prompt,
    providers::openai,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set OPENAI_API_KEY before running this example.
    let openai_client = openai::Client::from_env();
    let agent = openai_client.agent(openai::GPT_4O_MINI).build();

    let response = agent
        .prompt("You are a concise Rust assistant. In one sentence, explain what RIG is.")
        .await?;

    println!("{response}");

    Ok(())
}
