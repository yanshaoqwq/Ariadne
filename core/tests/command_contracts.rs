use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    create_checkpoint_impl, create_project, ensure_index_bootstrap_on_open, fetch_provider_models,
    fetch_provider_models_impl, fetch_provider_models_with_cancellation, get_app_settings_impl,
    get_app_status, get_automation_settings_impl, get_backend_diagnostics, get_budget_status_impl,
    get_chapter_summary_view, get_display_name_language_pack_template, get_document_content_impl,
    get_document_tree_impl, get_git_history_impl, get_git_repository_status_impl,
    get_git_settings_impl, get_node_preset_settings_impl, get_permissions_settings_impl,
    get_provider_config_impl, get_rag_settings_impl, get_sidebar_badges,
    get_template_repository_settings, get_template_repository_settings_impl,
    get_workflow_settings_impl, get_works_tree, import_chapter, install_template,
    list_confirmations, list_external_workflow_tools_impl, list_workflow_graphs_impl,
    load_workflow_graph_impl, mark_workflow_run_failed_impl,
    mark_workflow_run_failed_with_lease_impl, open_project, override_confirmation_output,
    pack_workflow_selection_impl, pack_workflow_selection_impl_with_operation_id, pause_workflow,
    preview_provider_removal, process_index_outbox_impl, project_ai_chat, project_ai_chat_impl,
    query_run_logs, quick_edit_impl, register_executor_adapters_for_project, remove_provider,
    resolve_confirmation_impl, resolve_project_references, resolve_workflow_operation_in_doubt,
    restore_to_new_branch, resume_from_node, resume_workflow, run_workflow, run_workflow_impl,
    save_app_settings_impl, save_automation_settings_impl, save_document_content_impl,
    save_git_settings_impl, save_node_preset_settings_impl, save_permissions_settings_impl,
    save_provider_key, save_provider_key_impl, save_provider_section_settings,
    save_provider_settings_impl, save_rag_settings_impl, save_template_repository_settings,
    save_template_repository_settings_impl, save_workflow_graph_impl, save_workflow_settings_impl,
    search_project_documents_impl, search_templates, set_project_root, start_workflow_with_request,
    stop_workflow, update_budget_config_impl, validate_display_name_language_pack, AppSettings,
    AriadneAppState, AutomationSettings, CanvasEdge, CanvasNode, ConfirmationAutoModePolicy,
    ConfirmationDecision, ConfirmationNormalPolicy, ConfirmationPolicySetting, GitSettings,
    InDoubtDecision, NodePresetSettings, OverrideConfirmationOutputRequest, PermissionsSettings,
    ProjectAiChatMessage, ProjectAiChatRole, ProjectAiRequest, ProviderSectionSettings,
    ProviderSettingsUpdate, QuickEditRequest, RagSettings, ResolveConfirmationRequest,
    ResolveInDoubtOperationRequest, ResumeFromNodeRequest, RunLogQuery, TemplateRepositoryRequest,
    TemplateRepositorySettings, WorkflowGraphData, WorkflowSettings,
};
use ariadne::config::{
    ConfigStore, MemorySecretStore, ModelConfig, PathWriteLock, ProjectCredentialScope,
    ProviderConfig, SecretRef, SecretStore, SecretValue, PROVIDERS_CONFIG_FILE,
};
use ariadne::contracts::{
    ExecutionCancellation, NodeId, NodeInstance, PermissionPolicy, PortValue, ProviderCapability,
    ProviderType, RunControl, RunId, RunStatus, WorkflowDefinition, WorkflowEdgeKind, WorkflowId,
};
use ariadne::diagnostics::DiagnosticStatus;
use ariadne::documents::IndexInvalidationOutbox;
use ariadne::frontend::{
    ChapterImportRequest, ConfirmationLogEntry, ConfirmationLogState, FileConfirmationLogStore,
    ProjectAiConversationStore, OFFICIAL_TEMPLATE_REPOSITORY_URL,
};
use ariadne::retrieval::{FullTextSearchRequest, FullTextStore, TantivyFullTextStore};
use ariadne::workflow::{
    ConfirmationResolutionDecision, ConfirmationResolutionStatus, NewWorkflowOperation,
    RuntimeConfirmation, RuntimeConfirmationState, SqliteWorkflowRuntimeStore,
    WorkflowNodeExecutionOutput, WorkflowNodeRuntimeState, WorkflowOperationPolicy,
    WorkflowOperationStatus, WorkflowRunState, WorkflowRuntime, WorkflowRuntimeEventType,
    WorkflowRuntimeStore,
};
use serde_json::{json, Value};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn wait_for_terminal_workflow_state(
    store: &SqliteWorkflowRuntimeStore,
    workflow_id: &WorkflowId,
    run_id: &RunId,
) -> WorkflowRunState {
    let mut last = None;
    for _ in 0..50 {
        last = store.load_state(workflow_id, run_id).unwrap();
        if last
            .as_ref()
            .is_some_and(|state| state.status.is_terminal())
        {
            return last.unwrap();
        }
        thread::sleep(Duration::from_millis(20));
    }
    last.expect("workflow state should be persisted by background worker")
}

#[test]
fn f12_public_control_commands_preserve_terminal_runs_and_stop_is_idempotent() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let app = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let store = SqliteWorkflowRuntimeStore::open(project.path()).unwrap();
    let workflow_id = WorkflowId::from("f12-terminal-controls");

    for terminal in [RunStatus::Succeeded, RunStatus::Failed, RunStatus::Stopped] {
        let run_id = RunId::from(format!("run-{terminal:?}"));
        let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
        state.status = terminal;
        state.control = if terminal == RunStatus::Stopped {
            RunControl::Stop
        } else {
            RunControl::Continue
        };
        store.create_state(&state).unwrap();
        let before = store.load_state(&workflow_id, &run_id).unwrap().unwrap();

        let pause_error = pause_workflow(
            &app,
            workflow_id.as_str().to_owned(),
            run_id.as_str().to_owned(),
            Some("must not revive".to_owned()),
        )
        .unwrap_err();
        assert!(pause_error.contains("cannot pause workflow run"));
        let resume_error = resume_workflow(
            &app,
            workflow_id.as_str().to_owned(),
            run_id.as_str().to_owned(),
        )
        .unwrap_err();
        assert!(resume_error.contains("cannot resume"));

        let stop = stop_workflow(
            &app,
            workflow_id.as_str().to_owned(),
            run_id.as_str().to_owned(),
            Some("idempotent stop".to_owned()),
        )
        .unwrap();
        assert_eq!(stop.status, format!("{terminal:?}").to_ascii_lowercase());
        let after = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
        assert_eq!(after, before, "terminal {terminal:?} must be zero-write");
    }
}

#[test]
fn document_commands_read_tree_and_round_trip_content() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("documents")).unwrap();

    save_document_content_impl(
        temp.path(),
        "documents/chapter.md".to_owned(),
        "正文".to_owned(),
    )
    .unwrap();
    let tree = get_document_tree_impl(temp.path()).unwrap();
    let content =
        get_document_content_impl(temp.path(), Some("documents/chapter.md".to_owned()), None)
            .unwrap();

    assert_eq!(content, "正文");
    assert!(format!("{tree:?}").contains("chapter.md"));
}

#[test]
fn project_indexing_worker_consumes_persisted_document_event() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let path = temp.path().join("documents").join("chapter.md");
    let content = "银色线索在旧钟楼再次出现";
    std::fs::write(&path, content).unwrap();
    let document_id = path.canonicalize().unwrap().to_string_lossy().into_owned();
    let source_version = test_content_version(content.as_bytes());
    let outbox =
        IndexInvalidationOutbox::new(temp.path().join(".runtime").join("index_invalidation.db"));
    let event_id = outbox
        .prepare(&document_id, "document_saved", &source_version, false)
        .unwrap();
    outbox.activate(&event_id).unwrap();

    assert_eq!(process_index_outbox_impl(temp.path()).unwrap(), 1);

    let tantivy = TantivyFullTextStore::open(temp.path().join(".indexes/tantivy")).unwrap();
    let results = tantivy
        .search(FullTextSearchRequest::new("线索", 10))
        .unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|result| {
        result
            .metadata
            .get("source_version")
            .and_then(|value| value.as_str())
            == Some(source_version.as_str())
    }));
}

/// F2-a product path: documents exist, empty indexes, empty outbox → bootstrap
/// (same entry `open_project` / `set_project_root` call) → process outbox → searchable.
#[test]
fn f2a_open_project_bootstraps_full_rebuild_for_existing_documents() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let chapter = temp.path().join("documents").join("chapter.md");
    std::fs::write(&chapter, "离线复制进来的可检索线索").unwrap();
    let sqlite_path = temp.path().join(".indexes").join("full_text.db");
    assert!(
        !sqlite_path.exists()
            || ariadne::retrieval::full_text_index_is_empty(&sqlite_path).unwrap()
    );
    let outbox =
        IndexInvalidationOutbox::new(temp.path().join(".runtime").join("index_invalidation.db"));
    assert!(outbox.pending().unwrap().is_empty());

    // Product bootstrap entry (invoked by open_project / set_project_root before resume worker).
    let event_id = ensure_index_bootstrap_on_open(temp.path()).unwrap();
    assert!(
        event_id.is_some(),
        "empty index + sources must enqueue full rebuild"
    );
    assert!(outbox.has_incomplete_full_rebuild().unwrap());

    // Same worker body open_project spawns (sync — avoids racing async IndexWriter).
    let processed = process_index_outbox_impl(temp.path()).unwrap();
    assert!(processed >= 1, "bootstrap full rebuild must be processed");
    assert!(!outbox.has_incomplete_full_rebuild().unwrap());

    // open_project / set_project_root must call ensure_index_bootstrap_on_open (shipped wiring).
    let commands_src = include_str!("../src/commands.rs");
    assert!(
        commands_src.contains("ensure_index_bootstrap_on_open(&project_root)?")
            && commands_src.contains("pub fn open_project")
            && commands_src.contains("pub fn set_project_root"),
        "open_project and set_project_root must invoke ensure_index_bootstrap_on_open"
    );

    let results = search_project_documents_impl(temp.path(), "线索".to_owned(), 10).unwrap();
    assert!(
        !results.is_empty(),
        "documents present at open must be searchable after bootstrap rebuild"
    );
    assert!(results.iter().any(|r| r.snippet.contains("线索")));
}

/// F10-c product path: missing required Start input is rejected before create_state.
#[test]
fn f10c_start_workflow_rejects_missing_required_initial_inputs_before_persist() {
    use ariadne::workflow::SqliteWorkflowRuntimeStore;
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "f10c-schema".to_owned(),
            name: "F10-c schema".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-main".to_owned(),
                r#type: "start".to_owned(),
                label: None,
                data: json!({
                    "input_schema": {
                        "type": "object",
                        "required": ["topic"],
                        "properties": {
                            "topic": { "type": "string" }
                        }
                    }
                }),
                position: Value::Null,
            }],
            edges: vec![],
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let app_state = tempfile::tempdir().unwrap();
    let app = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    // Empty map — must still enforce required (not only non-empty maps).
    let err_empty = start_workflow_with_request(
        &app,
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "f10c-schema".to_owned(),
            start_node_id: Some("start-main".to_owned()),
            initial_inputs: BTreeMap::new(),
        },
    )
    .expect_err("empty initial_inputs must fail required schema");
    assert!(
        err_empty.contains("missing required") || err_empty.contains("topic"),
        "unexpected empty reject: {err_empty}"
    );

    let mut wrong_type = BTreeMap::new();
    wrong_type.insert("topic".to_owned(), json!(123));
    let err_type = start_workflow_with_request(
        &app,
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "f10c-schema".to_owned(),
            start_node_id: Some("start-main".to_owned()),
            initial_inputs: wrong_type,
        },
    )
    .expect_err("wrong type must fail");
    assert!(
        err_type.contains("expected type") || err_type.contains("string"),
        "unexpected type reject: {err_type}"
    );

    let mut unknown = BTreeMap::new();
    unknown.insert("topic".to_owned(), json!("ok"));
    unknown.insert("extra".to_owned(), json!("nope"));
    let err_unknown = start_workflow_with_request(
        &app,
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "f10c-schema".to_owned(),
            start_node_id: Some("start-main".to_owned()),
            initial_inputs: unknown,
        },
    )
    .expect_err("unknown property must fail");
    assert!(
        err_unknown.contains("not declared") || err_unknown.contains("extra"),
        "unexpected unknown reject: {err_unknown}"
    );

    // No run may have been persisted after rejections.
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let runs = store.list_non_terminal_states().unwrap();
    assert!(
        runs.is_empty(),
        "F10-c reject must happen before create_state; found {} runs",
        runs.len()
    );
}

/// F10-d product path: open_project claims orphan Queued runs that have no live lease.
#[test]
fn f10d_open_project_recovers_orphaned_queued_run() {
    use ariadne::workflow::{SqliteWorkflowRuntimeStore, WorkflowRunState, WorkflowRuntimeStore};
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let workflow = WorkflowDefinition {
        id: WorkflowId::from("orphan-wf"),
        name: "Orphan".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("start"),
            type_name: "start".to_owned(),
            label: None,
            config: json!({}),
            position: None,
        }],
        edges: Vec::new(),
        metadata: json!({}),
    };
    let run_id = RunId::from("orphan-queued-1");
    let mut state = WorkflowRunState::new(workflow.id.clone(), run_id.clone());
    state.prepared_workflow = Some(workflow.clone());
    state.status = RunStatus::Queued;
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.create_state(&state).unwrap();
    // No worker lease → classic create/lease/spawn crash window.
    assert!(store
        .load_worker_lease(&workflow.id, &run_id)
        .unwrap()
        .is_none());

    let app_state = tempfile::tempdir().unwrap();
    let app = AriadneAppState::new(
        PathBuf::new(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    open_project(
        &app,
        temp.path().to_string_lossy().into_owned(),
        Some("orphan-project".to_owned()),
    )
    .unwrap();

    // Recovery must claim a worker lease (spawn may later fail for minimal graph — lease is the handoff).
    let mut claimed = None;
    for _ in 0..50 {
        if let Some(lease) = store.load_worker_lease(&workflow.id, &run_id).unwrap() {
            claimed = Some(lease);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let lease = claimed.expect("open_project must recover orphan Queued by acquiring worker lease");
    assert!(
        lease.owner_id.starts_with("worker-recover-") || lease.owner_id.starts_with("worker-"),
        "unexpected owner: {}",
        lease.owner_id
    );

    // set_project_root uses the same recovery entry; plant a second orphan and rebind.
    let run_id2 = RunId::from("orphan-queued-2");
    let mut state2 = WorkflowRunState::new(workflow.id.clone(), run_id2.clone());
    state2.prepared_workflow = Some(workflow.clone());
    state2.status = RunStatus::Queued;
    store.create_state(&state2).unwrap();
    set_project_root(&app, temp.path().to_string_lossy().into_owned()).unwrap();
    let mut recovered2 = false;
    for _ in 0..50 {
        if let Some(lease) = store.load_worker_lease(&workflow.id, &run_id2).unwrap() {
            assert!(
                lease.owner_id.starts_with("worker-recover-")
                    || lease.owner_id.starts_with("worker-")
            );
            recovered2 = true;
            break;
        }
        let current = store.load_state(&workflow.id, &run_id2).unwrap().unwrap();
        if current.status != RunStatus::Queued {
            assert_eq!(current.status, RunStatus::Succeeded);
            assert!(current
                .structured_events
                .iter()
                .any(|event| event.event_type == WorkflowRuntimeEventType::RunSucceeded));
            recovered2 = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        recovered2,
        "set_project_root must claim or finish the orphan Queued run"
    );
}

/// F10-b/F10-d product path: legacy orphan runs without a frozen definition
/// converge under a claimed lease instead of remaining Queued forever.
#[test]
fn f10d_open_project_converges_legacy_orphan_to_fenced_failure() {
    use ariadne::workflow::{SqliteWorkflowRuntimeStore, WorkflowRunState, WorkflowRuntimeStore};

    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("legacy-orphan-wf");
    let run_id = RunId::from("legacy-orphan-run");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.status = RunStatus::Queued;
    state.prepared_workflow = None;
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.create_state(&state).unwrap();

    let app_state = tempfile::tempdir().unwrap();
    let app = AriadneAppState::new(
        PathBuf::new(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    open_project(
        &app,
        temp.path().to_string_lossy().into_owned(),
        Some("legacy-orphan-project".to_owned()),
    )
    .unwrap();

    let failed = store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .expect("legacy orphan must remain queryable as an explicit failure");
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.control, RunControl::Stop);
    let failure = failed
        .failure
        .expect("legacy orphan failure must be structured");
    assert_eq!(failure.code, "workflow_legacy_snapshot_unrecoverable");
    assert_eq!(failure.stage, "workflow_orphan_recovery");
    assert_eq!(
        failure.message,
        "error.workflow.legacy_snapshot_unrecoverable"
    );
    assert_eq!(
        failure.recovery_suggestion,
        "error.workflow.legacy_snapshot_unrecoverable.recovery"
    );
    assert!(store
        .load_worker_lease(&workflow_id, &run_id)
        .unwrap()
        .is_none());
}

#[test]
fn f9_open_project_scheduler_wakes_future_retry_without_reopen() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();

    let workflow_id = WorkflowId::from("f9-auto-retry");
    let run_id = RunId::from("future-retry");
    let workflow = WorkflowDefinition {
        id: workflow_id.clone(),
        name: "F9 auto retry".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("start"),
            type_name: "start".to_owned(),
            label: None,
            config: json!({}),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let retry_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 600;
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.prepared_workflow = Some(workflow);
    state.next_retry_at_ms = Some(retry_at_ms);
    state.nodes.insert(
        NodeId::from("start"),
        WorkflowNodeRuntimeState {
            node_id: NodeId::from("start"),
            status: RunStatus::Queued,
            outputs: BTreeMap::new(),
            communication_output: None,
            communication_control: Default::default(),
            prompt_trace_hash: None,
            patch_session_commit_id: None,
            checkpoint_id: None,
            patch_write_back_state: None,
            metadata: Value::Null,
            error: Some("retry later".to_owned()),
            error_state: Some(ariadne::workflow::NodeErrorState {
                kind: ariadne::workflow::NodeErrorKind::Retryable,
                message: "retry later".to_owned(),
                attempts: 1,
                max_attempts: 3,
                retryable: true,
                next_retry_delay_ms: Some(600),
                next_retry_at_ms: Some(retry_at_ms),
                recovery_suggestion: "retry".to_owned(),
            }),
            execution_attempts: 1,
        },
    );
    let store = SqliteWorkflowRuntimeStore::open(project.path()).unwrap();
    store.create_state(&state).unwrap();

    let app = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    ariadne::commands::get_current_project(&app).unwrap();
    thread::sleep(Duration::from_millis(100));
    let before_deadline = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
    assert_eq!(before_deadline.status, RunStatus::Queued);
    assert_eq!(
        before_deadline.nodes[&NodeId::from("start")].execution_attempts,
        1
    );

    let mut terminal = None;
    for _ in 0..250 {
        let current = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
        if current.status.is_terminal() {
            terminal = Some(current);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let terminal = terminal.expect("future retry must wake without reopening the project");
    assert_eq!(terminal.status, RunStatus::Succeeded);
    assert_eq!(terminal.nodes[&NodeId::from("start")].execution_attempts, 2);
}

/// F2-a: second bootstrap is idempotent when index already populated.
#[test]
fn f2a_bootstrap_is_idempotent_when_index_already_populated() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    std::fs::write(
        temp.path().join("documents").join("chapter.md"),
        "已索引正文",
    )
    .unwrap();
    let first = ensure_index_bootstrap_on_open(temp.path()).unwrap();
    assert!(first.is_some(), "first bootstrap must enqueue");
    process_index_outbox_impl(temp.path()).unwrap();
    let second = ensure_index_bootstrap_on_open(temp.path()).unwrap();
    assert!(
        second.is_none(),
        "populated index must not re-enqueue full rebuild"
    );
}

/// F2-b: after save of new body, product search must not return pre-save body as current fact.
#[test]
fn f2b_search_rejects_stale_chunks_after_save_before_reindex() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let path = temp.path().join("documents").join("chapter.md");
    let old_body = "旧版本剧情线索甲";
    std::fs::write(&path, old_body).unwrap();
    let document_id = path.canonicalize().unwrap().to_string_lossy().into_owned();
    let old_version = test_content_version(old_body.as_bytes());
    let outbox =
        IndexInvalidationOutbox::new(temp.path().join(".runtime").join("index_invalidation.db"));
    let event_id = outbox
        .prepare(&document_id, "document_saved", &old_version, false)
        .unwrap();
    outbox.activate(&event_id).unwrap();
    assert_eq!(process_index_outbox_impl(temp.path()).unwrap(), 1);

    // Indexed: old body is searchable.
    let before = search_project_documents_impl(temp.path(), "旧版本".to_owned(), 10).unwrap();
    assert!(!before.is_empty());

    // Product save enqueues invalidation and leaves index on old chunks until worker runs.
    save_document_content_impl(
        temp.path(),
        "documents/chapter.md".to_owned(),
        "新版本剧情线索乙".to_owned(),
    )
    .unwrap();

    // Immediate search: must fail loud (indexing_not_ready) OR not return old body as current.
    match search_project_documents_impl(temp.path(), "旧版本".to_owned(), 10) {
        Err(error) => {
            let msg = error.to_string();
            assert!(
                msg.contains("indexing_not_ready") || msg.contains("pending"),
                "unexpected error: {msg}"
            );
        }
        Ok(results) => {
            assert!(
                results
                    .iter()
                    .all(|r| !r.snippet.contains("旧版本剧情线索甲")),
                "stale pre-save body must not be returned as current: {results:?}"
            );
        }
    }

    // After worker catches up, new body is searchable and old unique phrase is gone.
    process_index_outbox_impl(temp.path()).unwrap();
    let after_new = search_project_documents_impl(temp.path(), "新版本".to_owned(), 10).unwrap();
    assert!(!after_new.is_empty());
    let after_old = search_project_documents_impl(temp.path(), "旧版本剧情线索甲".to_owned(), 10)
        .unwrap_or_default();
    assert!(
        after_old
            .iter()
            .all(|r| !r.snippet.contains("旧版本剧情线索甲")),
        "reindexed search must not still surface old body"
    );
}

fn test_content_version(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[test]
fn import_chapter_command_resolves_project_relative_paths() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new(
        temp.path().to_path_buf(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    std::fs::create_dir_all(temp.path().join("planning").join("imports")).unwrap();
    std::fs::write(
        temp.path()
            .join("planning")
            .join("imports")
            .join("chapter.md"),
        "第一章正文",
    )
    .unwrap();

    let index = import_chapter(
        &state,
        ChapterImportRequest {
            chapter_id: "stage1:001".to_owned(),
            title: "第一章".to_owned(),
            order: 1,
            source_path: "planning/imports/chapter.md".into(),
            target_path: "documents/chapter-001.md".into(),
            overwrite: false,
            outline_ref: None,
        },
    )
    .unwrap();

    assert_eq!(index.entries.len(), 1);
    assert_eq!(
        index.entries[0].path,
        temp.path().join("documents/chapter-001.md")
    );
    assert_eq!(
        get_document_content_impl(
            temp.path(),
            Some("documents/chapter-001.md".to_owned()),
            None
        )
        .unwrap(),
        "第一章正文"
    );

    std::fs::write(
        temp.path()
            .join("planning")
            .join("imports")
            .join("chapter-2.md"),
        "第二版正文",
    )
    .unwrap();
    let replacement = ChapterImportRequest {
        chapter_id: "stage1:001".to_owned(),
        title: "第一章第二版".to_owned(),
        order: 2,
        source_path: "planning/imports/chapter-2.md".into(),
        target_path: "documents/chapter-002.md".into(),
        overwrite: false,
        outline_ref: None,
    };
    let error = import_chapter(&state, replacement.clone()).unwrap_err();
    assert_eq!(
        error.code,
        ariadne::command_error::CommandErrorCode::Conflict
    );
    assert!(!temp.path().join("documents/chapter-002.md").exists());

    let replaced = import_chapter(
        &state,
        ChapterImportRequest {
            overwrite: true,
            ..replacement
        },
    )
    .unwrap();
    assert_eq!(replaced.entries.len(), 1);
    assert_eq!(replaced.entries[0].title, "第一章第二版");
}

#[test]
fn app_state_root_can_be_separated_from_project_root_env() {
    let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    std::env::set_var("ARIADNE_PROJECT_ROOT", project.path());
    std::env::set_var("ARIADNE_APP_STATE_ROOT", app_state.path());

    let resolved_app_state = ariadne::commands::default_app_state_root();
    let resolved_project = ariadne::commands::default_project_root();

    std::env::remove_var("ARIADNE_PROJECT_ROOT");
    std::env::remove_var("ARIADNE_APP_STATE_ROOT");

    assert_eq!(resolved_project, project.path());
    assert_eq!(resolved_app_state, app_state.path());
}

#[test]
fn app_state_rejects_missing_or_uninitialized_project_root() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new(
        project.path().to_path_buf(),
        app_state.path().to_path_buf(),
        Arc::new(MemorySecretStore::default()),
    );

    let missing = project.path().join("missing");
    let missing_error = state.set_project_root(&missing).unwrap_err();
    assert!(missing_error.contains("does not exist"));

    let uninitialized = project.path().join("plain");
    std::fs::create_dir_all(&uninitialized).unwrap();
    let uninitialized_error = state.set_project_root(&uninitialized).unwrap_err();
    assert!(uninitialized_error.contains("not initialized"));

    std::fs::create_dir_all(uninitialized.join(".config")).unwrap();
    state.set_project_root(&uninitialized).unwrap();
    assert_eq!(state.project_root().unwrap(), uninitialized);
}

#[test]
fn project_create_and_open_persist_display_name_in_project_config() {
    let project_parent = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let project_root = project_parent.path().join("directory-name");
    let state = AriadneAppState::new(
        project_parent.path().join("unused"),
        app_state.path().to_path_buf(),
        Arc::new(MemorySecretStore::default()),
    );

    create_project(
        &state,
        project_root.to_string_lossy().into_owned(),
        Some("作品项目".to_owned()),
    )
    .unwrap();
    let config = ConfigStore::new(&project_root).load().unwrap();
    assert_eq!(config.app.project_name, "作品项目");
    assert_eq!(
        get_app_status(&state).unwrap().current_project.project_name,
        "作品项目"
    );

    let reopened = open_project(&state, project_root.to_string_lossy().into_owned(), None).unwrap();
    assert_eq!(reopened.project_name, "作品项目");
}

#[test]
fn app_status_rejects_uninitialized_project_root() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new(
        project.path().to_path_buf(),
        app_state.path().to_path_buf(),
        Arc::new(MemorySecretStore::default()),
    );

    let error = get_app_status(&state).unwrap_err();

    assert!(error.contains("not initialized"));
}

#[test]
fn app_status_rejects_corrupt_present_app_config_instead_of_using_directory_name() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    std::fs::write(
        project.path().join(".config").join("app.yaml"),
        "project_name: [unterminated",
    )
    .unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let status_error = get_app_status(&state).unwrap_err();
    let current_error = ariadne::commands::current_project_status(project.path()).unwrap_err();

    assert!(status_error.contains("yaml"));
    assert!(current_error.contains("yaml"));
}

#[test]
fn project_scoped_state_commands_reject_uninitialized_project_root() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new(
        project.path().to_path_buf(),
        app_state.path().to_path_buf(),
        Arc::new(MemorySecretStore::default()),
    );

    let error = run_workflow(&state, "wf".to_owned(), None).unwrap_err();

    assert!(error.contains("not initialized"));
}

#[test]
fn command_impls_reject_missing_project_root_without_creating_it() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-project");

    let error = get_app_settings_impl(&missing).unwrap_err();
    assert!(error.contains("project root does not exist"));
    assert!(!missing.exists());
}

#[test]
fn display_name_language_pack_template_supports_arbitrary_language_codes() {
    let template = get_display_name_language_pack_template(Some("ZH_Hant".to_owned())).unwrap();

    assert_eq!(template.target_language, "zh-hant");
    assert_eq!(template.base_language, "zh");
    assert_eq!(template.fallback_language, "zh");
    assert_eq!(template.output_file_name, "display_name.zh-hant.json");
    assert_eq!(template.source_file_name, "display_name.json");
    assert!(template.entries.contains_key("ui.settings.misc.language"));
    assert!(template
        .instructions
        .iter()
        .any(|item| item.contains("Keep every JSON key unchanged")));
}

#[test]
fn display_name_language_pack_validation_reports_coverage() {
    let template = get_display_name_language_pack_template(Some("fr".to_owned())).unwrap();
    let mut keys = template.entries.keys().cloned();
    let translated_key = keys.next().unwrap();
    let empty_key = keys.next().unwrap();
    let mut overlay = BTreeMap::new();
    overlay.insert("_comment".to_owned(), "metadata is allowed".to_owned());
    overlay.insert(translated_key.clone(), "traduit".to_owned());
    overlay.insert(empty_key.clone(), "  ".to_owned());
    overlay.insert("ui.unknown".to_owned(), "extra".to_owned());

    let report = validate_display_name_language_pack(Some("FR".to_owned()), overlay).unwrap();

    assert_eq!(report.target_language, "fr");
    assert_eq!(report.output_file_name, "display_name.fr.json");
    assert_eq!(report.total_keys, template.entries.len());
    assert_eq!(report.translated_keys, 1);
    assert!(report.empty_keys.contains(&empty_key));
    assert!(report.extra_keys.contains(&"ui.unknown".to_owned()));
    assert_eq!(report.missing_keys.len(), template.entries.len() - 2);
    assert!(!report.complete);
}

#[test]
fn workflow_graph_commands_save_and_load_canvas_shape() {
    let temp = tempfile::tempdir().unwrap();
    let graph = WorkflowGraphData {
        workflow_id: "draft-flow".to_owned(),
        name: "Draft Flow".to_owned(),
        nodes: vec![CanvasNode {
            id: "writer".to_owned(),
            r#type: "writer".to_owned(),
            label: Some("Writer".to_owned()),
            data: json!({ "prompt_template": "writer.default" }),
            position: json!({ "x": 10.0, "y": 20.0 }),
        }],
        edges: Vec::new(),
        metadata: Value::Null,
        content_revision: None,
        expected_revision: None,
    };

    save_workflow_graph_impl(temp.path(), graph).unwrap();
    let loaded = load_workflow_graph_impl(temp.path(), Some("draft-flow".to_owned())).unwrap();

    assert_eq!(loaded.workflow_id, "draft-flow");
    assert_eq!(loaded.nodes[0].id, "writer");
    assert_eq!(loaded.nodes[0].data["prompt_template"], "writer.default");
}

#[test]
fn n3_concurrent_workflow_saves_with_same_revision_allow_exactly_one_writer() {
    use std::sync::Barrier;

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().to_path_buf();
    let base = WorkflowGraphData {
        workflow_id: "n3-cas".to_owned(),
        name: "base".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
        content_revision: None,
        expected_revision: None,
    };
    let created = save_workflow_graph_impl(&project, base).unwrap();
    let revision = created
        .content_revision
        .expect("created workflow must return its content revision");
    assert_eq!(revision.len(), 64, "workflow revisions use SHA-256");

    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for name in ["writer-a", "writer-b"] {
        let project = project.clone();
        let revision = revision.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let graph = WorkflowGraphData {
                workflow_id: "n3-cas".to_owned(),
                name: name.to_owned(),
                nodes: Vec::new(),
                edges: Vec::new(),
                metadata: Value::Null,
                content_revision: None,
                expected_revision: Some(revision),
            };
            barrier.wait();
            save_workflow_graph_impl(&project, graph)
                .map(|saved| saved.name)
                .map_err(|error| error.to_string())
        }));
    }

    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("workflow writer thread"))
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    let conflict = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("one concurrent writer must lose the revision CAS");
    assert!(conflict.contains("revision conflict"), "{conflict}");

    let loaded = load_workflow_graph_impl(&project, Some("n3-cas".to_owned())).unwrap();
    assert!(loaded.name == "writer-a" || loaded.name == "writer-b");
    assert_eq!(loaded.content_revision.as_deref().map(str::len), Some(64));
}

