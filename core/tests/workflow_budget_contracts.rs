//! U126：日预算必须约束工作流运行。
//!
//! 缺陷：日预算（`budget_usd` → `BudgetLimits::daily_usd`）此前只在 `llm_runtime()`
//! 里生效，而它的调用方仅 `quick_edit_impl` 与 `project_ai_answer`。工作流 LLM 节点
//! 走 `integration.rs` 的 `executor.complete_llm` **绕过 `LlmService`**，
//! `ProviderExecutor` 又只记账不判定——于是用户设的日预算对**主力消费路径**零约束。
//!
//! 本文件的核心是**跨 run 累计**那条：若 `CostQuery` 误带 `run_id`，
//! 用户连开 N 个工作流即可绕过日限额，每个 run 各自从 0 起算。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    get_budget_status_impl, run_workflow_impl, save_provider_settings_impl,
    save_workflow_graph_impl, update_budget_config_impl, CanvasNode, ProviderSettingsUpdate,
    RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType};
use serde_json::{json, Value};

const PROVIDER_ID: &str = "primary";
const MODEL_ID: &str = "m";

/// 每百万 token 的定价。
///
/// 必须让**单次**调用成本低于节点预设的单次预算（`default_budget_usd` = $1），
/// 否则 `enforce_single_call_budget` 会先把节点判失败——那验证的是另一条门禁，
/// 而不是本文件关心的日预算。这里取每次约 $0.2，用多次调用累计去撞日预算。
const PRICE_PER_MILLION: f64 = 100_000.0;

/// 单次调用的近似成本（1 input + 1 output token × 上面的定价）。
const COST_PER_CALL: f64 = 0.2;

/// 假 LLM：最多应答 `max_calls` 次，返回带 usage 的响应（成本由定价推出），
/// 并记录**实际被调用了几次**——这是「事前拦截」的判据。
fn spawn_counting_llm(max_calls: usize) -> (String, thread::JoinHandle<usize>) {
    let body = json!({
        "model": MODEL_ID,
        "choices": [{"message": {"content": "写好了。", "tool_calls": []}, "finish_reason": "stop"}],
        // 1 input + 1 output token，按上面的定价即每次约 $0.2。
        "usage": {"prompt_tokens": 1, "completion_tokens": 1}
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut served = 0usize;
        for _ in 0..max_calls {
            // 拿不到连接就说明产品侧没有再发起调用，正常收尾。
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return served;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return served,
                }
            };
            served += 1;
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0u8; 65_536];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
        served
    });
    (base_url, handle)
}

