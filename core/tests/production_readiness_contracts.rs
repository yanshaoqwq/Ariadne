//! 生产可用性契约（2026-07-26 配置项审查 U107–U112）。
//!
//! 本文件的每个用例都断言**期望的正确行为**，因此在缺陷修复前会失败。
//! 失败即证明对应缺陷存在；修复后自动转绿，可直接作为回归套件。
//!
//! 详细分析见 `项目检验报告/发布前全量代码审查/13-配置项存在性与执行链路阻断审查.md`。

use ariadne::commands::{save_provider_settings_impl, ProviderSettingsUpdate};
use ariadne::config::{ConfigStore, ModelConfig, RagConfig, WorkflowConfig};
use ariadne::contracts::{ProviderCapability, ProviderType};
use ariadne::costs::{evaluate_budget, BudgetAction, BudgetLimits, BudgetUsage};

/// 构造一个最小可用的模型配置。
fn model(model_id: &str, capability: ProviderCapability) -> ModelConfig {
    ModelConfig {
        model_id: model_id.to_owned(),
        capability,
        max_context_tokens: None,
        input_cost_per_million_tokens: None,
        output_cost_per_million_tokens: None,
    }
}

// ————————————————————————————————————————————————
// U107：能力下拉框提供 tool_use，保存边界却拒绝它
// ————————————————————————————————————————————————

/// 设置页能力下拉框第 2 项就是 `tool_use`（工具调用），用户选中后必须能保存。
///
/// 当前 `ModelConfig::validate_provider_model_role()` 只放行
/// Llm/Embedding/Reranker/Search，因此该用例失败即证明 U107。
#[test]
fn u107_tool_use_model_capability_is_accepted_by_save_boundary() {
    let tool_use_model = model("gpt-4o", ProviderCapability::ToolUse);

    // 基础校验本来就通过，问题只出在保存边界的角色校验。
    assert!(
        tool_use_model.validate().is_ok(),
        "tool_use 模型的基础校验不应失败"
    );

    assert!(
        tool_use_model.validate_provider_model_role().is_ok(),
        "U107：设置页允许选择 tool_use，保存边界却拒绝它，用户无法保存合法配置"
    );
}

/// 端到端复现：把一个 tool_use 模型经真实保存命令写入项目配置。
#[test]
fn u107_provider_with_tool_use_model_can_be_saved_end_to_end() {
    let temp_dir = tempfile::tempdir().unwrap();
    let project_root = temp_dir.path();
    ConfigStore::new(project_root).load_or_create().unwrap();

    let update = ProviderSettingsUpdate {
        provider_id: "openai".to_owned(),
        provider_type: ProviderType::OpenAi,
        display_name: "OpenAI".to_owned(),
        enabled: true,
        base_url: None,
        // 用户在 UI 上给同一 Provider 配了一个纯文本模型和一个工具调用模型。
        models: vec![
            model("gpt-4o-mini", ProviderCapability::Llm),
            model("gpt-4o", ProviderCapability::ToolUse),
        ],
        make_default_llm: false,
        make_default_embedding: false,
        make_default_reranker: false,
        make_default_search: false,
    };

    let result = save_provider_settings_impl(project_root, update);
    assert!(
        result.is_ok(),
        "U107：含 tool_use 模型的 Provider 无法保存，错误：{:?}",
        result.err()
    );
}

/// 老项目若已存 tool_use 模型，对该 Provider 的任何后续修改都会被同一校验拖垮。
/// 这会让设置页对该 Provider 完全锁死。
#[test]
fn u107_existing_tool_use_model_does_not_lock_the_whole_provider() {
    let temp_dir = tempfile::tempdir().unwrap();
    let project_root = temp_dir.path();
    let store = ConfigStore::new(project_root);
    let mut config = store.load_or_create().unwrap();

    // 模拟历史配置：models.rs 的注释明确承认旧配置可读取 tool_use。
    config.providers.providers.push(ariadne::config::ProviderConfig {
        provider_id: "legacy".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Legacy".to_owned(),
        enabled: true,
        base_url: Some("https://api.example.com".to_owned()),
        api_key: None,
        models: vec![model("legacy-tool-model", ProviderCapability::ToolUse)],
    });
    store.save(&config).unwrap();

    // 用户只想改个显示名，模型清单原样回传。
    let update = ProviderSettingsUpdate {
        provider_id: "legacy".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Legacy Renamed".to_owned(),
        enabled: true,
        base_url: Some("https://api.example.com".to_owned()),
        models: vec![model("legacy-tool-model", ProviderCapability::ToolUse)],
        make_default_llm: false,
        make_default_embedding: false,
        make_default_reranker: false,
        make_default_search: false,
    };

    let result = save_provider_settings_impl(project_root, update);
    assert!(
        result.is_ok(),
        "U107：旧配置含 tool_use 模型时，改个显示名都存不了，设置页对该 Provider 锁死：{:?}",
        result.err()
    );
}

// ————————————————————————————————————————————————
// U109：reranker_enabled 缺少前置校验
// ————————————————————————————————————————————————

