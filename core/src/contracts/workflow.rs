use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::errors::{CoreError, CoreResult};
use crate::contracts::ports::PortDefinition;

/// 执行输入引脚名称；用于控制流触发节点运行。
pub const EXECUTION_INPUT_PORT: &str = "exec_in";
/// 执行输出引脚名称；用于连接后续节点的执行顺序。
pub const EXECUTION_OUTPUT_PORT: &str = "exec_out";
/// U125：condition/eval 节点「条件成立」分支的执行输出引脚。
///
/// 分支语义由**引脚身份**承载，而不是让用户去边上填一个自由文本标签：
/// 标签可以留空（留空时旧实现直接放行下游 → condition 形同虚设），
/// 而引脚不可能留空，也不可能填错——错误状态在 UI 上根本不存在。
pub const EXECUTION_OUTPUT_PORT_TRUE: &str = "exec_out_true";
/// U125：condition/eval 节点「条件不成立」分支的执行输出引脚。
pub const EXECUTION_OUTPUT_PORT_FALSE: &str = "exec_out_false";
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

// U116：原有 `pub type FeedbackEdgeConfig = CommunicationEdgeConfig;` 已删除，零引用。
// 注释称「便于分阶段迁移调用方」，但迁移早已完成——全仓只用 `CommunicationEdgeConfig`。

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
                if edge.kind == WorkflowEdgeKind::Communication {
                    return Err(CoreError::validation(format!(
                        "edge {} alias is not allowed on communication edges",
                        edge.id.as_str()
                    )));
                }
                if edge.kind == WorkflowEdgeKind::Data {
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

/// U125：判断端口名是否为合法的执行输出引脚（含 condition 的两个分支引脚）。
pub fn is_execution_output_port(port_name: &str) -> bool {
    matches!(
        port_name,
        EXECUTION_OUTPUT_PORT | EXECUTION_OUTPUT_PORT_TRUE | EXECUTION_OUTPUT_PORT_FALSE
    )
}

/// U125：把 condition 分支引脚名映射为 `branch` 输出值（`"true"` / `"false"`）。
///
/// **分支值 ↔ 引脚名的对应关系只在这里出现一次**：runtime 的门禁、保存边界校验、
/// 节点目录都取自本函数，避免同一映射散落多处后各自漂移。
/// 通用 `exec_out` 不承载分支语义，返回 `None`。
pub fn condition_branch_for_port(port_name: &str) -> Option<&'static str> {
    match port_name {
        EXECUTION_OUTPUT_PORT_TRUE => Some("true"),
        EXECUTION_OUTPUT_PORT_FALSE => Some("false"),
        _ => None,
    }
}

