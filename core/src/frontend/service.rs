use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contracts::{
    ensure_path_under_root, ArtifactKind, CoreError, CoreResult, DocumentPatch, NodeId,
    NodeInstance, PatchHunk, PermissionPolicy, PortEndpoint, PortValue, RunId, TextRange,
    WorkflowDefinition, WorkflowId,
};
use crate::diagnostics::{BackendDiagnosticsReport, DiagnosticStatus};
use crate::documents::{
    ArtifactWriteRequest, ChapterDocumentEntry, ChapterDocumentIndex, ChapterDocumentKind,
    DocumentReadRequest, DocumentRepository, DocumentWriteRequest, FileDocumentService,
};
use crate::git::GitService;
use crate::llm::{LlmRunRequest, LlmService, LlmServiceConfig};
use crate::providers::{ContentPart, LlmMessage, LlmProvider};
use crate::rag::{FindRequest, FindScope, MemoryWritingKnowledgeBase};
use crate::skills::{WorkflowManifest, WORKFLOW_MANIFEST_FILE};
use crate::workflow::{RuntimeConfirmationState, WorkflowRunState};

/// 最近项目和项目初始化状态，默认落在 `.runtime/recent_projects.json`。
#[derive(Debug, Clone)]
pub struct ProjectRegistryStore {
    path: PathBuf,
}

/// 最近项目条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProjectEntry {
    pub name: String,
    pub path: PathBuf,
    pub last_opened_ms: u64,
}

/// 项目初始化报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInitReport {
    pub project_root: PathBuf,
    #[serde(default)]
    pub created_dirs: Vec<PathBuf>,
    pub git_initialized: bool,
}

impl ProjectRegistryStore {
    /// 创建最近项目存储。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 使用项目根目录下默认 `.runtime/recent_projects.json`。
    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".runtime/recent_projects.json"))
    }

    /// 读取最近项目列表。
    pub fn read_all(&self) -> CoreResult<Vec<RecentProjectEntry>> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => serde_json::from_str(&content).map_err(Into::into),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// 写入最近项目列表。
    pub fn write_all(&self, entries: &[RecentProjectEntry]) -> CoreResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(entries)?)?;
        Ok(())
    }

    /// 记录最近打开项目；同一路径移动到顶部。
    pub fn record_opened(
        &self,
        name: impl Into<String>,
        project_root: impl Into<PathBuf>,
    ) -> CoreResult<Vec<RecentProjectEntry>> {
        let project_root = project_root.into();
        let mut entries = self.read_all()?;
        entries.retain(|entry| entry.path != project_root);
        entries.insert(
            0,
            RecentProjectEntry {
                name: name.into(),
                path: project_root,
                last_opened_ms: now_timestamp_ms(),
            },
        );
        entries.truncate(20);
        self.write_all(&entries)?;
        Ok(entries)
    }
}

/// 初始化项目目录结构和 Git 仓库。
pub fn initialize_project(project_root: impl AsRef<Path>) -> CoreResult<ProjectInitReport> {
    let project_root = project_root.as_ref();
    validate_non_empty("project_root", &project_root.to_string_lossy())?;
    std::fs::create_dir_all(project_root)?;
    let dirs = [
        ".config",
        ".runtime",
        "planning",
        "planning/stages",
        "planning/chapters",
        "documents",
        "workflows",
    ];
    let mut created_dirs = Vec::new();
    for dir in dirs {
        let path = project_root.join(dir);
        std::fs::create_dir_all(&path)?;
        created_dirs.push(path);
    }
    let git = GitService::new(project_root);
    git.init_repository()?;
    Ok(ProjectInitReport {
        project_root: project_root.to_path_buf(),
        created_dirs,
        git_initialized: true,
    })
}

/// 构造项目内文档读写权限，供 Module 12 后端服务实例化文档服务。
pub fn project_document_permission(project_root: impl AsRef<Path>) -> PermissionPolicy {
    PermissionPolicy {
        readable_file_roots: vec![project_root.as_ref().to_path_buf()],
        writable_file_roots: vec![project_root.as_ref().to_path_buf()],
        ..PermissionPolicy::default()
    }
}

/// 项目记忆存储，默认落在 `.runtime/project_memory.md`。
#[derive(Debug, Clone)]
pub struct ProjectMemoryStore {
    path: PathBuf,
}

impl ProjectMemoryStore {
    /// 创建项目记忆存储。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 使用项目根目录下的默认 `.runtime/project_memory.md`。
    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".runtime/project_memory.md"))
    }

    /// 读取项目记忆全文；文件不存在时返回空串。
    pub fn read_all(&self) -> CoreResult<String> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => Ok(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// 覆盖写入项目记忆。
    pub fn write_all(&self, content: &str) -> CoreResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, content)?;
        Ok(())
    }

    /// 追加项目记忆内容，自动补换行边界。
    pub fn append(&self, content: &str) -> CoreResult<String> {
        let mut existing = self.read_all()?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(content);
        if !existing.ends_with('\n') {
            existing.push('\n');
        }
        self.write_all(&existing)?;
        Ok(existing)
    }
}

/// 确认项日志状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationLogState {
    Pending,
    Approved,
    Rejected,
    AutoAudited,
}

/// 确认项日志条目，用于 `@确认项:<id>` 引用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationLogEntry {
    pub confirmation_id: String,
    pub kind: String,
    pub node_id: String,
    pub timestamp_ms: u64,
    pub state: ConfirmationLogState,
    pub handling_method: String,
    pub summary: String,
    pub diff: String,
}

/// 确认项引用返回值，不内联完整正文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationReference {
    pub confirmation_id: String,
    pub state: ConfirmationLogState,
    pub diff: String,
    pub summary: String,
}

/// 内存确认项日志，后续 IPC 可替换为 SQLite 持久化。
#[derive(Debug, Default)]
pub struct ConfirmationLogStore {
    entries: std::sync::Mutex<BTreeMap<String, ConfirmationLogEntry>>,
}

impl ConfirmationLogStore {
    /// 写入确认项日志。
    pub fn record(&self, entry: ConfirmationLogEntry) -> CoreResult<()> {
        validate_non_empty("confirmation_id", &entry.confirmation_id)?;
        validate_non_empty("kind", &entry.kind)?;
        validate_non_empty("node_id", &entry.node_id)?;
        self.entries
            .lock()
            .map_err(lock_error)?
            .insert(entry.confirmation_id.clone(), entry);
        Ok(())
    }

    /// 通过 `@确认项:<confirmation_id>` 或裸 id 解析引用。
    pub fn resolve_reference(&self, reference: &str) -> CoreResult<ConfirmationReference> {
        let confirmation_id = reference
            .strip_prefix("@确认项:")
            .unwrap_or(reference)
            .trim();
        validate_non_empty("confirmation_id", confirmation_id)?;
        let entries = self.entries.lock().map_err(lock_error)?;
        let entry = entries.get(confirmation_id).ok_or_else(|| {
            CoreError::validation(format!("confirmation log not found: {confirmation_id}"))
        })?;
        Ok(ConfirmationReference {
            confirmation_id: entry.confirmation_id.clone(),
            state: entry.state,
            diff: entry.diff.clone(),
            summary: entry.summary.clone(),
        })
    }
}

/// 文件型确认项日志，默认落在 `.runtime/confirmation_log.json`。
#[derive(Debug, Clone)]
pub struct FileConfirmationLogStore {
    path: PathBuf,
}

