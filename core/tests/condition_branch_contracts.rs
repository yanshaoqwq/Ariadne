//! U125：condition 的分支必须由**出边引脚**承载。
//!
//! 缺陷：runtime 靠「节点输出的 `branch`」与「边自己的 `alias`」比对来决定放行
//! 哪条出边，而前置判定 `is_condition_selector_edge` 要求 `edge.alias.is_some()`
//! ——**alias 为空的边根本不进这套判定，直接 Active**。
//!
//! 而 UI 上 alias 是一个自由文本「边标签」`TextBox`，无提示、无默认值、
//! 无约束，用户无从知道这里该填 `true`。于是最常见的画法（单条出边，
//! 「条件成立才继续」）下 condition **什么也拦不住**，且没有任何报错。
//!
//! 修复方向（13A 已定）：分支由 `exec_out_true` / `exec_out_false` 两个执行出
//! 引脚承载。引脚不可能留空、也不可能填错——错误状态在 UI 上根本不存在。
//!
//! 本文件自带最小 executor，不依赖 `workflow_contracts.rs` 的私有 helper。

use std::collections::BTreeMap;

use ariadne::contracts::{
    CoreResult, Edge, EdgeId, NodeId, NodeInstance, PortEndpoint, PortMap, PortValue, RunId,
    RunStatus, WorkflowDefinition, WorkflowEdgeKind, WorkflowExecutionLimits, WorkflowId,
    EXECUTION_INPUT_PORT, EXECUTION_OUTPUT_PORT, EXECUTION_OUTPUT_PORT_FALSE,
    EXECUTION_OUTPUT_PORT_TRUE,
};
use ariadne::workflow::{
    validate_workflow_execution_contracts, BuiltinWorkflowNodeExecutor, SqliteWorkflowRuntimeStore,
    WorkflowExternalNodeExecutor, WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest,
    WorkflowRuntime,
};
use serde_json::{json, Value};

/// 最小 external executor：记录每个节点被调用了几次。
///
/// 「下游有没有被执行」是本文件唯一的判据——只看运行状态不够，
/// 分支被错误放行时下游同样是 `Succeeded`。
#[derive(Default)]
struct CountingExecutor {
    calls: BTreeMap<String, usize>,
}

impl CountingExecutor {
    fn call_count(&self, node: &str) -> usize {
        self.calls.get(node).copied().unwrap_or(0)
    }
}

