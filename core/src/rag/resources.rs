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
/// U201-C：en/ja 叠加层也要内联进来。
///
/// 此前只内联了 zh 一份，因为后端只用 display_name 做**校验**（键存在性），
/// 而校验中文基底就够。现在多了一项职责：默认提示词占位符的**接受集合**要覆盖
/// 三种语言写法的并集（理由见 `rag/default_prompt.rs` 模块头——作者照抄的是屏幕
/// 上那一行，英文界面抄英文写法，只认一种等于不认他抄的东西），而并集必须从语言
/// 包现算、不能硬编码，否则 en/ja 补译之后表会漏。
///
/// 这两份是**叠加层**（缺 key 时前端回落中文），所以它们的键集合是 zh 的子集，
/// 不参与 `validate_display_name_resources` 的必需键校验。
const DISPLAY_NAME_EN_JSON: &str = include_str!("../../resources/display_name.en.json");
const DISPLAY_NAME_JA_JSON: &str = include_str!("../../resources/display_name.ja.json");

/// 加载内置提示词资源。
pub fn load_prompt_resources() -> crate::contracts::CoreResult<PromptResources> {
    let resources = serde_json::from_str::<PromptResources>(PROMPT_LIST_JSON)?;
    validate_prompt_resources(&resources)?;
    Ok(resources)
}

/// 加载内置显示名称资源。
pub fn load_display_name_resources() -> crate::contracts::CoreResult<DisplayNameResources> {
    let resources = serde_json::from_str::<DisplayNameResources>(DISPLAY_NAME_JSON)?;
    validate_display_name_resources(&resources)?;
    Ok(resources)
}

/// U201-C：加载全部语言的显示名资源（zh 基底 + en/ja 叠加层）。
///
/// 供「解析宽容、生成唯一」的接受集合建表用（`rag/default_prompt.rs`）。
///
/// # 为什么是 fail-loud（而不是跳过坏掉的那一份）
///
/// 首版用的是 `filter_map(...ok())`，那是错的：某份语言包 JSON 坏掉时并集会
/// **静默少一种写法**，症状回到「那门语言界面里手打的写法认不出」——
/// 而这正是整条功能最怕、也最难查的那一类失败（界面不报错，只是节点跑不起来，
/// 且错误信息指向作者没写过的一行）。
///
/// 改成 fail-loud 在这里**不花任何生产代价**：三份都是 `include_str!` 的
/// 编译期常量，内容对每个用户、每次运行完全相同 ⇒ 坏掉是「我们发了个坏二进制」，
/// 不是用户环境的偶发状况，测试套件在发版前就会红。相比之下静默降级的代价是
/// 真实的（上面那类失败）。
///
/// 口径因此与同模块的 `load_display_name_resources` 一致（都 `?` + 校验），
/// 不再是一宽一严两套。
pub fn all_display_name_packs() -> crate::contracts::CoreResult<Vec<DisplayNameResources>> {
    [
        ("display_name.json", DISPLAY_NAME_JSON),
        ("display_name.en.json", DISPLAY_NAME_EN_JSON),
        ("display_name.ja.json", DISPLAY_NAME_JA_JSON),
    ]
    .into_iter()
    .map(|(name, raw)| {
        serde_json::from_str::<DisplayNameResources>(raw).map_err(|error| {
            crate::contracts::CoreError::validation(format!(
                "bundled display name pack {name} is not a flat string map: {error}"
            ))
        })
    })
    .collect()
}

