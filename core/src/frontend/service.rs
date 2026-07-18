use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::{blocking::Client, Url};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::config::ConfigStore;
use crate::contracts::{
    ensure_path_under_root, ArtifactKind, CoreError, CoreResult, DocumentPatch,
    ExecutionCancellation, ExternalDispatchOutcome, NodeId, NodeInstance, PatchHunk,
    PermissionPolicy, PortEndpoint, PortValue, RunId, TextRange, WorkflowDefinition, WorkflowId,
};
use crate::diagnostics::{BackendDiagnosticsReport, DiagnosticStatus};
use crate::documents::{
    ArtifactWriteRequest, ChapterDocumentEntry, ChapterDocumentIndex, ChapterDocumentKind,
    DocumentReadRequest, DocumentRepository, DocumentWriteRequest, FileDocumentService,
};
use crate::git::GitService;
use crate::llm::{LlmRunRequest, LlmService, LlmServiceConfig};
use crate::providers::{ContentPart, LlmMessage, LlmProvider};
use crate::rag::{FindRequest, FindScope, KnowledgeRetrievalSnapshot, MemoryWritingKnowledgeBase};
use crate::skills::{PromptTemplateVersion, WorkflowManifest, WORKFLOW_MANIFEST_FILE};
use crate::workflow::{RuntimeConfirmationState, WorkflowRunState};

