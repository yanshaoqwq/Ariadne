use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::{CoreError, DocumentPatch, SourceSpan};

/// 写作节点中的 agent 类型；一个节点就是一个 agent。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingAgentKind {
    Outliner,
    Designer,
    Planner,
    Detail,
    Writer,
    Critic,
    Prudent,
    Polisher,
    Summarizer,
}

impl WritingAgentKind {
    /// 返回该节点/agent 的显示名资源 key。
    pub fn display_name_key(self) -> &'static str {
        match self {
            Self::Outliner => "agent.outliner",
            Self::Designer => "agent.designer",
            Self::Planner => "agent.planner",
            Self::Detail => "agent.detail",
            Self::Writer => "agent.writer",
            Self::Critic => "agent.critic",
            Self::Prudent => "agent.prudent",
            Self::Polisher => "agent.polisher",
            Self::Summarizer => "agent.summarizer",
        }
    }

    /// 返回节点提示词正文资源 key；模板中的 `{{节点提示词}}` 会读取这一项。
    pub fn prompt_key(self) -> &'static str {
        match self {
            Self::Outliner => "agent_prompt.outliner",
            Self::Designer => "agent_prompt.designer",
            Self::Planner => "agent_prompt.planner",
            Self::Detail => "agent_prompt.detail",
            Self::Writer => "agent_prompt.writer",
            Self::Critic => "agent_prompt.critic",
            Self::Prudent => "agent_prompt.prudent",
            Self::Polisher => "agent_prompt.polisher",
            Self::Summarizer => "agent_prompt.summarizer",
        }
    }

    /// 返回节点默认提示词模板资源 key；GUI 初始化“提示词”字段时使用。
    pub fn default_template_key(self) -> &'static str {
        match self {
            Self::Outliner => "node_template.outliner.default",
            Self::Designer => "node_template.designer.default",
            Self::Planner => "node_template.planner.default",
            Self::Detail => "node_template.detail.default",
            Self::Writer => "node_template.writer.default",
            Self::Critic => "node_template.critic.default",
            Self::Prudent => "node_template.prudent.default",
            Self::Polisher => "node_template.polisher.default",
            Self::Summarizer => "node_template.summarizer.default",
        }
    }
}

/// 写作节点定义；每个节点就是一个独立 agent。
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
    /// 返回内置写作节点定义。
    pub fn built_in_nodes() -> Vec<Self> {
        vec![
            Self::outliner(),
            Self::designer(),
            Self::planner(),
            Self::detail(),
            Self::writer(),
            Self::critic(),
            Self::prudent(),
            Self::polisher(),
            Self::summarizer(),
        ]
    }

    /// 返回 Outliner 总览者节点定义。
    pub fn outliner() -> Self {
        Self {
            agent: WritingAgentKind::Outliner,
            display_name_key: WritingAgentKind::Outliner.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Outliner)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Outliner)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            confirmation_kinds: vec![ConfirmationKind::OutlinerOutput],
        }
    }

    /// 返回 Designer 阶段设计师节点定义。
    pub fn designer() -> Self {
        Self {
            agent: WritingAgentKind::Designer,
            display_name_key: WritingAgentKind::Designer.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Designer)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Designer)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            confirmation_kinds: vec![ConfirmationKind::DesignerOutput],
        }
    }

    /// 返回 Planner 节点定义。
    pub fn planner() -> Self {
        Self {
            agent: WritingAgentKind::Planner,
            display_name_key: WritingAgentKind::Planner.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Planner)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Planner)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            confirmation_kinds: vec![
                ConfirmationKind::PlannerOutput,
                ConfirmationKind::PlannerRegister,
            ],
        }
    }

    /// 返回 Detail 节点定义。
    pub fn detail() -> Self {
        Self {
            agent: WritingAgentKind::Detail,
            display_name_key: WritingAgentKind::Detail.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Detail)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Detail)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            confirmation_kinds: Vec::new(),
        }
    }

    /// 返回 Writer 节点定义，包含查询、现实考据和行号 patch 工具。
    pub fn writer() -> Self {
        Self {
            agent: WritingAgentKind::Writer,
            display_name_key: WritingAgentKind::Writer.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Writer)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Writer)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            confirmation_kinds: vec![ConfirmationKind::WriterCorrectionPatch],
        }
    }

    /// 返回 Critic 意见者节点定义。
    pub fn critic() -> Self {
        Self {
            agent: WritingAgentKind::Critic,
            display_name_key: WritingAgentKind::Critic.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Critic)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Critic)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            confirmation_kinds: vec![ConfirmationKind::CriticReview],
        }
    }

    /// 返回 Prudent 审慎者节点定义。
    pub fn prudent() -> Self {
        Self {
            agent: WritingAgentKind::Prudent,
            display_name_key: WritingAgentKind::Prudent.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Prudent)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Prudent)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            confirmation_kinds: vec![ConfirmationKind::PrudentReview],
        }
    }

    /// 返回 Polisher 返修润色节点定义，专门消费审稿意见并修改当前章节正文。
    pub fn polisher() -> Self {
        Self {
            agent: WritingAgentKind::Polisher,
            display_name_key: WritingAgentKind::Polisher.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Polisher)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Polisher)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            confirmation_kinds: vec![ConfirmationKind::PolisherCorrectionPatch],
        }
    }

    /// 返回 Summarizer 节点定义；四步总结可按需调用项目检索。
    pub fn summarizer() -> Self {
        Self {
            agent: WritingAgentKind::Summarizer,
            display_name_key: WritingAgentKind::Summarizer.display_name_key().to_owned(),
            tool_names: expected_tool_names(WritingAgentKind::Summarizer)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            prompt_keys: expected_prompt_keys(WritingAgentKind::Summarizer)
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
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
    ) -> crate::contracts::CoreResult<()> {
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

        if self
            .prompt_keys
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != expected_prompt_keys(self.agent)
        {
            return Err(CoreError::validation(
                "writing node prompt_keys do not match agent contract",
            ));
        }

        Ok(())
    }
}

