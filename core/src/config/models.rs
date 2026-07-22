use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::migration::current_schema_version;
use crate::config::secrets::SecretRef;
use crate::contracts::{
    ensure_path_under_root, ApprovalPolicy, CoreError, CoreResult, PermissionPolicy,
    ProviderCapability, ProviderType,
};
use crate::node_capabilities::permission_tool_capabilities;

pub const CONFIG_DIR_NAME: &str = ".config";
pub const APP_CONFIG_FILE: &str = "app.yaml";
pub const PROVIDERS_CONFIG_FILE: &str = "providers.yaml";
pub const PERMISSIONS_CONFIG_FILE: &str = "permissions.yaml";
pub const RAG_CONFIG_FILE: &str = "rag.yaml";
pub const WORKFLOW_CONFIG_FILE: &str = "workflow.yaml";
pub const GIT_CONFIG_FILE: &str = "git.yaml";
pub const AUTO_MODE_CONFIG_FILE: &str = "auto_mode.yaml";

/// 聚合后的项目配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub app: AppConfig,
    pub providers: ProvidersConfig,
    pub permissions: PermissionsConfig,
    pub rag: RagConfig,
    pub workflow: WorkflowConfig,
    pub git: GitConfig,
    pub auto_mode: AutoModeConfig,
}

impl Default for ProjectConfig {
    /// 创建完整项目配置的默认值。
    fn default() -> Self {
        Self {
            app: AppConfig::default(),
            providers: ProvidersConfig::default(),
            permissions: PermissionsConfig::default(),
            rag: RagConfig::default(),
            workflow: WorkflowConfig::default(),
            git: GitConfig::default(),
            auto_mode: AutoModeConfig::default(),
        }
    }
}

impl ProjectConfig {
    /// 校验所有子配置。
    pub fn validate(&self) -> CoreResult<()> {
        self.app.validate()?;
        self.providers.validate()?;
        self.permissions.validate()?;
        self.workflow.validate()?;
        self.rag.validate()?;
        self.git.validate()?;
        Ok(())
    }
}

/// 应用基础配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_project_name")]
    pub project_name: String,
    #[serde(default = "default_locale")]
    pub locale: String,
    #[serde(default = "default_documents_dir")]
    pub documents_dir: String,
    #[serde(default = "default_workflows_dir")]
    pub workflows_dir: String,
    #[serde(default = "default_skills_dir")]
    pub skills_dir: String,
    #[serde(default = "default_exports_dir")]
    pub exports_dir: String,
}

impl Default for AppConfig {
    /// 创建应用基础配置默认值。
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            project_name: default_project_name(),
            locale: default_locale(),
            documents_dir: default_documents_dir(),
            workflows_dir: default_workflows_dir(),
            skills_dir: default_skills_dir(),
            exports_dir: default_exports_dir(),
        }
    }
}

impl AppConfig {
    /// 校验项目目录均为可移植的项目相对路径。
    pub fn validate(&self) -> CoreResult<()> {
        if self.project_name.trim().is_empty() {
            return Err(CoreError::validation("project_name cannot be empty"));
        }
        let mut directories = Vec::new();
        for (field, value) in [
            ("documents_dir", self.documents_dir.as_str()),
            ("workflows_dir", self.workflows_dir.as_str()),
            ("skills_dir", self.skills_dir.as_str()),
            ("exports_dir", self.exports_dir.as_str()),
        ] {
            let normalized = normalize_project_relative_directory(field, value)?;
            let first = normalized.split('/').next().unwrap_or_default();
            if matches!(first, CONFIG_DIR_NAME | ".runtime" | ".git") {
                return Err(CoreError::validation(format!(
                    "{field} cannot use reserved project directory: {first}"
                )));
            }
            directories.push((field, normalized));
        }

        for left in 0..directories.len() {
            for right in (left + 1)..directories.len() {
                let (left_field, left_path) = &directories[left];
                let (right_field, right_path) = &directories[right];
                if project_directories_overlap(left_path, right_path) {
                    return Err(CoreError::validation(format!(
                        "{left_field} and {right_field} must not overlap"
                    )));
                }
            }
        }
        Ok(())
    }

