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

/// `runtime_autosave_ms` 已被**移除**（2026-08-08），本用例钉住这个决定。
///
/// 它曾是设置页上一个纯假开关：用户能填、能存进 YAML，但后端没有一行代码读它。
/// 「消灭假开关」有两条路，这里选的是移除而非接线，理由是**接线会削弱持久性**——
/// 运行态目前经 `persist_if_needed` 每次状态跃迁同步落盘；改成按 ms 间隔节流，
/// 会在崩溃时丢掉窗口内的全部跃迁，等于拿可恢复性去换一个用户没要求的性能优化，
/// 与产品「Pause/Stop/Resume with checkpoints」的核心承诺直接冲突。
///
/// 判据取**序列化产物**而非结构体字段：字段不存在时代码根本不编译，
/// 那种「用例」实际什么都没断言；而 YAML 里残留该键才是真正会误导用户的形态。
#[test]
fn u111_runtime_autosave_ms_stays_removed_from_workflow_config() {
    let yaml = yaml_serde::to_string(&WorkflowConfig::default())
        .expect("WorkflowConfig 必须可序列化");

    assert!(
        !yaml.contains("runtime_autosave"),
        "`runtime_autosave_ms` 已于 2026-08-08 移除，不应再出现在配置产物里。\
         若确需恢复「运行态自动保存间隔」，请先解决它与同步持久化语义的冲突，\
         而不是把字段加回来——加回来只会重新变成一个没人读的假开关。\
         当前序列化结果：\n{yaml}"
    );
}

/// 契约层 `WorkflowExecutionLimits::default()` 与 `WorkflowConfig::default()`
/// 必须给出同一组出厂限制；两者若漂移，运行时回落值会与设置页展示值不一致。
#[test]
fn u113_contract_default_limits_match_workflow_config_defaults() {
    let from_config = WorkflowConfig::default().execution_limits();
    let from_contract = ariadne::contracts::WorkflowExecutionLimits::default();

    assert_eq!(
        from_config, from_contract,
        "契约层默认限制与 WorkflowConfig 出厂值漂移了"
    );
}

// ————————————————————————————————————————————————
// U113：工作流全局限制未接线（超时与循环上限）
// ————————————————————————————————————————————————

/// 设置页「自动化」分区并排三个工作流限制输入框，过去只有 `max_tool_rounds` 真正生效。
///
/// 三者现已收敛为契约层的 `WorkflowExecutionLimits`（唯一事实源），
/// 由 `WorkflowConfig::execution_limits()` 派生，供预检、节点超时回落与
/// tool-use 轮次共同消费。本用例守住配置 → 限制的派生保真。
#[test]
fn u113_workflow_config_derives_the_single_execution_limit_source() {
    let workflow = WorkflowConfig {
        default_timeout_ms: 45_000,
        max_loop_iterations: 3,
        max_tool_rounds: 6,
        ..WorkflowConfig::default()
    };
    let limits = workflow.execution_limits();

    assert_eq!(limits.default_timeout_ms, 45_000);
    assert_eq!(limits.max_loop_iterations, 3);
    assert_eq!(limits.max_tool_rounds, 6);

    // 节点声明 999 轮，远超全局上限 3，必须被拒绝。
    let runaway = ariadne::contracts::LoopPolicy {
        max_iterations: 999,
        timeout_ms: 30_000,
        budget_limit_usd: None,
        stop_condition: serde_json::json!({"kind": "manual"}),
    };
    assert!(
        runaway.validate_within(&limits).is_err(),
        "全局循环上限必须能拒绝越界的节点 policy"
    );

    // 上限内的 policy 必须放行，证明该校验不是无差别拒绝。
    let compliant = ariadne::contracts::LoopPolicy {
        max_iterations: 3,
        timeout_ms: 30_000,
        budget_limit_usd: None,
        stop_condition: serde_json::json!({"kind": "manual"}),
    };
    assert!(
        compliant.validate_within(&limits).is_ok(),
        "未越界的 loop policy 不应被全局上限拒绝：{:?}",
        compliant.validate_within(&limits).err()
    );
}

