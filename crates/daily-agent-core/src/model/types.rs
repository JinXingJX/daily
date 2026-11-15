use serde::{Deserialize, Serialize};

/// Lets hosts distinguish user work from context maintenance without interpreting task names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelRequestPurpose {
    #[default]
    Generation,
    ContextCompaction,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModelCompletionRequest {
    pub purpose: ModelRequestPurpose,
    pub messages: Vec<ModelMessage>,
    pub temperature: f32,
    pub max_output_tokens: Option<u32>,
    pub tools: Vec<ModelToolDefinition>,
    /// `None` keeps the tools declared, so the request prefix stays byte-stable for provider
    /// caches and reasoning replay, while forbidding the model from calling them.
    pub tool_choice: ModelToolChoice,
    pub request_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelToolChoice {
    #[default]
    Auto,
    None,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ModelCompletionResponse {
    pub raw_output: String,
    pub tool_calls: Vec<ModelToolCall>,
    pub stop_reason: ModelStopReason,
    pub usage: Option<ModelUsage>,
    /// Wire format that produced `provider_items` (see the `PROVIDER_*` constants).
    pub provider: String,
    /// Opaque items the provider wants back unchanged with this assistant turn: reasoning items,
    /// thinking blocks. Only the adapter that produced them reads them.
    pub provider_items: Vec<serde_json::Value>,
}

/// Why the model stopped generating.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModelStopReason {
    /// The model finished its answer.
    #[default]
    Complete,
    ToolCalls,
    /// The output hit the token limit and is cut off.
    MaxTokens,
    Refusal,
    Other,
}

/// Token counts of one model request, or the sum over several.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    /// All input tokens, including those read from the provider's prompt cache.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The part of `input_tokens` served from the prompt cache.
    #[serde(default)]
    pub cached_input_tokens: u64,
    /// The part of `output_tokens` spent on reasoning, when the provider reports it.
    #[serde(default)]
    pub reasoning_tokens: u64,
}

impl ModelUsage {
    pub fn add(&mut self, other: &ModelUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

pub struct ModelStreamChunk {
    pub text: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModelToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ModelToolCall {
    pub call_id: Option<String>,
    pub tool_id: String,
    pub arguments: serde_json::Value,
    /// Opaque per-call data the provider wants back with the call (e.g. a thought signature).
    pub provider_meta: serde_json::Value,
}

/// One message of a model conversation, in the provider-neutral shape every adapter maps to its
/// native format: text, images on user messages, tool calls on assistant messages, and tool
/// results as their own `Tool` messages.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ModelMessage {
    pub role: ModelMessageRole,
    pub content: String,
    /// Images attached to a user message.
    pub images: Vec<ModelInlineImage>,
    /// Tool calls an assistant message made.
    pub tool_calls: Vec<ModelToolCall>,
    /// Wire format that produced `provider_items`.
    pub provider: String,
    /// Opaque items returned with an assistant message, replayed unchanged to the same provider.
    pub provider_items: Vec<serde_json::Value>,
    /// The call a `Tool` message answers.
    pub tool_result: Option<ModelToolResultRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelToolResultRef {
    pub call_id: String,
    pub tool_id: String,
    pub is_error: bool,
}

impl ModelMessage {
    pub fn text(role: ModelMessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            ..Self::default()
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::text(ModelMessageRole::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::text(ModelMessageRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text(ModelMessageRole::Assistant, content)
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        tool_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: ModelMessageRole::Tool,
            content: content.into(),
            tool_result: Some(ModelToolResultRef {
                call_id: call_id.into(),
                tool_id: tool_id.into(),
                is_error,
            }),
            ..Self::default()
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModelInlineImage {
    pub name: Option<String>,
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelMessageRole {
    System,
    #[default]
    User,
    Assistant,
    /// The result of one tool call.
    Tool,
}

impl ModelMessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

pub const PROVIDER_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_OPENAI_RESPONSES: &str = "openai_responses";
pub const PROVIDER_GEMINI: &str = "gemini";