impl FileConfirmationLogStore {
    /// 创建文件型确认项日志。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 使用项目根目录下的默认 `.runtime/confirmation_log.json`。
    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".runtime/confirmation_log.json"))
    }

    /// 读取全部确认项日志。
    pub fn read_all(&self) -> CoreResult<Vec<ConfirmationLogEntry>> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => serde_json::from_str(&content).map_err(Into::into),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// 覆盖写入确认项日志。
    pub fn write_all(&self, entries: &[ConfirmationLogEntry]) -> CoreResult<()> {
        for entry in entries {
            validate_confirmation_entry(entry)?;
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(entries)?;
        std::fs::write(&self.path, content)?;
        Ok(())
    }

    /// 追加或更新确认项日志；同 id 后写覆盖，保持状态最新。
    pub fn record(&self, entry: ConfirmationLogEntry) -> CoreResult<()> {
        validate_confirmation_entry(&entry)?;
        let mut entries = self.read_all()?;
        if let Some(existing) = entries
            .iter_mut()
            .find(|existing| existing.confirmation_id == entry.confirmation_id)
        {
            *existing = entry;
        } else {
            entries.push(entry);
        }
        self.write_all(&entries)
    }

    /// 通过 `@确认项:<confirmation_id>` 或裸 id 解析持久化引用。
    pub fn resolve_reference(&self, reference: &str) -> CoreResult<ConfirmationReference> {
        let confirmation_id = reference
            .strip_prefix("@确认项:")
            .unwrap_or(reference)
            .trim();
        validate_non_empty("confirmation_id", confirmation_id)?;
        let entry = self
            .read_all()?
            .into_iter()
            .find(|entry| entry.confirmation_id == confirmation_id)
            .ok_or_else(|| {
                CoreError::validation(format!("confirmation log not found: {confirmation_id}"))
            })?;
        Ok(ConfirmationReference {
            confirmation_id: entry.confirmation_id,
            state: entry.state,
            diff: entry.diff,
            summary: entry.summary,
        })
    }
}

/// 项目空间 AI 的引用对象类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectReferenceKind {
    NodeInput,
    NodeOutput,
    Confirmation,
    Document,
    Chapter,
    Knowledge,
    Artifact,
}

/// 解析后的项目引用。大对象只返回引用和摘要，不强制内联正文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectReference {
    pub reference: String,
    pub kind: ProjectReferenceKind,
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub payload: Value,
}

/// 运行 artifact 的轻量登记项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactReferenceEntry {
    pub artifact_id: String,
    pub kind: ArtifactKind,
    pub storage_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// 解析项目空间 AI 的 `@...` 引用。
pub struct ProjectReferenceResolver<'a> {
    confirmations: Option<&'a FileConfirmationLogStore>,
    documents: Option<&'a FileDocumentService>,
    chapter_index: Option<&'a ChapterDocumentIndex>,
    knowledge: Option<&'a MemoryWritingKnowledgeBase>,
    runtime: Option<&'a WorkflowRunState>,
    artifacts: BTreeMap<String, ArtifactReferenceEntry>,
}

impl<'a> ProjectReferenceResolver<'a> {
    /// 创建空引用解析器，按需要注入各模块后端。
    pub fn new() -> Self {
        Self {
            confirmations: None,
            documents: None,
            chapter_index: None,
            knowledge: None,
            runtime: None,
            artifacts: BTreeMap::new(),
        }
    }

    pub fn with_confirmations(mut self, confirmations: &'a FileConfirmationLogStore) -> Self {
        self.confirmations = Some(confirmations);
        self
    }

    pub fn with_documents(mut self, documents: &'a FileDocumentService) -> Self {
        self.documents = Some(documents);
        self
    }

    pub fn with_chapter_index(mut self, chapter_index: &'a ChapterDocumentIndex) -> Self {
        self.chapter_index = Some(chapter_index);
        self
    }

    pub fn with_knowledge(mut self, knowledge: &'a MemoryWritingKnowledgeBase) -> Self {
        self.knowledge = Some(knowledge);
        self
    }

    pub fn with_runtime(mut self, runtime: &'a WorkflowRunState) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn with_artifacts(mut self, artifacts: Vec<ArtifactReferenceEntry>) -> Self {
        self.artifacts = artifacts
            .into_iter()
            .map(|artifact| (artifact.artifact_id.clone(), artifact))
            .collect();
        self
    }

    /// 解析单个引用。
    pub fn resolve(&self, reference: &str) -> CoreResult<ProjectReference> {
        let (prefix, id) = parse_project_reference(reference)?;
        match prefix {
            "确认项" => self.resolve_confirmation(reference, id),
            "文档" => self.resolve_document(reference, id),
            "章节" => self.resolve_chapter(reference, id),
            "知识" => self.resolve_knowledge(reference, id),
            "artifact" => self.resolve_artifact(reference, id),
            "节点" => self.resolve_node_reference(reference, id),
            _ => Err(CoreError::validation(format!(
                "unsupported project reference prefix: {prefix}"
            ))),
        }
    }

    fn resolve_confirmation(&self, reference: &str, id: &str) -> CoreResult<ProjectReference> {
        let store = self
            .confirmations
            .ok_or_else(|| CoreError::validation("confirmation store is not configured"))?;
        let confirmation = store.resolve_reference(id)?;
        Ok(ProjectReference {
            reference: reference.to_owned(),
            kind: ProjectReferenceKind::Confirmation,
            id: confirmation.confirmation_id,
            summary: confirmation.summary,
            payload: json!({
                "state": confirmation.state,
                "diff": confirmation.diff,
            }),
        })
    }

    fn resolve_document(&self, reference: &str, id: &str) -> CoreResult<ProjectReference> {
        let documents = self
            .documents
            .ok_or_else(|| CoreError::validation("document service is not configured"))?;
        let content = documents.open_document(DocumentReadRequest {
            path: PathBuf::from(id),
            format: None,
        })?;
        Ok(ProjectReference {
            reference: reference.to_owned(),
            kind: ProjectReferenceKind::Document,
            id: content.metadata.document_id.clone(),
            summary: format!(
                "{} bytes, version {}",
                content.metadata.size_bytes, content.metadata.version
            ),
            payload: json!({
                "path": content.metadata.path,
                "media_type": content.metadata.media_type,
                "version": content.metadata.version,
            }),
        })
    }

    fn resolve_chapter(&self, reference: &str, id: &str) -> CoreResult<ProjectReference> {
        let index = self
            .chapter_index
            .ok_or_else(|| CoreError::validation("chapter index is not configured"))?;
        let entry = index
            .chapter_bodies()
            .into_iter()
            .find(|entry| entry.chapter_id == id)
            .ok_or_else(|| CoreError::validation(format!("chapter not found: {id}")))?;
        Ok(ProjectReference {
            reference: reference.to_owned(),
            kind: ProjectReferenceKind::Chapter,
            id: entry.chapter_id.clone(),
            summary: entry.title.clone(),
            payload: serde_json::to_value(entry)?,
        })
    }

    fn resolve_knowledge(&self, reference: &str, id: &str) -> CoreResult<ProjectReference> {
        let knowledge = self
            .knowledge
            .ok_or_else(|| CoreError::validation("knowledge base is not configured"))?;
        let result = [
            FindScope::CharacterProfile,
            FindScope::CharacterPlan,
            FindScope::CharacterTraitPath,
            FindScope::RelationshipPath,
            FindScope::EventSegments,
            FindScope::SegmentText,
            FindScope::Foreshadowing,
            FindScope::ThemeAnchor,
            FindScope::ChapterSummary,
            FindScope::StageSummary,
        ]
        .into_iter()
        .find_map(|scope| {
            knowledge
                .find(FindRequest {
                    scope,
                    query: id.to_owned(),
                    include_text: false,
                    metadata: Value::Null,
                })
                .ok()
                .and_then(|response| response.results.into_iter().next())
        })
        .ok_or_else(|| CoreError::validation(format!("knowledge item not found: {id}")))?;
        Ok(ProjectReference {
            reference: reference.to_owned(),
            kind: ProjectReferenceKind::Knowledge,
            id: result.result_id,
            summary: result.snippet,
            payload: json!({ "title": result.title, "source": result.source }),
        })
    }