    /// 将目录字段规范为跨平台稳定的 `/` 分隔项目相对路径。
    pub fn normalize_directories(&mut self) -> CoreResult<()> {
        self.documents_dir =
            normalize_project_relative_directory("documents_dir", &self.documents_dir)?;
        self.workflows_dir =
            normalize_project_relative_directory("workflows_dir", &self.workflows_dir)?;
        self.skills_dir = normalize_project_relative_directory("skills_dir", &self.skills_dir)?;
        self.exports_dir = normalize_project_relative_directory("exports_dir", &self.exports_dir)?;
        Ok(())
    }
}

/// 项目配置目录的唯一运行时解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLayout {
    pub project_root: PathBuf,
    pub documents: PathBuf,
    pub workflows: PathBuf,
    pub skills: PathBuf,
    pub exports: PathBuf,
}

impl ProjectLayout {
    pub fn from_app(project_root: impl AsRef<Path>, app: &AppConfig) -> CoreResult<Self> {
        app.validate()?;
        let project_root = project_root.as_ref();
        if !project_root.is_absolute() {
            return Err(CoreError::validation("project_root must be absolute"));
        }
        let layout = Self {
            project_root: project_root.to_path_buf(),
            documents: project_root.join(normalize_project_relative_directory(
                "documents_dir",
                &app.documents_dir,
            )?),
            workflows: project_root.join(normalize_project_relative_directory(
                "workflows_dir",
                &app.workflows_dir,
            )?),
            skills: project_root.join(normalize_project_relative_directory(
                "skills_dir",
                &app.skills_dir,
            )?),
            exports: project_root.join(normalize_project_relative_directory(
                "exports_dir",
                &app.exports_dir,
            )?),
        };
        layout.ensure_contained()?;
        Ok(layout)
    }

    pub fn create_configured_directories(&self) -> CoreResult<()> {
        self.ensure_contained()?;
        for directory in [
            &self.documents,
            &self.workflows,
            &self.skills,
            &self.exports,
        ] {
            std::fs::create_dir_all(directory)?;
        }
        self.ensure_contained()?;
        Ok(())
    }

    fn ensure_contained(&self) -> CoreResult<()> {
        for directory in [
            &self.documents,
            &self.workflows,
            &self.skills,
            &self.exports,
        ] {
            ensure_path_under_root(&self.project_root, directory)?;
        }
        Ok(())
    }
}

fn project_directories_overlap(left: &str, right: &str) -> bool {
    let left = left.split('/').collect::<Vec<_>>();
    let right = right.split('/').collect::<Vec<_>>();
    left.starts_with(&right) || right.starts_with(&left)
}

pub fn normalize_project_relative_directory(field: &str, value: &str) -> CoreResult<String> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.ends_with('/')
        || normalized.contains(':')
    {
        return Err(CoreError::validation(format!(
            "{field} must be a non-empty project-relative directory"
        )));
    }
    let path = Path::new(&normalized);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || normalized
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(CoreError::validation(format!(
            "{field} must be a non-empty project-relative directory"
        )));
    }
    Ok(normalized)
}

/// Provider 配置集合。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvidersConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// 当前项目明确允许使用的应用级 Provider。凭据仍绑定到项目身份。
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub authorized_provider_ids: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_llm_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_embedding_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reranker_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_search_provider_id: Option<String>,
}

impl Default for ProvidersConfig {
    /// 创建 provider 配置默认值。
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            providers: Vec::new(),
            authorized_provider_ids: BTreeSet::new(),
            default_llm_provider_id: None,
            default_embedding_provider_id: None,
            default_reranker_provider_id: None,
            default_search_provider_id: None,
        }
    }
}

impl ProvidersConfig {
    /// 校验 provider 唯一性和默认 provider 引用。
    pub fn validate(&self) -> CoreResult<()> {
        let mut provider_ids = BTreeSet::new();
        for provider in &self.providers {
            provider.validate()?;
            if !provider_ids.insert(provider.provider_id.as_str()) {
                return Err(CoreError::validation(format!(
                    "duplicate provider_id: {}",
                    provider.provider_id
                )));
            }
        }

        for provider_id in &self.authorized_provider_ids {
            if provider_id.trim().is_empty() {
                return Err(CoreError::validation(
                    "authorized provider id cannot be empty",
                ));
            }
        }

        self.validate_default_provider(
            "llm",
            self.default_llm_provider_id.as_deref(),
            ProviderCapability::Llm,
        )?;
        self.validate_default_provider(
            "embedding",
            self.default_embedding_provider_id.as_deref(),
            ProviderCapability::Embedding,
        )?;
        self.validate_default_provider(
            "reranker",
            self.default_reranker_provider_id.as_deref(),
            ProviderCapability::Reranker,
        )?;
        self.validate_default_provider(
            "search",
            self.default_search_provider_id.as_deref(),
            ProviderCapability::Search,
        )?;

        Ok(())
    }

