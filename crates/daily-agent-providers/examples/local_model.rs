//! Exercise the extracted SDK/provider path against the repository's local model simulation.
//! Start the Postman simulation, then run:
//! cargo run -p daily-agent-providers --example local_model -- http://127.0.0.1:4968/model/v1
use daily_agent_core::{run_agent_task, AgentTask};
use daily_agent_providers::{ModelConfig, ModelProviderKind, ProviderModel};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:4968/model/v1".into());
    let url = reqwest::Url::parse(&endpoint)?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
    {
        return Err("This example requires a local HTTP stand-in.".into());
    }
    let provider = ProviderModel::new(
        reqwest::Client::builder().no_proxy().build()?,
        ModelConfig::for_provider(
            ModelProviderKind::OpenAiCompatible,
            endpoint,
            "fixture",
            "local-test",
        ),
    );
    let task = AgentTask::new(
        "sdk.simulation",
        Arc::new(provider),
        "Return the local fixture response.",
    );
    let result = run_agent_task(task, |_| Ok(())).await?;
    println!("{}", result.final_text);
    Ok(())
}
