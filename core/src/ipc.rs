use std::io::{self, BufRead};
use std::thread;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commands::{self, AriadneAppState, CommandResult};

#[derive(Debug, Deserialize)]
pub struct IpcRequest {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct IpcResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl IpcResponse {
    fn ok(data: impl Serialize) -> Self {
        Self {
            ok: true,
            data: Some(serde_json::to_value(data).unwrap_or(Value::Null)),
            error: None,
        }
    }

    fn error(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error.into()),
        }
    }
}

pub fn handle_request(state: &AriadneAppState, request: IpcRequest) -> IpcResponse {
    match dispatch_request(state, request) {
        Ok(value) => IpcResponse::ok(value),
        Err(error) => IpcResponse::error(error),
    }
}

pub fn run_json_line_stdio() -> io::Result<()> {
    let state = AriadneAppState::default_for_process();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<IpcRequest>(&line) {
            Ok(request) => handle_request(&state, request),
            Err(error) => IpcResponse::error(error.to_string()),
        };
        println!(
            "{}",
            serde_json::to_string(&response).expect("ipc response should serialize")
        );
    }
    Ok(())
}

pub fn run_single_call(method: &str, params_json: Option<&str>) -> CommandResult<IpcResponse> {
    if method.trim().is_empty() {
        return Err("ipc method cannot be empty".to_owned());
    }
    let state = AriadneAppState::default_for_process();
    Ok(handle_request(
        &state,
        IpcRequest {
            method: method.to_owned(),
            params: parse_call_params(params_json)?,
        },
    ))
}

pub fn run_watch_workflow_events(
    workflow_id: &str,
    run_id: &str,
    after_sequence: u64,
    limit: Option<usize>,
    interval_ms: u64,
) -> CommandResult<()> {
    if workflow_id.trim().is_empty() {
        return Err("workflow_id cannot be empty".to_owned());
    }
    if run_id.trim().is_empty() {
        return Err("run_id cannot be empty".to_owned());
    }
    let state = AriadneAppState::default_for_process();
    let mut next_sequence = after_sequence;
    let interval = Duration::from_millis(interval_ms.max(50));
    let mut missing_run_wait_ms = 0u64;
    loop {
        let result = match commands::get_workflow_events(
            &state,
            workflow_id.to_owned(),
            run_id.to_owned(),
            Some(next_sequence),
            limit,
        ) {
            Ok(result) => result,
            Err(error)
                if error.contains("workflow run not found") && missing_run_wait_ms < 30_000 =>
            {
                thread::sleep(interval);
                missing_run_wait_ms =
                    missing_run_wait_ms.saturating_add(interval.as_millis() as u64);
                continue;
            }
            Err(error) => return Err(error),
        };
        missing_run_wait_ms = 0;
        let terminal = workflow_status_is_terminal(&result.status);
        if !result.events.is_empty() || terminal {
            next_sequence = result.next_sequence;
            println!(
                "{}",
                serde_json::to_string(&IpcResponse::ok(&result))
                    .map_err(|error| error.to_string())?
            );
        }
        if terminal {
            return Ok(());
        }
        thread::sleep(interval);
    }
}

pub fn parse_call_params(params_json: Option<&str>) -> CommandResult<Value> {
    let Some(params_json) = params_json.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Value::Null);
    };
    serde_json::from_str(params_json).map_err(|error| format!("invalid ipc params JSON: {error}"))
}

fn workflow_status_is_terminal(status: &str) -> bool {
    matches!(status, "stopped" | "succeeded" | "failed")
}

