use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ariadne::contracts::{
    CommunicationEdgeConfig, DocumentPatch, Edge, EdgeId, NodeId, NodeInstance, PatchHunk,
    PermissionPolicy, PortEndpoint, PortValue, RunControl, RunId, RunStatus, TextRange,
    WorkflowDefinition, WorkflowEdgeKind, WorkflowId, COMMUNICATION_PORT, EXECUTION_INPUT_PORT,
    EXECUTION_OUTPUT_PORT,
};
use ariadne::documents::{DocumentReadRequest, DocumentRepository, FileDocumentService};
use ariadne::git::GitService;
use ariadne::workflow::{
    apply_confirmed_patch, BuiltinWorkflowNodeExecutor, CommunicationControl,
    DocumentWorkflowExportSink, FilesystemRuntimeReferenceResolver, NodeErrorKind, NodeRetryPolicy,
    NoopExternalNodeExecutor, PatchWriteBackState, RoutedExternalNodeExecutor, RuntimeConfirmation,
    RuntimeConfirmationState, RuntimeReferenceKind, RuntimeReferenceResolver,
    SqliteWorkflowRuntimeStore, WorkflowExportRequest, WorkflowExportSink,
    WorkflowExternalNodeExecutor, WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest,
    WorkflowNodeExecutor, WorkflowRuntime, WorkflowRuntimeEventType, WorkflowRuntimeStore,
};
use serde_json::{json, Value};

/// 预设节点输出并记录调度请求的测试执行器。
#[derive(Default)]
struct ScriptedExecutor {
    outputs: BTreeMap<String, Vec<WorkflowNodeExecutionOutput>>,
    errors: BTreeMap<String, Vec<ariadne::contracts::CoreError>>,
    calls: Vec<WorkflowNodeExecutionRequest>,
}

impl ScriptedExecutor {
    /// 为指定节点追加一次预设输出。
    fn push(&mut self, node_id: &str, output: WorkflowNodeExecutionOutput) {
        self.outputs
            .entry(node_id.to_owned())
            .or_default()
            .push(output);
    }

    /// 为指定节点追加一次预设错误。
    fn push_error(&mut self, node_id: &str, error: ariadne::contracts::CoreError) {
        self.errors
            .entry(node_id.to_owned())
            .or_default()
            .push(error);
    }

    /// 统计指定节点被 runtime 调用的次数。
    fn call_count(&self, node_id: &str) -> usize {
        self.calls
            .iter()
            .filter(|call| call.node_id.as_str() == node_id)
            .count()
    }
}

impl WorkflowNodeExecutor for ScriptedExecutor {
    /// 按节点 id 返回预设输出，并记录执行请求。
    fn execute(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowNodeExecutionOutput> {
        self.calls.push(request.clone());
        if let Some(errors) = self.errors.get_mut(request.node_id.as_str()) {
            if !errors.is_empty() {
                return Err(errors.remove(0));
            }
        }
        let outputs = self
            .outputs
            .get_mut(request.node_id.as_str())
            .ok_or_else(|| ariadne::contracts::CoreError::validation("missing scripted output"))?;
        if outputs.is_empty() {
            return Err(ariadne::contracts::CoreError::validation(
                "scripted output exhausted",
            ));
        }
        Ok(outputs.remove(0))
    }
}

struct MissingReferenceResolver;

impl RuntimeReferenceResolver for MissingReferenceResolver {
    /// 测试恢复诊断时始终报告文档缺失。
    fn document_exists(&self, _document_id: &str) -> ariadne::contracts::CoreResult<bool> {
        Ok(false)
    }

    /// 测试恢复诊断时始终报告 chunk 缺失。
    fn chunk_exists(&self, _chunk_id: &str) -> ariadne::contracts::CoreResult<bool> {
        Ok(false)
    }

    /// 测试恢复诊断时始终报告 artifact 缺失。
    fn artifact_exists(&self, _artifact_id: &str) -> ariadne::contracts::CoreResult<bool> {
        Ok(false)
    }

    /// 测试恢复诊断时始终报告 patch session commit 缺失。
    fn patch_session_commit_exists(
        &self,
        _patch_session_commit_id: &str,
    ) -> ariadne::contracts::CoreResult<bool> {
        Ok(false)
    }

