use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ariadne::commands::{save_workflow_graph_impl, AriadneAppState, CanvasNode, WorkflowGraphData};
use ariadne::config::MemorySecretStore;
use ariadne::contracts::{NodeId, RunId, RunStatus, WorkflowId};
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
}
