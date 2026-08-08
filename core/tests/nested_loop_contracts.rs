//! U124：嵌套 loop 的迭代计数重置契约。
//!
//! 缺陷：`advance_loop` 清理重跑下游时不重置 `loop_iterations`，而
//! `resume_from_node` 会重置。嵌套 loop 下 `collect_downstream_closure` 不区分
//! 节点类型，外层重跑会把内层 loop 收进闭包——内层状态被清、计数留在上限，
//! 于是外层第 2 轮起内层一进就 pause。
//!
//! 对文学场景即：外层「逐章循环」+ 内层「critic→polisher 返修循环」，
//! 第一章能正常返修 N 轮，**第二章起返修直接暂停**。
//!
//! 本文件独立于 `workflow_contracts.rs`，自带最小 executor，避免与该文件的
//! 私有 helper 耦合。

use std::collections::BTreeMap;

use ariadne::contracts::{
    CoreResult, Edge, EdgeId, NodeId, NodeInstance, PortEndpoint, PortMap, PortValue, RunStatus,
    RunId, WorkflowDefinition, WorkflowEdgeKind, WorkflowId, EXECUTION_INPUT_PORT,
    EXECUTION_OUTPUT_PORT,
};
use ariadne::workflow::{
    BuiltinWorkflowNodeExecutor, SqliteWorkflowRuntimeStore, WorkflowExternalNodeExecutor,
    WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest, WorkflowRuntime,
};
use serde_json::{json, Value};

/// 最小 external executor：按节点名依次吐出预设输出，并记录调用次数。
#[derive(Default)]
struct StubExecutor {
    /// 每个节点的输出队列；用尽后重复最后一个。
    outputs: BTreeMap<String, Vec<WorkflowNodeExecutionOutput>>,
    calls: BTreeMap<String, usize>,
}

impl StubExecutor {
    fn with_outputs(mut self, node: &str, outputs: Vec<WorkflowNodeExecutionOutput>) -> Self {
        self.outputs.insert(node.to_owned(), outputs);
        self
    }

    fn call_count(&self, node: &str) -> usize {
        self.calls.get(node).copied().unwrap_or(0)
    }
}

impl WorkflowExternalNodeExecutor for StubExecutor {
    // 用默认的 Untracked policy：本文件验证的是 loop 迭代计数，不涉及
    // operation journaling。`replayable_receipt` 会要求 executor 消费
    // dispatch authorization，那是另一条正交契约，掺进来只会让失败原因失焦。