    fn resolve_artifact(&self, reference: &str, id: &str) -> CoreResult<ProjectReference> {
        let artifact = self
            .artifacts
            .get(id)
            .ok_or_else(|| CoreError::validation(format!("artifact not found: {id}")))?;
        Ok(ProjectReference {
            reference: reference.to_owned(),
            kind: ProjectReferenceKind::Artifact,
            id: artifact.artifact_id.clone(),
            summary: artifact
                .summary
                .clone()
                .unwrap_or_else(|| artifact.storage_uri.clone()),
            payload: serde_json::to_value(artifact)?,
        })
    }

    fn resolve_node_reference(&self, reference: &str, id: &str) -> CoreResult<ProjectReference> {
        let runtime = self
            .runtime
            .ok_or_else(|| CoreError::validation("runtime state is not configured"))?;
        let (node_id, port_name, is_output) = parse_node_reference(id)?;
        let node = runtime.nodes.get(&NodeId::from(node_id)).ok_or_else(|| {
            CoreError::validation(format!("node runtime state not found: {node_id}"))
        })?;
        let value = if is_output {
            node.outputs
                .get(port_name)
                .cloned()
                .ok_or_else(|| CoreError::validation(format!("node output not found: {id}")))?
        } else {
            node.metadata
                .get("inputs")
                .and_then(|inputs| inputs.get(port_name))
                .and_then(|value| serde_json::from_value::<PortValue>(value.clone()).ok())
                .ok_or_else(|| CoreError::validation(format!("node input not found: {id}")))?
        };
        Ok(ProjectReference {
            reference: reference.to_owned(),
            kind: if is_output {
                ProjectReferenceKind::NodeOutput
            } else {
                ProjectReferenceKind::NodeInput
            },
            id: id.to_owned(),
            summary: format!(
                "node {node_id} {}",
                if is_output { "output" } else { "input" }
            ),
            payload: serde_json::to_value(value)?,
        })
    }
}

impl Default for ProjectReferenceResolver<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// UI 运行日志类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRunLogKind {
    Node,
    Tool,
    Provider,
    Cost,
    Confirmation,
    Error,
    Diagnostic,
}

/// UI 运行日志级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRunLogLevel {
    Info,
    Warning,
    Error,
}

/// 可检索运行日志条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiRunLogEntry {
    pub log_id: String,
    pub timestamp_ms: u64,
    pub kind: UiRunLogKind,
    pub level: UiRunLogLevel,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<WorkflowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub metadata: Value,
}

/// 运行日志过滤条件。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiRunLogFilter {
    pub kind: Option<UiRunLogKind>,
    pub level: Option<UiRunLogLevel>,
    pub node_id: Option<NodeId>,
    pub query: Option<String>,
}

/// toast 通知。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiToast {
    pub toast_id: String,
    pub timestamp_ms: u64,
    pub level: UiRunLogLevel,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// 侧栏徽标计数。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarBadgeCounts {
    pub run_logs: u32,
    pub confirmations: u32,
    pub diagnostics: u32,
}

/// 文件型运行日志，默认落在 `.runtime/run_log.json`。
#[derive(Debug, Clone)]
pub struct UiRunLogStore {
    path: PathBuf,
}

impl UiRunLogStore {
    /// 创建运行日志存储。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 使用项目根目录下默认 `.runtime/run_log.json`。
    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".runtime/run_log.json"))
    }

    /// 读取全部日志。
    pub fn read_all(&self) -> CoreResult<Vec<UiRunLogEntry>> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => serde_json::from_str(&content).map_err(Into::into),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// 覆盖写入日志。
    pub fn write_all(&self, entries: &[UiRunLogEntry]) -> CoreResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(entries)?)?;
        Ok(())
    }

    /// 追加日志并返回可展示 toast。
    pub fn append(&self, mut entry: UiRunLogEntry) -> CoreResult<UiToast> {
        validate_non_empty("log_id", &entry.log_id)?;
        validate_non_empty("message", &entry.message)?;
        if entry.timestamp_ms == 0 {
            entry.timestamp_ms = now_timestamp_ms();
        }
        entry.unread = true;
        let toast = UiToast {
            toast_id: format!("toast-{}", entry.log_id),
            timestamp_ms: entry.timestamp_ms,
            level: entry.level,
            message: entry.message.clone(),
            target: Some(match entry.kind {
                UiRunLogKind::Confirmation => "workspace.confirmations".to_owned(),
                UiRunLogKind::Error | UiRunLogKind::Diagnostic => "run_log".to_owned(),
                _ => "workspace.execution".to_owned(),
            }),
        };
        let mut entries = self.read_all()?;
        entries.push(entry);
        self.write_all(&entries)?;
        Ok(toast)
    }

    /// 按过滤条件查询日志。
    pub fn query(&self, filter: UiRunLogFilter) -> CoreResult<Vec<UiRunLogEntry>> {
        let query = filter.query.as_ref().map(|value| value.to_lowercase());
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|entry| filter.kind.map(|kind| entry.kind == kind).unwrap_or(true))
            .filter(|entry| {
                filter
                    .level
                    .map(|level| entry.level == level)
                    .unwrap_or(true)
            })
            .filter(|entry| {
                filter
                    .node_id
                    .as_ref()
                    .map(|node_id| entry.node_id.as_ref() == Some(node_id))
                    .unwrap_or(true)
            })
            .filter(|entry| {
                query
                    .as_ref()
                    .map(|query| entry.message.to_lowercase().contains(query))
                    .unwrap_or(true)
            })
            .collect())
    }

    /// 标记全部日志已读。
    pub fn mark_all_read(&self) -> CoreResult<()> {
        let mut entries = self.read_all()?;
        for entry in &mut entries {
            entry.unread = false;
        }
        self.write_all(&entries)
    }

    /// 汇总侧栏徽标。
    pub fn badge_counts(
        &self,
        confirmation_log: Option<&FileConfirmationLogStore>,
        diagnostics: Option<&BackendDiagnosticsReport>,
    ) -> CoreResult<SidebarBadgeCounts> {
        let entries = self.read_all()?;
        let run_logs = entries
            .iter()
            .filter(|entry| entry.unread)
            .filter(|entry| entry.level != UiRunLogLevel::Info)
            .count();
        let confirmations = confirmation_log
            .map(|store| {
                store
                    .read_all()
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|entry| entry.state == ConfirmationLogState::Pending)
                            .count()
                    })
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let diagnostics = diagnostics
            .map(|report| usize::from(report.status != DiagnosticStatus::Healthy))
            .unwrap_or(0);
        Ok(SidebarBadgeCounts {
            run_logs: usize_to_u32(run_logs)?,
            confirmations: usize_to_u32(confirmations)?,
            diagnostics: usize_to_u32(diagnostics)?,
        })
    }
}

/// 把 runtime 确认状态转为运行日志确认状态。
pub fn confirmation_state_from_runtime(state: RuntimeConfirmationState) -> ConfirmationLogState {
    match state {
        RuntimeConfirmationState::Pending => ConfirmationLogState::Pending,
        RuntimeConfirmationState::AutoAudited => ConfirmationLogState::AutoAudited,
        RuntimeConfirmationState::Approved => ConfirmationLogState::Approved,
        RuntimeConfirmationState::Rejected => ConfirmationLogState::Rejected,
    }
}

/// 作品导航树节点类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorksTreeNodeKind {
    GlobalOutline,
    StageOutline,
    Chapter,
}