/// 返回指定写作节点允许暴露的工具名。
fn expected_tool_names(agent: WritingAgentKind) -> &'static [&'static str] {
    match agent {
        WritingAgentKind::Outliner => &[
            "outliner-register",
            "outliner-find",
            "outliner-search",
            "outliner-web-search",
            "outliner-insert-lines",
            "outliner-replace-lines",
            "outliner-rewrite-file",
        ],
        WritingAgentKind::Designer => &[
            "designer-register",
            "designer-find",
            "designer-search",
            "designer-web-search",
            "designer-insert-lines",
            "designer-replace-lines",
            "designer-rewrite-file",
        ],
        WritingAgentKind::Planner => &[
            "planner-register",
            "planner-find",
            "planner-search",
            "planner-web-search",
            "planner-insert-lines",
            "planner-replace-lines",
            "planner-rewrite-file",
        ],
        WritingAgentKind::Detail => &["detail-find", "detail-search", "detail-web-search"],
        WritingAgentKind::Writer => &[
            "writer-find",
            "writer-search",
            "writer-web-search",
            "writer-insert-lines",
            "writer-replace-lines",
        ],
        WritingAgentKind::Critic => &["critic-find", "critic-search", "critic-web-search"],
        WritingAgentKind::Prudent => &["prudent-find", "prudent-search", "prudent-web-search"],
        WritingAgentKind::Polisher => &[
            "polisher-find",
            "polisher-search",
            "polisher-web-search",
            "polisher-insert-lines",
            "polisher-replace-lines",
        ],
        WritingAgentKind::Summarizer => &["summarizer-search", "summarizer-web-search"],
    }
}

/// 返回指定写作节点引用的提示词资源 key。
fn expected_prompt_keys(agent: WritingAgentKind) -> &'static [&'static str] {
    match agent {
        WritingAgentKind::Outliner => &["agent_prompt.outliner", "node_template.outliner.default"],
        WritingAgentKind::Designer => &["agent_prompt.designer", "node_template.designer.default"],
        WritingAgentKind::Planner => &["agent_prompt.planner", "node_template.planner.default"],
        WritingAgentKind::Detail => &["agent_prompt.detail", "node_template.detail.default"],
        WritingAgentKind::Writer => &["agent_prompt.writer", "node_template.writer.default"],
        WritingAgentKind::Critic => &["agent_prompt.critic", "node_template.critic.default"],
        WritingAgentKind::Prudent => &["agent_prompt.prudent", "node_template.prudent.default"],
        WritingAgentKind::Polisher => &["agent_prompt.polisher", "node_template.polisher.default"],
        WritingAgentKind::Summarizer => &[
            "agent_prompt.summarizer",
            "node_template.summarizer.default",
            "summarizer.segments",
            "summarizer.events",
            "summarizer.chapter_summary",
            "summarizer.stage_summary",
        ],
    }
}

