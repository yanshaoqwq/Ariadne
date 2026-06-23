use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{CoreError, DocumentPatch, SourceSpan};

/// 写作节点中的 agent 类型；一个节点就是一个 agent。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingAgentKind {
    Planner,
    Detail,
    Writer,
    Summarizer,
}

impl WritingAgentKind {
    /// 返回该节点/agent 的显示名资源 key。
    pub fn display_name_key(self) -> &'static str {
        match self {
            Self::Planner => "agent.planner",
            Self::Detail => "agent.detail",
            Self::Writer => "agent.writer",
            Self::Summarizer => "agent.summarizer",
        }
    }
}

/// 写作节点定义；Planner、Detail、Writer、Summarizer 都是独立节点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritingNodeDefinition {
    pub agent: WritingAgentKind,
    pub display_name_key: String,
    #[serde(default)]
    pub tool_names: Vec<String>,
    #[serde(default)]
    pub prompt_keys: Vec<String>,
    #[serde(default)]
    pub confirmation_kinds: Vec<ConfirmationKind>,
}

impl WritingNodeDefinition {
    /// 返回四个内置写作节点定义。
    pub fn built_in_nodes() -> Vec<Self> {
        vec![
            Self::planner(),
            Self::detail(),
            Self::writer(),
            Self::summarizer(),
        ]
    }

    /// 返回 Planner 节点定义。
    pub fn planner() -> Self {
        Self {
            agent: WritingAgentKind::Planner,
            display_name_key: WritingAgentKind::Planner.display_name_key().to_owned(),
            tool_names: vec![
                "planner-register".to_owned(),
                "planner-find".to_owned(),
                "planner-search".to_owned(),
            ],
            prompt_keys: Vec::new(),
            confirmation_kinds: vec![ConfirmationKind::PlannerRegister],
        }
    }

    /// 返回 Detail 节点定义。
    pub fn detail() -> Self {
        Self {
            agent: WritingAgentKind::Detail,
            display_name_key: WritingAgentKind::Detail.display_name_key().to_owned(),
            tool_names: vec!["detail-find".to_owned(), "detail-search".to_owned()],
            prompt_keys: Vec::new(),
            confirmation_kinds: Vec::new(),
        }
    }

    /// 返回 Writer 节点定义，包含查询、现实考据和行号 patch 工具。
    pub fn writer() -> Self {
        Self {
            agent: WritingAgentKind::Writer,
            display_name_key: WritingAgentKind::Writer.display_name_key().to_owned(),
            tool_names: vec![
                "writer-find".to_owned(),
                "writer-search".to_owned(),
                "writer-insert-lines".to_owned(),
                "writer-replace-lines".to_owned(),
            ],
            prompt_keys: Vec::new(),
            confirmation_kinds: vec![ConfirmationKind::WriterCorrectionPatch],
        }
    }

    /// 返回 Summarizer 节点定义；它是节点，但不暴露普通 tool 集合。
    pub fn summarizer() -> Self {
        Self {
            agent: WritingAgentKind::Summarizer,
            display_name_key: WritingAgentKind::Summarizer.display_name_key().to_owned(),
            tool_names: Vec::new(),
            prompt_keys: vec![
                "summarizer.segments".to_owned(),
                "summarizer.events".to_owned(),
                "summarizer.chapter_summary".to_owned(),
                "summarizer.stage_summary".to_owned(),
            ],
            confirmation_kinds: vec![
                ConfirmationKind::SegmentSummary,
                ConfirmationKind::EventSummary,
                ConfirmationKind::ChapterSummary,
                ConfirmationKind::StageSummary,
            ],
        }
    }
    /// 校验写作节点定义与内置资源、工具边界一致。
    pub fn validate(
        &self,
        prompts: &crate::rag::resources::PromptResources,
        display_names: &crate::rag::resources::DisplayNameResources,
    ) -> crate::core::CoreResult<()> {
        validate_resource_key(display_names, "display_name_key", &self.display_name_key)?;
        if self.display_name_key != self.agent.display_name_key() {
            return Err(CoreError::validation(
                "writing node display_name_key does not match agent",
            ));
        }
        let expected_tool_names = expected_tool_names(self.agent);
        if self
            .tool_names
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != expected_tool_names
        {
            return Err(CoreError::validation(
                "writing node tool_names do not match agent contract",
            ));
        }
        let expected_confirmation_kinds = expected_confirmation_kinds(self.agent);
        if self.confirmation_kinds.as_slice() != expected_confirmation_kinds {
            return Err(CoreError::validation(
                "writing node confirmation kinds do not match agent contract",
            ));
        }

        // 工具显示名和提示词分别存在不同资源表；这里用 tool name 推导资源 key。
        for tool_name in &self.tool_names {
            validate_resource_key(display_names, "tool_name", &format!("tool.{tool_name}"))?;
            validate_resource_key(
                prompts,
                "tool_prompt",
                &format!("tool.{}", tool_name.replace('-', "_")),
            )?;
        }
        for prompt_key in &self.prompt_keys {
            validate_resource_key(prompts, "prompt_key", prompt_key)?;
        }

        if self.agent == WritingAgentKind::Summarizer {
            let expected_prompt_keys = [
                "summarizer.segments",
                "summarizer.events",
                "summarizer.chapter_summary",
                "summarizer.stage_summary",
            ];
            if !self.tool_names.is_empty()
                || self
                    .prompt_keys
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    != expected_prompt_keys
            {
                return Err(CoreError::validation(
                    "summarizer node must use segment/event/chapter/stage pipeline definition",
                ));
            }
        } else if !self.prompt_keys.is_empty() {
            return Err(CoreError::validation(
                "planner/detail/writer nodes should not carry summarizer prompt keys",
            ));
        }

        Ok(())
    }
}