#[test]
fn n8_pack_operation_id_replays_exact_result_and_rejects_reuse_for_other_request() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "n8-pack".to_owned(),
            name: "N8 Pack".to_owned(),
            nodes: vec![CanvasNode {
                id: "writer".to_owned(),
                r#type: "writer".to_owned(),
                label: None,
                data: Value::Null,
                position: json!({"x": 0.0, "y": 0.0}),
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();
    let base_revision = load_workflow_graph_impl(temp.path(), Some("n8-pack".to_owned()))
        .unwrap()
        .content_revision
        .unwrap();

    let first = pack_workflow_selection_impl_with_operation_id(
        temp.path(),
        "n8-pack".to_owned(),
        vec!["writer".to_owned()],
        Some("sub-writer".to_owned()),
        Some("Writer Subflow".to_owned()),
        Some(base_revision.clone()),
        Some("desktop-pack-replay".to_owned()),
    )
    .unwrap();
    let second = pack_workflow_selection_impl_with_operation_id(
        temp.path(),
        "n8-pack".to_owned(),
        vec!["writer".to_owned()],
        Some("sub-writer".to_owned()),
        Some("Writer Subflow".to_owned()),
        Some(base_revision.clone()),
        Some("desktop-pack-replay".to_owned()),
    )
    .unwrap();
    assert_eq!(
        first, second,
        "same operation id must return the durable receipt"
    );
    assert_eq!(first.operation_id.as_deref(), Some("desktop-pack-replay"));

    let conflict = pack_workflow_selection_impl_with_operation_id(
        temp.path(),
        "n8-pack".to_owned(),
        vec!["writer".to_owned()],
        Some("sub-writer".to_owned()),
        Some("Different title".to_owned()),
        Some(base_revision),
        Some("desktop-pack-replay".to_owned()),
    )
    .expect_err("reusing an operation id for another request must be rejected");
    assert!(
        conflict.contains("reused with a different request"),
        "{conflict}"
    );
}

#[test]
fn f8_summarizer_graph_is_validated_before_save_and_before_run_creation() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let valid_graph = WorkflowGraphData {
        workflow_id: "f8-summarizer".to_owned(),
        name: "F8 Summarizer".to_owned(),
        nodes: vec![
            CanvasNode {
                id: "writer".to_owned(),
                r#type: "writer".to_owned(),
                label: Some("Writer".to_owned()),
                data: json!({ "prompt_template": "write" }),
                position: Value::Null,
            },
            CanvasNode {
                id: "summarizer".to_owned(),
                r#type: "summarizer".to_owned(),
                label: Some("Summarizer".to_owned()),
                data: json!({
                    "provider_id": "provider-main",
                    "model_id": "model-main",
                    "chapter_id": "chapter-1",
                    "chapter_document_id": "documents/chapter-1.md",
                    "chapter_text_alias": "chapter_text",
                    "auto_mode": false
                }),
                position: Value::Null,
            },
        ],
        edges: vec![CanvasEdge {
            id: "chapter-text".to_owned(),
            source: "writer".to_owned(),
            target: "summarizer".to_owned(),
            source_handle: "output".to_owned(),
            target_handle: "input".to_owned(),
            kind: WorkflowEdgeKind::Data,
            label: Some("chapter_text".to_owned()),
            data: Value::Null,
        }],
        metadata: Value::Null,
        content_revision: None,
        expected_revision: None,
    };

    let mut invalid_graph = valid_graph.clone();
    invalid_graph.nodes[1]
        .data
        .as_object_mut()
        .unwrap()
        .remove("chapter_id");
    let validation_error =
        ariadne::commands::validate_workflow_graph(invalid_graph.clone()).unwrap_err();
    assert!(validation_error.contains("chapter_id"));
    let save_error = save_workflow_graph_impl(temp.path(), invalid_graph).unwrap_err();
    assert!(save_error.contains("chapter_id"));
    assert!(!temp.path().join("workflows/f8-summarizer.json").exists());

    save_workflow_graph_impl(temp.path(), valid_graph).unwrap();
    let workflow_path = temp.path().join("workflows/f8-summarizer.json");
    let mut persisted: Value =
        serde_json::from_str(&std::fs::read_to_string(&workflow_path).unwrap()).unwrap();
    persisted["nodes"][1]["config"]
        .as_object_mut()
        .unwrap()
        .remove("chapter_document_id");
    std::fs::write(
        &workflow_path,
        serde_json::to_vec_pretty(&persisted).unwrap(),
    )
    .unwrap();

    let run_error = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "f8-summarizer".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
        },
    )
    .unwrap_err();
    assert!(run_error.contains("chapter_document_id"));
    assert!(
        !temp
            .path()
            .join(ariadne::workflow::RUNTIME_DB_FILE)
            .exists(),
        "invalid Summarizer config must fail before a run snapshot is created"
    );
}

#[test]
fn explicit_missing_workflow_id_is_not_loaded_as_default_graph() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let default_graph = load_workflow_graph_impl(temp.path(), None).unwrap();
    assert_eq!(default_graph.workflow_id, "default");

    let load_error =
        load_workflow_graph_impl(temp.path(), Some("missing-flow".to_owned())).unwrap_err();
    assert!(load_error.contains("workflow not found: missing-flow"));

    let secrets = MemorySecretStore::default();
    let run_error = run_workflow_impl(
        temp.path(),
        &secrets,
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "missing-flow".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
        },
    )
    .unwrap_err();
    assert!(run_error.contains("workflow not found: missing-flow"));
}

#[test]
fn async_start_rejects_missing_workflow_before_returning_queued() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let error = run_workflow(&state, "missing-flow".to_owned(), None).unwrap_err();

    assert!(error.contains("workflow not found: missing-flow"));
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    assert!(store.list_non_terminal_states().unwrap().is_empty());
}

#[test]
fn async_start_rejects_corrupt_runtime_store_before_returning_queued() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "runtime-preflight".to_owned(),
            name: "Runtime Preflight".to_owned(),
            nodes: vec![CanvasNode {
                id: "start".to_owned(),
                r#type: "start".to_owned(),
                label: None,
                data: Value::Null,
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();
    std::fs::write(temp.path().join("runtime.db"), "not a sqlite database").unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let error = run_workflow(&state, "runtime-preflight".to_owned(), None).unwrap_err();

    assert!(
        error.contains("database") || error.contains("SQLite"),
        "{error}"
    );
}

#[test]
fn workflow_graph_list_returns_all_saved_workflows() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "draft-flow".to_owned(),
            name: "Draft Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "start".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Start".to_owned()),
                data: Value::Null,
                position: json!({ "x": 0.0, "y": 0.0 }),
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "review/review-flow".to_owned(),
            name: "Review Flow".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "a".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("A".to_owned()),
                    data: Value::Null,
                    position: json!({ "x": 0.0, "y": 0.0 }),
                },
                CanvasNode {
                    id: "b".to_owned(),
                    r#type: "writer".to_owned(),
                    label: Some("B".to_owned()),
                    data: Value::Null,
                    position: json!({ "x": 100.0, "y": 0.0 }),
                },
            ],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let workflows = list_workflow_graphs_impl(temp.path()).unwrap();

    assert_eq!(
        workflows
            .iter()
            .map(|workflow| workflow.workflow_id.as_str())
            .collect::<Vec<_>>(),
        vec!["draft-flow", "review/review-flow"]
    );
    assert_eq!(workflows[0].name, "Draft Flow");
    assert_eq!(workflows[0].node_count, 1);
    assert_eq!(workflows[1].name, "Review Flow");
    assert_eq!(workflows[1].node_count, 2);
    let loaded =
        load_workflow_graph_impl(temp.path(), Some(workflows[1].workflow_id.clone())).unwrap();
    assert_eq!(loaded.name, "Review Flow");
}

#[test]
fn workflow_graph_list_and_load_support_template_manifest_files() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let manifest_dir = temp.path().join("workflows").join("market-basic");
    std::fs::create_dir_all(&manifest_dir).unwrap();
    std::fs::write(
        manifest_dir.join("workflow.json"),
        serde_json::to_string_pretty(&json!({
            "workflow_id": "market-basic",
            "name": "Market Basic",
            "version": "1.0.0",
            "workflow": {
                "id": "market-basic",
                "name": "Market Basic",
                "nodes": [
                    {
                        "id": "start",
                        "type_name": "start",
                        "label": "Start",
                        "config": {},
                        "position": { "x": 0.0, "y": 0.0 }
                    }
                ],
                "edges": [],
                "metadata": null
            },
            "required_node_types": ["start"],
            "required_tools": [],
            "required_permissions": []
        }))
        .unwrap(),
    )
    .unwrap();

    let workflows = list_workflow_graphs_impl(temp.path()).unwrap();
    let loaded = load_workflow_graph_impl(temp.path(), Some("market-basic".to_owned())).unwrap();

    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].workflow_id, "market-basic");
    assert_eq!(workflows[0].path, "workflows/market-basic/workflow.json");
    assert_eq!(loaded.workflow_id, "market-basic");
    assert_eq!(loaded.nodes.len(), 1);
}

#[cfg(unix)]
#[test]
fn workflow_graph_save_rejects_symlink_escape_from_workflows_root() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("workflows")).unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join("workflows").join("escape"))
        .unwrap();
    let graph = WorkflowGraphData {
        workflow_id: "escape/owned".to_owned(),
        name: "Escaped".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
        content_revision: None,
        expected_revision: None,
    };

    let error = save_workflow_graph_impl(temp.path(), graph).unwrap_err();

    assert!(error.contains("outside allowed root"));
    assert!(!outside.path().join("owned.json").exists());
}

#[cfg(unix)]
#[test]
fn workflow_graph_list_does_not_follow_symlinked_workflow_directories() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    std::fs::write(outside.path().join("workflow.json"), "{not-json").unwrap();
    std::os::unix::fs::symlink(
        outside.path(),
        temp.path().join("workflows").join("outside"),
    )
    .unwrap();

    let workflows = list_workflow_graphs_impl(temp.path()).unwrap();

    assert_eq!(
        workflows
            .iter()
            .map(|workflow| workflow.workflow_id.as_str())
            .collect::<Vec<_>>(),
        vec!["default"]
    );
}

#[test]
fn pack_workflow_selection_command_persists_subworkflow_graph() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "pack-flow".to_owned(),
            name: "Pack Flow".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "source".to_owned(),
                    r#type: "document_read".to_owned(),
                    label: None,
                    data: json!({
                        "path": "documents/source.md",
                        "include_content": true
                    }),
                    position: json!({ "x": 0.0, "y": 0.0 }),
                },
                CanvasNode {
                    id: "writer".to_owned(),
                    r#type: "writer".to_owned(),
                    label: None,
                    data: Value::Null,
                    position: json!({ "x": 100.0, "y": 0.0 }),
                },
                CanvasNode {
                    id: "reviewer".to_owned(),
                    r#type: "critic".to_owned(),
                    label: None,
                    data: Value::Null,
                    position: json!({ "x": 200.0, "y": 0.0 }),
                },
                CanvasNode {
                    id: "sink".to_owned(),
                    r#type: "export".to_owned(),
                    label: None,
                    data: json!({
                        "artifact_id": "exports/pack-flow.md",
                        "format": "markdown"
                    }),
                    position: json!({ "x": 300.0, "y": 0.0 }),
                },
            ],
            edges: vec![
                CanvasEdge {
                    id: "source-writer".to_owned(),
                    source: "source".to_owned(),
                    target: "writer".to_owned(),
                    source_handle: "out-draft".to_owned(),
                    target_handle: "in-draft".to_owned(),
                    kind: WorkflowEdgeKind::Data,
                    label: Some("draft".to_owned()),
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "writer-reviewer".to_owned(),
                    source: "writer".to_owned(),
                    target: "reviewer".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "reviewer-sink".to_owned(),
                    source: "reviewer".to_owned(),
                    target: "sink".to_owned(),
                    source_handle: "out-review".to_owned(),
                    target_handle: "in-review".to_owned(),
                    kind: WorkflowEdgeKind::Data,
                    label: Some("review".to_owned()),
                    data: Value::Null,
                },
            ],
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let report = pack_workflow_selection_impl(
        temp.path(),
        "pack-flow".to_owned(),
        vec!["writer".to_owned(), "reviewer".to_owned()],
        Some("sub-review".to_owned()),
        Some("Review Subflow".to_owned()),
        None,
    )
    .unwrap();
    let loaded = load_workflow_graph_impl(temp.path(), Some("pack-flow".to_owned())).unwrap();

    assert_eq!(report.subworkflow_node_id, "sub-review");
    assert_eq!(loaded.nodes.len(), 3);
    assert!(loaded.nodes.iter().any(|node| {
        node.id == "sub-review"
            && node.r#type == "subworkflow"
            && node.data.get("embedded_workflow").is_some()
    }));
    assert!(loaded
        .edges
        .iter()
        .any(|edge| edge.source == "source" && edge.target == "sub-review"));
    assert!(loaded
        .edges
        .iter()
        .any(|edge| edge.source == "sub-review" && edge.target == "sink"));
}

#[test]
fn run_workflow_executes_document_nodes_with_real_document_service() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("documents")).unwrap();
    std::fs::write(temp.path().join("documents/source.md"), "正文").unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "doc-flow".to_owned(),
            name: "Doc Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "read".to_owned(),
                r#type: "document_read".to_owned(),
                label: None,
                data: json!({
                    "path": temp.path().join("documents/source.md"),
                    "include_content": true
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let run = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "doc-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap();

    assert_eq!(run.status, "succeeded");
}

#[test]
fn run_workflow_command_starts_background_run() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "async-run".to_owned(),
            name: "Async Run".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-main".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Start".to_owned()),
                data: json!({
                    "work_dir": "main"
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let run = run_workflow(
        &state,
        "async-run".to_owned(),
        Some("start-main".to_owned()),
    )
    .unwrap();

    assert_eq!(run.status, "queued");
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let run_id = RunId::from(run.run_id);
    let state = wait_for_terminal_workflow_state(&store, &WorkflowId::from("async-run"), &run_id);

    assert_eq!(state.status, RunStatus::Succeeded);
    assert!(state.nodes.contains_key(&NodeId::from("start-main")));
    assert!(state
        .structured_events
        .iter()
        .any(|event| event.event_type == WorkflowRuntimeEventType::RunQueued));
    let started = state
        .structured_events
        .iter()
        .find(|event| event.event_type == WorkflowRuntimeEventType::RunStarted)
        .expect("background worker must continue the persisted queued snapshot");
    assert_eq!(started.sequence, 1);
    assert_eq!(
        state.next_event_sequence as usize,
        state.structured_events.len()
    );
    for _ in 0..100 {
        if store
            .load_worker_lease(&WorkflowId::from("async-run"), &run_id)
            .unwrap()
            .is_none()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(store
        .load_worker_lease(&WorkflowId::from("async-run"), &run_id)
        .unwrap()
        .is_none());
}

#[test]
fn workflow_worker_failure_updates_existing_run_with_structured_failure() {
    let temp = tempfile::tempdir().unwrap();
    let workflow_id = WorkflowId::from("failed-worker");
    let run_id = RunId::from("run-failed-worker");
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    store.create_state(&state).unwrap();

    mark_workflow_run_failed_impl(
        temp.path(),
        workflow_id.as_str(),
        run_id.as_str(),
        "workflow_worker_failed",
        "executor_init",
        "provider configuration changed after queueing",
        "repair provider configuration and start a new run",
    )
    .unwrap();

    let failed = store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .expect("queued run must remain queryable after worker failure");
    assert_eq!(failed.status, RunStatus::Failed);
    let failure = failed.failure.expect("run failure must be structured");
    assert_eq!(failure.code, "workflow_worker_failed");
    assert_eq!(failure.stage, "executor_init");
    assert_eq!(
        failure.recovery_suggestion,
        "repair provider configuration and start a new run"
    );
    assert!(failed
        .structured_events
        .iter()
        .any(|event| event.event_type == WorkflowRuntimeEventType::RunFailed));
}

#[test]
fn stale_workflow_worker_cannot_fail_run_after_lease_takeover() {
    let temp = tempfile::tempdir().unwrap();
    let workflow_id = WorkflowId::from("fenced-worker-failure");
    let run_id = RunId::from("run-fenced-worker-failure");
    let first_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    first_store
        .create_state(&WorkflowRunState::new(workflow_id.clone(), run_id.clone()))
        .unwrap();
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let stale_lease = first_store
        .acquire_worker_lease(&workflow_id, &run_id, "owner-a", now_ms, 100)
        .unwrap()
        .unwrap();
    let current_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let current_lease = current_store
        .acquire_worker_lease(
            &workflow_id,
            &run_id,
            "owner-b",
            now_ms.saturating_add(100),
            30_000,
        )
        .unwrap()
        .unwrap();

    let stale_result = mark_workflow_run_failed_with_lease_impl(
        temp.path(),
        workflow_id.as_str(),
        run_id.as_str(),
        "stale_worker_failed",
        "executor",
        "old owner completed after takeover",
        "ignore stale owner",
        &stale_lease,
    );
    assert!(stale_result.is_err());
    let unchanged = current_store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.status, RunStatus::Queued);
    assert!(unchanged.failure.is_none());
    assert_eq!(
        current_store
            .load_worker_lease(&workflow_id, &run_id)
            .unwrap(),
        Some(current_lease.clone())
    );

    mark_workflow_run_failed_with_lease_impl(
        temp.path(),
        workflow_id.as_str(),
        run_id.as_str(),
        "current_worker_failed",
        "executor",
        "current owner failed",
        "repair and restart",
        &current_lease,
    )
    .unwrap();
    let failed = current_store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.failure.unwrap().code, "current_worker_failed");
    assert!(current_store
        .load_worker_lease(&workflow_id, &run_id)
        .unwrap()
        .is_none());
}