/// 返回指定写作节点对应的确认项顺序。
fn expected_confirmation_kinds(agent: WritingAgentKind) -> &'static [ConfirmationKind] {
    match agent {
        WritingAgentKind::Outliner => &[ConfirmationKind::OutlinerOutput],
        WritingAgentKind::Designer => &[ConfirmationKind::DesignerOutput],
        WritingAgentKind::Planner => &[
            ConfirmationKind::PlannerOutput,
            ConfirmationKind::PlannerRegister,
        ],
        WritingAgentKind::Detail => &[],
        WritingAgentKind::Writer => &[ConfirmationKind::WriterCorrectionPatch],
        WritingAgentKind::Critic => &[ConfirmationKind::CriticReview],
        WritingAgentKind::Prudent => &[ConfirmationKind::PrudentReview],
        WritingAgentKind::Polisher => &[ConfirmationKind::PolisherCorrectionPatch],
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
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
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
    /// 校验事件必须有 id、概括及可追溯的故事段/章节引用。
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        validate_non_empty("event_id", &self.event_id)?;
        validate_non_empty("summary", &self.summary)?;
        if self.segment_ids.is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "story event requires at least one segment_id",
            ));
        }
        if self.chapter_ids.is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "story event requires at least one chapter_id",
            ));
        }
        Ok(())
    }
}

/// Planner register 支持的功能。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisterFunction {
    CharacterProfile,
    CharacterPlan,
    CharacterTrait,
    Relationship,
    Foreshadowing,
    ThemeAnchor,
}

