//! Module 11 工作流变量契约。
//!
//! 单独成文件而不是并入 `workflow_contracts.rs`：变量是一条独立能力线
//! （声明 → 注入 → 循环写回 → 模板渲染），放在一起会让那个已超 5000 行的
//! 文件更难定位。
//!
//! ## 判据分层（为什么后半段测试要跑真实 `run_workflow_impl`）
//!
//! 前半段用内存 runtime 测语义（作用域链、类型校验、模板渲染）——那是纯函数
//! 行为，内存态足够。但**接线**类缺陷内存态测不出来：`inject_variables`、
//! `ensure_required_variables_present`、`run_completion_report` 三者都可以
//! 「实现完整 + 有测试覆盖 + 生产零调用者」，那正是 U108/U114/U117 那一类
//! 缺陷的形状，而它们当年都是「测试全绿」的。
//!
//! 所以启动校验与完成回报改用真实 `run_workflow_impl`，判据取**磁盘/对话表**：
//! - `required` 非空校验 → 断言 `run_workflow_impl` 真的返回 Err 且指名变量；
//! - 变量注入 → 断言 runtime.db 里存的是注入值；
//! - 完成回报 → 断言**对话表里有那条消息**，而不是 `structured_events` 有记录。
//!   最后这条是本能力的闭环判据：AI 不读事件表，它读对话。只落事件的实现下，
//!   `run_completion_report()` 照样能构造出载荷、测试照样全绿，而 AI 侧仍然
//!   收不到任何东西 —— 「启动即断线」。
use std::collections::BTreeMap;

use ariadne::contracts::{
    render_summary_template, validate_variable_decls, variable_value_is_blank, Edge, EdgeId, NodeId,
    NodeInstance, PortEndpoint, PortValue, RunId, RunStatus, WorkflowDefinition, WorkflowEdgeKind,
    WorkflowId, WorkflowVariableDecl, WorkflowVariableKind, EXECUTION_INPUT_PORT,
    EXECUTION_OUTPUT_PORT,
};
use ariadne::commands::{
    run_workflow_impl, save_workflow_graph_impl, CanvasNode, RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::MemorySecretStore;
use ariadne::frontend::ProjectAiConversationStore;
use ariadne::rag::{render_prompt_template, PromptTemplateContext};
use ariadne::workflow::{
    BuiltinWorkflowNodeExecutor, SqliteWorkflowRuntimeStore, WorkflowExternalNodeExecutor,
    WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest, WorkflowRuntime, WorkflowRuntimeStore,
    WorkflowVariableSource,
};
use serde_json::{json, Value};

/// 构造一个带变量声明的 start 节点。
fn start_node_with_variables(decls: Value) -> NodeInstance {
    NodeInstance {
        id: NodeId::from("start"),
        type_name: "start".to_owned(),
        label: None,
        config: json!({ "variables": decls }),
        position: None,
    }
}

fn plain_node(id: &str, type_name: &str) -> NodeInstance {
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

/// 只按脚本回放节点输出的执行器；用于驱动变量写回。
#[derive(Default)]
struct VariableScriptExecutor {
    /// node_id → 每次调用依次返回的输出。
    scripted: BTreeMap<String, Vec<WorkflowNodeExecutionOutput>>,
    /// node_id → 每次调用时看到的变量快照，用于断言跨轮可见性。
    observed: Vec<(NodeId, BTreeMap<String, Value>)>,
}

impl WorkflowExternalNodeExecutor for VariableScriptExecutor {
    fn execute_external(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowNodeExecutionOutput> {
        self.observed
            .push((request.node_id.clone(), request.variables.clone()));
        let key = request.node_id.as_str().to_owned();
        let queue = self.scripted.get_mut(&key);
        match queue {
            Some(queue) if !queue.is_empty() => Ok(queue.remove(0)),
            _ => Ok(WorkflowNodeExecutionOutput::default()),
        }
    }
}

/// 变量声明校验：三种类型都接受匹配的默认值。
#[test]
fn variable_decls_accept_all_three_kinds_with_defaults() {
    let decls = vec![
        WorkflowVariableDecl {
            name: "chapter".to_owned(),
            kind: WorkflowVariableKind::Number,
            default: json!(1),
            required: false,
            hidden: false,
        },
        WorkflowVariableDecl {
            name: "title".to_owned(),
            kind: WorkflowVariableKind::String,
            default: json!("序章"),
            required: false,
            hidden: false,
        },
        WorkflowVariableDecl {
            name: "polish".to_owned(),
            kind: WorkflowVariableKind::Boolean,
            default: json!(false),
            required: false,
            hidden: false,
        },
    ];

    validate_variable_decls(&decls).unwrap();
}

/// 默认值类型必须与声明匹配：字符串 "3" 不是 number。
#[test]
fn variable_decl_rejects_default_of_wrong_kind() {
    let decl = WorkflowVariableDecl {
        name: "chapter".to_owned(),
        kind: WorkflowVariableKind::Number,
        default: json!("3"),
        required: false,
        hidden: false,
    };

    let error = decl.validate().unwrap_err();
    assert!(
        error.to_string().contains("does not match kind"),
        "unexpected error: {error}"
    );
}

/// required 与 hidden 同时置位是矛盾声明，校验期直接拒绝。
#[test]
fn variable_decl_rejects_required_and_hidden_together() {
    let decl = WorkflowVariableDecl {
        name: "chapter".to_owned(),
        kind: WorkflowVariableKind::Number,
        default: json!(1),
        required: true,
        hidden: true,
    };

    let error = decl.validate().unwrap_err();
    assert!(
        error.to_string().contains("both required and hidden"),
        "unexpected error: {error}"
    );
}

/// `required` 且无默认值是**合法**声明，也是它最常见的写法。
///
/// 曾按「required = 必须人工确认」的误解加过「required 必须带 default 以供
/// 预填」的校验。按正确语义（占位符不能替换成空白），正因为没有默认值才逼着
/// 每次填上 —— 那条规则把最正常的用法禁了。这个测试钉住它不再回来。
#[test]
fn required_variable_without_default_is_legal() {
    let decl = WorkflowVariableDecl {
        name: "chapter".to_owned(),
        kind: WorkflowVariableKind::Number,
        default: Value::Null,
        required: true,
        hidden: false,
    };

    decl.validate().unwrap();
}

/// 变量名重复直接拒绝。
#[test]
fn variable_decls_reject_duplicate_names() {
    let decls = vec![
        WorkflowVariableDecl {
            name: "chapter".to_owned(),
            kind: WorkflowVariableKind::Number,
            default: json!(1),
            required: false,
            hidden: false,
        },
        WorkflowVariableDecl {
            name: "chapter".to_owned(),
            kind: WorkflowVariableKind::String,
            default: json!("x"),
            required: false,
            hidden: false,
        },
    ];

    let error = validate_variable_decls(&decls).unwrap_err();
    assert!(
        error.to_string().contains("duplicate workflow variable"),
        "unexpected error: {error}"
    );
}

/// 变量名不能带点号：会与 `{{var.名字}}` 的命名空间分隔符冲突。
#[test]
fn variable_name_rejects_namespace_separator() {
    let decl = WorkflowVariableDecl {
        name: "chapter.index".to_owned(),
        kind: WorkflowVariableKind::Number,
        default: json!(1),
        required: false,
        hidden: false,
    };

    let error = decl.validate().unwrap_err();
    assert!(
        error.to_string().contains("unsupported character"),
        "unexpected error: {error}"
    );
}

/// start 节点声明的默认值进入根作用域，节点执行时可读。
#[test]
fn start_node_declarations_seed_root_scope() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "chapter", "kind": "number", "default": 1 },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };

    let runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    assert_eq!(runtime.state.variables.get("chapter"), Some(&json!(1)));
    assert_eq!(runtime.state.variable_decls.len(), 1);
}