    fn validate_default_provider(
        &self,
        role: &str,
        provider_id: Option<&str>,
        required_capability: ProviderCapability,
    ) -> CoreResult<()> {
        let Some(provider_id) = provider_id else {
            return Ok(());
        };
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.provider_id == provider_id)
            .ok_or_else(|| {
                CoreError::validation(format!(
                    "default provider id references missing provider: {provider_id}"
                ))
            })?;
        if !provider.enabled {
            return Err(CoreError::validation(format!(
                "default {role} provider must be enabled: {provider_id}"
            )));
        }
        if !provider
            .models
            .iter()
            .any(|model| model.capability == required_capability)
        {
            return Err(CoreError::validation(format!(
                "default {role} provider lacks required model capability: {provider_id}"
            )));
        }
        Ok(())
    }
}

/// 单个 provider 配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub provider_type: ProviderType,
    pub display_name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<SecretRef>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

impl ProviderConfig {
    /// 校验 provider 基本字段、base_url 和 model 唯一性。
    pub fn validate(&self) -> CoreResult<()> {
        if self.provider_id.trim().is_empty() {
            return Err(CoreError::validation("provider_id cannot be empty"));
        }

        if self.enabled && matches!(self.provider_type, ProviderType::Other) {
            return Err(CoreError::validation(
                "enabled provider must use an executable provider type",
            ));
        }

        if matches!(
            self.provider_type,
            ProviderType::OpenAiCompatible | ProviderType::Local
        ) && self
            .base_url
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(CoreError::validation(
                "open_ai_compatible and local providers require base_url",
            ));
        }
        if let Some(base_url) = self.base_url.as_deref() {
            validate_provider_base_url(base_url)?;
        }

        let mut model_ids = BTreeSet::new();
        for model in &self.models {
            model.validate()?;
            if !model_ids.insert(model.model_id.as_str()) {
                return Err(CoreError::validation(format!(
                    "duplicate model_id for provider {}: {}",
                    self.provider_id, model.model_id
                )));
            }
        }

        Ok(())
    }
}

fn validate_provider_base_url(base_url: &str) -> CoreResult<()> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(CoreError::validation("provider base_url cannot be empty"));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(())
    } else {
        let scheme = trimmed.split("://").next().unwrap_or(trimmed);
        Err(CoreError::validation(format!(
            "provider base_url must use http or https, got '{scheme}'"
        )))
    }
}

/// 单个模型配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelConfig {
    pub model_id: String,
    pub capability: ProviderCapability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_million_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_million_tokens: Option<f64>,
}

impl ModelConfig {
    /// 校验模型 id 和价格字段。
    pub fn validate(&self) -> CoreResult<()> {
        if self.model_id.trim().is_empty() {
            return Err(CoreError::validation("model_id cannot be empty"));
        }

        for (field, value) in [
            (
                "input_cost_per_million_tokens",
                self.input_cost_per_million_tokens,
            ),
            (
                "output_cost_per_million_tokens",
                self.output_cost_per_million_tokens,
            ),
        ] {
            if let Some(cost) = value {
                if !cost.is_finite() || cost < 0.0 {
                    return Err(CoreError::validation(format!(
                        "{field} must be finite and non-negative"
                    )));
                }
            }
        }

        Ok(())
    }

    /// Provider 模型清单中的 capability 是可路由角色，不承载流式或工具调用特性。
    /// 该校验仅用于新写入边界，旧配置仍可读取并由用户显式迁移。
    pub fn validate_provider_model_role(&self) -> CoreResult<()> {
        if matches!(
            self.capability,
            ProviderCapability::Llm
                | ProviderCapability::Embedding
                | ProviderCapability::Reranker
                | ProviderCapability::Search
        ) {
            Ok(())
        } else {
            Err(CoreError::validation(format!(
                "model capability must be an executable provider role: {}",
                self.model_id
            )))
        }
    }
}

