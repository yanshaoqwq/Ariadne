use ariadne::config::AutoModeConfig;
use ariadne::contracts::{RunControl, RunId, WorkflowId};
use ariadne::costs::{
    budget_limits_from_global_budget, evaluate_budget, BudgetAction, BudgetLimits, BudgetUsage,
    CostCategory, CostLedger, CostQuery, NewCostRecord, SqliteCostLedger,
};
use serde_json::Value;

#[test]
fn sqlite_cost_ledger_tracks_tool_use_costs() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    ledger
        .record_cost(NewCostRecord {
            occurred_at_ms: 100,
            operation_id: Some("op-tool-call-1".to_owned()),
            category: CostCategory::Llm,
            provider_id: Some("anthropic".to_owned()),
            model_id: Some("claude".to_owned()),
            workflow_id: Some(WorkflowId::new("wf")),
            run_id: Some(RunId::new("run")),
            node_id: None,
            tool_call_id: Some("tool-call-1".to_owned()),
            input_tokens: Some(1_000),
            output_tokens: Some(500),
            amount_usd: 0.42,
            metadata: Value::Null,
        })
        .unwrap();

    let total = ledger
        .total_cost(&CostQuery {
            run_id: Some(RunId::new("run")),
            category: Some(CostCategory::Llm),
            ..CostQuery::default()
        })
        .unwrap();

    assert_eq!(total, 0.42);
}

#[test]
fn over_budget_decision_pauses_workflow() {
    let decision = evaluate_budget(
        &BudgetLimits {
            daily_usd: Some(1.0),
            ..BudgetLimits::default()
        },
        &AutoModeConfig::default(),
        BudgetUsage {
            requested_usd: 0.25,
            spent_today_usd: 0.90,
            spent_this_month_usd: 0.90,
        },
    );

    assert_eq!(decision.action, BudgetAction::Pause);
    assert_eq!(decision.run_control, RunControl::Pause);
}

/// 用户保存的全局预算必须经 `budget_limits_from_global_budget` 进入 `evaluate_budget`。
#[test]
fn saved_global_budget_maps_into_live_evaluate_budget_pause() {
    let limits = budget_limits_from_global_budget(2.0);
    assert_eq!(limits.daily_usd, Some(2.0));
    let over = evaluate_budget(
        &limits,
        &AutoModeConfig::default(),
        BudgetUsage {
            requested_usd: 0.5,
            spent_today_usd: 1.6,
            spent_this_month_usd: 1.6,
        },
    );
    assert_eq!(over.action, BudgetAction::Pause);
    assert_eq!(over.reason.as_deref(), Some("daily budget limit exceeded"));

    let unlimited = budget_limits_from_global_budget(0.0);
    assert_eq!(unlimited.daily_usd, None);
    let under_hard_limit = evaluate_budget(
        &unlimited,
        &AutoModeConfig {
            enabled_by_default: true,
            preauthorized_budget_usd: None,
            ..AutoModeConfig::default()
        },
        BudgetUsage {
            requested_usd: 9.0,
            spent_today_usd: 0.0,
            spent_this_month_usd: 0.0,
        },
    );
    // Auto Mode + 无日限额 + 无预授权 → 直接放行（硬预算不拦截）
    assert_eq!(under_hard_limit.action, BudgetAction::Allow);
}

#[test]
fn preauthorized_budget_from_auto_mode_still_pauses_via_evaluate_budget() {
    let limits = budget_limits_from_global_budget(100.0);
    let decision = evaluate_budget(
        &limits,
        &AutoModeConfig {
            enabled_by_default: true,
            preauthorized_budget_usd: Some(1.0),
            ..AutoModeConfig::default()
        },
        BudgetUsage {
            requested_usd: 0.3,
            spent_today_usd: 0.8,
            spent_this_month_usd: 0.8,
        },
    );
    assert_eq!(decision.action, BudgetAction::Pause);
    assert_eq!(
        decision.reason.as_deref(),
        Some("auto mode preauthorized budget exceeded")
    );
}