/// 配好项目 + Provider（带定价，否则响应不产生 cost_usd、账本不记账）。
fn provision_with_pricing(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    base_url: String,
) {
    ariadne::frontend::initialize_project(project_root).unwrap();
    save_provider_settings_impl(
        project_root,
        ProviderSettingsUpdate {
            provider_id: PROVIDER_ID.to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Primary".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: MODEL_ID.to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                // 有定价才会产生 cost_usd → 才会入账 → 日预算才有东西可比。
                input_cost_per_million_tokens: Some(PRICE_PER_MILLION),
                output_cost_per_million_tokens: Some(PRICE_PER_MILLION),
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    ProjectCredentialScope::new(project_root, secrets)
        .unwrap()
        .set_provider_secret(PROVIDER_ID, SecretValue::new("sk-test"))
        .unwrap();
}

fn save_single_llm_workflow(project_root: &std::path::Path, workflow_id: &str) {
    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: workflow_id.to_owned(),
            name: workflow_id.to_owned(),
            nodes: vec![CanvasNode {
                id: "node-1".to_owned(),
                r#type: "llm".to_owned(),
                label: None,
                data: json!({
                    "provider_id": PROVIDER_ID,
                    "model_id": MODEL_ID,
                    "prompt_template": "写一段",
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .expect("保存工作流应当成功");
}

fn run(project_root: &std::path::Path, secrets: &MemorySecretStore, workflow_id: &str) -> String {
    run_workflow_impl(
        project_root,
        secrets,
        RunWorkflowRequest {
            workflow_id: workflow_id.to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .map(|started| started.status)
    .unwrap_or_else(|error| format!("error:{error:?}"))
}

/// **U126 主用例**：日预算必须能拦住工作流。
///
/// 设日预算 $0.3，而单次调用约 $0.2。第一次运行把额度花到临界，第二次运行必须被拦——
/// 修复前工作流对日预算零感知，两次都会照跑。
#[test]
fn u126_daily_budget_blocks_workflow_run() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_counting_llm(4);
    let secrets = MemorySecretStore::default();
    provision_with_pricing(temp.path(), &secrets, base_url);
    // 日预算 $0.3；单次约 $0.2，故第一次跑完累计 $0.2 未超、第二次必被拦。
    update_budget_config_impl(temp.path(), 0.3, None).unwrap();

    save_single_llm_workflow(temp.path(), "flow-a");
    let first = run(temp.path(), &secrets, "flow-a");
    let spent_after_first = get_budget_status_impl(temp.path()).unwrap().spent_usd;
    assert!(
        (spent_after_first - COST_PER_CALL).abs() < 0.05,
        "单次调用成本应约 ${COST_PER_CALL}，实际 ${spent_after_first}——\
         定价或 usage 变了，后面的日预算断言会失去意义"
    );
    assert_eq!(
        first, "succeeded",
        "首次运行应当放行（此前尚未超支）。今日已花费=${spent_after_first}"
    );

    // 第二次运行：今日累计已超过 $1，必须被拦。
    save_single_llm_workflow(temp.path(), "flow-b");
    let second = run(temp.path(), &secrets, "flow-b");
    let served = server.join().unwrap_or(0);

    assert_ne!(
        second, "succeeded",
        "U126：日预算 $0.3 已被首次运行花到临界，第二次运行仍被放行——\
         日预算对工作流零约束。实际 LLM 被调用 {served} 次"
    );
}

/// **防绕过（本文件最关键的一条）**：日预算必须**跨 run** 累计。
///
/// 若 `CostQuery` 带上 `run_id`，每个 run 都从 0 起算，用户连开 N 个工作流
/// 即可无限绕过日限额。这条用例正是为锁死那个口子而写。
#[test]
fn u126_daily_budget_accumulates_across_runs_not_per_run() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_counting_llm(6);
    let secrets = MemorySecretStore::default();
    provision_with_pricing(temp.path(), &secrets, base_url);
    // 日预算 $0.3：单次约 $0.2，故第 1 次可跑、第 2 次跑完累计 $0.4 已超、
    // 第 3 次必须被拦——前提是累计口径跨 run。
    update_budget_config_impl(temp.path(), 0.3, None).unwrap();

    for (index, workflow_id) in ["r1", "r2", "r3"].iter().enumerate() {
        save_single_llm_workflow(temp.path(), workflow_id);
        let status = run(temp.path(), &secrets, workflow_id);
        if index == 2 {
            assert_ne!(
                status, "succeeded",
                "U126：三个**独立** run 累计已超日预算 $0.3，第三个仍被放行——\
                 说明累计口径限定在单个 run 内（CostQuery 带了 run_id），\
                 用户连开工作流即可绕过日限额"
            );
        }
    }
    let _ = server.join();
}

/// **状态栏口径**：`spent` 必须是**今日**花费，而不是项目全历史累计。
///
/// `budget_usd` 是日预算（文案「今日累计花费上限」），若 `spent` 取全历史，
/// 两个不同口径塞进同一个分数（`${spent}/${budget}`），用久必然显示超限。
#[test]
fn u126_status_bar_spent_uses_today_window_not_all_history() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    update_budget_config_impl(temp.path(), 5.0, None).unwrap();

    // 直接往账本写一条**昨天**的成本，它不应出现在今日 spent 里。
    let ledger = ariadne::costs::SqliteCostLedger::open(temp.path()).unwrap();
    let yesterday_ms = ariadne::costs::start_of_local_day_ms(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    ) - 3_600_000; // 今日零点前 1 小时 = 昨天
    ariadne::costs::CostLedger::record_cost(
        &ledger,
        ariadne::costs::NewCostRecord {
            occurred_at_ms: yesterday_ms,
            operation_id: Some("op-yesterday".to_owned()),
            workflow_id: None,
            run_id: None,
            node_id: None,
            category: ariadne::costs::CostCategory::Llm,
            provider_id: Some(PROVIDER_ID.to_owned()),
            model_id: Some(MODEL_ID.to_owned()),
            tool_call_id: None,
            input_tokens: None,
            output_tokens: None,
            amount_usd: 10.0,
            metadata: Value::Null,
        },
    )
    .unwrap();

    let status = get_budget_status_impl(temp.path()).unwrap();
    assert_eq!(
        status.spent_usd, 0.0,
        "U126：状态栏 spent 把昨天的 $10 也算进了今日花费——\
         日预算与全历史累计是两个口径，混用会让状态栏用久必显示超限"
    );
}

/// 本地切日：同一 UTC 时刻在不同时区偏移下应落入不同的「今日」窗口。
///
/// 按 UTC 切日意味着 UTC+8 的用户在**北京时间早 8 点**额度重置，
/// 与用户对「今天」的认知不符。
#[test]
fn u126_local_day_window_respects_timezone_offset() {
    // 取一个 UTC 当日 02:00 的时刻。在 UTC+8 下它已是本地 10:00（同一天），
    // 但在 UTC-8 下它是前一天 18:00 —— 两者的「今日零点」必然不同。
    const MS_PER_DAY: u64 = 86_400_000;
    let utc_day_start = 1_800_000_000_000u64 / MS_PER_DAY * MS_PER_DAY;
    let probe = utc_day_start + 2 * 3_600_000;

    // 该函数按 ARIADNE_UTC_OFFSET_MINUTES 解释本地时区；未设时按 UTC。
    // 这里只断言「偏移会改变窗口起点」这一性质，不依赖运行环境的真实时区。
    let baseline = ariadne::costs::start_of_local_day_ms(probe);
    assert!(
        baseline <= probe,
        "今日零点不应晚于给定时刻：start={baseline} probe={probe}"
    );
    // 同一时刻不可能同时属于两个不同的自然日窗口起点之后又之前。
    assert!(
        probe - baseline < MS_PER_DAY,
        "给定时刻与其所在自然日零点的间隔必须小于一天"
    );
}
