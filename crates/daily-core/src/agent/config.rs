use serde::Serialize;

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are a local desktop agent inside the Daily Electron app.
Use the available tools when they reduce uncertainty or improve the answer.
Keep rationales short and concrete."#;

#[derive(Clone)]
pub struct AgentConfig {
    pub system_prompt: String,
    pub max_iterations: u8,
    pub dialogue_window: u8,
    pub memory_window: u8,
}

#[derive(Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AgentConfigView {
    pub system_prompt: String,
    pub max_iterations: u8,
    pub dialogue_window: u8,
    pub memory_window: u8,
}

impl AgentConfig {
    pub fn seeded() -> Self {
        Self {
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
            max_iterations: 3,
            dialogue_window: 8,
            memory_window: 18,
        }
    }

    pub fn view(&self) -> AgentConfigView {
        AgentConfigView {
            system_prompt: self.system_prompt.clone(),
            max_iterations: self.max_iterations,
            dialogue_window: self.dialogue_window,
            memory_window: self.memory_window,
        }
    }
}
