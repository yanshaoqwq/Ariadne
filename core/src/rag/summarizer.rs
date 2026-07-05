use serde::Deserialize;
use serde_json::json;

use crate::contracts::{CoreError, CoreResult, RunId, SourceSpan, TextRange};
use crate::costs::CostLedger;
use crate::providers::{
    ContentPart, LlmMessage, LlmProvider, LlmRequest, LlmResponse, ProviderCallContext,
    ProviderExecutor,
};
use crate::rag::models::{StoryEvent, StoryEventStatus, StorySegment, SummaryPipelineDraft};
use crate::rag::resources::PromptResources;

/// Summarizer 四步执行配置。
#[derive(Debug, Clone)]
pub struct SummarizerConfig {
    pub provider_id: String,
    pub model_id: String,
    /// 章节正文所属文档 id，用于给故事段构造 source span（不复制正文）。
    pub chapter_document_id: String,
    pub run_id: Option<String>,
    pub timeout_ms: u64,
}

/// Summarizer 四步执行器：故事段划分并概括 → 事件归并 → 章节总结 → 阶段概括。
/// 每步独立 LLM 调用，前一步结构化结果喂给后一步；产出 SummaryPipelineDraft
/// 交给 SummaryPipelineExecutor::apply_draft 落库并建立四层确认项。
pub struct SummarizerExecutor<'a, L: CostLedger> {
    provider: &'a dyn LlmProvider,
    ledger: &'a L,
    prompts: &'a PromptResources,
    config: SummarizerConfig,
}

impl<'a, L: CostLedger> SummarizerExecutor<'a, L> {
    /// 创建执行器。
    pub fn new(
        provider: &'a dyn LlmProvider,
        ledger: &'a L,
        prompts: &'a PromptResources,
        config: SummarizerConfig,
    ) -> Self {
        Self {
            provider,
            ledger,
            prompts,
            config,
        }
    }

    /// 执行四步总结，返回可交给流水线的完整草稿。
    pub fn summarize_chapter(
        &self,
        chapter_id: &str,
        chapter_text: &str,
    ) -> CoreResult<SummaryPipelineDraft> {
        if chapter_id.trim().is_empty() {
            return Err(CoreError::validation("chapter_id cannot be empty"));
        }
        if chapter_text.trim().is_empty() {
            return Err(CoreError::validation("chapter_text cannot be empty"));
        }

        // 步骤 1：故事段划分并概括。
        let segments = self.summarize_segments(chapter_id, chapter_text)?;
        // 步骤 2：事件归并（喂入本章段概括 + 既有事件概括）。
        let events = self.merge_events(chapter_id, &segments)?;
        // 步骤 3：章节总结（喂入正文 + 事件变动）。
        let chapter_summary = self.summarize_chapter_text(chapter_text, &events)?;
        // 步骤 4：阶段概括（喂入本章总结）。
        let (stage_id, stage_summary) = self.summarize_stage(chapter_id, &chapter_summary)?;

        Ok(SummaryPipelineDraft {
            chapter_id: chapter_id.to_owned(),
            segments,
            events,
            chapter_summary: Some(chapter_summary),
            stage_id: Some(stage_id),
            stage_summary: Some(stage_summary),
            realized_changes: Vec::new(),
            foreshadowing_updates: Vec::new(),
            metadata: json!({ "generated_by": "summarizer" }),
        })
    }

    /// 步骤 1：把正文按“连续且属于同一事件”切分为故事段，编号并概括。
    fn summarize_segments(
        &self,
        chapter_id: &str,
        chapter_text: &str,
    ) -> CoreResult<Vec<StorySegment>> {
        let instruction = format!(
            "{}\n\n请只输出 JSON，格式：\
             {{\"segments\":[{{\"number\":\"1\",\"summary\":\"本段概括\",\
             \"start_line\":1,\"end_line\":20}}]}}。\
             number 为故事段编号（可含小数如 1.5），start_line/end_line 为正文行范围。\n\n正文：\n{}",
            self.prompt("summarizer.segments")?,
            chapter_text,
        );
        let response = self.call_llm(&instruction)?;
        let parsed: SegmentsDto = parse_json(&llm_text(&response))?;
        if parsed.segments.is_empty() {
            return Err(CoreError::validation(
                "summarizer produced no story segments",
            ));
        }
        Ok(parsed
            .segments
            .into_iter()
            .enumerate()
            .map(|(index, dto)| StorySegment {
                segment_id: format!("{chapter_id}::seg-{}", index + 1),
                number: dto.number,
                chapter_id: chapter_id.to_owned(),
                summary: dto.summary,
                source: SourceSpan {
                    document_id: self.config.chapter_document_id.clone(),
                    range: TextRange {
                        start: dto.start_line,
                        end: dto.end_line,
                    },
                    version: None,
                },
                metadata: json!({}),
            })
            .collect())
    }

