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
        .preamble("Use the provided context to answer briefly.")
        .context("RIG is a Rust library for building LLM-powered applications.")
        .context("It focuses on simple APIs for completion, agents, and embeddings.")
        .build();

    let response = agent
        .prompt("What two ideas does this context highlight about RIG?")
        .await?;

    println!("{response}");

    Ok(())
}
