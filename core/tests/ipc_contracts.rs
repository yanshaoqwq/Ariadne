use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ariadne::commands::{
    process_index_outbox_impl, save_document_content_impl, save_permissions_settings_impl,
    save_workflow_graph_impl, AriadneAppState, CanvasEdge, CanvasNode, PermissionsSettings,
    WorkflowGraphData,
};
use ariadne::config::{
    ConfigStore, MemorySecretStore, ProviderConfig, QdrantAuthMode, SecretRef, VectorStoreBackend,
    PROVIDERS_CONFIG_FILE,
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
fn ipc_project_lifecycle_creates_enters_and_closes_a_complete_project() {
    let parent = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let project_root = parent.path().join("ipc-project");
    let state = AriadneAppState::new("", app_state.path(), Arc::new(MemorySecretStore::default()));

    let created = handle_request(
        &state,
        IpcRequest {
            method: "create_project".to_owned(),
            params: json!({
                "project_root": project_root.to_string_lossy(),
                "name": "IPC 作品"
            }),
        },
    );
    assert!(created.ok, "{:?}", created.error);
    let report = created.data.unwrap();
    assert_eq!(report["ready"], true);
    assert_eq!(report["git_initialized"], true);
    assert_eq!(report["project_name"], "IPC 作品");
    assert_eq!(report["created_config_files"].as_array().unwrap().len(), 7);
    assert!(report["created_dirs"].as_array().unwrap().len() >= 9);
    assert!(project_root.join(".config/app.yaml").is_file());
    assert!(project_root.join("skills").is_dir());
    assert!(project_root.join("exports").is_dir());

    let current = handle_request(
        &state,
        IpcRequest {
            method: "get_current_project".to_owned(),
            params: Value::Null,
        },
    );
    assert!(current.ok, "{:?}", current.error);
    assert_eq!(current.data.unwrap()["project_name"], "IPC 作品");

    for method in [
        "get_app_settings",
        "get_provider_config",
        "get_permissions_settings",
        "get_automation_settings",
        "list_workflow_graphs",
    ] {
        let response = handle_request(
            &state,
            IpcRequest {
                method: method.to_owned(),
                params: Value::Null,
            },
        );
        assert!(response.ok, "{method}: {:?}", response.error);
    }

    let project_ai = handle_request(
        &state,
        IpcRequest {
            method: "project_ai_chat".to_owned(),
            params: json!({
                "request": {
                    "message": "检查新项目",
                    "chat_history": [],
                    "references": []
                }
            }),
        },
    );
    assert!(!project_ai.ok, "new project has no configured LLM provider");
    let project_ai_error = project_ai.error.unwrap_or_default();
    assert!(!project_ai_error.contains("not initialized"));
    assert!(!project_ai_error.contains("No such file"));

    let closed = handle_request(
        &state,
        IpcRequest {
            method: "close_project".to_owned(),
            params: Value::Null,
        },
    );
    assert!(closed.ok, "{:?}", closed.error);
    assert!(state.project_root().unwrap().as_os_str().is_empty());

    let status = handle_request(
        &state,
        IpcRequest {
            method: "get_app_status".to_owned(),
            params: Value::Null,
        },
    );
    assert!(status.ok, "{:?}", status.error);
    assert_eq!(status.data.unwrap()["current_project"]["project_root"], "");
}

#[test]
fn ipc_recent_project_forget_and_relocate_are_explicit_registry_operations() {
    let app_state = tempfile::tempdir().unwrap();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(first.path()).unwrap();
    ariadne::frontend::initialize_project(second.path()).unwrap();
    let state = AriadneAppState::new("", app_state.path(), Arc::new(MemorySecretStore::default()));

    let opened = handle_request(
        &state,
        IpcRequest {
            method: "open_project".to_owned(),
            params: json!({ "project_root": first.path() }),
        },
    );
    assert!(opened.ok, "{:?}", opened.error);
    let first_root = first.path().canonicalize().unwrap();

    let relocated = handle_request(
        &state,
        IpcRequest {
            method: "relocate_recent_project".to_owned(),
            params: json!({
                "previous_project_root": first_root,
                "project_root": second.path(),
            }),
        },
    );
    assert!(relocated.ok, "{:?}", relocated.error);

    let forgotten = handle_request(
        &state,
        IpcRequest {
            method: "forget_recent_project".to_owned(),
            params: json!({ "project_root": second.path().canonicalize().unwrap() }),
        },
    );
    assert!(forgotten.ok, "{:?}", forgotten.error);
    assert!(forgotten.data.unwrap().as_array().unwrap().is_empty());
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

/// U158（P1）：**成功的运行必须在「运行日志」页可见**。
///
/// 原缺陷：`UiRunLogStore::append` 全生产只有两个调用点、**都在失败路径上**
/// （工作流 worker 出错 + Git 恢复诊断），**没有任何一处在「运行成功」时写日志**。
/// 于是跑成功 10 次日志页空白、跑失败 1 次出现一条红色错误——
/// 用户的结论是「这页只显示错误」或「日志功能坏了」。
/// 连带侧边栏徽章只因失败而跳数，用户无法从徽章感知「后台跑完了」。
///
/// 数据不是没产生：同一次运行的 runtime events 有若干条，
/// 只是产生在**另一张表**里而日志页不看那张表。
///
/// **判据取「日志页读得到这次运行」而不是「query_run_logs 不抛异常」**：
/// 后者现在（缺陷版本下）就是绿的，一个恒返回空列表的实现照样能过。
/// 这里走**真实运行**（IPC 入口 → 后台 worker → 终态）之后，
/// 用**真实查询命令** `query_run_logs` 按 run_id 过滤，断言拿得到条目——
/// 这条链路上任何一环没接线都会红。
#[test]
fn ipc_successful_run_is_visible_in_the_run_log_page() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    // 纯 start 节点：不调 LLM，因此能真的跑到 Succeeded。
    // 用会调模型的节点会让用例变成「测连接失败」，那覆盖不到成功路径——
    // 而本编号要防的恰恰是「成功路径不写日志」。
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "log-visible".to_owned(),
            name: "Log Visible".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-main".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Start".to_owned()),
                data: json!({ "work_dir": "main" }),
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
                "workflow_id": "log-visible",
                "start_node_id": "start-main"
            }),
        },
    );
    assert!(response.ok, "{:?}", response.error);
    let run_id = response
        .data
        .expect("run response must include data")["run_id"]
        .as_str()
        .expect("run_id should be a string")
        .to_owned();

    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let run_state = wait_for_terminal_workflow_state(
        &store,
        &WorkflowId::from("log-visible"),
        &RunId::from(run_id.clone()),
    );
    // 前置：这次运行**真的成功了**。若它其实失败了，下面拿到的日志会是失败路径
    // 那条既有的错误日志，用例就变成了空测。
    assert_eq!(
        run_state.status,
        RunStatus::Succeeded,
        "本用例的前提是运行成功；失败则测不到「成功路径写日志」"
    );

    // 走真实查询命令，按 run_id 过滤——与日志页的做法一致。
    let logs_response = handle_request(
        &state,
        IpcRequest {
            method: "query_run_logs".to_owned(),
            params: json!({ "filter": { "run_id": run_id } }),
        },
    );
    assert!(logs_response.ok, "{:?}", logs_response.error);
    let logs = logs_response
        .data
        .expect("query_run_logs must return data");
    let entries = logs.as_array().expect("run logs must be an array");

    assert!(
        !entries.is_empty(),
        "成功的运行必须在运行日志里留下条目，否则用户跑成功 10 次也只看到空白页（U158）"
    );
    // 内容判据：条目要能让用户认出「这是那次运行、结果是成功」。
    // 只断言「非空」挡不住「写了一条空消息」——那在页面上是一行空白，等于没写。
    let outcome = entries
        .iter()
        .find(|entry| entry["metadata"]["outcome"] == "succeeded")
        .expect("必须有一条标记为 succeeded 的条目");
    assert_eq!(outcome["run_id"], run_id);
    assert_eq!(
        outcome["level"], "info",
        "成功不该以 error/warning 呈现——那正是「这页只显示错误」的观感来源"
    );
    assert!(
        outcome["message"]
            .as_str()
            .is_some_and(|message| !message.trim().is_empty()),
        "条目必须带非空消息，否则页面上是一行空白"
    );
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