/// 作品导航树节点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorksTreeNode {
    pub node_id: String,
    pub kind: WorksTreeNodeKind,
    pub title: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_ref: Option<crate::contracts::SourceSpan>,
    #[serde(default)]
    pub children: Vec<WorksTreeNode>,
}

/// 章节导入请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterImportRequest {
    pub chapter_id: String,
    pub title: String,
    pub order: u64,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_ref: Option<crate::contracts::SourceSpan>,
}

/// 章节合并导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterExportFormat {
    Markdown,
    Epub,
    Pdf,
}

impl ChapterExportFormat {
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Epub => "application/epub+zip",
            Self::Pdf => "application/pdf",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Epub => "epub",
            Self::Pdf => "pdf",
        }
    }
}

impl Default for ChapterExportFormat {
    fn default() -> Self {
        Self::Markdown
    }
}

/// 章节导入报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterImportReport {
    pub entry: ChapterDocumentEntry,
    pub index_invalidation: crate::documents::IndexInvalidation,
}

/// 合并导出报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombinedExportReport {
    pub artifact_id: String,
    pub format: ChapterExportFormat,
    pub exported_chapter_ids: Vec<String>,
    pub document_ids: Vec<String>,
    pub storage_uri: String,
    pub size_bytes: Option<u64>,
}

/// 从章节索引生成作品页导航树。
pub fn build_works_tree(
    index: &ChapterDocumentIndex,
    planning_root: impl AsRef<Path>,
) -> CoreResult<WorksTreeNode> {
    index.validate()?;
    let planning_root = planning_root.as_ref();
    let mut stages = BTreeMap::<String, Vec<&ChapterDocumentEntry>>::new();
    for entry in index.chapter_bodies() {
        let stage_id = entry
            .chapter_id
            .split_once(':')
            .map(|(stage, _)| stage)
            .unwrap_or("default")
            .to_owned();
        stages.entry(stage_id).or_default().push(entry);
    }
    let children = stages
        .into_iter()
        .map(|(stage_id, entries)| WorksTreeNode {
            node_id: format!("stage:{stage_id}"),
            kind: WorksTreeNodeKind::StageOutline,
            title: stage_id.clone(),
            path: planning_root
                .join("stages")
                .join(format!("{}.md", safe_file_stem(&stage_id))),
            outline_ref: None,
            children: entries
                .into_iter()
                .map(|entry| WorksTreeNode {
                    node_id: format!("chapter:{}", entry.chapter_id),
                    kind: WorksTreeNodeKind::Chapter,
                    title: entry.title.clone(),
                    path: entry.path.clone(),
                    outline_ref: entry.outline_ref.clone(),
                    children: Vec::new(),
                })
                .collect(),
        })
        .collect();
    Ok(WorksTreeNode {
        node_id: "global".to_owned(),
        kind: WorksTreeNodeKind::GlobalOutline,
        title: "ui.works.global_outline".to_owned(),
        path: planning_root.join("global.md"),
        outline_ref: None,
        children,
    })
}

/// 导入外部稿件为章节正文，并返回章节索引条目。
pub fn import_chapter_document(
    documents: &FileDocumentService,
    request: ChapterImportRequest,
) -> CoreResult<ChapterImportReport> {
    validate_non_empty("chapter_id", &request.chapter_id)?;
    validate_non_empty("chapter title", &request.title)?;
    let source = documents.open_document(DocumentReadRequest {
        path: request.source_path,
        format: None,
    })?;
    let word_count = count_words_for_ui(&source.content);
    let report = documents.save_document(DocumentWriteRequest {
        path: request.target_path.clone(),
        content: source.content,
        format: None,
        base_version: None,
    })?;
    let entry = ChapterDocumentEntry {
        chapter_id: request.chapter_id,
        document_id: report.metadata.document_id,
        path: report.metadata.path,
        title: request.title,
        order: request.order,
        kind: ChapterDocumentKind::ChapterBody,
        version: report.metadata.version,
        word_count: Some(word_count),
        outline_ref: request.outline_ref,
    };
    entry.validate()?;
    Ok(ChapterImportReport {
        entry,
        index_invalidation: report.index_invalidation,
    })
}

/// 合并导出选中章节正文为指定格式 artifact。
pub fn export_chapters_combined(
    documents: &FileDocumentService,
    index: &ChapterDocumentIndex,
    selected_chapter_ids: &[String],
    artifact_id: &str,
    format: ChapterExportFormat,
) -> CoreResult<CombinedExportReport> {
    let document_ids = index.export_document_ids(selected_chapter_ids)?;
    let selected = if selected_chapter_ids.is_empty() {
        None
    } else {
        Some(
            selected_chapter_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
        )
    };
    let mut exported_chapter_ids = Vec::new();
    let mut chapters = Vec::new();
    for entry in index.chapter_bodies() {
        if selected
            .as_ref()
            .map(|selected| !selected.contains(&entry.chapter_id))
            .unwrap_or(false)
        {
            continue;
        }
        let content = documents.open_document(DocumentReadRequest {
            path: entry.path.clone(),
            format: None,
        })?;
        exported_chapter_ids.push(entry.chapter_id.clone());
        chapters.push((entry.title.clone(), content.content));
    }
    let bytes = match format {
        ChapterExportFormat::Markdown => render_chapters_markdown(&chapters).into_bytes(),
        ChapterExportFormat::Epub => render_chapters_epub(&chapters)?,
        ChapterExportFormat::Pdf => render_chapters_pdf(&chapters),
    };
    let artifact = documents.write_artifact(ArtifactWriteRequest {
        artifact_id: artifact_id.to_owned(),
        kind: ArtifactKind::Export,
        media_type: format.media_type().to_owned(),
        bytes,
        metadata: json!({
            "chapter_ids": exported_chapter_ids,
            "document_ids": document_ids,
            "format": format.extension(),
        }),
    })?;
    Ok(CombinedExportReport {
        artifact_id: artifact.descriptor.artifact_id,
        format,
        exported_chapter_ids,
        document_ids,
        storage_uri: artifact.descriptor.storage_uri,
        size_bytes: artifact.descriptor.size_bytes,
    })
}

/// 合并导出选中章节正文为 Markdown artifact。
pub fn export_chapters_markdown(
    documents: &FileDocumentService,
    index: &ChapterDocumentIndex,
    selected_chapter_ids: &[String],
    artifact_id: &str,
) -> CoreResult<CombinedExportReport> {
    export_chapters_combined(
        documents,
        index,
        selected_chapter_ids,
        artifact_id,
        ChapterExportFormat::Markdown,
    )
}

fn render_chapters_markdown(chapters: &[(String, String)]) -> String {
    let mut combined = String::new();
    for (title, content) in chapters {
        if !combined.is_empty() {
            combined.push_str("\n\n");
        }
        combined.push_str(&format!("# {title}\n\n"));
        combined.push_str(content.trim_end());
        combined.push('\n');
    }
    combined
}

