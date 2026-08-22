//! U196-D 回归：工作流某个节点失败后，**从那个节点重跑**，前面成功的节点不重跑。
//!
//! # 原缺陷
//!
//! 第 7 个节点失败时前 6 个的产出都在，而作者**只能整条重跑**——重烧前 6 步的
//! 钱和时间。三条既有恢复路全部拒绝失败的运行：`resume()` / `skip_node()` /
//! `resume_from_node()` 开头都判 `is_terminal()`，`store::claim_resume` 只接受
//! `Paused | Queued | Running`。
//!
//! # 为什么 `resume_from_node` 替代不了
//!
//! 那条的语义是「**注入外部正文**从指定节点重跑」：它把作者给的正文写成节点输出并
//! **置为成功**，节点本身不再执行。用它顶替，等于要求作者自己手写第 7 个节点
//! 本该产出的东西。本文件的 `injection_path_still_requires_non_terminal_run`
//! 把这个语义差别钉住，防止后人把两条合并。
//!
//! # 判据落在「重跑了几次」而不是「函数返回了 Ok」
//!
//! 核心断言是 **executor 的调用次数**：失败节点 +1、已成功的上游 +0。
//! 「函数返回 Ok」在整条重跑的实现下同样成立，那正是要避免的行为。

use ariadne::contracts::{
    CoreError, Edge, EdgeId, NodeId, NodeInstance, PortEndpoint, PortMap, PortValue, RunId,
    RunStatus, WorkflowDefinition, WorkflowEdgeKind, WorkflowId,
};
use ariadne::workflow::{
    NodeRetryPolicy, WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest,
    WorkflowNodeExecutor, WorkflowOperationPolicy, WorkflowRuntime, WorkflowRuntimeEventType,
};
use serde_json::{json, Value};

/// 三节点线性工作流：outliner → writer → summarizer。
///
/// 用控制边而不是数据边：数据边会引入端口 alias 校验，与本条要测的东西无关。
fn linear_workflow() -> WorkflowDefinition {
    WorkflowDefinition {
        id: WorkflowId::from("u196d-linear"),
        name: "U196-D Linear".to_owned(),
        nodes: vec![
            node("outliner", "outliner"),
            node("writer", "writer"),
            node("summarizer", "summarizer"),
        ],
        edges: vec![
            control_edge("e1", "outliner", "writer"),
            control_edge("e2", "writer", "summarizer"),
        ],
        metadata: Value::Null,
    }
}

fn node(id: &str, type_name: &str) -> NodeInstance {
    NodeInstance {
        id: NodeId::from(id),
        type_name: type_name.to_owned(),
        label: None,
        config: Value::Null,
        position: None,
    }
}

fn control_edge(id: &str, from: &str, to: &str) -> Edge {
    Edge {
        id: EdgeId::from(id),
        kind: WorkflowEdgeKind::Control,
        from: PortEndpoint {
            node_id: NodeId::from(from),
            port_name: ariadne::contracts::EXECUTION_OUTPUT_PORT.to_owned(),
        },
        to: PortEndpoint {
            node_id: NodeId::from(to),
            port_name: ariadne::contracts::EXECUTION_INPUT_PORT.to_owned(),
        },
        alias: None,
        communication: None,
    }
}

/// 按节点名记调用次数的 executor；可为指定节点排入若干次失败。
#[derive(Default)]
struct CountingExecutor {
    calls: Vec<String>,
    /// 每次执行的 (节点名, operation_id, operation_attempt)，用于钉住重跑拿到的是
    /// **全新的** operation 身份而不是失败那次的重放。
    operations: Vec<(String, String, u32)>,
    /// 每个节点还剩多少次要失败（消耗式），用不可重试的错误以直达 `Failed`。
    failures: std::collections::BTreeMap<String, usize>,
}

impl CountingExecutor {
    fn fail_once(mut self, node_id: &str) -> Self {
        *self.failures.entry(node_id.to_owned()).or_default() += 1;
        self
    }

    fn count(&self, node_id: &str) -> usize {
        self.calls.iter().filter(|name| *name == node_id).count()
    }

    /// 取该节点第一次执行的 (operation_id, operation_attempt)。
    fn first_operation(&self, node_id: &str) -> (String, u32) {
        self.operations
            .iter()
            .find(|(name, _, _)| name == node_id)
            .map(|(_, id, attempt)| (id.clone(), *attempt))
            .expect("该节点没有执行记录")
    }
}