/// 权限配置文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub policy: PermissionPolicy,
    /// 工作流节点与项目空间 AI 的权限覆盖；None 表示继承全局 policy。
    #[serde(default = "default_permission_scope_policies")]
    pub scoped_policies: BTreeMap<String, Option<PermissionPolicy>>,
    #[serde(default = "default_permission_tool_controls")]
    pub tool_controls: BTreeMap<String, BTreeMap<String, Option<bool>>>,
}

impl Default for PermissionsConfig {
    /// 创建权限配置默认值。
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            policy: PermissionPolicy::default(),
            scoped_policies: default_permission_scope_policies(),
            tool_controls: default_permission_tool_controls(),
        }
    }
}

impl PermissionsConfig {
    pub fn validate(&self) -> CoreResult<()> {
        self.policy.validate()?;
        for (scope, policy) in &self.scoped_policies {
            if scope.trim().is_empty() {
                return Err(CoreError::validation("permission scope cannot be empty"));
            }
            if let Some(policy) = policy {
                policy.validate()?;
            }
        }
        Ok(())
    }
}

pub fn default_permission_scope_policies() -> BTreeMap<String, Option<PermissionPolicy>> {
    BTreeMap::from([
        ("workflow_nodes".to_owned(), None),
        ("project_ai".to_owned(), None),
    ])
}

pub fn default_permission_tool_controls() -> BTreeMap<String, BTreeMap<String, Option<bool>>> {
    let mut controls = BTreeMap::new();
    controls.insert(
        "global".to_owned(),
        explicit_tool_controls(&[
            ("find", true),
            ("search", true),
            ("web-search", true),
            ("register", false),
            ("write", false),
            ("workflow-tools", false),
        ]),
    );
    controls.insert(
        "project_ai".to_owned(),
        inherited_tool_controls(&["project-ai-workflow-tools"]),
    );
    controls.insert(
        "outliner".to_owned(),
        inherited_tool_controls(&[
            "outliner-register",
            "outliner-find",
            "outliner-insert-lines",
            "outliner-replace-lines",
            "outliner-rewrite-file",
        ]),
    );
    controls.insert(
        "designer".to_owned(),
        inherited_tool_controls(&[
            "designer-register",
            "designer-find",
            "designer-insert-lines",
            "designer-replace-lines",
            "designer-rewrite-file",
        ]),
    );
    controls.insert(
        "planner".to_owned(),
        inherited_tool_controls(&[
            "planner-register",
            "planner-find",
            "planner-insert-lines",
            "planner-replace-lines",
            "planner-rewrite-file",
        ]),
    );
    controls.insert(
        "detail".to_owned(),
        inherited_tool_controls(&["detail-find"]),
    );
    controls.insert(
        "writer".to_owned(),
        inherited_tool_controls(&["writer-find", "writer-insert-lines", "writer-replace-lines"]),
    );
    controls.insert(
        "critic".to_owned(),
        inherited_tool_controls(&["critic-find"]),
    );
    controls.insert(
        "prudent".to_owned(),
        inherited_tool_controls(&["prudent-find"]),
    );
    controls.insert(
        "polisher".to_owned(),
        inherited_tool_controls(&[
            "polisher-find",
            "polisher-insert-lines",
            "polisher-replace-lines",
        ]),
    );
    for capability in permission_tool_capabilities() {
        let node_controls = controls
            .entry(capability.tool_scope.to_owned())
            .or_default();
        if let Some(tool) = capability.project_search_tool {
            node_controls.entry(tool.to_owned()).or_insert(None);
        }
        if let Some(tool) = capability.web_search_tool {
            node_controls.entry(tool.to_owned()).or_insert(None);
        }
    }
    controls
}

fn explicit_tool_controls(tools: &[(&str, bool)]) -> BTreeMap<String, Option<bool>> {
    tools
        .iter()
        .map(|(tool, enabled)| ((*tool).to_owned(), Some(*enabled)))
        .collect()
}

fn inherited_tool_controls(tools: &[&str]) -> BTreeMap<String, Option<bool>> {
    tools
        .iter()
        .map(|tool| ((*tool).to_owned(), None))
        .collect()
}