const MAX_TEMPLATE_REPOSITORY_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const TEMPLATE_REPOSITORY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TEMPLATE_REPOSITORY_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const TEMPLATE_REPOSITORY_CANCEL_POLL: Duration = Duration::from_millis(25);
const ALLOW_LOCAL_TEMPLATE_REPOSITORY_ENV: &str = "ARIADNE_ALLOW_LOCAL_TEMPLATE_REPOSITORY";
/// 随应用版本发布的官方内置模板仓库协议；仅此精确 URL 可使用 ariadne scheme。
pub const OFFICIAL_TEMPLATE_REPOSITORY_URL: &str = "ariadne://official-templates/v1";
const OFFICIAL_TEMPLATE_REPOSITORY_SCHEMA_VERSION: u32 = 1;
const OFFICIAL_TEMPLATE_REPOSITORY_PAGE_SIZE: usize = 20;
const MAX_QUICK_EDIT_DIFF_BYTES: usize = 16 * 1024;
const MAX_RUN_LOG_ENTRIES: i64 = 100_000;
const MAX_RESOLVED_CONFIRMATION_ENTRIES: i64 = 100_000;
/// C3：运行日志最长保留 90 天（与条数配额同时生效）。
const MAX_RUN_LOG_AGE_MS: i64 = 90 * 24 * 60 * 60 * 1000;
/// C3：ui_logs.db 软上限约 256 MiB（按 page_count * page_size 估算后裁剪最旧日志）。
const MAX_UI_LOG_DB_BYTES: i64 = 256 * 1024 * 1024;
const MAX_PROJECT_MEMORY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PROJECT_REFERENCE_TEXT_CHARS: usize = 32 * 1024;
const MAX_PROJECT_REFERENCE_MATCH_WINDOWS: usize = 8;
const MAX_PROJECT_REFERENCE_ARTIFACT_BYTES: u64 = 4 * 1024 * 1024;

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
        let bytes = serde_json::to_vec_pretty(entries)?;
        crate::config::store::atomic_write(&self.path, &bytes)?;
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
    ConfigStore::new(project_root).load_or_create()?;
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
    pub fn validate_content(content: &str) -> CoreResult<()> {
        if u64::try_from(content.len()).unwrap_or(u64::MAX) > MAX_PROJECT_MEMORY_BYTES {
            return Err(CoreError::ResourceLimitExceeded {
                resource: "project_memory_bytes".to_owned(),
                reason: format!(
                    "project memory exceeds {} bytes; compact or remove obsolete entries",
                    MAX_PROJECT_MEMORY_BYTES
                ),
            });
        }
        Ok(())
    }

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
        Self::validate_content(content)?;
        crate::config::store::atomic_write(&self.path, content.as_bytes())?;
        Ok(())
    }

    /// 追加项目记忆内容，自动补换行边界。
    ///
    /// D4：读改写后走 `write_all` / `atomic_write`，避免 OpenOptions 就地追加在崩溃窗口留下半行。
    pub fn append(&self, content: &str) -> CoreResult<String> {
        if content.trim().is_empty() {
            return self.read_all();
        }
        let existing = self.read_all()?;
        let mut next = existing;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(content);
        if !content.ends_with('\n') {
            next.push('\n');
        }
        self.write_all(&next)?;
        Ok(next)
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
    /// 所属工作流；旧日志可能缺失，审批时应优先用本字段而非会话内存。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow_id: String,
    /// 所属运行；旧日志可能缺失。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run_id: String,
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

/// SQLite 确认项日志；旧 JSON 文件在首次打开时幂等迁移。
#[derive(Debug, Clone)]
pub struct FileConfirmationLogStore {
    path: PathBuf,
    legacy_path: Option<PathBuf>,
}

impl FileConfirmationLogStore {
    /// 创建文件型确认项日志。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            legacy_path: None,
        }
    }

    /// 使用项目根目录下的统一 `.runtime/ui_logs.db`。
    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        let runtime = project_root.as_ref().join(".runtime");
        Self {
            path: runtime.join("ui_logs.db"),
            legacy_path: Some(runtime.join("confirmation_log.json")),
        }
    }

    /// 读取全部确认项日志。
    pub fn read_all(&self) -> CoreResult<Vec<ConfirmationLogEntry>> {
        let connection = self.open_database()?;
        let mut statement = connection
            .prepare(
                "SELECT entry_json FROM confirmation_logs ORDER BY timestamp_ms, confirmation_id",
            )
            .map_err(sqlite_frontend_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite_frontend_error)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(serde_json::from_str(&row.map_err(sqlite_frontend_error)?)?);
        }
        Ok(entries)
    }

    /// 覆盖写入确认项日志。
    pub fn write_all(&self, entries: &[ConfirmationLogEntry]) -> CoreResult<()> {
        for entry in entries {
            validate_confirmation_entry(entry)?;
        }
        let mut connection = self.open_database()?;
        let transaction = connection.transaction().map_err(sqlite_frontend_error)?;
        transaction
            .execute("DELETE FROM confirmation_logs", [])
            .map_err(sqlite_frontend_error)?;
        for entry in entries {
            upsert_confirmation_entry(&transaction, entry)?;
        }
        prune_confirmation_logs(&transaction)?;
        transaction.commit().map_err(sqlite_frontend_error)
    }

    /// 追加或更新确认项日志；同 id 后写覆盖，保持状态最新。
    pub fn record(&self, entry: ConfirmationLogEntry) -> CoreResult<()> {
        validate_confirmation_entry(&entry)?;
        let connection = self.open_database()?;
        upsert_confirmation_entry(&connection, &entry)?;
        prune_confirmation_logs(&connection)
    }

    /// 通过 `@确认项:<confirmation_id>` 或裸 id 解析持久化引用。
    pub fn resolve_reference(&self, reference: &str) -> CoreResult<ConfirmationReference> {
        let confirmation_id = reference
            .strip_prefix("@确认项:")
            .unwrap_or(reference)
            .trim();
        validate_non_empty("confirmation_id", confirmation_id)?;
        let connection = self.open_database()?;
        let entry_json = connection
            .query_row(
                "SELECT entry_json FROM confirmation_logs WHERE confirmation_id = ?1",
                params![confirmation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_frontend_error)?
            .ok_or_else(|| {
                CoreError::validation(format!("confirmation log not found: {confirmation_id}"))
            })?;
        let entry = serde_json::from_str::<ConfirmationLogEntry>(&entry_json)?;
        Ok(ConfirmationReference {
            confirmation_id: entry.confirmation_id,
            state: entry.state,
            diff: entry.diff,
            summary: entry.summary,
        })
    }

    /// 使用状态索引统计待确认项，侧栏徽标无需加载全部 JSON。
    pub fn pending_count(&self) -> CoreResult<u32> {
        let connection = self.open_database()?;
        let count = connection
            .query_row(
                "SELECT COUNT(*) FROM confirmation_logs WHERE state = 'pending'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sqlite_frontend_error)?;
        u32::try_from(count.max(0))
            .map_err(|_| CoreError::validation("confirmation badge count exceeds u32"))
    }

    fn open_database(&self) -> CoreResult<Connection> {
        open_ui_log_database(&self.path, self.legacy_path.as_deref(), None)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectReferenceTextFragment {
    source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_version: Option<String>,
    start_line: u64,
    end_line: u64,
    text: String,
}

enum ProjectKnowledgeReferenceSource<'a> {
    Memory(&'a MemoryWritingKnowledgeBase),
    Snapshot(&'a KnowledgeRetrievalSnapshot),
}

/// 解析项目空间 AI 的 `@...` 引用。
pub struct ProjectReferenceResolver<'a> {
    confirmations: Option<&'a FileConfirmationLogStore>,
    documents: Option<&'a FileDocumentService>,
    chapter_index: Option<&'a ChapterDocumentIndex>,
    knowledge: Option<ProjectKnowledgeReferenceSource<'a>>,
    runtime: Option<&'a WorkflowRunState>,
    artifacts: BTreeMap<String, ArtifactReferenceEntry>,
    artifact_root: Option<&'a Path>,
    document_root: Option<&'a Path>,
    query: Option<&'a str>,
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
            artifact_root: None,
            document_root: None,
            query: None,
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
        self.knowledge = Some(ProjectKnowledgeReferenceSource::Memory(knowledge));
        self
    }

    pub fn with_knowledge_snapshot(mut self, knowledge: &'a KnowledgeRetrievalSnapshot) -> Self {
        self.knowledge = Some(ProjectKnowledgeReferenceSource::Snapshot(knowledge));
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

    pub fn with_artifact_root(mut self, artifact_root: &'a Path) -> Self {
        self.artifact_root = Some(artifact_root);
        self
    }

    pub fn with_document_root(mut self, document_root: &'a Path) -> Self {
        self.document_root = Some(document_root);
        self
    }

    pub fn with_query(mut self, query: &'a str) -> Self {
        self.query = Some(query);
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
        let requested_path = PathBuf::from(id);
        let path = if requested_path.is_absolute() {
            requested_path
        } else if let Some(document_root) = self.document_root {
            document_root.join(requested_path)
        } else {
            requested_path
        };
        let content = documents.open_document(DocumentReadRequest { path, format: None })?;
        let (fragments, content_truncated) = project_reference_text_fragments(
            &content.metadata.document_id,
            Some(content.metadata.version.clone()),
            &content.content,
            self.query.unwrap_or_default(),
        );
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
                "fragments": fragments,
                "content_truncated": content_truncated,
                "content_chars": content.content.chars().count(),
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
            .as_ref()
            .ok_or_else(|| CoreError::validation("knowledge base is not configured"))?;
        match knowledge {
            ProjectKnowledgeReferenceSource::Memory(knowledge) => {
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
                            include_text: true,
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
                    payload: json!({
                        "title": result.title,
                        "source": result.source,
                        "spans": result.spans,
                        "text": result.text,
                        "metadata": result.metadata,
                    }),
                })
            }
            ProjectKnowledgeReferenceSource::Snapshot(snapshot) => {
                let normalized = id.trim();
                let normalized_lower = normalized.to_lowercase();
                let entry = snapshot
                    .entries
                    .iter()
                    .filter(|entry| {
                        entry.entity_id == normalized
                            || format!("{}:{}", entry.layer, entry.entity_id) == normalized
                            || entry.text.to_lowercase().contains(&normalized_lower)
                    })
                    .min_by_key(|entry| {
                        if entry.entity_id == normalized {
                            0
                        } else if format!("{}:{}", entry.layer, entry.entity_id) == normalized {
                            1
                        } else {
                            2
                        }
                    })
                    .ok_or_else(|| {
                        CoreError::validation(format!("knowledge item not found: {id}"))
                    })?;
                Ok(ProjectReference {
                    reference: reference.to_owned(),
                    kind: ProjectReferenceKind::Knowledge,
                    id: entry.entity_id.clone(),
                    summary: format!("{}: {}", entry.layer, take_first_chars(&entry.text, 240)),
                    payload: json!({
                        "revision": snapshot.revision,
                        "layer": entry.layer,
                        "text": entry.text,
                        "sources": entry.sources,
                        "metadata": entry.metadata,
                    }),
                })
            }
        }
    }

    fn resolve_artifact(&self, reference: &str, id: &str) -> CoreResult<ProjectReference> {
        let artifact = self
            .artifacts
            .get(id)
            .ok_or_else(|| CoreError::validation(format!("artifact not found: {id}")))?;
        let mut payload = serde_json::to_value(artifact)?;
        if let Some(artifact_root) = self.artifact_root {
            if let Some(path) = trusted_artifact_path(artifact_root, &artifact.storage_uri)? {
                if artifact_path_is_textual(&path) {
                    let (text, input_truncated) = read_bounded_utf8_artifact(&path)?;
                    let version = crate::contracts::content_version_for_bytes(text.as_bytes());
                    let (fragments, fragment_truncated) = project_reference_text_fragments(
                        &artifact.artifact_id,
                        Some(version),
                        &text,
                        self.query.unwrap_or_default(),
                    );
                    if let Some(object) = payload.as_object_mut() {
                        object.insert("fragments".to_owned(), serde_json::to_value(fragments)?);
                        object.insert(
                            "content_truncated".to_owned(),
                            Value::Bool(input_truncated || fragment_truncated),
                        );
                    }
                }
            }
        }
        Ok(ProjectReference {
            reference: reference.to_owned(),
            kind: ProjectReferenceKind::Artifact,
            id: artifact.artifact_id.clone(),
            summary: artifact
                .summary
                .clone()
                .unwrap_or_else(|| artifact.storage_uri.clone()),
            payload,
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
    pub workflow_id: Option<WorkflowId>,
    pub run_id: Option<RunId>,
    pub node_id: Option<NodeId>,
    pub query: Option<String>,
    pub after_timestamp_ms: Option<u64>,
    pub after_log_id: Option<String>,
    pub limit: Option<usize>,
    /// 为 true 时从最新记录向旧记录翻页；游标表示当前页最后一条记录。
    pub descending: bool,
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

/// SQLite 运行日志，默认与确认日志共用 `.runtime/ui_logs.db`。
#[derive(Debug, Clone)]
pub struct UiRunLogStore {
    path: PathBuf,
    legacy_path: Option<PathBuf>,
}

impl UiRunLogStore {
    /// 创建运行日志存储。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            legacy_path: None,
        }
    }

    /// 使用项目根目录下默认 `.runtime/ui_logs.db`。
    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        let runtime = project_root.as_ref().join(".runtime");
        Self {
            path: runtime.join("ui_logs.db"),
            legacy_path: Some(runtime.join("run_log.json")),
        }
    }

    /// 读取全部日志。
    pub fn read_all(&self) -> CoreResult<Vec<UiRunLogEntry>> {
        let connection = self.open_database()?;
        let mut statement = connection
            .prepare("SELECT entry_json FROM run_logs ORDER BY timestamp_ms, log_id")
            .map_err(sqlite_frontend_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite_frontend_error)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(serde_json::from_str(&row.map_err(sqlite_frontend_error)?)?);
        }
        Ok(entries)
    }

    /// 覆盖写入日志。
    pub fn write_all(&self, entries: &[UiRunLogEntry]) -> CoreResult<()> {
        let mut connection = self.open_database()?;
        let transaction = connection.transaction().map_err(sqlite_frontend_error)?;
        transaction
            .execute("DELETE FROM run_logs", [])
            .map_err(sqlite_frontend_error)?;
        for entry in entries {
            insert_run_log(&transaction, entry)?;
        }
        prune_run_logs(&transaction)?;
        transaction.commit().map_err(sqlite_frontend_error)
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
        let connection = self.open_database()?;
        insert_run_log(&connection, &entry)?;
        prune_run_logs(&connection)?;
        Ok(toast)
    }

    /// 按过滤条件查询日志。
    pub fn query(&self, filter: UiRunLogFilter) -> CoreResult<Vec<UiRunLogEntry>> {
        let connection = self.open_database()?;
        let kind = filter.kind.map(run_log_kind_str);
        let level = filter.level.map(run_log_level_str);
        let workflow_id = filter.workflow_id.as_ref().map(WorkflowId::as_str);
        let run_id = filter.run_id.as_ref().map(RunId::as_str);
        let node_id = filter.node_id.as_ref().map(NodeId::as_str);
        let query = filter
            .query
            .map(|value| format!("%{}%", value.to_lowercase()));
        let after_timestamp = filter
            .after_timestamp_ms
            .map(i64::try_from)
            .transpose()
            .map_err(|_| CoreError::validation("run log cursor timestamp exceeds SQLite i64"))?;
        let after_log_id = filter.after_log_id.as_deref().unwrap_or("");
        let limit = filter
            .limit
            .map(i64::try_from)
            .transpose()
            .map_err(|_| CoreError::validation("run log query limit exceeds SQLite i64"))?
            .unwrap_or(i64::MAX);
        let (cursor_comparison, order) = if filter.descending {
            ("<", "DESC")
        } else {
            (">", "ASC")
        };
        let sql = format!(
            "SELECT entry_json FROM run_logs
             WHERE (?1 IS NULL OR kind = ?1)
               AND (?2 IS NULL OR level = ?2)
               AND (?3 IS NULL OR workflow_id = ?3)
               AND (?4 IS NULL OR run_id = ?4)
               AND (?5 IS NULL OR node_id = ?5)
               AND (?6 IS NULL OR lower(message) LIKE ?6)
               AND (?7 IS NULL OR timestamp_ms {cursor_comparison} ?7
                    OR (timestamp_ms = ?7 AND log_id {cursor_comparison} ?8))
             ORDER BY timestamp_ms {order}, log_id {order}
             LIMIT ?9"
        );
        let mut statement = connection.prepare(&sql).map_err(sqlite_frontend_error)?;
        let rows = statement
            .query_map(
                params![
                    kind,
                    level,
                    workflow_id,
                    run_id,
                    node_id,
                    query,
                    after_timestamp,
                    after_log_id,
                    limit
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(sqlite_frontend_error)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(serde_json::from_str(&row.map_err(sqlite_frontend_error)?)?);
        }
        Ok(entries)
    }

    /// 标记全部日志已读。
    pub fn mark_all_read(&self) -> CoreResult<()> {
        self.mark_read(UiRunLogFilter::default()).map(|_| ())
    }

    /// 仅把与当前筛选匹配的日志标为已读，并返回实际更新条数。
    pub fn mark_read(&self, filter: UiRunLogFilter) -> CoreResult<usize> {
        let connection = self.open_database()?;
        let kind = filter.kind.map(run_log_kind_str);
        let level = filter.level.map(run_log_level_str);
        let workflow_id = filter.workflow_id.as_ref().map(WorkflowId::as_str);
        let run_id = filter.run_id.as_ref().map(RunId::as_str);
        let node_id = filter.node_id.as_ref().map(NodeId::as_str);
        let query = filter
            .query
            .map(|value| format!("%{}%", value.to_lowercase()));
        connection
            .execute(
                "UPDATE run_logs
                 SET unread = 0,
                     entry_json = json_set(entry_json, '$.unread', json('false'))
                 WHERE unread = 1
                   AND (?1 IS NULL OR kind = ?1)
                   AND (?2 IS NULL OR level = ?2)
                   AND (?3 IS NULL OR workflow_id = ?3)
                   AND (?4 IS NULL OR run_id = ?4)
                   AND (?5 IS NULL OR node_id = ?5)
                   AND (?6 IS NULL OR lower(message) LIKE ?6)",
                params![kind, level, workflow_id, run_id, node_id, query],
            )
            .map_err(sqlite_frontend_error)
    }

    /// 汇总侧栏徽标。
    pub fn badge_counts(
        &self,
        confirmation_log: Option<&FileConfirmationLogStore>,
        diagnostics: Option<&BackendDiagnosticsReport>,
    ) -> CoreResult<SidebarBadgeCounts> {
        let connection = self.open_database()?;
        let run_logs = connection
            .query_row(
                "SELECT COUNT(*) FROM run_logs WHERE unread = 1 AND level != 'info'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sqlite_frontend_error)?;
        let confirmations = confirmation_log
            .map(|store| store.pending_count().unwrap_or(0))
            .unwrap_or(0);
        let diagnostics = diagnostics
            .map(|report| usize::from(report.status != DiagnosticStatus::Healthy))
            .unwrap_or(0);
        Ok(SidebarBadgeCounts {
            run_logs: u32::try_from(run_logs.max(0))
                .map_err(|_| CoreError::validation("run log badge count exceeds u32"))?,
            confirmations,
            diagnostics: usize_to_u32(diagnostics)?,
        })
    }

    fn open_database(&self) -> CoreResult<Connection> {
        open_ui_log_database(&self.path, None, self.legacy_path.as_deref())
    }
}

fn open_ui_log_database(
    path: &Path,
    confirmation_legacy: Option<&Path>,
    run_legacy: Option<&Path>,
) -> CoreResult<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(path).map_err(sqlite_frontend_error)?;
    connection
        .execute_batch(
            "PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS ui_log_migrations (
                 name TEXT PRIMARY KEY,
                 applied_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS confirmation_logs (
                 confirmation_id TEXT PRIMARY KEY,
                 timestamp_ms INTEGER NOT NULL,
                 state TEXT NOT NULL,
                 entry_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_confirmation_logs_state
                 ON confirmation_logs(state, timestamp_ms);
             CREATE TABLE IF NOT EXISTS run_logs (
                 log_id TEXT PRIMARY KEY,
                 timestamp_ms INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 level TEXT NOT NULL,
                 message TEXT NOT NULL,
                 workflow_id TEXT,
                 run_id TEXT,
                 node_id TEXT,
                 unread INTEGER NOT NULL,
                 entry_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_run_logs_filter
                 ON run_logs(workflow_id, run_id, node_id, kind, level, timestamp_ms);
             CREATE INDEX IF NOT EXISTS idx_run_logs_unread
                 ON run_logs(unread, level);
            ",
        )
        .map_err(sqlite_frontend_error)?;
    if let Some(legacy) = confirmation_legacy {
        migrate_confirmation_log(&mut connection, legacy)?;
    }
    if let Some(legacy) = run_legacy {
        migrate_run_log(&mut connection, legacy)?;
    }
    Ok(connection)
}

fn migrate_confirmation_log(connection: &mut Connection, legacy: &Path) -> CoreResult<()> {
    if !legacy.exists() || ui_log_migration_applied(connection, "confirmation_log_json_v1")? {
        return Ok(());
    }
    let entries =
        serde_json::from_str::<Vec<ConfirmationLogEntry>>(&std::fs::read_to_string(legacy)?)?;
    let transaction = connection.transaction().map_err(sqlite_frontend_error)?;
    for entry in &entries {
        validate_confirmation_entry(entry)?;
        upsert_confirmation_entry(&transaction, entry)?;
    }
    prune_confirmation_logs(&transaction)?;
    record_ui_log_migration(&transaction, "confirmation_log_json_v1")?;
    transaction.commit().map_err(sqlite_frontend_error)
}

fn migrate_run_log(connection: &mut Connection, legacy: &Path) -> CoreResult<()> {
    if !legacy.exists() || ui_log_migration_applied(connection, "run_log_json_v1")? {
        return Ok(());
    }
    let entries = serde_json::from_str::<Vec<UiRunLogEntry>>(&std::fs::read_to_string(legacy)?)?;
    let transaction = connection.transaction().map_err(sqlite_frontend_error)?;
    for entry in &entries {
        insert_run_log(&transaction, entry)?;
    }
    prune_run_logs(&transaction)?;
    record_ui_log_migration(&transaction, "run_log_json_v1")?;
    transaction.commit().map_err(sqlite_frontend_error)
}

fn ui_log_migration_applied(connection: &Connection, name: &str) -> CoreResult<bool> {
    connection
        .query_row(
            "SELECT 1 FROM ui_log_migrations WHERE name = ?1",
            params![name],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(sqlite_frontend_error)
}

fn record_ui_log_migration(connection: &Connection, name: &str) -> CoreResult<()> {
    connection
        .execute(
            "INSERT OR IGNORE INTO ui_log_migrations(name, applied_at_ms) VALUES(?1, ?2)",
            params![name, i64::try_from(now_timestamp_ms()).unwrap_or(i64::MAX)],
        )
        .map_err(sqlite_frontend_error)?;
    Ok(())
}

fn upsert_confirmation_entry(
    connection: &Connection,
    entry: &ConfirmationLogEntry,
) -> CoreResult<()> {
    let timestamp = i64::try_from(entry.timestamp_ms)
        .map_err(|_| CoreError::validation("confirmation timestamp exceeds SQLite i64"))?;
    connection
        .execute(
            "INSERT INTO confirmation_logs(confirmation_id, timestamp_ms, state, entry_json)
             VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(confirmation_id) DO UPDATE SET
                 timestamp_ms=excluded.timestamp_ms, state=excluded.state, entry_json=excluded.entry_json",
            params![entry.confirmation_id, timestamp, confirmation_state_str(entry.state), serde_json::to_string(entry)?],
        )
        .map_err(sqlite_frontend_error)?;
    Ok(())
}

fn insert_run_log(connection: &Connection, entry: &UiRunLogEntry) -> CoreResult<()> {
    let timestamp = i64::try_from(entry.timestamp_ms)
        .map_err(|_| CoreError::validation("run log timestamp exceeds SQLite i64"))?;
    connection
        .execute(
            "INSERT INTO run_logs(log_id, timestamp_ms, kind, level, message, workflow_id, run_id, node_id, unread, entry_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(log_id) DO UPDATE SET
                 timestamp_ms=excluded.timestamp_ms, kind=excluded.kind, level=excluded.level,
                 message=excluded.message, workflow_id=excluded.workflow_id, run_id=excluded.run_id,
                 node_id=excluded.node_id, unread=excluded.unread, entry_json=excluded.entry_json",
            params![
                entry.log_id,
                timestamp,
                run_log_kind_str(entry.kind),
                run_log_level_str(entry.level),
                entry.message,
                entry.workflow_id.as_ref().map(WorkflowId::as_str),
                entry.run_id.as_ref().map(RunId::as_str),
                entry.node_id.as_ref().map(NodeId::as_str),
                i64::from(entry.unread),
                serde_json::to_string(entry)?,
            ],
        )
        .map_err(sqlite_frontend_error)?;
    Ok(())
}

fn prune_run_logs(connection: &Connection) -> CoreResult<()> {
    // 条数配额：保留最新 N 条。
    connection
        .execute(
            "DELETE FROM run_logs
             WHERE log_id IN (
                 SELECT log_id FROM run_logs
                 ORDER BY timestamp_ms DESC, log_id DESC
                 LIMIT -1 OFFSET ?1
             )",
            params![MAX_RUN_LOG_ENTRIES],
        )
        .map_err(sqlite_frontend_error)?;
    // 时间配额：删除超过保留窗口的最旧条目。
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let cutoff = now_ms.saturating_sub(MAX_RUN_LOG_AGE_MS);
    connection
        .execute(
            "DELETE FROM run_logs WHERE timestamp_ms < ?1",
            params![cutoff],
        )
        .map_err(sqlite_frontend_error)?;
    // 磁盘软配额：超限时继续删最旧，直到估算体积回到上限内。
    prune_run_logs_to_disk_budget(connection)?;
    Ok(())
}

fn prune_run_logs_to_disk_budget(connection: &Connection) -> CoreResult<()> {
    let page_count: i64 = connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .map_err(sqlite_frontend_error)?;
    let page_size: i64 = connection
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .map_err(sqlite_frontend_error)?;
    let mut estimated = page_count.saturating_mul(page_size);
    let mut guard = 0;
    while estimated > MAX_UI_LOG_DB_BYTES && guard < 32 {
        let deleted = connection
            .execute(
                "DELETE FROM run_logs
                 WHERE log_id IN (
                     SELECT log_id FROM run_logs
                     ORDER BY timestamp_ms ASC, log_id ASC
                     LIMIT 1000
                 )",
                [],
            )
            .map_err(sqlite_frontend_error)?;
        if deleted == 0 {
            break;
        }
        let page_count: i64 = connection
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .map_err(sqlite_frontend_error)?;
        estimated = page_count.saturating_mul(page_size);
        guard += 1;
    }
    Ok(())
}

fn prune_confirmation_logs(connection: &Connection) -> CoreResult<()> {
    // Pending 项属于待处理工作，不能因为历史配额被自动删除；只限制已解决历史。
    connection
        .execute(
            "DELETE FROM confirmation_logs
             WHERE confirmation_id IN (
                 SELECT confirmation_id FROM confirmation_logs
                 WHERE state != 'pending'
                 ORDER BY timestamp_ms DESC, confirmation_id DESC
                 LIMIT -1 OFFSET ?1
             )",
            params![MAX_RESOLVED_CONFIRMATION_ENTRIES],
        )
        .map_err(sqlite_frontend_error)?;
    Ok(())
}

fn run_log_kind_str(kind: UiRunLogKind) -> &'static str {
    match kind {
        UiRunLogKind::Node => "node",
        UiRunLogKind::Tool => "tool",
        UiRunLogKind::Provider => "provider",
        UiRunLogKind::Cost => "cost",
        UiRunLogKind::Confirmation => "confirmation",
        UiRunLogKind::Error => "error",
        UiRunLogKind::Diagnostic => "diagnostic",
    }
}

fn run_log_level_str(level: UiRunLogLevel) -> &'static str {
    match level {
        UiRunLogLevel::Info => "info",
        UiRunLogLevel::Warning => "warning",
        UiRunLogLevel::Error => "error",
    }
}

fn confirmation_state_str(state: ConfirmationLogState) -> &'static str {
    match state {
        ConfirmationLogState::Pending => "pending",
        ConfirmationLogState::Approved => "approved",
        ConfirmationLogState::Rejected => "rejected",
        ConfirmationLogState::AutoAudited => "auto_audited",
    }
}

fn sqlite_frontend_error(error: rusqlite::Error) -> CoreError {
    CoreError::External {
        service: "ui_log_sqlite".to_owned(),
        message: error.to_string(),
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
    pub chapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
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
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_ref: Option<crate::contracts::SourceSpan>,
}

/// 章节合并导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ChapterExportFormat {
    #[default]
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
    chapter_stage: &BTreeMap<String, String>,
    planning_root: impl AsRef<Path>,
) -> CoreResult<WorksTreeNode> {
    index.validate()?;
    let planning_root = planning_root.as_ref();
    let chapter_entries = index.chapter_bodies();
    let known_chapter_ids = chapter_entries
        .iter()
        .map(|entry| entry.chapter_id.as_str())
        .collect::<BTreeSet<_>>();
    for (chapter_id, stage_id) in chapter_stage {
        if chapter_id.trim().is_empty() || stage_id.trim().is_empty() {
            return Err(CoreError::validation(
                "chapter-stage relation contains an empty id",
            ));
        }
        if !known_chapter_ids.contains(chapter_id.as_str()) {
            return Err(CoreError::validation(format!(
                "chapter-stage relation references missing chapter index entry: {chapter_id}"
            )));
        }
    }

    let mut stages = BTreeMap::<Option<String>, Vec<&ChapterDocumentEntry>>::new();
    for entry in chapter_entries {
        let stage_id = chapter_stage.get(&entry.chapter_id).cloned();
        stages.entry(stage_id).or_default().push(entry);
    }
    let children = stages
        .into_iter()
        .map(|(stage_id, entries)| WorksTreeNode {
            node_id: stage_id
                .as_deref()
                .map(|stage_id| format!("stage:{stage_id}"))
                .unwrap_or_else(|| "stage:__unassigned__".to_owned()),
            kind: WorksTreeNodeKind::StageOutline,
            title: stage_id
                .clone()
                .unwrap_or_else(|| "ui.works.unassigned_stage".to_owned()),
            path: stage_id
                .as_deref()
                .map(|stage_id| {
                    planning_root
                        .join("stages")
                        .join(format!("{}.md", safe_file_stem(stage_id)))
                })
                .unwrap_or_default(),
            chapter_id: None,
            document_id: None,
            stage_id: stage_id.clone(),
            outline_ref: None,
            children: entries
                .into_iter()
                .map(|entry| WorksTreeNode {
                    node_id: format!("chapter:{}", entry.chapter_id),
                    kind: WorksTreeNodeKind::Chapter,
                    title: entry.title.clone(),
                    path: entry.path.clone(),
                    chapter_id: Some(entry.chapter_id.clone()),
                    document_id: Some(entry.document_id.clone()),
                    stage_id: stage_id.clone(),
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
        chapter_id: None,
        document_id: None,
        stage_id: None,
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
    let write_request = DocumentWriteRequest {
        path: request.target_path.clone(),
        content: source.content,
        format: None,
        base_version: None,
    };
    let report = if request.overwrite {
        documents.save_document(write_request)
    } else {
        documents.create_document(write_request)
    }?;
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
        operation_id: None,
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

pub(crate) fn render_chapters_markdown(chapters: &[(String, String)]) -> String {
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

pub(crate) fn render_chapters_epub(chapters: &[(String, String)]) -> CoreResult<Vec<u8>> {
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

pub(crate) fn render_chapters_pdf(chapters: &[(String, String)]) -> Vec<u8> {
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
    /// 主题自定义三色·昼（主底 / 表面 / 强调）。空字符串 = 使用主题预设 swatch。
    #[serde(default)]
    pub theme_main_color: String,
    #[serde(default)]
    pub theme_surface_color: String,
    #[serde(default)]
    pub theme_brand_color: String,
    /// 主题自定义三色·夜（跟随系统时使用）。
    #[serde(default)]
    pub theme_main_color_dark: String,
    #[serde(default)]
    pub theme_surface_color_dark: String,
    #[serde(default)]
    pub theme_brand_color_dark: String,
    /// 自定义颜色是否跟随系统明暗分别应用昼/夜。
    #[serde(default = "default_true")]
    pub theme_follow_system_colors: bool,
    pub project_panel_visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_panel_position: Option<(i32, i32)>,
    #[serde(default)]
    pub panel_states: BTreeMap<String, bool>,
    /// 关闭非必要位移/缩放过渡，仅保留即时状态反馈。
    #[serde(default)]
    pub reduce_motion: bool,
    pub onboarding_seen: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: "system".to_owned(),
            git_auto_color: "#8a8f98".to_owned(),
            git_manual_color: "#f59e0b".to_owned(),
            theme_main_color: String::new(),
            theme_surface_color: String::new(),
            theme_brand_color: String::new(),
            theme_main_color_dark: String::new(),
            theme_surface_color_dark: String::new(),
            theme_brand_color_dark: String::new(),
            theme_follow_system_colors: true,
            project_panel_visible: true,
            project_panel_position: None,
            panel_states: BTreeMap::new(),
            reduce_motion: false,
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

    /// 使用项目根目录下默认 `.runtime/ui_preferences.json`（旧路径，仅迁移兼容）。
    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".runtime/ui_preferences.json"))
    }

    /// 应用级（与项目分离）UI 偏好：落在 app_state_root/ui_preferences.json。
    pub fn default_for_app(app_state_root: impl AsRef<Path>) -> Self {
        Self::new(app_state_root.as_ref().join("ui_preferences.json"))
    }

    /// 先读应用级；若无则尝试从项目级迁移一次。
    pub fn read_global_or_migrate(
        app_state_root: impl AsRef<Path>,
        project_root: Option<&Path>,
    ) -> CoreResult<UiPreferences> {
        let global = Self::default_for_app(app_state_root.as_ref());
        if global.path.is_file() {
            return global.read();
        }
        if let Some(root) = project_root {
            let project = Self::default_for_project(root);
            if project.path.is_file() {
                let prefs = project.read()?;
                let _ = global.write(&prefs);
                return Ok(prefs);
            }
        }
        global.read()
    }

    /// 存储文件路径（测试 / 迁移用）。
    pub fn path(&self) -> &Path {
        &self.path
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
        validate_optional_color("theme_main_color", &preferences.theme_main_color)?;
        validate_optional_color("theme_surface_color", &preferences.theme_surface_color)?;
        validate_optional_color("theme_brand_color", &preferences.theme_brand_color)?;
        validate_optional_color("theme_main_color_dark", &preferences.theme_main_color_dark)?;
        validate_optional_color(
            "theme_surface_color_dark",
            &preferences.theme_surface_color_dark,
        )?;
        validate_optional_color(
            "theme_brand_color_dark",
            &preferences.theme_brand_color_dark,
        )?;
        let bytes = serde_json::to_string_pretty(preferences)?;
        crate::config::store::atomic_write(&self.path, bytes.as_bytes())?;
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

#[derive(Debug, Clone, Copy)]
struct OfficialTemplateRecord {
    id: &'static str,
    name_key: &'static str,
    tags: &'static [&'static str],
    search_terms: &'static [&'static str],
    manifest_bytes: &'static [u8],
    manifest_sha256: &'static str,
}

const OFFICIAL_TEMPLATE_RECORDS: &[OfficialTemplateRecord] = &[
    OfficialTemplateRecord {
        id: "official-novel-starter",
        name_key: "ui.template.builtin.novel_starter.name",
        tags: &["ui.template.tag.novel", "ui.template.tag.outline"],
        search_terms: &["小说", "写小说", "大纲", "novel", "outline", "小説"],
        manifest_bytes: include_bytes!("../../resources/template_repository/v1/novel_starter.json"),
        manifest_sha256: "5493a7ec4d6a943aecf245b2cef37c526a0a6b7d82ae065884d301fce0c61943",
    },
    OfficialTemplateRecord {
        id: "official-worldbuilding",
        name_key: "ui.template.builtin.worldbuilding.name",
        tags: &["ui.template.tag.worldbuilding"],
        search_terms: &[
            "世界观",
            "世界观构建",
            "worldbuilding",
            "world building",
            "世界観",
        ],
        manifest_bytes: include_bytes!("../../resources/template_repository/v1/worldbuilding.json"),
        manifest_sha256: "0c1295c7bf3efc394aa6d35c5258dee868c94c95cca4b8ff9bd8e788c0ee09ab",
    },
    OfficialTemplateRecord {
        id: "official-review-polish",
        name_key: "ui.template.builtin.review_polish.name",
        tags: &["ui.template.tag.review"],
        search_terms: &["审查", "润色", "review", "polish", "レビュー", "推敲"],
        manifest_bytes: include_bytes!("../../resources/template_repository/v1/review_polish.json"),
        manifest_sha256: "fa7a8d55ca34ff50ce1c9b0fea3387eae5249375cd99e5500c534b9b3fdc961d",
    },
];

/// 模板仓库客户端：支持随应用发布的官方内置目录与自定义远端目录。
#[derive(Debug, Clone)]
pub struct TemplateRepositoryClient {
    base_url: String,
    client: Client,
    cancellation: ExecutionCancellation,
}

impl TemplateRepositoryClient {
    /// 创建模板仓库客户端。
    pub fn new(base_url: impl Into<String>) -> CoreResult<Self> {
        Self::new_with_cancellation(base_url, ExecutionCancellation::new())
    }

    pub fn new_with_cancellation(
        base_url: impl Into<String>,
        cancellation: ExecutionCancellation,
    ) -> CoreResult<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        validate_template_repository_base_url_with_policy(
            &base_url,
            std::env::var_os(ALLOW_LOCAL_TEMPLATE_REPOSITORY_ENV).is_some(),
            Some(&cancellation),
        )?;
        Ok(Self {
            base_url,
            client: Client::builder()
                .connect_timeout(TEMPLATE_REPOSITORY_CONNECT_TIMEOUT)
                .timeout(TEMPLATE_REPOSITORY_REQUEST_TIMEOUT)
                .build()
                .map_err(template_repo_error)?,
            cancellation,
        })
    }

    /// 绑定 IPC 请求的取消令牌；网络线程最多继续到请求超时，不会在取消后写项目文件。
    pub fn with_cancellation(mut self, cancellation: ExecutionCancellation) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// 搜索模板。
    pub fn search(
        &self,
        query: &str,
        tags: &[String],
        page: u32,
    ) -> CoreResult<Vec<TemplateSummary>> {
        self.validate_base_url()?;
        if self.is_official_repository() {
            self.cancellation.check()?;
            return official_template_search(query, tags, page);
        }
        self.send_json(
            self.client
                .get(format!("{}/templates/search", self.base_url))
                .query(&[("query", query), ("page", &page.to_string())])
                .query(&[("tags", &tags.join(","))]),
        )
    }

    /// 获取模板详情。
    pub fn detail(&self, id: &str) -> CoreResult<TemplateDetail> {
        validate_non_empty("template id", id)?;
        self.validate_base_url()?;
        if self.is_official_repository() {
            self.cancellation.check()?;
            return official_template_detail(id);
        }
        self.send_json(self.client.get(format!("{}/templates/{id}", self.base_url)))
    }

    /// 下载模板 manifest。
    pub fn download(&self, id: &str) -> CoreResult<Value> {
        validate_non_empty("template id", id)?;
        self.validate_base_url()?;
        if self.is_official_repository() {
            self.cancellation.check()?;
            return official_template_manifest_value(id);
        }
        self.send_json(
            self.client
                .get(format!("{}/templates/{id}/download", self.base_url)),
        )
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

    fn send_json<T>(&self, request: reqwest::blocking::RequestBuilder) -> CoreResult<T>
    where
        T: serde::de::DeserializeOwned + Send + 'static,
    {
        self.cancellation.check()?;
        let blocking_permit = crate::contracts::acquire_detached_blocking_task_permit()?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("ariadne-template-http".to_owned())
            .spawn(move || {
                let _blocking_permit = blocking_permit;
                let result = request
                    .send()
                    .map_err(template_repo_error)
                    .and_then(parse_success_json);
                let _ = sender.send(result);
            })
            .map_err(CoreError::from)?;

        loop {
            if self.cancellation.is_cancelled() {
                return Err(CoreError::external_cancelled(
                    "template_repository",
                    ExternalDispatchOutcome::DispatchedUnknown,
                ));
            }
            match receiver.recv_timeout(TEMPLATE_REPOSITORY_CANCEL_POLL) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(template_repo_error(
                        "template repository request worker disconnected",
                    ));
                }
            }
        }
    }

    fn validate_base_url(&self) -> CoreResult<()> {
        validate_template_repository_base_url_with_policy(
            &self.base_url,
            std::env::var_os(ALLOW_LOCAL_TEMPLATE_REPOSITORY_ENV).is_some(),
            Some(&self.cancellation),
        )
    }

    fn is_official_repository(&self) -> bool {
        self.base_url == OFFICIAL_TEMPLATE_REPOSITORY_URL
    }
}

fn official_template_search(
    query: &str,
    tags: &[String],
    page: u32,
) -> CoreResult<Vec<TemplateSummary>> {
    let query = query.trim().to_lowercase();
    let requested_tags = tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    let offset = usize::try_from(page)
        .ok()
        .and_then(|value| value.checked_mul(OFFICIAL_TEMPLATE_REPOSITORY_PAGE_SIZE))
        .ok_or_else(|| CoreError::validation("template repository page is too large"))?;
    let records = OFFICIAL_TEMPLATE_RECORDS
        .iter()
        .filter(|record| {
            let matches_query = query.is_empty()
                || record.id.to_lowercase().contains(&query)
                || record.name_key.to_lowercase().contains(&query)
                || record
                    .search_terms
                    .iter()
                    .any(|term| term.to_lowercase().contains(&query));
            let matches_tags = requested_tags.iter().all(|requested| {
                record
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase() == *requested)
            });
            matches_query && matches_tags
        })
        .skip(offset)
        .take(OFFICIAL_TEMPLATE_REPOSITORY_PAGE_SIZE)
        .map(|record| TemplateSummary {
            id: record.id.to_owned(),
            name: record.name_key.to_owned(),
            tags: record.tags.iter().map(|tag| (*tag).to_owned()).collect(),
            requires_permissions: false,
        })
        .collect();
    Ok(records)
}

fn official_template_detail(id: &str) -> CoreResult<TemplateDetail> {
    let record = official_template_record(id)?;
    let manifest = official_template_manifest(record)?;
    Ok(TemplateDetail {
        id: record.id.to_owned(),
        name: record.name_key.to_owned(),
        version: manifest.version.clone(),
        manifest: serde_json::to_value(manifest)?,
        requires_permissions: false,
    })
}

fn official_template_manifest_value(id: &str) -> CoreResult<Value> {
    serde_json::to_value(official_template_manifest(official_template_record(id)?)?)
        .map_err(Into::into)
}

fn official_template_record(id: &str) -> CoreResult<&'static OfficialTemplateRecord> {
    OFFICIAL_TEMPLATE_RECORDS
        .iter()
        .find(|record| record.id == id)
        .ok_or_else(|| CoreError::RegistryMissing {
            registry: "official_template_repository",
            key: id.to_owned(),
        })
}

fn official_template_manifest(record: &OfficialTemplateRecord) -> CoreResult<WorkflowManifest> {
    let actual_sha256 = format!("{:x}", Sha256::digest(record.manifest_bytes));
    if actual_sha256 != record.manifest_sha256 {
        return Err(CoreError::validation(format!(
            "official template digest mismatch for {}",
            record.id
        )));
    }
    let manifest: WorkflowManifest = serde_json::from_slice(record.manifest_bytes)?;
    manifest.validate()?;
    if manifest.workflow_id != record.id || manifest.workflow.id.as_str() != record.id {
        return Err(CoreError::validation(format!(
            "official template identity mismatch for {}",
            record.id
        )));
    }
    Ok(manifest)
}

/// 将下载到的 workflow manifest 写入本地 `workflows/<id>/workflow.json`。
pub fn install_workflow_template_manifest(
    manifest_value: Value,
    workflows_root: impl AsRef<Path>,
    requires_permissions: bool,
) -> CoreResult<TemplateInstallReport> {
    let manifest: WorkflowManifest = serde_json::from_value(manifest_value)?;
    manifest.validate()?;
    if let Some(minimum_version) = manifest.minimum_ariadne_version.as_deref() {
        let required = PromptTemplateVersion::parse(minimum_version)?;
        let current = PromptTemplateVersion::parse(crate::PRODUCT_VERSION)?;
        if current < required {
            return Err(CoreError::validation(format!(
                "workflow template {} requires Ariadne {}, current version is {}",
                manifest.workflow_id, required, current
            )));
        }
    }
    validate_path_component("workflow_id", &manifest.workflow_id)?;
    let workflows_root = absolute_path(workflows_root.as_ref())?;
    reject_symlink_root(&workflows_root)?;
    let manifest_dir = workflows_root.join(&manifest.workflow_id);
    let manifest_path = manifest_dir.join(WORKFLOW_MANIFEST_FILE);
    ensure_path_under_root(&workflows_root, &manifest_path)?;
    std::fs::create_dir_all(&manifest_dir)?;
    let content = serde_json::to_string_pretty(&manifest)?;
    crate::config::store::atomic_write(&manifest_path, content.as_bytes())?;
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
        self.quick_edit_with_cancellation(
            selected_text,
            instruction,
            context_ref,
            &crate::contracts::CancellationToken::new(),
        )
    }

    pub fn quick_edit_with_cancellation(
        &self,
        selected_text: &str,
        instruction: &str,
        context_ref: Option<&str>,
        cancellation: &crate::contracts::CancellationToken,
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
                dispatch_authorization: Default::default(),
            },
            cancellation,
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

/// U52：行级 unified diff；相同上下文折叠；整 diff 强制 ≤ MAX_QUICK_EDIT_DIFF_BYTES。
/// 删除/新增两侧各预留约一半预算，避免单侧超长行吃光上限后看不到另一侧。
fn simple_diff(original: &str, suggested: &str) -> String {
    if original == suggested {
        return String::new();
    }
    let old_lines: Vec<&str> = original.split('\n').collect();
    let new_lines: Vec<&str> = suggested.split('\n').collect();
    // 整段替换（常见长章）时两侧各半预算，保证 `-` 与 `+` 都出现。
    if old_lines.len() == 1 && new_lines.len() == 1 {
        let half = MAX_QUICK_EDIT_DIFF_BYTES / 2;
        let (old_part, old_cut) = utf8_prefix(old_lines[0], half.saturating_sub(4));
        let (new_part, new_cut) = utf8_prefix(new_lines[0], half.saturating_sub(4));
        let mut out = String::with_capacity(MAX_QUICK_EDIT_DIFF_BYTES);
        out.push_str("- ");
        out.push_str(old_part);
        if old_cut {
            out.push_str("...");
        }
        out.push_str("\n+ ");
        out.push_str(new_part);
        if new_cut {
            out.push_str("...");
        }
        if out.len() > MAX_QUICK_EDIT_DIFF_BYTES {
            let (prefix, _) = utf8_prefix(&out, MAX_QUICK_EDIT_DIFF_BYTES);
            return prefix.to_owned();
        }
        return out;
    }

    let mut out = String::with_capacity(MAX_QUICK_EDIT_DIFF_BYTES.min(4096));
    let mut i = 0usize;
    let mut j = 0usize;
    while (i < old_lines.len() || j < new_lines.len()) && out.len() < MAX_QUICK_EDIT_DIFF_BYTES {
        let remaining = MAX_QUICK_EDIT_DIFF_BYTES.saturating_sub(out.len());
        if remaining < 8 {
            break;
        }
        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            let mut run = 1usize;
            while i + run < old_lines.len()
                && j + run < new_lines.len()
                && old_lines[i + run] == new_lines[j + run]
            {
                run += 1;
            }
            let line = if run == 1 {
                format!("  {}\n", old_lines[i])
            } else {
                format!("  ... ({run} unchanged lines)\n")
            };
            let (chunk, _) = utf8_prefix(&line, remaining);
            out.push_str(chunk);
            i += run;
            j += run;
            continue;
        }
        if i < old_lines.len()
            && (j >= new_lines.len()
                || !new_lines[j..]
                    .iter()
                    .take(8)
                    .any(|line| *line == old_lines[i]))
        {
            let line = format!("- {}\n", old_lines[i]);
            let (chunk, cut) = utf8_prefix(&line, remaining);
            out.push_str(chunk);
            if cut {
                break;
            }
            i += 1;
            continue;
        }
        if j < new_lines.len() {
            let line = format!("+ {}\n", new_lines[j]);
            let (chunk, cut) = utf8_prefix(&line, remaining);
            out.push_str(chunk);
            if cut {
                break;
            }
            j += 1;
            continue;
        }
        if i < old_lines.len() {
            let line = format!("- {}\n", old_lines[i]);
            let (chunk, cut) = utf8_prefix(&line, remaining);
            out.push_str(chunk);
            if cut {
                break;
            }
            i += 1;
        }
    }
    if out.len() > MAX_QUICK_EDIT_DIFF_BYTES {
        let (prefix, _) = utf8_prefix(&out, MAX_QUICK_EDIT_DIFF_BYTES);
        return prefix.to_owned();
    }
    out
}

fn utf8_prefix(value: &str, max_bytes: usize) -> (&str, bool) {
    if value.len() <= max_bytes {
        return (value, false);
    }
    let mut boundary = max_bytes.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (&value[..boundary], true)
}

pub fn validate_template_repository_base_url(base_url: &str) -> CoreResult<()> {
    validate_template_repository_base_url_with_policy(
        base_url,
        std::env::var_os(ALLOW_LOCAL_TEMPLATE_REPOSITORY_ENV).is_some(),
        None,
    )
}

fn validate_template_repository_base_url_with_policy(
    base_url: &str,
    allow_local: bool,
    cancellation: Option<&ExecutionCancellation>,
) -> CoreResult<()> {
    let trimmed = base_url.trim();
    validate_non_empty("template repository base_url", trimmed)?;
    if trimmed == OFFICIAL_TEMPLATE_REPOSITORY_URL {
        return Ok(());
    }
    let url = Url::parse(trimmed).map_err(|error| {
        CoreError::validation(format!(
            "template repository base_url must be a valid URL: {error}"
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(CoreError::validation(format!(
            "template repository base_url must use the official v{} URL or http/https, got '{}'",
            OFFICIAL_TEMPLATE_REPOSITORY_SCHEMA_VERSION,
            url.scheme()
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CoreError::validation(
            "template repository base_url cannot contain userinfo",
        ));
    }
    if allow_local {
        return Ok(());
    }

    let host = url
        .host_str()
        .ok_or_else(|| CoreError::validation("template repository base_url must include a host"))?;
    if matches!(host, "localhost" | "0.0.0.0") {
        return Err(CoreError::validation(
            "template repository host cannot target local addresses",
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_is_private_or_local(ip) {
            return Err(CoreError::validation(
                "template repository host cannot target private or local addresses",
            ));
        }
        return Ok(());
    }
    if let Some(port) = url.port_or_known_default() {
        let addresses = resolve_template_repository_addresses(host, port, cancellation)?;
        for address in addresses {
            if ip_is_private_or_local(address.ip()) {
                return Err(CoreError::validation(
                    "template repository host cannot target private or local addresses",
                ));
            }
        }
    }
    Ok(())
}

fn resolve_template_repository_addresses(
    host: &str,
    port: u16,
    cancellation: Option<&ExecutionCancellation>,
) -> CoreResult<Vec<std::net::SocketAddr>> {
    let Some(cancellation) = cancellation else {
        return (host, port)
            .to_socket_addrs()
            .map(|addresses| addresses.collect())
            .map_err(template_repo_error);
    };
    cancellation.check()?;
    let host = host.to_owned();
    let blocking_permit = crate::contracts::acquire_detached_blocking_task_permit()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("ariadne-template-dns".to_owned())
        .spawn(move || {
            let _blocking_permit = blocking_permit;
            let result = (host.as_str(), port)
                .to_socket_addrs()
                .map(|addresses| addresses.collect::<Vec<_>>())
                .map_err(template_repo_error);
            let _ = sender.send(result);
        })
        .map_err(CoreError::from)?;
    let started = std::time::Instant::now();
    loop {
        if cancellation.is_cancelled() {
            return Err(CoreError::Cancelled);
        }
        if started.elapsed() >= TEMPLATE_REPOSITORY_CONNECT_TIMEOUT {
            return Err(template_repo_error(format!(
                "template repository DNS resolution timed out after {} ms",
                TEMPLATE_REPOSITORY_CONNECT_TIMEOUT.as_millis()
            )));
        }
        match receiver.recv_timeout(TEMPLATE_REPOSITORY_CANCEL_POLL) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(template_repo_error(
                    "template repository DNS worker disconnected",
                ));
            }
        }
    }
}

fn ip_is_private_or_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
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
    if response
        .content_length()
        .is_some_and(|length| length > MAX_TEMPLATE_REPOSITORY_RESPONSE_BYTES)
    {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "template_repository_response".to_owned(),
            reason: format!("response exceeds {MAX_TEMPLATE_REPOSITORY_RESPONSE_BYTES} bytes"),
        });
    }

    let mut limited = response.take(MAX_TEMPLATE_REPOSITORY_RESPONSE_BYTES.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(template_repo_error)?;
    if bytes.len() as u64 > MAX_TEMPLATE_REPOSITORY_RESPONSE_BYTES {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "template_repository_response".to_owned(),
            reason: format!("response exceeds {MAX_TEMPLATE_REPOSITORY_RESPONSE_BYTES} bytes"),
        });
    }
    serde_json::from_slice::<T>(&bytes).map_err(template_repo_error)
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

/// 从用户消息中提取受支持的 `@...` 项目引用，保持首次出现顺序。
pub fn extract_project_reference_tokens(text: &str) -> Vec<String> {
    const PREFIXES: [&str; 6] = ["确认项", "文档", "章节", "知识", "artifact", "节点"];
    let mut references = Vec::new();
    let mut seen = BTreeSet::new();
    for (offset, character) in text.char_indices() {
        if character != '@' {
            continue;
        }
        let rest = &text[offset + character.len_utf8()..];
        if !PREFIXES.iter().any(|prefix| {
            rest.strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with(':'))
        }) {
            continue;
        }
        let end = rest
            .char_indices()
            .find_map(|(index, current)| {
                project_reference_token_terminator(current).then_some(index)
            })
            .unwrap_or(rest.len());
        let token = format!("@{}", &rest[..end]);
        if parse_project_reference(&token).is_ok() && seen.insert(token.clone()) {
            references.push(token);
        }
    }
    references
}

fn project_reference_token_terminator(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            ',' | ';'
                | '!'
                | '?'
                | '，'
                | '。'
                | '；'
                | '！'
                | '？'
                | '、'
                | '"'
                | '\''
                | '`'
                | '<'
                | '>'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '（'
                | '）'
                | '【'
                | '】'
        )
}

fn project_reference_text_fragments(
    source_id: &str,
    source_version: Option<String>,
    text: &str,
    query: &str,
) -> (Vec<ProjectReferenceTextFragment>, bool) {
    if text.is_empty() {
        return (Vec::new(), false);
    }
    let lines = text.lines().collect::<Vec<_>>();
    let total_lines = u64::try_from(lines.len().max(1)).unwrap_or(u64::MAX);
    let terms = project_reference_query_terms(query);
    let matching_lines = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let lower = line.to_lowercase();
            terms
                .iter()
                .any(|term| lower.contains(term))
                .then_some(index)
        })
        .collect::<Vec<_>>();

    let mut fragments = Vec::new();
    let mut remaining = MAX_PROJECT_REFERENCE_TEXT_CHARS;
    let mut covered_windows = BTreeSet::new();
    for line_index in matching_lines
        .into_iter()
        .take(MAX_PROJECT_REFERENCE_MATCH_WINDOWS)
    {
        let start = line_index.saturating_sub(2);
        let end = (line_index + 3).min(lines.len());
        if !covered_windows.insert((start, end)) || remaining == 0 {
            continue;
        }
        let joined = lines[start..end].join("\n");
        let fragment_text = take_chars_around_query(&joined, &terms, remaining.min(4 * 1024));
        remaining = remaining.saturating_sub(fragment_text.chars().count());
        fragments.push(ProjectReferenceTextFragment {
            source_id: source_id.to_owned(),
            source_version: source_version.clone(),
            start_line: u64::try_from(start + 1).unwrap_or(u64::MAX),
            end_line: u64::try_from(end).unwrap_or(u64::MAX),
            text: fragment_text,
        });
    }

    if fragments.is_empty() {
        let prefix_budget = MAX_PROJECT_REFERENCE_TEXT_CHARS / 2;
        let prefix = take_first_chars(text, prefix_budget);
        let prefix_lines = prefix.matches('\n').count().saturating_add(1);
        fragments.push(ProjectReferenceTextFragment {
            source_id: source_id.to_owned(),
            source_version: source_version.clone(),
            start_line: 1,
            end_line: u64::try_from(prefix_lines).unwrap_or(u64::MAX),
            text: prefix,
        });

        let included = fragments[0].text.chars().count();
        let remaining = MAX_PROJECT_REFERENCE_TEXT_CHARS.saturating_sub(included);
        if text.chars().count() > included && remaining > 0 {
            let suffix = take_last_chars(text, remaining);
            let suffix_line_count = suffix.matches('\n').count().saturating_add(1);
            let start_line = total_lines
                .saturating_sub(u64::try_from(suffix_line_count).unwrap_or(u64::MAX))
                .saturating_add(1);
            fragments.push(ProjectReferenceTextFragment {
                source_id: source_id.to_owned(),
                source_version,
                start_line,
                end_line: total_lines,
                text: suffix,
            });
        }
    }

    let included_chars = fragments
        .iter()
        .map(|fragment| fragment.text.chars().count())
        .sum::<usize>();
    (fragments, text.chars().count() > included_chars)
}