impl RegisterFunction {
    /// 从 `planner-register` 的 a 参数解析功能。
    pub fn parse(value: &str) -> crate::contracts::CoreResult<Self> {
        match value {
            "character_profile" | "人物实体" | "人物卡" => Ok(Self::CharacterProfile),
            "character_plan" | "人物出场计划" | "出场计划" => Ok(Self::CharacterPlan),
            "character_trait" | "人物性格" | "人物成长" => Ok(Self::CharacterTrait),
            "relationship" | "人物关系" => Ok(Self::Relationship),
            "foreshadowing" | "伏笔" => Ok(Self::Foreshadowing),
            "theme_anchor" | "主题锚点" | "主题" => Ok(Self::ThemeAnchor),
            other => Err(crate::contracts::CoreError::validation(format!(
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
    pub fn parse(value: &str) -> crate::contracts::CoreResult<Self> {
        match value {
            "list" => Ok(Self::List),
            "new" => Ok(Self::New),
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            other => Err(crate::contracts::CoreError::validation(format!(
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

/// 轻人物卡，记录需要长期复用的人物实体。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterProfileContent {
    pub character_id: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub narrative_role: String,
    pub initial_state: String,
    #[serde(default)]
    pub is_new_character: bool,
}

/// 人物出场计划，用于阶段或章节级别安排人物叙事功能。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterPlanContent {
    pub plan_id: String,
    pub character_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    pub narrative_function: String,
    pub appearance_goal: String,
    #[serde(default)]
    pub relation_to_theme: String,
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

/// 主题锚点，用于记录需要长期复用的主题、母题或表达目标。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeAnchorContent {
    pub anchor_id: String,
    pub title: String,
    pub statement: String,
    #[serde(default)]
    pub motifs: Vec<String>,
    #[serde(default)]
    pub stage_ids: Vec<String>,
    #[serde(default)]
    pub chapter_ids: Vec<String>,
}

/// 强类型 register 内容，避免把关键结构塞成自由文本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
pub enum RegisterContent {
    CharacterProfile(CharacterProfileContent),
    CharacterPlan(CharacterPlanContent),
    CharacterTrait(CharacterTraitContent),
    Relationship(RelationshipContent),
    Foreshadowing(ForeshadowingContent),
    ThemeAnchor(ThemeAnchorContent),
}

impl RegisterContent {
    /// 按功能解析 `planner-register` 的 c 参数。
    pub fn parse(function: RegisterFunction, value: Value) -> crate::contracts::CoreResult<Self> {
        match function {
            RegisterFunction::CharacterProfile => {
                Ok(Self::CharacterProfile(serde_json::from_value(value)?))
            }
            RegisterFunction::CharacterPlan => {
                Ok(Self::CharacterPlan(serde_json::from_value(value)?))
            }
            RegisterFunction::CharacterTrait => {
                Ok(Self::CharacterTrait(serde_json::from_value(value)?))
            }
            RegisterFunction::Relationship => {
                Ok(Self::Relationship(serde_json::from_value(value)?))
            }
            RegisterFunction::Foreshadowing => {
                Ok(Self::Foreshadowing(serde_json::from_value(value)?))
            }
            RegisterFunction::ThemeAnchor => Ok(Self::ThemeAnchor(serde_json::from_value(value)?)),
        }
    }

    /// 校验强类型内容字段。
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        match self {
            Self::CharacterProfile(content) => {
                validate_non_empty("character_id", &content.character_id)?;
                validate_non_empty("name", &content.name)?;
                validate_non_empty("narrative_role", &content.narrative_role)?;
                validate_non_empty("initial_state", &content.initial_state)
            }
            Self::CharacterPlan(content) => {
                validate_non_empty("plan_id", &content.plan_id)?;
                validate_non_empty("character_id", &content.character_id)?;
                validate_non_empty("narrative_function", &content.narrative_function)?;
                validate_non_empty("appearance_goal", &content.appearance_goal)?;
                if content.stage_id.is_none() && content.chapter_id.is_none() {
                    return Err(crate::contracts::CoreError::validation(
                        "character_plan requires stage_id or chapter_id",
                    ));
                }
                Ok(())
            }
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
            Self::ThemeAnchor(content) => {
                validate_non_empty("anchor_id", &content.anchor_id)?;
                validate_non_empty("title", &content.title)?;
                validate_non_empty("statement", &content.statement)
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
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        validate_non_empty("change_id", &self.change_id)?;
        match (&self.function, &self.content) {
            (RegisterFunction::CharacterProfile, RegisterContent::CharacterProfile(_))
            | (RegisterFunction::CharacterPlan, RegisterContent::CharacterPlan(_))
            | (RegisterFunction::ThemeAnchor, RegisterContent::ThemeAnchor(_))
            | (RegisterFunction::CharacterTrait, RegisterContent::CharacterTrait(_))
            | (RegisterFunction::Relationship, RegisterContent::Relationship(_))
            | (RegisterFunction::Foreshadowing, RegisterContent::Foreshadowing(_)) => {
                self.content.validate()
            }
            _ => Err(crate::contracts::CoreError::validation(
                "register content kind does not match function",
            )),
        }
    }

    /// Summarizer 是否应在指定章节核对该 Planner 变化。
    pub fn applies_to_chapter(&self, chapter_id: &str) -> bool {
        match &self.content {
            RegisterContent::CharacterPlan(plan) => plan
                .chapter_id
                .as_deref()
                .map(|id| id == chapter_id)
                .unwrap_or(true),
            RegisterContent::ThemeAnchor(anchor) => {
                anchor.chapter_ids.is_empty()
                    || anchor.chapter_ids.iter().any(|id| id == chapter_id)
            }
            _ => true,
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
    #[serde(default)]
    pub stage_chapters: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub chapter_stage: BTreeMap<String, String>,
    #[serde(default)]
    pub character_profile_plans: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub character_plan_profile: BTreeMap<String, String>,
    #[serde(default)]
    pub stage_character_plans: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub character_plan_stage: BTreeMap<String, String>,
    #[serde(default)]
    pub chapter_character_plans: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub character_plan_chapter: BTreeMap<String, String>,
    #[serde(default)]
    pub theme_stage_links: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub stage_theme_links: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub theme_chapter_links: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub chapter_theme_links: BTreeMap<String, Vec<String>>,
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
    OutlinerOutput,
    DesignerOutput,
    PlannerOutput,
    PlannerRegister,
    CriticReview,
    PrudentReview,
    SegmentSummary,
    EventSummary,
    ChapterSummary,
    StageSummary,
    WriterCorrectionPatch,
    PolisherCorrectionPatch,
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
    pub outliner_output: ConfirmationMode,
    pub designer_output: ConfirmationMode,
    pub planner_output: ConfirmationMode,
    pub planner_register: ConfirmationMode,
    pub critic_review: ConfirmationMode,
    pub prudent_review: ConfirmationMode,
    pub segment_summary: ConfirmationMode,
    pub event_summary: ConfirmationMode,
    pub chapter_summary: ConfirmationMode,
    pub stage_summary: ConfirmationMode,
    pub writer_correction_patch: ConfirmationMode,
    #[serde(default = "default_confirmation_mode_require_human")]
    pub polisher_correction_patch: ConfirmationMode,
}

impl WritingConfirmationPolicy {
    /// 普通模式默认全部进入待确认。
    pub fn normal_default() -> Self {
        Self {
            outliner_output: ConfirmationMode::RequireHuman,
            designer_output: ConfirmationMode::RequireHuman,
            planner_output: ConfirmationMode::RequireHuman,
            planner_register: ConfirmationMode::RequireHuman,
            critic_review: ConfirmationMode::RequireHuman,
            prudent_review: ConfirmationMode::RequireHuman,
            segment_summary: ConfirmationMode::RequireHuman,
            event_summary: ConfirmationMode::RequireHuman,
            chapter_summary: ConfirmationMode::RequireHuman,
            stage_summary: ConfirmationMode::RequireHuman,
            writer_correction_patch: ConfirmationMode::RequireHuman,
            polisher_correction_patch: ConfirmationMode::RequireHuman,
        }
    }

    /// Auto Mode 默认全部执行自动审计。
    pub fn auto_audit_default() -> Self {
        Self {
            outliner_output: ConfirmationMode::AutoAudit,
            designer_output: ConfirmationMode::AutoAudit,
            planner_output: ConfirmationMode::AutoAudit,
            planner_register: ConfirmationMode::AutoAudit,
            critic_review: ConfirmationMode::AutoAudit,
            prudent_review: ConfirmationMode::AutoAudit,
            segment_summary: ConfirmationMode::AutoAudit,
            event_summary: ConfirmationMode::AutoAudit,
            chapter_summary: ConfirmationMode::AutoAudit,
            stage_summary: ConfirmationMode::AutoAudit,
            writer_correction_patch: ConfirmationMode::AutoAudit,
            polisher_correction_patch: ConfirmationMode::AutoAudit,
        }
    }

    /// 返回指定确认项的处理模式。
    pub fn mode_for(&self, kind: ConfirmationKind) -> ConfirmationMode {
        match kind {
            ConfirmationKind::OutlinerOutput => self.outliner_output,
            ConfirmationKind::DesignerOutput => self.designer_output,
            ConfirmationKind::PlannerOutput => self.planner_output,
            ConfirmationKind::PlannerRegister => self.planner_register,
            ConfirmationKind::CriticReview => self.critic_review,
            ConfirmationKind::PrudentReview => self.prudent_review,
            ConfirmationKind::SegmentSummary => self.segment_summary,
            ConfirmationKind::EventSummary => self.event_summary,
            ConfirmationKind::ChapterSummary => self.chapter_summary,
            ConfirmationKind::StageSummary => self.stage_summary,
            ConfirmationKind::WriterCorrectionPatch => self.writer_correction_patch,
            ConfirmationKind::PolisherCorrectionPatch => self.polisher_correction_patch,
        }
    }

    /// 覆盖单个确认项模式；设置页与执行器通过 ConfirmationKind 共用同一映射。
    pub fn set_mode(&mut self, kind: ConfirmationKind, mode: ConfirmationMode) {
        match kind {
            ConfirmationKind::OutlinerOutput => self.outliner_output = mode,
            ConfirmationKind::DesignerOutput => self.designer_output = mode,
            ConfirmationKind::PlannerOutput => self.planner_output = mode,
            ConfirmationKind::PlannerRegister => self.planner_register = mode,
            ConfirmationKind::CriticReview => self.critic_review = mode,
            ConfirmationKind::PrudentReview => self.prudent_review = mode,
            ConfirmationKind::SegmentSummary => self.segment_summary = mode,
            ConfirmationKind::EventSummary => self.event_summary = mode,
            ConfirmationKind::ChapterSummary => self.chapter_summary = mode,
            ConfirmationKind::StageSummary => self.stage_summary = mode,
            ConfirmationKind::WriterCorrectionPatch => self.writer_correction_patch = mode,
            ConfirmationKind::PolisherCorrectionPatch => self.polisher_correction_patch = mode,
        }
    }

    /// 根据策略和 Auto Mode 状态计算确认项初始状态。
    pub fn initial_state(
        &self,
        kind: ConfirmationKind,
        auto_mode: &crate::contracts::AutoModeState,
    ) -> ConfirmationState {
        match self.mode_for(kind) {
            ConfirmationMode::RequireHuman => ConfirmationState::Pending,
            ConfirmationMode::Skip => ConfirmationState::Skipped,
            ConfirmationMode::AutoAudit if auto_mode.enabled => ConfirmationState::AutoAudited,
            ConfirmationMode::AutoAudit => ConfirmationState::Pending,
        }
    }
}

/// 旧策略反序列化时，新确认项默认走人工确认，避免自动放宽审批。
fn default_confirmation_mode_require_human() -> ConfirmationMode {
    ConfirmationMode::RequireHuman
}

/// 返回确认项对应的自动审计 prompt key。
pub fn confirmation_prompt_key(kind: ConfirmationKind) -> &'static str {
    match kind {
        ConfirmationKind::OutlinerOutput
        | ConfirmationKind::DesignerOutput
        | ConfirmationKind::PlannerOutput => "auto_audit.planning_output",
        ConfirmationKind::PlannerRegister => "auto_audit.register",
        ConfirmationKind::CriticReview | ConfirmationKind::PrudentReview => "auto_audit.review",
        ConfirmationKind::SegmentSummary
        | ConfirmationKind::EventSummary
        | ConfirmationKind::ChapterSummary
        | ConfirmationKind::StageSummary => "auto_audit.summary",
        ConfirmationKind::WriterCorrectionPatch | ConfirmationKind::PolisherCorrectionPatch => {
            "auto_audit.correction_patch"
        }
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
    pub user_intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_outline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_outline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_stage_outline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_summaries: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_chapter_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_draft_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critic_outputs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_context: Option<String>,
    #[serde(default)]
    pub template_inputs: BTreeMap<String, String>,
    #[serde(default)]
    pub metadata: Value,
}

/// find 工具的查询范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindScope {
    CharacterProfile,
    CharacterPlan,
    CharacterTraitPath,
    RelationshipPath,
    EventSegments,
    SegmentText,
    Foreshadowing,
    ThemeAnchor,
    ChapterSummary,
    StageSummary,
}

impl FindScope {
    /// 从 find 的 a 参数解析查询范围。
    pub fn parse(value: &str) -> crate::contracts::CoreResult<Self> {
        match value {
            "character_profile" | "人物实体" | "人物卡" => Ok(Self::CharacterProfile),
            "character_plan" | "人物出场计划" | "出场计划" => Ok(Self::CharacterPlan),
            "character_trait_path" | "人物性格路径" | "人物当前性格" => {
                Ok(Self::CharacterTraitPath)
            }
            "relationship_path" | "人物关系路径" | "人物当前关系" => {
                Ok(Self::RelationshipPath)
            }
            "event_segments" | "事件故事段" => Ok(Self::EventSegments),
            "segment_text" | "故事段文本" => Ok(Self::SegmentText),
            "foreshadowing" | "伏笔" | "未回收伏笔" => Ok(Self::Foreshadowing),
            "theme_anchor" | "主题锚点" | "主题" => Ok(Self::ThemeAnchor),
            "chapter_summary" | "章节总结" => Ok(Self::ChapterSummary),
            "stage_summary" | "阶段总结" => Ok(Self::StageSummary),
            other => Err(crate::contracts::CoreError::validation(format!(
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

impl SummaryPipelineStep {
    /// 返回从当前步骤开始需要重跑的后续步骤。
    pub fn affected_steps_from(self) -> Vec<Self> {
        match self {
            Self::Segment => vec![Self::Segment, Self::Event, Self::Chapter, Self::Stage],
            Self::Event => vec![Self::Event, Self::Chapter, Self::Stage],
            Self::Chapter => vec![Self::Chapter, Self::Stage],
            Self::Stage => vec![Self::Stage],
        }
    }
}

/// 章节总结流水线报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryPipelineReport {
    pub chapter_id: String,
    pub revision_id: String,
    pub completed_steps: Vec<SummaryPipelineStep>,
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    #[serde(default)]
    pub planner_issue_ids: Vec<String>,
    #[serde(default)]
    pub confirmation_ids: Vec<String>,
}

/// Summarizer 四步生成阶段所需的只读知识投影。
///
/// 该投影与提交工作集分离：生成阶段需要全量既有事件以及阶段历史，提交阶段仍只
/// 加载当前章节关系闭包，避免把长耗时外部调用放进写事务。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SummaryGenerationContext {
    /// 全部既有事件，供跨章事件复用稳定 event_id。
    #[serde(default)]
    pub existing_events: Vec<StoryEvent>,
    /// 当前章节仍需核对的 Planner 注册变化。
    #[serde(default)]
    pub planned_changes: Vec<RegisteredChange>,
    /// 伏笔当前状态；输出更新只能引用这里的正式 id。
    #[serde(default)]
    pub foreshadowing: Vec<ForeshadowingRecord>,
    /// 已知阶段及其章节总结，用于选择已有阶段或提议新阶段。
    #[serde(default)]
    pub stages: Vec<SummaryStageContext>,
    /// 章节重跑时的既有阶段归属；新章节通常为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_stage_id: Option<String>,
}

/// 单个阶段的生成上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryStageContext {
    pub stage_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_summary: Option<String>,
    /// 仅包含已有正式章节总结；Pending 草稿不得进入下一轮上下文。
    #[serde(default)]
    pub chapter_summaries: BTreeMap<String, String>,
}

/// 作品页使用的正式章节-阶段投影。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterStageSummaryView {
    pub stage_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub chapter_ids: Vec<String>,
}

/// 作品页展示的总结确认历史；不复制 pending payload 正文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterSummaryConfirmationView {
    pub confirmation_id: String,
    pub kind: ConfirmationKind,
    pub state: ConfirmationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

/// F19/F20：作品页读取的章节级正式总结投影。
///
/// active 知识与确认历史分开返回：Pending draft 不伪装成正式知识，但作者仍可看到
/// 对应确认状态。所有来源位置沿用统一 UTF-8 byte SourceSpan 契约。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChapterSummaryView {
    pub chapter_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<ChapterStageSummaryView>,
    #[serde(default)]
    pub segments: Vec<StorySegment>,
    #[serde(default)]
    pub events: Vec<StoryEvent>,
    #[serde(default)]
    pub realized_changes: Vec<RegisteredChange>,
    #[serde(default)]
    pub foreshadowing: Vec<ForeshadowingRecord>,
    #[serde(default)]
    pub confirmations: Vec<ChapterSummaryConfirmationView>,
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
    /// F25：是否提议新阶段。`None` 时：已存在 stage 则附着，否则按新阶段提议处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_new_stage: Option<bool>,
    #[serde(default)]
    pub realized_changes: Vec<RealizedChangeLink>,
    #[serde(default)]
    pub foreshadowing_updates: Vec<ForeshadowingUpdate>,
    #[serde(default)]
    pub metadata: Value,
}

/// 确认项是否已激活知识写入（F14：Pending/Rejected 不得作为 active 事实）。
pub fn confirmation_state_activates_knowledge(state: ConfirmationState) -> bool {
    matches!(
        state,
        ConfirmationState::Approved | ConfirmationState::AutoAudited | ConfirmationState::Skipped
    )
}

/// Summarizer 确认 Planner 注册项已经落地的链接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealizedChangeLink {
    pub change_id: String,
    pub segment_id: String,
}

/// Summarizer 在故事段中确认伏笔种植或回收后的写入。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeshadowingUpdate {
    pub foreshadowing_id: String,
    pub status: ForeshadowingStatus,
    pub segment_id: String,
}

/// Writer 补写 patch 写回后，章节总结流水线需要从受影响步骤重新执行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryRerunPlan {
    pub chapter_id: String,
    pub start_step: SummaryPipelineStep,
    pub affected_steps: Vec<SummaryPipelineStep>,
    pub reason: String,
}

impl SummaryRerunPlan {
    /// 创建从指定步骤开始的重跑计划。
    pub fn new(
        chapter_id: impl Into<String>,
        start_step: SummaryPipelineStep,
        reason: impl Into<String>,
    ) -> crate::contracts::CoreResult<Self> {
        let chapter_id = chapter_id.into();
        let reason = reason.into();
        validate_non_empty("chapter_id", &chapter_id)?;
        validate_non_empty("reason", &reason)?;
        Ok(Self {
            chapter_id,
            start_step,
            affected_steps: start_step.affected_steps_from(),
            reason,
        })
    }
}

/// 校验非空字符串字段。
pub(crate) fn validate_non_empty(field: &str, value: &str) -> crate::contracts::CoreResult<()> {
    if value.trim().is_empty() {
        return Err(crate::contracts::CoreError::validation(format!(
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
) -> crate::contracts::CoreResult<()> {
    if !resources.contains_key(key) {
        return Err(crate::contracts::CoreError::validation(format!(
            "{field} references missing resource key: {key}"
        )));
    }
    Ok(())
}