    /// 步骤 2：把故事段归并为事件，维护事件状态。
    fn merge_events(
        &self,
        chapter_id: &str,
        segments: &[StorySegment],
    ) -> CoreResult<Vec<StoryEvent>> {
        let segment_digest = segments
            .iter()
            .map(|seg| format!("段 {}（{}）: {}", seg.number, seg.segment_id, seg.summary))
            .collect::<Vec<_>>()
            .join("\n");
        let instruction = format!(
            "{}\n\n本章故事段：\n{}\n\n请只输出 JSON，格式：\
             {{\"events\":[{{\"event_id\":\"event-1\",\"summary\":\"事件概括\",\
             \"status\":\"ongoing\",\"segment_ids\":[\"{}::seg-1\"]}}]}}。\
             status 取 ongoing/paused/completed。segment_ids 必须引用上面的段 id。",
            self.prompt("summarizer.events")?,
            segment_digest,
            chapter_id,
        );
        let response = self.call_llm(&instruction)?;
        let parsed: EventsDto = parse_json(&llm_text(&response))?;
        if parsed.events.is_empty() {
            return Err(CoreError::validation("summarizer produced no events"));
        }
        Ok(parsed
            .events
            .into_iter()
            .map(|dto| StoryEvent {
                event_id: dto.event_id,
                summary: dto.summary,
                status: parse_event_status(&dto.status),
                segment_ids: dto.segment_ids,
                chapter_ids: vec![chapter_id.to_owned()],
                metadata: json!({}),
            })
            .collect())
    }

    /// 步骤 3：总结本章，写出事件进展。
    fn summarize_chapter_text(
        &self,
        chapter_text: &str,
        events: &[StoryEvent],
    ) -> CoreResult<String> {
        let event_digest = events
            .iter()
            .map(|e| format!("- {}（{:?}）: {}", e.event_id, e.status, e.summary))
            .collect::<Vec<_>>()
            .join("\n");
        let instruction = format!(
            "{}\n\n本章事件变动：\n{}\n\n请只输出 JSON，格式：{{\"summary\":\"章节总结\"}}。\n\n正文：\n{}",
            self.prompt("summarizer.chapter_summary")?,
            event_digest,
            chapter_text,
        );
        let response = self.call_llm(&instruction)?;
        let parsed: ChapterSummaryDto = parse_json(&llm_text(&response))?;
        if parsed.summary.trim().is_empty() {
            return Err(CoreError::validation(
                "summarizer produced empty chapter summary",
            ));
        }
        Ok(parsed.summary)
    }

    /// 步骤 4：判断章节所属阶段并生成阶段概括。
    fn summarize_stage(
        &self,
        chapter_id: &str,
        chapter_summary: &str,
    ) -> CoreResult<(String, String)> {
        let instruction = format!(
            "{}\n\n本章 id：{}\n本章总结：{}\n\n请只输出 JSON，格式：\
             {{\"stage_id\":\"stage-1\",\"stage_summary\":\"阶段概括\",\"is_new_stage\":false}}。",
            self.prompt("summarizer.stage_summary")?,
            chapter_id,
            chapter_summary,
        );
        let response = self.call_llm(&instruction)?;
        let parsed: StageSummaryDto = parse_json(&llm_text(&response))?;
        if parsed.stage_id.trim().is_empty() || parsed.stage_summary.trim().is_empty() {
            return Err(CoreError::validation(
                "summarizer produced empty stage id or summary",
            ));
        }
        Ok((parsed.stage_id, parsed.stage_summary))
    }

