use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::contracts::{CoreError, CoreResult, WorkflowDefinition};

pub const EXECUTOR_ADAPTER_NODE_PREFIX: &str = "executor_adapter:";

/// 从冻结工作流图编译出的不可变执行依赖集合。
///
/// 组合根只能消费这里声明的节点类型和 Skill id，不能再把项目中发现的全部
/// Skill 当作当前运行的依赖。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowExecutionDependencySet {
    node_types: BTreeSet<String>,
    executor_adapter_skill_ids: BTreeSet<String>,
}

impl WorkflowExecutionDependencySet {
    pub fn compile(workflow: &WorkflowDefinition) -> CoreResult<Self> {
        let mut node_types = BTreeSet::new();
        let mut executor_adapter_skill_ids = BTreeSet::new();
        for node in &workflow.nodes {
            node_types.insert(node.type_name.clone());
            if let Some(skill_id) = node.type_name.strip_prefix(EXECUTOR_ADAPTER_NODE_PREFIX) {
                if skill_id.is_empty() {
                    return Err(CoreError::validation(format!(
                        "executor adapter node {} has no skill id",
                        node.id.as_str()
                    )));
                }
                executor_adapter_skill_ids.insert(skill_id.to_owned());
            }
        }
        Ok(Self {
            node_types,
            executor_adapter_skill_ids,
        })
    }

    pub fn node_types(&self) -> &BTreeSet<String> {
        &self.node_types
    }

    pub fn uses_node_type(&self, type_name: &str) -> bool {
        self.node_types.contains(type_name)
    }

    pub fn executor_adapter_skill_ids(&self) -> &BTreeSet<String> {
        &self.executor_adapter_skill_ids
    }

    pub fn uses_executor_adapters(&self) -> bool {
        !self.executor_adapter_skill_ids.is_empty()
    }
}
