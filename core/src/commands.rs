use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(not(feature = "system-keychain"))]
use crate::config::LocalFileSecretStore;
#[cfg(feature = "system-keychain")]
use crate::config::SystemKeychainSecretStore;
use crate::config::{
    default_permission_tool_controls, AppConfig, ApprovalPromptConfig, ConfigStore, GitConfig,
    ModelConfig, ProviderConfig, RagConfig, SecretRef, SecretStore, SecretValue, WorkflowConfig,
};
use crate::contracts::{
    ensure_path_under_root, ApprovalPolicy, CoreResult, Edge, EdgeId, NodeId, NodeInstance,
    PermissionPolicy, PortEndpoint, ProviderCapability, ProviderType, RunId, WorkflowDefinition,
    WorkflowEdgeKind, WorkflowId,
};
use crate::costs::{CostLedger, CostQuery, SqliteCostLedger};
use crate::diagnostics::{BackendDiagnosticsReport, DiagnosticItem, DiagnosticStatus};
use crate::documents::{
    ChapterDocumentIndex, DocumentContent, DocumentReadRequest, DocumentRepository,
    DocumentWriteReport, DocumentWriteRequest, FileDocumentService,
};
use crate::frontend::{
    apply_node_detail_patch as apply_node_detail_patch_to_workflow, build_works_tree,
    confirmation_state_from_runtime, export_chapters_combined,
    export_workflow_selection as export_workflow_selection_from_workflow, import_chapter_document,
    initialize_project, pack_workflow_selection as pack_workflow_selection_in_workflow,
    project_document_permission, upsert_canvas_annotation as upsert_canvas_annotation_in_workflow,
    CanvasAnnotation, ChapterExportFormat, ChapterImportRequest, CombinedExportReport,
    ConfirmationLogEntry, FileConfirmationLogStore, NodeDetailPatch, ProjectInitReport,
    ProjectMemoryStore, ProjectReference, ProjectReferenceResolver, ProjectRegistryStore,
    QuickEditResult, QuickEditService, RecentProjectEntry, SidebarBadgeCounts, TemplateDetail,
    TemplateInstallReport, TemplateRepositoryClient, TemplateSummary, UiPreferences,
    UiPreferencesStore, UiRunLogEntry, UiRunLogFilter, UiRunLogKind, UiRunLogLevel, UiRunLogStore,
    WorksTreeNode,
};
use crate::git::{
    ArchivePoint, BranchGraphNode, GitCommitSummary, GitService, GitStagePolicy, RestoreReport,
};
use crate::llm::{LlmRunRequest, LlmService, LlmServiceConfig};
use crate::providers::{
    ContentPart, LlmMessage, LlmRole, OpenAiCompatibleLlmProvider, ProviderProtocol, ToolDefinition,
};
use crate::workflow::{
    execute_document_read_node_with_root, execute_llm_node, execute_summarizer_node,
    BuiltinWorkflowNodeExecutor, DocumentWorkflowExportSink, RoutedExternalNodeExecutor,
    RuntimeConfirmation, RuntimeConfirmationState, SqliteWorkflowRuntimeStore, WorkflowRuntime,
    WorkflowRuntimeEvent, WorkflowRuntimeStore,
};

pub const WORKFLOW_STATUS_UPDATE_EVENT: &str = "workflow_status_update";
pub const RUN_LOG_APPENDED_EVENT: &str = "run_log_appended";
pub const BUDGET_UPDATED_EVENT: &str = "budget_updated";
pub const CONFIRMATION_CREATED_EVENT: &str = "confirmation_created";
pub const DIAGNOSTICS_UPDATED_EVENT: &str = "diagnostics_updated";
pub const TOAST_CREATED_EVENT: &str = "toast_created";

const DEFAULT_PROJECT_ENV: &str = "ARIADNE_PROJECT_ROOT";
const APP_STATE_ENV: &str = "ARIADNE_APP_STATE_ROOT";
const APP_STATE_DIR: &str = ".ariadne-app";
const RECENT_PROJECTS_FILE: &str = "recent_projects.json";
const BUDGET_CONFIG_FILE: &str = "budget.json";
const CHAPTER_INDEX_FILE: &str = "chapter_index.json";
const UI_NODE_PRESETS_FILE: &str = "ui_node_presets.json";
const TEMPLATE_REPOSITORY_SETTINGS_FILE: &str = "template_repository_settings.json";
const CONFIRMATION_POLICY_SETTINGS_FILE: &str = "confirmation_policy_settings.json";
const DEFAULT_TEMPLATE_REPOSITORY_URL: &str = "";
const PROVIDER_MODEL_FETCH_TIMEOUT_SECS: u64 = 30;

/// 桌面前端共享状态。project_root 可由 Avalonia IPC 显式设置，也可用环境变量/当前目录兜底。
#[derive(Clone)]
pub struct AriadneAppState {
    project_root: Arc<Mutex<PathBuf>>,
    app_state_root: PathBuf,
    secret_store: Arc<dyn SecretStore>,
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
            .map_err(|_| "project root lock poisoned".to_owned())
    }

    pub fn app_state_root(&self) -> &Path {
        &self.app_state_root
    }

    pub fn set_project_root(&self, project_root: impl Into<PathBuf>) -> CommandResult<()> {
        let project_root = project_root.into();
        validate_initialized_project_root(&project_root)?;
        let mut locked = self
            .project_root
            .lock()
            .map_err(|_| "project root lock poisoned".to_owned())?;
        *locked = project_root;
        Ok(())
    }
}

pub type CommandResult<T> = Result<T, String>;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfirmationPolicySetting {
    pub confirmation_kind: String,
    #[serde(default)]
    pub normal_policy: ConfirmationNormalPolicy,
    #[serde(default)]
    pub auto_mode_policy: ConfirmationAutoModePolicy,
}

