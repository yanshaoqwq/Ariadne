use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::errors::{CoreError, CoreResult};
use crate::contracts::ports::{PortDefinition, PortMap};

/// 执行输入引脚名称；用于控制流触发节点运行。
pub const EXECUTION_INPUT_PORT: &str = "exec_in";
/// 执行输出引脚名称；用于连接后续节点的执行顺序。
pub const EXECUTION_OUTPUT_PORT: &str = "exec_out";
/// 默认通信引脚名称；UI 放在节点正上方。
pub const COMMUNICATION_PORT: &str = "communication";
/// 控制流引脚的类型名，和普通业务 typed port 分开展示。
pub const CONTROL_PORT_TYPE: &str = "control";
/// 通信引脚类型名，和普通业务 typed port 分开展示。
pub const COMMUNICATION_PORT_TYPE: &str = "communication";
/// 单次循环迭代的最小时长，防止配置出实际不可运行的高速循环。
pub const MIN_LOOP_ITERATION_TIMEOUT_MS: u64 = 1_000;
/// communication 边默认最多触发两条消息，避免隐式无限循环。
pub const DEFAULT_COMMUNICATION_MAX_MESSAGE_COUNT: u32 = 2;

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

/// 节点类型定义，描述画布节点的控制、通信和业务端口。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDefinition {
    pub type_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default = "default_execution_input_ports")]
    pub execution_input_ports: Vec<PortDefinition>,
    #[serde(default = "default_execution_output_ports")]
    pub execution_output_ports: Vec<PortDefinition>,
    #[serde(default = "default_communication_ports")]
    pub communication_ports: Vec<PortDefinition>,
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
            communication_ports: default_communication_ports(),
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
        validate_unique_ports("communication", &self.communication_ports)?;
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

/// 默认每个节点都有一个通信引脚；后续 UI 可按节点定义隐藏或扩展更多通信引脚。
fn default_communication_ports() -> Vec<PortDefinition> {
    vec![PortDefinition::new(
        COMMUNICATION_PORT,
        COMMUNICATION_PORT_TYPE,
        false,
    )]
}

/// 画布上的节点位置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CanvasPosition {
    pub x: f64,
    pub y: f64,
}

/// 工作流中的单个节点实例。
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

/// 边连接的节点端口端点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortEndpoint {
    pub node_id: NodeId,
    pub port_name: String,
}

/// 工作流边类型：数据边传 typed port，控制边只排运行顺序，通信边传返修消息。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEdgeKind {
    Data,
    Control,
    #[serde(alias = "feedback")]
    Communication,
}

impl Default for WorkflowEdgeKind {
    /// 旧工作流未声明 kind 时按数据边兼容读取。
    fn default() -> Self {
        Self::Data
    }
}

/// communication 边的通信配置；多轮审稿仍必须显式接 Loop 节点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicationEdgeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiator_node_id: Option<NodeId>,
    #[serde(default = "default_forward_alias")]
    pub forward_alias: String,
    #[serde(default = "default_reverse_alias")]
    pub reverse_alias: String,
    #[serde(default = "default_forward_template")]
    pub forward_template: String,
    #[serde(default = "default_reverse_template")]
    pub reverse_template: String,
    #[serde(default = "default_communication_max_message_count")]
    pub max_communication_count: u32,
}

impl Default for CommunicationEdgeConfig {
    /// communication 直连默认只允许有限通信。
    fn default() -> Self {
        Self {
            initiator_node_id: None,
            forward_alias: default_forward_alias(),
            reverse_alias: default_reverse_alias(),
            forward_template: default_forward_template(),
            reverse_template: default_reverse_template(),
            max_communication_count: DEFAULT_COMMUNICATION_MAX_MESSAGE_COUNT,
        }
    }
}

impl CommunicationEdgeConfig {
    /// 校验 communication 通信必须有方向、模板和非零上限。
    pub fn validate_for_edge(&self, edge: &Edge) -> CoreResult<()> {
        if self.max_communication_count == 0 {
            return Err(CoreError::validation(
                "communication edge max_communication_count must be greater than zero",
            ));
        }
        validate_non_empty("communication forward_alias", &self.forward_alias)?;
        validate_non_empty("communication reverse_alias", &self.reverse_alias)?;
        validate_non_empty("communication forward_template", &self.forward_template)?;
        validate_non_empty("communication reverse_template", &self.reverse_template)?;

        if !self
            .forward_template
            .contains(&format!("{{{{input.{}}}}}", self.forward_alias))
        {
            return Err(CoreError::validation(
                "communication forward_template must reference forward_alias",
            ));
        }
        if !self
            .reverse_template
            .contains(&format!("{{{{input.{}}}}}", self.reverse_alias))
        {
            return Err(CoreError::validation(
                "communication reverse_template must reference reverse_alias",
            ));
        }

        if let Some(initiator) = &self.initiator_node_id {
            if initiator != &edge.from.node_id && initiator != &edge.to.node_id {
                return Err(CoreError::validation(format!(
                    "communication edge {} initiator must be one of its endpoint nodes",
                    edge.id.as_str()
                )));
            }
        }
        Ok(())
    }

    /// 返回发起节点；旧 feedback 配置缺少方向时按 source 端兼容迁移。
    pub fn initiator_for_edge<'a>(&'a self, edge: &'a Edge) -> &'a NodeId {
        self.initiator_node_id
            .as_ref()
            .unwrap_or(&edge.from.node_id)
    }
}