    fn execute_external(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput> {
        let node = request.node_id.as_str().to_owned();
        let index = *self.calls.entry(node.clone()).or_insert(0);
        self.calls.insert(node.clone(), index + 1);
        let queue = self.outputs.get(&node).cloned().unwrap_or_default();
        if queue.is_empty() {
            return Ok(WorkflowNodeExecutionOutput::default());
        }
        Ok(queue[index.min(queue.len() - 1)].clone())
    }
}

/// 构造带 `approved` 布尔输出的节点结果。
fn approved(value: bool) -> WorkflowNodeExecutionOutput {
    signals(&[("approved", value)])
}

/// 一次产出多个布尔信号端口，供内外层 loop 各读各的。
fn signals(pairs: &[(&str, bool)]) -> WorkflowNodeExecutionOutput {
    let mut outputs = PortMap::new();
    for (port, value) in pairs {
        outputs.insert((*port).to_owned(), PortValue::inline(*value));
    }
    WorkflowNodeExecutionOutput {
        outputs,
        ..WorkflowNodeExecutionOutput::default()
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

fn loop_node(id: &str, max_iterations: u32, rerun: &[&str]) -> NodeInstance {
    loop_node_on(id, max_iterations, rerun, "approved")
}

fn loop_node_on(
    id: &str,
    max_iterations: u32,
    rerun: &[&str],
    stop_alias: &str,
) -> NodeInstance {
    NodeInstance {
        id: NodeId::from(id),
        type_name: "loop".to_owned(),
        label: None,
        config: json!({
            "max_iterations": max_iterations,
            "timeout_ms": 30_000,
            "stop_condition": { "input_alias": stop_alias, "equals": true },
            "rerun_node_ids": rerun,
        }),
        position: None,
    }
}

fn control_edge(id: &str, from: &str, to: &str) -> Edge {
    Edge {
        id: EdgeId::from(id),
        kind: WorkflowEdgeKind::Control,
        from: PortEndpoint {
            node_id: NodeId::from(from),
            port_name: EXECUTION_OUTPUT_PORT.to_owned(),
        },
        to: PortEndpoint {
            node_id: NodeId::from(to),
            port_name: EXECUTION_INPUT_PORT.to_owned(),
        },
        alias: None,
        communication: None,
    }
}

fn data_edge(id: &str, from: &str, to: &str) -> Edge {
    named_data_edge(id, from, to, "approved")
}

/// 内外层 loop 必须读**不同**的信号，否则同一个 `approved` 会让外层跟着内层
/// 一起满足停止条件、根本不触发重跑——那样就测不到 U124 了。
fn named_data_edge(id: &str, from: &str, to: &str, alias: &str) -> Edge {
    Edge {
        id: EdgeId::from(id),
        kind: WorkflowEdgeKind::Data,
        from: PortEndpoint {
            node_id: NodeId::from(from),
            port_name: alias.to_owned(),
        },
        to: PortEndpoint {
            node_id: NodeId::from(to),
            port_name: "condition".to_owned(),
        },
        alias: Some(alias.to_owned()),
        communication: None,
    }
}

/// 单层 loop 的基线：确认最小 executor 与图结构本身可用。
///
/// 这条先立住，后面嵌套用例的失败才能归因到嵌套本身而非脚手架。
#[test]
fn u124_single_loop_baseline_runs_to_completion() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-single"),
        name: "single".to_owned(),
        nodes: vec![node("writer", "writer"), loop_node("loop", 3, &["writer"])],
        edges: vec![
            data_edge("d1", "writer", "loop"),
            control_edge("c1", "writer", "loop"),
            control_edge("c2", "loop", "writer"),
        ],
        metadata: Value::Null,
    };

    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-single")).unwrap();
    // 第一轮不通过、第二轮通过。
    let mut external =
        StubExecutor::default().with_outputs("writer", vec![approved(false), approved(true)]);
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    assert_eq!(
        status,
        RunStatus::Succeeded,
        "单层 loop 基线应当跑通。pause_reason={:?} events={:?} iterations={:?} calls={}",
        runtime.state.pause_reason,
        runtime.state.events,
        runtime.state.loop_iterations,
        external.call_count("writer")
    );
    assert_eq!(external.call_count("writer"), 2);
}

/// **U124 主用例**：嵌套 loop 下，内层在外层第 2 轮仍能跑满自己的轮次。
///
/// 图结构（外层逐章、内层返修）：
/// ```text
/// polisher → inner(返修上限 2) → polisher      内层闭环
/// inner    → outer(逐章上限 2) → polisher      外层闭环
/// ```
/// 外层重跑清理下游时会把 `inner` 收进闭包。修复前 `inner` 的
/// `loop_iterations` 不被重置，外层第 2 轮起 `inner` 一进就 pause。
#[test]
fn u124_inner_loop_iterations_reset_when_outer_loop_reruns() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-nested"),
        name: "nested".to_owned(),
        nodes: vec![
            node("polisher", "polisher"),
            loop_node_on("inner", 2, &["polisher"], "revised"),
            loop_node_on("outer", 2, &["polisher"], "chapter_done"),
        ],
        edges: vec![
            // 内层「返修循环」读 revised
            named_data_edge("d-inner", "polisher", "inner", "revised"),
            control_edge("c-inner-in", "polisher", "inner"),
            control_edge("c-inner-back", "inner", "polisher"),
            // 外层「逐章循环」读 chapter_done
            named_data_edge("d-outer", "polisher", "outer", "chapter_done"),
            control_edge("c-outer-in", "inner", "outer"),
            control_edge("c-outer-back", "outer", "polisher"),
        ],
        metadata: Value::Null,
    };

    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-nested")).unwrap();
    // 内层第一轮不过、第二轮过；外层随后再来一遍同样的序列。
    let mut external = StubExecutor::default().with_outputs(
        "polisher",
        vec![
            // ── 第 1 章：内层必须把 max_iterations=2 用尽 ──
            // 关键点：若内层只重跑 1 次，第 2 轮它还剩 1 次余量，
            // 那么摘掉修复用例也会通过——测不到缺陷。
            signals(&[("revised", false), ("chapter_done", false)]), // 内层重跑 #1
            signals(&[("revised", false), ("chapter_done", false)]), // 内层重跑 #2 → 计数达上限 2
            // 内层通过，但全书未完 → 外层触发重跑，清理下游（含 inner）
            signals(&[("revised", true), ("chapter_done", false)]),
            // ── 第 2 章：内层需要再次重跑 ──
            // 修复前 inner 计数仍停在 2（已达上限）→ 一进就 pause
            signals(&[("revised", false), ("chapter_done", false)]),
            // 内层通过且全书完成 → 运行结束
            signals(&[("revised", true), ("chapter_done", true)]),
        ],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    // 判据：polisher 被调用了 4 次，即内层在外层两轮里各跑满 2 轮。
    // 修复前外层第 2 轮的内层立即 pause，polisher 只会被调 3 次。
    assert_eq!(
        status,
        RunStatus::Succeeded,
        "U124：外层第 2 轮时内层因计数未重置而 pause。\
         loop_iterations={:?} pause_reason={:?}",
        runtime.state.loop_iterations,
        runtime.state.pause_reason
    );
    assert_eq!(
        external.call_count("polisher"),
        5,
        "U124：内层 loop 在外层第 2 轮未能重跑——外层清理下游时没有重置内层的\
         迭代计数，内层一进就 pause。运行状态：{status:?}，\
         loop_iterations={:?}",
        runtime.state.loop_iterations
    );
}