fn render_chapters_epub(chapters: &[(String, String)]) -> CoreResult<Vec<u8>> {
    let mut files = Vec::new();
    files.push((
        "mimetype".to_owned(),
        b"application/epub+zip".to_vec(),
        ZipCompression::Stored,
    ));
    files.push((
        "META-INF/container.xml".to_owned(),
        br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#
            .to_vec(),
        ZipCompression::Deflated,
    ));
    let manifest_items = chapters
        .iter()
        .enumerate()
        .map(|(index, _)| {
            format!(
                r#"<item id="chapter{index}" href="chapter{index}.xhtml" media-type="application/xhtml+xml"/>"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n    ");
    let spine_items = chapters
        .iter()
        .enumerate()
        .map(|(index, _)| format!(r#"<itemref idref="chapter{index}"/>"#))
        .collect::<Vec<_>>()
        .join("\n    ");
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="bookid" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">urn:uuid:ariadne-export</dc:identifier>
    <dc:title>Ariadne Export</dc:title>
    <dc:language>zh-CN</dc:language>
  </metadata>
  <manifest>
    {manifest_items}
  </manifest>
  <spine>
    {spine_items}
  </spine>
</package>"#
    );
    files.push((
        "OEBPS/content.opf".to_owned(),
        opf.into_bytes(),
        ZipCompression::Deflated,
    ));
    for (index, (title, content)) in chapters.iter().enumerate() {
        let chapter = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh-CN">
  <head><title>{}</title></head>
  <body><h1>{}</h1>{}</body>
</html>"#,
            escape_xml(title),
            escape_xml(title),
            markdown_to_xhtml_body(content)
        );
        files.push((
            format!("OEBPS/chapter{index}.xhtml"),
            chapter.into_bytes(),
            ZipCompression::Deflated,
        ));
    }
    Ok(write_zip_archive(&files))
}

const PDF_MAX_LINE_WIDTH: usize = 68;
const PDF_LINES_PER_PAGE: usize = 52;

fn render_chapters_pdf(chapters: &[(String, String)]) -> Vec<u8> {
    let text = render_chapters_markdown(chapters);
    let pages = pdf_pages(&text);
    let page_count = pages.len();
    let font_object_id = 3 + page_count * 2;
    let page_kids = (0..page_count)
        .map(|index| format!("{} 0 R", 3 + index * 2))
        .collect::<Vec<_>>()
        .join(" ");

    let mut objects = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_owned(),
        format!("2 0 obj\n<< /Type /Pages /Kids [{page_kids}] /Count {page_count} >>\nendobj\n"),
    ];

    for (index, lines) in pages.iter().enumerate() {
        let page_object_id = 3 + index * 2;
        let content_object_id = page_object_id + 1;
        objects.push(format!(
            "{page_object_id} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 {font_object_id} 0 R >> >> /Contents {content_object_id} 0 R >>\nendobj\n"
        ));

        let mut stream = String::from("BT\n/F1 11 Tf\n50 780 Td\n14 TL\n");
        for line in lines {
            stream.push_str(&pdf_utf16_hex_text(line));
            stream.push_str(" Tj\nT*\n");
        }
        stream.push_str("ET\n");
        objects.push(format!(
            "{content_object_id} 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            stream.len(),
            stream
        ));
    }

    objects.push(format!(
        "{font_object_id} 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 2 >> /DW 1000 >>] >>\nendobj\n"
    ));

    let mut bytes = b"%PDF-1.4\n%\xFF\xFF\xFF\xFF\n".to_vec();
    let mut offsets = Vec::new();
    for object in &objects {
        offsets.push(bytes.len());
        bytes.extend_from_slice(object.as_bytes());
    }
    let xref_offset = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );
    bytes
}

fn pdf_pages(text: &str) -> Vec<Vec<String>> {
    let lines = pdf_wrapped_lines(text);
    if lines.is_empty() {
        return vec![Vec::new()];
    }
    lines
        .chunks(PDF_LINES_PER_PAGE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

fn markdown_to_xhtml_body(markdown: &str) -> String {
    let mut output = String::new();
    for block in markdown.split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            output.push_str("<h2>");
            output.push_str(&escape_xml(heading));
            output.push_str("</h2>");
        } else {
            output.push_str("<p>");
            output.push_str(&escape_xml(trimmed).replace('\n', "<br/>"));
            output.push_str("</p>");
        }
    }
    output
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn pdf_wrapped_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }

        let mut current = String::new();
        let mut width = 0usize;
        for ch in raw_line.chars() {
            let char_width = pdf_char_width(ch);
            if width > 0 && width + char_width > PDF_MAX_LINE_WIDTH {
                lines.push(current);
                current = String::new();
                width = 0;
            }
            current.push(ch);
            width += char_width;
        }
        lines.push(current);
    }
    lines
}

fn pdf_char_width(ch: char) -> usize {
    if ch.is_ascii() {
        1
    } else {
        2
    }
}

fn pdf_utf16_hex_text(value: &str) -> String {
    let mut encoded = String::from("<");
    for unit in value.encode_utf16() {
        encoded.push_str(&format!("{unit:04X}"));
    }
    encoded.push('>');
    encoded
}

#[derive(Debug, Clone, Copy)]
enum ZipCompression {
    Stored,
    Deflated,
}

fn write_zip_archive(files: &[(String, Vec<u8>, ZipCompression)]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut central_directory = Vec::new();
    for (name, data, compression) in files {
        let local_offset = output.len() as u32;
        let compressed = match compression {
            ZipCompression::Stored => data.clone(),
            ZipCompression::Deflated => deflate_stored_blocks(data),
        };
        let method = match compression {
            ZipCompression::Stored => 0u16,
            ZipCompression::Deflated => 8u16,
        };
        let crc = crc32(data);
        write_u32_le(&mut output, 0x04034b50);
        write_u16_le(&mut output, 20);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, method);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, 0);
        write_u32_le(&mut output, crc);
        write_u32_le(&mut output, compressed.len() as u32);
        write_u32_le(&mut output, data.len() as u32);
        write_u16_le(&mut output, name.len() as u16);
        write_u16_le(&mut output, 0);
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(&compressed);

        write_u32_le(&mut central_directory, 0x02014b50);
        write_u16_le(&mut central_directory, 20);
        write_u16_le(&mut central_directory, 20);
        write_u16_le(&mut central_directory, 0);
        write_u16_le(&mut central_directory, method);
        write_u16_le(&mut central_directory, 0);
        write_u16_le(&mut central_directory, 0);
        write_u32_le(&mut central_directory, crc);
        write_u32_le(&mut central_directory, compressed.len() as u32);
        write_u32_le(&mut central_directory, data.len() as u32);
        write_u16_le(&mut central_directory, name.len() as u16);
        write_u16_le(&mut central_directory, 0);
        write_u16_le(&mut central_directory, 0);
        write_u16_le(&mut central_directory, 0);
        write_u16_le(&mut central_directory, 0);
        write_u32_le(&mut central_directory, 0);
        write_u32_le(&mut central_directory, local_offset);
        central_directory.extend_from_slice(name.as_bytes());
    }
    let central_offset = output.len() as u32;
    let central_size = central_directory.len() as u32;
    output.extend_from_slice(&central_directory);
    write_u32_le(&mut output, 0x06054b50);
    write_u16_le(&mut output, 0);
    write_u16_le(&mut output, 0);
    write_u16_le(&mut output, files.len() as u16);
    write_u16_le(&mut output, files.len() as u16);
    write_u32_le(&mut output, central_size);
    write_u32_le(&mut output, central_offset);
    write_u16_le(&mut output, 0);
    output
}