impl WorkflowExternalNodeExecutor for CountingExecutor {
    fn execute_external(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput> {
        *self
            .calls
            .entry(request.node_id.as_str().to_owned())
            .or_insert(0) += 1;
        // 顺带产出一个 flag，供 condition 读取（source 节点用）。
        let mut outputs = PortMap::new();
        outputs.insert("flag".to_owned(), PortValue::inline(true));
        Ok(WorkflowNodeExecutionOutput {
            outputs,
            ..WorkflowNodeExecutionOutput::default()
        })
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

/// condition 节点：读 `flag`，与 `expected` 做 equals 比较。
fn condition_node(id: &str, expected: bool) -> NodeInstance {
    NodeInstance {
        id: NodeId::from(id),
        type_name: "condition".to_owned(),
        label: None,
        config: json!({
            "input_alias": "flag",
            "operator": "equals",
            "expected": expected,
        }),
        position: None,
    }
}

/// 控制边；`from_port` 指定源引脚（U125 的关键参数）。
fn control_edge(id: &str, from: &str, from_port: &str, to: &str) -> Edge {
    Edge {
        id: EdgeId::from(id),
        kind: WorkflowEdgeKind::Control,
        from: PortEndpoint {
            node_id: NodeId::from(from),
            port_name: from_port.to_owned(),
        },
        to: PortEndpoint {
            node_id: NodeId::from(to),
            port_name: EXECUTION_INPUT_PORT.to_owned(),
        },
        alias: None,
        communication: None,
    }
}

fn flag_data_edge(id: &str, from: &str, to: &str) -> Edge {
    Edge {
        id: EdgeId::from(id),
        kind: WorkflowEdgeKind::Data,
        from: PortEndpoint {
            node_id: NodeId::from(from),
            port_name: "flag".to_owned(),
        },
        to: PortEndpoint {
            node_id: NodeId::from(to),
            port_name: "input".to_owned(),
        },
        alias: Some("flag".to_owned()),
        communication: None,
    }
}

/// 单条出边、从指定引脚拉出的最小图：source → condition → downstream。
fn single_branch_workflow(from_port: &str, expected: bool) -> WorkflowDefinition {
    WorkflowDefinition {
        id: WorkflowId::from("wf-single-branch"),
        name: "single branch".to_owned(),
        nodes: vec![
            node("source", "writer"),
            condition_node("cond", expected),
            node("downstream", "writer"),
        ],
        edges: vec![
            flag_data_edge("d1", "source", "cond"),
            control_edge("c1", "source", EXECUTION_OUTPUT_PORT, "cond"),
            control_edge("c2", "cond", from_port, "downstream"),
        ],
        metadata: Value::Null,
    }
}

fn run(workflow: &WorkflowDefinition) -> (RunStatus, CountingExecutor) {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let mut runtime = WorkflowRuntime::new(workflow, RunId::from("run-1")).unwrap();
    let mut external = CountingExecutor::default();
    let status = {
        let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);
        runtime
            .run_persisted(workflow, &mut executor, &store)
            .unwrap()
    };
    (status, external)
}

/// **U125 主用例**：单条线从「真」引脚拉出时，条件为假必须拦住下游。
///
/// 这是最常见的画法（「条件成立才继续」）。修复前：边的 alias 为空 →
/// 不进分支判定 → 恒放行，condition 节点完全不起作用。
#[test]
fn u125_single_true_branch_blocks_downstream_when_condition_is_false() {
    // expected=false 而 source 产出 flag=true → equals 判定为假 → branch="false"。
    let workflow = single_branch_workflow(EXECUTION_OUTPUT_PORT_TRUE, false);
    let (status, external) = run(&workflow);

    assert_eq!(status, RunStatus::Succeeded, "图本身应当跑完");
    assert_eq!(
        external.call_count("downstream"),
        0,
        "U125：线从「真」引脚拉出、而条件判为假，下游仍被执行了——\
         分支门禁没有生效（修复前 alias 为空的边恒放行，condition 形同虚设）"
    );
}

/// 反向对照：同一张图，条件为真时下游必须执行。
///
/// 没有这条，上一条可以被「把所有分支边一律拦掉」这种错误实现骗过。
#[test]
fn u125_single_true_branch_runs_downstream_when_condition_is_true() {
    let workflow = single_branch_workflow(EXECUTION_OUTPUT_PORT_TRUE, true);
    let (status, external) = run(&workflow);

    assert_eq!(status, RunStatus::Succeeded, "图本身应当跑完");
    assert_eq!(
        external.call_count("downstream"),
        1,
        "条件为真时「真」分支必须放行，否则 condition 变成了「永不通过」"
    );
}

/// 两条线分别从真/假引脚拉出时，只走被选中的那一支。
#[test]
fn u125_two_branch_pins_execute_only_the_selected_side() {
    for expected in [true, false] {
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("wf-two-branch"),
            name: "two branch".to_owned(),
            nodes: vec![
                node("source", "writer"),
                condition_node("cond", expected),
                node("on-true", "writer"),
                node("on-false", "writer"),
            ],
            edges: vec![
                flag_data_edge("d1", "source", "cond"),
                control_edge("c1", "source", EXECUTION_OUTPUT_PORT, "cond"),
                control_edge("c2", "cond", EXECUTION_OUTPUT_PORT_TRUE, "on-true"),
                control_edge("c3", "cond", EXECUTION_OUTPUT_PORT_FALSE, "on-false"),
            ],
            metadata: Value::Null,
        };
        let (status, external) = run(&workflow);

        assert_eq!(status, RunStatus::Succeeded, "图本身应当跑完");
        // source 恒产出 flag=true，故 expected 即为判定结果。
        let (taken, skipped) = if expected {
            ("on-true", "on-false")
        } else {
            ("on-false", "on-true")
        };
        assert_eq!(
            external.call_count(taken),
            1,
            "expected={expected}：被选中的 {taken} 分支必须执行"
        );
        assert_eq!(
            external.call_count(skipped),
            0,
            "expected={expected}：未选中的 {skipped} 分支仍被执行——双分支同时跑"
        );
    }
}

/// 保存边界：同一分支引脚不得重复连出。
///
/// 若放行，两条线都从「真」引脚出去，等于又回到「一次放行多条」的状态。
#[test]
fn u125_save_boundary_rejects_duplicate_edges_on_same_branch_pin() {
    let mut workflow = single_branch_workflow(EXECUTION_OUTPUT_PORT_TRUE, true);
    workflow.nodes.push(node("downstream-2", "writer"));
    workflow.edges.push(control_edge(
        "c3",
        "cond",
        EXECUTION_OUTPUT_PORT_TRUE,
        "downstream-2",
    ));

    let error = validate_workflow_execution_contracts(&workflow, &WorkflowExecutionLimits::default())
        .expect_err("同一分支引脚重复连出必须被保存边界拒绝");
    assert!(
        error.to_string().contains("branch port"),
        "错误信息应指明是分支引脚重复，实际：{error}"
    );
}

/// 保存边界：condition 的控制出边必须从两个分支引脚之一拉出。
///
/// 用通用 `exec_out` 画线是修复前的**默认**画法，必须显式拒绝——
/// 否则旧画法会静默退化成「恒放行」，与修复前无异。
#[test]
fn u125_save_boundary_rejects_condition_control_edge_on_generic_exec_out() {
    let workflow = single_branch_workflow(EXECUTION_OUTPUT_PORT, true);

    let error = validate_workflow_execution_contracts(&workflow, &WorkflowExecutionLimits::default())
        .expect_err("condition 用通用 exec_out 连出必须被拒绝");
    let message = error.to_string();
    assert!(
        message.contains(EXECUTION_OUTPUT_PORT_TRUE) && message.contains(EXECUTION_OUTPUT_PORT_FALSE),
        "错误信息应提示应当改用哪两个引脚，实际：{message}"
    );
}

/// 放开控制边端口校验后**不得**产生回归：
/// 普通节点之间的控制边仍必须是 `exec_out → exec_in`。
///
/// `validate_edge_kind` 为了容纳分支引脚从「必须是 exec_out」松成「三者之一」，
/// 这条锁住松开的边界不会溢出到非 condition 节点：普通节点挂分支引脚却永远
/// 不产出 `branch`，其下游会永久停在 Waiting，表现为工作流静默卡住。
#[test]
fn u125_branch_pins_are_rejected_on_non_condition_nodes() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-plain"),
        name: "plain".to_owned(),
        nodes: vec![node("a", "writer"), node("b", "writer")],
        edges: vec![control_edge("c1", "a", EXECUTION_OUTPUT_PORT_TRUE, "b")],
        metadata: Value::Null,
    };

    let error = validate_workflow_execution_contracts(&workflow, &WorkflowExecutionLimits::default())
        .expect_err("非 condition 节点使用分支引脚必须被拒绝");
    assert!(
        error.to_string().contains("not a condition node"),
        "错误信息应指明源节点不是 condition，实际：{error}"
    );
}

