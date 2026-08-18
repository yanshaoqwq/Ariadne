use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::contracts::{
    content_version_for_bytes, CoreError, CoreResult, ExecutionCancellation,
    ExternalDispatchAuthorization, ExternalDispatchOutcome, NodeId, PermissionPolicy, RunId,
    SourceSpan, WorkflowId,
};
use crate::costs::CostLedger;
use crate::llm::{tool_result_message, ToolExecutionContext, ToolExecutor, ToolExecutorRouter};
use crate::providers::{
    ContentPart, LlmMessage, LlmProvider, LlmRequest, LlmResponse, ProviderCallContext,
    ProviderExecutor, SearchProvider, ToolDefinition, WebSearchToolExecutor,
};
use crate::rag::line_patch::line_range_to_text_range;
use crate::rag::models::{
    ConfirmationAuditDecision, ConfirmationKind, ForeshadowingStatus, ForeshadowingUpdate,
    RealizedChangeLink, RegisteredChangeStatus, StoryEvent, StoryEventStatus, StorySegment,
    SummaryGenerationContext, SummaryPipelineDraft,
};
use crate::rag::resources::PromptResources;
use crate::rag::store::{SqliteWritingKnowledgeStore, SummarizerStagePreparation};
use crate::retrieval::{ProjectRetrievalRuntime, ProjectSearchToolExecutor};
use crate::skills::stable_text_hash;

/// workflow 父 operation 传给四步总结器的持久化身份。
#[derive(Debug, Clone)]
pub struct SummarizerWorkflowOperationContext {
    pub project_root: PathBuf,
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub node_id: NodeId,
    pub operation_id: String,
    pub operation_attempt: u32,
    pub request_hash: String,
}

/// Summarizer 四步执行配置。
#[derive(Debug, Clone)]
pub struct SummarizerConfig {
    pub provider_id: String,
    pub model_id: String,
    /// 章节正文所属文档 id，用于给故事段构造 source span（不复制正文）。
    pub chapter_document_id: String,
    pub run_id: Option<String>,
    pub timeout_ms: u64,
    pub cancellation: ExecutionCancellation,
    pub dispatch_authorization: ExternalDispatchAuthorization,
    /// F24：作者/节点可编辑 prompt 或 template；非空时并入每步 LLM 指令。
    pub prompt_template: Option<String>,
    /// F15/F18：在 dispatch 前由持久化层严格加载的历史知识投影。
    pub generation_context: SummaryGenerationContext,
    /// F11：存在时四步 LLM 使用 SQLite 子 operation journal；缺失表示独立调用。
    pub workflow_operation: Option<SummarizerWorkflowOperationContext>,
}

/// Summarizer 四步执行器：故事段划分并概括 → 事件归并 → 章节总结 → 阶段概括。
/// 每步独立 LLM 调用，前一步结构化结果喂给后一步；产出 SummaryPipelineDraft
/// 交给 SummaryPipelineExecutor::apply_draft 落库并建立四层确认项。
pub struct SummarizerExecutor<'a, L: CostLedger> {
    provider: &'a dyn LlmProvider,
    ledger: &'a L,
    prompts: &'a PromptResources,
    config: SummarizerConfig,
    project_search: Option<SummarizerProjectSearch<'a>>,
    web_search: Option<SummarizerWebSearch<'a>>,
}

struct SummarizerProjectSearch<'a> {
    runtime: &'a ProjectRetrievalRuntime,
    tool: ToolDefinition,
    max_tool_rounds: u32,
}