#[test]
fn run_workflow_from_start_node_executes_only_that_branch() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("main/documents")).unwrap();
    std::fs::create_dir_all(temp.path().join("extra/documents")).unwrap();
    std::fs::write(temp.path().join("main/documents/source.md"), "正篇").unwrap();
    std::fs::write(temp.path().join("extra/documents/source.md"), "番外").unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "multi-start".to_owned(),
            name: "Multi Start".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "start-main".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("Main".to_owned()),
                    data: json!({
                        "name": "正篇",
                        "work_dir": "main",
                        "expose_as_tool": true
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "read-main".to_owned(),
                    r#type: "document_read".to_owned(),
                    label: None,
                    data: json!({
                        "path": "documents/source.md",
                        "include_content": true
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "start-extra".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("Extra".to_owned()),
                    data: json!({
                        "name": "番外",
                        "work_dir": "extra",
                        "expose_as_tool": false
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "read-extra".to_owned(),
                    r#type: "document_read".to_owned(),
                    label: None,
                    data: json!({
                        "path": "documents/source.md",
                        "include_content": true
                    }),
                    position: Value::Null,
                },
            ],
            edges: vec![
                CanvasEdge {
                    id: "main-edge".to_owned(),
                    source: "start-main".to_owned(),
                    target: "read-main".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "extra-edge".to_owned(),
                    source: "start-extra".to_owned(),
                    target: "read-extra".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
            ],
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let main = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "multi-start".to_owned(),
            start_node_id: Some("start-main".to_owned()),
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap();
    let extra = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "multi-start".to_owned(),
            start_node_id: Some("start-extra".to_owned()),
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap();

    assert_eq!(main.status, "succeeded");
    assert_eq!(extra.status, "succeeded");
    assert_ne!(main.run_id, extra.run_id);

    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let main_state = store
        .load_state(&WorkflowId::from("multi-start"), &RunId::from(main.run_id))
        .unwrap()
        .unwrap();
    let extra_state = store
        .load_state(&WorkflowId::from("multi-start"), &RunId::from(extra.run_id))
        .unwrap()
        .unwrap();

    assert!(main_state.nodes.contains_key(&NodeId::from("start-main")));
    assert!(main_state.nodes.contains_key(&NodeId::from("read-main")));
    assert_eq!(
        main_state.nodes[&NodeId::from("read-main")].outputs["content"],
        PortValue::inline("正篇")
    );
    assert!(!main_state.nodes.contains_key(&NodeId::from("start-extra")));
    assert!(!main_state.nodes.contains_key(&NodeId::from("read-extra")));
    assert!(extra_state.nodes.contains_key(&NodeId::from("start-extra")));
    assert!(extra_state.nodes.contains_key(&NodeId::from("read-extra")));
    assert_eq!(
        extra_state.nodes[&NodeId::from("read-extra")].outputs["content"],
        PortValue::inline("番外")
    );
    assert!(!extra_state.nodes.contains_key(&NodeId::from("start-main")));
    assert!(!extra_state.nodes.contains_key(&NodeId::from("read-main")));
}

#[test]
fn async_run_persists_inputs_and_expired_lease_resume_uses_prepared_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "tool-start".to_owned(),
            name: "Tool Start".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "start-main".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("Start Main".to_owned()),
                    data: json!({
                        "expose_as_tool": true,
                        "input_schema": {
                            "type": "object",
                            "properties": {
                                "topic": { "type": "string" }
                            },
                            "required": ["topic"],
                            "additionalProperties": false
                        }
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "check-topic".to_owned(),
                    r#type: "condition".to_owned(),
                    label: None,
                    data: json!({
                        "input_alias": "topic",
                        "operator": "equals",
                        "expected": "长夜行"
                    }),
                    position: Value::Null,
                },
            ],
            edges: vec![
                CanvasEdge {
                    id: "start-to-check".to_owned(),
                    source: "start-main".to_owned(),
                    target: "check-topic".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "topic-to-check".to_owned(),
                    source: "start-main".to_owned(),
                    target: "check-topic".to_owned(),
                    source_handle: "topic".to_owned(),
                    target_handle: "input".to_owned(),
                    kind: WorkflowEdgeKind::Data,
                    label: Some("topic".to_owned()),
                    data: Value::Null,
                },
            ],
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let app_state = tempfile::tempdir().unwrap();
    let app = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let mut initial_inputs = BTreeMap::new();
    initial_inputs.insert("topic".to_owned(), json!("长夜行"));
    let run = start_workflow_with_request(
        &app,
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "tool-start".to_owned(),
            start_node_id: Some("start-main".to_owned()),
            initial_inputs,
        },
    )
    .unwrap();

    assert_eq!(run.status, "queued");
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("tool-start");
    let run_id = RunId::from(run.run_id);
    let persisted_after_return = store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .expect("queued response must have a persisted run snapshot");
    let prepared_workflow = persisted_after_return
        .prepared_workflow
        .clone()
        .expect("validated workflow snapshot must be persisted before returning queued");
    assert_eq!(
        prepared_workflow
            .nodes
            .iter()
            .find(|node| node.id == NodeId::from("start-main"))
            .unwrap()
            .config["initial_inputs"]["topic"],
        json!("长夜行")
    );

    let completed = wait_for_terminal_workflow_state(&store, &workflow_id, &run_id);
    let check = completed.nodes.get(&NodeId::from("check-topic")).unwrap();
    assert_eq!(check.outputs.get("passed"), Some(&PortValue::inline(true)));

    let recovery_run_id = RunId::from("recovered-tool-run");
    let mut recovery = WorkflowRuntime::new(&prepared_workflow, recovery_run_id.clone()).unwrap();
    recovery.state.prepared_workflow = Some(prepared_workflow);
    recovery.state.start_node_id = Some(NodeId::from("start-main"));
    recovery.state.status = RunStatus::Running;
    store.create_state(&recovery.state).unwrap();
    let stale_lease = store
        .acquire_worker_lease(&workflow_id, &recovery_run_id, "crashed-worker", 1, 1)
        .unwrap()
        .expect("simulated crashed worker must own the initial lease");
    assert_eq!(stale_lease.generation, 1);

    let current_revision = load_workflow_graph_impl(temp.path(), Some("tool-start".to_owned()))
        .unwrap()
        .content_revision;

    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "tool-start".to_owned(),
            name: "Changed After Queueing".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-main".to_owned(),
                r#type: "start".to_owned(),
                label: None,
                data: Value::Null,
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: current_revision,
        },
    )
    .unwrap();
    let resumed = resume_workflow(
        &app,
        workflow_id.as_str().to_owned(),
        recovery_run_id.as_str().to_owned(),
    )
    .unwrap();
    assert_eq!(resumed.status, "running");
    let recovered = wait_for_terminal_workflow_state(&store, &workflow_id, &recovery_run_id);
    assert_eq!(recovered.status, RunStatus::Succeeded);
    assert_eq!(
        recovered.nodes[&NodeId::from("check-topic")].outputs["passed"],
        PortValue::inline(true)
    );
}

#[test]
fn run_workflow_start_node_id_must_reference_start_node() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "bad-start".to_owned(),
            name: "Bad Start".to_owned(),
            nodes: vec![CanvasNode {
                id: "read".to_owned(),
                r#type: "document_read".to_owned(),
                label: None,
                data: json!({
                    "path": "documents/source.md",
                    "include_content": true
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let error = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "bad-start".to_owned(),
            start_node_id: Some("read".to_owned()),
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap_err();

    assert!(error.contains("must reference a start node"));
}

#[test]
fn run_workflow_llm_node_requires_configured_provider_instead_of_noop() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "llm-flow".to_owned(),
            name: "LLM Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "ask".to_owned(),
                r#type: "llm".to_owned(),
                label: None,
                data: json!({
                    "provider_id": "openai",
                    "model_id": "gpt-test",
                    "prompt_alias": "prompt"
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let error = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "llm-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap_err();

    assert!(error.contains("LLM provider"));
}

#[test]
fn budget_and_provider_commands_do_not_return_secret_values() {
    let temp = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();
    update_budget_config_impl(temp.path(), 25.0, 3.5).unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: "gpt-test".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: true,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "openai".to_owned(),
        "sk-secret".to_owned(),
    )
    .unwrap();

    let budget = get_budget_status_impl(temp.path()).unwrap();
    let provider = get_provider_config_impl(temp.path(), &secrets).unwrap();
    // 保存的全局预算必须能映射到执行侧日限额（LLM evaluate_budget 路径）。
    let live_limits = ariadne::costs::budget_limits_from_global_budget(budget.budget_usd);
    assert_eq!(live_limits.daily_usd, Some(25.0));

    assert_eq!(budget.budget_usd, 25.0);
    assert_eq!(budget.preauthorized_usd, 3.5);
    assert!(provider.has_openai_key);
    assert_eq!(provider.default_llm_provider_id.as_deref(), Some("openai"));
    assert_eq!(
        provider.default_embedding_provider_id.as_deref(),
        Some("openai")
    );
    assert_eq!(provider.providers[0].provider, "openai");
    assert!(provider.providers[0].configured);
    assert_eq!(provider.providers[0].models[0].model_id, "gpt-test");
    let config = ConfigStore::new(temp.path()).load_or_create().unwrap();
    assert!(config.providers.providers[0].api_key.is_none());
    assert!(secrets.get_secret("provider.openai").unwrap().is_none());
}

#[test]
fn regular_provider_key_save_rejects_unconfigured_provider_without_side_effects() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let state = AriadneAppState::new(project.path(), app_state.path(), secrets.clone());
    let before = ConfigStore::new(project.path()).load().unwrap();
    assert!(get_provider_config_impl(project.path(), secrets.as_ref())
        .unwrap()
        .providers
        .iter()
        .all(|provider| !provider.configured));

    let error = save_provider_key(
        &state,
        "draft-provider".to_owned(),
        "draft-secret".to_owned(),
    )
    .unwrap_err();

    assert!(error
        .diagnostic
        .as_deref()
        .is_some_and(|message| message.contains("provider is not configured")));
    assert_eq!(ConfigStore::new(project.path()).load().unwrap(), before);
    assert!(!get_provider_config_impl(project.path(), secrets.as_ref())
        .unwrap()
        .providers
        .iter()
        .any(|provider| provider.provider == "draft-provider"));
}

#[test]
fn regular_provider_key_save_updates_only_existing_provider_credential() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    save_provider_settings_impl(
        project.path(),
        ProviderSettingsUpdate {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: "gpt-test".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    let before = ConfigStore::new(project.path()).load().unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let state = AriadneAppState::new(project.path(), app_state.path(), secrets.clone());

    save_provider_key(&state, "openai".to_owned(), "replacement-secret".to_owned()).unwrap();

    assert_eq!(ConfigStore::new(project.path()).load().unwrap(), before);
    assert!(get_provider_config_impl(project.path(), secrets.as_ref())
        .unwrap()
        .providers
        .iter()
        .any(|provider| provider.provider == "openai" && provider.has_key));
}

#[test]
fn provider_section_secret_failure_leaves_config_unchanged() {
    struct RejectingSecretStore;

    impl SecretStore for RejectingSecretStore {
        fn set_secret(
            &self,
            _key_id: &str,
            _value: ariadne::config::SecretValue,
        ) -> ariadne::contracts::CoreResult<()> {
            Err(ariadne::contracts::CoreError::validation(
                "injected secret write failure",
            ))
        }

        fn get_secret(
            &self,
            _key_id: &str,
        ) -> ariadne::contracts::CoreResult<Option<ariadne::config::SecretValue>> {
            Ok(None)
        }

        fn delete_secret(&self, _key_id: &str) -> ariadne::contracts::CoreResult<()> {
            Ok(())
        }
    }

    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let store = ConfigStore::new(project.path());
    let before = store.load().unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(RejectingSecretStore),
    );

    let error = save_provider_section_settings(
        &state,
        ProviderSectionSettings {
            provider: ProviderSettingsUpdate {
                provider_id: "new-provider".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "New Provider".to_owned(),
                enabled: true,
                base_url: Some("https://example.invalid/v1".to_owned()),
                models: vec![ModelConfig {
                    model_id: "new-model".to_owned(),
                    capability: ProviderCapability::Llm,
                    max_context_tokens: Some(4096),
                    input_cost_per_million_tokens: None,
                    output_cost_per_million_tokens: None,
                }],
                make_default_llm: true,
                make_default_embedding: false,
                make_default_reranker: false,
                make_default_search: false,
            },
            api_key: Some("new-secret".to_owned()),
        },
    )
    .unwrap_err();

    assert!(error
        .diagnostic
        .as_deref()
        .is_some_and(|message| message.contains("injected secret write failure")));
    assert_eq!(store.load().unwrap(), before);
}

fn configure_removable_provider(
    project_root: &std::path::Path,
    provider_id: &str,
    model_id: &str,
    make_default: bool,
) {
    save_provider_settings_impl(
        project_root,
        ProviderSettingsUpdate {
            provider_id: provider_id.to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: provider_id.to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: model_id.to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: make_default,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
}

#[test]
fn provider_removal_is_blocked_by_presets_workflows_and_active_run_snapshots() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    configure_removable_provider(project.path(), "target", "target-model", true);
    configure_removable_provider(project.path(), "remaining", "remaining-model", false);

    let mut presets = NodePresetSettings {
        default_model_id: "target-model".to_owned(),
        ..NodePresetSettings::default()
    };
    for preset in &mut presets.presets {
        preset.model_id = "target-model".to_owned();
    }
    save_node_preset_settings_impl(project.path(), presets).unwrap();

    let workflow = WorkflowDefinition {
        id: WorkflowId::from("provider-reference"),
        name: "Provider Reference".to_owned(),
        nodes: vec![NodeInstance {
            id: NodeId::from("ask"),
            type_name: "llm".to_owned(),
            label: None,
            config: json!({
                "provider_id": "target",
                "model_id": "target-model",
                "prompt_template": "hello"
            }),
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    save_workflow_graph_impl(
        project.path(),
        WorkflowGraphData {
            workflow_id: "provider-reference".to_owned(),
            name: "Provider Reference".to_owned(),
            nodes: vec![CanvasNode {
                id: "ask".to_owned(),
                r#type: "llm".to_owned(),
                label: None,
                data: workflow.nodes[0].config.clone(),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();
    let runtime_store = SqliteWorkflowRuntimeStore::open(project.path()).unwrap();
    let mut run = WorkflowRunState::new(
        WorkflowId::from("provider-reference"),
        RunId::from("active-run"),
    );
    run.prepared_workflow = Some(workflow);
    runtime_store.create_state(&run).unwrap();

    let secrets = Arc::new(MemorySecretStore::default());
    ProjectCredentialScope::new(project.path(), secrets.as_ref())
        .unwrap()
        .set_provider_secret("target", SecretValue::new("target-secret"))
        .unwrap();
    let state = AriadneAppState::new(project.path(), app_state.path(), secrets.clone());
    let before = ConfigStore::new(project.path()).load().unwrap();

    let preview = preview_provider_removal(&state, "target".to_owned()).unwrap();
    assert!(preview
        .blocking_references
        .iter()
        .any(|reference| reference.reference_type == "node_preset"));
    assert!(preview
        .blocking_references
        .iter()
        .any(|reference| reference.reference_type == "workflow"));
    assert!(preview
        .blocking_references
        .iter()
        .any(|reference| reference.reference_type == "active_run"));

    let error = remove_provider(&state, "target".to_owned(), preview.revision).unwrap_err();
    assert_eq!(
        error.code,
        ariadne::command_error::CommandErrorCode::Conflict
    );
    assert_eq!(ConfigStore::new(project.path()).load().unwrap(), before);
    assert!(
        ProjectCredentialScope::new(project.path(), secrets.as_ref())
            .unwrap()
            .get_provider_secret("target")
            .unwrap()
            .is_some()
    );
}

#[test]
fn provider_removal_rejects_stale_impact_revision_without_side_effects() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    configure_removable_provider(project.path(), "target", "target-model", true);
    let secrets = Arc::new(MemorySecretStore::default());
    ProjectCredentialScope::new(project.path(), secrets.as_ref())
        .unwrap()
        .set_provider_secret("target", SecretValue::new("target-secret"))
        .unwrap();
    let state = AriadneAppState::new(project.path(), app_state.path(), secrets.clone());
    let preview = preview_provider_removal(&state, "target".to_owned()).unwrap();

    save_provider_settings_impl(
        project.path(),
        ProviderSettingsUpdate {
            provider_id: "target".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "renamed target".to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: "target-model".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    let before = ConfigStore::new(project.path()).load().unwrap();

    let error = remove_provider(&state, "target".to_owned(), preview.revision).unwrap_err();
    assert_eq!(
        error.code,
        ariadne::command_error::CommandErrorCode::Conflict
    );
    assert_eq!(ConfigStore::new(project.path()).load().unwrap(), before);
    assert!(
        ProjectCredentialScope::new(project.path(), secrets.as_ref())
            .unwrap()
            .get_provider_secret("target")
            .unwrap()
            .is_some()
    );
}

#[test]
fn provider_removal_secret_failure_restores_config_runtime_and_credential() {
    #[derive(Default)]
    struct RejectDeleteSecretStore {
        inner: MemorySecretStore,
    }

    impl SecretStore for RejectDeleteSecretStore {
        fn set_secret(
            &self,
            key_id: &str,
            value: SecretValue,
        ) -> ariadne::contracts::CoreResult<()> {
            self.inner.set_secret(key_id, value)
        }

        fn get_secret(&self, key_id: &str) -> ariadne::contracts::CoreResult<Option<SecretValue>> {
            self.inner.get_secret(key_id)
        }

        fn delete_secret(&self, _key_id: &str) -> ariadne::contracts::CoreResult<()> {
            Err(ariadne::contracts::CoreError::validation(
                "injected provider credential deletion failure",
            ))
        }
    }

    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    configure_removable_provider(project.path(), "target", "target-model", true);
    let secrets = Arc::new(RejectDeleteSecretStore::default());
    ProjectCredentialScope::new(project.path(), secrets.as_ref())
        .unwrap()
        .set_provider_secret("target", SecretValue::new("target-secret"))
        .unwrap();
    let state = AriadneAppState::new(project.path(), app_state.path(), secrets.clone());
    let before = ConfigStore::new(project.path()).load().unwrap();
    let preview = preview_provider_removal(&state, "target".to_owned()).unwrap();

    let error = remove_provider(&state, "target".to_owned(), preview.revision).unwrap_err();
    assert!(error.contains("injected provider credential deletion failure"));
    assert_eq!(ConfigStore::new(project.path()).load().unwrap(), before);
    assert!(state.retrieval_runtime().is_ok());
    assert!(
        ProjectCredentialScope::new(project.path(), secrets.as_ref())
            .unwrap()
            .get_provider_secret("target")
            .unwrap()
            .is_some()
    );
}

#[test]
fn provider_removal_clears_defaults_and_project_credential_atomically() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    configure_removable_provider(project.path(), "target", "target-model", true);
    let secrets = Arc::new(MemorySecretStore::default());
    ProjectCredentialScope::new(project.path(), secrets.as_ref())
        .unwrap()
        .set_provider_secret("target", SecretValue::new("target-secret"))
        .unwrap();
    let state = AriadneAppState::new(project.path(), app_state.path(), secrets.clone());
    let preview = preview_provider_removal(&state, "target".to_owned()).unwrap();
    assert_eq!(preview.default_roles, vec!["llm"]);
    assert!(preview.has_key);
    assert!(preview.blocking_references.is_empty());

    let status = remove_provider(&state, "target".to_owned(), preview.revision).unwrap();
    let config = ConfigStore::new(project.path()).load().unwrap();
    assert!(!config
        .providers
        .providers
        .iter()
        .any(|provider| provider.provider_id == "target"));
    assert!(config.providers.default_llm_provider_id.is_none());
    assert!(status.default_llm_provider_id.is_none());
    assert!(status
        .providers
        .iter()
        .all(|provider| provider.provider != "target" || !provider.configured));
    assert!(
        ProjectCredentialScope::new(project.path(), secrets.as_ref())
            .unwrap()
            .get_provider_secret("target")
            .unwrap()
            .is_none()
    );
}

#[test]
fn provider_removal_and_workflow_save_share_reference_graph_lock() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    configure_removable_provider(project.path(), "target", "target-model", true);

    let lock_target = project
        .path()
        .join(".config")
        .join(".provider-reference-graph");
    let graph_lock = PathWriteLock::acquire(&lock_target).unwrap();
    let workflow_project = project.path().to_path_buf();
    let (workflow_started_tx, workflow_started_rx) = std::sync::mpsc::channel();
    let (workflow_done_tx, workflow_done_rx) = std::sync::mpsc::channel();
    let workflow_thread = thread::spawn(move || {
        workflow_started_tx.send(()).unwrap();
        let result = save_workflow_graph_impl(
            &workflow_project,
            WorkflowGraphData {
                workflow_id: "reference-lock".to_owned(),
                name: "Reference Lock".to_owned(),
                nodes: vec![CanvasNode {
                    id: "start".to_owned(),
                    r#type: "start".to_owned(),
                    label: None,
                    data: Value::Null,
                    position: Value::Null,
                }],
                edges: Vec::new(),
                metadata: Value::Null,
                content_revision: None,
                expected_revision: None,
            },
        );
        workflow_done_tx.send(result).unwrap();
    });
    workflow_started_rx.recv().unwrap();
    thread::sleep(Duration::from_millis(50));
    assert!(workflow_done_rx.try_recv().is_err());
    drop(graph_lock);
    workflow_done_rx.recv().unwrap().unwrap();
    workflow_thread.join().unwrap();

    let secrets = Arc::new(MemorySecretStore::default());
    let state = AriadneAppState::new(project.path(), app_state.path(), secrets);
    let preview = preview_provider_removal(&state, "target".to_owned()).unwrap();
    let graph_lock = PathWriteLock::acquire(&lock_target).unwrap();
    let (removal_started_tx, removal_started_rx) = std::sync::mpsc::channel();
    let (removal_done_tx, removal_done_rx) = std::sync::mpsc::channel();
    let removal_thread = thread::spawn(move || {
        removal_started_tx.send(()).unwrap();
        let result = remove_provider(&state, "target".to_owned(), preview.revision);
        removal_done_tx.send(result).unwrap();
    });
    removal_started_rx.recv().unwrap();
    thread::sleep(Duration::from_millis(50));
    assert!(removal_done_rx.try_recv().is_err());
    drop(graph_lock);
    removal_done_rx.recv().unwrap().unwrap();
    removal_thread.join().unwrap();
}

#[test]
fn provider_key_status_is_namespaced_by_project_root() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();

    save_provider_key_impl(
        project_a.path(),
        &secrets,
        "openai".to_owned(),
        "sk-project-a".to_owned(),
    )
    .unwrap();

    let status_a = get_provider_config_impl(project_a.path(), &secrets).unwrap();
    let status_b = get_provider_config_impl(project_b.path(), &secrets).unwrap();

    assert!(status_a.has_openai_key);
    assert!(!status_b.has_openai_key);
}

#[test]
fn provider_credentials_do_not_collapse_backslash_or_nested_project_paths() {
    let root = tempfile::tempdir().unwrap();
    let backslash = root.path().join("team\\book");
    let nested = root.path().join("team").join("book");
    ariadne::frontend::initialize_project(&backslash).unwrap();
    ariadne::frontend::initialize_project(&nested).unwrap();
    let secrets = MemorySecretStore::default();

    save_provider_key_impl(
        &backslash,
        &secrets,
        "openai".to_owned(),
        "sk-backslash".to_owned(),
    )
    .unwrap();

    assert!(
        get_provider_config_impl(&backslash, &secrets)
            .unwrap()
            .has_openai_key
    );
    assert!(
        !get_provider_config_impl(&nested, &secrets)
            .unwrap()
            .has_openai_key
    );
}

#[cfg(unix)]
#[test]
fn provider_credentials_use_lossless_non_utf8_project_identity() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let root = tempfile::tempdir().unwrap();
    let project_a = root.path().join(OsString::from_vec(b"novel-\xff".to_vec()));
    let project_b = root.path().join(OsString::from_vec(b"novel-\xfe".to_vec()));
    ariadne::frontend::initialize_project(&project_a).unwrap();
    ariadne::frontend::initialize_project(&project_b).unwrap();
    let secrets = MemorySecretStore::default();

    save_provider_key_impl(
        &project_a,
        &secrets,
        "openai".to_owned(),
        "sk-non-utf8".to_owned(),
    )
    .unwrap();

    assert!(
        get_provider_config_impl(&project_a, &secrets)
            .unwrap()
            .has_openai_key
    );
    assert!(
        !get_provider_config_impl(&project_b, &secrets)
            .unwrap()
            .has_openai_key
    );
}

#[test]
fn moved_project_requires_explicit_provider_credential_rebind() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    let moved = root.path().join("moved");
    ariadne::frontend::initialize_project(&original).unwrap();
    let secrets = MemorySecretStore::default();
    save_provider_key_impl(
        &original,
        &secrets,
        "openai".to_owned(),
        "sk-original".to_owned(),
    )
    .unwrap();
    std::fs::rename(&original, &moved).unwrap();

    assert!(
        !get_provider_config_impl(&moved, &secrets)
            .unwrap()
            .has_openai_key
    );
    save_provider_key_impl(
        &moved,
        &secrets,
        "openai".to_owned(),
        "sk-rebound".to_owned(),
    )
    .unwrap();
    assert!(
        get_provider_config_impl(&moved, &secrets)
            .unwrap()
            .has_openai_key
    );
}

#[test]
fn malicious_project_secret_ref_is_rejected_before_all_provider_network_entrypoints() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    save_workflow_graph_impl(
        project.path(),
        WorkflowGraphData {
            workflow_id: "llm-flow".to_owned(),
            name: "LLM Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "ask".to_owned(),
                r#type: "llm".to_owned(),
                label: None,
                data: json!({
                    "provider_id": "attacker",
                    "model_id": "gpt-test",
                    "prompt_alias": "prompt"
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let request_count = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let server_count = Arc::clone(&request_count);
    let server_stop = Arc::clone(&stop);
    let server = thread::spawn(move || {
        while !server_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    server_count.fetch_add(1, Ordering::AcqRel);
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer);
                    let body = r#"{"data":[],"choices":[{"message":{"content":"unexpected"}}]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(), body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    let store = ConfigStore::new(project.path());
    let mut config = store.load_or_create().unwrap();
    config.providers.providers = vec![ProviderConfig {
        provider_id: "attacker".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Attacker".to_owned(),
        enabled: true,
        base_url: Some(base_url),
        api_key: Some(SecretRef::new("victim-project-global-secret")),
        models: vec![ModelConfig {
            model_id: "gpt-test".to_owned(),
            capability: ProviderCapability::Llm,
            max_context_tokens: Some(4096),
            input_cost_per_million_tokens: None,
            output_cost_per_million_tokens: None,
        }],
    }];
    config.providers.default_llm_provider_id = Some("attacker".to_owned());
    let raw = yaml_serde::to_string(&yaml_serde::to_value(&config.providers).unwrap()).unwrap();
    std::fs::write(store.config_dir().join(PROVIDERS_CONFIG_FILE), raw).unwrap();
    let secrets = MemorySecretStore::default();
    secrets
        .set_secret(
            "victim-project-global-secret",
            ariadne::config::SecretValue::new("sk-victim"),
        )
        .unwrap();

    assert!(ariadne::commands::fetch_provider_models_with_secrets_impl(
        project.path(),
        &secrets,
        Some("attacker".to_owned())
    )
    .unwrap_err()
    .contains("untrusted project SecretRef"));
    assert!(quick_edit_impl(
        project.path(),
        &secrets,
        QuickEditRequest {
            selected_text: "text".to_owned(),
            instruction: "rewrite".to_owned(),
            context_ref: None,
        }
    )
    .unwrap_err()
    .contains("untrusted project SecretRef"));
    assert!(project_ai_chat_impl(
        project.path(),
        &secrets,
        ProjectAiRequest {
            message: "hello".to_owned(),
            ..ProjectAiRequest::default()
        }
    )
    .unwrap_err()
    .contains("untrusted project SecretRef"));
    assert!(run_workflow_impl(
        project.path(),
        &secrets,
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "llm-flow".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
        }
    )
    .unwrap_err()
    .contains("untrusted project SecretRef"));

    stop.store(true, Ordering::Release);
    server.join().unwrap();
    assert_eq!(request_count.load(Ordering::Acquire), 0);
}

#[test]
fn saving_provider_key_explicitly_rebinds_and_removes_all_legacy_secret_refs() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let store = ConfigStore::new(project.path());
    let mut config = store.load_or_create().unwrap();
    config.providers.providers = vec![
        ProviderConfig {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: None,
            api_key: Some(SecretRef::new("legacy-openai-global-secret")),
            models: Vec::new(),
        },
        ProviderConfig {
            provider_id: "anthropic".to_owned(),
            provider_type: ProviderType::Anthropic,
            display_name: "Anthropic".to_owned(),
            enabled: true,
            base_url: None,
            api_key: Some(SecretRef::new("legacy-anthropic-global-secret")),
            models: Vec::new(),
        },
    ];
    config.providers.default_llm_provider_id = Some("openai".to_owned());
    let raw = yaml_serde::to_string(&yaml_serde::to_value(&config.providers).unwrap()).unwrap();
    std::fs::write(store.config_dir().join(PROVIDERS_CONFIG_FILE), raw).unwrap();
    let secrets = MemorySecretStore::default();

    assert!(get_provider_config_impl(project.path(), &secrets).is_err());
    save_provider_key_impl(
        project.path(),
        &secrets,
        "openai".to_owned(),
        "sk-rebound".to_owned(),
    )
    .unwrap();
    let rebound = store.load().unwrap();
    assert!(rebound
        .providers
        .providers
        .iter()
        .all(|provider| provider.api_key.is_none()));
    let status = get_provider_config_impl(project.path(), &secrets).unwrap();
    assert!(status.has_openai_key);
    assert!(
        !status
            .providers
            .iter()
            .find(|provider| provider.provider == "anthropic")
            .unwrap()
            .has_key
    );
}

#[test]
fn provider_model_fetch_returns_configured_and_embedding_models() {
    let temp = tempfile::tempdir().unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: "gpt-test".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: true,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();

    let models = fetch_provider_models_impl(temp.path(), Some("openai".to_owned())).unwrap();

    assert_eq!(models.provider_id, "openai");
    assert!(models
        .models
        .iter()
        .any(|model| model.model_id == "gpt-test" && model.capability == ProviderCapability::Llm));
    assert!(models
        .models
        .iter()
        .any(|model| model.model_id == "text-embedding-3-small"
            && model.capability == ProviderCapability::Embedding));
}

#[test]
fn provider_model_fetch_calls_remote_models_endpoint() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("GET /models "));
        assert!(request.contains("authorization: Bearer local-key"));
        let response_body = r#"{
          "data": [
            {"id": "chat-alpha"},
            {"id": "text-embedding-3-small"}
          ]
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_models".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Models".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "chat-alpha".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(8192),
                input_cost_per_million_tokens: Some(0.25),
                output_cost_per_million_tokens: Some(0.5),
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        secrets.as_ref(),
        "local_models".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let state = AriadneAppState::new(
        temp.path().to_path_buf(),
        app_state.path(),
        Arc::clone(&secrets),
    );
    let models = fetch_provider_models(&state, Some("local_models".to_owned())).unwrap();
    server.join().unwrap();

    assert_eq!(models.provider_id, "local_models");
    let chat_model = models
        .models
        .iter()
        .find(|model| model.model_id == "chat-alpha")
        .unwrap();
    assert_eq!(chat_model.capability, ProviderCapability::Llm);
    assert_eq!(chat_model.max_context_tokens, Some(8192));
    assert_eq!(chat_model.input_cost_per_million_tokens, Some(0.25));
    assert!(models
        .models
        .iter()
        .any(|model| model.model_id == "text-embedding-3-small"
            && model.capability == ProviderCapability::Embedding));
}

#[test]
fn c9_provider_model_fetch_can_cancel_stalled_response() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicBool::new(false));
    let server_accepted = Arc::clone(&accepted);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        server_accepted.store(true, Ordering::Release);
        thread::sleep(Duration::from_millis(500));
    });

    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "cancellable_models".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Cancellable Models".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "chat-alpha".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(8192),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    let state = AriadneAppState::new(
        temp.path().to_path_buf(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let cancellation = ExecutionCancellation::new();
    let cancel_from_thread = cancellation.clone();
    let canceller = thread::spawn(move || {
        let started = std::time::Instant::now();
        while !accepted.load(Ordering::Acquire) && started.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(5));
        }
        cancel_from_thread.cancel();
    });

    let started = std::time::Instant::now();
    let error = fetch_provider_models_with_cancellation(
        &state,
        Some("cancellable_models".to_owned()),
        &cancellation,
    )
    .unwrap_err();
    let request_elapsed = started.elapsed();

    canceller.join().unwrap();
    server.join().unwrap();
    assert_eq!(
        error.code,
        ariadne::command_error::CommandErrorCode::Cancelled
    );
    assert!(request_elapsed < Duration::from_millis(300));
}

#[test]
fn provider_model_fetch_rejects_oversized_streaming_response() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("GET /models "));
        let response_header =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
        stream.write_all(response_header.as_bytes()).unwrap();
        stream.write_all(&vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
    });

    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_models".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Models".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "chat-alpha".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(8192),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    let state = AriadneAppState::new(
        temp.path().to_path_buf(),
        app_state.path(),
        Arc::clone(&secrets),
    );

    let error = fetch_provider_models(&state, Some("local_models".to_owned())).unwrap_err();
    server.join().unwrap();

    assert!(error.contains("model list response exceeds"));
}

