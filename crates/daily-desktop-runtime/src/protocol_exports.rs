//! Export shared SDK/provider wire types from the desktop package explicitly.
//! Dependency test functions are not run by `cargo test -p daily-desktop-runtime`.
use ts_rs::TS;

#[test]
fn export_bindings_agent_types() {
    let config = ts_rs::Config::from_env();
    macro_rules! export {
        ($($ty:ty),+ $(,)?) => { $(<$ty>::export_all(&config).unwrap();)+ };
    }
    export!(
        daily_agent_core::AgentToolSpec,
        daily_agent_core::SubAgentConfig,
        daily_agent_core::AgentToolCallRecord,
        daily_agent_core::AgentToolObservation,
        daily_agent_core::DynamicAgentToolDefinition,
        daily_agent_core::DynamicAgentToolExecution,
        daily_agent_core::model::ModelUsage,
        daily_agent_core::model::ModelStopReason,
        daily_agent_providers::ModelProviderKind,
        daily_agent_providers::codex::CodexAuthStatusView,
        daily_agent_providers::codex::CodexLoginStartView,
        daily_agent_providers::codex::StartCodexLoginInput,
        daily_agent_providers::antigravity::AntigravityAuthStatusView,
        daily_agent_providers::antigravity::AntigravityLoginStartView,
        daily_agent_providers::antigravity::AntigravityModelCatalog,
        daily_agent_providers::antigravity::RefreshAntigravityModelsInput,
    );
}