impl<'de> Deserialize<'de> for ConfirmationPolicySetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawConfirmationPolicySetting {
            confirmation_kind: String,
            #[serde(default)]
            normal_policy: ConfirmationNormalPolicy,
            #[serde(default)]
            auto_mode_policy: ConfirmationAutoModePolicy,
            #[serde(default, rename = "policy")]
            policy_code: String,
        }

        let raw = RawConfirmationPolicySetting::deserialize(deserializer)?;
        let (normal_policy, auto_mode_policy) = if raw.policy_code.trim().is_empty() {
            (raw.normal_policy, raw.auto_mode_policy)
        } else {
            policies_from_policy_code(&raw.policy_code)
        };

        Ok(Self {
            confirmation_kind: raw.confirmation_kind,
            normal_policy,
            auto_mode_policy,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationNormalPolicy {
    ManualReview,
    AllowByDefault,
}

impl Default for ConfirmationNormalPolicy {
    fn default() -> Self {
        Self::ManualReview
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationAutoModePolicy {
    AllowByDefault,
    AutoApproval,
}

impl Default for ConfirmationAutoModePolicy {
    fn default() -> Self {
        Self::AllowByDefault
    }
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
    #[serde(default)]
    pub providers: Vec<ProviderKeyStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderKeyStatus {
    pub provider: String,
    pub display_name: String,
    pub provider_type: ProviderType,
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
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
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
    pub append_memory: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectAiChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAiChatMessage {
    pub role: ProjectAiChatRole,
    pub content: String,
}

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectWorkflowTool {
    tool_name: String,
    display_name: String,
    workflow_id: String,
    start_node_id: String,
    input_schema: Value,
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
    let report = initialize_project(&project_root).map_err(error_to_string)?;
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
    state.set_project_root(project_root.clone())?;
    record_recent_project(state.app_state_root(), name, &project_root)?;
    current_project_status(&project_root)
}

pub fn get_current_project(state: &AriadneAppState) -> CommandResult<CurrentProjectStatus> {
    current_project_status(&project_root_from_state(&state, None)?)
}

pub fn get_app_status(state: &AriadneAppState) -> CommandResult<AppStatus> {
    let project_root = project_root_from_state(&state, None)?;
    Ok(AppStatus {
        current_project: current_project_status(&project_root)?,
        badges: get_sidebar_badges_impl(&project_root)?,
        preferences: UiPreferencesStore::default_for_project(&project_root)
            .read()
            .map_err(error_to_string)?,
    })
}

pub fn get_sidebar_badges(state: &AriadneAppState) -> CommandResult<SidebarBadgeCounts> {
    let project_root = project_root_from_state(&state, None)?;
    get_sidebar_badges_impl(&project_root)
}

pub fn set_project_root(state: &AriadneAppState, project_root: String) -> CommandResult<()> {
    let project_root = PathBuf::from(project_root);
    validate_initialized_project_root(&project_root)?;
    state.set_project_root(project_root)
}

pub fn get_works_tree(state: &AriadneAppState) -> CommandResult<WorksTreeNode> {
    let project_root = project_root_from_state(&state, None)?;
    let index = load_chapter_index(&project_root)?;
    build_works_tree(&index, project_root.join("planning")).map_err(error_to_string)
}

pub fn get_document_tree(
    state: &AriadneAppState,
    project_id: Option<String>,
) -> CommandResult<DocumentTreeNode> {
    let project_root = project_root_from_state(&state, project_id)?;
    get_document_tree_impl(&project_root)
}

pub fn get_document_content(
    state: &AriadneAppState,
    document_id: Option<String>,
    path: Option<String>,
) -> CommandResult<String> {
    let project_root = project_root_from_state(&state, None)?;
    get_document_content_impl(&project_root, document_id, path)
}

pub fn get_document_content_details(
    state: &AriadneAppState,
    document_id: Option<String>,
    path: Option<String>,
) -> CommandResult<DocumentContent> {
    let project_root = project_root_from_state(&state, None)?;
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
    let project_root = project_root_from_state(&state, None)?;
    save_document_content_report_impl(&project_root, document_id, content, base_version)
}

pub fn import_chapter(
    state: &AriadneAppState,
    request: ChapterImportRequest,
) -> CommandResult<ChapterDocumentIndex> {
    let project_root = project_root_from_state(&state, None)?;
    let documents = document_service(&project_root);
    let report = import_chapter_document(&documents, request).map_err(error_to_string)?;
    let mut index = load_chapter_index(&project_root)?;
    index
        .entries
        .retain(|entry| entry.chapter_id != report.entry.chapter_id);
    index.entries.push(report.entry);
    save_chapter_index(&project_root, &index)?;
    Ok(index)
}

pub fn export_chapters(
    state: &AriadneAppState,
    selected_chapter_ids: Vec<String>,
    artifact_id: String,
    format: Option<ChapterExportFormat>,
) -> CommandResult<CombinedExportReport> {
    let project_root = project_root_from_state(&state, None)?;
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
    let project_root = project_root_from_state(&state, None)?;
    load_workflow_graph_impl(&project_root, workflow_id)
}

pub fn validate_workflow_graph(graph_data: WorkflowGraphData) -> CommandResult<()> {
    graph_to_workflow(graph_data)?
        .validate_topology()
        .map_err(error_to_string)
}

pub fn save_workflow_graph(
    state: &AriadneAppState,
    graph_data: WorkflowGraphData,
) -> CommandResult<()> {
    let project_root = project_root_from_state(&state, None)?;
    save_workflow_graph_impl(&project_root, graph_data)
}

pub fn apply_node_detail_patch(
    state: &AriadneAppState,
    workflow_id: String,
    patch: NodeDetailPatch,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(&state, None)?;
    let mut workflow = load_workflow_definition(&project_root, Some(workflow_id))?;
    apply_node_detail_patch_to_workflow(&mut workflow, patch).map_err(error_to_string)?;
    let graph = workflow_to_graph(workflow.clone());
    save_workflow_graph_impl(&project_root, graph.clone())?;
    Ok(graph)
}

pub fn upsert_canvas_annotation(
    state: &AriadneAppState,
    workflow_id: String,
    annotation: CanvasAnnotation,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(&state, None)?;
    let mut workflow = load_workflow_definition(&project_root, Some(workflow_id))?;
    upsert_canvas_annotation_in_workflow(&mut workflow, annotation).map_err(error_to_string)?;
    let graph = workflow_to_graph(workflow.clone());
    save_workflow_graph_impl(&project_root, graph.clone())?;
    Ok(graph)
}

pub fn set_node_breakpoint(
    state: &AriadneAppState,
    workflow_id: String,
    node_id: String,
    enabled: bool,
) -> CommandResult<WorkflowGraphData> {
    let project_root = project_root_from_state(&state, None)?;
    let mut workflow = load_workflow_definition(&project_root, Some(workflow_id))?;
    crate::frontend::set_node_breakpoint(&mut workflow, &node_id, enabled)
        .map_err(error_to_string)?;
    let graph = workflow_to_graph(workflow.clone());
    save_workflow_graph_impl(&project_root, graph.clone())?;
    Ok(graph)
}

pub fn export_workflow_selection(
    state: &AriadneAppState,
    workflow_id: String,
    selected_node_ids: Vec<String>,
) -> CommandResult<crate::frontend::WorkflowSelectionExport> {
    let project_root = project_root_from_state(&state, None)?;
    let workflow = load_workflow_definition(&project_root, Some(workflow_id))?;
    export_workflow_selection_from_workflow(&workflow, &selected_node_ids).map_err(error_to_string)
}

pub fn pack_workflow_selection_impl(
    project_root: &Path,
    workflow_id: String,
    selected_node_ids: Vec<String>,
    subworkflow_node_id: Option<String>,
    title: Option<String>,
) -> CommandResult<crate::frontend::WorkflowPackReport> {
    let mut workflow = load_workflow_definition(project_root, Some(workflow_id))?;
    let report = pack_workflow_selection_in_workflow(
        &mut workflow,
        &selected_node_ids,
        subworkflow_node_id,
        title,
    )
    .map_err(error_to_string)?;
    save_workflow_graph_impl(project_root, workflow_to_graph(report.workflow.clone()))?;
    Ok(report)
}

pub fn pack_workflow_selection(
    state: &AriadneAppState,
    workflow_id: String,
    selected_node_ids: Vec<String>,
    subworkflow_node_id: Option<String>,
    title: Option<String>,
) -> CommandResult<crate::frontend::WorkflowPackReport> {
    let project_root = project_root_from_state(&state, None)?;
    pack_workflow_selection_impl(
        &project_root,
        workflow_id,
        selected_node_ids,
        subworkflow_node_id,
        title,
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
    start_workflow_request(
        &project_root,
        Arc::clone(&state.secret_store),
        RunWorkflowRequest {
            workflow_id,
            start_node_id,
            initial_inputs: BTreeMap::new(),
        },
    )
}

fn start_workflow_request(
    project_root: &Path,
    secrets: Arc<dyn SecretStore>,
    request: RunWorkflowRequest,
) -> CommandResult<WorkflowRunStarted> {
    validate_existing_project_root(project_root)?;
    let run_id = new_run_id()?;
    let run_id_text = run_id.as_str().to_owned();
    let worker_workflow_id = request.workflow_id.clone();
    let worker_root = project_root.to_path_buf();
    let worker_run_id = run_id.clone();
    let worker_run_id_text = run_id_text.clone();
    std::thread::Builder::new()
        .name(format!("ariadne-workflow-{}", run_id.as_str()))
        .spawn(move || {
            if let Err(error) = run_workflow_impl_with_run_id(
                &worker_root,
                secrets.as_ref(),
                request,
                worker_run_id,
            ) {
                record_workflow_worker_error(
                    &worker_root,
                    &worker_workflow_id,
                    &worker_run_id_text,
                    "workflow worker failed",
                    &error,
                );
                eprintln!("[ariadne] workflow worker failed: {error}");
            }
        })
        .map_err(error_to_string)?;
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
        &project_root_from_state(&state, None)?,
        workflow_id,
        run_id,
        |runtime| {
            runtime.request_pause(reason.unwrap_or_else(|| "paused by user".to_owned()));
            Ok(())
        },
    )
}

pub fn stop_workflow(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
    reason: Option<String>,
) -> CommandResult<WorkflowActionResult> {
    update_workflow_run_control(
        &project_root_from_state(&state, None)?,
        workflow_id,
        run_id,
        |runtime| {
            runtime.request_stop(reason.unwrap_or_else(|| "stopped by user".to_owned()));
            Ok(())
        },
    )
}

pub fn resume_workflow(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
) -> CommandResult<WorkflowActionResult> {
    let project_root = project_root_from_state(&state, None)?;
    let result = update_workflow_run_control(&project_root, workflow_id, run_id, |runtime| {
        runtime.resume()
    })?;
    spawn_continue_if_queued(&project_root, Arc::clone(&state.secret_store), &result)?;
    Ok(result)
}

/// 路径 B：把交流后同意的 Prudent 输出改写进关联节点并置为通过，解除暂停继续运行。
pub fn override_confirmation_output(
    state: &AriadneAppState,
    request: OverrideConfirmationOutputRequest,
) -> CommandResult<WorkflowActionResult> {
    let project_root = project_root_from_state(&state, None)?;
    let result = update_workflow_run_control(
        &project_root,
        request.workflow_id,
        request.run_id,
        |runtime| {
            runtime.override_confirmation_output(&request.confirmation_id, request.new_outputs)
        },
    )?;
    spawn_continue_if_queued(&project_root, Arc::clone(&state.secret_store), &result)?;
    Ok(result)
}

/// 路径 A：注入外部正文到指定节点并从其控制下游重跑，解除暂停继续运行。
pub fn resume_from_node(
    state: &AriadneAppState,
    request: ResumeFromNodeRequest,
) -> CommandResult<WorkflowActionResult> {
    let project_root = project_root_from_state(&state, None)?;
    let workflow = load_workflow_definition(&project_root, Some(request.workflow_id.clone()))
        .map_err(error_to_string)?;
    let result = update_workflow_run_control(
        &project_root,
        request.workflow_id,
        request.run_id,
        |runtime| {
            runtime.resume_from_node(
                &workflow,
                &NodeId::from(request.node_id.clone()),
                request.injected_outputs,
            )
        },
    )?;
    spawn_continue_if_queued(&project_root, Arc::clone(&state.secret_store), &result)?;
    Ok(result)
}

pub fn get_workflow_run_state(
    state: &AriadneAppState,
    workflow_id: String,
    run_id: String,
) -> CommandResult<Option<crate::workflow::WorkflowRunState>> {
    let project_root = project_root_from_state(&state, None)?;
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
    let project_root = project_root_from_state(&state, None)?;
    get_workflow_events_impl(&project_root, workflow_id, run_id, after_sequence, limit)
}

pub fn get_budget_status(state: &AriadneAppState) -> CommandResult<BudgetStatus> {
    let project_root = project_root_from_state(&state, None)?;
    get_budget_status_impl(&project_root)
}

pub fn get_app_settings(state: &AriadneAppState) -> CommandResult<AppSettings> {
    let project_root = project_root_from_state(&state, None)?;
    get_app_settings_impl(&project_root)
}

pub fn save_app_settings(
    state: &AriadneAppState,
    settings: AppSettings,
) -> CommandResult<AppSettings> {
    let project_root = project_root_from_state(&state, None)?;
    save_app_settings_impl(&project_root, settings)?;
    get_app_settings_impl(&project_root)
}

pub fn get_rag_settings(state: &AriadneAppState) -> CommandResult<RagSettings> {
    let project_root = project_root_from_state(&state, None)?;
    get_rag_settings_impl(&project_root)
}

pub fn save_rag_settings(
    state: &AriadneAppState,
    settings: RagSettings,
) -> CommandResult<RagSettings> {
    let project_root = project_root_from_state(&state, None)?;
    save_rag_settings_impl(&project_root, settings)?;
    get_rag_settings_impl(&project_root)
}

pub fn get_workflow_settings(state: &AriadneAppState) -> CommandResult<WorkflowSettings> {
    let project_root = project_root_from_state(&state, None)?;
    get_workflow_settings_impl(&project_root)
}

pub fn save_workflow_settings(
    state: &AriadneAppState,
    settings: WorkflowSettings,
) -> CommandResult<WorkflowSettings> {
    let project_root = project_root_from_state(&state, None)?;
    save_workflow_settings_impl(&project_root, settings)?;
    get_workflow_settings_impl(&project_root)
}

pub fn get_git_settings(state: &AriadneAppState) -> CommandResult<GitSettings> {
    let project_root = project_root_from_state(&state, None)?;
    get_git_settings_impl(&project_root)
}

pub fn save_git_settings(
    state: &AriadneAppState,
    settings: GitSettings,
) -> CommandResult<GitSettings> {
    let project_root = project_root_from_state(&state, None)?;
    save_git_settings_impl(&project_root, settings)?;
    get_git_settings_impl(&project_root)
}

pub fn get_template_repository_settings(
    state: &AriadneAppState,
) -> CommandResult<TemplateRepositorySettings> {
    let project_root = project_root_from_state(&state, None)?;
    get_template_repository_settings_impl(&project_root)
}

pub fn save_template_repository_settings(
    state: &AriadneAppState,
    settings: TemplateRepositorySettings,
) -> CommandResult<TemplateRepositorySettings> {
    let project_root = project_root_from_state(&state, None)?;
    save_template_repository_settings_impl(&project_root, &settings)?;
    get_template_repository_settings_impl(&project_root)
}

pub fn update_budget_config(
    state: &AriadneAppState,
    budget_usd: f64,
    preauthorized_usd: f64,
) -> CommandResult<()> {
    let project_root = project_root_from_state(&state, None)?;
    update_budget_config_impl(&project_root, budget_usd, preauthorized_usd)
}

pub fn set_auto_mode(state: &AriadneAppState, enabled: bool) -> CommandResult<()> {
    let project_root = project_root_from_state(&state, None)?;
    set_auto_mode_impl(&project_root, enabled)
}

pub fn get_automation_settings(state: &AriadneAppState) -> CommandResult<AutomationSettings> {
    let project_root = project_root_from_state(&state, None)?;
    get_automation_settings_impl(&project_root)
}

pub fn save_automation_settings(
    state: &AriadneAppState,
    settings: AutomationSettings,
) -> CommandResult<AutomationSettings> {
    let project_root = project_root_from_state(&state, None)?;
    save_automation_settings_impl(&project_root, settings)?;
    get_automation_settings_impl(&project_root)
}

pub fn get_permissions_settings(state: &AriadneAppState) -> CommandResult<PermissionsSettings> {
    let project_root = project_root_from_state(&state, None)?;
    get_permissions_settings_impl(&project_root)
}

pub fn save_permissions_settings(
    state: &AriadneAppState,
    settings: PermissionsSettings,
) -> CommandResult<PermissionsSettings> {
    let project_root = project_root_from_state(&state, None)?;
    save_permissions_settings_impl(&project_root, settings)?;
    get_permissions_settings_impl(&project_root)
}

pub fn get_node_preset_settings(state: &AriadneAppState) -> CommandResult<NodePresetSettings> {
    let project_root = project_root_from_state(&state, None)?;
    get_node_preset_settings_impl(&project_root)
}

pub fn save_node_preset_settings(
    state: &AriadneAppState,
    settings: NodePresetSettings,
) -> CommandResult<NodePresetSettings> {
    let project_root = project_root_from_state(&state, None)?;
    save_node_preset_settings_impl(&project_root, settings)
}

pub fn get_node_preset_settings_impl(project_root: &Path) -> CommandResult<NodePresetSettings> {
    read_node_preset_settings(project_root)
}

pub fn save_node_preset_settings_impl(
    project_root: &Path,
    settings: NodePresetSettings,
) -> CommandResult<NodePresetSettings> {
    write_node_preset_settings(project_root, &settings)?;
    read_node_preset_settings(project_root)
}

pub fn fetch_provider_models(
    state: &AriadneAppState,
    provider_id: Option<String>,
) -> CommandResult<ProviderModelsResult> {
    let project_root = project_root_from_state(&state, None)?;
    fetch_provider_models_with_secrets_impl(&project_root, state.secret_store.as_ref(), provider_id)
}

pub fn fetch_provider_models_with_secrets_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    provider_id: Option<String>,
) -> CommandResult<ProviderModelsResult> {
    validate_project_root(project_root)?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let selected = select_provider_for_model_fetch(&config.providers, provider_id)?.clone();
    let protocol = match ProviderProtocol::from_provider_type(&selected.provider_type) {
        Ok(protocol) => protocol,
        Err(_) => return configured_provider_models_result(&selected),
    };
    let api_key = provider_api_key(project_root, secrets, &selected)?;
    let fetched = fetch_remote_provider_models(&selected, protocol, api_key)?;
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

fn select_provider_for_model_fetch<'a>(
    providers: &'a crate::config::ProvidersConfig,
    provider_id: Option<String>,
) -> CommandResult<&'a ProviderConfig> {
    let requested = provider_id.as_deref().map(normalize_provider).transpose()?;
    if let Some(id) = requested {
        return providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == id)
            .ok_or_else(|| format!("provider is not configured: {id}"));
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
        .ok_or_else(|| "no provider configured".to_owned())
}

fn provider_api_key(
    project_root: &Path,
    secrets: &dyn SecretStore,
    provider: &ProviderConfig,
) -> CommandResult<Option<String>> {
    let key_id = provider
        .api_key
        .as_ref()
        .map(|secret| secret.key_id.clone())
        .unwrap_or_else(|| provider_key_id(project_root, &provider.provider_id));
    secrets
        .get_secret(&key_id)
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
) -> CommandResult<Vec<ModelConfig>> {
    if provider_requires_api_key(&provider.provider_type) && api_key.is_none() {
        return Err(format!(
            "provider {} requires an API key before fetching models",
            provider.provider_id
        ));
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
    let response = request.send().map_err(|error| {
        format!(
            "failed to fetch models from provider {}: {error}",
            provider.provider_id
        )
    })?;
    let status = response.status();
    let text = response.text().map_err(|error| {
        format!(
            "failed to read model list from provider {}: {error}",
            provider.provider_id
        )
    })?;
    if !status.is_success() {
        return Err(format!(
            "provider {} model list request failed with HTTP {}: {}",
            provider.provider_id,
            status.as_u16(),
            truncate_provider_error(&text)
        ));
    }
    let raw: Value = serde_json::from_str(&text).map_err(|error| {
        format!(
            "provider {} returned invalid model list JSON: {error}",
            provider.provider_id
        )
    })?;
    parse_remote_provider_models(protocol, &raw)
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
    let data = raw
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "provider model list response must contain data[]".to_owned())?;
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
    let data = raw
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "gemini model list response must contain models[]".to_owned())?;
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
        Err("provider returned no usable models".to_owned())
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
    let project_root = project_root_from_state(&state, None)?;
    FileConfirmationLogStore::default_for_project(&project_root)
        .read_all()
        .map_err(error_to_string)
}

pub fn get_confirmation(
    state: &AriadneAppState,
    confirmation_id: String,
) -> CommandResult<crate::frontend::ConfirmationReference> {
    let project_root = project_root_from_state(&state, None)?;
    FileConfirmationLogStore::default_for_project(&project_root)
        .resolve_reference(&confirmation_id)
        .map_err(error_to_string)
}

pub fn resolve_confirmation(
    state: &AriadneAppState,
    request: ResolveConfirmationRequest,
) -> CommandResult<ResolveConfirmationResult> {
    let project_root = project_root_from_state(&state, None)?;
    let should_continue = request.decision == ConfirmationDecision::Approve;
    let result = resolve_confirmation_impl(&project_root, request)?;
    if should_continue {
        spawn_continue_if_queued(
            &project_root,
            Arc::clone(&state.secret_store),
            &result.workflow,
        )?;
    }
    Ok(result)
}

pub fn get_git_history(state: &AriadneAppState) -> CommandResult<Vec<GitCommitSummary>> {
    let project_root = project_root_from_state(&state, None)?;
    get_git_history_impl(&project_root)
}

pub fn get_git_branch_graph(
    state: &AriadneAppState,
    limit: Option<usize>,
) -> CommandResult<Vec<BranchGraphNode>> {
    let project_root = project_root_from_state(&state, None)?;
    GitService::new(project_root)
        .branch_graph(limit.unwrap_or(200))
        .map_err(error_to_string)
}

pub fn create_checkpoint(state: &AriadneAppState, message: String) -> CommandResult<ArchivePoint> {
    let project_root = project_root_from_state(&state, None)?;
    create_checkpoint_impl(&project_root, message)
}

pub fn restore_to_new_branch(
    state: &AriadneAppState,
    commit_id: String,
    new_branch: String,
) -> CommandResult<RestoreReport> {
    let project_root = project_root_from_state(&state, None)?;
    GitService::new(project_root)
        .restore_to_new_branch(&commit_id, &new_branch)
        .map_err(error_to_string)
}

pub fn get_provider_config(state: &AriadneAppState) -> CommandResult<ProviderConfigStatus> {
    let project_root = project_root_from_state(&state, None)?;
    get_provider_config_impl(&project_root, state.secret_store.as_ref())
}

pub fn save_provider_key(
    state: &AriadneAppState,
    provider: String,
    key: String,
) -> CommandResult<()> {
    let project_root = project_root_from_state(&state, None)?;
    save_provider_key_impl(&project_root, state.secret_store.as_ref(), provider, key)
}

pub fn save_provider_settings(
    state: &AriadneAppState,
    update: ProviderSettingsUpdate,
) -> CommandResult<ProviderConfigStatus> {
    let project_root = project_root_from_state(&state, None)?;
    save_provider_settings_impl(&project_root, update)?;
    get_provider_config_impl(&project_root, state.secret_store.as_ref())
}

pub fn query_run_logs(
    state: &AriadneAppState,
    filter: Option<RunLogQuery>,
) -> CommandResult<Vec<UiRunLogEntry>> {
    let project_root = project_root_from_state(&state, None)?;
    let filter = filter.unwrap_or_default();
    UiRunLogStore::default_for_project(project_root)
        .query(UiRunLogFilter {
            kind: filter.kind,
            level: filter.level,
            node_id: filter.node_id.map(NodeId::from),
            query: filter.query,
        })
        .map_err(error_to_string)
}

pub fn mark_run_logs_read(state: &AriadneAppState) -> CommandResult<()> {
    let project_root = project_root_from_state(&state, None)?;
    UiRunLogStore::default_for_project(project_root)
        .mark_all_read()
        .map_err(error_to_string)
}

pub fn read_project_memory(state: &AriadneAppState) -> CommandResult<String> {
    let project_root = project_root_from_state(&state, None)?;
    ProjectMemoryStore::default_for_project(project_root)
        .read_all()
        .map_err(error_to_string)
}

pub fn append_project_memory(state: &AriadneAppState, content: String) -> CommandResult<String> {
    let project_root = project_root_from_state(&state, None)?;
    ProjectMemoryStore::default_for_project(project_root)
        .append(&content)
        .map_err(error_to_string)
}

pub fn write_project_memory(state: &AriadneAppState, content: String) -> CommandResult<()> {
    let project_root = project_root_from_state(&state, None)?;
    ProjectMemoryStore::default_for_project(project_root)
        .write_all(&content)
        .map_err(error_to_string)
}

pub fn quick_edit(
    state: &AriadneAppState,
    request: QuickEditRequest,
) -> CommandResult<QuickEditResult> {
    let project_root = project_root_from_state(&state, None)?;
    quick_edit_impl(&project_root, state.secret_store.as_ref(), request)
}

pub fn apply_quick_edit(
    state: &AriadneAppState,
    document_id: String,
    base_version: Option<String>,
    text: String,
    range: crate::contracts::TextRange,
    result: QuickEditResult,
) -> CommandResult<crate::documents::PatchApplyReport> {
    let project_root = project_root_from_state(&state, None)?;
    let documents = document_service(&project_root);
    crate::frontend::apply_quick_edit_patch(
        &documents,
        &document_id,
        base_version,
        &text,
        range,
        &result,
    )
    .map_err(error_to_string)
}

pub fn project_ai_chat(
    state: &AriadneAppState,
    request: ProjectAiRequest,
) -> CommandResult<ProjectAiResponse> {
    let project_root = project_root_from_state(&state, None)?;
    let runner_root = project_root.clone();
    let runner_secrets = Arc::clone(&state.secret_store);
    project_ai_chat_with_runner(
        &project_root,
        state.secret_store.as_ref(),
        request,
        &mut move |request| {
            start_workflow_request(&runner_root, Arc::clone(&runner_secrets), request)
        },
    )
}

pub fn resolve_project_reference(
    state: &AriadneAppState,
    reference: String,
) -> CommandResult<ProjectReference> {
    let project_root = project_root_from_state(&state, None)?;
    let documents = document_service(&project_root);
    let confirmations = FileConfirmationLogStore::default_for_project(&project_root);
    let chapter_index = load_chapter_index(&project_root)?;
    ProjectReferenceResolver::new()
        .with_documents(&documents)
        .with_confirmations(&confirmations)
        .with_chapter_index(&chapter_index)
        .resolve(&reference)
        .map_err(error_to_string)
}

pub fn get_ui_preferences(state: &AriadneAppState) -> CommandResult<UiPreferences> {
    let project_root = project_root_from_state(&state, None)?;
    UiPreferencesStore::default_for_project(project_root)
        .read()
        .map_err(error_to_string)
}

pub fn save_ui_preferences(
    state: &AriadneAppState,
    preferences: UiPreferences,
) -> CommandResult<()> {
    let project_root = project_root_from_state(&state, None)?;
    UiPreferencesStore::default_for_project(project_root)
        .write(&preferences)
        .map_err(error_to_string)
}

pub fn search_templates(
    request: TemplateRepositoryRequest,
    query: String,
    tags: Vec<String>,
    page: u32,
) -> CommandResult<Vec<TemplateSummary>> {
    template_client(request)?
        .search(&query, &tags, page)
        .map_err(error_to_string)
}

pub fn get_template_detail(
    request: TemplateRepositoryRequest,
    id: String,
) -> CommandResult<TemplateDetail> {
    template_client(request)?
        .detail(&id)
        .map_err(error_to_string)
}

pub fn install_template(
    state: &AriadneAppState,
    request: TemplateRepositoryRequest,
    id: String,
) -> CommandResult<TemplateInstallReport> {
    let project_root = project_root_from_state(&state, None)?;
    template_client(request)?
        .download_to_workflows(&id, project_root.join("workflows"))
        .map_err(error_to_string)
}

pub fn get_backend_diagnostics(state: &AriadneAppState) -> CommandResult<BackendDiagnosticsReport> {
    let project_root = project_root_from_state(&state, None)?;
    let config = ConfigStore::new(&project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let mut report = BackendDiagnosticsReport::collect(
        SqliteWorkflowRuntimeStore::health(&project_root),
        None,
        Vec::new(),
        Vec::new(),
    );
    report.extend_items(provider_config_diagnostic_items(&config.providers));
    Ok(report)
}

fn provider_config_diagnostic_items(
    providers: &crate::config::ProvidersConfig,
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
            reason: Some(reason),
        }),
    }

    let embedding_item = match providers.default_embedding_provider_id.as_deref() {
        Some(provider_id) => match providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == provider_id)
        {
            Some(provider) if !provider.enabled => DiagnosticItem {
                component: "providers.embedding.default".to_owned(),
                status: DiagnosticStatus::Unavailable,
                reason: Some(format!(
                    "default embedding provider is disabled: {provider_id}"
                )),
            },
            Some(provider)
                if provider
                    .models
                    .iter()
                    .any(|model| model.capability == ProviderCapability::Embedding) =>
            {
                DiagnosticItem {
                    component: "providers.embedding.default".to_owned(),
                    status: DiagnosticStatus::Healthy,
                    reason: None,
                }
            }
            Some(_) => DiagnosticItem {
                component: "providers.embedding.default".to_owned(),
                status: DiagnosticStatus::Degraded,
                reason: Some(format!(
                    "default embedding provider has no embedding model: {provider_id}"
                )),
            },
            None => DiagnosticItem {
                component: "providers.embedding.default".to_owned(),
                status: DiagnosticStatus::Unavailable,
                reason: Some(format!(
                    "default embedding provider is missing: {provider_id}"
                )),
            },
        },
        None => DiagnosticItem {
            component: "providers.embedding.default".to_owned(),
            status: DiagnosticStatus::Degraded,
            reason: Some("retrieval embedding model is not configured".to_owned()),
        },
    };
    items.push(embedding_item);
    items
}

