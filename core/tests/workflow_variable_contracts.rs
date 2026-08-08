//! Module 11 工作流变量契约。
//!
//! 单独成文件而不是并入 `workflow_contracts.rs`：变量是一条独立能力线
//! （声明 → 注入 → 循环写回 → 模板渲染），放在一起会让那个已超 5000 行的
//! 文件更难定位。
use std::collections::BTreeMap;

use ariadne::contracts::{
    render_summary_template, validate_variable_decls, variable_value_is_blank, Edge, EdgeId, NodeId,
    NodeInstance, PortEndpoint, PortValue, RunId, RunStatus, WorkflowDefinition, WorkflowEdgeKind,
    WorkflowId, WorkflowVariableDecl, WorkflowVariableKind, EXECUTION_INPUT_PORT,
    EXECUTION_OUTPUT_PORT,
};
use ariadne::rag::{render_prompt_template, PromptTemplateContext};
use ariadne::workflow::{
    BuiltinWorkflowNodeExecutor, WorkflowExternalNodeExecutor, WorkflowNodeExecutionOutput,
    WorkflowNodeExecutionRequest, WorkflowRuntime, WorkflowVariableSource,
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

/// required 变量必须带默认值，否则执行页无法预填。
#[test]
/// `required` 且无默认值是**合法**声明，也是它最常见的写法。
///
/// 曾按「required = 必须人工确认」的误解加过「required 必须带 default 以供
/// 预填」的校验。按正确语义（占位符不能替换成空白），正因为没有默认值才逼着
/// 每次填上 —— 那条规则把最正常的用法禁了。这个测试钉住它不再回来。
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
