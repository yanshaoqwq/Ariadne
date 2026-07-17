use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult};

use super::project_ai::{ProjectAiMemoryEntry, ProjectAiSummaryChunk};
use super::service::ProjectReference;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectAiChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAiChatMessage {
    pub role: ProjectAiChatRole,
    pub content: String,
}

/// Project AI 输入预算的唯一结果对象；command 层只消费已裁剪的上下文。
pub struct ProjectAiContextWindow {
    pub memory: String,
    pub conversation_summary: String,
    pub reference_context: String,
    pub history: Vec<ProjectAiChatMessage>,
    pub history_truncated: bool,
    pub memory_truncated: bool,
    pub references_truncated: bool,
    pub summary_truncated: bool,
    pub estimated_input_tokens: u64,
    pub context_limit_tokens: u32,
}

pub fn project_ai_context_window(
    project_memory: &[ProjectAiMemoryEntry],
    conversation_summaries: &[ProjectAiSummaryChunk],
    references: &[ProjectReference],
    chat_history: &[ProjectAiChatMessage],
    message: &str,
    configured_context_tokens: Option<u32>,
    configured_output_tokens: Option<u32>,
) -> CoreResult<ProjectAiContextWindow> {
    const DEFAULT_CONTEXT_TOKENS: u32 = 16_384;
    const SYSTEM_AND_TOOL_RESERVE_CHARS: usize = 4_000;
    let context_limit_tokens = configured_context_tokens.unwrap_or(DEFAULT_CONTEXT_TOKENS);
    let output_reserve_tokens =
        configured_output_tokens.unwrap_or_else(|| (context_limit_tokens / 4).clamp(256, 4_096));
    if context_limit_tokens <= output_reserve_tokens.saturating_add(1_024) {
        return Err(CoreError::validation(format!(
            "project AI context limit {context_limit_tokens} leaves insufficient input space after output reserve {output_reserve_tokens}"
        )));
    }
    let input_token_budget = context_limit_tokens - output_reserve_tokens;
    let input_char_budget = usize::try_from(input_token_budget)
        .unwrap_or(usize::MAX / 4)
        .saturating_mul(4);
    let message_chars = message.trim().chars().count();
    if message_chars.saturating_add(SYSTEM_AND_TOOL_RESERVE_CHARS) > input_char_budget {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "project_ai_context".to_owned(),
            reason: format!(
                "message is too large; approximately {input_token_budget} input tokens available"
            ),
        });
    }

    let available = input_char_budget
        .saturating_sub(SYSTEM_AND_TOOL_RESERVE_CHARS)
        .saturating_sub(message_chars);
    let memory_budget = available / 5;
    let summary_budget = available / 5;
    let reference_budget = available / 3;
    let history_budget = available
        .saturating_sub(memory_budget)
        .saturating_sub(summary_budget)
        .saturating_sub(reference_budget);

    let (memory, memory_truncated) =
        structured_project_memory_context(project_memory, memory_budget);
    let (conversation_summary, summary_truncated) =
        project_ai_summary_context(conversation_summaries, summary_budget);
    let (reference_context, references_truncated) =
        project_reference_context(references, reference_budget);

    let mut selected_reversed = Vec::new();
    let mut used_history_chars = 0usize;
    for history in chat_history.iter().rev() {
        let cost = history.content.chars().count().saturating_add(16);
        if used_history_chars.saturating_add(cost) > history_budget {
            break;
        }
        used_history_chars = used_history_chars.saturating_add(cost);
        selected_reversed.push(history.clone());
    }
    selected_reversed.reverse();
    if selected_reversed
        .first()
        .is_some_and(|entry| entry.role == ProjectAiChatRole::Assistant)
    {
        let first_non_assistant = selected_reversed
            .iter()
            .position(|entry| entry.role != ProjectAiChatRole::Assistant)
            .unwrap_or(selected_reversed.len());
        selected_reversed.drain(..first_non_assistant);
    }
    let history_truncated = selected_reversed.len() < chat_history.len();
    let estimated_chars = SYSTEM_AND_TOOL_RESERVE_CHARS
        .saturating_add(message_chars)
        .saturating_add(memory.chars().count())
        .saturating_add(conversation_summary.chars().count())
        .saturating_add(reference_context.chars().count())
        .saturating_add(
            selected_reversed
                .iter()
                .map(|entry| entry.content.chars().count().saturating_add(16))
                .sum::<usize>(),
        );
    Ok(ProjectAiContextWindow {
        memory,
        conversation_summary,
        reference_context,
        history: selected_reversed,
        history_truncated,
        memory_truncated,
        references_truncated,
        summary_truncated,
        estimated_input_tokens: u64::try_from(estimated_chars.div_ceil(4)).unwrap_or(u64::MAX),
        context_limit_tokens,
    })
}

