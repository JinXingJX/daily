//! LLM-summary context compaction shared by in-run tool observations and
//! cross-turn plugin agent sessions.
//!
//! Selection and fallback truncation are pure so they can be tested without a
//! model; only [`summarize`] talks to a provider.

use crate::model::{Model, ModelCompletionRequest, ModelMessage, ModelToolChoice};
use crate::AgentToolObservation;
use serde_json::{json, Value};

const SUMMARY_SYSTEM_PROMPT: &str = r#"You compress the working history of an AI agent so it can continue the task with a smaller context.

Write a faithful, dense summary in the same language as the history. Preserve:
- the user's requests and constraints, and any material the user supplied (keep key wording, names, numbers);
- decisions made and the reasons for them;
- every tool call that changed state (what changed, where, identifiers), and key facts learned from read-only tools;
- open questions, unfinished steps and what should happen next.
Drop chit-chat, repeated reads and raw text that can be re-read with tools. Output only the summary."#;

/// Upper bound for the text handed to the summarizer in one call.
const MAX_SUMMARY_INPUT_CHARS: usize = 240_000;
const SUMMARY_MAX_OUTPUT_TOKENS: u32 = 4_096;
const SUMMARY_METADATA_KEY: &str = "compactedSummary";
/// Identifies a compaction request so hosts can keep it separate from user-turn correlation.
pub const SUMMARY_REQUEST_NAME: &str = "agent.context.compact";

/// Asks the model to fold `history` into `previous` (if any) and returns the new summary.
pub async fn summarize(
    adapter: &dyn Model,
    previous: Option<&str>,
    history: &str,
) -> Result<String, String> {
    let response = adapter
        .complete(&summary_request(previous, history), None)
        .await?;
    summary_text(response.raw_output)
}

fn summary_request(previous: Option<&str>, history: &str) -> ModelCompletionRequest {
    let mut prompt = String::new();
    if let Some(previous) = previous.filter(|value| !value.trim().is_empty()) {
        prompt.push_str("Existing summary of even earlier history:\n");
        prompt.push_str(previous.trim());
        prompt.push_str("\n\n");
    }
    prompt.push_str("History to fold into the summary:\n");
    prompt.push_str(&truncate_middle(history, MAX_SUMMARY_INPUT_CHARS));

    ModelCompletionRequest {
        purpose: crate::model::ModelRequestPurpose::ContextCompaction,
        messages: vec![
            ModelMessage::system(SUMMARY_SYSTEM_PROMPT),
            ModelMessage::user(prompt),
        ],
        temperature: 0.2,
        max_output_tokens: Some(SUMMARY_MAX_OUTPUT_TOKENS),
        tools: Vec::new(),
        tool_choice: ModelToolChoice::Auto,
        request_name: Some(SUMMARY_REQUEST_NAME.into()),
    }
}

fn summary_text(text: String) -> Result<String, String> {
    let summary = text.trim().to_string();
    if summary.is_empty() {
        return Err("The model returned an empty context summary.".into());
    }
    Ok(summary)
}

pub(crate) async fn summarize_in(
    execution: &crate::RunContext,
    adapter: &dyn Model,
    previous: Option<&str>,
    history: &str,
) -> Result<String, crate::AgentError> {
    let response = execution
        .complete(adapter, &summary_request(previous, history), None)
        .await?;
    summary_text(response.raw_output).map_err(crate::AgentError::Output)
}

/// Keeps the head and tail of `text`, replacing the middle with a marker.
pub fn truncate_middle(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(64) / 2;
    let head = text.chars().take(keep).collect::<String>();
    let tail = text.chars().skip(total - keep).collect::<String>();
    format!("{head}\n…[truncated {} chars]…\n{tail}", total - keep * 2)
}

