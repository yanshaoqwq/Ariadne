use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::command_error::CommandError;

pub use crate::config::{
    ConfirmationAutoModePolicy, ConfirmationNormalPolicy, ConfirmationPolicySetting,
};

#[cfg(not(feature = "system-keychain"))]
use crate::config::LocalFileSecretStore;
#[cfg(feature = "system-keychain")]
use crate::config::SystemKeychainSecretStore;
use crate::config::{
    default_permission_tool_controls, policies_from_policy_code, policy_code_from_dual_policy,
    read_confirmation_policy_settings, AppConfig, AppPermissionsStore, AppRuntimeSettings,
    AppRuntimeSettingsStore, ApprovalPromptConfig, ConfigStore, GitConfig, ModelConfig,
    PermissionsConfig, ProjectConfig, ProjectCredentialScope, ProviderCatalogStore, ProviderConfig,
    RagConfig, SecretStore, SecretValue, WorkflowConfig,
};
use crate::contracts::{
    ensure_path_under_root, ApprovalPolicy, ArtifactKind, CoreError, CoreResult, Edge, EdgeId,
    NodeId, NodeInstance, PermissionPolicy, PermissionRequest, PortEndpoint, ProviderCapability,
    ProviderType, RunControl, RunId, WorkflowDefinition, WorkflowEdgeKind, WorkflowId,
};
use crate::costs::{budget_limits_from_global_budget, CostLedger, CostQuery, SqliteCostLedger};
use crate::diagnostics::{BackendDiagnosticsReport, DiagnosticItem, DiagnosticStatus};
use crate::documents::{
    ChapterDocumentIndex, DocumentContent, DocumentReadRequest, DocumentRepository,
    DocumentWriteReport, DocumentWriteRequest, FileDocumentService, IndexInvalidationOutbox,
    ProjectMaintenanceGuard, ProjectMutationGuard,
};
use crate::frontend::{
    apply_node_detail_patch as apply_node_detail_patch_to_workflow, build_works_tree,
    confirmation_state_from_runtime, export_chapters_combined,
    export_workflow_selection as export_workflow_selection_from_workflow,
    extract_project_reference_tokens, import_chapter_document,
    pack_workflow_selection as pack_workflow_selection_in_workflow, project_ai_context_window,
    project_ai_summary_context, project_document_permission, publish_initialized_project,
    structured_project_memory_context,
    upsert_canvas_annotation as upsert_canvas_annotation_in_workflow, ArtifactReferenceEntry,
    CanvasAnnotation, ChapterExportFormat, ChapterImportRequest, CombinedExportReport,
    ConfirmationLogEntry, FileConfirmationLogStore, NodeDetailPatch, ProjectAiAppendOutcome,
    ProjectAiContextWindow, ProjectAiConversationStore, ProjectAiMemoryEntry,
    ProjectAiStoredMessage, ProjectAiSummaryChunk, ProjectInitReport, ProjectMemoryStore,
    ProjectReference, ProjectReferenceResolver, ProjectRegistryStore, PublishedProjectCreation,
    QuickEditResult, QuickEditService, RecentProjectEntry, SidebarBadgeCounts, TemplateDetail,
    TemplateInstallReport, TemplateRepositoryClient, TemplateSummary, UiPreferences,
    UiPreferencesStore, UiRunLogEntry, UiRunLogFilter, UiRunLogKind, UiRunLogLevel, UiRunLogStore,
    WorksTreeNode, OFFICIAL_TEMPLATE_REPOSITORY_URL,
};
use crate::git::{
    ArchivePoint, BranchGraphNode, GitCommitSummary, GitHealthStatus, GitService, GitStagePolicy,
    RestoreReport,
};
use crate::llm::{
    tool_result_message, LlmRunRequest, LlmService, LlmServiceConfig, ToolExecutionContext,
    ToolExecutionOutput, ToolExecutor,
};
use crate::node_capabilities::{
    execution_tool_capability, model_tool_node_capabilities, workflow_node_capability,
    workflow_node_catalog, workflow_node_catalog_entry, WorkflowNodeExecutionKind,
    EXECUTOR_ADAPTER_TOOL_CAPABILITY, PROJECT_AI_TOOL_CAPABILITY,
};
use crate::providers::{
    web_search_tool_definition, ContentPart, HttpWebSearchProvider, LlmMessage, LlmRole,
    OpenAiCompatibleLlmProvider, Provider, ProviderProtocol, SearchProvider, ToolDefinition,
    WebSearchToolExecutor, EXECUTOR_ADAPTER_WEB_SEARCH_TOOL, PROJECT_AI_WEB_SEARCH_TOOL,
};
use crate::rag::SqliteWritingKnowledgeStore;
use crate::retrieval::{
    project_search_tool_definition, ProjectRetrievalRuntime, ProjectSearchToolExecutor,
    RetrievalResult, EXECUTOR_ADAPTER_SEARCH_TOOL, PROJECT_AI_SEARCH_TOOL,
};
use crate::skills::{
    ExecutorAdapterExecutionPlan, LoadedSkillManifest, SkillLoader, WorkflowManifest,
    WORKFLOW_MANIFEST_FILE,
};
use crate::workflow::{
    execute_document_read_node_with_root, execute_llm_node_with_defaults,
    execute_llm_node_with_search_tools, execute_project_retrieval_node_for_project,
    execute_summarizer_node, execute_summarizer_node_with_search_tools,
    merge_workflow_into_project_canvas, normalize_project_canvas_identity,
    validate_workflow_execution_contracts, BuiltinWorkflowNodeExecutor, DocumentWorkflowExportSink,
    RoutedExternalNodeExecutor, RuntimeConfirmation, RuntimeConfirmationState,
    SqliteWorkflowRuntimeStore, WorkflowExecutionDependencySet, WorkflowLlmNodeConfig,
    WorkflowLlmSearchOptions, WorkflowRunFailure, WorkflowRunnableClaimResult, WorkflowRuntime,
    WorkflowRuntimeEvent, WorkflowRuntimeEventType, WorkflowRuntimeStore,
    WorkflowStopRequestResult, WorkflowWorkerLease, EXECUTOR_ADAPTER_NODE_PREFIX,
    PROJECT_CANVAS_WORKFLOW_ID,
};

const WORKFLOW_WORKER_LEASE_TTL_MS: u64 = 30_000;
const WORKFLOW_WORKER_HEARTBEAT_INTERVAL_MS: u64 = 5_000;
const WORKFLOW_WORKER_LEASE_LOST_ERROR: &str = "workflow worker lease was lost during execution";
const WORKFLOW_SCHEDULER_LEASE_TTL_MS: u64 = 3_000;
const WORKFLOW_SCHEDULER_MAX_SLEEP_MS: u64 = 1_000;
const WORKFLOW_SCHEDULER_MAX_CLAIMS_PER_TICK: usize = 64;

pub const WORKFLOW_STATUS_UPDATE_EVENT: &str = "workflow_status_update";
pub const RUN_LOG_APPENDED_EVENT: &str = "run_log_appended";
pub const BUDGET_UPDATED_EVENT: &str = "budget_updated";
pub const CONFIRMATION_CREATED_EVENT: &str = "confirmation_created";
pub const DIAGNOSTICS_UPDATED_EVENT: &str = "diagnostics_updated";
pub const TOAST_CREATED_EVENT: &str = "toast_created";

const DEFAULT_PROJECT_ENV: &str = "ARIADNE_PROJECT_ROOT";
const RECENT_PROJECTS_FILE: &str = "recent_projects.json";
const BUDGET_CONFIG_FILE: &str = "budget.json";
const CHAPTER_INDEX_FILE: &str = "chapter_index.json";
const UI_NODE_PRESETS_FILE: &str = "ui_node_presets.json";
const APP_NODE_DEFAULTS_FILE: &str = "node_authoring_defaults.json";
const APP_NODE_DEFAULTS_LOCK_FILE: &str = ".node-authoring-defaults.lock";
const TEMPLATE_REPOSITORY_SETTINGS_FILE: &str = "template_repository_settings.json";
const PROVIDER_REFERENCE_GRAPH_LOCK: &str = ".provider-reference-graph";
const DEFAULT_TEMPLATE_REPOSITORY_URL: &str = OFFICIAL_TEMPLATE_REPOSITORY_URL;
const PROVIDER_MODEL_FETCH_TIMEOUT_SECS: u64 = 30;
const IPC_GIT_TIMEOUT_SECS: u64 = 120;
const MAX_PROVIDER_MODEL_LIST_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

struct WorkflowSchedulerHandle {
    project_root: PathBuf,
    stop_sender: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
    start_gate: Option<Arc<WorkflowSchedulerStartGate>>,
}

struct WorkflowSchedulerStartGate {
    state: Mutex<WorkflowSchedulerStartState>,
    wake: Condvar,
}

#[derive(Default)]
struct WorkflowSchedulerStartState {
    started: bool,
    cancelled: bool,
}

impl WorkflowSchedulerStartGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(WorkflowSchedulerStartState::default()),
            wake: Condvar::new(),
        })
    }

    fn wait(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.started && !state.cancelled {
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.started && !state.cancelled
    }

    fn start(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.started = true;
        self.wake.notify_all();
    }

    fn cancel(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cancelled = true;
        self.wake.notify_all();
    }
}

impl Drop for WorkflowSchedulerHandle {
    fn drop(&mut self) {
        if let Some(start_gate) = &self.start_gate {
            start_gate.cancel();
        }
        let _ = self.stop_sender.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 桌面前端共享状态。project_root 只能由显式环境变量或项目生命周期命令设置。
#[derive(Clone)]
pub struct AriadneAppState {
    project_root: Arc<Mutex<PathBuf>>,
    app_state_root: PathBuf,
    secret_store: Arc<dyn SecretStore>,
    retrieval_runtime: Arc<Mutex<Option<Arc<ProjectRetrievalRuntime>>>>,
    workflow_scheduler: Arc<Mutex<Option<WorkflowSchedulerHandle>>>,
    project_activation: Arc<Mutex<()>>,
    global_settings: Arc<Mutex<()>>,
}

impl AriadneAppState {
    pub fn new(
        project_root: impl Into<PathBuf>,
        app_state_root: impl Into<PathBuf>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            project_root: Arc::new(Mutex::new(project_root.into())),
            app_state_root: app_state_root.into(),
            secret_store,
            retrieval_runtime: Arc::new(Mutex::new(None)),
            workflow_scheduler: Arc::new(Mutex::new(None)),
            project_activation: Arc::new(Mutex::new(())),
            global_settings: Arc::new(Mutex::new(())),
        }
    }

    pub fn default_for_process() -> Self {
        Self::new(
            default_project_root(),
            default_app_state_root(),
            default_secret_store(),
        )
    }

    pub fn project_root(&self) -> CommandResult<PathBuf> {
        self.project_root
            .lock()
            .map(|root| root.clone())
            .map_err(|_| CommandError::internal("project root lock poisoned"))
    }

    pub fn app_state_root(&self) -> &Path {
        &self.app_state_root
    }

    fn lock_project_activation(&self) -> CommandResult<std::sync::MutexGuard<'_, ()>> {
        self.project_activation
            .lock()
            .map_err(|_| CommandError::internal("project activation lock poisoned"))
    }

    fn lock_global_settings(&self) -> CommandResult<std::sync::MutexGuard<'_, ()>> {
        self.global_settings
            .lock()
            .map_err(|_| CommandError::internal("global settings lock poisoned"))
    }

    fn prepare_workflow_scheduler(
        &self,
        project_root: &Path,
    ) -> CommandResult<WorkflowSchedulerHandle> {
        let project_root = project_root.to_path_buf();
        let retrieval_runtime = Arc::clone(&self.retrieval_runtime);
        let (stop_sender, stop_receiver) = std::sync::mpsc::channel::<()>();
        let start_gate = WorkflowSchedulerStartGate::new();
        let thread_gate = Arc::clone(&start_gate);
        let scheduler_root = project_root.clone();
        let secrets = Arc::clone(&self.secret_store);
        let thread = std::thread::Builder::new()
            .name("ariadne-workflow-scheduler".to_owned())
            .spawn(move || {
                if thread_gate.wait() {
                    workflow_scheduler_loop(
                        scheduler_root,
                        secrets,
                        retrieval_runtime,
                        stop_receiver,
                    );
                }
            })
            .map_err(error_to_string)?;
        Ok(WorkflowSchedulerHandle {
            project_root,
            stop_sender,
            thread: Some(thread),
            start_gate: Some(start_gate),
        })
    }

    fn ensure_workflow_scheduler(&self) -> CommandResult<()> {
        let project_root = canonicalize_initialized_project_root(&self.project_root()?)?;
        let mut slot = self
            .workflow_scheduler
            .lock()
            .map_err(|_| CommandError::internal("workflow scheduler lock poisoned"))?;
        if slot.as_ref().is_some_and(|scheduler| {
            scheduler.project_root == project_root
                && scheduler
                    .thread
                    .as_ref()
                    .is_some_and(|thread| !thread.is_finished())
        }) {
            return Ok(());
        }
        let scheduler = self.prepare_workflow_scheduler(&project_root)?;
        let start_gate = scheduler.start_gate.as_ref().map(Arc::clone);
        let previous = slot.replace(scheduler);
        drop(slot);
        drop(previous);
        if let Some(start_gate) = start_gate {
            start_gate.start();
        }
        Ok(())
    }

    pub fn set_project_root(&self, project_root: impl Into<PathBuf>) -> CommandResult<()> {
        let project_root = project_root.into();
        let project_root = canonicalize_initialized_project_root(&project_root)?;
        crate::config::bind_project_app_state(&project_root, &self.app_state_root)
            .map_err(error_to_string)?;
        let existing_runtime = self
            .retrieval_runtime
            .lock()
            .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?
            .as_ref()
            .filter(|runtime| runtime.project_root() == project_root)
            .cloned();
        let runtime = match existing_runtime {
            Some(runtime) => runtime,
            None => Arc::new(
                ProjectRetrievalRuntime::open(&project_root, self.secret_store.as_ref())
                    .map_err(error_to_string)?,
            ),
        };
        validate_project_activation_recovery(&project_root)?;
        let scheduler = self.prepare_workflow_scheduler(&project_root)?;
        self.commit_project_activation(project_root.clone(), runtime, scheduler)?;
        complete_committed_project_activation(self, &project_root);
        self.start_committed_workflow_scheduler(&project_root)
    }

    /// 在同一临界区替换项目 identity 与检索组合根；调用前候选 runtime 必须已就绪。
    fn commit_project_activation(
        &self,
        project_root: PathBuf,
        runtime: Arc<ProjectRetrievalRuntime>,
        scheduler: WorkflowSchedulerHandle,
    ) -> CommandResult<()> {
        if runtime.project_root() != project_root {
            return Err(CommandError::conflict(
                "candidate retrieval runtime belongs to a different project",
            ));
        }
        if scheduler.project_root != project_root {
            return Err(CommandError::conflict(
                "candidate workflow scheduler belongs to a different project",
            ));
        }
        let mut runtime_slot = self
            .retrieval_runtime
            .lock()
            .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?;
        let mut root_slot = self
            .project_root
            .lock()
            .map_err(|_| CommandError::internal("project root lock poisoned"))?;
        let mut scheduler_slot = self
            .workflow_scheduler
            .lock()
            .map_err(|_| CommandError::internal("workflow scheduler lock poisoned"))?;
        *root_slot = project_root;
        let previous = runtime_slot.replace(runtime);
        let previous_scheduler = scheduler_slot.replace(scheduler);
        drop(root_slot);
        drop(runtime_slot);
        drop(scheduler_slot);
        drop(previous_scheduler);
        drop(previous);
        Ok(())
    }

    fn start_committed_workflow_scheduler(&self, project_root: &Path) -> CommandResult<()> {
        let project_root = canonicalize_initialized_project_root(project_root)?;
        let slot = self
            .workflow_scheduler
            .lock()
            .map_err(|_| CommandError::internal("workflow scheduler lock poisoned"))?;
        let scheduler = slot.as_ref().ok_or_else(|| {
            CommandError::internal("committed project is missing its workflow scheduler")
        })?;
        if scheduler.project_root != project_root {
            return Err(CommandError::conflict(
                "committed workflow scheduler belongs to a different project",
            ));
        }
        if let Some(start_gate) = scheduler.start_gate.as_ref() {
            start_gate.start();
        }
        Ok(())
    }

    /// 清空当前项目及其项目级运行时；桌面“离开项目”必须同步清理后端状态。
    pub fn clear_project_root(&self) -> CommandResult<()> {
        let mut runtime_slot = self
            .retrieval_runtime
            .lock()
            .map_err(|_| CommandError::internal("project retrieval runtime lock poisoned"))?;
        let mut root_slot = self
            .project_root
            .lock()
            .map_err(|_| CommandError::internal("project root lock poisoned"))?;
        let mut scheduler_slot = self
            .workflow_scheduler
            .lock()
            .map_err(|_| CommandError::internal("workflow scheduler lock poisoned"))?;
        let runtime = runtime_slot.take();
        let scheduler = scheduler_slot.take();
        root_slot.clear();
        drop(scheduler_slot);
        drop(root_slot);
        drop(runtime_slot);
        drop(scheduler);
        drop(runtime);
        Ok(())
    }

    /// 返回当前项目唯一的检索组合根；测试直接构造 state 时按需初始化。
    pub fn retrieval_runtime(&self) -> CommandResult<Arc<ProjectRetrievalRuntime>> {
        let project_root = self.project_root()?;
        let canonical_root = canonicalize_initialized_project_root(&project_root)?;
        crate::config::bind_project_app_state(&canonical_root, &self.app_state_root)
            .map_err(error_to_string)?;
        let mut runtime_slot = self
            .retrieval_runtime
            .lock()
            .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?;
        if let Some(runtime) = runtime_slot
            .as_ref()
            .filter(|runtime| runtime.project_root() == canonical_root)
        {
            return Ok(Arc::clone(runtime));
        }
        let runtime = Arc::new(
            ProjectRetrievalRuntime::open(&canonical_root, self.secret_store.as_ref())
                .map_err(error_to_string)?,
        );
        let previous = runtime_slot.replace(Arc::clone(&runtime));
        drop(runtime_slot);
        drop(previous);
        Ok(runtime)
    }

    /// 配置或凭据变更后原子替换组合根，旧 sidecar 在新运行时就绪后再关闭。
    pub fn reload_retrieval_runtime(&self) -> CommandResult<Arc<ProjectRetrievalRuntime>> {
        let project_root = self.project_root()?;
        self.reload_retrieval_runtime_for_project(&project_root)
    }

    /// 只为调用方固定的项目替换组合根；项目身份漂移时 fail-loud。
    fn reload_retrieval_runtime_for_project(
        &self,
        expected_project_root: &Path,
    ) -> CommandResult<Arc<ProjectRetrievalRuntime>> {
        let project_root = canonicalize_initialized_project_root(expected_project_root)?;
        let current_project_root = canonicalize_initialized_project_root(&self.project_root()?)?;
        if current_project_root != project_root {
            return Err(CommandError::conflict(format!(
                "project activation changed during retrieval reload: expected {}, current {}",
                project_root.display(),
                current_project_root.display()
            )));
        }
        crate::config::bind_project_app_state(&project_root, &self.app_state_root)
            .map_err(error_to_string)?;
        let config = ConfigStore::new(&project_root)
            .load_or_create()
            .map_err(error_to_string)?;
        let config = effective_retrieval_config(&project_root, config)?;
        let mut runtime_slot = self
            .retrieval_runtime
            .lock()
            .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?;
        let index_changed = runtime_slot.as_ref().is_some_and(|runtime| {
            ProjectRetrievalRuntime::index_configuration_changed(runtime.config(), &config)
        });
        let vector_store_requires_exclusive_reopen = runtime_slot.as_ref().is_some_and(|runtime| {
            runtime.vector_enabled()
                && config.rag.vector_store.enabled
                && !runtime.uses_vector_config(&config.rag.vector_store)
        });
        if index_changed
            && runtime_slot
                .as_ref()
                .is_some_and(|runtime| Arc::strong_count(runtime) > 1)
        {
            return Err(CommandError::conflict(
                "cannot reload changed retrieval index configuration while retrieval operations are active",
            ));
        }
        let previous_config = runtime_slot
            .as_ref()
            .map(|runtime| runtime.config().clone());
        let detached = vector_store_requires_exclusive_reopen
            .then(|| runtime_slot.take())
            .flatten();
        drop(detached);
        let runtime = match ProjectRetrievalRuntime::from_config(
            &project_root,
            self.secret_store.as_ref(),
            &config,
            runtime_slot.as_deref(),
        ) {
            Ok(runtime) => Arc::new(runtime),
            Err(error) => {
                if let Some(previous_config) = previous_config {
                    if let Ok(restored) = ProjectRetrievalRuntime::from_config(
                        &project_root,
                        self.secret_store.as_ref(),
                        &previous_config,
                        None,
                    ) {
                        *runtime_slot = Some(Arc::new(restored));
                    }
                }
                return Err(error_to_string(error));
            }
        };
        if index_changed {
            if let Err(error) = runtime.enqueue_configuration_rebuild() {
                drop(runtime);
                if let Some(previous_config) = previous_config {
                    restore_retrieval_runtime(
                        &mut runtime_slot,
                        &project_root,
                        self.secret_store.as_ref(),
                        &previous_config,
                    )?;
                }
                return Err(error_to_string(error));
            }
        }
        let previous = runtime_slot.replace(Arc::clone(&runtime));
        drop(previous);
        Ok(runtime)
    }

    /// 两阶段提交检索相关配置：候选 generation 就绪后才落盘并切换。
    fn commit_retrieval_config(
        &self,
        expected: &ProjectConfig,
        candidate: ProjectConfig,
    ) -> CommandResult<Arc<ProjectRetrievalRuntime>> {
        candidate.validate().map_err(error_to_string)?;
        let project_root = canonicalize_initialized_project_root(&self.project_root()?)?;
        crate::config::bind_project_app_state(&project_root, &self.app_state_root)
            .map_err(error_to_string)?;
        let config_store = ConfigStore::new(&project_root);
        let mut runtime_slot = self
            .retrieval_runtime
            .lock()
            .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?;
        let current = config_store.load_or_create().map_err(error_to_string)?;
        if &current != expected {
            return Err(CommandError::conflict(
                "project configuration changed concurrently; reload and retry",
            ));
        }

        let expected_runtime = effective_retrieval_config(&project_root, expected.clone())?;
        let candidate_runtime = effective_retrieval_config(&project_root, candidate.clone())?;

        let index_changed = ProjectRetrievalRuntime::index_configuration_changed(
            &expected_runtime,
            &candidate_runtime,
        );
        let vector_store_requires_exclusive_reopen = runtime_slot.as_ref().is_some_and(|runtime| {
            runtime.vector_enabled()
                && candidate_runtime.rag.vector_store.enabled
                && !runtime.uses_vector_config(&candidate_runtime.rag.vector_store)
        });
        if index_changed
            && runtime_slot
                .as_ref()
                .is_some_and(|runtime| Arc::strong_count(runtime) > 1)
        {
            return Err(CommandError::conflict(
                "cannot change retrieval index configuration while retrieval operations are active",
            ));
        }

        let detached = vector_store_requires_exclusive_reopen
            .then(|| runtime_slot.take())
            .flatten();
        drop(detached);
        let runtime = match ProjectRetrievalRuntime::from_config(
            &project_root,
            self.secret_store.as_ref(),
            &candidate_runtime,
            runtime_slot.as_deref(),
        ) {
            Ok(runtime) => Arc::new(runtime),
            Err(error) => {
                restore_retrieval_runtime(
                    &mut runtime_slot,
                    &project_root,
                    self.secret_store.as_ref(),
                    &expected_runtime,
                )?;
                return Err(error_to_string(error));
            }
        };

        if let Err(error) = config_store.save(&candidate) {
            drop(runtime);
            let rollback_error = config_store.save(expected).err();
            restore_retrieval_runtime(
                &mut runtime_slot,
                &project_root,
                self.secret_store.as_ref(),
                &expected_runtime,
            )?;
            return Err(match rollback_error {
                Some(rollback_error) => CommandError::io(format!(
                    "failed to commit retrieval config: {error}; rollback also failed: {rollback_error}"
                )),
                None => error_to_string(error),
            });
        }

        if index_changed {
            if let Err(error) = runtime.enqueue_configuration_rebuild() {
                drop(runtime);
                let rollback_error = config_store.save(expected).err();
                restore_retrieval_runtime(
                    &mut runtime_slot,
                    &project_root,
                    self.secret_store.as_ref(),
                    &expected_runtime,
                )?;
                return Err(match rollback_error {
                    Some(rollback_error) => CommandError::io(format!(
                        "failed to enqueue retrieval rebuild: {error}; config rollback also failed: {rollback_error}"
                    )),
                    None => error_to_string(error),
                });
            }
        }

        let previous = runtime_slot.replace(Arc::clone(&runtime));
        drop(previous);
        Ok(runtime)
    }
}

fn effective_retrieval_config(
    project_root: &Path,
    mut config: ProjectConfig,
) -> CommandResult<ProjectConfig> {
    let app_state_root = crate::config::trusted_app_state_for_project(project_root);
    let settings =
        AppRuntimeSettingsStore::read_global_or_migrate(app_state_root, Some(project_root))
            .map_err(error_to_string)?;
    settings.apply_to_sidecar(&mut config.rag.vector_store.sidecar);
    config.validate().map_err(error_to_string)?;
    Ok(config)
}

fn restore_retrieval_runtime(
    slot: &mut Option<Arc<ProjectRetrievalRuntime>>,
    project_root: &Path,
    secrets: &dyn SecretStore,
    config: &ProjectConfig,
) -> CommandResult<()> {
    if slot.is_some() {
        return Ok(());
    }
    *slot = Some(Arc::new(
        ProjectRetrievalRuntime::from_config(project_root, secrets, config, None)
            .map_err(error_to_string)?,
    ));
    Ok(())
}

pub type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentTreeNode {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub kind: DocumentTreeNodeKind,
    #[serde(default)]
    pub children: Vec<DocumentTreeNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentTreeNodeKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowGraphData {
    pub workflow_id: String,
    pub name: String,
    #[serde(default)]
    pub nodes: Vec<CanvasNode>,
    #[serde(default)]
    pub edges: Vec<CanvasEdge>,
    #[serde(default)]
    pub metadata: Value,
    /// Content hash of the last durable workflow file (server-set on load/save).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_revision: Option<String>,
    /// Client-supplied CAS token from a prior load; required when overwriting an existing file (N3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<String>,
}

/// IPC/桌面专用的子工作流打包结果；领域工作流在边界处统一转换为画布 DTO。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowPackGraphReport {
    pub workflow: WorkflowGraphData,
    pub subworkflow_node_id: String,
    pub embedded_workflow: WorkflowGraphData,
    #[serde(default)]
    pub boundary_inputs: Vec<PortEndpoint>,
    #[serde(default)]
    pub boundary_outputs: Vec<PortEndpoint>,
    /// Stable id so clients can re-fetch the pack result if IPC response is lost (N8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

/// 子工作流打包的统一命令请求，避免恢复/CAS 参数在多层入口间按位置漂移。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowPackRequest {
    pub workflow_id: String,
    #[serde(default)]
    pub selected_node_ids: Vec<String>,
    #[serde(default)]
    pub subworkflow_node_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkflowPackOperationStatus {
    Prepared,
    Committed,
}

/// N8：打包先持久化可恢复意图，再替换工作流，避免“文件已写、回执未写”的盲区。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WorkflowPackOperationRecord {
    operation_id: String,
    request_hash: String,
    expected_revision: String,
    status: WorkflowPackOperationStatus,
    report: WorkflowPackGraphReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowSummary {
    pub workflow_id: String,
    pub name: String,
    pub path: String,
    pub node_count: usize,
    pub edge_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasNode {
    pub id: String,
    #[serde(default = "default_node_type")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub position: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default = "default_source_handle")]
    pub source_handle: String,
    #[serde(default = "default_target_handle")]
    pub target_handle: String,
    #[serde(default)]
    pub kind: WorkflowEdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunWorkflowRequest {
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub initial_inputs: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunStarted {
    pub run_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetStatus {
    pub budget_usd: f64,
    pub spent_usd: f64,
    pub preauthorized_usd: f64,
    pub auto_mode_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationSettings {
    pub budget: BudgetStatus,
    #[serde(default)]
    pub confirmation_policies: Vec<ConfirmationPolicySetting>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationSectionSettings {
    pub automation: AutomationSettings,
    pub workflow: WorkflowSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneralSectionSettings {
    pub app: AppSettings,
    pub project_memory: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MiscSectionSettings {
    pub rag: RagSettings,
    pub git: GitSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionsSettings {
    pub policy: PermissionPolicy,
    #[serde(default)]
    pub scoped_policies: BTreeMap<String, Option<PermissionPolicy>>,
    #[serde(default)]
    pub tool_controls: BTreeMap<String, BTreeMap<String, Option<bool>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePresetSettings {
    #[serde(default)]
    pub presets: Vec<NodeTypePreset>,
    /// 空值只用于兼容旧配置，表示沿用项目默认 LLM Provider。
    #[serde(default)]
    pub default_provider_id: String,
    #[serde(default = "default_node_preset_model_id")]
    pub default_model_id: String,
    #[serde(default = "default_node_preset_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default)]
    pub default_budget_usd: f64,
}

impl Default for NodePresetSettings {
    fn default() -> Self {
        Self {
            presets: default_node_type_presets(),
            default_provider_id: String::new(),
            default_model_id: default_node_preset_model_id(),
            default_timeout_ms: default_node_preset_timeout_ms(),
            default_budget_usd: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeTypePreset {
    pub node_type: String,
    pub display_name_key: String,
    /// Provider 与 model_id 共同构成模型身份；空值兼容旧版默认 Provider 语义。
    #[serde(default)]
    pub provider_id: String,
    pub model_id: String,
    pub timeout_ms: u64,
    pub budget_usd: f64,
    /// None 表示继承工作流节点权限默认值。
    #[serde(default)]
    pub permission_policy: Option<PermissionPolicy>,
    /// 工具动作覆盖；None 表示继承权限页的节点类型/全局工具设置。
    #[serde(default)]
    pub tool_controls: BTreeMap<String, Option<bool>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AppNodeDefaults {
    #[serde(default)]
    presets: Vec<AppNodeTypeDefault>,
    #[serde(default)]
    default_provider_id: String,
    #[serde(default = "default_node_preset_model_id")]
    default_model_id: String,
    #[serde(default = "default_node_preset_timeout_ms")]
    default_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AppNodeTypeDefault {
    node_type: String,
    display_name_key: String,
    #[serde(default)]
    provider_id: String,
    model_id: String,
    timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ProjectNodePresetOverrides {
    #[serde(default)]
    presets: Vec<ProjectNodeTypeOverride>,
    #[serde(default)]
    default_budget_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ProjectNodeTypeOverride {
    node_type: String,
    #[serde(default)]
    budget_usd: f64,
    #[serde(default)]
    permission_policy: Option<PermissionPolicy>,
    #[serde(default)]
    tool_controls: BTreeMap<String, Option<bool>>,
}

impl AppNodeDefaults {
    fn from_settings(settings: &NodePresetSettings) -> Self {
        Self {
            presets: settings
                .presets
                .iter()
                .map(|preset| AppNodeTypeDefault {
                    node_type: preset.node_type.clone(),
                    display_name_key: preset.display_name_key.clone(),
                    provider_id: preset.provider_id.clone(),
                    model_id: preset.model_id.clone(),
                    timeout_ms: preset.timeout_ms,
                })
                .collect(),
            default_provider_id: settings.default_provider_id.clone(),
            default_model_id: settings.default_model_id.clone(),
            default_timeout_ms: settings.default_timeout_ms,
        }
    }

    fn validate(&self) -> CommandResult<()> {
        if self.default_model_id.trim().is_empty() {
            return Err(CommandError::validation(
                "default node model id cannot be empty",
            ));
        }
        if self.default_timeout_ms == 0 {
            return Err(CommandError::validation(
                "default node timeout must be positive",
            ));
        }
        let mut node_types = HashSet::new();
        for preset in &self.presets {
            if preset.node_type.trim().is_empty()
                || preset.display_name_key.trim().is_empty()
                || preset.model_id.trim().is_empty()
                || preset.timeout_ms == 0
            {
                return Err(CommandError::validation(
                    "global node defaults contain an incomplete preset",
                ));
            }
            if !node_types.insert(preset.node_type.as_str()) {
                return Err(CommandError::validation(format!(
                    "duplicate global node default: {}",
                    preset.node_type
                )));
            }
        }
        Ok(())
    }
}

impl ProjectNodePresetOverrides {
    fn from_settings(settings: &NodePresetSettings) -> Self {
        Self {
            presets: settings
                .presets
                .iter()
                .map(|preset| ProjectNodeTypeOverride {
                    node_type: preset.node_type.clone(),
                    budget_usd: preset.budget_usd,
                    permission_policy: preset.permission_policy.clone(),
                    tool_controls: preset.tool_controls.clone(),
                })
                .collect(),
            default_budget_usd: settings.default_budget_usd,
        }
    }

    fn merge(self, app: AppNodeDefaults) -> NodePresetSettings {
        let project = self
            .presets
            .into_iter()
            .map(|preset| (preset.node_type.clone(), preset))
            .collect::<BTreeMap<_, _>>();
        NodePresetSettings {
            presets: app
                .presets
                .into_iter()
                .map(|preset| {
                    let project = project.get(&preset.node_type);
                    NodeTypePreset {
                        node_type: preset.node_type,
                        display_name_key: preset.display_name_key,
                        provider_id: preset.provider_id,
                        model_id: preset.model_id,
                        timeout_ms: preset.timeout_ms,
                        budget_usd: project.map_or(0.0, |item| item.budget_usd),
                        permission_policy: project.and_then(|item| item.permission_policy.clone()),
                        tool_controls: project
                            .map(|item| item.tool_controls.clone())
                            .unwrap_or_default(),
                    }
                })
                .collect(),
            default_provider_id: app.default_provider_id,
            default_model_id: app.default_model_id,
            default_timeout_ms: app.default_timeout_ms,
            default_budget_usd: self.default_budget_usd,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetConfigFile {
    pub budget_usd: f64,
}

impl Default for BudgetConfigFile {
    fn default() -> Self {
        Self { budget_usd: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub app: AppConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagSettings {
    pub rag: RagConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSettings {
    pub workflow: WorkflowConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSettings {
    pub git: GitConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRepositoryStatus {
    pub status: GitHealthStatus,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub dirty: bool,
    pub reason: Option<String>,
    pub diff_line_count: usize,
    pub diff_preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRepositorySettings {
    pub base_url: String,
}

impl Default for TemplateRepositorySettings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_TEMPLATE_REPOSITORY_URL.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayNameLanguagePackTemplate {
    pub target_language: String,
    pub base_language: String,
    pub fallback_language: String,
    pub output_file_name: String,
    pub source_file_name: String,
    pub instructions: Vec<String>,
    pub entries: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayNameLanguagePackValidation {
    pub target_language: String,
    pub output_file_name: String,
    pub total_keys: usize,
    pub translated_keys: usize,
    pub missing_keys: Vec<String>,
    pub empty_keys: Vec<String>,
    pub extra_keys: Vec<String>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfigStatus {
    pub has_openai_key: bool,
    pub has_anthropic_key: bool,
    pub has_gemini_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_llm_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_embedding_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reranker_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_search_provider_id: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderKeyStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderKeyStatus {
    pub provider: String,
    pub display_name: String,
    pub provider_type: ProviderType,
    pub configured: bool,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    pub has_key: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSettingsUpdate {
    pub provider_id: String,
    pub provider_type: ProviderType,
    pub display_name: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    #[serde(default)]
    pub make_default_llm: bool,
    #[serde(default)]
    pub make_default_embedding: bool,
    #[serde(default)]
    pub make_default_reranker: bool,
    #[serde(default)]
    pub make_default_search: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSectionSettings {
    pub provider: ProviderSettingsUpdate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRemovalReference {
    pub reference_type: String,
    pub owner_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRemovalPreview {
    pub provider_id: String,
    pub display_name: String,
    pub revision: String,
    pub has_key: bool,
    #[serde(default)]
    pub default_roles: Vec<String>,
    #[serde(default)]
    pub blocking_references: Vec<ProviderRemovalReference>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderModelsResult {
    pub provider_id: String,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentProjectStatus {
    pub project_root: PathBuf,
    pub project_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppStatus {
    pub current_project: CurrentProjectStatus,
    pub badges: SidebarBadgeCounts,
    pub preferences: UiPreferences,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLogQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<UiRunLogKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<UiRunLogLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_timestamp_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_log_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowActionResult {
    pub workflow_id: String,
    pub run_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowEventsResult {
    pub workflow_id: String,
    pub run_id: String,
    pub status: String,
    pub next_sequence: u64,
    #[serde(default)]
    pub events: Vec<WorkflowRuntimeEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationDecision {
    Approve,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveConfirmationRequest {
    pub workflow_id: String,
    pub run_id: String,
    pub confirmation_id: String,
    pub decision: ConfirmationDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InDoubtDecision {
    Retry,
    UseResponse,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveInDoubtOperationRequest {
    pub operation_id: String,
    pub decision: InDoubtDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveInDoubtOperationResult {
    pub operation_id: String,
    pub decision: InDoubtDecision,
    pub workflow: WorkflowActionResult,
}

/// 路径 B：把交流后同意的输出改写进被拒确认项的关联节点并通过。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverrideConfirmationOutputRequest {
    pub workflow_id: String,
    pub run_id: String,
    pub confirmation_id: String,
    /// 改写的节点输出，键为端口 alias，值为 PortValue（内联或引用）。
    #[serde(default)]
    pub new_outputs: crate::contracts::PortMap,
}

/// 路径 A：把外部正文注入为指定节点的输出，从该节点下游重跑。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeFromNodeRequest {
    pub workflow_id: String,
    pub run_id: String,
    pub node_id: String,
    /// 注入的节点输出（通常含 chapter_text 等正文端口）。
    #[serde(default)]
    pub injected_outputs: crate::contracts::PortMap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveConfirmationResult {
    pub workflow: WorkflowActionResult,
    pub confirmation: ConfirmationLogEntry,
    pub badges: SidebarBadgeCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandAcknowledgement {
    pub accepted: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRepositoryRequest {
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickEditRequest {
    pub selected_text: String,
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_ref: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectAiRequest {
    pub message: String,
    #[serde(default)]
    pub chat_history: Vec<ProjectAiChatMessage>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id_to_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append_memory: Option<String>,
}

pub use crate::frontend::{ProjectAiChatMessage, ProjectAiChatRole};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectAiResponse {
    pub answer: String,
    #[serde(default)]
    pub chat_history: Vec<ProjectAiChatMessage>,
    #[serde(default)]
    pub resolved_references: Vec<ProjectReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run: Option<WorkflowRunStarted>,
    pub project_memory: String,
    pub conversation_id: String,
    pub conversation_revision: u64,
    pub summary_revision: u64,
    #[serde(default)]
    pub new_messages: Vec<ProjectAiChatMessage>,
    #[serde(default)]
    pub conversation_snapshot: Vec<ProjectAiChatMessage>,
    #[serde(default)]
    pub conversation_summary: String,
    #[serde(default)]
    pub project_memory_revision: String,
    #[serde(default)]
    pub history_truncated: bool,
    #[serde(default)]
    pub memory_truncated: bool,
    #[serde(default)]
    pub references_truncated: bool,
    #[serde(default)]
    pub summary_truncated: bool,
    #[serde(default)]
    pub estimated_input_tokens: u64,
    #[serde(default)]
    pub context_limit_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectWorkflowTool {
    tool_name: String,
    display_name: String,
    workflow_id: String,
    start_node_id: String,
    input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkflowTool {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub workflow_id: String,
    pub start_node_id: String,
    pub input_schema: Value,
}

pub fn list_recent_projects(state: &AriadneAppState) -> CommandResult<Vec<RecentProjectEntry>> {
    recent_project_store(state.app_state_root())
        .read_all()
        .map_err(error_to_string)
}

pub fn create_project(
    state: &AriadneAppState,
    project_root: String,
    name: Option<String>,
) -> CommandResult<ProjectInitReport> {
    let _activation = state.lock_project_activation()?;
    let previous_root = state.project_root()?;
    let requested_root = absolute_path(Path::new(project_root.trim()));
    ensure_no_parent_traversal(&requested_root)?;
    let file_name = requested_root
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CommandError::validation("project root must name a new directory"))?;
    let parent = requested_root
        .parent()
        .ok_or_else(|| CommandError::validation("project root must have a parent directory"))?;
    validate_existing_project_root(parent)?;
    let parent = parent.canonicalize().map_err(error_to_string)?;
    let project_root = parent.join(file_name);

    let _creation_lock =
        crate::config::store::PathWriteLock::acquire(&project_root).map_err(error_to_string)?;
    if project_root.exists() {
        return Err(CommandError::conflict(format!(
            "project root already exists: {}",
            project_root.display()
        )));
    }

    let published =
        publish_initialized_project(&project_root, state.app_state_root(), name.as_deref())
            .map_err(error_to_string)?;
    let current = match complete_created_project_activation(state, &project_root) {
        Ok(current) => current,
        Err(error) => {
            return Err(rollback_created_project(
                state,
                &previous_root,
                &project_root,
                published,
                error,
            ))
        }
    };
    let mut report = published.commit();
    report.project_name = current.project_name;
    report.ready = true;
    Ok(report)
}

fn complete_created_project_activation(
    state: &AriadneAppState,
    project_root: &Path,
) -> CommandResult<CurrentProjectStatus> {
    let _project_mutation = acquire_project_mutation_guard(project_root, "project_create")?;
    activate_initialized_project(state, project_root, None)
}

fn rollback_created_project(
    state: &AriadneAppState,
    previous_root: &Path,
    project_root: &Path,
    published: PublishedProjectCreation,
    mut error: CommandError,
) -> CommandError {
    let mut rollback_errors = Vec::new();
    let cleared = match state.clear_project_root() {
        Ok(()) => true,
        Err(rollback_error) => {
            rollback_errors.push(format!("clear new project state: {rollback_error}"));
            false
        }
    };
    if cleared {
        if let Err(rollback_error) = published.rollback() {
            rollback_errors.push(format!("remove incomplete project: {rollback_error}"));
        }
        if !previous_root.as_os_str().is_empty() && previous_root != project_root {
            if let Err(rollback_error) = activate_initialized_project(state, previous_root, None) {
                rollback_errors.push(format!("restore previous project: {rollback_error}"));
            }
        }
    }
    if !rollback_errors.is_empty() {
        let diagnostic = format!(
            "{}; project creation rollback also failed: {}",
            error.diagnostic_text(),
            rollback_errors.join("; ")
        );
        error.diagnostic = Some(diagnostic);
    }
    error
}

pub fn open_project(
    state: &AriadneAppState,
    project_root: String,
    name: Option<String>,
) -> CommandResult<CurrentProjectStatus> {
    let _activation = state.lock_project_activation()?;
    let previous_root = state.project_root()?;
    let project_root = canonicalize_initialized_project_root(Path::new(&project_root))?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "project_open")?;
    let current = match activate_initialized_project(state, &project_root, name.as_deref()) {
        Ok(current) => current,
        Err(error) => {
            return Err(rollback_existing_project_activation(
                state,
                &previous_root,
                &project_root,
                error,
            ))
        }
    };
    Ok(current)
}

pub fn close_project(state: &AriadneAppState) -> CommandResult<()> {
    let _activation = state.lock_project_activation()?;
    state.clear_project_root()
}

pub fn get_current_project(state: &AriadneAppState) -> CommandResult<CurrentProjectStatus> {
    let project_root = project_root_from_state(state, None)?;
    state.retrieval_runtime()?;
    state.ensure_workflow_scheduler()?;
    current_project_status(&project_root)
}

pub fn get_app_status(state: &AriadneAppState) -> CommandResult<AppStatus> {
    // 空路径表示尚未打开项目；非空路径必须是已初始化项目，不能把损坏状态伪装成无项目。
    let configured_root = state.project_root()?;
    let active_project = if configured_root.as_os_str().is_empty() {
        None
    } else {
        validate_initialized_project_root(&configured_root)?;
        Some(configured_root)
    };

    // 个性化与项目解耦：始终可读全局偏好；仅真正无项目时 current_project / badges 走兜底。
    let preferences = UiPreferencesStore::read_global_or_migrate(
        state.app_state_root(),
        active_project.as_deref(),
    )
    .map_err(error_to_string)?;

    let (current_project, badges) = match active_project {
        Some(project_root) => {
            state.retrieval_runtime()?;
            state.ensure_workflow_scheduler()?;
            (
                current_project_status(&project_root)?,
                get_sidebar_badges_impl(&project_root)?,
            )
        }
        None => (
            CurrentProjectStatus {
                project_root: PathBuf::new(),
                project_name: String::new(),
            },
            SidebarBadgeCounts::default(),
        ),
    };

    Ok(AppStatus {
        current_project,
        badges,
        preferences,
    })
}

pub fn get_sidebar_badges(state: &AriadneAppState) -> CommandResult<SidebarBadgeCounts> {
    let project_root = project_root_from_state(state, None)?;
    get_sidebar_badges_impl(&project_root)
}

/// D3：项目维护（Git restore / 全量重建）状态；桌面横幅与门禁共用。
pub fn get_project_maintenance(
    state: &AriadneAppState,
) -> CommandResult<Option<crate::documents::ProjectMaintenanceState>> {
    let project_root = project_root_from_state(state, None)?;
    validate_initialized_project_root(&project_root)?;
    document_service(&project_root)
        .invalidation_outbox()
        .maintenance_state()
        .map_err(error_to_string)
}

/// N4：列出索引 dead-letter 事件。
pub fn list_index_dead_letters(
    state: &AriadneAppState,
) -> CommandResult<Vec<crate::documents::IndexInvalidationEvent>> {
    let project_root = project_root_from_state(state, None)?;
    validate_initialized_project_root(&project_root)?;
    document_service(&project_root)
        .invalidation_outbox()
        .list_dead_letters()
        .map_err(error_to_string)
}

/// N4：手动将 dead-letter 重新入队。
pub fn requeue_index_dead_letter(state: &AriadneAppState, event_id: String) -> CommandResult<()> {
    let project_root = project_root_from_state(state, None)?;
    validate_initialized_project_root(&project_root)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "index_dead_letter_requeue")?;
    document_service(&project_root)
        .invalidation_outbox()
        .requeue_dead_letter(&event_id)
        .map_err(error_to_string)
}

pub fn set_project_root(state: &AriadneAppState, project_root: String) -> CommandResult<()> {
    let _activation = state.lock_project_activation()?;
    let previous_root = state.project_root()?;
    let project_root = canonicalize_initialized_project_root(Path::new(&project_root))?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "project_open")?;
    if let Err(error) = activate_initialized_project(state, &project_root, None) {
        return Err(rollback_existing_project_activation(
            state,
            &previous_root,
            &project_root,
            error,
        ));
    }
    Ok(())
}

fn rollback_existing_project_activation(
    state: &AriadneAppState,
    previous_root: &Path,
    attempted_root: &Path,
    mut error: CommandError,
) -> CommandError {
    if previous_root == attempted_root {
        return error;
    }

    let active_root = match state.project_root() {
        Ok(root) => root,
        Err(rollback_error) => {
            error.diagnostic = Some(format!(
                "{}; project activation rollback could not inspect active root: {rollback_error}",
                error.diagnostic_text()
            ));
            return error;
        }
    };
    if active_root != attempted_root {
        return error;
    }

    let mut rollback_errors = Vec::new();
    if let Err(rollback_error) = state.clear_project_root() {
        rollback_errors.push(format!("clear attempted project: {rollback_error}"));
    } else if !previous_root.as_os_str().is_empty() {
        if let Err(rollback_error) = activate_initialized_project(state, previous_root, None) {
            rollback_errors.push(format!("restore previous project: {rollback_error}"));
        }
    }

    if !rollback_errors.is_empty() {
        error.diagnostic = Some(format!(
            "{}; project activation rollback also failed: {}",
            error.diagnostic_text(),
            rollback_errors.join("; ")
        ));
    }
    error
}

fn activate_initialized_project(
    state: &AriadneAppState,
    project_root: &Path,
    name: Option<&str>,
) -> CommandResult<CurrentProjectStatus> {
    let project_root = canonicalize_initialized_project_root(project_root)?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    let config_store = ConfigStore::new(&project_root);
    let original_config = config_store.load_or_create().map_err(error_to_string)?;
    let mut candidate_config = original_config.clone();
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        candidate_config.app.project_name = name.to_owned();
    }
    candidate_config.validate().map_err(error_to_string)?;

    let existing_runtime = state
        .retrieval_runtime
        .lock()
        .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?
        .as_ref()
        .filter(|runtime| runtime.project_root() == project_root)
        .cloned();
    let runtime = match existing_runtime {
        Some(runtime)
            if runtime
                .matches_project_config(&candidate_config)
                .map_err(error_to_string)? =>
        {
            runtime
        }
        previous => Arc::new(
            ProjectRetrievalRuntime::from_config(
                &project_root,
                state.secret_store.as_ref(),
                &candidate_config,
                previous.as_deref(),
            )
            .map_err(error_to_string)?,
        ),
    };
    validate_project_activation_recovery(&project_root)?;
    let scheduler = state.prepare_workflow_scheduler(&project_root)?;

    // 最近项目和显示名称在切换前完成；失败时只恢复候选文件，不触碰当前 A。
    let recent_store = recent_project_store(state.app_state_root());
    let recent_before = recent_store.read_all().map_err(error_to_string)?;
    let config_changed = candidate_config != original_config;
    if config_changed {
        if let Err(error) = config_store.save(&candidate_config) {
            drop(scheduler);
            return Err(error_to_string(error));
        }
    }
    let current = CurrentProjectStatus {
        project_root: project_root.clone(),
        project_name: if candidate_config.app.project_name.trim().is_empty() {
            project_root
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("Ariadne Project")
                .to_owned()
        } else {
            candidate_config.app.project_name.clone()
        },
    };
    if let Err(error) = record_current_project(state.app_state_root(), &current) {
        let mut diagnostic = error_to_string(error).diagnostic_text().to_owned();
        if config_changed {
            if let Err(rollback_error) = config_store.save(&original_config) {
                diagnostic = format!(
                    "{diagnostic}; project name rollback failed: {}",
                    error_to_string(rollback_error).diagnostic_text()
                );
            }
        }
        drop(scheduler);
        return Err(CommandError::internal(diagnostic));
    }

    if let Err(error) =
        state.commit_project_activation(current.project_root.clone(), runtime, scheduler)
    {
        let mut diagnostic = error.diagnostic_text().to_owned();
        if let Err(rollback_error) = recent_store.write_all(&recent_before) {
            diagnostic = format!(
                "{diagnostic}; recent project rollback failed: {}",
                error_to_string(rollback_error).diagnostic_text()
            );
        }
        if config_changed {
            if let Err(rollback_error) = config_store.save(&original_config) {
                diagnostic = format!(
                    "{diagnostic}; project name rollback failed: {}",
                    error_to_string(rollback_error).diagnostic_text()
                );
            }
        }
        return Err(CommandError::internal(diagnostic));
    }

    complete_committed_project_activation(state, &current.project_root);
    state.start_committed_workflow_scheduler(&current.project_root)?;
    Ok(current)
}

/// 只验证候选恢复存储可读，不领取 lease、不改 saga/outbox 状态。
fn validate_project_activation_recovery(project_root: &Path) -> CommandResult<()> {
    let runtime_path = project_root.join(crate::workflow::RUNTIME_DB_FILE);
    if runtime_path.exists() {
        let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
        store
            .list_recoverable_confirmation_resolutions()
            .map_err(error_to_string)?;
        store
            .list_orphaned_runnable_states(workflow_lease_now_ms()?)
            .map_err(error_to_string)?;
    }
    document_service(project_root)
        .invalidation_outbox()
        .pending()
        .map(|_| ())
        .map_err(error_to_string)
}

/// 提交项目身份后启动可重试的恢复动作；恢复失败不再触发跨项目补偿切换。
fn complete_committed_project_activation(state: &AriadneAppState, project_root: &Path) {
    let runtime = match state.retrieval_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("[ariadne] project activation runtime recovery deferred: {error}");
            return;
        }
    };
    let recovery_steps: [(&str, CommandResult<()>); 3] = [
        (
            "confirmation saga recovery",
            recover_confirmation_resolution_sagas(project_root).map(|_| ()),
        ),
        (
            "index bootstrap",
            ensure_index_bootstrap_on_open(project_root).map(|_| ()),
        ),
        (
            "index worker recovery",
            resume_indexing_worker_for_project(project_root, Arc::clone(&runtime)),
        ),
    ];
    for (stage, result) in recovery_steps {
        if let Err(error) = result {
            eprintln!(
                "[ariadne] project activation {stage} deferred for {}: {error}",
                project_root.display()
            );
        }
    }
    if let Err(error) = recover_orphaned_workflow_workers(
        project_root,
        Arc::clone(&state.secret_store),
        Some(runtime),
    ) {
        eprintln!(
            "[ariadne] project activation workflow recovery deferred for {}: {error}",
            project_root.display()
        );
    }
}

pub fn get_works_tree(state: &AriadneAppState) -> CommandResult<WorksTreeNode> {
    let project_root = project_root_from_state(state, None)?;
    let index = load_chapter_index(&project_root)?;
    let knowledge_store =
        crate::rag::SqliteWritingKnowledgeStore::open(&project_root).map_err(error_to_string)?;
    let chapter_stage = knowledge_store
        .load_chapter_stage_map()
        .map_err(error_to_string)?;
    build_works_tree(&index, &chapter_stage, project_root.join("planning")).map_err(error_to_string)
}

pub fn get_chapter_summary_view(
    state: &AriadneAppState,
    chapter_id: String,
) -> CommandResult<crate::rag::ChapterSummaryView> {
    let project_root = project_root_from_state(state, None)?;
    let index = load_chapter_index(&project_root)?;
    if !index
        .chapter_bodies()
        .iter()
        .any(|entry| entry.chapter_id == chapter_id)
    {
        return Err(CommandError::not_found(format!(
            "chapter is not present in the chapter index: {chapter_id}"
        )));
    }
    crate::rag::SqliteWritingKnowledgeStore::open(&project_root)
        .and_then(|store| store.load_chapter_summary_view(&chapter_id))
        .map_err(error_to_string)
}

pub fn get_document_tree(
    state: &AriadneAppState,
    project_id: Option<String>,
) -> CommandResult<DocumentTreeNode> {
    let project_root = project_root_from_state(state, project_id)?;
    get_document_tree_impl(&project_root)
}

pub fn get_document_content(
    state: &AriadneAppState,
    document_id: Option<String>,
    path: Option<String>,
) -> CommandResult<String> {
    let project_root = project_root_from_state(state, None)?;
    get_document_content_impl(&project_root, document_id, path)
}

pub fn get_document_content_details(
    state: &AriadneAppState,
    document_id: Option<String>,
    path: Option<String>,
) -> CommandResult<DocumentContent> {
    let project_root = project_root_from_state(state, None)?;
    get_document_content_details_impl(&project_root, document_id, path)
}

pub fn save_document_content(
    state: &AriadneAppState,
    document_id: String,
    content: String,
) -> CommandResult<DocumentWriteReport> {
    save_document_content_with_version(state, document_id, content, None)
}

pub fn save_document_content_with_version(
    state: &AriadneAppState,
    document_id: String,
    content: String,
    base_version: Option<String>,
) -> CommandResult<DocumentWriteReport> {
    let project_root = project_root_from_state(state, None)?;
    let report =
        save_document_content_report_impl(&project_root, document_id, content, base_version)?;
    spawn_indexing_worker_for_state(state)?;
    Ok(report)
}

pub fn import_chapter(
    state: &AriadneAppState,
    mut request: ChapterImportRequest,
) -> CommandResult<ChapterDocumentIndex> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "chapter_import")?;
    request.source_path = project_path_buf(&project_root, &request.source_path)?;
    request.target_path = project_path_buf(&project_root, &request.target_path)?;
    let mut index = load_chapter_index(&project_root)?;
    if !request.overwrite
        && index.entries.iter().any(|entry| {
            entry.chapter_id == request.chapter_id || entry.path == request.target_path
        })
    {
        return Err(CommandError::conflict(
            "chapter id or target document already exists; explicit overwrite is required",
        ));
    }
    let documents = document_service(&project_root);
    let report = import_chapter_document(&documents, request).map_err(error_to_string)?;
    index
        .entries
        .retain(|entry| entry.chapter_id != report.entry.chapter_id);
    index.entries.push(report.entry);
    save_chapter_index(&project_root, &index)?;
    spawn_indexing_worker_for_state(state)?;
    Ok(index)
}

pub fn export_chapters(
    state: &AriadneAppState,
    selected_chapter_ids: Vec<String>,
    artifact_id: String,
    format: Option<ChapterExportFormat>,
) -> CommandResult<CombinedExportReport> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "chapter_export")?;
    let documents = configured_document_service(&project_root)?;
    let index = load_chapter_index(&project_root)?;
    export_chapters_combined(
        &documents,
        &index,
        &selected_chapter_ids,
        &artifact_id,
        format.unwrap_or_default(),
    )
    .map_err(error_to_string)
}

pub fn load_workflow_graph(
    state: &AriadneAppState,
    workflow_id: Option<String>,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(state, None)?;
    load_workflow_graph_impl(&project_root, workflow_id)
}

pub fn load_project_canvas(state: &AriadneAppState) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(state, None)?;
    load_project_canvas_impl(&project_root)
}

pub fn list_workflow_graphs(state: &AriadneAppState) -> CommandResult<Vec<WorkflowSummary>> {
    let project_root = project_root_from_state(state, None)?;
    list_workflow_graphs_impl(&project_root)
}

pub fn validate_workflow_graph(graph_data: WorkflowGraphData) -> CommandResult<()> {
    let workflow = graph_to_workflow(graph_data)?;
    validate_workflow_execution_contracts(&workflow).map_err(error_to_string)
}

pub fn save_workflow_graph(
    state: &AriadneAppState,
    graph_data: WorkflowGraphData,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(state, None)?;
    save_workflow_graph_impl(&project_root, graph_data)
}

pub fn save_project_canvas(
    state: &AriadneAppState,
    graph_data: WorkflowGraphData,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(state, None)?;
    save_project_canvas_impl(&project_root, graph_data)
}

fn load_modify_save_workflow(
    project_root: &Path,
    workflow_id: String,
    mutate: impl FnOnce(&mut WorkflowDefinition) -> CommandResult<()>,
) -> CommandResult<WorkflowGraphData> {
    let (mut workflow, revision) =
        load_workflow_definition_with_revision(project_root, Some(workflow_id))?;
    mutate(&mut workflow)?;
    let mut graph = workflow_to_graph(workflow);
    if !revision.is_empty() {
        graph.expected_revision = Some(revision);
    }
    save_workflow_graph_impl(project_root, graph)
}

pub fn apply_node_detail_patch(
    state: &AriadneAppState,
    workflow_id: String,
    patch: NodeDetailPatch,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(state, None)?;
    load_modify_save_workflow(&project_root, workflow_id, |workflow| {
        apply_node_detail_patch_to_workflow(workflow, patch).map_err(error_to_string)
    })
}

pub fn upsert_canvas_annotation(
    state: &AriadneAppState,
    workflow_id: String,
    annotation: CanvasAnnotation,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(state, None)?;
    load_modify_save_workflow(&project_root, workflow_id, |workflow| {
        upsert_canvas_annotation_in_workflow(workflow, annotation).map_err(error_to_string)
    })
}

pub fn set_node_breakpoint(
    state: &AriadneAppState,
    workflow_id: String,
    node_id: String,
    enabled: bool,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(state, None)?;
    load_modify_save_workflow(&project_root, workflow_id, |workflow| {
        crate::frontend::set_node_breakpoint(workflow, &node_id, enabled).map_err(error_to_string)
    })
}

pub fn export_workflow_selection(
    state: &AriadneAppState,
    workflow_id: String,
    selected_node_ids: Vec<String>,
) -> CommandResult<crate::frontend::WorkflowSelectionExport> {
    let project_root = project_root_from_state(state, None)?;
    let workflow = load_workflow_definition(&project_root, Some(workflow_id))?;
    export_workflow_selection_from_workflow(&workflow, &selected_node_ids).map_err(error_to_string)
}

pub fn pack_workflow_selection_impl(
    project_root: &Path,
    workflow_id: String,
    selected_node_ids: Vec<String>,
    subworkflow_node_id: Option<String>,
    title: Option<String>,
    expected_revision: Option<String>,
) -> CommandResult<WorkflowPackGraphReport> {
    let app_state_root = crate::config::trusted_app_state_for_project(project_root);
    pack_workflow_selection_impl_with_operation_id_and_app_state(
        project_root,
        &app_state_root,
        WorkflowPackRequest {
            workflow_id,
            selected_node_ids,
            subworkflow_node_id,
            title,
            expected_revision,
            operation_id: None,
        },
    )
}

pub fn pack_workflow_selection_impl_with_operation_id(
    project_root: &Path,
    workflow_id: String,
    selected_node_ids: Vec<String>,
    subworkflow_node_id: Option<String>,
    title: Option<String>,
    expected_revision: Option<String>,
    operation_id: Option<String>,
) -> CommandResult<WorkflowPackGraphReport> {
    let app_state_root = crate::config::trusted_app_state_for_project(project_root);
    pack_workflow_selection_impl_with_operation_id_and_app_state(
        project_root,
        &app_state_root,
        WorkflowPackRequest {
            workflow_id,
            selected_node_ids,
            subworkflow_node_id,
            title,
            expected_revision,
            operation_id,
        },
    )
}

pub fn pack_workflow_selection_impl_with_operation_id_and_app_state(
    project_root: &Path,
    app_state_root: &Path,
    request: WorkflowPackRequest,
) -> CommandResult<WorkflowPackGraphReport> {
    let WorkflowPackRequest {
        workflow_id,
        selected_node_ids,
        subworkflow_node_id,
        title,
        expected_revision,
        operation_id,
    } = request;
    let _project_mutation = acquire_project_mutation_guard(project_root, "workflow_pack")?;
    let request_hash = pack_workflow_request_hash(
        &workflow_id,
        &selected_node_ids,
        subworkflow_node_id.as_deref(),
        title.as_deref(),
        expected_revision.as_deref(),
    )?;
    let (mut workflow, loaded_revision) =
        load_workflow_definition_with_revision(project_root, Some(workflow_id.clone()))?;
    let operation_id = match operation_id {
        Some(operation_id) => validate_pack_operation_id(operation_id)?,
        None => generated_pack_operation_id(
            &workflow_id,
            &request_hash,
            expected_revision.as_deref().unwrap_or(&loaded_revision),
        ),
    };
    let operation_path = pack_operation_path(project_root, app_state_root, &operation_id)?;
    let _operation_write =
        crate::config::store::PathWriteLock::acquire(&operation_path).map_err(error_to_string)?;
    if let Some(record) = load_pack_operation_record_if_exists(&operation_path, &operation_id)? {
        return resume_pack_operation(
            project_root,
            &operation_path,
            record,
            &request_hash,
            &loaded_revision,
        );
    }
    if let Some(expected_revision) = expected_revision.as_deref() {
        if expected_revision != loaded_revision {
            return Err(CommandError::conflict(format!(
                "workflow content revision conflict for {workflow_id}: expected {expected_revision}, actual {loaded_revision}"
            )));
        }
    }
    let base_revision = expected_revision
        .clone()
        .unwrap_or_else(|| loaded_revision.clone());

    let report = pack_workflow_selection_in_workflow(
        &mut workflow,
        &selected_node_ids,
        subworkflow_node_id,
        title,
    )
    .map_err(error_to_string)?;
    let mut graph = workflow_to_graph(report.workflow);
    graph.expected_revision = Some(base_revision.clone());
    let prepared_workflow = graph_to_workflow(graph.clone())?;
    validate_workflow_execution_contracts(&prepared_workflow).map_err(error_to_string)?;
    let prepared_body =
        serde_json::to_string_pretty(&prepared_workflow).map_err(error_to_string)?;
    let mut prepared_graph = workflow_to_graph(prepared_workflow);
    prepared_graph.content_revision = Some(content_revision_hash(prepared_body.as_bytes()));
    let pack_report = WorkflowPackGraphReport {
        workflow: prepared_graph,
        subworkflow_node_id: report.subworkflow_node_id.as_str().to_owned(),
        embedded_workflow: workflow_to_graph(report.embedded_workflow),
        boundary_inputs: report.boundary_inputs,
        boundary_outputs: report.boundary_outputs,
        operation_id: Some(operation_id.clone()),
    };
    let mut operation = WorkflowPackOperationRecord {
        operation_id,
        request_hash,
        expected_revision: base_revision,
        status: WorkflowPackOperationStatus::Prepared,
        report: pack_report,
    };
    // N8：意图必须先于工作流 replace 落盘。若进程在二者之间退出，同一 operation_id
    // 可从 prepared 记录继续；若 replace 已完成但状态尚未推进，可按结果 revision 对账。
    persist_pack_operation_record(&operation_path, &operation)?;
    let saved = save_workflow_graph_impl(project_root, graph)?;
    operation.report.workflow = saved;
    operation.status = WorkflowPackOperationStatus::Committed;
    persist_pack_operation_record(&operation_path, &operation)?;
    Ok(operation.report)
}

pub fn get_pack_operation(
    state: &AriadneAppState,
    operation_id: String,
) -> CommandResult<WorkflowPackGraphReport> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "workflow_pack_recovery")?;
    load_pack_operation(&project_root, state.app_state_root(), &operation_id)
}

pub fn pack_workflow_selection(
    state: &AriadneAppState,
    workflow_id: String,
    selected_node_ids: Vec<String>,
    subworkflow_node_id: Option<String>,
    title: Option<String>,
) -> CommandResult<WorkflowPackGraphReport> {
    pack_workflow_selection_with_revision(
        state,
        workflow_id,
        selected_node_ids,
        subworkflow_node_id,
        title,
        None,
    )
}

pub fn pack_workflow_selection_with_revision(
    state: &AriadneAppState,
    workflow_id: String,
    selected_node_ids: Vec<String>,
    subworkflow_node_id: Option<String>,
    title: Option<String>,
    expected_revision: Option<String>,
) -> CommandResult<WorkflowPackGraphReport> {
    pack_workflow_selection_with_operation_id(
        state,
        WorkflowPackRequest {
            workflow_id,
            selected_node_ids,
            subworkflow_node_id,
            title,
            expected_revision,
            operation_id: None,
        },
    )
}

pub fn pack_workflow_selection_with_operation_id(
    state: &AriadneAppState,
    request: WorkflowPackRequest,
) -> CommandResult<WorkflowPackGraphReport> {
    let project_root = project_root_from_state(state, None)?;
    pack_workflow_selection_impl_with_operation_id_and_app_state(
        &project_root,
        state.app_state_root(),
        request,
    )
}

pub fn run_workflow(
    state: &AriadneAppState,
    workflow_id: String,
    start_node_id: Option<String>,
) -> CommandResult<WorkflowRunStarted> {
    start_workflow(state, workflow_id, start_node_id)
}

pub fn start_workflow(
    state: &AriadneAppState,
    workflow_id: String,
    start_node_id: Option<String>,
) -> CommandResult<WorkflowRunStarted> {
    let project_root = project_root_from_state(state, None)?;
    let started = start_workflow_request(
        &project_root,
        Arc::clone(&state.secret_store),
        Some(state.retrieval_runtime()?),
        RunWorkflowRequest {
            workflow_id,
            start_node_id,
            initial_inputs: BTreeMap::new(),
        },
    )?;
    state.ensure_workflow_scheduler()?;
    Ok(started)
}

pub fn start_workflow_with_request(
    state: &AriadneAppState,
    request: RunWorkflowRequest,
) -> CommandResult<WorkflowRunStarted> {
    let project_root = project_root_from_state(state, None)?;
    let started = start_workflow_request(
        &project_root,
        Arc::clone(&state.secret_store),
        Some(state.retrieval_runtime()?),
        request,
    )?;
    state.ensure_workflow_scheduler()?;
    Ok(started)
}

fn start_workflow_request(
    project_root: &Path,
    secrets: Arc<dyn SecretStore>,
    retrieval_runtime: Option<Arc<ProjectRetrievalRuntime>>,
    request: RunWorkflowRequest,
) -> CommandResult<WorkflowRunStarted> {
    validate_existing_project_root(project_root)?;
    let project_mutation = acquire_project_mutation_guard(project_root, "workflow_start")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let run_id = new_run_id()?;
    let run_id_text = run_id.as_str().to_owned();
    let prepared = prepare_workflow_run_state(
        project_root,
        secrets.as_ref(),
        retrieval_runtime,
        &request,
        run_id.clone(),
    )?;
    let retrieval_runtime = prepared.retrieval_runtime.clone();
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    store
        .create_state(&prepared.state)
        .map_err(error_to_string)?;
    let worker_lease =
        acquire_workflow_worker_lease(&store, &prepared.state.workflow_id, &prepared.state.run_id)?
            .ok_or_else(|| {
                CommandError::internal(
                    "new workflow run could not acquire its initial worker lease",
                )
            })?;
    let worker_workflow_id = request.workflow_id.clone();
    let spawned_workflow_id = worker_workflow_id.clone();
    let worker_root = project_root.to_path_buf();
    let worker_run_id_text = run_id_text.clone();
    let spawned_worker_lease = worker_lease.clone();
    let spawned_project_mutation = Arc::clone(&project_mutation);
    let spawn_result = std::thread::Builder::new()
        .name(format!("ariadne-workflow-{}", run_id.as_str()))
        .spawn(move || {
            let _project_mutation = spawned_project_mutation;
            let execution_lease = spawned_worker_lease.clone();
            let worker_result = run_with_workflow_worker_lease(
                &worker_root,
                spawned_worker_lease,
                |cancellation| {
                    continue_workflow_run_impl(
                        &worker_root,
                        secrets.as_ref(),
                        retrieval_runtime.clone(),
                        spawned_workflow_id.clone(),
                        worker_run_id_text.clone(),
                        &execution_lease,
                        cancellation,
                    )
                },
            );
            if let Err(error) = worker_result {
                if error.diagnostic_text() != WORKFLOW_WORKER_LEASE_LOST_ERROR {
                    record_workflow_worker_error(
                        &worker_root,
                        &spawned_workflow_id,
                        &worker_run_id_text,
                        "workflow worker failed",
                        &error,
                        Some(&execution_lease),
                    );
                }
                eprintln!("[ariadne] workflow worker failed: {error}");
            }
        });
    if let Err(error) = spawn_result {
        let spawn_error = error_to_string(error);
        let _ = mark_workflow_run_failed_with_lease_impl(
            project_root,
            &worker_workflow_id,
            &run_id_text,
            "workflow_worker_spawn_failed",
            "worker_spawn",
            &spawn_error,
            "error.workflow.worker_spawn_failed.recovery",
            &worker_lease,
        );
        return Err(spawn_error);
    }
    Ok(WorkflowRunStarted {
        run_id: run_id_text,
        status: "queued".to_owned(),
    })
}

pub fn pause_workflow(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
    reason: Option<String>,
) -> CommandResult<WorkflowActionResult> {
    update_workflow_run_control(
        &project_root_from_state(state, None)?,
        workflow_id,
        run_id,
        |runtime| {
            runtime.request_pause(
                reason
                    .clone()
                    .unwrap_or_else(|| "paused by user".to_owned()),
            )
        },
    )
}

pub fn stop_workflow(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
    reason: Option<String>,
) -> CommandResult<WorkflowActionResult> {
    let project_root = project_root_from_state(state, None)?;
    let workflow_id_typed = WorkflowId::from(workflow_id.clone());
    let run_id_typed = RunId::from(run_id.clone());
    let reason = reason.unwrap_or_else(|| "stopped by user".to_owned());
    let store = SqliteWorkflowRuntimeStore::open(&project_root).map_err(error_to_string)?;
    let result = match store
        .request_stop(
            &workflow_id_typed,
            &run_id_typed,
            &reason,
            workflow_lease_now_ms()?,
        )
        .map_err(error_to_string)?
    {
        WorkflowStopRequestResult::Saved { state } => WorkflowActionResult {
            workflow_id,
            run_id,
            status: run_status_label(state.status).to_owned(),
        },
        WorkflowStopRequestResult::NotFound => {
            return Err(CommandError::not_found(format!(
                "workflow run not found: {workflow_id}/{run_id}"
            )));
        }
    };
    state.ensure_workflow_scheduler()?;
    Ok(result)
}

pub fn resume_workflow(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
) -> CommandResult<WorkflowActionResult> {
    let project_root = project_root_from_state(state, None)?;
    let project_mutation = acquire_project_mutation_guard(&project_root, "workflow_resume")?;
    let workflow_id_typed = WorkflowId::from(workflow_id.clone());
    let run_id_typed = RunId::from(run_id.clone());
    let owner_id = format!("worker-{}", new_run_id()?.as_str());
    let store = SqliteWorkflowRuntimeStore::open(&project_root).map_err(error_to_string)?;
    let result = match store
        .claim_resume(
            &workflow_id_typed,
            &run_id_typed,
            &owner_id,
            workflow_lease_now_ms()?,
            WORKFLOW_WORKER_LEASE_TTL_MS,
        )
        .map_err(error_to_string)?
    {
        crate::workflow::WorkflowResumeClaimResult::Claimed {
            state: run_state,
            lease,
        } => {
            let result = WorkflowActionResult {
                workflow_id,
                run_id,
                status: run_status_label(run_state.status).to_owned(),
            };
            spawn_continue_workflow_worker_with_lease(
                project_root,
                Arc::clone(&state.secret_store),
                Some(state.retrieval_runtime()?),
                result.workflow_id.clone(),
                result.run_id.clone(),
                lease,
                project_mutation,
            )?;
            result
        }
        crate::workflow::WorkflowResumeClaimResult::Busy { .. } => {
            let run_state = store
                .load_state(&workflow_id_typed, &run_id_typed)
                .map_err(error_to_string)?
                .ok_or_else(|| {
                    CommandError::not_found(format!(
                        "workflow run not found: {workflow_id}/{run_id}"
                    ))
                })?;
            WorkflowActionResult {
                workflow_id,
                run_id,
                status: run_status_label(run_state.status).to_owned(),
            }
        }
        crate::workflow::WorkflowResumeClaimResult::NotFound => {
            return Err(CommandError::not_found(format!(
                "workflow run not found: {workflow_id}/{run_id}"
            )));
        }
        crate::workflow::WorkflowResumeClaimResult::NotResumable { status } => {
            return Err(CommandError::conflict(format!(
                "workflow run cannot resume from status {}",
                run_status_label(status)
            )));
        }
    };
    state.ensure_workflow_scheduler()?;
    Ok(result)
}

/// 路径 B：把交流后同意的 Prudent 输出改写进关联节点并置为通过，解除暂停继续运行。
pub fn override_confirmation_output(
    state: &AriadneAppState,
    request: OverrideConfirmationOutputRequest,
) -> CommandResult<WorkflowActionResult> {
    let project_root = project_root_from_state(state, None)?;
    let project_mutation =
        acquire_project_mutation_guard(&project_root, "workflow_confirmation_override")?;
    let (result, lease) = mutate_workflow_run_and_claim(
        &project_root,
        request.workflow_id,
        request.run_id,
        |runtime| {
            runtime
                .override_confirmation_output(&request.confirmation_id, request.new_outputs.clone())
        },
    )?;
    if let Some(lease) = lease {
        spawn_continue_workflow_worker_with_lease(
            project_root,
            Arc::clone(&state.secret_store),
            Some(state.retrieval_runtime()?),
            result.workflow_id.clone(),
            result.run_id.clone(),
            lease,
            project_mutation,
        )?;
    }
    state.ensure_workflow_scheduler()?;
    Ok(result)
}

/// 路径 A：注入外部正文到指定节点并从其控制下游重跑，解除暂停继续运行。
pub fn resume_from_node(
    state: &AriadneAppState,
    request: ResumeFromNodeRequest,
) -> CommandResult<WorkflowActionResult> {
    let project_root = project_root_from_state(state, None)?;
    let project_mutation =
        acquire_project_mutation_guard(&project_root, "workflow_resume_from_node")?;
    let workflow_id = WorkflowId::from(request.workflow_id.clone());
    let run_id = RunId::from(request.run_id.clone());
    let store = SqliteWorkflowRuntimeStore::open(&project_root).map_err(error_to_string)?;
    let run_state = store
        .load_state(&workflow_id, &run_id)
        .map_err(error_to_string)?;
    let run_state = run_state.ok_or_else(|| {
        CommandError::not_found(format!(
            "workflow run not found: {}/{}",
            request.workflow_id, request.run_id
        ))
    })?;
    let (workflow, _) = workflow_for_run_state(&project_root, &workflow_id, &run_state)?;
    let (result, lease) = mutate_workflow_run_and_claim(
        &project_root,
        request.workflow_id,
        request.run_id,
        |runtime| {
            runtime.resume_from_node(
                &workflow,
                &NodeId::from(request.node_id.clone()),
                request.injected_outputs.clone(),
            )
        },
    )?;
    if let Some(lease) = lease {
        spawn_continue_workflow_worker_with_lease(
            project_root,
            Arc::clone(&state.secret_store),
            Some(state.retrieval_runtime()?),
            result.workflow_id.clone(),
            result.run_id.clone(),
            lease,
            project_mutation,
        )?;
    }
    state.ensure_workflow_scheduler()?;
    Ok(result)
}

pub fn get_workflow_run_state(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
) -> CommandResult<Option<crate::workflow::WorkflowRunState>> {
    let project_root = project_root_from_state(state, None)?;
    let store = SqliteWorkflowRuntimeStore::open(&project_root).map_err(error_to_string)?;
    store
        .load_state(
            &WorkflowId::from(workflow_id),
            &crate::contracts::RunId::from(run_id),
        )
        .map_err(error_to_string)
}

pub fn get_workflow_events(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
    after_sequence: Option<u64>,
    limit: Option<usize>,
) -> CommandResult<WorkflowEventsResult> {
    let project_root = project_root_from_state(state, None)?;
    get_workflow_events_impl(&project_root, workflow_id, run_id, after_sequence, limit)
}

pub fn get_budget_status(state: &AriadneAppState) -> CommandResult<BudgetStatus> {
    let project_root = project_root_from_state(state, None)?;
    get_budget_status_impl(&project_root)
}

pub fn get_app_settings(state: &AriadneAppState) -> CommandResult<AppSettings> {
    let project_root = project_root_from_state(state, None)?;
    get_app_settings_impl(&project_root)
}

pub fn get_app_runtime_settings(state: &AriadneAppState) -> CommandResult<AppRuntimeSettings> {
    let _global_settings = state.lock_global_settings()?;
    let project_root = state.project_root()?;
    let project_root = (!project_root.as_os_str().is_empty()).then_some(project_root);
    AppRuntimeSettingsStore::read_global_or_migrate(state.app_state_root(), project_root.as_deref())
        .map_err(error_to_string)
}

pub fn save_app_runtime_settings(
    state: &AriadneAppState,
    settings: AppRuntimeSettings,
) -> CommandResult<AppRuntimeSettings> {
    settings.validate().map_err(error_to_string)?;
    let _global_settings = state.lock_global_settings()?;
    let project_root = state.project_root()?;
    let active_project = if project_root.as_os_str().is_empty() {
        None
    } else {
        let root = canonicalize_initialized_project_root(&project_root)?;
        crate::config::bind_project_app_state(&root, state.app_state_root())
            .map_err(error_to_string)?;
        Some(root)
    };
    let _project_mutation = active_project
        .as_deref()
        .map(|root| acquire_project_mutation_guard(root, "app_runtime_settings_update"))
        .transpose()?;
    let store = AppRuntimeSettingsStore::default_for_app(state.app_state_root());
    let _runtime_transaction = store
        .lock_transaction_exclusive()
        .map_err(error_to_string)?;
    let previous = AppRuntimeSettingsStore::read_global_or_migrate(
        state.app_state_root(),
        active_project.as_deref(),
    )
    .map_err(error_to_string)?;
    store.write(&settings).map_err(error_to_string)?;

    if active_project.is_some() {
        if let Err(error) = state.reload_retrieval_runtime() {
            let rollback = store.write(&previous).map_err(error_to_string);
            let runtime_rollback = rollback
                .as_ref()
                .map(|_| state.reload_retrieval_runtime())
                .unwrap_or_else(|rollback_error| Err(rollback_error.clone()));
            return Err(match (rollback, runtime_rollback) {
                (Ok(()), Ok(_)) => error,
                (settings_rollback, runtime_rollback) => CommandError::internal(format!(
                    "failed to apply app runtime settings: {error}; settings rollback: {}; runtime rollback: {}",
                    rollback_result_text(settings_rollback),
                    rollback_result_text(runtime_rollback.map(|_| ()))
                )),
            });
        }
    }

    Ok(settings)
}

pub fn save_app_settings(
    state: &AriadneAppState,
    settings: AppSettings,
) -> CommandResult<AppSettings> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "app_settings_update")?;
    save_app_settings_impl(&project_root, settings)?;
    get_app_settings_impl(&project_root)
}

pub fn save_general_section_settings(
    state: &AriadneAppState,
    settings: GeneralSectionSettings,
) -> CommandResult<GeneralSectionSettings> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "general_settings_update")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    crate::frontend::ProjectMemoryStore::validate_content(&settings.project_memory)
        .map_err(error_to_string)?;
    let config_store = ConfigStore::with_app_state(&project_root, state.app_state_root());
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    apply_app_settings(&mut config, settings.app.app)?;
    crate::config::ProjectLayout::from_app(&project_root, &config.app)
        .and_then(|layout| layout.create_configured_directories())
        .map_err(error_to_string)?;
    commit_general_settings_files(
        &project_root,
        state.app_state_root(),
        &config,
        settings.project_memory.as_bytes(),
    )?;
    Ok(GeneralSectionSettings {
        app: AppSettings { app: config.app },
        project_memory: settings.project_memory,
    })
}

pub fn get_rag_settings(state: &AriadneAppState) -> CommandResult<RagSettings> {
    let project_root = project_root_from_state(state, None)?;
    get_rag_settings_impl(&project_root)
}

pub fn save_rag_settings(
    state: &AriadneAppState,
    settings: RagSettings,
) -> CommandResult<RagSettings> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "rag_settings_update")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    settings.rag.validate().map_err(error_to_string)?;
    let expected = ConfigStore::new(&project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let mut candidate = expected.clone();
    candidate.rag = settings.rag;
    let saved = candidate.rag.clone();
    state.commit_retrieval_config(&expected, candidate)?;
    spawn_indexing_worker_for_state(state)?;
    Ok(RagSettings { rag: saved })
}

pub fn get_workflow_settings(state: &AriadneAppState) -> CommandResult<WorkflowSettings> {
    let project_root = project_root_from_state(state, None)?;
    get_workflow_settings_impl(&project_root)
}

pub fn save_workflow_settings(
    state: &AriadneAppState,
    settings: WorkflowSettings,
) -> CommandResult<WorkflowSettings> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "workflow_settings_update")?;
    save_workflow_settings_impl(&project_root, settings)?;
    get_workflow_settings_impl(&project_root)
}

pub fn get_git_settings(state: &AriadneAppState) -> CommandResult<GitSettings> {
    let project_root = project_root_from_state(state, None)?;
    get_git_settings_impl(&project_root)
}

pub fn save_git_settings(
    state: &AriadneAppState,
    settings: GitSettings,
) -> CommandResult<GitSettings> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "git_settings_update")?;
    save_git_settings_impl(&project_root, settings)?;
    get_git_settings_impl(&project_root)
}

pub fn save_misc_section_settings(
    state: &AriadneAppState,
    settings: MiscSectionSettings,
) -> CommandResult<MiscSectionSettings> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "misc_settings_update")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    settings.rag.rag.validate().map_err(error_to_string)?;
    let expected = ConfigStore::new(&project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let mut candidate = expected.clone();
    candidate.rag = settings.rag.rag;
    candidate.git = settings.git.git;
    candidate
        .git
        .normalize_ignored_paths()
        .map_err(error_to_string)?;
    candidate.validate().map_err(error_to_string)?;
    let saved = MiscSectionSettings {
        rag: RagSettings {
            rag: candidate.rag.clone(),
        },
        git: GitSettings {
            git: candidate.git.clone(),
        },
    };
    state.commit_retrieval_config(&expected, candidate)?;
    spawn_indexing_worker_for_state(state)?;
    Ok(saved)
}

pub fn get_template_repository_settings(
    state: &AriadneAppState,
) -> CommandResult<TemplateRepositorySettings> {
    get_template_repository_settings_impl(state.app_state_root())
}

pub fn save_template_repository_settings(
    state: &AriadneAppState,
    settings: TemplateRepositorySettings,
) -> CommandResult<TemplateRepositorySettings> {
    save_template_repository_settings_impl(state.app_state_root(), &settings)?;
    Ok(settings)
}

pub fn update_budget_config(
    state: &AriadneAppState,
    budget_usd: f64,
    preauthorized_usd: f64,
) -> CommandResult<BudgetStatus> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "budget_settings_update")?;
    update_budget_config_impl(&project_root, budget_usd, preauthorized_usd)?;
    get_budget_status_impl(&project_root)
}

pub fn set_auto_mode(state: &AriadneAppState, enabled: bool) -> CommandResult<()> {
    let project_root = project_root_from_state(state, None)?;
    set_auto_mode_impl(&project_root, enabled)
}

pub fn get_automation_settings(state: &AriadneAppState) -> CommandResult<AutomationSettings> {
    let project_root = project_root_from_state(state, None)?;
    get_automation_settings_impl(&project_root)
}

pub fn save_automation_settings(
    state: &AriadneAppState,
    settings: AutomationSettings,
) -> CommandResult<AutomationSettings> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "automation_settings_update")?;
    save_automation_settings_impl_with_app_state(&project_root, state.app_state_root(), settings)?;
    get_automation_settings_impl(&project_root)
}

pub fn save_automation_section_settings(
    state: &AriadneAppState,
    settings: AutomationSectionSettings,
) -> CommandResult<AutomationSectionSettings> {
    let project_root = project_root_from_state(state, None)?;
    let saved = settings.clone();
    save_automation_settings_impl_with_app_state_and_workflow(
        &project_root,
        state.app_state_root(),
        settings.automation,
        Some(settings.workflow.workflow),
    )?;
    Ok(saved)
}

pub fn get_permissions_settings(state: &AriadneAppState) -> CommandResult<PermissionsSettings> {
    let _global_settings = state.lock_global_settings()?;
    let active_project = active_project_for_global_settings(state)?;
    let _project_mutation = active_project
        .as_deref()
        .map(|root| acquire_project_mutation_guard(root, "permissions_settings_read"))
        .transpose()?;
    let permissions = AppPermissionsStore::read_global_or_migrate(
        state.app_state_root(),
        active_project.as_deref(),
    )
    .map_err(error_to_string)?;
    Ok(permissions_settings_from_config(permissions))
}

pub fn save_permissions_settings(
    state: &AriadneAppState,
    settings: PermissionsSettings,
) -> CommandResult<PermissionsSettings> {
    let _global_settings = state.lock_global_settings()?;
    let active_project = active_project_for_global_settings(state)?;
    let _project_mutation = active_project
        .as_deref()
        .map(|root| acquire_project_mutation_guard(root, "permissions_settings_update"))
        .transpose()?;
    let _provider_references = active_project
        .as_deref()
        .map(acquire_provider_reference_graph_guard)
        .transpose()?;
    let permissions = permissions_config_from_settings(settings)?;
    AppPermissionsStore::default_for_app(state.app_state_root())
        .write(&permissions)
        .map_err(error_to_string)?;
    Ok(permissions_settings_from_config(permissions))
}

fn active_project_for_global_settings(state: &AriadneAppState) -> CommandResult<Option<PathBuf>> {
    let project_root = state.project_root()?;
    if project_root.as_os_str().is_empty() {
        return Ok(None);
    }
    let project_root = canonicalize_initialized_project_root(&project_root)?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    Ok(Some(project_root))
}

pub fn get_node_preset_settings(state: &AriadneAppState) -> CommandResult<NodePresetSettings> {
    let project_root = project_root_from_state(state, None)?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    read_node_preset_settings_with_app_state(&project_root, state.app_state_root())
}

pub fn save_node_preset_settings(
    state: &AriadneAppState,
    settings: NodePresetSettings,
) -> CommandResult<NodePresetSettings> {
    let project_root = project_root_from_state(state, None)?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    let _global_settings = state.lock_global_settings()?;
    save_node_preset_settings_with_app_state(&project_root, state.app_state_root(), settings)
}

pub fn get_node_preset_settings_impl(project_root: &Path) -> CommandResult<NodePresetSettings> {
    read_node_preset_settings(project_root)
}

pub fn save_node_preset_settings_impl(
    project_root: &Path,
    settings: NodePresetSettings,
) -> CommandResult<NodePresetSettings> {
    let app_state_root = crate::config::trusted_app_state_for_project(project_root);
    save_node_preset_settings_with_app_state(project_root, &app_state_root, settings)
}

fn save_node_preset_settings_with_app_state(
    project_root: &Path,
    app_state_root: &Path,
    settings: NodePresetSettings,
) -> CommandResult<NodePresetSettings> {
    let _project_mutation = acquire_project_mutation_guard(project_root, "node_preset_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    write_node_preset_settings(project_root, app_state_root, &settings)?;
    Ok(settings)
}

pub fn fetch_provider_models(
    state: &AriadneAppState,
    provider_id: Option<String>,
) -> CommandResult<ProviderModelsResult> {
    fetch_provider_models_with_cancellation(
        state,
        provider_id,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn fetch_provider_models_with_cancellation(
    state: &AriadneAppState,
    provider_id: Option<String>,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<ProviderModelsResult> {
    let project_root = project_root_from_state(state, None)?;
    fetch_provider_models_with_secrets_and_cancellation_impl(
        &project_root,
        state.secret_store.as_ref(),
        provider_id,
        cancellation,
    )
}

pub fn fetch_provider_models_with_secrets_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    provider_id: Option<String>,
) -> CommandResult<ProviderModelsResult> {
    fetch_provider_models_with_secrets_and_cancellation_impl(
        project_root,
        secrets,
        provider_id,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn fetch_provider_models_with_secrets_and_cancellation_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    provider_id: Option<String>,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<ProviderModelsResult> {
    validate_project_root(project_root)?;
    cancellation.check().map_err(CommandError::from)?;
    ensure_project_not_in_maintenance(project_root)?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let selected = select_provider_for_model_fetch(&config.providers, provider_id)?.clone();
    let protocol = match ProviderProtocol::from_provider_type(&selected.provider_type) {
        Ok(protocol) => protocol,
        Err(_) => return configured_provider_models_result(&selected),
    };
    let api_key = provider_api_key(project_root, secrets, &selected)?;
    let fetched = fetch_remote_provider_models(&selected, protocol, api_key, cancellation)?;
    Ok(ProviderModelsResult {
        provider_id: selected.provider_id,
        models: merge_remote_model_metadata(fetched, &selected.models),
    })
}

pub fn fetch_provider_models_impl(
    project_root: &Path,
    provider_id: Option<String>,
) -> CommandResult<ProviderModelsResult> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "provider_model_catalog")?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let selected = select_provider_for_model_fetch(&config.providers, provider_id)?;
    configured_provider_models_result(selected)
}

fn configured_provider_models_result(
    selected: &ProviderConfig,
) -> CommandResult<ProviderModelsResult> {
    let mut models = selected.models.clone();
    if models.is_empty() {
        models.push(default_llm_model_for_provider(&selected.provider_id));
    }
    if !models
        .iter()
        .any(|model| model.capability == ProviderCapability::Embedding)
    {
        if let Some(model) = default_embedding_model_for_provider(&selected.provider_id) {
            models.push(model);
        }
    }

    Ok(ProviderModelsResult {
        provider_id: selected.provider_id.clone(),
        models,
    })
}

fn select_provider_for_model_fetch(
    providers: &crate::config::ProvidersConfig,
    provider_id: Option<String>,
) -> CommandResult<&ProviderConfig> {
    let requested = provider_id.as_deref().map(normalize_provider).transpose()?;
    if let Some(id) = requested {
        return providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == id)
            .ok_or_else(|| CommandError::not_found(format!("provider is not configured: {id}")));
    }
    providers
        .default_llm_provider_id
        .as_ref()
        .and_then(|id| {
            providers
                .providers
                .iter()
                .find(|provider| provider.provider_id == *id)
        })
        .or_else(|| providers.providers.iter().find(|provider| provider.enabled))
        .or_else(|| providers.providers.first())
        .ok_or_else(|| CommandError::not_found("no provider configured"))
}

fn provider_api_key(
    project_root: &Path,
    secrets: &dyn SecretStore,
    provider: &ProviderConfig,
) -> CommandResult<Option<String>> {
    if provider.api_key.is_some() {
        return Err(CommandError::permission(format!(
            "provider '{}' contains an untrusted project SecretRef; re-enter the credential before network access",
            provider.provider_id
        )));
    }
    ProjectCredentialScope::new(project_root, secrets)
        .map_err(error_to_string)?
        .get_provider_secret(&provider.provider_id)
        .map_err(error_to_string)
        .map(|secret| {
            secret
                .map(|value| value.expose_secret().trim().to_owned())
                .filter(|value| !value.is_empty())
        })
}

fn fetch_remote_provider_models(
    provider: &ProviderConfig,
    protocol: ProviderProtocol,
    api_key: Option<String>,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<Vec<ModelConfig>> {
    cancellation.check().map_err(CommandError::from)?;
    if provider_requires_api_key(&provider.provider_type) && api_key.is_none() {
        return Err(CommandError::validation(format!(
            "provider {} requires an API key before fetching models",
            provider.provider_id
        )));
    }

    let base_url = crate::providers::resolve_base_url(provider)
        .map_err(error_to_string)?
        .trim_end_matches('/')
        .to_owned();
    let client = Client::builder()
        .timeout(Duration::from_secs(PROVIDER_MODEL_FETCH_TIMEOUT_SECS))
        .build()
        .map_err(error_to_string)?;
    let request = match protocol {
        ProviderProtocol::OpenAi => client.get(format!("{base_url}/models")),
        ProviderProtocol::Anthropic => client
            .get(format!("{base_url}/models"))
            .header("anthropic-version", "2023-06-01"),
        ProviderProtocol::Gemini => client.get(format!("{base_url}/models")),
    };
    let request = match (protocol, api_key.as_deref()) {
        (ProviderProtocol::OpenAi, Some(key)) => request.bearer_auth(key),
        (ProviderProtocol::Anthropic, Some(key)) => request.header("x-api-key", key),
        (ProviderProtocol::Gemini, Some(key)) => request.query(&[("key", key)]),
        _ => request,
    };
    let provider_id = provider.provider_id.clone();
    let blocking_permit =
        crate::contracts::acquire_detached_blocking_task_permit().map_err(CommandError::from)?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("ariadne-provider-model-fetch".to_owned())
        .spawn(move || {
            let _blocking_permit = blocking_permit;
            let result = request
                .send()
                .map_err(|error| {
                    CommandError::network(format!(
                        "failed to fetch models from provider {provider_id}: {error}"
                    ))
                })
                .and_then(|response| {
                    let status = response.status();
                    read_provider_model_response_text(response, &provider_id)
                        .map(|text| (status, text))
                });
            let _ = sender.send(result);
        })
        .map_err(CommandError::from)?;
    let (status, text) = loop {
        if cancellation.is_cancelled() {
            return Err(CommandError::new(
                crate::command_error::CommandErrorCode::Cancelled,
                "provider model list request cancelled",
            ));
        }
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(result) => break result?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(CommandError::internal(
                    "provider model list worker disconnected",
                ));
            }
        }
    };
    if !status.is_success() {
        return Err(CommandError::external(format!(
            "provider {} model list request failed with HTTP {}: {}",
            provider.provider_id,
            status.as_u16(),
            truncate_provider_error(&text)
        )));
    }
    let raw: Value = serde_json::from_str(&text).map_err(|error| {
        CommandError::external(format!(
            "provider {} returned invalid model list JSON: {error}",
            provider.provider_id
        ))
    })?;
    parse_remote_provider_models(protocol, &raw)
}

fn read_provider_model_response_text(
    response: reqwest::blocking::Response,
    provider_id: &str,
) -> CommandResult<String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_MODEL_LIST_RESPONSE_BYTES)
    {
        return Err(CommandError::new(crate::command_error::CommandErrorCode::ResourceLimit, format!(
            "provider {provider_id} model list response exceeds {MAX_PROVIDER_MODEL_LIST_RESPONSE_BYTES} bytes"
        )));
    }

    let mut limited = response.take(MAX_PROVIDER_MODEL_LIST_RESPONSE_BYTES.saturating_add(1));
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).map_err(|error| {
        CommandError::network(format!(
            "failed to read model list from provider {provider_id}: {error}"
        ))
    })?;
    if bytes.len() as u64 > MAX_PROVIDER_MODEL_LIST_RESPONSE_BYTES {
        return Err(CommandError::new(crate::command_error::CommandErrorCode::ResourceLimit, format!(
            "provider {provider_id} model list response exceeds {MAX_PROVIDER_MODEL_LIST_RESPONSE_BYTES} bytes"
        )));
    }

    String::from_utf8(bytes).map_err(|error| {
        CommandError::external(format!(
            "provider {provider_id} returned non-UTF-8 model list: {error}"
        ))
    })
}

fn provider_requires_api_key(provider_type: &ProviderType) -> bool {
    matches!(
        provider_type,
        ProviderType::OpenAi | ProviderType::Anthropic | ProviderType::Gemini
    )
}

fn parse_remote_provider_models(
    protocol: ProviderProtocol,
    raw: &Value,
) -> CommandResult<Vec<ModelConfig>> {
    match protocol {
        ProviderProtocol::OpenAi | ProviderProtocol::Anthropic => parse_openai_style_models(raw),
        ProviderProtocol::Gemini => parse_gemini_models(raw),
    }
}

fn parse_openai_style_models(raw: &Value) -> CommandResult<Vec<ModelConfig>> {
    let data = raw.get("data").and_then(Value::as_array).ok_or_else(|| {
        CommandError::external("provider model list response must contain data[]")
    })?;
    let mut models = Vec::new();
    for item in data {
        if let Some(model_id) = item.get("id").and_then(Value::as_str) {
            models.push(remote_model_config(
                model_id,
                infer_text_model_capability(model_id),
            ));
        }
    }
    deduplicate_remote_models(models)
}

fn parse_gemini_models(raw: &Value) -> CommandResult<Vec<ModelConfig>> {
    let data = raw.get("models").and_then(Value::as_array).ok_or_else(|| {
        CommandError::external("gemini model list response must contain models[]")
    })?;
    let mut models = Vec::new();
    for item in data {
        let Some(raw_name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let model_id = raw_name.strip_prefix("models/").unwrap_or(raw_name);
        let methods = item
            .get("supportedGenerationMethods")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let capability = if methods
            .iter()
            .any(|method| method.eq_ignore_ascii_case("embedContent"))
            || infer_text_model_capability(model_id) == ProviderCapability::Embedding
        {
            ProviderCapability::Embedding
        } else {
            ProviderCapability::Llm
        };
        models.push(remote_model_config(model_id, capability));
    }
    deduplicate_remote_models(models)
}

fn remote_model_config(model_id: &str, capability: ProviderCapability) -> ModelConfig {
    ModelConfig {
        model_id: model_id.trim().to_owned(),
        capability,
        max_context_tokens: None,
        input_cost_per_million_tokens: None,
        output_cost_per_million_tokens: None,
    }
}

fn infer_text_model_capability(model_id: &str) -> ProviderCapability {
    let lower = model_id.to_ascii_lowercase();
    if lower.contains("embed") || lower.contains("embedding") || lower.contains("rerank") {
        if lower.contains("rerank") {
            ProviderCapability::Reranker
        } else {
            ProviderCapability::Embedding
        }
    } else {
        ProviderCapability::Llm
    }
}

fn deduplicate_remote_models(models: Vec<ModelConfig>) -> CommandResult<Vec<ModelConfig>> {
    let mut seen = HashSet::new();
    let mut deduplicated = Vec::new();
    for model in models {
        if model.model_id.trim().is_empty() || !seen.insert(model.model_id.clone()) {
            continue;
        }
        deduplicated.push(model);
    }
    if deduplicated.is_empty() {
        Err(CommandError::external("provider returned no usable models"))
    } else {
        Ok(deduplicated)
    }
}

fn merge_remote_model_metadata(
    remote_models: Vec<ModelConfig>,
    configured_models: &[ModelConfig],
) -> Vec<ModelConfig> {
    let mut configured_by_id = BTreeMap::new();
    for model in configured_models {
        configured_by_id.insert(model.model_id.as_str(), model);
    }

    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for remote_model in remote_models {
        if !seen.insert(remote_model.model_id.clone()) {
            continue;
        }
        if let Some(configured) = configured_by_id.get(remote_model.model_id.as_str()) {
            let mut model = (*configured).clone();
            model.capability = remote_model.capability;
            merged.push(model);
        } else {
            merged.push(remote_model);
        }
    }
    for configured in configured_models {
        if seen.insert(configured.model_id.clone()) {
            merged.push(configured.clone());
        }
    }
    merged
}

fn truncate_provider_error(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const LIMIT: usize = 400;
    if compact.chars().count() <= LIMIT {
        compact
    } else {
        format!("{}...", compact.chars().take(LIMIT).collect::<String>())
    }
}

pub fn list_confirmations(state: &AriadneAppState) -> CommandResult<Vec<ConfirmationLogEntry>> {
    let project_root = project_root_from_state(state, None)?;
    list_pending_confirmations_impl(&project_root)
}

pub fn get_confirmation(
    state: &AriadneAppState,
    confirmation_id: String,
) -> CommandResult<crate::frontend::ConfirmationReference> {
    let project_root = project_root_from_state(state, None)?;
    FileConfirmationLogStore::default_for_project(&project_root)
        .resolve_reference(&confirmation_id)
        .map_err(error_to_string)
}

pub fn resolve_confirmation(
    state: &AriadneAppState,
    request: ResolveConfirmationRequest,
) -> CommandResult<ResolveConfirmationResult> {
    let project_root = project_root_from_state(state, None)?;
    let project_mutation =
        acquire_project_mutation_guard(&project_root, "confirmation_resolution")?;
    let (result, lease) = resolve_confirmation_impl_with_claim(&project_root, request)?;
    if let Some(lease) = lease {
        spawn_continue_workflow_worker_with_lease(
            project_root,
            Arc::clone(&state.secret_store),
            Some(state.retrieval_runtime()?),
            result.workflow.workflow_id.clone(),
            result.workflow.run_id.clone(),
            lease,
            project_mutation,
        )?;
    }
    state.ensure_workflow_scheduler()?;
    Ok(result)
}

pub fn get_git_history(state: &AriadneAppState) -> CommandResult<Vec<GitCommitSummary>> {
    get_git_history_with_cancellation(state, &crate::contracts::ExecutionCancellation::new())
}

pub fn get_git_history_with_cancellation(
    state: &AriadneAppState,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<Vec<GitCommitSummary>> {
    let project_root = project_root_from_state(state, None)?;
    get_git_history_impl_with_cancellation(&project_root, cancellation)
}

pub fn get_git_repository_status(state: &AriadneAppState) -> CommandResult<GitRepositoryStatus> {
    get_git_repository_status_with_cancellation(
        state,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn get_git_repository_status_with_cancellation(
    state: &AriadneAppState,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<GitRepositoryStatus> {
    let project_root = project_root_from_state(state, None)?;
    get_git_repository_status_impl_with_cancellation(&project_root, cancellation)
}

pub fn get_git_branch_graph(
    state: &AriadneAppState,
    limit: Option<usize>,
) -> CommandResult<Vec<BranchGraphNode>> {
    get_git_branch_graph_with_cancellation(
        state,
        limit,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn get_git_branch_graph_with_cancellation(
    state: &AriadneAppState,
    limit: Option<usize>,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<Vec<BranchGraphNode>> {
    let project_root = project_root_from_state(state, None)?;
    git_service_with_cancellation(project_root, cancellation)
        .branch_graph(limit.unwrap_or(200))
        .map_err(error_to_string)
}

pub fn create_checkpoint(state: &AriadneAppState, message: String) -> CommandResult<ArchivePoint> {
    create_checkpoint_with_cancellation(
        state,
        message,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn create_checkpoint_with_cancellation(
    state: &AriadneAppState,
    message: String,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<ArchivePoint> {
    let project_root = project_root_from_state(state, None)?;
    create_checkpoint_impl_with_cancellation(&project_root, message, cancellation)
}

pub fn restore_to_new_branch(
    state: &AriadneAppState,
    commit_id: String,
    new_branch: String,
) -> CommandResult<RestoreReport> {
    restore_to_new_branch_with_cancellation(
        state,
        commit_id,
        new_branch,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn restore_to_new_branch_with_cancellation(
    state: &AriadneAppState,
    commit_id: String,
    new_branch: String,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<RestoreReport> {
    let _activation = state.lock_project_activation()?;
    let project_root = project_root_from_state(state, None)?;
    cancellation.check().map_err(CommandError::from)?;
    let documents = document_service(&project_root);
    let maintenance = documents.invalidation_outbox();
    maintenance
        .begin_maintenance("git_restore", "stopping_runtime")
        .map_err(error_to_string)?;
    let result: CommandResult<RestoreReport> = (|| -> CommandResult<RestoreReport> {
        cancellation.check().map_err(CommandError::from)?;
        let _maintenance_fence = drain_project_mutations_for_restore(&project_root, maintenance)?;
        maintenance
            .update_maintenance_phase("checking_out_branch")
            .map_err(error_to_string)?;
        let config = ConfigStore::new(&project_root)
            .load_or_create()
            .map_err(error_to_string)?;
        let policy = git_stage_policy_from_config(&config);
        let mut report = git_service_with_cancellation(&project_root, cancellation)
            .restore_to_new_branch_with_policy(&commit_id, &new_branch, &policy)
            .map_err(error_to_string)?;
        cancellation.check().map_err(CommandError::from)?;
        maintenance
            .update_maintenance_phase("rebuilding_full_text_indexes")
            .map_err(error_to_string)?;
        maintenance
            .enqueue(
                &project_root.to_string_lossy(),
                "git_restore_full_rebuild",
                &report.base_commit,
                true,
            )
            .map_err(error_to_string)?;
        state
            .reload_retrieval_runtime_for_project(&project_root)?
            .process_outbox_with_cancellation(cancellation)
            .map_err(error_to_string)?;
        report.index_rebuild_required = false;
        report.runtime_rebind_required = false;
        Ok(report)
    })();
    match result {
        Ok(report) => {
            maintenance
                .complete_maintenance("completed")
                .map_err(error_to_string)?;
            record_git_restore_log(&project_root, &report);
            Ok(report)
        }
        Err(error) => {
            let _ = maintenance.fail_maintenance("restore_incomplete", &error);
            Err(error)
        }
    }
}

pub fn get_provider_config(state: &AriadneAppState) -> CommandResult<ProviderConfigStatus> {
    let project_root = project_root_from_state(state, None)?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    get_provider_config_impl_with_app_state(
        &project_root,
        state.app_state_root(),
        state.secret_store.as_ref(),
    )
}

pub fn save_provider_key(
    state: &AriadneAppState,
    provider: String,
    key: String,
) -> CommandResult<ProviderConfigStatus> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "provider_key_update")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    let provider = normalize_provider(&provider)?;
    if key.trim().is_empty() {
        return Err(CommandError::validation("provider key cannot be empty"));
    }
    let config = ConfigStore::new(&project_root)
        .load()
        .map_err(error_to_string)?;
    if !config
        .providers
        .providers
        .iter()
        .any(|configured| configured.provider_id == provider)
    {
        return Err(CommandError::not_found(format!(
            "provider is not configured: {provider}"
        )));
    }
    let credentials = ProjectCredentialScope::new(&project_root, state.secret_store.as_ref())
        .map_err(error_to_string)?;
    let previous_secret = credentials
        .get_provider_secret(&provider)
        .map_err(error_to_string)?;
    credentials
        .set_provider_secret(&provider, SecretValue::new(key))
        .map_err(error_to_string)?;
    let status = match provider_config_status_from_config_with_app_state(
        &project_root,
        config,
        state.secret_store.as_ref(),
        state.app_state_root(),
    ) {
        Ok(status) => status,
        Err(error) => {
            restore_provider_secret(&credentials, &provider, previous_secret).map_err(
                |rollback| {
                    CommandError::internal(format!(
                        "{error}; provider key rollback failed: {rollback}"
                    ))
                },
            )?;
            return Err(error);
        }
    };
    match state.reload_retrieval_runtime() {
        Ok(_) => {
            spawn_indexing_worker_for_state(state)?;
            Ok(status)
        }
        Err(error) => {
            restore_provider_secret(&credentials, &provider, previous_secret).map_err(
                |rollback| {
                    CommandError::internal(format!(
                        "{error}; provider key rollback failed: {rollback}"
                    ))
                },
            )?;
            state.reload_retrieval_runtime().map_err(|rollback| {
                CommandError::internal(format!(
                    "{error}; provider runtime rollback failed: {rollback}"
                ))
            })?;
            Err(error)
        }
    }
}

/// 为尚不能正常打开的导入/旧项目显式重新绑定 Provider 凭据。
///
/// 该入口不把目标项目设为当前项目，也不会启动任何网络或后台任务。
pub fn rebind_project_provider_key(
    state: &AriadneAppState,
    project_root: String,
    provider: String,
    key: String,
) -> CommandResult<()> {
    let project_root = PathBuf::from(project_root);
    validate_initialized_project_root(&project_root)?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    save_provider_key_impl(&project_root, state.secret_store.as_ref(), provider, key)
}

pub fn save_provider_settings(
    state: &AriadneAppState,
    update: ProviderSettingsUpdate,
) -> CommandResult<ProviderConfigStatus> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "provider_settings_update")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    let _global_settings = state.lock_global_settings()?;
    save_global_provider_settings_under_guards(state, &project_root, update)
}

pub fn save_provider_section_settings(
    state: &AriadneAppState,
    settings: ProviderSectionSettings,
) -> CommandResult<ProviderConfigStatus> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "provider_section_update")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    let _global_settings = state.lock_global_settings()?;
    let provider_id = normalize_provider(&settings.provider.provider_id)?;

    let credentials = ProjectCredentialScope::new(&project_root, state.secret_store.as_ref())
        .map_err(error_to_string)?;
    let previous_secret = if let Some(key) = settings.api_key.as_deref() {
        if key.trim().is_empty() {
            return Err(CommandError::validation("provider key cannot be empty"));
        }
        let previous = credentials
            .get_provider_secret(&provider_id)
            .map_err(error_to_string)?;
        credentials
            .set_provider_secret(&provider_id, SecretValue::new(key.to_owned()))
            .map_err(error_to_string)?;
        Some(previous)
    } else {
        None
    };

    let status = match save_global_provider_settings_under_guards(
        state,
        &project_root,
        settings.provider,
    ) {
        Ok(status) => status,
        Err(error) => {
            if let Some(previous) = previous_secret {
                restore_provider_secret(&credentials, &provider_id, previous).map_err(
                    |rollback| {
                        CommandError::internal(format!(
                            "provider status prepare failed: {error}; key rollback failed: {rollback}"
                        ))
                    },
                )?;
            }
            return Err(error);
        }
    };
    Ok(status)
}

fn save_global_provider_settings_under_guards(
    state: &AriadneAppState,
    project_root: &Path,
    update: ProviderSettingsUpdate,
) -> CommandResult<ProviderConfigStatus> {
    crate::config::bind_project_app_state(project_root, state.app_state_root())
        .map_err(error_to_string)?;
    let global_profile = provider_config_from_update(&update)?;
    let catalog_store = ProviderCatalogStore::default_for_app(state.app_state_root());
    // 全局目录读改写、项目提交与失败回滚共享同一把跨进程锁，避免多实例丢更新。
    let _catalog_lock = catalog_store.lock_exclusive().map_err(error_to_string)?;
    let previous_catalog = catalog_store.read_unlocked().map_err(error_to_string)?;
    let mut next_catalog = previous_catalog.clone();
    next_catalog
        .upsert(global_profile)
        .map_err(error_to_string)?;
    catalog_store
        .write_unlocked(&next_catalog)
        .map_err(error_to_string)?;

    let result = (|| -> CommandResult<ProviderConfigStatus> {
        // 全局目录已切换后重新读取 expected，避免把目录更新误判为项目并发修改。
        let expected = ConfigStore::with_app_state(project_root, state.app_state_root())
            .load_or_create()
            .map_err(error_to_string)?;
        let mut candidate = expected.clone();
        apply_provider_settings_update(&mut candidate, update)?;
        let status = provider_config_status_from_config_with_app_state(
            project_root,
            candidate.clone(),
            state.secret_store.as_ref(),
            state.app_state_root(),
        )?;
        state.commit_retrieval_config(&expected, candidate)?;
        spawn_indexing_worker_for_state(state)?;
        Ok(status)
    })();

    match result {
        Ok(status) => Ok(status),
        Err(error) => {
            let catalog_rollback = catalog_store
                .write_unlocked(&previous_catalog)
                .map_err(error_to_string);
            let runtime_rollback = state.reload_retrieval_runtime().map(|_| ());
            match (catalog_rollback, runtime_rollback) {
                (Ok(()), Ok(())) => Err(error),
                (catalog_result, runtime_result) => Err(CommandError::internal(format!(
                    "global provider update failed: {error}; catalog rollback: {}; runtime rollback: {}",
                    rollback_result_text(catalog_result),
                    rollback_result_text(runtime_result)
                ))),
            }
        }
    }
}

pub fn preview_provider_removal(
    state: &AriadneAppState,
    provider: String,
) -> CommandResult<ProviderRemovalPreview> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "provider_removal_preview")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    let provider = normalize_provider(&provider)?;
    let config = ConfigStore::new(&project_root)
        .load()
        .map_err(error_to_string)?;
    provider_removal_preview_from_config(
        &project_root,
        state.secret_store.as_ref(),
        &config,
        &provider,
    )
}

pub fn remove_provider(
    state: &AriadneAppState,
    provider: String,
    expected_revision: String,
) -> CommandResult<ProviderConfigStatus> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "provider_removal")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    let provider = normalize_provider(&provider)?;
    let expected = ConfigStore::new(&project_root)
        .load()
        .map_err(error_to_string)?;
    let preview = provider_removal_preview_from_config(
        &project_root,
        state.secret_store.as_ref(),
        &expected,
        &provider,
    )?;
    if preview.revision != expected_revision {
        return Err(CommandError::conflict(
            "provider removal impact changed; preview and confirm again",
        ));
    }
    if !preview.blocking_references.is_empty() {
        return Err(CommandError::conflict(
            "provider is still referenced; remove the references before deleting it",
        ));
    }

    let mut candidate = expected.clone();
    candidate
        .providers
        .providers
        .retain(|configured| configured.provider_id != provider);
    candidate
        .providers
        .authorized_provider_ids
        .remove(&provider);
    clear_provider_defaults(&mut candidate, &provider);

    let credentials = ProjectCredentialScope::new(&project_root, state.secret_store.as_ref())
        .map_err(error_to_string)?;
    let previous_secret = credentials
        .get_provider_secret(&provider)
        .map_err(error_to_string)?;
    let mut status = provider_config_status_from_config_with_app_state(
        &project_root,
        candidate.clone(),
        state.secret_store.as_ref(),
        state.app_state_root(),
    )?;
    clear_removed_provider_key_status(&mut status, &provider);

    let runtime = state.commit_retrieval_config(&expected, candidate.clone())?;
    if let Err(error) = credentials.delete_provider_secret(&provider) {
        let secret_rollback =
            restore_provider_secret(&credentials, &provider, previous_secret.clone());
        let config_rollback = state.commit_retrieval_config(&candidate, expected);
        return Err(match (secret_rollback, config_rollback) {
            (Ok(()), Ok(_)) => error_to_string(error),
            (secret_result, config_result) => CommandError::internal(format!(
                "provider credential deletion failed: {error}; secret rollback: {}; config/runtime rollback: {}",
                rollback_result_text(secret_result),
                rollback_result_text(config_result.map(|_| ()))
            )),
        });
    }

    if let Some(worker_key) = register_indexing_worker(&project_root) {
        spawn_registered_indexing_worker_with_runtime(project_root, worker_key, runtime);
    }
    Ok(status)
}

pub fn query_run_logs(
    state: &AriadneAppState,
    filter: Option<RunLogQuery>,
) -> CommandResult<Vec<UiRunLogEntry>> {
    let project_root = project_root_from_state(state, None)?;
    let filter = filter.unwrap_or_default();
    UiRunLogStore::default_for_project(project_root)
        .query(UiRunLogFilter {
            kind: filter.kind,
            level: filter.level,
            workflow_id: filter.workflow_id.map(WorkflowId::from),
            run_id: filter.run_id.map(RunId::from),
            node_id: filter.node_id.map(NodeId::from),
            query: filter.query,
            after_timestamp_ms: filter.after_timestamp_ms,
            after_log_id: filter.after_log_id,
            limit: filter.limit,
            descending: filter.descending,
        })
        .map_err(error_to_string)
}

pub fn mark_run_logs_read(
    state: &AriadneAppState,
    filter: Option<RunLogQuery>,
) -> CommandResult<usize> {
    let project_root = project_root_from_state(state, None)?;
    let filter = filter.unwrap_or_default();
    UiRunLogStore::default_for_project(project_root)
        .mark_read(UiRunLogFilter {
            kind: filter.kind,
            level: filter.level,
            workflow_id: filter.workflow_id.map(WorkflowId::from),
            run_id: filter.run_id.map(RunId::from),
            node_id: filter.node_id.map(NodeId::from),
            query: filter.query,
            after_timestamp_ms: None,
            after_log_id: None,
            limit: None,
            descending: false,
        })
        .map_err(error_to_string)
}

pub fn read_project_memory(state: &AriadneAppState) -> CommandResult<String> {
    let project_root = project_root_from_state(state, None)?;
    ProjectMemoryStore::default_for_project(project_root)
        .read_all()
        .map_err(error_to_string)
}

pub fn append_project_memory(state: &AriadneAppState, content: String) -> CommandResult<String> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "project_memory_append")?;
    ProjectMemoryStore::default_for_project(project_root)
        .append(&content)
        .map_err(error_to_string)
}

pub fn write_project_memory(state: &AriadneAppState, content: String) -> CommandResult<()> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "project_memory_write")?;
    ProjectMemoryStore::default_for_project(project_root)
        .write_all(&content)
        .map_err(error_to_string)
}

pub fn quick_edit(
    state: &AriadneAppState,
    request: QuickEditRequest,
) -> CommandResult<QuickEditResult> {
    quick_edit_with_cancellation(
        state,
        request,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn quick_edit_with_cancellation(
    state: &AriadneAppState,
    request: QuickEditRequest,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<QuickEditResult> {
    let project_root = project_root_from_state(state, None)?;
    quick_edit_impl_with_cancellation(
        &project_root,
        state.secret_store.as_ref(),
        request,
        cancellation,
    )
}

pub fn apply_quick_edit(
    state: &AriadneAppState,
    document_id: String,
    base_version: Option<String>,
    text: String,
    range: crate::contracts::TextRange,
    result: QuickEditResult,
) -> CommandResult<crate::documents::PatchApplyReport> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "quick_edit_apply")?;
    let documents = document_service(&project_root);
    let report = crate::frontend::apply_quick_edit_patch(
        &documents,
        &document_id,
        base_version,
        &text,
        range,
        &result,
    )
    .map_err(error_to_string)?;
    spawn_indexing_worker_for_state(state)?;
    Ok(report)
}

pub fn project_ai_chat(
    state: &AriadneAppState,
    request: ProjectAiRequest,
) -> CommandResult<ProjectAiResponse> {
    project_ai_chat_with_cancellation(
        state,
        request,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn project_ai_chat_with_cancellation(
    state: &AriadneAppState,
    request: ProjectAiRequest,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<ProjectAiResponse> {
    let project_root = project_root_from_state(state, None)?;
    let runner_root = project_root.clone();
    let runner_secrets = Arc::clone(&state.secret_store);
    let runner_retrieval = state.retrieval_runtime()?;
    let project_ai_retrieval = Arc::clone(&runner_retrieval);
    let scheduler_state = state.clone();
    let response = project_ai_chat_with_runner(
        &project_root,
        state.secret_store.as_ref(),
        request,
        project_ai_retrieval,
        cancellation,
        &mut move |request| {
            let started = start_workflow_request(
                &runner_root,
                Arc::clone(&runner_secrets),
                Some(Arc::clone(&runner_retrieval)),
                request,
            )?;
            scheduler_state.ensure_workflow_scheduler()?;
            Ok(started)
        },
    )?;
    state.ensure_workflow_scheduler()?;
    Ok(response)
}

pub fn resolve_project_reference(
    state: &AriadneAppState,
    reference: String,
) -> CommandResult<ProjectReference> {
    let project_root = project_root_from_state(state, None)?;
    resolve_project_references_with_context(&project_root, &[reference], "", None, None)?
        .into_iter()
        .next()
        .ok_or_else(|| CommandError::validation("project reference is required"))
}

pub fn get_ui_preferences(state: &AriadneAppState) -> CommandResult<UiPreferences> {
    // 与项目解耦：应用级偏好，无项目也可读写
    let project = project_root_from_state(state, None).ok();
    UiPreferencesStore::read_global_or_migrate(state.app_state_root(), project.as_deref())
        .map_err(error_to_string)
}

pub fn save_ui_preferences(
    state: &AriadneAppState,
    preferences: UiPreferences,
) -> CommandResult<()> {
    UiPreferencesStore::default_for_app(state.app_state_root())
        .write(&preferences)
        .map_err(error_to_string)
}

pub fn search_templates(
    request: TemplateRepositoryRequest,
    query: String,
    tags: Vec<String>,
    page: u32,
) -> CommandResult<Vec<TemplateSummary>> {
    search_templates_with_cancellation(
        request,
        query,
        tags,
        page,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn search_templates_with_cancellation(
    request: TemplateRepositoryRequest,
    query: String,
    tags: Vec<String>,
    page: u32,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<Vec<TemplateSummary>> {
    template_client(request, cancellation)?
        .search(&query, &tags, page)
        .map_err(error_to_string)
}

pub fn get_template_detail(
    request: TemplateRepositoryRequest,
    id: String,
) -> CommandResult<TemplateDetail> {
    get_template_detail_with_cancellation(
        request,
        id,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn get_template_detail_with_cancellation(
    request: TemplateRepositoryRequest,
    id: String,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<TemplateDetail> {
    template_client(request, cancellation)?
        .detail(&id)
        .map_err(error_to_string)
}

pub fn install_template(
    state: &AriadneAppState,
    request: TemplateRepositoryRequest,
    id: String,
) -> CommandResult<TemplateInstallReport> {
    install_template_with_cancellation(
        state,
        request,
        id,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn install_template_with_cancellation(
    state: &AriadneAppState,
    request: TemplateRepositoryRequest,
    id: String,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<TemplateInstallReport> {
    let expected_project_root =
        canonicalize_initialized_project_root(&project_root_from_state(state, None)?)?;
    install_template_for_active_project(state, request, id, &expected_project_root, cancellation)
}

pub fn install_template_for_project_with_cancellation(
    state: &AriadneAppState,
    request: TemplateRepositoryRequest,
    id: String,
    expected_project_root: String,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<TemplateInstallReport> {
    let expected_project_root =
        canonicalize_initialized_project_root(Path::new(expected_project_root.trim()))?;
    install_template_for_active_project(state, request, id, &expected_project_root, cancellation)
}

fn install_template_for_active_project(
    state: &AriadneAppState,
    request: TemplateRepositoryRequest,
    id: String,
    expected_project_root: &Path,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<TemplateInstallReport> {
    ensure_active_project_identity(state, expected_project_root, "before template download")?;
    let manifest = template_client(request, cancellation)?
        .download(&id)
        .map_err(error_to_string)?;
    cancellation.check().map_err(CommandError::from)?;

    // 项目切换也持有此锁。下载只产生候选 manifest；从身份重验到画布提交完成，
    // 当前项目不能切换，避免把迟到下载写回已经离开的项目。
    let _activation = state.lock_project_activation()?;
    cancellation.check().map_err(CommandError::from)?;
    ensure_active_project_identity(state, expected_project_root, "at template commit")?;
    let _project_mutation =
        acquire_project_mutation_guard(expected_project_root, "template_install")?;
    let _provider_references = acquire_provider_reference_graph_guard(expected_project_root)?;
    let layout = project_layout(expected_project_root)?;
    let report =
        crate::frontend::install_workflow_template_manifest(manifest, layout.workflows, false)
            .map_err(error_to_string)?;

    let raw = std::fs::read(&report.manifest_path).map_err(error_to_string)?;
    let content = std::str::from_utf8(&raw).map_err(|error| {
        CommandError::serialization(format!("template workflow is not valid UTF-8: {error}"))
    })?;
    let source = parse_workflow_file(content)?;
    source.validate_topology().map_err(error_to_string)?;
    let (mut canvas, canvas_revision) =
        load_workflow_definition_with_revision(expected_project_root, None)?;
    normalize_project_canvas_identity(&mut canvas);
    if merge_workflow_into_project_canvas(&mut canvas, source, content_revision_hash(&raw)) {
        validate_workflow_execution_contracts(&canvas).map_err(error_to_string)?;
        let mut graph = workflow_to_graph(canvas);
        graph.expected_revision = (!canvas_revision.is_empty()).then_some(canvas_revision);
        save_workflow_graph_locked(expected_project_root, graph)?;
    }
    Ok(report)
}

fn ensure_active_project_identity(
    state: &AriadneAppState,
    expected_project_root: &Path,
    boundary: &str,
) -> CommandResult<()> {
    let active_project_root =
        canonicalize_initialized_project_root(&project_root_from_state(state, None)?)?;
    if active_project_root == expected_project_root {
        return Ok(());
    }
    Err(CommandError::conflict(format!(
        "active project changed {boundary}"
    )))
}

pub fn get_backend_diagnostics(state: &AriadneAppState) -> CommandResult<BackendDiagnosticsReport> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "backend_diagnostics")?;
    let config = ConfigStore::new(&project_root).load_or_create();
    let retrieval_reports = match state.retrieval_runtime() {
        Ok(runtime) => runtime.health_check().unwrap_or_else(|error| {
            vec![crate::retrieval::StoreHealth::unavailable(
                "project_retrieval_runtime",
                error.to_string(),
            )]
        }),
        Err(error) => vec![crate::retrieval::StoreHealth::unavailable(
            "project_retrieval_runtime",
            error.to_string(),
        )],
    };
    let mut report = BackendDiagnosticsReport::collect(
        SqliteWorkflowRuntimeStore::health(&project_root),
        None,
        Vec::new(),
        retrieval_reports,
    );
    match config {
        Ok(config) => report.extend_items(provider_config_diagnostic_items(
            &config.providers,
            &config.rag,
        )),
        Err(error) => report.extend_items([DiagnosticItem {
            component: "project.config".to_owned(),
            status: DiagnosticStatus::Unavailable,
            reason: Some(error.to_string()),
        }]),
    }
    Ok(report)
}

fn provider_config_diagnostic_items(
    providers: &crate::config::ProvidersConfig,
    rag: &RagConfig,
) -> Vec<DiagnosticItem> {
    let mut items = Vec::new();
    match providers.validate() {
        Ok(()) => items.push(DiagnosticItem {
            component: "providers.config".to_owned(),
            status: DiagnosticStatus::Healthy,
            reason: None,
        }),
        Err(error) => items.push(DiagnosticItem {
            component: "providers.config".to_owned(),
            status: DiagnosticStatus::Unavailable,
            reason: Some(error.to_string()),
        }),
    }

    match select_llm_provider(providers).and_then(|provider| select_llm_model(&provider)) {
        Ok(model) => items.push(DiagnosticItem {
            component: "providers.llm.default".to_owned(),
            status: DiagnosticStatus::Healthy,
            reason: Some(format!("default LLM model: {}", model.model_id)),
        }),
        Err(reason) => items.push(DiagnosticItem {
            component: "providers.llm.default".to_owned(),
            status: DiagnosticStatus::Degraded,
            reason: Some(reason.to_string()),
        }),
    }

    items.push(provider_capability_config_diagnostic_item(
        providers,
        providers.default_embedding_provider_id.as_deref(),
        ProviderCapability::Embedding,
        "embedding",
        rag.vector_store.enabled,
    ));
    items.push(provider_capability_config_diagnostic_item(
        providers,
        providers.default_reranker_provider_id.as_deref(),
        ProviderCapability::Reranker,
        "reranker",
        rag.reranker_enabled,
    ));
    items
}

/// 配置存在只证明可构造条件之一；secret、endpoint 与响应合同由正式运行时健康项证明。
fn provider_capability_config_diagnostic_item(
    providers: &crate::config::ProvidersConfig,
    default_provider_id: Option<&str>,
    capability: ProviderCapability,
    label: &str,
    enabled: bool,
) -> DiagnosticItem {
    let component = format!("providers.{label}.default");
    if !enabled {
        return DiagnosticItem {
            component,
            status: DiagnosticStatus::Healthy,
            reason: Some(format!("diagnostics.providers.{label}.disabled")),
        };
    }
    let Some(provider_id) = default_provider_id else {
        return DiagnosticItem {
            component,
            status: DiagnosticStatus::Degraded,
            reason: Some(format!("diagnostics.providers.{label}.default_missing")),
        };
    };
    let Some(provider) = providers
        .providers
        .iter()
        .find(|provider| provider.provider_id == provider_id)
    else {
        return DiagnosticItem {
            component,
            status: DiagnosticStatus::Unavailable,
            reason: Some(format!(
                "diagnostics.providers.{label}.provider_unavailable"
            )),
        };
    };
    if !provider.enabled {
        return DiagnosticItem {
            component,
            status: DiagnosticStatus::Unavailable,
            reason: Some(format!(
                "diagnostics.providers.{label}.provider_unavailable"
            )),
        };
    }
    if !provider
        .models
        .iter()
        .any(|model| model.capability == capability)
    {
        return DiagnosticItem {
            component,
            status: DiagnosticStatus::Unavailable,
            reason: Some(format!("diagnostics.providers.{label}.model_missing")),
        };
    }
    DiagnosticItem {
        component,
        status: DiagnosticStatus::Degraded,
        reason: Some(format!(
            "diagnostics.providers.{label}.configured_unverified"
        )),
    }
}

pub fn default_project_root() -> PathBuf {
    std::env::var_os(DEFAULT_PROJECT_ENV)
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn default_app_state_root() -> PathBuf {
    crate::config::default_app_state_root()
}

#[cfg(feature = "system-keychain")]
pub fn default_secret_store() -> Arc<dyn SecretStore> {
    Arc::new(SystemKeychainSecretStore::default())
}

#[cfg(not(feature = "system-keychain"))]
pub fn default_secret_store() -> Arc<dyn SecretStore> {
    Arc::new(LocalFileSecretStore::new(
        default_app_state_root().join("secrets.json"),
    ))
}

pub fn get_document_tree_impl(project_root: &Path) -> CommandResult<DocumentTreeNode> {
    validate_project_root(project_root)?;
    let layout = project_layout(project_root)?;
    let roots = [
        project_root.join("planning"),
        layout.documents,
        layout.workflows,
    ];
    let mut children = Vec::new();
    for root in roots {
        if root.exists() {
            children.push(scan_tree(project_root, &root)?);
        }
    }
    Ok(DocumentTreeNode {
        id: "project".to_owned(),
        name: project_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .to_owned(),
        path: project_root.to_path_buf(),
        kind: DocumentTreeNodeKind::Directory,
        children,
    })
}

pub fn get_document_content_impl(
    project_root: &Path,
    document_id: Option<String>,
    path: Option<String>,
) -> CommandResult<String> {
    get_document_content_details_impl(project_root, document_id, path)
        .map(|document| document.content)
}

pub fn get_document_content_details_impl(
    project_root: &Path,
    document_id: Option<String>,
    path: Option<String>,
) -> CommandResult<DocumentContent> {
    let document_path = document_argument_path(project_root, document_id, path)?;
    let documents = document_service(project_root);
    documents
        .open_document(DocumentReadRequest {
            path: document_path,
            format: None,
        })
        .map_err(error_to_string)
}

pub fn save_document_content_impl(
    project_root: &Path,
    document_id: String,
    content: String,
) -> CommandResult<()> {
    save_document_content_report_impl(project_root, document_id, content, None).map(|_| ())
}

pub fn save_document_content_report_impl(
    project_root: &Path,
    document_id: String,
    content: String,
    base_version: Option<String>,
) -> CommandResult<DocumentWriteReport> {
    ensure_project_not_in_maintenance(project_root)?;
    let document_path = project_path(project_root, &document_id)?;
    let documents = document_service(project_root);
    let report = documents
        .save_document(DocumentWriteRequest {
            path: document_path,
            content,
            format: None,
            base_version,
        })
        .map_err(error_to_string)?;
    Ok(report)
}

fn ensure_project_not_in_maintenance(project_root: &Path) -> CommandResult<()> {
    document_service(project_root)
        .invalidation_outbox()
        .ensure_available()
        .map_err(error_to_string)
}

fn acquire_project_mutation_guard(
    project_root: &Path,
    kind: &str,
) -> CommandResult<Arc<ProjectMutationGuard>> {
    document_service(project_root)
        .invalidation_outbox()
        .acquire_project_mutation(kind)
        .map(Arc::new)
        .map_err(error_to_string)
}

fn acquire_provider_reference_graph_guard(
    project_root: &Path,
) -> CommandResult<crate::config::store::PathWriteLock> {
    crate::config::store::PathWriteLock::acquire(
        &project_root
            .join(".config")
            .join(PROVIDER_REFERENCE_GRAPH_LOCK),
    )
    .map_err(error_to_string)
}

fn drain_project_mutations_for_restore(
    project_root: &Path,
    maintenance: &IndexInvalidationOutbox,
) -> CommandResult<ProjectMaintenanceGuard> {
    let runtime_path = project_root.join(crate::workflow::RUNTIME_DB_FILE);
    let mut runtime_store = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if runtime_store.is_none() && runtime_path.exists() {
            runtime_store =
                Some(SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?);
        }
        if let Some(store) = &runtime_store {
            store
                .stop_non_terminal_for_restore("stopped for Git restore maintenance")
                .map_err(error_to_string)?;
        }
        if let Some(guard) = maintenance
            .try_acquire_maintenance_fence()
            .map_err(error_to_string)?
        {
            if runtime_store.is_none() && runtime_path.exists() {
                runtime_store =
                    Some(SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?);
            }
            if let Some(store) = &runtime_store {
                store
                    .stop_non_terminal_for_restore("stopped for Git restore maintenance")
                    .map_err(error_to_string)?;
            }
            return Ok(guard);
        }
        if Instant::now() >= deadline {
            return Err(CommandError::conflict(
                "timed out draining project workflow workers before Git restore",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// F2-a：打开/切根时若存在可索引源且全文索引为空，幂等入队 full rebuild。
/// 与 Git restore 共用 `full_rebuild_required` 事件形状；不替代 worker 消费。
pub fn ensure_index_bootstrap_on_open(project_root: &Path) -> CommandResult<Option<String>> {
    validate_initialized_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "index_bootstrap")?;
    let _config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let sqlite_path = project_root.join(".indexes").join("full_text.db");
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    let documents = document_service(project_root);
    crate::retrieval::enqueue_open_bootstrap_full_rebuild(
        project_root,
        &sqlite_path,
        documents.invalidation_outbox(),
    )
    .map_err(error_to_string)
}

/// F2-b 产品搜索入口：混合索引 + outbox 门禁 + source_version 过滤。
pub fn search_project_documents_impl(
    project_root: &Path,
    query: String,
    limit: usize,
) -> CommandResult<Vec<RetrievalResult>> {
    validate_initialized_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "project_search")?;
    let secrets = crate::config::MemorySecretStore::default();
    let runtime = ProjectRetrievalRuntime::open(project_root, &secrets).map_err(error_to_string)?;
    runtime
        .search(
            query,
            limit,
            crate::providers::ProviderCallContext::new("project_retrieval"),
        )
        .map_err(error_to_string)
}

/// 产品 IPC 搜索入口；复用 AriadneAppState 持有的项目级运行时与真实凭据。
pub fn search_project_documents(
    state: &AriadneAppState,
    query: String,
    limit: usize,
) -> CommandResult<Vec<RetrievalResult>> {
    search_project_documents_with_cancellation(
        state,
        query,
        limit,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn search_project_documents_with_cancellation(
    state: &AriadneAppState,
    query: String,
    limit: usize,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<Vec<RetrievalResult>> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "project_search")?;
    let mut context = crate::providers::ProviderCallContext::new("project_retrieval");
    context.cancellation = cancellation.clone();
    state
        .retrieval_runtime()?
        .search(query, limit, context)
        .map_err(error_to_string)
}

/// 同步消费当前项目的索引 outbox，供后台线程、诊断恢复和契约测试复用。
pub fn process_index_outbox_impl(project_root: &Path) -> CommandResult<usize> {
    let _project_mutation = acquire_project_mutation_guard(project_root, "indexing_worker")?;
    process_index_outbox_unfenced_impl(project_root)
}

fn process_index_outbox_unfenced_impl(project_root: &Path) -> CommandResult<usize> {
    validate_initialized_project_root(project_root)?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    config.rag.validate().map_err(error_to_string)?;
    if config.rag.vector_store.enabled {
        return Err(CommandError::validation(
            "vector retrieval is enabled; product indexing requires AriadneAppState credentials",
        ));
    }
    // 无 state 的测试/管理入口只允许显式全文-only 项目，禁止向量配置下悄悄漏写。
    let runtime =
        ProjectRetrievalRuntime::open(project_root, &crate::config::MemorySecretStore::default())
            .map_err(error_to_string)?;
    let worker = runtime.indexing_worker().map_err(error_to_string)?;
    let mut processed = 0usize;
    let mut first_error = None;
    loop {
        match worker.process_next() {
            Ok(Some(_)) => processed = processed.saturating_add(1),
            Ok(None) => break,
            Err(error) => {
                first_error.get_or_insert_with(|| error_to_string(error));
            }
        }
    }
    // D2：任一条失败不得伪装为整批成功（即使已处理部分事件）。
    if let Some(error) = first_error {
        return Err(if processed == 0 {
            error
        } else {
            CommandError::internal(format!(
                "index outbox partial failure after {processed} event(s): {error}"
            ))
        });
    }
    Ok(processed)
}

fn spawn_indexing_worker_for_state(state: &AriadneAppState) -> CommandResult<()> {
    let project_root = state.project_root()?;
    let runtime = state.retrieval_runtime()?;
    let Some(worker_key) = register_indexing_worker(&project_root) else {
        return Ok(());
    };
    spawn_registered_indexing_worker_with_runtime(project_root, worker_key, runtime);
    Ok(())
}

fn resume_indexing_worker_for_project(
    project_root: &Path,
    runtime: Arc<ProjectRetrievalRuntime>,
) -> CommandResult<()> {
    let Some(worker_key) = register_indexing_worker(project_root) else {
        return Ok(());
    };
    let documents = document_service(project_root);
    let project_mutation = match acquire_project_mutation_guard(project_root, "indexing_recovery") {
        Ok(project_mutation) => project_mutation,
        Err(error) => {
            unregister_indexing_worker(&worker_key);
            return Err(error);
        }
    };
    if let Err(error) = documents.invalidation_outbox().requeue_interrupted() {
        unregister_indexing_worker(&worker_key);
        return Err(error_to_string(error));
    }
    drop(project_mutation);
    spawn_registered_indexing_worker_with_runtime(project_root.to_path_buf(), worker_key, runtime);
    Ok(())
}

fn spawn_registered_indexing_worker_with_runtime(
    project_root: PathBuf,
    worker_key: PathBuf,
    runtime: Arc<ProjectRetrievalRuntime>,
) {
    let thread_root = project_root.clone();
    let thread_key = worker_key.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("ariadne-indexing-worker".to_owned())
        .spawn(move || {
            let _guard = IndexingWorkerGuard(thread_key);
            let _project_mutation =
                match acquire_project_mutation_guard(&thread_root, "indexing_worker") {
                    Ok(project_mutation) => project_mutation,
                    Err(error) => {
                        eprintln!("[ariadne] indexing worker blocked: {error}");
                        return;
                    }
                };
            if let Err(error) = runtime.process_outbox() {
                let error = error_to_string(error);
                record_workflow_worker_error(
                    &thread_root,
                    "indexing",
                    "indexing-worker",
                    "indexing worker failed",
                    &error,
                    None,
                );
                eprintln!("[ariadne] indexing worker failed: {error}");
            }
        })
    {
        unregister_indexing_worker(&worker_key);
        eprintln!("[ariadne] failed to spawn indexing worker: {error}");
    }
}

fn register_indexing_worker(project_root: &Path) -> Option<PathBuf> {
    let worker_key = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut active = active_indexing_workers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    active.insert(worker_key.clone()).then_some(worker_key)
}

fn unregister_indexing_worker(worker_key: &Path) {
    active_indexing_workers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(worker_key);
}

fn active_indexing_workers() -> &'static Mutex<HashSet<PathBuf>> {
    static ACTIVE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

struct IndexingWorkerGuard(PathBuf);

impl Drop for IndexingWorkerGuard {
    fn drop(&mut self) {
        unregister_indexing_worker(&self.0);
    }
}

pub fn load_workflow_graph_impl(
    project_root: &Path,
    workflow_id: Option<String>,
) -> CommandResult<WorkflowGraphData> {
    let (workflow, revision) = load_workflow_definition_with_revision(project_root, workflow_id)?;
    let mut graph = workflow_to_graph(workflow);
    graph.content_revision = Some(revision);
    Ok(graph)
}

/// 桌面工作区的唯一画布入口。旧工作流文件和模板 manifest 只在这里被并入规范画布，
/// 因而加载、保存、运行不再各自维护“当前工作流”选择状态。
pub fn load_project_canvas_impl(project_root: &Path) -> CommandResult<WorkflowGraphData> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "project_canvas_load")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    load_project_canvas_locked(project_root)
}

fn load_project_canvas_locked(project_root: &Path) -> CommandResult<WorkflowGraphData> {
    let (mut canvas, canvas_revision) = load_workflow_definition_with_revision(project_root, None)?;
    let previous_id = canvas.id.as_str().to_owned();
    let previous_name = canvas.name.clone();
    normalize_project_canvas_identity(&mut canvas);
    let mut changed = previous_id != canvas.id.as_str() || previous_name != canvas.name;

    let workflows_root = absolute_path(&project_layout(project_root)?.workflows);
    reject_symlink_root(&workflows_root)?;
    if workflows_root.exists() {
        let canonical_path =
            workflow_path(project_root, Some(PROJECT_CANVAS_WORKFLOW_ID.to_owned()))?;
        let mut paths = workflow_json_paths(&workflows_root)?;
        paths.sort();
        for path in paths {
            ensure_path_under_root(&workflows_root, &path).map_err(error_to_string)?;
            if path == canonical_path {
                continue;
            }
            let raw = std::fs::read(&path).map_err(error_to_string)?;
            let content = std::str::from_utf8(&raw).map_err(|error| {
                CommandError::serialization(format!(
                    "workflow file '{}' is not valid UTF-8: {error}",
                    path.display()
                ))
            })?;
            let source = parse_workflow_file(content)?;
            source.validate_topology().map_err(error_to_string)?;
            changed |= merge_workflow_into_project_canvas(
                &mut canvas,
                source,
                content_revision_hash(&raw),
            );
        }
    }

    if changed {
        validate_workflow_execution_contracts(&canvas).map_err(error_to_string)?;
        let mut graph = workflow_to_graph(canvas);
        graph.expected_revision = (!canvas_revision.is_empty()).then_some(canvas_revision);
        return save_workflow_graph_locked(project_root, graph);
    }

    let mut graph = workflow_to_graph(canvas);
    graph.content_revision = Some(canvas_revision);
    Ok(graph)
}

pub fn save_project_canvas_impl(
    project_root: &Path,
    mut graph_data: WorkflowGraphData,
) -> CommandResult<WorkflowGraphData> {
    graph_data.workflow_id = PROJECT_CANVAS_WORKFLOW_ID.to_owned();
    save_workflow_graph_impl(project_root, graph_data)
}

pub fn list_workflow_graphs_impl(project_root: &Path) -> CommandResult<Vec<WorkflowSummary>> {
    validate_project_root(project_root)?;
    let canvas = load_project_canvas_impl(project_root)?;
    let path = workflow_path(project_root, Some(PROJECT_CANVAS_WORKFLOW_ID.to_owned()))?;
    Ok(vec![WorkflowSummary {
        workflow_id: canvas.workflow_id,
        name: canvas.name,
        path: relative_id(project_root, &path)?,
        node_count: canvas.nodes.len(),
        edge_count: canvas.edges.len(),
    }])
}

pub fn save_workflow_graph_impl(
    project_root: &Path,
    graph_data: WorkflowGraphData,
) -> CommandResult<WorkflowGraphData> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "workflow_graph_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    save_workflow_graph_locked(project_root, graph_data)
}

fn save_workflow_graph_locked(
    project_root: &Path,
    graph_data: WorkflowGraphData,
) -> CommandResult<WorkflowGraphData> {
    let expected_revision = graph_data.expected_revision.clone();
    let workflow = graph_to_workflow(graph_data)?;
    validate_workflow_execution_contracts(&workflow).map_err(error_to_string)?;
    let path = workflow_path(project_root, Some(workflow.id.as_str().to_owned()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    // N3：revision compare 与最终 replace 必须处于同一写权边界。否则两个调用可同时
    // 通过同一 expected_revision，再依次覆盖文件并都向调用方报告成功。
    let _workflow_write =
        crate::config::store::PathWriteLock::acquire(&path).map_err(error_to_string)?;
    if path.exists() {
        let current_raw = std::fs::read(&path).map_err(error_to_string)?;
        let actual = content_revision_hash(&current_raw);
        match expected_revision.as_deref() {
            Some(expected) if expected == actual => {}
            Some(expected) => {
                return Err(CommandError::conflict(format!(
                    "workflow content revision conflict for {}: expected {expected}, actual {actual}",
                    workflow.id.as_str()
                )));
            }
            None => {
                return Err(CommandError::conflict(format!(
                    "expected_revision required to overwrite existing workflow {}",
                    workflow.id.as_str()
                )));
            }
        }
    }
    let body = serde_json::to_string_pretty(&workflow).map_err(error_to_string)?;
    crate::config::store::atomic_write(&path, body.as_bytes()).map_err(error_to_string)?;
    let mut out = workflow_to_graph(workflow);
    out.content_revision = Some(content_revision_hash(body.as_bytes()));
    out.expected_revision = None;
    Ok(out)
}

pub fn get_workflow_events_impl(
    project_root: &Path,
    workflow_id: String,
    run_id: String,
    after_sequence: Option<u64>,
    limit: Option<usize>,
) -> CommandResult<WorkflowEventsResult> {
    validate_existing_project_root(project_root)?;
    let workflow_id_typed = WorkflowId::from(workflow_id.clone());
    let run_id_typed = RunId::from(run_id.clone());
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let after_sequence = after_sequence.unwrap_or(0);
    let (status, events) = store
        .list_events_since(&workflow_id_typed, &run_id_typed, after_sequence, limit)
        .map_err(error_to_string)?
        .ok_or_else(|| {
            CommandError::not_found(format!("workflow run not found: {workflow_id}/{run_id}"))
        })?;
    let next_sequence = events
        .last()
        .map(|event| event.sequence.saturating_add(1))
        .unwrap_or(after_sequence);
    Ok(WorkflowEventsResult {
        workflow_id,
        run_id,
        status: run_status_label(status).to_owned(),
        next_sequence,
        events,
    })
}

pub fn run_workflow_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: RunWorkflowRequest,
) -> CommandResult<WorkflowRunStarted> {
    run_workflow_impl_with_run_id(project_root, secrets, request, new_run_id()?)
}

struct PreparedWorkflowRun {
    state: crate::workflow::WorkflowRunState,
    workflow: WorkflowDefinition,
    document_root: PathBuf,
    retrieval_runtime: Option<Arc<ProjectRetrievalRuntime>>,
    dependency_plan: WorkflowRuntimeDependencyPlan,
}

struct WorkflowRuntimeDependencyPlan {
    workflow: WorkflowExecutionDependencySet,
    project_config: ProjectConfig,
    node_presets: NodePresetSettings,
    llm_execution: WorkflowLlmExecutionPlan,
    executor_adapters: ExecutorAdapterExecutionPlan,
    executor_adapter_llm_providers: BTreeMap<String, OpenAiCompatibleLlmProvider>,
    credential_generations: BTreeMap<String, String>,
    requires_project_retrieval: bool,
    requires_web_search: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkflowLlmRoute {
    provider_id: String,
    model_id: String,
}

const FROZEN_WORKFLOW_DEPENDENCY_PLAN_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FrozenWorkflowRuntimeDependencyPlan {
    version: u32,
    workflow: WorkflowExecutionDependencySet,
    project_config: ProjectConfig,
    node_presets: NodePresetSettings,
    llm_node_routes: BTreeMap<String, WorkflowLlmRoute>,
    executor_adapter_manifests: Vec<LoadedSkillManifest>,
    credential_generations: BTreeMap<String, String>,
    requires_project_retrieval: bool,
    requires_web_search: bool,
}

struct WorkflowLlmExecutionPlan {
    node_routes: BTreeMap<String, WorkflowLlmRoute>,
    providers: BTreeMap<String, OpenAiCompatibleLlmProvider>,
}

fn compile_workflow_runtime_dependency_plan(
    project_root: &Path,
    secrets: &dyn SecretStore,
    workflow: &WorkflowDefinition,
) -> CommandResult<WorkflowRuntimeDependencyPlan> {
    let dependencies =
        WorkflowExecutionDependencySet::compile(workflow).map_err(error_to_string)?;
    let adapter_discovery_guard = if dependencies.uses_executor_adapters() {
        Some(acquire_project_mutation_guard(
            project_root,
            "executor_adapter_discovery",
        )?)
    } else {
        None
    };
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let executor_adapters = compile_executor_adapter_execution_plan(
        project_root,
        &project_config,
        dependencies.executor_adapter_skill_ids(),
    )?;
    let executor_adapter_llm_providers = resolve_executor_adapter_llm_providers(
        project_root,
        &project_config,
        &executor_adapters,
        &ExecutorAdapterLlmProviderSource::ProjectSecrets(secrets),
    )?;
    drop(adapter_discovery_guard);

    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls.clone());
    let node_presets = read_node_preset_settings(project_root)?;
    let llm_execution = compile_workflow_llm_execution_plan(
        project_root,
        secrets,
        workflow,
        &project_config,
        &node_presets,
    )?;
    let requires_project_retrieval = workflow_requires_project_retrieval(
        &dependencies,
        &tool_controls,
        &node_presets,
        executor_adapters.uses_llm(),
    );
    let requires_web_search = workflow_requires_web_search(
        &dependencies,
        &tool_controls,
        &project_config.permissions,
        &node_presets,
        executor_adapters.uses_llm(),
    );
    let credential_generations = workflow_dependency_credential_generations(
        project_root,
        secrets,
        &project_config,
        &llm_execution,
        &executor_adapters,
        requires_project_retrieval,
        requires_web_search,
    )?;
    Ok(WorkflowRuntimeDependencyPlan {
        workflow: dependencies,
        project_config,
        node_presets,
        llm_execution,
        executor_adapters,
        executor_adapter_llm_providers,
        credential_generations,
        requires_project_retrieval,
        requires_web_search,
    })
}

fn freeze_workflow_runtime_dependency_plan(
    plan: &WorkflowRuntimeDependencyPlan,
) -> FrozenWorkflowRuntimeDependencyPlan {
    let mut project_config = plan.project_config.clone();
    for provider in &mut project_config.providers.providers {
        provider.api_key = None;
    }
    FrozenWorkflowRuntimeDependencyPlan {
        version: FROZEN_WORKFLOW_DEPENDENCY_PLAN_VERSION,
        workflow: plan.workflow.clone(),
        project_config,
        node_presets: plan.node_presets.clone(),
        llm_node_routes: plan.llm_execution.node_routes.clone(),
        executor_adapter_manifests: plan.executor_adapters.manifests().to_vec(),
        credential_generations: plan.credential_generations.clone(),
        requires_project_retrieval: plan.requires_project_retrieval,
        requires_web_search: plan.requires_web_search,
    }
}

fn materialize_frozen_workflow_runtime_dependency_plan(
    project_root: &Path,
    secrets: &dyn SecretStore,
    workflow: &WorkflowDefinition,
    value: &Value,
) -> CommandResult<WorkflowRuntimeDependencyPlan> {
    let frozen = serde_json::from_value::<FrozenWorkflowRuntimeDependencyPlan>(value.clone())
        .map_err(error_to_string)?;
    if frozen.version != FROZEN_WORKFLOW_DEPENDENCY_PLAN_VERSION {
        return Err(CommandError::legacy_run(format!(
            "unsupported frozen workflow dependency plan version: {}",
            frozen.version
        )));
    }
    frozen.project_config.validate().map_err(error_to_string)?;
    if frozen
        .project_config
        .providers
        .providers
        .iter()
        .any(|provider| provider.api_key.is_some())
    {
        return Err(CommandError::permission(
            "frozen workflow dependency plan must not contain project SecretRef values",
        ));
    }
    let dependencies =
        WorkflowExecutionDependencySet::compile(workflow).map_err(error_to_string)?;
    if dependencies != frozen.workflow {
        return Err(CommandError::conflict(
            "frozen workflow dependency set does not match the prepared workflow",
        ));
    }

    let credential_scope =
        ProjectCredentialScope::new(project_root, secrets).map_err(error_to_string)?;
    for (provider_id, expected_generation) in &frozen.credential_generations {
        let actual_generation = credential_scope
            .provider_secret_generation(provider_id)
            .map_err(error_to_string)?;
        if &actual_generation != expected_generation {
            return Err(CommandError::conflict(format!(
                "provider credential generation changed after workflow preparation: {provider_id}"
            )));
        }
    }

    let executor_adapters =
        ExecutorAdapterExecutionPlan::from_frozen_manifests(frozen.executor_adapter_manifests)
            .map_err(error_to_string)?;
    let frozen_skill_ids = executor_adapters
        .manifests()
        .iter()
        .map(|loaded| loaded.manifest.skill_id.clone())
        .collect::<BTreeSet<_>>();
    if &frozen_skill_ids != dependencies.executor_adapter_skill_ids() {
        return Err(CommandError::conflict(
            "frozen ExecutorAdapter manifests do not match workflow dependencies",
        ));
    }
    let executor_adapter_llm_providers = resolve_executor_adapter_llm_providers(
        project_root,
        &frozen.project_config,
        &executor_adapters,
        &ExecutorAdapterLlmProviderSource::ProjectSecrets(secrets),
    )?;
    let llm_execution = materialize_frozen_workflow_llm_execution_plan(
        project_root,
        secrets,
        workflow,
        &frozen.project_config,
        frozen.llm_node_routes,
    )?;

    let tool_controls =
        normalize_tool_controls(frozen.project_config.permissions.tool_controls.clone());
    let requires_project_retrieval = workflow_requires_project_retrieval(
        &dependencies,
        &tool_controls,
        &frozen.node_presets,
        executor_adapters.uses_llm(),
    );
    let requires_web_search = workflow_requires_web_search(
        &dependencies,
        &tool_controls,
        &frozen.project_config.permissions,
        &frozen.node_presets,
        executor_adapters.uses_llm(),
    );
    if requires_project_retrieval != frozen.requires_project_retrieval
        || requires_web_search != frozen.requires_web_search
    {
        return Err(CommandError::conflict(
            "frozen workflow dependency flags do not match the frozen configuration",
        ));
    }

    Ok(WorkflowRuntimeDependencyPlan {
        workflow: dependencies,
        project_config: frozen.project_config,
        node_presets: frozen.node_presets,
        llm_execution,
        executor_adapters,
        executor_adapter_llm_providers,
        credential_generations: frozen.credential_generations,
        requires_project_retrieval,
        requires_web_search,
    })
}

fn materialize_frozen_workflow_llm_execution_plan(
    project_root: &Path,
    secrets: &dyn SecretStore,
    workflow: &WorkflowDefinition,
    project_config: &ProjectConfig,
    node_routes: BTreeMap<String, WorkflowLlmRoute>,
) -> CommandResult<WorkflowLlmExecutionPlan> {
    let expected_node_ids = workflow
        .nodes
        .iter()
        .filter(|node| is_llm_workflow_node_type(&node.type_name))
        .map(|node| node.id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let actual_node_ids = node_routes.keys().cloned().collect::<BTreeSet<_>>();
    if expected_node_ids != actual_node_ids {
        return Err(CommandError::conflict(
            "frozen LLM node routes do not match the prepared workflow",
        ));
    }

    let mut providers = BTreeMap::new();
    for route in node_routes.values() {
        let provider_config = project_config
            .providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == route.provider_id)
            .ok_or_else(|| {
                CommandError::not_found(format!(
                    "frozen LLM route references an unconfigured provider: {}",
                    route.provider_id
                ))
            })?;
        if !provider_config.enabled {
            return Err(CommandError::validation(format!(
                "frozen LLM route references a disabled provider: {}",
                route.provider_id
            )));
        }
        let model = provider_config
            .models
            .iter()
            .find(|model| model.model_id == route.model_id)
            .ok_or_else(|| {
                CommandError::not_found(format!(
                    "frozen LLM route references an unconfigured model: {}/{}",
                    route.provider_id, route.model_id
                ))
            })?;
        if !matches!(
            model.capability,
            ProviderCapability::Llm | ProviderCapability::ToolUse
        ) {
            return Err(CommandError::validation(format!(
                "frozen LLM route model is not LLM-capable: {}/{}",
                route.provider_id, route.model_id
            )));
        }
        if !providers.contains_key(&route.provider_id) {
            let api_key = provider_api_key(project_root, secrets, provider_config)?;
            providers.insert(
                route.provider_id.clone(),
                OpenAiCompatibleLlmProvider::new(provider_config.clone(), api_key)
                    .map_err(error_to_string)?,
            );
        }
    }
    Ok(WorkflowLlmExecutionPlan {
        node_routes,
        providers,
    })
}

fn workflow_dependency_credential_generations(
    project_root: &Path,
    secrets: &dyn SecretStore,
    project_config: &ProjectConfig,
    llm_execution: &WorkflowLlmExecutionPlan,
    executor_adapters: &ExecutorAdapterExecutionPlan,
    requires_project_retrieval: bool,
    requires_web_search: bool,
) -> CommandResult<BTreeMap<String, String>> {
    let mut provider_ids = llm_execution
        .providers
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    provider_ids.extend(executor_adapters.llm_provider_models().keys().cloned());
    if requires_web_search {
        provider_ids.insert(select_web_search_provider(&project_config.providers)?.provider_id);
    }
    if requires_project_retrieval && project_config.rag.vector_store.enabled {
        provider_ids.insert(provider_id_for_capability(
            project_config,
            project_config
                .providers
                .default_embedding_provider_id
                .as_deref(),
            ProviderCapability::Embedding,
            "embedding",
        )?);
    }
    if requires_project_retrieval && project_config.rag.reranker_enabled {
        provider_ids.insert(provider_id_for_capability(
            project_config,
            project_config
                .providers
                .default_reranker_provider_id
                .as_deref(),
            ProviderCapability::Reranker,
            "reranker",
        )?);
    }
    let scope = ProjectCredentialScope::new(project_root, secrets).map_err(error_to_string)?;
    provider_ids
        .into_iter()
        .map(|provider_id| {
            let generation = scope
                .provider_secret_generation(&provider_id)
                .map_err(error_to_string)?;
            Ok((provider_id, generation))
        })
        .collect()
}

fn provider_id_for_capability(
    project_config: &ProjectConfig,
    default_provider_id: Option<&str>,
    capability: ProviderCapability,
    role: &str,
) -> CommandResult<String> {
    let provider = default_provider_id
        .and_then(|provider_id| {
            project_config
                .providers
                .providers
                .iter()
                .find(|provider| provider.provider_id == provider_id)
        })
        .or_else(|| {
            project_config.providers.providers.iter().find(|provider| {
                provider.enabled
                    && provider
                        .models
                        .iter()
                        .any(|model| model.capability == capability)
            })
        })
        .ok_or_else(|| {
            CommandError::not_found(format!(
                "no enabled {role} provider is configured for frozen workflow dependencies"
            ))
        })?;
    Ok(provider.provider_id.clone())
}

fn prepare_workflow_run_state(
    project_root: &Path,
    secrets: &dyn SecretStore,
    retrieval_runtime: Option<Arc<ProjectRetrievalRuntime>>,
    request: &RunWorkflowRequest,
    run_id: RunId,
) -> CommandResult<PreparedWorkflowRun> {
    let start_node_id = request.start_node_id.clone();
    let workflow = load_workflow_definition(project_root, Some(request.workflow_id.clone()))?;
    let mut workflow = if let Some(start_node_id) = start_node_id.as_deref() {
        workflow_branch_from_start(&workflow, &NodeId::from(start_node_id))?
    } else {
        workflow
    };
    // F10-c：有 Start 声明 schema 时，即使 initial_inputs 为空也必须校验 required，
    // 禁止 `{}` / 缺字段在 create_state 之前静默通过。
    if let Some(start_node_id) = start_node_id.as_deref() {
        inject_start_node_initial_inputs(
            &mut workflow,
            start_node_id,
            request.initial_inputs.clone(),
        )?;
    } else if !request.initial_inputs.is_empty() {
        return Err(CommandError::validation(
            "initial_inputs require start_node_id",
        ));
    }
    validate_workflow_execution_contracts(&workflow).map_err(error_to_string)?;
    let document_root = workflow_document_root(project_root, &workflow, start_node_id.as_deref())?;
    let dependency_plan =
        compile_workflow_runtime_dependency_plan(project_root, secrets, &workflow)?;
    let retrieval_runtime = if dependency_plan.requires_project_retrieval {
        Some(match retrieval_runtime {
            Some(runtime)
                if runtime
                    .matches_project_config(&dependency_plan.project_config)
                    .map_err(error_to_string)? =>
            {
                runtime
            }
            Some(_) => return Err(CommandError::conflict(
                "shared project retrieval runtime does not match workflow preflight configuration",
            )),
            None => Arc::new(
                ProjectRetrievalRuntime::from_config(
                    project_root,
                    secrets,
                    &dependency_plan.project_config,
                    None,
                )
                .map_err(error_to_string)?,
            ),
        })
    } else {
        None
    };
    let mut runtime = WorkflowRuntime::new(&workflow, run_id).map_err(error_to_string)?;
    runtime.state.prepared_workflow = Some(workflow.clone());
    runtime.state.prepared_dependency_plan = Some(
        serde_json::to_value(freeze_workflow_runtime_dependency_plan(&dependency_plan))
            .map_err(error_to_string)?,
    );
    runtime.state.start_node_id = start_node_id.as_deref().map(NodeId::from);
    preflight_workflow_runtime_dependencies(
        project_root,
        &document_root,
        secrets,
        &dependency_plan,
        retrieval_runtime.as_deref(),
    )?;
    runtime.state.structured_events.push(WorkflowRuntimeEvent {
        sequence: 0,
        event_type: WorkflowRuntimeEventType::RunQueued,
        node_id: None,
        message: "workflow run queued".to_owned(),
        metadata: json!({ "stage": "preflight_complete" }),
    });
    runtime.state.next_event_sequence = 1;
    Ok(PreparedWorkflowRun {
        state: runtime.state,
        workflow,
        document_root,
        retrieval_runtime,
        dependency_plan,
    })
}

fn preflight_workflow_runtime_dependencies(
    project_root: &Path,
    document_root: &Path,
    secrets: &dyn SecretStore,
    dependency_plan: &WorkflowRuntimeDependencyPlan,
    retrieval_runtime: Option<&ProjectRetrievalRuntime>,
) -> CommandResult<()> {
    crate::config::ProjectLayout::from_app(project_root, &dependency_plan.project_config.app)
        .and_then(|layout| layout.create_configured_directories())
        .map_err(error_to_string)?;
    std::fs::create_dir_all(document_root.join("planning")).map_err(error_to_string)?;
    SqliteCostLedger::open(project_root).map_err(error_to_string)?;
    if dependency_plan.requires_project_retrieval {
        retrieval_runtime.ok_or_else(|| {
            CommandError::internal("search-capable workflow preflight is missing retrieval runtime")
        })?;
    }
    if dependency_plan.requires_web_search {
        web_search_runtime_for_config(project_root, secrets, &dependency_plan.project_config)?;
    }
    Ok(())
}

fn run_workflow_impl_with_run_id(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: RunWorkflowRequest,
    run_id: RunId,
) -> CommandResult<WorkflowRunStarted> {
    validate_existing_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "workflow_execution")?;
    let prepared =
        prepare_workflow_run_state(project_root, secrets, None, &request, run_id.clone())?;
    let PreparedWorkflowRun {
        state,
        workflow,
        document_root,
        retrieval_runtime,
        dependency_plan,
    } = prepared;
    let mut runtime = WorkflowRuntime::from_state(state);
    let status = execute_workflow_runtime(
        WorkflowExecutionContext {
            project_root,
            document_root: &document_root,
            secrets,
            retrieval_runtime,
            dependency_plan: Some(dependency_plan),
        },
        &workflow,
        &mut runtime,
        None,
        crate::contracts::ExecutionCancellation::new(),
    )?;
    Ok(WorkflowRunStarted {
        run_id: run_id.as_str().to_owned(),
        status: run_status_label(status).to_owned(),
    })
}

fn continue_workflow_run_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    retrieval_runtime: Option<Arc<ProjectRetrievalRuntime>>,
    workflow_id: String,
    run_id: String,
    worker_lease: &WorkflowWorkerLease,
    cancellation: crate::contracts::ExecutionCancellation,
) -> CommandResult<WorkflowRunStarted> {
    ensure_project_not_in_maintenance(project_root)?;
    validate_existing_project_root(project_root)?;
    let workflow_id_typed = WorkflowId::from(workflow_id.clone());
    let run_id_typed = RunId::from(run_id.clone());
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let state = store
        .load_state(&workflow_id_typed, &run_id_typed)
        .map_err(error_to_string)?
        .ok_or_else(|| {
            CommandError::not_found(format!("workflow run not found: {workflow_id}/{run_id}"))
        })?;
    if state.status.is_terminal() {
        return Ok(WorkflowRunStarted {
            run_id,
            status: run_status_label(state.status).to_owned(),
        });
    }
    let (workflow, start_node_id) =
        workflow_for_run_state(project_root, &workflow_id_typed, &state)?;
    let document_root = workflow_document_root(
        project_root,
        &workflow,
        start_node_id.as_ref().map(NodeId::as_str),
    )?;
    let dependency_value = state.prepared_dependency_plan.as_ref().ok_or_else(|| {
        CommandError::legacy_run(format!(
            "legacy run snapshot lacks prepared_dependency_plan for {workflow_id}/{run_id}; cannot safely resume from mutable project configuration"
        ))
    })?;
    let dependency_plan = materialize_frozen_workflow_runtime_dependency_plan(
        project_root,
        secrets,
        &workflow,
        dependency_value,
    )?;
    let mut runtime = WorkflowRuntime::from_state(state);
    let status = execute_workflow_runtime(
        WorkflowExecutionContext {
            project_root,
            document_root: &document_root,
            secrets,
            retrieval_runtime,
            dependency_plan: Some(dependency_plan),
        },
        &workflow,
        &mut runtime,
        Some(worker_lease),
        cancellation,
    )?;
    Ok(WorkflowRunStarted {
        run_id,
        status: run_status_label(status).to_owned(),
    })
}

fn workflow_for_run_state(
    project_root: &Path,
    workflow_id: &WorkflowId,
    state: &crate::workflow::WorkflowRunState,
) -> CommandResult<(WorkflowDefinition, Option<NodeId>)> {
    let _ = project_root;
    if let Some(workflow) = &state.prepared_workflow {
        if workflow.id != *workflow_id {
            return Err(CommandError::conflict(format!(
                "prepared workflow id mismatch: expected {}, got {}",
                workflow_id.as_str(),
                workflow.id.as_str()
            )));
        }
        workflow.validate_topology().map_err(error_to_string)?;
        return Ok((workflow.clone(), state.start_node_id.clone()));
    }

    // F10: do not silently rebuild from live disk — that reintroduces drift.
    // Legacy snapshots without prepared_workflow need an explicit recovery path.
    Err(CommandError::legacy_run(format!(
        "legacy run snapshot lacks prepared_workflow for {}/{}; cannot safely resume without frozen execution definition",
        workflow_id.as_str(),
        state.run_id.as_str()
    )))
}

/// F11：将项目 skills 目录中的 ExecutorAdapter 注册到生产 `RoutedExternalNodeExecutor`。
/// 返回成功注册的 `executor_adapter:{skill_id}` 列表。manifest 加载或注册失败 **fail-loud**。
pub fn register_executor_adapters_for_project(
    external: &mut RoutedExternalNodeExecutor,
    project_root: &Path,
    required_skill_ids: &BTreeSet<String>,
    provider: OpenAiCompatibleLlmProvider,
    ledger: Arc<SqliteCostLedger>,
) -> CommandResult<Vec<String>> {
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "executor_adapter_discovery")?;
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let plan =
        compile_executor_adapter_execution_plan(project_root, &project_config, required_skill_ids)?;
    let llm_providers = resolve_executor_adapter_llm_providers(
        project_root,
        &project_config,
        &plan,
        &ExecutorAdapterLlmProviderSource::Single(provider),
    )?;
    register_executor_adapters_for_project_with_search(
        external,
        &project_config,
        plan,
        llm_providers,
        ledger,
        None,
        None,
    )
}

enum ExecutorAdapterLlmProviderSource<'a> {
    ProjectSecrets(&'a dyn SecretStore),
    Single(OpenAiCompatibleLlmProvider),
}

fn resolve_executor_adapter_llm_providers(
    project_root: &Path,
    project_config: &ProjectConfig,
    plan: &ExecutorAdapterExecutionPlan,
    source: &ExecutorAdapterLlmProviderSource<'_>,
) -> CommandResult<BTreeMap<String, OpenAiCompatibleLlmProvider>> {
    let mut resolved = BTreeMap::new();
    for loaded in plan.manifests() {
        let crate::skills::SkillExecutorConfig::Llm(config) = &loaded.manifest.executor else {
            continue;
        };
        let provider_id = config.provider_id.trim();
        let model_id = config.model_id.trim();
        if provider_id.is_empty() || model_id.is_empty() {
            return Err(CommandError::validation(format!(
                "LLM ExecutorAdapter '{}' requires non-empty provider_id and model_id",
                loaded.manifest.skill_id
            )));
        }

        match source {
            ExecutorAdapterLlmProviderSource::ProjectSecrets(secrets) => {
                let provider_config = project_config
                    .providers
                    .providers
                    .iter()
                    .find(|provider| provider.provider_id == provider_id)
                    .ok_or_else(|| {
                        CommandError::not_found(format!(
                            "LLM ExecutorAdapter '{}' references an unconfigured provider: {provider_id}",
                            loaded.manifest.skill_id
                        ))
                    })?;
                if !provider_config.enabled {
                    return Err(CommandError::validation(format!(
                        "LLM ExecutorAdapter '{}' references a disabled provider: {provider_id}",
                        loaded.manifest.skill_id
                    )));
                }
                let model = provider_config
                    .models
                    .iter()
                    .find(|model| model.model_id == model_id)
                    .ok_or_else(|| {
                        CommandError::not_found(format!(
                            "LLM ExecutorAdapter '{}' references an unconfigured model: {provider_id}/{model_id}",
                            loaded.manifest.skill_id
                        ))
                    })?;
                if !matches!(
                    model.capability,
                    ProviderCapability::Llm | ProviderCapability::ToolUse
                ) {
                    return Err(CommandError::validation(format!(
                        "LLM ExecutorAdapter '{}' model is not LLM-capable: {provider_id}/{model_id}",
                        loaded.manifest.skill_id
                    )));
                }
                if !resolved.contains_key(provider_id) {
                    let api_key = provider_api_key(project_root, *secrets, provider_config)?;
                    let provider =
                        OpenAiCompatibleLlmProvider::new(provider_config.clone(), api_key)
                            .map_err(error_to_string)?;
                    resolved.insert(provider_id.to_owned(), provider);
                }
            }
            ExecutorAdapterLlmProviderSource::Single(provider) => {
                let actual_provider_id = provider.definition().provider_id;
                if actual_provider_id != provider_id {
                    return Err(CommandError::validation(format!(
                        "LLM ExecutorAdapter '{}' requires provider '{provider_id}', but registration supplied '{actual_provider_id}'",
                        loaded.manifest.skill_id
                    )));
                }
                resolved
                    .entry(provider_id.to_owned())
                    .or_insert_with(|| provider.clone());
            }
        }
    }
    Ok(resolved)
}

fn compile_executor_adapter_execution_plan(
    project_root: &Path,
    project_config: &ProjectConfig,
    required_skill_ids: &BTreeSet<String>,
) -> CommandResult<ExecutorAdapterExecutionPlan> {
    if required_skill_ids.is_empty() {
        return Ok(ExecutorAdapterExecutionPlan::empty());
    }
    let layout = crate::config::ProjectLayout::from_app(project_root, &project_config.app)
        .map_err(error_to_string)?;
    let loader = SkillLoader::new().with_project_root(layout.skills);
    ExecutorAdapterExecutionPlan::compile(&loader, required_skill_ids).map_err(error_to_string)
}

fn register_executor_adapters_for_project_with_search(
    external: &mut RoutedExternalNodeExecutor,
    project_config: &ProjectConfig,
    plan: ExecutorAdapterExecutionPlan,
    llm_providers: BTreeMap<String, OpenAiCompatibleLlmProvider>,
    ledger: Arc<SqliteCostLedger>,
    retrieval: Option<Arc<ProjectRetrievalRuntime>>,
    web_search_provider: Option<Arc<HttpWebSearchProvider>>,
) -> CommandResult<Vec<String>> {
    use crate::contracts::permissions::ExecutionPolicy;
    use crate::contracts::AutoModeState;
    use crate::skills::{
        NativeHttpSkillBackend, NativeWasmSkillBackend, SkillExecutionContext, SkillExecutor,
    };

    let auto_mode_config = project_config.auto_mode.clone();
    let execution_policy = ExecutionPolicy {
        auto_mode: AutoModeState {
            enabled: auto_mode_config.enabled_by_default,
            preauthorized_budget_usd: auto_mode_config.preauthorized_budget_usd,
        },
        // ExecutorAdapter 与普通工作流节点必须消费同一份项目硬权限。
        // 这里若重置为默认拒绝，会让已注入的 Web Search/HTTP/WASM 能力
        // 在真实执行边界永久不可达，并与设置页保存的权限产生第二状态源。
        permissions: permission_policy_for_scope(&project_config.permissions, "workflow_nodes"),
    };
    let max_tool_rounds = project_config.workflow.max_tool_rounds;
    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls.clone());
    let project_search_enabled = tool_control_enabled(
        &tool_controls,
        "executor_adapter",
        EXECUTOR_ADAPTER_SEARCH_TOOL,
    );
    let web_search_enabled = execution_policy
        .permissions
        .evaluate(&PermissionRequest::WebSearch)
        .allowed
        && tool_control_enabled(
            &tool_controls,
            "executor_adapter",
            EXECUTOR_ADAPTER_WEB_SEARCH_TOOL,
        );
    let mut registered = Vec::new();
    for loaded in plan.into_manifests() {
        let skill_id = loaded.manifest.skill_id.clone();
        let manifest = loaded.manifest.clone();
        let llm_provider = match &manifest.executor {
            crate::skills::SkillExecutorConfig::Llm(config) => llm_providers
                .get(config.provider_id.trim())
                .cloned()
                .ok_or_else(|| {
                    CommandError::internal(format!(
                        "resolved provider missing for LLM ExecutorAdapter '{}': {}",
                        manifest.skill_id, config.provider_id
                    ))
                })?
                .into(),
            crate::skills::SkillExecutorConfig::Http(_)
            | crate::skills::SkillExecutorConfig::Wasm(_) => None,
        };
        let ledger = Arc::clone(&ledger);
        let execution_policy = execution_policy.clone();
        let auto_mode_config = auto_mode_config.clone();
        let project_search = project_search_enabled
            .then(|| retrieval.as_ref().map(Arc::clone))
            .flatten();
        let web_search = web_search_enabled
            .then(|| web_search_provider.as_ref().map(Arc::clone))
            .flatten();
        let type_name = format!("executor_adapter:{skill_id}");
        let operation_policy = match &manifest.executor {
            crate::skills::SkillExecutorConfig::Http(config)
                if config.method.eq_ignore_ascii_case("GET")
                    || config.idempotency_header.is_some() =>
            {
                crate::workflow::WorkflowOperationPolicy::replayable_remote()
            }
            crate::skills::SkillExecutorConfig::Wasm(_) => {
                crate::workflow::WorkflowOperationPolicy::replayable_remote()
            }
            crate::skills::SkillExecutorConfig::Llm(_)
            | crate::skills::SkillExecutorConfig::Http(_) => {
                crate::workflow::WorkflowOperationPolicy::at_most_once()
            }
        };
        external
            .register_handler_with_policy(
                type_name.clone(),
                operation_policy,
                Box::new(move |request| {
                    let http_backend = NativeHttpSkillBackend;
                    let wasm_backend = NativeWasmSkillBackend::default();
                    let context = SkillExecutionContext {
                        execution_policy: &execution_policy,
                        auto_mode_config: &auto_mode_config,
                        budget_limits: Default::default(),
                        ledger: ledger.as_ref(),
                        llm_provider: llm_provider
                            .as_ref()
                            .map(|provider| provider as &dyn crate::providers::LlmProvider),
                        http_backend: Some(&http_backend),
                        wasm_backend: Some(&wasm_backend),
                    };
                    let executor = SkillExecutor::new(context);
                    let executor = match project_search.as_ref() {
                        Some(retrieval) => executor.with_project_search(
                            retrieval,
                            project_search_tool_definition(
                                EXECUTOR_ADAPTER_TOOL_CAPABILITY
                                    .project_search_tool
                                    .unwrap(),
                                EXECUTOR_ADAPTER_TOOL_CAPABILITY
                                    .project_search_description
                                    .unwrap(),
                            ),
                            max_tool_rounds,
                        ),
                        None => executor,
                    };
                    let executor = match web_search.as_ref() {
                        Some(search_provider) => executor.with_web_search(
                            search_provider.as_ref(),
                            web_search_tool_definition(
                                EXECUTOR_ADAPTER_TOOL_CAPABILITY.web_search_tool.unwrap(),
                                EXECUTOR_ADAPTER_TOOL_CAPABILITY
                                    .web_search_description
                                    .unwrap(),
                            ),
                            max_tool_rounds,
                        ),
                        None => executor,
                    };
                    crate::workflow::execute_executor_adapter_node(request, &manifest, &executor)
                }),
            )
            .map_err(error_to_string)?;
        registered.push(type_name);
    }
    Ok(registered)
}

struct WorkflowExecutionContext<'a> {
    project_root: &'a Path,
    document_root: &'a Path,
    secrets: &'a dyn SecretStore,
    retrieval_runtime: Option<Arc<ProjectRetrievalRuntime>>,
    dependency_plan: Option<WorkflowRuntimeDependencyPlan>,
}

struct WorkflowNodeSearchBindings {
    project_search: Option<(Arc<ProjectRetrievalRuntime>, ToolDefinition)>,
    web_search: Option<(Arc<HttpWebSearchProvider>, ToolDefinition)>,
    permission_policy: PermissionPolicy,
}

fn workflow_node_search_bindings(
    type_name: &str,
    controls: &BTreeMap<String, BTreeMap<String, Option<bool>>>,
    presets: &NodePresetSettings,
    permissions: &crate::config::PermissionsConfig,
    retrieval_runtime: Option<&Arc<ProjectRetrievalRuntime>>,
    web_search_runtime: Option<&Arc<HttpWebSearchProvider>>,
) -> WorkflowNodeSearchBindings {
    let permission_policy = permission_policy_for_node(permissions, presets, type_name);
    let project_search = workflow_search_tool_for_node(type_name)
        .filter(|(scope, tool, _)| {
            node_tool_control_enabled(controls, presets, type_name, scope, tool)
        })
        .and_then(|(_, tool, description)| {
            retrieval_runtime.map(|retrieval| {
                (
                    Arc::clone(retrieval),
                    project_search_tool_definition(tool, description),
                )
            })
        });
    let web_search = workflow_web_search_tool_for_node(type_name)
        .filter(|(scope, tool, _)| {
            permission_policy
                .evaluate(&PermissionRequest::WebSearch)
                .allowed
                && node_tool_control_enabled(controls, presets, type_name, scope, tool)
        })
        .and_then(|(_, tool, description)| {
            web_search_runtime.map(|provider| {
                (
                    Arc::clone(provider),
                    web_search_tool_definition(tool, description),
                )
            })
        });
    WorkflowNodeSearchBindings {
        project_search,
        web_search,
        permission_policy,
    }
}

fn execute_workflow_runtime(
    context: WorkflowExecutionContext<'_>,
    workflow: &WorkflowDefinition,
    runtime: &mut WorkflowRuntime,
    worker_lease: Option<&WorkflowWorkerLease>,
    cancellation: crate::contracts::ExecutionCancellation,
) -> CommandResult<crate::contracts::RunStatus> {
    let WorkflowExecutionContext {
        project_root,
        document_root,
        secrets,
        retrieval_runtime,
        dependency_plan,
    } = context;
    runtime.set_cancellation(cancellation);
    let WorkflowRuntimeDependencyPlan {
        workflow: dependencies,
        project_config,
        node_presets,
        llm_execution,
        executor_adapters: adapter_plan,
        executor_adapter_llm_providers,
        credential_generations: _,
        requires_project_retrieval,
        requires_web_search,
    } = match dependency_plan {
        Some(plan) => plan,
        None => {
            return Err(CommandError::legacy_run(
                "workflow execution requires a frozen dependency plan",
            ))
        }
    };
    let layout = crate::config::ProjectLayout::from_app(project_root, &project_config.app)
        .map_err(error_to_string)?;
    let documents = document_service_with_artifacts(
        document_root,
        project_root.join(".runtime").join("artifacts"),
        Some(layout.exports.clone()),
    );
    layout
        .create_configured_directories()
        .map_err(error_to_string)?;
    std::fs::create_dir_all(document_root.join("planning")).map_err(error_to_string)?;
    let ledger = Arc::new(SqliteCostLedger::open(project_root).map_err(error_to_string)?);
    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls.clone());
    let max_tool_rounds = project_config.workflow.max_tool_rounds;
    let retrieval_runtime = if requires_project_retrieval {
        Some(match retrieval_runtime {
            Some(runtime) => runtime,
            None => {
                return Err(CommandError::internal(
                    "workflow dependency plan requires the shared project retrieval runtime",
                ))
            }
        })
    } else {
        retrieval_runtime
    };
    let web_search_runtime = if requires_web_search {
        Some(Arc::new(web_search_runtime_for_config(
            project_root,
            secrets,
            &project_config,
        )?))
    } else {
        None
    };
    let mut external = RoutedExternalNodeExecutor::new();
    if !llm_execution.node_routes.is_empty() {
        let llm_routes = Arc::new(llm_execution.node_routes);
        let llm_providers = Arc::new(llm_execution.providers);
        // 普通 LLM 语义节点走 execute_llm_node。summarizer 例外：它是四步总结
        // 生产链（故事段划分并概括 → 事件 → 章节 → 阶段），走专用 handler 落库建索引。
        for capability in model_tool_node_capabilities()
            .filter(|capability| capability.execution_kind == WorkflowNodeExecutionKind::Model)
            .filter(|capability| dependencies.uses_node_type(capability.node_type))
        {
            let type_name = capability.node_type;
            let llm_routes = Arc::clone(&llm_routes);
            let llm_providers = Arc::clone(&llm_providers);
            let ledger = Arc::clone(&ledger);
            let node_preset = node_type_preset(&node_presets, type_name).cloned();
            let search_bindings = workflow_node_search_bindings(
                type_name,
                &tool_controls,
                &node_presets,
                &project_config.permissions,
                retrieval_runtime.as_ref(),
                web_search_runtime.as_ref(),
            );
            external
                .register_handler_with_policy(
                    type_name,
                    crate::workflow::WorkflowOperationPolicy::at_most_once(),
                    Box::new(move |request| {
                        let request = apply_node_type_preset(request, node_preset.as_ref());
                        let (request, provider, route) = route_workflow_llm_request(
                            request,
                            llm_routes.as_ref(),
                            llm_providers.as_ref(),
                        )?;
                        if search_bindings.project_search.is_none()
                            && search_bindings.web_search.is_none()
                        {
                            return execute_llm_node_with_defaults(
                                request,
                                provider,
                                ledger.as_ref(),
                                Some(&route.provider_id),
                                Some(&route.model_id),
                            );
                        }
                        execute_llm_node_with_search_tools(
                            request,
                            provider,
                            ledger.as_ref(),
                            WorkflowLlmSearchOptions {
                                default_provider_id: Some(&route.provider_id),
                                default_model_id: Some(&route.model_id),
                                project_search: search_bindings
                                    .project_search
                                    .as_ref()
                                    .map(|(retrieval, tool)| (retrieval.as_ref(), tool.clone())),
                                web_search: search_bindings.web_search.as_ref().map(
                                    |(search_provider, tool)| {
                                        (
                                            search_provider.as_ref() as &dyn SearchProvider,
                                            &search_bindings.permission_policy,
                                            tool.clone(),
                                        )
                                    },
                                ),
                                max_tool_rounds,
                            },
                        )
                    }),
                )
                .map_err(error_to_string)?;
        }

        // Summarizer 专用节点：加载写作知识库、四步总结、落库、生成四层确认项。
        if dependencies.uses_node_type("summarizer") {
            let llm_routes = Arc::clone(&llm_routes);
            let llm_providers = Arc::clone(&llm_providers);
            let ledger = Arc::clone(&ledger);
            let summarizer_root = project_root.to_path_buf();
            let node_preset = node_type_preset(&node_presets, "summarizer").cloned();
            let search_bindings = workflow_node_search_bindings(
                "summarizer",
                &tool_controls,
                &node_presets,
                &project_config.permissions,
                retrieval_runtime.as_ref(),
                web_search_runtime.as_ref(),
            );
            external
                .register_handler_with_policy(
                    "summarizer",
                    crate::workflow::WorkflowOperationPolicy::replayable_receipt(),
                    Box::new(move |request| {
                        let request = apply_node_type_preset(request, node_preset.as_ref());
                        let (request, provider, _) = route_workflow_llm_request(
                            request,
                            llm_routes.as_ref(),
                            llm_providers.as_ref(),
                        )?;
                        if search_bindings.project_search.is_none()
                            && search_bindings.web_search.is_none()
                        {
                            return execute_summarizer_node(
                                request,
                                provider,
                                ledger.as_ref(),
                                &summarizer_root,
                            );
                        }
                        execute_summarizer_node_with_search_tools(
                            request,
                            provider,
                            ledger.as_ref(),
                            &summarizer_root,
                            search_bindings
                                .project_search
                                .as_ref()
                                .map(|(retrieval, tool)| (retrieval.as_ref(), tool.clone())),
                            search_bindings
                                .web_search
                                .as_ref()
                                .map(|(search_provider, tool)| {
                                    (
                                        search_provider.as_ref() as &dyn SearchProvider,
                                        &search_bindings.permission_policy,
                                        tool.clone(),
                                    )
                                }),
                            max_tool_rounds,
                        )
                    }),
                )
                .map_err(error_to_string)?;
        }
    }
    if let Some(entry) = workflow_node_catalog_entry("document_read") {
        for type_name in std::iter::once(entry.node_type.as_str())
            .chain(entry.aliases.iter().map(String::as_str))
            .filter(|type_name| dependencies.uses_node_type(type_name))
        {
            let documents = documents.clone();
            let document_root = document_root.to_path_buf();
            external
                .register_handler(
                    type_name,
                    Box::new(move |request| {
                        execute_document_read_node_with_root(
                            request,
                            &documents,
                            Some(&document_root),
                        )
                    }),
                )
                .map_err(error_to_string)?;
        }
    }

    // F11：生产组合根注册 ExecutorAdapter（失败 fail-loud，不得静默丢弃）。
    // Skill 自己声明 provider/model；HTTP/WASM Skill 不得被默认 LLM 配置绑架。
    if dependencies.uses_executor_adapters() {
        register_executor_adapters_for_project_with_search(
            &mut external,
            &project_config,
            adapter_plan,
            executor_adapter_llm_providers,
            Arc::clone(&ledger),
            retrieval_runtime.clone(),
            web_search_runtime.clone(),
        )?;
    }
    if let Some(entry) = workflow_node_catalog_entry("search") {
        let search_types = std::iter::once(entry.node_type.as_str())
            .chain(entry.aliases.iter().map(String::as_str))
            .filter(|type_name| dependencies.uses_node_type(type_name))
            .collect::<Vec<_>>();
        if !search_types.is_empty() {
            let retrieval = match retrieval_runtime.clone() {
                Some(retrieval) => retrieval,
                None => Arc::new(
                    ProjectRetrievalRuntime::open(project_root, secrets)
                        .map_err(error_to_string)?,
                ),
            };
            for type_name in search_types {
                let retrieval = Arc::clone(&retrieval);
                let search_root = project_root.to_path_buf();
                external
                    .register_handler_with_policy(
                        type_name,
                        crate::workflow::WorkflowOperationPolicy::replayable_remote(),
                        Box::new(move |request| {
                            // F2-b：工作流 Search 节点与 IPC 搜索共用新鲜度门禁。
                            execute_project_retrieval_node_for_project(
                                &search_root,
                                request,
                                &retrieval,
                            )
                        }),
                    )
                    .map_err(error_to_string)?;
            }
        }
    }
    let mut export_sink = DocumentWorkflowExportSink::new(&documents);
    let mut executor =
        BuiltinWorkflowNodeExecutor::new(&mut external).with_export_sink(&mut export_sink);
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let store = if let Some(worker_lease) = worker_lease {
        store.with_worker_lease(worker_lease.clone())
    } else {
        store
    };
    runtime
        .run_persisted(workflow, &mut executor, &store)
        .map_err(error_to_string)
}

fn inject_start_node_initial_inputs(
    workflow: &mut WorkflowDefinition,
    start_node_id: &str,
    initial_inputs: BTreeMap<String, Value>,
) -> CommandResult<()> {
    let start_node = workflow
        .nodes
        .iter_mut()
        .find(|node| node.id == NodeId::from(start_node_id))
        .ok_or_else(|| CommandError::not_found(format!("start node not found: {start_node_id}")))?;
    if start_node.type_name != "start" {
        return Err(CommandError::validation(format!(
            "initial_inputs target must be a start node, got {} ({})",
            start_node.id.as_str(),
            start_node.type_name
        )));
    }
    // F10-c：若 Start 声明了 input schema，校验 initial_inputs 再持久化。
    // 空 map 仍会跑 required 校验；仅在有字段时写入 config，避免无输入时污染节点。
    validate_start_node_initial_inputs(start_node, &initial_inputs)?;
    if !initial_inputs.is_empty() {
        let mut config = start_node.config.as_object().cloned().unwrap_or_default();
        config.insert(
            "initial_inputs".to_owned(),
            Value::Object(initial_inputs.into_iter().collect()),
        );
        start_node.config = Value::Object(config);
    }
    Ok(())
}

/// F10-c：按 Start 节点 `input_schema` / `inputs` 声明校验 initial_inputs（有声明才强制）。
fn validate_start_node_initial_inputs(
    start_node: &crate::contracts::NodeInstance,
    initial_inputs: &BTreeMap<String, Value>,
) -> CommandResult<()> {
    let config = start_node.config.as_object();
    let Some(config) = config else {
        return Ok(());
    };
    // Prefer explicit input_schema.properties / required
    if let Some(schema) = config.get("input_schema").and_then(|v| v.as_object()) {
        if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
            for key in required {
                let Some(name) = key.as_str() else { continue };
                if !initial_inputs.contains_key(name) {
                    return Err(CommandError::validation(format!(
                        "initial_inputs missing required field '{name}' for start node {}",
                        start_node.id.as_str()
                    )));
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
            for (name, value) in initial_inputs {
                if !properties.contains_key(name) {
                    return Err(CommandError::validation(format!(
                        "initial_inputs field '{name}' is not declared in start node input_schema"
                    )));
                }
                if let Some(expected) = properties
                    .get(name)
                    .and_then(|p| p.get("type"))
                    .and_then(|t| t.as_str())
                {
                    if !json_value_matches_schema_type(value, expected) {
                        return Err(CommandError::validation(format!(
                            "initial_inputs field '{name}' expected type {expected}"
                        )));
                    }
                }
            }
        }
        return Ok(());
    }
    // Fallback: optional declared `inputs` array of port names in config
    if let Some(inputs) = config.get("inputs").and_then(|v| v.as_array()) {
        let allowed: std::collections::BTreeSet<_> = inputs
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        if !allowed.is_empty() {
            for name in initial_inputs.keys() {
                if !allowed.contains(name) {
                    return Err(CommandError::validation(format!(
                        "initial_inputs field '{name}' is not a declared start input"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn json_value_matches_schema_type(value: &Value, expected: &str) -> bool {
    match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn is_llm_workflow_node_type(type_name: &str) -> bool {
    workflow_node_capability(type_name).is_some_and(|capability| capability.supports_model_tools())
}

fn compile_workflow_llm_execution_plan(
    project_root: &Path,
    secrets: &dyn SecretStore,
    workflow: &WorkflowDefinition,
    project_config: &ProjectConfig,
    node_presets: &NodePresetSettings,
) -> CommandResult<WorkflowLlmExecutionPlan> {
    let mut node_routes = BTreeMap::new();
    let mut providers = BTreeMap::new();

    for node in workflow
        .nodes
        .iter()
        .filter(|node| is_llm_workflow_node_type(&node.type_name))
    {
        let config = serde_json::from_value::<WorkflowLlmNodeConfig>(node.config.clone()).map_err(
            |error| {
                CommandError::validation(format!(
                    "{} node {} has invalid LLM configuration: {error}",
                    node.type_name,
                    node.id.as_str()
                ))
            },
        )?;
        let configured_provider_id = config
            .provider_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let configured_model_id = config
            .model_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let preset = node_type_preset(node_presets, &node.type_name);
        let preset_provider_id = preset
            .map(|preset| preset.provider_id.trim())
            .filter(|value| !value.is_empty());
        let default_provider_id = (!node_presets.default_provider_id.trim().is_empty())
            .then(|| node_presets.default_provider_id.trim());
        // 任一节点级字段出现即进入显式覆盖层；缺失的另一半只从项目默认补齐，
        // 不能再拼入可能属于其它 Provider 的节点类型预设。
        let preferred_provider_id = configured_provider_id.or_else(|| {
            if configured_model_id.is_some() {
                default_provider_id
            } else {
                preset_provider_id.or(default_provider_id)
            }
        });
        let provider_config = match preferred_provider_id {
            Some(provider_id) => project_config
                .providers
                .providers
                .iter()
                .find(|provider| provider.provider_id == provider_id)
                .ok_or_else(|| {
                    CommandError::not_found(format!(
                        "{} node {} references an unconfigured provider: {provider_id}",
                        node.type_name,
                        node.id.as_str()
                    ))
                })?
                .clone(),
            None => select_llm_provider(&project_config.providers).map_err(|error| {
                    CommandError::validation(format!(
                        "{} node {} has no provider_id and the project default cannot be resolved: {error}",
                        node.type_name,
                        node.id.as_str()
                    ))
                })?,
        };
        if !provider_config.enabled {
            return Err(CommandError::validation(format!(
                "{} node {} references a disabled provider: {}",
                node.type_name,
                node.id.as_str(),
                provider_config.provider_id
            )));
        }

        let preset_model_id = preset
            .map(|preset| preset.model_id.trim())
            .filter(|value| !value.is_empty());
        let default_model_id = (!node_presets.default_model_id.trim().is_empty())
            .then(|| node_presets.default_model_id.trim());
        let preferred_model_id = configured_model_id.or_else(|| {
            if configured_provider_id.is_some() {
                None
            } else {
                preset_model_id.or(default_model_id)
            }
        });
        let model_config = if let Some(model_id) = preferred_model_id {
            provider_config
                .models
                .iter()
                .find(|model| model.model_id == model_id)
                .ok_or_else(|| {
                    CommandError::not_found(format!(
                        "{} node {} references an unconfigured model: {}/{}",
                        node.type_name,
                        node.id.as_str(),
                        provider_config.provider_id,
                        model_id
                    ))
                })?
                .clone()
        } else {
            select_llm_model(&provider_config)?
        };
        if !matches!(
            model_config.capability,
            ProviderCapability::Llm | ProviderCapability::ToolUse
        ) {
            return Err(CommandError::validation(format!(
                "{} node {} model is not LLM-capable: {}/{}",
                node.type_name,
                node.id.as_str(),
                provider_config.provider_id,
                model_config.model_id
            )));
        }

        if !providers.contains_key(&provider_config.provider_id) {
            let api_key = provider_api_key(project_root, secrets, &provider_config)?;
            let provider = OpenAiCompatibleLlmProvider::new(provider_config.clone(), api_key)
                .map_err(error_to_string)?;
            providers.insert(provider_config.provider_id.clone(), provider);
        }
        node_routes.insert(
            node.id.as_str().to_owned(),
            WorkflowLlmRoute {
                provider_id: provider_config.provider_id.clone(),
                model_id: model_config.model_id,
            },
        );
    }

    Ok(WorkflowLlmExecutionPlan {
        node_routes,
        providers,
    })
}

fn route_workflow_llm_request<'a>(
    mut request: crate::workflow::WorkflowNodeExecutionRequest,
    routes: &'a BTreeMap<String, WorkflowLlmRoute>,
    providers: &'a BTreeMap<String, OpenAiCompatibleLlmProvider>,
) -> CoreResult<(
    crate::workflow::WorkflowNodeExecutionRequest,
    &'a OpenAiCompatibleLlmProvider,
    &'a WorkflowLlmRoute,
)> {
    let route = routes.get(request.node_id.as_str()).ok_or_else(|| {
        CoreError::validation(format!(
            "frozen LLM route is missing for workflow node {}",
            request.node_id.as_str()
        ))
    })?;
    let provider = providers.get(&route.provider_id).ok_or_else(|| {
        CoreError::validation(format!(
            "frozen LLM provider is missing for workflow node {}: {}",
            request.node_id.as_str(),
            route.provider_id
        ))
    })?;
    let config = request.config.as_object_mut().ok_or_else(|| {
        CoreError::validation(format!(
            "workflow node {} LLM config must be an object",
            request.node_id.as_str()
        ))
    })?;
    config.insert("provider_id".to_owned(), json!(route.provider_id));
    config.insert("model_id".to_owned(), json!(route.model_id));
    Ok((request, provider, route))
}

fn workflow_search_tool_for_node(
    type_name: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    let capability = execution_tool_capability(type_name)?;
    Some((
        capability.tool_scope,
        capability.project_search_tool?,
        capability.project_search_description?,
    ))
}

fn workflow_search_tool_enabled(
    controls: &BTreeMap<String, BTreeMap<String, Option<bool>>>,
    presets: &NodePresetSettings,
    type_name: &str,
) -> bool {
    workflow_search_tool_for_node(type_name).is_some_and(|(scope, tool, _)| {
        node_tool_control_enabled(controls, presets, type_name, scope, tool)
    })
}

fn workflow_web_search_tool_for_node(
    type_name: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    let capability = execution_tool_capability(type_name)?;
    Some((
        capability.tool_scope,
        capability.web_search_tool?,
        capability.web_search_description?,
    ))
}

fn workflow_web_search_tool_enabled(
    controls: &BTreeMap<String, BTreeMap<String, Option<bool>>>,
    permissions: &crate::config::PermissionsConfig,
    presets: &NodePresetSettings,
    type_name: &str,
) -> bool {
    let policy = permission_policy_for_node(permissions, presets, type_name);
    policy.evaluate(&PermissionRequest::WebSearch).allowed
        && workflow_web_search_tool_for_node(type_name).is_some_and(|(scope, tool, _)| {
            node_tool_control_enabled(controls, presets, type_name, scope, tool)
        })
}

fn workflow_requires_project_retrieval(
    dependencies: &WorkflowExecutionDependencySet,
    controls: &BTreeMap<String, BTreeMap<String, Option<bool>>>,
    presets: &NodePresetSettings,
    adapter_uses_llm: bool,
) -> bool {
    dependencies.node_types().iter().any(|type_name| {
        workflow_node_catalog_entry(type_name).is_some_and(|entry| entry.config_kind == "search")
    }) || dependencies
        .node_types()
        .iter()
        .filter(|type_name| !type_name.starts_with(EXECUTOR_ADAPTER_NODE_PREFIX))
        .any(|type_name| workflow_search_tool_enabled(controls, presets, type_name))
        || (adapter_uses_llm
            && tool_control_enabled(controls, "executor_adapter", EXECUTOR_ADAPTER_SEARCH_TOOL))
}

fn workflow_requires_web_search(
    dependencies: &WorkflowExecutionDependencySet,
    controls: &BTreeMap<String, BTreeMap<String, Option<bool>>>,
    permissions: &crate::config::PermissionsConfig,
    presets: &NodePresetSettings,
    adapter_uses_llm: bool,
) -> bool {
    dependencies
        .node_types()
        .iter()
        .filter(|type_name| !type_name.starts_with(EXECUTOR_ADAPTER_NODE_PREFIX))
        .any(|type_name| {
            workflow_web_search_tool_enabled(controls, permissions, presets, type_name)
        })
        || (adapter_uses_llm
            && permission_policy_for_scope(permissions, "workflow_nodes")
                .evaluate(&PermissionRequest::WebSearch)
                .allowed
            && tool_control_enabled(
                controls,
                "executor_adapter",
                EXECUTOR_ADAPTER_WEB_SEARCH_TOOL,
            ))
}

pub fn get_budget_status_impl(project_root: &Path) -> CommandResult<BudgetStatus> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "budget_status")?;
    let config_store = ConfigStore::new(project_root);
    let config = config_store.load_or_create().map_err(error_to_string)?;
    let budget_config = read_budget_config(project_root)?;
    let ledger = SqliteCostLedger::open(project_root).map_err(error_to_string)?;
    let spent_usd = ledger
        .total_cost(&CostQuery::default())
        .map_err(error_to_string)?;
    Ok(BudgetStatus {
        budget_usd: budget_config.budget_usd,
        spent_usd,
        preauthorized_usd: config.auto_mode.preauthorized_budget_usd.unwrap_or(0.0),
        auto_mode_enabled: config.auto_mode.enabled_by_default,
    })
}

pub fn get_app_settings_impl(project_root: &Path) -> CommandResult<AppSettings> {
    validate_project_root(project_root)?;
    ensure_project_not_in_maintenance(project_root)?;
    validate_initialized_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "app_settings_read")?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(AppSettings { app: config.app })
}

pub fn save_app_settings_impl(project_root: &Path, settings: AppSettings) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "app_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    apply_app_settings(&mut config, settings.app)?;
    crate::config::ProjectLayout::from_app(project_root, &config.app)
        .and_then(|layout| layout.create_configured_directories())
        .map_err(error_to_string)?;
    config_store.save(&config).map_err(error_to_string)
}

fn apply_app_settings(
    config: &mut crate::config::ProjectConfig,
    app: crate::config::AppConfig,
) -> CommandResult<()> {
    config.app = app;
    config.app.project_name =
        non_empty_or("project_name", std::mem::take(&mut config.app.project_name))?;
    config.app.documents_dir = non_empty_or(
        "documents_dir",
        std::mem::take(&mut config.app.documents_dir),
    )?;
    config.app.workflows_dir = non_empty_or(
        "workflows_dir",
        std::mem::take(&mut config.app.workflows_dir),
    )?;
    config.app.skills_dir = non_empty_or("skills_dir", std::mem::take(&mut config.app.skills_dir))?;
    config.app.exports_dir =
        non_empty_or("exports_dir", std::mem::take(&mut config.app.exports_dir))?;
    config
        .app
        .normalize_directories()
        .map_err(error_to_string)?;
    config.app.validate().map_err(error_to_string)
}

pub fn commit_general_settings_files(
    project_root: &Path,
    app_state_root: &Path,
    config: &crate::config::ProjectConfig,
    project_memory: &[u8],
) -> CommandResult<()> {
    commit_general_settings_files_with_fail_after(
        project_root,
        app_state_root,
        config,
        project_memory,
        None,
    )
}

pub fn commit_general_settings_files_with_fail_after(
    project_root: &Path,
    app_state_root: &Path,
    config: &crate::config::ProjectConfig,
    project_memory: &[u8],
    fail_after: Option<usize>,
) -> CommandResult<()> {
    fn yaml_bytes<T: serde::Serialize>(value: &T) -> CommandResult<Vec<u8>> {
        let value = yaml_serde::to_value(value).map_err(error_to_string)?;
        Ok(yaml_serde::to_string(&value)
            .map_err(error_to_string)?
            .into_bytes())
    }
    let project_providers = ProviderCatalogStore::default_for_app(app_state_root)
        .read()
        .map_err(error_to_string)?
        .project_projection(&config.providers);
    let files = vec![
        (
            crate::config::AtomicCommitTarget::App,
            yaml_bytes(&config.app)?,
        ),
        (
            crate::config::AtomicCommitTarget::Providers,
            yaml_bytes(&project_providers)?,
        ),
        (
            crate::config::AtomicCommitTarget::Permissions,
            yaml_bytes(&PermissionsConfig::default())?,
        ),
        (
            crate::config::AtomicCommitTarget::Rag,
            yaml_bytes(&config.rag)?,
        ),
        (
            crate::config::AtomicCommitTarget::Workflow,
            yaml_bytes(&config.workflow)?,
        ),
        (
            crate::config::AtomicCommitTarget::Git,
            yaml_bytes(&config.git)?,
        ),
        (
            crate::config::AtomicCommitTarget::AutoMode,
            yaml_bytes(&config.auto_mode)?,
        ),
        (
            crate::config::AtomicCommitTarget::ProjectMemory,
            project_memory.to_vec(),
        ),
    ];
    crate::config::atomic_commit::commit_files_with_fail_after(
        project_root,
        app_state_root,
        crate::config::AtomicCommitProfile::GeneralSettings,
        &files,
        fail_after,
    )
    .map_err(error_to_string)
}

pub fn get_rag_settings_impl(project_root: &Path) -> CommandResult<RagSettings> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "rag_settings_read")?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(RagSettings { rag: config.rag })
}

pub fn save_rag_settings_impl(project_root: &Path, settings: RagSettings) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "rag_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    settings.rag.validate().map_err(error_to_string)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.rag = settings.rag;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_workflow_settings_impl(project_root: &Path) -> CommandResult<WorkflowSettings> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "workflow_settings_read")?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(WorkflowSettings {
        workflow: config.workflow,
    })
}

pub fn save_workflow_settings_impl(
    project_root: &Path,
    settings: WorkflowSettings,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "workflow_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    settings.workflow.validate().map_err(error_to_string)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.workflow = settings.workflow;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_git_settings_impl(project_root: &Path) -> CommandResult<GitSettings> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "git_settings_read")?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(GitSettings { git: config.git })
}

pub fn save_git_settings_impl(project_root: &Path, settings: GitSettings) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "git_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.git = settings.git;
    config
        .git
        .normalize_ignored_paths()
        .map_err(error_to_string)?;
    config.git.validate().map_err(error_to_string)?;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_template_repository_settings_impl(
    settings_root: &Path,
) -> CommandResult<TemplateRepositorySettings> {
    let path = template_repository_settings_path(settings_root);
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let settings = serde_json::from_str::<TemplateRepositorySettings>(&content)
                .map_err(error_to_string)?;
            if settings.base_url.trim().is_empty() {
                Ok(TemplateRepositorySettings::default())
            } else {
                Ok(settings)
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(TemplateRepositorySettings::default())
        }
        Err(error) => Err(error_to_string(error)),
    }
}

pub fn save_template_repository_settings_impl(
    settings_root: &Path,
    settings: &TemplateRepositorySettings,
) -> CommandResult<()> {
    if settings.base_url.trim().is_empty() {
        return Err(CommandError::validation(
            "template repository base_url cannot be empty",
        ));
    }
    validate_template_url(&settings.base_url)?;
    let path = template_repository_settings_path(settings_root);
    let body = serde_json::to_string_pretty(settings).map_err(error_to_string)?;
    crate::config::store::atomic_write(&path, body.as_bytes()).map_err(error_to_string)
}

pub fn get_display_name_language_pack_template(
    target_language: Option<String>,
) -> CommandResult<DisplayNameLanguagePackTemplate> {
    let target_language = normalize_language_pack_code(target_language.as_deref())?;
    let entries = crate::rag::resources::load_display_name_resources().map_err(error_to_string)?;
    Ok(DisplayNameLanguagePackTemplate {
        target_language: target_language.clone(),
        base_language: "zh".to_owned(),
        fallback_language: "zh".to_owned(),
        output_file_name: display_name_language_pack_file_name(&target_language),
        source_file_name: "display_name.json".to_owned(),
        instructions: vec![
            "Translate every value from Simplified Chinese into the target UI language.".to_owned(),
            "Keep every JSON key unchanged; do not add, remove, rename, or reorder keys unless the caller explicitly asks.".to_owned(),
            "Keep placeholders such as {name}, {count}, {{input.xxx}}, paths, command names, and model/provider IDs unchanged.".to_owned(),
            "Return valid UTF-8 JSON object content for the output file only.".to_owned(),
        ],
        entries,
    })
}

pub fn validate_display_name_language_pack(
    target_language: Option<String>,
    overlay: BTreeMap<String, String>,
) -> CommandResult<DisplayNameLanguagePackValidation> {
    let target_language = normalize_language_pack_code(target_language.as_deref())?;
    let base = crate::rag::resources::load_display_name_resources().map_err(error_to_string)?;

    let mut missing_keys = Vec::new();
    let mut empty_keys = Vec::new();
    let mut translated_keys = 0usize;
    for key in base.keys() {
        match overlay.get(key) {
            Some(value) if value.trim().is_empty() => empty_keys.push(key.clone()),
            Some(_) => translated_keys += 1,
            None => missing_keys.push(key.clone()),
        }
    }

    let extra_keys = overlay
        .keys()
        .filter(|key| !key.starts_with('_') && !base.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    let complete = missing_keys.is_empty() && empty_keys.is_empty() && extra_keys.is_empty();

    Ok(DisplayNameLanguagePackValidation {
        target_language: target_language.clone(),
        output_file_name: display_name_language_pack_file_name(&target_language),
        total_keys: base.len(),
        translated_keys,
        missing_keys,
        empty_keys,
        extra_keys,
        complete,
    })
}

fn normalize_language_pack_code(lang_code: Option<&str>) -> CommandResult<String> {
    let raw = lang_code.unwrap_or("en").trim().replace('_', "-");
    let mut lang = raw.to_lowercase();
    if lang.is_empty() {
        lang = "en".to_owned();
    }
    if lang == "jp" || lang.starts_with("jp-") {
        lang = lang.replacen("jp", "ja", 1);
    }
    validate_language_pack_code(&lang)?;
    Ok(lang)
}

fn validate_language_pack_code(lang: &str) -> CommandResult<()> {
    if lang == "." || lang == ".." || lang.starts_with('-') || lang.ends_with('-') {
        return Err(CommandError::validation(format!(
            "invalid language code: {lang}"
        )));
    }
    if lang
        .split('-')
        .any(|part| part.is_empty() || !part.chars().all(|ch| ch.is_ascii_alphanumeric()))
    {
        return Err(CommandError::validation(format!(
            "invalid language code: {lang}"
        )));
    }
    Ok(())
}

fn display_name_language_pack_file_name(lang: &str) -> String {
    if lang == "zh" {
        "display_name.json".to_owned()
    } else {
        format!("display_name.{lang}.json")
    }
}

pub fn update_budget_config_impl(
    project_root: &Path,
    budget_usd: f64,
    preauthorized_usd: f64,
) -> CommandResult<()> {
    let _project_mutation = acquire_project_mutation_guard(project_root, "budget_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    validate_money("budget_usd", budget_usd)?;
    validate_money("preauthorized_usd", preauthorized_usd)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    // 0.0 表示无限制（映射为 None），避免 exceeds(Some(0.0), positive) 阻断所有调用
    config.auto_mode.preauthorized_budget_usd = if preauthorized_usd > 0.0 {
        Some(preauthorized_usd)
    } else {
        None
    };
    config_store.save(&config).map_err(error_to_string)?;
    write_budget_config(project_root, &BudgetConfigFile { budget_usd })
}

pub fn set_auto_mode_impl(project_root: &Path, enabled: bool) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "auto_mode_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.auto_mode.enabled_by_default = enabled;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_automation_settings_impl(project_root: &Path) -> CommandResult<AutomationSettings> {
    validate_project_root(project_root)?;
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "automation_settings_read")?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let budget = get_budget_status_impl(project_root)?;
    let stored = read_confirmation_policy_settings(project_root).map_err(error_to_string)?;
    let policies = merge_confirmation_policy_settings(
        stored.as_deref(),
        &config.auto_mode.available_approval_prompts,
    )?;
    Ok(AutomationSettings {
        budget,
        confirmation_policies: policies,
    })
}

pub fn save_automation_settings_impl(
    project_root: &Path,
    settings: AutomationSettings,
) -> CommandResult<()> {
    save_automation_settings_impl_with_app_state(
        project_root,
        &crate::config::trusted_app_state_for_project(project_root),
        settings,
    )
}

fn save_automation_settings_impl_with_app_state(
    project_root: &Path,
    app_state_root: &Path,
    settings: AutomationSettings,
) -> CommandResult<()> {
    save_automation_settings_impl_with_app_state_and_workflow(
        project_root,
        app_state_root,
        settings,
        None,
    )
}

fn save_automation_settings_impl_with_app_state_and_workflow(
    project_root: &Path,
    app_state_root: &Path,
    settings: AutomationSettings,
    workflow: Option<crate::config::WorkflowConfig>,
) -> CommandResult<()> {
    // N2: single prepare → commit boundary. Build full in-memory snapshot, then atomic writes only.
    validate_project_root(project_root)?;
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "automation_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    validate_money("budget_usd", settings.budget.budget_usd)?;
    validate_money("preauthorized_usd", settings.budget.preauthorized_usd)?;

    let config_store = ConfigStore::with_app_state(project_root, app_state_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    if let Some(workflow) = workflow {
        workflow.validate().map_err(error_to_string)?;
        config.workflow = workflow;
    }
    config.auto_mode.preauthorized_budget_usd = if settings.budget.preauthorized_usd > 0.0 {
        Some(settings.budget.preauthorized_usd)
    } else {
        None
    };
    config.auto_mode.enabled_by_default = settings.budget.auto_mode_enabled;

    let mut normalized_settings = Vec::new();
    let allowed = confirmation_policy_keys();
    for mut setting in settings.confirmation_policies {
        if allowed.contains(&setting.confirmation_kind.as_str()) {
            setting.approval_prompt = setting.approval_prompt.trim().to_owned();
            if setting.auto_mode_policy == ConfirmationAutoModePolicy::AutoApproval
                && setting.approval_prompt.is_empty()
            {
                return Err(CommandError::validation(format!(
                    "auto approval prompt cannot be empty: {}",
                    setting.confirmation_kind
                )));
            }
            let policy = approval_policy_from_ui(&policy_code_from_dual_policy(
                setting.normal_policy,
                setting.auto_mode_policy,
            ))?;
            let prompt = ensure_approval_prompt(
                &mut config.auto_mode.available_approval_prompts,
                &setting.confirmation_kind,
            );
            prompt.default_policy = policy;
            if !setting.approval_prompt.is_empty() {
                prompt.prompt = setting.approval_prompt.clone();
            }
        }
        normalized_settings.push(setting);
    }

    let budget_body = serde_json::to_string_pretty(&BudgetConfigFile {
        budget_usd: settings.budget.budget_usd,
    })
    .map_err(error_to_string)?;
    let policies_body =
        serde_json::to_string_pretty(&normalized_settings).map_err(error_to_string)?;

    // Single multi-file commit boundary (N2): all config YAML + budget + policies
    // stage together, journaled, then renamed. Mid-crash recovers via journal.
    commit_automation_settings_files(
        project_root,
        app_state_root,
        &config,
        budget_body.as_bytes(),
        policies_body.as_bytes(),
    )
}

/// Build the full file set for automation settings and commit atomically (testable).
pub fn commit_automation_settings_files(
    project_root: &Path,
    app_state_root: &Path,
    config: &crate::config::ProjectConfig,
    budget_json: &[u8],
    policies_json: &[u8],
) -> CommandResult<()> {
    commit_automation_settings_files_with_fail_after(
        project_root,
        app_state_root,
        config,
        budget_json,
        policies_json,
        None,
    )
}

/// `fail_after`: test-only injection — fail after N successful renames (see atomic_commit).
pub fn commit_automation_settings_files_with_fail_after(
    project_root: &Path,
    app_state_root: &Path,
    config: &crate::config::ProjectConfig,
    budget_json: &[u8],
    policies_json: &[u8],
    fail_after: Option<usize>,
) -> CommandResult<()> {
    fn yaml_bytes<T: serde::Serialize>(value: &T) -> CommandResult<Vec<u8>> {
        let v = yaml_serde::to_value(value).map_err(error_to_string)?;
        let s = yaml_serde::to_string(&v).map_err(error_to_string)?;
        Ok(s.into_bytes())
    }
    let project_providers = ProviderCatalogStore::default_for_app(app_state_root)
        .read()
        .map_err(error_to_string)?
        .project_projection(&config.providers);
    let files: Vec<(crate::config::AtomicCommitTarget, Vec<u8>)> = vec![
        (
            crate::config::AtomicCommitTarget::App,
            yaml_bytes(&config.app)?,
        ),
        (
            crate::config::AtomicCommitTarget::Providers,
            yaml_bytes(&project_providers)?,
        ),
        (
            crate::config::AtomicCommitTarget::Permissions,
            yaml_bytes(&PermissionsConfig::default())?,
        ),
        (
            crate::config::AtomicCommitTarget::Rag,
            yaml_bytes(&config.rag)?,
        ),
        (
            crate::config::AtomicCommitTarget::Workflow,
            yaml_bytes(&config.workflow)?,
        ),
        (
            crate::config::AtomicCommitTarget::Git,
            yaml_bytes(&config.git)?,
        ),
        (
            crate::config::AtomicCommitTarget::AutoMode,
            yaml_bytes(&config.auto_mode)?,
        ),
        (
            crate::config::AtomicCommitTarget::Budget,
            budget_json.to_vec(),
        ),
        (
            crate::config::AtomicCommitTarget::ConfirmationPolicies,
            policies_json.to_vec(),
        ),
    ];
    crate::config::atomic_commit::commit_files_with_fail_after(
        project_root,
        app_state_root,
        crate::config::AtomicCommitProfile::AutomationSettings,
        &files,
        fail_after,
    )
    .map_err(error_to_string)
}

pub fn get_permissions_settings_impl(project_root: &Path) -> CommandResult<PermissionsSettings> {
    let app_state_root = crate::config::trusted_app_state_for_project(project_root);
    get_permissions_settings_impl_with_app_state(project_root, &app_state_root)
}

fn get_permissions_settings_impl_with_app_state(
    project_root: &Path,
    app_state_root: &Path,
) -> CommandResult<PermissionsSettings> {
    validate_project_root(project_root)?;
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "permissions_settings_read")?;
    let permissions =
        AppPermissionsStore::read_global_or_migrate(app_state_root, Some(project_root))
            .map_err(error_to_string)?;
    Ok(permissions_settings_from_config(permissions))
}

pub fn save_permissions_settings_impl(
    project_root: &Path,
    settings: PermissionsSettings,
) -> CommandResult<()> {
    let app_state_root = crate::config::trusted_app_state_for_project(project_root);
    save_permissions_settings_impl_with_app_state(project_root, &app_state_root, settings)
}

fn save_permissions_settings_impl_with_app_state(
    project_root: &Path,
    app_state_root: &Path,
    settings: PermissionsSettings,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "permissions_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let permissions = permissions_config_from_settings(settings)?;
    AppPermissionsStore::default_for_app(app_state_root)
        .write(&permissions)
        .map_err(error_to_string)
}

fn permissions_config_from_settings(
    settings: PermissionsSettings,
) -> CommandResult<PermissionsConfig> {
    let permissions = PermissionsConfig {
        schema_version: crate::config::current_schema_version(),
        policy: settings.policy,
        scoped_policies: normalize_scoped_permission_policies(settings.scoped_policies),
        tool_controls: normalize_tool_controls(settings.tool_controls),
    };
    permissions.validate().map_err(error_to_string)?;
    Ok(permissions)
}

fn permissions_settings_from_config(permissions: PermissionsConfig) -> PermissionsSettings {
    PermissionsSettings {
        policy: permissions.policy,
        scoped_policies: normalize_scoped_permission_policies(permissions.scoped_policies),
        tool_controls: normalize_tool_controls(permissions.tool_controls),
    }
}

fn normalize_tool_controls(
    mut controls: BTreeMap<String, BTreeMap<String, Option<bool>>>,
) -> BTreeMap<String, BTreeMap<String, Option<bool>>> {
    for (scope, defaults) in default_permission_tool_controls() {
        let scope_controls = controls.entry(scope).or_default();
        for (tool, enabled) in defaults {
            scope_controls.entry(tool).or_insert(enabled);
        }
    }
    controls
}

fn normalize_scoped_permission_policies(
    mut policies: BTreeMap<String, Option<PermissionPolicy>>,
) -> BTreeMap<String, Option<PermissionPolicy>> {
    for scope in ["workflow_nodes", "project_ai"] {
        policies.entry(scope.to_owned()).or_insert(None);
    }
    policies
}

fn permission_policy_for_scope(
    permissions: &crate::config::PermissionsConfig,
    scope: &str,
) -> PermissionPolicy {
    permissions
        .scoped_policies
        .get(scope)
        .and_then(Clone::clone)
        .unwrap_or_else(|| permissions.policy.clone())
}

fn node_type_preset<'a>(
    settings: &'a NodePresetSettings,
    type_name: &str,
) -> Option<&'a NodeTypePreset> {
    let canonical_type = workflow_node_catalog_entry(type_name)
        .map(|entry| entry.preset_type.as_str())
        .unwrap_or(type_name);
    settings
        .presets
        .iter()
        .find(|preset| preset.node_type == canonical_type)
}

fn permission_policy_for_node(
    permissions: &crate::config::PermissionsConfig,
    presets: &NodePresetSettings,
    type_name: &str,
) -> PermissionPolicy {
    node_type_preset(presets, type_name)
        .and_then(|preset| preset.permission_policy.clone())
        .unwrap_or_else(|| permission_policy_for_scope(permissions, "workflow_nodes"))
}

fn apply_node_type_preset(
    mut request: crate::workflow::WorkflowNodeExecutionRequest,
    preset: Option<&NodeTypePreset>,
) -> crate::workflow::WorkflowNodeExecutionRequest {
    let Some(preset) = preset else {
        return request;
    };
    let Some(config) = request.config.as_object_mut() else {
        return request;
    };

    let provider_is_missing = config
        .get("provider_id")
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty());
    let model_is_missing = config
        .get("model_id")
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty());
    if provider_is_missing && model_is_missing && !preset.provider_id.trim().is_empty() {
        config.insert("provider_id".to_owned(), json!(preset.provider_id));
    }
    if provider_is_missing && model_is_missing && !preset.model_id.trim().is_empty() {
        config.insert("model_id".to_owned(), json!(preset.model_id));
    }

    let timeout_is_missing = config
        .get("timeout_ms")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
        })
        .is_none_or(|value| value == 0);
    if timeout_is_missing {
        config.insert("timeout_ms".to_owned(), json!(preset.timeout_ms));
    }

    let budget_is_missing = ["budget_usd", "single_call_budget_usd"]
        .iter()
        .all(|field| config.get(*field).is_none_or(Value::is_null));
    if budget_is_missing {
        config.insert("budget_usd".to_owned(), json!(preset.budget_usd));
    }
    request
}

pub fn resolve_confirmation_impl(
    project_root: &Path,
    request: ResolveConfirmationRequest,
) -> CommandResult<ResolveConfirmationResult> {
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "confirmation_resolution")?;
    let (result, lease) = resolve_confirmation_impl_with_claim(project_root, request)?;
    if let Some(lease) = lease {
        let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
        store
            .release_worker_lease(
                &lease.workflow_id,
                &lease.run_id,
                &lease.owner_id,
                lease.generation,
            )
            .map_err(error_to_string)?;
    }
    Ok(result)
}

fn resolve_confirmation_impl_with_claim(
    project_root: &Path,
    request: ResolveConfirmationRequest,
) -> CommandResult<(ResolveConfirmationResult, Option<WorkflowWorkerLease>)> {
    validate_project_root(project_root)?;
    if request.workflow_id.trim().is_empty() {
        return Err(CommandError::validation("workflow_id cannot be empty"));
    }
    if request.run_id.trim().is_empty() {
        return Err(CommandError::validation("run_id cannot be empty"));
    }
    if request.confirmation_id.trim().is_empty() {
        return Err(CommandError::validation("confirmation_id cannot be empty"));
    }

    let workflow_id = WorkflowId::from(request.workflow_id.clone());
    let run_id = RunId::from(request.run_id.clone());
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let knowledge_store =
        crate::rag::SqliteWritingKnowledgeStore::open(project_root).map_err(error_to_string)?;
    let knowledge_required = knowledge_store
        .has_confirmation(&request.confirmation_id)
        .map_err(error_to_string)?;
    let decision = match request.decision {
        ConfirmationDecision::Approve => crate::workflow::ConfirmationResolutionDecision::Approve,
        ConfirmationDecision::Reject => crate::workflow::ConfirmationResolutionDecision::Reject,
    };
    let operation_id = confirmation_resolution_operation_id(
        &request.workflow_id,
        &request.run_id,
        &request.confirmation_id,
    );
    let request_hash = confirmation_resolution_request_hash(&request)?;
    let now_ms = workflow_lease_now_ms()?;
    let mut operation = store
        .prepare_confirmation_resolution(
            &operation_id,
            &workflow_id,
            &run_id,
            &request.confirmation_id,
            decision,
            request.review_reason.as_deref(),
            &request_hash,
            knowledge_required,
            now_ms,
        )
        .map_err(error_to_string)?;

    if operation.status == crate::workflow::ConfirmationResolutionStatus::Prepared {
        let response = json!({
            "workflow_id": request.workflow_id,
            "run_id": request.run_id,
            "confirmation_id": request.confirmation_id,
            "decision": request.decision,
        });
        let applied = knowledge_store
            .resolve_confirmation_with_operation(
                &request.confirmation_id,
                match request.decision {
                    ConfirmationDecision::Approve => crate::rag::ConfirmationState::Approved,
                    ConfirmationDecision::Reject => crate::rag::ConfirmationState::Rejected,
                },
                &operation_id,
                &request_hash,
                &response,
            )
            .map_err(error_to_string)?;
        if !applied {
            return Err(CommandError::not_found(format!(
                "knowledge confirmation disappeared after durable prepare: {}",
                request.confirmation_id
            )));
        }
        operation = store
            .mark_confirmation_knowledge_committed(&operation_id, &request_hash, now_ms)
            .map_err(error_to_string)?;
    }

    let runtime_state = match operation.status {
        crate::workflow::ConfirmationResolutionStatus::KnowledgeCommitted => match store
            .commit_confirmation_resolution(&operation_id, now_ms)
            .map_err(error_to_string)?
        {
            crate::workflow::ConfirmationResolutionCommitResult::Saved { state }
            | crate::workflow::ConfirmationResolutionCommitResult::AlreadyCommitted { state } => {
                state
            }
            crate::workflow::ConfirmationResolutionCommitResult::NotFound => {
                return Err(CommandError::not_found(format!(
                    "workflow run not found after durable confirmation knowledge commit; knowledge receipt preserved for explicit recovery: {}/{}",
                    request.workflow_id, request.run_id
                )));
            }
        },
        crate::workflow::ConfirmationResolutionStatus::Committed => store
            .load_state(&workflow_id, &run_id)
            .map_err(error_to_string)?
            .ok_or_else(|| {
                CommandError::not_found(format!(
                    "workflow run not found: {}/{}",
                    workflow_id.as_str(),
                    run_id.as_str()
                ))
            })?,
        crate::workflow::ConfirmationResolutionStatus::Prepared => {
            return Err(CommandError::internal(
                "confirmation knowledge commit did not advance",
            ));
        }
    };

    let runtime_confirmation = runtime_state
        .confirmations
        .get(&request.confirmation_id)
        .ok_or_else(|| {
            CommandError::not_found(format!(
                "confirmation item not found: {}",
                request.confirmation_id
            ))
        })?;
    let confirmation = confirmation_log_entry_from_runtime(
        runtime_confirmation,
        request.review_reason.as_deref(),
        &request.workflow_id,
        &request.run_id,
    );
    let owner_id = format!("worker-{}", new_run_id()?.as_str());
    let lease = match store
        .claim_resume(
            &workflow_id,
            &run_id,
            &owner_id,
            workflow_lease_now_ms()?,
            WORKFLOW_WORKER_LEASE_TTL_MS,
        )
        .map_err(error_to_string)?
    {
        crate::workflow::WorkflowResumeClaimResult::Claimed { lease, .. } => Some(lease),
        crate::workflow::WorkflowResumeClaimResult::Busy { .. }
        | crate::workflow::WorkflowResumeClaimResult::NotResumable { .. } => None,
        crate::workflow::WorkflowResumeClaimResult::NotFound => {
            return Err(CommandError::not_found(format!(
                "workflow run not found: {}/{}",
                workflow_id.as_str(),
                run_id.as_str()
            )));
        }
    };

    // F14-b：日志/徽标是可重建投影。领域提交成功后，投影失败仅保留 outbox，
    // 不能把已提交的确认伪装成失败，也不能阻止生产入口把 lease 交给 worker。
    let projection_result = FileConfirmationLogStore::default_for_project(project_root)
        .record(confirmation.clone())
        .map_err(error_to_string)
        .and_then(|_| {
            store
                .mark_confirmation_resolution_projected(&operation_id, now_ms)
                .map_err(error_to_string)
        });
    if let Err(error) = projection_result {
        eprintln!("[ariadne] confirmation projection deferred for {operation_id}: {error}");
    }
    let pending_confirmations = list_pending_confirmations_impl(project_root)
        .map(|items| items.len())
        .unwrap_or_else(|error| {
            eprintln!(
                "[ariadne] confirmation badge refresh deferred after {operation_id}: {error}"
            );
            runtime_state
                .confirmations
                .values()
                .filter(|item| item.state == RuntimeConfirmationState::Pending)
                .count()
        });
    let mut badges = UiRunLogStore::default_for_project(project_root)
        .badge_counts(None, None)
        .unwrap_or_else(|error| {
            eprintln!("[ariadne] run-log badge refresh deferred after {operation_id}: {error}");
            SidebarBadgeCounts::default()
        });
    badges.confirmations = u32::try_from(pending_confirmations).unwrap_or(u32::MAX);

    Ok((
        ResolveConfirmationResult {
            workflow: WorkflowActionResult {
                workflow_id: request.workflow_id,
                run_id: request.run_id,
                status: run_status_label(runtime_state.status).to_owned(),
            },
            confirmation,
            badges,
        },
        lease,
    ))
}

/// F14：打开项目时前向恢复 knowledge/runtime 跨库确认 saga，并重放日志投影。
fn recover_confirmation_resolution_sagas(project_root: &Path) -> CommandResult<usize> {
    let runtime_store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let knowledge_store =
        crate::rag::SqliteWritingKnowledgeStore::open(project_root).map_err(error_to_string)?;
    let mut recovered = 0usize;
    for mut operation in runtime_store
        .list_recoverable_confirmation_resolutions()
        .map_err(error_to_string)?
    {
        let now_ms = workflow_lease_now_ms()?;
        if operation.status == crate::workflow::ConfirmationResolutionStatus::Prepared {
            let response = json!({
                "workflow_id": operation.workflow_id,
                "run_id": operation.run_id,
                "confirmation_id": operation.confirmation_id,
                "decision": match operation.decision {
                    crate::workflow::ConfirmationResolutionDecision::Approve => "approve",
                    crate::workflow::ConfirmationResolutionDecision::Reject => "reject",
                },
            });
            let applied = knowledge_store
                .resolve_confirmation_with_operation(
                    &operation.confirmation_id,
                    match operation.decision {
                        crate::workflow::ConfirmationResolutionDecision::Approve => {
                            crate::rag::ConfirmationState::Approved
                        }
                        crate::workflow::ConfirmationResolutionDecision::Reject => {
                            crate::rag::ConfirmationState::Rejected
                        }
                    },
                    &operation.operation_id,
                    &operation.request_hash,
                    &response,
                )
                .map_err(error_to_string)?;
            if !applied {
                return Err(CommandError::not_found(format!(
                    "knowledge confirmation missing during saga recovery: {}",
                    operation.confirmation_id
                )));
            }
            operation = runtime_store
                .mark_confirmation_knowledge_committed(
                    &operation.operation_id,
                    &operation.request_hash,
                    now_ms,
                )
                .map_err(error_to_string)?;
        }
        if operation.status == crate::workflow::ConfirmationResolutionStatus::KnowledgeCommitted {
            match runtime_store
                .commit_confirmation_resolution(&operation.operation_id, now_ms)
                .map_err(error_to_string)?
            {
                crate::workflow::ConfirmationResolutionCommitResult::Saved { .. }
                | crate::workflow::ConfirmationResolutionCommitResult::AlreadyCommitted {
                    ..
                } => {}
                crate::workflow::ConfirmationResolutionCommitResult::NotFound => {
                    return Err(CommandError::not_found(format!(
                        "workflow run missing during confirmation saga recovery; knowledge receipt preserved for explicit recovery: {}/{}",
                        operation.workflow_id.as_str(),
                        operation.run_id.as_str()
                    )));
                }
            }
            operation.status = crate::workflow::ConfirmationResolutionStatus::Committed;
        }
        if operation.status == crate::workflow::ConfirmationResolutionStatus::Committed
            && !operation.projected
        {
            let state = runtime_store
                .load_state(&operation.workflow_id, &operation.run_id)
                .map_err(error_to_string)?
                .ok_or_else(|| {
                    CommandError::internal(format!(
                        "workflow run missing during confirmation projection recovery: {}/{}",
                        operation.workflow_id.as_str(),
                        operation.run_id.as_str()
                    ))
                })?;
            let confirmation = state
                .confirmations
                .get(&operation.confirmation_id)
                .ok_or_else(|| {
                    CommandError::internal(format!(
                        "confirmation missing during projection recovery: {}",
                        operation.confirmation_id
                    ))
                })?;
            let entry = confirmation_log_entry_from_runtime(
                confirmation,
                operation.review_reason.as_deref(),
                operation.workflow_id.as_str(),
                operation.run_id.as_str(),
            );
            FileConfirmationLogStore::default_for_project(project_root)
                .record(entry)
                .map_err(error_to_string)?;
            runtime_store
                .mark_confirmation_resolution_projected(&operation.operation_id, now_ms)
                .map_err(error_to_string)?;
        }
        recovered = recovered.saturating_add(1);
    }
    Ok(recovered)
}

pub fn get_git_history_impl(project_root: &Path) -> CommandResult<Vec<GitCommitSummary>> {
    get_git_history_impl_with_cancellation(
        project_root,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

fn get_git_history_impl_with_cancellation(
    project_root: &Path,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<Vec<GitCommitSummary>> {
    validate_project_root(project_root)?;
    git_service_with_cancellation(project_root, cancellation)
        .recent_commits(100)
        .map_err(error_to_string)
}

pub fn get_git_repository_status_impl(project_root: &Path) -> CommandResult<GitRepositoryStatus> {
    get_git_repository_status_impl_with_cancellation(
        project_root,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

fn get_git_repository_status_impl_with_cancellation(
    project_root: &Path,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<GitRepositoryStatus> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "git_status")?;
    let service = git_service_with_cancellation(project_root, cancellation);
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let policy = git_stage_policy_from_config(&config);
    let (health, status) = service
        .health_check_with_policy(&policy)
        .map_err(error_to_string)?;
    let diff = service
        .diff_preview_with_policy(&policy, 4000)
        .map_err(error_to_string)?;
    Ok(GitRepositoryStatus {
        status: health.status,
        branch: health.branch,
        head: health.head,
        dirty: !status.trim().is_empty() || diff.line_count > 0,
        reason: health.reason,
        diff_line_count: diff.line_count,
        diff_preview: diff.preview,
    })
}

pub fn create_checkpoint_impl(project_root: &Path, message: String) -> CommandResult<ArchivePoint> {
    create_checkpoint_impl_with_cancellation(
        project_root,
        message,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

fn create_checkpoint_impl_with_cancellation(
    project_root: &Path,
    message: String,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<ArchivePoint> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "git_checkpoint")?;
    let name = if message.trim().is_empty() {
        "manual-checkpoint".to_owned()
    } else {
        message.trim().to_owned()
    };
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let policy = git_stage_policy_from_config(&config);
    git_service_with_cancellation(project_root, cancellation)
        .create_archive_point_with_policy(&name, Some(&name), &policy)
        .map_err(error_to_string)
}

fn git_service_with_cancellation(
    project_root: impl Into<PathBuf>,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> GitService {
    GitService::new(project_root).with_execution_policy(
        cancellation.clone(),
        Duration::from_secs(IPC_GIT_TIMEOUT_SECS),
    )
}

fn record_git_restore_log(project_root: &Path, report: &RestoreReport) {
    let message = format!(
        "Git restore checked out branch {} from {}; index_rebuild_required={}, runtime_rebind_required={}",
        report.new_branch,
        report.base_commit,
        report.index_rebuild_required,
        report.runtime_rebind_required
    );
    let entry = UiRunLogEntry {
        log_id: format!("git-restore-{}", report.base_commit),
        timestamp_ms: 0,
        kind: UiRunLogKind::Diagnostic,
        level: UiRunLogLevel::Warning,
        message,
        workflow_id: None,
        run_id: None,
        node_id: None,
        unread: true,
        metadata: json!({
            "source": "git_restore",
            "new_branch": report.new_branch,
            "base_commit": report.base_commit,
            "index_rebuild_required": report.index_rebuild_required,
            "runtime_rebind_required": report.runtime_rebind_required,
        }),
    };
    if let Err(error) = UiRunLogStore::default_for_project(project_root).append(entry) {
        eprintln!("[ariadne] failed to record git restore log: {error}");
    }
}

fn git_stage_policy_from_config(config: &ProjectConfig) -> GitStagePolicy {
    let mut ignored_paths = config.git.ignored_paths.clone();
    ignored_paths.extend([
        "runtime.db-wal".to_owned(),
        "runtime.db-shm".to_owned(),
        "costs.db-wal".to_owned(),
        "costs.db-shm".to_owned(),
    ]);
    if !config.git.track_documents {
        ignored_paths.push(config.app.documents_dir.clone());
    }
    if !config.git.track_workflows {
        ignored_paths.push(config.app.workflows_dir.clone());
    }
    if !config.git.track_skills {
        ignored_paths.push(config.app.skills_dir.clone());
    }
    if !config.git.track_non_sensitive_config {
        ignored_paths.push(".config".to_owned());
    }
    GitStagePolicy::default().with_ignored_paths(ignored_paths)
}

pub fn get_provider_config_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
) -> CommandResult<ProviderConfigStatus> {
    let app_state_root = crate::config::trusted_app_state_for_project(project_root);
    get_provider_config_impl_with_app_state(project_root, &app_state_root, secrets)
}

fn get_provider_config_impl_with_app_state(
    project_root: &Path,
    app_state_root: &Path,
    secrets: &dyn SecretStore,
) -> CommandResult<ProviderConfigStatus> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "provider_settings_read")?;
    let config = ConfigStore::with_app_state(project_root, app_state_root)
        .load_or_create()
        .map_err(error_to_string)?;
    provider_config_status_from_config_with_app_state(project_root, config, secrets, app_state_root)
}

fn provider_config_status_from_config_with_app_state(
    project_root: &Path,
    config: ProjectConfig,
    secrets: &dyn SecretStore,
    app_state_root: &Path,
) -> CommandResult<ProviderConfigStatus> {
    let credentials =
        ProjectCredentialScope::new(project_root, secrets).map_err(error_to_string)?;
    let configured_ids = config
        .providers
        .providers
        .iter()
        .map(|provider| provider.provider_id.as_str())
        .collect::<HashSet<_>>();
    let mut available_providers = config.providers.providers.clone();
    let catalog = ProviderCatalogStore::default_for_app(app_state_root)
        .read()
        .map_err(error_to_string)?;
    for provider in catalog.providers {
        if !available_providers
            .iter()
            .any(|existing| existing.provider_id == provider.provider_id)
        {
            available_providers.push(provider);
        }
    }
    let providers = provider_status_list(project_root, &available_providers)
        .into_iter()
        .map(|provider| {
            let configured = configured_ids.contains(provider.provider_id.as_str());
            if provider.api_key.is_some() {
                return Err(CommandError::permission(format!(
                    "provider '{}' contains an untrusted project SecretRef; re-enter the credential",
                    provider.provider_id
                )));
            }
            let has_key = credentials
                .get_provider_secret(&provider.provider_id)
                .map(|secret| secret.is_some())
                .map_err(error_to_string)?;
            Ok(ProviderKeyStatus {
                provider: provider.provider_id,
                display_name: provider.display_name,
                provider_type: provider.provider_type,
                configured,
                enabled: provider.enabled,
                base_url: provider.base_url,
                models: provider.models,
                has_key,
            })
        })
        .collect::<CommandResult<Vec<_>>>()?;

    Ok(ProviderConfigStatus {
        has_openai_key: providers
            .iter()
            .any(|provider| provider.provider == "openai" && provider.has_key),
        has_anthropic_key: providers
            .iter()
            .any(|provider| provider.provider == "anthropic" && provider.has_key),
        has_gemini_key: providers
            .iter()
            .any(|provider| provider.provider == "gemini" && provider.has_key),
        default_llm_provider_id: config.providers.default_llm_provider_id,
        default_embedding_provider_id: config.providers.default_embedding_provider_id,
        default_reranker_provider_id: config.providers.default_reranker_provider_id,
        default_search_provider_id: config.providers.default_search_provider_id,
        providers,
    })
}

fn provider_removal_preview_from_config(
    project_root: &Path,
    secrets: &dyn SecretStore,
    config: &ProjectConfig,
    provider_id: &str,
) -> CommandResult<ProviderRemovalPreview> {
    let provider = config
        .providers
        .providers
        .iter()
        .find(|configured| configured.provider_id == provider_id)
        .ok_or_else(|| {
            CommandError::not_found(format!("provider is not configured: {provider_id}"))
        })?;
    let credentials =
        ProjectCredentialScope::new(project_root, secrets).map_err(error_to_string)?;
    let has_key = credentials
        .get_provider_secret(provider_id)
        .map(|secret| secret.is_some())
        .map_err(error_to_string)?;
    let default_roles = provider_default_roles(config, provider_id);
    let mut blocking_references = provider_preset_references(project_root, config, provider_id)?;
    blocking_references.extend(provider_workflow_references(
        project_root,
        config,
        provider_id,
    )?);
    blocking_references.sort_by(|left, right| {
        (
            left.reference_type.as_str(),
            left.owner_id.as_str(),
            left.node_id.as_deref(),
            left.model_id.as_deref(),
        )
            .cmp(&(
                right.reference_type.as_str(),
                right.owner_id.as_str(),
                right.node_id.as_deref(),
                right.model_id.as_deref(),
            ))
    });
    blocking_references.dedup();
    let revision_body = serde_json::to_vec(&json!({
        "provider": provider,
        "has_key": has_key,
        "default_roles": &default_roles,
        "blocking_references": &blocking_references,
    }))
    .map_err(error_to_string)?;
    Ok(ProviderRemovalPreview {
        provider_id: provider_id.to_owned(),
        display_name: provider.display_name.clone(),
        revision: content_revision_hash(&revision_body),
        has_key,
        default_roles,
        blocking_references,
    })
}

fn provider_default_roles(config: &ProjectConfig, provider_id: &str) -> Vec<String> {
    [
        ("llm", &config.providers.default_llm_provider_id),
        ("embedding", &config.providers.default_embedding_provider_id),
        ("reranker", &config.providers.default_reranker_provider_id),
        ("search", &config.providers.default_search_provider_id),
    ]
    .into_iter()
    .filter(|(_, configured)| configured.as_deref() == Some(provider_id))
    .map(|(role, _)| role.to_owned())
    .collect()
}

fn provider_preset_references(
    project_root: &Path,
    config: &ProjectConfig,
    provider_id: &str,
) -> CommandResult<Vec<ProviderRemovalReference>> {
    let removed_model_ids = config
        .providers
        .providers
        .iter()
        .find(|provider| provider.provider_id == provider_id)
        .into_iter()
        .flat_map(|provider| provider.models.iter())
        .map(|model| model.model_id.as_str())
        .collect::<HashSet<_>>();
    let remaining_model_ids = config
        .providers
        .providers
        .iter()
        .filter(|provider| provider.provider_id != provider_id)
        .flat_map(|provider| provider.models.iter())
        .map(|model| model.model_id.as_str())
        .collect::<HashSet<_>>();
    let settings = read_node_preset_settings(project_root)?;
    let mut references = Vec::new();
    let mut add_reference = |owner_id: String, configured_provider: &str, model_id: &str| {
        let exact_provider = configured_provider.trim() == provider_id;
        let legacy_unique_model = configured_provider.trim().is_empty()
            && removed_model_ids.contains(model_id)
            && !remaining_model_ids.contains(model_id);
        if exact_provider || legacy_unique_model {
            references.push(ProviderRemovalReference {
                reference_type: "node_preset".to_owned(),
                owner_id,
                node_id: None,
                model_id: Some(model_id.to_owned()),
            });
        }
    };
    add_reference(
        "default_model_id".to_owned(),
        &settings.default_provider_id,
        &settings.default_model_id,
    );
    for preset in settings.presets {
        add_reference(preset.node_type, &preset.provider_id, &preset.model_id);
    }
    Ok(references)
}

fn provider_workflow_references(
    project_root: &Path,
    config: &ProjectConfig,
    provider_id: &str,
) -> CommandResult<Vec<ProviderRemovalReference>> {
    let mut references = Vec::new();
    let workflows_root = absolute_path(
        &crate::config::ProjectLayout::from_app(project_root, &config.app)
            .map_err(error_to_string)?
            .workflows,
    );
    reject_symlink_root(&workflows_root)?;
    if workflows_root.exists() {
        let mut paths = workflow_json_paths(&workflows_root)?;
        paths.sort();
        for path in paths {
            ensure_path_under_root(&workflows_root, &path).map_err(error_to_string)?;
            let content = std::fs::read_to_string(path).map_err(error_to_string)?;
            let workflow = parse_workflow_file(&content)?;
            collect_workflow_provider_references(
                &workflow,
                "workflow",
                workflow.id.as_str(),
                config,
                provider_id,
                &mut references,
            );
        }
    }

    let runtime_path = project_root.join(crate::workflow::RUNTIME_DB_FILE);
    if runtime_path.exists() {
        let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
        for state in store.list_non_terminal_states().map_err(error_to_string)? {
            let workflow = state.prepared_workflow.ok_or_else(|| {
                CommandError::legacy_run(format!(
                    "non-terminal run {}/{} lacks prepared_workflow",
                    state.workflow_id.as_str(),
                    state.run_id.as_str()
                ))
            })?;
            let owner_id = format!("{}/{}", state.workflow_id.as_str(), state.run_id.as_str());
            collect_workflow_provider_references(
                &workflow,
                "active_run",
                &owner_id,
                config,
                provider_id,
                &mut references,
            );
        }
    }
    Ok(references)
}

fn collect_workflow_provider_references(
    workflow: &WorkflowDefinition,
    reference_type: &str,
    owner_id: &str,
    config: &ProjectConfig,
    provider_id: &str,
    references: &mut Vec<ProviderRemovalReference>,
) {
    let removes_default_llm =
        config.providers.default_llm_provider_id.as_deref() == Some(provider_id);
    for node in &workflow.nodes {
        if !is_llm_workflow_node_type(&node.type_name) {
            continue;
        }
        let explicit_provider = node
            .config
            .get("provider_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let uses_provider = explicit_provider == Some(provider_id)
            || (node.type_name != "summarizer"
                && explicit_provider.is_none()
                && removes_default_llm);
        if uses_provider {
            references.push(ProviderRemovalReference {
                reference_type: reference_type.to_owned(),
                owner_id: owner_id.to_owned(),
                node_id: Some(node.id.as_str().to_owned()),
                model_id: node
                    .config
                    .get("model_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            });
        }
    }
}

fn clear_provider_defaults(config: &mut ProjectConfig, provider_id: &str) {
    for configured in [
        &mut config.providers.default_llm_provider_id,
        &mut config.providers.default_embedding_provider_id,
        &mut config.providers.default_reranker_provider_id,
        &mut config.providers.default_search_provider_id,
    ] {
        if configured.as_deref() == Some(provider_id) {
            *configured = None;
        }
    }
}

fn clear_removed_provider_key_status(status: &mut ProviderConfigStatus, provider_id: &str) {
    for provider in &mut status.providers {
        if provider.provider == provider_id {
            provider.has_key = false;
        }
    }
    status.has_openai_key = status
        .providers
        .iter()
        .any(|provider| provider.provider == "openai" && provider.has_key);
    status.has_anthropic_key = status
        .providers
        .iter()
        .any(|provider| provider.provider == "anthropic" && provider.has_key);
    status.has_gemini_key = status
        .providers
        .iter()
        .any(|provider| provider.provider == "gemini" && provider.has_key);
}

fn rollback_result_text(result: CommandResult<()>) -> String {
    match result {
        Ok(()) => "ok".to_owned(),
        Err(error) => error.to_string(),
    }
}

pub fn save_provider_key_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    provider: String,
    key: String,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "provider_key_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let provider = normalize_provider(&provider)?;
    if key.trim().is_empty() {
        return Err(CommandError::validation("provider key cannot be empty"));
    }
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store
        .load_or_create_for_credential_rebind()
        .map_err(error_to_string)?;
    apply_provider_key_config(&mut config, &provider);
    let credentials =
        ProjectCredentialScope::new(project_root, secrets).map_err(error_to_string)?;
    let previous_secret = credentials
        .get_provider_secret(&provider)
        .map_err(error_to_string)?;
    credentials
        .set_provider_secret(&provider, SecretValue::new(key))
        .map_err(error_to_string)?;
    if let Err(error) = config_store.save(&config) {
        restore_provider_secret(&credentials, &provider, previous_secret).map_err(|rollback| {
            CommandError::internal(format!(
                "failed to save provider config: {error}; provider key rollback failed: {rollback}"
            ))
        })?;
        return Err(error_to_string(error));
    }
    Ok(())
}

pub fn save_provider_settings_impl(
    project_root: &Path,
    update: ProviderSettingsUpdate,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "provider_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    apply_provider_settings_update(&mut config, update)?;
    config_store.save(&config).map_err(error_to_string)
}

fn apply_provider_key_config(config: &mut ProjectConfig, provider: &str) {
    let provider_config = ensure_provider_config(&mut config.providers.providers, provider);
    // S3：项目配置不再持有全局 key id；重新输入密钥就是显式 rebind。
    provider_config.api_key = None;
    provider_config.enabled = true;
    if provider_config.models.is_empty() {
        provider_config
            .models
            .push(default_llm_model_for_provider(provider));
    }
    if config.providers.default_llm_provider_id.is_none() {
        config.providers.default_llm_provider_id = Some(provider.to_owned());
    }
    config
        .providers
        .authorized_provider_ids
        .insert(provider.to_owned());
}

fn restore_provider_secret(
    credentials: &ProjectCredentialScope<'_>,
    provider: &str,
    previous: Option<SecretValue>,
) -> CommandResult<()> {
    match previous {
        Some(secret) => credentials
            .set_provider_secret(provider, secret)
            .map_err(error_to_string),
        None => credentials
            .delete_provider_secret(provider)
            .map_err(error_to_string),
    }
}

fn apply_provider_settings_update(
    config: &mut ProjectConfig,
    update: ProviderSettingsUpdate,
) -> CommandResult<()> {
    let provider_config = provider_config_from_update(&update)?;
    let provider_id = provider_config.provider_id.clone();
    if let Some(index) = config
        .providers
        .providers
        .iter()
        .position(|provider| provider.provider_id == provider_id)
    {
        config.providers.providers[index] = provider_config;
    } else {
        config.providers.providers.push(provider_config);
    }
    config
        .providers
        .authorized_provider_ids
        .insert(provider_id.clone());
    apply_provider_default_choice(
        &mut config.providers.default_llm_provider_id,
        &provider_id,
        update.make_default_llm,
    );
    apply_provider_default_choice(
        &mut config.providers.default_embedding_provider_id,
        &provider_id,
        update.make_default_embedding,
    );
    apply_provider_default_choice(
        &mut config.providers.default_reranker_provider_id,
        &provider_id,
        update.make_default_reranker,
    );
    apply_provider_default_choice(
        &mut config.providers.default_search_provider_id,
        &provider_id,
        update.make_default_search,
    );
    config.validate().map_err(error_to_string)
}

fn apply_provider_default_choice(
    configured: &mut Option<String>,
    provider_id: &str,
    selected: bool,
) {
    if selected {
        *configured = Some(provider_id.to_owned());
    } else if configured.as_deref() == Some(provider_id) {
        *configured = None;
    }
}

fn provider_config_from_update(update: &ProviderSettingsUpdate) -> CommandResult<ProviderConfig> {
    let provider_config = ProviderConfig {
        provider_id: normalize_provider(&update.provider_id)?,
        provider_type: update.provider_type.clone(),
        display_name: non_empty_or("provider display_name", update.display_name.clone())?,
        enabled: update.enabled,
        base_url: update.base_url.clone(),
        api_key: None,
        models: update.models.clone(),
    };
    provider_config.validate().map_err(error_to_string)?;
    for model in &provider_config.models {
        model
            .validate_provider_model_role()
            .map_err(error_to_string)?;
    }
    Ok(provider_config)
}

pub fn quick_edit_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: QuickEditRequest,
) -> CommandResult<QuickEditResult> {
    quick_edit_impl_with_cancellation(
        project_root,
        secrets,
        request,
        &crate::contracts::ExecutionCancellation::new(),
    )
}

pub fn quick_edit_impl_with_cancellation(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: QuickEditRequest,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<QuickEditResult> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "quick_edit")?;
    let runtime = llm_runtime(project_root, secrets)?;
    let ledger = SqliteCostLedger::open(project_root).map_err(error_to_string)?;
    let service = LlmService::new(&ledger, runtime.auto_mode.clone());
    QuickEditService::new(service, &runtime.provider, runtime.config)
        .quick_edit_with_cancellation(
            &request.selected_text,
            &request.instruction,
            request.context_ref.as_deref(),
            cancellation,
        )
        .map_err(error_to_string)
}

pub fn project_ai_chat_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: ProjectAiRequest,
) -> CommandResult<ProjectAiResponse> {
    // 检索运行时会打开成本账本；维护门禁必须先于任何复合 AI 本地副作用。
    ensure_project_not_in_maintenance(project_root)?;
    let retrieval =
        Arc::new(ProjectRetrievalRuntime::open(project_root, secrets).map_err(error_to_string)?);
    project_ai_chat_with_runner(
        project_root,
        secrets,
        request,
        retrieval,
        &crate::contracts::ExecutionCancellation::new(),
        &mut |_request| {
            Err(CommandError::validation(
                "project AI workflow start requires the application runtime",
            ))
        },
    )
}

pub fn list_external_workflow_tools(
    state: &AriadneAppState,
) -> CommandResult<Vec<ExternalWorkflowTool>> {
    let project_root = project_root_from_state(state, None)?;
    list_external_workflow_tools_impl(&project_root)
}

pub fn list_external_workflow_tools_impl(
    project_root: &Path,
) -> CommandResult<Vec<ExternalWorkflowTool>> {
    project_ai_workflow_tools(project_root).map(|tools| {
        tools
            .into_iter()
            .map(|tool| ExternalWorkflowTool {
                name: tool.tool_name.clone(),
                display_name: tool.display_name.clone(),
                description: workflow_tool_description(&tool),
                workflow_id: tool.workflow_id,
                start_node_id: tool.start_node_id,
                input_schema: tool.input_schema,
            })
            .collect()
    })
}

fn project_ai_chat_with_runner(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: ProjectAiRequest,
    retrieval: Arc<ProjectRetrievalRuntime>,
    cancellation: &crate::contracts::ExecutionCancellation,
    workflow_runner: &mut dyn FnMut(RunWorkflowRequest) -> CommandResult<WorkflowRunStarted>,
) -> CommandResult<ProjectAiResponse> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "project_ai_chat")?;
    if request.message.trim().is_empty()
        && request.chat_history.is_empty()
        && request.references.is_empty()
        && request
            .append_memory
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        && request.workflow_id_to_run.is_none()
    {
        return Err(CommandError::validation(
            "project AI request cannot be empty",
        ));
    }

    let conversation_id = request
        .conversation_id
        .as_deref()
        .unwrap_or("default")
        .trim()
        .to_owned();
    let _conversation_guard =
        ProjectAiConversationStore::try_acquire_conversation(project_root, &conversation_id)
            .map_err(CommandError::from)?
            .ok_or_else(|| {
                CommandError::conflict(format!(
            "project AI conversation {conversation_id} is already processing another revision"
        ))
            })?;
    let conversation_store =
        ProjectAiConversationStore::open(project_root).map_err(CommandError::from)?;
    let seed = request
        .chat_history
        .iter()
        .map(|message| {
            (
                project_ai_role_name(message.role).to_owned(),
                message.content.clone(),
            )
        })
        .collect::<Vec<_>>();
    let conversation = conversation_store
        .load_or_seed(&conversation_id, &seed)
        .map_err(CommandError::from)?;
    if let Some(expected) = request.conversation_revision {
        if expected != conversation.revision {
            return Err(CommandError::conflict(format!(
                "project AI conversation revision conflict for {conversation_id}: expected {expected}, actual {}",
                conversation.revision
            )));
        }
    }
    let pending_client_history = project_ai_pending_client_history(
        &conversation.messages,
        &request.chat_history,
        conversation.revision,
    )?;
    let mut persisted_history = project_ai_messages_from_stored(&conversation.messages)?;
    persisted_history.extend(pending_client_history.iter().cloned());

    let reference_tokens = merge_project_ai_references(&request.references, &request.message);
    let memory_store = ProjectMemoryStore::default_for_project(project_root);
    if let Some(content) = request.append_memory.as_deref() {
        if !content.trim().is_empty() {
            memory_store.append(content).map_err(error_to_string)?;
        }
    }
    let project_memory = memory_store.read_all().map_err(error_to_string)?;
    let project_memory_revision = conversation_store
        .synchronize_project_memory(&project_memory)
        .map_err(CommandError::from)?;
    let structured_memory = conversation_store
        .select_project_memory(&request.message, 64)
        .map_err(CommandError::from)?;
    let summary_chunks = conversation_store
        .select_summary_chunks(&conversation_id, &request.message, 8)
        .map_err(CommandError::from)?;
    let resolved_references = resolve_project_references_with_context(
        project_root,
        &reference_tokens,
        &request.message,
        request.reference_workflow_id.as_deref(),
        request.reference_run_id.as_deref(),
    )?;
    let mut workflow_run = if let Some(workflow_id) = request.workflow_id_to_run.clone() {
        Some(workflow_runner(RunWorkflowRequest {
            workflow_id,
            start_node_id: None,
            initial_inputs: BTreeMap::new(),
        })?)
    } else {
        None
    };
    let workflow_tools = if request.message.trim().is_empty() || workflow_run.is_some() {
        Vec::new()
    } else {
        project_ai_workflow_tools(project_root)?
    };

    let (answer, context_window) = if request.message.trim().is_empty() {
        let (memory, memory_truncated) =
            structured_project_memory_context(&structured_memory, 16_384);
        let (conversation_summary, summary_truncated) =
            project_ai_summary_context(&summary_chunks, 8_192);
        let history_start = persisted_history.len().saturating_sub(64);
        let history = persisted_history[history_start..].to_vec();
        (
            "已处理项目记忆或工作流请求。".to_owned(),
            ProjectAiContextWindow {
                memory_truncated,
                memory,
                conversation_summary,
                reference_context: String::new(),
                history_truncated: history_start > 0,
                history,
                estimated_input_tokens: 0,
                context_limit_tokens: 0,
                references_truncated: false,
                summary_truncated,
            },
        )
    } else {
        let (answer, tool_workflow_run, context_window) = project_ai_answer(
            project_root,
            secrets,
            &structured_memory,
            &summary_chunks,
            &resolved_references,
            &persisted_history,
            &request.message,
            &workflow_tools,
            retrieval.as_ref(),
            cancellation,
            workflow_runner,
        )?;
        if workflow_run.is_none() {
            workflow_run = tool_workflow_run;
        }
        (answer, context_window)
    };
    let new_messages = project_ai_response_history(&[], &request.message, &answer)?;
    let mut messages_to_persist = pending_client_history
        .iter()
        .map(|message| {
            (
                project_ai_role_name(message.role).to_owned(),
                message.content.clone(),
            )
        })
        .collect::<Vec<_>>();
    messages_to_persist.extend(new_messages.iter().map(|message| {
        (
            project_ai_role_name(message.role).to_owned(),
            message.content.clone(),
        )
    }));
    let (conversation, appended) = if messages_to_persist.is_empty() {
        (conversation, Vec::new())
    } else {
        match conversation_store
            .append_messages(
                &conversation_id,
                conversation.revision,
                &messages_to_persist,
            )
            .map_err(CommandError::from)?
        {
            ProjectAiAppendOutcome::Saved { snapshot, appended } => (snapshot, appended),
            ProjectAiAppendOutcome::RevisionConflict { actual_revision } => {
                return Err(CommandError::conflict(format!(
                    "project AI conversation revision conflict for {conversation_id}: expected {}, actual {actual_revision}",
                    conversation.revision
                )))
            }
        }
    };
    let appended_current_turn = appended
        .into_iter()
        .rev()
        .take(new_messages.len())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let active_snapshot = project_ai_messages_from_stored(&conversation.messages)?;
    let revision_protocol = request.conversation_id.is_some();
    let conversation_snapshot = if revision_protocol && request.conversation_revision.is_none() {
        active_snapshot.clone()
    } else {
        Vec::new()
    };
    let chat_history = if revision_protocol {
        Vec::new()
    } else {
        active_snapshot
    };

    Ok(ProjectAiResponse {
        answer,
        chat_history,
        resolved_references,
        workflow_run,
        project_memory: context_window.memory,
        conversation_id,
        conversation_revision: conversation.revision,
        summary_revision: conversation.summary_revision,
        new_messages: project_ai_messages_from_stored(&appended_current_turn)?,
        conversation_snapshot,
        conversation_summary: context_window.conversation_summary,
        project_memory_revision,
        history_truncated: context_window.history_truncated || !summary_chunks.is_empty(),
        memory_truncated: context_window.memory_truncated,
        references_truncated: context_window.references_truncated,
        summary_truncated: context_window.summary_truncated,
        estimated_input_tokens: context_window.estimated_input_tokens,
        context_limit_tokens: context_window.context_limit_tokens,
    })
}

pub fn resolve_project_references(
    project_root: &Path,
    references: &[String],
) -> CommandResult<Vec<ProjectReference>> {
    resolve_project_references_with_context(project_root, references, "", None, None)
}

fn resolve_project_references_with_context(
    project_root: &Path,
    references: &[String],
    query: &str,
    reference_workflow_id: Option<&str>,
    reference_run_id: Option<&str>,
) -> CommandResult<Vec<ProjectReference>> {
    let documents = document_service(project_root);
    let confirmations = FileConfirmationLogStore::default_for_project(project_root);
    let chapter_index = load_chapter_index(project_root)?;
    let knowledge_snapshot = if references
        .iter()
        .any(|reference| project_reference_has_prefix(reference, "知识"))
    {
        Some(
            SqliteWritingKnowledgeStore::open(project_root)
                .map_err(error_to_string)?
                .load_retrieval_snapshot()
                .map_err(error_to_string)?,
        )
    } else {
        None
    };
    let artifact_root = absolute_path(&project_root.join(".runtime/artifacts"));
    let artifacts = if references
        .iter()
        .any(|reference| project_reference_has_prefix(reference, "artifact"))
    {
        artifact_reference_entries(&artifact_root)?
    } else {
        Vec::new()
    };
    let runtime_state = if references
        .iter()
        .any(|reference| project_reference_has_prefix(reference, "节点"))
    {
        let (Some(workflow_id), Some(run_id)) = (reference_workflow_id, reference_run_id) else {
            return Err(CommandError::validation(
                "node reference requires reference_workflow_id and reference_run_id",
            ));
        };
        Some(
            SqliteWorkflowRuntimeStore::open(project_root)
                .map_err(error_to_string)?
                .load_state(&WorkflowId::from(workflow_id), &RunId::from(run_id))
                .map_err(error_to_string)?
                .ok_or_else(|| {
                    CommandError::not_found(format!(
                        "workflow run not found for node reference: {workflow_id}/{run_id}"
                    ))
                })?,
        )
    } else {
        None
    };

    let mut resolver = ProjectReferenceResolver::new()
        .with_documents(&documents)
        .with_confirmations(&confirmations)
        .with_chapter_index(&chapter_index)
        .with_document_root(project_root)
        .with_query(query);
    if let Some(snapshot) = knowledge_snapshot.as_ref() {
        resolver = resolver.with_knowledge_snapshot(snapshot);
    }
    if !artifacts.is_empty() {
        resolver = resolver
            .with_artifacts(artifacts)
            .with_artifact_root(&artifact_root);
    }
    if let Some(runtime_state) = runtime_state.as_ref() {
        resolver = resolver.with_runtime(runtime_state);
    }
    references
        .iter()
        .map(|reference| resolver.resolve(reference).map_err(error_to_string))
        .collect()
}

fn merge_project_ai_references(explicit: &[String], message: &str) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    for reference in explicit
        .iter()
        .cloned()
        .chain(extract_project_reference_tokens(message))
    {
        let normalized = reference.trim().to_owned();
        if !normalized.is_empty() && seen.insert(normalized.clone()) {
            merged.push(normalized);
        }
    }
    merged
}

fn project_reference_has_prefix(reference: &str, expected: &str) -> bool {
    let trimmed = reference.trim().trim_start_matches('@');
    trimmed
        .strip_prefix(expected)
        .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with(':'))
}

fn artifact_reference_entries(artifact_root: &Path) -> CommandResult<Vec<ArtifactReferenceEntry>> {
    reject_symlink_root(artifact_root)?;
    if !artifact_root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = artifact_reference_paths(artifact_root)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            ensure_path_under_root(artifact_root, &path).map_err(error_to_string)?;
            let relative = path.strip_prefix(artifact_root).map_err(|_| {
                CommandError::validation(format!(
                    "artifact path is outside artifact root: {}",
                    path.display()
                ))
            })?;
            let artifact_id = relative.to_string_lossy().replace('\\', "/");
            let kind = if artifact_id.starts_with("exports/") {
                ArtifactKind::Export
            } else {
                ArtifactKind::Other
            };
            Ok(ArtifactReferenceEntry {
                artifact_id,
                kind,
                storage_uri: format!("file://{}", path.display()),
                summary: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn artifact_reference_paths(root: &Path) -> CommandResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root).map_err(error_to_string)? {
        let entry = entry.map_err(error_to_string)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(error_to_string)?;
        if file_type.is_symlink() || entry.file_name() == ".operations" {
            continue;
        }
        if file_type.is_dir() {
            paths.extend(artifact_reference_paths(&path)?);
        } else if file_type.is_file() {
            paths.push(path);
        }
    }
    Ok(paths)
}

const LIST_START_NODES_TOOL: &str = "list_start_nodes";

#[allow(clippy::too_many_arguments)]
fn project_ai_answer(
    project_root: &Path,
    secrets: &dyn SecretStore,
    project_memory: &[ProjectAiMemoryEntry],
    conversation_summaries: &[ProjectAiSummaryChunk],
    references: &[ProjectReference],
    chat_history: &[ProjectAiChatMessage],
    message: &str,
    workflow_tools: &[ProjectWorkflowTool],
    retrieval: &ProjectRetrievalRuntime,
    cancellation: &crate::contracts::ExecutionCancellation,
    workflow_runner: &mut dyn FnMut(RunWorkflowRequest) -> CommandResult<WorkflowRunStarted>,
) -> CommandResult<(String, Option<WorkflowRunStarted>, ProjectAiContextWindow)> {
    let runtime = llm_runtime(project_root, secrets)?;
    let ledger = SqliteCostLedger::open(project_root).map_err(error_to_string)?;
    let service = LlmService::new(&ledger, runtime.auto_mode.clone());
    let context_window = project_ai_context_window(
        project_memory,
        conversation_summaries,
        references,
        chat_history,
        message,
        runtime.config.max_context_tokens,
        runtime.config.max_output_tokens,
    )?;
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let project_ai_permission_policy =
        permission_policy_for_scope(&project_config.permissions, "project_ai");
    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls);
    let project_search_enabled =
        tool_control_enabled(&tool_controls, "project_ai", PROJECT_AI_SEARCH_TOOL);
    let web_search_enabled = project_ai_permission_policy
        .evaluate(&PermissionRequest::WebSearch)
        .allowed
        && tool_control_enabled(&tool_controls, "project_ai", PROJECT_AI_WEB_SEARCH_TOOL);
    let web_search_provider = if web_search_enabled {
        Some(web_search_runtime(project_root, secrets)?)
    } else {
        None
    };
    let mut messages = project_ai_llm_messages(
        &context_window.memory,
        &context_window.conversation_summary,
        &context_window.reference_context,
        &context_window.history,
        message,
        project_search_enabled,
        web_search_enabled,
    )?;
    let start_catalog = project_ai_start_node_catalog(project_root)?;
    let tool_definitions =
        project_ai_tool_definitions(workflow_tools, project_search_enabled, web_search_enabled);
    let mut config = runtime.config;
    if config.max_tool_rounds < 4 {
        config.max_tool_rounds = 4;
    }

    // 手写 tool-use 循环：list_start_nodes 至少一次后才允许 start 工具。
    let mut queried_start_nodes = false;
    let mut tool_workflow_run: Option<WorkflowRunStarted> = None;
    let max_rounds = config.max_tool_rounds;
    let mut final_text = String::new();

    for round in 0..=max_rounds {
        let report = service
            .complete_basic(
                &runtime.provider,
                LlmRunRequest {
                    config: config.clone(),
                    messages: messages.clone(),
                    tools: tool_definitions.clone(),
                    workflow_id: None,
                    run_id: None,
                    node_id: None,
                    metadata: json!({ "project_ai": true, "round": round }),
                    dispatch_authorization: Default::default(),
                },
                cancellation,
            )
            .map_err(error_to_string)?;

        final_text = message_text(report.response.message.content.clone());
        if report.response.tool_calls.is_empty() {
            break;
        }
        if round >= max_rounds {
            return Err(CommandError::new(
                crate::command_error::CommandErrorCode::ResourceLimit,
                "project AI tool-use max rounds exceeded before final answer",
            ));
        }

        messages.push(report.response.message.clone());
        for call in &report.response.tool_calls {
            let context = ProjectAiToolExecutionContext {
                start_catalog: &start_catalog,
                workflow_tools,
                retrieval,
                project_search_enabled,
                web_search_provider: web_search_provider.as_ref(),
                permission_policy: &project_ai_permission_policy,
                ledger: &ledger,
                web_search_enabled,
                cancellation,
                round,
            };
            let mut state = ProjectAiToolExecutionState {
                queried_start_nodes: &mut queried_start_nodes,
                workflow_runner,
                tool_workflow_run: &mut tool_workflow_run,
            };
            let output = project_ai_execute_tool_call(
                call.name.as_str(),
                &call.tool_call_id,
                &call.arguments,
                &context,
                &mut state,
            )?;
            messages.push(tool_result_message(call, output));
        }
    }

    let answer = if final_text.trim().is_empty() && tool_workflow_run.is_some() {
        "ui.project_ai.workflow_tool_started".to_owned()
    } else {
        final_text
    };
    Ok((answer, tool_workflow_run, context_window))
}

struct ProjectAiToolExecutionContext<'a> {
    start_catalog: &'a [ProjectAiStartNodeInfo],
    workflow_tools: &'a [ProjectWorkflowTool],
    retrieval: &'a ProjectRetrievalRuntime,
    project_search_enabled: bool,
    web_search_provider: Option<&'a HttpWebSearchProvider>,
    permission_policy: &'a PermissionPolicy,
    ledger: &'a SqliteCostLedger,
    web_search_enabled: bool,
    cancellation: &'a crate::contracts::ExecutionCancellation,
    round: u32,
}

struct ProjectAiToolExecutionState<'a> {
    queried_start_nodes: &'a mut bool,
    workflow_runner: &'a mut dyn FnMut(RunWorkflowRequest) -> CommandResult<WorkflowRunStarted>,
    tool_workflow_run: &'a mut Option<WorkflowRunStarted>,
}

fn project_ai_execute_tool_call(
    name: &str,
    tool_call_id: &str,
    arguments: &Value,
    context: &ProjectAiToolExecutionContext<'_>,
    state: &mut ProjectAiToolExecutionState<'_>,
) -> CommandResult<ToolExecutionOutput> {
    if name == PROJECT_AI_SEARCH_TOOL && context.project_search_enabled {
        let mut provider_context = crate::providers::ProviderCallContext::new("project_retrieval");
        provider_context.cancellation = context.cancellation.clone();
        let executor = ProjectSearchToolExecutor::new(
            context.retrieval,
            provider_context,
            [PROJECT_AI_SEARCH_TOOL.to_owned()],
        );
        return executor
            .execute(
                &ToolExecutionContext {
                    provider_id: "project_retrieval".to_owned(),
                    workflow_id: None,
                    run_id: None,
                    node_id: None,
                    round: context.round,
                },
                &crate::providers::ToolCall {
                    tool_call_id: tool_call_id.to_owned(),
                    name: name.to_owned(),
                    arguments: arguments.clone(),
                },
            )
            .map_err(error_to_string);
    }
    if name == PROJECT_AI_WEB_SEARCH_TOOL && context.web_search_enabled {
        let provider = context.web_search_provider.ok_or_else(|| {
            CommandError::not_found("Project AI Web Search provider is not configured")
        })?;
        let mut provider_context =
            crate::providers::ProviderCallContext::new(provider.definition().provider_id);
        provider_context.cancellation = context.cancellation.clone();
        let executor = WebSearchToolExecutor::new(
            provider,
            context.ledger,
            context.permission_policy,
            provider_context,
            [PROJECT_AI_WEB_SEARCH_TOOL.to_owned()],
        );
        return executor
            .execute(
                &ToolExecutionContext {
                    provider_id: provider.definition().provider_id,
                    workflow_id: None,
                    run_id: None,
                    node_id: None,
                    round: context.round,
                },
                &crate::providers::ToolCall {
                    tool_call_id: tool_call_id.to_owned(),
                    name: name.to_owned(),
                    arguments: arguments.clone(),
                },
            )
            .map_err(error_to_string);
    }
    if name == LIST_START_NODES_TOOL {
        *state.queried_start_nodes = true;
        let nodes: Vec<Value> = context
            .start_catalog
            .iter()
            .map(|node| {
                json!({
                    "id": node.node_id,
                    "name": node.name,
                    "user_note": node.user_note,
                    "workflow_id": node.workflow_id,
                    "expose_as_tool": node.expose_as_tool,
                    "work_dir": node.work_dir,
                })
            })
            .collect();
        return Ok(ToolExecutionOutput {
            value: json!({
                "ok": true,
                "start_nodes": nodes,
                "count": nodes.len(),
                "hint": "Pick a start node by id/name/user_note, then call its workflow start tool if expose_as_tool is true.",
            }),
            audit_metadata: json!({ "tool": LIST_START_NODES_TOOL }),
        });
    }

    let Some(tool) = context
        .workflow_tools
        .iter()
        .find(|tool| tool.tool_name == name)
        .cloned()
    else {
        return Ok(ToolExecutionOutput {
            value: json!({
                "ok": false,
                "error": format!("unknown tool: {name}"),
            }),
            audit_metadata: json!({ "tool": name, "unknown": true }),
        });
    };

    if !*state.queried_start_nodes {
        return Ok(ToolExecutionOutput {
            value: json!({
                "ok": false,
                "error": "Must call list_start_nodes once before starting any workflow. Query start node id/name/user_note first.",
                "required_tool": LIST_START_NODES_TOOL,
            }),
            audit_metadata: json!({
                "tool": name,
                "rejected": "start_without_list_start_nodes",
            }),
        });
    }

    let initial_inputs = workflow_tool_initial_inputs(arguments.clone())?;
    let started = (state.workflow_runner)(RunWorkflowRequest {
        workflow_id: tool.workflow_id.clone(),
        start_node_id: Some(tool.start_node_id.clone()),
        initial_inputs,
    })?;
    *state.tool_workflow_run = Some(started.clone());
    Ok(ToolExecutionOutput {
        value: json!({
            "ok": true,
            "workflow_id": tool.workflow_id,
            "start_node_id": tool.start_node_id,
            "run_id": started.run_id,
            "status": started.status,
        }),
        audit_metadata: json!({
            "tool": tool.tool_name,
            "start_node_id": tool.start_node_id,
        }),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectAiStartNodeInfo {
    workflow_id: String,
    node_id: String,
    name: String,
    user_note: String,
    work_dir: String,
    expose_as_tool: bool,
}

/// 扫描全部起始节点（不限于 expose_as_tool），供 list_start_nodes 与测试使用。
fn project_ai_start_node_catalog(
    project_root: &Path,
) -> CommandResult<Vec<ProjectAiStartNodeInfo>> {
    let workflows_root = absolute_path(&project_layout(project_root)?.workflows);
    reject_symlink_root(&workflows_root)?;
    if !workflows_root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = workflow_json_paths(&workflows_root)?;
    paths.sort();
    let mut catalog = Vec::new();
    for path in paths {
        ensure_path_under_root(&workflows_root, &path).map_err(error_to_string)?;
        let content = std::fs::read_to_string(&path).map_err(error_to_string)?;
        let workflow: WorkflowDefinition =
            serde_json::from_str(&content).map_err(error_to_string)?;
        for start_node in workflow
            .nodes
            .iter()
            .filter(|node| node.type_name == "start")
        {
            catalog.push(ProjectAiStartNodeInfo {
                workflow_id: workflow.id.as_str().to_owned(),
                node_id: start_node.id.as_str().to_owned(),
                name: start_node_tool_display_name(start_node),
                user_note: start_node
                    .config
                    .get("user_note")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned(),
                work_dir: start_node
                    .config
                    .get("work_dir")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned(),
                expose_as_tool: start_node_exposes_tool(start_node),
            });
        }
    }
    Ok(catalog)
}

fn project_ai_llm_messages(
    project_memory: &str,
    conversation_summary: &str,
    reference_context: &str,
    chat_history: &[ProjectAiChatMessage],
    message: &str,
    project_search_enabled: bool,
    web_search_enabled: bool,
) -> CommandResult<Vec<LlmMessage>> {
    let search_instruction = if project_search_enabled {
        "Use project-ai-search whenever current project documents or confirmed knowledge are needed, and cite the returned document_id/chunk_id in your reasoning when useful. "
    } else {
        ""
    };
    let web_search_instruction = if web_search_enabled {
        "Use project-ai-web-search for current public internet information; preserve and cite returned URLs when useful. "
    } else {
        ""
    };
    let mut messages = vec![
        LlmMessage {
            role: LlmRole::System,
            content: vec![ContentPart::text(
                format!("You are the Ariadne Project AI. Only answer based on project memory, explicit references, chat history, and user messages; do not fabricate project facts not provided. \
{search_instruction}\
{web_search_instruction}\
Before starting any workflow tool, you MUST call list_start_nodes once to read every start node's id, name, and user_note, then choose which start tool to run yourself. \
Do not start a workflow without querying start nodes first."),
            )],
            name: None,
            tool_call_id: None,
        },
        LlmMessage::user(format!(
            "项目结构化记忆：\n{}\n\n会话摘要：\n{}\n\n引用：\n{}",
            project_memory.trim(),
            conversation_summary.trim(),
            reference_context
        )),
    ];
    for history in chat_history {
        if let Some(message) = project_ai_history_to_llm_message(history) {
            messages.push(message);
        }
    }
    messages.push(LlmMessage::user(message.trim()));
    Ok(messages)
}

fn project_ai_workflow_tools(project_root: &Path) -> CommandResult<Vec<ProjectWorkflowTool>> {
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "workflow_tool_discovery")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls);
    if !tool_control_enabled(&tool_controls, "project_ai", "project-ai-workflow-tools") {
        return Ok(Vec::new());
    }

    let canvas = graph_to_workflow(load_project_canvas_locked(project_root)?)?;
    let mut tools = Vec::new();
    for start_node in canvas
        .nodes
        .iter()
        .filter(|node| node.type_name == "start" && start_node_exposes_tool(node))
    {
        let display_name = start_node_tool_display_name(start_node);
        let base_name = sanitize_tool_name(&display_name);
        let mut tool_name = if base_name.is_empty() {
            sanitize_tool_name(start_node.id.as_str())
        } else {
            base_name
        };
        if tool_name.is_empty() {
            tool_name = "workflow".to_owned();
        }
        if tools
            .iter()
            .any(|tool: &ProjectWorkflowTool| tool.tool_name == tool_name)
        {
            tool_name = format!(
                "{}_{}_{}",
                tool_name,
                sanitize_tool_name(canvas.id.as_str()),
                sanitize_tool_name(start_node.id.as_str())
            )
            .trim_matches('_')
            .to_owned();
        }
        if !project_ai_workflow_tool_enabled(&tool_controls, &tool_name) {
            continue;
        }
        tools.push(ProjectWorkflowTool {
            tool_name,
            display_name,
            workflow_id: canvas.id.as_str().to_owned(),
            start_node_id: start_node.id.as_str().to_owned(),
            input_schema: start_node_tool_input_schema(start_node),
        });
    }
    Ok(tools)
}

fn tool_control_enabled(
    controls: &BTreeMap<String, BTreeMap<String, Option<bool>>>,
    scope: &str,
    tool: &str,
) -> bool {
    if let Some(value) = controls
        .get(scope)
        .and_then(|scope_controls| scope_controls.get(tool))
        .and_then(|value| *value)
    {
        return value;
    }

    let default_key = tool_default_action(tool);
    controls
        .get("global")
        .and_then(|defaults| defaults.get(default_key))
        .and_then(|value| *value)
        .unwrap_or(false)
}

fn node_tool_control_enabled(
    controls: &BTreeMap<String, BTreeMap<String, Option<bool>>>,
    presets: &NodePresetSettings,
    type_name: &str,
    scope: &str,
    tool: &str,
) -> bool {
    let preset_override = node_type_preset(presets, type_name).and_then(|preset| {
        preset
            .tool_controls
            .get(tool)
            .or_else(|| preset.tool_controls.get(tool_default_action(tool)))
            .and_then(|value| *value)
    });
    preset_override.unwrap_or_else(|| tool_control_enabled(controls, scope, tool))
}

fn tool_default_action(tool: &str) -> &'static str {
    if tool == "project-ai-workflow-tools" {
        "workflow-tools"
    } else if tool.ends_with("-web-search") {
        "web-search"
    } else if tool.ends_with("-search") {
        "search"
    } else if tool.ends_with("-find") {
        "find"
    } else if tool.ends_with("-register") {
        "register"
    } else if tool.ends_with("-insert-lines")
        || tool.ends_with("-replace-lines")
        || tool.ends_with("-rewrite-file")
    {
        "write"
    } else {
        "unknown"
    }
}

fn project_ai_workflow_tool_enabled(
    controls: &BTreeMap<String, BTreeMap<String, Option<bool>>>,
    tool_name: &str,
) -> bool {
    controls
        .get("project_ai")
        .and_then(|scope_controls| scope_controls.get(tool_name))
        .and_then(|value| *value)
        .unwrap_or_else(|| {
            tool_control_enabled(controls, "project_ai", "project-ai-workflow-tools")
        })
}

fn workflow_json_paths(root: &Path) -> CommandResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(root).map_err(error_to_string)? {
        let entry = entry.map_err(error_to_string)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(error_to_string)?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            paths.extend(workflow_json_paths(&path)?);
        } else if file_type.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn start_node_exposes_tool(node: &crate::contracts::NodeInstance) -> bool {
    node.config
        .get("expose_as_tool")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn start_node_tool_display_name(node: &crate::contracts::NodeInstance) -> String {
    node.config
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or(node.label.as_deref())
        .unwrap_or_else(|| node.id.as_str())
        .to_owned()
}

fn start_node_tool_input_schema(node: &crate::contracts::NodeInstance) -> Value {
    node.config
        .get("tool_input_schema")
        .or_else(|| node.config.get("input_schema"))
        .filter(|schema| schema.as_object().is_some())
        .cloned()
        .unwrap_or_else(empty_tool_input_schema)
}

fn empty_tool_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

fn workflow_tool_initial_inputs(arguments: Value) -> CommandResult<BTreeMap<String, Value>> {
    match arguments {
        Value::Object(map) => Ok(map.into_iter().collect()),
        Value::Null => Ok(BTreeMap::new()),
        other => Err(CommandError::validation(format!(
            "workflow tool arguments must be a JSON object, got {}",
            json_value_kind(&other)
        ))),
    }
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

fn sanitize_tool_name(value: &str) -> String {
    let mut name = String::new();
    let mut previous_underscore = false;
    for character in value.chars() {
        let next = if character.is_ascii_alphanumeric() {
            Some(character.to_ascii_lowercase())
        } else if character == '_' || character == '-' || character.is_whitespace() {
            Some('_')
        } else {
            None
        };
        let Some(next) = next else {
            continue;
        };
        if next == '_' {
            if previous_underscore || name.is_empty() {
                continue;
            }
            previous_underscore = true;
        } else {
            previous_underscore = false;
        }
        name.push(next);
    }
    name.trim_matches('_').to_owned()
}

fn project_ai_tool_definitions(
    workflow_tools: &[ProjectWorkflowTool],
    project_search_enabled: bool,
    web_search_enabled: bool,
) -> Vec<ToolDefinition> {
    let mut tools = Vec::new();
    if project_search_enabled {
        tools.push(project_search_tool_definition(
            PROJECT_AI_TOOL_CAPABILITY.project_search_tool.unwrap(),
            PROJECT_AI_TOOL_CAPABILITY
                .project_search_description
                .unwrap(),
        ));
    }
    if web_search_enabled {
        tools.push(web_search_tool_definition(
            PROJECT_AI_TOOL_CAPABILITY.web_search_tool.unwrap(),
            PROJECT_AI_TOOL_CAPABILITY.web_search_description.unwrap(),
        ));
    }
    if workflow_tools.is_empty() {
        return tools;
    }
    tools.push(ToolDefinition {
        name: LIST_START_NODES_TOOL.to_owned(),
        description: "List all start nodes (id, name, user_note, work_dir, expose_as_tool). \
REQUIRED once before calling any workflow start tool. Use this to decide which start node to run."
            .to_owned(),
        input_schema: empty_tool_input_schema(),
    });
    tools.extend(workflow_tools.iter().map(|tool| ToolDefinition {
        name: tool.tool_name.clone(),
        description: workflow_tool_description(tool),
        input_schema: tool.input_schema.clone(),
    }));
    tools
}

fn workflow_tool_description(tool: &ProjectWorkflowTool) -> String {
    format!(
        "Start Ariadne workflow from start node '{}' (id={}, display='{}'). \
Only call after list_start_nodes has been used once in this turn.",
        tool.start_node_id, tool.start_node_id, tool.display_name
    )
}

fn project_ai_history_to_llm_message(history: &ProjectAiChatMessage) -> Option<LlmMessage> {
    let content = history.content.trim();
    if content.is_empty() {
        return None;
    }
    let role = match history.role {
        ProjectAiChatRole::System => LlmRole::System,
        ProjectAiChatRole::User => LlmRole::User,
        ProjectAiChatRole::Assistant => LlmRole::Assistant,
    };
    Some(LlmMessage {
        role,
        content: vec![ContentPart::text(content)],
        name: None,
        tool_call_id: None,
    })
}

fn project_ai_role_name(role: ProjectAiChatRole) -> &'static str {
    match role {
        ProjectAiChatRole::System => "system",
        ProjectAiChatRole::User => "user",
        ProjectAiChatRole::Assistant => "assistant",
    }
}

fn project_ai_role_from_name(role: &str) -> CommandResult<ProjectAiChatRole> {
    match role {
        "system" => Ok(ProjectAiChatRole::System),
        "user" => Ok(ProjectAiChatRole::User),
        "assistant" => Ok(ProjectAiChatRole::Assistant),
        other => Err(CommandError::validation(format!(
            "unsupported persisted project AI role: {other}"
        ))),
    }
}

fn project_ai_messages_from_stored(
    messages: &[ProjectAiStoredMessage],
) -> CommandResult<Vec<ProjectAiChatMessage>> {
    messages
        .iter()
        .map(|message| {
            Ok(ProjectAiChatMessage {
                role: project_ai_role_from_name(&message.role)?,
                content: message.content.clone(),
            })
        })
        .collect()
}

fn project_ai_pending_client_history(
    persisted: &[ProjectAiStoredMessage],
    incoming: &[ProjectAiChatMessage],
    revision: u64,
) -> CommandResult<Vec<ProjectAiChatMessage>> {
    let incoming = incoming
        .iter()
        .filter(|message| !message.content.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if incoming.is_empty() {
        return Ok(Vec::new());
    }
    if persisted.is_empty() {
        return if revision == 0 {
            Ok(incoming)
        } else {
            Err(CommandError::conflict(
                "project AI client history cannot be reconciled with persisted conversation",
            ))
        };
    }
    let stored = project_ai_messages_from_stored(persisted)?;
    if incoming.len() < stored.len() {
        return Err(CommandError::conflict(
            "project AI client history is older than persisted conversation",
        ));
    }
    let mut match_start = None;
    for start in 0..=incoming.len() - stored.len() {
        if incoming[start..start + stored.len()] == stored[..] {
            match_start = Some(start);
        }
    }
    let Some(start) = match_start else {
        return Err(CommandError::conflict(
            "project AI client history does not match persisted conversation revision",
        ));
    };
    Ok(incoming[start + stored.len()..].to_vec())
}

fn project_ai_response_history(
    chat_history: &[ProjectAiChatMessage],
    message: &str,
    answer: &str,
) -> CommandResult<Vec<ProjectAiChatMessage>> {
    let mut history = chat_history
        .iter()
        .filter(|entry| !entry.content.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if !message.trim().is_empty() {
        history.push(ProjectAiChatMessage {
            role: ProjectAiChatRole::User,
            content: message.trim().to_owned(),
        });
        history.push(ProjectAiChatMessage {
            role: ProjectAiChatRole::Assistant,
            content: answer.to_owned(),
        });
    }
    Ok(history)
}

pub fn append_run_log(project_root: &Path, entry: UiRunLogEntry) -> CommandResult<()> {
    let store = UiRunLogStore::default_for_project(project_root);
    store.append(entry).map(|_| ()).map_err(error_to_string)
}

pub fn recent_project_store(app_state_root: &Path) -> ProjectRegistryStore {
    ProjectRegistryStore::new(app_state_root.join(RECENT_PROJECTS_FILE))
}

fn record_current_project(
    app_state_root: &Path,
    current: &CurrentProjectStatus,
) -> CommandResult<Vec<RecentProjectEntry>> {
    recent_project_store(app_state_root)
        .record_opened(current.project_name.clone(), current.project_root.clone())
        .map_err(error_to_string)
}

pub fn current_project_status(project_root: &Path) -> CommandResult<CurrentProjectStatus> {
    validate_project_root(project_root)?;
    let app_config = project_root.join(".config").join("app.yaml");
    if !app_config.is_file() {
        // Git restore/重建维护期间，app.yaml 可能暂时不存在；状态栏仍需保持只读可见，
        // 但普通未初始化目录不能借此被当成有效项目。仅在已有维护 outbox 且状态仍在
        // active/failed 阶段时放行，避免为了读取状态创建新的 runtime 数据库。
        let outbox_path = project_root.join(".runtime").join("index_invalidation.db");
        let maintenance = if outbox_path.is_file() {
            document_service(project_root)
                .invalidation_outbox()
                .maintenance_state()
                .map_err(error_to_string)?
        } else {
            None
        };
        let maintenance_readable = maintenance
            .as_ref()
            .map(|state| state.status == "active" || state.status == "failed")
            .unwrap_or(false);
        if !maintenance_readable {
            validate_initialized_project_root(project_root)?;
        }
    }
    Ok(CurrentProjectStatus {
        project_root: project_root.to_path_buf(),
        project_name: project_display_name(project_root)?,
    })
}

fn project_display_name(project_root: &Path) -> CommandResult<String> {
    if let Some(config) = ConfigStore::new(project_root)
        .load_app_read_only_optional()
        .map_err(error_to_string)?
    {
        let project_name = config.project_name.trim();
        if !project_name.is_empty() {
            return Ok(project_name.to_owned());
        }
    }
    Ok(project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Ariadne Project")
        .to_owned())
}

pub fn get_sidebar_badges_impl(project_root: &Path) -> CommandResult<SidebarBadgeCounts> {
    let run_logs = UiRunLogStore::default_for_project(project_root);
    let mut badges = run_logs.badge_counts(None, None).map_err(error_to_string)?;
    // 待审数以 runtime 未终态运行为准，避免文件日志历史污染或 pending 未落盘导致徽章失真。
    let pending = list_pending_confirmations_impl(project_root)?.len();
    badges.confirmations = u32::try_from(pending).unwrap_or(u32::MAX);
    Ok(badges)
}

/// 聚合所有未终态运行中的 pending 确认项（含 workflow_id/run_id）。
fn list_pending_confirmations_impl(
    project_root: &Path,
) -> CommandResult<Vec<ConfirmationLogEntry>> {
    validate_project_root(project_root)?;
    let runtime_path = project_root.join(crate::workflow::RUNTIME_DB_FILE);
    if !runtime_path.exists() {
        return Ok(Vec::new());
    }
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let mut pending = Vec::new();
    for state in store.list_non_terminal_states().map_err(error_to_string)? {
        let workflow_id = state.workflow_id.as_str().to_owned();
        let run_id = state.run_id.as_str().to_owned();
        for confirmation in state.confirmations.values() {
            if confirmation.state != RuntimeConfirmationState::Pending {
                continue;
            }
            pending.push(confirmation_log_entry_from_runtime(
                confirmation,
                None,
                &workflow_id,
                &run_id,
            ));
        }
    }
    pending.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp_ms));
    Ok(pending)
}

fn confirmation_log_entry_from_runtime(
    confirmation: &RuntimeConfirmation,
    review_reason: Option<&str>,
    workflow_id: &str,
    run_id: &str,
) -> ConfirmationLogEntry {
    let metadata = &confirmation.metadata;
    let kind = metadata
        .get("kind")
        .or_else(|| metadata.get("prompt_key"))
        .or_else(|| metadata.get("prompt_id"))
        .and_then(Value::as_str)
        .unwrap_or("runtime_confirmation")
        .to_owned();
    let summary = metadata
        .get("summary")
        .or_else(|| metadata.get("title"))
        .or_else(|| metadata.get("reason"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(confirmation.confirmation_id.as_str())
        .to_owned();
    let diff = metadata
        .get("diff")
        .or_else(|| metadata.get("patch"))
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_default();
    let handling_method = review_reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(match confirmation.state {
            RuntimeConfirmationState::Approved => "approved",
            RuntimeConfirmationState::Rejected => "rejected",
            RuntimeConfirmationState::AutoAudited => "auto_audited",
            RuntimeConfirmationState::Pending => "pending",
        })
        .to_owned();

    ConfirmationLogEntry {
        confirmation_id: confirmation.confirmation_id.clone(),
        kind,
        node_id: confirmation.node_id.as_str().to_owned(),
        timestamp_ms: crate::frontend::now_timestamp_ms(),
        state: confirmation_state_from_runtime(confirmation.state),
        handling_method,
        summary,
        diff,
        workflow_id: workflow_id.to_owned(),
        run_id: run_id.to_owned(),
    }
}

fn chapter_index_path(project_root: &Path) -> PathBuf {
    project_root.join(".runtime").join(CHAPTER_INDEX_FILE)
}

fn load_chapter_index(project_root: &Path) -> CommandResult<ChapterDocumentIndex> {
    let path = chapter_index_path(project_root);
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map_err(error_to_string),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ChapterDocumentIndex::new("v1", Vec::new()).map_err(error_to_string)
        }
        Err(error) => Err(error_to_string(error)),
    }
}

fn save_chapter_index(project_root: &Path, index: &ChapterDocumentIndex) -> CommandResult<()> {
    index.validate().map_err(error_to_string)?;
    let path = chapter_index_path(project_root);
    let body = serde_json::to_string_pretty(index).map_err(error_to_string)?;
    crate::config::store::atomic_write(&path, body.as_bytes()).map_err(error_to_string)
}

fn budget_config_path(project_root: &Path) -> PathBuf {
    project_root.join(".config").join(BUDGET_CONFIG_FILE)
}

fn template_repository_settings_path(settings_root: &Path) -> PathBuf {
    settings_root.join(TEMPLATE_REPOSITORY_SETTINGS_FILE)
}

fn node_preset_settings_path(project_root: &Path) -> PathBuf {
    project_root.join(".runtime").join(UI_NODE_PRESETS_FILE)
}

fn app_node_defaults_path(app_state_root: &Path) -> PathBuf {
    app_state_root.join(APP_NODE_DEFAULTS_FILE)
}

fn read_node_preset_settings(project_root: &Path) -> CommandResult<NodePresetSettings> {
    let app_state_root = crate::config::trusted_app_state_for_project(project_root);
    read_node_preset_settings_with_app_state(project_root, &app_state_root)
}

fn read_node_preset_settings_with_app_state(
    project_root: &Path,
    app_state_root: &Path,
) -> CommandResult<NodePresetSettings> {
    validate_project_root(project_root)?;
    let _node_defaults_lock = crate::config::store::acquire_app_state_lock(
        app_state_root,
        APP_NODE_DEFAULTS_LOCK_FILE,
        "app_node_defaults_lock",
    )
    .map_err(error_to_string)?;
    let path = node_preset_settings_path(project_root);
    let legacy = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<NodePresetSettings>(&content).ok(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error_to_string(error)),
    };
    let app = read_app_node_defaults_unlocked(app_state_root, legacy.as_ref())?;
    let project = match std::fs::read_to_string(path) {
        Ok(content) => {
            serde_json::from_str::<ProjectNodePresetOverrides>(&content).map_err(error_to_string)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ProjectNodePresetOverrides::from_settings(&NodePresetSettings::default())
        }
        Err(error) => return Err(error_to_string(error)),
    };
    Ok(project.merge(app))
}

fn read_app_node_defaults_unlocked(
    app_state_root: &Path,
    legacy: Option<&NodePresetSettings>,
) -> CommandResult<AppNodeDefaults> {
    let path = app_node_defaults_path(app_state_root);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let settings: AppNodeDefaults =
                serde_json::from_str(&content).map_err(error_to_string)?;
            settings.validate()?;
            Ok(settings)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let default_settings = NodePresetSettings::default();
            let settings = AppNodeDefaults::from_settings(legacy.unwrap_or(&default_settings));
            settings.validate()?;
            let body = serde_json::to_vec_pretty(&settings).map_err(error_to_string)?;
            crate::config::store::atomic_write(&path, &body).map_err(error_to_string)?;
            Ok(settings)
        }
        Err(error) => Err(error_to_string(error)),
    }
}

fn write_node_preset_settings(
    project_root: &Path,
    app_state_root: &Path,
    settings: &NodePresetSettings,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let configured_models = configured_models_for_presets(project_root)?;
    for preset in &settings.presets {
        if preset.node_type.trim().is_empty() {
            return Err(CommandError::validation("node_type cannot be empty"));
        }
        if preset.model_id.trim().is_empty() {
            return Err(CommandError::validation(format!(
                "model_id cannot be empty for node_type {}",
                preset.node_type
            )));
        }
        if preset.timeout_ms == 0 {
            return Err(CommandError::validation(format!(
                "timeout_ms cannot be zero for node_type {}",
                preset.node_type
            )));
        }
        validate_money("budget_usd", preset.budget_usd)?;
        ensure_preset_model_is_configured(
            &configured_models,
            &preset.provider_id,
            &preset.model_id,
            &format!("preset {}", preset.node_type),
        )?;
    }
    if settings.default_model_id.trim().is_empty() {
        return Err(CommandError::validation("default_model_id cannot be empty"));
    }
    ensure_preset_model_is_configured(
        &configured_models,
        &settings.default_provider_id,
        &settings.default_model_id,
        "default_model_id",
    )?;
    if settings.default_timeout_ms == 0 {
        return Err(CommandError::validation(
            "default_timeout_ms cannot be zero",
        ));
    }
    validate_money("default_budget_usd", settings.default_budget_usd)?;
    let _node_defaults_lock = crate::config::store::acquire_app_state_lock(
        app_state_root,
        APP_NODE_DEFAULTS_LOCK_FILE,
        "app_node_defaults_lock",
    )
    .map_err(error_to_string)?;
    let app_path = app_node_defaults_path(app_state_root);
    let project_path = node_preset_settings_path(project_root);
    let previous_app = std::fs::read(&app_path).ok();
    let app_body = serde_json::to_vec_pretty(&AppNodeDefaults::from_settings(settings))
        .map_err(error_to_string)?;
    let project_body =
        serde_json::to_vec_pretty(&ProjectNodePresetOverrides::from_settings(settings))
            .map_err(error_to_string)?;
    crate::config::store::atomic_write(&app_path, &app_body).map_err(error_to_string)?;
    if let Err(error) = crate::config::store::atomic_write(&project_path, &project_body) {
        let rollback = restore_optional_file(&app_path, previous_app.as_deref());
        return Err(match rollback {
            Ok(()) => error_to_string(error),
            Err(rollback) => CommandError::internal(format!(
                "failed to save project node preset overrides: {error}; global defaults rollback failed: {rollback}"
            )),
        });
    }
    Ok(())
}

fn restore_optional_file(path: &Path, previous: Option<&[u8]>) -> CommandResult<()> {
    match previous {
        Some(bytes) => crate::config::store::atomic_write(path, bytes).map_err(error_to_string),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error_to_string(error)),
        },
    }
}

fn configured_models_for_presets(
    project_root: &Path,
) -> CommandResult<BTreeMap<String, HashSet<String>>> {
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(config
        .providers
        .providers
        .into_iter()
        .map(|provider| {
            (
                provider.provider_id,
                provider
                    .models
                    .into_iter()
                    .map(|model| model.model_id)
                    .collect(),
            )
        })
        .collect())
}

fn ensure_preset_model_is_configured(
    configured_models: &BTreeMap<String, HashSet<String>>,
    provider_id: &str,
    model_id: &str,
    field: &str,
) -> CommandResult<()> {
    if configured_models.is_empty() {
        return Ok(());
    }
    let provider_id = provider_id.trim();
    let model_id = model_id.trim();
    if !provider_id.is_empty() {
        let models = configured_models.get(provider_id).ok_or_else(|| {
            CommandError::validation(format!(
                "{field} references a Provider that is not configured: {provider_id}"
            ))
        })?;
        if models.contains(model_id) {
            return Ok(());
        }
        return Err(CommandError::validation(format!(
            "{field} references a model that is not configured for Provider {provider_id}: {model_id}"
        )));
    }

    let matches = configured_models
        .iter()
        .filter(|(_, models)| models.contains(model_id))
        .map(|(provider, _)| provider.as_str())
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        return Ok(());
    }
    if matches.len() > 1 {
        return Err(CommandError::validation(format!(
            "{field} model id is ambiguous across Providers; select a Provider explicitly: {model_id}"
        )));
    }
    Err(CommandError::validation(format!(
        "{field} references a model that is not configured in model settings: {model_id}"
    )))
}

fn default_node_preset_model_id() -> String {
    "gpt-4.1-mini".to_owned()
}

fn default_node_preset_timeout_ms() -> u64 {
    300_000
}

fn default_node_type_presets() -> Vec<NodeTypePreset> {
    workflow_node_catalog()
        .iter()
        .map(|entry| NodeTypePreset {
            node_type: entry.preset_type.clone(),
            display_name_key: entry.display_name_key.clone(),
            provider_id: String::new(),
            model_id: default_node_preset_model_id(),
            timeout_ms: default_node_preset_timeout_ms(),
            budget_usd: entry.default_budget_usd,
            permission_policy: None,
            tool_controls: BTreeMap::new(),
        })
        .collect()
}

/// 设置页可配置确认项全集。
///
/// 对齐：
/// - `指导性文件/配置项与确认项清单.md` §四：自动化 4 项 + 写作总结 12 类
/// - `指导性文件/创作总结机制(不可删除).md`：register **子功能**可独立配置
/// - `指导性文件/总结机制具体实现计划.md` §8：Outliner/Designer/Planner 输出、register 子功能、
///   Critic/Prudent、四步总结、Writer/Polisher patch
///
/// 禁止空列表；顺序即设置页表格顺序。
fn confirmation_policy_keys() -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = Vec::with_capacity(32);

    // —— 自动化运行门禁（配置项清单 §四）——
    keys.extend_from_slice(&[
        "chapter_write",
        "summary_write",
        "high_risk_permission",
        "budget_exceeded",
    ]);

    // —— 规划节点输出 / 纲领 patch（总结机制 §8）——
    keys.extend_from_slice(&["outliner_output", "designer_output", "planner_output"]);

    // —— register 子功能独立策略（创作总结机制：子功能是否跳过可独立配置）——
    // 总览者 / 设计师 / Planner 共用同一套 RegisterFunction；按 agent 分行便于策略不同。
    for agent in ["outliner", "designer", "planner"] {
        for func in register_function_policy_suffixes() {
            // 形如 planner_register_character_trait
            // 使用静态拼接表，避免 format! 产生非 'static
            keys.push(register_policy_key(agent, func));
        }
    }

    // 兼容旧聚合键（WritingConfirmationPolicy.planner_register / ConfirmationKind::PlannerRegister）
    keys.push("planner_register");

    // —— 审稿 ——
    keys.extend_from_slice(&["critic_review", "prudent_review"]);

    // —— 章节总结四步 ——
    keys.extend_from_slice(&[
        "segment_summary",
        "event_summary",
        "chapter_summary",
        "stage_summary",
    ]);

    // —— 正文修正 patch ——
    keys.extend_from_slice(&["writer_correction_patch", "polisher_correction_patch"]);

    // 再并入 WritingNodeDefinition 声明的 ConfirmationKind，防止模型漏项
    for node in crate::rag::models::WritingNodeDefinition::built_in_nodes() {
        for kind in node.confirmation_kinds {
            let s = confirmation_kind_policy_key(kind);
            if !keys.contains(&s) {
                keys.push(s);
            }
        }
    }

    keys
}

fn register_function_policy_suffixes() -> &'static [&'static str] {
    // 与 RegisterFunction 一一对应
    &[
        "character_profile",
        "character_plan",
        "character_trait",
        "relationship",
        "foreshadowing",
        "theme_anchor",
    ]
}

fn register_policy_key(agent: &str, func: &str) -> &'static str {
    match (agent, func) {
        ("outliner", "character_profile") => "outliner_register_character_profile",
        ("outliner", "character_plan") => "outliner_register_character_plan",
        ("outliner", "character_trait") => "outliner_register_character_trait",
        ("outliner", "relationship") => "outliner_register_relationship",
        ("outliner", "foreshadowing") => "outliner_register_foreshadowing",
        ("outliner", "theme_anchor") => "outliner_register_theme_anchor",
        ("designer", "character_profile") => "designer_register_character_profile",
        ("designer", "character_plan") => "designer_register_character_plan",
        ("designer", "character_trait") => "designer_register_character_trait",
        ("designer", "relationship") => "designer_register_relationship",
        ("designer", "foreshadowing") => "designer_register_foreshadowing",
        ("designer", "theme_anchor") => "designer_register_theme_anchor",
        ("planner", "character_profile") => "planner_register_character_profile",
        ("planner", "character_plan") => "planner_register_character_plan",
        ("planner", "character_trait") => "planner_register_character_trait",
        ("planner", "relationship") => "planner_register_relationship",
        ("planner", "foreshadowing") => "planner_register_foreshadowing",
        ("planner", "theme_anchor") => "planner_register_theme_anchor",
        _ => "planner_register",
    }
}

fn confirmation_kind_policy_key(kind: crate::rag::models::ConfirmationKind) -> &'static str {
    use crate::rag::models::ConfirmationKind::*;
    match kind {
        OutlinerOutput => "outliner_output",
        DesignerOutput => "designer_output",
        PlannerOutput => "planner_output",
        PlannerRegister => "planner_register",
        CriticReview => "critic_review",
        PrudentReview => "prudent_review",
        SegmentSummary => "segment_summary",
        EventSummary => "event_summary",
        ChapterSummary => "chapter_summary",
        StageSummary => "stage_summary",
        WriterCorrectionPatch => "writer_correction_patch",
        PolisherCorrectionPatch => "polisher_correction_patch",
    }
}

/// 合并磁盘已存策略 + 全集 keys，保证设置页永远是完整列表。
/// 旧文件只有 `planner_register` 时，会扩散到各 register 子功能（未单独配置的项）。
fn merge_confirmation_policy_settings(
    existing: Option<&[ConfirmationPolicySetting]>,
    prompts: &[ApprovalPromptConfig],
) -> CommandResult<Vec<ConfirmationPolicySetting>> {
    let prompt_resources =
        crate::rag::resources::load_prompt_resources().map_err(error_to_string)?;
    let mut map = std::collections::BTreeMap::<String, ConfirmationPolicySetting>::new();
    if let Some(items) = existing {
        for item in items {
            map.insert(item.confirmation_kind.clone(), item.clone());
        }
    }

    // 旧聚合键 → 各 agent 的 register 子功能
    if let Some(agg) = map.get("planner_register").cloned() {
        for func in register_function_policy_suffixes() {
            for agent in ["outliner", "designer", "planner"] {
                let key = register_policy_key(agent, func).to_owned();
                map.entry(key).or_insert_with(|| ConfirmationPolicySetting {
                    confirmation_kind: register_policy_key(agent, func).to_owned(),
                    normal_policy: agg.normal_policy,
                    auto_mode_policy: agg.auto_mode_policy,
                    approval_prompt: agg.approval_prompt.clone(),
                });
            }
        }
    }

    for kind in confirmation_policy_keys() {
        map.entry(kind.to_owned()).or_insert_with(|| {
            let policy = policy_for_kind(prompts, kind);
            let (normal_policy, auto_mode_policy) = policies_from_policy_code(&policy);
            ConfirmationPolicySetting {
                confirmation_kind: kind.to_owned(),
                normal_policy,
                auto_mode_policy,
                approval_prompt: approval_prompt_for_kind(prompts, &prompt_resources, kind),
            }
        });
    }
    for item in map.values_mut() {
        if item.approval_prompt.trim().is_empty() {
            item.approval_prompt =
                approval_prompt_for_kind(prompts, &prompt_resources, &item.confirmation_kind);
        }
    }
    // 先结束对 map 的逐项可变借用，再消费剩余扩展项。
    let mut ordered = confirmation_policy_keys()
        .into_iter()
        .filter_map(|k| map.remove(k))
        .collect::<Vec<_>>();
    ordered.extend(map.into_values());
    Ok(ordered)
}

fn approval_prompt_for_kind(
    prompts: &[ApprovalPromptConfig],
    resources: &crate::rag::resources::PromptResources,
    kind: &str,
) -> String {
    prompts
        .iter()
        .find(|prompt| prompt.prompt_id == kind && !prompt.prompt.trim().is_empty())
        .map(|prompt| prompt.prompt.trim().to_owned())
        .or_else(|| {
            resources
                .get(approval_prompt_resource_key(kind))
                .map(|resource| resource.prompt.trim().to_owned())
        })
        .unwrap_or_else(|| {
            "Review the proposed action and return an approval decision with reasons.".to_owned()
        })
}

fn approval_prompt_resource_key(kind: &str) -> &'static str {
    match kind {
        "chapter_write" => "auto_audit.chapter_write",
        "summary_write" => "auto_audit.summary_write",
        "high_risk_permission" => "auto_audit.high_risk_permission",
        "budget_exceeded" => "auto_audit.budget_exceeded",
        "outliner_output" | "designer_output" | "planner_output" => "auto_audit.planning_output",
        "critic_review" | "prudent_review" => "auto_audit.review",
        "segment_summary" | "event_summary" | "chapter_summary" | "stage_summary" => {
            "auto_audit.summary"
        }
        "writer_correction_patch" | "polisher_correction_patch" => "auto_audit.correction_patch",
        value if value == "planner_register" || value.contains("_register_") => {
            "auto_audit.register"
        }
        _ => "auto_audit.generic",
    }
}

fn policy_for_kind(prompts: &[ApprovalPromptConfig], kind: &str) -> String {
    let Some(prompt) = prompts.iter().find(|prompt| prompt.prompt_id == kind) else {
        return "manual".to_owned();
    };
    if !prompt.default_policy.allow_auto_approval {
        "manual".to_owned()
    } else if prompt.default_policy.require_human_on_conflict {
        "auto_audit".to_owned()
    } else {
        "auto_skip".to_owned()
    }
}

fn approval_policy_from_ui(policy: &str) -> CommandResult<ApprovalPolicy> {
    match policy {
        "manual" => Ok(ApprovalPolicy {
            allow_auto_approval: false,
            approval_prompt_id: None,
            require_human_on_conflict: true,
        }),
        "auto_skip" => Ok(ApprovalPolicy {
            allow_auto_approval: true,
            approval_prompt_id: None,
            require_human_on_conflict: false,
        }),
        "auto_audit" => Ok(ApprovalPolicy {
            allow_auto_approval: true,
            approval_prompt_id: Some("default-review".to_owned()),
            require_human_on_conflict: true,
        }),
        // manual_skip: 普通模式手动审批，Auto Mode 自动放行
        "manual_skip" => Ok(ApprovalPolicy {
            allow_auto_approval: true,
            approval_prompt_id: None,
            require_human_on_conflict: false,
        }),
        // auto_approve: 普通模式默认放行，Auto Mode 自动审批（含审查）
        "auto_approve" => Ok(ApprovalPolicy {
            allow_auto_approval: true,
            approval_prompt_id: Some("default-review".to_owned()),
            require_human_on_conflict: true,
        }),
        other => Err(CommandError::validation(format!(
            "unknown confirmation policy: {other}"
        ))),
    }
}

fn ensure_approval_prompt<'a>(
    prompts: &'a mut Vec<ApprovalPromptConfig>,
    kind: &str,
) -> &'a mut ApprovalPromptConfig {
    if let Some(index) = prompts.iter().position(|prompt| prompt.prompt_id == kind) {
        return &mut prompts[index];
    }
    prompts.push(ApprovalPromptConfig {
        prompt_id: kind.to_owned(),
        display_name: kind.to_owned(),
        prompt: "Review the proposed change and return an approval decision with reasons."
            .to_owned(),
        default_policy: ApprovalPolicy::default(),
    });
    prompts
        .last_mut()
        .expect("approval prompt should exist after push")
}

fn read_budget_config(project_root: &Path) -> CommandResult<BudgetConfigFile> {
    let path = budget_config_path(project_root);
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map_err(error_to_string),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(BudgetConfigFile::default())
        }
        Err(error) => Err(error_to_string(error)),
    }
}

fn write_budget_config(project_root: &Path, config: &BudgetConfigFile) -> CommandResult<()> {
    let path = budget_config_path(project_root);
    let body = serde_json::to_string_pretty(config).map_err(error_to_string)?;
    crate::config::store::atomic_write(&path, body.as_bytes()).map_err(error_to_string)
}

fn update_workflow_run_control(
    project_root: &Path,
    workflow_id: String,
    run_id: String,
    update: impl Fn(&mut WorkflowRuntime) -> CoreResult<()>,
) -> CommandResult<WorkflowActionResult> {
    let workflow_id_typed = WorkflowId::from(workflow_id.clone());
    let run_id_typed = RunId::from(run_id.clone());
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    for _ in 0..8 {
        let state = store
            .load_state(&workflow_id_typed, &run_id_typed)
            .map_err(error_to_string)?
            .ok_or_else(|| {
                CommandError::not_found(format!("workflow run not found: {workflow_id}/{run_id}"))
            })?;
        let mut runtime = WorkflowRuntime::from_state(state);
        update(&mut runtime).map_err(error_to_string)?;
        match store.save_state(&mut runtime.state, None) {
            Ok(()) => {
                return Ok(WorkflowActionResult {
                    workflow_id,
                    run_id,
                    status: run_status_label(runtime.state.status).to_owned(),
                });
            }
            Err(crate::contracts::CoreError::WorkflowStateRevisionConflict { .. }) => continue,
            Err(error) => return Err(error_to_string(error)),
        }
    }
    Err(CommandError::conflict(
        "workflow state CAS retry limit exceeded",
    ))
}

fn mutate_workflow_run_and_claim(
    project_root: &Path,
    workflow_id: String,
    run_id: String,
    mutate: impl FnOnce(&mut WorkflowRuntime) -> CoreResult<()>,
) -> CommandResult<(WorkflowActionResult, Option<WorkflowWorkerLease>)> {
    let workflow_id_typed = WorkflowId::from(workflow_id.clone());
    let run_id_typed = RunId::from(run_id.clone());
    let owner_id = format!("worker-{}", new_run_id()?.as_str());
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    match store
        .mutate_state_and_claim(
            &workflow_id_typed,
            &run_id_typed,
            &owner_id,
            workflow_lease_now_ms()?,
            WORKFLOW_WORKER_LEASE_TTL_MS,
            |run_state| {
                let mut runtime = WorkflowRuntime::from_state(run_state.clone());
                mutate(&mut runtime)?;
                *run_state = runtime.state;
                Ok(())
            },
        )
        .map_err(error_to_string)?
    {
        crate::workflow::WorkflowMutationClaimResult::Saved { state, lease } => Ok((
            WorkflowActionResult {
                workflow_id,
                run_id,
                status: run_status_label(state.status).to_owned(),
            },
            lease,
        )),
        crate::workflow::WorkflowMutationClaimResult::Busy { .. } => Err(CommandError::conflict(format!(
            "workflow run is busy and the requested mutation was not applied: {workflow_id}/{run_id}"
        ))),
        crate::workflow::WorkflowMutationClaimResult::NotFound => {
            Err(CommandError::not_found(format!(
                "workflow run not found: {workflow_id}/{run_id}"
            )))
        }
    }
}

/// F10-d：恢复 create_state 已落盘但 worker lease/spawn 未完成（或 lease 已过期）的
/// Queued/Running 运行。Paused 不自动 claim，避免绕过用户暂停。
fn recover_orphaned_workflow_workers(
    project_root: &Path,
    secrets: Arc<dyn SecretStore>,
    retrieval_runtime: Option<Arc<ProjectRetrievalRuntime>>,
) -> CommandResult<usize> {
    let runtime_path = project_root.join(crate::workflow::RUNTIME_DB_FILE);
    if !runtime_path.exists() {
        return Ok(0);
    }
    let project_mutation =
        acquire_project_mutation_guard(project_root, "workflow_orphan_recovery")?;
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let now_ms = workflow_lease_now_ms()?;
    let orphans = store
        .list_orphaned_runnable_states(now_ms)
        .map_err(error_to_string)?;
    let mut recovered = 0usize;
    for orphan in orphans {
        if orphan.status == crate::contracts::RunStatus::Stopping {
            if matches!(
                store
                    .request_stop(
                        &orphan.workflow_id,
                        &orphan.run_id,
                        orphan
                            .stop_reason
                            .as_deref()
                            .unwrap_or("stop requested before worker recovery"),
                        workflow_lease_now_ms()?,
                    )
                    .map_err(error_to_string)?,
                WorkflowStopRequestResult::Saved { .. }
            ) {
                recovered = recovered.saturating_add(1);
            }
            continue;
        }
        if !matches!(
            orphan.status,
            crate::contracts::RunStatus::Queued | crate::contracts::RunStatus::Running
        ) {
            continue;
        }
        let owner_id = format!("worker-recover-{}", new_run_id()?.as_str());
        match store
            .claim_resume(
                &orphan.workflow_id,
                &orphan.run_id,
                &owner_id,
                workflow_lease_now_ms()?,
                WORKFLOW_WORKER_LEASE_TTL_MS,
            )
            .map_err(error_to_string)?
        {
            crate::workflow::WorkflowResumeClaimResult::Claimed { lease, .. } => {
                // F10-b/F10-d：legacy 快照必须先由当前 recovery owner 认领，再以同一
                // generation fencing 收敛到明确失败；不能静默留在 Queued 无限重扫。
                if orphan.prepared_workflow.is_none() {
                    mark_workflow_run_failed_with_lease_impl(
                        project_root,
                        orphan.workflow_id.as_str(),
                        orphan.run_id.as_str(),
                        "workflow_legacy_snapshot_unrecoverable",
                        "workflow_orphan_recovery",
                        "error.workflow.legacy_snapshot_unrecoverable",
                        "error.workflow.legacy_snapshot_unrecoverable.recovery",
                        &lease,
                    )?;
                    recovered = recovered.saturating_add(1);
                    continue;
                }
                if let Err(error) = spawn_continue_workflow_worker_with_lease(
                    project_root.to_path_buf(),
                    Arc::clone(&secrets),
                    retrieval_runtime.clone(),
                    orphan.workflow_id.as_str().to_owned(),
                    orphan.run_id.as_str().to_owned(),
                    lease,
                    Arc::clone(&project_mutation),
                ) {
                    eprintln!(
                        "[ariadne] F10-d orphan recovery spawn failed for {}/{}: {error}",
                        orphan.workflow_id.as_str(),
                        orphan.run_id.as_str()
                    );
                    continue;
                }
                recovered = recovered.saturating_add(1);
            }
            crate::workflow::WorkflowResumeClaimResult::Busy { .. }
            | crate::workflow::WorkflowResumeClaimResult::NotFound
            | crate::workflow::WorkflowResumeClaimResult::NotResumable { .. } => {}
        }
    }
    Ok(recovered)
}

/// F9：应用级常驻 runnable scheduler。SQLite scheduler lease 决定唯一扫描者，
/// 每条任务再通过原子 claim_next_runnable 获取独立 worker lease 与 generation。
fn workflow_scheduler_loop(
    project_root: PathBuf,
    secrets: Arc<dyn SecretStore>,
    retrieval_runtime: Arc<Mutex<Option<Arc<ProjectRetrievalRuntime>>>>,
    stop_receiver: std::sync::mpsc::Receiver<()>,
) {
    let owner_id = match new_run_id() {
        Ok(id) => format!("scheduler-{}", id.as_str()),
        Err(error) => {
            eprintln!("[ariadne] workflow scheduler id failed: {error}");
            return;
        }
    };
    loop {
        let sleep_ms =
            match workflow_scheduler_tick(&project_root, &secrets, &retrieval_runtime, &owner_id) {
                Ok(sleep_ms) => sleep_ms,
                Err(error) => {
                    eprintln!("[ariadne] workflow scheduler tick failed: {error}");
                    WORKFLOW_SCHEDULER_MAX_SLEEP_MS
                }
            };
        match stop_receiver.recv_timeout(Duration::from_millis(sleep_ms.max(1))) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    if let Ok(store) = SqliteWorkflowRuntimeStore::open(&project_root) {
        let _ = store.release_scheduler_lease(&owner_id);
    }
}

fn workflow_scheduler_tick(
    project_root: &Path,
    secrets: &Arc<dyn SecretStore>,
    retrieval_runtime: &Arc<Mutex<Option<Arc<ProjectRetrievalRuntime>>>>,
    owner_id: &str,
) -> CommandResult<u64> {
    let runtime_path = project_root.join(crate::workflow::RUNTIME_DB_FILE);
    if !runtime_path.exists() {
        return Ok(WORKFLOW_SCHEDULER_MAX_SLEEP_MS);
    }
    ensure_project_not_in_maintenance(project_root)?;
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let now_ms = workflow_lease_now_ms()?;
    if !store
        .acquire_scheduler_lease(owner_id, now_ms, WORKFLOW_SCHEDULER_LEASE_TTL_MS)
        .map_err(error_to_string)?
    {
        return Ok(WORKFLOW_SCHEDULER_MAX_SLEEP_MS);
    }
    let project_mutation = acquire_project_mutation_guard(project_root, "workflow_scheduler")?;
    for _ in 0..WORKFLOW_SCHEDULER_MAX_CLAIMS_PER_TICK {
        let worker_owner = format!("worker-scheduled-{}", new_run_id()?.as_str());
        let claim = store
            .claim_next_runnable(
                owner_id,
                &worker_owner,
                workflow_lease_now_ms()?,
                WORKFLOW_WORKER_LEASE_TTL_MS,
            )
            .map_err(error_to_string)?;
        let (state, lease) = match claim {
            WorkflowRunnableClaimResult::Claimed { state, lease } => (state, lease),
            WorkflowRunnableClaimResult::Stopped { .. } => continue,
            WorkflowRunnableClaimResult::Empty => break,
        };
        let Some(prepared_workflow) = state.prepared_workflow.as_ref() else {
            mark_workflow_run_failed_with_lease_impl(
                project_root,
                state.workflow_id.as_str(),
                state.run_id.as_str(),
                "workflow_legacy_snapshot_unrecoverable",
                "workflow_scheduler",
                "error.workflow.legacy_snapshot_unrecoverable",
                "error.workflow.legacy_snapshot_unrecoverable.recovery",
                &lease,
            )?;
            continue;
        };
        let Some(prepared_dependency_plan) = state.prepared_dependency_plan.as_ref() else {
            mark_workflow_run_failed_with_lease_impl(
                project_root,
                state.workflow_id.as_str(),
                state.run_id.as_str(),
                "workflow_legacy_dependency_snapshot_unrecoverable",
                "workflow_scheduler",
                "error.workflow.legacy_snapshot_unrecoverable",
                "error.workflow.legacy_snapshot_unrecoverable.recovery",
                &lease,
            )?;
            continue;
        };
        let prepared_runtime = (|| -> CommandResult<_> {
            let dependency_plan = materialize_frozen_workflow_runtime_dependency_plan(
                project_root,
                secrets.as_ref(),
                prepared_workflow,
                prepared_dependency_plan,
            )?;
            scheduled_workflow_retrieval_runtime(
                project_root,
                secrets.as_ref(),
                retrieval_runtime,
                &dependency_plan,
            )
        })();
        let loaded_retrieval_runtime = match prepared_runtime {
            Ok(runtime) => runtime,
            Err(error) => {
                mark_workflow_run_failed_with_lease_impl(
                    project_root,
                    state.workflow_id.as_str(),
                    state.run_id.as_str(),
                    "workflow_scheduler_initialization_failed",
                    "workflow_scheduler_initialization",
                    error.diagnostic_text(),
                    "error.workflow.worker_failed.recovery",
                    &lease,
                )?;
                continue;
            }
        };
        if let Err(error) = spawn_continue_workflow_worker_with_lease(
            project_root.to_path_buf(),
            Arc::clone(secrets),
            loaded_retrieval_runtime,
            state.workflow_id.as_str().to_owned(),
            state.run_id.as_str().to_owned(),
            lease,
            Arc::clone(&project_mutation),
        ) {
            eprintln!(
                "[ariadne] workflow scheduler spawn failed for {}/{}: {error}",
                state.workflow_id.as_str(),
                state.run_id.as_str()
            );
            continue;
        }
    }
    let now_ms = workflow_lease_now_ms()?;
    let sleep_ms = store
        .next_runnable_at_ms(now_ms)
        .map_err(error_to_string)?
        .map(|deadline| deadline.saturating_sub(now_ms))
        .unwrap_or(WORKFLOW_SCHEDULER_MAX_SLEEP_MS)
        .min(WORKFLOW_SCHEDULER_MAX_SLEEP_MS);
    Ok(sleep_ms)
}

fn scheduled_workflow_retrieval_runtime(
    project_root: &Path,
    secrets: &dyn SecretStore,
    retrieval_runtime: &Arc<Mutex<Option<Arc<ProjectRetrievalRuntime>>>>,
    dependency_plan: &WorkflowRuntimeDependencyPlan,
) -> CommandResult<Option<Arc<ProjectRetrievalRuntime>>> {
    if !dependency_plan.requires_project_retrieval {
        return Ok(None);
    }
    let mut runtime_slot = retrieval_runtime
        .lock()
        .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?;
    if let Some(runtime) = runtime_slot.as_ref() {
        if runtime.project_root() != project_root {
            return Err(CommandError::conflict(
                "shared retrieval runtime belongs to a different project",
            ));
        }
        if !runtime
            .matches_project_config(&dependency_plan.project_config)
            .map_err(error_to_string)?
        {
            return Err(CommandError::conflict(
                "shared retrieval runtime configuration changed after workflow preparation",
            ));
        }
        return Ok(Some(Arc::clone(runtime)));
    }
    let runtime = Arc::new(
        ProjectRetrievalRuntime::from_config(
            project_root,
            secrets,
            &dependency_plan.project_config,
            None,
        )
        .map_err(error_to_string)?,
    );
    runtime_slot.replace(Arc::clone(&runtime));
    Ok(Some(runtime))
}

fn spawn_continue_workflow_worker_with_lease(
    project_root: PathBuf,
    secrets: Arc<dyn SecretStore>,
    retrieval_runtime: Option<Arc<ProjectRetrievalRuntime>>,
    workflow_id: String,
    run_id: String,
    worker_lease: WorkflowWorkerLease,
    project_mutation: Arc<ProjectMutationGuard>,
) -> CommandResult<()> {
    let failure_workflow_id = workflow_id.clone();
    let failure_run_id = run_id.clone();
    let spawned_worker_lease = worker_lease.clone();
    let spawned_project_mutation = Arc::clone(&project_mutation);
    let worker_root = project_root.clone();
    let spawn_result = std::thread::Builder::new()
        .name(format!("ariadne-workflow-resume-{run_id}"))
        .spawn(move || {
            let _project_mutation = spawned_project_mutation;
            let execution_lease = spawned_worker_lease.clone();
            let worker_result = run_with_workflow_worker_lease(
                &worker_root,
                spawned_worker_lease,
                |cancellation| {
                    continue_workflow_run_impl(
                        &worker_root,
                        secrets.as_ref(),
                        retrieval_runtime.clone(),
                        workflow_id.clone(),
                        run_id.clone(),
                        &execution_lease,
                        cancellation,
                    )
                },
            );
            if let Err(error) = worker_result {
                if error.diagnostic_text() != WORKFLOW_WORKER_LEASE_LOST_ERROR {
                    record_workflow_worker_error(
                        &worker_root,
                        &workflow_id,
                        &run_id,
                        "workflow resume worker failed",
                        &error,
                        Some(&execution_lease),
                    );
                }
                eprintln!("[ariadne] workflow resume worker failed: {error}");
            }
        });
    if let Err(error) = spawn_result {
        let spawn_error = error_to_string(error);
        mark_workflow_run_failed_with_lease_impl(
            &project_root,
            &failure_workflow_id,
            &failure_run_id,
            "workflow_worker_spawn_failed",
            "worker_spawn",
            &spawn_error,
            "error.workflow.worker_spawn_failed.recovery",
            &worker_lease,
        )?;
        return Err(spawn_error);
    }
    Ok(())
}

fn acquire_workflow_worker_lease(
    store: &SqliteWorkflowRuntimeStore,
    workflow_id: &WorkflowId,
    run_id: &RunId,
) -> CommandResult<Option<WorkflowWorkerLease>> {
    let owner_id = format!("worker-{}", new_run_id()?.as_str());
    store
        .acquire_worker_lease(
            workflow_id,
            run_id,
            &owner_id,
            workflow_lease_now_ms()?,
            WORKFLOW_WORKER_LEASE_TTL_MS,
        )
        .map_err(error_to_string)
}

fn run_with_workflow_worker_lease<T>(
    project_root: &Path,
    lease: WorkflowWorkerLease,
    work: impl FnOnce(crate::contracts::ExecutionCancellation) -> CommandResult<T>,
) -> CommandResult<T> {
    let (stop_sender, stop_receiver) = std::sync::mpsc::channel::<()>();
    let lease_valid = Arc::new(AtomicBool::new(true));
    let cancellation = crate::contracts::ExecutionCancellation::new();
    let heartbeat_cancellation = cancellation.clone();
    let heartbeat_valid = Arc::clone(&lease_valid);
    let heartbeat_root = project_root.to_path_buf();
    let heartbeat_lease = lease.clone();
    let heartbeat_result = std::thread::Builder::new()
        .name(format!(
            "ariadne-workflow-heartbeat-{}",
            lease.run_id.as_str()
        ))
        .spawn(move || {
            let store = match SqliteWorkflowRuntimeStore::open(&heartbeat_root) {
                Ok(store) => store,
                Err(_) => {
                    heartbeat_valid.store(false, Ordering::Release);
                    return;
                }
            };
            let mut last_heartbeat = Instant::now();
            loop {
                match stop_receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
                let now_ms = match workflow_lease_now_ms() {
                    Ok(now_ms) => now_ms,
                    Err(_) => {
                        heartbeat_valid.store(false, Ordering::Release);
                        heartbeat_cancellation.cancel();
                        return;
                    }
                };
                let should_cancel = match store.execution_should_cancel(&heartbeat_lease, now_ms) {
                    Ok(should_cancel) => should_cancel,
                    Err(_) => {
                        heartbeat_valid.store(false, Ordering::Release);
                        heartbeat_cancellation.cancel();
                        return;
                    }
                };
                if should_cancel {
                    let replaced_or_expired = store
                        .load_worker_lease(&heartbeat_lease.workflow_id, &heartbeat_lease.run_id)
                        .ok()
                        .flatten()
                        .is_some_and(|current| {
                            current.owner_id != heartbeat_lease.owner_id
                                || current.generation != heartbeat_lease.generation
                                || current.expires_at_ms <= now_ms
                        });
                    if replaced_or_expired {
                        heartbeat_valid.store(false, Ordering::Release);
                    }
                    heartbeat_cancellation.cancel();
                    return;
                }
                if last_heartbeat.elapsed()
                    < Duration::from_millis(WORKFLOW_WORKER_HEARTBEAT_INTERVAL_MS)
                {
                    continue;
                }
                let renewed = store
                    .heartbeat_worker_lease(
                        &heartbeat_lease.workflow_id,
                        &heartbeat_lease.run_id,
                        &heartbeat_lease.owner_id,
                        heartbeat_lease.generation,
                        now_ms,
                        WORKFLOW_WORKER_LEASE_TTL_MS,
                    )
                    .map_err(error_to_string)
                    .unwrap_or(false);
                if renewed {
                    last_heartbeat = Instant::now();
                } else {
                    heartbeat_valid.store(false, Ordering::Release);
                    heartbeat_cancellation.cancel();
                    return;
                }
            }
        });
    let heartbeat = match heartbeat_result {
        Ok(heartbeat) => heartbeat,
        Err(error) => {
            return Err(error_to_string(error));
        }
    };

    let result = work(cancellation);
    let _ = stop_sender.send(());
    let _ = heartbeat.join();
    let still_valid = lease_valid.load(Ordering::Acquire);
    if !still_valid {
        return Err(CommandError::conflict(WORKFLOW_WORKER_LEASE_LOST_ERROR));
    }
    if result.is_ok() {
        let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
        let _ = store
            .release_worker_lease(
                &lease.workflow_id,
                &lease.run_id,
                &lease.owner_id,
                lease.generation,
            )
            .map_err(error_to_string)?;
    }
    result
}

fn workflow_lease_now_ms() -> CommandResult<u64> {
    u64::try_from(now_timestamp_ms()?)
        .map_err(|_| CommandError::internal("workflow worker lease timestamp exceeds u64"))
}

fn record_workflow_worker_error(
    project_root: &Path,
    workflow_id: &str,
    run_id: &str,
    context: &str,
    error: &str,
    worker_lease: Option<&WorkflowWorkerLease>,
) {
    let persist_result = match worker_lease {
        Some(lease) => mark_workflow_run_failed_with_lease_impl(
            project_root,
            workflow_id,
            run_id,
            "workflow_worker_failed",
            context,
            error,
            "error.workflow.worker_failed.recovery",
            lease,
        ),
        None => mark_workflow_run_failed_impl(
            project_root,
            workflow_id,
            run_id,
            "workflow_worker_failed",
            context,
            error,
            "error.workflow.worker_failed.recovery",
        ),
    };
    if let Err(persist_error) = persist_result {
        eprintln!(
            "[ariadne] failed to persist workflow failure for {workflow_id}/{run_id}: {persist_error}"
        );
    }
    let entry = UiRunLogEntry {
        log_id: format!("{context}-{run_id}"),
        timestamp_ms: 0,
        kind: UiRunLogKind::Error,
        level: UiRunLogLevel::Error,
        message: format!("{context}: {error}"),
        workflow_id: Some(WorkflowId::from(workflow_id.to_owned())),
        run_id: Some(RunId::from(run_id.to_owned())),
        node_id: None,
        unread: true,
        metadata: json!({ "source": "workflow_worker" }),
    };
    if let Err(log_error) = UiRunLogStore::default_for_project(project_root).append(entry) {
        eprintln!("[ariadne] failed to record workflow worker error: {log_error}");
    }
}

#[allow(clippy::too_many_arguments)]
pub fn mark_workflow_run_failed_with_lease_impl(
    project_root: &Path,
    workflow_id: &str,
    run_id: &str,
    code: &str,
    stage: &str,
    message: &str,
    recovery_suggestion: &str,
    worker_lease: &WorkflowWorkerLease,
) -> CommandResult<()> {
    mark_workflow_run_failed_in_store(
        SqliteWorkflowRuntimeStore::open(project_root)
            .map_err(error_to_string)?
            .with_worker_lease(worker_lease.clone()),
        workflow_id,
        run_id,
        code,
        stage,
        message,
        recovery_suggestion,
    )
}

pub fn mark_workflow_run_failed_impl(
    project_root: &Path,
    workflow_id: &str,
    run_id: &str,
    code: &str,
    stage: &str,
    message: &str,
    recovery_suggestion: &str,
) -> CommandResult<()> {
    mark_workflow_run_failed_in_store(
        SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?,
        workflow_id,
        run_id,
        code,
        stage,
        message,
        recovery_suggestion,
    )
}

fn mark_workflow_run_failed_in_store(
    store: SqliteWorkflowRuntimeStore,
    workflow_id: &str,
    run_id: &str,
    code: &str,
    stage: &str,
    message: &str,
    recovery_suggestion: &str,
) -> CommandResult<()> {
    let workflow_id = WorkflowId::from(workflow_id.to_owned());
    let run_id = RunId::from(run_id.to_owned());
    let Some(mut state) = store
        .load_state(&workflow_id, &run_id)
        .map_err(error_to_string)?
    else {
        return Ok(());
    };
    if state.status.is_terminal() {
        return Ok(());
    }
    state.status = crate::contracts::RunStatus::Failed;
    state.control = RunControl::Stop;
    state.next_retry_at_ms = None;
    state.failure = Some(WorkflowRunFailure {
        code: code.to_owned(),
        stage: stage.to_owned(),
        message: message.to_owned(),
        recovery_suggestion: recovery_suggestion.to_owned(),
    });
    let sequence = state.next_event_sequence;
    state.next_event_sequence = state.next_event_sequence.saturating_add(1);
    state.structured_events.push(WorkflowRuntimeEvent {
        sequence,
        event_type: WorkflowRuntimeEventType::RunFailed,
        node_id: None,
        message: message.to_owned(),
        metadata: json!({
            "code": code,
            "stage": stage,
            "recovery_suggestion": recovery_suggestion,
        }),
    });
    store.save_state(&mut state, None).map_err(error_to_string)
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

fn template_client(
    request: TemplateRepositoryRequest,
    cancellation: &crate::contracts::ExecutionCancellation,
) -> CommandResult<TemplateRepositoryClient> {
    let base_url = request
        .base_url
        .unwrap_or_else(|| DEFAULT_TEMPLATE_REPOSITORY_URL.to_owned());
    if base_url.trim().is_empty() {
        return Err(CommandError::validation(
            "template repository is not configured; please set a base URL in settings",
        ));
    }
    TemplateRepositoryClient::new_with_cancellation(base_url, cancellation.clone())
        .map_err(error_to_string)
}

fn validate_template_url(url: &str) -> CommandResult<()> {
    crate::frontend::validate_template_repository_base_url(url).map_err(error_to_string)
}

struct CommandLlmRuntime {
    provider: OpenAiCompatibleLlmProvider,
    config: LlmServiceConfig,
    auto_mode: crate::config::AutoModeConfig,
}

fn web_search_runtime(
    project_root: &Path,
    secrets: &dyn SecretStore,
) -> CommandResult<HttpWebSearchProvider> {
    validate_project_root(project_root)?;
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    web_search_runtime_for_config(project_root, secrets, &project_config)
}

fn web_search_runtime_for_config(
    project_root: &Path,
    secrets: &dyn SecretStore,
    project_config: &ProjectConfig,
) -> CommandResult<HttpWebSearchProvider> {
    let provider_config = select_web_search_provider(&project_config.providers)?;
    if provider_config.api_key.is_some() {
        return Err(CommandError::permission(format!(
            "provider '{}' contains an untrusted project SecretRef; re-enter the credential before Web Search use",
            provider_config.provider_id
        )));
    }
    let api_key = ProjectCredentialScope::new(project_root, secrets)
        .map_err(error_to_string)?
        .get_provider_secret(&provider_config.provider_id)
        .map_err(error_to_string)?
        .map(|value| value.expose_secret().to_owned());
    HttpWebSearchProvider::new(provider_config, api_key).map_err(error_to_string)
}

fn select_web_search_provider(
    providers: &crate::config::ProvidersConfig,
) -> CommandResult<ProviderConfig> {
    if let Some(default_id) = providers.default_search_provider_id.as_deref() {
        return providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == default_id)
            .filter(|provider| provider.enabled)
            .cloned()
            .ok_or_else(|| {
                CommandError::not_found(format!(
                    "default Web Search provider is missing or disabled: {default_id}"
                ))
            });
    }
    providers
        .providers
        .iter()
        .find(|provider| {
            provider.enabled
                && provider.models.iter().any(|model| {
                    matches!(
                        model.capability,
                        ProviderCapability::Search | ProviderCapability::Llm
                    )
                })
        })
        .cloned()
        .ok_or_else(|| {
            CommandError::not_found(
                "no enabled Web Search provider is configured; select a default search provider",
            )
        })
}

fn llm_runtime(project_root: &Path, secrets: &dyn SecretStore) -> CommandResult<CommandLlmRuntime> {
    validate_project_root(project_root)?;
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let provider_config = select_llm_provider(&project_config.providers)?;
    let model_config = select_llm_model(&provider_config)?;
    if provider_config.api_key.is_some() {
        return Err(CommandError::permission(format!(
            "provider '{}' contains an untrusted project SecretRef; re-enter the credential before LLM use",
            provider_config.provider_id
        )));
    }
    let api_key = ProjectCredentialScope::new(project_root, secrets)
        .map_err(error_to_string)?
        .get_provider_secret(&provider_config.provider_id)
        .map_err(error_to_string)?
        .map(|value| value.expose_secret().to_owned());
    let provider = OpenAiCompatibleLlmProvider::new(provider_config.clone(), api_key)
        .map_err(error_to_string)?;
    let budget_config = read_budget_config(project_root)?;
    let mut config =
        LlmServiceConfig::new(provider_config.provider_id, model_config.model_id.clone())
            .with_model_config(&model_config);
    // 用户配置的日预算写入执行侧 daily_usd，供 LlmService::evaluate_budget 使用。
    config.budget_limits = budget_limits_from_global_budget(budget_config.budget_usd);
    // auto_mode 已含 preauthorized_budget_usd（update_budget_config 写入）。
    Ok(CommandLlmRuntime {
        provider,
        config,
        auto_mode: project_config.auto_mode,
    })
}

fn select_llm_provider(
    providers: &crate::config::ProvidersConfig,
) -> CommandResult<ProviderConfig> {
    if let Some(default_id) = providers.default_llm_provider_id.as_deref() {
        return providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == default_id)
            .filter(|provider| provider.enabled)
            .cloned()
            .ok_or_else(|| {
                CommandError::not_found(format!(
                    "default LLM provider is missing or disabled: {default_id}"
                ))
            });
    }
    providers
        .providers
        .iter()
        .find(|provider| {
            provider.enabled
                && (provider.models.iter().any(|model| {
                    model.capability == ProviderCapability::Llm
                        || model.capability == ProviderCapability::ToolUse
                }) || provider.models.is_empty())
        })
        .cloned()
        .ok_or_else(|| CommandError::not_found("no enabled LLM provider is configured"))
}

fn select_llm_model(provider: &ProviderConfig) -> CommandResult<ModelConfig> {
    provider
        .models
        .iter()
        .find(|model| model.capability == ProviderCapability::Llm)
        .or_else(|| {
            provider
                .models
                .iter()
                .find(|model| model.capability == ProviderCapability::ToolUse)
        })
        .cloned()
        .ok_or_else(|| {
            CommandError::not_found(format!(
                "provider {} has no model configured for LLM calls",
                provider.provider_id
            ))
        })
}

fn message_text(content: Vec<ContentPart>) -> String {
    content
        .into_iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text),
            ContentPart::Json { value } | ContentPart::ToolResult { value, .. } => {
                Some(value.to_string())
            }
            // ToolUse 是 assistant 发起的工具调用，通过 tool_calls 单独承载，不拼进文本。
            ContentPart::ToolUse { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn project_root_from_state(
    state: &AriadneAppState,
    project_id: Option<String>,
) -> CommandResult<PathBuf> {
    match project_id {
        Some(project_id) if !project_id.trim().is_empty() => {
            let path = PathBuf::from(project_id);
            validate_initialized_project_root(&path)?;
            crate::config::bind_project_app_state(&path, state.app_state_root())
                .map_err(error_to_string)?;
            Ok(path)
        }
        _ => {
            let path = state.project_root()?;
            validate_initialized_project_root(&path)?;
            crate::config::bind_project_app_state(&path, state.app_state_root())
                .map_err(error_to_string)?;
            Ok(path)
        }
    }
}

fn project_layout(project_root: &Path) -> CommandResult<crate::config::ProjectLayout> {
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    crate::config::ProjectLayout::from_app(project_root, &config.app).map_err(error_to_string)
}

fn document_service(project_root: &Path) -> FileDocumentService {
    document_service_with_artifacts(
        project_root,
        project_root.join(".runtime").join("artifacts"),
        None,
    )
}

fn configured_document_service(project_root: &Path) -> CommandResult<FileDocumentService> {
    let layout = project_layout(project_root)?;
    layout
        .create_configured_directories()
        .map_err(error_to_string)?;
    Ok(document_service_with_artifacts(
        project_root,
        project_root.join(".runtime").join("artifacts"),
        Some(layout.exports),
    ))
}

fn document_service_with_artifacts(
    document_root: &Path,
    artifact_root: PathBuf,
    export_root: Option<PathBuf>,
) -> FileDocumentService {
    let mut permissions = project_document_permission(document_root);
    permissions.readable_file_roots.push(artifact_root.clone());
    permissions.writable_file_roots.push(artifact_root.clone());
    if let Some(export_root) = &export_root {
        permissions.readable_file_roots.push(export_root.clone());
        permissions.writable_file_roots.push(export_root.clone());
    }
    let service = FileDocumentService::new(permissions, artifact_root);
    match export_root {
        Some(export_root) => service.with_export_root(export_root),
        None => service,
    }
}

fn workflow_document_root(
    project_root: &Path,
    workflow: &WorkflowDefinition,
    start_node_id: Option<&str>,
) -> CommandResult<PathBuf> {
    let Some(start_node_id) = start_node_id else {
        return Ok(project_root.to_path_buf());
    };
    let start_node = workflow
        .nodes
        .iter()
        .find(|node| node.id == NodeId::from(start_node_id))
        .ok_or_else(|| CommandError::not_found(format!("start node not found: {start_node_id}")))?;
    let work_dir = start_node
        .config
        .get("work_dir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|work_dir| !work_dir.is_empty())
        .unwrap_or(".");
    project_path(project_root, work_dir)
}

fn document_argument_path(
    project_root: &Path,
    document_id: Option<String>,
    path: Option<String>,
) -> CommandResult<PathBuf> {
    match (document_id, path) {
        (Some(document_id), _) if !document_id.trim().is_empty() => {
            project_path(project_root, &document_id)
        }
        (_, Some(path)) if !path.trim().is_empty() => project_path(project_root, &path),
        _ => Err(CommandError::validation("document_id or path is required")),
    }
}

fn scan_tree(project_root: &Path, root: &Path) -> CommandResult<DocumentTreeNode> {
    let mut children = Vec::new();
    if root.is_dir() {
        let mut entries = std::fs::read_dir(root)
            .map_err(error_to_string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error_to_string)?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if file_name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                children.push(scan_tree(project_root, &path)?);
            } else if is_supported_document(&path) {
                children.push(DocumentTreeNode {
                    id: relative_id(project_root, &path)?,
                    name: file_name,
                    path,
                    kind: DocumentTreeNodeKind::File,
                    children: Vec::new(),
                });
            }
        }
    }
    Ok(DocumentTreeNode {
        id: relative_id(project_root, root)?,
        name: root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("root")
            .to_owned(),
        path: root.to_path_buf(),
        kind: DocumentTreeNodeKind::Directory,
        children,
    })
}

fn workflow_path(project_root: &Path, workflow_id: Option<String>) -> CommandResult<PathBuf> {
    let workflows_root = absolute_path(&project_layout(project_root)?.workflows);
    reject_symlink_root(&workflows_root)?;
    match workflow_id {
        Some(workflow_id) if !workflow_id.trim().is_empty() => {
            let mut path = project_path(&workflows_root, &workflow_id)?;
            if path.extension().is_none() {
                path.set_extension("json");
            }
            ensure_path_under_root(&workflows_root, &path).map_err(error_to_string)?;
            Ok(path)
        }
        _ => Ok(workflows_root.join("default.json")),
    }
}

fn workflow_manifest_path(project_root: &Path, workflow_id: &str) -> CommandResult<PathBuf> {
    let workflows_root = absolute_path(&project_layout(project_root)?.workflows);
    reject_symlink_root(&workflows_root)?;
    let path = project_path(&workflows_root, workflow_id)?.join(WORKFLOW_MANIFEST_FILE);
    ensure_path_under_root(&workflows_root, &path).map_err(error_to_string)?;
    Ok(path)
}

/// F11: list in_doubt operations for a run so authors/API can recover deliberately.
pub fn list_in_doubt_operations(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
) -> CommandResult<Vec<crate::workflow::WorkflowOperation>> {
    let project_root = project_root_from_state(state, None)?;
    validate_existing_project_root(&project_root)?;
    let store = SqliteWorkflowRuntimeStore::open(&project_root).map_err(error_to_string)?;
    let ops = store
        .list_operations(&WorkflowId::from(workflow_id), &RunId::from(run_id))
        .map_err(error_to_string)?;
    Ok(ops
        .into_iter()
        .filter(|op| {
            matches!(op.status, crate::workflow::WorkflowOperationStatus::InDoubt)
                && op.response_policy
                    == crate::workflow::WorkflowOperationResponsePolicy::AllowExternalResponse
        })
        .collect())
}

/// F11 recovery: abort an in_doubt operation (mark failed, do not replay external call).
pub fn resolve_workflow_operation_in_doubt(
    state: &AriadneAppState,
    request: ResolveInDoubtOperationRequest,
) -> CommandResult<ResolveInDoubtOperationResult> {
    let project_root = project_root_from_state(state, None)?;
    validate_existing_project_root(&project_root)?;
    let project_mutation =
        acquire_project_mutation_guard(&project_root, "workflow_in_doubt_resolution")?;
    let store = SqliteWorkflowRuntimeStore::open(&project_root).map_err(error_to_string)?;
    let resolution = match request.decision {
        InDoubtDecision::Retry => crate::workflow::InDoubtResolution::Retry,
        InDoubtDecision::UseResponse => crate::workflow::InDoubtResolution::UseResponse {
            response: request
                .response
                .clone()
                .ok_or_else(|| CommandError::validation("use_response requires response"))?,
        },
        InDoubtDecision::Stop => crate::workflow::InDoubtResolution::Stop {
            reason: request
                .reason
                .clone()
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or_else(|| "in_doubt operation stopped by user".to_owned()),
        },
    };
    let owner_id = format!("worker-{}", new_run_id()?.as_str());
    let (run_state, lease) = match store
        .resolve_in_doubt_operation(
            &request.operation_id,
            resolution,
            &owner_id,
            workflow_lease_now_ms()?,
            WORKFLOW_WORKER_LEASE_TTL_MS,
        )
        .map_err(error_to_string)?
    {
        crate::workflow::InDoubtResolutionResult::Saved { state, lease } => (state, lease),
        crate::workflow::InDoubtResolutionResult::NotFound => {
            return Err(CommandError::not_found(format!(
                "operation not found: {}",
                request.operation_id
            )));
        }
        crate::workflow::InDoubtResolutionResult::NotInDoubt { status } => {
            return Err(CommandError::conflict(format!(
                "operation {} is not in_doubt (status={status:?})",
                request.operation_id
            )));
        }
        crate::workflow::InDoubtResolutionResult::Busy { .. } => {
            return Err(CommandError::conflict(format!(
                "workflow run is busy and in_doubt decision was not applied: {}",
                request.operation_id
            )));
        }
    };
    let workflow = WorkflowActionResult {
        workflow_id: run_state.workflow_id.as_str().to_owned(),
        run_id: run_state.run_id.as_str().to_owned(),
        status: run_status_label(run_state.status).to_owned(),
    };
    if let Some(lease) = lease {
        spawn_continue_workflow_worker_with_lease(
            project_root,
            Arc::clone(&state.secret_store),
            Some(state.retrieval_runtime()?),
            workflow.workflow_id.clone(),
            workflow.run_id.clone(),
            lease,
            project_mutation,
        )?;
    }
    state.ensure_workflow_scheduler()?;
    Ok(ResolveInDoubtOperationResult {
        operation_id: request.operation_id,
        decision: request.decision,
        workflow,
    })
}

fn content_revision_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn confirmation_resolution_operation_id(
    workflow_id: &str,
    run_id: &str,
    confirmation_id: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in [workflow_id, run_id, confirmation_id] {
        hasher.update(part.len().to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("confirmation-{:x}", hasher.finalize())
}

fn confirmation_resolution_request_hash(
    request: &ResolveConfirmationRequest,
) -> CommandResult<String> {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_vec(&json!({
        "workflow_id": request.workflow_id,
        "run_id": request.run_id,
        "confirmation_id": request.confirmation_id,
        "decision": request.decision,
        "review_reason": request.review_reason,
    }))
    .map_err(error_to_string)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn load_workflow_definition_with_revision(
    project_root: &Path,
    workflow_id: Option<String>,
) -> CommandResult<(WorkflowDefinition, String)> {
    let requested_workflow_id = workflow_id
        .as_deref()
        .map(str::trim)
        .filter(|workflow_id| !workflow_id.is_empty())
        .map(str::to_owned);
    let manifest_path = workflow_id
        .as_deref()
        .filter(|workflow_id| !workflow_id.trim().is_empty())
        .map(|workflow_id| workflow_manifest_path(project_root, workflow_id))
        .transpose()?;
    let path = workflow_path(project_root, workflow_id.clone())?;
    if !path.exists() {
        if let Some(manifest_path) = manifest_path.filter(|path| path.exists()) {
            let raw = std::fs::read(&manifest_path).map_err(error_to_string)?;
            let content = String::from_utf8_lossy(&raw).into_owned();
            let workflow = parse_workflow_file(&content)?;
            return Ok((workflow, content_revision_hash(&raw)));
        }
        if let Some(workflow_id) = requested_workflow_id
            .as_deref()
            .filter(|id| *id != "default")
        {
            return Err(CommandError::not_found(format!(
                "workflow not found: {workflow_id}"
            )));
        }
        return Ok((default_workflow_definition(), String::new()));
    }
    let raw = std::fs::read(&path).map_err(error_to_string)?;
    let content = String::from_utf8_lossy(&raw).into_owned();
    let workflow = parse_workflow_file(&content)?;
    Ok((workflow, content_revision_hash(&raw)))
}

fn load_workflow_definition(
    project_root: &Path,
    workflow_id: Option<String>,
) -> CommandResult<WorkflowDefinition> {
    Ok(load_workflow_definition_with_revision(project_root, workflow_id)?.0)
}

fn pack_workflow_request_hash(
    workflow_id: &str,
    selected_node_ids: &[String],
    subworkflow_node_id: Option<&str>,
    title: Option<&str>,
    expected_revision: Option<&str>,
) -> CommandResult<String> {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_vec(&json!({
        "workflow_id": workflow_id,
        "selected_node_ids": selected_node_ids,
        "subworkflow_node_id": subworkflow_node_id,
        "title": title,
        "expected_revision": expected_revision,
    }))
    .map_err(error_to_string)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn generated_pack_operation_id(
    workflow_id: &str,
    request_hash: &str,
    base_revision: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in [workflow_id, request_hash, base_revision] {
        hasher.update(part.len().to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("pack-{:x}", hasher.finalize())
}

fn validate_pack_operation_id(operation_id: String) -> CommandResult<String> {
    let operation_id = operation_id.trim().to_owned();
    if operation_id.is_empty() {
        return Err(CommandError::validation(
            "pack operation_id cannot be empty",
        ));
    }
    if operation_id.len() > 128
        || !operation_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
    {
        return Err(CommandError::validation(
            "pack operation_id must be at most 128 ASCII letters, digits, '-', '_' or ':'",
        ));
    }
    Ok(operation_id)
}

fn pack_operation_path(
    project_root: &Path,
    app_state_root: &Path,
    operation_id: &str,
) -> CommandResult<PathBuf> {
    validate_project_root(project_root)?;
    let safe = validate_pack_operation_id(operation_id.to_owned())?;
    let dir = crate::config::project_authority_dir(
        project_root,
        app_state_root,
        "workflow-pack-operations",
    )
    .map_err(error_to_string)?;
    let path = dir.join(format!("{safe}.json"));
    ensure_path_under_root(&dir, &path).map_err(error_to_string)?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(CommandError::permission(format!(
                "pack operation record must be a regular file: {}",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error_to_string(error)),
    }
    Ok(path)
}

fn persist_pack_operation_record(
    path: &Path,
    record: &WorkflowPackOperationRecord,
) -> CommandResult<()> {
    let body = serde_json::to_string_pretty(record).map_err(error_to_string)?;
    crate::config::store::atomic_write(path, body.as_bytes()).map_err(error_to_string)
}

fn load_pack_operation_record_if_exists(
    path: &Path,
    operation_id: &str,
) -> CommandResult<Option<WorkflowPackOperationRecord>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error_to_string(error)),
    };
    if let Ok(record) = serde_json::from_str::<WorkflowPackOperationRecord>(&raw) {
        if record.operation_id != operation_id {
            return Err(CommandError::conflict(format!(
                "pack operation id mismatch: requested {operation_id}, stored {}",
                record.operation_id
            )));
        }
        return Ok(Some(record));
    }
    // 兼容 2026-07-13 版直接持久化 WorkflowPackGraphReport 的旧回执。
    let mut report =
        serde_json::from_str::<WorkflowPackGraphReport>(&raw).map_err(error_to_string)?;
    if let Some(stored) = report.operation_id.as_deref() {
        if stored != operation_id {
            return Err(CommandError::conflict(format!(
                "pack operation id mismatch: requested {operation_id}, stored {stored}"
            )));
        }
    } else {
        report.operation_id = Some(operation_id.to_owned());
    }
    Ok(Some(WorkflowPackOperationRecord {
        operation_id: operation_id.to_owned(),
        request_hash: String::new(),
        expected_revision: String::new(),
        status: WorkflowPackOperationStatus::Committed,
        report,
    }))
}

fn resume_pack_operation(
    project_root: &Path,
    operation_path: &Path,
    mut record: WorkflowPackOperationRecord,
    request_hash: &str,
    loaded_revision: &str,
) -> CommandResult<WorkflowPackGraphReport> {
    if record.request_hash.is_empty() {
        return Err(CommandError::legacy_run(format!(
            "legacy pack operation {} has no request fingerprint and cannot be replayed",
            record.operation_id
        )));
    }
    if record.request_hash != request_hash {
        return Err(CommandError::conflict(format!(
            "pack operation_id {} was reused with a different request",
            record.operation_id
        )));
    }
    if record.status == WorkflowPackOperationStatus::Committed {
        return Ok(record.report);
    }

    let result_revision = record
        .report
        .workflow
        .content_revision
        .as_deref()
        .ok_or_else(|| {
            CommandError::internal("prepared pack operation is missing result revision")
        })?;
    if loaded_revision == result_revision {
        record.status = WorkflowPackOperationStatus::Committed;
        persist_pack_operation_record(operation_path, &record)?;
        return Ok(record.report);
    }
    if loaded_revision != record.expected_revision {
        return Err(CommandError::conflict(format!(
            "pack operation {} cannot resume: expected base revision {}, actual {}",
            record.operation_id, record.expected_revision, loaded_revision
        )));
    }

    let mut graph = record.report.workflow.clone();
    graph.content_revision = None;
    graph.expected_revision = Some(record.expected_revision.clone());
    let saved = save_workflow_graph_impl(project_root, graph)?;
    record.report.workflow = saved;
    record.status = WorkflowPackOperationStatus::Committed;
    persist_pack_operation_record(operation_path, &record)?;
    Ok(record.report)
}

fn load_pack_operation(
    project_root: &Path,
    app_state_root: &Path,
    operation_id: &str,
) -> CommandResult<WorkflowPackGraphReport> {
    let path = pack_operation_path(project_root, app_state_root, operation_id)?;
    let _operation_read =
        crate::config::store::PathWriteLock::acquire(&path).map_err(error_to_string)?;
    let mut record =
        load_pack_operation_record_if_exists(&path, operation_id)?.ok_or_else(|| {
            CommandError::not_found(format!("pack operation not found: {operation_id}"))
        })?;
    if record.status == WorkflowPackOperationStatus::Committed {
        return Ok(record.report);
    }

    let (_, current_revision) = load_workflow_definition_with_revision(
        project_root,
        Some(record.report.workflow.workflow_id.clone()),
    )?;
    let result_revision = record
        .report
        .workflow
        .content_revision
        .as_deref()
        .ok_or_else(|| {
            CommandError::internal("prepared pack operation is missing result revision")
        })?;
    if current_revision == result_revision {
        record.status = WorkflowPackOperationStatus::Committed;
        persist_pack_operation_record(&path, &record)?;
        return Ok(record.report);
    }
    if current_revision == record.expected_revision {
        let mut graph = record.report.workflow.clone();
        graph.content_revision = None;
        graph.expected_revision = Some(record.expected_revision.clone());
        let saved = save_workflow_graph_impl(project_root, graph)?;
        record.report.workflow = saved;
        record.status = WorkflowPackOperationStatus::Committed;
        persist_pack_operation_record(&path, &record)?;
        return Ok(record.report);
    }
    Err(CommandError::conflict(format!(
        "pack operation {operation_id} cannot be recovered: expected base revision {}, result revision {result_revision}, actual {current_revision}",
        record.expected_revision
    )))
}

fn default_workflow_definition() -> WorkflowDefinition {
    WorkflowDefinition {
        id: WorkflowId::from("default"),
        name: "Default Workflow".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
    }
}

fn parse_workflow_file(content: &str) -> CommandResult<WorkflowDefinition> {
    match serde_json::from_str::<WorkflowDefinition>(content) {
        Ok(workflow) => Ok(workflow),
        Err(workflow_error) => {
            let manifest = serde_json::from_str::<WorkflowManifest>(content).map_err(|error| {
                CommandError::serialization(format!(
                    "invalid workflow JSON: {workflow_error}; invalid workflow manifest: {error}"
                ))
            })?;
            manifest.import_definition().map_err(error_to_string)
        }
    }
}

fn workflow_to_graph(workflow: WorkflowDefinition) -> WorkflowGraphData {
    WorkflowGraphData {
        workflow_id: workflow.id.as_str().to_owned(),
        name: workflow.name,
        nodes: workflow
            .nodes
            .into_iter()
            .map(|node| CanvasNode {
                id: node.id.as_str().to_owned(),
                r#type: node.type_name,
                label: node.label,
                data: node.config,
                position: node
                    .position
                    .map(|position| json!({ "x": position.x, "y": position.y }))
                    .unwrap_or_else(|| json!({ "x": 0.0, "y": 0.0 })),
            })
            .collect(),
        edges: workflow
            .edges
            .into_iter()
            .map(|edge| CanvasEdge {
                id: edge.id.as_str().to_owned(),
                source: edge.from.node_id.as_str().to_owned(),
                target: edge.to.node_id.as_str().to_owned(),
                source_handle: edge.from.port_name,
                target_handle: edge.to.port_name,
                kind: edge.kind,
                label: edge.alias,
                data: edge
                    .communication
                    .map(serde_json::to_value)
                    .transpose()
                    .unwrap_or(None)
                    .unwrap_or(Value::Null),
            })
            .collect(),
        metadata: workflow.metadata,
        content_revision: None,
        expected_revision: None,
    }
}

fn graph_to_workflow(graph: WorkflowGraphData) -> CommandResult<WorkflowDefinition> {
    Ok(WorkflowDefinition {
        id: WorkflowId::from(non_empty_or("workflow_id", graph.workflow_id)?),
        name: non_empty_or("workflow name", graph.name)?,
        nodes: graph
            .nodes
            .into_iter()
            .map(|node| NodeInstance {
                id: NodeId::from(node.id),
                type_name: node.r#type,
                label: node.label,
                config: node.data,
                position: parse_position(node.position),
            })
            .collect(),
        edges: graph
            .edges
            .into_iter()
            .map(|edge| {
                let communication = if edge.kind == WorkflowEdgeKind::Communication {
                    Some(serde_json::from_value(edge.data.clone()).map_err(error_to_string)?)
                } else {
                    None
                };
                let source_handle = edge.source_handle;
                let target_handle = edge.target_handle;
                let alias = if edge.kind == WorkflowEdgeKind::Data {
                    edge.label
                        .as_deref()
                        .map(str::trim)
                        .filter(|label| !label.is_empty())
                        .map(str::to_owned)
                        .or_else(|| Some(default_data_edge_alias(&target_handle)))
                } else {
                    edge.label
                };
                Ok(Edge {
                    id: EdgeId::from(edge.id),
                    kind: edge.kind,
                    from: PortEndpoint {
                        node_id: NodeId::from(edge.source),
                        port_name: source_handle,
                    },
                    to: PortEndpoint {
                        node_id: NodeId::from(edge.target),
                        port_name: target_handle,
                    },
                    alias,
                    communication,
                })
            })
            .collect::<CommandResult<Vec<_>>>()?,
        metadata: graph.metadata,
    })
}

fn default_data_edge_alias(target_handle: &str) -> String {
    let trimmed = target_handle.trim();
    let alias = trimmed
        .strip_prefix("data-in-")
        .or_else(|| trimmed.strip_prefix("in-"))
        .unwrap_or(trimmed);
    if alias.is_empty() || alias == "input" || alias == "in" {
        "input".to_owned()
    } else {
        alias.to_owned()
    }
}

fn parse_position(value: Value) -> Option<crate::contracts::CanvasPosition> {
    Some(crate::contracts::CanvasPosition {
        x: value.get("x")?.as_f64()?,
        y: value.get("y")?.as_f64()?,
    })
}

fn ensure_provider_config<'a>(
    providers: &'a mut Vec<ProviderConfig>,
    provider: &str,
) -> &'a mut ProviderConfig {
    if let Some(index) = providers
        .iter()
        .position(|existing| existing.provider_id == provider)
    {
        return &mut providers[index];
    }

    providers.push(ProviderConfig {
        provider_id: provider.to_owned(),
        provider_type: match provider {
            "openai" => crate::contracts::ProviderType::OpenAi,
            "anthropic" => crate::contracts::ProviderType::Anthropic,
            "gemini" => crate::contracts::ProviderType::Gemini,
            _ => crate::contracts::ProviderType::OpenAiCompatible,
        },
        display_name: provider.to_owned(),
        enabled: true,
        base_url: (provider == "openai_compatible").then(|| "http://127.0.0.1:11434/v1".to_owned()),
        api_key: None,
        models: Vec::new(),
    });
    providers.last_mut().expect("provider was just pushed")
}

fn provider_status_list(project_root: &Path, configured: &[ProviderConfig]) -> Vec<ProviderConfig> {
    let mut providers = default_provider_status_configs(project_root);
    for configured_provider in configured {
        if let Some(existing) = providers
            .iter_mut()
            .find(|provider| provider.provider_id == configured_provider.provider_id)
        {
            *existing = configured_provider.clone();
        } else {
            providers.push(configured_provider.clone());
        }
    }
    providers
}

fn default_provider_status_configs(_project_root: &Path) -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: false,
            base_url: None,
            api_key: None,
            models: Vec::new(),
        },
        ProviderConfig {
            provider_id: "anthropic".to_owned(),
            provider_type: ProviderType::Anthropic,
            display_name: "Anthropic".to_owned(),
            enabled: false,
            base_url: None,
            api_key: None,
            models: Vec::new(),
        },
        ProviderConfig {
            provider_id: "gemini".to_owned(),
            provider_type: ProviderType::Gemini,
            display_name: "Gemini".to_owned(),
            enabled: false,
            base_url: None,
            api_key: None,
            models: Vec::new(),
        },
    ]
}

fn default_llm_model_for_provider(provider: &str) -> ModelConfig {
    ModelConfig {
        model_id: match provider {
            "openai" => "gpt-4.1-mini",
            "anthropic" => "claude-3-5-sonnet-latest",
            "gemini" => "gemini-1.5-pro",
            _ => "default",
        }
        .to_owned(),
        capability: ProviderCapability::Llm,
        max_context_tokens: None,
        input_cost_per_million_tokens: None,
        output_cost_per_million_tokens: None,
    }
}

fn default_embedding_model_for_provider(provider: &str) -> Option<ModelConfig> {
    let model_id = match provider {
        "openai" => "text-embedding-3-small",
        "gemini" => "text-embedding-004",
        _ => return None,
    };
    Some(ModelConfig {
        model_id: model_id.to_owned(),
        capability: ProviderCapability::Embedding,
        max_context_tokens: None,
        input_cost_per_million_tokens: None,
        output_cost_per_million_tokens: None,
    })
}

fn normalize_provider(provider: &str) -> CommandResult<String> {
    let provider = provider.trim().to_lowercase().replace('-', "_");
    if provider.is_empty() {
        return Err(CommandError::validation("provider cannot be empty"));
    }
    Ok(provider)
}

fn validate_project_root(project_root: &Path) -> CommandResult<()> {
    if project_root.as_os_str().is_empty() {
        return Err(CommandError::validation("project_root cannot be empty"));
    }
    if !project_root.exists() {
        return Err(CommandError::not_found(format!(
            "project root does not exist: {}",
            project_root.display()
        )));
    }
    if !project_root.is_dir() {
        return Err(CommandError::validation(format!(
            "project root is not a directory: {}",
            project_root.display()
        )));
    }
    Ok(())
}

fn validate_existing_project_root(project_root: &Path) -> CommandResult<()> {
    validate_project_root(project_root)
}

fn validate_initialized_project_root(project_root: &Path) -> CommandResult<()> {
    validate_existing_project_root(project_root)?;
    let app_config = project_root.join(".config").join("app.yaml");
    if !app_config.is_file() {
        return Err(CommandError::validation(format!(
            "project root is not initialized (missing .config/app.yaml): {}",
            project_root.display()
        )));
    }
    Ok(())
}

fn canonicalize_initialized_project_root(project_root: &Path) -> CommandResult<PathBuf> {
    validate_initialized_project_root(project_root)?;
    project_root.canonicalize().map_err(error_to_string)
}

fn validate_money(field: &str, value: f64) -> CommandResult<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(CommandError::validation(format!(
            "{field} must be finite and non-negative"
        )))
    }
}

fn project_path(root: &Path, input: &str) -> CommandResult<PathBuf> {
    project_path_buf(root, Path::new(input))
}

fn project_path_buf(root: &Path, input: &Path) -> CommandResult<PathBuf> {
    let raw = input;
    let path = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    ensure_no_parent_traversal(&path)?;
    ensure_path_under_root(root, &path).map_err(error_to_string)?;
    Ok(path)
}

fn ensure_no_parent_traversal(path: &Path) -> CommandResult<()> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CommandError::permission("path cannot contain '..'"));
    }
    Ok(())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn reject_symlink_root(path: &Path) -> CommandResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(CommandError::permission(format!(
                "workflow root cannot be a symbolic link: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error_to_string(error)),
    }
}

fn workflow_branch_from_start(
    workflow: &WorkflowDefinition,
    start_node_id: &NodeId,
) -> CommandResult<WorkflowDefinition> {
    let start_node = workflow
        .nodes
        .iter()
        .find(|node| node.id == *start_node_id)
        .ok_or_else(|| {
            CommandError::not_found(format!("start node not found: {}", start_node_id.as_str()))
        })?;
    if start_node.type_name != "start" {
        return Err(CommandError::validation(format!(
            "start_node_id must reference a start node, got {} ({})",
            start_node_id.as_str(),
            start_node.type_name
        )));
    }

    let reachable = reachable_nodes_from_start(workflow, start_node_id);
    let nodes = workflow
        .nodes
        .iter()
        .filter(|node| reachable.contains(&node.id))
        .cloned()
        .collect();
    let edges = workflow
        .edges
        .iter()
        .filter(|edge| {
            reachable.contains(&edge.from.node_id) && reachable.contains(&edge.to.node_id)
        })
        .cloned()
        .collect();
    let branch = WorkflowDefinition {
        id: workflow.id.clone(),
        name: workflow.name.clone(),
        nodes,
        edges,
        metadata: workflow.metadata.clone(),
    };
    branch.validate_topology().map_err(error_to_string)?;
    Ok(branch)
}

fn reachable_nodes_from_start(
    workflow: &WorkflowDefinition,
    start_node_id: &NodeId,
) -> Vec<NodeId> {
    let mut reachable_set = HashSet::new();
    let mut reachable = Vec::new();
    let mut stack = vec![start_node_id.clone()];
    while let Some(node_id) = stack.pop() {
        if reachable_set.contains(&node_id) {
            continue;
        }
        reachable_set.insert(node_id.clone());
        reachable.push(node_id.clone());
        for edge in workflow
            .edges
            .iter()
            .filter(|edge| edge.from.node_id == node_id)
        {
            if !reachable_set.contains(&edge.to.node_id) {
                stack.push(edge.to.node_id.clone());
            }
        }
    }
    reachable
}

fn relative_id(project_root: &Path, path: &Path) -> CommandResult<String> {
    path.strip_prefix(project_root)
        .map(|relative| relative.to_string_lossy().into_owned())
        .map_err(error_to_string)
}

fn is_supported_document(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| matches!(extension, "md" | "markdown" | "txt" | "json"))
        .unwrap_or(false)
}

fn default_node_type() -> String {
    "agent".to_owned()
}

fn default_source_handle() -> String {
    crate::contracts::EXECUTION_OUTPUT_PORT.to_owned()
}

fn default_target_handle() -> String {
    crate::contracts::EXECUTION_INPUT_PORT.to_owned()
}

fn non_empty_or(field: &str, value: String) -> CommandResult<String> {
    if value.trim().is_empty() {
        Err(CommandError::validation(format!("{field} cannot be empty")))
    } else {
        Ok(value)
    }
}

fn now_timestamp_ms() -> CommandResult<u128> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(error_to_string)
}

fn new_run_id() -> CommandResult<RunId> {
    Ok(RunId::from(format!(
        "run-{}-{:04x}",
        now_timestamp_ms()?,
        simple_random_u16()
    )))
}

fn error_to_string(error: impl Into<CommandError>) -> CommandError {
    error.into()
}

/// 生成一个简单的随机 u16，用于 run_id 后缀防止碰撞。
/// 不依赖 rand crate，直接从操作系统获取随机字节。
fn simple_random_u16() -> u16 {
    use std::fs::File;
    use std::io::Read;
    let mut buf = [0u8; 2];
    if File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok()
    {
        u16::from_ne_bytes(buf)
    } else {
        // fallback: 用高精度时间戳低 16 位
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u16)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod permission_and_preset_resolution_tests {
    use super::*;

    #[test]
    fn tool_resolution_uses_global_then_scope_then_node_preset() {
        let mut controls = normalize_tool_controls(BTreeMap::new());
        controls
            .get_mut("global")
            .unwrap()
            .insert("search".to_owned(), Some(false));
        let mut presets = NodePresetSettings::default();

        assert!(!node_tool_control_enabled(
            &controls,
            &presets,
            "writer",
            "writer",
            "writer-search"
        ));

        controls
            .get_mut("writer")
            .unwrap()
            .insert("writer-search".to_owned(), Some(true));
        assert!(node_tool_control_enabled(
            &controls,
            &presets,
            "writer",
            "writer",
            "writer-search"
        ));

        presets
            .presets
            .iter_mut()
            .find(|preset| preset.node_type == "writer")
            .unwrap()
            .tool_controls
            .insert("search".to_owned(), Some(false));
        assert!(!node_tool_control_enabled(
            &controls,
            &presets,
            "writer",
            "writer",
            "writer-search"
        ));
    }

    #[test]
    fn permission_resolution_separates_project_ai_workflow_and_node_preset() {
        let mut permissions = crate::config::PermissionsConfig::default();
        permissions.policy.allow_web_search = false;
        let mut workflow_policy = permissions.policy.clone();
        workflow_policy.allow_network = true;
        workflow_policy.allow_web_search = true;
        permissions
            .scoped_policies
            .insert("workflow_nodes".to_owned(), Some(workflow_policy));

        let mut presets = NodePresetSettings::default();
        assert!(
            permission_policy_for_node(&permissions, &presets, "writer")
                .evaluate(&PermissionRequest::WebSearch)
                .allowed
        );
        assert!(
            !permission_policy_for_scope(&permissions, "project_ai")
                .evaluate(&PermissionRequest::WebSearch)
                .allowed
        );

        presets
            .presets
            .iter_mut()
            .find(|preset| preset.node_type == "writer")
            .unwrap()
            .permission_policy = Some(PermissionPolicy::default());
        assert!(
            !permission_policy_for_node(&permissions, &presets, "writer")
                .evaluate(&PermissionRequest::WebSearch)
                .allowed
        );
    }

    #[test]
    fn node_preset_fills_only_missing_runtime_values() {
        let preset = NodeTypePreset {
            node_type: "writer".to_owned(),
            display_name_key: "agent.writer".to_owned(),
            provider_id: "preset-provider".to_owned(),
            model_id: "preset-model".to_owned(),
            timeout_ms: 456_000,
            budget_usd: 0.4,
            permission_policy: None,
            tool_controls: BTreeMap::new(),
        };
        let request = crate::workflow::WorkflowNodeExecutionRequest {
            workflow_id: WorkflowId::from("flow"),
            run_id: RunId::from("run"),
            node_id: NodeId::from("writer"),
            operation_id: "operation".to_owned(),
            operation_attempt: 1,
            request_hash: "hash".to_owned(),
            type_name: "writer".to_owned(),
            config: json!({"prompt_template":"write"}),
            inputs: Default::default(),
            communication_messages: Vec::new(),
            metadata: Value::Null,
            cancellation: Default::default(),
            dispatch_authorization: Default::default(),
        };

        let applied = apply_node_type_preset(request, Some(&preset));
        assert_eq!(applied.config["provider_id"], "preset-provider");
        assert_eq!(applied.config["model_id"], "preset-model");
        assert_eq!(applied.config["timeout_ms"], 456_000);
        assert_eq!(applied.config["budget_usd"], 0.4);

        let explicit = crate::workflow::WorkflowNodeExecutionRequest {
            config: json!({
                "provider_id":"node-provider",
                "model_id":"node-model",
                "timeout_ms":12_000,
                "budget_usd":0.2
            }),
            ..applied
        };
        let explicit = apply_node_type_preset(explicit, Some(&preset));
        assert_eq!(explicit.config["provider_id"], "node-provider");
        assert_eq!(explicit.config["model_id"], "node-model");
        assert_eq!(explicit.config["timeout_ms"], 12_000);
        assert_eq!(explicit.config["budget_usd"], 0.2);
    }

    #[test]
    fn scheduler_reuses_shared_runtime_for_implicit_writer_search() {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(crate::config::MemorySecretStore::default());
        let state = AriadneAppState::new(temp.path(), app_state.path(), secrets.clone());
        let status = save_provider_settings(
            &state,
            ProviderSettingsUpdate {
                provider_id: "writer-provider".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "Writer Provider".to_owned(),
                enabled: true,
                base_url: Some("http://127.0.0.1:41003".to_owned()),
                models: vec![ModelConfig {
                    model_id: "writer-model".to_owned(),
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
        let provider_id = status
            .providers
            .iter()
            .find(|provider| provider.configured)
            .map(|provider| provider.provider.clone())
            .expect("saved provider must be returned with its canonical identity");
        assert_eq!(provider_id, "writer_provider");
        let credentials = ProjectCredentialScope::new(temp.path(), secrets.as_ref()).unwrap();
        credentials
            .set_provider_secret(&provider_id, SecretValue::new("writer-secret"))
            .unwrap();

        let workflow = WorkflowDefinition {
            id: WorkflowId::from("implicit-writer-search"),
            name: "Implicit Writer Search".to_owned(),
            nodes: vec![NodeInstance {
                id: NodeId::from("writer"),
                type_name: "writer".to_owned(),
                label: None,
                config: json!({
                    "provider_id": provider_id,
                    "model_id": "writer-model",
                    "prompt_template": "write with project context"
                }),
                position: None,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        };
        let plan =
            compile_workflow_runtime_dependency_plan(temp.path(), secrets.as_ref(), &workflow)
                .unwrap();
        assert!(plan.requires_project_retrieval);
        assert!(!plan.workflow.uses_node_type("search"));

        let shared = Arc::clone(&state.retrieval_runtime);
        let configured_runtime = shared
            .lock()
            .unwrap()
            .as_ref()
            .cloned()
            .expect("provider save must retain the project shared runtime");
        let first =
            scheduled_workflow_retrieval_runtime(temp.path(), secrets.as_ref(), &shared, &plan)
                .unwrap()
                .expect("implicit writer search must open the shared runtime");
        let second =
            scheduled_workflow_retrieval_runtime(temp.path(), secrets.as_ref(), &shared, &plan)
                .unwrap()
                .expect("second recovery must reuse the shared runtime");
        assert!(Arc::ptr_eq(&configured_runtime, &first));
        assert!(Arc::ptr_eq(&first, &second));

        let worker_key = temp.path().canonicalize().unwrap();
        for _ in 0..200 {
            let worker_finished = !active_indexing_workers()
                .lock()
                .unwrap()
                .contains(&worker_key);
            if worker_finished {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!active_indexing_workers()
            .lock()
            .unwrap()
            .contains(&worker_key));
    }

    #[test]
    fn frozen_workflow_dependencies_ignore_later_provider_config_and_reject_credential_rotation() {
        let temp = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
        let secrets = crate::config::MemorySecretStore::default();
        save_provider_settings_impl(
            temp.path(),
            ProviderSettingsUpdate {
                provider_id: "declared".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "Declared".to_owned(),
                enabled: true,
                base_url: Some("http://127.0.0.1:41001".to_owned()),
                models: vec![ModelConfig {
                    model_id: "frozen-model".to_owned(),
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
        let credentials = ProjectCredentialScope::new(temp.path(), &secrets).unwrap();
        credentials
            .set_provider_secret("declared", SecretValue::new("first-secret"))
            .unwrap();
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("frozen-dependencies"),
            name: "Frozen Dependencies".to_owned(),
            nodes: vec![NodeInstance {
                id: NodeId::from("ask"),
                type_name: "llm".to_owned(),
                label: None,
                config: json!({
                    "provider_id": "declared",
                    "model_id": "frozen-model",
                    "prompt_template": "test"
                }),
                position: None,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        };
        let plan =
            compile_workflow_runtime_dependency_plan(temp.path(), &secrets, &workflow).unwrap();
        let frozen = freeze_workflow_runtime_dependency_plan(&plan);
        let frozen_value = serde_json::to_value(&frozen).unwrap();
        assert!(!frozen_value.to_string().contains("first-secret"));

        save_provider_settings_impl(
            temp.path(),
            ProviderSettingsUpdate {
                provider_id: "declared".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "Declared Changed".to_owned(),
                enabled: true,
                base_url: Some("http://127.0.0.1:41002".to_owned()),
                models: vec![ModelConfig {
                    model_id: "changed-model".to_owned(),
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

        let recovered = materialize_frozen_workflow_runtime_dependency_plan(
            temp.path(),
            &secrets,
            &workflow,
            &frozen_value,
        )
        .unwrap();
        let provider = recovered
            .project_config
            .providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == "declared")
            .unwrap();
        assert_eq!(provider.base_url.as_deref(), Some("http://127.0.0.1:41001"));
        assert_eq!(
            recovered.llm_execution.node_routes["ask"].model_id,
            "frozen-model"
        );

        credentials
            .set_provider_secret("declared", SecretValue::new("rotated-secret"))
            .unwrap();
        let error = materialize_frozen_workflow_runtime_dependency_plan(
            temp.path(),
            &secrets,
            &workflow,
            &frozen_value,
        )
        .err()
        .expect("credential rotation must invalidate the frozen run");
        assert!(error
            .diagnostic_text()
            .contains("credential generation changed"));
    }

    #[test]
    fn scheduler_initialization_failure_marks_claimed_run_failed() {
        let temp = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(crate::config::MemorySecretStore::default());
        save_provider_settings_impl(
            temp.path(),
            ProviderSettingsUpdate {
                provider_id: "scheduled".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "Scheduled".to_owned(),
                enabled: true,
                base_url: Some("http://127.0.0.1:41004".to_owned()),
                models: vec![ModelConfig {
                    model_id: "scheduled-model".to_owned(),
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
        let credentials = ProjectCredentialScope::new(temp.path(), secrets.as_ref()).unwrap();
        credentials
            .set_provider_secret("scheduled", SecretValue::new("initial-secret"))
            .unwrap();
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("scheduler-initialization-failure"),
            name: "Scheduler Initialization Failure".to_owned(),
            nodes: vec![NodeInstance {
                id: NodeId::from("ask"),
                type_name: "llm".to_owned(),
                label: None,
                config: json!({
                    "provider_id": "scheduled",
                    "model_id": "scheduled-model",
                    "prompt_template": "test"
                }),
                position: None,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        };
        let dependency_plan =
            compile_workflow_runtime_dependency_plan(temp.path(), secrets.as_ref(), &workflow)
                .unwrap();
        let run_id = RunId::from("scheduler-initialization-run");
        let mut state = WorkflowRuntime::new(&workflow, run_id.clone())
            .unwrap()
            .state;
        state.prepared_workflow = Some(workflow.clone());
        state.prepared_dependency_plan = Some(
            serde_json::to_value(freeze_workflow_runtime_dependency_plan(&dependency_plan))
                .unwrap(),
        );
        let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
        store.create_state(&state).unwrap();

        credentials
            .set_provider_secret("scheduled", SecretValue::new("rotated-secret"))
            .unwrap();
        workflow_scheduler_tick(
            temp.path(),
            &secrets,
            &Arc::new(Mutex::new(None)),
            "scheduler-initialization-test",
        )
        .unwrap();

        let failed = store
            .load_state(&workflow.id, &run_id)
            .unwrap()
            .expect("scheduler must retain a terminal run snapshot");
        assert_eq!(failed.status, crate::contracts::RunStatus::Failed);
        let failure = failed
            .failure
            .expect("scheduler failure must be structured");
        assert_eq!(failure.code, "workflow_scheduler_initialization_failed");
        assert_eq!(failure.stage, "workflow_scheduler_initialization");
        assert!(failure.message.contains("credential generation changed"));
    }

    #[test]
    fn provider_updates_reject_feature_flags_as_model_roles_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        crate::frontend::initialize_project(temp.path()).unwrap();
        let baseline = ConfigStore::new(temp.path()).load().unwrap();

        for capability in [ProviderCapability::Streaming, ProviderCapability::ToolUse] {
            let error = save_provider_settings_impl(
                temp.path(),
                ProviderSettingsUpdate {
                    provider_id: "invalid-role".to_owned(),
                    provider_type: ProviderType::OpenAiCompatible,
                    display_name: "Invalid Role".to_owned(),
                    enabled: true,
                    base_url: Some("https://example.invalid/v1".to_owned()),
                    models: vec![ModelConfig {
                        model_id: "feature-model".to_owned(),
                        capability,
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
            .expect_err("feature flags must not be persisted as singular model roles");
            assert!(error.diagnostic_text().contains("executable provider role"));
            assert_eq!(ConfigStore::new(temp.path()).load().unwrap(), baseline);
        }
    }
}