    /// 测试恢复诊断时始终报告 checkpoint 缺失。
    fn checkpoint_exists(&self, _checkpoint_id: &str) -> ariadne::contracts::CoreResult<bool> {
        Ok(false)
    }
}

/// 把 ScriptedExecutor 包装成外部节点执行器。
struct ScriptedExternalExecutor {
    scripted: ScriptedExecutor,
}

impl WorkflowExternalNodeExecutor for ScriptedExternalExecutor {
    /// 将外部节点请求转交给脚本执行器。
    fn execute_external(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowNodeExecutionOutput> {
        self.scripted.execute(request)
    }
}

/// 记录 Export 节点导出请求的测试 sink。
#[derive(Default)]
struct RecordingExportSink {
    exports: Vec<WorkflowExportRequest>,
}

impl WorkflowExportSink for RecordingExportSink {
    /// 记录导出请求并返回配置里的 artifact id。
    fn export_artifact(
        &mut self,
        _request: &WorkflowNodeExecutionRequest,
        export: WorkflowExportRequest,
    ) -> ariadne::contracts::CoreResult<String> {
        let artifact_id = export.artifact_id.clone();
        self.exports.push(export);
        Ok(artifact_id)
    }
}

/// 构造最小节点实例。
fn node(id: &str, type_name: &str) -> NodeInstance {
    NodeInstance {
        id: NodeId::from(id),
        type_name: type_name.to_owned(),
        label: None,
        config: Value::Null,
        position: None,
    }
}

/// 构造带单个 inline 输出的节点结果。
fn inline_output(port: &str, value: &str) -> WorkflowNodeExecutionOutput {
    let mut outputs = ariadne::contracts::PortMap::new();
    outputs.insert(port.to_owned(), PortValue::inline(json!(value)));
    WorkflowNodeExecutionOutput {
        outputs,
        ..WorkflowNodeExecutionOutput::default()
    }
}

/// 构造一次 communication 文本输出。
fn communication_output(value: &str) -> WorkflowNodeExecutionOutput {
    WorkflowNodeExecutionOutput {
        communication_output: Some(value.to_owned()),
        ..WorkflowNodeExecutionOutput::default()
    }
}

/// 构造声明 communication 结束的节点输出。
fn ending_communication_output(value: &str) -> WorkflowNodeExecutionOutput {
    WorkflowNodeExecutionOutput {
        communication_output: Some(value.to_owned()),
        communication_control: CommunicationControl {
            continue_communication: false,
            approved: false,
        },
        ..WorkflowNodeExecutionOutput::default()
    }
}

/// 构造包含待确认 patch 的节点输出。
fn confirmation_output(node_id: &str, confirmation_id: &str) -> WorkflowNodeExecutionOutput {
    WorkflowNodeExecutionOutput {
        patch_session_commit_id: Some("patch-1".to_owned()),
        confirmations: vec![RuntimeConfirmation {
            confirmation_id: confirmation_id.to_owned(),
            node_id: NodeId::from(node_id),
            state: RuntimeConfirmationState::Pending,
            artifact_id: Some("patches/patch-1.json".to_owned()),
            patch_session_commit_id: Some("patch-1".to_owned()),
            metadata: Value::Null,
        }],
        ..WorkflowNodeExecutionOutput::default()
    }
}

/// 构造测试用控制边。
fn control_edge(edge_id: &str, from: &str, to: &str) -> Edge {
    Edge {
        id: EdgeId::from(edge_id),
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

/// 构造包含缺失 artifact 和 patch 引用的节点输出。
fn artifact_patch_output(node_id: &str) -> WorkflowNodeExecutionOutput {
    let mut outputs = ariadne::contracts::PortMap::new();
    outputs.insert(
        "patch_artifact".to_owned(),
        PortValue::artifact_ref("missing-artifact"),
    );
    WorkflowNodeExecutionOutput {
        outputs,
        patch_session_commit_id: Some("missing-patch".to_owned()),
        confirmations: vec![RuntimeConfirmation {
            confirmation_id: "confirm-patch".to_owned(),
            node_id: NodeId::from(node_id),
            state: RuntimeConfirmationState::Pending,
            artifact_id: Some("missing-confirmation-artifact".to_owned()),
            patch_session_commit_id: Some("missing-patch".to_owned()),
            metadata: Value::Null,
        }],
        ..WorkflowNodeExecutionOutput::default()
    }
}

/// 构造双向 communication 边配置。
fn communication_config(max: u32) -> CommunicationEdgeConfig {
    CommunicationEdgeConfig {
        initiator_node_id: Some(NodeId::from("prudent")),
        forward_alias: "review".to_owned(),
        reverse_alias: "revision".to_owned(),
        forward_template: "请处理：{{input.review}}".to_owned(),
        reverse_template: "请复核：{{input.revision}}".to_owned(),
        max_communication_count: max,
    }
}

/// 验证 control 边按顺序调度节点，并把 data 边 alias 汇入输入。
#[test]
fn runtime_schedules_control_edges_and_passes_data_aliases() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Control".to_owned(),
        nodes: vec![node("planner", "planner"), node("writer", "writer")],
        edges: vec![
            Edge {
                id: EdgeId::from("data-1"),
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("planner"),
                    port_name: "outline".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: "prompt_input".to_owned(),
                },
                alias: Some("本章大纲".to_owned()),
                communication: None,
            },
            Edge {
                id: EdgeId::from("control-1"),
                kind: WorkflowEdgeKind::Control,
                from: PortEndpoint {
                    node_id: NodeId::from("planner"),
                    port_name: EXECUTION_OUTPUT_PORT.to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: EXECUTION_INPUT_PORT.to_owned(),
                },
                alias: None,
                communication: None,
            },
        ],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("planner", inline_output("outline", "去旧城"));
    executor.push("writer", WorkflowNodeExecutionOutput::default());

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(executor.calls[0].node_id.as_str(), "planner");
    assert_eq!(executor.calls[1].node_id.as_str(), "writer");
    assert!(executor.calls[1].inputs.contains_key("本章大纲"));
}

/// 验证 data 边缺失上游输出时不会静默跳过，而是记录节点失败事件。
#[test]
fn runtime_fails_target_when_data_edge_output_is_missing() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Missing data output".to_owned(),
        nodes: vec![node("planner", "planner"), node("writer", "writer")],
        edges: vec![
            Edge {
                id: EdgeId::from("data-1"),
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("planner"),
                    port_name: "outline".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: "prompt_input".to_owned(),
                },
                alias: Some("outline".to_owned()),
                communication: None,
            },
            control_edge("control-1", "planner", "writer"),
        ],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("planner", WorkflowNodeExecutionOutput::default());
    executor.push("writer", WorkflowNodeExecutionOutput::default());

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Failed);
    assert_eq!(executor.call_count("writer"), 0);
    let writer = runtime
        .state
        .nodes
        .get(&NodeId::from("writer"))
        .expect("writer state should be recorded");
    assert_eq!(writer.status, RunStatus::Failed);
    assert!(writer
        .error_state
        .as_ref()
        .is_some_and(|error| error.message.contains("has no output port outline")));
    assert!(runtime
        .events_for_node(&NodeId::from("writer"))
        .iter()
        .any(|event| event.event_type == WorkflowRuntimeEventType::NodeFailed));
}