fn project_reference_query_terms(query: &str) -> Vec<String> {
    let mut terms = query
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2)
        .filter(|term| {
            !matches!(
                *term,
                "确认项" | "文档" | "章节" | "知识" | "artifact" | "节点"
            )
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms
}

fn take_chars_around_query(value: &str, terms: &[String], limit: usize) -> String {
    if value.chars().count() <= limit || terms.is_empty() {
        return take_first_chars(value, limit);
    }
    let lower = value.to_lowercase();
    let match_start = terms.iter().find_map(|term| lower.find(term));
    let Some(match_start) = match_start else {
        return take_first_chars(value, limit);
    };
    let match_start_chars = lower[..match_start].chars().count();
    let start = match_start_chars.saturating_sub(limit / 2);
    value.chars().skip(start).take(limit).collect()
}

fn trusted_artifact_path(artifact_root: &Path, storage_uri: &str) -> CoreResult<Option<PathBuf>> {
    let Some(path) = storage_uri.strip_prefix("file://") else {
        return Ok(None);
    };
    let root = absolute_path(artifact_root)?;
    let path = absolute_path(Path::new(path))?;
    ensure_path_under_root(&root, &path)?;
    Ok(Some(path))
}

fn artifact_path_is_textual(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "txt"
                | "md"
                | "markdown"
                | "json"
                | "jsonl"
                | "yaml"
                | "yml"
                | "toml"
                | "csv"
                | "xml"
                | "html"
                | "htm"
                | "log"
                | "diff"
                | "patch"
        )
    )
}

