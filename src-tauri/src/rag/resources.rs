use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// prompt_list.json 的单条提示词资源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptResource {
    pub prompt: String,
    pub describe: String,
}

/// 内置提示词资源集合。
pub type PromptResources = BTreeMap<String, PromptResource>;

/// 内置显示名称资源集合。
pub type DisplayNameResources = BTreeMap<String, String>;

const PROMPT_LIST_JSON: &str = include_str!("../../resources/prompt_list.json");
const DISPLAY_NAME_JSON: &str = include_str!("../../resources/display_name.json");

/// 加载内置提示词资源。
pub fn load_prompt_resources() -> crate::core::CoreResult<PromptResources> {
    let resources = serde_json::from_str::<PromptResources>(PROMPT_LIST_JSON)?;
    validate_prompt_resources(&resources)?;
    Ok(resources)
}

/// 加载内置显示名称资源。
pub fn load_display_name_resources() -> crate::core::CoreResult<DisplayNameResources> {
    let resources = serde_json::from_str::<DisplayNameResources>(DISPLAY_NAME_JSON)?;
    validate_display_name_resources(&resources)?;
    Ok(resources)
}

/// 校验提示词资源的必需 key 和字段。
pub fn validate_prompt_resources(resources: &PromptResources) -> crate::core::CoreResult<()> {
    for key in [
        "tool.planner_register",       // planner-register 工具提示词
        "tool.planner_find",           // planner-find 工具提示词
        "tool.planner_search",         // planner-search 工具提示词
        "tool.detail_find",            // detail-find 工具提示词
        "tool.detail_search",          // detail-search 工具提示词
        "tool.writer_find",            // writer-find 工具提示词
        "tool.writer_search",          // writer-search 工具提示词
        "tool.writer_insert_lines",    // writer-insert-lines 工具提示词
        "tool.writer_replace_lines",   // writer-replace-lines 工具提示词
        "auto_audit.register",         // register 自动审计提示词
        "auto_audit.summary",          // summary 自动审计提示词
        "auto_audit.correction_patch", // 自动修正 patch 审计提示词
        "summarizer.segments",         // 故事段总结提示词
        "summarizer.events",           // 事件总结提示词
        "summarizer.chapter_summary",  // 章节总结提示词
        "summarizer.stage_summary",    // 阶段总结提示词
    ] {
        let Some(resource) = resources.get(key) else {
            return Err(crate::core::CoreError::validation(format!(
                "missing prompt resource: {key}"
            )));
        };
        if resource.prompt.trim().is_empty() || resource.describe.trim().is_empty() {
            return Err(crate::core::CoreError::validation(format!(
                "prompt resource fields cannot be empty: {key}"
            )));
        }
    }

    Ok(())
}

/// 校验显示名称资源的必需 key 和字段。
pub fn validate_display_name_resources(
    resources: &DisplayNameResources,
) -> crate::core::CoreResult<()> {
    for key in [
        "agent.planner",             // PlannerAgent 显示名
        "agent.detail",              // DetailAgent 显示名
        "agent.writer",              // WriterAgent 显示名
        "agent.summarizer",          // SummarizerAgent 显示名
        "tool.planner-register",     // planner-register 显示名
        "tool.planner-find",         // planner-find 显示名
        "tool.planner-search",       // planner-search 显示名
        "tool.detail-find",          // detail-find 显示名
        "tool.detail-search",        // detail-search 显示名
        "tool.writer-find",          // writer-find 显示名
        "tool.writer-search",        // writer-search 显示名
        "tool.writer-insert-lines",  // writer-insert-lines 显示名
        "tool.writer-replace-lines", // writer-replace-lines 显示名
        "confirmation.planner.register.character_trait",
        "confirmation.planner.register.relationship",
        "confirmation.planner.register.foreshadowing",
        "confirmation.summarizer.segment",
        "confirmation.summarizer.event",
        "confirmation.summarizer.chapter",
        "confirmation.summarizer.stage",
        "confirmation.writer.correction_patch",
    ] {
        let Some(value) = resources.get(key) else {
            return Err(crate::core::CoreError::validation(format!(
                "missing display name resource: {key}"
            )));
        };
        if value.trim().is_empty() {
            return Err(crate::core::CoreError::validation(format!(
                "display name cannot be empty: {key}"
            )));
        }
    }

    Ok(())
}
