use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::commands::{self, AriadneAppState, RunLogQuery, RunWorkflowRequest, WorkflowRunStarted};
#[cfg(not(feature = "system-keychain"))]
use crate::config::LocalFileSecretStore;
use crate::config::SecretStore;
#[cfg(feature = "system-keychain")]
use crate::config::SystemKeychainSecretStore;
use crate::contracts::RunStatus;
use crate::frontend::{UiRunLogKind, UiRunLogLevel};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInvocationResult {
    pub exit_code: i32,
    pub stdout: String,
}

pub fn run_cli(args: impl IntoIterator<Item = String>) -> CliInvocationResult {
    match run_cli_command(args) {
        Ok(data) => CliInvocationResult {
            exit_code: 0,
            stdout: serialize_cli_response(json!({ "ok": true, "data": data })),
        },
        Err(error) => CliInvocationResult {
            exit_code: 1,
            stdout: serialize_cli_response(json!({ "ok": false, "error": error })),
        },
    }
}

pub fn run_cli_command(args: impl IntoIterator<Item = String>) -> Result<Value, String> {
    let args = args.into_iter().collect::<Vec<_>>();
    if args.is_empty() || is_help(args.first()) {
        return Ok(json!({ "usage": usage() }));
    }

    match args[0].as_str() {
        "version" | "--version" | "-V" => Ok(json!({
            "product_version": crate::PRODUCT_VERSION,
            "ipc_schema_version": crate::IPC_SCHEMA_VERSION,
        })),
        "workflow" => run_workflow_command(&args[1..]),
        "tools" => run_tools_command(&args[1..]),
        other => Err(format!("unsupported ariadne command: {other}\n{}", usage())),
    }
}

fn run_workflow_command(args: &[String]) -> Result<Value, String> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(workflow_usage());
    };
    if is_help(args.first()) {
        return Ok(json!({ "usage": workflow_usage() }));
    }
    let options = CliOptions::parse(&args[1..])?;
    let state = state_from_options(&options)?;

    match action {
        "run" => {
            let workflow_id = required(&options.workflow, "--workflow")?;
            let initial_inputs = options
                .inputs_json
                .as_deref()
                .map(parse_json_object)
                .transpose()?
                .unwrap_or_default();
            let request = RunWorkflowRequest {
                workflow_id,
                start_node_id: options.start.clone(),
                initial_inputs,
            };
            commands::start_workflow_with_request(&state, request.clone()).map(|started| {
                workflow_started_json(request.workflow_id, request.start_node_id, started)
            })
        }
        "status" => {
            let workflow_id = required(&options.workflow, "--workflow")?;
            let run_id = required(&options.run, "--run")?;
            let state =
                commands::get_workflow_run_state(&state, workflow_id.clone(), run_id.clone())?
                    .ok_or_else(|| format!("workflow run not found: {workflow_id}/{run_id}"))?;
            Ok(json!({
                "workflow_id": workflow_id,
                "run_id": run_id,
                "status": run_status_label(state.status),
                "start_node_id": state.start_node_id.map(|id| id.as_str().to_owned()),
                "event_count": state.structured_events.len(),
            }))
        }
        "events" => {
            let workflow_id = required(&options.workflow, "--workflow")?;
            let run_id = required(&options.run, "--run")?;
            commands::get_workflow_events(
                &state,
                workflow_id,
                run_id,
                options.after_sequence,
                options.limit,
            )
            .map(|events| json!(events))
        }
        "logs" => {
            let run_id = required(&options.run, "--run")?;
            let query = RunLogQuery {
                kind: options.kind,
                level: options.level,
                workflow_id: options.workflow.clone(),
                run_id: Some(run_id),
                node_id: options.node.clone(),
                query: options.query.clone(),
                after_timestamp_ms: None,
                after_log_id: None,
                limit: options.limit,
            };
            commands::query_run_logs(&state, Some(query)).map(|items| json!({ "items": items }))
        }
        "pause" => {
            let workflow_id = required(&options.workflow, "--workflow")?;
            let run_id = required(&options.run, "--run")?;
            commands::pause_workflow(&state, workflow_id, run_id, options.reason)
                .map(|result| json!(result))
        }
        "resume" => {
            let workflow_id = required(&options.workflow, "--workflow")?;
            let run_id = required(&options.run, "--run")?;
            commands::resume_workflow(&state, workflow_id, run_id).map(|result| json!(result))
        }
        "stop" => {
            let workflow_id = required(&options.workflow, "--workflow")?;
            let run_id = required(&options.run, "--run")?;
            commands::stop_workflow(&state, workflow_id, run_id, options.reason)
                .map(|result| json!(result))
        }
        other => Err(format!(
            "unsupported workflow command: {other}\n{}",
            workflow_usage()
        )),
    }
}