/// 开启重排序但没有可用的 reranker 默认路由时，必须在**保存边界**拒绝，
/// 而不是等到运行时构造 `ProjectRetrievalRuntime` 才失败。
///
/// 后者会连带击穿向量与全文检索（`retrieval/project.rs` 的 `?` 位于运行时构造过程中），
/// 用户看到的错误与“重排序”毫无表面关联。
///
/// 注意校验层级：`RagConfig::validate()` 拿不到 `ProvidersConfig`，
/// 因此正确的落点是命令边界 `save_rag_settings_impl`，本用例据此断言。
#[test]
fn u109_enabling_reranker_without_provider_is_rejected_at_save() {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path();
    ConfigStore::new(project_root).load_or_create().unwrap();

    let mut rag = RagConfig::default();
    rag.reranker_enabled = true;

    let result = ariadne::commands::save_rag_settings_impl(
        project_root,
        ariadne::commands::RagSettings {
            rag,
            qdrant_api_key: None,
            clear_qdrant_api_key: false,
            has_qdrant_api_key: false,
        },
    );

    assert!(
        result.is_err(),
        "U109：开启 reranker 却未配置 reranker Provider 时，保存边界必须 fail-loud，\
         否则运行时会连带击穿整个检索运行时"
    );
}

/// 修复 U109 时不得矫枉过正：历史上已经开着 reranker 的项目，
/// 用户改动其它检索设置（如 chunk 大小）时不应被追加阻断，否则设置页会被锁死。
#[test]
fn u109_fix_does_not_lock_projects_that_already_enabled_reranker() {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path();
    let store = ConfigStore::new(project_root);

    // 历史状态：reranker 早已开启（且当时未配路由）。
    let mut config = store.load_or_create().unwrap();
    config.rag.reranker_enabled = true;
    store.save(&config).unwrap();

    // 用户这次只是调了 chunk 大小，没有重新打开 reranker。
    let mut rag = RagConfig::default();
    rag.reranker_enabled = true;
    rag.chunk_size_chars = 1_500;

    let result = ariadne::commands::save_rag_settings_impl(
        project_root,
        ariadne::commands::RagSettings {
            rag,
            qdrant_api_key: None,
            clear_qdrant_api_key: false,
            has_qdrant_api_key: false,
        },
    );

    assert!(
        result.is_ok(),
        "U109 的修复不应阻断历史配置的无关改动，否则设置页被锁死：{:?}",
        result.err()
    );
}

// ————————————————————————————————————————————————
// U111：假开关——后端零消费点
// ————————————————————————————————————————————————

/// `WorkflowConfig::validate()` 校验了 timeout / loop / tool_rounds 三项，
/// 唯独漏掉 `runtime_autosave_ms`。零值自动保存间隔必须被拒绝。
///
/// 该字段当前在后端**完全没有消费点**（全仓 grep 仅命中定义、默认值与一处测试赋值），
/// 补校验只是接线前的第一步。
#[test]
fn u111_zero_runtime_autosave_interval_is_rejected() {
    let mut workflow = WorkflowConfig::default();
    workflow.runtime_autosave_ms = 0;

    assert!(
        workflow.validate().is_err(),
        "U111：runtime_autosave_ms = 0 必须被拒绝；\
         该字段目前既无校验也无任何后端消费点，设置页却提供输入框"
    );
}

// ————————————————————————————————————————————————
// U113：工作流全局限制未接线（超时与循环上限）
// ————————————————————————————————————————————————

/// 设置页「自动化」分区并排三个工作流限制输入框，但只有 `max_tool_rounds` 真正生效。
///
/// `default_timeout_ms` 与 `max_loop_iterations` 唯一的可达路径是
/// `WorkflowConfig::validate_loop_policy`，而该函数在全仓**零调用者**，
/// 故两项配置从未参与任何执行判定。
///
/// 本用例用一个**明显违规**的 loop policy 作探针：若全局限制真的生效，
/// 超出上限的 policy 必须被拒绝。
#[test]
fn u113_global_loop_limit_actually_constrains_node_policies() {
    let workflow = WorkflowConfig {
        max_loop_iterations: 5,
        ..WorkflowConfig::default()
    };

    // 节点声明 999 轮，远超全局上限 5。
    let runaway = ariadne::contracts::LoopPolicy {
        max_iterations: 999,
        timeout_ms: 60_000,
        budget_limit_usd: None,
        stop_condition: serde_json::json!({"kind": "manual"}),
    };

    assert!(
        workflow.validate_loop_policy(&runaway).is_err(),
        "全局循环上限必须能拒绝越界的节点 policy"
    );

    // 上面的断言即使通过也只证明函数本身可用；真正的问题是它没有被接线。
    // 运行时 (`workflow/runtime.rs`) 判定用的是节点自带的 `policy.max_iterations`，
    // 全局上限从未参与，因此 999 轮循环会照跑 999 轮。
    // 该缺陷无法用纯配置层单测覆盖，接线后应补一条工作流执行层用例。
}