/// 返回指定写作节点允许暴露的工具名。
fn expected_tool_names(agent: WritingAgentKind) -> &'static [&'static str] {
    match agent {
        WritingAgentKind::Planner => &["planner-register", "planner-find", "planner-search"],
        WritingAgentKind::Detail => &["detail-find", "detail-search"],
        WritingAgentKind::Writer => &[
            "writer-find",
            "writer-search",
            "writer-insert-lines",
            "writer-replace-lines",
        ],
        WritingAgentKind::Summarizer => &[],
    }
}

/// 返回指定写作节点对应的确认项顺序。
fn expected_confirmation_kinds(agent: WritingAgentKind) -> &'static [ConfirmationKind] {
    match agent {
        WritingAgentKind::Planner => &[ConfirmationKind::PlannerRegister],
        WritingAgentKind::Detail => &[],
        WritingAgentKind::Writer => &[ConfirmationKind::WriterCorrectionPatch],
        WritingAgentKind::Summarizer => &[
            ConfirmationKind::SegmentSummary,
            ConfirmationKind::EventSummary,
            ConfirmationKind::ChapterSummary,
            ConfirmationKind::StageSummary,
        ],
    }
}

/// Detail 节点的预设类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailPreset {
    Environment,
    Psychology,
    Setting,
}

/// 故事段记录，只保存正文来源引用，不复制正文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorySegment {
    pub segment_id: String,
    pub number: String,
    pub chapter_id: String,
    pub summary: String,
    pub source: SourceSpan,
    #[serde(default)]
    pub metadata: Value,
}

impl StorySegment {
    /// 校验故事段的编号、章节、概括和来源。
    pub fn validate(&self) -> crate::core::CoreResult<()> {
        validate_non_empty("segment_id", &self.segment_id)?;
        validate_non_empty("number", &self.number)?;
        validate_non_empty("chapter_id", &self.chapter_id)?;
        validate_non_empty("summary", &self.summary)?;
        crate::rag::numbering::parse_segment_number(&self.number)?;
        Ok(())
    }
}

/// 事件生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryEventStatus {
    Ongoing,
    Paused,
    Completed,
}

/// 事件记录，关联多个故事段和章节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoryEvent {
    pub event_id: String,
    pub summary: String,
    pub status: StoryEventStatus,
    #[serde(default)]
    pub segment_ids: Vec<String>,
    #[serde(default)]
    pub chapter_ids: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

impl StoryEvent {
    /// 校验事件必须有 id 和概括。
    pub fn validate(&self) -> crate::core::CoreResult<()> {
        validate_non_empty("event_id", &self.event_id)?;
        validate_non_empty("summary", &self.summary)?;
        Ok(())
    }
}

/// Planner register 支持的功能。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisterFunction {
    CharacterTrait,
    Relationship,
    Foreshadowing,
}

impl RegisterFunction {
    /// 从 `planner-register` 的 a 参数解析功能。
    pub fn parse(value: &str) -> crate::core::CoreResult<Self> {
        match value {
            "character_trait" | "人物性格" | "人物成长" => Ok(Self::CharacterTrait),
            "relationship" | "人物关系" => Ok(Self::Relationship),
            "foreshadowing" | "伏笔" => Ok(Self::Foreshadowing),
            other => Err(crate::core::CoreError::validation(format!(
                "unknown register function: {other}"
            ))),
        }
    }
}

