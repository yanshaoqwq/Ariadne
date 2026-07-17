use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::command_error::{CommandError, CommandErrorCode};
use crate::commands::{self, AriadneAppState, CommandResult};

#[derive(Debug, Deserialize)]
pub struct IpcRequest {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Deserialize)]
struct IpcEnvelopeRequest {
    #[serde(default)]
    request_id: Option<String>,
    #[serde(flatten)]
    request: IpcRequest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Free-form diagnostic for logs/tools. Not the author-facing primary string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Stable failure identity for UI localization (U1). Always set when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Localization key (`ui.error.*`). Desktop prefers this over inventing keys from diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_key: Option<String>,
}

impl IpcResponse {
    fn ok(data: impl Serialize) -> Self {
        Self {
            request_id: None,
            ok: true,
            data: Some(serde_json::to_value(data).unwrap_or(Value::Null)),
            error: None,
            error_code: None,
            error_key: None,
        }
    }

    fn error(error: CommandError) -> Self {
        Self {
            request_id: None,
            ok: false,
            data: None,
            error: error.diagnostic.clone(),
            error_code: Some(error.code.as_str().to_owned()),
            error_key: Some(error.message_key),
        }
    }

    fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }
}

pub fn handle_request(state: &AriadneAppState, request: IpcRequest) -> IpcResponse {
    match dispatch_request(
        state,
        request,
        &crate::contracts::ExecutionCancellation::new(),
    ) {
        Ok(value) => IpcResponse::ok(value),
        Err(error) => IpcResponse::error(error),
    }
}

pub fn run_json_line_stdio() -> io::Result<()> {
    let state = AriadneAppState::default_for_process();
    let stdin = io::stdin();
    run_json_line_session(stdin.lock(), Arc::new(Mutex::new(io::stdout())), state)
}

const MAX_CONCURRENT_IPC_REQUESTS: usize = 8;
const MAX_PENDING_IPC_REQUESTS: usize = 256;
const MAX_IPC_REQUEST_ID_BYTES: usize = 128;

struct IpcWorkItem {
    request_id: String,
    request: IpcRequest,
    cancellation: crate::contracts::ExecutionCancellation,
}

type IpcRequestHandler =
    dyn Fn(IpcRequest, &crate::contracts::ExecutionCancellation) -> IpcResponse + Send + Sync;

fn run_json_line_session<R, W>(
    reader: R,
    writer: Arc<Mutex<W>>,
    state: AriadneAppState,
) -> io::Result<()>
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let project_gate = Arc::new(RwLock::new(()));
    let handler = Arc::new(
        move |request: IpcRequest, cancellation: &crate::contracts::ExecutionCancellation| {
            let changes_project = matches!(
                request.method.as_str(),
                "create_project" | "open_project" | "set_project_root"
            );
            let result = if changes_project {
                project_gate
                    .write()
                    .map_err(|_| CommandError::internal("ipc project gate poisoned"))
                    .and_then(|_guard| dispatch_request(&state, request, cancellation))
            } else {
                project_gate
                    .read()
                    .map_err(|_| CommandError::internal("ipc project gate poisoned"))
                    .and_then(|_guard| dispatch_request(&state, request, cancellation))
            };
            match result {
                Ok(value) => IpcResponse::ok(value),
                Err(error) => IpcResponse::error(error),
            }
        },
    );
    run_json_line_session_with_handler(reader, writer, handler)
}

