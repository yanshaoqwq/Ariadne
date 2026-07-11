use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ariadne::commands::{
    save_permissions_settings_impl, save_workflow_graph_impl, AriadneAppState, CanvasNode,
    PermissionsSettings, WorkflowGraphData,
};
use ariadne::config::MemorySecretStore;
use ariadne::contracts::{NodeId, PermissionPolicy, RunId, RunStatus, WorkflowId};
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
    assert!(next_data["events"].as_array().unwrap().len() >= 1);
    assert!(next_data["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["sequence"].as_u64().unwrap() >= 1));
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