impl WorkflowNodeExecutor for CountingExecutor {
    fn operation_policy(
        &self,
        _request: &WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowOperationPolicy> {
        Ok(WorkflowOperationPolicy::Untracked)
    }

    fn execute(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowNodeExecutionOutput> {
        let name = request.node_id.as_str().to_owned();
        self.calls.push(name.clone());
        self.operations.push((
            name.clone(),
            request.operation_id.clone(),
            request.operation_attempt,
        ));
        if let Some(remaining) = self.failures.get_mut(&name) {
            if *remaining > 0 {
                *remaining -= 1;
                // PermissionDenied 被 `node_error_kind` 判为 Permission ⇒ 不可重试
                // ⇒ 运行直接进 `Failed`（终态），正是本条要恢复的那个状态。
                return Err(CoreError::PermissionDenied {
                    action: name.clone(),
                    reason: format!("scripted failure for {name}"),
                });
            }
        }
        let mut outputs = PortMap::new();
        outputs.insert("output".to_owned(), PortValue::inline(json!("ok")));
        Ok(WorkflowNodeExecutionOutput {
            outputs,
            ..Default::default()
        })
    }
}

/// 跑到失败为止，返回 (runtime, executor, workflow)。
fn run_until_failed() -> (WorkflowRuntime, CountingExecutor, WorkflowDefinition) {
    let workflow = linear_workflow();
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("u196d-run-1")).expect("runtime");
    // max_attempts=1：让「不可重试」这件事不依赖退避轮次，失败即终态。
    runtime
        .set_retry_policy(NodeRetryPolicy {
            max_attempts: 1,
            ..NodeRetryPolicy::default()
        })
        .expect("retry policy");
    let mut executor = CountingExecutor::default().fail_once("writer");
    let status = runtime.run(&workflow, &mut executor).expect("run");
    assert_eq!(
        status,
        RunStatus::Failed,
        "前提不成立：本文件测的是「运行已进入 Failed 终态」之后的恢复"
    );
    (runtime, executor, workflow)
}

/// 本条是 U196-D 的**本体判据**：重跑只重跑失败的那个，上游一次都不再跑。
///
/// 判据刻意选 **executor 调用次数**而不是「返回 Ok」或「状态回到 Queued」：
/// 整条重跑的实现同样满足后两者，而它恰恰是这条缺陷要消除的行为
/// —— 作者要省的就是前 6 步的钱。
#[test]
fn retry_failed_node_reruns_only_the_failed_node_and_keeps_upstream_outputs() {
    let (mut runtime, executor, workflow) = run_until_failed();

    // 前提核对：outliner 已成功且产出在，writer 失败。
    assert_eq!(executor.count("outliner"), 1);
    assert_eq!(executor.count("writer"), 1);
    assert_eq!(executor.count("summarizer"), 0);
    let outliner_before = runtime
        .state
        .nodes
        .get(&NodeId::from("outliner"))
        .expect("outliner 快照")
        .clone();
    assert_eq!(outliner_before.status, RunStatus::Succeeded);

    runtime
        .retry_failed_node(&workflow, &NodeId::from("writer"))
        .expect("失败节点应当可以重跑");

    // 终态被解除，且是 Queued（而不是 Running）——lease 的领取由 store 侧按
    // Queued|Running 判定，见 `mutate_state_and_claim`。
    assert_eq!(runtime.state.status, RunStatus::Queued);
    // failure 必须清空：它是前端唯一显示的失败字段，留着的话重跑成功后
    // 画布上仍写着上一次的失败原因。
    assert!(runtime.state.failure.is_none());

    // 已成功的上游产出**一个字节都没动**（不是「还在」而是「与重跑前完全相同」）。
    assert_eq!(
        runtime.state.nodes.get(&NodeId::from("outliner")),
        Some(&outliner_before),
        "已成功的上游节点快照被改动 ⇒ 它会重跑，作者要省的钱就没省下来"
    );
    // 失败节点的快照被清掉 ⇒ 它会真的再执行一次（而不是被当成已成功跳过）。
    assert!(!runtime.state.nodes.contains_key(&NodeId::from("writer")));

    // 续跑：这一次 writer 不再被排入失败。
    let mut resumed = CountingExecutor::default();
    let status = runtime.run(&workflow, &mut resumed).expect("续跑");
    assert_eq!(status, RunStatus::Succeeded);

    // **本条的核心两行**：failed 节点重跑了一次，已成功的上游一次都没有。
    assert_eq!(
        resumed.count("writer"),
        1,
        "失败的节点没有重跑 ⇒ 「从失败节点重跑」没有发生"
    );
    assert_eq!(
        resumed.count("outliner"),
        0,
        "已成功的上游被重跑 ⇒ 这就是「整条重跑」，U196-D 的缺陷原样保留"
    );
    assert_eq!(resumed.count("summarizer"), 1, "失败节点的下游应当续跑");
}

/// 反向：**不该重跑的不能重跑**。
///
/// 没有这条，「无条件清掉目标节点及其下游」也能让上一条全绿——而那是把
/// 「无法补救一次失败」换成「一点就烧掉一批已付费的产出」，对作者更糟。
/// 作者在画布上点错一个方块是常事，这道门就是那次点错的唯一防线。
#[test]
fn retry_refuses_nodes_that_did_not_fail_and_leaves_their_outputs_intact() {
    let (mut runtime, _executor, workflow) = run_until_failed();
    let before = runtime.state.nodes.clone();

    let error = runtime
        .retry_failed_node(&workflow, &NodeId::from("outliner"))
        .expect_err("对着已成功的节点重跑应当被拒");
    assert!(
        error.to_string().contains("did not fail"),
        "拒绝理由没说清是「这个节点没失败」：{error}"
    );

    // 被拒绝时**状态一个字节都不能动**：半途清掉快照再报错，等于既没补救又烧了产出。
    assert_eq!(runtime.state.nodes, before);
    assert_eq!(runtime.state.status, RunStatus::Failed);
    assert!(runtime.state.failure.is_some());

    // 图里不存在的节点同样拒（前端传来的 node_id 来自 failure.stage，不可信）。
    let missing = runtime
        .retry_failed_node(&workflow, &NodeId::from("no-such-node"))
        .expect_err("图里没有的节点应当被拒");
    assert!(missing.to_string().contains("not found in workflow"));
}

/// 只有**失败的运行**才能走这条路。
///
/// 它是唯一允许从终态进入的恢复入口，所以边界必须钉住：拿它当
/// 「随时重跑任意节点」用的话，运行中的节点会被并发清快照。
#[test]
fn retry_refuses_runs_that_are_not_failed() {
    let workflow = linear_workflow();
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("u196d-run-2")).expect("runtime");
    assert_eq!(runtime.state.status, RunStatus::Queued);

