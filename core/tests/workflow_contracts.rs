use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use ariadne::config::{ModelConfig, ProviderConfig};
use ariadne::contracts::{
    CommunicationEdgeConfig, DocumentPatch, Edge, EdgeId, ExternalDispatchAuthorization, NodeId,
    NodeInstance, PatchHunk, PermissionPolicy, PortEndpoint, PortMap, PortValue,
    ProviderCapability, ProviderDefinition, ProviderType, RunControl, RunId, RunStatus, TextRange,
    WorkflowDefinition, WorkflowEdgeKind, WorkflowId, COMMUNICATION_PORT, EXECUTION_INPUT_PORT,
    EXECUTION_OUTPUT_PORT,
};
use ariadne::costs::{CostLedger, CostQuery, SqliteCostLedger, TokenUsage};
use ariadne::documents::{DocumentReadRequest, DocumentRepository, FileDocumentService};
use ariadne::git::GitService;
use ariadne::providers::{
    LlmMessage, LlmProvider, LlmRequest, LlmResponse, OpenAiCompatibleLlmProvider, Provider,
    ProviderCallContext, ProviderHealth,
};
use ariadne::rag::{
    MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, SummarizerStageOperationStatus,
};
use ariadne::retrieval::{
    FullTextRecord, FullTextStore, HybridSearchEngine, MemoryFullTextStore, MemoryVectorStore,
};
use ariadne::workflow::{
    apply_confirmed_patch, execute_llm_node, execute_llm_node_with_defaults,
    execute_project_search_node_for_test_fixture, execute_summarizer_node,
    validate_workflow_execution_contracts, BuiltinWorkflowNodeExecutor, CommunicationControl,
    DocumentWorkflowExportSink, FilesystemRuntimeReferenceResolver, NewWorkflowOperation,
    NodeErrorKind, NodeRetryPolicy, NoopExternalNodeExecutor, PatchWriteBackState,
    RoutedExternalNodeExecutor, RuntimeConfirmation, RuntimeConfirmationState,
    RuntimeReferenceKind, RuntimeReferenceResolver, SqliteWorkflowRuntimeStore,
    WorkflowExportRequest, WorkflowExportSink, WorkflowExternalNodeExecutor,
    WorkflowMutationClaimResult, WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest,
    WorkflowNodeExecutor, WorkflowOperationPolicy, WorkflowOperationRecoveryPolicy,
    WorkflowOperationResponsePolicy, WorkflowOperationStatus, WorkflowResumeClaimResult,
    WorkflowRunState, WorkflowRuntime, WorkflowRuntimeEventType, WorkflowRuntimeStore,
    WorkflowStopRequestResult,
};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc, Mutex};

#[derive(Default)]
struct RecordingLlmProvider {
    requests: Mutex<Vec<LlmRequest>>,
    contexts: Mutex<Vec<ProviderCallContext>>,
    /// 测试可注入返回的 cost_usd（F13 单次预算合同）。
    cost_usd: Mutex<Option<f64>>,
}

#[test]
fn f8_summarizer_contract_requires_complete_config_and_matching_chapter_text_edge() {
    let mut workflow = WorkflowDefinition {
        id: WorkflowId::from("f8-summarizer-contract"),
        name: "F8 Summarizer Contract".to_owned(),
        nodes: vec![
            node("writer", "writer"),
            NodeInstance {
                id: NodeId::from("summarizer"),
                type_name: "summarizer".to_owned(),
                label: None,
                config: json!({
                    "provider_id": "provider-main",
                    "model_id": "model-main",
                    "chapter_id": "chapter-1",
                    "chapter_document_id": "documents/chapter-1.md",
                    "chapter_text_alias": "chapter_body",
                    "auto_mode": true
                }),
                position: None,
            },
        ],
        edges: Vec::new(),
        metadata: Value::Null,
    };

    let missing_edge = validate_workflow_execution_contracts(&workflow).unwrap_err();
    assert!(missing_edge
        .to_string()
        .contains("incoming data edge with alias chapter_body"));

    workflow.edges.push(Edge {
        id: EdgeId::from("chapter-body"),
        kind: WorkflowEdgeKind::Data,
        from: PortEndpoint {
            node_id: NodeId::from("writer"),
            port_name: "output".to_owned(),
        },
        to: PortEndpoint {
            node_id: NodeId::from("summarizer"),
            port_name: "input".to_owned(),
        },
        alias: Some("chapter_body".to_owned()),
        communication: None,
    });
    validate_workflow_execution_contracts(&workflow).unwrap();

    workflow.nodes[1].config["chapter_document_id"] = json!("  ");
    let missing_document = validate_workflow_execution_contracts(&workflow).unwrap_err();
    assert!(missing_document
        .to_string()
        .contains("chapter_document_id cannot be empty"));
}

#[test]
fn runtime_store_create_state_rejects_duplicate_without_overwriting() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("create-only");
    let run_id = RunId::from("run-1");
    let original = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    store.create_state(&original).unwrap();

    let mut conflicting = original.clone();
    conflicting.status = RunStatus::Failed;
    assert!(store.create_state(&conflicting).is_err());

    let loaded = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
    assert_eq!(loaded.status, RunStatus::Queued);
}

#[test]
fn runtime_store_round_trips_prepared_workflow_snapshot() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("prepared-snapshot"),
        name: "Prepared Snapshot".to_owned(),
        nodes: vec![node("start", "start")],
        edges: Vec::new(),
        metadata: json!({ "immutable": true }),
    };
    let run_id = RunId::from("run-1");
    let mut state = WorkflowRunState::new(workflow.id.clone(), run_id.clone());
    state.prepared_workflow = Some(workflow.clone());
    state.start_node_id = Some(NodeId::from("start"));

    store.create_state(&state).unwrap();
    let loaded = store.load_state(&workflow.id, &run_id).unwrap().unwrap();

    assert_eq!(loaded.prepared_workflow, Some(workflow));
    assert_eq!(loaded.start_node_id, Some(NodeId::from("start")));
}

/// C10：revision>0 后 state_json 不再重复整图，但 load 仍从独立列补回 prepared_workflow。
#[test]
fn c10_save_slims_prepared_workflow_but_load_rehydrates_from_column() {
    let temp = tempfile::tempdir().unwrap();
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("c10-slim"),
        name: "C10 Slim Workflow Unique Marker".to_owned(),
        nodes: vec![node("start", "start"), node("llm", "llm")],
        edges: Vec::new(),
        metadata: json!({ "marker": "c10-prepared-body-must-not-rewrite" }),
    };
    let run_id = RunId::from("run-c10");
    let mut state = WorkflowRunState::new(workflow.id.clone(), run_id.clone());
    state.prepared_workflow = Some(workflow.clone());
    state.start_node_id = Some(NodeId::from("start"));
    store.create_state(&state).unwrap();

    // revision 0 → 1：仍可含 definition；再 save 一次强制 slim。
    state.status = RunStatus::Running;
    store.save_state(&mut state, None).unwrap();
    assert_eq!(state.state_revision, 1);
    state.status = RunStatus::Queued;
    store.save_state(&mut state, None).unwrap();
    assert_eq!(state.state_revision, 2);

    // 原始 state_json 不应再嵌完整图标记（写放大关闭）。
    let db =
        rusqlite::Connection::open(temp.path().join(ariadne::workflow::RUNTIME_DB_FILE)).unwrap();
    let state_json: String = db
        .query_row(
            "SELECT state_json FROM workflow_runs WHERE workflow_id = ?1 AND run_id = ?2",
            rusqlite::params![workflow.id.as_str(), run_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !state_json.contains("C10 Slim Workflow Unique Marker"),
        "state_json after slim must not re-embed full prepared_workflow body"
    );
    let column: Option<String> = db
        .query_row(
            "SELECT prepared_workflow_json FROM workflow_runs WHERE workflow_id = ?1 AND run_id = ?2",
            rusqlite::params![workflow.id.as_str(), run_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        column
            .as_deref()
            .is_some_and(|raw| raw.contains("C10 Slim Workflow Unique Marker")),
        "prepared_workflow_json column must keep frozen definition"
    );

    let loaded = store.load_state(&workflow.id, &run_id).unwrap().unwrap();
    assert_eq!(loaded.prepared_workflow, Some(workflow));
    assert_eq!(loaded.status, RunStatus::Queued);
    assert_eq!(loaded.state_revision, 2);
}

/// F10-d：create 后无 lease / lease 过期的 Queued|Running 可被 orphan 列表发现。
#[test]
fn f10d_lists_orphaned_queued_and_running_without_live_lease() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("orphan-wf"),
        name: "Orphan".to_owned(),
        nodes: vec![node("start", "start")],
        edges: Vec::new(),
        metadata: json!({}),
    };

    // Queued without any lease row (create then crash before acquire).
    let mut queued = WorkflowRunState::new(workflow.id.clone(), RunId::from("orphan-queued"));
    queued.prepared_workflow = Some(workflow.clone());
    store.create_state(&queued).unwrap();

    // Running with expired lease.
    let mut running = WorkflowRunState::new(workflow.id.clone(), RunId::from("orphan-running"));
    running.prepared_workflow = Some(workflow.clone());
    running.status = RunStatus::Running;
    store.create_state(&running).unwrap();
    store
        .acquire_worker_lease(
            &workflow.id,
            &RunId::from("orphan-running"),
            "dead",
            1_000,
            50,
        )
        .unwrap()
        .unwrap();

    // Live lease must not be listed.
    let mut live = WorkflowRunState::new(workflow.id.clone(), RunId::from("live-running"));
    live.prepared_workflow = Some(workflow.clone());
    live.status = RunStatus::Running;
    store.create_state(&live).unwrap();
    store
        .acquire_worker_lease(
            &workflow.id,
            &RunId::from("live-running"),
            "alive",
            10_000,
            5_000,
        )
        .unwrap()
        .unwrap();

    // Paused is not orphan-runnable for auto recovery.
    let mut paused = WorkflowRunState::new(workflow.id.clone(), RunId::from("paused"));
    paused.prepared_workflow = Some(workflow);
    paused.status = RunStatus::Paused;
    store.create_state(&paused).unwrap();

    let orphans = store.list_orphaned_runnable_states(10_100).unwrap();
    let ids: Vec<_> = orphans
        .iter()
        .map(|s| s.run_id.as_str().to_owned())
        .collect();
    assert!(ids.contains(&"orphan-queued".to_owned()));
    assert!(ids.contains(&"orphan-running".to_owned()));
    assert!(!ids.contains(&"live-running".to_owned()));
    assert!(!ids.contains(&"paused".to_owned()));

    // claim_resume 可接管 orphan Queued。
    let claimed = store
        .claim_resume(
            &WorkflowId::from("orphan-wf"),
            &RunId::from("orphan-queued"),
            "recover-owner",
            10_200,
            500,
        )
        .unwrap();
    assert!(matches!(claimed, WorkflowResumeClaimResult::Claimed { .. }));
}

/// F12-c：终态不得被 Pause/Stop 复活。
#[test]
fn f12c_pause_and_stop_reject_terminal_runs() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("f12c"),
        name: "f12c".to_owned(),
        nodes: vec![node("start", "start")],
        edges: Vec::new(),
        metadata: json!({}),
    };
    for terminal in [RunStatus::Succeeded, RunStatus::Failed, RunStatus::Stopped] {
        let mut runtime =
            WorkflowRuntime::new(&workflow, RunId::from(format!("r-{terminal:?}"))).unwrap();
        runtime.state.status = terminal;
        assert!(runtime.request_pause("no").is_err(), "pause {terminal:?}");
        assert!(runtime.request_stop("no").is_err(), "stop {terminal:?}");
    }
}

/// F12-a：Queued/Paused 上 Stop 直接 Stopped；Running 进入 Stopping。
#[test]
fn f12a_stop_sets_stopping_or_stopped() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("f12a"),
        name: "f12a".to_owned(),
        nodes: vec![node("start", "start")],
        edges: Vec::new(),
        metadata: json!({}),
    };
    let mut queued = WorkflowRuntime::new(&workflow, RunId::from("q")).unwrap();
    queued.state.status = RunStatus::Queued;
    queued.request_stop("user").unwrap();
    assert_eq!(queued.state.status, RunStatus::Stopped);

    let mut running = WorkflowRuntime::new(&workflow, RunId::from("r")).unwrap();
    running.state.status = RunStatus::Running;
    running.request_stop("user").unwrap();
    assert_eq!(running.state.status, RunStatus::Stopping);
}

struct GatedDispatchExecutor {
    dispatch_before_control: bool,
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    side_effects: Arc<AtomicUsize>,
}