/// 验证 communication 到达最大次数后暂停运行。
#[test]
fn runtime_pauses_when_communication_count_is_exhausted() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Communication".to_owned(),
        nodes: vec![node("prudent", "prudent"), node("polisher", "polisher")],
        edges: vec![Edge {
            id: EdgeId::from("comm-1"),
            kind: WorkflowEdgeKind::Communication,
            from: PortEndpoint {
                node_id: NodeId::from("prudent"),
                port_name: COMMUNICATION_PORT.to_owned(),
            },
            to: PortEndpoint {
                node_id: NodeId::from("polisher"),
                port_name: COMMUNICATION_PORT.to_owned(),
            },
            alias: None,
            communication: Some(communication_config(1)),
        }],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("prudent", communication_output("这里节奏太快"));
    executor.push("polisher", communication_output("已放慢节奏"));

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Paused);
    assert_eq!(
        runtime
            .state
            .communication_edges
            .get(&EdgeId::from("comm-1"))
            .unwrap()
            .message_count,
        1
    );
}

/// 验证节点声明结束时 communication 边会完成并记录末条消息哈希。
#[test]
fn runtime_stops_communication_when_node_declares_end() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Communication".to_owned(),
        nodes: vec![node("prudent", "prudent"), node("polisher", "polisher")],
        edges: vec![Edge {
            id: EdgeId::from("comm-1"),
            kind: WorkflowEdgeKind::Communication,
            from: PortEndpoint {
                node_id: NodeId::from("prudent"),
                port_name: COMMUNICATION_PORT.to_owned(),
            },
            to: PortEndpoint {
                node_id: NodeId::from("polisher"),
                port_name: COMMUNICATION_PORT.to_owned(),
            },
            alias: None,
            communication: Some(communication_config(3)),
        }],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("prudent", communication_output("这里节奏太快"));
    executor.push("polisher", ending_communication_output("已完成"));

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert!(
        runtime
            .state
            .communication_edges
            .get(&EdgeId::from("comm-1"))
            .unwrap()
            .completed
    );
    let communication = runtime
        .state
        .communication_edges
        .get(&EdgeId::from("comm-1"))
        .unwrap();
    assert_eq!(
        communication.completed_reason.as_deref(),
        Some("node_declared_complete")
    );
    assert!(communication.last_message_hash.is_some());
}

/// 验证待确认项会暂停运行，审批后再次 run 不会重复执行已完成节点。
#[test]
fn runtime_pauses_for_pending_confirmation_and_resumes_idempotently() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Confirmation".to_owned(),
        nodes: vec![node("writer", "writer"), node("summarizer", "summarizer")],
        edges: vec![Edge {
            id: EdgeId::from("control-1"),
            kind: WorkflowEdgeKind::Control,
            from: PortEndpoint {
                node_id: NodeId::from("writer"),
                port_name: EXECUTION_OUTPUT_PORT.to_owned(),
            },
            to: PortEndpoint {
                node_id: NodeId::from("summarizer"),
                port_name: EXECUTION_INPUT_PORT.to_owned(),
            },
            alias: None,
            communication: None,
        }],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("writer", confirmation_output("writer", "confirm-writer"));
    executor.push("summarizer", WorkflowNodeExecutionOutput::default());

    let first_status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(first_status, RunStatus::Paused);
    assert_eq!(executor.call_count("writer"), 1);

    runtime
        .update_confirmation_state("confirm-writer", RuntimeConfirmationState::Approved)
        .unwrap();
    let second_status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(second_status, RunStatus::Succeeded);
    assert_eq!(executor.call_count("writer"), 1);
    assert_eq!(executor.call_count("summarizer"), 1);
}

/// 验证节点断点会在执行前暂停一次，Resume 后继续执行该节点。
#[test]
fn runtime_pauses_once_before_breakpoint_node() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-breakpoint"),
        name: "Breakpoint".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("writer"),
            type_name: "writer".to_owned(),
            label: None,
            config: json!({ "breakpoint": true }),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-breakpoint")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("writer", WorkflowNodeExecutionOutput::default());

    let first = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(first, RunStatus::Paused);
    assert_eq!(executor.call_count("writer"), 0);
    assert!(runtime
        .state
        .pause_reason
        .as_deref()
        .unwrap()
        .contains("breakpoint before node writer"));

    runtime.resume().unwrap();
    let second = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(second, RunStatus::Succeeded);
    assert_eq!(executor.call_count("writer"), 1);
}

/// 验证断点暂停后可以跳过该节点，并继续执行控制流下游节点。
#[test]
fn runtime_can_skip_breakpoint_node_and_continue_downstream() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-skip-breakpoint"),
        name: "Skip Breakpoint".to_owned(),
        nodes: vec![
            NodeInstance {
                id: NodeId::from("writer"),
                type_name: "writer".to_owned(),
                label: None,
                config: json!({ "breakpoint": true }),
                position: None,
            },
            NodeInstance {
                id: NodeId::from("summarizer"),
                type_name: "summarizer".to_owned(),
                label: None,
                config: Value::Null,
                position: None,
            },
        ],
        edges: vec![control_edge("writer-summarizer", "writer", "summarizer")],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-skip-breakpoint")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("summarizer", WorkflowNodeExecutionOutput::default());

    let first = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(first, RunStatus::Paused);
    runtime
        .skip_node(&workflow, &NodeId::from("writer"))
        .unwrap();
    let second = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(second, RunStatus::Succeeded);
    assert_eq!(executor.call_count("writer"), 0);
    assert_eq!(executor.call_count("summarizer"), 1);
    assert!(runtime
        .state
        .nodes
        .get(&NodeId::from("writer"))
        .unwrap()
        .metadata
        .get("skipped")
        .and_then(Value::as_bool)
        .unwrap());
    assert!(runtime
        .state
        .structured_events
        .iter()
        .any(|event| event.event_type == WorkflowRuntimeEventType::NodeSkipped));
}

