use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::{
    CoreError, CoreResult, PortDefinition, PortMap, SkillExecutorKind, WorkflowDefinition,
};

/// Skill manifest 文件名。
pub const SKILL_MANIFEST_FILE: &str = "skill.json";
/// 旧 LLM/HTTP/WASM 执行适配器沿用的 manifest 文件名，后续迁移时替代 Skill 命名。
pub const EXECUTOR_ADAPTER_MANIFEST_FILE: &str = SKILL_MANIFEST_FILE;
/// PromptTemplate manifest 文件名。
pub const PROMPT_TEMPLATE_MANIFEST_FILE: &str = "prompt_template.json";
/// Workflow 模板 manifest 文件名。
pub const WORKFLOW_MANIFEST_FILE: &str = "workflow.json";

/// Skill 输入/输出 schema 的单个字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillPortSchema {
    pub name: String,
    #[serde(default = "default_port_type")]
    pub type_name: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl SkillPortSchema {
    /// 转换为核心 typed port 定义。
    pub fn to_port_definition(&self) -> CoreResult<PortDefinition> {
        if self.name.trim().is_empty() {
            return Err(CoreError::validation("skill port name cannot be empty"));
        }
        if self.type_name.trim().is_empty() {
            return Err(CoreError::validation(
                "skill port type_name cannot be empty",
            ));
        }
        let mut port = PortDefinition::new(&self.name, &self.type_name, self.required);
        if let Some(description) = &self.description {
            port = port.with_description(description.clone());
        }
        Ok(port)
    }
}

/// Skill 输入输出 schema。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SkillIoSchema {
    #[serde(default)]
    pub inputs: Vec<SkillPortSchema>,
    #[serde(default)]
    pub outputs: Vec<SkillPortSchema>,
}

impl SkillIoSchema {
    /// 生成输入端口定义。
    pub fn input_ports(&self) -> CoreResult<Vec<PortDefinition>> {
        self.inputs
            .iter()
            .map(SkillPortSchema::to_port_definition)
            .collect()
    }

    /// 生成输出端口定义。
    pub fn output_ports(&self) -> CoreResult<Vec<PortDefinition>> {
        self.outputs
            .iter()
            .map(SkillPortSchema::to_port_definition)
            .collect()
    }
}

/// Skill 运行限制。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillLimits {
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,
}

impl Default for SkillLimits {
    /// 创建默认限制，避免 Skill 无边界运行。
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            max_output_bytes: 1_048_576,
            max_memory_bytes: Some(128 * 1024 * 1024),
        }
    }
}

impl SkillLimits {
    /// 校验限制必须可执行。
    pub fn validate(&self) -> CoreResult<()> {
        if self.timeout_ms == 0 {
            return Err(CoreError::validation("skill timeout_ms cannot be zero"));
        }
        if self.max_output_bytes == 0 {
            return Err(CoreError::validation(
                "skill max_output_bytes cannot be zero",
            ));
        }
        Ok(())
    }
}

/// HTTP Skill 配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpSkillConfig {
    pub host: String,
    pub method: String,
    pub path: String,
    /// 远端声明支持的幂等请求头。工作流执行时其值固定为 operation ID；未声明的
    /// POST 使用 at-most-once 策略，未知结果不会自动重发。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_header: Option<String>,
}

/// WASM Skill 配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmSkillConfig {
    pub module_path: String,
    #[serde(default)]
    pub allow_network: bool,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

/// LLM Skill 配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmSkillConfig {
    pub provider_id: String,
    pub model_id: String,
    pub prompt_template: String,
}

/// Skill executor 的具体配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillExecutorConfig {
    Llm(LlmSkillConfig),
    Http(HttpSkillConfig),
    Wasm(WasmSkillConfig),
}

impl SkillExecutorConfig {
    /// 返回核心注册表使用的 executor 类型。
    pub fn kind(&self) -> SkillExecutorKind {
        match self {
            Self::Llm(_) => SkillExecutorKind::Llm,
            Self::Http(_) => SkillExecutorKind::Http,
            Self::Wasm(_) => SkillExecutorKind::Wasm,
        }
    }
}

/// Skill manifest，项目目录和全局目录都使用同一结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillManifest {
    pub skill_id: String,
    pub name: String,
    pub version: String,
    pub executor: SkillExecutorConfig,
    #[serde(default)]
    pub schema: SkillIoSchema,
    #[serde(default)]
    pub limits: SkillLimits,
    #[serde(default)]
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub config_schema: Value,
    #[serde(default)]
    pub metadata: Value,
}

impl SkillManifest {
    /// 校验 manifest 并生成核心 SkillDefinition。
    pub fn to_core_definition(&self) -> CoreResult<crate::contracts::SkillDefinition> {
        self.validate()?;
        Ok(crate::contracts::SkillDefinition {
            skill_id: self.skill_id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            executor: self.executor.kind(),
            input_ports: self.schema.input_ports()?,
            output_ports: self.schema.output_ports()?,
            config_schema: self.config_schema.clone(),
        })
    }