/// RAG 配置文件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub vector_store: VectorStoreConfig,
    #[serde(default)]
    pub full_text_store: FullTextStoreConfig,
    #[serde(default)]
    pub reranker_enabled: bool,
    #[serde(default = "default_chunk_size")]
    pub chunk_size_chars: u32,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap_chars: u32,
}

impl Default for RagConfig {
    /// 创建 RAG 配置默认值。
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            vector_store: VectorStoreConfig::default(),
            full_text_store: FullTextStoreConfig::default(),
            reranker_enabled: false,
            chunk_size_chars: default_chunk_size(),
            chunk_overlap_chars: default_chunk_overlap(),
        }
    }
}

impl RagConfig {
    /// 校验 chunk 大小和 overlap。
    pub fn validate(&self) -> CoreResult<()> {
        if self.chunk_size_chars == 0 {
            return Err(CoreError::validation("chunk_size_chars cannot be zero"));
        }

        if self.chunk_overlap_chars >= self.chunk_size_chars {
            return Err(CoreError::validation(
                "chunk_overlap_chars must be smaller than chunk_size_chars",
            ));
        }

        self.vector_store.validate()?;

        Ok(())
    }
}

/// 向量存储配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreConfig {
    /// 显式启用真实向量链；缺失字段的旧项目按全文-only 处理，禁止生成假向量。
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub backend: VectorStoreBackend,
    #[serde(default = "default_qdrant_collection")]
    pub collection: String,
    #[serde(default = "default_qdrant_vector_dimensions")]
    pub vector_dimensions: u32,
    #[serde(default)]
    pub sidecar: SidecarConfig,
}

impl Default for VectorStoreConfig {
    /// 创建向量存储配置默认值。
    fn default() -> Self {
        Self {
            enabled: false,
            backend: VectorStoreBackend::QdrantSidecar,
            collection: default_qdrant_collection(),
            vector_dimensions: default_qdrant_vector_dimensions(),
            sidecar: SidecarConfig::default(),
        }
    }
}

impl VectorStoreConfig {
    /// 仅在显式启用时要求 Qdrant 和 embedding 维度配置完整。
    pub fn validate(&self) -> CoreResult<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.collection.trim().is_empty() {
            return Err(CoreError::validation(
                "qdrant collection cannot be empty when vector retrieval is enabled",
            ));
        }
        if self.vector_dimensions == 0 {
            return Err(CoreError::validation(
                "qdrant vector_dimensions must be positive",
            ));
        }
        if self.sidecar.host.trim().is_empty() {
            return Err(CoreError::validation("qdrant host cannot be empty"));
        }
        if matches!(self.backend, VectorStoreBackend::QdrantSidecar) {
            if self.sidecar.data_dir.trim().is_empty() {
                return Err(CoreError::validation("qdrant data_dir cannot be empty"));
            }
            if self.sidecar.binary_path.trim().is_empty() {
                return Err(CoreError::validation(
                    "qdrant sidecar binary_path cannot be empty",
                ));
            }
            if self.sidecar.startup_timeout_ms == 0 {
                return Err(CoreError::validation(
                    "qdrant sidecar startup_timeout_ms must be positive",
                ));
            }
        } else if self.sidecar.port == 0 {
            return Err(CoreError::validation(
                "external qdrant port must be positive",
            ));
        }
        Ok(())
    }
}

/// 向量存储后端类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreBackend {
    QdrantSidecar,
    ExternalQdrant,
}

impl Default for VectorStoreBackend {
    /// 默认使用本地 Qdrant sidecar。
    fn default() -> Self {
        Self::QdrantSidecar
    }
}

/// sidecar 基础配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarConfig {
    #[serde(default = "default_qdrant_host")]
    pub host: String,
    #[serde(default = "default_qdrant_port")]
    pub port: u16,
    #[serde(default = "default_qdrant_data_dir")]
    pub data_dir: String,
    /// 旧项目兼容输入；正式持久化和运行时事实源位于 app-state。
    #[serde(default = "default_qdrant_binary_path", skip_serializing)]
    pub binary_path: String,
    /// 旧项目兼容输入；正式持久化和运行时事实源位于 app-state。
    #[serde(default = "default_qdrant_startup_timeout_ms", skip_serializing)]
    pub startup_timeout_ms: u64,
}