struct SummarizerWebSearch<'a> {
    provider: &'a dyn SearchProvider,
    policy: &'a PermissionPolicy,
    tool: ToolDefinition,
    max_tool_rounds: u32,
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
            project_search: None,
            web_search: None,
        }
    }

    /// 为四步总结器启用项目级 Search tool。
    pub fn with_project_search(
        mut self,
        runtime: &'a ProjectRetrievalRuntime,
        tool: ToolDefinition,
        max_tool_rounds: u32,
    ) -> Self {
        self.project_search = Some(SummarizerProjectSearch {
            runtime,
            tool,
            max_tool_rounds,
        });
        self
    }

    /// 为四步总结器启用外部 Web Search tool。
    pub fn with_web_search(
        mut self,
        provider: &'a dyn SearchProvider,
        policy: &'a PermissionPolicy,
        tool: ToolDefinition,
        max_tool_rounds: u32,
    ) -> Self {
        self.web_search = Some(SummarizerWebSearch {
            provider,
            policy,
            tool,
            max_tool_rounds,
        });
        self
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
        let stage_store = self
            .config
            .workflow_operation
            .as_ref()
            .map(|operation| SqliteWritingKnowledgeStore::open(&operation.project_root))
            .transpose()?;

        let context = &self.config.generation_context;

        // 步骤 1：故事段划分并概括。
        let segments = self.summarize_segments(stage_store.as_ref(), chapter_id, chapter_text)?;
        // 步骤 2：事件归并（喂入本章段概括 + 既有事件概括）。
        let events =
            self.merge_events(stage_store.as_ref(), chapter_id, chapter_text, &segments, context)?;
        // 步骤 3：章节总结（喂入正文、事件对比、计划变化和伏笔）。
        let chapter = self.summarize_chapter_text(
            stage_store.as_ref(),
            chapter_text,
            &segments,
            &events,
            context,
        )?;
        // 步骤 4：阶段概括（喂入当前阶段全部章节总结 + 本章正文/总结）。
        let (stage_id, stage_summary, is_new_stage) = self.summarize_stage(
            stage_store.as_ref(),
            chapter_id,
            chapter_text,
            &chapter.summary,
            context,
        )?;

        Ok(SummaryPipelineDraft {
            chapter_id: chapter_id.to_owned(),
            segments,
            events,
            chapter_summary: Some(chapter.summary),
            stage_id: Some(stage_id),
            stage_summary: Some(stage_summary),
            is_new_stage: Some(is_new_stage),
            realized_changes: chapter.realized_changes,
            foreshadowing_updates: chapter.foreshadowing_updates,
            metadata: json!({
                "generated_by": "summarizer",
                "source_version": content_version_for_bytes(chapter_text.as_bytes()),
                "existing_event_count": context.existing_events.len(),
            }),
        })
    }

    /// 使用当前节点的 provider/model 执行单项自动审批，并返回结构化决定。
    ///
    /// U114 遗漏路径（2026-08-18 修）：审批提示词此前是 `format!` 原文直传，
    /// **不过 `render_prompt_template`**。作者在设置页的审批提示词框里写
    /// `{{本章大纲}}`，审阅模型收到的就是字面量 `{{本章大纲}}`——
    /// 它在「审的是占位符」的前提下给出通过判定，是安全缺口而非文案瑕疵
    /// （与 13-B 那条「`{{ref:...}}` 字面量进请求体」同源）。
    ///
    /// 两处刻意的取舍：
    /// - **不复用 `WritingContextAssembler`**。装配器要 `MemoryWritingKnowledgeBase`，
    ///   而这里在写锁之外、手上只有 `SummaryGenerationContext` 与 `draft`
    ///   （见 `integration.rs` 的调用点：`load_summary_working_set` 要等到取写锁后
    ///   才带 draft 重新读）。硬凑一个知识库等于把长耗时 LLM 调用挪进写锁内。
    ///   所以这里自建变量表，别名沿用 `known_section_aliases` 的同一套中文名——
    ///   **作者在两个框里写同一个 `{{本章大纲}}` 必须指同一件东西**。
    /// - **变量按确认类型分别给**（报告验收第 3 条）。`StageSummary` 要阶段总纲，
    ///   `SegmentSummary` 要本章正文与注册项；无差别塞全部上下文会撑爆审阅调用的
    ///   token，且让模型在噪声里找不到该对照的那一条。
    pub fn audit_confirmation(
        &self,
        kind: ConfirmationKind,
        approval_prompt: &str,
        chapter_text: &str,
        draft: &SummaryPipelineDraft,
    ) -> CoreResult<ConfirmationAuditDecision> {
        if approval_prompt.trim().is_empty() {
            return Err(CoreError::validation(format!(
                "Auto Mode approval prompt cannot be empty for {kind:?}"
            )));
        }
        let stage_store = self
            .config
            .workflow_operation
            .as_ref()
            .map(|operation| SqliteWritingKnowledgeStore::open(&operation.project_root))
            .transpose()?;
        let candidate = audit_candidate(kind, chapter_text, draft)?;
        let approval_prompt = self.render_approval_prompt(kind, approval_prompt, chapter_text)?;
        let instruction = format!(
            "{}\n\n待审确认类型：{}\n\n候选内容：\n{}\n\n只输出 JSON：\
             {{\"approved\":true,\"reason\":\"明确说明通过或拒绝的依据\"}}。\
             approved 必须是布尔值；存在冲突、信息不足或不安全副作用时必须返回 false。",
            approval_prompt.trim(),
            confirmation_kind_name(kind),
            candidate,
        );
        let response = self.call_llm(
            stage_store.as_ref(),
            &format!("auto_audit_{}", confirmation_kind_name(kind)),
            &instruction,
        )?;
        let parsed: ConfirmationAuditDto = parse_json(&llm_text(&response))?;
        if parsed.reason.trim().is_empty() {
            return Err(CoreError::validation(format!(
                "Auto Mode audit returned an empty reason for {kind:?}"
            )));
        }
        Ok(ConfirmationAuditDecision {
            approved: parsed.approved,
            reason: parsed.reason.trim().to_owned(),
        })
    }

    /// 把审批提示词过一遍模板渲染器，并按确认类型灌入对应的规划依据。
    ///
    /// **不含 `{{` 的提示词原样返回**——内置那 10 条审批提示词都不带占位符，
    /// 不能让它们白跑一趟装配（也不能因为「审阅上下文凑不齐」而把它们判失败）。
    ///
    /// 未知变量走 `render_prompt_template` 的 fail-loud：作者把
    /// `{{本章大纲}}` 拼错成 `{{章节大纲}}` 时当场报错指名变量，
    /// 而不是把字面量发给审阅模型换回一个虚假通过。
    fn render_approval_prompt(
        &self,
        kind: ConfirmationKind,
        approval_prompt: &str,
        chapter_text: &str,
    ) -> CoreResult<String> {
        let approval_prompt = approval_prompt.trim();
        if !approval_prompt.contains("{{") {
            return Ok(approval_prompt.to_owned());
        }
        let mut context = crate::rag::prompt_template::PromptTemplateContext {
            // 审批提示词本身就是「角色设定」那一格的内容：审阅者的身份由作者
            // 在这个框里定，没有另一份 agent 提示词可填。指向自身会造成
            // `{{角色设定}}` 自引用，但渲染器只扫模板不扫代入值（见
            // `prompt_template.rs` 的说明），不会无限展开。
            prompt_text: approval_prompt.to_owned(),
            ..Default::default()
        };
        for (alias, content) in self.audit_context_sections(kind, chapter_text) {
            // 每个区块登记全部中文别名：作者在审批框和节点提示词框里写同一个
            // `{{本章大纲}}`，必须指向同一件东西，否则「两个框的变量名不一样」
            // 会成为一类永远查不出的困惑。
            context.inputs.insert(alias.to_owned(), content);
        }
        crate::rag::prompt_template::render_prompt_template(approval_prompt, &context)
    }

    /// 按确认类型给出可供对照的规划依据（别名 → 正文）。
    ///
    /// 报告验收第 3 条要求「不同 `ConfirmationKind` 拿到的上下文不同」：
    /// 审阅一份**阶段**总结要看阶段总纲与该阶段已有章节总结；审阅**故事段**
    /// 划分要看本章正文与本章待核对的注册项。全都塞一遍反而降低判准。
    ///
    /// 数据只取 `SummaryGenerationContext`（dispatch 前由持久化层严格加载，
    /// 见 `SummarizerConfig::generation_context` 的 F15/F18 注释）——
    /// 不在这里读盘：审阅发生在写锁之外，读盘会让它和主链路的快照漂移。
    fn audit_context_sections(
        &self,
        kind: ConfirmationKind,
        chapter_text: &str,
    ) -> Vec<(&'static str, String)> {
        let context = &self.config.generation_context;
        let mut sections: Vec<(&'static str, String)> = Vec::new();
        let mut push = |aliases: &[&'static str], content: String| {
            for alias in aliases {
                sections.push((alias, content.clone()));
            }
        };

        // 本章正文对每一类都是对照基准：判「总结写的是不是正文里确实发生的」
        // 离不开正文本身（内置 `auto_audit.summary` 提示词正是这么要求的）。
        push(&["chapter_text", "当前章节正文"], chapter_text.to_owned());

        // 未回收伏笔：审「伏笔更新」必须知道原本登记的是什么，否则模型只能
        // 确认自己刚写的东西自洽，构不成审阅。
        if !context.foreshadowing.is_empty() {
            let digest = context
                .foreshadowing
                .iter()
                .filter(|record| {
                    !matches!(
                        record.status,
                        ForeshadowingStatus::Recovered | ForeshadowingStatus::Abandoned
                    )
                })
                .map(|record| {
                    format!(
                        "{}（{}，状态 {:?}）：{}",
                        record.title, record.foreshadowing_id, record.status, record.description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            push(&["unresolved_foreshadowing", "未回收伏笔"], digest);
        }

        match kind {
            // 段/事件/章节三层的对照物是「本章原本计划改什么」——
            // Planner 登记的待核对变化。
            ConfirmationKind::SegmentSummary
            | ConfirmationKind::EventSummary
            | ConfirmationKind::ChapterSummary => {
                if !context.planned_changes.is_empty() {
                    let digest = context
                        .planned_changes
                        .iter()
                        .filter(|change| change.status == RegisteredChangeStatus::Planned)
                        .map(|change| {
                            format!("{}（{:?}）", change.change_id, change.function)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    push(&["planned_changes", "本章待核对登记项"], digest);
                }
            }
            // 阶段总结的对照物是阶段总纲与该阶段已有的正式章节总结：
            // 判「本章该不该归到这个阶段」只能靠这两样。
            ConfirmationKind::StageSummary => {
                let current = context.current_stage_id.as_deref();
                if let Some(stage) = context
                    .stages
                    .iter()
                    .find(|stage| Some(stage.stage_id.as_str()) == current)
                    .or_else(|| context.stages.last())
                {
                    if let Some(summary) = stage.stage_summary.as_deref() {
                        push(
                            &["stage_outline", "阶段总纲", "当前阶段总纲"],
                            summary.to_owned(),
                        );
                    }
                    if !stage.chapter_summaries.is_empty() {
                        let digest = stage
                            .chapter_summaries
                            .iter()
                            .map(|(chapter, summary)| format!("{chapter}：{summary}"))
                            .collect::<Vec<_>>()
                            .join("\n");
                        push(&["chapter_summaries", "章节概括"], digest);
                    }
                }
            }
            // 其余确认类型不由 summarizer 发起（`audit_candidate` 对它们直接报错），
            // 这里不预置上下文，留给 fail-loud 指名。
            _ => {}
        }
        sections
    }

    /// 步骤 1：把正文按“连续且属于同一事件”切分为故事段，编号并概括。
    fn summarize_segments(
        &self,
        stage_store: Option<&SqliteWritingKnowledgeStore>,
        chapter_id: &str,
        chapter_text: &str,
    ) -> CoreResult<Vec<StorySegment>> {
        let instruction = format!(
            "{}{}\n\n请只输出 JSON，格式：\
             {{\"segments\":[{{\"number\":\"1\",\"summary\":\"本段概括\",\
             \"start_line\":1,\"end_line\":20}}]}}。\
             number 为故事段编号（可含小数如 1.5），start_line/end_line 为正文行范围。\n\n正文：\n{}",
            self.author_template_prefix(chapter_text)?,
            self.prompt("summarizer.segments")?,
            chapter_text,
        );
        let response = self.call_llm(stage_store, "segments", &instruction)?;
        let parsed: SegmentsDto = parse_json(&llm_text(&response))?;
        if parsed.segments.is_empty() {
            return Err(CoreError::validation(
                "summarizer produced no story segments",
            ));
        }
        let source_version = content_version_for_bytes(chapter_text.as_bytes());
        let mut expected_start = 0u64;
        let mut segments = Vec::with_capacity(parsed.segments.len());
        for (index, dto) in parsed.segments.into_iter().enumerate() {
            let range = line_range_to_text_range(chapter_text, dto.start_line, dto.end_line)?;
            if range.start != expected_start {
                return Err(CoreError::validation(format!(
                    "summarizer segment line ranges must be ordered, non-overlapping and gap-free: \
                     segment {} starts at byte {}, expected {}",
                    index + 1,
                    range.start,
                    expected_start
                )));
            }
            expected_start = range.end;
            let segment = StorySegment {
                segment_id: format!("{chapter_id}::seg-{}", index + 1),
                number: dto.number,
                chapter_id: chapter_id.to_owned(),
                summary: dto.summary,
                source: SourceSpan {
                    document_id: self.config.chapter_document_id.clone(),
                    range,
                    version: Some(source_version.clone()),
                },
                metadata: json!({
                    "source_line_range": {
                        "start_line": dto.start_line,
                        "end_line": dto.end_line,
                    }
                }),
            };
            segment.validate()?;
            segments.push(segment);
        }
        if expected_start != chapter_text.len() as u64 {
            return Err(CoreError::validation(format!(
                "summarizer segment line ranges do not cover the full chapter: covered through \
                 byte {expected_start}, chapter length is {}",
                chapter_text.len()
            )));
        }
        Ok(segments)
    }

    /// 步骤 2：把故事段归并为事件，维护事件状态。
    fn merge_events(
        &self,
        stage_store: Option<&SqliteWritingKnowledgeStore>,
        chapter_id: &str,
        // U175 遗留修复：作者模板里的 `{{当前章节正文}}` 要渲染，故本步也需正文。
        // 事件归并本身只吃段落摘要，正文仅用于渲染前缀，不进指令主体。
        chapter_text: &str,
        segments: &[StorySegment],
        context: &SummaryGenerationContext,
    ) -> CoreResult<Vec<StoryEvent>> {
        let segment_digest = segments
            .iter()
            .map(|seg| format!("段 {}（{}）: {}", seg.number, seg.segment_id, seg.summary))
            .collect::<Vec<_>>()
            .join("\n");
        let existing_event_digest = serde_json::to_string_pretty(&context.existing_events)?;
        let instruction = format!(
            "{}{}\n\n全部既有事件（同一事件必须复用稳定 event_id，并在此基础上更新概括/状态）：\n{}\
             \n\n本章故事段：\n{}\n\n请只输出 JSON，格式：\
             {{\"events\":[{{\"event_id\":\"event-1\",\"summary\":\"事件概括\",\
             \"status\":\"ongoing\",\"segment_ids\":[\"{}::seg-1\"]}}]}}。\
             status 取 ongoing/paused/completed。segment_ids 只能引用本章上面的段 id，\
             不要重复输出既有跨章 segment id；系统会按复用的 event_id 保留历史双向关系。\
             每个本章故事段必须至少属于一个事件。",
            self.author_template_prefix(chapter_text)?,
            self.prompt("summarizer.events")?,
            existing_event_digest,
            segment_digest,
            chapter_id,
        );
        let response = self.call_llm(stage_store, "events", &instruction)?;
        let parsed: EventsDto = parse_json(&llm_text(&response))?;
        if parsed.events.is_empty() {
            return Err(CoreError::validation("summarizer produced no events"));
        }
        let allowed_segments = segments
            .iter()
            .map(|segment| segment.segment_id.clone())
            .collect::<BTreeSet<_>>();
        let mut covered_segments = BTreeSet::new();
        let mut event_ids = BTreeSet::new();
        let mut events = Vec::with_capacity(parsed.events.len());
        for dto in parsed.events {
            if dto.event_id.trim().is_empty() || dto.summary.trim().is_empty() {
                return Err(CoreError::validation(
                    "summarizer event contains an empty id or summary",
                ));
            }
            if !event_ids.insert(dto.event_id.clone()) {
                return Err(CoreError::validation(format!(
                    "summarizer produced duplicate event id: {}",
                    dto.event_id
                )));
            }
            if dto.segment_ids.is_empty() {
                return Err(CoreError::validation(format!(
                    "summarizer event {} has no current chapter segment",
                    dto.event_id
                )));
            }
            let mut local_segments = BTreeSet::new();
            for segment_id in &dto.segment_ids {
                if !allowed_segments.contains(segment_id) {
                    return Err(CoreError::validation(format!(
                        "summarizer event {} references missing or cross-chapter segment: {}",
                        dto.event_id, segment_id
                    )));
                }
                if !local_segments.insert(segment_id.clone()) {
                    return Err(CoreError::validation(format!(
                        "summarizer event {} repeats segment: {}",
                        dto.event_id, segment_id
                    )));
                }
                covered_segments.insert(segment_id.clone());
            }
            let event = StoryEvent {
                event_id: dto.event_id,
                summary: dto.summary,
                status: parse_event_status(&dto.status)?,
                segment_ids: dto.segment_ids,
                chapter_ids: vec![chapter_id.to_owned()],
                metadata: json!({}),
            };
            event.validate()?;
            events.push(event);
        }
        if let Some(uncovered) = allowed_segments.difference(&covered_segments).next() {
            return Err(CoreError::validation(format!(
                "summarizer event output leaves story segment uncovered: {uncovered}"
            )));
        }
        Ok(events)
    }

    /// 步骤 3：总结本章，写出事件进展。
    fn summarize_chapter_text(
        &self,
        stage_store: Option<&SqliteWritingKnowledgeStore>,
        chapter_text: &str,
        segments: &[StorySegment],
        events: &[StoryEvent],
        context: &SummaryGenerationContext,
    ) -> CoreResult<GeneratedChapterSummary> {
        let event_digest = render_event_change_digest(&context.existing_events, events)?;
        let planned_change_digest = serde_json::to_string_pretty(&context.planned_changes)?;
        let foreshadowing_digest = serde_json::to_string_pretty(&context.foreshadowing)?;
        let segment_ids = segments
            .iter()
            .map(|segment| segment.segment_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let instruction = format!(
            "{}{}\n\n本章事件与既有事件的变化对比：\n{}\
             \n\nPlanner 待核对变化（其中 character_trait / relationship 的 from_value 与 to_value \
             必须结合正文判断是否真正落地）：\n{}\
             \n\n正式伏笔状态：\n{}\
             \n\n本章可引用故事段 id：{}\
             \n\n请只输出 JSON，格式：\
             {{\"summary\":\"章节总结\",\
             \"realized_changes\":[{{\"change_id\":\"change-1\",\"segment_id\":\"chapter::seg-1\"}}],\
             \"foreshadowing_updates\":[{{\"foreshadowing_id\":\"foreshadow-1\",\
             \"status\":\"planted\",\"segment_id\":\"chapter::seg-1\"}}]}}。\
             realized_changes 只能引用上面的 Planned change 与本章段；未落地项不要输出。\
             foreshadowing status 只能是 planted/recovered，必须引用正式伏笔 id 与本章段。\
             没有变化时输出空数组，不得虚构 id。\n\n正文：\n{}",
            self.author_template_prefix(chapter_text)?,
            self.prompt("summarizer.chapter_summary")?,
            event_digest,
            planned_change_digest,
            foreshadowing_digest,
            segment_ids,
            chapter_text,
        );
        let response = self.call_llm(stage_store, "chapter", &instruction)?;
        let parsed: ChapterSummaryDto = parse_json(&llm_text(&response))?;
        if parsed.summary.trim().is_empty() {
            return Err(CoreError::validation(
                "summarizer produced empty chapter summary",
            ));
        }
        validate_realized_changes(&parsed.realized_changes, segments, context)?;
        validate_foreshadowing_updates(&parsed.foreshadowing_updates, segments, context)?;
        Ok(GeneratedChapterSummary {
            summary: parsed.summary,
            realized_changes: parsed.realized_changes,
            foreshadowing_updates: parsed.foreshadowing_updates,
        })
    }

    /// 步骤 4：判断章节所属阶段并生成阶段概括。
    fn summarize_stage(
        &self,
        stage_store: Option<&SqliteWritingKnowledgeStore>,
        chapter_id: &str,
        chapter_text: &str,
        chapter_summary: &str,
        context: &SummaryGenerationContext,
    ) -> CoreResult<(String, String, bool)> {
        let stage_digest = serde_json::to_string_pretty(&context.stages)?;
        let current_stage = context.current_stage_id.as_deref().unwrap_or("未归属");
        let instruction = format!(
            "{}{}\n\n本章既有阶段归属：{}\
             \n\n全部已知阶段、阶段总结及其正式章节总结：\n{}\
             \n\n本章 id：{}\n本章正文：\n{}\n\n本章总结：{}\n\n请只输出 JSON，格式：\
             {{\"stage_id\":\"stage-1\",\"stage_summary\":\"阶段概括\",\"is_new_stage\":false}}。\
             已有阶段请 is_new_stage=false 且使用已有 stage_id；新阶段 is_new_stage=true。",
            self.author_template_prefix(chapter_text)?,
            self.prompt("summarizer.stage_summary")?,
            current_stage,
            stage_digest,
            chapter_id,
            chapter_text,
            chapter_summary,
        );
        let response = self.call_llm(stage_store, "stage", &instruction)?;
        let parsed: StageSummaryDto = parse_json(&llm_text(&response))?;
        if parsed.stage_id.trim().is_empty() || parsed.stage_summary.trim().is_empty() {
            return Err(CoreError::validation(
                "summarizer produced empty stage id or summary",
            ));
        }
        let known_stage = context
            .stages
            .iter()
            .any(|stage| stage.stage_id == parsed.stage_id);
        match (parsed.is_new_stage, known_stage) {
            (true, true) => {
                return Err(CoreError::validation(format!(
                    "summarizer marked an existing stage as new: {}",
                    parsed.stage_id
                )))
            }
            (false, false) => {
                return Err(CoreError::validation(format!(
                    "summarizer selected an unknown existing stage: {}",
                    parsed.stage_id
                )))
            }
            _ => {}
        }
        Ok((parsed.stage_id, parsed.stage_summary, parsed.is_new_stage))
    }

    /// F24：作者/节点 template 前缀（可空），**并渲染其中的占位符**。
    ///
    /// U175 遗留（2026-08-18 修）：此前是 `prompt_template` 原文直传。
    /// 而产品自带的 `node_template.summarizer.default` 就是
    /// `{{角色设定}}\n\n本章定稿正文：\n{{当前章节正文}}` ⇒ 手动粘贴该模板时
    /// **占位符字面量进请求体**，模型收到的是「本章定稿正文：{{当前章节正文}}」。
    ///
    /// ⚠️ **这条比 U114 那条更隐蔽**：总结链不校验模板，
    /// 于是节点**照样跑完、照样报成功**，四层总结全部基于一份空洞的指令产出——
    /// 用户拿到的是「看起来正常」的总结，而它没读过正文。
    ///
    /// 与 `render_approval_prompt` 复用同一套机制（变量名沿用
    /// `known_section_aliases` 的中文别名），理由见那个函数的注释：
    /// 作者在不同框里写同一个 `{{当前章节正文}}` 必须指同一件东西。
    ///
    /// 不含 `{{` 的模板原样返回——前端默认预填的是无占位符的
    /// `agent_prompt.summarizer`，不能让它白跑一趟。
    fn author_template_prefix(&self, chapter_text: &str) -> CoreResult<String> {
        let Some(template) = self
            .config
            .prompt_template
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(String::new());
        };
        if !template.contains("{{") {
            return Ok(format!("{template}\n\n"));
        }
        let mut context = crate::rag::prompt_template::PromptTemplateContext {
            // 这里的「角色设定」就是作者在节点提示词框里写的这份模板自身
            // （summarizer 没有另一份 agent 提示词可填）。渲染器只扫模板、
            // 不扫代入值，所以自引用不会无限展开。
            prompt_text: template.to_owned(),
            ..Default::default()
        };
        for alias in ["chapter_text", "当前章节正文"] {
            context
                .inputs
                .insert(alias.to_owned(), chapter_text.to_owned());
        }
        let rendered = crate::rag::prompt_template::render_prompt_template(template, &context)?;
        Ok(format!("{rendered}\n\n"))
    }

    /// 取指定提示词本体。
    fn prompt(&self, key: &str) -> CoreResult<&str> {
        self.prompts
            .get(key)
            .map(|resource| resource.prompt.as_str())
            .ok_or_else(|| CoreError::validation(format!("missing prompt resource: {key}")))
    }

    /// 测试/诊断：构造某步的完整用户指令（含 F24 template）。
    pub fn build_step_instruction_for_test(
        &self,
        step_prompt_key: &str,
        body: &str,
    ) -> CoreResult<String> {
        Ok(format!(
            "{}{}\n\n{}",
            self.author_template_prefix(body)?,
            self.prompt(step_prompt_key)?,
            body
        ))
    }

    /// 单次 LLM 调用；workflow 路径先写 stage journal，再 dispatch，并在成本落账前
    /// 固化完整 response。恢复只能重放 receipt，不能猜测是否需要再次调用模型。
    fn call_llm(
        &self,
        stage_store: Option<&SqliteWritingKnowledgeStore>,
        step: &str,
        instruction: &str,
    ) -> CoreResult<LlmResponse> {
        let executor = ProviderExecutor::new(self.ledger);
        let request = LlmRequest {
            model_id: self.config.model_id.clone(),
            messages: vec![LlmMessage::user(instruction.to_owned())],
            tools: self
                .project_search
                .iter()
                .map(|search| search.tool.clone())
                .chain(self.web_search.iter().map(|search| search.tool.clone()))
                .collect(),
            temperature: None,
            max_output_tokens: None,
            stream: false,
            metadata: json!({ "summarizer_step": step }),
        };
        let Some(parent) = self.config.workflow_operation.as_ref() else {
            return self.complete_stage_tool_loop(&executor, step, request, None, |_, _| Ok(()));
        };
        let store = stage_store.ok_or_else(|| {
            CoreError::validation("summarizer workflow operation requires a stage store")
        })?;
        let request_hash = stable_text_hash(&serde_json::to_string(&json!({
            "provider_id": self.config.provider_id.as_str(),
            "request": &request,
        }))?);
        let preparation = store.prepare_summarizer_stage_operation(
            &parent.operation_id,
            &parent.operation_id,
            parent.operation_attempt,
            &parent.request_hash,
            step,
            &request_hash,
        )?;
        match preparation {
            SummarizerStagePreparation::Replay {
                operation_id,
                response_json,
            } => {
                let (response, rounds_completed) = decode_stage_response(response_json)?;
                let cost_operation_id = if self.search_tools_enabled() {
                    format!(
                        "{operation_id}:llm-round-{}",
                        rounds_completed.saturating_sub(1)
                    )
                } else {
                    operation_id
                };
                executor.record_llm_response_cost(
                    &self.provider_call_context(step, Some(cost_operation_id)),
                    &self.config.model_id,
                    &response,
                )?;
                Ok(response)
            }
            SummarizerStagePreparation::InDoubt { operation_id } => {
                Err(CoreError::ProviderRequest {
                    service: "summarizer_stage".to_owned(),
                    outcome: ExternalDispatchOutcome::DispatchedUnknown,
                    message: format!(
                        "stage {step} operation {operation_id} has no durable response receipt"
                    ),
                })
            }
            SummarizerStagePreparation::Execute { operation_id } => {
                if let Err(error) = self.config.cancellation.check() {
                    store.abort_summarizer_stage_operation(&operation_id)?;
                    let _ = error;
                    return Err(CoreError::external_cancelled(
                        "summarizer_stage",
                        ExternalDispatchOutcome::NotDispatched,
                    ));
                }
                store.mark_summarizer_stage_dispatched(&operation_id)?;
                let receipt_completed = Cell::new(false);
                let result = self.complete_stage_tool_loop(
                    &executor,
                    step,
                    request,
                    Some(&operation_id),
                    |response, rounds_completed| {
                        let response_json = serde_json::to_value(SummarizerStageResponseReceipt {
                            response: response.clone(),
                            rounds_completed,
                        })?;
                        store.complete_summarizer_stage_operation(&operation_id, &response_json)?;
                        receipt_completed.set(true);
                        Ok(())
                    },
                );
                match result {
                    Ok(response) => Ok(response),
                    Err(error) if receipt_completed.get() => Err(error),
                    Err(error) => {
                        let transition = match error.external_dispatch_outcome() {
                            Some(
                                ExternalDispatchOutcome::NotDispatched
                                | ExternalDispatchOutcome::ResponseReceived,
                            ) => store.abort_summarizer_stage_operation(&operation_id),
                            Some(ExternalDispatchOutcome::DispatchedUnknown) | None => {
                                store.mark_summarizer_stage_in_doubt(&operation_id)
                            }
                        };
                        if let Err(journal_error) = transition {
                            return Err(CoreError::External {
                                service: "summarizer_stage_journal".to_owned(),
                                message: format!(
                                    "{error}; failed to settle stage {step}: {journal_error}"
                                ),
                            });
                        }
                        Err(error)
                    }
                }
            }
        }
    }

    fn complete_stage_tool_loop(
        &self,
        executor: &ProviderExecutor<'_, L>,
        step: &str,
        mut request: LlmRequest,
        operation_id: Option<&str>,
        mut observe_final: impl FnMut(&LlmResponse, u32) -> CoreResult<()>,
    ) -> CoreResult<LlmResponse> {
        let max_tool_rounds = self
            .project_search
            .as_ref()
            .map(|search| search.max_tool_rounds)
            .into_iter()
            .chain(
                self.web_search
                    .as_ref()
                    .map(|search| search.max_tool_rounds),
            )
            .max()
            .unwrap_or(0);
        if max_tool_rounds > 32 {
            return Err(CoreError::validation(
                "summarizer max_tool_rounds cannot exceed 32",
            ));
        }
        let project_search_executor = self.project_search.as_ref().map(|search| {
            ProjectSearchToolExecutor::new(
                search.runtime,
                self.provider_call_context(step, operation_id.map(str::to_owned)),
                [search.tool.name.clone()],
            )
        });
        let web_search_executor = self.web_search.as_ref().map(|search| {
            WebSearchToolExecutor::new(
                search.provider,
                self.ledger,
                search.policy,
                self.provider_call_context(step, operation_id.map(str::to_owned)),
                [search.tool.name.clone()],
            )
        });
        let mut tool_router = ToolExecutorRouter::new();
        if let (Some(search), Some(executor)) = (
            self.project_search.as_ref(),
            project_search_executor.as_ref(),
        ) {
            tool_router.register(search.tool.name.clone(), executor)?;
        }
        if let (Some(search), Some(executor)) =
            (self.web_search.as_ref(), web_search_executor.as_ref())
        {
            tool_router.register(search.tool.name.clone(), executor)?;
        }

        for round in 0..=max_tool_rounds {
            self.config.cancellation.check()?;
            let round_operation_id = operation_id.map(|operation_id| {
                if self.search_tools_enabled() {
                    format!("{operation_id}:llm-round-{round}")
                } else {
                    operation_id.to_owned()
                }
            });
            let context = self.provider_call_context(step, round_operation_id);
            let response = executor.complete_llm_with_response_observer(
                self.provider,
                &context,
                request.clone(),
                |response| {
                    if response.tool_calls.is_empty() {
                        observe_final(response, round.saturating_add(1))?;
                    }
                    Ok(())
                },
            )?;
            if response.tool_calls.is_empty() {
                return Ok(response);
            }
            if round >= max_tool_rounds {
                return Err(CoreError::validation(
                    "summarizer search tool max rounds exceeded before final answer",
                ));
            }
            if tool_router.is_empty() {
                return Err(CoreError::validation(
                    "summarizer returned tool calls without an enabled tool",
                ));
            }
            request.messages.push(response.message.clone());
            for call in &response.tool_calls {
                let output = tool_router.execute(
                    &ToolExecutionContext {
                        provider_id: self.config.provider_id.clone(),
                        workflow_id: self
                            .config
                            .workflow_operation
                            .as_ref()
                            .map(|operation| operation.workflow_id.clone()),
                        run_id: self
                            .config
                            .workflow_operation
                            .as_ref()
                            .map(|operation| operation.run_id.clone())
                            .or_else(|| self.config.run_id.clone().map(RunId::from)),
                        node_id: self
                            .config
                            .workflow_operation
                            .as_ref()
                            .map(|operation| operation.node_id.clone()),
                        round,
                    },
                    call,
                )?;
                request.messages.push(tool_result_message(call, output));
            }
        }
        Err(CoreError::validation(
            "summarizer search tool loop ended unexpectedly",
        ))
    }

    fn search_tools_enabled(&self) -> bool {
        self.project_search.is_some() || self.web_search.is_some()
    }

    fn provider_call_context(
        &self,
        step: &str,
        operation_id: Option<String>,
    ) -> ProviderCallContext {
        let workflow = self.config.workflow_operation.as_ref();
        ProviderCallContext {
            provider_id: self.config.provider_id.clone(),
            operation_id,
            workflow_id: workflow.map(|operation| operation.workflow_id.clone()),
            run_id: workflow
                .map(|operation| operation.run_id.clone())
                .or_else(|| self.config.run_id.clone().map(RunId::from)),
            node_id: workflow.map(|operation| operation.node_id.clone()),
            tool_call_id: None,
            timeout_ms: self.config.timeout_ms,
            max_retries: 0,
            metadata: json!({ "stage": "summarizer", "summarizer_step": step }),
            cancellation: self.config.cancellation.clone(),
            dispatch_authorization: self.config.dispatch_authorization.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SummarizerStageResponseReceipt {
    response: LlmResponse,
    rounds_completed: u32,
}

fn decode_stage_response(value: serde_json::Value) -> CoreResult<(LlmResponse, u32)> {
    match serde_json::from_value::<SummarizerStageResponseReceipt>(value.clone()) {
        Ok(receipt) => Ok((receipt.response, receipt.rounds_completed.max(1))),
        Err(_) => Ok((serde_json::from_value::<LlmResponse>(value)?, 1)),
    }
}

// U116：曾有一个 `run_and_apply(executor, pipeline, ..)` 薄封装把 summarize_chapter 与
// apply_draft 直接串起来，已删。生产路径（`workflow/integration.rs` 的 summarizer 节点）
// **必须**在两步之间做四件事：跑 Auto Mode 审计决策、取写锁、复查幂等回执、按 draft
// 重新加载工作集。省掉这些正是并发下覆盖别人确认决策的原因，所以这个封装无法被接线。

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
    #[serde(default)]
    realized_changes: Vec<RealizedChangeLink>,
    #[serde(default)]
    foreshadowing_updates: Vec<ForeshadowingUpdate>,
}

struct GeneratedChapterSummary {
    summary: String,
    realized_changes: Vec<RealizedChangeLink>,
    foreshadowing_updates: Vec<ForeshadowingUpdate>,
}

#[derive(Debug, Deserialize)]
struct StageSummaryDto {
    stage_id: String,
    stage_summary: String,
    #[serde(default)]
    is_new_stage: bool,
}

#[derive(Debug, Deserialize)]
struct ConfirmationAuditDto {
    approved: bool,
    reason: String,
}

// ── 辅助函数 ─────────────────────────────────────────────────────────────────

fn confirmation_kind_name(kind: ConfirmationKind) -> &'static str {
    match kind {
        ConfirmationKind::SegmentSummary => "segment_summary",
        ConfirmationKind::EventSummary => "event_summary",
        ConfirmationKind::ChapterSummary => "chapter_summary",
        ConfirmationKind::StageSummary => "stage_summary",
        ConfirmationKind::OutlinerOutput => "outliner_output",
        ConfirmationKind::DesignerOutput => "designer_output",
        ConfirmationKind::PlannerOutput => "planner_output",
        ConfirmationKind::PlannerRegister => "planner_register",
        ConfirmationKind::CriticReview => "critic_review",
        ConfirmationKind::PrudentReview => "prudent_review",
        ConfirmationKind::WriterCorrectionPatch => "writer_correction_patch",
        ConfirmationKind::PolisherCorrectionPatch => "polisher_correction_patch",
    }
}

fn audit_candidate(
    kind: ConfirmationKind,
    chapter_text: &str,
    draft: &SummaryPipelineDraft,
) -> CoreResult<String> {
    let value = match kind {
        ConfirmationKind::SegmentSummary => json!({
            "chapter_text": chapter_text,
            "segments": draft.segments,
            "realized_changes": draft.realized_changes,
            "foreshadowing_updates": draft.foreshadowing_updates,
        }),
        ConfirmationKind::EventSummary => json!({
            "segments": draft.segments,
            "events": draft.events,
        }),
        ConfirmationKind::ChapterSummary => json!({
            "chapter_text": chapter_text,
            "events": draft.events,
            "chapter_summary": draft.chapter_summary,
            "realized_changes": draft.realized_changes,
            "foreshadowing_updates": draft.foreshadowing_updates,
        }),
        ConfirmationKind::StageSummary => json!({
            "chapter_summary": draft.chapter_summary,
            "stage_id": draft.stage_id,
            "stage_summary": draft.stage_summary,
            "is_new_stage": draft.is_new_stage,
        }),
        _ => {
            return Err(CoreError::validation(format!(
                "unsupported summarizer Auto Mode confirmation kind {kind:?}"
            )))
        }
    };
    serde_json::to_string_pretty(&value).map_err(Into::into)
}

fn render_event_change_digest(
    existing_events: &[StoryEvent],
    current_events: &[StoryEvent],
) -> CoreResult<String> {
    let existing = existing_events
        .iter()
        .map(|event| (event.event_id.as_str(), event))
        .collect::<BTreeMap<_, _>>();
    let comparisons = current_events
        .iter()
        .map(|event| {
            let before = existing.get(event.event_id.as_str()).map(|previous| {
                json!({
                    "summary": previous.summary,
                    "status": previous.status,
                    "chapter_ids": previous.chapter_ids,
                    "segment_ids": previous.segment_ids,
                })
            });
            json!({
                "event_id": event.event_id,
                "before": before,
                "after": {
                    "summary": event.summary,
                    "status": event.status,
                    "current_chapter_segment_ids": event.segment_ids,
                }
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&comparisons).map_err(Into::into)
}

fn validate_realized_changes(
    links: &[RealizedChangeLink],
    segments: &[StorySegment],
    context: &SummaryGenerationContext,
) -> CoreResult<()> {
    let allowed_segments = segments
        .iter()
        .map(|segment| segment.segment_id.as_str())
        .collect::<BTreeSet<_>>();
    let planned = context
        .planned_changes
        .iter()
        .map(|change| (change.change_id.as_str(), change.status))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for link in links {
        if link.change_id.trim().is_empty() || link.segment_id.trim().is_empty() {
            return Err(CoreError::validation(
                "summarizer realized change contains an empty id",
            ));
        }
        if planned.get(link.change_id.as_str()) != Some(&RegisteredChangeStatus::Planned) {
            return Err(CoreError::validation(format!(
                "summarizer realized change references an unknown or non-planned change: {}",
                link.change_id
            )));
        }
        if !allowed_segments.contains(link.segment_id.as_str()) {
            return Err(CoreError::validation(format!(
                "summarizer realized change references a missing or cross-chapter segment: {}",
                link.segment_id
            )));
        }
        if !seen.insert((link.change_id.as_str(), link.segment_id.as_str())) {
            return Err(CoreError::validation(format!(
                "summarizer repeats realized change link: {} -> {}",
                link.change_id, link.segment_id
            )));
        }
    }
    Ok(())
}

fn validate_foreshadowing_updates(
    updates: &[ForeshadowingUpdate],
    segments: &[StorySegment],
    context: &SummaryGenerationContext,
) -> CoreResult<()> {
    let allowed_segments = segments
        .iter()
        .map(|segment| segment.segment_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut states = context
        .foreshadowing
        .iter()
        .map(|record| (record.foreshadowing_id.clone(), record.status))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for update in updates {
        if update.foreshadowing_id.trim().is_empty() || update.segment_id.trim().is_empty() {
            return Err(CoreError::validation(
                "summarizer foreshadowing update contains an empty id",
            ));
        }
        if !allowed_segments.contains(update.segment_id.as_str()) {
            return Err(CoreError::validation(format!(
                "summarizer foreshadowing update references a missing or cross-chapter segment: {}",
                update.segment_id
            )));
        }
        if !seen.insert((update.foreshadowing_id.as_str(), update.segment_id.as_str())) {
            return Err(CoreError::validation(format!(
                "summarizer repeats foreshadowing link: {} -> {}",
                update.foreshadowing_id, update.segment_id
            )));
        }
        let current = states
            .get(&update.foreshadowing_id)
            .copied()
            .ok_or_else(|| {
                CoreError::validation(format!(
                    "summarizer foreshadowing update references an unknown id: {}",
                    update.foreshadowing_id
                ))
            })?;
        let allowed = matches!(
            (current, update.status),
            (ForeshadowingStatus::Planned, ForeshadowingStatus::Planted)
                | (ForeshadowingStatus::Planted, ForeshadowingStatus::Planted)
                | (ForeshadowingStatus::Planted, ForeshadowingStatus::Recovered)
                | (
                    ForeshadowingStatus::Recovered,
                    ForeshadowingStatus::Recovered
                )
        );
        if !allowed {
            return Err(CoreError::validation(format!(
                "invalid foreshadowing transition for {}: {:?} -> {:?}",
                update.foreshadowing_id, current, update.status
            )));
        }
        states.insert(update.foreshadowing_id.clone(), update.status);
    }
    Ok(())
}

fn parse_event_status(s: &str) -> CoreResult<StoryEventStatus> {
    match s.trim().to_ascii_lowercase().as_str() {
        "ongoing" => Ok(StoryEventStatus::Ongoing),
        "paused" => Ok(StoryEventStatus::Paused),
        "completed" => Ok(StoryEventStatus::Completed),
        other => Err(CoreError::validation(format!(
            "summarizer produced unknown story event status: {other}"
        ))),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_status_parser_rejects_unknown_values() {
        assert_eq!(
            parse_event_status("completed").unwrap(),
            StoryEventStatus::Completed
        );
        assert!(parse_event_status("completeed").is_err());
    }
}