/// 注入值类型不匹配时启动前失败，不做隐式转换。
#[test]
fn injected_variable_rejects_type_mismatch() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "chapter", "kind": "number", "default": 1 },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    let mut values = BTreeMap::new();
    values.insert("chapter".to_owned(), json!("3"));
    let error = runtime
        .inject_variables(&values, WorkflowVariableSource::ProjectAi)
        .unwrap_err();

    assert!(
        error.to_string().contains("expects number but received"),
        "unexpected error: {error}"
    );
}

/// hidden 变量拒绝来自执行页的注入，但接受项目空间 AI 注入。
#[test]
fn hidden_variable_rejects_execution_page_injection_only() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "internal", "kind": "number", "default": 0, "hidden": true },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    let mut values = BTreeMap::new();
    values.insert("internal".to_owned(), json!(7));

    let error = runtime
        .inject_variables(&values, WorkflowVariableSource::ExecutionPage)
        .unwrap_err();
    assert!(
        error.to_string().contains("hidden"),
        "unexpected error: {error}"
    );

    // 自动化路径仍可写入，否则 hidden 变量将完全无法被赋值。
    runtime
        .inject_variables(&values, WorkflowVariableSource::ProjectAi)
        .unwrap();
    assert_eq!(runtime.state.variables.get("internal"), Some(&json!(7)));
}

/// 注入未声明的变量名直接失败，避免静默产生野变量。
#[test]
fn injected_variable_rejects_unknown_name() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "chapter", "kind": "number", "default": 1 },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    let mut values = BTreeMap::new();
    values.insert("nonexistent".to_owned(), json!(1));
    let error = runtime
        .inject_variables(&values, WorkflowVariableSource::ProjectAi)
        .unwrap_err();

    assert!(
        error.to_string().contains("unknown workflow variable"),
        "unexpected error: {error}"
    );
}

/// 赋值优先级：执行页人工输入覆盖项目空间 AI 注入，两者都覆盖默认值。
#[test]
fn execution_page_input_overrides_project_ai_injection() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "chapter", "kind": "number", "default": 1 },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    assert_eq!(runtime.state.variables.get("chapter"), Some(&json!(1)));

    let mut ai = BTreeMap::new();
    ai.insert("chapter".to_owned(), json!(5));
    runtime
        .inject_variables(&ai, WorkflowVariableSource::ProjectAi)
        .unwrap();
    assert_eq!(runtime.state.variables.get("chapter"), Some(&json!(5)));

    let mut manual = BTreeMap::new();
    manual.insert("chapter".to_owned(), json!(9));
    runtime
        .inject_variables(&manual, WorkflowVariableSource::ExecutionPage)
        .unwrap();
    assert_eq!(runtime.state.variables.get("chapter"), Some(&json!(9)));
}

/// 节点执行时能在请求里读到变量当前值。
#[test]
fn node_request_carries_current_variable_values() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![
            start_node_with_variables(json!([
                { "name": "chapter", "kind": "number", "default": 3 },
            ])),
            plain_node("writer", "writer"),
        ],
        edges: vec![control_edge("c1", "start", "writer")],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut external = VariableScriptExecutor::default();
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Succeeded);

    let writer_view = external
        .observed
        .iter()
        .find(|(node_id, _)| node_id.as_str() == "writer")
        .map(|(_, vars)| vars.clone())
        .expect("writer should have been executed");
    assert_eq!(writer_view.get("chapter"), Some(&json!(3)));
}