/// 验证 patch 写回状态只能在确认后进入 Applied，且 Applied 不可回退。
#[test]
fn runtime_updates_patch_write_back_state_idempotently() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Patch".to_owned(),
        nodes: vec![node("writer", "writer")],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("writer", confirmation_output("writer", "confirm-writer"));

    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Paused);

    assert!(runtime
        .mark_patch_write_back_state(&NodeId::from("writer"), PatchWriteBackState::Applied)
        .is_err());
    runtime
        .update_confirmation_state("confirm-writer", RuntimeConfirmationState::Approved)
        .unwrap();
    runtime
        .mark_patch_write_back_state(&NodeId::from("writer"), PatchWriteBackState::Applied)
        .unwrap();
    runtime
        .mark_patch_write_back_state(&NodeId::from("writer"), PatchWriteBackState::Applied)
        .unwrap();

    let node = runtime.state.nodes.get(&NodeId::from("writer")).unwrap();
    assert_eq!(
        node.patch_write_back_state,
        Some(PatchWriteBackState::Applied)
    );
    assert!(runtime
        .mark_patch_write_back_state(&NodeId::from("writer"), PatchWriteBackState::Failed)
        .is_err());
}

/// 验证恢复诊断能报告 runtime 快照里的缺失引用。
#[test]
fn runtime_reference_validation_reports_missing_runtime_references() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Recovery".to_owned(),
        nodes: vec![node("writer", "writer")],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("writer", artifact_patch_output("writer"));

    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Paused);

    let report = runtime
        .validate_references(&MissingReferenceResolver)
        .unwrap();

    assert!(!report.is_clean());
    assert!(report.checked_reference_count >= 3);
    assert!(report
        .missing_references
        .iter()
        .any(|item| item.kind == RuntimeReferenceKind::Artifact && item.id == "missing-artifact"));
    assert!(report.missing_references.iter().any(|item| {
        item.kind == RuntimeReferenceKind::PatchSessionCommit && item.id == "missing-patch"
    }));
}

/// 验证 runtime.db 能保存并加载完整运行快照。
#[test]
fn runtime_persists_and_loads_run_state() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Persisted".to_owned(),
        nodes: vec![node("writer", "writer")],
        edges: vec![],
        metadata: Value::Null,
    };
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("writer", inline_output("draft", "正文"));

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();
    assert_eq!(status, RunStatus::Succeeded);

    let loaded = store
        .load_state(&WorkflowId::from("wf"), &RunId::from("run-1"))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.status, RunStatus::Succeeded);
    assert!(loaded
        .nodes
        .get(&NodeId::from("writer"))
        .unwrap()
        .outputs
        .contains_key("draft"));
}

/// 验证 Stop 控制会保留当前节点输出并跳过下游节点。
#[test]
fn runtime_stop_keeps_completed_nodes_and_skips_downstream() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Stop".to_owned(),
        nodes: vec![node("writer", "writer"), node("export", "export")],
        edges: vec![Edge {
            id: EdgeId::from("control-1"),
            kind: WorkflowEdgeKind::Control,
            from: PortEndpoint {
                node_id: NodeId::from("writer"),
                port_name: EXECUTION_OUTPUT_PORT.to_owned(),
            },
            to: PortEndpoint {
                node_id: NodeId::from("export"),
                port_name: EXECUTION_INPUT_PORT.to_owned(),
            },
            alias: None,
            communication: None,
        }],
        metadata: Value::Null,
    };
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push(
        "writer",
        WorkflowNodeExecutionOutput {
            run_control: Some(RunControl::Stop),
            ..inline_output("draft", "正文")
        },
    );
    executor.push("export", WorkflowNodeExecutionOutput::default());

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    assert_eq!(status, RunStatus::Stopped);
    assert_eq!(executor.call_count("writer"), 1);
    assert_eq!(executor.call_count("export"), 0);
    assert!(runtime.state.nodes.contains_key(&NodeId::from("writer")));
    assert!(runtime.state.stop_reason.is_some());
}

/// 验证节点请求包含 type_name、config，并在 Resume 时携带上次 metadata。
#[test]
fn runtime_request_includes_node_type_config_and_previous_metadata() {
    let mut writer = node("writer", "llm");
    writer.config = json!({ "model": "mock" });
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Request".to_owned(),
        nodes: vec![writer],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push(
        "writer",
        WorkflowNodeExecutionOutput {
            run_control: Some(RunControl::Pause),
            metadata: json!({ "round": 1 }),
            ..WorkflowNodeExecutionOutput::default()
        },
    );
    executor.push("writer", WorkflowNodeExecutionOutput::default());

    let first_status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(first_status, RunStatus::Paused);
    runtime.resume().unwrap();
    let second_status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(second_status, RunStatus::Succeeded);

    assert_eq!(executor.calls[0].type_name, "llm");
    assert_eq!(executor.calls[0].config, json!({ "model": "mock" }));
    assert_eq!(executor.calls[1].metadata, json!({ "round": 1 }));
}

