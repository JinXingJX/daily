//! Plumbing shared by the HTTP adapters: sending a request with typed errors, reading
//! server-sent events with an idle timeout, and API-safe tool names.
use super::{ModelError, ModelErrorKind, ModelToolDefinition};
use eventsource_stream::{Event, EventStreamError, Eventsource};
use futures_util::{Stream, StreamExt};
use reqwest::{RequestBuilder, Response};
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;

/// A stream that sends nothing for this long is considered dead. Models writing a long tool
/// call can stay silent for minutes, so this is generous.
pub(crate) const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Upper bound for a request whose whole response arrives at once.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(900);
const MAX_ERROR_BODY_CHARS: usize = 4_000;

/// Sends `request` and turns transport failures and non-success statuses into [`ModelError`]s.
pub(crate) async fn send(
    label: &str,
    request: RequestBuilder,
    stream: bool,
) -> Result<Response, ModelError> {
    let request = if stream {
        request
    } else {
        request.timeout(REQUEST_TIMEOUT)
    };
    let response = request
        .send()
        .await
        .map_err(|error| crate::transport_error(label, &error))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map(Duration::from_secs_f64);
    let body = response
        .text()
        .await
        .map_err(|error| crate::transport_error(label, &error))?;
    let body = if body.chars().count() > MAX_ERROR_BODY_CHARS {
        format!(
            "{}…",
            body.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>()
        )
    } else {
        body
    };
    Err(ModelError::from_status(
        label,
        status.as_u16(),
        &body,
        retry_after,
    ))
}

type EventStream =
    Pin<Box<dyn Stream<Item = Result<Event, EventStreamError<reqwest::Error>>> + Send>>;

/// The `data` payloads of a server-sent event response.
pub(crate) struct SseEvents {
    label: String,
    events: EventStream,
}

impl SseEvents {
    pub(crate) fn new(label: &str, response: Response) -> Self {
        Self {
            label: label.to_string(),
            events: Box::pin(response.bytes_stream().eventsource()),
        }
    }

    /// The next non-empty payload, or `None` at the end of the stream.
    pub(crate) async fn next(&mut self) -> Result<Option<String>, ModelError> {
        loop {
            let event = tokio::time::timeout(STREAM_IDLE_TIMEOUT, self.events.next())
                .await
                .map_err(|_| {
                    ModelError::new(
                        ModelErrorKind::Timeout,
                        format!(
                            "{} stream sent nothing for {}s.",
                            self.label,
                            STREAM_IDLE_TIMEOUT.as_secs()
                        ),
                    )
                })?;
            match event {
                None => return Ok(None),
                Some(Ok(event)) => {
                    let data = event.data.trim();
                    if !data.is_empty() && data != "[DONE]" {
                        return Ok(Some(data.to_string()));
                    }
                }
                Some(Err(error)) => {
                    return Err(ModelError::new(
                        ModelErrorKind::Network,
                        format!(
                            "{} stream was interrupted: {}",
                            self.label,
                            crate::net::describe_error_chain(&error)
                        ),
                    ))
                }
            }
        }
    }
}

/// A tool id as a function name every provider accepts: letters, digits, `_` and `-`,
/// starting with a letter, at most 64 characters.
pub(crate) fn api_tool_name(tool_id: &str) -> String {
    let mut name = tool_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if !name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic())
    {
        name = format!("tool_{name}");
    }
    name.chars().take(64).collect()
}

/// Prompts and plugins refer to tools by their Daily id (`writer.readDocument`), which is not
/// a valid function name everywhere, so the description keeps the id next to the API name.
pub(crate) fn tool_description(tool: &ModelToolDefinition) -> String {
    format!(
        "{}: {} (Daily tool id: {})",
        tool.name, tool.description, tool.id
    )
}

/// API function name to tool id, for mapping the model's calls back.
pub(crate) fn tool_name_map(tools: &[ModelToolDefinition]) -> HashMap<String, String> {
    tools
        .iter()
        .map(|tool| (api_tool_name(&tool.id), tool.id.clone()))
        .collect()
}