    let error = runtime
        .retry_failed_node(&workflow, &NodeId::from("writer"))
        .expect_err("运行没失败时不应当接受重跑");
    assert!(
        error.to_string().contains("is not failed"),
        "拒绝理由应当指出运行并未失败：{error}"
    );
}

/// 钉住与 `resume_from_node` 的**语义分界**，防止后人把两条合并。
///
/// 报告建议「D 与 U187-B 合并设计」；实现上不能合并成一个方法：
/// 注入路径必须在**非终态**下调用（它是 Prudent 拒绝后的暂停处置），
/// 且它把节点**置为成功**、不再执行。这条用例证明拿它顶替 D 会直接报错，
/// 而不是「效果差一点」。
#[test]
fn injection_path_still_requires_non_terminal_run() {
    let (mut runtime, _executor, workflow) = run_until_failed();

    let mut injected = PortMap::new();
    injected.insert("output".to_owned(), PortValue::inline(json!("人工正文")));
    let error = runtime
        .resume_from_node(&workflow, &NodeId::from("writer"), injected)
        .expect_err("注入路径不接受终态运行——这正是 D 需要独立入口的理由");
    assert!(error.to_string().contains("terminal"));
}

/// 重跑要留下可查的事件；且**不得**复用注入路径的事件类型。
///
/// 两者在运行日志里必须分得开：一条是「作者自己写了正文」，
/// 另一条是「机器又跑了一次」。日志把它们混为一谈时，
/// 「这段正文是谁写的」永远查不回来。
#[test]
fn retry_records_its_own_event_type() {
    let (mut runtime, _executor, workflow) = run_until_failed();
    runtime
        .retry_failed_node(&workflow, &NodeId::from("writer"))
        .expect("重跑");

    let events = runtime.events_for_node(&NodeId::from("writer"));
    assert!(
        events
            .iter()
            .any(|event| event.event_type == WorkflowRuntimeEventType::NodeRetriedFromFailure),
        "重跑没有留下 NodeRetriedFromFailure 事件"
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == WorkflowRuntimeEventType::NodeResumedWithInjection),
        "重跑复用了注入路径的事件类型 ⇒ 日志里两件事分不开"
    );
}

/// 重跑必须拿到**全新的 operation 身份**，否则 journal 会把它认成重放。
///
/// `retry_failed_node` 刻意**不清** `node_operation_sequences`。清掉的话
/// `next_node_operation_attempt` 会重新从 1 算起，`operation_id`（由
/// workflow/run/node/attempt 哈希而成）与失败那次完全相同 ⇒ 走 journaled 策略的
/// 节点会命中 `prepare_operation_journal` 的重放分支，把上次的结果原样取回。
/// 「重跑」于是退化成「再看一遍同一个错误」，而这在 UI 上与「重跑了但又失败了」
/// 完全同形 —— 作者会以为是同一个故障复发，永远不会怀疑重跑没真的发生。
#[test]
fn retry_gets_a_fresh_operation_identity_so_the_journal_cannot_replay_the_failure() {
    let (mut runtime, executor, workflow) = run_until_failed();
    let (failed_operation_id, failed_attempt) = executor.first_operation("writer");
    assert_eq!(failed_attempt, 1);

    runtime
        .retry_failed_node(&workflow, &NodeId::from("writer"))
        .expect("重跑");
    // 序号被保留：这是「新身份」的来源，清掉就会撞回失败那次的 operation_id。
    assert_eq!(
        runtime
            .state
            .node_operation_sequences
            .get(&NodeId::from("writer")),
        Some(&1),
        "node_operation_sequences 被清 ⇒ 重跑会算出与失败那次相同的 operation_id"
    );

    let mut resumed = CountingExecutor::default();
    runtime.run(&workflow, &mut resumed).expect("续跑");
    let (retry_operation_id, retry_attempt) = resumed.first_operation("writer");
    assert_eq!(retry_attempt, 2, "重跑的 attempt 应当递增");
    assert_ne!(
        retry_operation_id, failed_operation_id,
        "重跑用了与失败那次相同的 operation_id ⇒ journal 会重放上次的失败结果"
    );
}