/// 节点写回的变量落进运行状态。
#[test]
fn node_variable_write_lands_in_run_state() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![
            start_node_with_variables(json!([
                { "name": "chapter", "kind": "number", "default": 1 },
            ])),
            plain_node("writer", "writer"),
        ],
        edges: vec![control_edge("c1", "start", "writer")],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    let mut writes = BTreeMap::new();
    writes.insert("chapter".to_owned(), json!(2));
    let mut external = VariableScriptExecutor::default();
    external.scripted.insert(
        "writer".to_owned(),
        vec![WorkflowNodeExecutionOutput {
            variable_writes: writes,
            ..WorkflowNodeExecutionOutput::default()
        }],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(runtime.state.variables.get("chapter"), Some(&json!(2)));
}

/// 写回类型不匹配时保留原值，并记录被拒事件，不静默改写。
#[test]
fn node_variable_write_of_wrong_type_is_rejected_and_recorded() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![
            start_node_with_variables(json!([
                { "name": "chapter", "kind": "number", "default": 1 },
            ])),
            plain_node("writer", "writer"),
        ],
        edges: vec![control_edge("c1", "start", "writer")],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    let mut writes = BTreeMap::new();
    writes.insert("chapter".to_owned(), json!("第二章"));
    let mut external = VariableScriptExecutor::default();
    external.scripted.insert(
        "writer".to_owned(),
        vec![WorkflowNodeExecutionOutput {
            variable_writes: writes,
            ..WorkflowNodeExecutionOutput::default()
        }],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    runtime.run(&workflow, &mut executor).unwrap();

    // 原值保留，不被错误类型污染。
    assert_eq!(runtime.state.variables.get("chapter"), Some(&json!(1)));
    let rejected = runtime
        .state
        .structured_events
        .iter()
        .any(|event| format!("{:?}", event.event_type).contains("VariableWriteRejected"));
    assert!(rejected, "expected a VariableWriteRejected event");
}

/// 写回未声明的变量名被拒绝，不产生野变量。
#[test]
fn node_variable_write_of_unknown_name_is_rejected() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![
            start_node_with_variables(json!([
                { "name": "chapter", "kind": "number", "default": 1 },
            ])),
            plain_node("writer", "writer"),
        ],
        edges: vec![control_edge("c1", "start", "writer")],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    let mut writes = BTreeMap::new();
    writes.insert("ghost".to_owned(), json!(1));
    let mut external = VariableScriptExecutor::default();
    external.scripted.insert(
        "writer".to_owned(),
        vec![WorkflowNodeExecutionOutput {
            variable_writes: writes,
            ..WorkflowNodeExecutionOutput::default()
        }],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(runtime.state.variables.get("ghost"), None);
}

/// 核心语义：循环体内的写回对下一轮可见。
///
/// 这是「第一章写完了写第二章」成立的前提 —— 若每轮从初值重新开始
/// （快照隔离），writer 永远只看到 chapter=1。
#[test]
fn loop_variable_write_back_is_visible_in_next_iteration() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Loop vars".to_owned(),
        nodes: vec![
            start_node_with_variables(json!([
                { "name": "chapter", "kind": "number", "default": 1 },
            ])),
            plain_node("writer", "writer"),
            NodeInstance {
                id: NodeId::from("loop"),
                type_name: "loop".to_owned(),
                label: None,
                config: json!({
                    "max_iterations": 3,
                    "timeout_ms": 30_000,
                    "stop_condition": { "input_alias": "done", "equals": true },
                    "rerun_node_ids": ["writer"]
                }),
                position: None,
            },
        ],
        edges: vec![
            control_edge("c1", "start", "writer"),
            control_edge("c2", "writer", "loop"),
            control_edge("c3", "loop", "writer"),
            Edge {
                id: EdgeId::from("d1"),
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: "done".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("loop"),
                    port_name: "condition".to_owned(),
                },
                alias: Some("done".to_owned()),
                communication: None,
            },
        ],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    // writer 三轮：每轮把 chapter 加 1，第三轮宣布 done。
    let mut external = VariableScriptExecutor::default();
    let round = |chapter: i64, done: bool| {
        let mut writes = BTreeMap::new();
        writes.insert("chapter".to_owned(), json!(chapter));
        let mut outputs = ariadne::contracts::PortMap::new();
        outputs.insert("done".to_owned(), PortValue::inline(done));
        WorkflowNodeExecutionOutput {
            outputs,
            variable_writes: writes,
            ..WorkflowNodeExecutionOutput::default()
        }
    };
    external.scripted.insert(
        "writer".to_owned(),
        vec![round(2, false), round(3, false), round(3, true)],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Succeeded);

    // writer 每轮看到的 chapter 应递增，而不是恒为初值 1。
    let seen = external
        .observed
        .iter()
        .filter(|(node_id, _)| node_id.as_str() == "writer")
        .map(|(_, vars)| vars.get("chapter").cloned().unwrap_or(Value::Null))
        .collect::<Vec<_>>();
    assert_eq!(
        seen,
        vec![json!(1), json!(2), json!(3)],
        "loop write-back must be visible in the next iteration"
    );
}

