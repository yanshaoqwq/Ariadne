use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::migration::current_schema_version;
use crate::config::secrets::SecretRef;
use crate::core::{
    ApprovalPolicy, CoreError, CoreResult, PermissionPolicy, ProviderCapability, ProviderType,
};

pub const CONFIG_DIR_NAME: &str = ".config";
pub const APP_CONFIG_FILE: &str = "app.yaml";
pub const PROVIDERS_CONFIG_FILE: &str = "providers.yaml";
pub const PERMISSIONS_CONFIG_FILE: &str = "permissions.yaml";
pub const RAG_CONFIG_FILE: &str = "rag.yaml";
pub const WORKFLOW_CONFIG_FILE: &str = "workflow.yaml";
pub const GIT_CONFIG_FILE: &str = "git.yaml";
pub const AUTO_MODE_CONFIG_FILE: &str = "auto_mode.yaml";

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
    pub fn validate(&self) -> CoreResult<()> {
        self.providers.validate()?;
        self.workflow.validate()?;
        self.rag.validate()?;
        Ok(())
    }
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvidersConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
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
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            providers: Vec::new(),
            default_llm_provider_id: None,
            default_embedding_provider_id: None,
            default_reranker_provider_id: None,
            default_search_provider_id: None,
        }
    }
}

impl ProvidersConfig {
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

        for id in [
            &self.default_llm_provider_id,
            &self.default_embedding_provider_id,
            &self.default_reranker_provider_id,
            &self.default_search_provider_id,
        ]
        .into_iter()
        .flatten()
        {
            if !provider_ids.contains(id.as_str()) {
                return Err(CoreError::validation(format!(
                    "default provider id references missing provider: {id}"
                )));
            }
        }

        Ok(())
    }
}

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
    pub fn validate(&self) -> CoreResult<()> {
        if self.provider_id.trim().is_empty() {
            return Err(CoreError::validation("provider_id cannot be empty"));
        }

        if matches!(self.provider_type, ProviderType::OpenAiCompatible)
            && self
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(CoreError::validation(
                "open_ai_compatible provider requires base_url",
            ));
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub policy: PermissionPolicy,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            policy: PermissionPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub vector_store: VectorStoreConfig,
    #[serde(default)]
    pub full_text_store: FullTextStoreConfig,
    #[serde(default = "default_true")]
    pub reranker_enabled: bool,
    #[serde(default = "default_chunk_size")]
    pub chunk_size_chars: u32,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap_chars: u32,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            vector_store: VectorStoreConfig::default(),
            full_text_store: FullTextStoreConfig::default(),
            reranker_enabled: true,
            chunk_size_chars: default_chunk_size(),
            chunk_overlap_chars: default_chunk_overlap(),
        }
    }
}

impl RagConfig {
    pub fn validate(&self) -> CoreResult<()> {
        if self.chunk_size_chars == 0 {
            return Err(CoreError::validation("chunk_size_chars cannot be zero"));
        }

        if self.chunk_overlap_chars >= self.chunk_size_chars {
            return Err(CoreError::validation(
                "chunk_overlap_chars must be smaller than chunk_size_chars",
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreConfig {
    #[serde(default)]
    pub backend: VectorStoreBackend,
    #[serde(default)]
    pub sidecar: SidecarConfig,
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            backend: VectorStoreBackend::QdrantSidecar,
            sidecar: SidecarConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreBackend {
    QdrantSidecar,
    ExternalQdrant,
}

impl Default for VectorStoreBackend {
    fn default() -> Self {
        Self::QdrantSidecar
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarConfig {
    #[serde(default = "default_qdrant_host")]
    pub host: String,
    #[serde(default = "default_qdrant_port")]
    pub port: u16,
    #[serde(default = "default_qdrant_data_dir")]
    pub data_dir: String,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            host: default_qdrant_host(),
            port: default_qdrant_port(),
            data_dir: default_qdrant_data_dir(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextStoreConfig {
    #[serde(default)]
    pub backend: FullTextStoreBackend,
    #[serde(default = "default_tantivy_index_dir")]
    pub index_dir: String,
}

impl Default for FullTextStoreConfig {
    fn default() -> Self {
        Self {
            backend: FullTextStoreBackend::Tantivy,
            index_dir: default_tantivy_index_dir(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullTextStoreBackend {
    Tantivy,
}

impl Default for FullTextStoreBackend {
    fn default() -> Self {
        Self::Tantivy
    }
}

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

    pub fn validate_loop_policy(&self, policy: &crate::core::LoopPolicy) -> CoreResult<()> {
        policy.validate_against_limits(self.max_loop_iterations, self.default_timeout_ms)
    }
}

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
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            enabled_by_default: false,
            preauthorized_budget_usd: None,
            available_approval_prompts: vec![ApprovalPromptConfig::default()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPromptConfig {
    pub prompt_id: String,
    pub display_name: String,
    pub prompt: String,
    #[serde(default)]
    pub default_policy: ApprovalPolicy,
}

impl Default for ApprovalPromptConfig {
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

fn default_project_name() -> String {
    "Untitled Literature Project".to_owned()
}

fn default_locale() -> String {
    "en-US".to_owned()
}

fn default_documents_dir() -> String {
    "documents".to_owned()
}

fn default_workflows_dir() -> String {
    "workflows".to_owned()
}

fn default_skills_dir() -> String {
    "skills".to_owned()
}

fn default_exports_dir() -> String {
    "exports".to_owned()
}

fn default_true() -> bool {
    true
}

fn default_chunk_size() -> u32 {
    2_000
}

fn default_chunk_overlap() -> u32 {
    200
}

fn default_qdrant_host() -> String {
    "127.0.0.1".to_owned()
}

fn default_qdrant_port() -> u16 {
    6333
}

fn default_qdrant_data_dir() -> String {
    ".indexes/qdrant".to_owned()
}

fn default_tantivy_index_dir() -> String {
    ".indexes/tantivy".to_owned()
}

fn default_workflow_timeout_ms() -> u64 {
    300_000
}

fn default_max_loop_iterations() -> u32 {
    5
}

fn default_max_tool_rounds() -> u32 {
    8
}

fn default_runtime_autosave_ms() -> u64 {
    5_000
}

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