/// Planner register 支持的操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisterOperation {
    List,
    New,
    Update,
    Delete,
}

impl RegisterOperation {
    /// 从 `planner-register` 的 b 参数解析操作。
    pub fn parse(value: &str) -> crate::core::CoreResult<Self> {
        match value {
            "list" => Ok(Self::List),
            "new" => Ok(Self::New),
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            other => Err(crate::core::CoreError::validation(format!(
                "unknown register operation: {other}"
            ))),
        }
    }
}

/// 注册项生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisteredChangeStatus {
    Planned,
    Realized,
    Deleted,
}

/// 人物性格注册内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterTraitContent {
    pub character: String,
    pub trait_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_value: Option<String>,
    pub to_value: String,
    #[serde(default)]
    pub reason: String,
}

/// 人物关系注册内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipContent {
    pub character_a: String,
    pub character_b: String,
    pub relationship_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_value: Option<String>,
    pub to_value: String,
    #[serde(default)]
    pub reason: String,
}

/// 伏笔注册内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeshadowingContent {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub intended_payoff: String,
}

/// 强类型 register 内容，避免把关键结构塞成自由文本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
pub enum RegisterContent {
    CharacterTrait(CharacterTraitContent),
    Relationship(RelationshipContent),
    Foreshadowing(ForeshadowingContent),
}

impl RegisterContent {
    /// 按功能解析 `planner-register` 的 c 参数。
    pub fn parse(function: RegisterFunction, value: Value) -> crate::core::CoreResult<Self> {
        match function {
            RegisterFunction::CharacterTrait => {
                Ok(Self::CharacterTrait(serde_json::from_value(value)?))
            }
            RegisterFunction::Relationship => {
                Ok(Self::Relationship(serde_json::from_value(value)?))
            }
            RegisterFunction::Foreshadowing => {
                Ok(Self::Foreshadowing(serde_json::from_value(value)?))
            }
        }
    }

    /// 校验强类型内容字段。
    pub fn validate(&self) -> crate::core::CoreResult<()> {
        match self {
            Self::CharacterTrait(content) => {
                validate_non_empty("character", &content.character)?;
                validate_non_empty("trait_name", &content.trait_name)?;
                validate_non_empty("to_value", &content.to_value)
            }
            Self::Relationship(content) => {
                validate_non_empty("character_a", &content.character_a)?;
                validate_non_empty("character_b", &content.character_b)?;
                validate_non_empty("relationship_name", &content.relationship_name)?;
                validate_non_empty("to_value", &content.to_value)
            }
            Self::Foreshadowing(content) => {
                validate_non_empty("title", &content.title)?;
                validate_non_empty("description", &content.description)
            }
        }
    }
}

/// Planner 注册的计划变化。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredChange {
    pub change_id: String,
    pub function: RegisterFunction,
    pub status: RegisteredChangeStatus,
    pub content: RegisterContent,
    #[serde(default)]
    pub linked_segment_ids: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

impl RegisteredChange {
    /// 校验注册项 id、功能和强类型内容一致。
    pub fn validate(&self) -> crate::core::CoreResult<()> {
        validate_non_empty("change_id", &self.change_id)?;
        match (&self.function, &self.content) {
            (RegisterFunction::CharacterTrait, RegisterContent::CharacterTrait(_))
            | (RegisterFunction::Relationship, RegisterContent::Relationship(_))
            | (RegisterFunction::Foreshadowing, RegisterContent::Foreshadowing(_)) => {
                self.content.validate()
            }
            _ => Err(crate::core::CoreError::validation(
                "register content kind does not match function",
            )),
        }
    }
}

/// 伏笔生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForeshadowingStatus {
    Planned,
    Planted,
    Recovered,
    Abandoned,
}

/// 独立伏笔记录，用于 Planner 上下文和 find 查询。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeshadowingRecord {
    pub foreshadowing_id: String,
    pub title: String,
    pub description: String,
    pub status: ForeshadowingStatus,
    #[serde(default)]
    pub planted_segment_ids: Vec<String>,
    #[serde(default)]
    pub recovered_segment_ids: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// 双向索引记录。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BidirectionalIndex {
    #[serde(default)]
    pub chapter_segments: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub segment_chapter: BTreeMap<String, String>,
    #[serde(default)]
    pub event_segments: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub segment_events: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub event_chapters: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub chapter_events: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub change_segments: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub segment_changes: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub foreshadowing_segments: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub segment_foreshadowing: BTreeMap<String, Vec<String>>,
}