fn project_reference_context(
    references: &[ProjectReference],
    char_budget: usize,
) -> (String, bool) {
    let mut output = String::new();
    let mut truncated = false;
    for reference in references {
        let header = format!(
            "- {} [{}]: {}\n",
            reference.reference, reference.id, reference.summary
        );
        if !append_complete_context_piece(&mut output, &header, char_budget) {
            truncated = true;
            break;
        }

        if let Some(fragments) = reference.payload.get("fragments").and_then(Value::as_array) {
            for fragment in fragments {
                let source_id = fragment
                    .get("source_id")
                    .and_then(Value::as_str)
                    .unwrap_or(reference.id.as_str());
                let version = fragment
                    .get("source_version")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let start_line = fragment
                    .get("start_line")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let end_line = fragment
                    .get("end_line")
                    .and_then(Value::as_u64)
                    .unwrap_or(start_line);
                let prefix = format!(
                    "  fragment source={source_id} version={version} lines={start_line}-{end_line}\n"
                );
                if !append_complete_context_piece(&mut output, &prefix, char_budget) {
                    truncated = true;
                    break;
                }
                let text = fragment.get("text").and_then(Value::as_str).unwrap_or("");
                if !append_text_context_piece(&mut output, text, char_budget) {
                    truncated = true;
                    break;
                }
                if !append_complete_context_piece(&mut output, "\n", char_budget) {
                    truncated = true;
                    break;
                }
            }
        } else if let Some(text) = reference.payload.get("text").and_then(Value::as_str) {
            let provenance = json!({
                "revision": reference.payload.get("revision"),
                "layer": reference.payload.get("layer"),
                "sources": reference.payload.get("sources"),
            });
            let prefix = format!("  provenance: {provenance}\n  text:\n");
            if !append_complete_context_piece(&mut output, &prefix, char_budget)
                || !append_text_context_piece(&mut output, text, char_budget)
            {
                truncated = true;
                break;
            }
            if !append_complete_context_piece(&mut output, "\n", char_budget) {
                truncated = true;
                break;
            }
        } else {
            let payload = format!("  payload: {}\n", reference.payload);
            if !append_complete_context_piece(&mut output, &payload, char_budget) {
                truncated = true;
                break;
            }
        }
        if reference
            .payload
            .get("content_truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            truncated = true;
        }
    }
    (output, truncated)
}

pub fn structured_project_memory_context(
    entries: &[ProjectAiMemoryEntry],
    char_budget: usize,
) -> (String, bool) {
    let mut output = String::new();
    let mut truncated = false;
    for entry in entries {
        let header = format!(
            "- entity_id={} key={} source={} revision={} source_line={}\n  ",
            entry.entity_id,
            entry.logical_key,
            entry.source,
            entry.source_version,
            entry.source_line
        );
        if !append_complete_context_piece(&mut output, &header, char_budget) {
            truncated = true;
            break;
        }
        if !append_text_context_piece(&mut output, &entry.value, char_budget) {
            truncated = true;
            break;
        }
        if !append_complete_context_piece(&mut output, "\n", char_budget) {
            truncated = true;
            break;
        }
    }
    (output, truncated)
}

pub fn project_ai_summary_context(
    summaries: &[ProjectAiSummaryChunk],
    char_budget: usize,
) -> (String, bool) {
    let mut output = String::new();
    let mut truncated = false;
    for summary in summaries {
        let header = format!(
            "- summary_id={} revisions={}-{} sequences={}-{}\n",
            summary.summary_id,
            summary.from_revision,
            summary.to_revision,
            summary.from_sequence,
            summary.to_sequence
        );
        if !append_complete_context_piece(&mut output, &header, char_budget) {
            truncated = true;
            break;
        }
        if !append_text_context_piece(&mut output, &summary.text, char_budget) {
            truncated = true;
            break;
        }
    }
    (output, truncated)
}

fn append_complete_context_piece(output: &mut String, value: &str, budget: usize) -> bool {
    if output.chars().count().saturating_add(value.chars().count()) > budget {
        return false;
    }
    output.push_str(value);
    true
}

fn append_text_context_piece(output: &mut String, value: &str, budget: usize) -> bool {
    let used = output.chars().count();
    let remaining = budget.saturating_sub(used);
    let selected = value.chars().take(remaining).collect::<String>();
    let complete = selected.chars().count() == value.chars().count();
    output.push_str(&selected);
    complete
}