/// 验证网络/rate limit/timeout 类错误会按指数退避重试，并最终清除错误状态。
#[test]
fn runtime_retries_retryable_external_errors_with_backoff_events() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Retry".to_owned(),
        nodes: vec![node("llm", "llm")],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    runtime
        .set_retry_policy(NodeRetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 1_000,
            backoff_multiplier: 2,
        })
        .unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push_error(
        "llm",
        ariadne::contracts::CoreError::External {
            service: "mock-provider".to_owned(),
            message: "timeout while calling provider".to_owned(),
        },
    );
    executor.push_error(
        "llm",
        ariadne::contracts::CoreError::External {
            service: "mock-provider".to_owned(),
            message: "HTTP 429 rate limit".to_owned(),
        },
    );
    executor.push("llm", inline_output("draft", "ok"));

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Queued);
    assert_eq!(executor.call_count("llm"), 1);
    let retry_events = runtime
        .events_for_node(&NodeId::from("llm"))
        .into_iter()
        .filter(|event| event.event_type == WorkflowRuntimeEventType::NodeRetryScheduled)
        .collect::<Vec<_>>();
    assert_eq!(retry_events.len(), 1);
    assert_eq!(retry_events[0].metadata["next_retry_delay_ms"], 1_000);
    assert!(retry_events[0].metadata["next_retry_at_ms"].is_number());

    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Queued);
    assert_eq!(executor.call_count("llm"), 1);

    runtime
        .state
        .nodes
        .get_mut(&NodeId::from("llm"))
        .unwrap()
        .error_state
        .as_mut()
        .unwrap()
        .next_retry_at_ms = Some(0);
    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Queued);
    assert_eq!(executor.call_count("llm"), 2);
    let retry_events = runtime
        .events_for_node(&NodeId::from("llm"))
        .into_iter()
        .filter(|event| event.event_type == WorkflowRuntimeEventType::NodeRetryScheduled)
        .collect::<Vec<_>>();
    assert_eq!(retry_events.len(), 2);
    assert_eq!(retry_events[1].metadata["next_retry_delay_ms"], 2_000);

    runtime
        .state
        .nodes
        .get_mut(&NodeId::from("llm"))
        .unwrap()
        .error_state
        .as_mut()
        .unwrap()
        .next_retry_at_ms = Some(0);
    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(executor.call_count("llm"), 3);
    let node = runtime.state.nodes.get(&NodeId::from("llm")).unwrap();
    assert!(node.error_state.is_none());
    assert_eq!(node.execution_attempts, 3);
}

/// 验证工具参数/JSON schema 错误最多重试 3 次，耗尽后 Pause 并序列化错误状态。
#[test]
fn runtime_pauses_after_tool_argument_errors_exhaust_retries() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "ToolArgs".to_owned(),
        nodes: vec![node("writer", "writer")],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    runtime
        .set_retry_policy(NodeRetryPolicy::default())
        .unwrap();
    let mut executor = ScriptedExecutor::default();
    for _ in 0..3 {
        executor.push_error(
            "writer",
            ariadne::contracts::CoreError::validation("tool argument schema mismatch"),
        );
    }

    let status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(status, RunStatus::Queued);
    assert_eq!(executor.call_count("writer"), 1);
    for _ in 0..2 {
        runtime
            .state
            .nodes
            .get_mut(&NodeId::from("writer"))
            .unwrap()
            .error_state
            .as_mut()
            .unwrap()
            .next_retry_at_ms = Some(0);
        runtime.run(&workflow, &mut executor).unwrap();
    }

    assert_eq!(runtime.state.status, RunStatus::Paused);
    assert_eq!(executor.call_count("writer"), 3);
    let node = runtime.state.nodes.get(&NodeId::from("writer")).unwrap();
    let error_state = node.error_state.as_ref().unwrap();
    assert_eq!(error_state.kind, NodeErrorKind::ToolArguments);
    assert_eq!(error_state.attempts, 3);
    assert_eq!(error_state.max_attempts, 3);
    assert!(error_state.retryable);
    assert_eq!(error_state.next_retry_delay_ms, None);
    assert!(error_state.recovery_suggestion.contains("重试次数已耗尽"));

    let serialized = serde_json::to_value(&runtime.state).unwrap();
    assert_eq!(
        serialized["nodes"]["writer"]["error_state"]["kind"],
        "tool_arguments"
    );
}

/// 验证权限错误不会自动重试，会直接进入失败状态并给出恢复建议。
#[test]
fn runtime_does_not_retry_permission_errors() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Permission".to_owned(),
        nodes: vec![node("http", "http")],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push_error(
        "http",
        ariadne::contracts::CoreError::PermissionDenied {
            action: "http".to_owned(),
            reason: "host not allowed".to_owned(),
        },
    );

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Failed);
    assert_eq!(executor.call_count("http"), 1);
    let error_state = runtime
        .state
        .nodes
        .get(&NodeId::from("http"))
        .unwrap()
        .error_state
        .as_ref()
        .unwrap();
    assert_eq!(error_state.kind, NodeErrorKind::Permission);
    assert!(!error_state.retryable);
    assert!(error_state.recovery_suggestion.contains("权限"));
}

/// 验证内建 condition 节点输出 passed/reason/branch。
#[test]
fn builtin_condition_node_outputs_branch_values() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Condition".to_owned(),
        nodes: vec![
            node("source", "source"),
            NodeInstance {
                id: NodeId::from("condition"),
                type_name: "condition".to_owned(),
                label: None,
                config: json!({
                    "input_alias": "flag",
                    "operator": "equals",
                    "expected": true,
                }),
                position: None,
            },
        ],
        edges: vec![
            Edge {
                id: EdgeId::from("data-1"),
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("source"),
                    port_name: "flag".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("condition"),
                    port_name: "input".to_owned(),
                },
                alias: Some("flag".to_owned()),
                communication: None,
            },
            Edge {
                id: EdgeId::from("control-1"),
                kind: WorkflowEdgeKind::Control,
                from: PortEndpoint {
                    node_id: NodeId::from("source"),
                    port_name: EXECUTION_OUTPUT_PORT.to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("condition"),
                    port_name: EXECUTION_INPUT_PORT.to_owned(),
                },
                alias: None,
                communication: None,
            },
        ],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut external = ScriptedExternalExecutor {
        scripted: ScriptedExecutor::default(),
    };
    let mut source_output = ariadne::contracts::PortMap::new();
    source_output.insert("flag".to_owned(), PortValue::inline(true));
    external.scripted.outputs.insert(
        "source".to_owned(),
        vec![WorkflowNodeExecutionOutput {
            outputs: source_output,
            ..WorkflowNodeExecutionOutput::default()
        }],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert!(runtime
        .state
        .nodes
        .get(&NodeId::from("condition"))
        .unwrap()
        .outputs
        .contains_key("passed"));
}