fn run_json_line_session_with_handler<R, W>(
    reader: R,
    writer: Arc<Mutex<W>>,
    handler: Arc<IpcRequestHandler>,
) -> io::Result<()>
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let worker_count = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .clamp(2, MAX_CONCURRENT_IPC_REQUESTS);
    let (work_sender, work_receiver) = mpsc::channel::<IpcWorkItem>();
    let work_receiver = Arc::new(Mutex::new(work_receiver));
    let active = Arc::new(Mutex::new(HashMap::<
        String,
        crate::contracts::ExecutionCancellation,
    >::new()));
    let write_error = Arc::new(Mutex::new(None::<io::Error>));
    let mut workers = Vec::with_capacity(worker_count);
    for index in 0..worker_count {
        let receiver = Arc::clone(&work_receiver);
        let active = Arc::clone(&active);
        let writer = Arc::clone(&writer);
        let handler = Arc::clone(&handler);
        let write_error = Arc::clone(&write_error);
        workers.push(
            thread::Builder::new()
                .name(format!("ariadne-ipc-worker-{index}"))
                .spawn(move || loop {
                    let item = match receiver.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => return,
                    };
                    let Ok(item) = item else { return };
                    let response = handler(item.request, &item.cancellation)
                        .with_request_id(Some(item.request_id.clone()));
                    if let Ok(mut active) = active.lock() {
                        active.remove(&item.request_id);
                    }
                    if let Err(error) = write_ipc_response(&writer, &response) {
                        record_ipc_write_error(&write_error, error);
                        return;
                    }
                })?,
        );
    }

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let envelope = match serde_json::from_str::<IpcEnvelopeRequest>(&line) {
            Ok(request) => request,
            Err(error) => {
                write_ipc_response(
                    &writer,
                    &IpcResponse::error(CommandError::validation(error.to_string())),
                )?;
                continue;
            }
        };
        if envelope.request.method == "cancel_request" {
            let response =
                cancel_ipc_request(&active, envelope.request_id, envelope.request.params);
            write_ipc_response(&writer, &response)?;
            continue;
        }
        let Some(request_id) = envelope.request_id else {
            let cancellation = crate::contracts::ExecutionCancellation::new();
            let response = handler(envelope.request, &cancellation);
            write_ipc_response(&writer, &response)?;
            continue;
        };
        if request_id.trim().is_empty() || request_id.len() > MAX_IPC_REQUEST_ID_BYTES {
            write_ipc_response(
                &writer,
                &IpcResponse::error(CommandError::validation(
                    "ipc request_id must contain 1 to 128 bytes",
                ))
                .with_request_id(Some(request_id)),
            )?;
            continue;
        }
        let cancellation = crate::contracts::ExecutionCancellation::new();
        let accepted = {
            let mut active = active
                .lock()
                .map_err(|_| io::Error::other("ipc active request lock poisoned"))?;
            if active.contains_key(&request_id) {
                false
            } else if active.len() >= MAX_PENDING_IPC_REQUESTS {
                drop(active);
                write_ipc_response(
                    &writer,
                    &IpcResponse::error(CommandError::new(
                        CommandErrorCode::ResourceLimit,
                        "ipc pending request limit exceeded",
                    ))
                    .with_request_id(Some(request_id.clone())),
                )?;
                continue;
            } else {
                active.insert(request_id.clone(), cancellation.clone());
                true
            }
        };
        if !accepted {
            write_ipc_response(
                &writer,
                &IpcResponse::error(CommandError::conflict("duplicate ipc request_id"))
                    .with_request_id(Some(request_id)),
            )?;
            continue;
        }
        if work_sender
            .send(IpcWorkItem {
                request_id,
                request: envelope.request,
                cancellation,
            })
            .is_err()
        {
            return Err(io::Error::other("ipc worker queue is unavailable"));
        }
    }

    drop(work_sender);
    for worker in workers {
        if worker.join().is_err() {
            return Err(io::Error::other("ipc worker panicked"));
        }
    }
    if let Some(error) = write_error
        .lock()
        .map_err(|_| io::Error::other("ipc write error lock poisoned"))?
        .take()
    {
        return Err(error);
    }
    Ok(())
}