/// U156（P0）：显式 `"variables": null` 必须被当作空表，不能拒收整条请求。
///
/// 缺陷形态：`RunWorkflowParams.variables` 是**非 `Option`** 的 `BTreeMap`
/// + `#[serde(default)]`，而 **`default` 只对「键缺失」生效、不接受显式 null**。
/// 桌面端曾发出 `"variables": null`（匿名对象做不到按需加键），于是
/// **参数解析就失败**、报 `invalid type: null, expected a map`——
/// 产品主功能全废，点运行什么都不会发生。
///
/// **判据取「错误信息是 workflow not found 而不是 invalid ipc params」**：
/// 前者证明参数**已成功解析、命令已执行**（只是探针没建那个 workflow），
/// 后者是解析层就被挡住。两者的区别正好是这个缺陷的全部——
/// 而「响应 ok 为 false」在两种情况下都成立，**拿它当判据就是空测**。
#[test]
fn ipc_run_workflow_accepts_explicit_null_variables() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    for method in ["run_workflow", "start_workflow"] {
        let response = handle_request(
            &state,
            IpcRequest {
                method: method.to_owned(),
                params: json!({
                    "workflow_id": "missing-workflow",
                    "start_node_id": null,
                    "variables": null,
                }),
            },
        );

        let error = response
            .error
            .unwrap_or_else(|| panic!("{method} 应当带回一条错误信息"));
        assert!(
            !error.contains("invalid ipc params"),
            "{method} 不得因显式 null 而在参数解析层被拒（U156）：{error}"
        );
        assert!(
            error.contains("workflow not found"),
            "{method} 的错误必须来自命令执行而非参数解析——\
             那才证明 null 已被当作空表吃下：{error}"
        );
    }
}