/// 验证 approval 节点暂停运行，并在确认后回写审批输出。
#[test]
fn builtin_approval_node_pauses_and_resume_publishes_result() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Approval".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("approval"),
            type_name: "approval".to_owned(),
            label: None,
            config: json!({
                "approval_id": "approve-1",
                "prompt_id": "default",
                "auto_approve": false,
            }),
            position: None,
        }],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut external = NoopExternalNodeExecutor::default();
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let first_status = runtime.run(&workflow, &mut executor).unwrap();
    assert_eq!(first_status, RunStatus::Paused);
    runtime
        .update_confirmation_state("approve-1", RuntimeConfirmationState::Approved)
        .unwrap();

    let approval_node = runtime.state.nodes.get(&NodeId::from("approval")).unwrap();
    assert!(matches!(
        approval_node.outputs.get("approved"),
        Some(PortValue::Inline { value }) if value == &json!(true)
    ));
}

/// 验证 loop 节点按显式目标重跑，直到停止条件满足。
#[test]
fn builtin_loop_node_reruns_explicit_target_until_condition_passes() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Loop".to_owned(),
        nodes: vec![
            node("writer", "writer"),
            NodeInstance {
                id: NodeId::from("loop"),
                type_name: "loop".to_owned(),
                label: None,
                config: json!({
                    "max_iterations": 2,
                    "timeout_ms": 3000,
                    "stop_condition": {
                        "input_alias": "approved",
                        "equals": true
                    },
                    "rerun_node_ids": ["writer"]
                }),
                position: None,
            },
        ],
        edges: vec![
            Edge {
                id: EdgeId::from("data-1"),
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: "approved".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("loop"),
                    port_name: "condition".to_owned(),
                },
                alias: Some("approved".to_owned()),
                communication: None,
            },
            Edge {
                id: EdgeId::from("control-1"),
                kind: WorkflowEdgeKind::Control,
                from: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: EXECUTION_OUTPUT_PORT.to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("loop"),
                    port_name: EXECUTION_INPUT_PORT.to_owned(),
                },
                alias: None,
                communication: None,
            },
            Edge {
                id: EdgeId::from("control-2"),
                kind: WorkflowEdgeKind::Control,
                from: PortEndpoint {
                    node_id: NodeId::from("loop"),
                    port_name: EXECUTION_OUTPUT_PORT.to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: EXECUTION_INPUT_PORT.to_owned(),
                },
                alias: None,
                communication: None,
            },
        ],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut external = ScriptedExternalExecutor {
        scripted: ScriptedExecutor::default(),
    };
    external
        .scripted
        .push("writer", inline_output("approved", "not-used"));
    let mut first_outputs = ariadne::contracts::PortMap::new();
    first_outputs.insert("approved".to_owned(), PortValue::inline(false));
    let mut second_outputs = ariadne::contracts::PortMap::new();
    second_outputs.insert("approved".to_owned(), PortValue::inline(true));
    external.scripted.outputs.insert(
        "writer".to_owned(),
        vec![
            WorkflowNodeExecutionOutput {
                outputs: first_outputs,
                ..WorkflowNodeExecutionOutput::default()
            },
            WorkflowNodeExecutionOutput {
                outputs: second_outputs,
                ..WorkflowNodeExecutionOutput::default()
            },
        ],
    );
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(external.scripted.call_count("writer"), 2);
    assert_eq!(
        runtime.state.loop_iterations.get(&NodeId::from("loop")),
        Some(&1)
    );
}

/// 验证 export 节点调用导出 sink 并返回 artifact_ref。
#[test]
fn builtin_export_node_returns_artifact_reference() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Export".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("export"),
            type_name: "export".to_owned(),
            label: None,
            config: json!({
                "artifact_id": "exports/book.md",
                "format": "markdown",
                "title": "Book",
            }),
            position: None,
        }],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut external = NoopExternalNodeExecutor::default();
    let mut sink = RecordingExportSink::default();
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external).with_export_sink(&mut sink);

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(sink.exports.len(), 1);
    assert!(matches!(
        runtime
            .state
            .nodes
            .get(&NodeId::from("export"))
            .unwrap()
            .outputs
            .get("artifact"),
        Some(PortValue::ArtifactRef { artifact_id }) if artifact_id == "exports/book.md"
    ));
}

/// 创建允许在临时目录内读写的文档服务。
fn test_document_service(root: &Path) -> FileDocumentService {
    let artifact_root = root.join(".runtime").join("artifacts");
    let policy = PermissionPolicy {
        readable_file_roots: vec![root.to_path_buf()],
        writable_file_roots: vec![root.to_path_buf()],
        ..PermissionPolicy::default()
    };
    FileDocumentService::new(policy, artifact_root)
}

/// 初始化测试 Git 仓库，并配置本地提交身份。
fn init_test_git(root: &Path) -> GitService {
    let service = GitService::new(root);
    service.init_repository().unwrap();
    run_git(root, ["config", "user.name", "Ariadne Test"]);
    run_git(root, ["config", "user.email", "ariadne@example.test"]);
    service
}