fn deflate_stored_blocks(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        let mut output = vec![0x01];
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, !0u16);
        return output;
    }
    let mut output = Vec::new();
    let chunk_count = data.chunks(u16::MAX as usize).count();
    for (index, chunk) in data.chunks(u16::MAX as usize).enumerate() {
        output.push(if index + 1 == chunk_count { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        write_u16_le(&mut output, len);
        write_u16_le(&mut output, !len);
        output.extend_from_slice(chunk);
    }
    output
}

fn write_u16_le(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_u32_le(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// 画布批注框，视觉分组但不改变 workflow 拓扑。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasAnnotation {
    pub annotation_id: String,
    pub title: String,
    #[serde(default)]
    pub node_ids: Vec<NodeId>,
    #[serde(default)]
    pub metadata: Value,
}

/// 节点细节配置 patch。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDetailPatch {
    pub node_id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_template: Option<String>,
    #[serde(default)]
    pub input_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub tool_enabled: BTreeMap<String, bool>,
    #[serde(default)]
    pub approval_policy: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// 应用节点细节 patch 到节点 config，保证内联和右侧详情同源。
pub fn apply_node_detail_patch(
    workflow: &mut WorkflowDefinition,
    patch: NodeDetailPatch,
) -> CoreResult<()> {
    let input_aliases = patch.input_aliases.clone();
    let node = workflow
        .nodes
        .iter_mut()
        .find(|node| node.id == patch.node_id)
        .ok_or_else(|| {
            CoreError::validation(format!("node not found: {}", patch.node_id.as_str()))
        })?;
    let mut config = node.config.as_object().cloned().unwrap_or_default();
    if let Some(prompt_template) = patch.prompt_template {
        config.insert("prompt_template".to_owned(), Value::String(prompt_template));
    }
    if !patch.input_aliases.is_empty() {
        config.insert(
            "input_aliases".to_owned(),
            serde_json::to_value(patch.input_aliases)?,
        );
    }
    if !patch.tool_enabled.is_empty() {
        config.insert(
            "tool_enabled".to_owned(),
            serde_json::to_value(patch.tool_enabled)?,
        );
    }
    if !patch.approval_policy.is_empty() {
        config.insert(
            "approval_policy".to_owned(),
            serde_json::to_value(patch.approval_policy)?,
        );
    }
    if let Some(model_id) = patch.model_id {
        config.insert("model_id".to_owned(), Value::String(model_id));
    }
    if let Some(budget_usd) = patch.budget_usd {
        if !budget_usd.is_finite() || budget_usd < 0.0 {
            return Err(CoreError::validation("budget_usd must be non-negative"));
        }
        config.insert("budget_usd".to_owned(), json!(budget_usd));
    }
    if let Some(timeout_ms) = patch.timeout_ms {
        config.insert("timeout_ms".to_owned(), json!(timeout_ms));
    }
    if !input_aliases.is_empty() {
        let next_inputs = input_aliases
            .values()
            .map(|alias| alias.trim().to_owned())
            .filter(|alias| !alias.is_empty())
            .collect::<Vec<_>>();
        config.insert("inputs".to_owned(), serde_json::to_value(next_inputs)?);
    }
    node.config = Value::Object(config);

    if !input_aliases.is_empty() {
        for edge in workflow.edges.iter_mut().filter(|edge| {
            edge.kind == crate::contracts::WorkflowEdgeKind::Data
                && edge.to.node_id == patch.node_id
        }) {
            if let Some(current_alias) = edge.alias.as_deref() {
                if let Some(next_alias) = input_aliases.get(current_alias) {
                    validate_non_empty("input alias", next_alias)?;
                    edge.alias = Some(next_alias.trim().to_owned());
                    edge.to.port_name = format!("data-in-{}", next_alias.trim());
                }
            } else if let Some(next_alias) = input_aliases.get(&edge.to.port_name) {
                validate_non_empty("input alias", next_alias)?;
                edge.alias = Some(next_alias.trim().to_owned());
                edge.to.port_name = format!("data-in-{}", next_alias.trim());
            }
        }
    }
    Ok(())
}

/// 更新或插入 workflow metadata 中的批注框。
pub fn upsert_canvas_annotation(
    workflow: &mut WorkflowDefinition,
    annotation: CanvasAnnotation,
) -> CoreResult<()> {
    validate_non_empty("annotation_id", &annotation.annotation_id)?;
    let mut metadata = workflow.metadata.as_object().cloned().unwrap_or_default();
    let mut annotations = metadata
        .remove("canvas_annotations")
        .and_then(|value| serde_json::from_value::<Vec<CanvasAnnotation>>(value).ok())
        .unwrap_or_default();
    if let Some(existing) = annotations
        .iter_mut()
        .find(|existing| existing.annotation_id == annotation.annotation_id)
    {
        *existing = annotation;
    } else {
        annotations.push(annotation);
    }
    metadata.insert(
        "canvas_annotations".to_owned(),
        serde_json::to_value(annotations)?,
    );
    workflow.metadata = Value::Object(metadata);
    Ok(())
}

/// UI 外观和交互偏好，默认落在 `.runtime/ui_preferences.json`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiPreferences {
    pub theme: String,
    pub git_auto_color: String,
    pub git_manual_color: String,
    pub project_panel_visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_panel_position: Option<(i32, i32)>,
    #[serde(default)]
    pub panel_states: BTreeMap<String, bool>,
    pub onboarding_seen: bool,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: "system".to_owned(),
            git_auto_color: "#8a8f98".to_owned(),
            git_manual_color: "#f59e0b".to_owned(),
            project_panel_visible: true,
            project_panel_position: None,
            panel_states: BTreeMap::new(),
            onboarding_seen: false,
        }
    }
}

/// UI 偏好文件存储。
#[derive(Debug, Clone)]
pub struct UiPreferencesStore {
    path: PathBuf,
}

impl UiPreferencesStore {
    /// 创建 UI 偏好存储。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 使用项目根目录下默认 `.runtime/ui_preferences.json`。
    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".runtime/ui_preferences.json"))
    }

    /// 读取 UI 偏好；不存在时返回默认值。
    pub fn read(&self) -> CoreResult<UiPreferences> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => serde_json::from_str(&content).map_err(Into::into),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(UiPreferences::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// 写入 UI 偏好。
    pub fn write(&self, preferences: &UiPreferences) -> CoreResult<()> {
        validate_color("git_auto_color", &preferences.git_auto_color)?;
        validate_color("git_manual_color", &preferences.git_manual_color)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(preferences)?)?;
        Ok(())
    }
}

/// 框选导出的子流程片段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSelectionExport {
    pub workflow: WorkflowDefinition,
    #[serde(default)]
    pub boundary_inputs: Vec<PortEndpoint>,
    #[serde(default)]
    pub boundary_outputs: Vec<PortEndpoint>,
}

/// 子工作流打包报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowPackReport {
    pub workflow: WorkflowDefinition,
    pub subworkflow_node_id: NodeId,
    pub embedded_workflow: WorkflowDefinition,
    #[serde(default)]
    pub boundary_inputs: Vec<PortEndpoint>,
    #[serde(default)]
    pub boundary_outputs: Vec<PortEndpoint>,
}

/// 为节点配置断点。
pub fn set_node_breakpoint(
    workflow: &mut WorkflowDefinition,
    node_id: &str,
    enabled: bool,
) -> CoreResult<()> {
    let node = workflow
        .nodes
        .iter_mut()
        .find(|node| node.id.as_str() == node_id)
        .ok_or_else(|| CoreError::validation(format!("node not found: {node_id}")))?;
    let mut config = node.config.as_object().cloned().unwrap_or_default();
    config.insert("breakpoint".to_owned(), Value::Bool(enabled));
    node.config = Value::Object(config);
    Ok(())
}