    /// 校验 manifest 的基础字段和限制。
    pub fn validate(&self) -> CoreResult<()> {
        if self.skill_id.trim().is_empty() {
            return Err(CoreError::validation("skill_id cannot be empty"));
        }
        if self.name.trim().is_empty() {
            return Err(CoreError::validation("skill name cannot be empty"));
        }
        if self.version.trim().is_empty() {
            return Err(CoreError::validation("skill version cannot be empty"));
        }
        self.limits.validate()?;
        if !self.estimated_cost_usd.is_finite() || self.estimated_cost_usd < 0.0 {
            return Err(CoreError::validation(
                "skill estimated_cost_usd must be finite and non-negative",
            ));
        }
        match &self.executor {
            SkillExecutorConfig::Llm(config) => {
                if config.provider_id.trim().is_empty() || config.model_id.trim().is_empty() {
                    return Err(CoreError::validation(
                        "llm skill requires provider_id and model_id",
                    ));
                }
            }
            SkillExecutorConfig::Http(config) => {
                if config.host.trim().is_empty() {
                    return Err(CoreError::validation("http skill host cannot be empty"));
                }
                if let Some(header) = config.idempotency_header.as_deref() {
                    let header = header.trim();
                    if header.is_empty()
                        || reqwest::header::HeaderName::from_bytes(header.as_bytes()).is_err()
                    {
                        return Err(CoreError::validation(
                            "http skill idempotency_header must be a valid HTTP header name",
                        ));
                    }
                    if !config.method.eq_ignore_ascii_case("POST") {
                        return Err(CoreError::validation(
                            "http skill idempotency_header is only valid for POST",
                        ));
                    }
                }
            }
            SkillExecutorConfig::Wasm(config) => {
                if config.module_path.trim().is_empty() {
                    return Err(CoreError::validation(
                        "wasm skill module_path cannot be empty",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Skill 执行请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRunRequest {
    pub skill_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub inputs: PortMap,
    #[serde(default)]
    pub metadata: Value,
}

/// Skill 执行输出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRunOutput {
    #[serde(default)]
    pub outputs: PortMap,
    #[serde(default)]
    pub logs: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// 后端执行结果，统一进入输出大小和日志脱敏检查。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillBackendOutput {
    #[serde(default)]
    pub outputs: PortMap,
    #[serde(default)]
    pub logs: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// 旧 Skill manifest 的工程新名称；保留类型别名以便分阶段迁移调用方。
pub type ExecutorAdapterManifest = SkillManifest;
/// 旧 Skill executor 配置的工程新名称。
pub type ExecutorAdapterConfig = SkillExecutorConfig;
/// 旧 Skill 运行请求的工程新名称。
pub type ExecutorAdapterRunRequest = SkillRunRequest;
/// 旧 Skill 运行输出的工程新名称。
pub type ExecutorAdapterRunOutput = SkillRunOutput;

/// SemVer 版本号，PromptTemplate 和 Workflow 都使用同一解析规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PromptTemplateVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl PromptTemplateVersion {
    /// 解析 `major.minor.patch` 版本号，拒绝缺段或非数字版本。
    pub fn parse(value: &str) -> CoreResult<Self> {
        let mut parts = value.split('.');
        let major = parse_version_part(parts.next(), "major")?;
        let minor = parse_version_part(parts.next(), "minor")?;
        let patch = parse_version_part(parts.next(), "patch")?;
        if parts.next().is_some() {
            return Err(CoreError::validation(format!(
                "version must use SemVer major.minor.patch: {value}"
            )));
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    /// 判断候选版本相对当前锁定版本的更新类型。
    pub fn update_kind(self, candidate: Self) -> PromptTemplateUpdateKind {
        if candidate <= self {
            PromptTemplateUpdateKind::None
        } else if candidate.major != self.major {
            PromptTemplateUpdateKind::Major
        } else if candidate.minor != self.minor {
            PromptTemplateUpdateKind::Minor
        } else {
            PromptTemplateUpdateKind::Patch
        }
    }
}

impl std::fmt::Display for PromptTemplateVersion {
    /// 输出标准 SemVer 文本。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// PromptTemplate 更新类型，用于 GUI 区分安全更新和不兼容更新。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptTemplateUpdateKind {
    None,
    Patch,
    Minor,
    Major,
}

/// 可复用内联提示词模板 manifest，不负责执行外部代码或模型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptTemplateManifest {
    pub template_id: String,
    pub name: String,
    pub version: String,
    pub template: String,
    pub describe: String,
    #[serde(default)]
    pub parameter_schema: Value,
    #[serde(default)]
    pub metadata: Value,
}

impl PromptTemplateManifest {
    /// 校验模板基础字段、SemVer 版本和参数 schema 形态。
    pub fn validate(&self) -> CoreResult<()> {
        validate_non_empty("template_id", &self.template_id)?;
        validate_non_empty("template name", &self.name)?;
        validate_non_empty("template version", &self.version)?;
        validate_non_empty("template body", &self.template)?;
        validate_non_empty("template describe", &self.describe)?;
        PromptTemplateVersion::parse(&self.version)?;
        if !(self.parameter_schema.is_null() || self.parameter_schema.is_object()) {
            return Err(CoreError::validation(
                "prompt template parameter_schema must be an object or null",
            ));
        }
        Ok(())
    }

    /// 返回模板内容 hash；节点锁定时记录它，避免版本号相同但内容漂移。
    pub fn content_hash(&self) -> CoreResult<String> {
        stable_json_hash(&serde_json::json!({
            "template_id": self.template_id,
            "version": self.version,
            "template": self.template,
            "describe": self.describe,
            "parameter_schema": self.parameter_schema,
        }))
    }
}

/// 节点锁定的 PromptTemplate 引用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptTemplateReference {
    pub template_id: String,
    pub version: String,
    pub content_hash: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
}

impl PromptTemplateReference {
    /// 从 manifest 创建固定版本引用。
    pub fn from_manifest(manifest: &PromptTemplateManifest) -> CoreResult<Self> {
        manifest.validate()?;
        Ok(Self {
            template_id: manifest.template_id.clone(),
            version: manifest.version.clone(),
            content_hash: manifest.content_hash()?,
            parameters: BTreeMap::new(),
        })
    }

    /// 校验引用字段和 SemVer 版本。
    pub fn validate(&self) -> CoreResult<()> {
        validate_non_empty("template_id", &self.template_id)?;
        validate_non_empty("template version", &self.version)?;
        validate_non_empty("template content_hash", &self.content_hash)?;
        PromptTemplateVersion::parse(&self.version)?;
        Ok(())
    }
}

/// PromptTemplate 更新检测结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptTemplateUpdateStatus {
    pub template_id: String,
    pub locked_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    pub update_kind: PromptTemplateUpdateKind,
}

/// 单次 prompt 渲染 trace，只保存 hash 和来源映射，不保存展开后的完整 prompt。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptRenderTrace {
    pub original_template_hash: String,
    #[serde(default)]
    pub template_dependencies: Vec<PromptTemplateReference>,
    #[serde(default)]
    pub input_sources: BTreeMap<String, String>,
    pub final_prompt_hash: String,
}

impl PromptRenderTrace {
    /// 从原始模板、最终 prompt 和依赖信息创建最小可审计 trace。
    pub fn new(
        original_template: &str,
        final_prompt: &str,
        template_dependencies: Vec<PromptTemplateReference>,
        input_sources: BTreeMap<String, String>,
    ) -> CoreResult<Self> {
        Ok(Self {
            original_template_hash: stable_text_hash(original_template),
            template_dependencies,
            input_sources,
            final_prompt_hash: stable_text_hash(final_prompt),
        })
    }
}

/// Workflow 模板 manifest；导入时复制展开为普通工作流定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowManifest {
    pub workflow_id: String,
    pub name: String,
    pub version: String,
    pub workflow: WorkflowDefinition,
    #[serde(default)]
    pub prompt_templates: Vec<PromptTemplateReference>,
    #[serde(default)]
    pub required_node_types: Vec<String>,
    #[serde(default)]
    pub required_tools: Vec<String>,
    #[serde(default)]
    pub required_permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_ariadne_version: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

impl WorkflowManifest {
    /// 校验 workflow 模板本身以及依赖引用。
    pub fn validate(&self) -> CoreResult<()> {
        validate_non_empty("workflow_id", &self.workflow_id)?;
        validate_non_empty("workflow name", &self.name)?;
        validate_non_empty("workflow version", &self.version)?;
        PromptTemplateVersion::parse(&self.version)?;
        self.workflow.validate_topology()?;
        for reference in &self.prompt_templates {
            reference.validate()?;
        }
        for node_type in &self.required_node_types {
            validate_non_empty("required node type", node_type)?;
        }
        Ok(())
    }

    /// 导入 Workflow 时复制展开普通节点和边，不和源模板保持自动同步。
    pub fn import_definition(&self) -> CoreResult<WorkflowDefinition> {
        self.validate()?;
        Ok(self.workflow.clone())
    }
}

/// 解析 SemVer 单段版本号。
fn parse_version_part(part: Option<&str>, name: &str) -> CoreResult<u64> {
    let part =
        part.ok_or_else(|| CoreError::validation(format!("version is missing {name} segment")))?;
    if part.is_empty() {
        return Err(CoreError::validation(format!(
            "version {name} segment cannot be empty"
        )));
    }
    part.parse::<u64>()
        .map_err(|_| CoreError::validation(format!("version {name} segment must be an integer")))
}

/// 校验字符串字段非空。
fn validate_non_empty(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }
    Ok(())
}

/// 对文本做稳定 FNV-1a hash，避免运行时随机 hash 影响审计记录。
pub fn stable_text_hash(text: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// 对 JSON 做稳定 hash，先使用 serde_json 的确定序列化，再计算文本 hash。
pub fn stable_json_hash(value: &Value) -> CoreResult<String> {
    Ok(stable_text_hash(&serde_json::to_string(value)?))
}

fn default_port_type() -> String {
    "inline".to_owned()
}

fn default_true() -> bool {
    true
}