/// **防修复过头**：外层自己的迭代计数不得被清零。
///
/// `collect_downstream_closure` 在闭环图上一定会把发起重跑的 loop 自己收进
/// `all_affected`。若无差别清零，`current` 恒为 0 → `current >= max_iterations`
/// 永不成立 → 无限循环 + 成本失控，比原缺陷危险得多。
///
/// 这条用「外层永不满足停止条件」构造：外层必须在达到 max_iterations 后 pause，
/// 而不是无限跑下去。
#[test]
fn u124_outer_loop_own_counter_survives_and_still_hits_global_limit() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-runaway"),
        name: "runaway".to_owned(),
        nodes: vec![
            node("polisher", "polisher"),
            loop_node_on("inner", 2, &["polisher"], "revised"),
            loop_node_on("outer", 2, &["polisher"], "chapter_done"),
        ],
        edges: vec![
            named_data_edge("d-inner", "polisher", "inner", "revised"),
            control_edge("c-inner-in", "polisher", "inner"),
            control_edge("c-inner-back", "inner", "polisher"),
            named_data_edge("d-outer", "polisher", "outer", "chapter_done"),
            control_edge("c-outer-in", "inner", "outer"),
            control_edge("c-outer-back", "outer", "polisher"),
        ],
        metadata: Value::Null,
    };

    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-runaway")).unwrap();
    // 永远不通过：内外层都会不断想重跑，唯一的终止来源就是 max_iterations。
    // 内层每次都通过（不消耗内层计数），外层永不通过——这样压力全落在
    // **外层自己的**计数上。若外层计数被误清零，`current >= max_iterations`
    // 永不成立，运行会无限循环（表现为本用例挂死而非失败）。
    let mut external = StubExecutor::default().with_outputs(
        "polisher",
        vec![signals(&[("revised", true), ("chapter_done", false)])],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    // 只要能返回（而非挂死），就证明上限生效了。
    assert_eq!(
        status,
        RunStatus::Paused,
        "停止条件永不满足时，loop 必须因达到 max_iterations 而 pause"
    );

    // 调用次数必须有界。内层 2 轮 × 外层 2 轮 = 4 次量级；
    // 给一个宽松上界，只用来证明「没有无限循环」。
    assert!(
        external.call_count("polisher") <= 12,
        "U124 施工陷阱：外层自己的迭代计数被清零了，导致无限循环。\
         polisher 被调用 {} 次，loop_iterations={:?}",
        external.call_count("polisher"),
        runtime.state.loop_iterations
    );
}