/// 校验提示词资源的必需 key 和字段。
pub fn validate_prompt_resources(resources: &PromptResources) -> crate::contracts::CoreResult<()> {
    for key in [
        "agent_prompt.outliner",   // Outliner 节点提示词
        "agent_prompt.designer",   // Designer 节点提示词
        "agent_prompt.planner",    // Planner 节点提示词
        "agent_prompt.detail",     // Detail 节点提示词
        "agent_prompt.writer",     // Writer 节点提示词
        "agent_prompt.critic",     // Critic 节点提示词
        "agent_prompt.prudent",    // Prudent 节点提示词
        "agent_prompt.polisher",   // Polisher 节点提示词
        "agent_prompt.summarizer", // Summarizer 节点提示词
        "node_template.outliner.default",
        "node_template.designer.default",
        "node_template.planner.default",
        "node_template.detail.default",
        "node_template.writer.default",
        "node_template.critic.default",
        "node_template.prudent.default",
        "node_template.polisher.default",
        "node_template.summarizer.default",
        "tool.outliner_register", // outliner-register 工具提示词
        "tool.outliner_find",     // outliner-find 工具提示词
        "tool.outliner_search",   // outliner-search 工具提示词
        "tool.outliner_web_search",
        "tool.outliner_insert_lines", // outliner-insert-lines 工具提示词
        "tool.outliner_replace_lines", // outliner-replace-lines 工具提示词
        "tool.outliner_rewrite_file", // outliner-rewrite-file 工具提示词
        "tool.designer_register",     // designer-register 工具提示词
        "tool.designer_find",         // designer-find 工具提示词
        "tool.designer_search",       // designer-search 工具提示词
        "tool.designer_web_search",
        "tool.designer_insert_lines", // designer-insert-lines 工具提示词
        "tool.designer_replace_lines", // designer-replace-lines 工具提示词
        "tool.designer_rewrite_file", // designer-rewrite-file 工具提示词
        "tool.planner_register",      // planner-register 工具提示词
        "tool.planner_find",          // planner-find 工具提示词
        "tool.planner_search",        // planner-search 工具提示词
        "tool.planner_web_search",
        "tool.planner_insert_lines",  // planner-insert-lines 工具提示词
        "tool.planner_replace_lines", // planner-replace-lines 工具提示词
        "tool.planner_rewrite_file",  // planner-rewrite-file 工具提示词
        "tool.detail_find",           // detail-find 工具提示词
        "tool.detail_search",         // detail-search 工具提示词
        "tool.detail_web_search",
        "tool.writer_find",   // writer-find 工具提示词
        "tool.writer_search", // writer-search 工具提示词
        "tool.writer_web_search",
        "tool.writer_insert_lines",  // writer-insert-lines 工具提示词
        "tool.writer_replace_lines",
        "tool.writer_rewrite_file", // U123：整章重写 // writer-replace-lines 工具提示词
        "tool.critic_find",          // critic-find 工具提示词
        "tool.critic_search",        // critic-search 工具提示词
        "tool.critic_web_search",
        "tool.prudent_find",   // prudent-find 工具提示词
        "tool.prudent_search", // prudent-search 工具提示词
        "tool.prudent_web_search",
        "tool.polisher_find",   // polisher-find 工具提示词
        "tool.polisher_search", // polisher-search 工具提示词
        "tool.polisher_web_search",
        "tool.polisher_insert_lines", // polisher-insert-lines 工具提示词
        "tool.polisher_replace_lines",
        "tool.polisher_rewrite_file", // U123：整章重写 // polisher-replace-lines 工具提示词
        "tool.summarizer_search",     // summarizer-search 工具提示词
        "tool.summarizer_web_search",
        "auto_audit.planning_output",  // 规划节点输出自动审计提示词
        "auto_audit.register",         // register 自动审计提示词
        "auto_audit.review",           // 审稿节点输出自动审计提示词
        "auto_audit.summary",          // summary 自动审计提示词
        "auto_audit.correction_patch", // 自动修正 patch 审计提示词
        "auto_audit.chapter_write",
        "auto_audit.summary_write",
        "auto_audit.high_risk_permission",
        "auto_audit.budget_exceeded",
        "auto_audit.generic",
        "summarizer.segments",        // 故事段总结提示词
        "summarizer.events",          // 事件总结提示词
        "summarizer.chapter_summary", // 章节总结提示词
        "summarizer.stage_summary",   // 阶段总结提示词
    ] {
        let Some(resource) = resources.get(key) else {
            return Err(crate::contracts::CoreError::validation(format!(
                "missing prompt resource: {key}"
            )));
        };
        if resource.prompt.trim().is_empty() || resource.describe.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(format!(
                "prompt resource fields cannot be empty: {key}"
            )));
        }
    }

    Ok(())
}