impl WorkflowNodeExecutor for GatedDispatchExecutor {
    fn operation_policy(
        &self,
        _request: &WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowOperationPolicy> {
        Ok(WorkflowOperationPolicy::remote_response())
    }

    fn execute(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowNodeExecutionOutput> {
        if self.dispatch_before_control {
            request.dispatch_authorization.authorize_dispatch()?;
            self.side_effects.fetch_add(1, Ordering::SeqCst);
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
            return Err(ariadne::contracts::CoreError::external_cancelled(
                "gated_dispatch",
                ariadne::contracts::ExternalDispatchOutcome::DispatchedUnknown,
            ));
        }

        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        request.dispatch_authorization.authorize_dispatch()?;
        self.side_effects.fetch_add(1, Ordering::SeqCst);
        Ok(WorkflowNodeExecutionOutput::default())
    }
}

#[derive(Clone, Copy)]
enum F12ControlRace {
    Pause,
    Stop,
}

fn run_control_dispatch_race(
    control: F12ControlRace,
    dispatch_before_control: bool,
) -> (
    WorkflowRunState,
    Vec<ariadne::workflow::WorkflowOperation>,
    usize,
) {
    let temp = tempfile::tempdir().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from(match (control, dispatch_before_control) {
            (F12ControlRace::Pause, false) => "f12-pause-wins",
            (F12ControlRace::Pause, true) => "f12-dispatch-before-pause",
            (F12ControlRace::Stop, false) => "f12-stop-wins",
            (F12ControlRace::Stop, true) => "f12-dispatch-wins",
        }),
        name: "F12 dispatch barrier".to_owned(),
        nodes: vec![node("remote", "writer")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    let control_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    control_store.create_state(&runtime.state).unwrap();
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let lease = control_store
        .acquire_worker_lease(&workflow.id, &run_id, "f12-worker", now_ms, 60_000)
        .unwrap()
        .unwrap();
    let worker_store = SqliteWorkflowRuntimeStore::open(temp.path())
        .unwrap()
        .with_worker_lease(lease);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let side_effects = Arc::new(AtomicUsize::new(0));
    let executor_side_effects = Arc::clone(&side_effects);
    let thread_workflow = workflow.clone();
    let expected_worker_status = match control {
        F12ControlRace::Pause => RunStatus::Paused,
        F12ControlRace::Stop => RunStatus::Stopped,
    };
    let worker = std::thread::spawn(move || {
        let mut runtime = runtime;
        let mut executor = GatedDispatchExecutor {
            dispatch_before_control,
            entered: entered_tx,
            release: release_rx,
            side_effects: executor_side_effects,
        };
        assert_eq!(
            runtime
                .run_persisted(&thread_workflow, &mut executor, &worker_store)
                .unwrap(),
            expected_worker_status
        );
    });

    entered_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    match control {
        F12ControlRace::Pause => {
            let mut state = control_store
                .load_state(&workflow.id, &run_id)
                .unwrap()
                .unwrap();
            let mut control_runtime = WorkflowRuntime::from_state(state.clone());
            control_runtime.request_pause("user pause").unwrap();
            state = control_runtime.state;
            control_store.save_state(&mut state, None).unwrap();
            assert_eq!(state.status, RunStatus::Paused);
            assert_eq!(state.control, RunControl::Pause);
        }
        F12ControlRace::Stop => {
            let stop = control_store
                .request_stop(&workflow.id, &run_id, "user stop", now_ms + 1)
                .unwrap();
            let WorkflowStopRequestResult::Saved { state: stopping } = stop else {
                panic!("run must exist");
            };
            assert_eq!(stopping.status, RunStatus::Stopping);
            assert_eq!(stopping.control, RunControl::Stop);
        }
    }
    release_tx.send(()).unwrap();
    worker.join().unwrap();

    let persisted = control_store
        .load_state(&workflow.id, &run_id)
        .unwrap()
        .unwrap();
    let operations = control_store
        .list_operations(&workflow.id, &run_id)
        .unwrap();
    (persisted, operations, side_effects.load(Ordering::SeqCst))
}

#[test]
fn f12_stop_wins_before_dispatch_without_external_side_effect() {
    let (state, operations, side_effects) = run_control_dispatch_race(F12ControlRace::Stop, false);
    assert_eq!(state.status, RunStatus::Stopped);
    assert_eq!(state.control, RunControl::Stop);
    assert_eq!(side_effects, 0);
    assert!(operations.is_empty(), "Prepared must be cleaned safely");
    assert!(state
        .structured_events
        .iter()
        .any(|event| event.event_type == WorkflowRuntimeEventType::RunStopRequested));
    assert!(state
        .structured_events
        .iter()
        .any(|event| event.event_type == WorkflowRuntimeEventType::RunStopped));
}

#[test]
fn f12_dispatch_wins_then_unknown_result_stops_but_preserves_in_doubt() {
    let (state, operations, side_effects) = run_control_dispatch_race(F12ControlRace::Stop, true);
    assert_eq!(state.status, RunStatus::Stopped);
    assert_eq!(state.control, RunControl::Stop);
    assert_eq!(side_effects, 1);
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].status, WorkflowOperationStatus::InDoubt);
}

#[test]
fn f12_pause_wins_before_dispatch_without_external_side_effect() {
    let (state, operations, side_effects) = run_control_dispatch_race(F12ControlRace::Pause, false);
    assert_eq!(state.status, RunStatus::Paused);
    assert_eq!(state.control, RunControl::Pause);
    assert_eq!(side_effects, 0);
    assert!(operations.is_empty(), "Prepared must be cleaned safely");
    assert!(state
        .structured_events
        .iter()
        .any(|event| event.event_type == WorkflowRuntimeEventType::RunPaused));
}

#[test]
fn f12_dispatch_wins_before_pause_and_unknown_result_preserves_in_doubt() {
    let (state, operations, side_effects) = run_control_dispatch_race(F12ControlRace::Pause, true);
    assert_eq!(state.status, RunStatus::Paused);
    assert_eq!(state.control, RunControl::Pause);
    assert_eq!(side_effects, 1);
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].status, WorkflowOperationStatus::InDoubt);
}

#[test]
fn f12_orphaned_stopping_run_converges_after_lease_expiry() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("f12-orphan-stop");
    let run_id = RunId::from("run-1");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.status = RunStatus::Running;
    store.create_state(&state).unwrap();
    store
        .acquire_worker_lease(&workflow_id, &run_id, "dead-worker", 1_000, 100)
        .unwrap()
        .unwrap();

    let WorkflowStopRequestResult::Saved { state: stopping } = store
        .request_stop(&workflow_id, &run_id, "stop before crash", 1_050)
        .unwrap()
    else {
        panic!("run must exist");
    };
    assert_eq!(stopping.status, RunStatus::Stopping);
    let orphans = store.list_orphaned_runnable_states(1_101).unwrap();
    assert!(orphans.iter().any(|state| state.run_id == run_id));

    let WorkflowStopRequestResult::Saved { state: stopped } = store
        .request_stop(&workflow_id, &run_id, "stop before crash", 1_101)
        .unwrap()
    else {
        panic!("run must exist");
    };
    assert_eq!(stopped.status, RunStatus::Stopped);
    assert_eq!(stopped.control, RunControl::Stop);
    assert_eq!(
        stopped
            .structured_events
            .iter()
            .filter(|event| event.event_type == WorkflowRuntimeEventType::RunStopRequested)
            .count(),
        1
    );
    assert_eq!(
        stopped
            .structured_events
            .iter()
            .filter(|event| event.event_type == WorkflowRuntimeEventType::RunStopped)
            .count(),
        1
    );
}

struct UnfencedSuccessExecutor {
    leaked_authorization: Arc<Mutex<Option<ExternalDispatchAuthorization>>>,
}

impl WorkflowNodeExecutor for UnfencedSuccessExecutor {
    fn operation_policy(
        &self,
        _request: &WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowOperationPolicy> {
        Ok(WorkflowOperationPolicy::remote_response())
    }

    fn execute(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowNodeExecutionOutput> {
        *self.leaked_authorization.lock().unwrap() = Some(request.dispatch_authorization.clone());
        Ok(WorkflowNodeExecutionOutput::default())
    }
}

#[test]
fn f12_unfenced_success_is_atomically_quarantined_and_late_dispatch_is_rejected() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("f12-unfenced-success"),
        name: "F12 unfenced success".to_owned(),
        nodes: vec![node("remote", "writer")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    let leaked_authorization = Arc::new(Mutex::new(None));
    let mut executor = UnfencedSuccessExecutor {
        leaked_authorization: leaked_authorization.clone(),
    };

    assert_eq!(
        runtime
            .run_persisted(&workflow, &mut executor, &store)
            .unwrap(),
        RunStatus::Paused
    );
    let operation = store
        .list_operations(&workflow.id, &run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(operation.status, WorkflowOperationStatus::InDoubt);
    assert!(operation.response_json.is_none());
    let node = runtime.state.nodes.get(&NodeId::from("remote")).unwrap();
    assert!(node
        .error
        .as_deref()
        .is_some_and(|error| error.contains("workflow executor contract violation")));

    let late_error = leaked_authorization
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .authorize_dispatch()
        .unwrap_err();
    assert_eq!(
        late_error.external_dispatch_outcome(),
        Some(ariadne::contracts::ExternalDispatchOutcome::NotDispatched)
    );
    assert_eq!(
        store
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap()
            .status,
        WorkflowOperationStatus::InDoubt
    );
}

#[test]
fn runtime_worker_lease_is_unique_and_expired_owner_can_be_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let first_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let second_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("lease-workflow");
    let run_id = RunId::from("lease-run");
    first_store
        .create_state(&WorkflowRunState::new(workflow_id.clone(), run_id.clone()))
        .unwrap();

    let first = first_store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-a", 1_000, 100)
        .unwrap()
        .unwrap();
    assert_eq!(first.generation, 1);
    assert_eq!(first.expires_at_ms, 1_100);

    assert!(second_store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-b", 1_099, 100)
        .unwrap()
        .is_none());

    let takeover = second_store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-b", 1_100, 200)
        .unwrap()
        .unwrap();
    assert_eq!(takeover.generation, 2);
    assert_eq!(
        second_store
            .load_worker_lease(&workflow_id, &run_id)
            .unwrap(),
        Some(takeover.clone())
    );

    assert!(!first_store
        .heartbeat_worker_lease(
            &workflow_id,
            &run_id,
            "owner-a",
            first.generation,
            1_101,
            100
        )
        .unwrap());
    assert!(!first_store
        .release_worker_lease(&workflow_id, &run_id, "owner-a", first.generation)
        .unwrap());
}

#[test]
fn runtime_worker_lease_allows_only_one_concurrent_sqlite_claim() {
    let temp = tempfile::tempdir().unwrap();
    let workflow_id = WorkflowId::from("concurrent-lease-workflow");
    let run_id = RunId::from("concurrent-lease-run");
    SqliteWorkflowRuntimeStore::open(temp.path())
        .unwrap()
        .create_state(&WorkflowRunState::new(workflow_id.clone(), run_id.clone()))
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut workers = Vec::new();
    for index in 0..8 {
        let root = temp.path().to_path_buf();
        let barrier = Arc::clone(&barrier);
        let workflow_id = workflow_id.clone();
        let run_id = run_id.clone();
        workers.push(std::thread::spawn(move || {
            let store = SqliteWorkflowRuntimeStore::open(root).unwrap();
            barrier.wait();
            store
                .acquire_worker_lease(
                    &workflow_id,
                    &run_id,
                    &format!("owner-{index}"),
                    4_000,
                    1_000,
                )
                .unwrap()
        }));
    }
    let leases = workers
        .into_iter()
        .filter_map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].generation, 1);
    let persisted = SqliteWorkflowRuntimeStore::open(temp.path())
        .unwrap()
        .load_worker_lease(&workflow_id, &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, leases[0]);
}

#[test]
fn runtime_worker_lease_heartbeat_and_release_require_current_owner() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("lease-owner-workflow");
    let run_id = RunId::from("lease-owner-run");
    store
        .create_state(&WorkflowRunState::new(workflow_id.clone(), run_id.clone()))
        .unwrap();
    let lease = store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-a", 2_000, 100)
        .unwrap()
        .unwrap();

    assert!(!store
        .heartbeat_worker_lease(
            &workflow_id,
            &run_id,
            "owner-b",
            lease.generation,
            2_010,
            100
        )
        .unwrap());
    assert!(!store
        .heartbeat_worker_lease(
            &workflow_id,
            &run_id,
            "owner-a",
            lease.generation + 1,
            2_010,
            100
        )
        .unwrap());
    assert!(store
        .heartbeat_worker_lease(
            &workflow_id,
            &run_id,
            "owner-a",
            lease.generation,
            2_010,
            200
        )
        .unwrap());
    assert!(store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-b", 2_150, 100)
        .unwrap()
        .is_none());
    assert!(!store
        .release_worker_lease(&workflow_id, &run_id, "owner-b", lease.generation)
        .unwrap());
    assert!(!store
        .release_worker_lease(&workflow_id, &run_id, "owner-a", lease.generation + 1)
        .unwrap());
    assert!(store
        .release_worker_lease(&workflow_id, &run_id, "owner-a", lease.generation)
        .unwrap());
    assert!(store
        .load_worker_lease(&workflow_id, &run_id)
        .unwrap()
        .is_none());
    let next = store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-c", 2_151, 100)
        .unwrap()
        .unwrap();
    assert_eq!(next.generation, lease.generation + 1);
    assert_eq!(
        store.schema_version().unwrap(),
        Some(ariadne::workflow::RUNTIME_SCHEMA_VERSION)
    );
}

#[test]
fn workflow_operation_journal_persists_identity_and_cas_transitions() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("operation-workflow");
    let run_id = RunId::from("operation-run");
    store
        .create_state(&WorkflowRunState::new(workflow_id.clone(), run_id.clone()))
        .unwrap();
    let operation = NewWorkflowOperation {
        operation_id: "op-stable-1".to_owned(),
        workflow_id: workflow_id.clone(),
        run_id: run_id.clone(),
        node_id: NodeId::from("llm-node"),
        attempt: 2,
        kind: "provider_call".to_owned(),
        provider: "provider-a".to_owned(),
        request_hash: "sha256:request".to_owned(),
        lease_generation: 7,
        recovery_policy: WorkflowOperationRecoveryPolicy::ManualResolution,
        response_policy: WorkflowOperationResponsePolicy::AllowExternalResponse,
    };
    store.create_operation(&operation, 1_000).unwrap();
    assert!(store.create_operation(&operation, 1_001).is_err());

    let prepared = store.load_operation("op-stable-1").unwrap().unwrap();
    assert_eq!(prepared.status, WorkflowOperationStatus::Prepared);
    assert_eq!(prepared.lease_generation, 7);
    assert_eq!(
        prepared.recovery_policy,
        WorkflowOperationRecoveryPolicy::ManualResolution
    );
    assert_eq!(
        prepared.response_policy,
        WorkflowOperationResponsePolicy::AllowExternalResponse
    );
    assert_eq!(prepared.created_at_ms, 1_000);
    assert!(prepared.response_json.is_none());

    assert!(store
        .transition_operation(
            "op-stable-1",
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched,
            None,
            1_010,
        )
        .unwrap());
    assert!(!store
        .transition_operation(
            "op-stable-1",
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched,
            None,
            1_011,
        )
        .unwrap());
    let response = json!({"text": "done", "usage": {"input": 3}});
    assert!(store
        .transition_operation(
            "op-stable-1",
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::Completed,
            Some(&response),
            1_020,
        )
        .unwrap());
    assert!(store
        .transition_operation(
            "op-stable-1",
            WorkflowOperationStatus::Completed,
            WorkflowOperationStatus::Committed,
            None,
            1_030,
        )
        .unwrap());

    let committed = store.load_operation("op-stable-1").unwrap().unwrap();
    assert_eq!(committed.status, WorkflowOperationStatus::Committed);
    assert_eq!(committed.response_json, Some(response));
    assert_eq!(committed.dispatched_at_ms, Some(1_010));
    assert_eq!(committed.completed_at_ms, Some(1_020));
    assert_eq!(committed.committed_at_ms, Some(1_030));
    assert_eq!(
        store.list_operations(&workflow_id, &run_id).unwrap(),
        vec![committed]
    );
}