/// 执行测试用 Git 命令，失败时输出 stderr。
fn run_git<const N: usize>(repo: &Path, args: [&str; N]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 验证文件系统引用解析器能检查文档、artifact 和已登记运行引用。
#[test]
fn filesystem_reference_resolver_checks_documents_artifacts_and_registered_ids() {
    let temp_dir = tempfile::tempdir().unwrap();
    let document_path = temp_dir.path().join("chapter.md");
    let artifact_root = temp_dir.path().join(".runtime").join("artifacts");
    let artifact_path = artifact_root.join("exports").join("book.md");
    fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
    fs::write(&document_path, "正文").unwrap();
    fs::write(&artifact_path, "export").unwrap();

    let resolver = FilesystemRuntimeReferenceResolver::new(&artifact_root)
        .with_checkpoint("checkpoint-1")
        .with_patch_commit("patch-1");

    assert!(resolver
        .document_exists(document_path.to_str().unwrap())
        .unwrap());
    assert!(resolver.artifact_exists("exports/book.md").unwrap());
    assert!(resolver.patch_session_commit_exists("patch-1").unwrap());
    assert!(resolver.checkpoint_exists("checkpoint-1").unwrap());
    assert!(!resolver.chunk_exists("chunk-1").unwrap());
}

/// 验证 Documents export sink 会把导出请求写入 artifact 根目录。
#[test]
fn document_export_sink_writes_export_artifact() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_document_service(temp_dir.path());
    let mut sink = DocumentWorkflowExportSink::new(&service);
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf"),
        run_id: RunId::from("run"),
        node_id: NodeId::from("export"),
        type_name: "export".to_owned(),
        config: Value::Null,
        inputs: ariadne::contracts::PortMap::new(),
        communication_messages: Vec::new(),
        metadata: Value::Null,
    };

    let artifact_id = sink
        .export_artifact(
            &request,
            WorkflowExportRequest {
                artifact_id: "exports/book.md".to_owned(),
                format: "markdown".to_owned(),
                title: Some("Book".to_owned()),
                inputs: ariadne::contracts::PortMap::new(),
            },
        )
        .unwrap();

    assert_eq!(artifact_id, "exports/book.md");
    assert!(temp_dir
        .path()
        .join(".runtime")
        .join("artifacts")
        .join("exports")
        .join("book.md")
        .exists());
}

/// 验证已审批 patch 能写回正文、创建 checkpoint，并同步 runtime 状态。
#[test]
fn apply_confirmed_patch_writes_document_and_records_checkpoint() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_document_service(temp_dir.path());
    let git = init_test_git(temp_dir.path());
    let path = temp_dir.path().join("chapter.txt");
    fs::write(&path, "alpha beta gamma").unwrap();
    git.create_archive_point("base", None).unwrap();
    let document = service
        .open_document(DocumentReadRequest {
            path: path.clone(),
            format: None,
        })
        .unwrap();
    let beta_start = document.content.find("beta").unwrap() as u64;
    let patch = DocumentPatch {
        document_id: document.metadata.document_id.clone(),
        base_version: Some(document.metadata.version.clone()),
        hunks: vec![PatchHunk {
            range: TextRange {
                start: beta_start,
                end: beta_start + 4,
            },
            replacement: "delta".to_owned(),
        }],
    };
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Patch".to_owned(),
        nodes: vec![node("writer", "writer")],
        edges: vec![],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("writer", confirmation_output("writer", "confirm-writer"));
    assert_eq!(
        runtime.run(&workflow, &mut executor).unwrap(),
        RunStatus::Paused
    );
    runtime
        .update_confirmation_state("confirm-writer", RuntimeConfirmationState::Approved)
        .unwrap();

    let outcome = apply_confirmed_patch(
        &mut runtime,
        &service,
        Some(&git),
        &NodeId::from("writer"),
        &patch,
        Some("apply writer patch"),
    )
    .unwrap();

    let node_state = runtime.state.nodes.get(&NodeId::from("writer")).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "alpha delta gamma");
    assert_eq!(
        node_state.patch_write_back_state,
        Some(PatchWriteBackState::Applied)
    );
    assert!(node_state.checkpoint_id.is_some());
    assert_eq!(outcome.report.index_invalidation.reason, "patch_applied");
    assert!(runtime
        .ensure_patch_write_back_can_start(&NodeId::from("writer"))
        .is_err());
}