fn run_tools_command(args: &[String]) -> Result<Value, String> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(tools_usage());
    };
    if is_help(args.first()) {
        return Ok(json!({ "usage": tools_usage() }));
    }
    let options = CliOptions::parse(&args[1..])?;
    let state = state_from_options(&options)?;
    match action {
        "list" => {
            commands::list_external_workflow_tools(&state).map(|tools| json!({ "tools": tools }))
        }
        other => Err(format!(
            "unsupported tools command: {other}\n{}",
            tools_usage()
        )),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct CliOptions {
    project: Option<PathBuf>,
    app_state: Option<PathBuf>,
    workflow: Option<String>,
    run: Option<String>,
    start: Option<String>,
    inputs_json: Option<String>,
    after_sequence: Option<u64>,
    limit: Option<usize>,
    reason: Option<String>,
    level: Option<UiRunLogLevel>,
    kind: Option<UiRunLogKind>,
    node: Option<String>,
    query: Option<String>,
}

impl CliOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Self::default();
        let mut index = 0usize;
        while index < args.len() {
            match args[index].as_str() {
                "--json" => index += 1,
                "--project" => {
                    options.project =
                        Some(PathBuf::from(next_value(args, &mut index, "--project")?));
                }
                "--app-state" => {
                    options.app_state =
                        Some(PathBuf::from(next_value(args, &mut index, "--app-state")?));
                }
                "--workflow" => {
                    options.workflow = Some(next_value(args, &mut index, "--workflow")?)
                }
                "--run" => options.run = Some(next_value(args, &mut index, "--run")?),
                "--start" => options.start = Some(next_value(args, &mut index, "--start")?),
                "--inputs-json" => {
                    options.inputs_json = Some(next_value(args, &mut index, "--inputs-json")?);
                }
                "--after" | "--after-sequence" => {
                    let value = next_value(args, &mut index, "--after")?;
                    options.after_sequence = Some(parse_u64("--after", &value)?);
                }
                "--limit" => {
                    let value = next_value(args, &mut index, "--limit")?;
                    options.limit = Some(parse_usize("--limit", &value)?);
                }
                "--reason" => options.reason = Some(next_value(args, &mut index, "--reason")?),
                "--level" => {
                    let value = next_value(args, &mut index, "--level")?;
                    options.level = Some(parse_enum("--level", &value)?);
                }
                "--kind" => {
                    let value = next_value(args, &mut index, "--kind")?;
                    options.kind = Some(parse_enum("--kind", &value)?);
                }
                "--node" | "--node-id" => {
                    options.node = Some(next_value(args, &mut index, "--node")?);
                }
                "--query" | "-q" => {
                    options.query = Some(next_value(args, &mut index, "--query")?);
                }
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unsupported option: {other}")),
            }
        }
        Ok(options)
    }
}

fn next_value(args: &[String], index: &mut usize, name: &'static str) -> Result<String, String> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| format!("{name} requires a value"))?
        .clone();
    *index += 1;
    Ok(value)
}

fn state_from_options(options: &CliOptions) -> Result<AriadneAppState, String> {
    let project_root = options
        .project
        .clone()
        .or_else(|| std::env::var_os("ARIADNE_PROJECT_ROOT").map(PathBuf::from))
        .ok_or_else(|| "--project is required unless ARIADNE_PROJECT_ROOT is set".to_owned())?;
    let app_state_root = options
        .app_state
        .clone()
        .or_else(|| std::env::var_os("ARIADNE_APP_STATE_ROOT").map(PathBuf::from))
        .unwrap_or_else(commands::default_app_state_root);
    Ok(AriadneAppState::new(
        project_root,
        app_state_root.clone(),
        cli_secret_store(&app_state_root),
    ))
}

#[cfg(feature = "system-keychain")]
fn cli_secret_store(_app_state_root: &Path) -> Arc<dyn SecretStore> {
    Arc::new(SystemKeychainSecretStore::default())
}

#[cfg(not(feature = "system-keychain"))]
fn cli_secret_store(app_state_root: &Path) -> Arc<dyn SecretStore> {
    Arc::new(LocalFileSecretStore::new(
        app_state_root.join("secrets.json"),
    ))
}

fn required(value: &Option<String>, name: &'static str) -> Result<String, String> {
    value
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| format!("{name} is required"))
}

fn parse_json_object(value: &str) -> Result<std::collections::BTreeMap<String, Value>, String> {
    match serde_json::from_str::<Value>(value)
        .map_err(|error| format!("invalid --inputs-json: {error}"))?
    {
        Value::Object(map) => Ok(map.into_iter().collect()),
        other => Err(format!(
            "--inputs-json must be a JSON object, got {}",
            json_value_kind(&other)
        )),
    }
}

fn parse_u64(name: &'static str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{name} must be an unsigned integer"))
}