#[test]
fn workflow_operation_v7_migration_recovers_from_a_single_added_policy_column() {
    let temp = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("runtime.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE workflow_operations (
                 operation_id TEXT PRIMARY KEY,
                 workflow_id TEXT NOT NULL,
                 run_id TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 attempt INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 request_hash TEXT NOT NULL,
                 lease_generation INTEGER NOT NULL,
                 recovery_policy TEXT NOT NULL DEFAULT 'manual_resolution',
                 status TEXT NOT NULL CHECK(status IN (
                     'prepared', 'dispatched', 'completed', 'in_doubt', 'aborted', 'committed'
                 )),
                 response_json TEXT,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL,
                 dispatched_at_ms INTEGER,
                 completed_at_ms INTEGER,
                 in_doubt_at_ms INTEGER,
                 committed_at_ms INTEGER
             );",
        )
        .unwrap();
    drop(connection);

    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    assert_eq!(
        store.schema_version().unwrap(),
        Some(ariadne::workflow::RUNTIME_SCHEMA_VERSION)
    );
    let connection = rusqlite::Connection::open(temp.path().join("runtime.db")).unwrap();
    let mut statement = connection
        .prepare("PRAGMA table_info(workflow_operations)")
        .unwrap();
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(columns.iter().any(|column| column == "recovery_policy"));
    assert!(columns.iter().any(|column| column == "response_policy"));
}

#[test]
fn workflow_operation_journal_tracks_in_doubt_and_rejects_invalid_transitions() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("in-doubt-workflow");
    let run_id = RunId::from("in-doubt-run");
    store
        .create_state(&WorkflowRunState::new(workflow_id.clone(), run_id.clone()))
        .unwrap();
    store
        .create_operation(
            &NewWorkflowOperation {
                operation_id: "op-in-doubt".to_owned(),
                workflow_id,
                run_id,
                node_id: NodeId::from("node"),
                attempt: 1,
                kind: "provider_call".to_owned(),
                provider: "provider".to_owned(),
                request_hash: "hash".to_owned(),
                lease_generation: 1,
                recovery_policy: WorkflowOperationRecoveryPolicy::ManualResolution,
                response_policy: WorkflowOperationResponsePolicy::AllowExternalResponse,
            },
            2_000,
        )
        .unwrap();
    assert!(store
        .transition_operation(
            "op-in-doubt",
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched,
            None,
            2_010,
        )
        .unwrap());
    assert!(store
        .transition_operation(
            "op-in-doubt",
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::InDoubt,
            None,
            2_020,
        )
        .unwrap());
    assert!(store
        .transition_operation(
            "op-in-doubt",
            WorkflowOperationStatus::InDoubt,
            WorkflowOperationStatus::Committed,
            None,
            2_030,
        )
        .is_err());
    assert!(store
        .transition_operation(
            "op-in-doubt",
            WorkflowOperationStatus::InDoubt,
            WorkflowOperationStatus::Completed,
            None,
            2_030,
        )
        .is_err());
    let persisted = store.load_operation("op-in-doubt").unwrap().unwrap();
    assert_eq!(persisted.status, WorkflowOperationStatus::InDoubt);
    assert_eq!(persisted.in_doubt_at_ms, Some(2_020));
}

#[test]
fn replayable_in_doubt_operation_reenters_same_attempt_without_becoming_terminal() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("replay-in-doubt"),
        name: "Replay in doubt".to_owned(),
        nodes: vec![node("writer", "writer")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    let unknown = || ariadne::contracts::CoreError::ProviderRequest {
        service: "replay-provider".to_owned(),
        outcome: ariadne::contracts::ExternalDispatchOutcome::DispatchedUnknown,
        message: "response unknown".to_owned(),
    };
    let mut executor = ScriptedExecutor::default()
        .with_operation_policy(WorkflowOperationPolicy::replayable_receipt());
    executor.push_error("writer", unknown());
    executor.push_error("writer", unknown());

    assert_eq!(
        runtime
            .run_persisted(&workflow, &mut executor, &store)
            .unwrap(),
        RunStatus::Paused
    );
    let first_operation = store
        .list_operations(&workflow.id, &run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first_operation.status, WorkflowOperationStatus::InDoubt);
    assert_eq!(first_operation.attempt, 1);

    runtime.resume().unwrap();
    assert_eq!(
        runtime
            .run_persisted(&workflow, &mut executor, &store)
            .unwrap(),
        RunStatus::Paused
    );
    assert_eq!(executor.call_count("writer"), 2);
    assert_eq!(
        executor.calls[0].operation_id,
        executor.calls[1].operation_id
    );
    assert_eq!(executor.calls[1].operation_attempt, 1);
    let operations = store.list_operations(&workflow.id, &run_id).unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].status, WorkflowOperationStatus::InDoubt);
}

#[test]
fn workflow_operation_journal_only_deletes_prepared_records() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("delete-operation-workflow");
    let run_id = RunId::from("delete-operation-run");
    store
        .create_state(&WorkflowRunState::new(workflow_id.clone(), run_id.clone()))
        .unwrap();
    let new_operation = |operation_id: &str| NewWorkflowOperation {
        operation_id: operation_id.to_owned(),
        workflow_id: workflow_id.clone(),
        run_id: run_id.clone(),
        node_id: NodeId::from("node"),
        attempt: 1,
        kind: "provider_call".to_owned(),
        provider: "provider".to_owned(),
        request_hash: "hash".to_owned(),
        lease_generation: 1,
        recovery_policy: WorkflowOperationRecoveryPolicy::ManualResolution,
        response_policy: WorkflowOperationResponsePolicy::AllowExternalResponse,
    };
    store
        .create_operation(&new_operation("prepared"), 1)
        .unwrap();
    assert!(store.delete_prepared_operation("prepared").unwrap());
    assert!(store.load_operation("prepared").unwrap().is_none());

    store
        .create_operation(&new_operation("dispatched"), 2)
        .unwrap();
    store
        .transition_operation(
            "dispatched",
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched,
            None,
            3,
        )
        .unwrap();
    assert!(!store.delete_prepared_operation("dispatched").unwrap());
    assert_eq!(
        store.load_operation("dispatched").unwrap().unwrap().status,
        WorkflowOperationStatus::Dispatched
    );
}

#[test]
fn persisted_external_node_commits_operation_journal_after_state_save() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("journal-runtime"),
        name: "Journal runtime".to_owned(),
        nodes: vec![node("writer", "llm")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default()
        .with_operation_policy(ariadne::workflow::WorkflowOperationPolicy::remote_response());
    executor.push("writer", inline_output("draft", "completed once"));

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(executor.call_count("writer"), 1);
    let operations = store
        .list_operations(&workflow.id, &runtime.state.run_id)
        .unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].status, WorkflowOperationStatus::Committed);
    assert_eq!(operations[0].attempt, 1);
    assert!(operations[0].response_json.is_some());
    assert_eq!(
        runtime
            .state
            .nodes
            .get(&NodeId::from("writer"))
            .unwrap()
            .execution_attempts,
        1
    );
}

#[test]
fn external_node_name_does_not_enable_operation_journal_without_declared_policy() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("untracked-llm-name"),
        name: "Untracked LLM name".to_owned(),
        nodes: vec![node("writer", "llm")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("writer", inline_output("draft", "untracked"));

    assert_eq!(
        runtime
            .run_persisted(&workflow, &mut executor, &store)
            .unwrap(),
        RunStatus::Succeeded
    );
    assert!(store
        .list_operations(&workflow.id, &runtime.state.run_id)
        .unwrap()
        .is_empty());
}

#[test]
fn executor_adapter_tool_call_uses_journal_and_unknown_failure_does_not_reexecute() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("journal-tool-adapter"),
        name: "Journal tool adapter".to_owned(),
        nodes: vec![node("tool", "executor_adapter")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default()
        .with_operation_policy(ariadne::workflow::WorkflowOperationPolicy::remote_response());
    executor.push_error(
        "tool",
        ariadne::contracts::CoreError::External {
            service: "http-skill".to_owned(),
            message: "connection closed after tool dispatch".to_owned(),
        },
    );

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    assert_eq!(status, RunStatus::Paused);
    assert_eq!(executor.call_count("tool"), 1);
    let operation = store
        .list_operations(&workflow.id, &runtime.state.run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(operation.kind, "executor_adapter");
    assert_eq!(operation.status, WorkflowOperationStatus::InDoubt);

    runtime.resume().unwrap();
    assert_eq!(
        runtime
            .run_persisted(&workflow, &mut executor, &store)
            .unwrap(),
        RunStatus::Paused
    );
    assert_eq!(executor.call_count("tool"), 1);
}

#[test]
fn persisted_external_error_becomes_in_doubt_without_automatic_retry() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("journal-in-doubt"),
        name: "Journal in doubt".to_owned(),
        nodes: vec![node("writer", "llm")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default()
        .with_operation_policy(ariadne::workflow::WorkflowOperationPolicy::remote_response());
    executor.push_error(
        "writer",
        ariadne::contracts::CoreError::External {
            service: "mock-provider".to_owned(),
            message: "connection closed after dispatch".to_owned(),
        },
    );

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    assert_eq!(status, RunStatus::Paused);
    assert_eq!(executor.call_count("writer"), 1);
    let operation = store
        .list_operations(&workflow.id, &runtime.state.run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(operation.status, WorkflowOperationStatus::InDoubt);
    let node_state = runtime.state.nodes.get(&NodeId::from("writer")).unwrap();
    assert_eq!(node_state.status, RunStatus::Paused);
    assert_eq!(node_state.execution_attempts, 1);

    runtime.resume().unwrap();
    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();
    assert_eq!(status, RunStatus::Paused);
    assert_eq!(executor.call_count("writer"), 1);
}

#[test]
fn confirmed_not_dispatched_error_aborts_operation_and_uses_normal_retry_policy() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("journal-safe-retry"),
        name: "Journal safe retry".to_owned(),
        nodes: vec![node("writer", "llm")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut executor = ScriptedExecutor::default()
        .with_operation_policy(ariadne::workflow::WorkflowOperationPolicy::remote_response());
    executor.push_error(
        "writer",
        ariadne::contracts::CoreError::ProviderRequest {
            service: "mock-provider".to_owned(),
            outcome: ariadne::contracts::ExternalDispatchOutcome::NotDispatched,
            message: "connection refused before request dispatch".to_owned(),
        },
    );
    executor.push("writer", inline_output("draft", "retried safely"));

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();
    assert_eq!(status, RunStatus::Queued);
    assert_eq!(executor.call_count("writer"), 1);
    let first_operation = store
        .list_operations(&workflow.id, &runtime.state.run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first_operation.status, WorkflowOperationStatus::Aborted);

    runtime
        .state
        .nodes
        .get_mut(&NodeId::from("writer"))
        .unwrap()
        .error_state
        .as_mut()
        .unwrap()
        .next_retry_at_ms = Some(0);
    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();
    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(executor.call_count("writer"), 2);
    let operations = store
        .list_operations(&workflow.id, &runtime.state.run_id)
        .unwrap();
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].status, WorkflowOperationStatus::Aborted);
    assert_eq!(operations[1].status, WorkflowOperationStatus::Committed);
}

#[test]
fn in_doubt_retry_atomically_aborts_old_operation_and_claims_run() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("resolve-in-doubt");
    let run_id = RunId::from("run-1");
    let node_id = NodeId::from("writer");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.status = RunStatus::Paused;
    state.control = RunControl::Pause;
    state.pause_reason = Some("operation is in doubt".to_owned());
    state.nodes.insert(
        node_id.clone(),
        ariadne::workflow::WorkflowNodeRuntimeState {
            node_id: node_id.clone(),
            status: RunStatus::Paused,
            outputs: PortMap::new(),
            communication_output: None,
            communication_control: Default::default(),
            prompt_trace_hash: None,
            patch_session_commit_id: None,
            checkpoint_id: None,
            patch_write_back_state: None,
            metadata: Value::Null,
            error: Some("unknown remote result".to_owned()),
            error_state: None,
            execution_attempts: 1,
        },
    );
    store.create_state(&state).unwrap();
    store
        .create_operation(
            &NewWorkflowOperation {
                operation_id: "op-in-doubt".to_owned(),
                workflow_id: workflow_id.clone(),
                run_id: run_id.clone(),
                node_id: node_id.clone(),
                attempt: 1,
                kind: "llm".to_owned(),
                provider: "provider".to_owned(),
                request_hash: "hash".to_owned(),
                lease_generation: 1,
                recovery_policy: WorkflowOperationRecoveryPolicy::ManualResolution,
                response_policy: WorkflowOperationResponsePolicy::AllowExternalResponse,
            },
            1_000,
        )
        .unwrap();
    store
        .transition_operation(
            "op-in-doubt",
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched,
            None,
            1_001,
        )
        .unwrap();
    store
        .transition_operation(
            "op-in-doubt",
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::InDoubt,
            None,
            1_002,
        )
        .unwrap();

    let result = store
        .resolve_in_doubt_operation(
            "op-in-doubt",
            ariadne::workflow::InDoubtResolution::Retry,
            "recovery-owner",
            2_000,
            1_000,
        )
        .unwrap();
    let ariadne::workflow::InDoubtResolutionResult::Saved { state, lease } = result else {
        panic!("expected saved recovery result");
    };
    assert_eq!(state.status, RunStatus::Queued);
    assert_eq!(state.control, RunControl::Continue);
    let node = state.nodes.get(&node_id).unwrap();
    assert_eq!(node.status, RunStatus::Queued);
    assert!(node.error.is_none());
    assert_eq!(
        store.load_operation("op-in-doubt").unwrap().unwrap().status,
        WorkflowOperationStatus::Aborted
    );
    assert_eq!(lease.unwrap().owner_id, "recovery-owner");
}