fn read_bounded_utf8_artifact(path: &Path) -> CoreResult<(String, bool)> {
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_PROJECT_REFERENCE_ARTIFACT_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let truncated =
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PROJECT_REFERENCE_ARTIFACT_BYTES;
    if truncated {
        bytes.truncate(usize::try_from(MAX_PROJECT_REFERENCE_ARTIFACT_BYTES).unwrap_or(usize::MAX));
        while std::str::from_utf8(&bytes).is_err() && !bytes.is_empty() {
            bytes.pop();
        }
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| CoreError::validation("artifact reference is not valid UTF-8 text"))?;
    Ok((text, truncated))
}

fn take_first_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn take_last_chars(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    value.chars().skip(count.saturating_sub(limit)).collect()
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

/// 空字符串表示「跟随主题预设」；非空则须为 #RRGGBB。
fn validate_optional_color(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        Ok(())
    } else {
        validate_color(field, value.trim())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_repository_url_rejects_local_targets_without_override() {
        let local =
            validate_template_repository_base_url_with_policy("http://127.0.0.1:8080", false, None)
                .unwrap_err();
        assert!(local
            .to_string()
            .contains("cannot target private or local addresses"));

        let userinfo = validate_template_repository_base_url_with_policy(
            "https://user:pass@example.com",
            false,
            None,
        )
        .unwrap_err();
        assert!(userinfo.to_string().contains("cannot contain userinfo"));

        let scheme =
            validate_template_repository_base_url_with_policy("file:///tmp/templates", false, None)
                .unwrap_err();
        assert!(scheme
            .to_string()
            .contains("must use the official v1 URL or http/https"));
    }

    #[test]
    fn template_repository_url_allows_local_targets_with_explicit_override() {
        validate_template_repository_base_url_with_policy("http://127.0.0.1:8080", true, None)
            .unwrap();
    }
}
