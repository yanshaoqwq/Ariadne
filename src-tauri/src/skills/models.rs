use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{CoreError, CoreResult, PortDefinition, PortMap, SkillExecutorKind};

/// Skill manifest 文件名。
pub const SKILL_MANIFEST_FILE: &str = "skill.json";

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
    pub fn to_core_definition(&self) -> CoreResult<crate::core::SkillDefinition> {
        self.validate()?;
        Ok(crate::core::SkillDefinition {
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

fn default_port_type() -> String {
    "inline".to_owned()
}

fn default_true() -> bool {
    true
}