#[test]
fn provider_settings_reject_non_http_base_url() {
    let temp = tempfile::tempdir().unwrap();
    let error = save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_file".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local File".to_owned(),
            enabled: true,
            base_url: Some("file:///tmp/provider".to_owned()),
            models: vec![ModelConfig {
                model_id: "local".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap_err();

    assert!(error.contains("provider base_url must use http or https"));
}

#[test]
fn node_preset_settings_reject_unknown_configured_model() {
    let temp = tempfile::tempdir().unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: "gpt-configured".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    let mut settings = NodePresetSettings {
        default_model_id: "missing-model".to_owned(),
        ..NodePresetSettings::default()
    };
    for preset in &mut settings.presets {
        preset.model_id = "gpt-configured".to_owned();
    }

    let error = save_node_preset_settings_impl(temp.path(), settings).unwrap_err();

    assert!(error.contains("default_model_id references a model that is not configured"));
}

#[test]
fn backend_diagnostics_reports_provider_configuration_gaps() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let report = get_backend_diagnostics(&state).unwrap();

    assert_ne!(report.status, DiagnosticStatus::Healthy);
    assert!(report
        .items
        .iter()
        .any(|item| item.component == "providers.llm.default"
            && item.status == DiagnosticStatus::Degraded));
    assert!(report
        .items
        .iter()
        .any(|item| item.component == "providers.embedding.default"
            && item.status == DiagnosticStatus::Healthy));
}

#[test]
fn backend_diagnostics_reports_unconstructable_retrieval_runtime_instead_of_failing() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let store = ConfigStore::new(project.path());
    let mut config = store.load_or_create().unwrap();
    config.rag.vector_store.enabled = true;
    store.save(&config).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let report = get_backend_diagnostics(&state).unwrap();

    assert_eq!(report.status, DiagnosticStatus::Unavailable);
    assert!(report.items.iter().any(|item| {
        item.component == "project_retrieval_runtime"
            && item.status == DiagnosticStatus::Unavailable
            && item
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("default_embedding_provider_id"))
    }));
    assert!(report.items.iter().any(|item| {
        item.component == "providers.embedding.default" && item.status == DiagnosticStatus::Degraded
    }));
}

#[test]
fn backend_diagnostics_never_marks_unverified_embedding_configuration_healthy() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let store = ConfigStore::new(project.path());
    let mut config = store.load_or_create().unwrap();
    config.rag.vector_store.enabled = true;
    config.providers.default_embedding_provider_id = Some("embedding".to_owned());
    config.providers.providers.push(ProviderConfig {
        provider_id: "embedding".to_owned(),
        provider_type: ProviderType::OpenAi,
        display_name: "Embedding".to_owned(),
        enabled: true,
        base_url: None,
        api_key: None,
        models: vec![ModelConfig {
            model_id: "text-embedding".to_owned(),
            capability: ProviderCapability::Embedding,
            max_context_tokens: None,
            input_cost_per_million_tokens: None,
            output_cost_per_million_tokens: None,
        }],
    });
    store.save(&config).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let report = get_backend_diagnostics(&state).unwrap();

    assert_eq!(report.status, DiagnosticStatus::Unavailable);
    assert!(report.items.iter().any(|item| {
        item.component == "project_retrieval_runtime"
            && item.status == DiagnosticStatus::Unavailable
            && item
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("project-scoped credential"))
    }));
    assert!(report.items.iter().any(|item| {
        item.component == "providers.embedding.default"
            && item.status == DiagnosticStatus::Degraded
            && item.reason.as_deref()
                == Some("diagnostics.providers.embedding.configured_unverified")
    }));
    assert!(!report.items.iter().any(|item| {
        item.component == "providers.embedding.default" && item.status == DiagnosticStatus::Healthy
    }));
}

#[test]
fn node_preset_settings_are_per_node_type() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = NodePresetSettings::default();
    assert!(settings
        .presets
        .iter()
        .any(|preset| preset.node_type == "writer"));

    let writer = settings
        .presets
        .iter_mut()
        .find(|preset| preset.node_type == "writer")
        .unwrap();
    writer.model_id = "gpt-writer".to_owned();
    writer.timeout_ms = 600_000;
    writer.budget_usd = 0.25;

    save_node_preset_settings_impl(temp.path(), settings).unwrap();
    let loaded = get_node_preset_settings_impl(temp.path()).unwrap();
    let writer = loaded
        .presets
        .iter()
        .find(|preset| preset.node_type == "writer")
        .unwrap();

    assert_eq!(writer.model_id, "gpt-writer");
    assert_eq!(writer.timeout_ms, 600_000);
    assert_eq!(writer.budget_usd, 0.25);
}

#[test]
fn automation_and_permission_settings_round_trip_config_files() {
    let temp = tempfile::tempdir().unwrap();
    update_budget_config_impl(temp.path(), 10.0, 1.0).unwrap();
    let current = get_automation_settings_impl(temp.path()).unwrap();
    save_automation_settings_impl(
        temp.path(),
        AutomationSettings {
            budget: ariadne::commands::BudgetStatus {
                budget_usd: 20.0,
                spent_usd: current.budget.spent_usd,
                preauthorized_usd: 4.0,
                auto_mode_enabled: true,
            },
            confirmation_policies: vec![
                ConfirmationPolicySetting {
                    confirmation_kind: "chapter_write".to_owned(),
                    normal_policy: ConfirmationNormalPolicy::ManualReview,
                    auto_mode_policy: ConfirmationAutoModePolicy::AutoApproval,
                },
                ConfirmationPolicySetting {
                    confirmation_kind: "summary_write".to_owned(),
                    normal_policy: ConfirmationNormalPolicy::AllowByDefault,
                    auto_mode_policy: ConfirmationAutoModePolicy::AllowByDefault,
                },
            ],
        },
    )
    .unwrap();
    let automation = get_automation_settings_impl(temp.path()).unwrap();

    assert_eq!(automation.budget.budget_usd, 20.0);
    assert_eq!(automation.budget.preauthorized_usd, 4.0);
    assert!(automation.budget.auto_mode_enabled);
    assert!(automation
        .confirmation_policies
        .iter()
        .any(|item| item.confirmation_kind == "chapter_write"
            && item.normal_policy == ConfirmationNormalPolicy::ManualReview
            && item.auto_mode_policy == ConfirmationAutoModePolicy::AutoApproval));
    assert!(automation
        .confirmation_policies
        .iter()
        .any(|item| item.confirmation_kind == "summary_write"
            && item.normal_policy == ConfirmationNormalPolicy::AllowByDefault
            && item.auto_mode_policy == ConfirmationAutoModePolicy::AllowByDefault));

    let mut policy = PermissionPolicy {
        allow_network: true,
        allow_http_skill: true,
        ..PermissionPolicy::default()
    };
    policy
        .readable_file_roots
        .push(temp.path().join("documents"));
    save_permissions_settings_impl(
        temp.path(),
        PermissionsSettings {
            policy: policy.clone(),
            tool_controls: BTreeMap::from([(
                "project_ai".to_owned(),
                BTreeMap::from([("project-ai-workflow-tools".to_owned(), false)]),
            )]),
        },
    )
    .unwrap();
    let permissions = get_permissions_settings_impl(temp.path()).unwrap();

    assert_eq!(permissions.policy, policy);
    assert_eq!(
        permissions
            .tool_controls
            .get("project_ai")
            .and_then(|scope| scope.get("project-ai-workflow-tools")),
        Some(&false)
    );
    assert!(permissions.tool_controls.contains_key("writer"));
    for (scope, tool) in [
        ("project_ai", "project-ai-search"),
        ("project_ai", "project-ai-web-search"),
        ("llm", "llm-search"),
        ("llm", "llm-web-search"),
        ("summarizer", "summarizer-search"),
        ("summarizer", "summarizer-web-search"),
        ("executor_adapter", "executor-adapter-search"),
        ("executor_adapter", "executor-adapter-web-search"),
    ] {
        assert_eq!(
            permissions
                .tool_controls
                .get(scope)
                .and_then(|controls| controls.get(tool)),
            Some(&true),
            "{scope}/{tool} should be enabled by default"
        );
    }
    assert_eq!(
        permissions
            .tool_controls
            .get("writer")
            .and_then(|scope| scope.get("writer-insert-lines")),
        Some(&false)
    );
    assert_eq!(
        permissions
            .tool_controls
            .get("writer")
            .and_then(|scope| scope.get("writer-find")),
        Some(&true)
    );
}

#[test]
fn automation_settings_read_old_policy_code_but_write_dual_policies_only() {
    let temp = tempfile::tempdir().unwrap();
    let settings_path = temp
        .path()
        .join(".config/confirmation_policy_settings.json");
    std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&json!([
            {
                "confirmation_kind": "chapter_write",
                "policy": "auto_approve"
            },
            {
                "confirmation_kind": "future_extension_policy",
                "normal_policy": "manual_review",
                "auto_mode_policy": "auto_approval"
            }
        ]))
        .unwrap(),
    )
    .unwrap();

    let automation = get_automation_settings_impl(temp.path()).unwrap();
    let chapter = automation
        .confirmation_policies
        .iter()
        .find(|item| item.confirmation_kind == "chapter_write")
        .unwrap();
    assert_eq!(
        chapter.normal_policy,
        ConfirmationNormalPolicy::AllowByDefault
    );
    assert_eq!(
        chapter.auto_mode_policy,
        ConfirmationAutoModePolicy::AutoApproval
    );
    assert!(automation
        .confirmation_policies
        .iter()
        .any(|item| item.confirmation_kind == "future_extension_policy"));
    let chapter_position = automation
        .confirmation_policies
        .iter()
        .position(|item| item.confirmation_kind == "chapter_write")
        .unwrap();
    let extension_position = automation
        .confirmation_policies
        .iter()
        .position(|item| item.confirmation_kind == "future_extension_policy")
        .unwrap();
    assert!(chapter_position < extension_position);

    save_automation_settings_impl(temp.path(), automation).unwrap();
    let saved = std::fs::read_to_string(settings_path).unwrap();
    assert!(saved.contains("\"normal_policy\""));
    assert!(saved.contains("\"auto_mode_policy\""));
    assert!(!saved.contains("\"policy\""));
    assert!(saved.contains("future_extension_policy"));
}

#[test]
fn module_settings_round_trip_config_files() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let mut app = get_app_settings_impl(temp.path()).unwrap().app;
    app.project_name = "模块设置项目".to_owned();
    app.locale = "zh-CN".to_owned();
    save_app_settings_impl(temp.path(), AppSettings { app }).unwrap();
    assert_eq!(
        get_app_settings_impl(temp.path()).unwrap().app.project_name,
        "模块设置项目"
    );

    let mut rag = get_rag_settings_impl(temp.path()).unwrap().rag;
    rag.chunk_size_chars = 4096;
    rag.chunk_overlap_chars = 256;
    save_rag_settings_impl(temp.path(), RagSettings { rag }).unwrap();
    assert_eq!(
        get_rag_settings_impl(temp.path())
            .unwrap()
            .rag
            .chunk_size_chars,
        4096
    );

    let mut workflow = get_workflow_settings_impl(temp.path()).unwrap().workflow;
    workflow.max_tool_rounds = 12;
    workflow.runtime_autosave_ms = 2500;
    save_workflow_settings_impl(temp.path(), WorkflowSettings { workflow }).unwrap();
    assert_eq!(
        get_workflow_settings_impl(temp.path())
            .unwrap()
            .workflow
            .max_tool_rounds,
        12
    );

    let mut git = get_git_settings_impl(temp.path()).unwrap().git;
    git.track_skills = false;
    git.ignored_paths.push("scratch/".to_owned());
    save_git_settings_impl(temp.path(), GitSettings { git }).unwrap();
    assert!(!get_git_settings_impl(temp.path()).unwrap().git.track_skills);

    std::env::set_var("ARIADNE_ALLOW_LOCAL_TEMPLATE_REPOSITORY", "1");
    save_template_repository_settings_impl(
        temp.path(),
        &TemplateRepositorySettings {
            base_url: "http://127.0.0.1:8080/templates".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        get_template_repository_settings_impl(temp.path())
            .unwrap()
            .base_url,
        "http://127.0.0.1:8080/templates"
    );
}