/// 普通控制边（`exec_out → exec_in`）在放开校验后仍必须被接受。
#[test]
fn u125_plain_control_edge_still_accepted() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-plain-ok"),
        name: "plain ok".to_owned(),
        nodes: vec![node("a", "writer"), node("b", "writer")],
        edges: vec![control_edge("c1", "a", EXECUTION_OUTPUT_PORT, "b")],
        metadata: Value::Null,
    };

    validate_workflow_execution_contracts(&workflow, &WorkflowExecutionLimits::default())
        .expect("普通节点之间的 exec_out → exec_in 控制边不应受 U125 影响");
}

/// 目标引脚不得随之放开：控制边终点仍必须是 `exec_in`。
#[test]
fn u125_control_edge_target_port_is_still_restricted_to_exec_in() {
    let mut workflow = single_branch_workflow(EXECUTION_OUTPUT_PORT_TRUE, true);
    workflow.edges[2].to.port_name = EXECUTION_OUTPUT_PORT_TRUE.to_owned();

    let error = workflow
        .validate_topology()
        .expect_err("控制边终点必须仍限定为 exec_in");
    assert!(
        error.to_string().contains(EXECUTION_INPUT_PORT),
        "错误信息应指明终点必须是 exec_in，实际：{error}"
    );
}

/// 节点目录必须声明 condition 的两个分支引脚。
///
/// 桌面画布从同一份 `workflow_node_catalog.json` 读引脚，
/// 声明缺失就意味着 UI 上根本渲染不出第二个引脚。
#[test]
fn u125_node_catalog_declares_two_branch_pins_for_condition() {
    let entry = ariadne::node_capabilities::workflow_node_catalog_entry("condition")
        .expect("节点目录必须有 condition 条目");
    assert_eq!(
        entry.execution_output_ports,
        vec![
            EXECUTION_OUTPUT_PORT_TRUE.to_owned(),
            EXECUTION_OUTPUT_PORT_FALSE.to_owned()
        ],
        "condition 必须声明两个分支执行出引脚，否则桌面画布渲染不出第二个引脚"
    );
    // 别名 eval 必须解析到同一条目，否则用 eval 画的图拿不到分支引脚。
    let alias = ariadne::node_capabilities::workflow_node_catalog_entry("eval")
        .expect("eval 别名必须解析到 condition 条目");
    assert_eq!(alias.execution_output_ports, entry.execution_output_ports);
}

/// 普通节点在目录里仍是单个通用执行出引脚（缺省行为不被 U125 改动）。
#[test]
fn u125_node_catalog_keeps_single_exec_out_for_other_nodes() {
    let entry = ariadne::node_capabilities::workflow_node_catalog_entry("writer")
        .expect("节点目录必须有 writer 条目");
    assert_eq!(
        entry.execution_output_ports,
        vec![EXECUTION_OUTPUT_PORT.to_owned()],
        "非 condition 节点应保持单个通用 exec_out"
    );
}