/// 变量当前值参与 request_hash：不同轮次的请求不得撞成同一次重放。
#[test]
fn variable_values_participate_in_request_hash() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![
            start_node_with_variables(json!([
                { "name": "chapter", "kind": "number", "default": 1 },
            ])),
            plain_node("writer", "writer"),
        ],
        edges: vec![control_edge("c1", "start", "writer")],
        metadata: Value::Null,
    };

    let hash_with = |chapter: i64| {
        let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
        let mut values = BTreeMap::new();
        values.insert("chapter".to_owned(), json!(chapter));
        runtime
            .inject_variables(&values, WorkflowVariableSource::ProjectAi)
            .unwrap();
        let mut external = VariableScriptExecutor::default();
        let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);
        runtime.run(&workflow, &mut executor).unwrap();
        external
            .observed
            .iter()
            .find(|(node_id, _)| node_id.as_str() == "writer")
            .map(|(_, vars)| vars.clone())
            .unwrap()
    };

    assert_ne!(hash_with(1), hash_with(2));
}

/// `{{var.名字}}` 复用既有模板管线渲染。
#[test]
fn template_renders_var_namespace() {
    let mut variables = BTreeMap::new();
    variables.insert("chapter".to_owned(), json!(2));
    variables.insert("title".to_owned(), json!("觉醒"));
    variables.insert("polish".to_owned(), json!(true));
    let context = PromptTemplateContext::default().with_variables(variables);

    let rendered = render_prompt_template(
        "写第{{var.chapter}}章《{{var.title}}》，润色={{var.polish}}",
        &context,
    )
    .unwrap();

    assert_eq!(rendered, "写第2章《觉醒》，润色=true");
}

/// 未声明的变量引用在渲染期报缺失变量，不静默替换为空串。
#[test]
fn template_rejects_unresolved_var_reference() {
    let context = PromptTemplateContext::default();

    let error = render_prompt_template("第{{var.chapter}}章", &context).unwrap_err();

    assert!(
        error.to_string().contains("unresolved"),
        "unexpected error: {error}"
    );
}

/// 字符串变量渲染时不带 JSON 引号。
#[test]
fn string_variable_renders_without_json_quotes() {
    let mut variables = BTreeMap::new();
    variables.insert("title".to_owned(), json!("觉醒"));
    let context = PromptTemplateContext::default().with_variables(variables);

    let rendered = render_prompt_template("《{{var.title}}》", &context).unwrap();

    assert_eq!(rendered, "《觉醒》");
}

// ── 空值判定 ────────────────────────────────────────────────────────────

/// `0` 与 `false` 不算空：`chapter=0`、`polish=false` 都是合法取值。
#[test]
fn zero_and_false_are_not_blank_values() {
    assert!(!variable_value_is_blank(&json!(0)));
    assert!(!variable_value_is_blank(&json!(false)));
    assert!(!variable_value_is_blank(&json!("雪落时")));
}

/// `null` 与空白串算空 —— 它们替换进模板后会在稿子里留个洞。
#[test]
fn null_and_whitespace_are_blank_values() {
    assert!(variable_value_is_blank(&Value::Null));
    assert!(variable_value_is_blank(&json!("")));
    assert!(variable_value_is_blank(&json!("   ")));
}

/// required 变量取空串时拒绝启动，并指名是哪个变量。
#[test]
fn blank_required_variable_blocks_start() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "title", "kind": "string", "required": true },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    // 无默认值时根层没有这个键，同样算空。
    let error = runtime.ensure_required_variables_present().unwrap_err();
    assert!(
        error.to_string().contains("title") && error.to_string().contains("blank"),
        "unexpected error: {error}"
    );

    // 填了空白串仍算空。
    let mut blank = BTreeMap::new();
    blank.insert("title".to_owned(), json!("  "));
    runtime
        .inject_variables(&blank, WorkflowVariableSource::ExecutionPage)
        .unwrap();
    runtime.ensure_required_variables_present().unwrap_err();

    // 填了真实值放行。
    let mut filled = BTreeMap::new();
    filled.insert("title".to_owned(), json!("雪落时"));
    runtime
        .inject_variables(&filled, WorkflowVariableSource::ExecutionPage)
        .unwrap();
    runtime.ensure_required_variables_present().unwrap();
}

/// required 的数字变量取 0 时放行 —— 第 0 章是合法的。
#[test]
fn required_number_variable_accepts_zero() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "chapter", "kind": "number", "required": true },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    let mut values = BTreeMap::new();
    values.insert("chapter".to_owned(), json!(0));
    runtime
        .inject_variables(&values, WorkflowVariableSource::ProjectAi)
        .unwrap();

    runtime.ensure_required_variables_present().unwrap();
}

// ── 摘要句式 ────────────────────────────────────────────────────────────

/// 句式把占位符替换成当前取值，字符串不带 JSON 引号。
#[test]
fn summary_template_renders_current_values() {
    let mut values = BTreeMap::new();
    values.insert("chapter".to_owned(), json!(3));
    values.insert("title".to_owned(), json!("雪落时"));
    values.insert("tone".to_owned(), json!("克制"));

    let rendered = render_summary_template(
        "写第{{var.chapter}}章《{{var.title}}》，落笔要{{var.tone}}",
        &values,
    );

    assert_eq!(rendered, "写第3章《雪落时》，落笔要克制");
}

/// 缺失取值就地留空，不输出占位符本身。
///
/// 折叠行是给人看的预览：句子里那个空洞本身就是「稿子里会缺这块」的提示，
/// 比把 `{{var.title}}` 原样露出来更能说明问题。
#[test]
fn summary_template_leaves_blank_for_missing_value() {
    let mut values = BTreeMap::new();
    values.insert("chapter".to_owned(), json!(3));

    let rendered = render_summary_template("写第{{var.chapter}}章《{{var.title}}》", &values);

    assert_eq!(rendered, "写第3章《》");
}

