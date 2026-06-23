use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::errors::{CoreError, CoreResult};
use crate::core::ports::{PortDefinition, PortMap};

/// 单次循环迭代的最小时长，防止配置出实际不可运行的高速循环。
pub const MIN_LOOP_ITERATION_TIMEOUT_MS: u64 = 1_000;

/// 定义简单字符串 ID 类型，保证序列化形态稳定且避免混用裸 String。
macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

string_id!(WorkflowId);
string_id!(RunId);
string_id!(NodeId);
string_id!(EdgeId);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDefinition {
    pub type_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub input_ports: Vec<PortDefinition>,
    #[serde(default)]
    pub output_ports: Vec<PortDefinition>,
    pub supports_checkpoint: bool,
    pub supports_auto_approval: bool,
}

impl NodeDefinition {
    /// 创建节点类型定义。
    pub fn new(type_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            display_name: None,
            input_ports: Vec::new(),
            output_ports: Vec::new(),
            supports_checkpoint: false,
            supports_auto_approval: false,
        }
    }

    /// 校验节点类型名和输入/输出端口定义。
    pub fn validate(&self) -> CoreResult<()> {
        if self.type_name.trim().is_empty() {
            return Err(CoreError::validation("node type_name cannot be empty"));
        }

        validate_unique_ports("input", &self.input_ports)?;
        validate_unique_ports("output", &self.output_ports)?;
        Ok(())
    }
}