/// A rough token count that needs no tokenizer: about four ASCII characters per token, and
/// one token per other character (CJK text runs close to that). Good enough to decide when a
/// context is getting large; providers report the exact numbers after each request.
pub fn estimate_tokens(text: &str) -> usize {
    let ascii = text.bytes().filter(u8::is_ascii).count();
    let other = text.chars().filter(|ch| !ch.is_ascii()).count();
    ascii.div_ceil(4) + other
}

pub fn observation_tokens(observation: &AgentToolObservation) -> usize {
    estimate_tokens(&observation.observation)
        + estimate_tokens(&observation.input.to_string())
        + observation
            .call
            .as_ref()
            .map(|call| estimate_tokens(&call.assistant_text))
            .unwrap_or(0)
}

pub fn observations_tokens(observations: &[AgentToolObservation]) -> usize {
    observations.iter().map(observation_tokens).sum()
}

/// Whether two neighbouring observations answer calls of the same model response.
fn same_round(left: &AgentToolObservation, right: &AgentToolObservation) -> bool {
    match (left.call.as_ref(), right.call.as_ref()) {
        (Some(left), Some(right)) => left.ply == right.ply,
        _ => false,
    }
}

/// Number of leading observations to fold into a summary, or `None` when the
/// list fits the budget or there is nothing old enough to fold. The split never
/// falls inside a tool round: a model response and all its results stay together.
pub fn observation_compaction_split(
    observations: &[AgentToolObservation],
    budget_tokens: usize,
    keep_recent: usize,
) -> Option<usize> {
    if observations_tokens(observations) <= budget_tokens {
        return None;
    }
    let mut split = observations.len().saturating_sub(keep_recent.max(1));
    while split > 0
        && split < observations.len()
        && same_round(&observations[split - 1], &observations[split])
    {
        split -= 1;
    }
    // Never fold only an existing summary into itself.
    if split == 0 || (split == 1 && is_summary_observation(&observations[0])) {
        split = 0;
    }
    (split > 0).then_some(split)
}