fn dispatch_request(state: &AriadneAppState, request: IpcRequest) -> CommandResult<Value> {
    match request.method.as_str() {
        "list_recent_projects" => ok(commands::list_recent_projects(state)?),
        "create_project" => {
            let params: ProjectSelectionParams = params(request.params)?;
            ok(commands::create_project(
                state,
                params.project_root,
                params.name,
            )?)
        }
        "open_project" => {
            let params: ProjectSelectionParams = params(request.params)?;
            ok(commands::open_project(
                state,
                params.project_root,
                params.name,
            )?)
        }
        "get_current_project" => ok(commands::get_current_project(state)?),
        "get_app_status" => ok(commands::get_app_status(state)?),
        "get_sidebar_badges" => ok(commands::get_sidebar_badges(state)?),
        "set_project_root" => {
            let params: ProjectRootParams = params(request.params)?;
            ok(commands::set_project_root(state, params.project_root)?)
        }
        "get_works_tree" => ok(commands::get_works_tree(state)?),
        "get_document_tree" => {
            let params: DocumentTreeParams = params(request.params)?;
            ok(commands::get_document_tree(state, params.project_id)?)
        }
        "get_document_content" => {
            let params: DocumentContentParams = params(request.params)?;
            ok(commands::get_document_content(
                state,
                params.document_id,
                params.path,
            )?)
        }
        "get_document_content_details" => {
            let params: DocumentContentParams = params(request.params)?;
            ok(commands::get_document_content_details(
                state,
                params.document_id,
                params.path,
            )?)
        }
        "save_document_content" => {
            let params: SaveDocumentContentParams = params(request.params)?;
            ok(commands::save_document_content_with_version(
                state,
                params.document_id,
                params.content,
                params.base_version,
            )?)
        }
        "import_chapter" => {
            let params: RequestParam<crate::frontend::ChapterImportRequest> =
                params(request.params)?;
            ok(commands::import_chapter(state, params.request)?)
        }
        "export_chapters" => {
            let params: ExportChaptersParams = params(request.params)?;
            ok(commands::export_chapters(
                state,
                params.selected_chapter_ids,
                params.artifact_id,
                params.format,
            )?)
        }
        "load_workflow_graph" => {
            let params: WorkflowIdParams = params(request.params)?;
            ok(commands::load_workflow_graph(state, params.workflow_id)?)
        }
        "list_workflow_graphs" => ok(commands::list_workflow_graphs(state)?),
        "validate_workflow_graph" => {
            let params: WorkflowGraphParams = params(request.params)?;
            ok(commands::validate_workflow_graph(params.graph_data)?)
        }
        "save_workflow_graph" => {
            let params: WorkflowGraphParams = params(request.params)?;
            ok(commands::save_workflow_graph(state, params.graph_data)?)
        }
        "apply_node_detail_patch" => {
            let params: ApplyNodeDetailPatchParams = params(request.params)?;
            ok(commands::apply_node_detail_patch(
                state,
                params.workflow_id,
                params.patch,
            )?)
        }
        "upsert_canvas_annotation" => {
            let params: UpsertCanvasAnnotationParams = params(request.params)?;
            ok(commands::upsert_canvas_annotation(
                state,
                params.workflow_id,
                params.annotation,
            )?)
        }
        "set_node_breakpoint" => {
            let params: SetNodeBreakpointParams = params(request.params)?;
            ok(commands::set_node_breakpoint(
                state,
                params.workflow_id,
                params.node_id,
                params.enabled,
            )?)
        }
        "export_workflow_selection" => {
            let params: WorkflowSelectionParams = params(request.params)?;
            ok(commands::export_workflow_selection(
                state,
                params.workflow_id,
                params.selected_node_ids,
            )?)
        }
        "pack_workflow_selection" => {
            let params: PackWorkflowSelectionParams = params(request.params)?;
            ok(commands::pack_workflow_selection(
                state,
                params.workflow_id,
                params.selected_node_ids,
                params.subworkflow_node_id,
                params.title,
            )?)
        }
        "run_workflow" => {
            let params: RunWorkflowParams = params(request.params)?;
            ok(commands::run_workflow(
                state,
                params.workflow_id,
                params.start_node_id,
            )?)
        }
        "start_workflow" => {
            let params: RunWorkflowParams = params(request.params)?;
            ok(commands::start_workflow(
                state,
                params.workflow_id,
                params.start_node_id,
            )?)
        }
        "pause_workflow" => {
            let params: RunControlParams = params(request.params)?;
            ok(commands::pause_workflow(
                state,
                params.workflow_id,
                params.run_id,
                params.reason,
            )?)
        }
        "stop_workflow" => {
            let params: RunControlParams = params(request.params)?;
            ok(commands::stop_workflow(
                state,
                params.workflow_id,
                params.run_id,
                params.reason,
            )?)
        }
        "resume_workflow" => {
            let params: RunIdentityParams = params(request.params)?;
            ok(commands::resume_workflow(
                state,
                params.workflow_id,
                params.run_id,
            )?)
        }
        "get_workflow_run_state" => {
            let params: RunIdentityParams = params(request.params)?;
            ok(commands::get_workflow_run_state(
                state,
                params.workflow_id,
                params.run_id,
            )?)
        }
        "get_workflow_events" => {
            let params: WorkflowEventsParams = params(request.params)?;
            ok(commands::get_workflow_events(
                state,
                params.workflow_id,
                params.run_id,
                params.after_sequence,
                params.limit,
            )?)
        }
        "get_budget_status" => ok(commands::get_budget_status(state)?),
        "get_app_settings" => ok(commands::get_app_settings(state)?),
        "save_app_settings" => {
            let params: SettingsParam<commands::AppSettings> = params(request.params)?;
            ok(commands::save_app_settings(state, params.settings)?)
        }
        "get_rag_settings" => ok(commands::get_rag_settings(state)?),
        "save_rag_settings" => {
            let params: SettingsParam<commands::RagSettings> = params(request.params)?;
            ok(commands::save_rag_settings(state, params.settings)?)
        }
        "get_workflow_settings" => ok(commands::get_workflow_settings(state)?),
        "save_workflow_settings" => {
            let params: SettingsParam<commands::WorkflowSettings> = params(request.params)?;
            ok(commands::save_workflow_settings(state, params.settings)?)
        }
        "get_git_settings" => ok(commands::get_git_settings(state)?),
        "save_git_settings" => {
            let params: SettingsParam<commands::GitSettings> = params(request.params)?;
            ok(commands::save_git_settings(state, params.settings)?)
        }
        "get_template_repository_settings" => {
            ok(commands::get_template_repository_settings(state)?)
        }
        "save_template_repository_settings" => {
            let params: SettingsParam<commands::TemplateRepositorySettings> =
                params(request.params)?;
            ok(commands::save_template_repository_settings(
                state,
                params.settings,
            )?)
        }
        "get_display_name_language_pack_template" => {
            let params: LanguagePackParams = params(request.params)?;
            ok(commands::get_display_name_language_pack_template(
                params.target_language,
            )?)
        }
        "validate_display_name_language_pack" => {
            let params: LanguagePackValidationParams = params(request.params)?;
            ok(commands::validate_display_name_language_pack(
                params.target_language,
                params.overlay,
            )?)
        }
        "update_budget_config" => {
            let params: UpdateBudgetParams = params(request.params)?;
            ok(commands::update_budget_config(
                state,
                params.budget_usd,
                params.preauthorized_usd,
            )?)
        }
        "set_auto_mode" => {
            let params: EnabledParams = params(request.params)?;
            ok(commands::set_auto_mode(state, params.enabled)?)
        }
        "get_automation_settings" => ok(commands::get_automation_settings(state)?),
        "save_automation_settings" => {
            let params: SettingsParam<commands::AutomationSettings> = params(request.params)?;
            ok(commands::save_automation_settings(state, params.settings)?)
        }
        "get_permissions_settings" => ok(commands::get_permissions_settings(state)?),
        "save_permissions_settings" => {
            let params: SettingsParam<commands::PermissionsSettings> = params(request.params)?;
            ok(commands::save_permissions_settings(state, params.settings)?)
        }
        "get_node_preset_settings" => ok(commands::get_node_preset_settings(state)?),
        "save_node_preset_settings" => {
            let params: SettingsParam<commands::NodePresetSettings> = params(request.params)?;
            ok(commands::save_node_preset_settings(state, params.settings)?)
        }
        "fetch_provider_models" => {
            let params: ProviderModelsParams = params(request.params)?;
            ok(commands::fetch_provider_models(state, params.provider_id)?)
        }
        "list_confirmations" => ok(commands::list_confirmations(state)?),
        "get_confirmation" => {
            let params: ConfirmationIdParams = params(request.params)?;
            ok(commands::get_confirmation(state, params.confirmation_id)?)
        }
        "resolve_confirmation" => {
            let params: RequestParam<commands::ResolveConfirmationRequest> =
                params(request.params)?;
            ok(commands::resolve_confirmation(state, params.request)?)
        }
        // 路径 B：改写被拒确认项输出并通过。
        "override_confirmation_output" => {
            let params: RequestParam<commands::OverrideConfirmationOutputRequest> =
                params(request.params)?;
            ok(commands::override_confirmation_output(
                state,
                params.request,
            )?)
        }
        // 路径 A：注入外部正文并从指定节点下游重跑。
        "resume_from_node" => {
            let params: RequestParam<commands::ResumeFromNodeRequest> = params(request.params)?;
            ok(commands::resume_from_node(state, params.request)?)
        }
        "get_git_history" => ok(commands::get_git_history(state)?),
        "get_git_branch_graph" => {
            let params: LimitParams = params(request.params)?;
            ok(commands::get_git_branch_graph(state, params.limit)?)
        }
        "create_checkpoint" => {
            let params: MessageParams = params(request.params)?;
            ok(commands::create_checkpoint(state, params.message)?)
        }
        "restore_to_new_branch" => {
            let params: RestoreToNewBranchParams = params(request.params)?;
            ok(commands::restore_to_new_branch(
                state,
                params.commit_id,
                params.new_branch,
            )?)
        }
        "get_provider_config" => ok(commands::get_provider_config(state)?),
        "save_provider_key" => {
            let params: SaveProviderKeyParams = params(request.params)?;
            ok(commands::save_provider_key(
                state,
                params.provider,
                params.key,
            )?)
        }
        "save_provider_settings" => {
            let params: UpdateParam<commands::ProviderSettingsUpdate> = params(request.params)?;
            ok(commands::save_provider_settings(state, params.update)?)
        }
        "query_run_logs" => {
            let params: RunLogFilterParams = params(request.params)?;
            ok(commands::query_run_logs(state, params.filter)?)
        }
        "mark_run_logs_read" => ok(commands::mark_run_logs_read(state)?),
        "read_project_memory" => ok(commands::read_project_memory(state)?),
        "append_project_memory" => {
            let params: ContentParams = params(request.params)?;
            ok(commands::append_project_memory(state, params.content)?)
        }
        "write_project_memory" => {
            let params: ContentParams = params(request.params)?;
            ok(commands::write_project_memory(state, params.content)?)
        }
        "quick_edit" => {
            let params: RequestParam<commands::QuickEditRequest> = params(request.params)?;
            ok(commands::quick_edit(state, params.request)?)
        }
        "apply_quick_edit" => {
            let params: ApplyQuickEditParams = params(request.params)?;
            ok(commands::apply_quick_edit(
                state,
                params.document_id,
                params.base_version,
                params.text,
                params.range,
                params.result,
            )?)
        }
        "project_ai_chat" => {
            let params: RequestParam<commands::ProjectAiRequest> = params(request.params)?;
            ok(commands::project_ai_chat(state, params.request)?)
        }
        "list_workflow_tools" => ok(commands::list_external_workflow_tools(state)?),
        "resolve_project_reference" => {
            let params: ReferenceParams = params(request.params)?;
            ok(commands::resolve_project_reference(
                state,
                params.reference,
            )?)
        }
        "get_ui_preferences" => ok(commands::get_ui_preferences(state)?),
        "save_ui_preferences" => {
            let params: PreferencesParams = params(request.params)?;
            ok(commands::save_ui_preferences(state, params.preferences)?)
        }
        "search_templates" => {
            let params: SearchTemplatesParams = params(request.params)?;
            ok(commands::search_templates(
                params.request,
                params.query,
                params.tags,
                params.page,
            )?)
        }
        "get_template_detail" => {
            let params: TemplateDetailParams = params(request.params)?;
            ok(commands::get_template_detail(params.request, params.id)?)
        }
        "install_template" => {
            let params: TemplateDetailParams = params(request.params)?;
            ok(commands::install_template(
                state,
                params.request,
                params.id,
            )?)
        }
        "get_backend_diagnostics" => ok(commands::get_backend_diagnostics(state)?),
        "backend_info" => Ok(json!({
            "project_root": commands::default_project_root(),
            "app_state_root": commands::default_app_state_root(),
        })),
        other => Err(format!("unsupported ipc method: {other}")),
    }
}

