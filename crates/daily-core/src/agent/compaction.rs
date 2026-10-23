//! LLM-summary context compaction shared by in-run tool observations and
//! cross-turn plugin agent sessions.
//!
//! Selection and fallback truncation are pure so they can be tested without a
//! model; only [`summarize`] talks to a provider.

use crate::agent::sdk::AgentToolObservation;
use crate::model::{
    adapter_for, ModelCompletionRequest, ModelConfig, ModelMessage, ModelMessageRole,
};
use reqwest::Client;
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

/// Asks the model to fold `history` into `previous` (if any) and returns the new summary.
pub(crate) async fn summarize(
    http: &Client,
    model: &ModelConfig,
    previous: Option<&str>,
    history: &str,
) -> Result<String, String> {
    let mut prompt = String::new();
    if let Some(previous) = previous.filter(|value| !value.trim().is_empty()) {
        prompt.push_str("Existing summary of even earlier history:\n");
        prompt.push_str(previous.trim());
        prompt.push_str("\n\n");
    }
    prompt.push_str("History to fold into the summary:\n");
    prompt.push_str(&truncate_middle(history, MAX_SUMMARY_INPUT_CHARS));

    let adapter = adapter_for(model.provider_kind);
    let response = adapter
        .complete(
            http,
            model,
            &ModelCompletionRequest {
                messages: vec![
                    ModelMessage {
                        role: ModelMessageRole::System,
                        content: SUMMARY_SYSTEM_PROMPT.into(),
                    },
                    ModelMessage {
                        role: ModelMessageRole::User,
                        content: prompt,
                    },
                ],
                temperature: 0.2,
                max_output_tokens: Some(SUMMARY_MAX_OUTPUT_TOKENS),
                inline_images: Vec::new(),
                tools: Vec::new(),
                session_id: None,
                turn_id: None,
                request_name: Some("agent.context.compact".into()),
            },
            None,
        )
        .await?;
    let summary = response.raw_output.trim().to_string();
    if summary.is_empty() {
        return Err("The model returned an empty context summary.".into());
    }
    Ok(summary)
}

/// Keeps the head and tail of `text`, replacing the middle with a marker.
pub(crate) fn truncate_middle(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(64) / 2;
    let head = text.chars().take(keep).collect::<String>();
    let tail = text.chars().skip(total - keep).collect::<String>();
    format!("{head}\n…[truncated {} chars]…\n{tail}", total - keep * 2)
}

pub(crate) fn observation_chars(observation: &AgentToolObservation) -> usize {
    observation.observation.chars().count()
        + observation.rationale.chars().count()
        + observation.input.to_string().chars().count()
        + observation
            .pending_message
            .as_deref()
            .map(|value| value.chars().count())
            .unwrap_or(0)
}

pub(crate) fn observations_chars(observations: &[AgentToolObservation]) -> usize {
    observations.iter().map(observation_chars).sum()
}

/// Number of leading observations to fold into a summary, or `None` when the
/// list fits the budget or there is nothing old enough to fold.
pub(crate) fn observation_compaction_split(
    observations: &[AgentToolObservation],
    budget_chars: usize,
    keep_recent: usize,
) -> Option<usize> {
    if observations_chars(observations) <= budget_chars {
        return None;
    }
    let mut split = observations.len().saturating_sub(keep_recent.max(1));
    // Never fold only an existing summary into itself.
    if split == 0 || (split == 1 && is_summary_observation(&observations[0])) {
        split = 0;
    }
    (split > 0).then_some(split)
}

pub(crate) fn is_summary_observation(observation: &AgentToolObservation) -> bool {
    observation
        .metadata
        .get(SUMMARY_METADATA_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Renders observations as plain text for the summarizer.
pub(crate) fn render_observations_for_summary(observations: &[AgentToolObservation]) -> String {
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
pub(crate) fn replace_with_summary(
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
}

/// Fallback when the summarizer fails: shrink the oldest results until the list fits.
pub(crate) fn truncate_observations(
    observations: &mut [AgentToolObservation],
    budget_chars: usize,
    keep_recent: usize,
) {
    let protected = observations.len().saturating_sub(keep_recent.max(1));
    for index in 0..protected {
        if observations_chars(observations) <= budget_chars {
            return;
        }
        let observation = &mut observations[index];
        observation.observation = truncate_middle(&observation.observation, 1_200);
        if observation.input.to_string().chars().count() > 1_200 {
            observation.input =
                Value::String(truncate_middle(&observation.input.to_string(), 1_200));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(id: &str, size: usize) -> AgentToolObservation {
        AgentToolObservation {
            tool_id: id.into(),
            tool_name: id.into(),
            rationale: String::new(),
            input: Value::Null,
            pending_message: None,
            observation: "x".repeat(size),
            metadata: Value::Null,
        }
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
    fn replace_with_summary_folds_prefix_and_is_not_refolded_alone() {
        let mut list = vec![
            observation("a", 500),
            observation("b", 500),
            observation("c", 10),
        ];
        replace_with_summary(&mut list, 2, "short".into());
        assert_eq!(list.len(), 2);
        assert!(is_summary_observation(&list[0]));
        assert_eq!(list[1].tool_id, "c");
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
        assert_eq!(list[1].observation.len(), 10_000);
    }

    #[test]
    fn truncate_middle_keeps_head_and_tail() {
        let text = format!("{}{}", "a".repeat(500), "b".repeat(500));
        let out = truncate_middle(&text, 200);
        assert!(out.starts_with('a') && out.ends_with('b') && out.contains("truncated"));
        assert_eq!(truncate_middle("short", 200), "short");
    }
}
