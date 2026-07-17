use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ariadne::commands::{
    process_index_outbox_impl, save_document_content_impl, save_permissions_settings_impl,
    save_workflow_graph_impl, AriadneAppState, CanvasEdge, CanvasNode, PermissionsSettings,
    WorkflowGraphData,
};
use ariadne::config::{
    ConfigStore, MemorySecretStore, ProviderConfig, SecretRef, PROVIDERS_CONFIG_FILE,
};
use ariadne::contracts::{
    NodeId, PermissionPolicy, ProviderType, RunId, RunStatus, WorkflowEdgeKind, WorkflowId,
};
use ariadne::frontend::{
    now_timestamp_ms, UiRunLogEntry, UiRunLogKind, UiRunLogLevel, UiRunLogStore,
};
use ariadne::ipc::{handle_request, parse_call_params, IpcRequest};
use ariadne::workflow::{SqliteWorkflowRuntimeStore, WorkflowRunState, WorkflowRuntimeStore};
use serde_json::{json, Value};

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
fn call_params_parse_json_or_default_to_null() {
    assert_eq!(parse_call_params(None).unwrap(), Value::Null);
    assert_eq!(parse_call_params(Some("")).unwrap(), Value::Null);
    assert_eq!(
        parse_call_params(Some(r#"{"workflow_id":"wf"}"#)).unwrap(),
        json!({ "workflow_id": "wf" })
    );
    assert!(parse_call_params(Some("{not-json}")).is_err());
}

#[test]
fn ipc_update_budget_returns_saved_budget_status_instead_of_null() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let response = handle_request(
        &state,
        IpcRequest {
            method: "update_budget_config".to_owned(),
            params: json!({
                "budget_usd": 25.0,
                "preauthorized_usd": 3.5
            }),
        },
    );

    assert!(response.ok, "{:?}", response.error);
    let data = response.data.expect("budget update must return data");
    assert_eq!(data["budget_usd"], 25.0);
    assert_eq!(data["preauthorized_usd"], 3.5);
    assert!(data.get("spent_usd").is_some());
    assert!(data.get("auto_mode_enabled").is_some());
}

#[test]
fn ipc_run_logs_preserve_context_page_newest_first_and_mark_only_filtered_scope() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let store = UiRunLogStore::default_for_project(project.path());
    let timestamp_ms = now_timestamp_ms();
    for (log_id, timestamp_ms, run_id, level) in [
        ("a", timestamp_ms, "run-a", UiRunLogLevel::Error),
        ("b", timestamp_ms + 1, "run-b", UiRunLogLevel::Error),
        ("c", timestamp_ms + 2, "run-a", UiRunLogLevel::Info),
    ] {
        store
            .append(UiRunLogEntry {
                log_id: log_id.to_owned(),
                timestamp_ms,
                kind: UiRunLogKind::Node,
                level,
                message: format!("message-{log_id}"),
                workflow_id: Some(WorkflowId::from("workflow-a")),
                run_id: Some(RunId::from(run_id)),
                node_id: Some(NodeId::from("writer")),
                unread: false,
                metadata: Value::Null,
            })
            .unwrap();
    }

    let queried = handle_request(
        &state,
        IpcRequest {
            method: "query_run_logs".to_owned(),
            params: json!({
                "filter": {
                    "level": "error",
                    "descending": true,
                    "limit": 2
                }
            }),
        },
    );
    assert!(queried.ok, "{:?}", queried.error);
    let logs = queried.data.unwrap();
    assert_eq!(logs[0]["log_id"], "b");
    assert_eq!(logs[0]["workflow_id"], "workflow-a");
    assert_eq!(logs[0]["run_id"], "run-b");
    assert_eq!(logs[0]["node_id"], "writer");
    assert_eq!(logs[0]["unread"], true);

    let marked = handle_request(
        &state,
        IpcRequest {
            method: "mark_run_logs_read".to_owned(),
            params: json!({ "filter": { "run_id": "run-a" } }),
        },
    );
    assert!(marked.ok, "{:?}", marked.error);
    assert_eq!(marked.data, Some(json!(2)));

    let entries = store.read_all().unwrap();
    assert!(
        entries
            .iter()
            .find(|entry| entry.log_id == "b")
            .unwrap()
            .unread
    );
    assert!(entries
        .iter()
        .filter(|entry| entry
            .run_id
            .as_ref()
            .is_some_and(|id| id.as_str() == "run-a"))
        .all(|entry| !entry.unread));
}