/// 句式可以引用 `hidden` 变量：取值真实替换，只是表单里改不到。
#[test]
fn summary_template_renders_hidden_variable_reference() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("start"),
            type_name: "start".to_owned(),
            label: None,
            config: json!({
                "variables": [
                    { "name": "chapter", "kind": "number", "default": 2 },
                    { "name": "pass", "kind": "number", "default": 7, "hidden": true },
                ],
                "summary_template": "第{{var.chapter}}章（内部轮次 {{var.pass}}）",
            }),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };

    let runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    assert_eq!(runtime.render_variable_summary(), "第2章（内部轮次 7）");
}

/// 未闭合的括号按字面量输出，不吞掉作者写的内容。
#[test]
fn summary_template_keeps_unclosed_braces_literal() {
    let values = BTreeMap::new();
    assert_eq!(
        render_summary_template("写第{{var.chapter 章", &values),
        "写第{{var.chapter 章"
    );
}

/// 非 `var.` 命名空间原样保留，便于排查写错的句式。
#[test]
fn summary_template_preserves_other_namespaces() {
    let values = BTreeMap::new();
    assert_eq!(
        render_summary_template("{{input.x}} 与 {{var.y}}", &values),
        "{{input.x}} 与 "
    );
}

/// 缺省句式退回按声明顺序拼接，且跳过 `hidden` 变量。
#[test]
fn summary_falls_back_to_declaration_order_without_hidden() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "chapter", "kind": "number", "default": 3 },
            { "name": "title", "kind": "string", "default": "雪落时" },
            { "name": "pass", "kind": "number", "default": 7, "hidden": true },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };

    let runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    assert_eq!(runtime.render_variable_summary(), "chapter 3 · title 雪落时");
}

/// 执行页待输入清单不含 `hidden` 变量，也不给计数。
#[test]
fn visible_variable_decls_exclude_hidden() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "title", "kind": "string", "default": "雪落时" },
            { "name": "pass", "kind": "number", "default": 7, "hidden": true },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };

    let runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

    let visible = runtime.visible_variable_decls();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].name, "title");
}

// ── 运行完成回报 ────────────────────────────────────────────────────────

/// 人工启动的运行没有发起对话，不产生回报。
#[test]
fn manual_run_produces_no_completion_report() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([
            { "name": "chapter", "kind": "number", "default": 1 },
        ]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut external = VariableScriptExecutor::default();
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);
    runtime.run(&workflow, &mut executor).unwrap();

    assert!(runtime.run_completion_report().is_none());
}

/// 项目 AI 启动的运行跑完后产生回报，含状态与变量终值。
///
/// 变量终值是闭环的关键：AI 据此判断循环推进到哪一章，才能接着写下一章。
#[test]
fn project_ai_run_reports_completion_with_final_variables() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![
            start_node_with_variables(json!([
                { "name": "chapter", "kind": "number", "default": 1 },
            ])),
            plain_node("writer", "writer"),
        ],
        edges: vec![control_edge("c1", "start", "writer")],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    runtime.state.origin_conversation_id = Some("conv-1".to_owned());

    let mut writes = BTreeMap::new();
    writes.insert("chapter".to_owned(), json!(3));
    let mut external = VariableScriptExecutor::default();
    external.scripted.insert(
        "writer".to_owned(),
        vec![WorkflowNodeExecutionOutput {
            variable_writes: writes,
            ..WorkflowNodeExecutionOutput::default()
        }],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);
    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Succeeded);

    let report = runtime
        .run_completion_report()
        .expect("project AI run must report completion");
    assert_eq!(report.conversation_id, "conv-1");
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(report.variables.get("chapter"), Some(&json!(3)));
}

/// 未到终态时不回报 —— 否则 AI 会以为跑完了。
#[test]
fn queued_run_produces_no_completion_report() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![start_node_with_variables(json!([]))],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    runtime.state.origin_conversation_id = Some("conv-1".to_owned());

    // 尚未运行，状态是 Queued。
    assert!(runtime.run_completion_report().is_none());
}

// ── 闭环用例：回报必须真实落进对话 ──────────────────────────────────────
//
// 上面三条只断言 `run_completion_report()` 能**构造**出载荷。那种判据挡不住
// 本提案要修的缺陷形状：只要 `deliver_workflow_run_completion` 没有在
// `execute_workflow_runtime` 里被调用，载荷照样构造得出来，测试照样全绿，
// 而 AI 侧仍然收不到任何东西 —— 这正是 U117「能构造出确认项」而非
// 「run_workflow_impl 之后确认项是否真入库」的假测试形状。
//
// 所以下面走**真实 `run_workflow_impl`**，判据取**对话表里的消息**。