impl Default for SidecarConfig {
    /// 创建 sidecar 配置默认值。
    fn default() -> Self {
        Self {
            host: default_qdrant_host(),
            port: default_qdrant_port(),
            data_dir: default_qdrant_data_dir(),
            binary_path: default_qdrant_binary_path(),
            startup_timeout_ms: default_qdrant_startup_timeout_ms(),
        }
    }
}

/// 全文存储配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextStoreConfig {
    #[serde(default)]
    pub backend: FullTextStoreBackend,
    #[serde(default = "default_tantivy_index_dir")]
    pub index_dir: String,
}

impl Default for FullTextStoreConfig {
    /// 创建全文索引配置默认值。
    fn default() -> Self {
        Self {
            backend: FullTextStoreBackend::Tantivy,
            index_dir: default_tantivy_index_dir(),
        }
    }
}

/// 全文存储后端类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullTextStoreBackend {
    Tantivy,
}

impl Default for FullTextStoreBackend {
    /// 默认使用 Tantivy 全文索引。
    fn default() -> Self {
        Self::Tantivy
    }
}

/// 工作流运行配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_workflow_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default = "default_max_loop_iterations")]
    pub max_loop_iterations: u32,
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: u32,
    #[serde(default = "default_true")]
    pub checkpoint_enabled: bool,
    #[serde(default = "default_runtime_autosave_ms")]
    pub runtime_autosave_ms: u64,
}

impl Default for WorkflowConfig {
    /// 创建工作流运行配置默认值。
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            default_timeout_ms: default_workflow_timeout_ms(),
            max_loop_iterations: default_max_loop_iterations(),
            max_tool_rounds: default_max_tool_rounds(),
            checkpoint_enabled: true,
            runtime_autosave_ms: default_runtime_autosave_ms(),
        }
    }
}

impl WorkflowConfig {
    /// 校验 workflow 全局限制。
    pub fn validate(&self) -> CoreResult<()> {
        if self.default_timeout_ms == 0 {
            return Err(CoreError::validation("default_timeout_ms cannot be zero"));
        }

        if self.max_loop_iterations == 0 {
            return Err(CoreError::validation("max_loop_iterations cannot be zero"));
        }

        if self.max_tool_rounds == 0 {
            return Err(CoreError::validation("max_tool_rounds cannot be zero"));
        }

        Ok(())
    }

    /// 用 workflow 全局限制校验单个 loop policy。
    pub fn validate_loop_policy(&self, policy: &crate::contracts::LoopPolicy) -> CoreResult<()> {
        policy.validate_against_limits(self.max_loop_iterations, self.default_timeout_ms)
    }
}

/// Git 跟踪策略配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_true")]
    pub track_documents: bool,
    #[serde(default = "default_true")]
    pub track_workflows: bool,
    #[serde(default = "default_true")]
    pub track_skills: bool,
    #[serde(default = "default_true")]
    pub track_non_sensitive_config: bool,
    #[serde(default = "default_ignored_paths")]
    pub ignored_paths: Vec<String>,
}

impl Default for GitConfig {
    /// 创建 Git 跟踪配置默认值。
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            track_documents: true,
            track_workflows: true,
            track_skills: true,
            track_non_sensitive_config: true,
            ignored_paths: default_ignored_paths(),
        }
    }
}

impl GitConfig {
    pub fn validate(&self) -> CoreResult<()> {
        let mut unique = BTreeSet::new();
        for path in &self.ignored_paths {
            let normalized = normalize_git_ignored_path(path)?;
            if !unique.insert(normalized.clone()) {
                return Err(CoreError::validation(format!(
                    "duplicate git ignored path: {normalized}"
                )));
            }
        }
        Ok(())
    }

    pub fn normalize_ignored_paths(&mut self) -> CoreResult<()> {
        self.ignored_paths = self
            .ignored_paths
            .iter()
            .map(|path| normalize_git_ignored_path(path))
            .collect::<CoreResult<BTreeSet<_>>>()?
            .into_iter()
            .collect();
        Ok(())
    }
}

pub fn normalize_git_ignored_path(path: &str) -> CoreResult<String> {
    let replaced = path.trim().replace('\\', "/");
    let normalized = replaced.trim_end_matches('/');
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(CoreError::validation(format!(
            "git ignored path must be project-relative: {path}"
        )));
    }
    Ok(normalized.to_owned())
}

