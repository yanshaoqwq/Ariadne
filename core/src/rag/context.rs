use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult};
use crate::rag::line_patch::line_numbered_text;
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    RegisterContent, RegisteredChangeStatus, WritingAgentKind, WritingContextBundle,
    WritingContextRequest, WritingContextSection,
};

/// 写作节点上下文组装器；一个节点就是一个 agent。
pub struct WritingContextAssembler<'a> {
    knowledge: &'a MemoryWritingKnowledgeBase,
}

impl<'a> WritingContextAssembler<'a> {
    /// 创建上下文组装器。
    pub fn new(knowledge: &'a MemoryWritingKnowledgeBase) -> Self {
        Self { knowledge }
    }

    /// 按节点/agent 类型组装上下文。
    pub fn assemble(&self, request: WritingContextRequest) -> CoreResult<WritingContextBundle> {
        validate_chapter_id(&request.chapter_id)?;
        let sections = match request.agent {
            WritingAgentKind::Outliner => self.outliner_sections(&request)?,
            WritingAgentKind::Designer => self.designer_sections(&request)?,
            WritingAgentKind::Planner => self.planner_sections(&request)?,
            WritingAgentKind::Detail => self.detail_sections(&request)?,
            WritingAgentKind::Writer => self.writer_sections(&request)?,
            WritingAgentKind::Critic => self.critic_sections(&request)?,
            WritingAgentKind::Prudent => self.prudent_sections(&request)?,
            WritingAgentKind::Polisher => self.polisher_sections(&request)?,
            WritingAgentKind::Summarizer => self.summarizer_sections(&request)?,
        };

        Ok(WritingContextBundle {
            agent: request.agent,
            chapter_id: request.chapter_id,
            sections,
            metadata: request.metadata,
        })
    }

