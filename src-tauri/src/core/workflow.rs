use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::errors::{CoreError, CoreResult};
use crate::core::ports::{PortDefinition, PortMap};

/// 执行输入引脚名称；用于控制流和反馈流触发节点运行。
pub const EXECUTION_INPUT_PORT: &str = "exec_in";
/// 执行输出引脚名称；用于连接后续节点的执行顺序。
pub const EXECUTION_OUTPUT_PORT: &str = "exec_out";
/// 控制流引脚的类型名，和普通业务 typed port 分开展示。
pub const CONTROL_PORT_TYPE: &str = "control";
/// 单次循环迭代的最小时长，防止配置出实际不可运行的高速循环。
pub const MIN_LOOP_ITERATION_TIMEOUT_MS: u64 = 1_000;
/// feedback 边默认最多触发两次单轮返修通信，避免隐式无限循环。
pub const DEFAULT_FEEDBACK_MAX_COMMUNICATION_COUNT: u32 = 2;

/// 定义简单字符串 ID 类型，保证序列化形态稳定且避免混用裸 String。
macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// 创建新的强类型 ID。
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// 返回底层字符串引用。
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            /// 从字符串切片创建强类型 ID。
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            /// 从 String 创建强类型 ID。
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
    #[serde(default = "default_execution_input_ports")]
    pub execution_input_ports: Vec<PortDefinition>,
    #[serde(default = "default_execution_output_ports")]
    pub execution_output_ports: Vec<PortDefinition>,
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
            execution_input_ports: default_execution_input_ports(),
            execution_output_ports: default_execution_output_ports(),
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

        validate_unique_ports("execution input", &self.execution_input_ports)?;
        validate_unique_ports("execution output", &self.execution_output_ports)?;
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

/// 默认每个节点都有一个执行输入引脚，多个输入边在调度层按 AND join 处理。
fn default_execution_input_ports() -> Vec<PortDefinition> {
    vec![PortDefinition::new(
        EXECUTION_INPUT_PORT,
        CONTROL_PORT_TYPE,
        false,
    )]
}

/// 默认每个节点都有一个执行输出引脚，便于工作流显式表达运行顺序。
fn default_execution_output_ports() -> Vec<PortDefinition> {
    vec![PortDefinition::new(
        EXECUTION_OUTPUT_PORT,
        CONTROL_PORT_TYPE,
        false,
    )]
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

/// 工作流边类型：数据边传 typed port，控制边只排运行顺序，反馈边传返修通信。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEdgeKind {
    Data,
    Control,
    Feedback,
}

impl Default for WorkflowEdgeKind {
    /// 旧工作流未声明 kind 时按数据边兼容读取。
    fn default() -> Self {
        Self::Data
    }
}

/// feedback 边的通信上限；多轮审稿仍必须显式接 Loop 节点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackEdgeConfig {
    #[serde(default = "default_feedback_max_communication_count")]
    pub max_communication_count: u32,
}

impl Default for FeedbackEdgeConfig {
    /// feedback 直连默认只允许有限通信。
    fn default() -> Self {
        Self {
            max_communication_count: DEFAULT_FEEDBACK_MAX_COMMUNICATION_COUNT,
        }
    }
}