/// 节点未声明超时时，回落值必须取自项目配置，而不是运行时硬编码常量。
///
/// 修复前 `resolve_node_timeout_ms` 硬编码回落 120_000ms，而设置页展示的
/// `WorkflowConfig::default().default_timeout_ms` 是 300_000ms——用户看到
/// 「默认超时 300 秒」，未配置的节点却按 120 秒超时，且改配置不产生任何效果。
#[test]
fn u113_node_timeout_falls_back_to_configured_default_not_a_constant() {
    // 取一个既非旧硬编码值（120s）也非出厂值（300s）的配置，
    // 这样「回落值真的来自配置」与「回落到某个常量」不可能同时成立。
    let configured = WorkflowConfig {
        default_timeout_ms: 77_000,
        ..WorkflowConfig::default()
    };
    let limits = configured.execution_limits();

    assert_eq!(
        limits.resolve_node_timeout_ms(None),
        77_000,
        "U113：节点未声明超时时必须回落到项目配置的 default_timeout_ms"
    );
    assert_eq!(
        limits.resolve_node_timeout_ms(Some(0)),
        77_000,
        "0 等同未声明，同样回落到配置值"
    );
    assert_eq!(
        limits.resolve_node_timeout_ms(Some(9_000)),
        9_000,
        "节点显式声明的超时优先于全局默认"
    );

    // 出厂配置下的回落值就是设置页展示的那个数，两者不得再次漂移。
    assert_eq!(
        WorkflowConfig::default()
            .execution_limits()
            .resolve_node_timeout_ms(None),
        WorkflowConfig::default().default_timeout_ms,
        "设置页展示的默认超时必须等于运行时真实回落值"
    );
}

// ————————————————————————————————————————————————
// U112：预授权预算 0 值语义
// ————————————————————————————————————————————————

/// **安全契约**：预授权预算的 `Some(0.0)` 是用户**显式设定的零额度**，
/// 任何有成本的调用都必须暂停。
///
/// 这与全局日预算的 `0`（= 不设上限）语义相反，且该差异是**有意**的：
/// 把预授权的 0 也当成「不限制」会静默解除用户刻意设下的零额度，是安全性倒退。
/// 两个字段的区别由文案承担，不能靠统一数值语义消除。
/// （本文件首版曾断言二者应同义，已否决——见 `costs/budget.rs` 的就地说明。）
#[test]
fn u112_explicit_zero_preauthorized_budget_blocks_spending() {
    let decision = evaluate_budget(
        &BudgetLimits::default(),
        &ariadne::config::AutoModeConfig {
            enabled_by_default: true,
            preauthorized_budget_usd: Some(0.0),
            ..ariadne::config::AutoModeConfig::default()
        },
        BudgetUsage {
            requested_usd: 0.01,
            spent_today_usd: 0.0,
            spent_this_month_usd: 0.0,
        },
    );

    assert_eq!(
        decision.action,
        BudgetAction::Pause,
        "显式设定的零预授权额度必须暂停一切有成本的调用"
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

/// U112 的真实缺陷：读取侧把 `None` 折叠成 `0` 显示，保存侧再把这个 `0`
/// 原样写回成 `Some(0.0)`——用户只是打开设置页保存一次无关改动，
/// 「不限制」就被静默翻转成「全部暂停」。
///
/// 修复后 DTO 用 `Option<f64>` 区分「未设置」与「零额度」，故这里断言
/// **真实命令边界**的空往返：读回什么就写回什么，语义不得改变。
#[test]
fn u112_empty_settings_roundtrip_preserves_unlimited_semantics() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ConfigStore::new(temp.path()).load_or_create().unwrap();

    let before = ConfigStore::new(temp.path()).load_or_create().unwrap();
    assert_eq!(
        before.auto_mode.preauthorized_budget_usd, None,
        "新项目的预授权预算应为未设置"
    );

    // 用户打开设置页只改日预算，预授权原样回传（读取侧给出的就是「未设置」）。
    let status = ariadne::commands::get_budget_status_impl(temp.path()).unwrap();
    ariadne::commands::update_budget_config_impl(temp.path(), 10.0, status.preauthorized_usd)
        .unwrap();

    let after = ConfigStore::new(temp.path()).load_or_create().unwrap();
    assert_eq!(
        after.auto_mode.preauthorized_budget_usd, None,
        "U112：一次『打开设置页再保存』的空往返把「不限制」翻成了「零额度、全部暂停」"
    );
}

/// 反向护栏：用户**显式**填 0 时必须被持久化为零额度，不得当成「未设置」丢弃。
#[test]
fn u112_explicit_zero_input_is_persisted_not_discarded() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ConfigStore::new(temp.path()).load_or_create().unwrap();

    ariadne::commands::update_budget_config_impl(temp.path(), 10.0, Some(0.0)).unwrap();

    let config = ConfigStore::new(temp.path()).load_or_create().unwrap();
    assert_eq!(
        config.auto_mode.preauthorized_budget_usd,
        Some(0.0),
        "用户显式填的 0 是零额度，必须原样持久化"
    );
}
