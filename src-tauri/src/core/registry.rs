use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::errors::{CoreError, CoreResult};
use crate::core::ports::PortDefinition;
use crate::core::workflow::NodeDefinition;

pub trait RegistryItem {
    fn registry_key(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct TypedRegistry<T> {
    name: &'static str,
    entries: BTreeMap<String, T>,
}

impl<T> TypedRegistry<T> {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            entries: BTreeMap::new(),
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn get(&self, key: &str) -> CoreResult<&T> {
        self.entries
            .get(key)
            .ok_or_else(|| CoreError::RegistryMissing {
                registry: self.name,
                key: key.to_owned(),
            })
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<T: RegistryItem> TypedRegistry<T> {
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
    fn registry_key(&self) -> &str {
        &self.type_name
    }
}

#[derive(Debug, Clone)]
pub struct NodeRegistry {
    inner: TypedRegistry<NodeDefinition>,
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self {
            inner: TypedRegistry::new("node"),
        }
    }
}

impl NodeRegistry {
    pub fn register(&mut self, definition: NodeDefinition) -> CoreResult<()> {
        definition.validate()?;
        self.inner.register(definition)
    }

    pub fn get(&self, type_name: &str) -> CoreResult<&NodeDefinition> {
        self.inner.get(type_name)
    }

    pub fn contains(&self, type_name: &str) -> bool {
        self.inner.contains(type_name)
    }

    pub fn values(&self) -> impl Iterator<Item = &NodeDefinition> {
        self.inner.values()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillExecutorKind {
    Llm,
    Http,
    Wasm,
}

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
    fn registry_key(&self) -> &str {
        &self.skill_id
    }
}

#[derive(Debug, Clone)]
pub struct SkillRegistry {
    inner: TypedRegistry<SkillDefinition>,
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self {
            inner: TypedRegistry::new("skill"),
        }
    }
}

impl SkillRegistry {
    pub fn register(&mut self, definition: SkillDefinition) -> CoreResult<()> {
        if definition.skill_id.trim().is_empty() {
            return Err(CoreError::validation("skill_id cannot be empty"));
        }

        self.inner.register(definition)
    }

    pub fn get(&self, skill_id: &str) -> CoreResult<&SkillDefinition> {
        self.inner.get(skill_id)
    }

    pub fn values(&self) -> impl Iterator<Item = &SkillDefinition> {
        self.inner.values()
    }
}

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
    fn registry_key(&self) -> &str {
        &self.provider_id
    }
}

#[derive(Debug, Clone)]
pub struct ProviderRegistry {
    inner: TypedRegistry<ProviderDefinition>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self {
            inner: TypedRegistry::new("provider"),
        }
    }
}

impl ProviderRegistry {
    pub fn register(&mut self, definition: ProviderDefinition) -> CoreResult<()> {
        if definition.provider_id.trim().is_empty() {
            return Err(CoreError::validation("provider_id cannot be empty"));
        }

        self.inner.register(definition)
    }

    pub fn get(&self, provider_id: &str) -> CoreResult<&ProviderDefinition> {
        self.inner.get(provider_id)
    }

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