/// 旧名称保留为类型别名，便于分阶段迁移调用方。
pub type FeedbackEdgeConfig = CommunicationEdgeConfig;

/// 工作流边定义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    #[serde(default)]
    pub kind: WorkflowEdgeKind,
    pub from: PortEndpoint,
    pub to: PortEndpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, alias = "feedback", skip_serializing_if = "Option::is_none")]
    pub communication: Option<CommunicationEdgeConfig>,
}

/// 完整工作流定义，包含节点、边和附加元数据。
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
    if edge.kind == WorkflowEdgeKind::Communication {
        if edge.from.port_name != COMMUNICATION_PORT || edge.to.port_name != COMMUNICATION_PORT {
            return Err(CoreError::validation(format!(
                "communication edge {} must connect {COMMUNICATION_PORT} pins",
                edge.id.as_str()
            )));
        }
        let Some(config) = &edge.communication else {
            return Err(CoreError::validation(format!(
                "communication edge {} requires configuration",
                edge.id.as_str()
            )));
        };
        config.validate_for_edge(edge)?;
    } else if edge.communication.is_some() {
        return Err(CoreError::validation(format!(
            "edge {} communication config is only allowed on communication edges",
            edge.id.as_str()
        )));
    }
    Ok(())
}

/// serde 默认函数，保持字段缺省时为有限通信次数。
fn default_communication_max_message_count() -> u32 {
    DEFAULT_COMMUNICATION_MAX_MESSAGE_COUNT
}

/// communication 正向消息默认输入别名。
fn default_forward_alias() -> String {
    "forward_output".to_owned()
}

/// communication 反向消息默认输入别名。
fn default_reverse_alias() -> String {
    "reverse_output".to_owned()
}

/// communication 正向消息默认提示词模板。
fn default_forward_template() -> String {
    "这是对你的文章提出的意见，你需要合理汲取并作出改进：{{input.forward_output}}".to_owned()
}

/// communication 反向消息默认提示词模板。
fn default_reverse_template() -> String {
    "这是改进，请你检查还有没有哪里需要改进：{{input.reverse_output}}".to_owned()
}

/// 校验字符串字段非空。
fn validate_non_empty(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }
    Ok(())
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

/// 工作流运行状态。
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

/// 旧版节点运行快照结构；保留给已有调用方兼容。
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

    /// 验证 LoopPolicy 拒绝没有最大迭代次数的无界循环。
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

    /// 验证 LoopPolicy 拒绝平均每轮时间过短的配置。
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

    /// 验证 LoopPolicy 用向上取整计算每轮最小时长。
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

    /// 验证 LoopPolicy 会受到工作流全局限制约束。
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

    /// 验证工作流拓扑会拒绝缺失节点引用。
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
                communication: None,
            }],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }

    /// 验证节点定义默认带执行输入、执行输出和通信引脚。
    #[test]
    fn node_definition_has_default_execution_ports() {
        let node = NodeDefinition::new("writer");

        assert_eq!(node.execution_input_ports[0].name, EXECUTION_INPUT_PORT);
        assert_eq!(node.execution_output_ports[0].name, EXECUTION_OUTPUT_PORT);
        assert_eq!(node.communication_ports[0].name, COMMUNICATION_PORT);
        assert!(node.validate().is_ok());
    }

    /// 验证控制边必须连接固定执行引脚。
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
                communication: None,
            }],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }

    /// 验证同一目标节点不能收到重复 data alias。
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
                    communication: None,
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
                    communication: None,
                },
            ],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }

    /// 验证通信边必须提供有界通信配置。
    #[test]
    fn workflow_communication_edges_require_bounded_config() {
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
                id: EdgeId::from("communication-1"),
                kind: WorkflowEdgeKind::Communication,
                from: PortEndpoint {
                    node_id: NodeId::from("prudent"),
                    port_name: COMMUNICATION_PORT.to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: COMMUNICATION_PORT.to_owned(),
                },
                alias: None,
                communication: None,
            }],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }

    /// 验证通信边只能连接通信引脚。
    #[test]
    fn workflow_communication_edges_require_communication_pins() {
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
                id: EdgeId::from("communication-1"),
                kind: WorkflowEdgeKind::Communication,
                from: PortEndpoint {
                    node_id: NodeId::from("prudent"),
                    port_name: "revision_context".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: COMMUNICATION_PORT.to_owned(),
                },
                alias: None,
                communication: Some(CommunicationEdgeConfig::default()),
            }],
            metadata: Value::Null,
        };

        assert!(workflow.validate_topology().is_err());
    }

    /// 验证旧 feedback kind 能兼容读取为 communication。
    #[test]
    fn workflow_reads_legacy_feedback_kind_as_communication() {
        let json = json!({
            "id": "wf-legacy",
            "name": "Legacy",
            "nodes": [
                { "id": "prudent", "type_name": "prudent", "config": null },
                { "id": "writer", "type_name": "writer", "config": null }
            ],
            "edges": [{
                "id": "feedback-1",
                "kind": "feedback",
                "from": { "node_id": "prudent", "port_name": "communication" },
                "to": { "node_id": "writer", "port_name": "communication" },
                "feedback": { "max_communication_count": 2 }
            }],
            "metadata": null
        });
        let workflow: WorkflowDefinition = serde_json::from_value(json).unwrap();

        assert_eq!(workflow.edges[0].kind, WorkflowEdgeKind::Communication);
        assert!(workflow.edges[0].communication.is_some());
        assert!(workflow.validate_topology().is_ok());
    }
}