/// 落一个只有 start 节点的工作流；不需要 provider，因此无需配 LLM 假服务。
///
/// 变量声明挂在 start 节点的 `data` 上 —— `graph_to_workflow` 把 `data`
/// 原样搬进 `NodeInstance::config`，`collect_variable_decls` 读的就是它。
fn save_start_only_workflow(project_root: &std::path::Path, workflow_id: &str, data: Value) {
    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: workflow_id.to_owned(),
            name: workflow_id.to_owned(),
            nodes: vec![CanvasNode {
                id: "start".to_owned(),
                r#type: "start".to_owned(),
                label: None,
                data,
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

/// 读某个对话里的全部消息内容。
fn conversation_contents(project_root: &std::path::Path, conversation_id: &str) -> Vec<String> {
    ProjectAiConversationStore::open(project_root)
        .expect("打开对话库应当成功")
        .load(conversation_id)
        .expect("载入对话应当成功")
        .messages
        .into_iter()
        .map(|message| message.content)
        .collect()
}

/// **闭环主用例**：项目 AI 启动的运行到达终态后，发起对话必须真的收到完成消息。
///
/// 判据取**对话载荷**而不是 `structured_events`：AI 不读事件表，它读对话。
/// 只落事件等于没有回报 —— 那种实现下 AI 仍然是「启动即断线」。
#[test]
fn project_ai_run_delivers_completion_message_into_conversation() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets = MemorySecretStore::default();
    save_start_only_workflow(
        project.path(),
        "closed-loop",
        json!({
            "variables": [{ "name": "chapter", "kind": "number", "default": 2 }],
        }),
    );

    let started = run_workflow_impl(
        project.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "closed-loop".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
            variables: BTreeMap::new(),
            // 关键：标明发起对话。人工启动时这里是 None，也就不该有回报。
            origin_conversation_id: Some("conv-closed-loop".to_owned()),
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .expect("运行工作流应当成功");
    assert_eq!(started.status, "succeeded");

    let contents = conversation_contents(project.path(), "conv-closed-loop");
    let report = contents
        .iter()
        .find(|content| content.contains("[工作流运行回报]"))
        .unwrap_or_else(|| {
            panic!("发起对话必须真实收到完成回报，实际消息：{contents:?}")
        });

    // run 标识、终态、变量终值都要在，AI 才能判断推进到哪一章。
    assert!(report.contains("closed-loop"), "回报缺工作流标识：{report}");
    assert!(
        report.contains(&started.run_id),
        "回报缺 run 标识：{report}"
    );
    assert!(report.contains("已完成"), "回报缺终态：{report}");
    assert!(report.contains("chapter=2"), "回报缺变量终值：{report}");
}

/// 人工启动（无发起对话）不得往任何对话里写东西。
///
/// 与上一条互为对照：若实现改成「无脑给某个默认对话发消息」，这条会转红。
#[test]
fn manual_run_delivers_nothing_to_any_conversation() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets = MemorySecretStore::default();
    save_start_only_workflow(
        project.path(),
        "manual-run",
        json!({
            "variables": [{ "name": "chapter", "kind": "number", "default": 1 }],
        }),
    );

    // 先建出对话，确保「表里没有这条消息」不是因为对话不存在。
    let store = ProjectAiConversationStore::open(project.path()).unwrap();
    store
        .load_or_seed(
            "conv-manual",
            &[("user".to_owned(), "先聊两句".to_owned())],
        )
        .unwrap();

    run_workflow_impl(
        project.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "manual-run".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
            variables: BTreeMap::new(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .expect("运行工作流应当成功");

    let contents = conversation_contents(project.path(), "conv-manual");
    assert!(
        !contents.iter().any(|c| c.contains("[工作流运行回报]")),
        "人工启动不该产生回报，实际消息：{contents:?}"
    );
}

/// 项目 AI 传入的变量真的进了这次运行 —— 回报里的终值即证据。
///
/// 这条钉住第 4 项（IPC 启动时传入变量）的**生产链路**：变量从
/// `RunWorkflowRequest` 一路走到 run state，而不是被静默丢掉。
#[test]
fn project_ai_injected_variables_reach_the_run() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets = MemorySecretStore::default();
    save_start_only_workflow(
        project.path(),
        "inject",
        json!({
            "variables": [
                { "name": "chapter", "kind": "number", "default": 1 },
                { "name": "title", "kind": "string", "default": "序章" },
            ],
        }),
    );

    let mut variables = BTreeMap::new();
    variables.insert("chapter".to_owned(), json!(7));
    variables.insert("title".to_owned(), json!("雪落时"));
    let started = run_workflow_impl(
        project.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "inject".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
            variables,
            origin_conversation_id: Some("conv-inject".to_owned()),
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .expect("运行工作流应当成功");

    // 判据一：运行态里存的是注入值，不是声明的默认值。
    let state = SqliteWorkflowRuntimeStore::open(project.path())
        .unwrap()
        .load_state(
            &WorkflowId::from("inject"),
            &RunId::from(started.run_id.as_str()),
        )
        .unwrap()
        .expect("运行态应当落库");
    assert_eq!(state.variables.get("chapter"), Some(&json!(7)));
    assert_eq!(state.variables.get("title"), Some(&json!("雪落时")));
    // 发起对话 id 必须随快照持久化，否则 Resume 后终态无从回报。
    assert_eq!(
        state.origin_conversation_id.as_deref(),
        Some("conv-inject")
    );

    // 判据二：回报里的变量终值也是注入值。
    let contents = conversation_contents(project.path(), "conv-inject");
    let report = contents
        .iter()
        .find(|content| content.contains("[工作流运行回报]"))
        .unwrap_or_else(|| panic!("应当收到回报，实际消息：{contents:?}"));
    assert!(report.contains("chapter=7"), "回报变量终值不对：{report}");
    assert!(report.contains("title=雪落时"), "回报变量终值不对：{report}");
}

/// `required` 变量取空时，运行在建 run 之前就被拒，且错误指名变量。
///
/// 走真实 `run_workflow_impl` 而非只调 `ensure_required_variables_present`：
/// 后者证明不了这道校验被接进了启动链路。
#[test]
fn blank_required_variable_blocks_real_start_and_names_it() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets = MemorySecretStore::default();
    save_start_only_workflow(
        project.path(),
        "required-gate",
        json!({
            "variables": [{ "name": "title", "kind": "string", "required": true }],
        }),
    );

    // 不填：required 无默认值，根层没有这个键，算空 → 拒绝启动。
    let error = run_workflow_impl(
        project.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "required-gate".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
            variables: BTreeMap::new(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .expect_err("required 变量留空必须拒绝启动");
    let message = format!("{error:?}");
    assert!(
        message.contains("title"),
        "拒绝理由必须指名变量：{message}"
    );

    // 填空白串仍算空。
    let mut blank = BTreeMap::new();
    blank.insert("title".to_owned(), json!("   "));
    run_workflow_impl(
        project.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "required-gate".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
            variables: blank,
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .expect_err("空白串必须视为空");

    // 填真实值放行。
    let mut filled = BTreeMap::new();
    filled.insert("title".to_owned(), json!("雪落时"));
    let started = run_workflow_impl(
        project.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "required-gate".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
            variables: filled,
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .expect("填了真实值应当放行");
    assert_eq!(started.status, "succeeded");
}

/// `required` 的 `0` / `false` 是合法取值，必须放行到真实运行。
///
/// 这条是「空值判定只认 null 和空串」的生产侧证据：若判据误用
/// JSON falsy，`chapter=0` 与 `polish=false` 都会被错拦。
#[test]
fn required_zero_and_false_pass_real_start() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets = MemorySecretStore::default();
    save_start_only_workflow(
        project.path(),
        "falsy-ok",
        json!({
            "variables": [
                { "name": "chapter", "kind": "number", "required": true },
                { "name": "polish", "kind": "boolean", "required": true },
            ],
        }),
    );

    let mut variables = BTreeMap::new();
    variables.insert("chapter".to_owned(), json!(0));
    variables.insert("polish".to_owned(), json!(false));
    let started = run_workflow_impl(
        project.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "falsy-ok".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
            variables,
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .expect("0 与 false 是合法取值，必须放行");
    assert_eq!(started.status, "succeeded");
}

/// 循环写回后，折叠行显示的是最新取值。
#[test]
fn summary_reflects_loop_write_back() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![
            NodeInstance {
                id: NodeId::from("start"),
                type_name: "start".to_owned(),
                label: None,
                config: json!({
                    "variables": [{ "name": "chapter", "kind": "number", "default": 1 }],
                    "summary_template": "写第{{var.chapter}}章",
                }),
                position: None,
            },
            plain_node("writer", "writer"),
        ],
        edges: vec![control_edge("c1", "start", "writer")],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    assert_eq!(runtime.render_variable_summary(), "写第1章");

    let mut writes = BTreeMap::new();
    writes.insert("chapter".to_owned(), json!(4));
    let mut external = VariableScriptExecutor::default();
    external.scripted.insert(
        "writer".to_owned(),
        vec![WorkflowNodeExecutionOutput {
            variable_writes: writes,
            ..WorkflowNodeExecutionOutput::default()
        }],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);
    runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(runtime.render_variable_summary(), "写第4章");
}

// ── 嵌套作用域与 Resume ─────────────────────────────────────────────────

/// 嵌套循环：内层的局部量不污染外层。
///
/// 作用域链的写入规则是「落到声明该变量的那一层，没有声明则落最内层」。
/// 已声明变量（`chapter`）不论在第几层写都回到根层，因此跨轮可见；未声明的
/// 局部量（`scratch`）落在内层帧，随帧一起丢弃 —— 这两条合起来就是
/// 「嵌套循环互不污染」。若写入无条件落最内层，`chapter` 的累积值会随内层
/// 弹帧一起消失；若无条件落根层，内层局部量会漏到外层。两种错法都让这条转红。
#[test]
fn inner_loop_frame_does_not_pollute_outer_scope() {
    use ariadne::workflow::WorkflowVariableScopes;

    let mut root = BTreeMap::new();
    root.insert("chapter".to_owned(), json!(1));
    let mut scopes = WorkflowVariableScopes::with_root(root);

    // 外层循环开帧后写已声明变量：按规则回到根层，跨轮可见。
    scopes.push_loop_frame(NodeId::from("outer"));
    scopes.set("chapter", json!(2));
    assert_eq!(scopes.frames[0].values.get("chapter"), Some(&json!(2)));

    // 内层循环开帧后写一个**未声明**的局部量：落在内层帧，不进根层。
    scopes.push_loop_frame(NodeId::from("inner"));
    scopes.set("scratch", json!("内层草稿"));
    assert_eq!(scopes.get("scratch"), Some(&json!("内层草稿")));
    assert!(
        !scopes.frames[0].values.contains_key("scratch"),
        "未声明的局部量不得落到根层"
    );

    // 内层再写已声明变量：仍然回到根层，内层帧不留副本。
    scopes.set("chapter", json!(3));
    assert_eq!(scopes.frames[0].values.get("chapter"), Some(&json!(3)));

    // 内层结束：局部量随帧丢弃，已声明变量的累积值仍在。
    scopes.pop_loop_frame(&NodeId::from("inner"));
    assert_eq!(scopes.get("scratch"), None, "内层局部量必须随帧丢弃");
    assert_eq!(scopes.get("chapter"), Some(&json!(3)));

    // 弹帧只弹自己开的那层：拿内层 id 再弹一次不该动到外层帧。
    scopes.pop_loop_frame(&NodeId::from("inner"));
    assert_eq!(scopes.frames.len(), 2, "只能弹掉自己开的那一层");

    scopes.pop_loop_frame(&NodeId::from("outer"));
    assert_eq!(scopes.frames.len(), 1);
    assert_eq!(scopes.get("chapter"), Some(&json!(3)));
}

/// Resume 后变量从**最后一次写回值**继续，不回退到声明默认值。
///
/// 判据取「重建 runtime 后 writer 看到的取值」：`WorkflowRuntime::new` 会把
/// 声明默认值重新写进根层，若 Resume 走的是 `new` 而不是 `from_state`，
/// chapter 会退回 1，循环等于从第一章重来 —— 那正是本能力要消除的故障。
#[test]
fn resume_continues_from_last_written_variable_value() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![
            start_node_with_variables(json!([
                { "name": "chapter", "kind": "number", "default": 1 },
            ])),
            plain_node("writer", "writer"),
        ],
        edges: vec![control_edge("c1", "start", "writer")],
        metadata: Value::Null,
    };

    // 第一段运行：写回 chapter=5。
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut writes = BTreeMap::new();
    writes.insert("chapter".to_owned(), json!(5));
    let mut external = VariableScriptExecutor::default();
    external.scripted.insert(
        "writer".to_owned(),
        vec![WorkflowNodeExecutionOutput {
            variable_writes: writes,
            ..WorkflowNodeExecutionOutput::default()
        }],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);
    runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(runtime.state.variables.get("chapter"), Some(&json!(5)));

    // 快照过一遍 JSON：Resume 实际是从 runtime.db 的 state_json 反序列化回来的，
    // 直接复用内存里的 state 会漏掉「字段没进序列化」这类缺陷。
    let snapshot = serde_json::to_string(&runtime.state).unwrap();
    let restored: ariadne::workflow::WorkflowRunState =
        serde_json::from_str(&snapshot).unwrap();
    assert_eq!(restored.variables.get("chapter"), Some(&json!(5)));

    // 从快照恢复的 runtime 必须看到 5，而不是声明默认值 1。
    let resumed = WorkflowRuntime::from_state(restored);
    assert_eq!(resumed.state.variables.get("chapter"), Some(&json!(5)));
    // 已完成的节点保持 Succeeded —— Resume 不重放已完成轮次。
    assert_eq!(
        resumed.state.nodes[&NodeId::from("writer")].status,
        RunStatus::Succeeded
    );
}

/// 摘要句式随快照持久化，Resume 后折叠行仍按句式渲染。
///
/// `summary_template` 若不进 `state_json`，Resume 后折叠行会退回
/// 「按声明顺序拼接」，作者会以为自己写的句式丢了。
#[test]
fn summary_template_survives_state_round_trip() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Vars".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("start"),
            type_name: "start".to_owned(),
            label: None,
            config: json!({
                "variables": [{ "name": "chapter", "kind": "number", "default": 3 }],
                "summary_template": "写第{{var.chapter}}章",
            }),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    assert_eq!(runtime.render_variable_summary(), "写第3章");

    let snapshot = serde_json::to_string(&runtime.state).unwrap();
    let restored: ariadne::workflow::WorkflowRunState =
        serde_json::from_str(&snapshot).unwrap();
    let resumed = WorkflowRuntime::from_state(restored);

    assert_eq!(resumed.render_variable_summary(), "写第3章");
}

/// **变量整簇必须真的进 runtime.db** —— 判据取「经真实 store 存取一轮」。
///
/// 上面几条 round-trip 用例走的是 `serde_json::to_string(&runtime.state)`，
/// 那是 `WorkflowRunState` 自己的 `Serialize`。而落库实际走 `store.rs` 里
/// 手抄的投影结构 `PersistedWorkflowRunState`，**两者字段集可以不一致**：
/// 投影漏掉的字段序列化时被丢弃，读回时又因 `#[serde(default)]` 静默变成
/// 空默认值 —— 全程不报错。variables / variable_decls / summary_template /
/// origin_conversation_id 四个字段就曾整簇漏在投影之外，表现为
/// 「注入的取值进不了库、AI 启动的运行丢掉发起对话 id」。
///
/// 所以这条必须经 `SqliteWorkflowRuntimeStore` 真存真取，不能只测
/// `WorkflowRunState` 自身的序列化 —— 那正是当初漏掉这个缺陷的判据。
#[test]
fn variable_cluster_survives_a_real_store_round_trip() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("persist"),
        name: "Vars".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("start"),
            type_name: "start".to_owned(),
            label: None,
            config: json!({
                "variables": [
                    { "name": "chapter", "kind": "number", "default": 1 },
                    { "name": "pass", "kind": "number", "default": 0, "hidden": true },
                ],
                "summary_template": "写第{{var.chapter}}章",
            }),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-persist")).unwrap();

    // 模拟循环跑到第 3 章、并记下发起对话。
    let mut values = BTreeMap::new();
    values.insert("chapter".to_owned(), json!(3));
    runtime
        .inject_variables(&values, WorkflowVariableSource::ProjectAi)
        .unwrap();
    runtime.state.origin_conversation_id = Some("conv-persist".to_owned());

    let store = SqliteWorkflowRuntimeStore::open(project.path()).unwrap();
    store.create_state(&runtime.state).unwrap();

    let loaded = store
        .load_state(&WorkflowId::from("persist"), &RunId::from("run-persist"))
        .unwrap()
        .expect("运行态应当落库");

    // 四个字段一个都不能丢。
    assert_eq!(
        loaded.variables.get("chapter"),
        Some(&json!(3)),
        "变量取值必须落库，否则 Resume 后循环从第一章重来"
    );
    assert_eq!(
        loaded.variable_decls.len(),
        2,
        "变量声明必须落库，否则 Resume 后写回全被判成未声明变量"
    );
    assert_eq!(loaded.summary_template.as_deref(), Some("写第{{var.chapter}}章"));
    assert_eq!(
        loaded.origin_conversation_id.as_deref(),
        Some("conv-persist"),
        "发起对话 id 必须落库，否则终态回报没有投递目标"
    );

    // 恢复出来的 runtime 行为也要一致。
    let resumed = WorkflowRuntime::from_state(loaded);
    assert_eq!(resumed.render_variable_summary(), "写第3章");
}