/// 校验同一方向的端口名不为空且不重复。
fn validate_unique_ports(kind: &str, ports: &[PortDefinition]) -> CoreResult<()> {
    let mut names = BTreeSet::new();
    for port in ports {
        if port.name.trim().is_empty() {
            return Err(CoreError::validation(format!(
                "{kind} port name cannot be empty"
            )));
        }

        if !names.insert(port.name.as_str()) {
            return Err(CoreError::validation(format!(
                "duplicate {kind} port name: {}",
                port.name
            )));
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CanvasPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeInstance {
    pub id: NodeId,
    pub type_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub config: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<CanvasPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortEndpoint {
    pub node_id: NodeId,
    pub port_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub from: PortEndpoint,
    pub to: PortEndpoint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: WorkflowId,
    pub name: String,
    #[serde(default)]
    pub nodes: Vec<NodeInstance>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub metadata: Value,
}

impl WorkflowDefinition {
    /// 校验节点和边的拓扑引用关系。
    pub fn validate_topology(&self) -> CoreResult<()> {
        let mut node_ids = BTreeSet::new();
        for node in &self.nodes {
            if !node_ids.insert(node.id.as_str()) {
                return Err(CoreError::validation(format!(
                    "duplicate node id: {}",
                    node.id.as_str()
                )));
            }
        }

        let mut edge_ids = BTreeSet::new();
        for edge in &self.edges {
            if !edge_ids.insert(edge.id.as_str()) {
                return Err(CoreError::validation(format!(
                    "duplicate edge id: {}",
                    edge.id.as_str()
                )));
            }

            if !node_ids.contains(edge.from.node_id.as_str()) {
                return Err(CoreError::validation(format!(
                    "edge {} references missing source node {}",
                    edge.id.as_str(),
                    edge.from.node_id.as_str()
                )));
            }

            if !node_ids.contains(edge.to.node_id.as_str()) {
                return Err(CoreError::validation(format!(
                    "edge {} references missing target node {}",
                    edge.id.as_str(),
                    edge.to.node_id.as_str()
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunControl {
    /// 继续执行。
    Continue,
    /// 暂停并保留可恢复状态。
    Pause,
    /// 停止当前运行，但保留已完成产物。
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Paused,
    Stopping,
    Stopped,
    Succeeded,
    Failed,
}

impl RunStatus {
    /// 判断运行状态是否已经不可继续迁移。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Succeeded | Self::Failed)
    }
}

/// Loop 节点的硬限制策略。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopPolicy {
    pub max_iterations: u32,
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_limit_usd: Option<f64>,
    pub stop_condition: Value,
}

impl LoopPolicy {
    /// 校验循环自身是否具备边界条件、超时和可选预算限制。
    pub fn validate(&self) -> CoreResult<()> {
        if self.max_iterations == 0 {
            return Err(CoreError::validation(
                "loop policy requires max_iterations greater than zero",
            ));
        }

        if self.timeout_ms == 0 {
            return Err(CoreError::validation(
                "loop policy requires timeout_ms greater than zero",
            ));
        }

        // 这里用总超时除以最大轮次做保守估算，避免配置出每轮不足 1 秒的循环。
        let per_iteration_ms = self.timeout_ms / u64::from(self.max_iterations);
        if per_iteration_ms < MIN_LOOP_ITERATION_TIMEOUT_MS {
            return Err(CoreError::validation(format!(
                "loop policy timeout_ms {} is too small for {} iterations; at least {}ms per iteration is required",
                self.timeout_ms, self.max_iterations, MIN_LOOP_ITERATION_TIMEOUT_MS
            )));
        }

        if let Some(limit) = self.budget_limit_usd {
            if !limit.is_finite() || limit < 0.0 {
                return Err(CoreError::validation(
                    "loop policy budget_limit_usd must be finite and non-negative",
                ));
            }
        }

        if self.stop_condition.is_null() {
            return Err(CoreError::validation(
                "loop policy requires a non-null stop_condition",
            ));
        }

        Ok(())
    }

    /// 在全局 workflow 限制下校验单个 loop policy。
    pub fn validate_against_limits(
        &self,
        max_loop_iterations: u32,
        max_timeout_ms: u64,
    ) -> CoreResult<()> {
        self.validate()?;

        if max_loop_iterations == 0 {
            return Err(CoreError::validation(
                "workflow max_loop_iterations cannot be zero",
            ));
        }

        if max_timeout_ms == 0 {
            return Err(CoreError::validation(
                "workflow max_timeout_ms cannot be zero",
            ));
        }

        if self.max_iterations > max_loop_iterations {
            return Err(CoreError::validation(format!(
                "loop max_iterations {} exceeds workflow limit {}",
                self.max_iterations, max_loop_iterations
            )));
        }

        if self.timeout_ms > max_timeout_ms {
            return Err(CoreError::validation(format!(
                "loop timeout_ms {} exceeds workflow timeout limit {}",
                self.timeout_ms, max_timeout_ms
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRunState {
    pub node_id: NodeId,
    pub status: RunStatus,
    #[serde(default)]
    pub outputs: PortMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn loop_policy_rejects_unbounded_loop() {
        let policy = LoopPolicy {
            max_iterations: 0,
            timeout_ms: 2_000,
            budget_limit_usd: Some(1.0),
            stop_condition: json!({ "kind": "score_at_least", "value": 0.95 }),
        };

        assert!(policy.validate().is_err());
    }

    #[test]
    fn loop_policy_rejects_unrealistic_iteration_timeout() {
        let policy = LoopPolicy {
            max_iterations: 10,
            timeout_ms: 5_000,
            budget_limit_usd: Some(1.0),
            stop_condition: json!({ "kind": "score_at_least", "value": 0.95 }),
        };

        assert!(policy.validate().is_err());
    }

    #[test]
    fn loop_policy_validates_against_workflow_limits() {
        let policy = LoopPolicy {
            max_iterations: 6,
            timeout_ms: 60_000,
            budget_limit_usd: Some(1.0),
            stop_condition: json!({ "kind": "score_at_least", "value": 0.95 }),
        };

        assert!(policy.validate_against_limits(5, 60_000).is_err());
    }

    #[test]
    fn workflow_topology_rejects_missing_node_reference() {
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("wf-1"),
            name: "Test".to_owned(),
            nodes: vec![NodeInstance {
                id: NodeId::from("node-1"),
                type_name: "llm".to_owned(),
                label: None,
                config: Value::Null,
                position: None,
            }],
            edges: vec![Edge {
                id: EdgeId::from("edge-1"),
                from: PortEndpoint {
                    node_id: NodeId::from("node-1"),
                    port_name: "out".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("missing"),
                    port_name: "in".to_owned(),
                },
            }],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }
}
