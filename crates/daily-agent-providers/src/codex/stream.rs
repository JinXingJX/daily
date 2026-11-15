//! Parsing of Codex Responses API stream events and payloads.

use super::*;

pub(super) fn codex_text_from_stream_event(
    event: &Value,
    allow_fallback: bool,
) -> Result<String, String> {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "response.output_text.delta" => Ok(event
            .get("delta")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()),
        "response.output_item.done" if allow_fallback => Ok(event
            .get("item")
            .map(extract_response_text)
            .unwrap_or_default()),
        "response.completed" if allow_fallback => Ok(event
            .get("response")
            .map(extract_response_text)
            .unwrap_or_default()),
        "response.failed" | "response.incomplete" | "error" => Err(format!(
            "Codex stream failed: {}",
            extract_codex_error_message(event).unwrap_or_else(|| event.to_string())
        )),
        _ => Ok(String::new()),
    }
}

pub(super) fn codex_tool_calls_from_stream_event(
    event: &Value,
    tool_name_map: &std::collections::HashMap<String, String>,
) -> Result<Vec<ModelToolCall>, String> {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "response.output_item.done" => event
            .get("item")
            .map(|item| extract_response_tool_calls(item, tool_name_map))
            .unwrap_or_else(|| Ok(Vec::new())),
        "response.completed" => event
            .get("response")
            .map(|response| extract_response_tool_calls(response, tool_name_map))
            .unwrap_or_else(|| Ok(Vec::new())),
        _ => Ok(Vec::new()),
    }
}

pub(super) fn extract_codex_error_message(value: &Value) -> Option<String> {
    value
        .pointer("/response/error/message")
        .or_else(|| value.pointer("/error/message"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .pointer("/response/incomplete_details/reason")
                .and_then(Value::as_str)
                .map(|reason| format!("incomplete response: {reason}"))
        })
}

pub(super) fn extract_response_text(value: &Value) -> String {
    if let Some(text) = value.get("output_text").and_then(Value::as_str) {
        return text.to_string();
    }

    if let Some(output) = value.get("output").and_then(Value::as_array) {
        return output
            .iter()
            .map(extract_response_item_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
    }

    extract_response_item_text(value)
}

pub(super) fn extract_response_tool_calls(
    value: &Value,
    tool_name_map: &std::collections::HashMap<String, String>,
) -> Result<Vec<ModelToolCall>, String> {
    if let Some(output) = value.get("output").and_then(Value::as_array) {
        let mut calls = Vec::new();
        for item in output {
            calls.extend(extract_response_tool_calls(item, tool_name_map)?);
        }
        return Ok(calls);
    }

    if value.get("type").and_then(Value::as_str) != Some("function_call") {
        return Ok(Vec::new());
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if name.is_empty() {
        return Ok(Vec::new());
    }
    let raw_arguments = value
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let arguments = if raw_arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(raw_arguments).map_err(|error| {
            format!("Failed to parse Codex tool call arguments for {name}: {error}")
        })?
    };
    Ok(vec![ModelToolCall {
        call_id: value
            .get("call_id")
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        tool_id: tool_name_map.get(&name).cloned().unwrap_or(name),
        arguments,
        provider_meta: Value::Null,
    }])
}

pub(super) fn append_unique_tool_calls(target: &mut Vec<ModelToolCall>, calls: Vec<ModelToolCall>) {
    for call in calls {
        let duplicate = target.iter().any(|existing| {
            existing.call_id == call.call_id
                && existing.tool_id == call.tool_id
                && existing.arguments == call.arguments
        });
        if !duplicate {
            target.push(call);
        }
    }
}

pub(super) fn extract_response_item_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Keeps the reasoning items of a response so the next request of the tool round can send them
/// back. Requests are not stored server-side, so an item is only usable with its encrypted
/// content, and its id (a reference to stored state) is left out.
pub(super) fn collect_reasoning_items(target: &mut Vec<Value>, event: &Value) {
    if event.get("type").and_then(Value::as_str) != Some("response.output_item.done") {
        return;
    }
    let Some(item) = event.get("item") else {
        return;
    };
    if item.get("type").and_then(Value::as_str) != Some("reasoning") {
        return;
    }
    let Some(encrypted) = item
        .get("encrypted_content")
        .filter(|value| value.is_string())
    else {
        return;
    };
    target.push(json!({
        "type": "reasoning",
        "summary": item.get("summary").cloned().unwrap_or_else(|| json!([])),
        "encrypted_content": encrypted,
    }));
}

pub(super) fn codex_usage_from_stream_event(event: &Value) -> Option<ModelUsage> {
    if event.get("type").and_then(Value::as_str) != Some("response.completed") {
        return None;
    }
    let usage = event.pointer("/response/usage")?;
    let count = |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64);
    Some(ModelUsage {
        input_tokens: count("/input_tokens")?,
        output_tokens: count("/output_tokens").unwrap_or(0),
        cached_input_tokens: count("/input_tokens_details/cached_tokens").unwrap_or(0),
        reasoning_tokens: count("/output_tokens_details/reasoning_tokens").unwrap_or(0),
    })
}