#[test]
fn ipc_lists_workflow_tools_for_external_agents() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project_with_app_state(temp.path(), app_state.path(), None)
        .unwrap();
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
            scoped_policies: std::collections::BTreeMap::new(),
            tool_controls: std::collections::BTreeMap::from([(
                "project_ai".to_owned(),
                std::collections::BTreeMap::from([(
                    "project-ai-workflow-tools".to_owned(),
                    Some(true),
                )]),
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
    assert_eq!(data[0]["workflow_id"], "default");
    assert_eq!(data[0]["start_node_id"], "agent-tools--start-draft");
    assert_eq!(
        data[0]["input_schema"]["properties"]["topic"]["type"],
        "string"
    );
}

#[test]
fn ipc_lists_the_single_merged_project_canvas() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project_with_app_state(temp.path(), app_state.path(), None)
        .unwrap();
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
    assert_eq!(ids, vec!["default"]);
    assert_eq!(summaries[0]["name"], "Project Canvas");
    assert_eq!(summaries[0]["path"], "workflows/default.json");
    assert_eq!(summaries[0]["node_count"], 2);
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
    // U207-F（2026-08-20）：原断言是 `diff_line_count == 0`。
    //
    // 那个 0 **编码的正是 U207-F 要修的缺陷**：这个项目没有任何 commit、
    // 12 章正文都是未跟踪文件 ⇒ status 带 `--untracked-files=all` 说「脏」，
    // 而裸 `git diff` 只比「工作区↔索引」、看不见未跟踪文件 ⇒ 报 0 行。
    // 同一屏上「存在未提交变更」与「0 行 diff」互相打脸，作者无法判断有没有东西要存。
    //
    // 口径对齐后它必然非 0。⚠️ 断言改成 `> 0` 而不是钉死 177：
    // 那个数字取决于测试装置写了多少行正文，钉死它会让「往装置里多加一章」
    // 这种无关改动把用例弄红。要守的性质是**两行数据不再互相矛盾**。
    let diff_lines = data["diff_line_count"]
        .as_u64()
        .expect("diff_line_count should be a number");
    assert!(
        diff_lines > 0,
        "dirty=true 却报 {diff_lines} 行 diff ⇒ 版本页又会同屏显示两个互相打脸的值（U207-F）"
    );
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
fn ipc_settings_error_preserves_field_recovery_and_correlation_context() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let mut config = ConfigStore::new(project.path()).load_or_create().unwrap();
    config.rag.vector_store.backend = VectorStoreBackend::ExternalQdrant;
    config.rag.vector_store.sidecar.host = "qdrant.example".to_owned();
    config.rag.vector_store.sidecar.auth_mode = QdrantAuthMode::ApiKey;

    let response = handle_request(
        &state,
        IpcRequest {
            method: "save_rag_settings".to_owned(),
            params: json!({
                "settings": {
                    "rag": config.rag,
                    "qdrant_api_key": null,
                    "clear_qdrant_api_key": false,
                    "has_qdrant_api_key": false
                }
            }),
        },
    );

    assert!(!response.ok);
    assert_eq!(response.error_code.as_deref(), Some("validation"));
    assert_eq!(response.error_field.as_deref(), Some("qdrant_api_key"));
    assert_eq!(response.error_section.as_deref(), Some("retrieval"));
    assert_eq!(
        response.recovery_action.as_deref(),
        Some("replace_credential")
    );
    assert!(response
        .correlation_id
        .as_deref()
        .is_some_and(|value| value.starts_with("err-")));
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

/// U118：无系统钥匙链时，保存 Provider 密钥的**完整出路必须真实可达**。
///
/// 原缺陷不在「保存失败」本身，而在失败信息指向一个**产品里不存在**的操作：
/// 它让用户去「设置本地主密码」，而当时 IPC 零命令、UI 零入口、文案零条目。
/// GUI 用户照着提示做只会撞墙，等于所有需要 API Key 的功能全部不可用。
///
/// 所以这条用例走**真实 IPC 派发**：只有命令在派发表里可达，修复才算数。
/// 单测直接调函数是证明不了这一点的——那正是缺陷能长期存在的原因。
#[test]
fn ipc_exposes_a_reachable_way_out_when_keychain_is_unavailable() {
    let parent = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let project_root = parent.path().join("u118-project");
    // 用本地文件 store 且不给主密码：等价于 Linux 无 Secret Service 的处境。
    let secrets = Arc::new(ariadne::config::LocalFileSecretStore::new(
        app_state.path().join("secrets.json"),
    ));
    let state = AriadneAppState::new("", app_state.path(), secrets);

    let created = handle_request(
        &state,
        IpcRequest {
            method: "create_project".to_owned(),
            params: json!({ "project_root": project_root.to_string_lossy(), "name": "U118" }),
        },
    );
    assert!(created.ok, "{:?}", created.error);

    // 1) 状态可查，且明确告知需要处置——UI 据此才知道要弹窗。
    let status = handle_request(
        &state,
        IpcRequest {
            method: "get_secret_protection".to_owned(),
            params: Value::Null,
        },
    );
    assert!(status.ok, "{:?}", status.error);
    let status = status.data.unwrap();
    assert_eq!(status["status"], "locked");
    assert_eq!(status["requires_setup"], true);

    // 2) 出路一：设主密码。命令必须在派发表里真实可达。
    let unlocked = handle_request(
        &state,
        IpcRequest {
            method: "set_local_secret_master_password".to_owned(),
            params: json!({ "master_password": "u118-master" }),
        },
    );
    assert!(
        unlocked.ok,
        "U118：set_local_secret_master_password 不可达，错误信息仍指向不存在的操作：{:?}",
        unlocked.error
    );
    assert_eq!(unlocked.data.unwrap()["status"], "encrypted");

    // 3) 解锁后保存密钥应当成功——这才是用户真正想做的事。
    //    save_provider_key 要求 provider 已配置，先建一个。
    let configured = handle_request(
        &state,
        IpcRequest {
            method: "save_provider_settings".to_owned(),
            params: json!({
                "update": {
                    "provider_id": "openai",
                    "provider_type": "open_ai_compatible",
                    "display_name": "OpenAI",
                    "enabled": true,
                    "base_url": "https://api.openai.com/v1",
                    "models": [{
                        "model_id": "gpt-4.1-mini",
                        "capability": "llm"
                    }],
                    "make_default_llm": true,
                    "make_default_embedding": false,
                    "make_default_reranker": false,
                    "make_default_search": false
                }
            }),
        },
    );
    assert!(configured.ok, "{:?}", configured.error);

    let saved = handle_request(
        &state,
        IpcRequest {
            method: "save_provider_key".to_owned(),
            params: json!({ "provider": "openai", "key": "sk-u118" }),
        },
    );
    assert!(saved.ok, "解锁后保存 Provider 密钥应当成功：{:?}", saved.error);

    // 4) 凭据必须以密文落盘，明文不得出现在文件里。
    let secrets_file = std::fs::read_to_string(app_state.path().join("secrets.json")).unwrap();
    assert!(
        !secrets_file.contains("sk-u118"),
        "设了主密码后凭据仍以明文落盘"
    );
}

/// U118：明文模式必须是**显式选择**，且选择后在诊断里持续可见。
///
/// 用户当时点头同意了明文，但三个月后未必记得自己的 API Key 正躺在磁盘上。
/// 诊断面板是「这台机器现在处于什么状态」的唯一常驻答案，所以明文必须报
/// `degraded` 而非 `healthy`——否则「同意过」就成了一次性的、再也看不见的决定。
#[test]
fn ipc_unprotected_mode_is_opt_in_and_stays_visible_in_diagnostics() {
    let parent = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let project_root = parent.path().join("u118-plain");
    let secrets = Arc::new(ariadne::config::LocalFileSecretStore::new(
        app_state.path().join("secrets.json"),
    ));
    let state = AriadneAppState::new("", app_state.path(), secrets);
    let created = handle_request(
        &state,
        IpcRequest {
            method: "create_project".to_owned(),
            params: json!({ "project_root": project_root.to_string_lossy(), "name": "U118P" }),
        },
    );
    assert!(created.ok, "{:?}", created.error);

    let allowed = handle_request(
        &state,
        IpcRequest {
            method: "allow_unprotected_local_secrets".to_owned(),
            params: Value::Null,
        },
    );
    assert!(allowed.ok, "{:?}", allowed.error);
    assert_eq!(allowed.data.unwrap()["status"], "unprotected");

    let diagnostics = handle_request(
        &state,
        IpcRequest {
            method: "get_backend_diagnostics".to_owned(),
            params: Value::Null,
        },
    );
    assert!(diagnostics.ok, "{:?}", diagnostics.error);
    let items = diagnostics.data.unwrap();
    let item = items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["component"] == "secrets.protection")
        .cloned()
        .expect("诊断必须常驻报告凭据保护状态");
    assert_eq!(
        item["status"], "degraded",
        "明文存储必须报 degraded；报 healthy 会让用户以为凭据是受保护的"
    );
}
