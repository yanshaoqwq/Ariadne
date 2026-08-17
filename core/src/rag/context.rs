use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult};
use crate::rag::line_patch::line_numbered_text;
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    RegisterContent, RegisteredChangeStatus, WritingAgentKind, WritingContextBundle,
    WritingContextRequest, WritingContextSection,
};
use crate::rag::reference::{
    contains_content_reference, expand_content_references, ReferenceDocumentSource,
    ReferenceExpansionLimits, ReferenceWarning,
};

/// 写作节点上下文组装器；一个节点就是一个 agent。
pub struct WritingContextAssembler<'a> {
    knowledge: &'a MemoryWritingKnowledgeBase,
    /// 13-B：正文引用 `{{ref:...}}` 的文档来源。
    ///
    /// 没挂来源时，含引用的区块会 **fail-loud**（见 `expand_section_references`），
    /// 而不是把 `{{ref:...}}` 字面量送进 LLM 请求体。
    reference_documents: Option<&'a dyn ReferenceDocumentSource>,
    reference_limits: ReferenceExpansionLimits,
}

impl<'a> WritingContextAssembler<'a> {
    /// 创建上下文组装器。
    pub fn new(knowledge: &'a MemoryWritingKnowledgeBase) -> Self {
        Self {
            knowledge,
            reference_documents: None,
            reference_limits: ReferenceExpansionLimits::default(),
        }
    }

    /// 13-B：挂上正文引用的文档来源，使大纲里的 `{{ref:...}}` 就地展开为原文。
    ///
    /// 由调用方注入而非组装器自己读盘：解析 `document_id` 需要项目根、路径沙箱与
    /// 章节索引，那些都在 `documents` / `commands` 层，沙箱责任应留在已经持有
    /// `ensure_path_under_root` 的那一层。
    pub fn with_reference_documents(mut self, documents: &'a dyn ReferenceDocumentSource) -> Self {
        self.reference_documents = Some(documents);
        self
    }

    /// 覆盖引用展开的长度与条数护栏。
    pub fn with_reference_limits(mut self, limits: ReferenceExpansionLimits) -> Self {
        self.reference_limits = limits;
        self
    }