pub fn is_summary_observation(observation: &AgentToolObservation) -> bool {
    observation
        .metadata
        .get(SUMMARY_METADATA_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Renders observations as plain text for the summarizer.
pub fn render_observations_for_summary(observations: &[AgentToolObservation]) -> String {
    observations
        .iter()
        .enumerate()
        .map(|(index, observation)| {
            format!(
                "#{} {} ({})\nrationale: {}\ninput: {}\nresult:\n{}",
                index + 1,
                observation.tool_name,
                observation.tool_id,
                observation.rationale,
                observation.input,
                observation.observation
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Replaces the first `split` observations with one summary observation.
pub fn replace_with_summary(
    observations: &mut Vec<AgentToolObservation>,
    split: usize,
    summary: String,
) {
    let folded = split.min(observations.len());
    let mut message = AgentToolObservation::runtime_message(format!(
        "Summary of {folded} earlier tool observations (compacted to save context):\n{summary}"
    ));
    message.metadata = json!({ "source": "runtime", SUMMARY_METADATA_KEY: true, "folded": folded });
    observations.splice(0..folded, std::iter::once(message));
    // Reasoning the provider returned with the kept rounds was produced with the full history
    // in front of it. Providers that bind reasoning to its conversation reject it after the
    // history changed, so the kept rounds go back as plain calls and results.
    for observation in observations.iter_mut() {
        if let Some(call) = observation.call.as_mut() {
            call.provider_items.clear();
        }
    }
}

/// Fallback when the summarizer fails: shrink the oldest results until the list fits.
pub fn truncate_observations(
    observations: &mut [AgentToolObservation],
    budget_tokens: usize,
    keep_recent: usize,
) {
    let protected = observations.len().saturating_sub(keep_recent.max(1));
    for index in 0..protected {
        if observations_tokens(observations) <= budget_tokens {
            return;
        }
        let observation = &mut observations[index];
        observation.observation = truncate_middle(&observation.observation, 1_200);
        if observation.input.to_string().chars().count() > 1_200 {
            observation.input = json!({
                "truncated": truncate_middle(&observation.input.to_string(), 1_200),
            });
        }
        // Edited history: see `replace_with_summary`.
        if let Some(call) = observation.call.as_mut() {
            call.provider_items.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentToolCallRecord;

    /// An observation whose result is `tokens` estimated tokens long.
    fn observation(id: &str, tokens: usize) -> AgentToolObservation {
        AgentToolObservation {
            tool_id: id.into(),
            tool_name: id.into(),
            rationale: String::new(),
            input: Value::Null,
            pending_message: None,
            observation: "x".repeat(tokens * 4),
            metadata: Value::Null,
            call: None,
        }
    }

    fn in_round(
        mut observation: AgentToolObservation,
        ply: usize,
        index: usize,
    ) -> AgentToolObservation {
        observation.call = Some(AgentToolCallRecord {
            call_id: format!("call_{ply}_{index}"),
            ply,
            index,
            provider_items: vec![json!({ "type": "reasoning" })],
            ..AgentToolCallRecord::default()
        });
        observation
    }

    #[test]
    fn token_estimates_count_ascii_in_fours_and_other_characters_singly() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("你好世界"), 4);
        assert_eq!(estimate_tokens("你好 ok"), 3);
    }

    #[test]
    fn split_keeps_recent_observations_and_skips_lists_within_budget() {
        let list = vec![
            observation("a", 100),
            observation("b", 100),
            observation("c", 100),
        ];
        assert_eq!(observation_compaction_split(&list, 1_000, 2), None);
        assert_eq!(observation_compaction_split(&list, 200, 2), Some(1));
        assert_eq!(observation_compaction_split(&list, 200, 5), None);
    }

    #[test]
    fn split_never_falls_inside_a_tool_round() {
        let list = vec![
            in_round(observation("a", 100), 1, 0),
            in_round(observation("b", 100), 2, 0),
            in_round(observation("c", 100), 2, 1),
            in_round(observation("d", 100), 3, 0),
        ];
        // Keeping two would cut round 2 in half, so the whole round is kept.
        assert_eq!(observation_compaction_split(&list, 100, 2), Some(1));
        // A single oversized round cannot be folded.
        let one_round = vec![
            in_round(observation("a", 100), 1, 0),
            in_round(observation("b", 100), 1, 1),
        ];
        assert_eq!(observation_compaction_split(&one_round, 10, 1), None);
    }

    #[test]
    fn replace_with_summary_folds_prefix_and_is_not_refolded_alone() {
        let mut list = vec![
            observation("a", 500),
            observation("b", 500),
            in_round(observation("c", 10), 3, 0),
        ];
        replace_with_summary(&mut list, 2, "short".into());
        assert_eq!(list.len(), 2);
        assert!(is_summary_observation(&list[0]));
        assert_eq!(list[1].tool_id, "c");
        // Kept rounds lose their provider reasoning: it was bound to the folded history.
        assert!(list[1].call.as_ref().unwrap().provider_items.is_empty());
        // Only the summary is older than the kept tail, so nothing more to fold.
        let mut big = list.clone();
        big[1].observation = "y".repeat(5_000);
        assert_eq!(observation_compaction_split(&big, 100, 1), None);
    }

    #[test]
    fn truncation_fallback_shrinks_old_results_but_keeps_recent_verbatim() {
        let mut list = vec![observation("a", 10_000), observation("b", 10_000)];
        truncate_observations(&mut list, 5_000, 1);
        assert!(list[0].observation.contains("truncated"));
        assert_eq!(list[1].observation.len(), 40_000);
    }

    #[test]
    fn truncate_middle_keeps_head_and_tail() {
        let text = format!("{}{}", "a".repeat(500), "b".repeat(500));
        let out = truncate_middle(&text, 200);
        assert!(out.starts_with('a') && out.ends_with('b') && out.contains("truncated"));
        assert_eq!(truncate_middle("short", 200), "short");
    }
}
