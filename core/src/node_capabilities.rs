use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// 项目 AI 与工作流模型节点使用的工具身份。工具执行器与配置默认值均从这里取值，
/// 避免 Provider、检索、权限和 commands 分别维护字符串。
pub const PROJECT_AI_SEARCH_TOOL: &str = "project-ai-search";
pub const PROJECT_AI_WEB_SEARCH_TOOL: &str = "project-ai-web-search";
pub const GENERIC_LLM_SEARCH_TOOL: &str = "llm-search";
pub const GENERIC_LLM_WEB_SEARCH_TOOL: &str = "llm-web-search";
pub const OUTLINER_SEARCH_TOOL: &str = "outliner-search";
pub const OUTLINER_WEB_SEARCH_TOOL: &str = "outliner-web-search";
pub const DESIGNER_SEARCH_TOOL: &str = "designer-search";
pub const DESIGNER_WEB_SEARCH_TOOL: &str = "designer-web-search";
pub const PLANNER_SEARCH_TOOL: &str = "planner-search";
pub const PLANNER_WEB_SEARCH_TOOL: &str = "planner-web-search";
pub const DETAIL_SEARCH_TOOL: &str = "detail-search";
pub const DETAIL_WEB_SEARCH_TOOL: &str = "detail-web-search";
pub const WRITER_SEARCH_TOOL: &str = "writer-search";
pub const WRITER_WEB_SEARCH_TOOL: &str = "writer-web-search";
pub const CRITIC_SEARCH_TOOL: &str = "critic-search";
pub const CRITIC_WEB_SEARCH_TOOL: &str = "critic-web-search";
pub const PRUDENT_SEARCH_TOOL: &str = "prudent-search";
pub const PRUDENT_WEB_SEARCH_TOOL: &str = "prudent-web-search";
pub const POLISHER_SEARCH_TOOL: &str = "polisher-search";
pub const POLISHER_WEB_SEARCH_TOOL: &str = "polisher-web-search";
pub const SUMMARIZER_SEARCH_TOOL: &str = "summarizer-search";
pub const SUMMARIZER_WEB_SEARCH_TOOL: &str = "summarizer-web-search";
pub const EXECUTOR_ADAPTER_SEARCH_TOOL: &str = "executor-adapter-search";
pub const EXECUTOR_ADAPTER_WEB_SEARCH_TOOL: &str = "executor-adapter-web-search";
pub const EXECUTOR_ADAPTER_NODE_PREFIX: &str = "executor_adapter:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeExecutionKind {
    Builtin,
    Model,
    Summarizer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeLibraryGroup {
    Entry,
    Writing,
    Utility,
}

/// 画布、设置默认值和运行时能力共同消费的产品节点目录项。
///
/// 目录来自 `resources/workflow_node_catalog.json`，因此桌面节点库、Rust 预设和
/// 节点别名不会再分别维护一份类型清单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNodeCatalogEntry {
    pub node_type: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub preset_type: String,
    pub display_name_key: String,
    pub library_group: WorkflowNodeLibraryGroup,
    pub config_kind: String,
    pub execution_kind: WorkflowNodeExecutionKind,
    pub default_budget_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_search_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search_tool: Option<String>,
}

static WORKFLOW_NODE_CATALOG: OnceLock<Vec<WorkflowNodeCatalogEntry>> = OnceLock::new();

pub fn workflow_node_catalog() -> &'static [WorkflowNodeCatalogEntry] {
    WORKFLOW_NODE_CATALOG
        .get_or_init(|| {
            serde_json::from_str(include_str!("../resources/workflow_node_catalog.json"))
                .expect("shipped workflow node catalog must be valid")
        })
        .as_slice()
}