#[test]
fn receipt_only_in_doubt_operation_rejects_external_response_without_mutation() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("receipt-only-response");
    let run_id = RunId::from("run-1");
    let node_id = NodeId::from("summarizer");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.status = RunStatus::Paused;
    state.control = RunControl::Pause;
    state.pause_reason = Some("operation is in doubt".to_owned());
    state.nodes.insert(
        node_id.clone(),
        ariadne::workflow::WorkflowNodeRuntimeState {
            node_id: node_id.clone(),
            status: RunStatus::Paused,
            outputs: PortMap::new(),
            communication_output: None,
            communication_control: Default::default(),
            prompt_trace_hash: None,
            patch_session_commit_id: None,
            checkpoint_id: None,
            patch_write_back_state: None,
            metadata: Value::Null,
            error: Some("receipt not visible yet".to_owned()),
            error_state: None,
            execution_attempts: 1,
        },
    );
    store.create_state(&state).unwrap();
    store
        .create_operation(
            &NewWorkflowOperation {
                operation_id: "op-receipt-only".to_owned(),
                workflow_id: workflow_id.clone(),
                run_id: run_id.clone(),
                node_id,
                attempt: 1,
                kind: "summarizer".to_owned(),
                provider: "knowledge".to_owned(),
                request_hash: "request-hash".to_owned(),
                lease_generation: 1,
                recovery_policy: WorkflowOperationRecoveryPolicy::ReconcileReceipt,
                response_policy: WorkflowOperationResponsePolicy::RequireExecutorReceipt,
            },
            1_000,
        )
        .unwrap();
    store
        .transition_operation(
            "op-receipt-only",
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched,
            None,
            1_001,
        )
        .unwrap();
    store
        .transition_operation(
            "op-receipt-only",
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::InDoubt,
            None,
            1_002,
        )
        .unwrap();

    let error = store
        .resolve_in_doubt_operation(
            "op-receipt-only",
            ariadne::workflow::InDoubtResolution::UseResponse {
                response: serde_json::to_value(WorkflowNodeExecutionOutput::default()).unwrap(),
            },
            "recovery-owner",
            2_000,
            1_000,
        )
        .unwrap_err();

    assert!(error.to_string().contains("executor receipt"));
    assert_eq!(
        store
            .load_operation("op-receipt-only")
            .unwrap()
            .unwrap()
            .status,
        WorkflowOperationStatus::InDoubt
    );
    assert_eq!(
        store.load_state(&workflow_id, &run_id).unwrap().unwrap(),
        state
    );
    assert!(store
        .load_worker_lease(&workflow_id, &run_id)
        .unwrap()
        .is_none());
}

#[test]
fn completed_operation_response_is_reused_without_second_provider_call() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("journal-replay"),
        name: "Journal replay".to_owned(),
        nodes: vec![node("writer", "llm")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    store.create_state(&runtime.state).unwrap();
    let operation_id = ariadne::skills::stable_text_hash(
        "workflow-operation-v1\0journal-replay\0run-1\0writer\x001",
    );
    let request_hash = ariadne::skills::stable_text_hash(
        &serde_json::to_string(&json!({
            "type_name": "llm",
            "config": Value::Null,
            "inputs": PortMap::new(),
            "communication_messages": Vec::<ariadne::workflow::CommunicationMessage>::new(),
            "metadata": Value::Null,
        }))
        .unwrap(),
    );
    store
        .create_operation(
            &NewWorkflowOperation {
                operation_id: operation_id.clone(),
                workflow_id: workflow.id.clone(),
                run_id: run_id.clone(),
                node_id: NodeId::from("writer"),
                attempt: 1,
                kind: "llm".to_owned(),
                provider: "llm".to_owned(),
                request_hash,
                lease_generation: 0,
                recovery_policy: WorkflowOperationRecoveryPolicy::ManualResolution,
                response_policy: WorkflowOperationResponsePolicy::AllowExternalResponse,
            },
            1_000,
        )
        .unwrap();
    assert!(store
        .transition_operation(
            &operation_id,
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched,
            None,
            1_001,
        )
        .unwrap());
    let cached_output = inline_output("draft", "cached response");
    assert!(store
        .transition_operation(
            &operation_id,
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::Completed,
            Some(&serde_json::to_value(&cached_output).unwrap()),
            1_002,
        )
        .unwrap());
    let mut executor = ScriptedExecutor::default()
        .with_operation_policy(WorkflowOperationPolicy::remote_response());

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(executor.call_count("writer"), 0);
    assert_eq!(
        store.load_operation(&operation_id).unwrap().unwrap().status,
        WorkflowOperationStatus::Committed
    );
    assert!(format!(
        "{:?}",
        runtime
            .state
            .nodes
            .get(&NodeId::from("writer"))
            .unwrap()
            .outputs
    )
    .contains("cached response"));
}

#[test]
fn persisted_operation_rejects_recovery_policy_drift_for_the_same_identity() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("journal-policy-drift"),
        name: "Journal policy drift".to_owned(),
        nodes: vec![node("writer", "llm")],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    store.create_state(&runtime.state).unwrap();
    let operation_id = ariadne::skills::stable_text_hash(
        "workflow-operation-v1\0journal-policy-drift\0run-1\0writer\x001",
    );
    let request_hash = ariadne::skills::stable_text_hash(
        &serde_json::to_string(&json!({
            "type_name": "llm",
            "config": Value::Null,
            "inputs": PortMap::new(),
            "communication_messages": Vec::<ariadne::workflow::CommunicationMessage>::new(),
            "metadata": Value::Null,
        }))
        .unwrap(),
    );
    store
        .create_operation(
            &NewWorkflowOperation {
                operation_id: operation_id.clone(),
                workflow_id: workflow.id.clone(),
                run_id,
                node_id: NodeId::from("writer"),
                attempt: 1,
                kind: "llm".to_owned(),
                provider: "llm".to_owned(),
                request_hash,
                lease_generation: 0,
                recovery_policy: WorkflowOperationRecoveryPolicy::ManualResolution,
                response_policy: WorkflowOperationResponsePolicy::AllowExternalResponse,
            },
            1_000,
        )
        .unwrap();
    let mut executor = ScriptedExecutor::default()
        .with_operation_policy(WorkflowOperationPolicy::reconcilable_receipt());
    executor.push("writer", inline_output("draft", "must not execute"));

    let error = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap_err();

    assert!(error.to_string().contains("operation identity mismatch"));
    assert_eq!(executor.call_count("writer"), 0);
    let operation = store.load_operation(&operation_id).unwrap().unwrap();
    assert_eq!(operation.status, WorkflowOperationStatus::Prepared);
    assert_eq!(
        operation.recovery_policy,
        WorkflowOperationRecoveryPolicy::ManualResolution
    );
    assert_eq!(
        operation.response_policy,
        WorkflowOperationResponsePolicy::AllowExternalResponse
    );
}

#[test]
fn atomic_node_commit_rolls_back_snapshot_events_and_operation_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("atomic-operation-commit");
    let run_id = RunId::from("run-1");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    store.create_state(&state).unwrap();
    let operation_id = "atomic-operation-commit-id".to_owned();
    store
        .create_operation(
            &NewWorkflowOperation {
                operation_id: operation_id.clone(),
                workflow_id: workflow_id.clone(),
                run_id: run_id.clone(),
                node_id: NodeId::from("writer"),
                attempt: 1,
                kind: "llm".to_owned(),
                provider: "provider".to_owned(),
                request_hash: "request-hash".to_owned(),
                lease_generation: 0,
                recovery_policy: WorkflowOperationRecoveryPolicy::ManualResolution,
                response_policy: WorkflowOperationResponsePolicy::AllowExternalResponse,
            },
            1_000,
        )
        .unwrap();
    store
        .transition_operation(
            &operation_id,
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched,
            None,
            1_001,
        )
        .unwrap();
    store
        .transition_operation(
            &operation_id,
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::Completed,
            Some(&serde_json::to_value(inline_output("draft", "cached")).unwrap()),
            1_002,
        )
        .unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("runtime.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER abort_operation_commit
             BEFORE UPDATE OF status ON workflow_operations
             WHEN NEW.status = 'committed'
             BEGIN SELECT RAISE(ABORT, 'forced operation commit failure'); END;",
        )
        .unwrap();
    state.status = RunStatus::Succeeded;
    state
        .structured_events
        .push(ariadne::workflow::WorkflowRuntimeEvent {
            sequence: 0,
            event_type: WorkflowRuntimeEventType::RunSucceeded,
            node_id: None,
            message: "must roll back".to_owned(),
            metadata: Value::Null,
        });
    state.next_event_sequence = 1;

    assert!(store.save_state(&mut state, Some(&operation_id)).is_err());
    let persisted = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
    assert_eq!(persisted.state_revision, 0);
    assert_eq!(persisted.status, RunStatus::Queued);
    assert!(persisted.structured_events.is_empty());
    assert_eq!(
        store.load_operation(&operation_id).unwrap().unwrap().status,
        WorkflowOperationStatus::Completed
    );
}

#[test]
fn real_http_response_is_reused_after_snapshot_commit_failure_without_second_call_or_cost() {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let request_count = Arc::new(AtomicUsize::new(0));
    let server_count = Arc::clone(&request_count);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("snapshot crash test"));
        server_count.fetch_add(1, Ordering::SeqCst);
        let response_body = r#"{
          "model":"test-model",
          "choices":[{"message":{"content":"remote result"},"finish_reason":"stop"}],
          "usage":{"prompt_tokens":10,"completion_tokens":5}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "real-http".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Real HTTP".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "test-model".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(1.0),
                output_cost_per_million_tokens: Some(2.0),
            }],
        },
        None,
    )
    .unwrap();
    let ledger = Arc::new(SqliteCostLedger::open(temp.path()).unwrap());
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("real-http-snapshot-crash"),
        name: "Real HTTP snapshot crash".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("writer"),
            type_name: "llm".to_owned(),
            label: None,
            config: json!({
                "provider_id": "real-http",
                "model_id": "test-model",
                "prompt_template": "snapshot crash test",
            }),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.create_state(&runtime.state).unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("runtime.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER abort_real_http_operation_commit
             BEFORE UPDATE OF status ON workflow_operations
             WHEN NEW.status = 'committed'
             BEGIN SELECT RAISE(ABORT, 'forced snapshot commit failure'); END;",
        )
        .unwrap();
    let mut external = RoutedExternalNodeExecutor::new();
    let first_provider = provider.clone();
    let first_ledger = Arc::clone(&ledger);
    external
        .register_handler_with_policy(
            "llm",
            WorkflowOperationPolicy::remote_response(),
            Box::new(move |request| {
                execute_llm_node(request, &first_provider, first_ledger.as_ref())
            }),
        )
        .unwrap();
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let first_error = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap_err();
    server.join().unwrap();
    assert!(first_error
        .to_string()
        .contains("forced snapshot commit failure"));
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    let operation = store
        .list_operations(&workflow.id, &run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(operation.status, WorkflowOperationStatus::Completed);
    let operation_id = operation.operation_id.clone();
    assert_eq!(
        ledger
            .list_costs(&CostQuery {
                operation_id: Some(operation_id.clone()),
                ..CostQuery::default()
            })
            .unwrap()
            .len(),
        1
    );

    connection
        .execute_batch("DROP TRIGGER abort_real_http_operation_commit;")
        .unwrap();
    let persisted = store.load_state(&workflow.id, &run_id).unwrap().unwrap();
    let mut recovered = WorkflowRuntime::from_state(persisted);
    let mut replay_external = RoutedExternalNodeExecutor::new();
    let replay_provider = provider;
    let replay_ledger = Arc::clone(&ledger);
    replay_external
        .register_handler_with_policy(
            "llm",
            WorkflowOperationPolicy::remote_response(),
            Box::new(move |request| {
                execute_llm_node(request, &replay_provider, replay_ledger.as_ref())
            }),
        )
        .unwrap();
    let mut replay_executor = BuiltinWorkflowNodeExecutor::new(&mut replay_external);

    let status = recovered
        .run_persisted(&workflow, &mut replay_executor, &store)
        .unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.load_operation(&operation_id).unwrap().unwrap().status,
        WorkflowOperationStatus::Committed
    );
    assert_eq!(
        ledger
            .list_costs(&CostQuery {
                operation_id: Some(operation_id),
                ..CostQuery::default()
            })
            .unwrap()
            .len(),
        1
    );
    assert!(format!(
        "{:?}",
        recovered.state.nodes[&NodeId::from("writer")].outputs
    )
    .contains("remote result"));
}

#[test]
fn real_http_disconnect_after_dispatch_pauses_in_doubt_without_second_request_on_resume() {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let request_count = Arc::new(AtomicUsize::new(0));
    let server_count = Arc::clone(&request_count);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("unknown dispatch test"));
        server_count.fetch_add(1, Ordering::SeqCst);
        // 已读取完整请求但不返回响应，模拟远端可能已执行而连接中断。
    });
    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "real-http-unknown".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Real HTTP unknown".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "test-model".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(1.0),
                output_cost_per_million_tokens: Some(2.0),
            }],
        },
        None,
    )
    .unwrap();
    let ledger = Arc::new(SqliteCostLedger::open(temp.path()).unwrap());
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("real-http-dispatch-unknown"),
        name: "Real HTTP dispatch unknown".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("writer"),
            type_name: "llm".to_owned(),
            label: None,
            config: json!({
                "provider_id": "real-http-unknown",
                "model_id": "test-model",
                "prompt_template": "unknown dispatch test",
            }),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.create_state(&runtime.state).unwrap();
    let mut external = RoutedExternalNodeExecutor::new();
    let handler_provider = provider;
    let handler_ledger = Arc::clone(&ledger);
    external
        .register_handler_with_policy(
            "llm",
            WorkflowOperationPolicy::remote_response(),
            Box::new(move |request| {
                execute_llm_node(request, &handler_provider, handler_ledger.as_ref())
            }),
        )
        .unwrap();
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let first_status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();
    server.join().unwrap();
    assert_eq!(first_status, RunStatus::Paused);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    let operation = store
        .list_operations(&workflow.id, &run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(operation.status, WorkflowOperationStatus::InDoubt);
    assert!(ledger
        .list_costs(&CostQuery {
            operation_id: Some(operation.operation_id.clone()),
            ..CostQuery::default()
        })
        .unwrap()
        .is_empty());

    runtime.resume().unwrap();
    let resumed_status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    assert_eq!(resumed_status, RunStatus::Paused);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap()
            .status,
        WorkflowOperationStatus::InDoubt
    );
}

