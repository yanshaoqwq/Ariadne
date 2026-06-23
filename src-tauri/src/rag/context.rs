use serde_json::{json, Value};

use crate::core::{CoreError, CoreResult};
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
            WritingAgentKind::Planner => self.planner_sections(&request)?,
            WritingAgentKind::Detail => self.detail_sections(&request)?,
            WritingAgentKind::Writer => self.writer_sections(&request)?,
            WritingAgentKind::Summarizer => self.summarizer_sections(&request)?,
        };

        Ok(WritingContextBundle {
            agent: request.agent,
            chapter_id: request.chapter_id,
            sections,
            metadata: request.metadata,
        })
    }

    /// Planner 上下文包含前文总结、人物当前状态、未回收伏笔和上一章正文。
    fn planner_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
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
