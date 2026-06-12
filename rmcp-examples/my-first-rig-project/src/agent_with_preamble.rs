use rig::{
    client::{CompletionClient, ProviderClient},
    completion::Prompt,
    providers::openai,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = openai::Client::from_env();
    let agent = client
        .agent(openai::GPT_4O_MINI)
        .preamble("You are a concise Rust assistant. Always answer in one short sentence.")
        .temperature(0.2)
        .build();

    let response = agent
        .prompt("Why is Rust a good fit for AI agents?")
        .await?;

    println!("{response}");

    Ok(())
}