#[test]
fn ipc_provider_removal_previews_revision_then_deletes_config_and_key() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    let state = AriadneAppState::new(project.path(), app_state.path(), secrets.clone());

    let saved = handle_request(
        &state,
        IpcRequest {
            method: "save_provider_settings".to_owned(),
            params: json!({
                "update": {
                    "provider_id": "target",
                    "provider_type": "open_ai",
                    "display_name": "Target",
                    "enabled": true,
                    "models": [{
                        "model_id": "target-model",
                        "capability": "llm"
                    }],
                    "make_default_llm": true,
                    "make_default_embedding": false,
                    "make_default_reranker": false
                }
            }),
        },
    );
    assert!(saved.ok, "{:?}", saved.error);
    let key_saved = handle_request(
        &state,
        IpcRequest {
            method: "save_provider_key".to_owned(),
            params: json!({ "provider": "target", "key": "secret" }),
        },
    );
    assert!(key_saved.ok, "{:?}", key_saved.error);

    let preview = handle_request(
        &state,
        IpcRequest {
            method: "preview_provider_removal".to_owned(),
            params: json!({ "provider": "target" }),
        },
    );
    assert!(preview.ok, "{:?}", preview.error);
    let preview = preview.data.unwrap();
    assert_eq!(preview["has_key"], true);
    assert_eq!(preview["default_roles"], json!(["llm"]));
    assert_eq!(preview["blocking_references"], json!([]));
    let revision = preview["revision"].as_str().unwrap().to_owned();

    let removed = handle_request(
        &state,
        IpcRequest {
            method: "remove_provider".to_owned(),
            params: json!({
                "provider": "target",
                "expected_revision": revision
            }),
        },
    );
    assert!(removed.ok, "{:?}", removed.error);
    let status = removed.data.unwrap();
    assert!(status["default_llm_provider_id"].is_null());
    assert!(!status["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|provider| provider["provider"] == "target" && provider["configured"] == true));
    assert!(
        !ariadne::commands::get_provider_config_impl(project.path(), secrets.as_ref())
            .unwrap()
            .providers
            .iter()
            .any(|provider| provider.provider == "target" && provider.has_key)
    );
}

#[test]
fn ipc_works_tree_and_summary_share_official_stage_projection() {
    use ariadne::contracts::{SourceSpan, TextRange};
    use ariadne::documents::{ChapterDocumentEntry, ChapterDocumentIndex, ChapterDocumentKind};
    use ariadne::rag::{MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, StorySegment};

    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let document_path = project.path().join("documents/chapter-ipc.md");
    std::fs::write(&document_path, "正文").unwrap();
    let index = ChapterDocumentIndex::new(
        "v1",
        vec![ChapterDocumentEntry {
            chapter_id: "wrong-prefix:chapter-ipc".to_owned(),
            document_id: "documents/chapter-ipc.md".to_owned(),
            path: document_path,
            title: "IPC 章节".to_owned(),
            order: 1,
            kind: ChapterDocumentKind::ChapterBody,
            version: "v1".to_owned(),
            word_count: None,
            outline_ref: None,
        }],
    )
    .unwrap();
    std::fs::write(
        project.path().join(".runtime/chapter_index.json"),
        serde_json::to_vec_pretty(&index).unwrap(),
    )
    .unwrap();
    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: "wrong-prefix:chapter-ipc::seg-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "wrong-prefix:chapter-ipc".to_owned(),
            summary: "IPC 故事段".to_owned(),
            source: SourceSpan {
                document_id: "documents/chapter-ipc.md".to_owned(),
                range: TextRange { start: 0, end: 6 },
                version: Some("v1".to_owned()),
            },
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_chapter_summary("wrong-prefix:chapter-ipc", "IPC 章节总结")
        .unwrap();
    knowledge
        .upsert_stage_summary("official-ipc-stage", "IPC 阶段总结")
        .unwrap();
    knowledge
        .link_chapter_stage("wrong-prefix:chapter-ipc", "official-ipc-stage")
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

    let tree = handle_request(
        &state,
        IpcRequest {
            method: "get_works_tree".to_owned(),
            params: Value::Null,
        },
    );
    assert!(tree.ok, "{:?}", tree.error);
    let summary = handle_request(
        &state,
        IpcRequest {
            method: "get_chapter_summary_view".to_owned(),
            params: json!({ "chapter_id": "wrong-prefix:chapter-ipc" }),
        },
    );
    assert!(summary.ok, "{:?}", summary.error);

    let tree_data = tree.data.unwrap();
    let tree_stage = tree_data["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["stage_id"] == "official-ipc-stage")
        .unwrap();
    assert_eq!(
        tree_stage["children"][0]["chapter_id"],
        "wrong-prefix:chapter-ipc"
    );
    let summary_data = summary.data.unwrap();
    assert_eq!(summary_data["stage"]["stage_id"], "official-ipc-stage");
    assert_eq!(summary_data["chapter_summary"], "IPC 章节总结");
    assert_eq!(
        summary_data["segments"][0]["source"]["document_id"],
        "documents/chapter-ipc.md"
    );
}

#[test]
fn ipc_search_project_documents_uses_project_retrieval_runtime() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    save_document_content_impl(
        project.path(),
        "documents/ipc-search.md".to_owned(),
        "月光下的银色线索".to_owned(),
    )
    .unwrap();
    process_index_outbox_impl(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let response = handle_request(
        &state,
        IpcRequest {
            method: "search_project_documents".to_owned(),
            params: json!({ "query": "银色线索", "limit": 5 }),
        },
    );

    assert!(response.ok, "{:?}", response.error);
    let results = response.data.unwrap().as_array().cloned().unwrap();
    assert!(results.iter().any(|result| result["snippet"]
        .as_str()
        .is_some_and(|text| text.contains("银色线索"))));
}

#[test]
fn ipc_can_explicitly_rebind_legacy_project_credentials_before_open() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let store = ConfigStore::new(project.path());
    let mut config = store.load_or_create().unwrap();
    config.providers.providers = vec![ProviderConfig {
        provider_id: "openai".to_owned(),
        provider_type: ProviderType::OpenAi,
        display_name: "OpenAI".to_owned(),
        enabled: true,
        base_url: None,
        api_key: Some(SecretRef::new("legacy-global-secret")),
        models: Vec::new(),
    }];
    config.providers.default_llm_provider_id = Some("openai".to_owned());
    let raw = yaml_serde::to_string(&yaml_serde::to_value(&config.providers).unwrap()).unwrap();
    std::fs::write(store.config_dir().join(PROVIDERS_CONFIG_FILE), raw).unwrap();

    let secrets = Arc::new(MemorySecretStore::default());
    let state = AriadneAppState::new("", app_state.path(), secrets.clone());
    let open = handle_request(
        &state,
        IpcRequest {
            method: "open_project".to_owned(),
            params: json!({ "project_root": project.path() }),
        },
    );
    assert!(!open.ok);
    assert!(open
        .error
        .as_deref()
        .is_some_and(|error| error.contains("untrusted project SecretRef")));

    let rebind = handle_request(
        &state,
        IpcRequest {
            method: "rebind_project_provider_key".to_owned(),
            params: json!({
                "project_root": project.path(),
                "provider": "openai",
                "key": "sk-rebound"
            }),
        },
    );
    assert!(rebind.ok, "{:?}", rebind.error);
    assert!(state.project_root().unwrap().as_os_str().is_empty());
    assert!(store
        .load()
        .unwrap()
        .providers
        .providers
        .iter()
        .all(|provider| provider.api_key.is_none()));
    assert!(
        ariadne::commands::get_provider_config_impl(project.path(), secrets.as_ref())
            .unwrap()
            .has_openai_key
    );
}

#[test]
fn ipc_pack_workflow_selection_returns_report_with_nested_workflow_graph() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "ipc-pack".to_owned(),
            name: "IPC Pack".to_owned(),
            nodes: vec![CanvasNode {
                id: "writer".to_owned(),
                r#type: "writer".to_owned(),
                label: Some("Writer".to_owned()),
                data: Value::Null,
                position: json!({ "x": 10.0, "y": 20.0 }),
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

    let response = handle_request(
        &state,
        IpcRequest {
            method: "pack_workflow_selection".to_owned(),
            params: json!({
                "workflow_id": "ipc-pack",
                "selected_node_ids": ["writer"],
                "subworkflow_node_id": "sub-writer",
                "title": "Writer Subflow",
                "operation_id": "ipc-pack-receipt"
            }),
        },
    );

    assert!(response.ok, "{:?}", response.error);
    let data = response
        .data
        .expect("pack response should include report data");
    assert_eq!(data["subworkflow_node_id"], "sub-writer");
    assert_eq!(data["workflow"]["workflow_id"], "ipc-pack");
    assert_eq!(data["workflow"]["nodes"][0]["id"], "sub-writer");
    assert_eq!(data["embedded_workflow"]["nodes"][0]["id"], "writer");
    assert!(
        data.get("nodes").is_none(),
        "report must not masquerade as a graph"
    );

    let recovered = handle_request(
        &state,
        IpcRequest {
            method: "get_pack_operation".to_owned(),
            params: json!({"operation_id": "ipc-pack-receipt"}),
        },
    );
    assert!(recovered.ok, "{:?}", recovered.error);
    assert_eq!(
        recovered
            .data
            .as_ref()
            .and_then(|value| value.get("operation_id"))
            .and_then(Value::as_str),
        Some("ipc-pack-receipt")
    );
    assert_eq!(
        recovered
            .data
            .as_ref()
            .and_then(|value| value.get("workflow"))
            .and_then(|value| value.get("content_revision")),
        data.get("workflow")
            .and_then(|value| value.get("content_revision"))
    );
}

#[test]
fn n8_get_pack_operation_commits_a_prepared_receipt_after_crash_window() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "ipc-pack-recovery".to_owned(),
            name: "IPC Pack Recovery".to_owned(),
            nodes: vec![CanvasNode {
                id: "writer".to_owned(),
                r#type: "writer".to_owned(),
                label: Some("Writer".to_owned()),
                data: Value::Null,
                position: json!({ "x": 10.0, "y": 20.0 }),
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .unwrap();
    let workflow_path = temp.path().join("workflows/ipc-pack-recovery.json");
    let base_workflow = std::fs::read(&workflow_path).unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let packed = handle_request(
        &state,
        IpcRequest {
            method: "pack_workflow_selection".to_owned(),
            params: json!({
                "workflow_id": "ipc-pack-recovery",
                "selected_node_ids": ["writer"],
                "subworkflow_node_id": "sub-writer",
                "title": "Writer Subflow",
                "operation_id": "ipc-pack-prepared-recovery"
            }),
        },
    );
    assert!(packed.ok, "{:?}", packed.error);
    let result_revision = packed.data.as_ref().unwrap()["workflow"]["content_revision"]
        .as_str()
        .unwrap()
        .to_owned();

    let operation_path = ariadne::config::project_authority_dir(
        temp.path(),
        app_state.path(),
        "workflow-pack-operations",
    )
    .unwrap()
    .join("ipc-pack-prepared-recovery.json");
    assert!(!operation_path.starts_with(temp.path()));
    assert!(!temp.path().join(".ariadne/ops").exists());
    let mut operation: Value =
        serde_json::from_str(&std::fs::read_to_string(&operation_path).unwrap()).unwrap();
    operation["status"] = Value::String("prepared".to_owned());
    std::fs::write(
        &operation_path,
        serde_json::to_vec_pretty(&operation).unwrap(),
    )
    .unwrap();
    std::fs::write(&workflow_path, base_workflow).unwrap();

    let recovered = handle_request(
        &state,
        IpcRequest {
            method: "get_pack_operation".to_owned(),
            params: json!({"operation_id": "ipc-pack-prepared-recovery"}),
        },
    );
    assert!(recovered.ok, "{:?}", recovered.error);
    assert_eq!(
        recovered.data.as_ref().unwrap()["workflow"]["content_revision"],
        result_revision
    );
    let loaded = ariadne::commands::load_workflow_graph_impl(
        temp.path(),
        Some("ipc-pack-recovery".to_owned()),
    )
    .unwrap();
    assert_eq!(
        loaded.content_revision.as_deref(),
        Some(result_revision.as_str())
    );
    assert_eq!(loaded.nodes.len(), 1);
    assert_eq!(loaded.nodes[0].id, "sub-writer");
    let committed: Value =
        serde_json::from_str(&std::fs::read_to_string(operation_path).unwrap()).unwrap();
    assert_eq!(committed["status"], "committed");
}

#[test]
fn n8_project_owned_pack_receipt_has_no_recovery_authority() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let project_ops = temp.path().join(".ariadne/ops");
    std::fs::create_dir_all(&project_ops).unwrap();
    std::fs::write(
        project_ops.join("forged-project-operation.json"),
        serde_json::to_vec_pretty(&json!({
            "operation_id": "forged-project-operation",
            "request_hash": "forged",
            "expected_revision": "forged",
            "status": "prepared",
            "report": {
                "workflow": {
                    "workflow_id": "default",
                    "name": "Forged",
                    "nodes": [],
                    "edges": [],
                    "metadata": {},
                    "content_revision": "forged"
                },
                "subworkflow_node_id": "forged",
                "embedded_workflow": {
                    "workflow_id": "forged",
                    "name": "Forged",
                    "nodes": [],
                    "edges": [],
                    "metadata": {}
                },
                "boundary_inputs": [],
                "boundary_outputs": [],
                "operation_id": "forged-project-operation"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let response = handle_request(
        &state,
        IpcRequest {
            method: "get_pack_operation".to_owned(),
            params: json!({"operation_id": "forged-project-operation"}),
        },
    );
    assert!(!response.ok);
    assert!(response
        .error
        .as_deref()
        .is_some_and(|error| error.contains("pack operation not found")));
    assert!(!temp.path().join("workflows/default.json").exists());
}

#[test]
fn ipc_run_workflow_starts_background_run_for_tool_callers() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "ipc-run".to_owned(),
            name: "IPC Run".to_owned(),
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

    let response = handle_request(
        &state,
        IpcRequest {
            method: "run_workflow".to_owned(),
            params: json!({
                "workflow_id": "ipc-run",
                "start_node_id": "start-main"
            }),
        },
    );

    assert!(response.ok, "{:?}", response.error);
    let data = response.data.expect("ipc response should include run data");
    assert_eq!(data["status"], "queued");
    let run_id = data["run_id"].as_str().expect("run_id should be a string");
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let initial = store
        .load_state(&WorkflowId::from("ipc-run"), &RunId::from(run_id))
        .unwrap()
        .expect("queued response must already have a queryable snapshot");
    assert!(initial
        .structured_events
        .iter()
        .any(|event| event.event_type == ariadne::workflow::WorkflowRuntimeEventType::RunQueued));
    let run_state = wait_for_terminal_workflow_state(
        &store,
        &WorkflowId::from("ipc-run"),
        &RunId::from(run_id),
    );

    assert_eq!(run_state.status, RunStatus::Succeeded);
    assert!(run_state.nodes.contains_key(&NodeId::from("start-main")));

    let events_response = handle_request(
        &state,
        IpcRequest {
            method: "get_workflow_events".to_owned(),
            params: json!({
                "workflow_id": "ipc-run",
                "run_id": run_id,
                "after_sequence": 0,
                "limit": 1
            }),
        },
    );
    assert!(events_response.ok, "{:?}", events_response.error);
    let events_data = events_response
        .data
        .expect("ipc response should include workflow events");
    assert_eq!(events_data["status"], "succeeded");
    assert_eq!(events_data["next_sequence"], 1);
    assert_eq!(events_data["events"].as_array().unwrap().len(), 1);
    assert_eq!(events_data["events"][0]["sequence"], 0);

    let next_response = handle_request(
        &state,
        IpcRequest {
            method: "get_workflow_events".to_owned(),
            params: json!({
                "workflow_id": "ipc-run",
                "run_id": run_id,
                "after_sequence": events_data["next_sequence"].as_u64().unwrap()
            }),
        },
    );
    assert!(next_response.ok, "{:?}", next_response.error);
    let next_data = next_response
        .data
        .expect("ipc response should include incremental events");
    assert!(!next_data["events"].as_array().unwrap().is_empty());
    assert!(next_data["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["sequence"].as_u64().unwrap() >= 1));
}

#[test]
fn ipc_project_ai_submits_workflow_without_waiting_for_approval() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "project-ai-approval".to_owned(),
            name: "Project AI Approval".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "start-main".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("Start".to_owned()),
                    data: Value::Null,
                    position: Value::Null,
                },
                CanvasNode {
                    id: "approval".to_owned(),
                    r#type: "approval".to_owned(),
                    label: Some("Approval".to_owned()),
                    data: json!({
                        "approval_id": "project-ai-approval-1",
                        "auto_approve": false
                    }),
                    position: Value::Null,
                },
            ],
            edges: vec![CanvasEdge {
                id: "start-approval".to_owned(),
                source: "start-main".to_owned(),
                target: "approval".to_owned(),
                source_handle: "exec_out".to_owned(),
                target_handle: "exec_in".to_owned(),
                kind: WorkflowEdgeKind::Control,
                label: None,
                data: Value::Null,
            }],
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

    let response = handle_request(
        &state,
        IpcRequest {
            method: "project_ai_chat".to_owned(),
            params: json!({
                "request": {
                    "message": "",
                    "workflow_id_to_run": "project-ai-approval"
                }
            }),
        },
    );

    assert!(response.ok, "{:?}", response.error);
    let data = response
        .data
        .expect("project AI response should include data");
    let run = data["workflow_run"]
        .as_object()
        .expect("project AI response should include a workflow run");
    assert_eq!(run["status"], "queued");
    let run_id = RunId::from(run["run_id"].as_str().unwrap());
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let workflow_id = WorkflowId::from("project-ai-approval");
    let queued = store
        .load_state(&workflow_id, &run_id)
        .unwrap()
        .expect("queued project AI run should already be queryable");
    assert!(queued
        .structured_events
        .iter()
        .any(|event| event.event_type == ariadne::workflow::WorkflowRuntimeEventType::RunQueued));

    let mut paused = None;
    for _ in 0..100 {
        paused = store.load_state(&workflow_id, &run_id).unwrap();
        if paused
            .as_ref()
            .is_some_and(|state| state.status == RunStatus::Paused)
        {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let paused = paused.expect("background project AI run should remain queryable");
    assert_eq!(paused.status, RunStatus::Paused);
    assert!(paused.confirmations.contains_key("project-ai-approval-1"));
}

#[test]
fn ipc_start_workflow_preflight_failure_does_not_return_queued() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let response = handle_request(
        &state,
        IpcRequest {
            method: "run_workflow".to_owned(),
            params: json!({ "workflow_id": "missing-workflow" }),
        },
    );

    assert!(!response.ok);
    assert!(response.data.is_none());
    assert!(response
        .error
        .expect("preflight error must be returned")
        .contains("workflow not found: missing-workflow"));
}

#[test]
fn ipc_lists_workflow_tools_for_external_agents() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "agent-tools".to_owned(),
            name: "Agent Tools".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-draft".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Draft Tool".to_owned()),
                data: json!({
                    "name": "Draft Tool",
                    "expose_as_tool": true,
                    "tool_input_schema": {
                        "type": "object",
                        "properties": {
                            "topic": { "type": "string" }
                        },
                        "required": ["topic"]
                    }
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
            tool_controls: std::collections::BTreeMap::from([(
                "project_ai".to_owned(),
                std::collections::BTreeMap::from([("project-ai-workflow-tools".to_owned(), true)]),
            )]),
        },
    )
    .unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let response = handle_request(
        &state,
        IpcRequest {
            method: "list_workflow_tools".to_owned(),
            params: Value::Null,
        },
    );

    assert!(response.ok, "{:?}", response.error);
    let data = response.data.expect("ipc response should include tools");
    assert_eq!(data[0]["name"], "draft_tool");
    assert_eq!(data[0]["workflow_id"], "agent-tools");
    assert_eq!(data[0]["start_node_id"], "start-draft");
    assert_eq!(
        data[0]["input_schema"]["properties"]["topic"]["type"],
        "string"
    );
}

#[test]
fn ipc_lists_saved_workflow_graphs_for_desktop_selector() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "draft/main".to_owned(),
            name: "Draft Main".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-draft".to_owned(),
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
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "review".to_owned(),
            name: "Review".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-review".to_owned(),
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
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let response = handle_request(
        &state,
        IpcRequest {
            method: "list_workflow_graphs".to_owned(),
            params: Value::Null,
        },
    );

    assert!(response.ok, "{:?}", response.error);
    let data = response
        .data
        .expect("ipc response should include workflow summaries");
    let summaries = data
        .as_array()
        .expect("workflow summaries should be a list");
    let ids: Vec<_> = summaries
        .iter()
        .map(|summary| summary["workflow_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["draft/main", "review"]);
    assert_eq!(summaries[0]["name"], "Draft Main");
    assert_eq!(summaries[0]["path"], "workflows/draft/main.json");
    assert_eq!(summaries[0]["node_count"], 1);
    assert_eq!(summaries[0]["edge_count"], 0);
}

#[test]
fn ipc_reports_git_repository_status_for_desktop_details() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let response = handle_request(
        &state,
        IpcRequest {
            method: "get_git_repository_status".to_owned(),
            params: Value::Null,
        },
    );

    assert!(response.ok, "{:?}", response.error);
    let data = response
        .data
        .expect("ipc response should include git repository status");
    assert_eq!(data["status"], "degraded");
    assert_eq!(data["dirty"], true);
    assert_eq!(data["diff_line_count"], 0);
}

#[test]
fn ipc_project_scoped_commands_reject_uninitialized_project_root() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let response = handle_request(
        &state,
        IpcRequest {
            method: "list_workflow_graphs".to_owned(),
            params: Value::Null,
        },
    );

    assert!(!response.ok);
    assert!(response
        .error
        .expect("ipc response should include project validation error")
        .contains("not initialized"));
}

#[test]
fn ipc_error_response_includes_stable_error_code() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = ariadne::commands::AriadneAppState::new(
        temp.path(),
        app_state.path(),
        std::sync::Arc::new(ariadne::config::MemorySecretStore::default()),
    );
    // Product dispatch creates the stable identity directly; no diagnostic keyword classifier exists.
    let response = handle_request(
        &state,
        IpcRequest {
            method: "unsupported_method".to_owned(),
            params: Value::Null,
        },
    );
    assert!(!response.ok, "unsupported method must fail");
    assert_eq!(response.error_code.as_deref(), Some("not_found"));
    assert_eq!(response.error_key.as_deref(), Some("ui.error.not_found"));
    assert!(
        response.error.as_ref().is_some_and(|e| !e.is_empty()),
        "diagnostic error string still present for tools"
    );
}

#[test]
fn ipc_save_workflow_requires_expected_revision_for_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let state = ariadne::commands::AriadneAppState::new(
        temp.path(),
        app_state.path(),
        std::sync::Arc::new(ariadne::config::MemorySecretStore::default()),
    );

    // First save creates workflow (no file yet for custom id).
    let create = handle_request(
        &state,
        IpcRequest {
            method: "save_workflow_graph".to_owned(),
            params: json!({
                "graph_data": {
                    "workflow_id": "cas-wf",
                    "name": "CAS",
                    "nodes": [],
                    "edges": [],
                    "metadata": null
                }
            }),
        },
    );
    assert!(create.ok, "{:?}", create.error);
    let rev1 = create.data.unwrap()["content_revision"]
        .as_str()
        .unwrap()
        .to_owned();

    // Stale revision rejected.
    let stale = handle_request(
        &state,
        IpcRequest {
            method: "save_workflow_graph".to_owned(),
            params: json!({
                "graph_data": {
                    "workflow_id": "cas-wf",
                    "name": "CAS2",
                    "nodes": [],
                    "edges": [],
                    "metadata": null,
                    "expected_revision": "deadbeef"
                }
            }),
        },
    );
    assert!(!stale.ok);
    assert_eq!(stale.error_code.as_deref(), Some("conflict"));

    // Matching revision succeeds and rotates hash.
    let ok = handle_request(
        &state,
        IpcRequest {
            method: "save_workflow_graph".to_owned(),
            params: json!({
                "graph_data": {
                    "workflow_id": "cas-wf",
                    "name": "CAS3",
                    "nodes": [],
                    "edges": [],
                    "metadata": null,
                    "expected_revision": rev1
                }
            }),
        },
    );
    assert!(ok.ok, "{:?}", ok.error);
    let rev2 = ok.data.unwrap()["content_revision"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(rev1, rev2);
}

#[test]
fn ipc_error_includes_error_key_from_structured_path() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = ariadne::commands::AriadneAppState::new(
        temp.path(),
        app_state.path(),
        std::sync::Arc::new(ariadne::config::MemorySecretStore::default()),
    );
    let response = handle_request(
        &state,
        IpcRequest {
            method: "open_project".to_owned(),
            params: json!({ "project_root": temp.path().join("nope").to_string_lossy() }),
        },
    );
    assert!(!response.ok);
    assert!(response.error_code.is_some());
    assert!(response
        .error_key
        .as_ref()
        .is_some_and(|k| k.starts_with("ui.error.")));
}