fn cancel_ipc_request(
    active: &Arc<Mutex<HashMap<String, crate::contracts::ExecutionCancellation>>>,
    request_id: Option<String>,
    request_params: Value,
) -> IpcResponse {
    #[derive(Deserialize)]
    struct CancelParams {
        target_request_id: String,
    }
    let result = params::<CancelParams>(request_params).and_then(|params| {
        let active = active
            .lock()
            .map_err(|_| CommandError::internal("ipc active request lock poisoned"))?;
        let cancelled = active
            .get(params.target_request_id.trim())
            .map(|token| {
                token.cancel();
                true
            })
            .unwrap_or(false);
        Ok(json!({
            "target_request_id": params.target_request_id,
            "cancelled": cancelled,
        }))
    });
    match result {
        Ok(value) => IpcResponse::ok(value),
        Err(error) => IpcResponse::error(error),
    }
    .with_request_id(request_id)
}

fn write_ipc_response<W: Write>(writer: &Arc<Mutex<W>>, response: &IpcResponse) -> io::Result<()> {
    let body = serde_json::to_string(response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut writer = writer
        .lock()
        .map_err(|_| io::Error::other("ipc stdout lock poisoned"))?;
    writer.write_all(body.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn record_ipc_write_error(slot: &Arc<Mutex<Option<io::Error>>>, error: io::Error) {
    if let Ok(mut slot) = slot.lock() {
        if slot.is_none() {
            *slot = Some(error);
        }
    }
}

pub fn run_single_call(method: &str, params_json: Option<&str>) -> CommandResult<IpcResponse> {
    if method.trim().is_empty() {
        return Err(CommandError::validation("ipc method cannot be empty"));
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
        return Err(CommandError::validation("workflow_id cannot be empty"));
    }
    if run_id.trim().is_empty() {
        return Err(CommandError::validation("run_id cannot be empty"));
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
                if error.code == CommandErrorCode::NotFound && missing_run_wait_ms < 30_000 =>
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
                serde_json::to_string(&IpcResponse::ok(&result)).map_err(CommandError::from)?
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
    serde_json::from_str(params_json)
        .map_err(|error| CommandError::validation(format!("invalid ipc params JSON: {error}")))
}

fn workflow_status_is_terminal(status: &str) -> bool {
    matches!(status, "stopped" | "succeeded" | "failed")
}

fn dispatch_request(
    state: &AriadneAppState,
    request: IpcRequest,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<Value> {
    cancellation.check().map_err(CommandError::from)?;
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
        "get_chapter_summary_view" => {
            let params: ChapterIdParams = params(request.params)?;
            ok(commands::get_chapter_summary_view(
                state,
                params.chapter_id,
            )?)
        }
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
            let params: commands::WorkflowPackRequest = params(request.params)?;
            ok(commands::pack_workflow_selection_with_operation_id(
                state, params,
            )?)
        }
        "get_pack_operation" => {
            let params: PackOperationParams = params(request.params)?;
            ok(commands::get_pack_operation(state, params.operation_id)?)
        }
        "list_in_doubt_operations" => {
            let params: RunControlParams = params(request.params)?;
            ok(commands::list_in_doubt_operations(
                state,
                params.workflow_id,
                params.run_id,
            )?)
        }
        "get_project_maintenance" => ok(commands::get_project_maintenance(state)?),
        "list_index_dead_letters" => ok(commands::list_index_dead_letters(state)?),
        "requeue_index_dead_letter" => {
            let params: RequeueIndexDeadLetterParams = params(request.params)?;
            ok(commands::requeue_index_dead_letter(state, params.event_id)?)
        }
        "resolve_workflow_operation_in_doubt" => {
            let params: ResolveInDoubtParams = params(request.params)?;
            ok(commands::resolve_workflow_operation_in_doubt(
                state,
                commands::ResolveInDoubtOperationRequest {
                    operation_id: params.operation_id,
                    decision: params.decision,
                    response: params.response,
                    reason: params.reason,
                },
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
        "save_general_section_settings" => {
            let params: SettingsParam<commands::GeneralSectionSettings> = params(request.params)?;
            ok(commands::save_general_section_settings(
                state,
                params.settings,
            )?)
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
        "save_misc_section_settings" => {
            let params: SettingsParam<commands::MiscSectionSettings> = params(request.params)?;
            ok(commands::save_misc_section_settings(
                state,
                params.settings,
            )?)
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
        "save_automation_section_settings" => {
            let params: SettingsParam<commands::AutomationSectionSettings> =
                params(request.params)?;
            ok(commands::save_automation_section_settings(
                state,
                params.settings,
            )?)
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
            ok(commands::fetch_provider_models_with_cancellation(
                state,
                params.provider_id,
                cancellation,
            )?)
        }
        "save_provider_section_settings" => {
            let params: SettingsParam<commands::ProviderSectionSettings> = params(request.params)?;
            ok(commands::save_provider_section_settings(
                state,
                params.settings,
            )?)
        }
        "preview_provider_removal" => {
            let params: ProviderRemovalParams = params(request.params)?;
            ok(commands::preview_provider_removal(state, params.provider)?)
        }
        "remove_provider" => {
            let params: RemoveProviderParams = params(request.params)?;
            ok(commands::remove_provider(
                state,
                params.provider,
                params.expected_revision,
            )?)
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
        "get_git_history" => ok(commands::get_git_history_with_cancellation(
            state,
            cancellation,
        )?),
        "get_git_repository_status" => ok(commands::get_git_repository_status_with_cancellation(
            state,
            cancellation,
        )?),
        "get_git_branch_graph" => {
            let params: LimitParams = params(request.params)?;
            ok(commands::get_git_branch_graph_with_cancellation(
                state,
                params.limit,
                cancellation,
            )?)
        }
        "create_checkpoint" => {
            let params: MessageParams = params(request.params)?;
            ok(commands::create_checkpoint_with_cancellation(
                state,
                params.message,
                cancellation,
            )?)
        }
        "restore_to_new_branch" => {
            let params: RestoreToNewBranchParams = params(request.params)?;
            ok(commands::restore_to_new_branch_with_cancellation(
                state,
                params.commit_id,
                params.new_branch,
                cancellation,
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
        "rebind_project_provider_key" => {
            let params: RebindProviderKeyParams = params(request.params)?;
            ok(commands::rebind_project_provider_key(
                state,
                params.project_root,
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
        "mark_run_logs_read" => {
            let params: RunLogFilterParams = params(request.params)?;
            ok(commands::mark_run_logs_read(state, params.filter)?)
        }
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
            ok(commands::quick_edit_with_cancellation(
                state,
                params.request,
                cancellation,
            )?)
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
        "search_project_documents" => {
            let params: SearchProjectDocumentsParams = params(request.params)?;
            ok(commands::search_project_documents_with_cancellation(
                state,
                params.query,
                params.limit.unwrap_or(20),
                cancellation,
            )?)
        }
        "project_ai_chat" => {
            let params: RequestParam<commands::ProjectAiRequest> = params(request.params)?;
            ok(commands::project_ai_chat_with_cancellation(
                state,
                params.request,
                cancellation,
            )?)
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
            ok(commands::search_templates_with_cancellation(
                params.request,
                params.query,
                params.tags,
                params.page,
                cancellation,
            )?)
        }
        "get_template_detail" => {
            let params: TemplateDetailParams = params(request.params)?;
            ok(commands::get_template_detail_with_cancellation(
                params.request,
                params.id,
                cancellation,
            )?)
        }
        "install_template" => {
            let params: TemplateDetailParams = params(request.params)?;
            ok(commands::install_template_with_cancellation(
                state,
                params.request,
                params.id,
                cancellation,
            )?)
        }
        "get_backend_diagnostics" => ok(commands::get_backend_diagnostics(state)?),
        "backend_info" => Ok(json!({
            "project_root": commands::default_project_root(),
            "app_state_root": commands::default_app_state_root(),
            "product_version": crate::PRODUCT_VERSION,
            "ipc_schema_version": crate::IPC_SCHEMA_VERSION,
        })),
        other => Err(CommandError::not_found(format!(
            "unsupported ipc method: {other}"
        ))),
    }
}

fn ok(data: impl Serialize) -> CommandResult<Value> {
    serde_json::to_value(data).map_err(CommandError::from)
}

fn params<T: DeserializeOwned>(value: Value) -> CommandResult<T> {
    let value = if value.is_null() { json!({}) } else { value };
    serde_json::from_value(value)
        .map_err(|error| CommandError::validation(format!("invalid ipc params: {error}")))
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

#[derive(Debug, Deserialize)]
struct ChapterIdParams {
    chapter_id: String,
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
struct ProviderRemovalParams {
    provider: String,
}

#[derive(Debug, Deserialize)]
struct RemoveProviderParams {
    provider: String,
    expected_revision: String,
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
struct PackOperationParams {
    operation_id: String,
}

#[derive(Debug, Deserialize)]
struct ResolveInDoubtParams {
    operation_id: String,
    decision: commands::InDoubtDecision,
    #[serde(default)]
    response: Option<serde_json::Value>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RequeueIndexDeadLetterParams {
    event_id: String,
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

#[derive(Debug, Deserialize)]
struct RebindProviderKeyParams {
    project_root: String,
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
struct SearchProjectDocumentsParams {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    #[test]
    fn c9_stdio_routes_fast_response_before_slow_request() {
        let input = Cursor::new(
            b"{\"request_id\":\"slow\",\"method\":\"slow\"}\n{\"request_id\":\"fast\",\"method\":\"fast\"}\n",
        );
        let output = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(
            |request: IpcRequest, _: &crate::contracts::ExecutionCancellation| {
                if request.method == "slow" {
                    thread::sleep(Duration::from_millis(120));
                }
                IpcResponse::ok(json!({ "method": request.method }))
            },
        );

        run_json_line_session_with_handler(input, Arc::clone(&output), handler).unwrap();

        let body = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        let responses = body
            .lines()
            .map(|line| serde_json::from_str::<IpcResponse>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0].request_id.as_deref(), Some("fast"));
        assert_eq!(responses[1].request_id.as_deref(), Some("slow"));
    }

    #[test]
    fn c9_cancel_request_reaches_active_execution_token() {
        let input = Cursor::new(
            b"{\"request_id\":\"target\",\"method\":\"slow\"}\n{\"request_id\":\"cancel\",\"method\":\"cancel_request\",\"params\":{\"target_request_id\":\"target\"}}\n",
        );
        let output = Arc::new(Mutex::new(Vec::new()));
        let observed_cancellation = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&observed_cancellation);
        let handler = Arc::new(
            move |_: IpcRequest, cancellation: &crate::contracts::ExecutionCancellation| {
                let started = Instant::now();
                while !cancellation.is_cancelled() && started.elapsed() < Duration::from_secs(1) {
                    thread::sleep(Duration::from_millis(5));
                }
                if cancellation.is_cancelled() {
                    observed.store(true, Ordering::Release);
                    IpcResponse::error(CommandError::from(crate::contracts::CoreError::Cancelled))
                } else {
                    IpcResponse::ok(Value::Null)
                }
            },
        );

        run_json_line_session_with_handler(input, Arc::clone(&output), handler).unwrap();

        assert!(observed_cancellation.load(Ordering::Acquire));
        let body = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        let responses = body
            .lines()
            .map(|line| serde_json::from_str::<IpcResponse>(line).unwrap())
            .collect::<Vec<_>>();
        let cancel = responses
            .iter()
            .find(|response| response.request_id.as_deref() == Some("cancel"))
            .unwrap();
        let target = responses
            .iter()
            .find(|response| response.request_id.as_deref() == Some("target"))
            .unwrap();
        assert_eq!(cancel.data.as_ref().unwrap()["cancelled"], true);
        assert_eq!(target.error_code.as_deref(), Some("cancelled"));
    }
}