/// The tool id a called function name stands for; unknown names pass through so the agent
/// loop can tell the model the tool does not exist.
pub(crate) fn tool_id_for(names: &HashMap<String, String>, api_name: &str) -> String {
    names
        .get(api_name)
        .cloned()
        .unwrap_or_else(|| api_name.to_string())
}

/// Parses the JSON arguments of a tool call. `truncated` is whether the output hit the token
/// limit, which is the usual reason for incomplete JSON.
pub(crate) fn parse_tool_arguments(
    label: &str,
    name: &str,
    raw: &str,
    truncated: bool,
) -> Result<serde_json::Value, ModelError> {
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(raw).map_err(|error| {
        if truncated {
            ModelError::protocol(format!(
                "{label} output hit the token limit in the middle of the {name} tool call. Raise the max output tokens and continue."
            ))
        } else {
            ModelError::protocol(format!(
                "Failed to parse {label} tool call arguments for {name}: {error}"
            ))
        }
    })
}

/// What to do when a provider rejects the requested output-token limit as too large for the
/// model: `Some(limit)` is the largest value the error message names below the request,
/// `None` means the message names none and the request should go without a limit.
/// Returns `Err(())` when the error is about something else.
pub(crate) fn lower_output_limit(error: &ModelError, requested: u32) -> Result<Option<u32>, ()> {
    if error.kind != ModelErrorKind::InvalidRequest {
        return Err(());
    }
    let lower = error.message.to_ascii_lowercase();
    let about_limit = [
        "max_tokens",
        "max_completion_tokens",
        "max_output_tokens",
        "maxoutputtokens",
        "max output tokens",
        "maximum output",
        "output token",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    if !about_limit {
        return Err(());
    }
    let allowed = lower
        .split(|ch: char| !ch.is_ascii_digit())
        .filter_map(|number| number.parse::<u32>().ok())
        // The message starts with the HTTP status, which is not a limit.
        .filter(|number| Some(*number) != error.status.map(u32::from))
        .filter(|number| (256..requested).contains(number))
        .max();
    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_api_safe_and_map_back() {
        assert_eq!(api_tool_name("workspace.read"), "workspace_read");
        assert_eq!(api_tool_name("9lives"), "tool_9lives");
        assert_eq!(api_tool_name(""), "tool_");
        assert_eq!(api_tool_name(&"a".repeat(80)).len(), 64);
        let tools = vec![ModelToolDefinition {
            id: "workspace.read".into(),
            name: "Read".into(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        }];
        let names = tool_name_map(&tools);
        assert_eq!(tool_id_for(&names, "workspace_read"), "workspace.read");
        assert_eq!(tool_id_for(&names, "made_up"), "made_up");
    }

    #[test]
    fn rejected_output_limits_are_lowered_to_what_the_error_names() {
        let error = |body: &str| ModelError::from_status("Test", 400, body, None);
        assert_eq!(
            lower_output_limit(
                &error("max_tokens is too large: 32000. This model supports at most 16384 completion tokens"),
                32_000
            ),
            Ok(Some(16_384))
        );
        assert_eq!(
            lower_output_limit(
                &error("max_tokens: 32000 > 8192, which is the maximum allowed number of output tokens"),
                32_000
            ),
            Ok(Some(8_192))
        );
        assert_eq!(
            lower_output_limit(&error("maxOutputTokens is out of range"), 32_000),
            Ok(None)
        );
        assert_eq!(lower_output_limit(&error("unknown model"), 32_000), Err(()));
        assert_eq!(
            lower_output_limit(
                &ModelError::from_status("Test", 429, "max_tokens", None),
                32_000
            ),
            Err(())
        );
    }

    #[test]
    fn truncated_tool_arguments_explain_the_token_limit() {
        assert_eq!(
            parse_tool_arguments("Test", "read", " ", false).unwrap(),
            serde_json::json!({})
        );
        let cut = parse_tool_arguments("Test", "read", "{\"path\":\"a", true).unwrap_err();
        assert!(cut.message.contains("token limit"));
        let bad = parse_tool_arguments("Test", "read", "{", false).unwrap_err();
        assert!(bad.message.contains("Failed to parse"));
    }
}