/// Summarizer 发现 Planner 注册项未落地时生成的问题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerIssue {
    pub issue_id: String,
    pub change_id: String,
    pub chapter_id: String,
    pub reason: String,
    #[serde(default)]
    pub related_sources: Vec<SourceSpan>,
    #[serde(default)]
    pub planner_explanation: Option<String>,
    #[serde(default)]
    pub correction_patch: Option<DocumentPatch>,
}

/// 确认项类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationKind {
    PlannerRegister,
    SegmentSummary,
    EventSummary,
    ChapterSummary,
    StageSummary,
    WriterCorrectionPatch,
}

/// 确认项处理状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationState {
    Pending,
    Skipped,
    AutoAudited,
    Approved,
    Rejected,
}

/// 总结机制确认项记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationItem {
    pub confirmation_id: String,
    pub kind: ConfirmationKind,
    pub state: ConfirmationState,
    pub prompt_key: String,
    #[serde(default)]
    pub metadata: Value,
}

impl ConfirmationItem {
    /// 创建确认项并按确认策略计算初始状态。
    pub fn new(
        confirmation_id: impl Into<String>,
        kind: ConfirmationKind,
        state: ConfirmationState,
        metadata: Value,
    ) -> Self {
        Self {
            confirmation_id: confirmation_id.into(),
            kind,
            state,
            prompt_key: confirmation_prompt_key(kind).to_owned(),
            metadata,
        }
    }
}

/// 确认项处理模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationMode {
    RequireHuman,
    Skip,
    AutoAudit,
}

/// 总结机制确认策略；普通模式默认人工确认，Auto Mode 默认自动审计。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritingConfirmationPolicy {
    pub planner_register: ConfirmationMode,
    pub segment_summary: ConfirmationMode,
    pub event_summary: ConfirmationMode,
    pub chapter_summary: ConfirmationMode,
    pub stage_summary: ConfirmationMode,
    pub writer_correction_patch: ConfirmationMode,
}

impl WritingConfirmationPolicy {
    /// 普通模式默认全部进入待确认。
    pub fn normal_default() -> Self {
        Self {
            planner_register: ConfirmationMode::RequireHuman,
            segment_summary: ConfirmationMode::RequireHuman,
            event_summary: ConfirmationMode::RequireHuman,
            chapter_summary: ConfirmationMode::RequireHuman,
            stage_summary: ConfirmationMode::RequireHuman,
            writer_correction_patch: ConfirmationMode::RequireHuman,
        }
    }

    /// Auto Mode 默认全部执行自动审计。
    pub fn auto_audit_default() -> Self {
        Self {
            planner_register: ConfirmationMode::AutoAudit,
            segment_summary: ConfirmationMode::AutoAudit,
            event_summary: ConfirmationMode::AutoAudit,
            chapter_summary: ConfirmationMode::AutoAudit,
            stage_summary: ConfirmationMode::AutoAudit,
            writer_correction_patch: ConfirmationMode::AutoAudit,
        }
    }

    /// 返回指定确认项的处理模式。
    pub fn mode_for(&self, kind: ConfirmationKind) -> ConfirmationMode {
        match kind {
            ConfirmationKind::PlannerRegister => self.planner_register,
            ConfirmationKind::SegmentSummary => self.segment_summary,
            ConfirmationKind::EventSummary => self.event_summary,
            ConfirmationKind::ChapterSummary => self.chapter_summary,
            ConfirmationKind::StageSummary => self.stage_summary,
            ConfirmationKind::WriterCorrectionPatch => self.writer_correction_patch,
        }
    }

    /// 根据策略和 Auto Mode 状态计算确认项初始状态。
    pub fn initial_state(
        &self,
        kind: ConfirmationKind,
        auto_mode: &crate::core::AutoModeState,
    ) -> ConfirmationState {
        match self.mode_for(kind) {
            ConfirmationMode::RequireHuman => ConfirmationState::Pending,
            ConfirmationMode::Skip => ConfirmationState::Skipped,
            ConfirmationMode::AutoAudit if auto_mode.enabled => ConfirmationState::AutoAudited,
            ConfirmationMode::AutoAudit => ConfirmationState::Pending,
        }
    }
}