/// Auto Mode 配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoModeConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub enabled_by_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preauthorized_budget_usd: Option<f64>,
    #[serde(default)]
    pub available_approval_prompts: Vec<ApprovalPromptConfig>,
}

impl Default for AutoModeConfig {
    /// 创建 Auto Mode 配置默认值。
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            enabled_by_default: false,
            preauthorized_budget_usd: None,
            available_approval_prompts: vec![ApprovalPromptConfig::default()],
        }
    }
}

/// 可选审批提示词配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPromptConfig {
    pub prompt_id: String,
    pub display_name: String,
    pub prompt: String,
    #[serde(default)]
    pub default_policy: ApprovalPolicy,
}

impl Default for ApprovalPromptConfig {
    /// 创建默认审批提示词配置。
    fn default() -> Self {
        Self {
            prompt_id: "default-review".to_owned(),
            display_name: "Default Review".to_owned(),
            prompt: "Review the proposed change and return an approval decision with reasons."
                .to_owned(),
            default_policy: ApprovalPolicy::default(),
        }
    }
}

/// 默认项目名称。
fn default_project_name() -> String {
    "Untitled Literature Project".to_owned()
}

/// 默认语言地区。
fn default_locale() -> String {
    "en-US".to_owned()
}

/// 默认文档目录。
fn default_documents_dir() -> String {
    "documents".to_owned()
}

/// 默认工作流目录。
fn default_workflows_dir() -> String {
    "workflows".to_owned()
}

/// 默认 Skill 目录。
fn default_skills_dir() -> String {
    "skills".to_owned()
}

/// 默认导出目录。
fn default_exports_dir() -> String {
    "exports".to_owned()
}

/// serde 默认 true helper。
fn default_true() -> bool {
    true
}

/// 默认 chunk 字符数。
fn default_chunk_size() -> u32 {
    2_000
}

/// 默认 chunk 重叠字符数。
fn default_chunk_overlap() -> u32 {
    200
}

/// 默认 Qdrant host。
fn default_qdrant_host() -> String {
    "127.0.0.1".to_owned()
}

/// 默认 Qdrant HTTP port。
fn default_qdrant_port() -> u16 {
    6333
}

/// 默认 Qdrant 数据目录。
fn default_qdrant_data_dir() -> String {
    ".indexes/qdrant".to_owned()
}

/// 默认 Qdrant collection。
fn default_qdrant_collection() -> String {
    "ariadne_chunks".to_owned()
}

/// 默认向量维度；可按所选 embedding 模型显式覆盖。
fn default_qdrant_vector_dimensions() -> u32 {
    1_536
}

/// 默认由 Ariadne 在首次启用向量检索时供应固定版本的 Qdrant sidecar。
fn default_qdrant_binary_path() -> String {
    "qdrant".to_owned()
}

/// 默认等待 sidecar 启动十秒。
fn default_qdrant_startup_timeout_ms() -> u64 {
    10_000
}

/// 默认 Tantivy 索引目录。
fn default_tantivy_index_dir() -> String {
    ".indexes/tantivy".to_owned()
}

/// 默认 workflow 超时。
fn default_workflow_timeout_ms() -> u64 {
    300_000
}

/// 默认最大 loop 轮次。
fn default_max_loop_iterations() -> u32 {
    5
}

/// 默认最大 tool-use 轮次。
fn default_max_tool_rounds() -> u32 {
    8
}

/// 默认 runtime 自动保存间隔。
fn default_runtime_autosave_ms() -> u64 {
    5_000
}

/// 默认 Git 忽略路径。
fn default_ignored_paths() -> Vec<String> {
    vec![
        ".cache/".to_owned(),
        ".runtime/".to_owned(),
        ".indexes/".to_owned(),
        ".knowledge/".to_owned(),
        "costs.db".to_owned(),
        "runtime.db".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_ignored_directory_paths_accept_and_remove_trailing_separator() {
        assert_eq!(normalize_git_ignored_path(".cache/").unwrap(), ".cache");
        assert_eq!(normalize_git_ignored_path("drafts\\").unwrap(), "drafts");
    }

    #[test]
    fn git_ignored_paths_still_reject_absolute_and_parent_escape() {
        assert!(normalize_git_ignored_path("/tmp/cache/").is_err());
        assert!(normalize_git_ignored_path("drafts/../secrets").is_err());
    }
}