fn ok(data: impl Serialize) -> CommandResult<Value> {
    serde_json::to_value(data).map_err(|error| error.to_string())
}

fn params<T: DeserializeOwned>(value: Value) -> CommandResult<T> {
    let value = if value.is_null() { json!({}) } else { value };
    serde_json::from_value(value).map_err(|error| format!("invalid ipc params: {error}"))
}

#[derive(Debug, Deserialize)]
struct ProjectSelectionParams {
    project_root: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectRootParams {
    project_root: String,
}

#[derive(Debug, Deserialize, Default)]
struct DocumentTreeParams {
    #[serde(default)]
    project_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct DocumentContentParams {
    #[serde(default)]
    document_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SaveDocumentContentParams {
    document_id: String,
    content: String,
    #[serde(default)]
    base_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RequestParam<T> {
    request: T,
}

#[derive(Debug, Deserialize)]
struct SettingsParam<T> {
    settings: T,
}

#[derive(Debug, Deserialize, Default)]
struct LanguagePackParams {
    #[serde(default)]
    target_language: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct LanguagePackValidationParams {
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    overlay: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct UpdateParam<T> {
    update: T,
}

#[derive(Debug, Deserialize)]
struct ExportChaptersParams {
    #[serde(default)]
    selected_chapter_ids: Vec<String>,
    artifact_id: String,
    #[serde(default)]
    format: Option<crate::frontend::ChapterExportFormat>,
}

#[derive(Debug, Deserialize, Default)]
struct WorkflowIdParams {
    #[serde(default)]
    workflow_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ProviderModelsParams {
    #[serde(default)]
    provider_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkflowGraphParams {
    graph_data: commands::WorkflowGraphData,
}

#[derive(Debug, Deserialize)]
struct ApplyNodeDetailPatchParams {
    workflow_id: String,
    patch: crate::frontend::NodeDetailPatch,
}

#[derive(Debug, Deserialize)]
struct UpsertCanvasAnnotationParams {
    workflow_id: String,
    annotation: crate::frontend::CanvasAnnotation,
}

#[derive(Debug, Deserialize)]
struct SetNodeBreakpointParams {
    workflow_id: String,
    node_id: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct WorkflowSelectionParams {
    workflow_id: String,
    #[serde(default)]
    selected_node_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackWorkflowSelectionParams {
    workflow_id: String,
    #[serde(default)]
    selected_node_ids: Vec<String>,
    #[serde(default)]
    subworkflow_node_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RunWorkflowParams {
    workflow_id: String,
    #[serde(default)]
    start_node_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RunIdentityParams {
    workflow_id: String,
    run_id: String,
}

#[derive(Debug, Deserialize)]
struct WorkflowEventsParams {
    workflow_id: String,
    run_id: String,
    #[serde(default)]
    after_sequence: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RunControlParams {
    workflow_id: String,
    run_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateBudgetParams {
    budget_usd: f64,
    preauthorized_usd: f64,
}

#[derive(Debug, Deserialize)]
struct EnabledParams {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct ConfirmationIdParams {
    confirmation_id: String,
}

#[derive(Debug, Deserialize, Default)]
struct LimitParams {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct MessageParams {
    message: String,
}

#[derive(Debug, Deserialize)]
struct RestoreToNewBranchParams {
    commit_id: String,
    new_branch: String,
}

#[derive(Debug, Deserialize)]
struct SaveProviderKeyParams {
    provider: String,
    key: String,
}

#[derive(Debug, Deserialize, Default)]
struct RunLogFilterParams {
    #[serde(default)]
    filter: Option<commands::RunLogQuery>,
}

#[derive(Debug, Deserialize)]
struct ContentParams {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ApplyQuickEditParams {
    document_id: String,
    #[serde(default)]
    base_version: Option<String>,
    text: String,
    range: crate::contracts::TextRange,
    result: crate::frontend::QuickEditResult,
}

#[derive(Debug, Deserialize)]
struct ReferenceParams {
    reference: String,
}

#[derive(Debug, Deserialize)]
struct PreferencesParams {
    preferences: crate::frontend::UiPreferences,
}

#[derive(Debug, Deserialize)]
struct SearchTemplatesParams {
    request: commands::TemplateRepositoryRequest,
    query: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    page: u32,
}

#[derive(Debug, Deserialize)]
struct TemplateDetailParams {
    request: commands::TemplateRepositoryRequest,
    id: String,
}