/// 判断节点是否配置断点。
pub fn node_has_breakpoint(node: &NodeInstance) -> bool {
    node.config
        .get("breakpoint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 导出框选节点及其内部连线，边界连线映射为入口/出口描述。
pub fn export_workflow_selection(
    workflow: &WorkflowDefinition,
    selected_node_ids: &[String],
) -> CoreResult<WorkflowSelectionExport> {
    if selected_node_ids.is_empty() {
        return Err(CoreError::validation(
            "workflow selection requires at least one node",
        ));
    }
    let selected = selected_node_ids.iter().cloned().collect::<BTreeSet<_>>();
    let nodes = workflow
        .nodes
        .iter()
        .filter(|node| selected.contains(node.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if nodes.len() != selected.len() {
        return Err(CoreError::validation(
            "workflow selection references missing node",
        ));
    }

    let mut internal_edges = Vec::new();
    let mut boundary_inputs = Vec::new();
    let mut boundary_outputs = Vec::new();
    for edge in &workflow.edges {
        let source_selected = selected.contains(edge.from.node_id.as_str());
        let target_selected = selected.contains(edge.to.node_id.as_str());
        match (source_selected, target_selected) {
            (true, true) => internal_edges.push(edge.clone()),
            (false, true) => boundary_inputs.push(edge.to.clone()),
            (true, false) => boundary_outputs.push(edge.from.clone()),
            (false, false) => {}
        }
    }

    let exported = WorkflowDefinition {
        id: WorkflowId::from(format!("{}::selection", workflow.id.as_str())),
        name: format!("{} selection", workflow.name),
        nodes,
        edges: internal_edges,
        metadata: json!({
            "source_workflow_id": workflow.id.as_str(),
            "selected_node_ids": selected_node_ids,
        }),
    };
    exported.validate_topology()?;
    Ok(WorkflowSelectionExport {
        workflow: exported,
        boundary_inputs,
        boundary_outputs,
    })
}

/// 把框选节点折叠成一个 subworkflow 节点，并把原片段嵌入节点 config。
pub fn pack_workflow_selection(
    workflow: &mut WorkflowDefinition,
    selected_node_ids: &[String],
    subworkflow_node_id: Option<String>,
    title: Option<String>,
) -> CoreResult<WorkflowPackReport> {
    let selection = export_workflow_selection(workflow, selected_node_ids)?;
    let selected = selected_node_ids.iter().cloned().collect::<BTreeSet<_>>();
    let subworkflow_node_id = NodeId::from(
        subworkflow_node_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("subworkflow-{}", now_timestamp_ms())),
    );
    if workflow
        .nodes
        .iter()
        .any(|node| node.id == subworkflow_node_id)
    {
        return Err(CoreError::validation(format!(
            "subworkflow node already exists: {}",
            subworkflow_node_id.as_str()
        )));
    }
    let average_position = selection
        .workflow
        .nodes
        .iter()
        .filter_map(|node| node.position)
        .fold((0.0, 0.0, 0usize), |(sum_x, sum_y, count), position| {
            (sum_x + position.x, sum_y + position.y, count + 1)
        });
    let (x, y) = if average_position.2 == 0 {
        (160.0, 160.0)
    } else {
        (
            average_position.0 / average_position.2 as f64,
            average_position.1 / average_position.2 as f64,
        )
    };
    let subworkflow_node = NodeInstance {
        id: subworkflow_node_id.clone(),
        type_name: "subworkflow".to_owned(),
        label: Some(title.unwrap_or_else(|| "ui.workspace.subworkflow".to_owned())),
        config: json!({
            "embedded_workflow": selection.workflow.clone(),
            "boundary_inputs": selection.boundary_inputs.clone(),
            "boundary_outputs": selection.boundary_outputs.clone(),
        }),
        position: Some(crate::contracts::CanvasPosition { x, y }),
    };
    let mut new_edges = Vec::new();
    for edge in workflow.edges.iter().filter(|edge| {
        !(selected.contains(edge.from.node_id.as_str())
            && selected.contains(edge.to.node_id.as_str()))
    }) {
        let source_selected = selected.contains(edge.from.node_id.as_str());
        let target_selected = selected.contains(edge.to.node_id.as_str());
        let mut edge = edge.clone();
        if source_selected {
            edge.from.node_id = subworkflow_node_id.clone();
        }
        if target_selected {
            edge.to.node_id = subworkflow_node_id.clone();
        }
        if let Some(config) = edge.communication.as_mut() {
            if config
                .initiator_node_id
                .as_ref()
                .is_some_and(|node_id| selected.contains(node_id.as_str()))
            {
                config.initiator_node_id = Some(subworkflow_node_id.clone());
            }
        }
        if source_selected || target_selected {
            edge.id = crate::contracts::EdgeId::from(format!(
                "{}-{}",
                subworkflow_node_id.as_str(),
                edge.id.as_str()
            ));
        }
        new_edges.push(edge);
    }
    workflow
        .nodes
        .retain(|node| !selected.contains(node.id.as_str()));
    workflow.nodes.push(subworkflow_node);
    workflow.edges = new_edges;
    workflow.validate_topology()?;
    let embedded_workflow = workflow
        .nodes
        .iter()
        .find(|node| node.id == subworkflow_node_id)
        .and_then(|node| node.config.get("embedded_workflow"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| selection.workflow.clone());
    Ok(WorkflowPackReport {
        workflow: workflow.clone(),
        subworkflow_node_id,
        embedded_workflow,
        boundary_inputs: selection.boundary_inputs,
        boundary_outputs: selection.boundary_outputs,
    })
}

/// 在线模板摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub requires_permissions: bool,
}

/// 在线模板详情。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateDetail {
    pub id: String,
    pub name: String,
    pub version: String,
    pub manifest: Value,
    #[serde(default)]
    pub requires_permissions: bool,
}

/// 模板下载写入本地后的报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateInstallReport {
    pub workflow_id: String,
    pub version: String,
    pub manifest_path: PathBuf,
    pub requires_permissions: bool,
    #[serde(default)]
    pub required_permissions: Vec<String>,
}

/// 在线模板仓库客户端。
#[derive(Debug, Clone)]
pub struct TemplateRepositoryClient {
    base_url: String,
    client: Client,
}

impl TemplateRepositoryClient {
    /// 创建模板仓库客户端。
    pub fn new(base_url: impl Into<String>) -> CoreResult<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        validate_non_empty("template repository base_url", &base_url)?;
        Ok(Self {
            base_url,
            client: Client::new(),
        })
    }

    /// 搜索模板。
    pub fn search(
        &self,
        query: &str,
        tags: &[String],
        page: u32,
    ) -> CoreResult<Vec<TemplateSummary>> {
        let response = self
            .client
            .get(format!("{}/templates/search", self.base_url))
            .query(&[("query", query), ("page", &page.to_string())])
            .query(&[("tags", &tags.join(","))])
            .send()
            .map_err(template_repo_error)?;
        parse_success_json(response)
    }

    /// 获取模板详情。
    pub fn detail(&self, id: &str) -> CoreResult<TemplateDetail> {
        validate_non_empty("template id", id)?;
        let response = self
            .client
            .get(format!("{}/templates/{id}", self.base_url))
            .send()
            .map_err(template_repo_error)?;
        parse_success_json(response)
    }

    /// 下载模板 manifest。
    pub fn download(&self, id: &str) -> CoreResult<Value> {
        validate_non_empty("template id", id)?;
        let response = self
            .client
            .get(format!("{}/templates/{id}/download", self.base_url))
            .send()
            .map_err(template_repo_error)?;
        parse_success_json(response)
    }

    /// 下载并写入本地 workflows 目录，写入前校验 WorkflowManifest。
    pub fn download_to_workflows(
        &self,
        id: &str,
        workflows_root: impl AsRef<Path>,
    ) -> CoreResult<TemplateInstallReport> {
        let manifest_value = self.download(id)?;
        install_workflow_template_manifest(manifest_value, workflows_root, false)
    }
}

/// 将下载到的 workflow manifest 写入本地 `workflows/<id>/workflow.json`。
pub fn install_workflow_template_manifest(
    manifest_value: Value,
    workflows_root: impl AsRef<Path>,
    requires_permissions: bool,
) -> CoreResult<TemplateInstallReport> {
    let manifest: WorkflowManifest = serde_json::from_value(manifest_value)?;
    manifest.validate()?;
    validate_path_component("workflow_id", &manifest.workflow_id)?;
    let workflows_root = absolute_path(workflows_root.as_ref())?;
    reject_symlink_root(&workflows_root)?;
    let manifest_dir = workflows_root.join(&manifest.workflow_id);
    let manifest_path = manifest_dir.join(WORKFLOW_MANIFEST_FILE);
    ensure_path_under_root(&workflows_root, &manifest_path)?;
    std::fs::create_dir_all(&manifest_dir)?;
    let content = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(&manifest_path, content)?;
    Ok(TemplateInstallReport {
        workflow_id: manifest.workflow_id,
        version: manifest.version,
        manifest_path,
        requires_permissions: requires_permissions || !manifest.required_permissions.is_empty(),
        required_permissions: manifest.required_permissions,
    })
}