#[test]
fn expired_worker_generation_cannot_save_after_takeover() {
    let temp = tempfile::tempdir().unwrap();
    let workflow_id = WorkflowId::from("fenced-save-workflow");
    let run_id = RunId::from("fenced-save-run");
    let base_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    base_store.create_state(&state).unwrap();
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let stale_lease = base_store
        .acquire_worker_lease(
            &workflow_id,
            &run_id,
            "owner-a",
            now_ms.saturating_sub(1_000),
            100,
        )
        .unwrap()
        .unwrap();
    let current_lease = base_store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-b", now_ms, 60_000)
        .unwrap()
        .unwrap();
    assert!(current_lease.generation > stale_lease.generation);

    let stale_store = SqliteWorkflowRuntimeStore::open(temp.path())
        .unwrap()
        .with_worker_lease(stale_lease);
    let mut stale_state = state.clone();
    stale_state.status = RunStatus::Failed;
    let error = stale_store.save_state(&mut stale_state, None).unwrap_err();
    assert!(error.to_string().contains("worker lease lost"));

    let current_store = SqliteWorkflowRuntimeStore::open(temp.path())
        .unwrap()
        .with_worker_lease(current_lease);
    let mut current_state = state;
    current_state.status = RunStatus::Running;
    current_store.save_state(&mut current_state, None).unwrap();
    assert_eq!(
        base_store
            .load_state(&workflow_id, &run_id)
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Running
    );
}

#[test]
fn worker_save_running_and_stopping_keeps_current_lease() {
    for status in [RunStatus::Running, RunStatus::Stopping] {
        let temp = tempfile::tempdir().unwrap();
        let workflow_id = WorkflowId::from(format!("lease-retained-{status:?}"));
        let run_id = RunId::from("run");
        let base_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
        let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
        base_store.create_state(&state).unwrap();
        let now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let lease = base_store
            .acquire_worker_lease(&workflow_id, &run_id, "owner", now_ms, 60_000)
            .unwrap()
            .unwrap();
        let worker_store = SqliteWorkflowRuntimeStore::open(temp.path())
            .unwrap()
            .with_worker_lease(lease.clone());

        state.status = status;
        worker_store.save_state(&mut state, None).unwrap();

        assert_eq!(state.state_revision, 1);
        assert_eq!(
            base_store.load_worker_lease(&workflow_id, &run_id).unwrap(),
            Some(lease)
        );
    }
}

#[test]
fn worker_save_yield_statuses_atomically_release_current_lease() {
    for status in [
        RunStatus::Queued,
        RunStatus::Paused,
        RunStatus::Stopped,
        RunStatus::Succeeded,
        RunStatus::Failed,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let workflow_id = WorkflowId::from(format!("lease-yielded-{status:?}"));
        let run_id = RunId::from("run");
        let base_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
        let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
        base_store.create_state(&state).unwrap();
        let now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let lease = base_store
            .acquire_worker_lease(&workflow_id, &run_id, "owner", now_ms, 60_000)
            .unwrap()
            .unwrap();
        let worker_store = SqliteWorkflowRuntimeStore::open(temp.path())
            .unwrap()
            .with_worker_lease(lease.clone());

        state.status = status;
        worker_store.save_state(&mut state, None).unwrap();

        assert_eq!(state.state_revision, 1);
        assert_eq!(
            base_store
                .load_state(&workflow_id, &run_id)
                .unwrap()
                .unwrap()
                .status,
            status
        );
        assert!(base_store
            .load_worker_lease(&workflow_id, &run_id)
            .unwrap()
            .is_none());
        let connection = rusqlite::Connection::open(temp.path().join("runtime.db")).unwrap();
        let tombstone = connection
            .query_row(
                "SELECT owner_id, generation, expires_at_ms
                 FROM workflow_run_worker_leases
                 WHERE workflow_id = ?1 AND run_id = ?2",
                rusqlite::params![workflow_id.as_str(), run_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(tombstone.0, None);
        assert_eq!(tombstone.1, i64::try_from(lease.generation).unwrap());
        assert_eq!(tombstone.2, 0);
    }
}

#[test]
fn worker_yield_release_trigger_abort_rolls_back_state_revision_events_and_lease() {
    let temp = tempfile::tempdir().unwrap();
    let workflow_id = WorkflowId::from("lease-yield-rollback");
    let run_id = RunId::from("run");
    let base_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let original = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    base_store.create_state(&original).unwrap();
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let lease = base_store
        .acquire_worker_lease(&workflow_id, &run_id, "owner", now_ms, 60_000)
        .unwrap()
        .unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("runtime.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER abort_worker_yield_release
             BEFORE UPDATE OF owner_id ON workflow_run_worker_leases
             WHEN OLD.owner_id IS NOT NULL AND NEW.owner_id IS NULL
             BEGIN
                 SELECT RAISE(ABORT, 'test worker yield release abort');
             END;",
        )
        .unwrap();
    drop(connection);
    let worker_store = SqliteWorkflowRuntimeStore::open(temp.path())
        .unwrap()
        .with_worker_lease(lease.clone());
    let mut yielded = original.clone();
    yielded.status = RunStatus::Paused;
    yielded.events.push("must roll back".to_owned());

    let error = worker_store.save_state(&mut yielded, None).unwrap_err();

    assert!(error
        .to_string()
        .contains("test worker yield release abort"));
    assert_eq!(yielded.state_revision, 0);
    assert_eq!(
        base_store
            .load_state(&workflow_id, &run_id)
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(
        base_store.load_worker_lease(&workflow_id, &run_id).unwrap(),
        Some(lease)
    );
}

#[test]
fn runtime_worker_lease_rejects_non_runnable_runs() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("lease-paused-workflow");
    let run_id = RunId::from("lease-paused-run");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.status = RunStatus::Paused;
    store.create_state(&state).unwrap();

    assert!(store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-a", 3_000, 100)
        .unwrap()
        .is_none());
}

#[test]
fn atomic_resume_claim_transitions_paused_state_and_takes_over_expired_lease() {
    let temp = tempfile::tempdir().unwrap();
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("atomic-resume-workflow");
    let run_id = RunId::from("atomic-resume-run");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.events.push("event before resume".to_owned());
    store.create_state(&state).unwrap();
    let expired = store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-a", 1_000, 100)
        .unwrap()
        .unwrap();
    state.status = RunStatus::Paused;
    state.control = RunControl::Pause;
    state.pause_reason = Some("manual pause".to_owned());
    store.save_state(&mut state, None).unwrap();

    let result = store
        .claim_resume(&workflow_id, &run_id, "owner-b", 1_100, 500)
        .unwrap();
    let WorkflowResumeClaimResult::Claimed {
        state: claimed,
        lease,
    } = result
    else {
        panic!("expected atomic resume claim");
    };
    assert_eq!(lease.owner_id, "owner-b");
    assert_eq!(lease.generation, expired.generation + 1);
    assert_eq!(claimed.status, RunStatus::Queued);
    assert_eq!(claimed.control, RunControl::Continue);
    assert_eq!(claimed.pause_reason, None);
    assert_eq!(claimed.state_revision, state.state_revision + 1);
    assert_eq!(claimed.events, vec!["event before resume"]);

    let persisted = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
    assert_eq!(persisted, claimed);
    assert_eq!(
        store.load_worker_lease(&workflow_id, &run_id).unwrap(),
        Some(lease)
    );
}

#[test]
fn atomic_resume_claim_busy_result_does_not_mutate_paused_state() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("busy-resume-workflow");
    let run_id = RunId::from("busy-resume-run");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    store.create_state(&state).unwrap();
    let active = store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-a", 2_000, 500)
        .unwrap()
        .unwrap();
    state.status = RunStatus::Paused;
    state.control = RunControl::Pause;
    state.pause_reason = Some("must remain paused".to_owned());
    store.save_state(&mut state, None).unwrap();
    let revision = state.state_revision;

    let result = store
        .claim_resume(&workflow_id, &run_id, "owner-b", 2_100, 500)
        .unwrap();
    assert_eq!(
        result,
        WorkflowResumeClaimResult::Busy {
            lease: active.clone()
        }
    );
    let persisted = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
    assert_eq!(persisted.status, RunStatus::Paused);
    assert_eq!(persisted.control, RunControl::Pause);
    assert_eq!(
        persisted.pause_reason.as_deref(),
        Some("must remain paused")
    );
    assert_eq!(persisted.state_revision, revision);
    assert_eq!(
        store.load_worker_lease(&workflow_id, &run_id).unwrap(),
        Some(active)
    );
}

#[test]
fn atomic_resume_claim_rejects_non_resumable_state_without_creating_lease() {
    for status in [
        RunStatus::Stopping,
        RunStatus::Stopped,
        RunStatus::Succeeded,
        RunStatus::Failed,
    ] {
        let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
        let workflow_id = WorkflowId::from(format!("non-resumable-{status:?}"));
        let run_id = RunId::from("run");
        let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
        state.status = status;
        state.control = RunControl::Stop;
        store.create_state(&state).unwrap();

        assert_eq!(
            store
                .claim_resume(&workflow_id, &run_id, "owner", 3_000, 500)
                .unwrap(),
            WorkflowResumeClaimResult::NotResumable { status }
        );
        let persisted = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
        assert_eq!(persisted.status, status);
        assert_eq!(persisted.control, RunControl::Stop);
        assert_eq!(persisted.state_revision, 0);
        assert!(store
            .load_worker_lease(&workflow_id, &run_id)
            .unwrap()
            .is_none());
    }
}

#[test]
fn atomic_resume_claim_reports_missing_run_without_creating_state() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("missing-resume-workflow");
    let run_id = RunId::from("missing-resume-run");
    assert_eq!(
        store
            .claim_resume(&workflow_id, &run_id, "owner", 4_000, 500)
            .unwrap(),
        WorkflowResumeClaimResult::NotFound
    );
    assert!(store.load_state(&workflow_id, &run_id).unwrap().is_none());
}

#[test]
fn atomic_mutation_claim_busy_rolls_back_all_state_changes() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("busy-mutation-workflow");
    let run_id = RunId::from("busy-mutation-run");
    let original = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    store.create_state(&original).unwrap();
    let active = store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-a", 5_000, 500)
        .unwrap()
        .unwrap();

    let result = store
        .mutate_state_and_claim(&workflow_id, &run_id, "owner-b", 5_100, 500, |state| {
            state.status = RunStatus::Running;
            state.pause_reason = Some("must be rolled back".to_owned());
            state.events.push("transient mutation".to_owned());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        result,
        WorkflowMutationClaimResult::Busy {
            lease: active.clone()
        }
    );
    assert_eq!(
        store.load_state(&workflow_id, &run_id).unwrap().unwrap(),
        original
    );
    assert_eq!(
        store.load_worker_lease(&workflow_id, &run_id).unwrap(),
        Some(active)
    );
}

#[test]
fn atomic_mutation_claim_saves_non_runnable_state_without_lease() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let workflow_id = WorkflowId::from("no-lease-mutation-workflow");
    let run_id = RunId::from("no-lease-mutation-run");
    store
        .create_state(&WorkflowRunState::new(workflow_id.clone(), run_id.clone()))
        .unwrap();

    let result = store
        .mutate_state_and_claim(&workflow_id, &run_id, "", 6_000, 0, |state| {
            state.status = RunStatus::Paused;
            state.control = RunControl::Pause;
            state.pause_reason = Some("waiting for confirmation".to_owned());
            state.events.push("pause persisted".to_owned());
            Ok(())
        })
        .unwrap();
    let WorkflowMutationClaimResult::Saved { state, lease } = result else {
        panic!("expected saved mutation without worker lease");
    };
    assert_eq!(lease, None);
    assert_eq!(state.status, RunStatus::Paused);
    assert_eq!(state.control, RunControl::Pause);
    assert_eq!(state.state_revision, 1);
    assert_eq!(
        state.pause_reason.as_deref(),
        Some("waiting for confirmation")
    );
    assert_eq!(state.events, vec!["pause persisted"]);
    assert_eq!(
        store.load_state(&workflow_id, &run_id).unwrap().unwrap(),
        state
    );
    assert!(store
        .load_worker_lease(&workflow_id, &run_id)
        .unwrap()
        .is_none());
}

impl Provider for RecordingLlmProvider {
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: "default-provider".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "default-provider".to_owned(),
            capabilities: vec![ProviderCapability::Llm],
            config_schema: Value::Null,
        }
    }

    fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
        Ok(ProviderHealth::Healthy)
    }
}

impl LlmProvider for RecordingLlmProvider {
    fn complete(
        &self,
        context: &ProviderCallContext,
        request: LlmRequest,
    ) -> ariadne::contracts::CoreResult<LlmResponse> {
        self.contexts.lock().unwrap().push(context.clone());
        self.requests.lock().unwrap().push(request);
        let cost_usd = *self.cost_usd.lock().unwrap();
        Ok(LlmResponse {
            message: LlmMessage::assistant("完成"),
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 2,
            }),
            finish_reason: Some("stop".to_owned()),
            cost_usd,
            raw: Value::Null,
        })
    }
}

#[test]
fn ui_llm_node_uses_prompt_template_and_project_provider_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let provider = RecordingLlmProvider::default();
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf"),
        run_id: RunId::from("run"),
        node_id: NodeId::from("writer"),
        operation_id: "op-writer".to_owned(),
        operation_attempt: 1,
        request_hash: "request-writer".to_owned(),
        type_name: "writer".to_owned(),
        config: json!({
            "schema_version": 1,
            "prompt_template": "你是 Writer 节点",
        }),
        inputs: PortMap::from([(
            "input".to_owned(),
            PortValue::inline(json!("根据本章大纲续写")),
        )]),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: ariadne::contracts::ExecutionCancellation::new(),
        dispatch_authorization: Default::default(),
    };

    let output = execute_llm_node_with_defaults(
        request,
        &provider,
        &ledger,
        Some("default-provider"),
        Some("default-model"),
    )
    .unwrap();

    assert!(format!("{:?}", output.outputs).contains("完成"));
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests[0].model_id, "default-model");
    let prompt = format!("{:?}", requests[0].messages);
    assert!(prompt.contains("你是 Writer 节点"));
    assert!(prompt.contains("根据本章大纲续写"));
}