/// 节点超时的实际默认值必须与用户在设置页看到的默认值一致。
///
/// 当前 `workflow/integration.rs` 的 `resolve_node_timeout_ms` 硬编码回落 120_000ms，
/// 而 `WorkflowConfig::default().default_timeout_ms` 是 300_000ms。
/// 用户看到「默认超时 300 秒」，实际未配置的节点按 120 秒超时。
#[test]
fn u113_node_timeout_fallback_matches_configured_default() {
    // 与 workflow/integration.rs:809 `resolve_node_timeout_ms` 的硬编码值保持同步。
    const HARDCODED_NODE_TIMEOUT_FALLBACK_MS: u64 = 120_000;

    assert_eq!(
        WorkflowConfig::default().default_timeout_ms,
        HARDCODED_NODE_TIMEOUT_FALLBACK_MS,
        "U113：设置页展示的默认超时（{}ms）与运行时实际回落值（{}ms）不一致；\
         且 workflow.default_timeout_ms 在后端零消费点，改它不产生任何效果",
        WorkflowConfig::default().default_timeout_ms,
        HARDCODED_NODE_TIMEOUT_FALLBACK_MS
    );
}

// ————————————————————————————————————————————————
// U112：预授权预算 0 值语义
// ————————————————————————————————————————————————

/// 同一设置分区内两个相邻的金额输入框，对 `0` 的解释必须一致。
///
/// 当前：全局预算 `0` = 不限制（见 `budget_limits_from_global_budget` 文档注释），
/// 预授权预算 `0` = 零额度并暂停一切调用。样式、单位、位置都相同，含义却相反。
#[test]
fn u112_zero_means_the_same_thing_for_both_budget_fields() {
    let limits = ariadne::costs::budget_limits_from_global_budget(0.0);
    let global_zero_means_unlimited = limits.daily_usd.is_none();

    let auto_mode = ariadne::config::AutoModeConfig {
        enabled_by_default: true,
        preauthorized_budget_usd: Some(0.0),
        ..ariadne::config::AutoModeConfig::default()
    };
    let decision = evaluate_budget(
        &BudgetLimits::default(),
        &auto_mode,
        BudgetUsage {
            requested_usd: 0.01,
            spent_today_usd: 0.0,
            spent_this_month_usd: 0.0,
        },
    );
    let preauthorized_zero_means_unlimited = decision.action != BudgetAction::Pause;

    assert_eq!(
        global_zero_means_unlimited, preauthorized_zero_means_unlimited,
        "U112：全局预算的 0 表示『不限制』，预授权预算的 0 却表示『零额度、全部暂停』；\
         两个相邻输入框对同一个数字给出相反语义"
    );
}

/// 后端默认 `preauthorized_budget_usd = None`（不限制）。
/// 读取侧不得把 `None` 折叠成 `0` 显示——否则用户保存任意无关设置时，
/// 该 `0` 会被回写为 `Some(0.0)`，静默把“不限制”翻转成“全部暂停”。
#[test]
fn u112_unset_preauthorized_budget_does_not_block_auto_mode() {
    let auto_mode = ariadne::config::AutoModeConfig {
        enabled_by_default: true,
        // 新建项目的真实默认值。
        preauthorized_budget_usd: None,
        ..ariadne::config::AutoModeConfig::default()
    };

    let decision = evaluate_budget(
        &BudgetLimits::default(),
        &auto_mode,
        BudgetUsage {
            requested_usd: 0.01,
            spent_today_usd: 0.0,
            spent_this_month_usd: 0.0,
        },
    );

    assert_ne!(
        decision.action,
        BudgetAction::Pause,
        "未设置预授权预算的新项目不应被暂停：{:?}",
        decision.reason
    );
}

/// 语义回归护栏：把 `None` 经“显示为 0 → 原样保存”的往返后，
/// Auto Mode 的行为必须与往返前一致。
#[test]
fn u112_display_save_roundtrip_preserves_unlimited_semantics() {
    let stored: Option<f64> = None;

    // 读取侧当前实现：commands.rs 的 `.unwrap_or(0.0)`。
    let displayed = stored.unwrap_or(0.0);
    // 保存侧当前实现：update_budget_config_impl 的 `Some(preauthorized_usd)`。
    let saved_back = Some(displayed);

    let usage = BudgetUsage {
        requested_usd: 0.01,
        spent_today_usd: 0.0,
        spent_this_month_usd: 0.0,
    };
    let before = evaluate_budget(
        &BudgetLimits::default(),
        &ariadne::config::AutoModeConfig {
            enabled_by_default: true,
            preauthorized_budget_usd: stored,
            ..ariadne::config::AutoModeConfig::default()
        },
        usage,
    );
    let after = evaluate_budget(
        &BudgetLimits::default(),
        &ariadne::config::AutoModeConfig {
            enabled_by_default: true,
            preauthorized_budget_usd: saved_back,
            ..ariadne::config::AutoModeConfig::default()
        },
        usage,
    );

    assert_eq!(
        before.action, after.action,
        "U112：一次『打开设置页再保存』的空往返改变了预算语义（{:?} → {:?}）",
        before.action, after.action
    );
}