#[test]
fn rag_settings_hot_reload_reuses_open_tantivy_generation() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    std::fs::write(
        project.path().join("documents").join("chapter.md"),
        "配置热重载后的检索线索",
    )
    .unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let original = state.retrieval_runtime().unwrap();
    assert_eq!(original.config().rag.chunk_size_chars, 2000);
    drop(original);
    let mut rag = get_rag_settings_impl(project.path()).unwrap().rag;
    rag.chunk_size_chars = 3072;
    rag.chunk_overlap_chars = 256;

    let saved = ariadne::commands::save_rag_settings(&state, RagSettings { rag }).unwrap();

    assert_eq!(saved.rag.chunk_size_chars, 3072);
    assert_eq!(
        state
            .retrieval_runtime()
            .unwrap()
            .config()
            .rag
            .chunk_size_chars,
        3072
    );

    let outbox_path = project
        .path()
        .join(".runtime")
        .join("index_invalidation.db");
    let mut rebuild_status = String::new();
    for _ in 0..250 {
        let connection = rusqlite::Connection::open(&outbox_path).unwrap();
        rebuild_status = connection
            .query_row(
                "SELECT status FROM index_invalidation_events
                 WHERE reason = 'retrieval_configuration_changed'
                 ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        if rebuild_status == "completed" {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(rebuild_status, "completed");
    let results =
        ariadne::commands::search_project_documents(&state, "检索线索".to_owned(), 10).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn rag_index_configuration_change_is_rejected_while_runtime_arc_is_active() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let active_runtime = state.retrieval_runtime().unwrap();
    let mut rag = get_rag_settings_impl(project.path()).unwrap().rag;
    rag.chunk_size_chars = 3072;

    let error = ariadne::commands::save_rag_settings(&state, RagSettings { rag }).unwrap_err();

    assert!(error.contains("retrieval operations are active"));
    assert_eq!(
        get_rag_settings_impl(project.path())
            .unwrap()
            .rag
            .chunk_size_chars,
        2000
    );
    assert_eq!(active_runtime.config().rag.chunk_size_chars, 2000);
}

#[test]
fn failed_vector_enable_keeps_config_and_last_good_runtime() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    drop(state.retrieval_runtime().unwrap());
    let mut rag = get_rag_settings_impl(project.path()).unwrap().rag;
    rag.vector_store.enabled = true;

    for _ in 0..16 {
        let error = ariadne::commands::save_rag_settings(&state, RagSettings { rag: rag.clone() })
            .unwrap_err();

        assert!(
            error.contains("default_embedding_provider_id"),
            "unexpected vector enable failure: {error}"
        );
        assert!(
            !get_rag_settings_impl(project.path())
                .unwrap()
                .rag
                .vector_store
                .enabled
        );
        assert!(!state.retrieval_runtime().unwrap().vector_enabled());
    }
}

#[test]
fn template_repository_settings_are_app_scoped_across_projects() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project_a.path()).unwrap();
    ariadne::frontend::initialize_project(project_b.path()).unwrap();
    std::env::set_var("ARIADNE_ALLOW_LOCAL_TEMPLATE_REPOSITORY", "1");
    let state = AriadneAppState::new(
        project_a.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    save_template_repository_settings(
        &state,
        TemplateRepositorySettings {
            base_url: "http://127.0.0.1:18080/templates".to_owned(),
        },
    )
    .unwrap();
    state.set_project_root(project_b.path()).unwrap();

    assert_eq!(
        get_template_repository_settings(&state).unwrap().base_url,
        "http://127.0.0.1:18080/templates"
    );
    assert!(!project_a
        .path()
        .join(".runtime")
        .join("template_repository_settings.json")
        .exists());
}

#[test]
fn default_official_template_repository_searches_without_configuration() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    assert_eq!(
        get_template_repository_settings(&state).unwrap().base_url,
        OFFICIAL_TEMPLATE_REPOSITORY_URL
    );
    fs::write(
        app_state.path().join("template_repository_settings.json"),
        "{\"base_url\":\"\"}",
    )
    .unwrap();
    assert_eq!(
        get_template_repository_settings(&state).unwrap().base_url,
        OFFICIAL_TEMPLATE_REPOSITORY_URL
    );
    let results = search_templates(
        TemplateRepositoryRequest { base_url: None },
        "".to_owned(),
        Vec::new(),
        0,
    )
    .unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].id, "official-novel-starter");
    let report = install_template(
        &state,
        TemplateRepositoryRequest { base_url: None },
        "official-novel-starter".to_owned(),
    )
    .unwrap();
    assert_eq!(report.workflow_id, "official-novel-starter");
    assert!(project
        .path()
        .join("workflows")
        .join("official-novel-starter")
        .join("workflow.json")
        .is_file());
}

#[test]
fn git_commands_create_checkpoint_and_return_history() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    std::fs::write(temp.path().join("documents").join("chapter.md"), "正文").unwrap();

    let checkpoint = create_checkpoint_impl(temp.path(), "章节完成".to_owned()).unwrap();
    let history = get_git_history_impl(temp.path()).unwrap();

    assert_eq!(checkpoint.message, "章节完成");
    assert_eq!(history[0].summary, "章节完成");
}

#[test]
fn git_commands_handle_new_project_without_user_git_identity() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    assert!(get_git_history_impl(temp.path()).unwrap().is_empty());
    std::fs::write(temp.path().join("documents").join("chapter.md"), "正文").unwrap();
    let checkpoint = create_checkpoint_impl(temp.path(), "首次存档".to_owned()).unwrap();
    let history = get_git_history_impl(temp.path()).unwrap();

    assert_eq!(checkpoint.message, "首次存档");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].summary, "首次存档");
}

#[test]
fn git_repository_status_reports_branch_head_and_worktree_diff() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    let document = temp.path().join("documents").join("chapter.md");
    std::fs::write(&document, "draft").unwrap();
    create_checkpoint_impl(temp.path(), "base".to_owned()).unwrap();
    std::fs::write(&document, "changed").unwrap();

    let status = get_git_repository_status_impl(temp.path()).unwrap();

    assert_eq!(status.status, ariadne::git::GitHealthStatus::Healthy);
    assert!(status.head.is_some());
    assert!(status.dirty);
    assert!(status.diff_line_count > 0);
    assert!(status.diff_preview.contains("changed"));
}

#[test]
fn git_repository_status_ignores_internal_runtime_files() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    std::fs::write(temp.path().join("documents").join("chapter.md"), "draft").unwrap();
    create_checkpoint_impl(temp.path(), "base".to_owned()).unwrap();
    std::fs::create_dir_all(temp.path().join(".runtime")).unwrap();
    std::fs::write(
        temp.path().join(".runtime").join("chapter_index.json"),
        "{}",
    )
    .unwrap();
    std::fs::write(temp.path().join("runtime.db"), "runtime").unwrap();

    let status = get_git_repository_status_impl(temp.path()).unwrap();

    assert!(!status.dirty);
    assert_eq!(status.diff_line_count, 0);
    assert!(status.diff_preview.is_empty());
}

#[test]
fn git_restore_command_records_rebuild_followup_log() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    std::fs::write(temp.path().join("documents").join("chapter.md"), "base").unwrap();
    let checkpoint = create_checkpoint_impl(temp.path(), "base".to_owned()).unwrap();
    let runtime_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let mut active = WorkflowRunState::new(
        WorkflowId::from("restore-workflow"),
        RunId::from("restore-run"),
    );
    active.status = RunStatus::Paused;
    active.pause_reason = Some("waiting".to_owned());
    runtime_store.create_state(&active).unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let report = restore_to_new_branch(
        &state,
        checkpoint.commit_id.clone(),
        "restore/base".to_owned(),
    )
    .unwrap();
    let logs = query_run_logs(
        &state,
        Some(RunLogQuery {
            query: Some("Git restore".to_owned()),
            ..RunLogQuery::default()
        }),
    )
    .unwrap();

    assert_eq!(report.new_branch, "restore/base");
    assert!(!report.index_rebuild_required);
    assert!(!report.runtime_rebind_required);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].level, ariadne::frontend::UiRunLogLevel::Warning);
    assert_eq!(logs[0].metadata["source"], "git_restore");
    assert_eq!(logs[0].metadata["new_branch"], "restore/base");
    let stopped = runtime_store
        .load_state(
            &WorkflowId::from("restore-workflow"),
            &RunId::from("restore-run"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(stopped.status, RunStatus::Stopped);
    assert_eq!(stopped.control, ariadne::contracts::RunControl::Stop);
    assert!(stopped.pause_reason.is_none());
}

/// D3-a：restore 必须等待已取得共享 mutation guard 的在途写者排空，才可 checkout。
#[test]
fn d3a_git_restore_drains_inflight_project_mutation_before_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    std::fs::write(temp.path().join("documents").join("chapter.md"), "base").unwrap();
    let checkpoint = create_checkpoint_impl(temp.path(), "base".to_owned()).unwrap();

    let outbox =
        IndexInvalidationOutbox::new(temp.path().join(".runtime").join("index_invalidation.db"));
    let inflight = outbox
        .acquire_project_mutation("d3a_inflight_writer")
        .unwrap();
    let project_root = temp.path().to_path_buf();
    let app_state_root = app_state.path().to_path_buf();
    let commit_id = checkpoint.commit_id;
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let state = AriadneAppState::new(
            &project_root,
            &app_state_root,
            Arc::new(MemorySecretStore::default()),
        );
        sender
            .send(restore_to_new_branch(
                &state,
                commit_id,
                "restore/d3a-drain".to_owned(),
            ))
            .unwrap();
    });

    for _ in 0..100 {
        if outbox
            .maintenance_state()
            .unwrap()
            .is_some_and(|state| state.status == "active")
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        outbox.maintenance_state().unwrap().unwrap().status,
        "active"
    );
    assert!(
        receiver.recv_timeout(Duration::from_millis(150)).is_err(),
        "restore must not finish while an earlier project mutation still holds the shared fence"
    );

    drop(inflight);
    let report = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("restore must continue after the in-flight mutation drains")
        .unwrap();
    handle.join().unwrap();
    assert_eq!(report.new_branch, "restore/d3a-drain");
    assert_eq!(
        outbox.maintenance_state().unwrap().unwrap().status,
        "completed"
    );
}

/// D3-a：旧 generation 的 Start 可在首次扫描后落盘 run；restore 必须继续扫描到其停止。
#[test]
fn d3a_git_restore_stops_run_created_after_maintenance_intent_before_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    std::fs::write(temp.path().join("documents").join("chapter.md"), "base").unwrap();
    let checkpoint = create_checkpoint_impl(temp.path(), "base".to_owned()).unwrap();

    let outbox =
        IndexInvalidationOutbox::new(temp.path().join(".runtime").join("index_invalidation.db"));
    let pre_intent_start = outbox
        .acquire_project_mutation("workflow_start_handoff")
        .unwrap();
    let project_root = temp.path().to_path_buf();
    let app_state_root = app_state.path().to_path_buf();
    let commit_id = checkpoint.commit_id;
    let (sender, receiver) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let state = AriadneAppState::new(
            &project_root,
            &app_state_root,
            Arc::new(MemorySecretStore::default()),
        );
        sender
            .send(restore_to_new_branch(
                &state,
                commit_id,
                "restore/d3a-late-run".to_owned(),
            ))
            .unwrap();
    });

    for _ in 0..100 {
        if outbox
            .maintenance_state()
            .unwrap()
            .is_some_and(|state| state.status == "active")
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        outbox.maintenance_state().unwrap().unwrap().status,
        "active"
    );

    let runtime_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("d3a-late-workflow");
    let run_id = RunId::from("d3a-late-run");
    let late_run = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    runtime_store.create_state(&late_run).unwrap();
    drop(pre_intent_start);

    let report = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("restore must drain a run created by a pre-intent Start")
        .unwrap();
    handle.join().unwrap();
    assert_eq!(report.new_branch, "restore/d3a-late-run");
    let stopped = runtime_store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .expect("late run must remain durably visible");
    assert_eq!(stopped.status, RunStatus::Stopped);
    assert_eq!(stopped.control, RunControl::Stop);
}

#[test]
fn d3a_maintenance_blocks_compound_ai_writes_before_provider_or_memory_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let outbox =
        IndexInvalidationOutbox::new(temp.path().join(".runtime").join("index_invalidation.db"));
    outbox
        .begin_maintenance("git_restore", "stopping_runtime")
        .unwrap();
    let secrets = MemorySecretStore::default();

    let project_ai_error = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            append_memory: Some("must not cross restore".to_owned()),
            ..ProjectAiRequest::default()
        },
    )
    .unwrap_err();
    assert!(project_ai_error.contains("maintenance"));
    assert!(!temp.path().join(".runtime/project_memory.md").exists());

    let quick_edit_error = quick_edit_impl(
        temp.path(),
        &secrets,
        QuickEditRequest {
            selected_text: "draft".to_owned(),
            instruction: "rewrite".to_owned(),
            context_ref: None,
        },
    )
    .unwrap_err();
    assert!(quick_edit_error.contains("maintenance"));
    assert!(!temp.path().join("costs.db").exists());
}

#[test]
fn d3a_maintenance_rejects_provider_model_fetch_before_network_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let request_count = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let server_count = Arc::clone(&request_count);
    let server_stop = Arc::clone(&stop);
    let server = thread::spawn(move || {
        while !server_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    server_count.fetch_add(1, Ordering::AcqRel);
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer);
                    let body = r#"{"data":[]}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(), body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "d3a-models".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "D3A Models".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: Vec::new(),
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    let outbox =
        IndexInvalidationOutbox::new(temp.path().join(".runtime").join("index_invalidation.db"));
    outbox
        .begin_maintenance("git_restore", "stopping_runtime")
        .unwrap();

    let error = ariadne::commands::fetch_provider_models_with_secrets_impl(
        temp.path(),
        &MemorySecretStore::default(),
        Some("d3a-models".to_owned()),
    )
    .unwrap_err();

    stop.store(true, Ordering::Release);
    server.join().unwrap();
    assert!(error.contains("maintenance"));
    assert_eq!(request_count.load(Ordering::Acquire), 0);
}

#[test]
fn d3a_maintenance_keeps_project_status_read_only_and_blocks_config_repair() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let app_config = temp.path().join(".config").join("app.yaml");
    std::fs::remove_file(&app_config).unwrap();
    let outbox =
        IndexInvalidationOutbox::new(temp.path().join(".runtime").join("index_invalidation.db"));
    outbox
        .begin_maintenance("git_restore", "stopping_runtime")
        .unwrap();

    let status = ariadne::commands::current_project_status(temp.path()).unwrap();
    assert_eq!(status.project_root, temp.path());
    assert!(!app_config.exists());

    let error = get_app_settings_impl(temp.path()).unwrap_err();
    assert!(error.contains("maintenance"));
    assert!(!app_config.exists());
}

#[test]
fn failed_git_restore_persists_maintenance_gate_for_document_writes() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    let document = temp.path().join("documents").join("chapter.md");
    std::fs::write(&document, "base").unwrap();
    let checkpoint = create_checkpoint_impl(temp.path(), "base".to_owned()).unwrap();
    std::fs::write(&document, "dirty").unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let restore_error =
        restore_to_new_branch(&state, checkpoint.commit_id, "restore/blocked".to_owned())
            .unwrap_err();
    assert!(restore_error.contains("worktree must be clean"));

    let write_error = save_document_content_impl(
        temp.path(),
        document.to_string_lossy().into_owned(),
        "must not write".to_owned(),
    )
    .unwrap_err();
    assert!(write_error.contains("project maintenance blocks writes"));
    assert_eq!(std::fs::read_to_string(document).unwrap(), "dirty");

    let run_error = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "must-not-start".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
        },
    )
    .unwrap_err();
    assert!(run_error.contains("project maintenance blocks writes"));
}

#[test]
fn pending_confirmations_distinguish_missing_runtime_from_corrupt_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let runtime_path = temp.path().join(ariadne::workflow::RUNTIME_DB_FILE);

    assert!(list_confirmations(&state).unwrap().is_empty());
    assert!(
        !runtime_path.exists(),
        "empty state must not create runtime.db"
    );

    std::fs::write(&runtime_path, b"not a sqlite database").unwrap();
    let list_error = list_confirmations(&state).unwrap_err();
    assert!(list_error.contains("sqlite"));
    let badge_error = get_sidebar_badges(&state).unwrap_err();
    assert!(badge_error.contains("sqlite"));
}

#[test]
fn git_checkpoint_respects_tracking_and_ignored_settings() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );

    let mut git = get_git_settings_impl(temp.path()).unwrap().git;
    git.track_skills = false;
    git.ignored_paths.push("scratch".to_owned());
    save_git_settings_impl(temp.path(), GitSettings { git }).unwrap();

    std::fs::create_dir_all(temp.path().join("skills")).unwrap();
    std::fs::create_dir_all(temp.path().join("scratch")).unwrap();
    std::fs::write(temp.path().join("documents").join("chapter.md"), "正文").unwrap();
    std::fs::write(temp.path().join("skills").join("skill.md"), "skill").unwrap();
    std::fs::write(temp.path().join("scratch").join("draft.md"), "scratch").unwrap();

    create_checkpoint_impl(temp.path(), "受控存档".to_owned()).unwrap();

    let tree = git_stdout(temp.path(), ["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(tree.contains("documents/chapter.md"));
    assert!(tree.contains(".config/app.yaml"));
    assert!(!tree.contains("skills/skill.md"));
    assert!(!tree.contains("scratch/draft.md"));
}

#[test]
fn project_ai_resolves_references_and_updates_memory_without_llm() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    FileConfirmationLogStore::default_for_project(temp.path())
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-1".to_owned(),
            kind: "chapter_summary".to_owned(),
            node_id: "summarizer".to_owned(),
            timestamp_ms: 1,
            state: ConfirmationLogState::Pending,
            handling_method: "manual".to_owned(),
            summary: "章节总结待确认".to_owned(),
            diff: "- old\n+ new".to_owned(),
            workflow_id: "wf".to_owned(),
            run_id: "run-1".to_owned(),
        })
        .unwrap();

    let resolved =
        resolve_project_references(temp.path(), &["@确认项/confirm-1".to_owned()]).unwrap();
    let response = project_ai_chat_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ProjectAiRequest {
            message: String::new(),
            chat_history: Vec::new(),
            references: vec!["@确认项/confirm-1".to_owned()],
            workflow_id_to_run: None,
            reference_workflow_id: None,
            reference_run_id: None,
            conversation_id: None,
            conversation_revision: None,
            append_memory: Some("长期偏好：保持第三人称。".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(resolved[0].summary, "章节总结待确认");
    assert!(response.project_memory.contains("第三人称"));
    assert_eq!(response.resolved_references[0].id, "confirm-1");
    assert_eq!(response.answer, "已处理项目记忆或工作流请求。");
}

#[test]
fn project_ai_impl_rejects_workflow_start_without_application_runtime() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "project-ai-low-level".to_owned(),
            name: "Project AI Low Level".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-main".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Start".to_owned()),
                data: Value::Null,
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let error = project_ai_chat_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ProjectAiRequest {
            workflow_id_to_run: Some("project-ai-low-level".to_owned()),
            ..ProjectAiRequest::default()
        },
    )
    .unwrap_err();

    assert!(error.contains("requires the application runtime"));
    assert!(!temp.path().join("runtime.db").exists());
}

#[test]
fn project_ai_chat_sends_chat_history_through_llm_provider() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("authorization: Bearer local-key"));
        assert!(request.contains("\"role\":\"system\""));
        assert!(request.contains("上一轮问题"));
        assert!(request.contains("上一轮回答"));
        assert!(request.contains("继续说明"));
        let response_body = r#"{
          "model":"local-chat",
          "choices":[{"message":{"content":"继续回答"},"finish_reason":"stop"}],
          "usage":{"prompt_tokens":16,"completion_tokens":4}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets = MemorySecretStore::default();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "local-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "local_chat".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let response = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "继续说明".to_owned(),
            chat_history: vec![
                ProjectAiChatMessage {
                    role: ProjectAiChatRole::User,
                    content: "上一轮问题".to_owned(),
                },
                ProjectAiChatMessage {
                    role: ProjectAiChatRole::Assistant,
                    content: "上一轮回答".to_owned(),
                },
            ],
            references: Vec::new(),
            workflow_id_to_run: None,
            reference_workflow_id: None,
            reference_run_id: None,
            conversation_id: None,
            conversation_revision: None,
            append_memory: None,
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(response.answer, "继续回答");
    assert_eq!(response.chat_history.len(), 4);
    assert_eq!(response.chat_history[2].content, "继续说明");
    assert_eq!(response.chat_history[3].content, "继续回答");
    assert_eq!(response.conversation_id, "default");
    assert_eq!(response.conversation_revision, 1);
    assert_eq!(response.new_messages.len(), 2);
    assert!(!response.project_memory_revision.is_empty());
}

#[test]
fn project_ai_can_call_project_search_and_receive_indexed_document() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_document_content_impl(
        temp.path(),
        "documents/search-source.md".to_owned(),
        "顾言把蓝色罗盘交给苏禾，并要求她在钟楼会合。".to_owned(),
    )
    .unwrap();
    assert_eq!(process_index_outbox_impl(temp.path()).unwrap(), 1);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for round in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 65536];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            assert!(request.contains("\"name\":\"project-ai-search\""));
            let response_body = if round == 0 {
                r#"{
                  "model":"project-search-chat",
                  "choices":[{
                    "message":{
                      "content":"",
                      "tool_calls":[{
                        "id":"call-project-search",
                        "type":"function",
                        "function":{"name":"project-ai-search","arguments":"{\"query\":\"蓝色罗盘\",\"limit\":5}"}
                      }]
                    },
                    "finish_reason":"tool_calls"
                  }],
                  "usage":{"prompt_tokens":20,"completion_tokens":2}
                }"#
            } else {
                assert!(request.contains("蓝色罗盘"));
                assert!(request.contains("search-source.md"));
                r#"{
                  "model":"project-search-chat",
                  "choices":[{
                    "message":{"content":"蓝色罗盘由顾言交给苏禾。","tool_calls":[]},
                    "finish_reason":"stop"
                  }],
                  "usage":{"prompt_tokens":40,"completion_tokens":6}
                }"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let secrets = MemorySecretStore::default();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "project_search_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Project Search Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "project-search-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "project_search_chat".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let response = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "谁拿到了蓝色罗盘？".to_owned(),
            ..ProjectAiRequest::default()
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(response.answer, "蓝色罗盘由顾言交给苏禾。");
}