    /// 按节点/agent 类型组装上下文。
    pub fn assemble(&self, request: WritingContextRequest) -> CoreResult<WritingContextBundle> {
        validate_chapter_id(&request.chapter_id)?;
        let mut sections = match request.agent {
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

        // 13-B 约束 ③：展开在**唯一一处**收口，就在全部区块装配完之后。
        //
        // 刻意不放进各 `*_sections`：那样每新增一个 agent 就多一次「记得展开」的
        // 机会，而漏掉的那一条就是一个会把 `{{ref:...}}` 送进 LLM 请求体的安全
        // 缺口（约束 ②）。在这里收口意味着**没有任何 agent 路径能绕过它**。
        let warnings = self.expand_section_references(&mut sections)?;

        Ok(WritingContextBundle {
            agent: request.agent,
            chapter_id: request.chapter_id,
            sections,
            metadata: merge_reference_warnings(request.metadata, &warnings)?,
        })
    }

    /// 就地展开全部区块里的正文引用，并把引用坐标记进 `section.sources`。
    ///
    /// 返回全部警告，交由调用方落进运行日志与确认项 metadata。
    ///
    /// **没挂文档来源却遇到引用 → fail-loud。** 三个候选做法里只有这个是对的：
    /// - 原样放过 → `{{ref:...}}` 进 LLM 请求体，Auto Mode 的审计 LLM 会在「审的
    ///   是占位符」的前提下给出虚假通过（约束 ②），这是安全倒退；
    /// - 静默删掉 → Writer 以为那条指示后面本来就没有原文，人也看不出少了什么；
    /// - fail-loud → 报错点名是哪个区块、缺什么，用户当场知道该配什么。
    fn expand_section_references(
        &self,
        sections: &mut [WritingContextSection],
    ) -> CoreResult<Vec<ReferenceWarning>> {
        let mut warnings = Vec::new();
        for section in sections.iter_mut() {
            if !section_expands_content_references(&section.section_id) {
                continue;
            }
            if !contains_content_reference(&section.content) {
                continue;
            }
            let Some(documents) = self.reference_documents else {
                return Err(CoreError::validation(format!(
                    "writing context section `{}` contains {{{{ref:...}}}} content references but \
                     no reference document source is configured; wire \
                     WritingContextAssembler::with_reference_documents so the excerpts are \
                     expanded before the text reaches any model",
                    section.section_id
                )));
            };
            let mut expansion =
                expand_content_references(&section.content, documents, &self.reference_limits)?;
            // ⚠️ 正文用 `take` 换出来、不做 `expansion.text` 的字段移动：
            // `source_spans(&self)` 与 `merge_section_expansion_metadata(&expansion)`
            // 后面都还要借用整个 `expansion`，而字段移动会让它**部分移动**，
            // 之后任何借用都编译失败（E0382）。
            // 用 `take` 而不是 `clone`：正文可能是整章数千字，克隆正是本项目
            // 「引用式数据流」要避免的拷贝。
            section.content = std::mem::take(&mut expansion.text);
            // 展开后置条件：这个区块里**不能**再有 `{{ref:` 残留。
            //
            // 这条断言不是防御性冗余，它钉住的是约束 ②「任何送进 LLM 请求体的路径
            // 都必须展开」。展开器内部有若干「无法展开」的分支（文档缺失、越权、
            // 超条数、嵌套），每一条都必须把占位符换成可诊断标记；哪天有人新加一
            // 条分支忘了替换，占位符就会一路进到请求体，而 Auto Mode 的审计 LLM
            // 会在「审的是占位符」的前提下给出虚假通过。在这里 fail-loud，比在
            // 生产里静默批准变更好得多。
            if contains_content_reference(&section.content) {
                return Err(CoreError::validation(format!(
                    "writing context section `{}` still contains {{{{ref:...}}}} after expansion; \
                     refusing to hand unexpanded content references to a model",
                    section.section_id
                )));
            }
            // 引用坐标进 `sources`：这是既有字段（`rag/models.rs`），正好用来记录
            // 本区块展开了哪些引用，供审计溯源「Writer 看到的这段来自哪里」。
            section.sources.extend(expansion.source_spans());
            section.metadata = merge_section_expansion_metadata(
                std::mem::take(&mut section.metadata),
                &expansion,
            )?;
            warnings.extend(expansion.warnings);
        }
        Ok(warnings)
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
        // ⚠️ 下面三个区块是**知识库派生**的，必须**无条件产出**，空时给明确空态文本。
        //
        // 原实现是 `if !xxx.is_empty()`：空知识库下整个区块不入 sections，
        // 而 `node_template.planner.default` **无条件**引用 `{{前文总结}}`
        // `{{人物与关系当前状态}}` `{{未回收伏笔}}` 三者，于是
        // **新项目写第一章时 Planner 节点必然渲染失败**（渲染器对未知变量 fail-loud）——
        // 恰好是最该顺畅的那条路径。这不是 U149 引入的，从更早就有，
        // 只因当时的用例只渲染 writer 模板而没被发现。
        //
        // 为什么修装配层而不是把模板改成条件引用：渲染器没有条件语法，
        // 而「让每个模板作者自己记住哪几个变量可能缺席」是把陷阱留在原地。
        // 空态文本也比缺席更好——它明确告诉模型「这里查过了，确实没有」，
        // 而缺席会让模型以为是自己没拿到资料。
        let chapter_summaries = self.knowledge.chapter_summaries()?;
        sections.push(section(
            "previous_summaries",
            "前文总结",
            if chapter_summaries.is_empty() {
                "（暂无前文总结：这是开篇，没有需要承接的既有情节）".to_owned()
            } else {
                format_ordered_map(&chapter_summaries)
            },
            Value::Null,
        ));

        let character_state =
            current_character_and_relationship_state(&self.knowledge.registered_changes()?);
        sections.push(section(
            "character_state",
            "人物与关系当前状态",
            if character_state.is_empty() {
                "（暂无已登记的人物与关系：人物可在本章首次登场）".to_owned()
            } else {
                character_state
            },
            Value::Null,
        ));

        let foreshadowing = self.knowledge.unresolved_foreshadowing()?;
        sections.push(section(
            "unresolved_foreshadowing",
            "未回收伏笔",
            if foreshadowing.is_empty() {
                "（暂无未回收伏笔）".to_owned()
            } else {
                foreshadowing
                    .iter()
                    .map(|record| format!("- {}: {}", record.title, record.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            Value::Null,
        ));

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
        // 同 planner_sections 的理由：`node_template.detail.default` 无条件引用
        // `{{章节总结}}`，而这个区块原先只在知识库里已有该章总结时才产出——
        // 于是「先写细节、还没做总结」这条正常顺序下 Detail 节点必然渲染失败。
        // 空态文本比缺席好：它明确告诉模型「这章还没有总结」。
        sections.push(section(
            "chapter_summary",
            "章节总结",
            self.knowledge
                .chapter_summary(&request.chapter_id)?
                .unwrap_or_else(|| "（本章暂无总结：尚未写作或尚未生成总结）".to_owned()),
            Value::Null,
        ));
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

/// 哪些区块允许含正文引用。
///
/// 13-B 的引用是 **Planner 写进大纲**的，所以放行的是大纲类与规划类区块，以及
/// 数据边灌进来的 `input.*`（Planner 的产出常经数据边传给 Writer 节点）。
///
/// **正文类区块刻意不放行**（`previous_chapter_text` / `line_numbered_draft` /
/// `chapter_text` / `target_text`）：那些是小说正文本身。若正文里恰好出现
/// `{{ref:` 字样（作者写了一段讲模板语法的对话、或粘了段配置），展开它等于按
/// 用户正文里的字符串去读别的文件——把内容当指令，属注入面。放行清单是白名单
/// 而非黑名单，正是为了让「新增区块默认不可注入」。
///
/// 带行号正文尤其不能展开：行号是 patch 工具的坐标系，展开会插入若干行，
/// 使模型数出来的 `after_line` 与真实文档全部错位。
fn section_expands_content_references(section_id: &str) -> bool {
    section_id.starts_with("input.")
        || matches!(
            section_id,
            "outline"
                | "details"
                | "global_outline"
                | "stage_outline"
                | "previous_stage_outline"
                | "chapter_summaries"
                | "revision_context"
                | "revision_basis"
                | "critic_outputs"
        )
}

/// 把本区块的展开结果记进区块 metadata。
///
/// 人类 UI 靠这份结构化列表做折叠渲染（约束 ③：上游只产出展开态，折叠是纯前端
/// 的显示变换）。`expanded_range` 给出展开块在**展开后文本**里的位置，正是折叠时
/// 要替换掉的那一段。
fn merge_section_expansion_metadata(
    metadata: Value,
    expansion: &crate::rag::reference::ExpandedOutline,
) -> CoreResult<Value> {
    let mut map = match metadata {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("previous_metadata".to_owned(), other);
            map
        }
    };
    map.insert(
        "expanded_content_references".to_owned(),
        serde_json::to_value(&expansion.expanded)?,
    );
    Ok(Value::Object(map))
}

/// 把展开警告汇总进 bundle metadata。
///
/// 警告必须随 bundle 一起往上走，否则「引用失效 / 被截断」只能靠肉眼在正文里找
/// 失效标记——而确认项 payload 与运行日志都读 metadata。无警告时不插入这个键，
/// 免得每个 bundle 都多一个空数组。
fn merge_reference_warnings(
    metadata: Value,
    warnings: &[crate::rag::reference::ReferenceWarning],
) -> CoreResult<Value> {
    if warnings.is_empty() {
        return Ok(metadata);
    }
    let mut map = match metadata {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("previous_metadata".to_owned(), other);
            map
        }
    };
    map.insert(
        "content_reference_warnings".to_owned(),
        serde_json::to_value(warnings)?,
    );
    Ok(Value::Object(map))
}