/// 校验不同边类型的最低结构约束；端口是否存在由注册表/执行器继续校验。
fn validate_edge_kind(edge: &Edge) -> CoreResult<()> {
    // U125：控制边的源引脚放开为「通用执行出口 + 两个 condition 分支出口」三者之一。
    // 这里只能做结构校验——本函数拿不到节点类型，无从判断源节点是否真是 condition，
    // 「只有 condition 节点可用分支引脚」那一条由 `validate_workflow_execution_contracts`
    // 在有节点上下文的地方把住。目标引脚仍必须是 `exec_in`，不随之放开。
    if edge.kind == WorkflowEdgeKind::Control
        && (!is_execution_output_port(&edge.from.port_name)
            || edge.to.port_name != EXECUTION_INPUT_PORT)
    {
        return Err(CoreError::validation(format!(
            "control edge {} must connect an execution output port \
             ({EXECUTION_OUTPUT_PORT}/{EXECUTION_OUTPUT_PORT_TRUE}/{EXECUTION_OUTPUT_PORT_FALSE}) \
             to {EXECUTION_INPUT_PORT}",
            edge.id.as_str()
        )));
    }
    if edge.kind == WorkflowEdgeKind::Data {
        match edge.alias.as_deref().map(str::trim) {
            Some(alias) if !alias.is_empty() => {}
            _ => {
                return Err(CoreError::validation(format!(
                    "data edge {} requires a non-empty alias",
                    edge.id.as_str()
                )));
            }
        }
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
    value / divisor + u64::from(!value.is_multiple_of(divisor))
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

/// 工作流全局执行限制。
///
/// U113：`WorkflowConfig` 的三个限制字段过去没有统一消费入口——`max_tool_rounds`
/// 被单独传参、`default_timeout_ms` 被运行时硬编码成 120s、`max_loop_iterations`
/// 的唯一可达路径 `validate_loop_policy` 零调用者。三者各行其是，用户在设置页
/// 改动后不产生任何效果。这里把它们收敛成一个值对象，作为「工作流全局限制」的
/// 唯一事实源：预检、节点超时回落和 tool-use 轮次上限都只能从它读取。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowExecutionLimits {
    /// 节点未显式声明超时时的回落值，同时是 loop 超时的上限。
    pub default_timeout_ms: u64,
    /// 单个 loop 节点允许声明的最大轮次。
    pub max_loop_iterations: u32,
    /// 单次 LLM 调用允许的最大 tool-use 往返轮次。
    pub max_tool_rounds: u32,
}

impl Default for WorkflowExecutionLimits {
    /// 与 `WorkflowConfig::default()` 保持同一组出厂值；两者若漂移，
    /// `config` 层的 `execution_limits()` 合同测试会失败。
    fn default() -> Self {
        Self {
            default_timeout_ms: 300_000,
            max_loop_iterations: 5,
            max_tool_rounds: 8,
        }
    }
}

impl WorkflowExecutionLimits {
    /// 校验限制自身可用；零值意味着「任何节点都跑不起来」，必须在保存边界就被拒绝。
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

    /// 节点超时回落：节点显式声明优先，缺省时用全局配置值而非硬编码常量。
    pub fn resolve_node_timeout_ms(&self, node_timeout_ms: Option<u64>) -> u64 {
        node_timeout_ms
            .filter(|value| *value > 0)
            .unwrap_or(self.default_timeout_ms)
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
    pub fn validate_within(&self, limits: &WorkflowExecutionLimits) -> CoreResult<()> {
        self.validate_against_limits(limits.max_loop_iterations, limits.default_timeout_ms)
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

/// 工作流变量类型。三类都允许带默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowVariableKind {
    Number,
    String,
    Boolean,
}

impl WorkflowVariableKind {
    /// 判断取值是否符合本类型。不做隐式转换：字符串 "3" 不是 number。
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            Self::Number => value.is_number(),
            Self::String => value.is_string(),
            Self::Boolean => value.is_boolean(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::String => "string",
            Self::Boolean => "boolean",
        }
    }
}

/// 工作流变量声明，挂在 `start` 节点配置上。
///
/// 变量名由用户自己写，runtime 不预置 `chapter_index` 之类的固定字段。
/// `required` 与 `hidden` 是两个独立开关：`hidden` 变量仍可带默认值，
/// `required` 变量的默认值作为执行页预填值。同时置两者是矛盾声明
/// （要求人工输入却不展示），校验期直接拒绝。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowVariableDecl {
    pub name: String,
    pub kind: WorkflowVariableKind,
    #[serde(default)]
    pub default: Value,
    /// 执行页必须由人工确认或修改后才能启动。
    #[serde(default)]
    pub required: bool,
    /// 执行页不展示，只走默认值或循环体写回。
    #[serde(default)]
    pub hidden: bool,
}

impl WorkflowVariableDecl {
    /// 校验单条声明：变量名合法、默认值类型匹配、开关组合不矛盾。
    pub fn validate(&self) -> CoreResult<()> {
        validate_variable_name(&self.name)?;

        if self.required && self.hidden {
            return Err(CoreError::validation(format!(
                "workflow variable {} cannot be both required and hidden",
                self.name
            )));
        }

        // default 允许省略（null）；一旦给出就必须与声明类型一致。
        if !self.default.is_null() && !self.kind.matches(&self.default) {
            return Err(CoreError::validation(format!(
                "workflow variable {} default value does not match kind {}",
                self.name,
                self.kind.as_str()
            )));
        }

        // 这里**不校验**「required 但无 default」。
        // required 的语义是「占位符不能替换成空白」，不是「必须人工确认」——
        // 正因为没有默认值，才逼着每次运行前填上。曾按「必须确认」误加过
        // 「required 必须带 default 以供预填」的规则，那会把最常用的写法禁掉。
        // 非空要求在启动前由 ensure_required_variables_present 校验。

        Ok(())
    }
}

/// 变量名规则：非空、仅 ASCII 字母数字下划线、不以数字开头。
///
/// 收紧到这个集合是因为变量要进 `{{var.名字}}` 模板：允许点号会与命名空间
/// 分隔符冲突，允许空格或花括号会让模板无法稳定解析。
pub fn validate_variable_name(name: &str) -> CoreResult<()> {
    if name.is_empty() {
        return Err(CoreError::validation(
            "workflow variable name cannot be empty",
        ));
    }

    let mut chars = name.chars();
    let first = chars.next().unwrap_or_default();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(CoreError::validation(format!(
            "workflow variable name {name} must start with a letter or underscore"
        )));
    }

    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
    {
        return Err(CoreError::validation(format!(
            "workflow variable name {name} contains unsupported character {bad:?}"
        )));
    }

    Ok(())
}