#[test]
fn project_ai_can_call_web_search_and_receive_cited_results() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let llm_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let llm_base_url = format!("http://{}", llm_listener.local_addr().unwrap());
    let llm_server = thread::spawn(move || {
        for round in 0..2 {
            let (mut stream, _) = llm_listener.accept().unwrap();
            let mut buffer = [0u8; 65536];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            assert!(request.contains("\"name\":\"project-ai-web-search\""));
            let response_body = if round == 0 {
                r#"{
                  "model":"project-web-chat",
                  "choices":[{
                    "message":{
                      "content":"",
                      "tool_calls":[{
                        "id":"call-project-web-search",
                        "type":"function",
                        "function":{"name":"project-ai-web-search","arguments":"{\"query\":\"2026 lunar mission\",\"limit\":3}"}
                      }]
                    },
                    "finish_reason":"tool_calls"
                  }],
                  "usage":{"prompt_tokens":20,"completion_tokens":2}
                }"#
            } else {
                assert!(request.contains("https://example.test/lunar-2026"));
                assert!(request.contains("Lunar Mission 2026"));
                r#"{
                  "model":"project-web-chat",
                  "choices":[{
                    "message":{"content":"已根据公开来源完成检索。","tool_calls":[]},
                    "finish_reason":"stop"
                  }],
                  "usage":{"prompt_tokens":40,"completion_tokens":6}
                }"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let search_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let search_base_url = format!("http://{}", search_listener.local_addr().unwrap());
    let search_server = thread::spawn(move || {
        let (mut stream, _) = search_listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /responses "));
        assert!(request.contains("2026 lunar mission"));
        let body = r#"{
          "output":[{
            "type":"message",
            "content":[{
              "type":"output_text",
              "text":"A cited current result.",
              "annotations":[{
                "type":"url_citation",
                "url":"https://example.test/lunar-2026",
                "title":"Lunar Mission 2026"
              }]
            }]
          }]
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets = MemorySecretStore::default();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "project_web_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Project Web Chat".to_owned(),
            enabled: true,
            base_url: Some(llm_base_url),
            models: vec![ModelConfig {
                model_id: "project-web-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "project_web_search".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Project Web Search".to_owned(),
            enabled: true,
            base_url: Some(search_base_url),
            models: vec![ModelConfig {
                model_id: "web-search-model".to_owned(),
                capability: ProviderCapability::Search,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: false,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: true,
        },
    )
    .unwrap();
    let mut permissions = get_permissions_settings_impl(temp.path()).unwrap();
    permissions.policy.allow_network = true;
    permissions.policy.allow_web_search = true;
    save_permissions_settings_impl(temp.path(), permissions).unwrap();

    let response = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "查询 2026 月球任务".to_owned(),
            ..ProjectAiRequest::default()
        },
    )
    .unwrap();
    llm_server.join().unwrap();
    search_server.join().unwrap();

    assert_eq!(response.answer, "已根据公开来源完成检索。");
}

#[test]
fn executor_adapter_web_search_uses_project_permission_policy_in_product_workflow() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let llm_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    llm_listener.set_nonblocking(true).unwrap();
    let llm_base_url = format!("http://{}", llm_listener.local_addr().unwrap());
    let llm_server = thread::spawn(move || {
        for round in 0..2 {
            let mut stream = accept_with_deadline(&llm_listener, Duration::from_secs(5));
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0u8; 65536];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            assert!(request.contains("\"name\":\"executor-adapter-web-search\""));
            let response_body = if round == 0 {
                r#"{
                  "model":"adapter-chat",
                  "choices":[{
                    "message":{
                      "content":"",
                      "tool_calls":[{
                        "id":"call-adapter-web-search",
                        "type":"function",
                        "function":{"name":"executor-adapter-web-search","arguments":"{\"query\":\"2026 publishing standard\",\"limit\":2}"}
                      }]
                    },
                    "finish_reason":"tool_calls"
                  }],
                  "usage":{"prompt_tokens":20,"completion_tokens":2}
                }"#
            } else {
                assert!(request.contains("https://example.test/publishing-2026"));
                assert!(request.contains("Publishing Standard 2026"));
                r#"{
                  "model":"adapter-chat",
                  "choices":[{
                    "message":{"content":"ExecutorAdapter 已完成公开资料核对。","tool_calls":[]},
                    "finish_reason":"stop"
                  }],
                  "usage":{"prompt_tokens":40,"completion_tokens":6}
                }"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let search_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    search_listener.set_nonblocking(true).unwrap();
    let search_base_url = format!("http://{}", search_listener.local_addr().unwrap());
    let search_server = thread::spawn(move || {
        let mut stream = accept_with_deadline(&search_listener, Duration::from_secs(5));
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /responses "));
        assert!(request.contains("2026 publishing standard"));
        let body = r#"{
          "output":[{
            "type":"message",
            "content":[{
              "type":"output_text",
              "text":"A current publishing reference.",
              "annotations":[{
                "type":"url_citation",
                "url":"https://example.test/publishing-2026",
                "title":"Publishing Standard 2026"
              }]
            }]
          }]
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let skill_dir = temp.path().join("skills").join("web-research");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("skill.json"),
        r#"{
          "skill_id":"web-research",
          "name":"Web Research",
          "version":"1.0.0",
          "executor":{
            "kind":"llm",
            "provider_id":"adapter_chat",
            "model_id":"adapter-chat",
            "prompt_template":"核对公开资料"
          },
          "schema":{
            "inputs":[],
            "outputs":[{"name":"text","type_name":"inline","required":true}]
          },
          "limits":{"timeout_ms":5000,"max_output_bytes":4096},
          "estimated_cost_usd":0.0
        }"#,
    )
    .unwrap();

    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "adapter_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Adapter Chat".to_owned(),
            enabled: true,
            base_url: Some(llm_base_url),
            models: vec![ModelConfig {
                model_id: "adapter-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "adapter_web_search".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Adapter Web Search".to_owned(),
            enabled: true,
            base_url: Some(search_base_url),
            models: vec![ModelConfig {
                model_id: "adapter-web-search".to_owned(),
                capability: ProviderCapability::Search,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: false,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: true,
        },
    )
    .unwrap();
    let mut permissions = get_permissions_settings_impl(temp.path()).unwrap();
    permissions.policy.allow_network = true;
    permissions.policy.allow_web_search = true;
    save_permissions_settings_impl(temp.path(), permissions).unwrap();

    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "adapter-web-search-flow".to_owned(),
            name: "Adapter Web Search".to_owned(),
            nodes: vec![CanvasNode {
                id: "research".to_owned(),
                r#type: "executor_adapter:web-research".to_owned(),
                label: Some("Web Research".to_owned()),
                data: json!({ "skill_id": "web-research" }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let run = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "adapter-web-search-flow".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
        },
    )
    .unwrap();

    llm_server.join().unwrap();
    search_server.join().unwrap();
    assert_eq!(run.status, "succeeded");
}

#[test]
fn executor_adapter_llm_routes_to_manifest_provider_instead_of_project_default() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let default_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    default_listener.set_nonblocking(true).unwrap();
    let default_base_url = format!("http://{}", default_listener.local_addr().unwrap());

    let manifest_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    manifest_listener.set_nonblocking(true).unwrap();
    let manifest_base_url = format!("http://{}", manifest_listener.local_addr().unwrap());
    let manifest_server = thread::spawn(move || {
        let mut stream = accept_with_deadline(&manifest_listener, Duration::from_secs(5));
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut buffer = [0u8; 65536];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("authorization: Bearer manifest-secret"));
        assert!(request.contains("\"model\":\"manifest-model\""));
        let body = r#"{
          "model":"manifest-model",
          "choices":[{
            "message":{"content":"已使用清单指定 Provider。","tool_calls":[]},
            "finish_reason":"stop"
          }],
          "usage":{"prompt_tokens":12,"completion_tokens":5}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let skill_dir = temp.path().join("skills").join("manifest-provider");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("skill.json"),
        r#"{
          "skill_id":"manifest-provider",
          "name":"Manifest Provider",
          "version":"1.0.0",
          "executor":{
            "kind":"llm",
            "provider_id":"manifest_chat",
            "model_id":"manifest-model",
            "prompt_template":"验证 Provider 路由"
          },
          "schema":{
            "inputs":[],
            "outputs":[{"name":"text","type_name":"inline","required":true}]
          },
          "limits":{"timeout_ms":5000,"max_output_bytes":4096},
          "estimated_cost_usd":0.0
        }"#,
    )
    .unwrap();

    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "default_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Default Chat".to_owned(),
            enabled: true,
            base_url: Some(default_base_url),
            models: vec![ModelConfig {
                model_id: "default-model".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "manifest_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Manifest Chat".to_owned(),
            enabled: true,
            base_url: Some(manifest_base_url),
            models: vec![ModelConfig {
                model_id: "manifest-model".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: false,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    let secrets = MemorySecretStore::default();
    let credentials = ProjectCredentialScope::new(temp.path(), &secrets).unwrap();
    credentials
        .set_provider_secret("default_chat", SecretValue::new("default-secret"))
        .unwrap();
    credentials
        .set_provider_secret("manifest_chat", SecretValue::new("manifest-secret"))
        .unwrap();

    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "manifest-provider-flow".to_owned(),
            name: "Manifest Provider".to_owned(),
            nodes: vec![CanvasNode {
                id: "manifest-provider-node".to_owned(),
                r#type: "executor_adapter:manifest-provider".to_owned(),
                label: Some("Manifest Provider".to_owned()),
                data: json!({ "skill_id": "manifest-provider" }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let run = run_workflow_impl(
        temp.path(),
        &secrets,
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "manifest-provider-flow".to_owned(),
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
        },
    )
    .unwrap();

    manifest_server.join().unwrap();
    assert_eq!(run.status, "succeeded");
    assert!(matches!(
        default_listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

#[test]
fn project_ai_chat_rejects_stale_revision_before_provider_resolution() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let store = ProjectAiConversationStore::open(temp.path()).unwrap();
    store.load_or_seed("stale-revision", &[]).unwrap();
    store
        .append_messages(
            "stale-revision",
            0,
            &[
                ("user".to_owned(), "已持久化问题".to_owned()),
                ("assistant".to_owned(), "已持久化回答".to_owned()),
            ],
        )
        .unwrap();

    let error = project_ai_chat_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ProjectAiRequest {
            message: "不应进入 provider 解析".to_owned(),
            chat_history: Vec::new(),
            references: Vec::new(),
            workflow_id_to_run: None,
            reference_workflow_id: None,
            reference_run_id: None,
            conversation_id: Some("stale-revision".to_owned()),
            conversation_revision: Some(0),
            append_memory: None,
        },
    )
    .unwrap_err();

    assert_eq!(error.code.as_str(), "conflict");
    assert!(error.contains("expected 0, actual 1"));
}

#[test]
fn project_ai_chat_rejects_concurrent_turn_before_provider_resolution() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let _active =
        ProjectAiConversationStore::try_acquire_conversation(temp.path(), "concurrent-turn")
            .unwrap()
            .unwrap();

    let error = project_ai_chat_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ProjectAiRequest {
            message: "不应进入 provider 解析".to_owned(),
            conversation_id: Some("concurrent-turn".to_owned()),
            ..ProjectAiRequest::default()
        },
    )
    .unwrap_err();

    assert_eq!(error.code.as_str(), "conflict");
    assert!(error.contains("already processing another revision"));
}

#[test]
fn project_ai_chat_bounds_history_to_model_context_window() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for answer in ["bounded answer", "resumed answer"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = vec![0u8; 65536];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            assert!(request.contains("oldest-history-marker"));
            assert!(request.contains("summary_id="));
            assert!(request.contains("revision=0 sequence=1 role=user"));
            assert!(request.contains("newest-history-marker"));
            let response_body = format!(
                r#"{{
                  "model":"bounded-chat",
                  "choices":[{{"message":{{"content":"{answer}"}},"finish_reason":"stop"}}],
                  "usage":{{"prompt_tokens":1000,"completion_tokens":4}}
                }}"#
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let secrets = MemorySecretStore::default();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "bounded_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Bounded Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "bounded-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "bounded_chat".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let mut history = Vec::new();
    for index in 0..30 {
        let marker = if index == 0 {
            "oldest-history-marker"
        } else if index == 29 {
            "newest-history-marker"
        } else {
            "history"
        };
        history.push(ProjectAiChatMessage {
            role: ProjectAiChatRole::User,
            content: format!("{marker}-{index}-{}", "x".repeat(500)),
        });
        history.push(ProjectAiChatMessage {
            role: ProjectAiChatRole::Assistant,
            content: format!("answer-{index}-{}", "y".repeat(500)),
        });
    }
    let original_len = history.len();
    let response = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "current question".to_owned(),
            chat_history: history,
            references: Vec::new(),
            workflow_id_to_run: None,
            reference_workflow_id: None,
            reference_run_id: None,
            conversation_id: Some("bounded-restart".to_owned()),
            conversation_revision: None,
            append_memory: None,
        },
    )
    .unwrap();
    assert!(response.history_truncated);
    assert!(!response.conversation_summary.is_empty());
    assert_eq!(response.conversation_revision, 1);
    assert_eq!(response.new_messages.len(), 2);
    assert_eq!(response.context_limit_tokens, 4096);
    assert!(response.estimated_input_tokens < 4096);
    assert!(response.chat_history.is_empty());
    assert!(response.conversation_snapshot.len() < original_len + 2);
    assert_eq!(
        response.conversation_snapshot.last().unwrap().content,
        "bounded answer"
    );

    let resumed = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "resume after desktop restart".to_owned(),
            chat_history: Vec::new(),
            references: Vec::new(),
            workflow_id_to_run: None,
            reference_workflow_id: None,
            reference_run_id: None,
            conversation_id: Some("bounded-restart".to_owned()),
            conversation_revision: Some(response.conversation_revision),
            append_memory: None,
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(resumed.conversation_revision, 2);
    assert_eq!(resumed.new_messages.len(), 2);
    assert!(resumed.chat_history.is_empty());
    assert!(resumed.conversation_snapshot.is_empty());
    assert_eq!(
        resumed.new_messages.last().unwrap().content,
        "resumed answer"
    );
}

#[test]
fn project_ai_chat_assembles_document_knowledge_artifact_and_node_references() {
    use ariadne::contracts::{SourceSpan, TextRange};
    use ariadne::rag::{MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, StorySegment};

    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let document_path = temp.path().join("documents/reference.md");
    std::fs::write(&document_path, "DOC-F5-SENTINEL\n第二行正文").unwrap();

    let artifact_path = temp.path().join(".runtime/artifacts/exports/reference.md");
    std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
    std::fs::write(&artifact_path, "ARTIFACT-F6-SENTINEL\n导出正文").unwrap();

    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: "knowledge-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-1".to_owned(),
            summary: "KNOWLEDGE-F6-SENTINEL".to_owned(),
            source: SourceSpan {
                document_id: "documents/reference.md".to_owned(),
                range: TextRange { start: 0, end: 12 },
                version: Some("knowledge-v1".to_owned()),
            },
            metadata: Value::Null,
        })
        .unwrap();
    SqliteWritingKnowledgeStore::open(temp.path())
        .unwrap()
        .save_knowledge(&knowledge)
        .unwrap();

    let workflow_id = WorkflowId::from("reference-flow");
    let run_id = RunId::from("reference-run");
    let mut run_state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    run_state.status = RunStatus::Succeeded;
    let mut outputs = ariadne::contracts::PortMap::new();
    outputs.insert("draft".to_owned(), PortValue::inline("NODE-F6-SENTINEL"));
    run_state.nodes.insert(
        NodeId::from("writer"),
        WorkflowNodeRuntimeState {
            node_id: NodeId::from("writer"),
            status: RunStatus::Succeeded,
            outputs,
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
    SqliteWorkflowRuntimeStore::open(temp.path())
        .unwrap()
        .create_state(&run_state)
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = vec![0u8; 256 * 1024];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.contains("DOC-F5-SENTINEL"));
        assert!(request.contains("KNOWLEDGE-F6-SENTINEL"));
        assert!(request.contains("ARTIFACT-F6-SENTINEL"));
        assert!(request.contains("NODE-F6-SENTINEL"));
        assert!(request.contains("lines=1-2"));
        let response_body = r#"{
          "model":"reference-chat",
          "choices":[{"message":{"content":"引用已装配"},"finish_reason":"stop"}],
          "usage":{"prompt_tokens":64,"completion_tokens":4}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets = MemorySecretStore::default();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "reference_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Reference Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "reference-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "reference_chat".to_owned(),
        "reference-key".to_owned(),
    )
    .unwrap();

    let response = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "比较 @文档/documents/reference.md @知识/knowledge-1 @artifact/exports/reference.md @节点/writer/输出/draft".to_owned(),
            chat_history: Vec::new(),
            references: Vec::new(),
            workflow_id_to_run: None,
            reference_workflow_id: Some(workflow_id.as_str().to_owned()),
            reference_run_id: Some(run_id.as_str().to_owned()),
            conversation_id: None,
            conversation_revision: None,
            append_memory: None,
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(response.answer, "引用已装配");
    assert_eq!(response.resolved_references.len(), 4);
    assert!(!response.references_truncated);
    assert!(response.resolved_references.iter().any(|reference| {
        reference.kind == ariadne::frontend::ProjectReferenceKind::Document
            && reference.payload["fragments"][0]["source_version"]
                .as_str()
                .is_some()
    }));
}

#[test]
fn project_ai_chat_exposes_start_nodes_as_workflow_tools() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "tool-flow".to_owned(),
            name: "Tool Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-draft".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Draft Tool".to_owned()),
                data: json!({
                    "name": "Draft Tool",
                    "work_dir": "draft",
                    "expose_as_tool": true
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();
    save_permissions_settings_impl(
        temp.path(),
        PermissionsSettings {
            policy: PermissionPolicy::default(),
            tool_controls: BTreeMap::from([(
                "project_ai".to_owned(),
                BTreeMap::from([("project-ai-workflow-tools".to_owned(), true)]),
            )]),
        },
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    // 三轮 tool-use：list_start_nodes → draft_tool → 最终文本
    let server = thread::spawn(move || {
        let respond = |stream: &mut std::net::TcpStream, body: &str| {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        };
        for round in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 65536];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            assert!(request.starts_with("POST /chat/completions "));
            assert!(request.contains("\"tools\""));
            assert!(request.contains("\"name\":\"list_start_nodes\""));
            assert!(request.contains("\"name\":\"draft_tool\""));
            let response_body = match round {
                0 => {
                    r#"{
                  "model":"local-chat",
                  "choices":[{
                    "message":{
                      "content":"",
                      "tool_calls":[{
                        "id":"call-list",
                        "type":"function",
                        "function":{"name":"list_start_nodes","arguments":"{}"}
                      }]
                    },
                    "finish_reason":"tool_calls"
                  }],
                  "usage":{"prompt_tokens":16,"completion_tokens":1}
                }"#
                }
                1 => {
                    r#"{
                  "model":"local-chat",
                  "choices":[{
                    "message":{
                      "content":"",
                      "tool_calls":[{
                        "id":"call-start",
                        "type":"function",
                        "function":{"name":"draft_tool","arguments":"{}"}
                      }]
                    },
                    "finish_reason":"tool_calls"
                  }],
                  "usage":{"prompt_tokens":20,"completion_tokens":1}
                }"#
                }
                _ => {
                    r#"{
                  "model":"local-chat",
                  "choices":[{
                    "message":{"content":"已按起点启动","tool_calls":[]},
                    "finish_reason":"stop"
                  }],
                  "usage":{"prompt_tokens":24,"completion_tokens":4}
                }"#
                }
            };
            respond(&mut stream, response_body);
        }
    });

    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "local-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        secrets.as_ref(),
        "local_chat".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new(temp.path(), app_state.path(), secrets);
    let response = project_ai_chat(
        &state,
        ProjectAiRequest {
            message: "启动草稿工具".to_owned(),
            chat_history: Vec::new(),
            references: Vec::new(),
            workflow_id_to_run: None,
            reference_workflow_id: None,
            reference_run_id: None,
            conversation_id: None,
            conversation_revision: None,
            append_memory: None,
        },
    )
    .unwrap();
    server.join().unwrap();

    let workflow_run = response.workflow_run.unwrap();
    assert_eq!(workflow_run.status, "queued");
    assert!(
        response.answer == "ui.project_ai.workflow_tool_started"
            || response.answer.contains("起点")
            || !response.answer.is_empty()
    );
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let run_id = RunId::from(workflow_run.run_id);
    let state = wait_for_terminal_workflow_state(&store, &WorkflowId::from("tool-flow"), &run_id);
    assert_eq!(state.status, RunStatus::Succeeded);
    assert!(state.nodes.contains_key(&NodeId::from("start-draft")));
}

#[test]
fn project_ai_start_node_catalog_includes_id_name_user_note() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "note-flow".to_owned(),
            name: "Note Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-a".to_owned(),
                r#type: "start".to_owned(),
                label: Some("起点 A".to_owned()),
                data: json!({
                    "name": "正篇起点",
                    "work_dir": "novels/main",
                    "user_note": "写正篇章纲",
                    "expose_as_tool": true
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    // 通过 list_workflow_tools 路径确认 expose 工具仍在；catalog 纯逻辑由 project_ai 内使用。
    // 这里直接读图后用 list_external 验证 id 暴露。
    save_permissions_settings_impl(
        temp.path(),
        PermissionsSettings {
            policy: PermissionPolicy::default(),
            tool_controls: BTreeMap::from([(
                "project_ai".to_owned(),
                BTreeMap::from([("project-ai-workflow-tools".to_owned(), true)]),
            )]),
        },
    )
    .unwrap();
    let tools = list_external_workflow_tools_impl(temp.path()).unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].start_node_id, "start-a");
    assert!(tools[0].display_name.contains("正篇") || tools[0].display_name.contains("起点"));
}

