use rig::{
    client::{CompletionClient, ProviderClient},
    completion::Prompt,
    providers::openai,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = openai::Client::from_env();
    let agent = client.agent(openai::GPT_4O_MINI).build();

    let response = agent
        .prompt("Say hello and explain what RIG is in one sentence.")
        .await?;

    println!("{response}");

    Ok(())
}
