use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commands::{self, AriadneAppState, RunLogQuery, RunWorkflowRequest, WorkflowRunStarted};
use crate::frontend::{UiRunLogKind, UiRunLogLevel};

const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 32 * 1024;
const HTTP_IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestServerConfig {
    pub bind: String,
    pub bearer_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestResponse {
    pub status_code: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestWorkflowRunStarted {
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_node_id: Option<String>,
    pub run_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RestControlRequest {
    #[serde(default)]
    reason: Option<String>,
}

pub fn run_rest_server(state: AriadneAppState, config: RestServerConfig) -> io::Result<()> {
    validate_rest_server_config(&config)?;
    let listener = TcpListener::bind(&config.bind)?;
    let shared_state = Arc::new(state);
    let shared_config = Arc::new(config);
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        let state = Arc::clone(&shared_state);
        let config = Arc::clone(&shared_config);
        std::thread::Builder::new()
            .name("ariadne-rest-request".to_owned())
            .spawn(move || {
                if let Err(error) = handle_connection(&mut stream, &state, &config) {
                    let response = error_response(500, error.to_string());
                    let _ = write_http_response(&mut stream, response);
                }
            })?;
    }
    Ok(())
}

pub fn handle_rest_request(
    state: &AriadneAppState,
    config: &RestServerConfig,
    request: RestRequest,
) -> RestResponse {
    if let Err(error) = validate_rest_server_config(config) {
        return error_response(500, error.to_string());
    }

    let route = match RestRoute::parse(&request.path) {
        Ok(route) => route,
        Err(error) => return error_response(400, error),
    };

    if !route.is_public() && !authorized(&request, &config.bearer_token) {
        return error_response(401, "missing or invalid bearer token");
    }

    match dispatch_rest_route(state, request, route) {
        Ok(value) => json_response(200, value),
        Err(RestRouteError {
            status_code,
            message,
        }) => error_response(status_code, message),
    }
}

fn handle_connection(
    stream: &mut TcpStream,
    state: &AriadneAppState,
    config: &RestServerConfig,
) -> io::Result<()> {
    stream.set_read_timeout(Some(HTTP_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(HTTP_IO_TIMEOUT))?;
    let request = read_http_request(stream)?;
    let response = handle_rest_request(state, config, request);
    write_http_response(stream, response)
}

fn dispatch_rest_route(
    state: &AriadneAppState,
    request: RestRequest,
    route: RestRoute,
) -> Result<Value, RestRouteError> {
    match route {
        RestRoute::Health => {
            require_method(&request, "GET")?;
            Ok(json!({ "status": "ok" }))
        }
        RestRoute::Tools => {
            require_method(&request, "GET")?;
            commands::list_external_workflow_tools(state)
                .map(|tools| json!({ "tools": tools }))
                .map_err(command_error)
        }
        RestRoute::RunWorkflow { workflow_id } => {
            require_method(&request, "POST")?;
            let payload = if request.body.is_empty() {
                RunWorkflowBody::default()
            } else {
                parse_json_body::<RunWorkflowBody>(&request)?
            };
            let request = RunWorkflowRequest {
                workflow_id: workflow_id.clone(),
                start_node_id: payload.start_node_id,
                initial_inputs: payload.inputs,
            };
            let started = commands::start_workflow_with_request(state, request.clone())
                .map_err(command_error)?;
            Ok(json!(RestWorkflowRunStarted::from_parts(
                workflow_id,
                request.start_node_id,
                started,
            )))
        }
        RestRoute::RunStatus {
            workflow_id,
            run_id,
        } => {
            require_method(&request, "GET")?;
            let state =
                commands::get_workflow_run_state(state, workflow_id.clone(), run_id.clone())
                    .map_err(command_error)?
                    .ok_or_else(|| RestRouteError::new(404, "workflow run not found"))?;
            Ok(json!({
                "workflow_id": workflow_id,
                "run_id": run_id,
                "status": run_status_label(state.status),
                "start_node_id": state.start_node_id.map(|id| id.as_str().to_owned()),
                "event_count": state.structured_events.len(),
            }))
        }
        RestRoute::RunEvents {
            workflow_id,
            run_id,
            query,
        } => {
            require_method(&request, "GET")?;
            let after_sequence = query
                .get("after_sequence")
                .or_else(|| query.get("after"))
                .map(|value| parse_u64("after_sequence", value))
                .transpose()?;
            let limit = query
                .get("limit")
                .map(|value| parse_usize("limit", value))
                .transpose()?;
            commands::get_workflow_events(state, workflow_id, run_id, after_sequence, limit)
                .map(|events| json!(events))
                .map_err(command_error)
        }
        RestRoute::RunLogs {
            workflow_id,
            run_id,
            query,
        } => {
            require_method(&request, "GET")?;
            let query = run_log_query_from_rest(workflow_id, run_id, query)?;
            commands::query_run_logs(state, Some(query))
                .map(|items| json!({ "items": items }))
                .map_err(command_error)
        }
        RestRoute::RunControl {
            workflow_id,
            run_id,
            action,
        } => {
            require_method(&request, "POST")?;
            let payload = if request.body.is_empty() {
                RestControlRequest::default()
            } else {
                parse_json_body::<RestControlRequest>(&request)?
            };
            let result = match action.as_str() {
                "pause" => commands::pause_workflow(state, workflow_id, run_id, payload.reason),
                "resume" => commands::resume_workflow(state, workflow_id, run_id),
                "stop" => commands::stop_workflow(state, workflow_id, run_id, payload.reason),
                _ => return Err(RestRouteError::new(404, "unsupported workflow run action")),
            }
            .map_err(command_error)?;
            Ok(json!(result))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RestRoute {
    Health,
    Tools,
    RunWorkflow {
        workflow_id: String,
    },
    RunStatus {
        workflow_id: String,
        run_id: String,
    },
    RunEvents {
        workflow_id: String,
        run_id: String,
        query: BTreeMap<String, String>,
    },
    RunLogs {
        workflow_id: Option<String>,
        run_id: String,
        query: BTreeMap<String, String>,
    },
    RunControl {
        workflow_id: String,
        run_id: String,
        action: String,
    },
}

impl RestRoute {
    fn parse(path: &str) -> Result<Self, String> {
        let (path, query) = split_path_query(path);
        let segments = path_segments(path)?;
        let segments = segments.iter().map(String::as_str).collect::<Vec<_>>();
        match segments.as_slice() {
            ["health"] | ["v1", "health"] => Ok(Self::Health),
            ["v1", "tools", "workflows"] | ["v1", "projects", _, "tools", "workflows"] => {
                Ok(Self::Tools)
            }
            ["v1", "workflows", workflow_id, "runs"]
            | ["v1", "projects", _, "workflows", workflow_id, "runs"] => Ok(Self::RunWorkflow {
                workflow_id: workflow_id.to_string(),
            }),
            ["v1", "workflows", workflow_id, "runs", run_id]
            | ["v1", "projects", _, "workflows", workflow_id, "runs", run_id] => {
                Ok(Self::RunStatus {
                    workflow_id: workflow_id.to_string(),
                    run_id: run_id.to_string(),
                })
            }
            ["v1", "workflows", workflow_id, "runs", run_id, "events"]
            | ["v1", "projects", _, "workflows", workflow_id, "runs", run_id, "events"] => {
                Ok(Self::RunEvents {
                    workflow_id: workflow_id.to_string(),
                    run_id: run_id.to_string(),
                    query,
                })
            }
            ["v1", "runs", run_id, "logs"] | ["v1", "projects", _, "runs", run_id, "logs"] => {
                Ok(Self::RunLogs {
                    workflow_id: None,
                    run_id: run_id.to_string(),
                    query,
                })
            }
            ["v1", "workflows", workflow_id, "runs", run_id, "logs"]
            | ["v1", "projects", _, "workflows", workflow_id, "runs", run_id, "logs"] => {
                Ok(Self::RunLogs {
                    workflow_id: Some(workflow_id.to_string()),
                    run_id: run_id.to_string(),
                    query,
                })
            }
            ["v1", "workflows", workflow_id, "runs", run_id, action]
            | ["v1", "projects", _, "workflows", workflow_id, "runs", run_id, action] => {
                Ok(Self::RunControl {
                    workflow_id: workflow_id.to_string(),
                    run_id: run_id.to_string(),
                    action: action.to_string(),
                })
            }
            _ => Err("unsupported REST route".to_owned()),
        }
    }

    fn is_public(&self) -> bool {
        matches!(self, Self::Health)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RunWorkflowBody {
    #[serde(default)]
    start_node_id: Option<String>,
    #[serde(default)]
    inputs: BTreeMap<String, Value>,
}

impl RestWorkflowRunStarted {
    fn from_parts(
        workflow_id: String,
        start_node_id: Option<String>,
        started: WorkflowRunStarted,
    ) -> Self {
        Self {
            workflow_id,
            start_node_id,
            run_id: started.run_id,
            status: started.status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RestRouteError {
    status_code: u16,
    message: String,
}

impl RestRouteError {
    fn new(status_code: u16, message: impl Into<String>) -> Self {
        Self {
            status_code,
            message: message.into(),
        }
    }
}

fn validate_rest_server_config(config: &RestServerConfig) -> io::Result<()> {
    if config.bearer_token.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "REST bearer token is required",
        ));
    }
    if !bind_is_loopback(&config.bind) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "REST bind address must be loopback; remote plaintext HTTP is not supported",
        ));
    }
    Ok(())
}

fn bind_is_loopback(bind: &str) -> bool {
    bind.starts_with("127.")
        || bind.starts_with("localhost:")
        || bind.starts_with("[::1]:")
        || bind.starts_with("::1:")
}

fn authorized(request: &RestRequest, token: &str) -> bool {
    request
        .headers
        .get("authorization")
        .and_then(|value| value.split_once(char::is_whitespace))
        .is_some_and(|(scheme, value)| scheme.eq_ignore_ascii_case("bearer") && value == token)
}

fn require_method(request: &RestRequest, expected: &str) -> Result<(), RestRouteError> {
    if request.method.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(RestRouteError::new(
            405,
            format!("method must be {expected}"),
        ))
    }
}

fn parse_json_body<T: for<'de> Deserialize<'de>>(
    request: &RestRequest,
) -> Result<T, RestRouteError> {
    serde_json::from_slice(&request.body)
        .map_err(|error| RestRouteError::new(400, format!("invalid JSON body: {error}")))
}

fn command_error(error: crate::command_error::CommandError) -> RestRouteError {
    use crate::command_error::CommandErrorCode;

    let status = match error.code {
        CommandErrorCode::Validation | CommandErrorCode::Serialization => 400,
        CommandErrorCode::Permission => 403,
        CommandErrorCode::NotFound | CommandErrorCode::LegacyRun => 404,
        CommandErrorCode::Conflict
        | CommandErrorCode::Cancelled
        | CommandErrorCode::Paused
        | CommandErrorCode::Stopped
        | CommandErrorCode::ExternalOutcomeUnknown => 409,
        CommandErrorCode::Budget | CommandErrorCode::ResourceLimit => 429,
        CommandErrorCode::Network | CommandErrorCode::External => 502,
        CommandErrorCode::Io
        | CommandErrorCode::Ipc
        | CommandErrorCode::Internal
        | CommandErrorCode::Unknown => 500,
    };
    RestRouteError::new(status, error.to_string())
}

fn parse_u64(field: &str, value: &str) -> Result<u64, RestRouteError> {
    value
        .parse::<u64>()
        .map_err(|_| RestRouteError::new(400, format!("{field} must be an unsigned integer")))
}

fn parse_usize(field: &str, value: &str) -> Result<usize, RestRouteError> {
    value
        .parse::<usize>()
        .map_err(|_| RestRouteError::new(400, format!("{field} must be an unsigned integer")))
}

fn run_log_query_from_rest(
    workflow_id: Option<String>,
    run_id: String,
    query: BTreeMap<String, String>,
) -> Result<RunLogQuery, RestRouteError> {
    Ok(RunLogQuery {
        kind: query
            .get("kind")
            .map(|value| parse_query_enum::<UiRunLogKind>("kind", value))
            .transpose()?,
        level: query
            .get("level")
            .map(|value| parse_query_enum::<UiRunLogLevel>("level", value))
            .transpose()?,
        workflow_id,
        run_id: Some(run_id),
        node_id: query.get("node_id").cloned(),
        query: query.get("query").or_else(|| query.get("q")).cloned(),
        after_timestamp_ms: query
            .get("after_timestamp_ms")
            .map(|value| {
                value.parse::<u64>().map_err(|_| {
                    RestRouteError::new(400, "after_timestamp_ms must be an unsigned integer")
                })
            })
            .transpose()?,
        after_log_id: query.get("after_log_id").cloned(),
        limit: query
            .get("limit")
            .map(|value| parse_usize("limit", value))
            .transpose()?,
        descending: query
            .get("descending")
            .map(|value| value.parse::<bool>())
            .transpose()
            .map_err(|_| RestRouteError::new(400, "descending must be true or false"))?
            .unwrap_or(false),
    })
}

fn parse_query_enum<T: for<'de> Deserialize<'de>>(
    field: &str,
    value: &str,
) -> Result<T, RestRouteError> {
    serde_json::from_value(json!(value))
        .map_err(|_| RestRouteError::new(400, format!("invalid {field}: {value}")))
}

fn split_path_query(path: &str) -> (&str, BTreeMap<String, String>) {
    let Some((path, query_raw)) = path.split_once('?') else {
        return (path, BTreeMap::new());
    };
    let mut query = BTreeMap::new();
    for pair in query_raw.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if let (Ok(key), Ok(value)) = (percent_decode(key), percent_decode(value)) {
            query.insert(key, value);
        }
    }
    (path, query)
}

fn path_segments(path: &str) -> Result<Vec<String>, String> {
    path.trim_start_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .map(percent_decode)
        .collect()
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err("invalid percent encoding".to_owned());
                }
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .map_err(|_| "invalid percent encoding".to_owned())?;
                let byte = u8::from_str_radix(hex, 16)
                    .map_err(|_| "invalid percent encoding".to_owned())?;
                output.push(byte);
                index += 3;
            }
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).map_err(|_| "invalid utf-8 in URL".to_owned())
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<RestRequest> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut header_bytes = request_line.len();
    if header_bytes > MAX_HTTP_HEADER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP request headers are too large",
        ));
    }
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }
    let method = parts[0].to_owned();
    let path = parts[1].to_owned();
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        header_bytes = header_bytes.saturating_add(line.len());
        if header_bytes > MAX_HTTP_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP request headers are too large",
            ));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid content-length"))?
        .unwrap_or(0);
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request body is too large",
        ));
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    Ok(RestRequest {
        method,
        path,
        headers,
        body,
    })
}