pub fn default_project_root() -> PathBuf {
    std::env::var_os(DEFAULT_PROJECT_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_app_state_root() -> PathBuf {
    if let Some(path) = std::env::var_os(APP_STATE_ENV) {
        return PathBuf::from(path);
    }
    default_project_root().join(APP_STATE_DIR)
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
    let document_path = project_path(project_root, &document_id)?;
    let documents = document_service(project_root);
    documents
        .save_document(DocumentWriteRequest {
            path: document_path,
            content,
            format: None,
            base_version,
        })
        .map_err(error_to_string)
}

pub fn load_workflow_graph_impl(
    project_root: &Path,
    workflow_id: Option<String>,
) -> CommandResult<WorkflowGraphData> {
    let workflow = load_workflow_definition(project_root, workflow_id)?;
    Ok(workflow_to_graph(workflow))
}

pub fn save_workflow_graph_impl(
    project_root: &Path,
    graph_data: WorkflowGraphData,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let workflow = graph_to_workflow(graph_data)?;
    workflow.validate_topology().map_err(error_to_string)?;
    let path = workflow_path(project_root, Some(workflow.id.as_str().to_owned()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&workflow).map_err(error_to_string)?,
    )
    .map_err(error_to_string)
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
    let state = store
        .load_state(&workflow_id_typed, &run_id_typed)
        .map_err(error_to_string)?
        .ok_or_else(|| format!("workflow run not found: {workflow_id}/{run_id}"))?;
    let after_sequence = after_sequence.unwrap_or(0);
    let mut events = state
        .structured_events
        .iter()
        .filter(|event| event.sequence >= after_sequence)
        .cloned()
        .collect::<Vec<_>>();
    if let Some(limit) = limit {
        events.truncate(limit);
    }
    let next_sequence = events
        .last()
        .map(|event| event.sequence.saturating_add(1))
        .unwrap_or(after_sequence);
    Ok(WorkflowEventsResult {
        workflow_id,
        run_id,
        status: run_status_label(state.status).to_owned(),
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

fn run_workflow_impl_with_run_id(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: RunWorkflowRequest,
    run_id: RunId,
) -> CommandResult<WorkflowRunStarted> {
    validate_existing_project_root(project_root)?;
    let start_node_id = request.start_node_id.clone();
    let workflow = load_workflow_definition(project_root, Some(request.workflow_id))?;
    let mut workflow = if let Some(start_node_id) = start_node_id.as_deref() {
        workflow_branch_from_start(&workflow, &NodeId::from(start_node_id))?
    } else {
        workflow
    };
    if !request.initial_inputs.is_empty() {
        let Some(start_node_id) = start_node_id.as_deref() else {
            return Err("initial_inputs require start_node_id".to_owned());
        };
        inject_start_node_initial_inputs(&mut workflow, start_node_id, request.initial_inputs)?;
    }
    let document_root = workflow_document_root(project_root, &workflow, start_node_id.as_deref())?;
    let mut runtime = WorkflowRuntime::new(&workflow, run_id.clone()).map_err(error_to_string)?;
    runtime.state.start_node_id = start_node_id.as_deref().map(NodeId::from);
    let status = execute_workflow_runtime(
        project_root,
        &document_root,
        secrets,
        &workflow,
        &mut runtime,
    )?;
    Ok(WorkflowRunStarted {
        run_id: run_id.as_str().to_owned(),
        status: run_status_label(status).to_owned(),
    })
}

fn continue_workflow_run_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    workflow_id: String,
    run_id: String,
) -> CommandResult<WorkflowRunStarted> {
    validate_existing_project_root(project_root)?;
    let workflow_id_typed = WorkflowId::from(workflow_id.clone());
    let run_id_typed = RunId::from(run_id.clone());
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let state = store
        .load_state(&workflow_id_typed, &run_id_typed)
        .map_err(error_to_string)?
        .ok_or_else(|| format!("workflow run not found: {workflow_id}/{run_id}"))?;
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
        project_root,
        &document_root,
        secrets,
        &workflow,
        &mut runtime,
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
    let workflow = load_workflow_definition(project_root, Some(workflow_id.as_str().to_owned()))?;
    if let Some(start_node_id) = &state.start_node_id {
        let branch = workflow_branch_from_start(&workflow, start_node_id)?;
        return Ok((branch, Some(start_node_id.clone())));
    }

    let executed_start_nodes = workflow
        .nodes
        .iter()
        .filter(|node| node.type_name == "start" && state.nodes.contains_key(&node.id))
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    if executed_start_nodes.len() == 1 {
        let start_node_id = executed_start_nodes[0].clone();
        let branch = workflow_branch_from_start(&workflow, &start_node_id)?;
        return Ok((branch, Some(start_node_id)));
    }

    Ok((workflow, None))
}

fn execute_workflow_runtime(
    project_root: &Path,
    document_root: &Path,
    secrets: &dyn SecretStore,
    workflow: &WorkflowDefinition,
    runtime: &mut WorkflowRuntime,
) -> CommandResult<crate::contracts::RunStatus> {
    std::fs::create_dir_all(document_root.join("documents")).map_err(error_to_string)?;
    std::fs::create_dir_all(document_root.join("planning")).map_err(error_to_string)?;
    let documents = document_service_with_artifacts(
        document_root,
        project_root.join(".runtime").join("artifacts"),
    );
    let ledger = Arc::new(SqliteCostLedger::open(project_root).map_err(error_to_string)?);
    let llm_provider = if workflow_requires_llm_provider(&workflow) {
        Some(llm_runtime(project_root, secrets)?.provider)
    } else {
        None
    };
    let mut external = RoutedExternalNodeExecutor::new();
    if let Some(provider) = llm_provider {
        // 普通 LLM 语义节点走 execute_llm_node。summarizer 例外：它是四步总结
        // 生产链（故事段划分并概括 → 事件 → 章节 → 阶段），走专用 handler 落库建索引。
        for type_name in [
            "llm", "writer", "outliner", "designer", "planner", "detail", "critic", "prudent",
            "polisher",
        ] {
            let provider = provider.clone();
            let ledger = Arc::clone(&ledger);
            external
                .register_handler(
                    type_name,
                    Box::new(move |request| execute_llm_node(request, &provider, ledger.as_ref())),
                )
                .map_err(error_to_string)?;
        }

        // Summarizer 专用节点：加载写作知识库、四步总结、落库、生成四层确认项。
        {
            let provider = provider.clone();
            let ledger = Arc::clone(&ledger);
            let summarizer_root = project_root.to_path_buf();
            external
                .register_handler(
                    "summarizer",
                    Box::new(move |request| {
                        execute_summarizer_node(
                            request,
                            &provider,
                            ledger.as_ref(),
                            &summarizer_root,
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
    let mut export_sink = DocumentWorkflowExportSink::new(&documents);
    let mut executor =
        BuiltinWorkflowNodeExecutor::new(&mut external).with_export_sink(&mut export_sink);
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
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
        .ok_or_else(|| format!("start node not found: {start_node_id}"))?;
    if start_node.type_name != "start" {
        return Err(format!(
            "initial_inputs target must be a start node, got {} ({})",
            start_node.id.as_str(),
            start_node.type_name
        ));
    }
    let mut config = start_node.config.as_object().cloned().unwrap_or_default();
    config.insert(
        "initial_inputs".to_owned(),
        Value::Object(initial_inputs.into_iter().collect()),
    );
    start_node.config = Value::Object(config);
    Ok(())
}

fn workflow_requires_llm_provider(workflow: &WorkflowDefinition) -> bool {
    workflow
        .nodes
        .iter()
        .any(|node| is_llm_workflow_node_type(&node.type_name))
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

pub fn get_budget_status_impl(project_root: &Path) -> CommandResult<BudgetStatus> {
    validate_project_root(project_root)?;
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
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(AppSettings { app: config.app })
}

pub fn save_app_settings_impl(project_root: &Path, settings: AppSettings) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.app = settings.app;
    config.app.project_name = non_empty_or("project_name", config.app.project_name)?;
    config.app.documents_dir = non_empty_or("documents_dir", config.app.documents_dir)?;
    config.app.workflows_dir = non_empty_or("workflows_dir", config.app.workflows_dir)?;
    config.app.skills_dir = non_empty_or("skills_dir", config.app.skills_dir)?;
    config.app.exports_dir = non_empty_or("exports_dir", config.app.exports_dir)?;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_rag_settings_impl(project_root: &Path) -> CommandResult<RagSettings> {
    validate_project_root(project_root)?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(RagSettings { rag: config.rag })
}

pub fn save_rag_settings_impl(project_root: &Path, settings: RagSettings) -> CommandResult<()> {
    validate_project_root(project_root)?;
    settings.rag.validate().map_err(error_to_string)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.rag = settings.rag;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_workflow_settings_impl(project_root: &Path) -> CommandResult<WorkflowSettings> {
    validate_project_root(project_root)?;
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
    settings.workflow.validate().map_err(error_to_string)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.workflow = settings.workflow;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_git_settings_impl(project_root: &Path) -> CommandResult<GitSettings> {
    validate_project_root(project_root)?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    Ok(GitSettings { git: config.git })
}

pub fn save_git_settings_impl(project_root: &Path, settings: GitSettings) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.git = settings.git;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_template_repository_settings_impl(
    project_root: &Path,
) -> CommandResult<TemplateRepositorySettings> {
    validate_project_root(project_root)?;
    let path = template_repository_settings_path(project_root);
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map_err(error_to_string),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(TemplateRepositorySettings::default())
        }
        Err(error) => Err(error_to_string(error)),
    }
}

pub fn save_template_repository_settings_impl(
    project_root: &Path,
    settings: &TemplateRepositorySettings,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    if settings.base_url.trim().is_empty() {
        return Err("template repository base_url cannot be empty".to_owned());
    }
    validate_template_url(&settings.base_url)?;
    let path = template_repository_settings_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(settings).map_err(error_to_string)?,
    )
    .map_err(error_to_string)
}

pub fn update_budget_config_impl(
    project_root: &Path,
    budget_usd: f64,
    preauthorized_usd: f64,
) -> CommandResult<()> {
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
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    config.auto_mode.enabled_by_default = enabled;
    config_store.save(&config).map_err(error_to_string)
}

pub fn get_automation_settings_impl(project_root: &Path) -> CommandResult<AutomationSettings> {
    validate_project_root(project_root)?;
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let budget = get_budget_status_impl(project_root)?;
    let policies = read_confirmation_policy_settings(project_root)?.unwrap_or_else(|| {
        confirmation_policy_settings_from_prompts(&config.auto_mode.available_approval_prompts)
    });
    Ok(AutomationSettings {
        budget,
        confirmation_policies: policies,
    })
}

pub fn save_automation_settings_impl(
    project_root: &Path,
    settings: AutomationSettings,
) -> CommandResult<()> {
    update_budget_config_impl(
        project_root,
        settings.budget.budget_usd,
        settings.budget.preauthorized_usd,
    )?;
    set_auto_mode_impl(project_root, settings.budget.auto_mode_enabled)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    let mut normalized_settings = Vec::new();
    for setting in settings.confirmation_policies {
        if !confirmation_policy_keys().contains(&setting.confirmation_kind.as_str()) {
            return Err(format!(
                "unknown confirmation kind: {}",
                setting.confirmation_kind
            ));
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
        normalized_settings.push(setting);
    }
    config_store.save(&config).map_err(error_to_string)?;
    write_confirmation_policy_settings(project_root, &normalized_settings)
}

pub fn get_permissions_settings_impl(project_root: &Path) -> CommandResult<PermissionsSettings> {
    validate_project_root(project_root)?;
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
    validate_project_root(project_root)?;
    if request.workflow_id.trim().is_empty() {
        return Err("workflow_id cannot be empty".to_owned());
    }
    if request.run_id.trim().is_empty() {
        return Err("run_id cannot be empty".to_owned());
    }
    if request.confirmation_id.trim().is_empty() {
        return Err("confirmation_id cannot be empty".to_owned());
    }

    let workflow_id = WorkflowId::from(request.workflow_id.clone());
    let run_id = RunId::from(request.run_id.clone());
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let state = store
        .load_state(&workflow_id, &run_id)
        .map_err(error_to_string)?
        .ok_or_else(|| {
            format!(
                "workflow run not found: {}/{}",
                request.workflow_id, request.run_id
            )
        })?;
    let mut runtime = WorkflowRuntime::from_state(state);
    let next_state = match request.decision {
        ConfirmationDecision::Approve => RuntimeConfirmationState::Approved,
        ConfirmationDecision::Reject => RuntimeConfirmationState::Rejected,
    };
    if let Some(reason) = request
        .review_reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
    {
        if let Some(confirmation) = runtime
            .state
            .confirmations
            .get_mut(&request.confirmation_id)
        {
            if !confirmation.metadata.is_object() {
                confirmation.metadata = json!({});
            }
            if let Some(metadata) = confirmation.metadata.as_object_mut() {
                metadata.insert("reason".to_owned(), Value::String(reason.to_owned()));
            }
        }
    }
    runtime
        .update_confirmation_state(&request.confirmation_id, next_state)
        .map_err(error_to_string)?;
    store.save_state(&runtime.state).map_err(error_to_string)?;

    let runtime_confirmation = runtime
        .state
        .confirmations
        .get(&request.confirmation_id)
        .ok_or_else(|| format!("confirmation item not found: {}", request.confirmation_id))?;
    let confirmation =
        confirmation_log_entry_from_runtime(runtime_confirmation, request.review_reason.as_deref());
    let confirmation_store = FileConfirmationLogStore::default_for_project(project_root);
    confirmation_store
        .record(confirmation.clone())
        .map_err(error_to_string)?;

    Ok(ResolveConfirmationResult {
        workflow: WorkflowActionResult {
            workflow_id: request.workflow_id,
            run_id: request.run_id,
            status: run_status_label(runtime.state.status).to_owned(),
        },
        confirmation,
        badges: get_sidebar_badges_impl(project_root)?,
    })
}

pub fn get_git_history_impl(project_root: &Path) -> CommandResult<Vec<GitCommitSummary>> {
    validate_project_root(project_root)?;
    GitService::new(project_root)
        .recent_commits(100)
        .map_err(error_to_string)
}

pub fn create_checkpoint_impl(project_root: &Path, message: String) -> CommandResult<ArchivePoint> {
    validate_project_root(project_root)?;
    let name = if message.trim().is_empty() {
        "manual-checkpoint".to_owned()
    } else {
        message.trim().to_owned()
    };
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let policy = git_stage_policy_from_config(&config.git);
    GitService::new(project_root)
        .create_archive_point_with_policy(&name, Some(&name), &policy)
        .map_err(error_to_string)
}

fn git_stage_policy_from_config(config: &GitConfig) -> GitStagePolicy {
    let mut ignored_paths = config.ignored_paths.clone();
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
    let config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let providers = provider_status_list(project_root, &config.providers.providers)
        .into_iter()
        .map(|provider| {
            let key_id = provider
                .api_key
                .as_ref()
                .map(|secret| secret.key_id.clone())
                .unwrap_or_else(|| provider_key_id(project_root, &provider.provider_id));
            let has_key = secrets
                .get_secret(&key_id)
                .map(|secret| secret.is_some())
                .map_err(error_to_string)?;
            Ok(ProviderKeyStatus {
                provider: provider.provider_id,
                display_name: provider.display_name,
                provider_type: provider.provider_type,
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
        providers,
    })
}

pub fn save_provider_key_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    provider: String,
    key: String,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let provider = normalize_provider(&provider)?;
    if key.trim().is_empty() {
        return Err("provider key cannot be empty".to_owned());
    }
    let key_id = provider_key_id(project_root, &provider);
    secrets
        .set_secret(&key_id, SecretValue::new(key))
        .map_err(error_to_string)?;

    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    let provider_config = ensure_provider_config(&mut config.providers.providers, &provider);
    provider_config.api_key = Some(SecretRef::new(key_id));
    provider_config.enabled = true;
    if provider_config.models.is_empty() {
        provider_config
            .models
            .push(default_llm_model_for_provider(&provider));
    }
    if config.providers.default_llm_provider_id.is_none() {
        config.providers.default_llm_provider_id = Some(provider);
    }
    config_store.save(&config).map_err(error_to_string)
}

pub fn save_provider_settings_impl(
    project_root: &Path,
    update: ProviderSettingsUpdate,
) -> CommandResult<()> {
    validate_project_root(project_root)?;
    let provider_id = normalize_provider(&update.provider_id)?;
    let config_store = ConfigStore::new(project_root);
    let mut config = config_store.load_or_create().map_err(error_to_string)?;
    let existing_secret = config
        .providers
        .providers
        .iter()
        .find(|provider| provider.provider_id == provider_id)
        .and_then(|provider| provider.api_key.clone());
    let provider_config = ProviderConfig {
        provider_id: provider_id.clone(),
        provider_type: update.provider_type,
        display_name: non_empty_or("provider display_name", update.display_name)?,
        enabled: update.enabled,
        base_url: update.base_url,
        api_key: existing_secret,
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
        config.providers.default_reranker_provider_id = Some(provider_id);
    }
    config_store.save(&config).map_err(error_to_string)
}

pub fn quick_edit_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: QuickEditRequest,
) -> CommandResult<QuickEditResult> {
    let runtime = llm_runtime(project_root, secrets)?;
    let ledger = SqliteCostLedger::open(project_root).map_err(error_to_string)?;
    let service = LlmService::new(&ledger, runtime.auto_mode.clone());
    QuickEditService::new(service, &runtime.provider, runtime.config)
        .quick_edit(
            &request.selected_text,
            &request.instruction,
            request.context_ref.as_deref(),
        )
        .map_err(error_to_string)
}

pub fn project_ai_chat_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: ProjectAiRequest,
) -> CommandResult<ProjectAiResponse> {
    project_ai_chat_with_runner(project_root, secrets, request, &mut |request| {
        run_workflow_impl(project_root, secrets, request)
    })
}

fn project_ai_chat_with_runner(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: ProjectAiRequest,
    workflow_runner: &mut dyn FnMut(RunWorkflowRequest) -> CommandResult<WorkflowRunStarted>,
) -> CommandResult<ProjectAiResponse> {
    validate_project_root(project_root)?;
    if request.message.trim().is_empty()
        && request.chat_history.is_empty()
        && request
            .append_memory
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        && request.workflow_id_to_run.is_none()
    {
        return Err("project AI request cannot be empty".to_owned());
    }

    let memory_store = ProjectMemoryStore::default_for_project(project_root);
    if let Some(content) = request.append_memory.as_deref() {
        if !content.trim().is_empty() {
            memory_store.append(content).map_err(error_to_string)?;
        }
    }
    let project_memory = memory_store.read_all().map_err(error_to_string)?;
    let resolved_references = resolve_project_references(project_root, &request.references)?;
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

    let answer = if request.message.trim().is_empty() {
        "已处理项目记忆或工作流请求。".to_owned()
    } else {
        let (answer, tool_workflow_run) = project_ai_answer(
            project_root,
            secrets,
            &project_memory,
            &resolved_references,
            &request.chat_history,
            &request.message,
            &workflow_tools,
            workflow_runner,
        )?;
        if workflow_run.is_none() {
            workflow_run = tool_workflow_run;
        }
        answer
    };
    let chat_history =
        project_ai_response_history(&request.chat_history, &request.message, &answer)?;

    Ok(ProjectAiResponse {
        answer,
        chat_history,
        resolved_references,
        workflow_run,
        project_memory,
    })
}

pub fn resolve_project_references(
    project_root: &Path,
    references: &[String],
) -> CommandResult<Vec<ProjectReference>> {
    let documents = document_service(project_root);
    let confirmations = FileConfirmationLogStore::default_for_project(project_root);
    let chapter_index = load_chapter_index(project_root)?;
    let resolver = ProjectReferenceResolver::new()
        .with_documents(&documents)
        .with_confirmations(&confirmations)
        .with_chapter_index(&chapter_index);
    references
        .iter()
        .map(|reference| resolver.resolve(reference).map_err(error_to_string))
        .collect()
}

fn project_ai_answer(
    project_root: &Path,
    secrets: &dyn SecretStore,
    project_memory: &str,
    references: &[ProjectReference],
    chat_history: &[ProjectAiChatMessage],
    message: &str,
    workflow_tools: &[ProjectWorkflowTool],
    workflow_runner: &mut dyn FnMut(RunWorkflowRequest) -> CommandResult<WorkflowRunStarted>,
) -> CommandResult<(String, Option<WorkflowRunStarted>)> {
    let runtime = llm_runtime(project_root, secrets)?;
    let ledger = SqliteCostLedger::open(project_root).map_err(error_to_string)?;
    let service = LlmService::new(&ledger, runtime.auto_mode.clone());
    let messages = project_ai_llm_messages(project_memory, references, chat_history, message)?;
    let tool_definitions = project_ai_tool_definitions(workflow_tools);
    let report = service
        .complete_basic(
            &runtime.provider,
            LlmRunRequest {
                config: runtime.config,
                messages,
                tools: tool_definitions,
                workflow_id: None,
                run_id: None,
                node_id: None,
                metadata: json!({ "project_ai": true }),
            },
            &crate::contracts::CancellationToken::new(),
        )
        .map_err(error_to_string)?;
    let tool_workflow_run = if let Some((tool, arguments)) =
        report.response.tool_calls.iter().find_map(|call| {
            workflow_tools
                .iter()
                .find(|tool| tool.tool_name == call.name)
                .cloned()
                .map(|tool| (tool, call.arguments.clone()))
        }) {
        Some(workflow_runner(RunWorkflowRequest {
            workflow_id: tool.workflow_id,
            start_node_id: Some(tool.start_node_id),
            initial_inputs: workflow_tool_initial_inputs(arguments)?,
        })?)
    } else {
        None
    };
    let text = message_text(report.response.message.content);
    let answer = if text.trim().is_empty() && tool_workflow_run.is_some() {
        "ui.project_ai.workflow_tool_started".to_owned()
    } else {
        text
    };
    Ok((answer, tool_workflow_run))
}

fn project_ai_llm_messages(
    project_memory: &str,
    references: &[ProjectReference],
    chat_history: &[ProjectAiChatMessage],
    message: &str,
) -> CommandResult<Vec<LlmMessage>> {
    let reference_context = references
        .iter()
        .map(|reference| {
            format!(
                "- {} [{}]: {}\n  payload: {}",
                reference.reference, reference.id, reference.summary, reference.payload
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut messages = vec![
        LlmMessage {
            role: LlmRole::System,
            content: vec![ContentPart::text(
                "You are the Ariadne Project AI. Only answer based on project memory, explicit references, chat history, and user messages; do not fabricate project facts not provided.",
            )],
            name: None,
            tool_call_id: None,
        },
        LlmMessage::user(format!(
            "项目记忆：\n{}\n\n引用：\n{}",
            project_memory.trim(),
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
        if path.is_dir() {
            paths.extend(workflow_json_paths(&path)?);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
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
        other => Err(format!(
            "workflow tool arguments must be a JSON object, got {}",
            json_value_kind(&other)
        )),
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

fn project_ai_tool_definitions(workflow_tools: &[ProjectWorkflowTool]) -> Vec<ToolDefinition> {
    workflow_tools
        .iter()
        .map(|tool| ToolDefinition {
            name: tool.tool_name.clone(),
            description: format!(
                "Start Ariadne workflow '{}' from start node '{}'.",
                tool.display_name, tool.start_node_id
            ),
            input_schema: tool.input_schema.clone(),
        })
        .collect()
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
    let name = name.unwrap_or_else(|| project_display_name(project_root));
    recent_project_store(app_state_root)
        .record_opened(name, project_root)
        .map_err(error_to_string)
}

pub fn current_project_status(project_root: &Path) -> CommandResult<CurrentProjectStatus> {
    validate_project_root(project_root)?;
    Ok(CurrentProjectStatus {
        project_root: project_root.to_path_buf(),
        project_name: project_display_name(project_root),
    })
}

fn project_display_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Ariadne Project")
        .to_owned()
}

pub fn get_sidebar_badges_impl(project_root: &Path) -> CommandResult<SidebarBadgeCounts> {
    let run_logs = UiRunLogStore::default_for_project(project_root);
    let confirmations = FileConfirmationLogStore::default_for_project(project_root);
    run_logs
        .badge_counts(Some(&confirmations), None)
        .map_err(error_to_string)
}

fn confirmation_log_entry_from_runtime(
    confirmation: &RuntimeConfirmation,
    review_reason: Option<&str>,
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(index).map_err(error_to_string)?,
    )
    .map_err(error_to_string)
}

fn budget_config_path(project_root: &Path) -> PathBuf {
    project_root.join(".config").join(BUDGET_CONFIG_FILE)
}

fn template_repository_settings_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".runtime")
        .join(TEMPLATE_REPOSITORY_SETTINGS_FILE)
}

fn confirmation_policy_settings_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".config")
        .join(CONFIRMATION_POLICY_SETTINGS_FILE)
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
            return Err("node_type cannot be empty".to_owned());
        }
        if preset.model_id.trim().is_empty() {
            return Err(format!(
                "model_id cannot be empty for node_type {}",
                preset.node_type
            ));
        }
        if preset.timeout_ms == 0 {
            return Err(format!(
                "timeout_ms cannot be zero for node_type {}",
                preset.node_type
            ));
        }
        validate_money("budget_usd", preset.budget_usd)?;
        ensure_preset_model_is_configured(
            &configured_model_ids,
            &preset.model_id,
            &format!("preset {}", preset.node_type),
        )?;
    }
    if settings.default_model_id.trim().is_empty() {
        return Err("default_model_id cannot be empty".to_owned());
    }
    ensure_preset_model_is_configured(
        &configured_model_ids,
        &settings.default_model_id,
        "default_model_id",
    )?;
    if settings.default_timeout_ms == 0 {
        return Err("default_timeout_ms cannot be zero".to_owned());
    }
    validate_money("default_budget_usd", settings.default_budget_usd)?;
    let path = node_preset_settings_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(settings).map_err(error_to_string)?,
    )
    .map_err(error_to_string)
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
    Err(format!(
        "{field} references a model that is not configured in model settings: {model_id}"
    ))
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

fn confirmation_policy_keys() -> [&'static str; 4] {
    [
        "chapter_write",
        "summary_write",
        "high_risk_permission",
        "budget_exceeded",
    ]
}

fn confirmation_policy_settings_from_prompts(
    prompts: &[ApprovalPromptConfig],
) -> Vec<ConfirmationPolicySetting> {
    confirmation_policy_keys()
        .into_iter()
        .map(|kind| {
            let policy = policy_for_kind(prompts, kind);
            let (normal_policy, auto_mode_policy) = policies_from_policy_code(&policy);
            ConfirmationPolicySetting {
                confirmation_kind: kind.to_owned(),
                normal_policy,
                auto_mode_policy,
            }
        })
        .collect()
}

fn policies_from_policy_code(
    policy: &str,
) -> (ConfirmationNormalPolicy, ConfirmationAutoModePolicy) {
    match policy {
        "auto_skip" => (
            ConfirmationNormalPolicy::AllowByDefault,
            ConfirmationAutoModePolicy::AllowByDefault,
        ),
        "auto_audit" => (
            ConfirmationNormalPolicy::ManualReview,
            ConfirmationAutoModePolicy::AutoApproval,
        ),
        "manual_skip" => (
            ConfirmationNormalPolicy::ManualReview,
            ConfirmationAutoModePolicy::AllowByDefault,
        ),
        "auto_approve" => (
            ConfirmationNormalPolicy::AllowByDefault,
            ConfirmationAutoModePolicy::AutoApproval,
        ),
        "manual" => (
            ConfirmationNormalPolicy::ManualReview,
            ConfirmationAutoModePolicy::AllowByDefault,
        ),
        _ => (
            ConfirmationNormalPolicy::ManualReview,
            ConfirmationAutoModePolicy::AllowByDefault,
        ),
    }
}

fn policy_code_from_dual_policy(
    normal_policy: ConfirmationNormalPolicy,
    auto_mode_policy: ConfirmationAutoModePolicy,
) -> String {
    match (normal_policy, auto_mode_policy) {
        (ConfirmationNormalPolicy::AllowByDefault, ConfirmationAutoModePolicy::AllowByDefault) => {
            "auto_skip".to_owned()
        }
        (ConfirmationNormalPolicy::ManualReview, ConfirmationAutoModePolicy::AutoApproval) => {
            "auto_audit".to_owned()
        }
        (ConfirmationNormalPolicy::ManualReview, ConfirmationAutoModePolicy::AllowByDefault) => {
            "manual_skip".to_owned()
        }
        (ConfirmationNormalPolicy::AllowByDefault, ConfirmationAutoModePolicy::AutoApproval) => {
            "auto_approve".to_owned()
        }
    }
}

fn read_confirmation_policy_settings(
    project_root: &Path,
) -> CommandResult<Option<Vec<ConfirmationPolicySetting>>> {
    let path = confirmation_policy_settings_path(project_root);
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let settings = serde_json::from_str::<Vec<ConfirmationPolicySetting>>(&content)
                .map_err(error_to_string)?;
            Ok(Some(settings))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error_to_string(error)),
    }
}

fn write_confirmation_policy_settings(
    project_root: &Path,
    settings: &[ConfirmationPolicySetting],
) -> CommandResult<()> {
    let path = confirmation_policy_settings_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(settings).map_err(error_to_string)?,
    )
    .map_err(error_to_string)
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
        other => Err(format!("unknown confirmation policy: {other}")),
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(config).map_err(error_to_string)?,
    )
    .map_err(error_to_string)
}

fn update_workflow_run_control(
    project_root: &Path,
    workflow_id: String,
    run_id: String,
    update: impl FnOnce(&mut WorkflowRuntime) -> CoreResult<()>,
) -> CommandResult<WorkflowActionResult> {
    let workflow_id_typed = WorkflowId::from(workflow_id.clone());
    let run_id_typed = RunId::from(run_id.clone());
    let store = SqliteWorkflowRuntimeStore::open(project_root).map_err(error_to_string)?;
    let state = store
        .load_state(&workflow_id_typed, &run_id_typed)
        .map_err(error_to_string)?
        .ok_or_else(|| format!("workflow run not found: {workflow_id}/{run_id}"))?;
    let mut runtime = WorkflowRuntime::from_state(state);
    update(&mut runtime).map_err(error_to_string)?;
    store.save_state(&runtime.state).map_err(error_to_string)?;
    Ok(WorkflowActionResult {
        workflow_id,
        run_id,
        status: run_status_label(runtime.state.status).to_owned(),
    })
}

fn spawn_continue_if_queued(
    project_root: &Path,
    secrets: Arc<dyn SecretStore>,
    result: &WorkflowActionResult,
) -> CommandResult<()> {
    if result.status != "queued" {
        return Ok(());
    }
    spawn_continue_workflow_worker(
        project_root.to_path_buf(),
        secrets,
        result.workflow_id.clone(),
        result.run_id.clone(),
    )
}

fn spawn_continue_workflow_worker(
    project_root: PathBuf,
    secrets: Arc<dyn SecretStore>,
    workflow_id: String,
    run_id: String,
) -> CommandResult<()> {
    std::thread::Builder::new()
        .name(format!("ariadne-workflow-resume-{run_id}"))
        .spawn(move || {
            if let Err(error) = continue_workflow_run_impl(
                &project_root,
                secrets.as_ref(),
                workflow_id.clone(),
                run_id.clone(),
            ) {
                record_workflow_worker_error(
                    &project_root,
                    &workflow_id,
                    &run_id,
                    "workflow resume worker failed",
                    &error,
                );
                eprintln!("[ariadne] workflow resume worker failed: {error}");
            }
        })
        .map(|_| ())
        .map_err(error_to_string)
}

fn record_workflow_worker_error(
    project_root: &Path,
    workflow_id: &str,
    run_id: &str,
    context: &str,
    error: &str,
) {
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

fn template_client(request: TemplateRepositoryRequest) -> CommandResult<TemplateRepositoryClient> {
    let base_url = request
        .base_url
        .unwrap_or_else(|| DEFAULT_TEMPLATE_REPOSITORY_URL.to_owned());
    if base_url.trim().is_empty() {
        return Err(
            "template repository is not configured; please set a base URL in settings".to_owned(),
        );
    }
    validate_template_url(&base_url)?;
    TemplateRepositoryClient::new(base_url).map_err(error_to_string)
}

fn validate_template_url(url: &str) -> CommandResult<()> {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(())
    } else {
        let scheme = trimmed.split("://").next().unwrap_or(trimmed);
        Err(format!(
            "template URL must use http or https, got '{scheme}'"
        ))
    }
}

struct CommandLlmRuntime {
    provider: OpenAiCompatibleLlmProvider,
    config: LlmServiceConfig,
    auto_mode: crate::config::AutoModeConfig,
}

fn llm_runtime(project_root: &Path, secrets: &dyn SecretStore) -> CommandResult<CommandLlmRuntime> {
    validate_project_root(project_root)?;
    let project_config = ConfigStore::new(project_root)
        .load_or_create()
        .map_err(error_to_string)?;
    let provider_config = select_llm_provider(&project_config.providers)?;
    let model_config = select_llm_model(&provider_config)?;
    let api_key = provider_config
        .api_key
        .as_ref()
        .map(|secret_ref| {
            secrets
                .get_secret(&secret_ref.key_id)
                .map_err(error_to_string)?
                .map(|value| value.expose_secret().to_owned())
                .ok_or_else(|| format!("missing provider secret: {}", secret_ref.key_id))
        })
        .transpose()?;
    let provider = OpenAiCompatibleLlmProvider::new(provider_config.clone(), api_key)
        .map_err(error_to_string)?;
    Ok(CommandLlmRuntime {
        provider,
        config: LlmServiceConfig::new(provider_config.provider_id, model_config.model_id.clone())
            .with_model_config(&model_config),
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
            .ok_or_else(|| format!("default LLM provider is missing or disabled: {default_id}"));
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
        .ok_or_else(|| "no enabled LLM provider is configured".to_owned())
}

fn select_llm_model(provider: &ProviderConfig) -> CommandResult<ModelConfig> {
    provider
        .models
        .iter()
        .find(|model| model.capability == ProviderCapability::Llm)
        .or_else(|| provider.models.first())
        .cloned()
        .ok_or_else(|| {
            format!(
                "provider {} has no model configured for LLM calls",
                provider.provider_id
            )
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
            validate_project_root(&path)?;
            Ok(path)
        }
        _ => state.project_root(),
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
        .ok_or_else(|| format!("start node not found: {start_node_id}"))?;
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
        _ => Err("document_id or path is required".to_owned()),
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

fn load_workflow_definition(
    project_root: &Path,
    workflow_id: Option<String>,
) -> CommandResult<WorkflowDefinition> {
    let path = workflow_path(project_root, workflow_id)?;
    if !path.exists() {
        return Ok(WorkflowDefinition {
            id: WorkflowId::from("default"),
            name: "Default Workflow".to_owned(),
            nodes: Vec::new(),
            edges: Vec::new(),
            metadata: Value::Null,
        });
    }
    let content = std::fs::read_to_string(path).map_err(error_to_string)?;
    serde_json::from_str(&content).map_err(error_to_string)
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

fn default_provider_status_configs(project_root: &Path) -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: false,
            base_url: None,
            api_key: Some(SecretRef::new(provider_key_id(project_root, "openai"))),
            models: Vec::new(),
        },
        ProviderConfig {
            provider_id: "anthropic".to_owned(),
            provider_type: ProviderType::Anthropic,
            display_name: "Anthropic".to_owned(),
            enabled: false,
            base_url: None,
            api_key: Some(SecretRef::new(provider_key_id(project_root, "anthropic"))),
            models: Vec::new(),
        },
        ProviderConfig {
            provider_id: "gemini".to_owned(),
            provider_type: ProviderType::Gemini,
            display_name: "Gemini".to_owned(),
            enabled: false,
            base_url: None,
            api_key: Some(SecretRef::new(provider_key_id(project_root, "gemini"))),
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
        return Err("provider cannot be empty".to_owned());
    }
    Ok(provider)
}

fn provider_key_id(project_root: &Path, provider: &str) -> String {
    format!(
        "project.{}.provider.{provider}",
        project_secret_namespace(project_root)
    )
}

fn project_secret_namespace(project_root: &Path) -> String {
    let root = project_root
        .canonicalize()
        .unwrap_or_else(|_| absolute_path(project_root));
    let normalized = root.to_string_lossy().replace('\\', "/");
    crate::skills::stable_text_hash(&normalized)
}

fn validate_project_root(project_root: &Path) -> CommandResult<()> {
    if project_root.as_os_str().is_empty() {
        return Err("project_root cannot be empty".to_owned());
    }
    if !project_root.exists() {
        return Err(format!(
            "project root does not exist: {}",
            project_root.display()
        ));
    }
    if !project_root.is_dir() {
        return Err(format!(
            "project root is not a directory: {}",
            project_root.display()
        ));
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
        return Err(format!(
            "project root is not initialized: {}",
            project_root.display()
        ));
    }
    Ok(())
}

fn validate_money(field: &str, value: f64) -> CommandResult<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{field} must be finite and non-negative"))
    }
}

fn project_path(root: &Path, input: &str) -> CommandResult<PathBuf> {
    let raw = PathBuf::from(input);
    let path = if raw.is_absolute() {
        raw
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
        return Err("path cannot contain '..'".to_owned());
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
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "workflow root cannot be a symbolic link: {}",
            path.display()
        )),
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
        .ok_or_else(|| format!("start node not found: {}", start_node_id.as_str()))?;
    if start_node.type_name != "start" {
        return Err(format!(
            "start_node_id must reference a start node, got {} ({})",
            start_node_id.as_str(),
            start_node.type_name
        ));
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
        Err(format!("{field} cannot be empty"))
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

fn error_to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
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