impl FeedbackEdgeConfig {
    /// 校验 feedback 通信必须有非零上限。
    pub fn validate(&self) -> CoreResult<()> {
        if self.max_communication_count == 0 {
            return Err(CoreError::validation(
                "feedback edge max_communication_count must be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    #[serde(default)]
    pub kind: WorkflowEdgeKind,
    pub from: PortEndpoint,
    pub to: PortEndpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<FeedbackEdgeConfig>,
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
        let mut data_aliases = BTreeSet::new();
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

            validate_edge_endpoint("source", &edge.from)?;
            validate_edge_endpoint("target", &edge.to)?;
            validate_edge_kind(edge)?;

            if let Some(alias) = &edge.alias {
                if alias.trim().is_empty() {
                    return Err(CoreError::validation(format!(
                        "edge {} alias cannot be empty",
                        edge.id.as_str()
                    )));
                }
                if edge.kind != WorkflowEdgeKind::Data {
                    return Err(CoreError::validation(format!(
                        "edge {} alias is only allowed on data edges",
                        edge.id.as_str()
                    )));
                }
                let key = (edge.to.node_id.as_str().to_owned(), alias.trim().to_owned());
                if !data_aliases.insert(key) {
                    return Err(CoreError::validation(format!(
                        "duplicate input alias for node {}: {}",
                        edge.to.node_id.as_str(),
                        alias.trim()
                    )));
                }
            }
        }

        Ok(())
    }
}

/// 校验边端点端口名，避免保存不可诊断的空引脚。
fn validate_edge_endpoint(kind: &str, endpoint: &PortEndpoint) -> CoreResult<()> {
    if endpoint.port_name.trim().is_empty() {
        return Err(CoreError::validation(format!(
            "{kind} edge endpoint port_name cannot be empty"
        )));
    }
    Ok(())
}

/// 校验不同边类型的最低结构约束；端口是否存在由注册表/执行器继续校验。
fn validate_edge_kind(edge: &Edge) -> CoreResult<()> {
    if edge.kind == WorkflowEdgeKind::Control
        && (edge.from.port_name != EXECUTION_OUTPUT_PORT
            || edge.to.port_name != EXECUTION_INPUT_PORT)
    {
        return Err(CoreError::validation(format!(
            "control edge {} must connect {EXECUTION_OUTPUT_PORT} to {EXECUTION_INPUT_PORT}",
            edge.id.as_str()
        )));
    }
    if edge.kind == WorkflowEdgeKind::Feedback {
        let Some(config) = &edge.feedback else {
            return Err(CoreError::validation(format!(
                "feedback edge {} requires max_communication_count",
                edge.id.as_str()
            )));
        };
        config.validate()?;
    } else if edge.feedback.is_some() {
        return Err(CoreError::validation(format!(
            "edge {} feedback config is only allowed on feedback edges",
            edge.id.as_str()
        )));
    }
    Ok(())
}

/// serde 默认函数，保持字段缺省时为有限通信次数。
fn default_feedback_max_communication_count() -> u32 {
    DEFAULT_FEEDBACK_MAX_COMMUNICATION_COUNT
}

/// 计算非零除数的向上取整除法。
fn ceil_div_u64(value: u64, divisor: u64) -> u64 {
    value / divisor + u64::from(value % divisor != 0)
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

        // 这里按向上取整估算每轮时长，避免 1999ms/2 这类边界被整数除法误判。
        let per_iteration_ms = ceil_div_u64(self.timeout_ms, u64::from(self.max_iterations));
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
    fn loop_policy_uses_ceiling_division_for_iteration_timeout() {
        let policy = LoopPolicy {
            max_iterations: 2,
            timeout_ms: 1_999,
            budget_limit_usd: Some(1.0),
            stop_condition: json!({ "kind": "score_at_least", "value": 0.95 }),
        };

        assert!(policy.validate().is_ok());
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
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("node-1"),
                    port_name: "out".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("missing"),
                    port_name: "in".to_owned(),
                },
                alias: None,
                feedback: None,
            }],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }

    #[test]
    fn node_definition_has_default_execution_ports() {
        let node = NodeDefinition::new("writer");

        assert_eq!(node.execution_input_ports[0].name, EXECUTION_INPUT_PORT);
        assert_eq!(node.execution_output_ports[0].name, EXECUTION_OUTPUT_PORT);
        assert!(node.validate().is_ok());
    }

    #[test]
    fn workflow_control_edges_must_use_execution_ports() {
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("wf-1"),
            name: "Test".to_owned(),
            nodes: vec![
                NodeInstance {
                    id: NodeId::from("node-1"),
                    type_name: "writer".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
                NodeInstance {
                    id: NodeId::from("node-2"),
                    type_name: "summarizer".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
            ],
            edges: vec![Edge {
                id: EdgeId::from("edge-1"),
                kind: WorkflowEdgeKind::Control,
                from: PortEndpoint {
                    node_id: NodeId::from("node-1"),
                    port_name: "draft".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("node-2"),
                    port_name: EXECUTION_INPUT_PORT.to_owned(),
                },
                alias: None,
                feedback: None,
            }],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }

    #[test]
    fn workflow_rejects_duplicate_data_aliases_for_same_target() {
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("wf-1"),
            name: "Test".to_owned(),
            nodes: vec![
                NodeInstance {
                    id: NodeId::from("source-1"),
                    type_name: "planner".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
                NodeInstance {
                    id: NodeId::from("source-2"),
                    type_name: "detail".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
                NodeInstance {
                    id: NodeId::from("writer"),
                    type_name: "writer".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
            ],
            edges: vec![
                Edge {
                    id: EdgeId::from("edge-1"),
                    kind: WorkflowEdgeKind::Data,
                    from: PortEndpoint {
                        node_id: NodeId::from("source-1"),
                        port_name: "outline".to_owned(),
                    },
                    to: PortEndpoint {
                        node_id: NodeId::from("writer"),
                        port_name: "prompt_input".to_owned(),
                    },
                    alias: Some("本章大纲".to_owned()),
                    feedback: None,
                },
                Edge {
                    id: EdgeId::from("edge-2"),
                    kind: WorkflowEdgeKind::Data,
                    from: PortEndpoint {
                        node_id: NodeId::from("source-2"),
                        port_name: "details".to_owned(),
                    },
                    to: PortEndpoint {
                        node_id: NodeId::from("writer"),
                        port_name: "prompt_input".to_owned(),
                    },
                    alias: Some("本章大纲".to_owned()),
                    feedback: None,
                },
            ],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }

    #[test]
    fn workflow_feedback_edges_require_bounded_config() {
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("wf-1"),
            name: "Test".to_owned(),
            nodes: vec![
                NodeInstance {
                    id: NodeId::from("prudent"),
                    type_name: "prudent".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
                NodeInstance {
                    id: NodeId::from("writer"),
                    type_name: "writer".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
            ],
            edges: vec![Edge {
                id: EdgeId::from("feedback-1"),
                kind: WorkflowEdgeKind::Feedback,
                from: PortEndpoint {
                    node_id: NodeId::from("prudent"),
                    port_name: "revision_context".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: "feedback_in".to_owned(),
                },
                alias: None,
                feedback: None,
            }],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }
}