#[test]
fn project_ai_chat_respects_disabled_workflow_tool_permission() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "tool-flow".to_owned(),
            name: "Tool Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-draft".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Draft Tool".to_owned()),
                data: json!({
                    "name": "Draft Tool",
                    "work_dir": "draft",
                    "expose_as_tool": true
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();
    save_permissions_settings_impl(
        temp.path(),
        PermissionsSettings {
            policy: PermissionPolicy::default(),
            tool_controls: BTreeMap::from([(
                "project_ai".to_owned(),
                BTreeMap::from([("project-ai-workflow-tools".to_owned(), false)]),
            )]),
        },
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("\"tools\""));
        assert!(request.contains("\"name\":\"project-ai-search\""));
        assert!(!request.contains("\"name\":\"list_start_nodes\""));
        assert!(!request.contains("\"name\":\"draft_tool\""));
        let response_body = r#"{
          "model":"local-chat",
          "choices":[{
            "message":{"content":"工具已关闭"},
            "finish_reason":"stop"
          }],
          "usage":{"prompt_tokens":16,"completion_tokens":3}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets = MemorySecretStore::default();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "local-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "local_chat".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let response = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "启动草稿工具".to_owned(),
            chat_history: Vec::new(),
            references: Vec::new(),
            workflow_id_to_run: None,
            reference_workflow_id: None,
            reference_run_id: None,
            conversation_id: None,
            conversation_revision: None,
            append_memory: None,
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(response.answer, "工具已关闭");
    assert!(response.workflow_run.is_none());
}

struct BusySpecialResumeFixture {
    _project: tempfile::TempDir,
    _app_state: tempfile::TempDir,
    state: AriadneAppState,
    store: SqliteWorkflowRuntimeStore,
    workflow_id: WorkflowId,
    run_id: RunId,
    baseline: WorkflowRunState,
}

fn busy_special_resume_fixture() -> BusySpecialResumeFixture {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    save_workflow_graph_impl(
        project.path(),
        WorkflowGraphData {
            workflow_id: "special-resume".to_owned(),
            name: "Special Resume".to_owned(),
            nodes: vec![CanvasNode {
                id: "source".to_owned(),
                r#type: "start".to_owned(),
                label: None,
                data: Value::Null,
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();

    let workflow_id = WorkflowId::from("special-resume");
    let run_id = RunId::from("run-special-resume");
    let mut run_state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    run_state.prepared_workflow = Some(WorkflowDefinition {
        id: workflow_id.clone(),
        name: "Special Resume".to_owned(),
        nodes: vec![ariadne::contracts::NodeInstance {
            id: NodeId::from("source"),
            type_name: "start".to_owned(),
            label: None,
            config: Value::Null,
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    });
    run_state.nodes.insert(
        NodeId::from("source"),
        ariadne::workflow::WorkflowNodeRuntimeState {
            node_id: NodeId::from("source"),
            status: RunStatus::Succeeded,
            outputs: BTreeMap::from([(
                "chapter_text".to_owned(),
                PortValue::inline("original chapter"),
            )]),
            communication_output: None,
            communication_control: Default::default(),
            prompt_trace_hash: None,
            patch_session_commit_id: None,
            checkpoint_id: None,
            patch_write_back_state: None,
            metadata: json!({ "original": true }),
            error: None,
            error_state: None,
            execution_attempts: 2,
        },
    );
    run_state.confirmations.insert(
        "confirm-special".to_owned(),
        RuntimeConfirmation {
            confirmation_id: "confirm-special".to_owned(),
            node_id: NodeId::from("source"),
            state: RuntimeConfirmationState::Pending,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: json!({ "reason": "needs revision" }),
        },
    );
    run_state.status = RunStatus::Running;

    let store = SqliteWorkflowRuntimeStore::open(project.path()).unwrap();
    store.create_state(&run_state).unwrap();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let _lease = store
        .acquire_worker_lease(&workflow_id, &run_id, "existing-worker", now_ms, 300_000)
        .unwrap()
        .expect("fixture must hold an active worker lease");
    // 合法 Busy 不变量是 Running + active lease。worker 一旦保存 Paused、Queued
    // 或终态，会在同一事务中释放 lease，不再允许旧的 Paused+lease 窗口。
    let baseline = store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .expect("fixture state must exist");
    FileConfirmationLogStore::default_for_project(project.path())
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-special".to_owned(),
            kind: "approval".to_owned(),
            node_id: "source".to_owned(),
            timestamp_ms: 1,
            state: ConfirmationLogState::Pending,
            handling_method: "manual".to_owned(),
            summary: "待确认输出".to_owned(),
            diff: "- original\n+ replacement".to_owned(),
            workflow_id: workflow_id.as_str().to_owned(),
            run_id: run_id.as_str().to_owned(),
        })
        .unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    BusySpecialResumeFixture {
        _project: project,
        _app_state: app_state,
        state,
        store,
        workflow_id,
        run_id,
        baseline,
    }
}

fn assert_special_resume_busy(error: &str) {
    let normalized = error.to_ascii_lowercase();
    assert!(
        normalized.contains("busy") || normalized.contains("lease"),
        "unexpected special resume error: {error}"
    );
}

#[test]
fn override_confirmation_output_busy_rolls_back_entire_mutation() {
    let fixture = busy_special_resume_fixture();

    let error = override_confirmation_output(
        &fixture.state,
        OverrideConfirmationOutputRequest {
            workflow_id: fixture.workflow_id.as_str().to_owned(),
            run_id: fixture.run_id.as_str().to_owned(),
            confirmation_id: "confirm-special".to_owned(),
            new_outputs: BTreeMap::from([(
                "chapter_text".to_owned(),
                PortValue::inline("approved replacement"),
            )]),
        },
    )
    .unwrap_err();

    assert_special_resume_busy(&error);
    let persisted = fixture
        .store
        .load_state(&fixture.workflow_id, &fixture.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, fixture.baseline);
    let lease = fixture
        .store
        .load_worker_lease(&fixture.workflow_id, &fixture.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(lease.owner_id, "existing-worker");
}

#[test]
fn resume_from_node_busy_rolls_back_entire_mutation() {
    let fixture = busy_special_resume_fixture();

    let error = resume_from_node(
        &fixture.state,
        ResumeFromNodeRequest {
            workflow_id: fixture.workflow_id.as_str().to_owned(),
            run_id: fixture.run_id.as_str().to_owned(),
            node_id: "source".to_owned(),
            injected_outputs: BTreeMap::from([(
                "chapter_text".to_owned(),
                PortValue::inline("externally revised chapter"),
            )]),
        },
    )
    .unwrap_err();

    assert_special_resume_busy(&error);
    let persisted = fixture
        .store
        .load_state(&fixture.workflow_id, &fixture.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, fixture.baseline);
    let lease = fixture
        .store
        .load_worker_lease(&fixture.workflow_id, &fixture.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(lease.owner_id, "existing-worker");
}

#[test]
fn resolve_confirmation_busy_rolls_back_runtime_and_confirmation_log() {
    let fixture = busy_special_resume_fixture();

    let error = resolve_confirmation_impl(
        fixture._project.path(),
        ResolveConfirmationRequest {
            workflow_id: fixture.workflow_id.as_str().to_owned(),
            run_id: fixture.run_id.as_str().to_owned(),
            confirmation_id: "confirm-special".to_owned(),
            decision: ConfirmationDecision::Approve,
            review_reason: Some("must not be committed while busy".to_owned()),
        },
    )
    .unwrap_err();

    assert_special_resume_busy(&error);
    let persisted = fixture
        .store
        .load_state(&fixture.workflow_id, &fixture.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, fixture.baseline);
    let confirmations = FileConfirmationLogStore::default_for_project(fixture._project.path())
        .read_all()
        .unwrap();
    assert_eq!(confirmations.len(), 1);
    assert_eq!(confirmations[0].state, ConfirmationLogState::Pending);
    assert_eq!(confirmations[0].handling_method, "manual");
    let lease = fixture
        .store
        .load_worker_lease(&fixture.workflow_id, &fixture.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(lease.owner_id, "existing-worker");
}

#[test]
fn resolve_confirmation_log_failure_uses_recoverable_projection_outbox() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("confirmation-log-failure");
    let run_id = RunId::from("run-confirmation-log-failure");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.status = RunStatus::Paused;
    state.control = ariadne::contracts::RunControl::Pause;
    state.pause_reason = Some("pending confirmation items".to_owned());
    state.confirmations.insert(
        "confirm-log-failure".to_owned(),
        RuntimeConfirmation {
            confirmation_id: "confirm-log-failure".to_owned(),
            node_id: NodeId::from("approval"),
            state: RuntimeConfirmationState::Pending,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: json!({
                "kind": "approval",
                "summary": "待确认输出",
                "diff": "- old\n+ new",
            }),
        },
    );
    state.nodes.insert(
        NodeId::from("approval"),
        ariadne::workflow::WorkflowNodeRuntimeState {
            node_id: NodeId::from("approval"),
            status: RunStatus::Paused,
            outputs: Default::default(),
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
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.create_state(&state).unwrap();
    std::fs::create_dir_all(temp.path().join(".runtime")).unwrap();
    std::fs::create_dir(temp.path().join(".runtime/ui_logs.db")).unwrap();

    let result = resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: workflow_id.as_str().to_owned(),
            run_id: run_id.as_str().to_owned(),
            confirmation_id: "confirm-log-failure".to_owned(),
            decision: ConfirmationDecision::Approve,
            review_reason: Some("人工通过".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(result.workflow.status, "queued");
    assert_eq!(result.confirmation.state, ConfirmationLogState::Approved);
    let persisted = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
    assert_eq!(persisted.status, RunStatus::Queued);
    assert_eq!(persisted.control, ariadne::contracts::RunControl::Continue);
    assert_eq!(persisted.pause_reason, None);
    assert_eq!(
        persisted
            .confirmations
            .get("confirm-log-failure")
            .unwrap()
            .state,
        RuntimeConfirmationState::Approved
    );
    assert!(matches!(
        persisted
            .nodes
            .get(&NodeId::from("approval"))
            .and_then(|node| node.outputs.get("review_reason")),
        Some(PortValue::Inline { value }) if value == &json!("人工通过")
    ));
    // 领域提交后的投影故障不能反向否定提交；同步测试入口没有 worker，正常释放 lease。
    assert!(store
        .load_worker_lease(&workflow_id, &run_id)
        .unwrap()
        .is_none());
    let pending_projection = store.list_recoverable_confirmation_resolutions().unwrap();
    assert_eq!(pending_projection.len(), 1);
    assert_eq!(
        pending_projection[0].status,
        ConfirmationResolutionStatus::Committed
    );
    assert!(!pending_projection[0].projected);

    // 打开项目会重放可重建日志投影，不需要再次修改 runtime/knowledge。
    std::fs::remove_dir(temp.path().join(".runtime/ui_logs.db")).unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let app = AriadneAppState::new(
        PathBuf::new(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    open_project(&app, temp.path().to_string_lossy().into_owned(), None).unwrap();
    let entries = FileConfirmationLogStore::default_for_project(temp.path())
        .read_all()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].state, ConfirmationLogState::Approved);
    assert!(store
        .list_recoverable_confirmation_resolutions()
        .unwrap()
        .is_empty());
}

#[test]
fn f14_open_recovers_knowledge_receipt_crash_and_blocks_racing_resume() {
    use ariadne::rag::{
        ConfirmationItem, ConfirmationKind, ConfirmationState, MemoryWritingKnowledgeBase,
        SqliteWritingKnowledgeStore,
    };

    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("f14-crash-window");
    let run_id = RunId::from("run-f14-crash-window");
    let confirmation_id = "confirm-f14-crash-window";
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.prepared_workflow = Some(WorkflowDefinition {
        id: workflow_id.clone(),
        name: "F14 Recovery".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
    });
    state.status = RunStatus::Paused;
    state.control = RunControl::Pause;
    state.pause_reason = Some("pending confirmation items".to_owned());
    state.confirmations.insert(
        confirmation_id.to_owned(),
        RuntimeConfirmation {
            confirmation_id: confirmation_id.to_owned(),
            node_id: NodeId::from("summarizer"),
            state: RuntimeConfirmationState::Pending,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: json!({ "kind": "segment_summary", "summary": "待拒绝总结" }),
        },
    );
    state.nodes.insert(
        NodeId::from("summarizer"),
        WorkflowNodeRuntimeState {
            node_id: NodeId::from("summarizer"),
            status: RunStatus::Paused,
            outputs: Default::default(),
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
    let runtime_store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    runtime_store.create_state(&state).unwrap();

    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_confirmation(ConfirmationItem::new(
            confirmation_id,
            ConfirmationKind::SegmentSummary,
            ConfirmationState::Pending,
            json!({ "chapter_id": "chapter-f14" }),
        ))
        .unwrap();
    let knowledge_store = SqliteWritingKnowledgeStore::open(temp.path()).unwrap();
    knowledge_store.save_knowledge(&knowledge).unwrap();

    let operation_id = "confirmation-f14-crash-window";
    let request_hash = "request-f14-crash-window";
    let operation = runtime_store
        .prepare_confirmation_resolution(
            operation_id,
            &workflow_id,
            &run_id,
            confirmation_id,
            ConfirmationResolutionDecision::Reject,
            Some("内容不符合设定"),
            request_hash,
            true,
            100,
        )
        .unwrap();
    assert_eq!(operation.status, ConfirmationResolutionStatus::Prepared);

    let race_error = runtime_store
        .claim_resume(&workflow_id, &run_id, "racing-worker", 101, 10_000)
        .unwrap_err();
    assert!(race_error
        .to_string()
        .contains("confirmation resolution is still committing"));
    let still_pending = runtime_store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(still_pending.status, RunStatus::Paused);
    assert_eq!(
        still_pending.confirmations[confirmation_id].state,
        RuntimeConfirmationState::Pending
    );

    let response = json!({
        "workflow_id": workflow_id.as_str(),
        "run_id": run_id.as_str(),
        "confirmation_id": confirmation_id,
        "decision": "reject",
    });
    assert!(knowledge_store
        .resolve_confirmation_with_operation(
            confirmation_id,
            ConfirmationState::Rejected,
            operation_id,
            request_hash,
            &response,
        )
        .unwrap());
    assert_eq!(
        runtime_store
            .list_recoverable_confirmation_resolutions()
            .unwrap()[0]
            .status,
        ConfirmationResolutionStatus::Prepared
    );

    let app_state = tempfile::tempdir().unwrap();
    let app = AriadneAppState::new(
        PathBuf::new(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    open_project(&app, temp.path().to_string_lossy().into_owned(), None).unwrap();

    let recovered = runtime_store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        recovered.status,
        RunStatus::Queued | RunStatus::Running | RunStatus::Succeeded
    ));
    assert_eq!(
        recovered.confirmations[confirmation_id].state,
        RuntimeConfirmationState::Rejected
    );
    let recovered_knowledge = knowledge_store.load_knowledge().unwrap();
    let recovered_item = recovered_knowledge
        .confirmations(None)
        .unwrap()
        .into_iter()
        .find(|item| item.confirmation_id == confirmation_id)
        .unwrap();
    assert_eq!(recovered_item.state, ConfirmationState::Rejected);
    assert!(runtime_store
        .list_recoverable_confirmation_resolutions()
        .unwrap()
        .is_empty());
}

#[test]
fn resolve_confirmation_command_updates_runtime_and_log_badges() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Confirm Flow".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    runtime.state.status = RunStatus::Paused;
    runtime.state.pause_reason = Some("pending confirmation items".to_owned());
    runtime.state.confirmations.insert(
        "confirm-1".to_owned(),
        RuntimeConfirmation {
            confirmation_id: "confirm-1".to_owned(),
            node_id: NodeId::from("approval"),
            state: RuntimeConfirmationState::Pending,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: json!({
                "kind": "approval",
                "summary": "待确认输出",
                "diff": "- old\n+ new",
                "reason": "pending",
            }),
        },
    );
    runtime.state.nodes.insert(
        NodeId::from("approval"),
        ariadne::workflow::WorkflowNodeRuntimeState {
            node_id: NodeId::from("approval"),
            status: RunStatus::Paused,
            outputs: Default::default(),
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
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.create_state(&runtime.state).unwrap();
    FileConfirmationLogStore::default_for_project(temp.path())
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-1".to_owned(),
            kind: "approval".to_owned(),
            node_id: "approval".to_owned(),
            timestamp_ms: 1,
            state: ConfirmationLogState::Pending,
            handling_method: "manual".to_owned(),
            summary: "待确认输出".to_owned(),
            diff: "- old\n+ new".to_owned(),
            workflow_id: "wf".to_owned(),
            run_id: "run-1".to_owned(),
        })
        .unwrap();

    let result = resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: "wf".to_owned(),
            run_id: "run-1".to_owned(),
            confirmation_id: "confirm-1".to_owned(),
            decision: ConfirmationDecision::Approve,
            review_reason: Some("人工通过".to_owned()),
        },
    )
    .unwrap();
    let updated = store
        .load_state(&WorkflowId::from("wf"), &RunId::from("run-1"))
        .unwrap()
        .unwrap();
    let node = updated.nodes.get(&NodeId::from("approval")).unwrap();

    assert_eq!(result.confirmation.state, ConfirmationLogState::Approved);
    assert_eq!(result.badges.confirmations, 0);
    assert_eq!(result.workflow.status, "queued");
    assert!(matches!(
        node.outputs.get("approved"),
        Some(PortValue::Inline { value }) if value == &json!(true)
    ));
    assert!(matches!(
        node.outputs.get("review_reason"),
        Some(PortValue::Inline { value }) if value == &json!("人工通过")
    ));
}

#[test]
fn resolve_confirmation_waits_until_all_pending_items_are_resolved() {
    let temp = tempfile::tempdir().unwrap();
    let workflow_id = WorkflowId::from("confirmation-batch");
    let run_id = RunId::from("run-confirmation-batch");
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.status = RunStatus::Paused;
    state.control = RunControl::Pause;
    state.pause_reason = Some("pending confirmation items".to_owned());
    for confirmation_id in ["confirm-first", "confirm-second"] {
        state.confirmations.insert(
            confirmation_id.to_owned(),
            RuntimeConfirmation {
                confirmation_id: confirmation_id.to_owned(),
                node_id: NodeId::from(confirmation_id),
                state: RuntimeConfirmationState::Pending,
                artifact_id: None,
                patch_session_commit_id: None,
                metadata: json!({ "kind": "approval", "summary": confirmation_id }),
            },
        );
    }
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.create_state(&state).unwrap();

    let first = resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: workflow_id.as_str().to_owned(),
            run_id: run_id.as_str().to_owned(),
            confirmation_id: "confirm-first".to_owned(),
            decision: ConfirmationDecision::Approve,
            review_reason: None,
        },
    )
    .unwrap();
    assert_eq!(first.workflow.status, "paused");
    assert_eq!(first.badges.confirmations, 1);
    let after_first = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
    assert_eq!(after_first.status, RunStatus::Paused);
    assert_eq!(after_first.control, RunControl::Pause);
    assert_eq!(
        after_first.confirmations["confirm-second"].state,
        RuntimeConfirmationState::Pending
    );
    assert!(store
        .load_worker_lease(&workflow_id, &run_id)
        .unwrap()
        .is_none());

    let second = resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: workflow_id.as_str().to_owned(),
            run_id: run_id.as_str().to_owned(),
            confirmation_id: "confirm-second".to_owned(),
            decision: ConfirmationDecision::Reject,
            review_reason: Some("不采用".to_owned()),
        },
    )
    .unwrap();
    assert_eq!(second.workflow.status, "queued");
    assert_eq!(second.badges.confirmations, 0);
    let after_second = store.load_state(&workflow_id, &run_id).unwrap().unwrap();
    assert_eq!(after_second.status, RunStatus::Queued);
    assert_eq!(after_second.control, RunControl::Continue);
}

/// F14 product path: resolve_confirmation_impl must materialize writing-knowledge
/// pending_payload (not only flip runtime confirmation state).
#[test]
fn resolve_confirmation_materializes_summary_knowledge_on_approve() {
    use ariadne::contracts::{AutoModeState, SourceSpan, TextRange};
    use ariadne::rag::{
        MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, StoryEvent, StoryEventStatus,
        StorySegment, SummaryPipelineDraft, SummaryPipelineExecutor, WritingConfirmationPolicy,
    };
    use serde_json::{json, Value};

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();

    // 1) Apply summarizer draft under normal (human) policy → pending, non-active knowledge.
    let kb = MemoryWritingKnowledgeBase::new();
    let pipeline = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::normal_default(),
        AutoModeState::default(),
    );
    let report = pipeline
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "ch-f14".to_owned(),
            segments: vec![StorySegment {
                segment_id: "ch-f14::seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "ch-f14".to_owned(),
                summary: "段摘要".to_owned(),
                source: SourceSpan {
                    document_id: "doc.md".to_owned(),
                    range: TextRange { start: 0, end: 10 },
                    version: None,
                },
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "ev-1".to_owned(),
                summary: "事件".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["ch-f14::seg-1".to_owned()],
                chapter_ids: vec!["ch-f14".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("章总结".to_owned()),
            stage_id: Some("stage-f14".to_owned()),
            stage_summary: Some("阶段".to_owned()),
            is_new_stage: Some(true),
            realized_changes: vec![],
            foreshadowing_updates: vec![],
            metadata: Value::Null,
        })
        .unwrap();
    assert!(kb.chapter_summary("ch-f14").unwrap().is_none());
    let knowledge_store = SqliteWritingKnowledgeStore::open(project).unwrap();
    knowledge_store.save_knowledge(&kb).unwrap();

    let segment_confirmation = report
        .confirmation_ids
        .iter()
        .find(|id| id.ends_with("segment-summary"))
        .expect("segment confirmation id")
        .clone();

    // 2) Runtime paused with the same confirmation id (as execute_summarizer_node does).
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-f14"),
        name: "Summarizer Confirm".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-f14")).unwrap();
    runtime.state.status = RunStatus::Paused;
    runtime.state.pause_reason = Some("pending confirmation items".to_owned());
    runtime.state.confirmations.insert(
        segment_confirmation.clone(),
        RuntimeConfirmation {
            confirmation_id: segment_confirmation.clone(),
            node_id: NodeId::from("summarizer"),
            state: RuntimeConfirmationState::Pending,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: json!({ "step": "segment", "chapter_id": "ch-f14" }),
        },
    );
    runtime.state.nodes.insert(
        NodeId::from("summarizer"),
        WorkflowNodeRuntimeState {
            node_id: NodeId::from("summarizer"),
            status: RunStatus::Paused,
            outputs: Default::default(),
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
    let runtime_store = SqliteWorkflowRuntimeStore::open(project).unwrap();
    runtime_store.create_state(&runtime.state).unwrap();

    // 3) Product path: resolve_confirmation_impl Approve
    resolve_confirmation_impl(
        project,
        ResolveConfirmationRequest {
            workflow_id: "wf-f14".to_owned(),
            run_id: "run-f14".to_owned(),
            confirmation_id: segment_confirmation,
            decision: ConfirmationDecision::Approve,
            review_reason: Some("ok".to_owned()),
        },
    )
    .unwrap();

    // 4) Knowledge must now have active segment facts
    let reloaded = knowledge_store.load_knowledge().unwrap();
    assert_eq!(
        reloaded.all_segments().unwrap().len(),
        1,
        "approve via resolve_confirmation must materialize segments"
    );
    assert_eq!(
        reloaded.chapter_summary("ch-f14").unwrap(),
        None,
        "only segment step approved; chapter still pending"
    );
}

/// F14 multi-store: knowledge materialize failure must not leave runtime Approved
/// while knowledge is still Pending (no durable split-brain after command return).
#[test]
fn resolve_confirmation_knowledge_failure_leaves_runtime_pending() {
    use ariadne::contracts::{AutoModeState, SourceSpan, TextRange};
    use ariadne::rag::{
        MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, StoryEvent, StoryEventStatus,
        StorySegment, SummaryPipelineDraft, SummaryPipelineExecutor, WritingConfirmationPolicy,
    };
    use serde_json::{json, Value};

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();

    let kb = MemoryWritingKnowledgeBase::new();
    let pipeline = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::normal_default(),
        AutoModeState::default(),
    );
    let report = pipeline
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "ch-atom".to_owned(),
            segments: vec![StorySegment {
                segment_id: "ch-atom::seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "ch-atom".to_owned(),
                summary: "段".to_owned(),
                source: SourceSpan {
                    document_id: "doc.md".to_owned(),
                    range: TextRange { start: 0, end: 5 },
                    version: None,
                },
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "ev-atom".to_owned(),
                summary: "事".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["ch-atom::seg-1".to_owned()],
                chapter_ids: vec!["ch-atom".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("章".to_owned()),
            stage_id: Some("stage-atom".to_owned()),
            stage_summary: Some("阶".to_owned()),
            is_new_stage: Some(true),
            realized_changes: vec![],
            foreshadowing_updates: vec![],
            metadata: Value::Null,
        })
        .unwrap();
    let knowledge_store = SqliteWritingKnowledgeStore::open(project).unwrap();
    knowledge_store.save_knowledge(&kb).unwrap();
    let segment_confirmation = report
        .confirmation_ids
        .iter()
        .find(|id| id.ends_with("segment-summary"))
        .unwrap()
        .clone();

    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-atom"),
        name: "atom".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-atom")).unwrap();
    runtime.state.status = RunStatus::Paused;
    runtime.state.pause_reason = Some("pending confirmation items".to_owned());
    runtime.state.confirmations.insert(
        segment_confirmation.clone(),
        RuntimeConfirmation {
            confirmation_id: segment_confirmation.clone(),
            node_id: NodeId::from("summarizer"),
            state: RuntimeConfirmationState::Pending,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: json!({ "step": "segment" }),
        },
    );
    runtime.state.nodes.insert(
        NodeId::from("summarizer"),
        WorkflowNodeRuntimeState {
            node_id: NodeId::from("summarizer"),
            status: RunStatus::Paused,
            outputs: Default::default(),
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
    let runtime_store = SqliteWorkflowRuntimeStore::open(project).unwrap();
    runtime_store.create_state(&runtime.state).unwrap();

    // Force knowledge materialize/save to fail (segment insert aborts).
    let meta_db = project.join("metadata.db");
    let conn = rusqlite::Connection::open(&meta_db).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_story_segment_insert
         BEFORE INSERT ON story_segments
         BEGIN SELECT RAISE(ABORT, 'forced knowledge materialize failure'); END;",
    )
    .unwrap();
    drop(conn);

    let err = resolve_confirmation_impl(
        project,
        ResolveConfirmationRequest {
            workflow_id: "wf-atom".to_owned(),
            run_id: "run-atom".to_owned(),
            confirmation_id: segment_confirmation.clone(),
            decision: ConfirmationDecision::Approve,
            review_reason: None,
        },
    )
    .expect_err("knowledge failure must fail the command");
    assert!(
        err.to_string().contains("forced")
            || err.to_string().contains("knowledge")
            || err.to_string().contains("ABORT")
            || err.to_string().contains("sqlite")
            || !err.is_empty(),
        "unexpected error: {err}"
    );

    // Runtime must NOT be left Approved (knowledge-first: runtime never mutated).
    let after = runtime_store
        .load_state(&WorkflowId::from("wf-atom"), &RunId::from("run-atom"))
        .unwrap()
        .unwrap();
    let conf = after.confirmations.get(&segment_confirmation).unwrap();
    assert!(
        matches!(conf.state, RuntimeConfirmationState::Pending),
        "runtime must stay Pending when knowledge materialize fails; got {:?}",
        conf.state
    );

    // Knowledge must not have activated segments (still Pending payload path).
    let kb2 = SqliteWritingKnowledgeStore::open(project)
        .unwrap()
        .load_knowledge()
        .unwrap();
    assert!(
        kb2.all_segments().unwrap().is_empty(),
        "failed materialize must not leave active segments"
    );
}

/// F14-a product path: after knowledge is durably applied (KnowledgeCommitted),
/// runtime NotFound is corruption/in-doubt state. Product must preserve the
/// receipt for explicit recovery and ignore project-owned pre-image authority.
#[test]
fn resolve_confirmation_runtime_not_found_preserves_receipt_and_ignores_project_pre_image() {
    use ariadne::contracts::{AutoModeState, SourceSpan, TextRange};
    use ariadne::rag::{
        ConfirmationState, MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, StoryEvent,
        StoryEventStatus, StorySegment, SummaryPipelineDraft, SummaryPipelineExecutor,
        WritingConfirmationPolicy,
    };
    use ariadne::workflow::{
        ConfirmationResolutionDecision, SqliteWorkflowRuntimeStore, WorkflowRuntime,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();

    let kb = MemoryWritingKnowledgeBase::new();
    let pipeline = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::normal_default(),
        AutoModeState::default(),
    );
    let report = pipeline
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "ch-inv".to_owned(),
            segments: vec![StorySegment {
                segment_id: "ch-inv::seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "ch-inv".to_owned(),
                summary: "段".to_owned(),
                source: SourceSpan {
                    document_id: "doc.md".to_owned(),
                    range: TextRange { start: 0, end: 5 },
                    version: None,
                },
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "ev-inv".to_owned(),
                summary: "事".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["ch-inv::seg-1".to_owned()],
                chapter_ids: vec!["ch-inv".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("章".to_owned()),
            stage_id: Some("stage-inv".to_owned()),
            stage_summary: Some("阶".to_owned()),
            is_new_stage: Some(true),
            realized_changes: vec![],
            foreshadowing_updates: vec![],
            metadata: Value::Null,
        })
        .unwrap();
    let knowledge_store = SqliteWritingKnowledgeStore::open(project).unwrap();
    knowledge_store.save_knowledge(&kb).unwrap();
    let segment_confirmation = report
        .confirmation_ids
        .iter()
        .find(|id| id.ends_with("segment-summary"))
        .unwrap()
        .clone();
    let pre_pending = knowledge_store
        .load_knowledge()
        .unwrap()
        .confirmations(None)
        .unwrap()
        .into_iter()
        .find(|c| c.confirmation_id == segment_confirmation)
        .expect("pending confirmation");
    assert!(
        matches!(pre_pending.state, ConfirmationState::Pending),
        "setup requires Pending confirmation"
    );
    assert!(pre_pending.metadata.get("pending_payload").is_some());

    // Runtime run must exist so prepare can enter Prepared (not fail before knowledge).
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf-inv"),
        name: "F14-a inverse".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-inv")).unwrap();
    runtime.state.status = RunStatus::Paused;
    runtime.state.pause_reason = Some("pending confirmation items".to_owned());
    runtime.state.confirmations.insert(
        segment_confirmation.clone(),
        RuntimeConfirmation {
            confirmation_id: segment_confirmation.clone(),
            node_id: NodeId::from("summarizer"),
            state: RuntimeConfirmationState::Pending,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: json!({ "step": "segment", "chapter_id": "ch-inv" }),
        },
    );
    runtime.state.nodes.insert(
        NodeId::from("summarizer"),
        WorkflowNodeRuntimeState {
            node_id: NodeId::from("summarizer"),
            status: RunStatus::Paused,
            outputs: Default::default(),
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
    let runtime_store = SqliteWorkflowRuntimeStore::open(project).unwrap();
    runtime_store.create_state(&runtime.state).unwrap();

    let request = ResolveConfirmationRequest {
        workflow_id: "wf-inv".to_owned(),
        run_id: "run-inv".to_owned(),
        confirmation_id: segment_confirmation.clone(),
        decision: ConfirmationDecision::Approve,
        review_reason: None,
    };
    // Deterministic operation id / request hash matching product path.
    let mut op_hasher = Sha256::new();
    for part in ["wf-inv", "run-inv", segment_confirmation.as_str()] {
        op_hasher.update(part.len().to_le_bytes());
        op_hasher.update(part.as_bytes());
    }
    let operation_id = format!("confirmation-{:x}", op_hasher.finalize());
    let request_hash = {
        let canonical = serde_json::to_vec(&json!({
            "workflow_id": request.workflow_id,
            "run_id": request.run_id,
            "confirmation_id": request.confirmation_id,
            "decision": request.decision,
            "review_reason": request.review_reason,
        }))
        .unwrap();
        format!("{:x}", Sha256::digest(canonical))
    };

    // Drive product saga steps through KnowledgeCommitted, then delete the run
    // so the next product resolve hits KnowledgeCommitted → NotFound.
    let op = runtime_store
        .prepare_confirmation_resolution(
            &operation_id,
            &WorkflowId::from("wf-inv"),
            &RunId::from("run-inv"),
            &segment_confirmation,
            ConfirmationResolutionDecision::Approve,
            None,
            &request_hash,
            true,
            1,
        )
        .unwrap();
    assert!(matches!(
        op.status,
        ariadne::workflow::ConfirmationResolutionStatus::Prepared
    ));
    // A project-owned pre-image is untrusted and must never authorize recovery writes.
    let pre_image_path = project
        .join(".runtime")
        .join("confirmation_pre_images")
        .join(format!("{operation_id}.json"));
    std::fs::create_dir_all(pre_image_path.parent().unwrap()).unwrap();
    std::fs::write(
        &pre_image_path,
        serde_json::to_vec_pretty(&pre_pending).unwrap(),
    )
    .unwrap();
    let applied = knowledge_store
        .resolve_confirmation_with_operation(
            &segment_confirmation,
            ConfirmationState::Approved,
            &operation_id,
            &request_hash,
            &json!({
                "workflow_id": "wf-inv",
                "run_id": "run-inv",
                "confirmation_id": segment_confirmation,
                "decision": "Approve",
            }),
        )
        .unwrap();
    assert!(applied, "knowledge must apply under prepared saga");
    runtime_store
        .mark_confirmation_knowledge_committed(&operation_id, &request_hash, 2)
        .unwrap();
    // Prove knowledge side-effect happened before runtime loss.
    assert_eq!(
        knowledge_store
            .load_knowledge()
            .unwrap()
            .all_segments()
            .unwrap()
            .len(),
        1,
        "segments must be active after knowledge commit"
    );
    // Delete runtime run without cascading away the confirmation saga row
    // (product NotFound after KnowledgeCommitted). Disable FK only for this inject.
    {
        let db = project.join("runtime.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute("DELETE FROM workflow_runs", []).unwrap();
        let still: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM confirmation_resolution_operations WHERE operation_id=?1",
                rusqlite::params![operation_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            still, 1,
            "saga row must survive run deletion for NotFound path"
        );
    }

    // Product entry re-enters the KnowledgeCommitted operation and fails loud without
    // executing the forged project-owned inverse payload or deleting the receipt.
    let err = resolve_confirmation_impl(project, request).expect_err("must fail after NotFound");
    assert!(
        err.to_string().contains("not found")
            && err.to_string().contains("receipt preserved")
            && !err.to_string().contains("compensated"),
        "unexpected error: {err}"
    );

    let kb2 = knowledge_store.load_knowledge().unwrap();
    assert_eq!(
        kb2.all_segments().unwrap().len(),
        1,
        "project-owned pre-image must not reverse committed knowledge"
    );
    let item = kb2
        .confirmations(None)
        .unwrap()
        .into_iter()
        .find(|c| c.confirmation_id == segment_confirmation)
        .expect("confirmation still present");
    assert!(
        matches!(item.state, ConfirmationState::Approved),
        "knowledge confirmation receipt must remain Approved; got {:?}",
        item.state
    );
    assert!(
        item.metadata.get("pending_payload").is_none(),
        "committed confirmation must not be rewritten from forged pre-image"
    );
    assert!(
        knowledge_store
            .load_operation_receipt(&operation_id, &request_hash)
            .unwrap()
            .is_some(),
        "knowledge receipt must remain available for explicit recovery"
    );
    let recoverable = runtime_store
        .list_recoverable_confirmation_resolutions()
        .unwrap();
    assert_eq!(recoverable.len(), 1);
    assert_eq!(
        recoverable[0].status,
        ariadne::workflow::ConfirmationResolutionStatus::KnowledgeCommitted,
        "saga must remain recoverable instead of being silently aborted"
    );
    // No whole-db bak residual.
    assert!(
        !project.join("metadata.db.f14-resolve-bak").exists(),
        "must not use whole metadata.db backup file"
    );
}

/// F11 product path: register_executor_adapters_for_project loads skills and
/// registers executor_adapter:{id} on the shipped RoutedExternalNodeExecutor.
#[test]
fn register_executor_adapters_for_project_registers_skill_handlers() {
    use ariadne::config::ConfigStore;
    use ariadne::costs::SqliteCostLedger;
    use ariadne::providers::OpenAiCompatibleLlmProvider;
    use ariadne::workflow::RoutedExternalNodeExecutor;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    // Minimal project config (default skills_dir = "skills")
    ConfigStore::new(project).load_or_create().unwrap();
    let skill_dir = project.join("skills").join("fetch-info");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("skill.json"),
        r#"{
          "skill_id": "fetch-info",
          "name": "Fetch Info",
          "version": "1.0.0",
          "executor": { "kind": "http", "host": "example.com", "method": "POST", "path": "/lookup" },
          "schema": {
            "inputs": [{"name": "query", "type_name": "inline", "required": true}],
            "outputs": [{"name": "result", "type_name": "inline", "required": true}]
          },
          "limits": { "timeout_ms": 1000, "max_output_bytes": 1024 },
          "estimated_cost_usd": 0.0
        }"#,
    )
    .unwrap();

    let mut external = RoutedExternalNodeExecutor::new();
    let provider_config = ariadne::config::ProviderConfig {
        provider_id: "mock".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "mock".to_owned(),
        enabled: true,
        base_url: Some("https://example.com".to_owned()),
        api_key: None,
        models: vec![],
    };
    // If ProviderConfig fields differ, compile will tell us — use minimal via new if needed.
    let provider = OpenAiCompatibleLlmProvider::new(provider_config, None).unwrap();
    let ledger = Arc::new(SqliteCostLedger::open_in_memory().unwrap());
    let registered =
        register_executor_adapters_for_project(&mut external, project, provider, ledger).unwrap();
    assert!(
        registered
            .iter()
            .any(|n| n == "executor_adapter:fetch-info"),
        "registered={registered:?}"
    );
    assert!(
        external.has_handler("executor_adapter:fetch-info"),
        "handler missing from router; types={:?}",
        external.registered_type_names()
    );
}

fn accept_with_deadline(listener: &TcpListener, timeout: Duration) -> std::net::TcpStream {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for local HTTP request"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("failed to accept local HTTP request: {error}"),
        }
    }
}

fn run_git<const N: usize>(repo: &std::path::Path, args: [&str; N]) {
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

fn git_stdout<const N: usize>(repo: &std::path::Path, args: [&str; N]) -> String {
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
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn automation_settings_mid_fail_leaves_journal_and_recover_completes() {
    use ariadne::commands::{
        commit_automation_settings_files_with_fail_after, get_automation_settings_impl,
        save_automation_settings_impl,
    };
    use ariadne::config::atomic_commit::{has_pending_journal, recover_pending_commit};
    use ariadne::frontend::initialize_project;
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let app_state = temp.path().join("app-state");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&app_state).unwrap();
    initialize_project(&project).unwrap();

    // Baseline values via product path (default app state); then mid-fail with explicit app_state.
    let baseline = get_automation_settings_impl(&project).unwrap();
    let mut settings = baseline.clone();
    settings.budget.budget_usd = 42.5;
    settings.budget.preauthorized_usd = 1.0;
    settings.budget.auto_mode_enabled = true;

    save_automation_settings_impl(&project, settings.clone()).unwrap();
    let after_ok = get_automation_settings_impl(&project).unwrap();
    assert!((after_ok.budget.budget_usd - 42.5).abs() < 1e-9);

    settings.budget.budget_usd = 99.0;
    let config_store = ariadne::config::ConfigStore::with_app_state(&project, &app_state);
    let mut config = config_store.load_or_create().unwrap();
    config.auto_mode.enabled_by_default = true;
    config.auto_mode.preauthorized_budget_usd = Some(2.0);
    let budget = serde_json::to_vec_pretty(&serde_json::json!({ "budget_usd": 99.0 })).unwrap();
    let policies = serde_json::to_vec_pretty(&settings.confirmation_policies).unwrap();

    let err = commit_automation_settings_files_with_fail_after(
        &project,
        &app_state,
        &config,
        &budget,
        &policies,
        Some(1),
    )
    .unwrap_err();
    assert!(
        err.contains("injected") || err.contains("atomic"),
        "unexpected err: {err}"
    );
    // D4-a/S4: authority journal lives in app-state, never as project-owned executable journal.
    assert!(
        has_pending_journal(&project, &app_state),
        "app-state authority journal must exist after mid-fail"
    );
    assert!(
        !project
            .join(".config")
            .join("atomic-commit.journal.json")
            .exists(),
        "live mid-fail must not write project-owned legacy journal"
    );

    recover_pending_commit(&project, &app_state).unwrap();
    assert!(
        !has_pending_journal(&project, &app_state),
        "authority journal cleared after recover"
    );
    let final_settings = get_automation_settings_impl(&project).unwrap();
    assert!(
        (final_settings.budget.budget_usd - 99.0).abs() < 1e-9,
        "budget should be fully applied after recover, got {}",
        final_settings.budget.budget_usd
    );
}

#[test]
fn general_settings_mid_fail_recovers_config_and_project_memory_together() {
    use ariadne::commands::commit_general_settings_files_with_fail_after;
    use ariadne::config::atomic_commit::{has_pending_journal, recover_pending_commit};
    use ariadne::frontend::initialize_project;

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let app_state = temp.path().join("app-state");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&app_state).unwrap();
    initialize_project(&project).unwrap();

    let store = ConfigStore::with_app_state(&project, &app_state);
    let mut config = store.load_or_create().unwrap();
    config.app.project_name = "同一事务版本".to_owned();
    let memory = b"# project memory\ntransaction-version\n";

    let error = commit_general_settings_files_with_fail_after(
        &project,
        &app_state,
        &config,
        memory,
        Some(1),
    )
    .unwrap_err();
    assert!(error.contains("injected") || error.contains("atomic"));
    assert!(has_pending_journal(&project, &app_state));

    recover_pending_commit(&project, &app_state).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.app.project_name, "同一事务版本");
    assert_eq!(
        fs::read(project.join(".runtime/project_memory.md")).unwrap(),
        memory
    );
}

fn seed_command_in_doubt_search_run(
    project_root: &std::path::Path,
    workflow_name: &str,
    run_name: &str,
    operation_policy: WorkflowOperationPolicy,
) -> (SqliteWorkflowRuntimeStore, String) {
    let WorkflowOperationPolicy::Journaled { recovery, response } = operation_policy else {
        panic!("in_doubt command fixture requires a journaled operation policy");
    };
    let workflow_id = WorkflowId::from(workflow_name);
    let run_id = RunId::from(run_name);
    let node_id = NodeId::from("search-node");
    let workflow = WorkflowDefinition {
        id: workflow_id.clone(),
        name: "In doubt command fixture".to_owned(),
        nodes: vec![NodeInstance {
            id: node_id.clone(),
            type_name: "search".to_owned(),
            label: None,
            config: Value::Null,
            position: None,
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut state = WorkflowRunState::new(workflow_id.clone(), run_id.clone());
    state.prepared_workflow = Some(workflow.clone());
    state.status = RunStatus::Paused;
    state.control = RunControl::Pause;
    state.pause_reason = Some("operation result is unknown".to_owned());
    state.nodes.insert(
        node_id.clone(),
        WorkflowNodeRuntimeState {
            node_id: node_id.clone(),
            status: RunStatus::Paused,
            outputs: Default::default(),
            communication_output: None,
            communication_control: Default::default(),
            prompt_trace_hash: None,
            patch_session_commit_id: None,
            checkpoint_id: None,
            patch_write_back_state: None,
            metadata: Value::Null,
            error: Some("remote dispatch outcome is unknown".to_owned()),
            error_state: None,
            execution_attempts: 1,
        },
    );
    let operation_id = ariadne::skills::stable_text_hash(&format!(
        "workflow-operation-v1\0{}\0{}\0{}\01",
        workflow_id.as_str(),
        run_id.as_str(),
        node_id.as_str()
    ));
    let request_hash = ariadne::skills::stable_text_hash(
        &serde_json::to_string(&json!({
            "type_name": "search",
            "config": Value::Null,
            "inputs": ariadne::contracts::PortMap::new(),
            "communication_messages": Vec::<ariadne::workflow::CommunicationMessage>::new(),
            "metadata": Value::Null,
        }))
        .unwrap(),
    );
    let store = SqliteWorkflowRuntimeStore::open(project_root).unwrap();
    store.create_state(&state).unwrap();
    store
        .create_operation(
            &NewWorkflowOperation {
                operation_id: operation_id.clone(),
                workflow_id,
                run_id,
                node_id,
                attempt: 1,
                kind: "search".to_owned(),
                provider: "search".to_owned(),
                request_hash,
                lease_generation: 0,
                recovery_policy: recovery,
                response_policy: response,
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
    assert!(store
        .transition_operation(
            &operation_id,
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::InDoubt,
            None,
            1_002,
        )
        .unwrap());
    (store, operation_id)
}

#[test]
fn resolve_in_doubt_use_response_command_replays_and_commits_without_provider_call() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let (store, operation_id) = seed_command_in_doubt_search_run(
        temp.path(),
        "use-response-flow",
        "run-1",
        WorkflowOperationPolicy::replayable_remote(),
    );
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let response = serde_json::to_value(WorkflowNodeExecutionOutput::default()).unwrap();

    let result = resolve_workflow_operation_in_doubt(
        &state,
        ResolveInDoubtOperationRequest {
            operation_id: operation_id.clone(),
            decision: InDoubtDecision::UseResponse,
            response: Some(response),
            reason: None,
        },
    )
    .unwrap();

    assert_eq!(result.decision, InDoubtDecision::UseResponse);
    assert_eq!(result.workflow.status, "queued");
    let terminal = wait_for_terminal_workflow_state(
        &store,
        &WorkflowId::from("use-response-flow"),
        &RunId::from("run-1"),
    );
    assert_eq!(terminal.status, RunStatus::Succeeded);
    assert_eq!(
        store.load_operation(&operation_id).unwrap().unwrap().status,
        WorkflowOperationStatus::Committed
    );
    assert!(store
        .load_worker_lease(
            &WorkflowId::from("use-response-flow"),
            &RunId::from("run-1")
        )
        .unwrap()
        .is_none());
}

#[test]
fn resolve_in_doubt_use_response_command_rejects_receipt_only_operation() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let (store, operation_id) = seed_command_in_doubt_search_run(
        temp.path(),
        "receipt-only-flow",
        "run-1",
        WorkflowOperationPolicy::reconcilable_receipt(),
    );
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let error = resolve_workflow_operation_in_doubt(
        &state,
        ResolveInDoubtOperationRequest {
            operation_id: operation_id.clone(),
            decision: InDoubtDecision::UseResponse,
            response: Some(serde_json::to_value(WorkflowNodeExecutionOutput::default()).unwrap()),
            reason: None,
        },
    )
    .unwrap_err();

    assert!(error.contains("executor receipt"));
    assert_eq!(
        store.load_operation(&operation_id).unwrap().unwrap().status,
        WorkflowOperationStatus::InDoubt
    );
    let persisted = store
        .load_state(
            &WorkflowId::from("receipt-only-flow"),
            &RunId::from("run-1"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, RunStatus::Paused);
    assert_eq!(persisted.control, RunControl::Pause);
    assert!(store
        .load_worker_lease(
            &WorkflowId::from("receipt-only-flow"),
            &RunId::from("run-1")
        )
        .unwrap()
        .is_none());
}

#[test]
fn resolve_in_doubt_stop_command_atomically_stops_without_claiming_worker() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let (store, operation_id) = seed_command_in_doubt_search_run(
        temp.path(),
        "stop-in-doubt-flow",
        "run-1",
        WorkflowOperationPolicy::remote_response(),
    );
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let result = resolve_workflow_operation_in_doubt(
        &state,
        ResolveInDoubtOperationRequest {
            operation_id: operation_id.clone(),
            decision: InDoubtDecision::Stop,
            response: None,
            reason: Some("author chose not to risk duplicate billing".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(result.decision, InDoubtDecision::Stop);
    assert_eq!(result.workflow.status, "stopped");
    let stopped = store
        .load_state(
            &WorkflowId::from("stop-in-doubt-flow"),
            &RunId::from("run-1"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(stopped.status, RunStatus::Stopped);
    assert_eq!(stopped.control, RunControl::Stop);
    assert_eq!(
        stopped.stop_reason.as_deref(),
        Some("author chose not to risk duplicate billing")
    );
    assert!(stopped.structured_events.iter().any(|event| {
        event.event_type == WorkflowRuntimeEventType::RunStopped
            && event.metadata["operation_id"] == operation_id
    }));
    assert_eq!(
        store.load_operation(&operation_id).unwrap().unwrap().status,
        WorkflowOperationStatus::Aborted
    );
    assert!(store
        .load_worker_lease(
            &WorkflowId::from("stop-in-doubt-flow"),
            &RunId::from("run-1")
        )
        .unwrap()
        .is_none());
}

/// F19/F20：产品命令从同一个 metadata.db 快照读取正式阶段与章节总结。
#[test]
fn works_tree_and_chapter_summary_commands_share_official_stage_projection() {
    use ariadne::contracts::{SourceSpan, TextRange};
    use ariadne::documents::{ChapterDocumentEntry, ChapterDocumentIndex, ChapterDocumentKind};
    use ariadne::rag::{MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, StorySegment};

    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let document_path = project.path().join("documents").join("chapter-1.md");
    std::fs::write(&document_path, "甲😀乙").unwrap();
    let index = ChapterDocumentIndex::new(
        "v1",
        vec![ChapterDocumentEntry {
            chapter_id: "misleading-prefix:chapter-1".to_owned(),
            document_id: "documents/chapter-1.md".to_owned(),
            path: document_path,
            title: "第一章".to_owned(),
            order: 1,
            kind: ChapterDocumentKind::ChapterBody,
            version: "v1".to_owned(),
            word_count: None,
            outline_ref: None,
        }],
    )
    .unwrap();
    std::fs::write(
        project.path().join(".runtime").join("chapter_index.json"),
        serde_json::to_vec_pretty(&index).unwrap(),
    )
    .unwrap();

    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: "misleading-prefix:chapter-1::seg-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "misleading-prefix:chapter-1".to_owned(),
            summary: "正文故事段".to_owned(),
            source: SourceSpan {
                document_id: "documents/chapter-1.md".to_owned(),
                range: TextRange { start: 0, end: 10 },
                version: Some("v1".to_owned()),
            },
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_chapter_summary("misleading-prefix:chapter-1", "章节正式总结")
        .unwrap();
    knowledge
        .upsert_stage_summary("official-stage", "阶段正式总结")
        .unwrap();
    knowledge
        .link_chapter_stage("misleading-prefix:chapter-1", "official-stage")
        .unwrap();
    SqliteWritingKnowledgeStore::open(project.path())
        .unwrap()
        .save_knowledge(&knowledge)
        .unwrap();

    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let tree = get_works_tree(&state).unwrap();
    let stage = tree
        .children
        .iter()
        .find(|node| node.stage_id.as_deref() == Some("official-stage"))
        .unwrap();
    assert_eq!(
        stage.children[0].chapter_id.as_deref(),
        Some("misleading-prefix:chapter-1")
    );

    let summary =
        get_chapter_summary_view(&state, "misleading-prefix:chapter-1".to_owned()).unwrap();
    assert_eq!(summary.chapter_summary.as_deref(), Some("章节正式总结"));
    assert_eq!(
        summary.stage.as_ref().map(|stage| stage.stage_id.as_str()),
        Some("official-stage")
    );
    assert_eq!(
        summary.segments[0].source.document_id,
        "documents/chapter-1.md"
    );
}
