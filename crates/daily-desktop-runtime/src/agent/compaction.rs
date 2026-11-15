use crate::agent::sdk::run_loop::DesktopModel;
use crate::model::{ModelAdapter, ModelConfig};
pub(crate) use daily_agent_core::compaction::{estimate_tokens, truncate_middle};

pub(crate) async fn summarize(
    http: &reqwest::Client,
    adapter: &dyn ModelAdapter,
    model: &ModelConfig,
    previous: Option<&str>,
    history: &str,
) -> Result<String, String> {
    daily_agent_core::compaction::summarize(
        &DesktopModel {
            http,
            adapter,
            config: model,
            session_id: None,
            turn_id: None,
        },
        previous,
        history,
    )
    .await
}