/// quick edit 输出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuickEditResult {
    pub original: String,
    pub suggested: String,
    pub diff: String,
}

/// Cmd/K 快捷 AI 后端。
pub struct QuickEditService<'a, L: crate::costs::CostLedger> {
    llm: LlmService<'a, L>,
    provider: &'a dyn LlmProvider,
    config: LlmServiceConfig,
}

impl<'a, L: crate::costs::CostLedger> QuickEditService<'a, L> {
    /// 创建 quick edit 服务。
    pub fn new(
        llm: LlmService<'a, L>,
        provider: &'a dyn LlmProvider,
        config: LlmServiceConfig,
    ) -> Self {
        Self {
            llm,
            provider,
            config,
        }
    }

    /// 执行一次轻量 LLM 改写，不进入完整 workflow。
    pub fn quick_edit(
        &self,
        selected_text: &str,
        instruction: &str,
        context_ref: Option<&str>,
    ) -> CoreResult<QuickEditResult> {
        validate_non_empty("selected_text", selected_text)?;
        validate_non_empty("instruction", instruction)?;
        let prompt = format!(
            "请只返回改写后的文本。\n指令：{instruction}\n上下文引用：{}\n原文：\n{selected_text}",
            context_ref.unwrap_or("")
        );
        let report = self.llm.complete_basic(
            self.provider,
            LlmRunRequest {
                config: self.config.clone(),
                messages: vec![LlmMessage::user(prompt)],
                tools: Vec::new(),
                workflow_id: None,
                run_id: None,
                node_id: None,
                metadata: json!({ "quick_edit": true }),
            },
            &crate::contracts::CancellationToken::new(),
        )?;
        let suggested = report
            .response
            .message
            .content
            .into_iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(QuickEditResult {
            original: selected_text.to_owned(),
            diff: simple_diff(selected_text, &suggested),
            suggested,
        })
    }
}

/// 将 quick edit 结果转换为当前文档 patch。
pub fn quick_edit_to_patch(
    document: &str,
    document_id: &str,
    base_version: Option<String>,
    range: TextRange,
    result: &QuickEditResult,
) -> CoreResult<crate::contracts::DocumentPatch> {
    validate_non_empty("document_id", document_id)?;
    let start = usize::try_from(range.start)
        .map_err(|_| CoreError::validation("quick edit range start exceeds usize"))?;
    let end = usize::try_from(range.end)
        .map_err(|_| CoreError::validation("quick edit range end exceeds usize"))?;
    if start > end
        || end > document.len()
        || !document.is_char_boundary(start)
        || !document.is_char_boundary(end)
    {
        return Err(CoreError::validation(
            "quick edit range is not a valid UTF-8 slice",
        ));
    }
    let selected = &document[start..end];
    let hunks = if selected == result.suggested {
        Vec::new()
    } else {
        vec![PatchHunk {
            range,
            replacement: result.suggested.clone(),
        }]
    };
    Ok(DocumentPatch {
        document_id: document_id.to_owned(),
        base_version,
        hunks,
    })
}

/// 为当前文档应用 quick edit patch。
pub fn apply_quick_edit_patch(
    documents: &FileDocumentService,
    document_id: &str,
    base_version: Option<String>,
    text: &str,
    range: TextRange,
    result: &QuickEditResult,
) -> CoreResult<crate::documents::PatchApplyReport> {
    let patch = quick_edit_to_patch(text, document_id, base_version, range, result)?;
    documents.apply_patch(&patch, None, None)
}

fn simple_diff(original: &str, suggested: &str) -> String {
    if original == suggested {
        return String::new();
    }
    format!("- {original}\n+ {suggested}")
}

fn parse_success_json<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> CoreResult<T> {
    if !response.status().is_success() {
        return Err(template_repo_error(format!(
            "template repository returned {}",
            response.status()
        )));
    }
    response.json::<T>().map_err(template_repo_error)
}

fn validate_non_empty(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }
    Ok(())
}

fn validate_confirmation_entry(entry: &ConfirmationLogEntry) -> CoreResult<()> {
    validate_non_empty("confirmation_id", &entry.confirmation_id)?;
    validate_non_empty("kind", &entry.kind)?;
    validate_non_empty("node_id", &entry.node_id)?;
    Ok(())
}

fn validate_path_component(field: &str, value: &str) -> CoreResult<()> {
    validate_non_empty(field, value)?;
    let path = Path::new(value);
    if path.components().count() != 1
        || value.contains(std::path::MAIN_SEPARATOR)
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
    {
        return Err(CoreError::validation(format!(
            "{field} must be a single safe path component"
        )));
    }
    Ok(())
}

fn absolute_path(path: &Path) -> CoreResult<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(path))
}

fn reject_symlink_root(path: &Path) -> CoreResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CoreError::PermissionDenied {
            action: format!("path:{}", path.display()),
            reason: "workflow root cannot be a symbolic link".to_owned(),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn parse_project_reference(reference: &str) -> CoreResult<(&str, &str)> {
    let trimmed = reference.trim().trim_start_matches('@');
    let separator = match (trimmed.find(':'), trimmed.find('/')) {
        (Some(colon), Some(slash)) => colon.min(slash),
        (Some(colon), None) => colon,
        (None, Some(slash)) => slash,
        (None, None) => {
            return Err(CoreError::validation(
                "project reference must contain ':' or '/'",
            ))
        }
    };
    let (prefix, id) = trimmed.split_at(separator);
    let id = &id[1..];
    validate_non_empty("project reference prefix", prefix)?;
    validate_non_empty("project reference id", id)?;
    Ok((prefix, id))
}

fn parse_node_reference(id: &str) -> CoreResult<(&str, &str, bool)> {
    let (node_id, rest) = id.split_once('/').ok_or_else(|| {
        CoreError::validation("node reference must be @节点/<node>/<输入|输出>/<port>")
    })?;
    let (direction, port_name) = rest
        .split_once('/')
        .ok_or_else(|| CoreError::validation("node reference must include input/output port"))?;
    let is_output = match direction {
        "输出" | "output" => true,
        "输入" | "input" => false,
        other => {
            return Err(CoreError::validation(format!(
                "unsupported node reference direction: {other}"
            )))
        }
    };
    validate_non_empty("node_id", node_id)?;
    validate_non_empty("port_name", port_name)?;
    Ok((node_id, port_name, is_output))
}

fn usize_to_u32(value: usize) -> CoreResult<u32> {
    u32::try_from(value).map_err(|_| CoreError::validation("count exceeds u32"))
}

fn count_words_for_ui(content: &str) -> u64 {
    content
        .split_whitespace()
        .filter(|part| !part.trim().is_empty())
        .count() as u64
}

fn safe_file_stem(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn validate_color(field: &str, value: &str) -> CoreResult<()> {
    let valid_hex = value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|ch| ch.is_ascii_hexdigit());
    if valid_hex {
        Ok(())
    } else {
        Err(CoreError::validation(format!(
            "{field} must be a #RRGGBB color"
        )))
    }
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation(format!("frontend service lock poisoned: {error}"))
}

fn template_repo_error(message: impl std::fmt::Display) -> CoreError {
    CoreError::External {
        service: "template_repository".to_owned(),
        message: message.to_string(),
    }
}

/// 当前毫秒时间戳，供确认项日志构造使用。
pub fn now_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