fn parse_usize(name: &'static str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be an unsigned integer"))
}

fn parse_enum<T: DeserializeOwned>(name: &'static str, value: &str) -> Result<T, String> {
    serde_json::from_value(json!(value)).map_err(|_| format!("invalid {name}: {value}"))
}

fn workflow_started_json(
    workflow_id: String,
    start_node_id: Option<String>,
    started: WorkflowRunStarted,
) -> Value {
    json!({
        "workflow_id": workflow_id,
        "start_node_id": start_node_id,
        "run_id": started.run_id,
        "status": started.status,
    })
}

fn serialize_cli_response(value: Value) -> String {
    let mut output = serde_json::to_string(&value).unwrap_or_else(|_| {
        r#"{"ok":false,"error":"failed to serialize CLI response"}"#.to_owned()
    });
    output.push('\n');
    output
}

fn is_help(value: Option<&String>) -> bool {
    matches!(value.map(String::as_str), Some("--help" | "-h" | "help"))
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn run_status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::Stopping => "stopping",
        RunStatus::Stopped => "stopped",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

fn usage() -> String {
    [
        "usage:",
        "  ariadne workflow run --project <path> --workflow <id> [--start <id>] [--inputs-json <json>] [--json]",
        "  ariadne workflow status --project <path> --workflow <id> --run <run-id> [--json]",
        "  ariadne workflow events --project <path> --workflow <id> --run <run-id> [--after <n>] [--limit <n>] [--json]",
        "  ariadne workflow logs --project <path> --run <run-id> [--workflow <id>] [--level info|warning|error] [--kind <kind>] [--query <text>] [--json]",
        "  ariadne workflow pause|resume|stop --project <path> --workflow <id> --run <run-id> [--reason <text>] [--json]",
        "  ariadne tools list --project <path> [--json]",
    ]
    .join("\n")
}

fn workflow_usage() -> String {
    usage()
}

fn tools_usage() -> String {
    "usage: ariadne tools list --project <path> [--json]".to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    use crate::commands::{
        append_run_log, save_permissions_settings_impl, save_workflow_graph_impl, CanvasNode,
        PermissionsSettings, WorkflowGraphData,
    };
    use crate::contracts::{PermissionPolicy, RunId, WorkflowId};
    use crate::frontend::{UiRunLogEntry, UiRunLogKind, UiRunLogLevel};

    use super::run_cli_command;

    #[test]
    fn cli_tools_list_exposes_workflow_tools() {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
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
                        "expose_as_tool": true,
                        "tool_input_schema": {
                            "type": "object",
                            "properties": {
                                "topic": { "type": "string" }
                            }
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
                tool_controls: BTreeMap::from([(
                    "project_ai".to_owned(),
                    BTreeMap::from([("project-ai-workflow-tools".to_owned(), true)]),
                )]),
            },
        )
        .unwrap();

        let output = run_cli_command(
            [
                "tools",
                "list",
                "--project",
                temp.path().to_str().unwrap(),
                "--app-state",
                app_state.path().to_str().unwrap(),
                "--json",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap();

        assert_eq!(output["tools"][0]["name"], "draft_tool");
        assert_eq!(output["tools"][0]["workflow_id"], "tool-flow");
        assert_eq!(output["tools"][0]["start_node_id"], "start-draft");
    }

    #[test]
    fn cli_workflow_logs_filter_by_run_and_level() {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
        append_run_log(
            temp.path(),
            UiRunLogEntry {
                log_id: "log-a".to_owned(),
                timestamp_ms: 0,
                kind: UiRunLogKind::Error,
                level: UiRunLogLevel::Error,
                message: "writer failed".to_owned(),
                workflow_id: Some(WorkflowId::from("wf")),
                run_id: Some(RunId::from("run-a")),
                node_id: None,
                unread: false,
                metadata: Value::Null,
            },
        )
        .unwrap();
        append_run_log(
            temp.path(),
            UiRunLogEntry {
                log_id: "log-b".to_owned(),
                timestamp_ms: 0,
                kind: UiRunLogKind::Node,
                level: UiRunLogLevel::Info,
                message: "other".to_owned(),
                workflow_id: Some(WorkflowId::from("wf")),
                run_id: Some(RunId::from("run-b")),
                node_id: None,
                unread: false,
                metadata: Value::Null,
            },
        )
        .unwrap();

        let output = run_cli_command(
            [
                "workflow",
                "logs",
                "--project",
                temp.path().to_str().unwrap(),
                "--app-state",
                app_state.path().to_str().unwrap(),
                "--run",
                "run-a",
                "--level",
                "error",
                "--query",
                "writer",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap();

        assert_eq!(output["items"].as_array().unwrap().len(), 1);
        assert_eq!(output["items"][0]["log_id"], "log-a");
    }
}