pub fn workflow_node_catalog_entry(node_type: &str) -> Option<&'static WorkflowNodeCatalogEntry> {
    workflow_node_catalog().iter().find(|entry| {
        entry.node_type == node_type || entry.aliases.iter().any(|alias| alias == node_type)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowNodeToolCapability {
    pub node_type: &'static str,
    pub execution_kind: WorkflowNodeExecutionKind,
    pub tool_scope: &'static str,
    pub project_search_tool: Option<&'static str>,
    pub project_search_description: Option<&'static str>,
    pub web_search_tool: Option<&'static str>,
    pub web_search_description: Option<&'static str>,
}

impl WorkflowNodeToolCapability {
    pub const fn builtin(node_type: &'static str) -> Self {
        Self {
            node_type,
            execution_kind: WorkflowNodeExecutionKind::Builtin,
            tool_scope: node_type,
            project_search_tool: None,
            project_search_description: None,
            web_search_tool: None,
            web_search_description: None,
        }
    }

    pub const fn model(
        node_type: &'static str,
        project_search_tool: &'static str,
        project_search_description: &'static str,
        web_search_tool: &'static str,
        web_search_description: &'static str,
    ) -> Self {
        Self {
            node_type,
            execution_kind: WorkflowNodeExecutionKind::Model,
            tool_scope: node_type,
            project_search_tool: Some(project_search_tool),
            project_search_description: Some(project_search_description),
            web_search_tool: Some(web_search_tool),
            web_search_description: Some(web_search_description),
        }
    }

    pub const fn model_in_scope(
        node_type: &'static str,
        tool_scope: &'static str,
        project_search_tool: &'static str,
        project_search_description: &'static str,
        web_search_tool: &'static str,
        web_search_description: &'static str,
    ) -> Self {
        Self {
            node_type,
            execution_kind: WorkflowNodeExecutionKind::Model,
            tool_scope,
            project_search_tool: Some(project_search_tool),
            project_search_description: Some(project_search_description),
            web_search_tool: Some(web_search_tool),
            web_search_description: Some(web_search_description),
        }
    }

    pub const fn summarizer() -> Self {
        Self {
            node_type: "summarizer",
            execution_kind: WorkflowNodeExecutionKind::Summarizer,
            tool_scope: "summarizer",
            project_search_tool: Some(SUMMARIZER_SEARCH_TOOL),
            project_search_description: Some(
                "检索当前项目的正文与已确认知识，为四层总结补充跨章节上下文。",
            ),
            web_search_tool: Some(SUMMARIZER_WEB_SEARCH_TOOL),
            web_search_description: Some("搜索公开互联网，为总结中涉及的现实事实提供外部核对。"),
        }
    }

    pub const fn supports_model_tools(self) -> bool {
        matches!(
            self.execution_kind,
            WorkflowNodeExecutionKind::Model | WorkflowNodeExecutionKind::Summarizer
        )
    }
}

/// 不属于画布节点、但与模型节点共用 Search/Web Search 协议的执行目标。
/// 权限默认值、依赖编译与命令装配必须消费这两个描述符，不能再各写一份字符串。
pub const PROJECT_AI_TOOL_CAPABILITY: WorkflowNodeToolCapability =
    WorkflowNodeToolCapability::model_in_scope(
        "project_ai",
        "project_ai",
        PROJECT_AI_SEARCH_TOOL,
        "检索当前项目文档、规划与已确认知识。回答项目事实前应优先使用本工具核对。",
        PROJECT_AI_WEB_SEARCH_TOOL,
        "搜索公开互联网，返回标题、URL 与摘要；结果不自动写入项目知识库。",
    );

pub const EXECUTOR_ADAPTER_TOOL_CAPABILITY: WorkflowNodeToolCapability =
    WorkflowNodeToolCapability::model_in_scope(
        "executor_adapter",
        "executor_adapter",
        EXECUTOR_ADAPTER_SEARCH_TOOL,
        "检索当前项目文档与已确认知识，为 LLM ExecutorAdapter 补充项目上下文。",
        EXECUTOR_ADAPTER_WEB_SEARCH_TOOL,
        "搜索公开互联网，为 LLM ExecutorAdapter 补充外部资料。",
    );

/// 通用节点能力矩阵。确定性节点也登记在册，但只有真实模型/工具调用路径的节点
/// 声明 search/web_search；运行时、权限默认值和验收测试共用这一事实源。
pub const WORKFLOW_NODE_CAPABILITIES: &[WorkflowNodeToolCapability] = &[
    WorkflowNodeToolCapability::builtin("start"),
    WorkflowNodeToolCapability::builtin("document_read"),
    WorkflowNodeToolCapability::builtin("condition"),
    WorkflowNodeToolCapability::builtin("loop"),
    WorkflowNodeToolCapability::builtin("approval"),
    WorkflowNodeToolCapability::builtin("export"),
    WorkflowNodeToolCapability::builtin("search"),
    WorkflowNodeToolCapability::model(
        "llm",
        GENERIC_LLM_SEARCH_TOOL,
        "检索当前项目文档与已确认知识，为回答补充可追溯的项目事实。",
        GENERIC_LLM_WEB_SEARCH_TOOL,
        "搜索公开互联网，为回答补充时效性资料；返回标题、URL 与摘要，不自动写入项目知识库。",
    ),
    WorkflowNodeToolCapability::model(
        "outliner",
        OUTLINER_SEARCH_TOOL,
        "检索当前项目的正文、规划与已确认知识，为全局总纲补充项目事实。",
        OUTLINER_WEB_SEARCH_TOOL,
        "搜索公开互联网，为全局规划进行现实资料考据。",
    ),
    WorkflowNodeToolCapability::model(
        "designer",
        DESIGNER_SEARCH_TOOL,
        "检索当前项目的正文、规划与已确认知识，为阶段设计补充项目事实。",
        DESIGNER_WEB_SEARCH_TOOL,
        "搜索公开互联网，为阶段设计进行现实资料考据。",
    ),
    WorkflowNodeToolCapability::model(
        "planner",
        PLANNER_SEARCH_TOOL,
        "检索当前项目的正文、规划与已确认知识，为章节规划补充项目事实。",
        PLANNER_WEB_SEARCH_TOOL,
        "搜索公开互联网，为章节规划进行现实资料考据。",
    ),
    WorkflowNodeToolCapability::model(
        "detail",
        DETAIL_SEARCH_TOOL,
        "检索当前项目的正文、规划与已确认知识，为细节生成补充上下文。",
        DETAIL_WEB_SEARCH_TOOL,
        "搜索公开互联网，为环境、心理或设定细节补充现实资料。",
    ),
    WorkflowNodeToolCapability::model(
        "writer",
        WRITER_SEARCH_TOOL,
        "检索当前项目的前文、规划与已确认知识，保持正文连续性和设定一致性。",
        WRITER_WEB_SEARCH_TOOL,
        "搜索公开互联网，核对当前写作位置涉及的现实情况。",
    ),
    WorkflowNodeToolCapability::model(
        "critic",
        CRITIC_SEARCH_TOOL,
        "检索当前项目的正文、规划与已确认知识，为审稿判断提供依据。",
        CRITIC_WEB_SEARCH_TOOL,
        "搜索公开互联网，为合理性与事实性审稿提供外部依据。",
    ),
    WorkflowNodeToolCapability::model(
        "prudent",
        PRUDENT_SEARCH_TOOL,
        "检索当前项目的正文、规划与已确认知识，为审慎判断提供依据。",
        PRUDENT_WEB_SEARCH_TOOL,
        "搜索公开互联网，复核意见者引用的现实事实。",
    ),
    WorkflowNodeToolCapability::model(
        "polisher",
        POLISHER_SEARCH_TOOL,
        "检索当前项目的前文、规划与已确认知识，为有限返修提供上下文。",
        POLISHER_WEB_SEARCH_TOOL,
        "搜索公开互联网，为有限返修核对现实资料。",
    ),
    WorkflowNodeToolCapability::summarizer(),
];

pub fn workflow_node_capability(node_type: &str) -> Option<&'static WorkflowNodeToolCapability> {
    WORKFLOW_NODE_CAPABILITIES.iter().find(|capability| {
        capability.node_type == node_type
            || workflow_node_catalog_entry(node_type)
                .is_some_and(|entry| entry.node_type == capability.node_type)
    })
}

/// 将动态 ExecutorAdapter 类型与普通画布节点统一映射到能力事实源。
pub fn execution_tool_capability(node_type: &str) -> Option<&'static WorkflowNodeToolCapability> {
    if node_type.starts_with(EXECUTOR_ADAPTER_NODE_PREFIX) {
        return Some(&EXECUTOR_ADAPTER_TOOL_CAPABILITY);
    }
    workflow_node_capability(node_type)
}