/// 判定取值对 `required` 而言是否算「空白」。
///
/// 只有 `null` 与空白字符串算空：`0` 与 `false` 是合法取值，不能被拦
/// （`chapter=0` 合法，`polish=false` 也合法）。判据是「替换进
/// `{{var.x}}` 后会不会在稿子里留下一个洞」，不是「值看起来重不重要」。
pub fn variable_value_is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        _ => false,
    }
}

/// 渲染摘要句式：把 `{{var.名字}}` 替换成当前取值。
///
/// 用于执行页折叠态显示「写第 3 章《雪落时》，落笔要克制」这样一句话。
/// 与提示词模板同一套 `{{var.x}}` 语法，作者一眼能对上执行页显示的句子
/// 与实际送进 LLM 的模板。
///
/// 与 `resolve_variable` 的关键差异：**缺失取值不报错**，就地留空。
/// 折叠行是给人看的预览，`required` 变量留空时那个空洞本身就是
/// 「稿子里会缺这一块」的提示；提示词渲染路径仍然严格报缺失变量。
/// 允许引用 `hidden` 变量：取值是真实的，只是作者在表单里改不到。
pub fn render_summary_template(template: &str, values: &BTreeMap<String, Value>) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find("{{") {
        rendered.push_str(&rest[..open]);
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("}}") else {
            // 没有闭合括号：剩余部分按字面量输出，不吞掉作者写的内容。
            rendered.push_str(&rest[open..]);
            return rendered;
        };

        let name = after_open[..close].trim();
        match name.strip_prefix("var.") {
            Some(variable) => {
                if let Some(value) = values.get(variable.trim()) {
                    rendered.push_str(&summary_value_text(value));
                }
                // 无取值：留空，不输出占位符本身。
            }
            // 非 var. 命名空间不属摘要句式的职责，原样保留便于排查。
            None => rendered.push_str(&rest[open..open + 2 + close + 2]),
        }

        rest = &after_open[close + 2..];
    }

    rendered.push_str(rest);
    rendered
}

/// 摘要句式里的取值文本：字符串取原文，其余走 JSON 字面量。
///
/// 字符串不能带 JSON 引号，否则句子会渲染成「《"雪落时"》」。
fn summary_value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// 校验一组变量声明：逐条合法且名字不重复。
pub fn validate_variable_decls(decls: &[WorkflowVariableDecl]) -> CoreResult<()> {
    let mut seen = BTreeSet::new();
    for decl in decls {
        decl.validate()?;
        if !seen.insert(decl.name.as_str()) {
            return Err(CoreError::validation(format!(
                "duplicate workflow variable name: {}",
                decl.name
            )));
        }
    }
    Ok(())
}

// U116：原有 `pub struct NodeRunState` 已删除，零引用（生产、测试、桌面端皆无）。
// 注释称「保留给已有调用方兼容」，但那些调用方并不存在——运行态快照实际用的是
// `workflow/runtime.rs` 的 `WorkflowNodeRuntimeState`（字段更全：通信输出、
// patch 会话、重试计数等）。两个形似的结构并存只会让人选错。

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

    /// 验证 data 边必须声明输入 alias，避免运行时输入静默丢失。
    #[test]
    fn workflow_rejects_data_edge_without_alias() {
        let workflow = WorkflowDefinition {
            id: WorkflowId::from("wf-1"),
            name: "Test".to_owned(),
            nodes: vec![
                NodeInstance {
                    id: NodeId::from("source"),
                    type_name: "llm".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
                NodeInstance {
                    id: NodeId::from("target"),
                    type_name: "llm".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
            ],
            edges: vec![Edge {
                id: EdgeId::from("edge-1"),
                kind: WorkflowEdgeKind::Data,
                from: PortEndpoint {
                    node_id: NodeId::from("source"),
                    port_name: "out".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("target"),
                    port_name: "in".to_owned(),
                },
                alias: None,
                communication: None,
            }],
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