/// 验证外部节点路由器按 type_name 分发，并拒绝重复注册。
#[test]
fn routed_external_executor_dispatches_registered_handlers() {
    let mut executor = RoutedExternalNodeExecutor::new();
    executor
        .register_handler(
            "custom",
            Box::new(|request| {
                let mut outputs = ariadne::contracts::PortMap::new();
                outputs.insert(
                    "seen".to_owned(),
                    PortValue::inline(request.node_id.as_str()),
                );
                Ok(WorkflowNodeExecutionOutput {
                    outputs,
                    ..WorkflowNodeExecutionOutput::default()
                })
            }),
        )
        .unwrap();
    assert!(executor
        .register_handler(
            "custom",
            Box::new(|_| Ok(WorkflowNodeExecutionOutput::default())),
        )
        .is_err());

    let output = executor
        .execute_external(WorkflowNodeExecutionRequest {
            workflow_id: WorkflowId::from("wf"),
            run_id: RunId::from("run"),
            node_id: NodeId::from("node-1"),
            type_name: "custom".to_owned(),
            config: Value::Null,
            inputs: ariadne::contracts::PortMap::new(),
            communication_messages: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();

    assert!(matches!(
        output.outputs.get("seen"),
        Some(PortValue::Inline { value }) if value == &json!("node-1")
    ));
}

// ─── Prudent 拒绝处置 A/B 测试 ───────────────────────────────────────────

#[cfg(test)]
mod prudent_rejection_contracts {
    use ariadne::contracts::{NodeId, PortMap, PortValue, RunId, WorkflowId};
    use ariadne::workflow::{
        BuiltinWorkflowNodeExecutor, NoopExternalNodeExecutor, RuntimeConfirmationState,
        WorkflowRuntime,
    };
    use serde_json::json;

    use super::*;

    fn make_runtime_with_prudent_confirmation() -> (WorkflowRuntime, WorkflowDefinition) {
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("wf-prudent"),
            name: "Prudent Test".to_owned(),
            nodes: vec![
                NodeInstance {
                    id: NodeId::from("writer"),
                    type_name: "writer".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
                NodeInstance {
                    id: NodeId::from("prudent"),
                    type_name: "prudent".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
                NodeInstance {
                    id: NodeId::from("summarizer"),
                    type_name: "summarizer".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
            ],
            edges: vec![
                Edge {
                    id: EdgeId::from("e1"),
                    kind: WorkflowEdgeKind::Control,
                    from: PortEndpoint {
                        node_id: NodeId::from("writer"),
                        port_name: EXECUTION_OUTPUT_PORT.to_owned(),
                    },
                    to: PortEndpoint {
                        node_id: NodeId::from("prudent"),
                        port_name: EXECUTION_INPUT_PORT.to_owned(),
                    },
                    alias: None,
                    communication: None,
                },
                Edge {
                    id: EdgeId::from("e2"),
                    kind: WorkflowEdgeKind::Control,
                    from: PortEndpoint {
                        node_id: NodeId::from("prudent"),
                        port_name: EXECUTION_OUTPUT_PORT.to_owned(),
                    },
                    to: PortEndpoint {
                        node_id: NodeId::from("summarizer"),
                        port_name: EXECUTION_INPUT_PORT.to_owned(),
                    },
                    alias: None,
                    communication: None,
                },
            ],
            metadata: Value::Null,
        };

        let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();

        // 模拟 prudent 节点已成功，生成一个被拒的确认项
        use ariadne::contracts::RunStatus;
        use ariadne::workflow::{RuntimeConfirmation, WorkflowNodeRuntimeState, WorkflowRunState};
        runtime.state.nodes.insert(
            NodeId::from("writer"),
            WorkflowNodeRuntimeState {
                node_id: NodeId::from("writer"),
                status: RunStatus::Succeeded,
                outputs: {
                    let mut m = PortMap::new();
                    m.insert("text".to_owned(), PortValue::inline("原始正文".to_owned()));
                    m
                },
                communication_output: None,
                communication_control: Default::default(),
                prompt_trace_hash: None,
                patch_session_commit_id: None,
                checkpoint_id: None,
                patch_write_back_state: None,
                metadata: Value::Null,
                error: None,
                error_state: None,
                execution_attempts: 1,
            },
        );
        runtime.state.nodes.insert(
            NodeId::from("prudent"),
            WorkflowNodeRuntimeState {
                node_id: NodeId::from("prudent"),
                status: RunStatus::Succeeded,
                outputs: {
                    let mut m = PortMap::new();
                    m.insert(
                        "revision_context".to_owned(),
                        PortValue::inline("被拒原因".to_owned()),
                    );
                    m
                },
                communication_output: None,
                communication_control: Default::default(),
                prompt_trace_hash: None,
                patch_session_commit_id: None,
                checkpoint_id: None,
                patch_write_back_state: None,
                metadata: Value::Null,
                error: None,
                error_state: None,
                execution_attempts: 1,
            },
        );
        runtime.state.confirmations.insert(
            "prudent-review".to_owned(),
            RuntimeConfirmation {
                confirmation_id: "prudent-review".to_owned(),
                node_id: NodeId::from("prudent"),
                state: RuntimeConfirmationState::Rejected,
                artifact_id: None,
                patch_session_commit_id: None,
                metadata: json!({}),
            },
        );
        runtime.state.status = ariadne::contracts::RunStatus::Paused;
        runtime.state.control = ariadne::contracts::RunControl::Pause;
        runtime.state.pause_reason = Some("prudent rejected".to_owned());

        (runtime, workflow)
    }

    #[test]
    fn path_b_override_confirmation_output_approves_and_resumes() {
        let (mut runtime, _workflow) = make_runtime_with_prudent_confirmation();

        let mut new_outputs = PortMap::new();
        new_outputs.insert(
            "revision_context".to_owned(),
            PortValue::inline("交流后同意的返修目标".to_owned()),
        );

        runtime
            .override_confirmation_output("prudent-review", new_outputs)
            .unwrap();

        // 确认项已置为通过
        let item = runtime.state.confirmations.get("prudent-review").unwrap();
        assert_eq!(item.state, RuntimeConfirmationState::Approved);

        // 节点输出被改写
        let node = runtime.state.nodes.get(&NodeId::from("prudent")).unwrap();
        let ctx = node.outputs.get("revision_context").unwrap();
        assert!(
            matches!(ctx, PortValue::Inline { value } if value.as_str() == Some("交流后同意的返修目标"))
        );

        // 暂停解除（无其他 pending 确认项）
        assert_eq!(runtime.state.status, ariadne::contracts::RunStatus::Queued);
    }

    #[test]
    fn path_a_resume_from_node_injects_and_clears_downstream() {
        let (mut runtime, workflow) = make_runtime_with_prudent_confirmation();

        // 模拟 summarizer 节点已有旧快照
        use ariadne::contracts::RunStatus;
        use ariadne::workflow::WorkflowNodeRuntimeState;
        runtime.state.nodes.insert(
            NodeId::from("summarizer"),
            WorkflowNodeRuntimeState {
                node_id: NodeId::from("summarizer"),
                status: RunStatus::Succeeded,
                outputs: PortMap::new(),
                communication_output: None,
                communication_control: Default::default(),
                prompt_trace_hash: None,
                patch_session_commit_id: None,
                checkpoint_id: None,
                patch_write_back_state: None,
                metadata: Value::Null,
                error: None,
                error_state: None,
                execution_attempts: 1,
            },
        );

        let mut injected = PortMap::new();
        injected.insert(
            "chapter_text".to_owned(),
            PortValue::inline("人工修改后的正文".to_owned()),
        );

        runtime
            .resume_from_node(&workflow, &NodeId::from("writer"), injected)
            .unwrap();

        // writer 节点输出被注入
        let writer = runtime.state.nodes.get(&NodeId::from("writer")).unwrap();
        assert!(matches!(
            writer.outputs.get("chapter_text"),
            Some(PortValue::Inline { value }) if value.as_str() == Some("人工修改后的正文")
        ));

        // prudent 和 summarizer 下游快照被清除
        assert!(!runtime.state.nodes.contains_key(&NodeId::from("prudent")));
        assert!(!runtime
            .state
            .nodes
            .contains_key(&NodeId::from("summarizer")));

        // 暂停解除
        assert_eq!(runtime.state.status, ariadne::contracts::RunStatus::Queued);
    }
}
