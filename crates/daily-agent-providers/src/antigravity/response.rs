use super::{append_response, tool_names, RESPONSE_TIMEOUT};
use crate::{
    ModelCompletionResponse, ModelStopReason, ModelStreamChunk, ModelToolDefinition, ModelUsage,
    PROVIDER_GEMINI,
};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::Response;
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
pub(super) enum Recovery {
    Empty,
    MoreTokens,
    MalformedToolCall,
}

#[derive(Debug)]
pub(super) struct ResponseFailure {
    pub message: String,
    pub recovery: Option<Recovery>,
}

impl From<String> for ResponseFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            recovery: None,
        }
    }
}

impl From<&str> for ResponseFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

#[derive(Default)]
struct ResponseMetadata {
    finish_reason: Option<String>,
    block_reason: Option<String>,
    thoughts_tokens: Option<u64>,
    usage: Option<ModelUsage>,
}

impl ResponseMetadata {
    fn observe(&mut self, payload: &Value) {
        let response = payload.get("response").unwrap_or(payload);
        if let Some(reason) = response
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(reason.into());
        }
        if let Some(reason) = response
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
        {
            if reason != "BLOCK_REASON_UNSPECIFIED" {
                self.block_reason = Some(reason.into());
            }
        }
        if let Some(tokens) = response
            .pointer("/usageMetadata/thoughtsTokenCount")
            .and_then(Value::as_u64)
        {
            self.thoughts_tokens = Some(tokens);
        }
        if let Some(usage) = response.get("usageMetadata") {
            let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
            let thoughts = count("thoughtsTokenCount");
            self.usage = Some(ModelUsage {
                input_tokens: count("promptTokenCount"),
                output_tokens: count("candidatesTokenCount") + thoughts,
                cached_input_tokens: count("cachedContentTokenCount"),
                reasoning_tokens: thoughts,
            });
        }
    }

    fn stop_reason(&self, has_tool_calls: bool) -> ModelStopReason {
        match self.finish_reason.as_deref() {
            _ if has_tool_calls => ModelStopReason::ToolCalls,
            Some("MAX_TOKENS") => ModelStopReason::MaxTokens,
            Some("STOP") | None => ModelStopReason::Complete,
            Some(_) => ModelStopReason::Other,
        }
    }

    fn empty_failure(&self) -> ResponseFailure {
        if self.block_reason.is_some() {
            return "Antigravity blocked this request. Adjust the content and try again.".into();
        }
        let (message, recovery) = match self.finish_reason.as_deref() {
            Some("MAX_TOKENS") => {
                let usage = self.thoughts_tokens.map(|tokens| format!(", {tokens} of them on thinking")).unwrap_or_default();
                (format!("Antigravity ran out of output tokens (MAX_TOKENS{usage}) before producing text or a tool call. Raise the output limit or switch models."), Some(Recovery::MoreTokens))
            }
            Some("MALFORMED_FUNCTION_CALL") => (
                "Antigravity produced an invalid tool call (MALFORMED_FUNCTION_CALL). Continue from the checkpoint or switch models.".into(),
                Some(Recovery::MalformedToolCall),
            ),
            Some("UNEXPECTED_TOOL_CALL") => (
                "Antigravity called a tool that is not enabled (UNEXPECTED_TOOL_CALL). Continue from the checkpoint or switch models.".into(),
                Some(Recovery::MalformedToolCall),
            ),
            Some("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY") => (
                "Antigravity blocked the generated content. Adjust the content and try again.".into(), None,
            ),
            None | Some("STOP" | "OTHER" | "FINISH_REASON_UNSPECIFIED") => (
                "Antigravity returned no text or tool call. Continue from the checkpoint or try again later.".into(), Some(Recovery::Empty),
            ),
            Some(_) => ("Antigravity did not finish generating. Switch models or check the account in the official client.".into(), None),
        };
        ResponseFailure { message, recovery }
    }
}

pub(super) async fn read_response(
    response: Response,
    stream: bool,
    tools: &[ModelToolDefinition],
    mut on_chunk: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
) -> Result<ModelCompletionResponse, ResponseFailure> {
    let names = tool_names(tools);
    let mut result = ModelCompletionResponse {
        provider: PROVIDER_GEMINI.into(),
        ..ModelCompletionResponse::default()
    };
    let mut metadata = ResponseMetadata::default();
    if stream {
        let mut events = response.bytes_stream().eventsource();
        while let Some(event) = tokio::time::timeout(RESPONSE_TIMEOUT, events.next())
            .await
            .map_err(|_| "Timed out waiting for the Antigravity stream. Try again.")?
        {
            let event = event.map_err(|_| "The Antigravity stream was interrupted. Try again.")?;
            let data = event.data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let payload: Value = serde_json::from_str(data)
                .map_err(|_| "The Antigravity stream returned an invalid event.")?;
            metadata.observe(&payload);
            let text = append_response(&payload, &names, &mut result)?;
            if !text.is_empty() {
                if let Some(callback) = on_chunk.as_mut() {
                    callback(ModelStreamChunk { text })?;
                }
            }
        }
    } else {
        let payload: Value = response
            .json()
            .await
            .map_err(|_| "Antigravity returned an invalid response.")?;
        metadata.observe(&payload);
        let text = append_response(&payload, &names, &mut result)?;
        if !text.is_empty() {
            if let Some(callback) = on_chunk.as_mut() {
                callback(ModelStreamChunk { text })?;
            }
        }
    }
    if result.raw_output.is_empty() && result.tool_calls.is_empty() {
        return Err(metadata.empty_failure());
    }
    if result.raw_output.trim().is_empty() && result.tool_calls.is_empty() {
        // Whitespace may already have been streamed; do not replay it.
        return Err(ResponseFailure {
            recovery: None,
            ..metadata.empty_failure()
        });
    }
    result.stop_reason = metadata.stop_reason(!result.tool_calls.is_empty());
    result.usage = metadata.usage;
    Ok(result)
}