/// 校验显示名称资源的必需 key 和字段。
pub fn validate_display_name_resources(
    resources: &DisplayNameResources,
) -> crate::contracts::CoreResult<()> {
    for code in crate::command_error::CommandErrorCode::ALL {
        let key = code.message_key();
        let Some(value) = resources.get(&key) else {
            return Err(crate::contracts::CoreError::validation(format!(
                "missing command error display resource: {key}"
            )));
        };
        if value.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(format!(
                "command error display resource cannot be empty: {key}"
            )));
        }
    }

    for key in [
        "agent.outliner",         // OutlinerAgent 显示名
        "agent.designer",         // DesignerAgent 显示名
        "agent.planner",          // PlannerAgent 显示名
        "agent.detail",           // DetailAgent 显示名
        "agent.writer",           // WriterAgent 显示名
        "agent.critic",           // CriticAgent 显示名
        "agent.prudent",          // PrudentAgent 显示名
        "agent.polisher",         // PolisherAgent 显示名
        "agent.summarizer",       // SummarizerAgent 显示名
        "tool.outliner-register", // outliner-register 显示名
        "tool.outliner-find",     // outliner-find 显示名
        "tool.outliner-search",   // outliner-search 显示名
        "tool.outliner-web-search",
        "tool.outliner-insert-lines",
        "tool.outliner-replace-lines",
        "tool.outliner-rewrite-file",
        "tool.designer-register", // designer-register 显示名
        "tool.designer-find",     // designer-find 显示名
        "tool.designer-search",   // designer-search 显示名
        "tool.designer-web-search",
        "tool.designer-insert-lines",
        "tool.designer-replace-lines",
        "tool.designer-rewrite-file",
        "tool.planner-register", // planner-register 显示名
        "tool.planner-find",     // planner-find 显示名
        "tool.planner-search",   // planner-search 显示名
        "tool.planner-web-search",
        "tool.planner-insert-lines",
        "tool.planner-replace-lines",
        "tool.planner-rewrite-file",
        "tool.detail-find",   // detail-find 显示名
        "tool.detail-search", // detail-search 显示名
        "tool.detail-web-search",
        "tool.writer-find",   // writer-find 显示名
        "tool.writer-search", // writer-search 显示名
        "tool.writer-web-search",
        "tool.writer-insert-lines",  // writer-insert-lines 显示名
        "tool.writer-replace-lines", // writer-replace-lines 显示名
        "tool.critic-find",          // critic-find 显示名
        "tool.critic-search",        // critic-search 显示名
        "tool.critic-web-search",
        "tool.prudent-find",   // prudent-find 显示名
        "tool.prudent-search", // prudent-search 显示名
        "tool.prudent-web-search",
        "tool.polisher-find",   // polisher-find 显示名
        "tool.polisher-search", // polisher-search 显示名
        "tool.polisher-web-search",
        "tool.polisher-insert-lines",
        "tool.polisher-replace-lines",
        "tool.summarizer-search",
        "tool.summarizer-web-search",
        "confirmation.outliner.output",
        "confirmation.designer.output",
        "confirmation.planner.output",
        "confirmation.planner.register.character_trait",
        "confirmation.planner.register.relationship",
        "confirmation.planner.register.foreshadowing",
        "confirmation.critic.review",
        "confirmation.prudent.review",
        "confirmation.summarizer.segment",
        "confirmation.summarizer.event",
        "confirmation.summarizer.chapter",
        "confirmation.summarizer.stage",
        "confirmation.writer.correction_patch",
        "confirmation.polisher.correction_patch",
        // U201-C：节点默认提示词占位符的字面量。缺一个 = 那个 agent 的占位符
        // 在**所有**语言里都认不出（并集里没有它），而症状是「节点报未展开占位符」
        // 而非「少一行文案」——所以按必需键校验，让缺失在启动时就暴露。
        "ui.prompt.default_placeholder.outliner",
        "ui.prompt.default_placeholder.designer",
        "ui.prompt.default_placeholder.planner",
        "ui.prompt.default_placeholder.detail",
        "ui.prompt.default_placeholder.writer",
        "ui.prompt.default_placeholder.critic",
        "ui.prompt.default_placeholder.prudent",
        "ui.prompt.default_placeholder.polisher",
        "ui.prompt.default_placeholder.summarizer",
    ] {
        let Some(value) = resources.get(key) else {
            return Err(crate::contracts::CoreError::validation(format!(
                "missing display name resource: {key}"
            )));
        };
        if value.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(format!(
                "display name cannot be empty: {key}"
            )));
        }
    }

    Ok(())
}