/// F13：节点 config.timeout_ms / budget_usd 必须进入真实 LLM 调用上下文，不可硬编码 120s 忽略。
#[test]
fn f13_llm_node_timeout_and_budget_enter_provider_context() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let provider = RecordingLlmProvider::default();
    *provider.cost_usd.lock().unwrap() = Some(0.25);
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf-f13"),
        run_id: RunId::from("run-f13"),
        node_id: NodeId::from("writer"),
        operation_id: "op-f13".to_owned(),
        operation_attempt: 1,
        request_hash: "hash-f13".to_owned(),
        type_name: "writer".to_owned(),
        config: json!({
            "schema_version": 1,
            "provider_id": "p",
            "model_id": "m",
            "prompt_template": "写",
            "timeout_ms": 7_500,
            "budget_usd": 1.0,
        }),
        inputs: PortMap::new(),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: ariadne::contracts::ExecutionCancellation::new(),
        dispatch_authorization: Default::default(),
    };

    execute_llm_node(request, &provider, &ledger).unwrap();

    let contexts = provider.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(
        contexts[0].timeout_ms, 7_500,
        "node timeout_ms must drive ProviderCallContext"
    );
    assert_eq!(contexts[0].metadata["node_timeout_ms"], json!(7_500));
    assert_eq!(
        contexts[0].metadata["node_single_call_budget_usd"],
        json!(1.0)
    );
}

/// F13：桌面历史图可能把 timeout/budget 写成 JSON string；执行路径必须仍能进入上下文。
#[test]
fn f13_llm_node_accepts_ui_shaped_string_timeout_and_budget() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let provider = RecordingLlmProvider::default();
    *provider.cost_usd.lock().unwrap() = Some(0.1);
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf-f13-str"),
        run_id: RunId::from("run-f13-str"),
        node_id: NodeId::from("writer"),
        operation_id: "op-f13-str".to_owned(),
        operation_attempt: 1,
        request_hash: "hash-f13-str".to_owned(),
        type_name: "writer".to_owned(),
        // 与桌面 MergeUiFields 旧行为 / 已存 graph JSON 一致：string 形态
        config: json!({
            "schema_version": 1,
            "provider_id": "p",
            "model_id": "m",
            "prompt_template": "写",
            "timeout_ms": "7500",
            "budget_usd": "1.0",
        }),
        inputs: PortMap::new(),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: ariadne::contracts::ExecutionCancellation::new(),
        dispatch_authorization: Default::default(),
    };

    execute_llm_node(request, &provider, &ledger).unwrap();
    let contexts = provider.contexts.lock().unwrap();
    assert_eq!(contexts[0].timeout_ms, 7_500);
    assert_eq!(
        contexts[0].metadata["node_single_call_budget_usd"],
        json!(1.0)
    );
}

/// F13：单次成本超过节点 budget_usd 时节点必须失败，不能当作成功完成。
#[test]
fn f13_llm_node_fails_when_cost_exceeds_single_call_budget() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let provider = RecordingLlmProvider::default();
    *provider.cost_usd.lock().unwrap() = Some(2.5);
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf-f13-budget"),
        run_id: RunId::from("run-f13-budget"),
        node_id: NodeId::from("writer"),
        operation_id: "op-f13-budget".to_owned(),
        operation_attempt: 1,
        request_hash: "hash-f13-budget".to_owned(),
        type_name: "writer".to_owned(),
        config: json!({
            "schema_version": 1,
            "provider_id": "p",
            "model_id": "m",
            "prompt_template": "写",
            "single_call_budget_usd": 1.0,
        }),
        inputs: PortMap::new(),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: ariadne::contracts::ExecutionCancellation::new(),
        dispatch_authorization: Default::default(),
    };

    let err = execute_llm_node(request, &provider, &ledger).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("single-call") || message.contains("budget") || message.contains("1"),
        "expected single-call budget failure, got: {message}"
    );
    assert_eq!(
        provider.contexts.lock().unwrap()[0].timeout_ms,
        120_000,
        "unset timeout still defaults to 120s"
    );
}