    /// Outliner 上下文用于全局开局规划，重点接收用户初始意图和已有长期知识。
    fn outliner_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        if let Some(intent) = non_empty_optional(&request.user_intent) {
            sections.push(section(
                "user_intent",
                "用户初始意图",
                intent.to_owned(),
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.global_outline) {
            sections.push(section(
                "global_outline",
                "已有全局总纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        let character_state =
            current_character_and_relationship_state(&self.knowledge.registered_changes()?);
        if !character_state.is_empty() {
            sections.push(section(
                "character_state",
                "人物与关系当前状态",
                character_state,
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Designer 上下文用于阶段粒度规划，包含全局总纲、既有阶段总纲和章节概括。
    fn designer_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        if let Some(outline) = non_empty_optional(&request.global_outline) {
            sections.push(section(
                "global_outline",
                "全局总纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.previous_stage_outline) {
            sections.push(section(
                "previous_stage_outline",
                "之前阶段总纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.stage_outline) {
            sections.push(section(
                "stage_outline",
                "既有阶段总纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        if let Some(summaries) = non_empty_optional(&request.chapter_summaries) {
            sections.push(section(
                "chapter_summaries",
                "章节概括",
                summaries.to_owned(),
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Planner 上下文包含前文总结、人物当前状态、未回收伏笔和上一章正文。
    fn planner_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        if let Some(outline) = non_empty_optional(&request.global_outline) {
            sections.push(section(
                "global_outline",
                "全局总纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.stage_outline) {
            sections.push(section(
                "stage_outline",
                "当前阶段总纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        if let Some(summaries) = non_empty_optional(&request.chapter_summaries) {
            sections.push(section(
                "chapter_summaries",
                "当前阶段章节概括",
                summaries.to_owned(),
                Value::Null,
            ));
        }
        let chapter_summaries = self.knowledge.chapter_summaries()?;
        if !chapter_summaries.is_empty() {
            sections.push(section(
                "previous_summaries",
                "前文总结",
                format_ordered_map(&chapter_summaries),
                Value::Null,
            ));
        }

        let character_state =
            current_character_and_relationship_state(&self.knowledge.registered_changes()?);
        if !character_state.is_empty() {
            sections.push(section(
                "character_state",
                "人物与关系当前状态",
                character_state,
                Value::Null,
            ));
        }

        let foreshadowing = self.knowledge.unresolved_foreshadowing()?;
        if !foreshadowing.is_empty() {
            let content = foreshadowing
                .iter()
                .map(|record| format!("- {}: {}", record.title, record.description))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(section(
                "unresolved_foreshadowing",
                "未回收伏笔",
                content,
                Value::Null,
            ));
        }

        if let Some(text) = non_empty_optional(&request.previous_chapter_text) {
            sections.push(section(
                "previous_chapter_text",
                "上一章全文",
                text.to_owned(),
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Detail 上下文聚焦当前章节大纲和已有总结，不直接塞 Writer 草稿。
    fn detail_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        if let Some(summary) = self.knowledge.chapter_summary(&request.chapter_id)? {
            sections.push(section("chapter_summary", "章节总结", summary, Value::Null));
        }
        if let Some(outline) = non_empty_optional(&request.outline) {
            sections.push(section(
                "outline",
                "本章大纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Writer 上下文包含大纲、细节、上一章和带行号草稿；不默认塞未回收伏笔。
    fn writer_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        if let Some(text) = non_empty_optional(&request.previous_chapter_text) {
            sections.push(section(
                "previous_chapter_text",
                "上一章全文",
                text.to_owned(),
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.outline) {
            sections.push(section(
                "outline",
                "本章大纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        if let Some(details) = non_empty_optional(&request.details) {
            sections.push(section(
                "details",
                "本章细节",
                details.to_owned(),
                Value::Null,
            ));
        }
        if let Some(draft) = non_empty_optional(&request.current_draft_text) {
            sections.push(section(
                "line_numbered_draft",
                "带行号正文",
                line_numbered_text(draft),
                json!({ "line_numbered": true }),
            ));
        }
        if let Some(revision) = non_empty_optional(&request.revision_context) {
            sections.push(section(
                "revision_context",
                "审慎者返修上下文",
                revision.to_owned(),
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Critic 上下文用于评价正文，可接入待评价文本、章节/阶段规划和上游 alias。
    fn critic_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        if let Some(text) = non_empty_optional(&request.target_text) {
            sections.push(section(
                "target_text",
                "待评价文本",
                text.to_owned(),
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.outline) {
            sections.push(section(
                "outline",
                "本章大纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.stage_outline) {
            sections.push(section(
                "stage_outline",
                "阶段总纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        if sections.is_empty() {
            return Err(CoreError::validation("critic context requires target_text"));
        }
        Ok(sections)
    }

    /// Prudent 上下文接收一个或多个 Critic 输出，并形成返修判断依据。
    fn prudent_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        if let Some(outputs) = non_empty_optional(&request.critic_outputs) {
            sections.push(section(
                "critic_outputs",
                "意见者输出",
                outputs.to_owned(),
                Value::Null,
            ));
        }
        if let Some(text) = non_empty_optional(&request.target_text) {
            sections.push(section(
                "target_text",
                "待评价文本",
                text.to_owned(),
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.outline) {
            sections.push(section(
                "outline",
                "本章大纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        if !sections
            .iter()
            .any(|section| section.section_id == "critic_outputs" || section.title == "意见者输出")
        {
            return Err(CoreError::validation(
                "prudent context requires critic_outputs",
            ));
        }
        Ok(sections)
    }

    /// Polisher 上下文必须包含当前正文，并至少包含 Critic 或 Prudent 返修依据之一。
    fn polisher_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        let critic_outputs = non_empty_optional(&request.critic_outputs);
        let revision_context = non_empty_optional(&request.revision_context);
        if let Some(draft) = non_empty_optional(&request.current_draft_text) {
            sections.push(section(
                "line_numbered_draft",
                "带行号正文",
                line_numbered_text(draft),
                json!({ "line_numbered": true }),
            ));
        }
        if let Some(outputs) = critic_outputs {
            sections.push(section(
                "critic_outputs",
                "意见者输出",
                outputs.to_owned(),
                Value::Null,
            ));
        }
        if let Some(revision) = revision_context {
            sections.push(section(
                "revision_context",
                "审慎者返修上下文",
                revision.to_owned(),
                Value::Null,
            ));
        }
        let revision_basis = [critic_outputs, revision_context]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
        if !revision_basis.is_empty() {
            sections.push(section(
                "revision_basis",
                "返修依据",
                revision_basis,
                Value::Null,
            ));
        }
        if let Some(outline) = non_empty_optional(&request.outline) {
            sections.push(section(
                "outline",
                "本章大纲",
                outline.to_owned(),
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        if !sections
            .iter()
            .any(|section| section.section_id == "line_numbered_draft")
        {
            return Err(CoreError::validation(
                "polisher context requires current_draft_text",
            ));
        }
        if !sections.iter().any(|section| {
            section.section_id == "critic_outputs" || section.section_id == "revision_context"
        }) {
            return Err(CoreError::validation(
                "polisher context requires critic_outputs or revision_context",
            ));
        }
        Ok(sections)
    }

    /// Summarizer 上下文接收当前正文草稿，后续由流水线执行器消费结构化结果。
    fn summarizer_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        if let Some(draft) = non_empty_optional(&request.current_draft_text) {
            sections.push(section(
                "chapter_text",
                "当前章节正文",
                draft.to_owned(),
                Value::Null,
            ));
        }
        if sections.is_empty() {
            return Err(CoreError::validation(
                "summarizer context requires current_draft_text",
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }
}

/// 生成上下文区块。
fn section(
    section_id: impl Into<String>,
    title: impl Into<String>,
    content: impl Into<String>,
    metadata: Value,
) -> WritingContextSection {
    WritingContextSection {
        section_id: section_id.into(),
        title: title.into(),
        content: content.into(),
        sources: Vec::new(),
        metadata,
    }
}

/// 将上游数据边 alias 展开为模板可引用的上下文区块。
fn append_template_inputs(
    sections: &mut Vec<WritingContextSection>,
    inputs: &std::collections::BTreeMap<String, String>,
) -> CoreResult<()> {
    for (alias, content) in inputs {
        if alias.trim().is_empty() {
            return Err(CoreError::validation(
                "template input alias cannot be empty",
            ));
        }
        if content.trim().is_empty() {
            continue;
        }
        sections.push(section(
            format!("input.{alias}"),
            alias.clone(),
            content.clone(),
            json!({ "from_template_input": true }),
        ));
    }
    Ok(())
}

/// 格式化有序摘要表。
fn format_ordered_map(values: &std::collections::BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("- {key}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从已落地注册项中生成当前人物和关系状态。
fn current_character_and_relationship_state(
    changes: &[crate::rag::models::RegisteredChange],
) -> String {
    changes
        .iter()
        .filter(|change| change.status == RegisteredChangeStatus::Realized)
        .filter_map(|change| match &change.content {
            RegisterContent::CharacterProfile(content) => Some(format!(
                "- 人物 {}({}): {}; 初始状态 {}",
                content.name, content.character_id, content.narrative_role, content.initial_state
            )),
            RegisterContent::CharacterPlan(content) => Some(format!(
                "- 人物计划 {}: {} 于 {} 承担 {}; 目标 {}",
                content.plan_id,
                content.character_id,
                match (&content.stage_id, &content.chapter_id) {
                    (Some(stage_id), Some(chapter_id)) =>
                        format!("阶段 {stage_id} / 章节 {chapter_id}"),
                    (Some(stage_id), None) => format!("阶段 {stage_id}"),
                    (None, Some(chapter_id)) => format!("章节 {chapter_id}"),
                    (None, None) => "未指定范围".to_owned(),
                },
                content.narrative_function,
                content.appearance_goal
            )),
            RegisterContent::CharacterTrait(content) => Some(format!(
                "- {} / {}: {}",
                content.character, content.trait_name, content.to_value
            )),
            RegisterContent::Relationship(content) => Some(format!(
                "- {} 与 {} / {}: {}",
                content.character_a,
                content.character_b,
                content.relationship_name,
                content.to_value
            )),
            RegisterContent::ThemeAnchor(content) => {
                Some(format!("- 主题 {}: {}", content.title, content.statement))
            }
            RegisterContent::Foreshadowing(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 返回非空可选字符串。
fn non_empty_optional(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// 校验章节 id。
fn validate_chapter_id(chapter_id: &str) -> CoreResult<()> {
    if chapter_id.trim().is_empty() {
        return Err(CoreError::validation("chapter_id cannot be empty"));
    }
    Ok(())
}