/// 返回确认项对应的自动审计 prompt key。
pub fn confirmation_prompt_key(kind: ConfirmationKind) -> &'static str {
    match kind {
        ConfirmationKind::PlannerRegister => "auto_audit.register",
        ConfirmationKind::SegmentSummary
        | ConfirmationKind::EventSummary
        | ConfirmationKind::ChapterSummary
        | ConfirmationKind::StageSummary => "auto_audit.summary",
        ConfirmationKind::WriterCorrectionPatch => "auto_audit.correction_patch",
    }
}

/// 写作上下文区块，供 Planner/Detail/Writer 节点组装 prompt。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WritingContextSection {
    pub section_id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub sources: Vec<SourceSpan>,
    #[serde(default)]
    pub metadata: Value,
}

/// 写作节点上下文包；一个节点就是一个 agent。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WritingContextBundle {
    pub agent: WritingAgentKind,
    pub chapter_id: String,
    #[serde(default)]
    pub sections: Vec<WritingContextSection>,
    #[serde(default)]
    pub metadata: Value,
}

/// 写作上下文组装请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WritingContextRequest {
    pub agent: WritingAgentKind,
    pub chapter_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_chapter_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_draft_text: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// find 工具的查询范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindScope {
    CharacterTraitPath,
    RelationshipPath,
    EventSegments,
    SegmentText,
    Foreshadowing,
    ChapterSummary,
    StageSummary,
}

impl FindScope {
    /// 从 find 的 a 参数解析查询范围。
    pub fn parse(value: &str) -> crate::core::CoreResult<Self> {
        match value {
            "character_trait_path" | "人物性格路径" | "人物当前性格" => {
                Ok(Self::CharacterTraitPath)
            }
            "relationship_path" | "人物关系路径" | "人物当前关系" => {
                Ok(Self::RelationshipPath)
            }
            "event_segments" | "事件故事段" => Ok(Self::EventSegments),
            "segment_text" | "故事段文本" => Ok(Self::SegmentText),
            "foreshadowing" | "伏笔" | "未回收伏笔" => Ok(Self::Foreshadowing),
            "chapter_summary" | "章节总结" => Ok(Self::ChapterSummary),
            "stage_summary" | "阶段总结" => Ok(Self::StageSummary),
            other => Err(crate::core::CoreError::validation(format!(
                "unknown find scope: {other}"
            ))),
        }
    }
}

/// find 工具请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindRequest {
    pub scope: FindScope,
    pub query: String,
    #[serde(default)]
    pub include_text: bool,
    #[serde(default)]
    pub metadata: Value,
}

/// find 工具返回的单条结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindResult {
    pub result_id: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    pub source: String,
    #[serde(default)]
    pub spans: Vec<SourceSpan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// find 工具响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindResponse {
    pub results: Vec<FindResult>,
}

/// planner/detail search 响应，外部搜索结果不自动入库。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WritingSearchResponse {
    pub results: Vec<FindResult>,
    pub persisted_to_knowledge: bool,
}

/// 章节总结流水线的阶段状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryPipelineStep {
    Segment,
    Event,
    Chapter,
    Stage,
}

/// 章节总结流水线报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryPipelineReport {
    pub chapter_id: String,
    pub completed_steps: Vec<SummaryPipelineStep>,
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
}

/// Summarizer 流水线输入草稿；真实 LLM 生成后由工作流模块填充。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryPipelineDraft {
    pub chapter_id: String,
    #[serde(default)]
    pub segments: Vec<StorySegment>,
    #[serde(default)]
    pub events: Vec<StoryEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_summary: Option<String>,
    #[serde(default)]
    pub realized_changes: Vec<RealizedChangeLink>,
    #[serde(default)]
    pub metadata: Value,
}

/// Summarizer 确认 Planner 注册项已经落地的链接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealizedChangeLink {
    pub change_id: String,
    pub segment_id: String,
}

/// 校验非空字符串字段。
pub(crate) fn validate_non_empty(field: &str, value: &str) -> crate::core::CoreResult<()> {
    if value.trim().is_empty() {
        return Err(crate::core::CoreError::validation(format!(
            "{field} cannot be empty"
        )));
    }
    Ok(())
}

/// 校验资源 key 已存在。
fn validate_resource_key<T>(
    resources: &BTreeMap<String, T>,
    field: &str,
    key: &str,
) -> crate::core::CoreResult<()> {
    if !resources.contains_key(key) {
        return Err(crate::core::CoreError::validation(format!(
            "{field} references missing resource key: {key}"
        )));
    }
    Ok(())
}