pub fn model_tool_node_capabilities() -> impl Iterator<Item = &'static WorkflowNodeToolCapability> {
    WORKFLOW_NODE_CAPABILITIES
        .iter()
        .filter(|capability| capability.supports_model_tools())
}

/// 所有需要落入权限配置的模型工具作用域，包括项目 AI 和 ExecutorAdapter。
pub fn permission_tool_capabilities() -> impl Iterator<Item = &'static WorkflowNodeToolCapability> {
    [
        &PROJECT_AI_TOOL_CAPABILITY,
        &EXECUTOR_ADAPTER_TOOL_CAPABILITY,
    ]
    .into_iter()
    .chain(model_tool_node_capabilities())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn general_node_capability_matrix_has_unique_node_types() {
        let mut node_types = BTreeSet::new();
        for capability in WORKFLOW_NODE_CAPABILITIES {
            assert!(
                node_types.insert(capability.node_type),
                "duplicate node capability: {}",
                capability.node_type
            );
        }
    }

    #[test]
    fn every_model_tool_node_declares_independent_search_tools() {
        for capability in permission_tool_capabilities() {
            let project_search = capability
                .project_search_tool
                .expect("model node must declare project search");
            let web_search = capability
                .web_search_tool
                .expect("model node must declare web search");
            assert_ne!(project_search, web_search);
            assert!(capability.project_search_description.is_some());
            assert!(capability.web_search_description.is_some());
        }
    }

    #[test]
    fn dynamic_executor_adapter_uses_shared_tool_capability() {
        assert_eq!(
            execution_tool_capability("executor_adapter:custom"),
            Some(&EXECUTOR_ADAPTER_TOOL_CAPABILITY)
        );
        assert_eq!(
            execution_tool_capability("writer"),
            workflow_node_capability("writer")
        );
    }

    #[test]
    fn shipped_catalog_has_unique_primary_types_aliases_and_presets() {
        let mut node_types = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        let mut preset_types = BTreeSet::new();
        for entry in workflow_node_catalog() {
            assert!(node_types.insert(entry.node_type.as_str()));
            assert!(preset_types.insert(entry.preset_type.as_str()));
            assert!(!entry.display_name_key.trim().is_empty());
            assert!(!entry.config_kind.trim().is_empty());
            for alias in &entry.aliases {
                assert!(aliases.insert(alias.as_str()));
                assert!(!node_types.contains(alias.as_str()));
            }
        }
    }

    #[test]
    fn runtime_tool_matrix_matches_the_shipped_product_catalog() {
        for capability in WORKFLOW_NODE_CAPABILITIES {
            let entry = workflow_node_catalog_entry(capability.node_type)
                .unwrap_or_else(|| panic!("catalog missing node type {}", capability.node_type));
            assert_eq!(entry.execution_kind, capability.execution_kind);
            assert_eq!(
                entry.project_search_tool.as_deref(),
                capability.project_search_tool,
                "project search drift for {}",
                capability.node_type
            );
            assert_eq!(
                entry.web_search_tool.as_deref(),
                capability.web_search_tool,
                "web search drift for {}",
                capability.node_type
            );
        }
        for entry in workflow_node_catalog() {
            assert!(workflow_node_capability(&entry.node_type).is_some());
        }
    }
}
