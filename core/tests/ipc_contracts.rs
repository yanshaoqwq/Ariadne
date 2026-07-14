use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ariadne::commands::{
    process_index_outbox_impl, save_document_content_impl, save_permissions_settings_impl,
    save_workflow_graph_impl, AriadneAppState, CanvasNode, PermissionsSettings, WorkflowGraphData,
};
use ariadne::config::{
    ConfigStore, MemorySecretStore, ProviderConfig, SecretRef, PROVIDERS_CONFIG_FILE,
};
use ariadne::contracts::{NodeId, PermissionPolicy, ProviderType, RunId, RunStatus, WorkflowId};
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
                "title": "Writer Subflow"
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
    assert_eq!(
        ariadne::ipc::classify_ipc_error("validation failed: name required"),
        "validation"
    );
    assert_eq!(
        ariadne::ipc::classify_ipc_error("permission denied for tool: network"),
        "permission"
    );
    assert_eq!(
        ariadne::ipc::classify_ipc_error("workflow run not found: wf/run"),
        "not_found"
    );
    assert_eq!(
        ariadne::ipc::classify_ipc_error("connection refused to 127.0.0.1"),
        "network"
    );

    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = ariadne::commands::AriadneAppState::new(
        temp.path(),
        app_state.path(),
        std::sync::Arc::new(ariadne::config::MemorySecretStore::default()),
    );
    // Open a path that is not an Ariadne project → validation/not_found style failure with error_code.
    let response = handle_request(
        &state,
        IpcRequest {
            method: "open_project".to_owned(),
            params: json!({
                "project_root": temp.path().join("missing-project").to_string_lossy(),
            }),
        },
    );
    assert!(!response.ok, "expected failure for missing project");
    let code = response
        .error_code
        .as_deref()
        .expect("ok:false must include error_code");
    assert!(!code.is_empty(), "error_code must be non-empty");
    assert!(
        response.error.as_ref().is_some_and(|e| !e.is_empty()),
        "diagnostic error string still present for tools"
    );
    // Free-form English stays in diagnostic only; code is stable identity.
    assert!(
        matches!(
            code,
            "validation" | "not_found" | "io" | "unknown" | "permission"
        ),
        "unexpected code {code}"
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
