use ariadne::config::AutoModeConfig;
use ariadne::contracts::{RunControl, RunId, WorkflowId};
use ariadne::costs::{
    evaluate_budget, BudgetAction, BudgetLimits, BudgetUsage, CostCategory, CostLedger, CostQuery,
    NewCostRecord, SqliteCostLedger,
};
use serde_json::Value;

#[test]
fn sqlite_cost_ledger_tracks_tool_use_costs() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    ledger
        .record_cost(NewCostRecord {
            occurred_at_ms: 100,
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
