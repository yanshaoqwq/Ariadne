use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
    read_confirmation_policy_settings, AppConfig, ApprovalPromptConfig, ConfigStore, GitConfig,
    ModelConfig, ProjectConfig, ProjectCredentialScope, ProviderConfig, RagConfig, SecretStore,
    SecretValue, WorkflowConfig,
};
use crate::contracts::{
    ensure_path_under_root, ApprovalPolicy, ArtifactKind, CoreResult, Edge, EdgeId, NodeId,
    NodeInstance, PermissionPolicy, PermissionRequest, PortEndpoint, ProviderCapability,
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
    extract_project_reference_tokens, import_chapter_document, initialize_project,
    pack_workflow_selection as pack_workflow_selection_in_workflow, project_ai_context_window,
    project_ai_summary_context, project_document_permission, structured_project_memory_context,
    upsert_canvas_annotation as upsert_canvas_annotation_in_workflow, ArtifactReferenceEntry,
    CanvasAnnotation, ChapterExportFormat, ChapterImportRequest, CombinedExportReport,
    ConfirmationLogEntry, FileConfirmationLogStore, NodeDetailPatch, ProjectAiAppendOutcome,
    ProjectAiContextWindow, ProjectAiConversationStore, ProjectAiMemoryEntry,
    ProjectAiStoredMessage, ProjectAiSummaryChunk, ProjectInitReport, ProjectMemoryStore,
    ProjectReference, ProjectReferenceResolver, ProjectRegistryStore, QuickEditResult,
    QuickEditService, RecentProjectEntry, SidebarBadgeCounts, TemplateDetail,
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
use crate::providers::{
    web_search_tool_definition, ContentPart, HttpWebSearchProvider, LlmMessage, LlmRole,
    OpenAiCompatibleLlmProvider, Provider, ProviderProtocol, SearchProvider, ToolDefinition,
    WebSearchToolExecutor, EXECUTOR_ADAPTER_WEB_SEARCH_TOOL, GENERIC_LLM_WEB_SEARCH_TOOL,
    PROJECT_AI_WEB_SEARCH_TOOL, SUMMARIZER_WEB_SEARCH_TOOL,
};
use crate::rag::SqliteWritingKnowledgeStore;
use crate::retrieval::{
    project_search_tool_definition, ProjectRetrievalRuntime, ProjectSearchToolExecutor,
    RetrievalResult, EXECUTOR_ADAPTER_SEARCH_TOOL, GENERIC_LLM_SEARCH_TOOL, PROJECT_AI_SEARCH_TOOL,
    SUMMARIZER_SEARCH_TOOL,
};
use crate::skills::{SkillLoader, WorkflowManifest, WORKFLOW_MANIFEST_FILE};
use crate::workflow::{
    execute_document_read_node_with_root, execute_llm_node_with_defaults,
    execute_llm_node_with_search_tools, execute_project_retrieval_node_for_project,
    execute_summarizer_node, execute_summarizer_node_with_search_tools,
    validate_workflow_execution_contracts, BuiltinWorkflowNodeExecutor, DocumentWorkflowExportSink,
    RoutedExternalNodeExecutor, RuntimeConfirmation, RuntimeConfirmationState,
    SqliteWorkflowRuntimeStore, WorkflowLlmSearchOptions, WorkflowRunFailure,
    WorkflowRunnableClaimResult, WorkflowRuntime, WorkflowRuntimeEvent, WorkflowRuntimeEventType,
    WorkflowRuntimeStore, WorkflowStopRequestResult, WorkflowWorkerLease,
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
}

impl Drop for WorkflowSchedulerHandle {
    fn drop(&mut self) {
        let _ = self.stop_sender.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 桌面前端共享状态。project_root 可由 Avalonia IPC 显式设置，也可用环境变量/当前目录兜底。
#[derive(Clone)]
pub struct AriadneAppState {
    project_root: Arc<Mutex<PathBuf>>,
    app_state_root: PathBuf,
    secret_store: Arc<dyn SecretStore>,
    retrieval_runtime: Arc<Mutex<Option<Arc<ProjectRetrievalRuntime>>>>,
    workflow_scheduler: Arc<Mutex<Option<WorkflowSchedulerHandle>>>,
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

    fn ensure_workflow_scheduler(&self) -> CommandResult<()> {
        let project_root = canonicalize_initialized_project_root(&self.project_root()?)?;
        let retrieval_runtime = Arc::clone(&self.retrieval_runtime);
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
        drop(slot.take());
        let (stop_sender, stop_receiver) = std::sync::mpsc::channel::<()>();
        let scheduler_root = project_root.clone();
        let secrets = Arc::clone(&self.secret_store);
        let thread = std::thread::Builder::new()
            .name("ariadne-workflow-scheduler".to_owned())
            .spawn(move || {
                workflow_scheduler_loop(scheduler_root, secrets, retrieval_runtime, stop_receiver);
            })
            .map_err(error_to_string)?;
        *slot = Some(WorkflowSchedulerHandle {
            project_root,
            stop_sender,
            thread: Some(thread),
        });
        Ok(())
    }

    pub fn set_project_root(&self, project_root: impl Into<PathBuf>) -> CommandResult<()> {
        let project_root = project_root.into();
        let project_root = canonicalize_initialized_project_root(&project_root)?;
        crate::config::bind_project_app_state(&project_root, &self.app_state_root)
            .map_err(error_to_string)?;
        let mut runtime_slot = self
            .retrieval_runtime
            .lock()
            .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?;
        if runtime_slot
            .as_ref()
            .is_some_and(|runtime| runtime.project_root() == project_root)
        {
            let mut locked = self
                .project_root
                .lock()
                .map_err(|_| CommandError::internal("project root lock poisoned"))?;
            *locked = project_root;
            drop(locked);
            drop(runtime_slot);
            if let Ok(mut scheduler_slot) = self.workflow_scheduler.lock() {
                drop(scheduler_slot.take());
            }
            return Ok(());
        }
        let runtime = Arc::new(
            ProjectRetrievalRuntime::open(&project_root, self.secret_store.as_ref())
                .map_err(error_to_string)?,
        );
        let mut locked = self
            .project_root
            .lock()
            .map_err(|_| CommandError::internal("project root lock poisoned"))?;
        *locked = project_root;
        let previous = runtime_slot.replace(runtime);
        drop(locked);
        drop(runtime_slot);
        if let Ok(mut scheduler_slot) = self.workflow_scheduler.lock() {
            drop(scheduler_slot.take());
        }
        // 旧运行时可能仍被已领取的索引任务持有；最后一个 Arc 释放时再停止 sidecar，
        // 避免项目切换在任务中途杀掉其基础设施。
        drop(previous);
        Ok(())
    }

    /// 返回当前项目唯一的检索组合根；测试直接构造 state 时按需初始化。
    pub fn retrieval_runtime(&self) -> CommandResult<Arc<ProjectRetrievalRuntime>> {
        let project_root = self.project_root()?;
        let canonical_root = canonicalize_initialized_project_root(&project_root)?;
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
        let project_root = canonicalize_initialized_project_root(&self.project_root()?)?;
        let config = ConfigStore::new(&project_root)
            .load_or_create()
            .map_err(error_to_string)?;
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

        let index_changed =
            ProjectRetrievalRuntime::index_configuration_changed(expected, &candidate);
        let vector_store_requires_exclusive_reopen = runtime_slot.as_ref().is_some_and(|runtime| {
            runtime.vector_enabled()
                && candidate.rag.vector_store.enabled
                && !runtime.uses_vector_config(&candidate.rag.vector_store)
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
            &candidate,
            runtime_slot.as_deref(),
        ) {
            Ok(runtime) => Arc::new(runtime),
            Err(error) => {
                restore_retrieval_runtime(
                    &mut runtime_slot,
                    &project_root,
                    self.secret_store.as_ref(),
                    expected,
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
                expected,
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
                    expected,
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
    pub tool_controls: BTreeMap<String, BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePresetSettings {
    #[serde(default)]
    pub presets: Vec<NodeTypePreset>,
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
    pub model_id: String,
    pub timeout_ms: u64,
    pub budget_usd: f64,
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
    let project_root = PathBuf::from(project_root);
    std::fs::create_dir_all(&project_root).map_err(error_to_string)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "project_create")?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    let report = initialize_project(&project_root).map_err(error_to_string)?;
    persist_project_name(&project_root, name.as_deref())?;
    state.set_project_root(project_root.clone())?;
    record_recent_project(state.app_state_root(), name, &project_root)?;
    Ok(report)
}

pub fn open_project(
    state: &AriadneAppState,
    project_root: String,
    name: Option<String>,
) -> CommandResult<CurrentProjectStatus> {
    let project_root = PathBuf::from(project_root);
    validate_initialized_project_root(&project_root)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "project_open")?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    ensure_project_config(&project_root)?;
    persist_project_name(&project_root, name.as_deref())?;
    state.set_project_root(project_root.clone())?;
    record_recent_project(state.app_state_root(), name, &project_root)?;
    recover_confirmation_resolution_sagas(&project_root)?;
    // F2-a：打开时检测空索引并幂等入队 full rebuild，再 resume worker。
    ensure_index_bootstrap_on_open(&project_root)?;
    resume_indexing_worker_for_state(state)?;
    // F10-d：create/lease/spawn 崩溃窗口留下的 Queued/Running 孤儿，打开项目时 claim 并续跑。
    recover_orphaned_workflow_workers(
        &project_root,
        Arc::clone(&state.secret_store),
        Some(state.retrieval_runtime()?),
    )?;
    state.ensure_workflow_scheduler()?;
    current_project_status(&project_root)
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
    let project_root = PathBuf::from(project_root);
    validate_initialized_project_root(&project_root)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "project_open")?;
    crate::config::bind_project_app_state(&project_root, state.app_state_root())
        .map_err(error_to_string)?;
    ensure_project_config(&project_root)?;
    state.set_project_root(project_root.clone())?;
    recover_confirmation_resolution_sagas(&project_root)?;
    ensure_index_bootstrap_on_open(&project_root)?;
    resume_indexing_worker_for_state(state)?;
    // F10-d：切换/绑定项目根时同样恢复孤儿 Queued/Running。
    recover_orphaned_workflow_workers(
        &project_root,
        Arc::clone(&state.secret_store),
        Some(state.retrieval_runtime()?),
    )?;
    state.ensure_workflow_scheduler()?;
    Ok(())
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
    let documents = document_service(&project_root);
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
    let project_root = project_root_from_state(state, None)?;
    get_permissions_settings_impl(&project_root)
}

pub fn save_permissions_settings(
    state: &AriadneAppState,
    settings: PermissionsSettings,
) -> CommandResult<PermissionsSettings> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "permissions_settings_update")?;
    save_permissions_settings_impl(&project_root, settings)?;
    get_permissions_settings_impl(&project_root)
}

pub fn get_node_preset_settings(state: &AriadneAppState) -> CommandResult<NodePresetSettings> {
    let project_root = project_root_from_state(state, None)?;
    get_node_preset_settings_impl(&project_root)
}

pub fn save_node_preset_settings(
    state: &AriadneAppState,
    settings: NodePresetSettings,
) -> CommandResult<NodePresetSettings> {
    let project_root = project_root_from_state(state, None)?;
    save_node_preset_settings_impl(&project_root, settings)
}

pub fn get_node_preset_settings_impl(project_root: &Path) -> CommandResult<NodePresetSettings> {
    read_node_preset_settings(project_root)
}

pub fn save_node_preset_settings_impl(
    project_root: &Path,
    settings: NodePresetSettings,
) -> CommandResult<NodePresetSettings> {
    let _project_mutation = acquire_project_mutation_guard(project_root, "node_preset_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    write_node_preset_settings(project_root, &settings)?;
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
        let policy = git_stage_policy_from_config(&config.git);
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
            .reload_retrieval_runtime()?
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
    get_provider_config_impl(&project_root, state.secret_store.as_ref())
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
    let status = match provider_config_status_from_config(
        &project_root,
        config,
        state.secret_store.as_ref(),
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
    let expected = ConfigStore::new(&project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let mut candidate = expected.clone();
    apply_provider_settings_update(&mut candidate, update)?;
    let status = provider_config_status_from_config(
        &project_root,
        candidate.clone(),
        state.secret_store.as_ref(),
    )?;
    state.commit_retrieval_config(&expected, candidate)?;
    spawn_indexing_worker_for_state(state)?;
    Ok(status)
}

pub fn save_provider_section_settings(
    state: &AriadneAppState,
    settings: ProviderSectionSettings,
) -> CommandResult<ProviderConfigStatus> {
    let project_root = project_root_from_state(state, None)?;
    let _project_mutation =
        acquire_project_mutation_guard(&project_root, "provider_section_update")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    let provider_id = normalize_provider(&settings.provider.provider_id)?;
    let expected = ConfigStore::new(&project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let mut candidate = expected.clone();
    apply_provider_settings_update(&mut candidate, settings.provider)?;

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

    let status = match provider_config_status_from_config(
        &project_root,
        candidate.clone(),
        state.secret_store.as_ref(),
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

    if let Err(error) = state.commit_retrieval_config(&expected, candidate) {
        if let Some(previous) = previous_secret {
            restore_provider_secret(&credentials, &provider_id, previous).map_err(|rollback| {
                CommandError::internal(format!(
                    "provider section commit failed: {error}; key rollback failed: {rollback}"
                ))
            })?;
            state.reload_retrieval_runtime().map_err(|rollback| {
                CommandError::internal(format!(
                    "provider section commit failed: {error}; runtime rollback failed: {rollback}"
                ))
            })?;
        }
        return Err(error);
    }
    spawn_indexing_worker_for_state(state)?;
    Ok(status)
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
    clear_provider_defaults(&mut candidate, &provider);

    let credentials = ProjectCredentialScope::new(&project_root, state.secret_store.as_ref())
        .map_err(error_to_string)?;
    let previous_secret = credentials
        .get_provider_secret(&provider)
        .map_err(error_to_string)?;
    let mut status = provider_config_status_from_config(
        &project_root,
        candidate.clone(),
        state.secret_store.as_ref(),
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
    let project_root = project_root_from_state(state, None)?;
    let manifest = template_client(request, cancellation)?
        .download(&id)
        .map_err(error_to_string)?;
    cancellation.check().map_err(CommandError::from)?;
    let _project_mutation = acquire_project_mutation_guard(&project_root, "template_install")?;
    let _provider_references = acquire_provider_reference_graph_guard(&project_root)?;
    crate::frontend::install_workflow_template_manifest(
        manifest,
        project_root.join("workflows"),
        false,
    )
    .map_err(error_to_string)
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
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
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
    let roots = [
        project_root.join("planning"),
        project_root.join("documents"),
        project_root.join("workflows"),
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

/// 项目打开/切换后的产品恢复入口，确保向量链使用 state 中的可信凭据。
fn resume_indexing_worker_for_state(state: &AriadneAppState) -> CommandResult<()> {
    let project_root = state.project_root()?;
    let runtime = state.retrieval_runtime()?;
    let Some(worker_key) = register_indexing_worker(&project_root) else {
        return Ok(());
    };
    let documents = document_service(&project_root);
    let project_mutation = match acquire_project_mutation_guard(&project_root, "indexing_recovery")
    {
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
    spawn_registered_indexing_worker_with_runtime(project_root, worker_key, runtime);
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

pub fn list_workflow_graphs_impl(project_root: &Path) -> CommandResult<Vec<WorkflowSummary>> {
    validate_project_root(project_root)?;
    let workflows_root = absolute_path(&project_root.join("workflows"));
    reject_symlink_root(&workflows_root)?;
    if !workflows_root.exists() {
        return Ok(vec![WorkflowSummary {
            workflow_id: "default".to_owned(),
            name: "Default Workflow".to_owned(),
            path: "workflows/default.json".to_owned(),
            node_count: 0,
            edge_count: 0,
        }]);
    }

    let mut paths = workflow_json_paths(&workflows_root)?;
    paths.sort();
    let mut summaries = Vec::new();
    for path in paths {
        ensure_path_under_root(&workflows_root, &path).map_err(error_to_string)?;
        let content = std::fs::read_to_string(&path).map_err(error_to_string)?;
        let workflow = parse_workflow_file(&content)?;
        let workflow_id = workflow.id.as_str().to_owned();
        summaries.push(WorkflowSummary {
            workflow_id,
            name: workflow.name,
            path: relative_id(project_root, &path)?,
            node_count: workflow.nodes.len(),
            edge_count: workflow.edges.len(),
        });
    }
    if summaries.is_empty() {
        summaries.push(WorkflowSummary {
            workflow_id: "default".to_owned(),
            name: "Default Workflow".to_owned(),
            path: "workflows/default.json".to_owned(),
            node_count: 0,
            edge_count: 0,
        });
    }
    Ok(summaries)
}

pub fn save_workflow_graph_impl(
    project_root: &Path,
    graph_data: WorkflowGraphData,
) -> CommandResult<WorkflowGraphData> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "workflow_graph_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
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
    let needs_retrieval = workflow_requires_project_retrieval(project_root, &workflow)?;
    let retrieval_runtime = if needs_retrieval {
        Some(match retrieval_runtime {
            Some(runtime) => runtime,
            None => Arc::new(
                ProjectRetrievalRuntime::open(project_root, secrets).map_err(error_to_string)?,
            ),
        })
    } else {
        None
    };
    let mut runtime = WorkflowRuntime::new(&workflow, run_id).map_err(error_to_string)?;
    runtime.state.prepared_workflow = Some(workflow.clone());
    runtime.state.start_node_id = start_node_id.as_deref().map(NodeId::from);
    preflight_workflow_runtime_dependencies(
        project_root,
        &document_root,
        secrets,
        &workflow,
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
    })
}

fn preflight_workflow_runtime_dependencies(
    project_root: &Path,
    document_root: &Path,
    secrets: &dyn SecretStore,
    workflow: &WorkflowDefinition,
    retrieval_runtime: Option<&ProjectRetrievalRuntime>,
) -> CommandResult<()> {
    std::fs::create_dir_all(document_root.join("documents")).map_err(error_to_string)?;
    std::fs::create_dir_all(document_root.join("planning")).map_err(error_to_string)?;
    SqliteCostLedger::open(project_root).map_err(error_to_string)?;
    if workflow_requires_llm_provider(workflow) {
        llm_runtime(project_root, secrets)?;
    }
    if workflow_requires_project_retrieval(project_root, workflow)? {
        retrieval_runtime.ok_or_else(|| {
            CommandError::internal("search-capable workflow preflight is missing retrieval runtime")
        })?;
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
    let retrieval_runtime = prepared.retrieval_runtime.clone();
    let mut runtime = WorkflowRuntime::from_state(prepared.state);
    let status = execute_workflow_runtime(
        WorkflowExecutionContext {
            project_root,
            document_root: &prepared.document_root,
            secrets,
            retrieval_runtime,
        },
        &prepared.workflow,
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
    let mut runtime = WorkflowRuntime::from_state(state);
    let status = execute_workflow_runtime(
        WorkflowExecutionContext {
            project_root,
            document_root: &document_root,
            secrets,
            retrieval_runtime,
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
    provider: OpenAiCompatibleLlmProvider,
    ledger: Arc<SqliteCostLedger>,
) -> CommandResult<Vec<String>> {
    register_executor_adapters_for_project_with_search(
        external,
        project_root,
        provider,
        ledger,
        None,
        None,
    )
}

fn register_executor_adapters_for_project_with_search(
    external: &mut RoutedExternalNodeExecutor,
    project_root: &Path,
    provider: OpenAiCompatibleLlmProvider,
    ledger: Arc<SqliteCostLedger>,
    retrieval: Option<Arc<ProjectRetrievalRuntime>>,
    web_search_provider: Option<Arc<HttpWebSearchProvider>>,
) -> CommandResult<Vec<String>> {
    use crate::contracts::permissions::ExecutionPolicy;
    use crate::contracts::AutoModeState;
    use crate::skills::{
        NativeHttpSkillBackend, NativeWasmSkillBackend, SkillExecutionContext, SkillExecutor,
    };

    let _project_mutation =
        acquire_project_mutation_guard(project_root, "executor_adapter_discovery")?;
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let skills_dir = project_config.app.skills_dir.trim();
    // Empty skills_dir means adapters disabled — not an error.
    if skills_dir.is_empty() {
        return Ok(Vec::new());
    }
    let global = project_root.join(skills_dir);
    let project_skills = project_root.join("skills");
    let loader = SkillLoader::new()
        .with_global_root(&global)
        .with_project_root(&project_skills);
    let manifests = loader.load_manifests().map_err(error_to_string)?;
    let execution_policy = ExecutionPolicy {
        auto_mode: AutoModeState::default(),
        permissions: crate::contracts::permissions::PermissionPolicy::default(),
    };
    let auto_mode_config = project_config.auto_mode.clone();
    let max_tool_rounds = project_config.workflow.max_tool_rounds;
    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls);
    let project_search_enabled = tool_control_enabled(
        &tool_controls,
        "executor_adapter",
        EXECUTOR_ADAPTER_SEARCH_TOOL,
    );
    let web_search_enabled = project_config
        .permissions
        .policy
        .evaluate(&PermissionRequest::WebSearch)
        .allowed
        && tool_control_enabled(
            &tool_controls,
            "executor_adapter",
            EXECUTOR_ADAPTER_WEB_SEARCH_TOOL,
        );
    let mut registered = Vec::new();
    for loaded in manifests {
        let skill_id = loaded.manifest.skill_id.clone();
        let manifest = loaded.manifest.clone();
        let provider = provider.clone();
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
                        llm_provider: Some(&provider),
                        http_backend: Some(&http_backend),
                        wasm_backend: Some(&wasm_backend),
                    };
                    let executor = SkillExecutor::new(context);
                    let executor = match project_search.as_ref() {
                        Some(retrieval) => executor.with_project_search(
                            retrieval,
                            project_search_tool_definition(
                                EXECUTOR_ADAPTER_SEARCH_TOOL,
                                "检索当前项目文档与已确认知识，为 LLM ExecutorAdapter 补充项目上下文。",
                            ),
                            max_tool_rounds,
                        ),
                        None => executor,
                    };
                    let executor = match web_search.as_ref() {
                        Some(search_provider) => executor.with_web_search(
                            search_provider.as_ref(),
                            web_search_tool_definition(
                                EXECUTOR_ADAPTER_WEB_SEARCH_TOOL,
                                "搜索公开互联网，为 LLM ExecutorAdapter 补充外部资料。",
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
    } = context;
    runtime.set_cancellation(cancellation);
    let documents = document_service_with_artifacts(
        document_root,
        project_root.join(".runtime").join("artifacts"),
    );
    std::fs::create_dir_all(document_root.join("documents")).map_err(error_to_string)?;
    std::fs::create_dir_all(document_root.join("planning")).map_err(error_to_string)?;
    let ledger = Arc::new(SqliteCostLedger::open(project_root).map_err(error_to_string)?);
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls.clone());
    let permission_policy = project_config.permissions.policy.clone();
    let max_tool_rounds = project_config.workflow.max_tool_rounds;
    let retrieval_runtime = if workflow_requires_project_retrieval(project_root, workflow)? {
        Some(match retrieval_runtime {
            Some(runtime) => runtime,
            None => Arc::new(
                ProjectRetrievalRuntime::open(project_root, secrets).map_err(error_to_string)?,
            ),
        })
    } else {
        retrieval_runtime
    };
    let llm_runtime = if workflow_requires_llm_provider(workflow) {
        Some(llm_runtime(project_root, secrets)?)
    } else {
        None
    };
    let web_search_runtime = if workflow.nodes.iter().any(|node| {
        workflow_web_search_tool_enabled(&tool_controls, &permission_policy, &node.type_name)
    }) {
        Some(Arc::new(web_search_runtime(project_root, secrets)?))
    } else {
        None
    };
    let mut external = RoutedExternalNodeExecutor::new();
    if let Some(ref llm_runtime) = llm_runtime {
        let provider = llm_runtime.provider.clone();
        let default_provider_id = llm_runtime.config.provider_id.clone();
        let default_model_id = llm_runtime.config.model_id.clone();
        // 普通 LLM 语义节点走 execute_llm_node。summarizer 例外：它是四步总结
        // 生产链（故事段划分并概括 → 事件 → 章节 → 阶段），走专用 handler 落库建索引。
        for type_name in [
            "llm", "writer", "outliner", "designer", "planner", "detail", "critic", "prudent",
            "polisher",
        ] {
            let provider = provider.clone();
            let ledger = Arc::clone(&ledger);
            let default_provider_id = default_provider_id.clone();
            let default_model_id = default_model_id.clone();
            let project_search = workflow_search_tool_for_node(type_name)
                .filter(|(scope, tool, _)| tool_control_enabled(&tool_controls, scope, tool))
                .and_then(|(_, tool, description)| {
                    retrieval_runtime.as_ref().map(|retrieval| {
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
                        && tool_control_enabled(&tool_controls, scope, tool)
                })
                .and_then(|(_, tool, description)| {
                    web_search_runtime.as_ref().map(|provider| {
                        (
                            Arc::clone(provider),
                            web_search_tool_definition(tool, description),
                        )
                    })
                });
            let permission_policy = permission_policy.clone();
            external
                .register_handler_with_policy(
                    type_name,
                    crate::workflow::WorkflowOperationPolicy::at_most_once(),
                    Box::new(move |request| {
                        if project_search.is_none() && web_search.is_none() {
                            return execute_llm_node_with_defaults(
                                request,
                                &provider,
                                ledger.as_ref(),
                                Some(&default_provider_id),
                                Some(&default_model_id),
                            );
                        }
                        execute_llm_node_with_search_tools(
                            request,
                            &provider,
                            ledger.as_ref(),
                            WorkflowLlmSearchOptions {
                                default_provider_id: Some(&default_provider_id),
                                default_model_id: Some(&default_model_id),
                                project_search: project_search
                                    .as_ref()
                                    .map(|(retrieval, tool)| (retrieval.as_ref(), tool.clone())),
                                web_search: web_search.as_ref().map(|(search_provider, tool)| {
                                    (
                                        search_provider.as_ref() as &dyn SearchProvider,
                                        &permission_policy,
                                        tool.clone(),
                                    )
                                }),
                                max_tool_rounds,
                            },
                        )
                    }),
                )
                .map_err(error_to_string)?;
        }

        // Summarizer 专用节点：加载写作知识库、四步总结、落库、生成四层确认项。
        {
            let provider = provider.clone();
            let ledger = Arc::clone(&ledger);
            let summarizer_root = project_root.to_path_buf();
            let project_search = workflow_search_tool_for_node("summarizer")
                .filter(|(scope, tool, _)| tool_control_enabled(&tool_controls, scope, tool))
                .and_then(|(_, tool, description)| {
                    retrieval_runtime.as_ref().map(|retrieval| {
                        (
                            Arc::clone(retrieval),
                            project_search_tool_definition(tool, description),
                        )
                    })
                });
            let web_search = workflow_web_search_tool_for_node("summarizer")
                .filter(|(scope, tool, _)| {
                    permission_policy
                        .evaluate(&PermissionRequest::WebSearch)
                        .allowed
                        && tool_control_enabled(&tool_controls, scope, tool)
                })
                .and_then(|(_, tool, description)| {
                    web_search_runtime.as_ref().map(|provider| {
                        (
                            Arc::clone(provider),
                            web_search_tool_definition(tool, description),
                        )
                    })
                });
            let permission_policy = permission_policy.clone();
            external
                .register_handler_with_policy(
                    "summarizer",
                    crate::workflow::WorkflowOperationPolicy::replayable_receipt(),
                    Box::new(move |request| {
                        if project_search.is_none() && web_search.is_none() {
                            return execute_summarizer_node(
                                request,
                                &provider,
                                ledger.as_ref(),
                                &summarizer_root,
                            );
                        }
                        execute_summarizer_node_with_search_tools(
                            request,
                            &provider,
                            ledger.as_ref(),
                            &summarizer_root,
                            project_search
                                .as_ref()
                                .map(|(retrieval, tool)| (retrieval.as_ref(), tool.clone())),
                            web_search.as_ref().map(|(search_provider, tool)| {
                                (
                                    search_provider.as_ref() as &dyn SearchProvider,
                                    &permission_policy,
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
    for type_name in ["document", "document_read"] {
        let documents = documents.clone();
        let document_root = document_root.to_path_buf();
        external
            .register_handler(
                type_name,
                Box::new(move |request| {
                    execute_document_read_node_with_root(request, &documents, Some(&document_root))
                }),
            )
            .map_err(error_to_string)?;
    }

    // F11：生产组合根注册 ExecutorAdapter（失败 fail-loud，不得静默丢弃）。
    if let Some(llm_runtime) = llm_runtime.as_ref() {
        register_executor_adapters_for_project_with_search(
            &mut external,
            project_root,
            llm_runtime.provider.clone(),
            Arc::clone(&ledger),
            retrieval_runtime.clone(),
            web_search_runtime.clone(),
        )?;
    }
    if workflow.nodes.iter().any(|node| node.type_name == "search") {
        let retrieval = match retrieval_runtime.clone() {
            Some(retrieval) => retrieval,
            None => Arc::new(
                ProjectRetrievalRuntime::open(project_root, secrets).map_err(error_to_string)?,
            ),
        };
        let search_root = project_root.to_path_buf();
        external
            .register_handler_with_policy(
                "search",
                crate::workflow::WorkflowOperationPolicy::replayable_remote(),
                Box::new(move |request| {
                    // F2-b：工作流 Search 节点与 IPC 搜索共用新鲜度门禁。
                    execute_project_retrieval_node_for_project(&search_root, request, &retrieval)
                }),
            )
            .map_err(error_to_string)?;
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

fn workflow_requires_llm_provider(workflow: &WorkflowDefinition) -> bool {
    workflow.nodes.iter().any(|node| {
        is_llm_workflow_node_type(&node.type_name)
            || node.type_name.starts_with("executor_adapter:")
    })
}

fn is_llm_workflow_node_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "llm"
            | "writer"
            | "outliner"
            | "designer"
            | "planner"
            | "detail"
            | "critic"
            | "prudent"
            | "polisher"
            | "summarizer"
    )
}

fn workflow_search_tool_for_node(
    type_name: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    match type_name {
        "llm" => Some((
            "llm",
            GENERIC_LLM_SEARCH_TOOL,
            "检索当前项目文档与已确认知识，为回答补充可追溯的项目事实。",
        )),
        "outliner" => Some((
            "outliner",
            "outliner-search",
            "检索当前项目的正文、规划与已确认知识，为全局总纲补充项目事实。",
        )),
        "designer" => Some((
            "designer",
            "designer-search",
            "检索当前项目的正文、规划与已确认知识，为阶段设计补充项目事实。",
        )),
        "planner" => Some((
            "planner",
            "planner-search",
            "检索当前项目的正文、规划与已确认知识，为章节规划补充项目事实。",
        )),
        "detail" => Some((
            "detail",
            "detail-search",
            "检索当前项目的正文、规划与已确认知识，为细节生成补充上下文。",
        )),
        "writer" => Some((
            "writer",
            "writer-search",
            "检索当前项目的前文、规划与已确认知识，保持正文连续性和设定一致性。",
        )),
        "critic" => Some((
            "critic",
            "critic-search",
            "检索当前项目的正文、规划与已确认知识，为审稿判断提供依据。",
        )),
        "prudent" => Some((
            "prudent",
            "prudent-search",
            "检索当前项目的正文、规划与已确认知识，为审慎判断提供依据。",
        )),
        "polisher" => Some((
            "polisher",
            "polisher-search",
            "检索当前项目的前文、规划与已确认知识，为有限返修提供上下文。",
        )),
        "summarizer" => Some((
            "summarizer",
            SUMMARIZER_SEARCH_TOOL,
            "检索当前项目的正文与已确认知识，为四层总结补充跨章节上下文。",
        )),
        type_name if type_name.starts_with("executor_adapter:") => Some((
            "executor_adapter",
            EXECUTOR_ADAPTER_SEARCH_TOOL,
            "检索当前项目文档与已确认知识，为 LLM ExecutorAdapter 补充项目上下文。",
        )),
        _ => None,
    }
}

fn workflow_search_tool_enabled(
    controls: &BTreeMap<String, BTreeMap<String, bool>>,
    type_name: &str,
) -> bool {
    workflow_search_tool_for_node(type_name)
        .is_some_and(|(scope, tool, _)| tool_control_enabled(controls, scope, tool))
}

fn workflow_web_search_tool_for_node(
    type_name: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    match type_name {
        "llm" => Some((
            "llm",
            GENERIC_LLM_WEB_SEARCH_TOOL,
            "搜索公开互联网，为回答补充时效性资料；返回标题、URL 与摘要，不自动写入项目知识库。",
        )),
        "outliner" => Some((
            "outliner",
            "outliner-web-search",
            "搜索公开互联网，为全局规划进行现实资料考据。",
        )),
        "designer" => Some((
            "designer",
            "designer-web-search",
            "搜索公开互联网，为阶段设计进行现实资料考据。",
        )),
        "planner" => Some((
            "planner",
            "planner-web-search",
            "搜索公开互联网，为章节规划进行现实资料考据。",
        )),
        "detail" => Some((
            "detail",
            "detail-web-search",
            "搜索公开互联网，为环境、心理或设定细节补充现实资料。",
        )),
        "writer" => Some((
            "writer",
            "writer-web-search",
            "搜索公开互联网，核对当前写作位置涉及的现实情况。",
        )),
        "critic" => Some((
            "critic",
            "critic-web-search",
            "搜索公开互联网，为合理性与事实性审稿提供外部依据。",
        )),
        "prudent" => Some((
            "prudent",
            "prudent-web-search",
            "搜索公开互联网，复核意见者引用的现实事实。",
        )),
        "polisher" => Some((
            "polisher",
            "polisher-web-search",
            "搜索公开互联网，为有限返修核对现实资料。",
        )),
        "summarizer" => Some((
            "summarizer",
            SUMMARIZER_WEB_SEARCH_TOOL,
            "搜索公开互联网，为总结中涉及的现实事实提供外部核对。",
        )),
        type_name if type_name.starts_with("executor_adapter:") => Some((
            "executor_adapter",
            EXECUTOR_ADAPTER_WEB_SEARCH_TOOL,
            "搜索公开互联网，为 LLM ExecutorAdapter 补充外部资料。",
        )),
        _ => None,
    }
}

fn workflow_web_search_tool_enabled(
    controls: &BTreeMap<String, BTreeMap<String, bool>>,
    policy: &PermissionPolicy,
    type_name: &str,
) -> bool {
    policy.evaluate(&PermissionRequest::WebSearch).allowed
        && workflow_web_search_tool_for_node(type_name)
            .is_some_and(|(scope, tool, _)| tool_control_enabled(controls, scope, tool))
}

fn workflow_requires_project_retrieval(
    project_root: &Path,
    workflow: &WorkflowDefinition,
) -> CommandResult<bool> {
    if workflow.nodes.iter().any(|node| node.type_name == "search") {
        return Ok(true);
    }
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let controls = normalize_tool_controls(config.permissions.tool_controls);
    Ok(workflow
        .nodes
        .iter()
        .any(|node| workflow_search_tool_enabled(&controls, &node.type_name)))
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
    Ok(())
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
    let files = vec![
        (
            crate::config::AtomicCommitTarget::App,
            yaml_bytes(&config.app)?,
        ),
        (
            crate::config::AtomicCommitTarget::Providers,
            yaml_bytes(&config.providers)?,
        ),
        (
            crate::config::AtomicCommitTarget::Permissions,
            yaml_bytes(&config.permissions)?,
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
    );
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
    for setting in settings.confirmation_policies {
        if allowed.contains(&setting.confirmation_kind.as_str()) {
            let policy = approval_policy_from_ui(&policy_code_from_dual_policy(
                setting.normal_policy,
                setting.auto_mode_policy,
            ))?;
            let prompt = ensure_approval_prompt(
                &mut config.auto_mode.available_approval_prompts,
                &setting.confirmation_kind,
            );
            prompt.default_policy = policy;
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
    let files: Vec<(crate::config::AtomicCommitTarget, Vec<u8>)> = vec![
        (
            crate::config::AtomicCommitTarget::App,
            yaml_bytes(&config.app)?,
        ),
        (
            crate::config::AtomicCommitTarget::Providers,
            yaml_bytes(&config.providers)?,
        ),
        (
            crate::config::AtomicCommitTarget::Permissions,
            yaml_bytes(&config.permissions)?,
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
    validate_project_root(project_root)?;
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "permissions_settings_read")?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(PermissionsSettings {
        policy: config.permissions.policy,
        tool_controls: normalize_tool_controls(config.permissions.tool_controls),
    })
}

pub fn save_permissions_settings_impl(
    project_root: &Path,
    settings: PermissionsSettings,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let _project_mutation =
        acquire_project_mutation_guard(project_root, "permissions_settings_save")?;
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.permissions.policy = settings.policy;
    config.permissions.tool_controls = normalize_tool_controls(settings.tool_controls);
    config_store.save(&config).map_err(error_to_string)
}

fn normalize_tool_controls(
    mut controls: BTreeMap<String, BTreeMap<String, bool>>,
) -> BTreeMap<String, BTreeMap<String, bool>> {
    for (scope, defaults) in default_permission_tool_controls() {
        let scope_controls = controls.entry(scope).or_default();
        for (tool, enabled) in defaults {
            scope_controls.entry(tool).or_insert(enabled);
        }
    }
    controls
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
    let policy = git_stage_policy_from_config(&config.git);
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
    let policy = git_stage_policy_from_config(&config.git);
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

fn git_stage_policy_from_config(config: &GitConfig) -> GitStagePolicy {
    let mut ignored_paths = config.ignored_paths.clone();
    ignored_paths.extend([
        "runtime.db-wal".to_owned(),
        "runtime.db-shm".to_owned(),
        "costs.db-wal".to_owned(),
        "costs.db-shm".to_owned(),
    ]);
    if !config.track_documents {
        ignored_paths.push("documents".to_owned());
    }
    if !config.track_workflows {
        ignored_paths.push("workflows".to_owned());
    }
    if !config.track_skills {
        ignored_paths.push("skills".to_owned());
    }
    if !config.track_non_sensitive_config {
        ignored_paths.push(".config".to_owned());
    }
    GitStagePolicy::default().with_ignored_paths(ignored_paths)
}

pub fn get_provider_config_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
) -> CommandResult<ProviderConfigStatus> {
    validate_project_root(project_root)?;
    let _project_mutation = acquire_project_mutation_guard(project_root, "provider_settings_read")?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    provider_config_status_from_config(project_root, config, secrets)
}

fn provider_config_status_from_config(
    project_root: &Path,
    config: ProjectConfig,
    secrets: &dyn SecretStore,
) -> CommandResult<ProviderConfigStatus> {
    let credentials =
        ProjectCredentialScope::new(project_root, secrets).map_err(error_to_string)?;
    let configured_ids = config
        .providers
        .providers
        .iter()
        .map(|provider| provider.provider_id.as_str())
        .collect::<HashSet<_>>();
    let providers = provider_status_list(project_root, &config.providers.providers)
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
    if !node_preset_settings_path(project_root).exists() {
        return Ok(Vec::new());
    }
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
    if remaining_model_ids.is_empty() {
        return Ok(Vec::new());
    }
    let settings = read_node_preset_settings(project_root)?;
    let mut references = Vec::new();
    let mut add_reference = |owner_id: String, model_id: &str| {
        if removed_model_ids.contains(model_id) && !remaining_model_ids.contains(model_id) {
            references.push(ProviderRemovalReference {
                reference_type: "node_preset".to_owned(),
                owner_id,
                node_id: None,
                model_id: Some(model_id.to_owned()),
            });
        }
    };
    add_reference("default_model_id".to_owned(), &settings.default_model_id);
    for preset in settings.presets {
        add_reference(preset.node_type, &preset.model_id);
    }
    Ok(references)
}

fn provider_workflow_references(
    project_root: &Path,
    config: &ProjectConfig,
    provider_id: &str,
) -> CommandResult<Vec<ProviderRemovalReference>> {
    let mut references = Vec::new();
    let workflows_root = absolute_path(&project_root.join("workflows"));
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
    let provider_id = normalize_provider(&update.provider_id)?;
    let provider_config = ProviderConfig {
        provider_id: provider_id.clone(),
        provider_type: update.provider_type,
        display_name: non_empty_or("provider display_name", update.display_name)?,
        enabled: update.enabled,
        base_url: update.base_url,
        api_key: None,
        models: update.models,
    };
    provider_config.validate().map_err(error_to_string)?;
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
    if update.make_default_llm {
        config.providers.default_llm_provider_id = Some(provider_id.clone());
    }
    if update.make_default_embedding {
        config.providers.default_embedding_provider_id = Some(provider_id.clone());
    }
    if update.make_default_reranker {
        config.providers.default_reranker_provider_id = Some(provider_id.clone());
    }
    if update.make_default_search {
        config.providers.default_search_provider_id = Some(provider_id);
    }
    config.validate().map_err(error_to_string)
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
    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls);
    let project_search_enabled =
        tool_control_enabled(&tool_controls, "project_ai", PROJECT_AI_SEARCH_TOOL);
    let web_search_enabled = project_config
        .permissions
        .policy
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
                permission_policy: &project_config.permissions.policy,
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
    let workflows_root = absolute_path(&project_root.join("workflows"));
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
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let tool_controls = normalize_tool_controls(project_config.permissions.tool_controls);
    if !tool_control_enabled(&tool_controls, "project_ai", "project-ai-workflow-tools") {
        return Ok(Vec::new());
    }

    let workflows_root = absolute_path(&project_root.join("workflows"));
    reject_symlink_root(&workflows_root)?;
    if !workflows_root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = workflow_json_paths(&workflows_root)?;
    paths.sort();

    let mut tools = Vec::new();
    for path in paths {
        ensure_path_under_root(&workflows_root, &path).map_err(error_to_string)?;
        let content = std::fs::read_to_string(&path).map_err(error_to_string)?;
        let workflow: WorkflowDefinition =
            serde_json::from_str(&content).map_err(error_to_string)?;
        for start_node in workflow
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
                    sanitize_tool_name(workflow.id.as_str()),
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
                workflow_id: workflow.id.as_str().to_owned(),
                start_node_id: start_node.id.as_str().to_owned(),
                input_schema: start_node_tool_input_schema(start_node),
            });
        }
    }
    Ok(tools)
}

fn tool_control_enabled(
    controls: &BTreeMap<String, BTreeMap<String, bool>>,
    scope: &str,
    tool: &str,
) -> bool {
    controls
        .get(scope)
        .and_then(|scope_controls| scope_controls.get(tool).copied())
        .unwrap_or(false) // 未显式配置的工具默认禁用，需在 default_permission_tool_controls 中注册
}

fn project_ai_workflow_tool_enabled(
    controls: &BTreeMap<String, BTreeMap<String, bool>>,
    tool_name: &str,
) -> bool {
    controls
        .get("project_ai")
        .and_then(|scope_controls| scope_controls.get(tool_name).copied())
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
            PROJECT_AI_SEARCH_TOOL,
            "检索当前项目文档、规划与已确认知识。回答项目事实前应优先使用本工具核对。",
        ));
    }
    if web_search_enabled {
        tools.push(web_search_tool_definition(
            PROJECT_AI_WEB_SEARCH_TOOL,
            "搜索公开互联网，返回标题、URL 与摘要；结果不自动写入项目知识库。",
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

fn record_recent_project(
    app_state_root: &Path,
    name: Option<String>,
    project_root: &Path,
) -> CommandResult<Vec<RecentProjectEntry>> {
    let explicit_name = name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let name = match explicit_name {
        Some(name) => name,
        None => project_display_name(project_root)?,
    };
    recent_project_store(app_state_root)
        .record_opened(name, project_root)
        .map_err(error_to_string)
}

pub fn current_project_status(project_root: &Path) -> CommandResult<CurrentProjectStatus> {
    validate_initialized_project_root(project_root)?;
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

fn ensure_project_config(project_root: &Path) -> CommandResult<()> {
    ConfigStore::new(project_root)
        .load_or_create()
        .map(|_| ())
        .map_err(error_to_string)
}

fn persist_project_name(project_root: &Path, name: Option<&str>) -> CommandResult<()> {
    let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
        return Ok(());
    };
    let _provider_references = acquire_provider_reference_graph_guard(project_root)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.app.project_name = name.to_owned();
    config_store.save(&config).map_err(error_to_string)
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

fn read_node_preset_settings(project_root: &Path) -> CommandResult<NodePresetSettings> {
    validate_project_root(project_root)?;
    let path = node_preset_settings_path(project_root);
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map_err(error_to_string),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(NodePresetSettings::default())
        }
        Err(error) => Err(error_to_string(error)),
    }
}

fn write_node_preset_settings(
    project_root: &Path,
    settings: &NodePresetSettings,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let configured_model_ids = configured_model_ids_for_presets(project_root)?;
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
            &configured_model_ids,
            &preset.model_id,
            &format!("preset {}", preset.node_type),
        )?;
    }
    if settings.default_model_id.trim().is_empty() {
        return Err(CommandError::validation("default_model_id cannot be empty"));
    }
    ensure_preset_model_is_configured(
        &configured_model_ids,
        &settings.default_model_id,
        "default_model_id",
    )?;
    if settings.default_timeout_ms == 0 {
        return Err(CommandError::validation(
            "default_timeout_ms cannot be zero",
        ));
    }
    validate_money("default_budget_usd", settings.default_budget_usd)?;
    let path = node_preset_settings_path(project_root);
    let body = serde_json::to_string_pretty(settings).map_err(error_to_string)?;
    crate::config::store::atomic_write(&path, body.as_bytes()).map_err(error_to_string)
}

fn configured_model_ids_for_presets(project_root: &Path) -> CommandResult<HashSet<String>> {
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(config
        .providers
        .providers
        .into_iter()
        .flat_map(|provider| provider.models.into_iter())
        .map(|model| model.model_id)
        .collect())
}

fn ensure_preset_model_is_configured(
    configured_model_ids: &HashSet<String>,
    model_id: &str,
    field: &str,
) -> CommandResult<()> {
    if configured_model_ids.is_empty() || configured_model_ids.contains(model_id) {
        return Ok(());
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
    // 非推理节点（start/document/search/condition/loop/approval/export）无需 LLM 预算；
    // 推理节点（llm/writer/outliner 等）设 1.0 USD 单次上限，防止失控调用。
    let llm_budget = 1.0;
    let no_budget = 0.0;
    [
        ("start", "ui.workspace.start_node.title", no_budget),
        ("llm", "ui.node.llm", llm_budget),
        ("document", "ui.node.document", no_budget),
        ("search", "ui.node.search", no_budget),
        ("condition", "ui.node.condition", no_budget),
        ("loop", "ui.node.loop", no_budget),
        ("approval", "ui.node.approval", no_budget),
        ("export", "ui.node.export", no_budget),
        ("outliner", "agent.outliner", llm_budget),
        ("designer", "agent.designer", llm_budget),
        ("planner", "agent.planner", llm_budget),
        ("detail", "agent.detail", llm_budget),
        ("writer", "agent.writer", llm_budget),
        ("critic", "agent.critic", llm_budget),
        ("prudent", "agent.prudent", llm_budget),
        ("polisher", "agent.polisher", llm_budget),
        ("summarizer", "agent.summarizer", llm_budget),
    ]
    .into_iter()
    .map(|(node_type, display_name_key, budget_usd)| NodeTypePreset {
        node_type: node_type.to_owned(),
        display_name_key: display_name_key.to_owned(),
        model_id: default_node_preset_model_id(),
        timeout_ms: default_node_preset_timeout_ms(),
        budget_usd,
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
) -> Vec<ConfirmationPolicySetting> {
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
            }
        });
    }
    // 先结束对 map 的逐项可变借用，再消费剩余扩展项。
    let mut ordered = confirmation_policy_keys()
        .into_iter()
        .filter_map(|k| map.remove(k))
        .collect::<Vec<_>>();
    ordered.extend(map.into_values());
    ordered
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
        let loaded_retrieval_runtime = scheduled_workflow_retrieval_runtime(
            project_root,
            secrets.as_ref(),
            retrieval_runtime,
            prepared_workflow,
        )?;
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
    workflow: &WorkflowDefinition,
) -> CommandResult<Option<Arc<ProjectRetrievalRuntime>>> {
    if !workflow.nodes.iter().any(|node| node.type_name == "search") {
        return Ok(None);
    }
    let mut runtime_slot = retrieval_runtime
        .lock()
        .map_err(|_| CommandError::internal("retrieval runtime lock poisoned"))?;
    if let Some(runtime) = runtime_slot
        .as_ref()
        .filter(|runtime| runtime.project_root() == project_root)
    {
        return Ok(Some(Arc::clone(runtime)));
    }
    let runtime =
        Arc::new(ProjectRetrievalRuntime::open(project_root, secrets).map_err(error_to_string)?);
    let previous = runtime_slot.replace(Arc::clone(&runtime));
    drop(runtime_slot);
    drop(previous);
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
    // 用户配置的全局预算写入执行侧日限额，供 LlmService::evaluate_budget 使用。
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
        .or_else(|| provider.models.first())
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

fn document_service(project_root: &Path) -> FileDocumentService {
    document_service_with_artifacts(
        project_root,
        project_root.join(".runtime").join("artifacts"),
    )
}

fn document_service_with_artifacts(
    document_root: &Path,
    artifact_root: PathBuf,
) -> FileDocumentService {
    let mut permissions = project_document_permission(document_root);
    permissions.readable_file_roots.push(artifact_root.clone());
    permissions.writable_file_roots.push(artifact_root.clone());
    FileDocumentService::new(permissions, artifact_root)
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
    let workflows_root = absolute_path(&project_root.join("workflows"));
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
    let workflows_root = absolute_path(&project_root.join("workflows"));
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
    let config_dir = project_root.join(".config");
    if !config_dir.is_dir() {
        return Err(CommandError::validation(format!(
            "project root is not initialized: {}",
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