#[test]
fn summarizer_reuses_committed_knowledge_receipt_without_provider_call() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let provider = RecordingLlmProvider::default();
    let cancellation = ariadne::contracts::ExecutionCancellation::new();
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf-summary-replay"),
        run_id: RunId::from("run-summary-replay"),
        node_id: NodeId::from("summarizer"),
        operation_id: "knowledge-replay-op".to_owned(),
        operation_attempt: 1,
        request_hash: "knowledge-replay-hash".to_owned(),
        type_name: "summarizer".to_owned(),
        config: json!({
            "provider_id": "unused-provider",
            "model_id": "unused-model",
            "chapter_id": "chapter-1",
            "chapter_document_id": "documents/chapter-1.md",
            "chapter_text_alias": "chapter_text",
            "auto_mode": false
        }),
        inputs: PortMap::from([(
            "chapter_text".to_owned(),
            PortValue::inline(json!("正文不会再次发送给模型")),
        )]),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: cancellation.clone(),
        dispatch_authorization: Default::default(),
    };
    let expected = WorkflowNodeExecutionOutput {
        outputs: PortMap::from([(
            "chapter_id".to_owned(),
            PortValue::inline(json!("chapter-1")),
        )]),
        metadata: json!({"replayed": true}),
        ..WorkflowNodeExecutionOutput::default()
    };
    SqliteWritingKnowledgeStore::open(temp.path())
        .unwrap()
        .save_knowledge_with_operation(
            &MemoryWritingKnowledgeBase::new(),
            &request.operation_id,
            &request.request_hash,
            &serde_json::to_value(&expected).unwrap(),
            &cancellation,
        )
        .unwrap();

    let actual = execute_summarizer_node(request, &provider, &ledger, temp.path()).unwrap();

    assert_eq!(actual, expected);
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[test]
fn summarizer_receipt_replays_dispatched_runtime_operation_without_second_llm_pipeline() {
    struct SequenceSummarizerProvider {
        responses: Mutex<Vec<String>>,
        calls: AtomicUsize,
    }

    impl Provider for SequenceSummarizerProvider {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "summarizer-sequence".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "Summarizer sequence".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }

        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }

    impl LlmProvider for SequenceSummarizerProvider {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            _request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let response = self.responses.lock().unwrap().remove(0);
            Ok(LlmResponse {
                message: LlmMessage::assistant(response),
                tool_calls: Vec::new(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                cost_usd: None,
                raw: Value::Null,
            })
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let provider = Arc::new(SequenceSummarizerProvider {
        responses: Mutex::new(vec![
            json!({
                "segments": [{
                    "number": "1",
                    "summary": "开端",
                    "start_line": 1,
                    "end_line": 2
                }]
            })
            .to_string(),
            json!({
                "events": [{
                    "event_id": "event-1",
                    "summary": "事件概括",
                    "status": "ongoing",
                    "segment_ids": ["chapter-1::seg-1"]
                }]
            })
            .to_string(),
            json!({"summary": "章节总结"}).to_string(),
            json!({
                "stage_id": "stage-1",
                "stage_summary": "阶段总结",
                "is_new_stage": true
            })
            .to_string(),
        ]),
        calls: AtomicUsize::new(0),
    });
    let ledger = Arc::new(SqliteCostLedger::open(temp.path()).unwrap());
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("summarizer-receipt-reconciliation"),
        name: "Summarizer receipt reconciliation".to_owned(),
        nodes: vec![
            NodeInstance {
                id: NodeId::from("start"),
                type_name: "start".to_owned(),
                label: None,
                config: json!({
                    "initial_inputs": {"chapter_text": "第一行\n第二行"}
                }),
                position: None,
            },
            NodeInstance {
                id: NodeId::from("summarizer"),
                type_name: "summarizer".to_owned(),
                label: None,
                config: json!({
                    "provider_id": "summarizer-sequence",
                    "model_id": "test-model",
                    "chapter_id": "chapter-1",
                    "chapter_document_id": "documents/chapter-1.md",
                    "chapter_text_alias": "chapter_text",
                    "auto_mode": true
                }),
                position: None,
            },
        ],
        edges: vec![
            control_edge("start-summary-control", "start", "summarizer"),
            Edge {
                id: EdgeId::from("start-summary-data"),
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("start"),
                    port_name: "chapter_text".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("summarizer"),
                    port_name: "chapter_text".to_owned(),
                },
                alias: Some("chapter_text".to_owned()),
                communication: None,
            },
        ],
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.create_state(&runtime.state).unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("runtime.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER abort_summarizer_operation_complete
             BEFORE UPDATE OF status ON workflow_operations
             WHEN NEW.status = 'completed'
             BEGIN SELECT RAISE(ABORT, 'forced summarizer completion failure'); END;",
        )
        .unwrap();
    let mut external = RoutedExternalNodeExecutor::new();
    let first_provider = Arc::clone(&provider);
    let first_ledger = Arc::clone(&ledger);
    let first_root = temp.path().to_path_buf();
    external
        .register_handler_with_policy(
            "summarizer",
            WorkflowOperationPolicy::replayable_receipt(),
            Box::new(move |request| {
                execute_summarizer_node(
                    request,
                    first_provider.as_ref(),
                    first_ledger.as_ref(),
                    &first_root,
                )
            }),
        )
        .unwrap();
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    let first_error = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap_err();

    assert!(first_error
        .to_string()
        .contains("forced summarizer completion failure"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
    let operation = store
        .list_operations(&workflow.id, &run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(operation.status, WorkflowOperationStatus::Dispatched);
    assert_eq!(
        operation.recovery_policy,
        WorkflowOperationRecoveryPolicy::ReplayExecutor
    );
    assert_eq!(
        operation.response_policy,
        WorkflowOperationResponsePolicy::RequireExecutorReceipt
    );
    assert!(SqliteWritingKnowledgeStore::open(temp.path())
        .unwrap()
        .load_operation_receipt(&operation.operation_id, &operation.request_hash)
        .unwrap()
        .is_some());

    connection
        .execute_batch("DROP TRIGGER abort_summarizer_operation_complete;")
        .unwrap();
    let persisted = store.load_state(&workflow.id, &run_id).unwrap().unwrap();
    let mut recovered = WorkflowRuntime::from_state(persisted);
    let mut replay_external = RoutedExternalNodeExecutor::new();
    let replay_provider = Arc::clone(&provider);
    let replay_ledger = Arc::clone(&ledger);
    let replay_root = temp.path().to_path_buf();
    replay_external
        .register_handler_with_policy(
            "summarizer",
            WorkflowOperationPolicy::replayable_receipt(),
            Box::new(move |request| {
                execute_summarizer_node(
                    request,
                    replay_provider.as_ref(),
                    replay_ledger.as_ref(),
                    &replay_root,
                )
            }),
        )
        .unwrap();
    let mut replay_executor = BuiltinWorkflowNodeExecutor::new(&mut replay_external);

    assert_eq!(
        recovered
            .run_persisted(&workflow, &mut replay_executor, &store)
            .unwrap(),
        RunStatus::Succeeded
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        store
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap()
            .status,
        WorkflowOperationStatus::Committed
    );
    let knowledge = SqliteWritingKnowledgeStore::open(temp.path())
        .unwrap()
        .load_knowledge()
        .unwrap();
    assert_eq!(
        knowledge.chapter_summary("chapter-1").unwrap(),
        Some("章节总结".to_owned())
    );
    assert!(knowledge.has_stage("stage-1").unwrap());
}

#[test]
fn summarizer_unknown_stage_response_pauses_same_parent_operation_without_redispatch() {
    struct UnknownSecondStageProvider {
        calls: AtomicUsize,
    }

    impl Provider for UnknownSecondStageProvider {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "summarizer-unknown-stage".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "Summarizer unknown stage".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }

        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }

    impl LlmProvider for UnknownSecondStageProvider {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            _request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            match self.calls.fetch_add(1, Ordering::SeqCst) {
                0 => Ok(LlmResponse {
                    message: LlmMessage::assistant(
                        json!({
                            "segments": [{
                                "number": "1",
                                "summary": "开端",
                                "start_line": 1,
                                "end_line": 2
                            }]
                        })
                        .to_string(),
                    ),
                    tool_calls: Vec::new(),
                    usage: None,
                    finish_reason: Some("stop".to_owned()),
                    cost_usd: None,
                    raw: Value::Null,
                }),
                1 => Err(ariadne::contracts::CoreError::ProviderRequest {
                    service: "summarizer-unknown-stage".to_owned(),
                    outcome: ariadne::contracts::ExternalDispatchOutcome::DispatchedUnknown,
                    message: "events response was lost after dispatch".to_owned(),
                }),
                call => panic!("summarizer provider must not be redispatched, call {call}"),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let provider = Arc::new(UnknownSecondStageProvider {
        calls: AtomicUsize::new(0),
    });
    let ledger = Arc::new(SqliteCostLedger::open(temp.path()).unwrap());
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("summarizer-unknown-stage-recovery"),
        name: "Summarizer unknown stage recovery".to_owned(),
        nodes: vec![
            NodeInstance {
                id: NodeId::from("start"),
                type_name: "start".to_owned(),
                label: None,
                config: json!({
                    "initial_inputs": {"chapter_text": "第一行\n第二行"}
                }),
                position: None,
            },
            NodeInstance {
                id: NodeId::from("summarizer"),
                type_name: "summarizer".to_owned(),
                label: None,
                config: json!({
                    "provider_id": "summarizer-unknown-stage",
                    "model_id": "test-model",
                    "chapter_id": "chapter-1",
                    "chapter_document_id": "documents/chapter-1.md",
                    "chapter_text_alias": "chapter_text",
                    "auto_mode": true
                }),
                position: None,
            },
        ],
        edges: vec![
            control_edge("start-summary-control", "start", "summarizer"),
            Edge {
                id: EdgeId::from("start-summary-data"),
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("start"),
                    port_name: "chapter_text".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("summarizer"),
                    port_name: "chapter_text".to_owned(),
                },
                alias: Some("chapter_text".to_owned()),
                communication: None,
            },
        ],
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    let runtime_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    runtime_store.create_state(&runtime.state).unwrap();
    let mut external = RoutedExternalNodeExecutor::new();
    let handler_provider = Arc::clone(&provider);
    let handler_ledger = Arc::clone(&ledger);
    let project_root = temp.path().to_path_buf();
    external
        .register_handler_with_policy(
            "summarizer",
            WorkflowOperationPolicy::replayable_receipt(),
            Box::new(move |request| {
                execute_summarizer_node(
                    request,
                    handler_provider.as_ref(),
                    handler_ledger.as_ref(),
                    &project_root,
                )
            }),
        )
        .unwrap();
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external);

    assert_eq!(
        runtime
            .run_persisted(&workflow, &mut executor, &runtime_store)
            .unwrap(),
        RunStatus::Paused
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let operations = runtime_store
        .list_operations(&workflow.id, &run_id)
        .unwrap();
    assert_eq!(operations.len(), 1);
    let parent_operation = &operations[0];
    assert_eq!(parent_operation.status, WorkflowOperationStatus::InDoubt);
    assert_eq!(parent_operation.attempt, 1);
    let knowledge_store = SqliteWritingKnowledgeStore::open(temp.path()).unwrap();
    let first_stages = knowledge_store
        .list_summarizer_stage_operations(&parent_operation.operation_id)
        .unwrap();
    assert_eq!(first_stages.len(), 2);
    let segments = first_stages
        .iter()
        .find(|operation| operation.step == "segments")
        .unwrap();
    let events = first_stages
        .iter()
        .find(|operation| operation.step == "events")
        .unwrap();
    assert_eq!(segments.status, SummarizerStageOperationStatus::Completed);
    assert!(segments.response_json.is_some());
    assert_eq!(events.status, SummarizerStageOperationStatus::InDoubt);
    assert!(events.response_json.is_none());

    runtime.resume().unwrap();
    assert_eq!(
        runtime
            .run_persisted(&workflow, &mut executor, &runtime_store)
            .unwrap(),
        RunStatus::Paused
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let resumed_operations = runtime_store
        .list_operations(&workflow.id, &run_id)
        .unwrap();
    assert_eq!(resumed_operations.len(), 1);
    assert_eq!(
        resumed_operations[0].operation_id,
        parent_operation.operation_id
    );
    assert_eq!(resumed_operations[0].attempt, 1);
    assert_eq!(
        resumed_operations[0].status,
        WorkflowOperationStatus::InDoubt
    );
    assert_eq!(
        knowledge_store
            .list_summarizer_stage_operations(&parent_operation.operation_id)
            .unwrap(),
        first_stages
    );
}

#[test]
fn project_search_node_returns_indexed_project_results() {
    let vector = Arc::new(MemoryVectorStore::new());
    let text = Arc::new(MemoryFullTextStore::new());
    text.upsert(vec![FullTextRecord {
        chunk: ariadne::retrieval::ChunkDocument::new(
            "chunk-1",
            "chapter.md",
            "银色线索出现在旧钟楼",
        ),
    }])
    .unwrap();
    let retrieval = HybridSearchEngine::new(vector, text);
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf"),
        run_id: RunId::from("run"),
        node_id: NodeId::from("search"),
        operation_id: "op-search".to_owned(),
        operation_attempt: 1,
        request_hash: "request-search".to_owned(),
        type_name: "search".to_owned(),
        config: json!({"query_alias": "query", "limit": 5}),
        inputs: PortMap::from([("query".to_owned(), PortValue::inline(json!("线索")))]),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: ariadne::contracts::ExecutionCancellation::new(),
        dispatch_authorization: Default::default(),
    };

    let output = execute_project_search_node_for_test_fixture(request, &retrieval).unwrap();

    let results = output.outputs.get("results").unwrap();
    assert!(format!("{results:?}").contains("chapter.md"));
    assert_eq!(output.metadata["retrieval_scope"], "project");
}

#[test]
fn project_search_node_rejects_product_limit_before_retrieval_dispatch() {
    let retrieval = HybridSearchEngine::new(
        Arc::new(MemoryVectorStore::new()),
        Arc::new(MemoryFullTextStore::new()),
    );
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf"),
        run_id: RunId::from("run"),
        node_id: NodeId::from("search"),
        operation_id: "op-search-limit".to_owned(),
        operation_attempt: 1,
        request_hash: "request-search-limit".to_owned(),
        type_name: "search".to_owned(),
        config: json!({"query_alias": "query", "limit": 51}),
        inputs: PortMap::from([("query".to_owned(), PortValue::inline(json!("线索")))]),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: ariadne::contracts::ExecutionCancellation::new(),
        dispatch_authorization: Default::default(),
    };

    let error = execute_project_search_node_for_test_fixture(request, &retrieval).unwrap_err();

    assert!(error.to_string().contains("project search limit"));
}

#[test]
fn project_search_node_rejects_oversized_inline_results() {
    let text = Arc::new(MemoryFullTextStore::new());
    text.upsert(vec![FullTextRecord {
        chunk: ariadne::retrieval::ChunkDocument::new(
            "oversized",
            "chapter.md",
            format!("线索{}", "文".repeat(128 * 1024)),
        ),
    }])
    .unwrap();
    let retrieval = HybridSearchEngine::new(Arc::new(MemoryVectorStore::new()), text);
    let request = WorkflowNodeExecutionRequest {
        workflow_id: WorkflowId::from("wf"),
        run_id: RunId::from("run"),
        node_id: NodeId::from("search"),
        operation_id: "op-search-budget".to_owned(),
        operation_attempt: 1,
        request_hash: "request-search-budget".to_owned(),
        type_name: "search".to_owned(),
        config: json!({"query_alias": "query", "limit": 1}),
        inputs: PortMap::from([("query".to_owned(), PortValue::inline(json!("线索")))]),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: ariadne::contracts::ExecutionCancellation::new(),
        dispatch_authorization: Default::default(),
    };

    let error = execute_project_search_node_for_test_fixture(request, &retrieval).unwrap_err();

    assert!(error.to_string().contains("exceeding inline budget"));
}

/// 预设节点输出并记录调度请求的测试执行器。
#[derive(Default)]
struct ScriptedExecutor {
    outputs: BTreeMap<String, Vec<WorkflowNodeExecutionOutput>>,
    errors: BTreeMap<String, Vec<ariadne::contracts::CoreError>>,
    calls: Vec<WorkflowNodeExecutionRequest>,
    operation_policy: ariadne::workflow::WorkflowOperationPolicy,
}

impl ScriptedExecutor {
    fn with_operation_policy(
        mut self,
        operation_policy: ariadne::workflow::WorkflowOperationPolicy,
    ) -> Self {
        self.operation_policy = operation_policy;
        self
    }

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
    fn operation_policy(
        &self,
        _request: &WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<ariadne::workflow::WorkflowOperationPolicy> {
        Ok(self.operation_policy)
    }

    /// 按节点 id 返回预设输出，并记录执行请求。
    fn execute(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowNodeExecutionOutput> {
        self.calls.push(request.clone());
        request.dispatch_authorization.authorize_dispatch()?;
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
    fn operation_policy(
        &self,
        request: &WorkflowNodeExecutionRequest,
    ) -> ariadne::contracts::CoreResult<WorkflowOperationPolicy> {
        self.scripted.operation_policy(request)
    }

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

#[test]
fn runtime_schedules_large_chain_with_prebuilt_graph_indexes() {
    const NODE_COUNT: usize = 500;
    let nodes = (0..NODE_COUNT)
        .map(|index| node(&format!("node-{index}"), "test"))
        .collect::<Vec<_>>();
    let edges = (1..NODE_COUNT)
        .map(|index| {
            control_edge(
                &format!("edge-{index}"),
                &format!("node-{}", index - 1),
                &format!("node-{index}"),
            )
        })
        .collect::<Vec<_>>();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("large-chain"),
        name: "Large chain".to_owned(),
        nodes,
        edges,
        metadata: Value::Null,
    };
    let mut executor = ScriptedExecutor::default();
    for index in 0..NODE_COUNT {
        executor.push(
            &format!("node-{index}"),
            WorkflowNodeExecutionOutput::default(),
        );
    }
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("large-run")).unwrap();

    let status = runtime.run(&workflow, &mut executor).unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(executor.calls.len(), NODE_COUNT);
    assert_eq!(executor.calls[0].node_id.as_str(), "node-0");
    assert_eq!(executor.calls[NODE_COUNT - 1].node_id.as_str(), "node-499");
}

/// 验证没有 control 边时，data 边本身也会作为依赖阻塞目标节点。
#[test]
fn runtime_treats_data_edges_as_dependencies_without_control_edge() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Data dependency".to_owned(),
        nodes: vec![node("planner", "planner"), node("writer", "writer")],
        edges: vec![Edge {
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
        }],
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
    assert!(matches!(
        executor.calls[1].inputs.get("outline"),
        Some(PortValue::Inline { value }) if value == &json!("去旧城")
    ));
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

#[test]
fn runtime_rejects_data_edge_alias_missing_before_execution() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Missing alias".to_owned(),
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
                alias: None,
                communication: None,
            },
            control_edge("control-1", "planner", "writer"),
        ],
        metadata: Value::Null,
    };
    let error = match WorkflowRuntime::new(&workflow, RunId::from("run-1")) {
        Ok(_) => panic!("missing data alias should be rejected before execution"),
        Err(error) => error,
    };

    assert!(format!("{error:?}").contains("requires a non-empty alias"));
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

/// 验证恢复路径会完整重置 communication 边，包括 next_sender/hash/message 缓存。
#[test]
fn resume_from_node_resets_communication_runtime_state() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Communication reset".to_owned(),
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

    assert_eq!(
        runtime.run(&workflow, &mut executor).unwrap(),
        RunStatus::Succeeded
    );
    let completed = runtime
        .state
        .communication_edges
        .get(&EdgeId::from("comm-1"))
        .unwrap();
    assert!(completed.completed);
    assert_eq!(completed.next_sender_node_id, NodeId::from("prudent"));
    assert!(completed.last_message_hash.is_some());
    assert!(!completed.messages.is_empty());

    runtime.state.status = RunStatus::Paused;
    runtime.state.control = RunControl::Pause;
    runtime.state.pause_reason = Some("manual resume test".to_owned());
    runtime
        .resume_from_node(&workflow, &NodeId::from("prudent"), PortMap::new())
        .unwrap();

    let reset = runtime
        .state
        .communication_edges
        .get(&EdgeId::from("comm-1"))
        .unwrap();
    assert_eq!(reset.initiator_node_id, NodeId::from("prudent"));
    assert_eq!(reset.next_sender_node_id, NodeId::from("prudent"));
    assert!(!reset.completed);
    assert_eq!(reset.completed_reason, None);
    assert_eq!(reset.pause_reason, None);
    assert_eq!(reset.message_count, 0);
    assert_eq!(reset.last_message_hash, None);
    assert!(reset.messages.is_empty());
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

/// 运行事件使用追加表持久化，主快照不再随事件历史增长而反复重写全部事件。
#[test]
fn runtime_store_persists_events_append_only_outside_main_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-events"),
        name: "Events".to_owned(),
        nodes: vec![node("first", "writer"), node("second", "writer")],
        edges: vec![Edge {
            id: EdgeId::from("control"),
            kind: WorkflowEdgeKind::Control,
            from: PortEndpoint {
                node_id: NodeId::from("first"),
                port_name: EXECUTION_OUTPUT_PORT.to_owned(),
            },
            to: PortEndpoint {
                node_id: NodeId::from("second"),
                port_name: EXECUTION_INPUT_PORT.to_owned(),
            },
            alias: None,
            communication: None,
        }],
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-events")).unwrap();
    let mut executor = ScriptedExecutor::default();
    executor.push("first", WorkflowNodeExecutionOutput::default());
    executor.push("second", WorkflowNodeExecutionOutput::default());
    runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    let connection = rusqlite::Connection::open(temp.path().join("runtime.db")).unwrap();
    let state_json: String = connection
        .query_row("SELECT state_json FROM workflow_runs", [], |row| row.get(0))
        .unwrap();
    let event_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM workflow_run_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(!state_json.contains("structured_events"));
    assert!(!state_json.contains("\"events\""));
    assert_eq!(event_count as usize, runtime.state.structured_events.len());

    store.save_state(&mut runtime.state, None).unwrap();
    let repeated_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM workflow_run_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(repeated_count, event_count);
    let loaded = store
        .load_state(&workflow.id, &RunId::from("run-events"))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.structured_events, runtime.state.structured_events);
}

