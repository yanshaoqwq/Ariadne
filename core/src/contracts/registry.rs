use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::errors::{CoreError, CoreResult};
use crate::contracts::ports::PortDefinition;
use crate::contracts::workflow::{NodeDefinition, COMMUNICATION_PORT, EXECUTION_OUTPUT_PORT};

/// 可注册项需要提供稳定 key。
pub trait RegistryItem {
    /// 返回注册表 key。
    fn registry_key(&self) -> &str;
}

/// 通用类型注册表。
#[derive(Debug, Clone)]
pub struct TypedRegistry<T> {
    name: &'static str,
    entries: BTreeMap<String, T>,
}

impl<T> TypedRegistry<T> {
    /// 创建命名注册表。
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            entries: BTreeMap::new(),
        }
    }

    /// 判断 key 是否存在。
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// 按 key 读取注册项。
    pub fn get(&self, key: &str) -> CoreResult<&T> {
        self.entries
            .get(key)
            .ok_or_else(|| CoreError::RegistryMissing {
                registry: self.name,
                key: key.to_owned(),
            })
    }

    /// 遍历所有注册项。
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.values()
    }

    /// 返回注册项数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 判断注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<T: RegistryItem> TypedRegistry<T> {
    /// 注册新条目，并拒绝重复 key。
    pub fn register(&mut self, item: T) -> CoreResult<()> {
        let key = item.registry_key().to_owned();
        if self.entries.contains_key(&key) {
            return Err(CoreError::RegistryDuplicate {
                registry: self.name,
                key,
            });
        }

        self.entries.insert(key, item);
        Ok(())
    }
}

impl RegistryItem for NodeDefinition {
    /// 节点类型名作为注册 key。
    fn registry_key(&self) -> &str {
        &self.type_name
    }
}

/// 节点定义注册表。
#[derive(Debug, Clone)]
pub struct NodeRegistry {
    inner: TypedRegistry<NodeDefinition>,
}

impl Default for NodeRegistry {
    /// 创建带内建节点定义的节点注册表。
    fn default() -> Self {
        let mut registry = Self {
            inner: TypedRegistry::new("node"),
        };
        registry
            .register(builtin_start_node_definition())
            .expect("builtin start node definition must be valid");
        registry
    }
}

impl NodeRegistry {
    /// 注册节点定义。
    pub fn register(&mut self, definition: NodeDefinition) -> CoreResult<()> {
        definition.validate()?;
        self.inner.register(definition)
    }

    /// 读取节点定义。
    pub fn get(&self, type_name: &str) -> CoreResult<&NodeDefinition> {
        self.inner.get(type_name)
    }

    /// 判断节点类型是否存在。
    pub fn contains(&self, type_name: &str) -> bool {
        self.inner.contains(type_name)
    }

    /// 遍历节点定义。
    pub fn values(&self) -> impl Iterator<Item = &NodeDefinition> {
        self.inner.values()
    }
}

fn builtin_start_node_definition() -> NodeDefinition {
    NodeDefinition {
        type_name: "start".to_owned(),
        display_name: Some("ui.node.start".to_owned()),
        execution_input_ports: Vec::new(),
        execution_output_ports: vec![PortDefinition::new(
            EXECUTION_OUTPUT_PORT,
            crate::contracts::CONTROL_PORT_TYPE,
            false,
        )],
        communication_ports: vec![PortDefinition::new(
            COMMUNICATION_PORT,
            crate::contracts::COMMUNICATION_PORT_TYPE,
            false,
        )],
        input_ports: Vec::new(),
        output_ports: Vec::new(),
        supports_checkpoint: false,
        supports_auto_approval: false,
    }
}

/// Skill 执行器类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillExecutorKind {
    Llm,
    Http,
    Wasm,
}

/// Skill 定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub skill_id: String,
    pub name: String,
    pub version: String,
    pub executor: SkillExecutorKind,
    #[serde(default)]
    pub input_ports: Vec<PortDefinition>,
    #[serde(default)]
    pub output_ports: Vec<PortDefinition>,
    #[serde(default)]
    pub config_schema: Value,
}

impl RegistryItem for SkillDefinition {
    /// skill_id 作为注册 key。
    fn registry_key(&self) -> &str {
        &self.skill_id
    }
}

/// Skill 注册表。
#[derive(Debug, Clone)]
pub struct SkillRegistry {
    inner: TypedRegistry<SkillDefinition>,
}

impl Default for SkillRegistry {
    /// 创建空的 Skill 注册表。
    fn default() -> Self {
        Self {
            inner: TypedRegistry::new("skill"),
        }
    }
}

impl SkillRegistry {
    /// 注册 Skill 定义。
    pub fn register(&mut self, definition: SkillDefinition) -> CoreResult<()> {
        if definition.skill_id.trim().is_empty() {
            return Err(CoreError::validation("skill_id cannot be empty"));
        }

        self.inner.register(definition)
    }

    /// 读取 Skill 定义。
    pub fn get(&self, skill_id: &str) -> CoreResult<&SkillDefinition> {
        self.inner.get(skill_id)
    }

    /// 遍历 Skill 定义。
    pub fn values(&self) -> impl Iterator<Item = &SkillDefinition> {
        self.inner.values()
    }
}

/// Provider 类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    OpenAi,
    Anthropic,
    Gemini,
    OpenAiCompatible,
    Local,
    Other,
}

/// Provider 能力。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    Llm,
    Embedding,
    Reranker,
    Search,
    Streaming,
    ToolUse,
}

/// Provider 定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderDefinition {
    pub provider_id: String,
    pub provider_type: ProviderType,
    pub display_name: String,
    #[serde(default)]
    pub capabilities: Vec<ProviderCapability>,
    #[serde(default)]
    pub config_schema: Value,
}

impl RegistryItem for ProviderDefinition {
    /// provider_id 作为注册 key。
    fn registry_key(&self) -> &str {
        &self.provider_id
    }
}

/// Provider 定义注册表。
#[derive(Debug, Clone)]
pub struct ProviderRegistry {
    inner: TypedRegistry<ProviderDefinition>,
}

impl Default for ProviderRegistry {
    /// 创建空的 Provider 注册表。
    fn default() -> Self {
        Self {
            inner: TypedRegistry::new("provider"),
        }
    }
}

impl ProviderRegistry {
    /// 注册 Provider 定义。
    pub fn register(&mut self, definition: ProviderDefinition) -> CoreResult<()> {
        if definition.provider_id.trim().is_empty() {
            return Err(CoreError::validation("provider_id cannot be empty"));
        }

        self.inner.register(definition)
    }

    /// 读取 Provider 定义。
    pub fn get(&self, provider_id: &str) -> CoreResult<&ProviderDefinition> {
        self.inner.get(provider_id)
    }

    /// 遍历 Provider 定义。
    pub fn values(&self) -> impl Iterator<Item = &ProviderDefinition> {
        self.inner.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_registry_rejects_duplicates() {
        let mut registry = NodeRegistry::default();
        registry
            .register(NodeDefinition::new("llm.generate"))
            .expect("first registration should succeed");

        assert!(matches!(
            registry.register(NodeDefinition::new("llm.generate")),
            Err(CoreError::RegistryDuplicate {
                registry: "node",
                ..
            })
        ));
    }
}