    /// 取指定提示词本体。
    fn prompt(&self, key: &str) -> CoreResult<&str> {
        self.prompts
            .get(key)
            .map(|resource| resource.prompt.as_str())
            .ok_or_else(|| CoreError::validation(format!("missing prompt resource: {key}")))
    }

    /// 单次 LLM 调用，走 ProviderExecutor 以记录成本。
    fn call_llm(&self, instruction: &str) -> CoreResult<LlmResponse> {
        let executor = ProviderExecutor::new(self.ledger);
        executor.complete_llm(
            self.provider,
            &ProviderCallContext {
                provider_id: self.config.provider_id.clone(),
                workflow_id: None,
                run_id: self.config.run_id.clone().map(RunId::from),
                node_id: None,
                tool_call_id: None,
                timeout_ms: self.config.timeout_ms,
                max_retries: 0,
                metadata: json!({ "stage": "summarizer" }),
            },
            LlmRequest {
                model_id: self.config.model_id.clone(),
                messages: vec![LlmMessage::user(instruction.to_owned())],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: json!({}),
            },
        )
    }
}

/// 组装并落库：执行四步总结后交给流水线，返回流水线报告。
/// 这是生产链的顶层入口：Summarizer 节点执行 → 解析 → apply_draft 落库建索引。
pub fn run_and_apply<L: CostLedger>(
    executor: &SummarizerExecutor<'_, L>,
    pipeline: &crate::rag::pipeline::SummaryPipelineExecutor<'_>,
    chapter_id: &str,
    chapter_text: &str,
) -> CoreResult<crate::rag::models::SummaryPipelineReport> {
    let draft = executor.summarize_chapter(chapter_id, chapter_text)?;
    pipeline.apply_draft(draft)
}

// ── LLM 输出 DTO ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SegmentsDto {
    segments: Vec<SegmentDto>,
}
#[derive(Debug, Deserialize)]
struct SegmentDto {
    number: String,
    summary: String,
    #[serde(default)]
    start_line: u64,
    #[serde(default)]
    end_line: u64,
}

#[derive(Debug, Deserialize)]
struct EventsDto {
    events: Vec<EventDto>,
}
#[derive(Debug, Deserialize)]
struct EventDto {
    event_id: String,
    summary: String,
    #[serde(default = "default_status")]
    status: String,
    #[serde(default)]
    segment_ids: Vec<String>,
}
fn default_status() -> String {
    "ongoing".to_owned()
}

#[derive(Debug, Deserialize)]
struct ChapterSummaryDto {
    summary: String,
}

#[derive(Debug, Deserialize)]
struct StageSummaryDto {
    stage_id: String,
    stage_summary: String,
    #[serde(default)]
    #[allow(dead_code)]
    is_new_stage: bool,
}

// ── 辅助函数 ─────────────────────────────────────────────────────────────────

fn parse_event_status(s: &str) -> StoryEventStatus {
    match s.trim().to_ascii_lowercase().as_str() {
        "paused" => StoryEventStatus::Paused,
        "completed" => StoryEventStatus::Completed,
        _ => StoryEventStatus::Ongoing,
    }
}

/// 提取 LLM 响应文本。
fn llm_text(response: &LlmResponse) -> String {
    response
        .message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// 从可能包裹在代码块或前后噪声中的文本里解析 JSON。
/// 解析失败归类为可重试的参数错误，交给 runtime 重试机制。
fn parse_json<T: for<'de> Deserialize<'de>>(text: &str) -> CoreResult<T> {
    let trimmed = extract_json_object(text);
    serde_json::from_str::<T>(trimmed)
        .map_err(|e| CoreError::validation(format!("failed to parse summarizer JSON output: {e}")))
}

/// 从文本中截取第一个 `{` 到最后一个 `}` 的片段，容忍 ```json 包裹。
fn extract_json_object(text: &str) -> &str {
    let start = text.find('{');
    let end = text.rfind('}');
    match (start, end) {
        (Some(s), Some(e)) if e >= s => &text[s..=e],
        _ => text,
    }
}