/// Git 回档停机应在一次批量事务中更新全部非终态快照，并保留终态运行。
#[test]
fn runtime_store_stops_all_non_terminal_runs_for_restore() {
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    for (run_id, status) in [
        ("queued", RunStatus::Queued),
        ("running", RunStatus::Running),
        ("paused", RunStatus::Paused),
        ("succeeded", RunStatus::Succeeded),
    ] {
        let mut state =
            ariadne::workflow::WorkflowRunState::new(WorkflowId::from("wf"), RunId::from(run_id));
        state.status = status;
        store.create_state(&state).unwrap();
    }

    assert_eq!(
        store.stop_non_terminal_for_restore("git restore").unwrap(),
        3
    );
    for run_id in ["queued", "running", "paused"] {
        let state = store
            .load_state(&WorkflowId::from("wf"), &RunId::from(run_id))
            .unwrap()
            .unwrap();
        assert_eq!(state.status, RunStatus::Stopped);
        assert_eq!(state.control, RunControl::Stop);
        assert_eq!(state.stop_reason.as_deref(), Some("git restore"));
    }
    let succeeded = store
        .load_state(&WorkflowId::from("wf"), &RunId::from("succeeded"))
        .unwrap()
        .unwrap();
    assert_eq!(succeeded.status, RunStatus::Succeeded);
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
    assert_eq!(executor.calls[0].operation_attempt, 1);
    assert_eq!(executor.calls[1].operation_attempt, 2);
    assert_eq!(
        executor.calls[0].operation_id,
        ariadne::skills::stable_text_hash("workflow-operation-v1\0wf\0run-1\0writer\x001")
    );
    assert_ne!(
        executor.calls[0].operation_id,
        executor.calls[1].operation_id
    );
    assert!(!executor.calls[0].request_hash.is_empty());
    assert_ne!(
        executor.calls[0].request_hash,
        executor.calls[1].request_hash
    );
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
    let store = SqliteWorkflowRuntimeStore::open_in_memory().unwrap();
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    let mut external = ScriptedExternalExecutor {
        scripted: ScriptedExecutor::default()
            .with_operation_policy(WorkflowOperationPolicy::replayable_receipt()),
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

    let status = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(external.scripted.call_count("writer"), 2);
    let writer_calls = external
        .scripted
        .calls
        .iter()
        .filter(|request| request.node_id == NodeId::from("writer"))
        .collect::<Vec<_>>();
    assert_eq!(writer_calls[0].operation_attempt, 1);
    assert_eq!(writer_calls[1].operation_attempt, 2);
    assert_ne!(writer_calls[0].operation_id, writer_calls[1].operation_id);
    let operations = store
        .list_operations(&workflow.id, &runtime.state.run_id)
        .unwrap();
    assert_eq!(operations.len(), 2);
    assert!(operations
        .iter()
        .all(|operation| operation.status == WorkflowOperationStatus::Committed));
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
        operation_id: "op-export".to_owned(),
        operation_attempt: 1,
        request_hash: "request-export".to_owned(),
        type_name: "export".to_owned(),
        config: Value::Null,
        inputs: ariadne::contracts::PortMap::new(),
        communication_messages: Vec::new(),
        metadata: Value::Null,
        cancellation: ariadne::contracts::ExecutionCancellation::new(),
        dispatch_authorization: Default::default(),
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

#[test]
fn export_operation_recovers_dispatched_artifact_without_duplicate_side_effect() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_document_service(temp_dir.path());
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("export-recovery"),
        name: "Export recovery".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("export"),
            type_name: "export".to_owned(),
            label: None,
            config: json!({
                "artifact_id": "exports/recovered.md",
                "format": "markdown",
                "title": "Recovered",
            }),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let run_id = RunId::from("run-1");
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).unwrap();
    let store = SqliteWorkflowRuntimeStore::open(temp_dir.path()).unwrap();
    store.create_state(&runtime.state).unwrap();
    let connection = rusqlite::Connection::open(temp_dir.path().join("runtime.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER abort_export_operation_complete
             BEFORE UPDATE OF status ON workflow_operations
             WHEN NEW.status = 'completed'
             BEGIN SELECT RAISE(ABORT, 'forced export completion failure'); END;",
        )
        .unwrap();
    let mut external = NoopExternalNodeExecutor::default();
    let mut sink = DocumentWorkflowExportSink::new(&service);
    let mut executor = BuiltinWorkflowNodeExecutor::new(&mut external).with_export_sink(&mut sink);

    let first_error = runtime
        .run_persisted(&workflow, &mut executor, &store)
        .unwrap_err();

    assert!(first_error
        .to_string()
        .contains("forced export completion failure"));
    let operation = store
        .list_operations(&workflow.id, &run_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(operation.status, WorkflowOperationStatus::Dispatched);
    let artifact_path = temp_dir
        .path()
        .join(".runtime/artifacts/exports/recovered.md");
    let first_bytes = fs::read(&artifact_path).unwrap();
    let receipt_root = temp_dir.path().join(".runtime/artifacts/.operations");
    assert_eq!(fs::read_dir(&receipt_root).unwrap().count(), 1);

    connection
        .execute_batch("DROP TRIGGER abort_export_operation_complete;")
        .unwrap();
    let persisted = store.load_state(&workflow.id, &run_id).unwrap().unwrap();
    let mut recovered = WorkflowRuntime::from_state(persisted);
    let mut replay_external = NoopExternalNodeExecutor::default();
    let mut replay_sink = DocumentWorkflowExportSink::new(&service);
    let mut replay_executor =
        BuiltinWorkflowNodeExecutor::new(&mut replay_external).with_export_sink(&mut replay_sink);

    let status = recovered
        .run_persisted(&workflow, &mut replay_executor, &store)
        .unwrap();

    assert_eq!(status, RunStatus::Succeeded);
    assert_eq!(fs::read(&artifact_path).unwrap(), first_bytes);
    assert_eq!(fs::read_dir(&receipt_root).unwrap().count(), 1);
    assert_eq!(
        store
            .load_operation(&operation.operation_id)
            .unwrap()
            .unwrap()
            .status,
        WorkflowOperationStatus::Committed
    );
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
            operation_id: "op-custom".to_owned(),
            operation_attempt: 1,
            request_hash: "request-custom".to_owned(),
            type_name: "custom".to_owned(),
            config: Value::Null,
            inputs: ariadne::contracts::PortMap::new(),
            communication_messages: Vec::new(),
            metadata: Value::Null,
            cancellation: ariadne::contracts::ExecutionCancellation::new(),
            dispatch_authorization: Default::default(),
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
    use ariadne::workflow::{RuntimeConfirmationState, WorkflowRuntime};
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
        use ariadne::workflow::{RuntimeConfirmation, WorkflowNodeRuntimeState};
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

/// F17：项目保存的四项策略在普通模式与 Auto Mode 分别映射到人工、跳过和自动审计。
#[test]
fn f17_summarizer_uses_saved_confirmation_policies_in_both_modes() {
    use ariadne::rag::{ConfirmationKind, ConfirmationState};

    struct PolicyProvider {
        calls: AtomicUsize,
    }
    impl Provider for PolicyProvider {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "policy-provider".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "policy-provider".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for PolicyProvider {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let body = match request.metadata["summarizer_step"].as_str().unwrap() {
                "segments" => {
                    r#"{"segments":[{"number":"1","summary":"段","start_line":1,"end_line":1}]}"#
                }
                "events" => {
                    r#"{"events":[{"event_id":"event-policy","summary":"事件","status":"ongoing","segment_ids":["chapter-policy::seg-1"]}]}"#
                }
                "chapter" => r#"{"summary":"章节总结"}"#,
                "stage" => {
                    r#"{"stage_id":"stage-policy","stage_summary":"阶段总结","is_new_stage":true}"#
                }
                other => panic!("unexpected summarizer step {other}"),
            };
            Ok(LlmResponse {
                message: LlmMessage::assistant(body),
                tool_calls: Vec::new(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                cost_usd: None,
                raw: Value::Null,
            })
        }
    }

    let run_case = |auto_mode: bool| {
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp
            .path()
            .join(".config/confirmation_policy_settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            &settings_path,
            serde_json::to_vec_pretty(&json!([
                {
                    "confirmation_kind": "segment_summary",
                    "normal_policy": "allow_by_default",
                    "auto_mode_policy": "auto_approval"
                },
                {
                    "confirmation_kind": "event_summary",
                    "normal_policy": "manual_review",
                    "auto_mode_policy": "allow_by_default"
                },
                {
                    "confirmation_kind": "chapter_summary",
                    "normal_policy": "allow_by_default",
                    "auto_mode_policy": "allow_by_default"
                },
                {
                    "confirmation_kind": "stage_summary",
                    "normal_policy": "manual_review",
                    "auto_mode_policy": "auto_approval"
                }
            ]))
            .unwrap(),
        )
        .unwrap();
        let provider = PolicyProvider {
            calls: AtomicUsize::new(0),
        };
        let ledger = SqliteCostLedger::open(temp.path()).unwrap();
        let request = WorkflowNodeExecutionRequest {
            workflow_id: WorkflowId::from("wf-policy"),
            run_id: RunId::from(if auto_mode { "run-auto" } else { "run-normal" }),
            node_id: NodeId::from("summarizer"),
            operation_id: format!("op-policy-{auto_mode}"),
            operation_attempt: 1,
            request_hash: format!("hash-policy-{auto_mode}"),
            type_name: "summarizer".to_owned(),
            config: json!({
                "provider_id": "policy-provider",
                "model_id": "m",
                "chapter_id": "chapter-policy",
                "chapter_document_id": "documents/chapter-policy.md",
                "chapter_text_alias": "chapter_text",
                "auto_mode": auto_mode,
            }),
            inputs: PortMap::from([("chapter_text".to_owned(), PortValue::inline(json!("正文")))]),
            communication_messages: Vec::new(),
            metadata: Value::Null,
            cancellation: ariadne::contracts::ExecutionCancellation::new(),
            dispatch_authorization: Default::default(),
        };
        let output = execute_summarizer_node(request, &provider, &ledger, temp.path()).unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
        let knowledge = SqliteWritingKnowledgeStore::open(temp.path())
            .unwrap()
            .load_knowledge()
            .unwrap();
        let states = knowledge
            .confirmations(None)
            .unwrap()
            .into_iter()
            .map(|item| (item.kind, item.state))
            .collect::<BTreeMap<_, _>>();
        (states, output.run_control)
    };

    let (normal, normal_control) = run_case(false);
    assert_eq!(
        normal[&ConfirmationKind::SegmentSummary],
        ConfirmationState::Skipped
    );
    assert_eq!(
        normal[&ConfirmationKind::EventSummary],
        ConfirmationState::Pending
    );
    assert_eq!(
        normal[&ConfirmationKind::ChapterSummary],
        ConfirmationState::Skipped
    );
    assert_eq!(
        normal[&ConfirmationKind::StageSummary],
        ConfirmationState::Pending
    );
    assert_eq!(normal_control, Some(RunControl::Pause));

    let (auto, auto_control) = run_case(true);
    assert_eq!(
        auto[&ConfirmationKind::SegmentSummary],
        ConfirmationState::AutoAudited
    );
    assert_eq!(
        auto[&ConfirmationKind::EventSummary],
        ConfirmationState::Skipped
    );
    assert_eq!(
        auto[&ConfirmationKind::ChapterSummary],
        ConfirmationState::Skipped
    );
    assert_eq!(
        auto[&ConfirmationKind::StageSummary],
        ConfirmationState::AutoAudited
    );
    assert_eq!(auto_control, None);
}

/// F17：策略文件损坏必须在首次 provider dispatch 前阻断。
#[test]
fn f17_malformed_confirmation_policy_blocks_summarizer_before_provider_call() {
    struct MustNotCall(AtomicUsize);
    impl Provider for MustNotCall {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "must-not-call-policy".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "must-not-call-policy".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for MustNotCall {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            _request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("malformed policy must block before provider call")
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let settings_path = temp
        .path()
        .join(".config/confirmation_policy_settings.json");
    std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    std::fs::write(&settings_path, b"[{not-json]").unwrap();
    let provider = MustNotCall(AtomicUsize::new(0));
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let error = execute_summarizer_node(
        WorkflowNodeExecutionRequest {
            workflow_id: WorkflowId::from("wf-bad-policy"),
            run_id: RunId::from("run-bad-policy"),
            node_id: NodeId::from("summarizer"),
            operation_id: "op-bad-policy".to_owned(),
            operation_attempt: 1,
            request_hash: "hash-bad-policy".to_owned(),
            type_name: "summarizer".to_owned(),
            config: json!({
                "provider_id": "must-not-call-policy",
                "model_id": "m",
                "chapter_id": "chapter",
                "chapter_document_id": "documents/chapter.md",
                "chapter_text_alias": "chapter_text",
                "auto_mode": false,
            }),
            inputs: PortMap::from([("chapter_text".to_owned(), PortValue::inline(json!("正文")))]),
            communication_messages: Vec::new(),
            metadata: Value::Null,
            cancellation: ariadne::contracts::ExecutionCancellation::new(),
            dispatch_authorization: Default::default(),
        },
        &provider,
        &ledger,
        temp.path(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("json"));
    assert_eq!(provider.0.load(Ordering::SeqCst), 0);
}

/// F18：相关知识 JSON 损坏不得降级为空上下文并继续调用模型。
#[test]
fn f18_corrupt_knowledge_blocks_summarizer_before_provider_call() {
    struct MustNotCall(AtomicUsize);
    impl Provider for MustNotCall {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "must-not-call-knowledge".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "must-not-call-knowledge".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for MustNotCall {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            _request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("corrupt knowledge must block before provider call")
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(ariadne::rag::StorySegment {
            segment_id: "old::seg-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "old".to_owned(),
            summary: "旧段".to_owned(),
            source: ariadne::contracts::SourceSpan {
                document_id: "documents/old.md".to_owned(),
                range: TextRange { start: 0, end: 3 },
                version: Some("v1".to_owned()),
            },
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_event(ariadne::rag::StoryEvent {
            event_id: "event-corrupt".to_owned(),
            summary: "旧事件".to_owned(),
            status: ariadne::rag::StoryEventStatus::Ongoing,
            segment_ids: vec!["old::seg-1".to_owned()],
            chapter_ids: vec!["old".to_owned()],
            metadata: Value::Null,
        })
        .unwrap();
    SqliteWritingKnowledgeStore::open(temp.path())
        .unwrap()
        .save_knowledge(&knowledge)
        .unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("metadata.db")).unwrap();
    connection
        .execute(
            "UPDATE story_events SET metadata_json = '{broken-json' WHERE event_id = ?1",
            rusqlite::params!["event-corrupt"],
        )
        .unwrap();
    drop(connection);

    let provider = MustNotCall(AtomicUsize::new(0));
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let error = execute_summarizer_node(
        WorkflowNodeExecutionRequest {
            workflow_id: WorkflowId::from("wf-corrupt-knowledge"),
            run_id: RunId::from("run-corrupt-knowledge"),
            node_id: NodeId::from("summarizer"),
            operation_id: "op-corrupt-knowledge".to_owned(),
            operation_attempt: 1,
            request_hash: "hash-corrupt-knowledge".to_owned(),
            type_name: "summarizer".to_owned(),
            config: json!({
                "provider_id": "must-not-call-knowledge",
                "model_id": "m",
                "chapter_id": "chapter",
                "chapter_document_id": "documents/chapter.md",
                "chapter_text_alias": "chapter_text",
                "auto_mode": false,
            }),
            inputs: PortMap::from([("chapter_text".to_owned(), PortValue::inline(json!("正文")))]),
            communication_messages: Vec::new(),
            metadata: Value::Null,
            cancellation: ariadne::contracts::ExecutionCancellation::new(),
            dispatch_authorization: Default::default(),
        },
        &provider,
        &ledger,
        temp.path(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("json"));
    assert_eq!(provider.0.load(Ordering::SeqCst), 0);
}