fn write_http_response(stream: &mut TcpStream, response: RestResponse) -> io::Result<()> {
    let reason = http_reason(response.status_code);
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status_code,
        reason,
        response.body.len()
    )?;
    stream.write_all(&response.body)
}

fn json_response(status_code: u16, data: Value) -> RestResponse {
    let body = serde_json::to_vec(&json!({
        "ok": true,
        "data": data,
    }))
    .unwrap_or_else(|_| br#"{"ok":false,"error":"failed to serialize response"}"#.to_vec());
    RestResponse { status_code, body }
}

fn error_response(status_code: u16, error: impl Into<String>) -> RestResponse {
    let body = serde_json::to_vec(&json!({
        "ok": false,
        "error": error.into(),
    }))
    .unwrap_or_else(|_| br#"{"ok":false,"error":"failed to serialize error"}"#.to_vec());
    RestResponse { status_code, body }
}

fn http_reason(status_code: u16) -> &'static str {
    match status_code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn run_status_label(status: crate::contracts::RunStatus) -> &'static str {
    match status {
        crate::contracts::RunStatus::Queued => "queued",
        crate::contracts::RunStatus::Running => "running",
        crate::contracts::RunStatus::Paused => "paused",
        crate::contracts::RunStatus::Stopping => "stopping",
        crate::contracts::RunStatus::Stopped => "stopped",
        crate::contracts::RunStatus::Succeeded => "succeeded",
        crate::contracts::RunStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use serde_json::{json, Value};

    use crate::commands::{
        append_run_log, save_permissions_settings_impl, save_workflow_graph_impl, AriadneAppState,
        CanvasNode, PermissionsSettings, WorkflowGraphData,
    };
    use crate::config::MemorySecretStore;
    use crate::contracts::{PermissionPolicy, RunId, WorkflowId};
    use crate::frontend::{UiRunLogEntry, UiRunLogKind, UiRunLogLevel};

    use super::{handle_rest_request, RestRequest, RestServerConfig};

    fn test_config() -> RestServerConfig {
        RestServerConfig {
            bind: "127.0.0.1:4817".to_owned(),
            bearer_token: "test-token".to_owned(),
        }
    }

    #[test]
    fn remote_bind_is_rejected_without_override() {
        let config = RestServerConfig {
            bind: "0.0.0.0:4817".to_owned(),
            bearer_token: "test-token".to_owned(),
        };
        assert!(super::validate_rest_server_config(&config).is_err());
    }

    fn json_body(response: super::RestResponse) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn health_route_does_not_require_authentication() {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
        let state = AriadneAppState::new(
            temp.path().to_path_buf(),
            app_state.path().to_path_buf(),
            Arc::new(MemorySecretStore::default()),
        );

        let response = handle_rest_request(
            &state,
            &test_config(),
            RestRequest {
                method: "GET".to_owned(),
                path: "/health".to_owned(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            },
        );
        let body = json_body(response.clone());

        assert_eq!(response.status_code, 200);
        assert_eq!(body["data"]["status"], "ok");
    }

    #[test]
    fn protected_routes_require_bearer_token() {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
        let state = AriadneAppState::new(
            temp.path().to_path_buf(),
            app_state.path().to_path_buf(),
            Arc::new(MemorySecretStore::default()),
        );

        let response = handle_rest_request(
            &state,
            &test_config(),
            RestRequest {
                method: "GET".to_owned(),
                path: "/v1/tools/workflows".to_owned(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            },
        );

        assert_eq!(response.status_code, 401);
    }

    #[test]
    fn workflow_tools_route_lists_exposed_start_nodes() {
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
                        "work_dir": "draft",
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
                tool_controls: BTreeMap::from([(
                    "project_ai".to_owned(),
                    BTreeMap::from([("project-ai-workflow-tools".to_owned(), true)]),
                )]),
            },
        )
        .unwrap();
        let state = AriadneAppState::new(
            temp.path().to_path_buf(),
            app_state.path().to_path_buf(),
            Arc::new(MemorySecretStore::default()),
        );
        let mut headers = BTreeMap::new();
        headers.insert("authorization".to_owned(), "Bearer test-token".to_owned());

        let response = handle_rest_request(
            &state,
            &test_config(),
            RestRequest {
                method: "GET".to_owned(),
                path: "/v1/projects/current/tools/workflows".to_owned(),
                headers,
                body: Vec::new(),
            },
        );
        let body = json_body(response.clone());

        assert_eq!(response.status_code, 200);
        assert_eq!(body["data"]["tools"][0]["name"], "draft_tool");
        assert_eq!(body["data"]["tools"][0]["workflow_id"], "tool-flow");
        assert_eq!(body["data"]["tools"][0]["start_node_id"], "start-draft");
        assert_eq!(
            body["data"]["tools"][0]["input_schema"]["properties"]["topic"]["type"],
            "string"
        );
    }

    #[test]
    fn workflow_run_route_accepts_empty_body_for_default_run() {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
        save_workflow_graph_impl(
            temp.path(),
            WorkflowGraphData {
                workflow_id: "empty-run".to_owned(),
                name: "Empty Run".to_owned(),
                nodes: vec![CanvasNode {
                    id: "start-main".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("Start".to_owned()),
                    data: json!({
                        "name": "Start",
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
            temp.path().to_path_buf(),
            app_state.path().to_path_buf(),
            Arc::new(MemorySecretStore::default()),
        );
        let mut headers = BTreeMap::new();
        headers.insert("authorization".to_owned(), "bearer test-token".to_owned());

        let response = handle_rest_request(
            &state,
            &test_config(),
            RestRequest {
                method: "POST".to_owned(),
                path: "/v1/workflows/empty-run/runs".to_owned(),
                headers,
                body: Vec::new(),
            },
        );
        let body = json_body(response.clone());

        assert_eq!(response.status_code, 200);
        assert_eq!(body["data"]["workflow_id"], "empty-run");
        assert_eq!(body["data"]["status"], "queued");
        assert!(body["data"]["run_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
    }

    #[test]
    fn run_logs_route_filters_by_run_and_query_params() {
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
                message: "other run".to_owned(),
                workflow_id: Some(WorkflowId::from("wf")),
                run_id: Some(RunId::from("run-b")),
                node_id: None,
                unread: false,
                metadata: Value::Null,
            },
        )
        .unwrap();
        let state = AriadneAppState::new(
            temp.path().to_path_buf(),
            app_state.path().to_path_buf(),
            Arc::new(MemorySecretStore::default()),
        );
        let mut headers = BTreeMap::new();
        headers.insert("authorization".to_owned(), "Bearer test-token".to_owned());

        let response = handle_rest_request(
            &state,
            &test_config(),
            RestRequest {
                method: "GET".to_owned(),
                path: "/v1/projects/current/runs/run-a/logs?level=error&q=writer".to_owned(),
                headers,
                body: Vec::new(),
            },
        );
        let body = json_body(response.clone());

        assert_eq!(response.status_code, 200);
        assert_eq!(body["data"]["items"].as_array().unwrap().len(), 1);
        assert_eq!(body["data"]["items"][0]["log_id"], "log-a");
        assert_eq!(body["data"]["items"][0]["run_id"], "run-a");
    }
}
