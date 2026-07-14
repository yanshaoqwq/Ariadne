use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contracts::{
    CommunicationEdgeConfig, CoreError, CoreResult, Edge, EdgeId, ExecutionCancellation,
    ExternalDispatchAuthorization, LoopPolicy, NodeId, PortMap, PortValue, RunControl, RunId,
    RunStatus, WorkflowDefinition, WorkflowEdgeKind, WorkflowId,
};
use crate::skills::stable_text_hash;

/// 节点执行请求，包含运行态已经汇总好的 typed inputs 和通信消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNodeExecutionRequest {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub node_id: NodeId,
    /// 由 workflow/run/node/attempt 确定生成，恢复时不得随机变化。
    #[serde(default)]
    pub operation_id: String,
    /// 本次节点执行尝试序号，从 1 开始。
    #[serde(default)]
    pub operation_attempt: u32,
    /// 对节点类型、配置、输入、通信消息和前态 metadata 的稳定摘要。
    #[serde(default)]
    pub request_hash: String,
    #[serde(default)]
    pub type_name: String,
    #[serde(default)]
    pub config: Value,
    #[serde(default)]
    pub inputs: PortMap,
    #[serde(default)]
    pub communication_messages: Vec<CommunicationMessage>,
    #[serde(default)]
    pub metadata: Value,
    /// 当前 worker 执行链的共享取消信号；不持久化，也不参与 operation hash。
    #[serde(skip, default)]
    pub cancellation: ExecutionCancellation,
    /// 运行控制、worker lease 与 operation journal 的跨层派发栅栏。
    #[serde(skip, default)]
    pub dispatch_authorization: ExternalDispatchAuthorization,
}

/// 节点重试策略；用于网络、rate limit、超时和工具参数错误的可诊断恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub backoff_multiplier: u32,
}

impl Default for NodeRetryPolicy {
    /// 默认最多重试 3 次，退避序列为 1s、2s、4s。
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 1_000,
            backoff_multiplier: 2,
        }
    }
}

/// 节点错误类别，供前端和恢复流程区分自动重试、人工介入和系统降级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeErrorKind {
    Retryable,
    ToolArguments,
    Permission,
    Budget,
    Cancelled,
    External,
    System,
    Unknown,
}

/// 节点错误状态；序列化进 runtime.db，供恢复入口和节点详情面板使用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeErrorState {
    pub kind: NodeErrorKind,
    pub message: String,
    pub attempts: u32,
    pub max_attempts: u32,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_delay_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at_ms: Option<u64>,
    pub recovery_suggestion: String,
}

/// 结构化运行事件，避免上层只能解析自由文本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRuntimeEvent {
    pub sequence: u64,
    pub event_type: WorkflowRuntimeEventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    pub message: String,
    #[serde(default)]
    pub metadata: Value,
}

/// 运行事件类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRuntimeEventType {
    RunQueued,
    RunStarted,
    RunPaused,
    RunStopRequested,
    RunStopped,
    RunSucceeded,
    RunFailed,
    NodeStarted,
    NodeSucceeded,
    NodePaused,
    NodeSkipped,
    NodeRetryScheduled,
    NodeFailed,
    ConfirmationUpdated,
    /// 审慎者被拒确认项的输出经交流后被改写并通过（路径 B）。
    ConfirmationOutputOverridden,
    /// 外部注入正文后从指定节点重入（路径 A）。
    NodeResumedWithInjection,
    PatchWriteBackUpdated,
    CommunicationMessage,
    LoopUpdated,
}

/// 运行级失败详情；与节点错误分离，覆盖 worker 创建和执行器初始化失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRunFailure {
    pub code: String,
    pub stage: String,
    pub message: String,
    pub recovery_suggestion: String,
}

/// 节点通信控制输出，供 runtime 判断是否继续本条 communication 边。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicationControl {
    #[serde(default = "default_continue_communication")]
    pub continue_communication: bool,
    #[serde(default)]
    pub approved: bool,
}

impl Default for CommunicationControl {
    /// 默认继续通信，直到节点输出明确结束或次数耗尽。
    fn default() -> Self {
        Self {
            continue_communication: true,
            approved: false,
        }
    }
}

/// 节点执行输出，包含 typed outputs 和 communication 专用输出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNodeExecutionOutput {
    #[serde(default)]
    pub outputs: PortMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_control: Option<RunControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub communication_output: Option<String>,
    #[serde(default)]
    pub communication_control: CommunicationControl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_trace_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_session_commit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    #[serde(default)]
    pub confirmations: Vec<RuntimeConfirmation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_control: Option<RuntimeLoopControl>,
    #[serde(default)]
    pub metadata: Value,
}

impl Default for WorkflowNodeExecutionOutput {
    /// 创建空节点输出。
    fn default() -> Self {
        Self {
            outputs: PortMap::new(),
            run_control: None,
            communication_output: None,
            communication_control: CommunicationControl::default(),
            prompt_trace_hash: None,
            patch_session_commit_id: None,
            checkpoint_id: None,
            confirmations: Vec::new(),
            loop_control: None,
            metadata: Value::Null,
        }
    }
}

/// Loop 节点输出控制；只能由显式 Loop 节点触发重跑。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeLoopControl {
    pub continue_loop: bool,
    #[serde(default)]
    pub rerun_node_ids: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 工作流节点执行器，后续接 LLM、Document、ExecutorAdapter 和写作节点。
pub trait WorkflowNodeExecutor {
    /// 声明本次节点执行的副作用 journal 与恢复能力。
    fn operation_policy(
        &self,
        _request: &WorkflowNodeExecutionRequest,
    ) -> CoreResult<crate::workflow::WorkflowOperationPolicy> {
        Ok(crate::workflow::WorkflowOperationPolicy::Untracked)
    }

    /// 查询执行器自己的最终 receipt；只用于 `ReconcileReceipt` 恢复策略。
    fn reconcile_operation(
        &mut self,
        _request: &WorkflowNodeExecutionRequest,
    ) -> CoreResult<Option<WorkflowNodeExecutionOutput>> {
        Ok(None)
    }

    /// 执行一个节点。
    fn execute(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput>;
}

/// Workflow runtime 持久化抽象，SQLite 和测试内存存储共用同一契约。
pub trait WorkflowRuntimeStore {
    /// 只创建首次运行快照；run id 冲突必须失败，禁止覆盖已有运行。
    fn create_state(&self, state: &WorkflowRunState) -> CoreResult<()>;

    /// 保存当前运行快照。
    /// 原子保存运行快照；传入 operation id 时，同事务完成 completed→committed。
    fn save_state(
        &self,
        state: &mut WorkflowRunState,
        commit_operation_id: Option<&str>,
    ) -> CoreResult<()>;

    /// 加载指定运行快照。
    fn load_state(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
    ) -> CoreResult<Option<WorkflowRunState>>;

    /// 当前 worker fencing generation；同步/内存执行可返回 0。
    fn operation_lease_generation(&self) -> u64 {
        0
    }

    fn load_operation(
        &self,
        _operation_id: &str,
    ) -> CoreResult<Option<crate::workflow::WorkflowOperation>> {
        Ok(None)
    }

    fn create_operation(
        &self,
        _operation: &crate::workflow::NewWorkflowOperation,
        _now_ms: u64,
    ) -> CoreResult<()> {
        Ok(())
    }

    fn transition_operation(
        &self,
        _operation_id: &str,
        _expected: crate::workflow::WorkflowOperationStatus,
        _next: crate::workflow::WorkflowOperationStatus,
        _response_json: Option<&Value>,
        _now_ms: u64,
    ) -> CoreResult<bool> {
        Ok(true)
    }

    /// 原子完成 operation 响应，或把未消费派发授权的成功响应隔离为 InDoubt。
    fn complete_operation_response(
        &self,
        _operation_id: &str,
        _response_json: &Value,
        _now_ms: u64,
    ) -> CoreResult<crate::workflow::WorkflowOperationCompletionOutcome>;

    /// 创建一个可被真实副作用边界消费的持久化派发授权器。
    fn operation_dispatch_authorization(
        &self,
        _operation_id: &str,
    ) -> ExternalDispatchAuthorization {
        ExternalDispatchAuthorization::default()
    }

    /// 清理尚未派发的 operation；用于配置校验、Stop 或 fencing 在边界前拒绝。
    fn delete_prepared_operation(&self, _operation_id: &str) -> CoreResult<bool> {
        Ok(true)
    }
}

/// Runtime 确认项状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeConfirmationState {
    Pending,
    AutoAudited,
    Approved,
    Rejected,
}

/// Runtime 确认项，关联节点、patch 或 artifact。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfirmation {
    pub confirmation_id: String,
    pub node_id: NodeId,
    pub state: RuntimeConfirmationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_session_commit_id: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// patch 写回状态，保证 Resume 不重复应用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchWriteBackState {
    NotRequested,
    PendingConfirmation,
    Applied,
    Failed,
}

/// Runtime 引用类型，用于恢复前诊断 PortValue 和 patch 引用是否仍存在。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeReferenceKind {
    Document,
    Chunk,
    Artifact,
    PatchSessionCommit,
    Checkpoint,
}

/// 单个 runtime 引用检查结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeReferenceCheck {
    pub kind: RuntimeReferenceKind,
    pub id: String,
    pub node_id: NodeId,
    pub field_name: String,
    pub exists: bool,
    pub message: String,
}

/// 恢复诊断报告；缺引用时上层应 Pause 或进入可诊断失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRecoveryReport {
    pub checked_reference_count: usize,
    #[serde(default)]
    pub missing_references: Vec<RuntimeReferenceCheck>,
    #[serde(default)]
    pub degraded_reasons: Vec<String>,
}

impl RuntimeRecoveryReport {
    /// 创建空恢复报告。
    pub fn new() -> Self {
        Self {
            checked_reference_count: 0,
            missing_references: Vec::new(),
            degraded_reasons: Vec::new(),
        }
    }

    /// 判断恢复状态是否没有发现缺失或降级项。
    pub fn is_clean(&self) -> bool {
        self.missing_references.is_empty() && self.degraded_reasons.is_empty()
    }
}

impl Default for RuntimeRecoveryReport {
    fn default() -> Self {
        Self::new()
    }
}

/// runtime 恢复阶段的引用解析器；真实实现由 Document/Artifact/Checkpoint 存储提供。
pub trait RuntimeReferenceResolver {
    /// 判断文档引用是否仍存在。
    fn document_exists(&self, document_id: &str) -> CoreResult<bool>;

    /// 判断分块引用是否仍存在。
    fn chunk_exists(&self, chunk_id: &str) -> CoreResult<bool>;

    /// 判断 artifact 引用是否仍存在。
    fn artifact_exists(&self, artifact_id: &str) -> CoreResult<bool>;

    /// 判断 patch session commit 引用是否仍存在。
    fn patch_session_commit_exists(&self, patch_session_commit_id: &str) -> CoreResult<bool>;

    /// 判断 checkpoint 引用是否仍存在。
    fn checkpoint_exists(&self, checkpoint_id: &str) -> CoreResult<bool>;
}

/// 单条通信消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunicationMessage {
    pub edge_id: EdgeId,
    pub from_node_id: NodeId,
    pub to_node_id: NodeId,
    pub alias: String,
    pub content: String,
    pub message_index: u32,
}

/// 单条 communication 边的运行状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunicationRuntimeState {
    pub edge_id: EdgeId,
    pub initiator_node_id: NodeId,
    pub next_sender_node_id: NodeId,
    pub message_count: u32,
    pub max_message_count: u32,
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_hash: Option<String>,
    #[serde(default)]
    pub messages: Vec<CommunicationMessage>,
}

/// 节点运行快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNodeRuntimeState {
    pub node_id: NodeId,
    pub status: RunStatus,
    #[serde(default)]
    pub outputs: PortMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub communication_output: Option<String>,
    #[serde(default)]
    pub communication_control: CommunicationControl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_trace_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_session_commit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    #[serde(default)]
    pub patch_write_back_state: Option<PatchWriteBackState>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_state: Option<NodeErrorState>,
    #[serde(default)]
    pub execution_attempts: u32,
}

/// 工作流运行快照，后续可序列化进 runtime.db。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunState {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    /// SQLite 快照的乐观并发版本；由 store 加载/保存维护，不进入 state_json。
    #[serde(skip)]
    pub state_revision: u64,
    /// 启动预检后冻结的执行定义。仅写入 runtime.db，不作为运行状态 API 的显示载荷返回。
    #[serde(default, skip_serializing)]
    pub prepared_workflow: Option<WorkflowDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_node_id: Option<NodeId>,
    pub status: RunStatus,
    /// 运行因节点退避而排队时的持久化唤醒时间；None 表示可立即领取。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at_ms: Option<u64>,
    #[serde(default = "default_run_control")]
    pub control: RunControl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<WorkflowRunFailure>,
    #[serde(default)]
    pub nodes: BTreeMap<NodeId, WorkflowNodeRuntimeState>,
    /// 每个节点已经分配过的 operation 序号。节点因 Loop/回注被移出 `nodes`
    /// 后仍保留，避免同一 run 内复用旧 operation id。
    #[serde(default)]
    pub node_operation_sequences: BTreeMap<NodeId, u32>,
    #[serde(default)]
    pub communication_edges: BTreeMap<EdgeId, CommunicationRuntimeState>,
    #[serde(default)]
    pub loop_iterations: BTreeMap<NodeId, u32>,
    #[serde(default)]
    pub rerun_queue: Vec<NodeId>,
    #[serde(default)]
    pub confirmations: BTreeMap<String, RuntimeConfirmation>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub structured_events: Vec<WorkflowRuntimeEvent>,
    #[serde(default)]
    pub next_event_sequence: u64,
    #[serde(default)]
    pub retry_policy: NodeRetryPolicy,
}

impl WorkflowRunState {
    /// 创建新的运行状态。
    pub fn new(workflow_id: WorkflowId, run_id: RunId) -> Self {
        Self {
            workflow_id,
            run_id,
            state_revision: 0,
            prepared_workflow: None,
            start_node_id: None,
            status: RunStatus::Queued,
            next_retry_at_ms: None,
            control: RunControl::Continue,
            pause_reason: None,
            stop_reason: None,
            failure: None,
            nodes: BTreeMap::new(),
            node_operation_sequences: BTreeMap::new(),
            communication_edges: BTreeMap::new(),
            loop_iterations: BTreeMap::new(),
            rerun_queue: Vec::new(),
            confirmations: BTreeMap::new(),
            events: Vec::new(),
            structured_events: Vec::new(),
            next_event_sequence: 0,
            retry_policy: NodeRetryPolicy::default(),
        }
    }

    /// 判断当前是否仍有待确认项。
    pub fn has_pending_confirmations(&self) -> bool {
        self.confirmations
            .values()
            .any(|item| item.state == RuntimeConfirmationState::Pending)
    }
}

/// 同步工作流运行器，先固定调度和恢复契约。
pub struct WorkflowRuntime {
    pub state: WorkflowRunState,
    cancellation: ExecutionCancellation,
}

impl WorkflowRuntime {
    /// 创建运行器并初始化 communication 状态。
    pub fn new(workflow: &WorkflowDefinition, run_id: RunId) -> CoreResult<Self> {
        workflow.validate_topology()?;
        let mut state = WorkflowRunState::new(workflow.id.clone(), run_id);
        for edge in workflow
            .edges
            .iter()
            .filter(|edge| edge.kind == WorkflowEdgeKind::Communication)
        {
            let config = edge.communication.as_ref().ok_or_else(|| {
                CoreError::validation("communication edge requires configuration")
            })?;
            let initiator = config.initiator_for_edge(edge).clone();
            state.communication_edges.insert(
                edge.id.clone(),
                CommunicationRuntimeState {
                    edge_id: edge.id.clone(),
                    initiator_node_id: initiator.clone(),
                    next_sender_node_id: initiator,
                    message_count: 0,
                    max_message_count: config.max_communication_count,
                    completed: false,
                    completed_reason: None,
                    pause_reason: None,
                    last_message_hash: None,
                    messages: Vec::new(),
                },
            );
        }
        Ok(Self {
            state,
            cancellation: ExecutionCancellation::new(),
        })
    }

    /// 运行到成功、暂停或失败。
    pub fn run(
        &mut self,
        workflow: &WorkflowDefinition,
        executor: &mut dyn WorkflowNodeExecutor,
    ) -> CoreResult<RunStatus> {
        self.run_inner(workflow, executor, None)
    }

    /// 运行并在关键状态变更后保存到 runtime store。
    pub fn run_persisted(
        &mut self,
        workflow: &WorkflowDefinition,
        executor: &mut dyn WorkflowNodeExecutor,
        store: &dyn WorkflowRuntimeStore,
    ) -> CoreResult<RunStatus> {
        if store
            .load_state(&self.state.workflow_id, &self.state.run_id)?
            .is_none()
        {
            store.create_state(&self.state)?;
        }
        self.run_inner(workflow, executor, Some(store))
    }

    /// 从持久化状态恢复运行器。
    pub fn from_state(state: WorkflowRunState) -> Self {
        Self {
            state,
            cancellation: ExecutionCancellation::new(),
        }
    }

    /// 将 runtime 绑定到 worker 生命周期共享的取消信号。
    pub fn set_cancellation(&mut self, cancellation: ExecutionCancellation) {
        self.cancellation = cancellation;
    }

    pub fn cancellation(&self) -> &ExecutionCancellation {
        &self.cancellation
    }

    /// 覆盖节点错误重试策略，主要供项目配置或测试注入。
    pub fn set_retry_policy(&mut self, retry_policy: NodeRetryPolicy) -> CoreResult<()> {
        if retry_policy.max_attempts == 0 {
            return Err(CoreError::validation(
                "workflow retry policy max_attempts must be greater than zero",
            ));
        }
        if retry_policy.initial_backoff_ms == 0 {
            return Err(CoreError::validation(
                "workflow retry policy initial_backoff_ms must be greater than zero",
            ));
        }
        if retry_policy.backoff_multiplier < 1 {
            return Err(CoreError::validation(
                "workflow retry policy backoff_multiplier must be at least one",
            ));
        }
        self.state.retry_policy = retry_policy;
        Ok(())
    }

    /// 查询指定节点的结构化事件。
    pub fn events_for_node(&self, node_id: &NodeId) -> Vec<WorkflowRuntimeEvent> {
        self.state
            .structured_events
            .iter()
            .filter(|event| event.node_id.as_ref() == Some(node_id))
            .cloned()
            .collect()
    }

    /// 请求暂停运行；下一次 run 会先保持 Paused。
    /// F12-c：终态 / Stopping 不得被 Pause 复活。
    pub fn request_pause(&mut self, reason: impl Into<String>) -> CoreResult<()> {
        if self.state.status.is_terminal() || self.state.status == RunStatus::Stopping {
            return Err(CoreError::validation(format!(
                "cannot pause workflow run in status {:?}",
                self.state.status
            )));
        }
        self.pause(reason);
        Ok(())
    }

    /// 请求停止运行并保留已完成结果。
    /// F12-a：写入 Stopping/Stopped，使 `execution_should_cancel` 能取消 in-flight。
    /// F12-c：终态不得被 Stop 反复改写。
    pub fn request_stop(&mut self, reason: impl Into<String>) -> CoreResult<()> {
        if self.state.status.is_terminal() {
            return Err(CoreError::validation(format!(
                "cannot stop workflow run in terminal status {:?}",
                self.state.status
            )));
        }
        let reason = reason.into();
        self.state.control = RunControl::Stop;
        self.state.stop_reason = Some(reason.clone());
        self.state.next_retry_at_ms = None;
        // Paused/Queued：无 in-flight 节点，直接 Stopped；Running：Stopping 触发取消。
        let event_type = if matches!(self.state.status, RunStatus::Paused | RunStatus::Queued) {
            self.state.status = RunStatus::Stopped;
            WorkflowRuntimeEventType::RunStopped
        } else {
            self.state.status = RunStatus::Stopping;
            WorkflowRuntimeEventType::RunStopRequested
        };
        self.state
            .events
            .push(format!("run stop requested: {reason}"));
        self.record_event(
            event_type,
            None,
            format!("run stop requested: {reason}"),
            Value::Null,
        );
        Ok(())
    }

    /// 恢复暂停运行。待确认项未解决时，下一次 run 会再次暂停。
    pub fn resume(&mut self) -> CoreResult<()> {
        if self.state.status.is_terminal() || self.state.status == RunStatus::Stopping {
            return Err(CoreError::validation("terminal workflow run cannot resume"));
        }
        self.state.control = RunControl::Continue;
        self.state.pause_reason = None;
        self.state.next_retry_at_ms = None;
        if self.state.status == RunStatus::Paused {
            self.state.status = RunStatus::Queued;
        }
        Ok(())
    }

    /// 跳过指定节点，用于断点暂停后的人工“跳过”操作。
    pub fn skip_node(&mut self, workflow: &WorkflowDefinition, node_id: &NodeId) -> CoreResult<()> {
        if self.state.status.is_terminal() {
            return Err(CoreError::validation(
                "terminal workflow run cannot skip node",
            ));
        }
        if !workflow.nodes.iter().any(|node| node.id == *node_id) {
            return Err(CoreError::validation(format!(
                "node {} not found in workflow",
                node_id.as_str()
            )));
        }
        let attempts = previous_attempts(&self.state, node_id);
        let mut metadata = serde_json::Map::new();
        metadata.insert("skipped".to_owned(), Value::Bool(true));
        self.state.nodes.insert(
            node_id.clone(),
            WorkflowNodeRuntimeState {
                node_id: node_id.clone(),
                status: RunStatus::Succeeded,
                outputs: PortMap::new(),
                communication_output: None,
                communication_control: CommunicationControl::default(),
                prompt_trace_hash: None,
                patch_session_commit_id: None,
                checkpoint_id: None,
                patch_write_back_state: None,
                metadata: Value::Object(metadata),
                error: None,
                error_state: None,
                execution_attempts: attempts,
            },
        );
        self.state
            .events
            .push(format!("node {} skipped", node_id.as_str()));
        self.record_event(
            WorkflowRuntimeEventType::NodeSkipped,
            Some(node_id.clone()),
            format!("node {} skipped", node_id.as_str()),
            Value::Null,
        );
        if self.state.status == RunStatus::Paused {
            self.state.control = RunControl::Continue;
            self.state.pause_reason = None;
            self.state.next_retry_at_ms = None;
            self.state.status = RunStatus::Queued;
        }
        Ok(())
    }

    /// 运行到成功、暂停、停止或失败。
    fn run_inner(
        &mut self,
        workflow: &WorkflowDefinition,
        executor: &mut dyn WorkflowNodeExecutor,
        store: Option<&dyn WorkflowRuntimeStore>,
    ) -> CoreResult<RunStatus> {
        workflow.validate_topology()?;
        let graph_index = WorkflowGraphIndex::build(workflow);
        let mut ready_queue =
            ReadyQueue::from_nodes(ready_nodes(workflow, &graph_index, &self.state));
        if self.state.status.is_terminal() {
            return Ok(self.state.status);
        }

        // 先处理外部控制信号，避免已经请求 Stop/Pause 的运行在进入调度循环后
        // 又启动新的节点。这里也负责把控制状态写回持久化存储。
        if self.state.control == RunControl::Stop {
            self.stop("stop requested before run");
            persist_if_needed(store, &mut self.state)?;
            return Ok(self.state.status);
        }
        if self.state.control == RunControl::Pause && self.state.status == RunStatus::Paused {
            persist_if_needed(store, &mut self.state)?;
            return Ok(self.state.status);
        }

        self.state.control = RunControl::Continue;
        self.state.pause_reason = None;
        self.state.next_retry_at_ms = None;
        self.state.status = RunStatus::Running;
        self.record_event(
            WorkflowRuntimeEventType::RunStarted,
            None,
            "workflow run started",
            Value::Null,
        );
        persist_if_needed(store, &mut self.state)?;

        loop {
            refresh_external_control(store, &mut self.state)?;
            if self.state.control == RunControl::Stop {
                self.stop("stop requested during run");
                persist_if_needed(store, &mut self.state)?;
                return Ok(self.state.status);
            }
            if self.state.control == RunControl::Pause {
                self.pause(
                    self.state
                        .pause_reason
                        .clone()
                        .unwrap_or_else(|| "pause requested during run".to_owned()),
                );
                persist_if_needed(store, &mut self.state)?;
                return Ok(self.state.status);
            }

            // 待确认项是运行时硬暂停点。确认项没有解决前，下游节点不能依赖
            // pending 输出继续执行，否则会把未审批 patch 或意见传给后续节点。
            if self.state.has_pending_confirmations() {
                self.pause("pending confirmation items");
                persist_if_needed(store, &mut self.state)?;
                return Ok(self.state.status);
            }

            // 启动/恢复时只扫描一次全图；之后节点完成只唤醒直接下游、通信邻居
            // 和 Loop 重跑目标，避免链式工作流每完成一步都重新遍历全部节点。
            let Some(node_id) = ready_queue.pop() else {
                if all_nodes_succeeded(workflow, &self.state) {
                    self.state.next_retry_at_ms = None;
                    self.state.status = RunStatus::Succeeded;
                    self.record_event(
                        WorkflowRuntimeEventType::RunSucceeded,
                        None,
                        "workflow run succeeded",
                        Value::Null,
                    );
                    persist_if_needed(store, &mut self.state)?;
                    return Ok(self.state.status);
                }
                if let Some(next_retry_at_ms) = next_pending_retry_at_ms(&self.state) {
                    self.state.next_retry_at_ms = Some(next_retry_at_ms);
                    self.state.status = RunStatus::Queued;
                    persist_if_needed(store, &mut self.state)?;
                    return Ok(self.state.status);
                }
                self.pause("no runnable nodes are ready");
                persist_if_needed(store, &mut self.state)?;
                return Ok(self.state.status);
            };
            if !node_is_ready(workflow, &graph_index, &self.state, &node_id) {
                continue;
            }
            // rerun_queue 只负责唤醒目标节点一次；节点开始执行前移除，避免
            // Resume 时重复消费同一个 Loop 触发。
            self.state.rerun_queue.retain(|queued| queued != &node_id);
            if self.node_succeeded(&node_id)
                && !has_pending_communication_for_node(&self.state, &node_id)
            {
                continue;
            }

            // Stop 可能在同一轮循环中由前一个节点设置。每个节点启动前都检查，
            // 确保 Stop 不会继续推进下游。
            refresh_external_control(store, &mut self.state)?;
            if self.state.control == RunControl::Stop {
                self.stop("stop requested during run");
                persist_if_needed(store, &mut self.state)?;
                return Ok(self.state.status);
            }
            if self.state.control == RunControl::Pause {
                self.pause(
                    self.state
                        .pause_reason
                        .clone()
                        .unwrap_or_else(|| "pause requested during run".to_owned()),
                );
                persist_if_needed(store, &mut self.state)?;
                return Ok(self.state.status);
            }
            let Some(node_instance) = graph_index.node(workflow, &node_id) else {
                return Err(CoreError::validation(format!(
                    "node {} not found in workflow",
                    node_id.as_str()
                )));
            };
            if should_pause_for_breakpoint(&mut self.state, node_instance) {
                self.pause(format!("breakpoint before node {}", node_id.as_str()));
                persist_if_needed(store, &mut self.state)?;
                return Ok(self.state.status);
            }
            let inputs = match collect_data_inputs(workflow, &graph_index, &self.state, &node_id) {
                Ok(inputs) => inputs,
                Err(error) => {
                    if self.record_node_error(node_id.clone(), error) {
                        persist_if_needed(store, &mut self.state)?;
                        continue;
                    }
                    self.state.next_retry_at_ms = None;
                    self.state.status = if self
                        .state
                        .nodes
                        .get(&node_id)
                        .and_then(|node| node.error_state.as_ref())
                        .is_some_and(|error| error.retryable)
                    {
                        RunStatus::Paused
                    } else {
                        RunStatus::Failed
                    };
                    persist_if_needed(store, &mut self.state)?;
                    return Ok(self.state.status);
                }
            };
            let operation_attempt = next_node_operation_attempt(&self.state, &node_id);
            let communication_messages = collect_inbound_messages(&self.state, &node_id);
            let metadata = self
                .state
                .nodes
                .get(&node_id)
                .map(|node| node.metadata.clone())
                .unwrap_or(Value::Null);
            let operation_id = stable_text_hash(&format!(
                "workflow-operation-v1\0{}\0{}\0{}\0{}",
                workflow.id.as_str(),
                self.state.run_id.as_str(),
                node_id.as_str(),
                operation_attempt
            ));
            let request_hash = stable_text_hash(&serde_json::to_string(&json!({
                "type_name": node_instance.type_name,
                "config": node_instance.config,
                "inputs": inputs,
                "communication_messages": communication_messages,
                "metadata": metadata,
            }))?);
            let mut request = WorkflowNodeExecutionRequest {
                workflow_id: workflow.id.clone(),
                run_id: self.state.run_id.clone(),
                node_id: node_id.clone(),
                operation_id,
                operation_attempt,
                request_hash,
                type_name: node_instance.type_name.clone(),
                config: node_instance.config.clone(),
                inputs,
                communication_messages,
                metadata,
                cancellation: self.cancellation.clone(),
                dispatch_authorization: ExternalDispatchAuthorization::default(),
            };
            let operation_policy = executor.operation_policy(&request)?;
            if operation_is_journaled(operation_policy) {
                if let Some(runtime_store) = store {
                    request.dispatch_authorization =
                        runtime_store.operation_dispatch_authorization(&request.operation_id);
                }
            }
            let journal_action =
                prepare_operation_journal(store, &request, operation_policy, executor)?;
            if matches!(journal_action, OperationJournalAction::InDoubt) {
                self.pause(format!(
                    "operation {} is in doubt and requires confirmation",
                    request.operation_id
                ));
                persist_if_needed(store, &mut self.state)?;
                return Ok(self.state.status);
            }
            self.record_node_started(&node_id, operation_attempt);
            let execution_result = match &journal_action {
                OperationJournalAction::Execute => executor.execute(request.clone()),
                OperationJournalAction::Replay(output) => Ok(output.as_ref().clone()),
                OperationJournalAction::InDoubt => unreachable!("handled before execution"),
            };
            let execution_result = match execution_result {
                Ok(output) if matches!(journal_action, OperationJournalAction::Execute) => {
                    match complete_operation_journal(store, &request, operation_policy, &output) {
                        Ok(()) => Ok(output),
                        Err(error @ CoreError::WorkflowExecutorContractViolation { .. }) => {
                            Err(error)
                        }
                        Err(error) => return Err(error),
                    }
                }
                result => result,
            };
            refresh_external_control(store, &mut self.state)?;
            let external_stop_requested = self.state.control == RunControl::Stop;
            match execution_result {
                Ok(output) => {
                    let requested_control = output.run_control;
                    let loop_control = output.loop_control.clone();
                    if external_stop_requested {
                        self.record_node_success(node_id.clone(), output);
                        self.stop(
                            self.state.stop_reason.clone().unwrap_or_else(|| {
                                "stop requested during node execution".to_owned()
                            }),
                        );
                    } else {
                        match requested_control {
                            Some(RunControl::Pause) => {
                                // 节点主动 Pause 表示这次节点尚未完成。保存中间输出和
                                // metadata，但节点状态保持 Paused，Resume 后允许重试。
                                self.record_node_paused(node_id.clone(), output);
                                self.pause(format!("node {} requested pause", node_id.as_str()));
                            }
                            Some(RunControl::Stop) => {
                                // 节点主动 Stop 表示当前节点结果有效，但整个运行不再
                                // 继续下游。先记录成功输出，再停止运行。
                                self.record_node_success(node_id.clone(), output);
                                self.stop(format!("node {} requested stop", node_id.as_str()));
                            }
                            Some(RunControl::Continue) | None => {
                                // 普通完成路径先固化节点输出，再推进 communication 和
                                // Loop。二者都可能把运行切成 Paused。
                                self.record_node_success(node_id.clone(), output);
                                self.advance_communication(workflow, &node_id)?;
                                self.advance_loop(workflow, &node_id, loop_control.as_ref())?;
                            }
                        }
                    }
                    persist_operation_if_needed(
                        store,
                        &mut self.state,
                        operation_is_journaled(operation_policy)
                            .then_some(request.operation_id.as_str()),
                    )?;
                }
                Err(error) => {
                    let error_text = error.to_string();
                    let operation_in_doubt =
                        settle_operation_failure(store, &request, operation_policy, &error)?;
                    // F12：executor 内 Pause/Stop 可能已在 store 中胜出；先 rebase 控制态，
                    // 避免把「dispatch 被门禁拒绝」误记为节点 Failed 覆盖用户 Pause。
                    refresh_external_control(store, &mut self.state)?;
                    let control_stop = external_stop_requested
                        || self.state.control == RunControl::Stop
                        || matches!(self.state.status, RunStatus::Stopping | RunStatus::Stopped);
                    let control_pause = self.state.control == RunControl::Pause
                        || self.state.status == RunStatus::Paused;
                    if control_stop {
                        if let Some(node) = self.state.nodes.get_mut(&node_id) {
                            node.status = RunStatus::Stopped;
                            node.error = Some(error_text);
                            node.error_state = None;
                        }
                        self.stop(
                            self.state.stop_reason.clone().unwrap_or_else(|| {
                                "stop requested during node execution".to_owned()
                            }),
                        );
                        persist_if_needed(store, &mut self.state)?;
                        return Ok(self.state.status);
                    }
                    if control_pause && !operation_in_doubt {
                        if let Some(node) = self.state.nodes.get_mut(&node_id) {
                            node.status = RunStatus::Paused;
                            node.error = Some(error_text);
                            node.error_state = None;
                        }
                        self.pause(
                            self.state.pause_reason.clone().unwrap_or_else(|| {
                                "pause requested during node execution".to_owned()
                            }),
                        );
                        persist_if_needed(store, &mut self.state)?;
                        return Ok(self.state.status);
                    }
                    if operation_in_doubt {
                        if let Some(node) = self.state.nodes.get_mut(&node_id) {
                            node.status = RunStatus::Paused;
                            node.error = Some(error_text);
                            node.error_state = None;
                        }
                        self.pause(format!(
                            "operation {} may have produced an external side effect: {error}",
                            request.operation_id
                        ));
                        persist_if_needed(store, &mut self.state)?;
                        if self.state.control == RunControl::Stop {
                            self.stop(self.state.stop_reason.clone().unwrap_or_else(|| {
                                "stop requested during in_doubt settlement".to_owned()
                            }));
                            persist_if_needed(store, &mut self.state)?;
                        }
                        return Ok(self.state.status);
                    }
                    if self.record_node_error(node_id.clone(), error) {
                        persist_if_needed(store, &mut self.state)?;
                        continue;
                    }
                    self.state.next_retry_at_ms = None;
                    self.state.status = if self
                        .state
                        .nodes
                        .get(&node_id)
                        .and_then(|node| node.error_state.as_ref())
                        .is_some_and(|error| error.retryable)
                    {
                        RunStatus::Paused
                    } else {
                        RunStatus::Failed
                    };
                    persist_if_needed(store, &mut self.state)?;
                    if self.state.control == RunControl::Stop {
                        self.stop(
                            self.state
                                .stop_reason
                                .clone()
                                .unwrap_or_else(|| "stop requested during node failure".to_owned()),
                        );
                        persist_if_needed(store, &mut self.state)?;
                    }
                    return Ok(self.state.status);
                }
            }

            ready_queue.extend(graph_index.dependent_nodes(&node_id).iter().cloned());
            ready_queue.extend(
                graph_index
                    .communication_neighbors(&node_id)
                    .iter()
                    .cloned(),
            );
            ready_queue.extend(self.state.rerun_queue.iter().cloned());

            if matches!(self.state.status, RunStatus::Paused | RunStatus::Stopped) {
                return Ok(self.state.status);
            }
        }
    }

    /// 更新确认项状态，供 Resume 前调用。
    pub fn update_confirmation_state(
        &mut self,
        confirmation_id: &str,
        state: RuntimeConfirmationState,
    ) -> CoreResult<()> {
        let item = self
            .state
            .confirmations
            .get_mut(confirmation_id)
            .ok_or_else(|| {
                CoreError::validation(format!("confirmation item not found: {confirmation_id}"))
            })?;
        item.state = state;
        let node_id = item.node_id.clone();
        let reason = item
            .metadata
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Some(node) = self.state.nodes.get_mut(&node_id) {
            // 确认项状态需要回写成普通 typed outputs，保证下游节点不用直接读
            // confirmations map，也能通过 data edge 消费审批结果。
            match state {
                RuntimeConfirmationState::Approved | RuntimeConfirmationState::AutoAudited => {
                    node.outputs
                        .insert("approved".to_owned(), PortValue::inline(true));
                    node.outputs
                        .insert("rejected".to_owned(), PortValue::inline(false));
                }
                RuntimeConfirmationState::Rejected => {
                    node.outputs
                        .insert("approved".to_owned(), PortValue::inline(false));
                    node.outputs
                        .insert("rejected".to_owned(), PortValue::inline(true));
                }
                RuntimeConfirmationState::Pending => {}
            }
            if !reason.is_empty() {
                node.outputs
                    .insert("review_reason".to_owned(), PortValue::inline(reason));
            }
        }
        self.record_event(
            WorkflowRuntimeEventType::ConfirmationUpdated,
            Some(node_id),
            format!("confirmation {confirmation_id} updated to {state:?}"),
            Value::Null,
        );
        // 保持旧调用兼容：如果唯一暂停原因是待确认项，并且全部确认已经解决，
        // 调用方可以直接再次 run，不必额外调用 resume。
        if !self.state.has_pending_confirmations()
            && self.state.status == RunStatus::Paused
            && self.state.pause_reason.as_deref() == Some("pending confirmation items")
        {
            self.state.control = RunControl::Continue;
            self.state.pause_reason = None;
            self.state.next_retry_at_ms = None;
            self.state.status = RunStatus::Queued;
        }
        Ok(())
    }

    /// 路径 B：审慎者输出被拒后，人工或项目空间 AI 与其交流得到同意的输出，
    /// 把交流结果作为该确认项关联节点的输出改写并置为通过，然后解除暂停继续。
    /// 用于机制文档「把同意的输出作为 prudent 的输出继续」。
    pub fn override_confirmation_output(
        &mut self,
        confirmation_id: &str,
        new_outputs: PortMap,
    ) -> CoreResult<()> {
        if self.state.status.is_terminal() {
            return Err(CoreError::validation(
                "terminal workflow run cannot override confirmation output",
            ));
        }
        let node_id = {
            let item = self
                .state
                .confirmations
                .get(confirmation_id)
                .ok_or_else(|| {
                    CoreError::validation(format!("confirmation item not found: {confirmation_id}"))
                })?;
            item.node_id.clone()
        };
        // 把交流后同意的输出合并进关联节点，覆盖被拒的原输出（如 revision_context、
        // 判断结果），保证下游节点通过 data edge 消费到新输出。
        if let Some(node) = self.state.nodes.get_mut(&node_id) {
            for (key, value) in new_outputs.iter() {
                node.outputs.insert(key.clone(), value.clone());
            }
        } else {
            return Err(CoreError::validation(format!(
                "node runtime state not found: {}",
                node_id.as_str()
            )));
        }
        self.record_event(
            WorkflowRuntimeEventType::ConfirmationOutputOverridden,
            Some(node_id.clone()),
            format!("confirmation {confirmation_id} output overridden and approved"),
            Value::Null,
        );
        // 改写后置为通过，复用 update_confirmation_state 的回写逻辑。
        self.update_confirmation_state(confirmation_id, RuntimeConfirmationState::Approved)?;
        // override 操作本身就是对"被拒暂停"的解决，无论 pause_reason 是什么都直接恢复。
        if self.state.status == RunStatus::Paused {
            self.state.control = RunControl::Continue;
            self.state.pause_reason = None;
            self.state.next_retry_at_ms = None;
            self.state.status = RunStatus::Queued;
        }
        Ok(())
    }

    /// 路径 A：审慎者拒绝后暂停期间，人工或项目空间 AI 修改正文得到章节正文，
    /// 把该正文作为指定节点的输出注入并置为成功，然后清理其控制下游快照使其重跑，
    /// 从「任意需要正文的节点」继续。用于机制文档路径 A。
    pub fn resume_from_node(
        &mut self,
        workflow: &WorkflowDefinition,
        node_id: &NodeId,
        injected_outputs: PortMap,
    ) -> CoreResult<()> {
        if self.state.status.is_terminal() {
            return Err(CoreError::validation(
                "terminal workflow run cannot resume from node",
            ));
        }
        if !workflow.nodes.iter().any(|node| node.id == *node_id) {
            return Err(CoreError::validation(format!(
                "node {} not found in workflow",
                node_id.as_str()
            )));
        }
        // 清理该节点 control/data 下游的既有快照，使其在下一次 run 时以注入正文为输入重跑。
        let mut closure = Vec::new();
        collect_downstream_closure(workflow, node_id, &mut closure);
        for downstream in &closure {
            if downstream != node_id {
                self.state.nodes.remove(downstream);
                // 重置被清理节点关联的 loop 迭代计数，否则已耗尽 max_iterations
                // 的 loop 节点在恢复后会立即再次暂停。
                self.state.loop_iterations.remove(downstream);
            }
        }
        // 重置涉及被清理节点的 communication 边状态，否则上一轮 completed=true
        // 会导致重跑后 communication 被跳过，返修循环静默失效。
        reset_communication_edges_for_nodes(&mut self.state, &closure, workflow);
        // 把外部得到的正文注入为该节点输出，置成功，避免重跑注入源节点本身。
        let attempts = previous_attempts(&self.state, node_id);
        let mut metadata = serde_json::Map::new();
        metadata.insert("injected".to_owned(), Value::Bool(true));
        self.state.nodes.insert(
            node_id.clone(),
            WorkflowNodeRuntimeState {
                node_id: node_id.clone(),
                status: RunStatus::Succeeded,
                outputs: injected_outputs,
                communication_output: None,
                communication_control: CommunicationControl::default(),
                prompt_trace_hash: None,
                patch_session_commit_id: None,
                checkpoint_id: None,
                patch_write_back_state: None,
                metadata: Value::Object(metadata),
                error: None,
                error_state: None,
                execution_attempts: attempts,
            },
        );
        self.record_event(
            WorkflowRuntimeEventType::NodeResumedWithInjection,
            Some(node_id.clone()),
            format!("node {} resumed with injected outputs", node_id.as_str()),
            Value::Null,
        );
        if self.state.status == RunStatus::Paused {
            self.state.control = RunControl::Continue;
            self.state.pause_reason = None;
            self.state.next_retry_at_ms = None;
            self.state.status = RunStatus::Queued;
        }
        Ok(())
    }

    /// 更新 patch 写回状态；Resume 时用该状态避免重复应用同一 patch。
    pub fn mark_patch_write_back_state(
        &mut self,
        node_id: &NodeId,
        state: PatchWriteBackState,
    ) -> CoreResult<()> {
        let commit_id = {
            let node = self.state.nodes.get(node_id).ok_or_else(|| {
                CoreError::validation(format!(
                    "node runtime state not found: {}",
                    node_id.as_str()
                ))
            })?;
            let commit_id = node.patch_session_commit_id.clone().ok_or_else(|| {
                CoreError::validation(format!(
                    "node {} has no patch session commit",
                    node_id.as_str()
                ))
            })?;
            if node.patch_write_back_state == Some(PatchWriteBackState::Applied)
                && state != PatchWriteBackState::Applied
            {
                return Err(CoreError::validation(format!(
                    "node {} patch write-back state cannot move back from applied",
                    node_id.as_str()
                )));
            }
            commit_id
        };

        if state == PatchWriteBackState::Applied {
            self.ensure_patch_confirmation_allows_apply(node_id, &commit_id)?;
        }

        let node = self.state.nodes.get_mut(node_id).ok_or_else(|| {
            CoreError::validation(format!(
                "node runtime state not found: {}",
                node_id.as_str()
            ))
        })?;
        node.patch_write_back_state = Some(state);
        self.state.events.push(format!(
            "node {} patch write-back state set to {:?}",
            node_id.as_str(),
            state
        ));
        self.record_event(
            WorkflowRuntimeEventType::PatchWriteBackUpdated,
            Some(node_id.clone()),
            format!(
                "node {} patch write-back state set to {:?}",
                node_id.as_str(),
                state
            ),
            Value::Null,
        );
        Ok(())
    }

    /// 校验 patch 是否允许开始实际文件写回，但不修改 runtime 状态。
    pub fn ensure_patch_write_back_can_start(&self, node_id: &NodeId) -> CoreResult<()> {
        let node = self.state.nodes.get(node_id).ok_or_else(|| {
            CoreError::validation(format!(
                "node runtime state not found: {}",
                node_id.as_str()
            ))
        })?;
        if node.patch_write_back_state == Some(PatchWriteBackState::Applied) {
            return Err(CoreError::validation(format!(
                "node {} patch write-back was already applied",
                node_id.as_str()
            )));
        }
        let commit_id = node.patch_session_commit_id.clone().ok_or_else(|| {
            CoreError::validation(format!(
                "node {} has no patch session commit",
                node_id.as_str()
            ))
        })?;
        self.ensure_patch_confirmation_allows_apply(node_id, &commit_id)
    }

    /// 校验关联确认项是否允许 patch 写回。
    fn ensure_patch_confirmation_allows_apply(
        &self,
        node_id: &NodeId,
        commit_id: &str,
    ) -> CoreResult<()> {
        // Applied 是不可逆写回点，必须确认关联 patch 没有 pending/rejected
        // 审批项。这样 Resume 时不会绕过人工确认直接写回正文。
        for confirmation in self.state.confirmations.values().filter(|item| {
            item.node_id == *node_id && item.patch_session_commit_id.as_deref() == Some(commit_id)
        }) {
            match confirmation.state {
                RuntimeConfirmationState::Pending => {
                    return Err(CoreError::validation(format!(
                        "node {} patch {} cannot be applied before confirmation {} is resolved",
                        node_id.as_str(),
                        commit_id,
                        confirmation.confirmation_id
                    )));
                }
                RuntimeConfirmationState::Rejected => {
                    return Err(CoreError::validation(format!(
                        "node {} patch {} cannot be applied after confirmation {} was rejected",
                        node_id.as_str(),
                        commit_id,
                        confirmation.confirmation_id
                    )));
                }
                RuntimeConfirmationState::AutoAudited | RuntimeConfirmationState::Approved => {}
            }
        }
        Ok(())
    }

    /// 校验当前运行快照里保存的引用是否仍可解析。
    pub fn validate_references(
        &self,
        resolver: &dyn RuntimeReferenceResolver,
    ) -> CoreResult<RuntimeRecoveryReport> {
        let mut report = RuntimeRecoveryReport::new();
        for node in self.state.nodes.values() {
            // 失败节点本身不一定缺引用，但恢复时需要显式展示为降级原因，
            // 由 UI 或上层恢复流程决定是否重试。
            if node.status == RunStatus::Failed {
                report.degraded_reasons.push(format!(
                    "node {} is failed and requires manual recovery",
                    node.node_id.as_str()
                ));
            }

            for (port_name, value) in &node.outputs {
                check_port_value_reference(
                    &mut report,
                    resolver,
                    &node.node_id,
                    port_name.as_str(),
                    value,
                )?;
            }

            if let Some(commit_id) = &node.patch_session_commit_id {
                let exists = resolver.patch_session_commit_exists(commit_id)?;
                record_reference_check(
                    &mut report,
                    RuntimeReferenceKind::PatchSessionCommit,
                    commit_id,
                    &node.node_id,
                    "patch_session_commit_id",
                    exists,
                );
            }
            if let Some(checkpoint_id) = &node.checkpoint_id {
                let exists = resolver.checkpoint_exists(checkpoint_id)?;
                record_reference_check(
                    &mut report,
                    RuntimeReferenceKind::Checkpoint,
                    checkpoint_id,
                    &node.node_id,
                    "checkpoint_id",
                    exists,
                );
            }
        }

        for confirmation in self.state.confirmations.values() {
            if let Some(artifact_id) = &confirmation.artifact_id {
                let exists = resolver.artifact_exists(artifact_id)?;
                record_reference_check(
                    &mut report,
                    RuntimeReferenceKind::Artifact,
                    artifact_id,
                    &confirmation.node_id,
                    &format!("confirmation.{}.artifact_id", confirmation.confirmation_id),
                    exists,
                );
            }
            if let Some(commit_id) = &confirmation.patch_session_commit_id {
                let exists = resolver.patch_session_commit_exists(commit_id)?;
                record_reference_check(
                    &mut report,
                    RuntimeReferenceKind::PatchSessionCommit,
                    commit_id,
                    &confirmation.node_id,
                    &format!(
                        "confirmation.{}.patch_session_commit_id",
                        confirmation.confirmation_id
                    ),
                    exists,
                );
            }
        }

        Ok(report)
    }

    /// 判断节点是否已成功，用于 Resume 幂等跳过。
    fn node_succeeded(&self, node_id: &NodeId) -> bool {
        self.state
            .nodes
            .get(node_id)
            .is_some_and(|node| node.status == RunStatus::Succeeded)
    }

    /// 记录节点成功输出。
    fn record_node_success(&mut self, node_id: NodeId, output: WorkflowNodeExecutionOutput) {
        self.record_node_output(node_id.clone(), output, RunStatus::Succeeded);
        self.state
            .events
            .push(format!("node {} succeeded", node_id.as_str()));
        self.record_event(
            WorkflowRuntimeEventType::NodeSucceeded,
            Some(node_id.clone()),
            format!("node {} succeeded", node_id.as_str()),
            Value::Null,
        );
    }

    /// 记录节点暂停输出，Resume 后该节点会重试。
    fn record_node_paused(&mut self, node_id: NodeId, output: WorkflowNodeExecutionOutput) {
        self.record_node_output(node_id.clone(), output, RunStatus::Paused);
        self.state
            .events
            .push(format!("node {} paused", node_id.as_str()));
        self.record_event(
            WorkflowRuntimeEventType::NodePaused,
            Some(node_id.clone()),
            format!("node {} paused", node_id.as_str()),
            Value::Null,
        );
    }

    /// 记录节点输出到指定状态。
    fn record_node_output(
        &mut self,
        node_id: NodeId,
        output: WorkflowNodeExecutionOutput,
        status: RunStatus,
    ) {
        let patch_state = if output.patch_session_commit_id.is_some() {
            if output
                .confirmations
                .iter()
                .any(|item| item.state == RuntimeConfirmationState::Pending)
            {
                Some(PatchWriteBackState::PendingConfirmation)
            } else {
                Some(PatchWriteBackState::NotRequested)
            }
        } else {
            None
        };

        for confirmation in output.confirmations {
            self.state
                .confirmations
                .insert(confirmation.confirmation_id.clone(), confirmation);
        }

        let attempts = self
            .state
            .nodes
            .get(&node_id)
            .map(|node| node.execution_attempts)
            .unwrap_or(1);

        self.state.nodes.insert(
            node_id.clone(),
            WorkflowNodeRuntimeState {
                node_id: node_id.clone(),
                status,
                outputs: output.outputs,
                communication_output: output.communication_output,
                communication_control: output.communication_control,
                prompt_trace_hash: output.prompt_trace_hash,
                patch_session_commit_id: output.patch_session_commit_id,
                checkpoint_id: output.checkpoint_id,
                patch_write_back_state: patch_state,
                metadata: output.metadata,
                error: None,
                error_state: None,
                execution_attempts: attempts,
            },
        );
    }

    /// 记录节点启动事件并固化本次 operation 序号。
    fn record_node_started(&mut self, node_id: &NodeId, attempts: u32) {
        self.state
            .node_operation_sequences
            .insert(node_id.clone(), attempts);
        self.state
            .nodes
            .entry(node_id.clone())
            .and_modify(|node| {
                node.status = RunStatus::Running;
                node.execution_attempts = attempts;
            })
            .or_insert_with(|| WorkflowNodeRuntimeState {
                node_id: node_id.clone(),
                status: RunStatus::Running,
                outputs: PortMap::new(),
                communication_output: None,
                communication_control: CommunicationControl::default(),
                prompt_trace_hash: None,
                patch_session_commit_id: None,
                checkpoint_id: None,
                patch_write_back_state: None,
                metadata: Value::Null,
                error: None,
                error_state: None,
                execution_attempts: attempts,
            });
        self.record_event(
            WorkflowRuntimeEventType::NodeStarted,
            Some(node_id.clone()),
            format!("node {} started attempt {}", node_id.as_str(), attempts),
            json!({ "attempt": attempts }),
        );
    }

    /// 记录节点错误；返回 true 表示已安排重试，false 表示运行应暂停或失败。
    fn record_node_error(&mut self, node_id: NodeId, error: CoreError) -> bool {
        let attempts = self
            .state
            .nodes
            .get(&node_id)
            .map(|node| node.execution_attempts)
            .unwrap_or(1);
        let error_state = classify_node_error(&error, attempts, self.state.retry_policy);
        let retry_scheduled = error_state.retryable && attempts < error_state.max_attempts;
        let status = if retry_scheduled {
            RunStatus::Queued
        } else if error_state.retryable {
            RunStatus::Paused
        } else {
            RunStatus::Failed
        };
        self.state.nodes.insert(
            node_id.clone(),
            WorkflowNodeRuntimeState {
                node_id: node_id.clone(),
                status,
                outputs: PortMap::new(),
                communication_output: None,
                communication_control: CommunicationControl::default(),
                prompt_trace_hash: None,
                patch_session_commit_id: None,
                checkpoint_id: None,
                patch_write_back_state: None,
                metadata: Value::Null,
                error: Some(error_state.message.clone()),
                error_state: Some(error_state.clone()),
                execution_attempts: attempts,
            },
        );
        if retry_scheduled {
            self.record_event(
                WorkflowRuntimeEventType::NodeRetryScheduled,
                Some(node_id.clone()),
                format!(
                    "node {} retry scheduled after {}ms",
                    node_id.as_str(),
                    error_state.next_retry_delay_ms.unwrap_or(0)
                ),
                json!({
                    "attempts": error_state.attempts,
                    "max_attempts": error_state.max_attempts,
                    "next_retry_delay_ms": error_state.next_retry_delay_ms,
                    "next_retry_at_ms": error_state.next_retry_at_ms,
                    "kind": error_state.kind,
                }),
            );
            self.state
                .events
                .push(format!("node {} retry scheduled", node_id.as_str()));
            return true;
        }

        if error_state.retryable {
            self.pause(format!(
                "node {} exhausted retry attempts: {}",
                node_id.as_str(),
                error_state.message
            ));
        } else {
            self.state
                .events
                .push(format!("node {} failed", node_id.as_str()));
            self.record_event(
                WorkflowRuntimeEventType::NodeFailed,
                Some(node_id.clone()),
                format!("node {} failed: {}", node_id.as_str(), error_state.message),
                json!({
                    "attempts": error_state.attempts,
                    "max_attempts": error_state.max_attempts,
                    "kind": error_state.kind,
                    "recovery_suggestion": error_state.recovery_suggestion,
                }),
            );
        }
        false
    }

    /// 推进所有由该节点发出的 communication 边。
    fn advance_communication(
        &mut self,
        workflow: &WorkflowDefinition,
        node_id: &NodeId,
    ) -> CoreResult<()> {
        let outgoing_edges = workflow
            .edges
            .iter()
            .filter(|edge| edge.kind == WorkflowEdgeKind::Communication)
            .filter(|edge| edge.from.node_id == *node_id || edge.to.node_id == *node_id)
            .cloned()
            .collect::<Vec<_>>();

        for edge in outgoing_edges {
            let Some(config) = edge.communication.as_ref() else {
                continue;
            };
            // 读取一份 communication 状态快照用于判断，后续真正修改时再取
            // mutable 引用，避免同时持有可变/不可变借用。
            let state = self
                .state
                .communication_edges
                .get(&edge.id)
                .cloned()
                .ok_or_else(|| CoreError::validation("communication state not initialized"))?;
            if state.completed || state.next_sender_node_id != *node_id {
                continue;
            }

            let Some(node_state) = self.state.nodes.get(node_id) else {
                continue;
            };
            let output = node_state.communication_output.clone().unwrap_or_default();
            let communication_approved = node_state.communication_control.approved;
            let continue_communication = node_state.communication_control.continue_communication;
            if output.trim().is_empty() {
                continue;
            }

            if state.message_count >= state.max_message_count {
                // 次数在本轮发送前已经耗尽，记录在边状态里方便前端定位是哪条
                // communication 边导致 Pause。
                if let Some(communication_state) = self.state.communication_edges.get_mut(&edge.id)
                {
                    communication_state.pause_reason =
                        Some("max_message_count_exhausted".to_owned());
                }
                self.pause("communication max message count exhausted");
                return Ok(());
            }

            let (receiver, alias, template) = communication_receiver(config, &edge, node_id)?;
            let content = render_communication_template(template, alias, &output)?;
            let message = CommunicationMessage {
                edge_id: edge.id.clone(),
                from_node_id: node_id.clone(),
                to_node_id: receiver.clone(),
                alias: alias.to_owned(),
                content,
                message_index: state.message_count + 1,
            };

            let (message_count, last_message_hash) = {
                let communication_state = self.state.communication_edges.get_mut(&edge.id).unwrap();
                // 每条消息按单条计数，不按一来一回计数。last_message_hash 用于审计
                // 和恢复时快速确认最后一条加工消息有没有漂移。
                communication_state.message_count =
                    communication_state.message_count.saturating_add(1);
                communication_state.next_sender_node_id = receiver.clone();
                communication_state.last_message_hash = Some(stable_text_hash(&message.content));
                communication_state.pause_reason = None;
                communication_state.messages.push(message);
                (
                    communication_state.message_count,
                    communication_state.last_message_hash.clone(),
                )
            };
            self.record_event(
                WorkflowRuntimeEventType::CommunicationMessage,
                Some(node_id.clone()),
                format!(
                    "communication {} message {}",
                    edge.id.as_str(),
                    message_count
                ),
                json!({
                    "edge_id": edge.id,
                    "to_node_id": receiver,
                    "alias": alias,
                    "message_index": message_count,
                    "last_message_hash": last_message_hash,
                }),
            );

            let node_ended_reason = if communication_approved {
                Some("approved")
            } else if !continue_communication {
                Some("node_declared_complete")
            } else {
                None
            };
            let should_pause = {
                let communication_state = self.state.communication_edges.get_mut(&edge.id).unwrap();
                if let Some(reason) = node_ended_reason {
                    // 节点可以显式停止本条 communication；approved 也视为自然完成，
                    // 不再继续向对端发送返修消息。
                    communication_state.completed = true;
                    communication_state.completed_reason = Some(reason.to_owned());
                }
                let exhausted = communication_state.message_count
                    >= communication_state.max_message_count
                    && !communication_state.completed;
                if exhausted {
                    communication_state.pause_reason =
                        Some("max_message_count_exhausted".to_owned());
                }
                exhausted
            };
            if should_pause {
                self.pause("communication max message count exhausted");
                return Ok(());
            }
        }
        Ok(())
    }

    /// 推进显式 Loop 节点，只有 bounded loop policy 允许重跑上游节点。
    fn advance_loop(
        &mut self,
        workflow: &WorkflowDefinition,
        node_id: &NodeId,
        loop_control: Option<&RuntimeLoopControl>,
    ) -> CoreResult<()> {
        let Some(loop_control) = loop_control else {
            return Ok(());
        };
        // 只有显式 Loop 节点可以发出 loop_control，避免普通节点伪造闭环导致
        // 隐式无限循环。
        let node = workflow
            .nodes
            .iter()
            .find(|node| node.id == *node_id)
            .ok_or_else(|| CoreError::validation("loop node not found"))?;
        if node.type_name != "loop" {
            return Err(CoreError::validation(format!(
                "node {} emitted loop_control but is not a loop node",
                node_id.as_str()
            )));
        }
        let policy =
            serde_json::from_value::<LoopPolicy>(node.config.clone()).map_err(|error| {
                CoreError::validation(format!("invalid loop policy config: {error}"))
            })?;
        policy.validate()?;

        if !loop_control.continue_loop {
            // Loop 节点判断停止条件已满足，本轮闭环结束，不修改 rerun_queue。
            self.state.events.push(format!(
                "loop {} completed: {}",
                node_id.as_str(),
                loop_control
                    .reason
                    .as_deref()
                    .unwrap_or("condition satisfied")
            ));
            self.record_event(
                WorkflowRuntimeEventType::LoopUpdated,
                Some(node_id.clone()),
                format!("loop {} completed", node_id.as_str()),
                json!({ "reason": loop_control.reason }),
            );
            return Ok(());
        }

        // max_iterations 表示允许触发重跑的次数，而不是 Loop 节点运行次数。
        // 达到上限后进入 Pause，等待用户介入，而不是静默失败或继续循环。
        let current = self
            .state
            .loop_iterations
            .get(node_id)
            .copied()
            .unwrap_or(0);
        if current >= policy.max_iterations {
            self.pause(format!(
                "loop {} max iterations exhausted",
                node_id.as_str()
            ));
            return Ok(());
        }

        // 如果节点输出没有指定目标，就默认使用 Loop 的 control 出边。这样画布
        // 上的显式边仍然是循环结构的唯一来源。
        let rerun_nodes = if loop_control.rerun_node_ids.is_empty() {
            workflow
                .edges
                .iter()
                .filter(|edge| {
                    edge.kind == WorkflowEdgeKind::Control && edge.from.node_id == *node_id
                })
                .map(|edge| edge.to.node_id.clone())
                .collect::<Vec<_>>()
        } else {
            loop_control.rerun_node_ids.clone()
        };

        if rerun_nodes.is_empty() {
            self.pause(format!(
                "loop {} requested continue but has no rerun targets",
                node_id.as_str()
            ));
            return Ok(());
        }

        // 触发重跑时必须清理目标及其 control/data 下游快照。否则下游节点会因为
        // 旧状态是 Succeeded 而被幂等跳过，读到上一轮结果。
        *self
            .state
            .loop_iterations
            .entry(node_id.clone())
            .or_insert(0) += 1;
        self.record_event(
            WorkflowRuntimeEventType::LoopUpdated,
            Some(node_id.clone()),
            format!("loop {} scheduled rerun", node_id.as_str()),
            json!({
                "iteration": self.state.loop_iterations.get(node_id),
                "reason": loop_control.reason,
            }),
        );
        let mut all_affected = Vec::new();
        for target in rerun_nodes {
            if !workflow.nodes.iter().any(|node| node.id == target) {
                return Err(CoreError::validation(format!(
                    "loop {} references missing rerun target {}",
                    node_id.as_str(),
                    target.as_str()
                )));
            }
            collect_downstream_closure(workflow, &target, &mut all_affected);
            if !self.state.rerun_queue.contains(&target) {
                self.state.rerun_queue.push(target);
            }
        }
        for affected_node in &all_affected {
            self.state.nodes.remove(affected_node);
        }
        reset_communication_edges_for_nodes(&mut self.state, &all_affected, workflow);
        Ok(())
    }

    /// 暂停运行并记录原因。
    fn pause(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.state.control = RunControl::Pause;
        self.state.pause_reason = Some(reason.clone());
        self.state.next_retry_at_ms = None;
        self.state.events.push(reason);
        self.state.status = RunStatus::Paused;
        self.record_event(
            WorkflowRuntimeEventType::RunPaused,
            None,
            self.state.pause_reason.clone().unwrap_or_default(),
            Value::Null,
        );
    }

    /// 停止运行并保留当前快照。
    fn stop(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.state.control = RunControl::Stop;
        self.state.stop_reason = Some(reason.clone());
        self.state.next_retry_at_ms = None;
        self.state.events.push(reason);
        self.state.status = RunStatus::Stopped;
        self.record_event(
            WorkflowRuntimeEventType::RunStopped,
            None,
            self.state.stop_reason.clone().unwrap_or_default(),
            Value::Null,
        );
    }

    /// 记录结构化事件并维护单调序列号。
    fn record_event(
        &mut self,
        event_type: WorkflowRuntimeEventType,
        node_id: Option<NodeId>,
        message: impl Into<String>,
        metadata: Value,
    ) {
        let sequence = self.state.next_event_sequence;
        self.state.next_event_sequence = self.state.next_event_sequence.saturating_add(1);
        self.state.structured_events.push(WorkflowRuntimeEvent {
            sequence,
            event_type,
            node_id,
            message: message.into(),
            metadata,
        });
    }
}

/// 如果本轮启用了持久化，则保存当前运行快照。
///
/// 放在单独函数里是为了让调度主循环只表达“何时保存”，不关心具体
/// store 是否存在。
#[derive(Debug, Clone)]
enum OperationJournalAction {
    Execute,
    Replay(Box<WorkflowNodeExecutionOutput>),
    InDoubt,
}

fn operation_is_journaled(policy: crate::workflow::WorkflowOperationPolicy) -> bool {
    matches!(
        policy,
        crate::workflow::WorkflowOperationPolicy::Journaled { .. }
    )
}

/// 只有 provider 明确报告未发送/已收到失败响应时，父 operation 才能安全 abort。
/// 裸 `Cancelled` 的跨模块合同等价于 dispatch 前取消。
fn prepare_operation_journal(
    store: Option<&dyn WorkflowRuntimeStore>,
    request: &WorkflowNodeExecutionRequest,
    policy: crate::workflow::WorkflowOperationPolicy,
    executor: &mut dyn WorkflowNodeExecutor,
) -> CoreResult<OperationJournalAction> {
    let crate::workflow::WorkflowOperationPolicy::Journaled { recovery, response } = policy else {
        return Ok(OperationJournalAction::Execute);
    };
    let Some(store) = store else {
        return Ok(OperationJournalAction::Execute);
    };
    let now_ms = unix_timestamp_ms();
    let existing = store.load_operation(&request.operation_id)?;
    let operation = if let Some(operation) = existing {
        if operation.workflow_id != request.workflow_id
            || operation.run_id != request.run_id
            || operation.node_id != request.node_id
            || operation.attempt != request.operation_attempt
            || operation.request_hash != request.request_hash
            || operation.recovery_policy != recovery
            || operation.response_policy != response
        {
            return Err(CoreError::validation(format!(
                "workflow operation identity mismatch: {}",
                request.operation_id
            )));
        }
        operation
    } else {
        let provider = request
            .config
            .get("provider_id")
            .and_then(Value::as_str)
            .unwrap_or(&request.type_name)
            .to_owned();
        store.create_operation(
            &crate::workflow::NewWorkflowOperation {
                operation_id: request.operation_id.clone(),
                workflow_id: request.workflow_id.clone(),
                run_id: request.run_id.clone(),
                node_id: request.node_id.clone(),
                attempt: request.operation_attempt,
                kind: request.type_name.clone(),
                provider,
                request_hash: request.request_hash.clone(),
                lease_generation: store.operation_lease_generation(),
                recovery_policy: recovery,
                response_policy: response,
            },
            now_ms,
        )?;
        store
            .load_operation(&request.operation_id)?
            .ok_or_else(|| CoreError::validation("workflow operation disappeared after create"))?
    };
    match operation.status {
        crate::workflow::WorkflowOperationStatus::Prepared => Ok(OperationJournalAction::Execute),
        crate::workflow::WorkflowOperationStatus::Dispatched => {
            match operation.recovery_policy {
                crate::workflow::WorkflowOperationRecoveryPolicy::ReplayExecutor => {
                    return Ok(OperationJournalAction::Execute);
                }
                crate::workflow::WorkflowOperationRecoveryPolicy::ReconcileReceipt => {
                    if let Some(output) = executor.reconcile_operation(request)? {
                        complete_operation_journal(Some(store), request, policy, &output)?;
                        return Ok(OperationJournalAction::Replay(Box::new(output)));
                    }
                }
                crate::workflow::WorkflowOperationRecoveryPolicy::ManualResolution => {}
            }
            let _ = store.transition_operation(
                &request.operation_id,
                crate::workflow::WorkflowOperationStatus::Dispatched,
                crate::workflow::WorkflowOperationStatus::InDoubt,
                None,
                now_ms,
            )?;
            Ok(OperationJournalAction::InDoubt)
        }
        crate::workflow::WorkflowOperationStatus::InDoubt => {
            if operation.recovery_policy
                == crate::workflow::WorkflowOperationRecoveryPolicy::ReplayExecutor
            {
                return Ok(OperationJournalAction::Execute);
            }
            if operation.recovery_policy
                == crate::workflow::WorkflowOperationRecoveryPolicy::ReconcileReceipt
            {
                if let Some(output) = executor.reconcile_operation(request)? {
                    let response_json = serde_json::to_value(&output)?;
                    if !store.transition_operation(
                        &request.operation_id,
                        crate::workflow::WorkflowOperationStatus::InDoubt,
                        crate::workflow::WorkflowOperationStatus::Completed,
                        Some(&response_json),
                        unix_timestamp_ms(),
                    )? {
                        return Err(CoreError::validation(
                            "workflow operation changed during receipt reconciliation",
                        ));
                    }
                    return Ok(OperationJournalAction::Replay(Box::new(output)));
                }
            }
            Ok(OperationJournalAction::InDoubt)
        }
        crate::workflow::WorkflowOperationStatus::Aborted => Err(CoreError::validation(
            "aborted workflow operation cannot be dispatched again with the same attempt",
        )),
        crate::workflow::WorkflowOperationStatus::Completed => {
            let response = operation.response_json.ok_or_else(|| {
                CoreError::validation("completed workflow operation response is missing")
            })?;
            Ok(OperationJournalAction::Replay(Box::new(
                serde_json::from_value(response)?,
            )))
        }
        crate::workflow::WorkflowOperationStatus::Committed => Err(CoreError::validation(
            "committed workflow operation has no matching node snapshot",
        )),
    }
}

fn complete_operation_journal(
    store: Option<&dyn WorkflowRuntimeStore>,
    request: &WorkflowNodeExecutionRequest,
    policy: crate::workflow::WorkflowOperationPolicy,
    output: &WorkflowNodeExecutionOutput,
) -> CoreResult<()> {
    if !operation_is_journaled(policy) {
        return Ok(());
    }
    let Some(store) = store else {
        return Ok(());
    };
    let response = serde_json::to_value(output)?;
    request.dispatch_authorization.seal()?;
    match store.complete_operation_response(
        &request.operation_id,
        &response,
        unix_timestamp_ms(),
    )? {
        crate::workflow::WorkflowOperationCompletionOutcome::Completed => Ok(()),
        crate::workflow::WorkflowOperationCompletionOutcome::DispatchAuthorizationMissing => {
            Err(CoreError::WorkflowExecutorContractViolation {
                operation_id: request.operation_id.clone(),
                message: "journaled executor returned success without consuming external dispatch authorization; operation quarantined as in_doubt"
                    .to_owned(),
            })
        }
    }
}

fn settle_operation_failure(
    store: Option<&dyn WorkflowRuntimeStore>,
    request: &WorkflowNodeExecutionRequest,
    policy: crate::workflow::WorkflowOperationPolicy,
    error: &CoreError,
) -> CoreResult<bool> {
    if !operation_is_journaled(policy) {
        return Ok(false);
    }
    let Some(store) = store else {
        return Ok(false);
    };
    let Some(operation) = store.load_operation(&request.operation_id)? else {
        return Ok(false);
    };
    let definitely_settled = matches!(
        error.external_dispatch_outcome(),
        Some(
            crate::contracts::ExternalDispatchOutcome::NotDispatched
                | crate::contracts::ExternalDispatchOutcome::ResponseReceived
        )
    );
    match operation.status {
        crate::workflow::WorkflowOperationStatus::Prepared => {
            if error.external_dispatch_outcome()
                == Some(crate::contracts::ExternalDispatchOutcome::DispatchedUnknown)
            {
                let changed = store.transition_operation(
                    &request.operation_id,
                    crate::workflow::WorkflowOperationStatus::Prepared,
                    crate::workflow::WorkflowOperationStatus::InDoubt,
                    None,
                    unix_timestamp_ms(),
                )?;
                if !changed {
                    return Err(CoreError::validation(
                        "workflow operation changed while preserving an unfenced dispatch",
                    ));
                }
                Ok(true)
            } else {
                if !store.delete_prepared_operation(&request.operation_id)? {
                    return Err(CoreError::validation(
                        "workflow operation changed before prepared cleanup",
                    ));
                }
                Ok(false)
            }
        }
        crate::workflow::WorkflowOperationStatus::InDoubt => Ok(true),
        crate::workflow::WorkflowOperationStatus::Dispatched => {
            let next = if definitely_settled {
                crate::workflow::WorkflowOperationStatus::Aborted
            } else {
                crate::workflow::WorkflowOperationStatus::InDoubt
            };
            if !store.transition_operation(
                &request.operation_id,
                crate::workflow::WorkflowOperationStatus::Dispatched,
                next,
                None,
                unix_timestamp_ms(),
            )? {
                return Err(CoreError::validation(
                    "workflow operation changed during failure settlement",
                ));
            }
            Ok(next == crate::workflow::WorkflowOperationStatus::InDoubt)
        }
        crate::workflow::WorkflowOperationStatus::Aborted => Ok(false),
        crate::workflow::WorkflowOperationStatus::Completed
        | crate::workflow::WorkflowOperationStatus::Committed => Err(CoreError::validation(
            "completed workflow operation cannot fail before state commit",
        )),
    }
}

fn persist_if_needed(
    store: Option<&dyn WorkflowRuntimeStore>,
    state: &mut WorkflowRunState,
) -> CoreResult<()> {
    persist_operation_if_needed(store, state, None)
}

fn persist_operation_if_needed(
    store: Option<&dyn WorkflowRuntimeStore>,
    state: &mut WorkflowRunState,
    commit_operation_id: Option<&str>,
) -> CoreResult<()> {
    let Some(store) = store else {
        return Ok(());
    };
    for _ in 0..8 {
        match store.save_state(state, commit_operation_id) {
            Ok(()) => return Ok(()),
            Err(CoreError::WorkflowStateRevisionConflict { .. }) => {
                let latest = store
                    .load_state(&state.workflow_id, &state.run_id)?
                    .ok_or_else(|| CoreError::WorkflowRunNotFound {
                        workflow_id: state.workflow_id.as_str().to_owned(),
                        run_id: state.run_id.as_str().to_owned(),
                    })?;
                rebase_worker_state_after_conflict(state, latest);
            }
            Err(error) => return Err(error),
        }
    }
    Err(CoreError::validation(
        "workflow state CAS retry limit exceeded",
    ))
}

fn rebase_worker_state_after_conflict(
    worker_state: &mut WorkflowRunState,
    mut latest: WorkflowRunState,
) {
    let structured_prefix = worker_state
        .structured_events
        .iter()
        .zip(&latest.structured_events)
        .take_while(|(left, right)| left == right)
        .count();
    let local_structured_tail = worker_state.structured_events[structured_prefix..].to_vec();
    let legacy_prefix = worker_state
        .events
        .iter()
        .zip(&latest.events)
        .take_while(|(left, right)| left == right)
        .count();
    let local_legacy_tail = worker_state.events[legacy_prefix..].to_vec();

    latest.nodes.extend(worker_state.nodes.clone());
    for (node_id, sequence) in &worker_state.node_operation_sequences {
        latest
            .node_operation_sequences
            .entry(node_id.clone())
            .and_modify(|persisted| *persisted = (*persisted).max(*sequence))
            .or_insert(*sequence);
    }
    latest
        .communication_edges
        .extend(worker_state.communication_edges.clone());
    latest
        .loop_iterations
        .extend(worker_state.loop_iterations.clone());
    latest.rerun_queue = worker_state.rerun_queue.clone();
    for (confirmation_id, confirmation) in &worker_state.confirmations {
        latest
            .confirmations
            .entry(confirmation_id.clone())
            .or_insert_with(|| confirmation.clone());
    }
    latest.retry_policy = worker_state.retry_policy;
    if latest.control == RunControl::Continue {
        latest.status = worker_state.status;
        latest.next_retry_at_ms = worker_state.next_retry_at_ms;
        latest.failure = worker_state.failure.clone();
    }
    latest.events.extend(local_legacy_tail);
    for mut event in local_structured_tail {
        event.sequence = latest.next_event_sequence;
        latest.next_event_sequence = latest.next_event_sequence.saturating_add(1);
        latest.structured_events.push(event);
    }
    *worker_state = latest;
}

fn refresh_external_control(
    store: Option<&dyn WorkflowRuntimeStore>,
    state: &mut WorkflowRunState,
) -> CoreResult<()> {
    let Some(store) = store else {
        return Ok(());
    };
    let Some(latest) = store.load_state(&state.workflow_id, &state.run_id)? else {
        return Ok(());
    };
    if latest.status.is_terminal() {
        *state = latest;
        return Ok(());
    }
    // 控制命令与 worker 事件共享同一 revision/event 序列。只复制 control 会丢失
    // StopRequested 事件并造成 sequence 冲突，因此复用 CAS rebase 合并完整快照。
    rebase_worker_state_after_conflict(state, latest);
    Ok(())
}

/// 返回节点此前记录的执行次数，用于成功/暂停输出不清零尝试计数。
fn previous_attempts(state: &WorkflowRunState, node_id: &NodeId) -> u32 {
    let node_attempts = state
        .nodes
        .get(node_id)
        .map(|node| node.execution_attempts)
        .unwrap_or(0);
    state
        .node_operation_sequences
        .get(node_id)
        .copied()
        .unwrap_or(0)
        .max(node_attempts)
}

/// 返回下一次 operation 序号。人工尚未裁决的 in_doubt 恢复必须复用原序号；
/// 正常重试、Loop 和回注重跑则分配新序号。
fn next_node_operation_attempt(state: &WorkflowRunState, node_id: &NodeId) -> u32 {
    let previous = previous_attempts(state, node_id);
    let resumes_in_doubt = state.nodes.get(node_id).is_some_and(|node| {
        node.status == RunStatus::Paused && node.error.is_some() && node.error_state.is_none()
    });
    if resumes_in_doubt && previous > 0 {
        previous
    } else {
        previous.saturating_add(1)
    }
}

/// 将 CoreError 转成节点级错误状态。
fn classify_node_error(
    error: &CoreError,
    attempts: u32,
    retry_policy: NodeRetryPolicy,
) -> NodeErrorState {
    let kind = node_error_kind(error);
    let retryable = matches!(
        kind,
        NodeErrorKind::Retryable | NodeErrorKind::ToolArguments
    );
    let next_retry_delay_ms = if retryable && attempts < retry_policy.max_attempts {
        Some(retry_delay_ms(retry_policy, attempts))
    } else {
        None
    };
    let next_retry_at_ms =
        next_retry_delay_ms.map(|delay| unix_timestamp_ms().saturating_add(delay));
    NodeErrorState {
        kind,
        message: error.to_string(),
        attempts,
        max_attempts: retry_policy.max_attempts,
        retryable,
        next_retry_delay_ms,
        next_retry_at_ms,
        recovery_suggestion: recovery_suggestion(kind, retryable, next_retry_delay_ms.is_some()),
    }
}

/// 根据错误类型和消息做保守分类，避免把权限/预算错误自动重试。
fn node_error_kind(error: &CoreError) -> NodeErrorKind {
    match error {
        CoreError::PermissionDenied { .. } => NodeErrorKind::Permission,
        CoreError::BudgetExceeded { .. } | CoreError::Paused { .. } => NodeErrorKind::Budget,
        CoreError::Cancelled
        | CoreError::ExternalCancellation { .. }
        | CoreError::Stopped { .. } => NodeErrorKind::Cancelled,
        CoreError::External { message, .. } => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("timeout")
                || lower.contains("rate limit")
                || lower.contains("429")
                || lower.contains("503")
                || lower.contains("504")
            {
                NodeErrorKind::Retryable
            } else {
                NodeErrorKind::External
            }
        }
        CoreError::ProviderRequest {
            outcome, message, ..
        }
        | CoreError::ExternalOperation {
            outcome, message, ..
        } => {
            if *outcome == crate::contracts::ExternalDispatchOutcome::NotDispatched {
                NodeErrorKind::Retryable
            } else {
                let lower = message.to_ascii_lowercase();
                if lower.contains("rate limit")
                    || lower.contains("429")
                    || lower.contains("503")
                    || lower.contains("504")
                {
                    NodeErrorKind::Retryable
                } else {
                    NodeErrorKind::External
                }
            }
        }
        CoreError::Validation { message } => {
            let lower = message_or_error_text(error, message).to_ascii_lowercase();
            if lower.contains("schema")
                || lower.contains("argument")
                || lower.contains("tool")
                || lower.contains("json")
            {
                NodeErrorKind::ToolArguments
            } else {
                NodeErrorKind::System
            }
        }
        CoreError::Json(_) => NodeErrorKind::ToolArguments,
        CoreError::Io(_) | CoreError::Yaml(_) | CoreError::ResourceLimitExceeded { .. } => {
            NodeErrorKind::System
        }
        CoreError::RegistryDuplicate { .. }
        | CoreError::RegistryMissing { .. }
        | CoreError::PortMissing { .. }
        | CoreError::PortTypeMismatch { .. }
        | CoreError::WorkflowStateRevisionConflict { .. }
        | CoreError::WorkflowRunNotFound { .. }
        | CoreError::WorkflowExecutorContractViolation { .. } => NodeErrorKind::System,
    }
}

/// 兼容 Validation 和 JSON 错误的分类文本。
fn message_or_error_text(error: &CoreError, validation_message: &str) -> String {
    if validation_message.trim().is_empty() {
        error.to_string()
    } else {
        validation_message.to_owned()
    }
}

/// 计算第 attempts 次失败后的指数退避延迟。
fn retry_delay_ms(policy: NodeRetryPolicy, attempts: u32) -> u64 {
    let exponent = attempts.saturating_sub(1);
    let multiplier = u64::from(policy.backoff_multiplier).saturating_pow(exponent);
    policy.initial_backoff_ms.saturating_mul(multiplier)
}

/// 当前 UNIX 毫秒时间戳；系统时间异常时退回 0，避免调度路径 panic。
fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// 给前端和恢复流程的人类可读建议。
fn recovery_suggestion(kind: NodeErrorKind, retryable: bool, retry_scheduled: bool) -> String {
    if retry_scheduled {
        return "等待退避后自动重试；也可以手动暂停并检查 provider 或工具参数".to_owned();
    }
    if retryable {
        return "重试次数已耗尽；请检查网络、provider 状态或工具参数后手动恢复".to_owned();
    }
    match kind {
        NodeErrorKind::Permission => "检查项目权限配置后重试".to_owned(),
        NodeErrorKind::Budget => "调整预算或审批后继续运行".to_owned(),
        NodeErrorKind::Cancelled => "运行已取消或停止；需要用户重新启动".to_owned(),
        NodeErrorKind::External => "检查外部 provider/service 健康状态后重试".to_owned(),
        NodeErrorKind::System => "检查 runtime.db、文件锁、磁盘和索引状态后进入恢复流程".to_owned(),
        NodeErrorKind::Retryable | NodeErrorKind::ToolArguments | NodeErrorKind::Unknown => {
            "检查错误详情后手动恢复".to_owned()
        }
    }
}

/// 计算当前可运行节点。
#[derive(Debug)]
struct WorkflowGraphIndex {
    node_positions: HashMap<NodeId, usize>,
    dependency_edges: HashMap<NodeId, Vec<usize>>,
    data_edges: HashMap<NodeId, Vec<usize>>,
    dependent_nodes: HashMap<NodeId, Vec<NodeId>>,
    communication_neighbors: HashMap<NodeId, Vec<NodeId>>,
    communication_nodes: HashSet<NodeId>,
    loop_nodes: HashSet<NodeId>,
}

impl WorkflowGraphIndex {
    fn build(workflow: &WorkflowDefinition) -> Self {
        let node_positions = workflow
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let loop_nodes = workflow
            .nodes
            .iter()
            .filter(|node| node.type_name == "loop")
            .map(|node| node.id.clone())
            .collect::<HashSet<_>>();
        let mut dependency_edges = HashMap::<NodeId, Vec<usize>>::new();
        let mut data_edges = HashMap::<NodeId, Vec<usize>>::new();
        let mut dependent_nodes = HashMap::<NodeId, Vec<NodeId>>::new();
        let mut communication_neighbors = HashMap::<NodeId, Vec<NodeId>>::new();
        let mut communication_nodes = HashSet::new();
        for (index, edge) in workflow.edges.iter().enumerate() {
            match edge.kind {
                WorkflowEdgeKind::Control => {
                    dependency_edges
                        .entry(edge.to.node_id.clone())
                        .or_default()
                        .push(index);
                    dependent_nodes
                        .entry(edge.from.node_id.clone())
                        .or_default()
                        .push(edge.to.node_id.clone());
                }
                WorkflowEdgeKind::Data => {
                    dependency_edges
                        .entry(edge.to.node_id.clone())
                        .or_default()
                        .push(index);
                    data_edges
                        .entry(edge.to.node_id.clone())
                        .or_default()
                        .push(index);
                    dependent_nodes
                        .entry(edge.from.node_id.clone())
                        .or_default()
                        .push(edge.to.node_id.clone());
                }
                WorkflowEdgeKind::Communication => {
                    communication_nodes.insert(edge.from.node_id.clone());
                    communication_nodes.insert(edge.to.node_id.clone());
                    communication_neighbors
                        .entry(edge.from.node_id.clone())
                        .or_default()
                        .push(edge.to.node_id.clone());
                    communication_neighbors
                        .entry(edge.to.node_id.clone())
                        .or_default()
                        .push(edge.from.node_id.clone());
                }
            }
        }
        Self {
            node_positions,
            dependency_edges,
            data_edges,
            dependent_nodes,
            communication_neighbors,
            communication_nodes,
            loop_nodes,
        }
    }

    fn node<'a>(
        &self,
        workflow: &'a WorkflowDefinition,
        node_id: &NodeId,
    ) -> Option<&'a crate::contracts::NodeInstance> {
        self.node_positions
            .get(node_id)
            .and_then(|index| workflow.nodes.get(*index))
    }

    fn dependency_edge_indices(&self, node_id: &NodeId) -> &[usize] {
        self.dependency_edges
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn data_edge_indices(&self, node_id: &NodeId) -> &[usize] {
        self.data_edges
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn dependent_nodes(&self, node_id: &NodeId) -> &[NodeId] {
        self.dependent_nodes
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn communication_neighbors(&self, node_id: &NodeId) -> &[NodeId] {
        self.communication_neighbors
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

struct ReadyQueue {
    queue: VecDeque<NodeId>,
    queued: HashSet<NodeId>,
}

impl ReadyQueue {
    fn from_nodes(nodes: impl IntoIterator<Item = NodeId>) -> Self {
        let mut ready = Self {
            queue: VecDeque::new(),
            queued: HashSet::new(),
        };
        ready.extend(nodes);
        ready
    }

    fn extend(&mut self, nodes: impl IntoIterator<Item = NodeId>) {
        for node_id in nodes {
            if self.queued.insert(node_id.clone()) {
                self.queue.push_back(node_id);
            }
        }
    }

    fn pop(&mut self) -> Option<NodeId> {
        let node_id = self.queue.pop_front()?;
        self.queued.remove(&node_id);
        Some(node_id)
    }
}

fn ready_nodes(
    workflow: &WorkflowDefinition,
    graph_index: &WorkflowGraphIndex,
    state: &WorkflowRunState,
) -> Vec<NodeId> {
    let mut ready = Vec::new();
    for node_id in &state.rerun_queue {
        if graph_index.node_positions.contains_key(node_id) {
            ready.push(node_id.clone());
        }
    }
    if !ready.is_empty() {
        return ready;
    }
    for node in &workflow.nodes {
        let succeeded = state
            .nodes
            .get(&node.id)
            .is_some_and(|node| node.status == RunStatus::Succeeded);
        let pending_communication = has_pending_communication_for_node(state, &node.id);
        if succeeded && !pending_communication {
            continue;
        }
        if retry_backoff_ready(state, &node.id)
            && (pending_communication
                || (!succeeded
                    && dependencies_satisfied(workflow, graph_index, state, &node.id)
                    && communication_start_ready(graph_index, state, &node.id)))
        {
            ready.push(node.id.clone());
        }
    }
    ready
}

fn node_is_ready(
    workflow: &WorkflowDefinition,
    graph_index: &WorkflowGraphIndex,
    state: &WorkflowRunState,
    node_id: &NodeId,
) -> bool {
    if state.rerun_queue.contains(node_id) {
        return graph_index.node_positions.contains_key(node_id);
    }
    let succeeded = state
        .nodes
        .get(node_id)
        .is_some_and(|node| node.status == RunStatus::Succeeded);
    let pending_communication = has_pending_communication_for_node(state, node_id);
    if succeeded && !pending_communication {
        return false;
    }
    retry_backoff_ready(state, node_id)
        && (pending_communication
            || (!succeeded
                && dependencies_satisfied(workflow, graph_index, state, node_id)
                && communication_start_ready(graph_index, state, node_id)))
}

/// 判断排队节点的退避窗口是否已经到期。
fn retry_backoff_ready(state: &WorkflowRunState, node_id: &NodeId) -> bool {
    let Some(node) = state.nodes.get(node_id) else {
        return true;
    };
    if node.status != RunStatus::Queued {
        return true;
    }
    let Some(next_retry_at_ms) = node
        .error_state
        .as_ref()
        .and_then(|error| error.next_retry_at_ms)
    else {
        return true;
    };
    unix_timestamp_ms() >= next_retry_at_ms
}

/// 返回当前运行最早的未到期退避时间。
fn next_pending_retry_at_ms(state: &WorkflowRunState) -> Option<u64> {
    let now = unix_timestamp_ms();
    state
        .nodes
        .values()
        .filter(|node| node.status == RunStatus::Queued)
        .filter_map(|node| {
            node.error_state
                .as_ref()
                .and_then(|error| error.next_retry_at_ms)
        })
        .filter(|next_retry_at_ms| *next_retry_at_ms > now)
        .min()
}

fn should_pause_for_breakpoint(
    state: &mut WorkflowRunState,
    node_instance: &crate::contracts::NodeInstance,
) -> bool {
    let breakpoint_enabled = node_instance
        .config
        .get("breakpoint")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !breakpoint_enabled {
        return false;
    }
    let node = state
        .nodes
        .entry(node_instance.id.clone())
        .or_insert_with(|| WorkflowNodeRuntimeState {
            node_id: node_instance.id.clone(),
            status: RunStatus::Queued,
            outputs: PortMap::new(),
            communication_output: None,
            communication_control: CommunicationControl::default(),
            prompt_trace_hash: None,
            patch_session_commit_id: None,
            checkpoint_id: None,
            patch_write_back_state: None,
            metadata: Value::Null,
            error: None,
            error_state: None,
            execution_attempts: 0,
        });
    let already_paused = node
        .metadata
        .get("breakpoint_consumed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if already_paused {
        return false;
    }
    let mut metadata = node.metadata.as_object().cloned().unwrap_or_default();
    metadata.insert("breakpoint_consumed".to_owned(), Value::Bool(true));
    node.metadata = Value::Object(metadata);
    true
}

/// control/data 边依赖全部满足后节点才可运行。
fn dependencies_satisfied(
    workflow: &WorkflowDefinition,
    graph_index: &WorkflowGraphIndex,
    state: &WorkflowRunState,
    node_id: &NodeId,
) -> bool {
    graph_index
        .dependency_edge_indices(node_id)
        .iter()
        .filter_map(|index| workflow.edges.get(*index))
        .all(|edge| {
            // Loop 回边是显式循环触发器，不应阻塞首轮运行。首轮时 Loop 节点还
            // 没有状态，因此把未完成的 Loop 回边视为已满足。
            if graph_index.loop_nodes.contains(&edge.from.node_id)
                && !state.nodes.contains_key(&edge.from.node_id)
            {
                return true;
            }
            state
                .nodes
                .get(&edge.from.node_id)
                .is_some_and(|node| node.status == RunStatus::Succeeded)
        })
}

/// 收集从某个节点出发的 control/data 下游闭包。
fn collect_downstream_closure(
    workflow: &WorkflowDefinition,
    node_id: &NodeId,
    affected: &mut Vec<NodeId>,
) {
    let mut seen = affected.iter().cloned().collect::<HashSet<_>>();
    collect_downstream_closure_inner(workflow, node_id, affected, &mut seen);
}

fn collect_downstream_closure_inner(
    workflow: &WorkflowDefinition,
    node_id: &NodeId,
    affected: &mut Vec<NodeId>,
    seen: &mut HashSet<NodeId>,
) {
    if !seen.insert(node_id.clone()) {
        return;
    }
    affected.push(node_id.clone());
    for edge in workflow
        .edges
        .iter()
        .filter(|edge| edge.from.node_id == *node_id)
        .filter(|edge| {
            matches!(
                edge.kind,
                WorkflowEdgeKind::Control | WorkflowEdgeKind::Data
            )
        })
    {
        collect_downstream_closure_inner(workflow, &edge.to.node_id, affected, seen);
    }
}

/// 判断是否有通信消息等待该节点处理。
fn has_pending_communication_for_node(state: &WorkflowRunState, node_id: &NodeId) -> bool {
    state.communication_edges.values().any(|communication| {
        !communication.completed
            && communication.next_sender_node_id == *node_id
            && !communication.messages.is_empty()
    })
}

/// communication 初始阶段只允许发起方先运行，避免接收方在没有消息时抢跑。
fn communication_start_ready(
    graph_index: &WorkflowGraphIndex,
    state: &WorkflowRunState,
    node_id: &NodeId,
) -> bool {
    if !graph_index.communication_nodes.contains(node_id) {
        return true;
    }

    state.communication_edges.values().any(|communication| {
        communication.next_sender_node_id == *node_id && communication.messages.is_empty()
    })
}

/// 判断所有节点是否都成功。
fn all_nodes_succeeded(workflow: &WorkflowDefinition, state: &WorkflowRunState) -> bool {
    workflow.nodes.iter().all(|node| {
        state
            .nodes
            .get(&node.id)
            .is_some_and(|node| node.status == RunStatus::Succeeded)
    })
}

/// 汇总数据边输入。
fn collect_data_inputs(
    workflow: &WorkflowDefinition,
    graph_index: &WorkflowGraphIndex,
    state: &WorkflowRunState,
    node_id: &NodeId,
) -> CoreResult<PortMap> {
    let mut inputs = PortMap::new();
    for edge in graph_index
        .data_edge_indices(node_id)
        .iter()
        .filter_map(|index| workflow.edges.get(*index))
    {
        let Some(alias) = &edge.alias else {
            return Err(CoreError::validation(format!(
                "data edge {} to node {} requires a non-empty alias",
                edge.id.as_str(),
                node_id.as_str()
            )));
        };
        let source = state.nodes.get(&edge.from.node_id).ok_or_else(|| {
            CoreError::validation(format!(
                "data edge {} source node {} has no runtime state",
                edge.id.as_str(),
                edge.from.node_id.as_str()
            ))
        })?;
        let value = source.outputs.get(&edge.from.port_name).ok_or_else(|| {
            CoreError::validation(format!(
                "data edge {} source node {} has no output port {}",
                edge.id.as_str(),
                edge.from.node_id.as_str(),
                edge.from.port_name
            ))
        })?;
        inputs.insert(alias.clone(), value.clone());
    }
    Ok(inputs)
}

/// 收集发给目标节点的 communication 消息。
fn collect_inbound_messages(
    state: &WorkflowRunState,
    node_id: &NodeId,
) -> Vec<CommunicationMessage> {
    state
        .communication_edges
        .values()
        .flat_map(|edge| edge.messages.iter())
        .filter(|message| message.to_node_id == *node_id)
        .cloned()
        .collect()
}

/// 计算当前消息接收方和模板。
fn communication_receiver<'a>(
    config: &'a CommunicationEdgeConfig,
    edge: &'a Edge,
    sender: &NodeId,
) -> CoreResult<(NodeId, &'a str, &'a str)> {
    if *sender == edge.from.node_id {
        Ok((
            edge.to.node_id.clone(),
            config.forward_alias.as_str(),
            config.forward_template.as_str(),
        ))
    } else if *sender == edge.to.node_id {
        Ok((
            edge.from.node_id.clone(),
            config.reverse_alias.as_str(),
            config.reverse_template.as_str(),
        ))
    } else {
        Err(CoreError::validation(
            "communication sender is not an edge endpoint",
        ))
    }
}

/// 渲染 communication 边内模板。
fn render_communication_template(template: &str, alias: &str, output: &str) -> CoreResult<String> {
    let variable = format!("{{{{input.{alias}}}}}");
    if !template.contains(&variable) {
        return Err(CoreError::validation(
            "communication template does not reference expected alias",
        ));
    }
    Ok(template.replace(&variable, output))
}

/// 检查单个 PortValue 引用是否仍可解析。
fn check_port_value_reference(
    report: &mut RuntimeRecoveryReport,
    resolver: &dyn RuntimeReferenceResolver,
    node_id: &NodeId,
    field_name: &str,
    value: &PortValue,
) -> CoreResult<()> {
    match value {
        PortValue::Inline { .. } => {}
        PortValue::DocumentRef { document_id, .. } => {
            let exists = resolver.document_exists(document_id)?;
            record_reference_check(
                report,
                RuntimeReferenceKind::Document,
                document_id,
                node_id,
                field_name,
                exists,
            );
        }
        PortValue::ChunkRef { chunk_id } => {
            let exists = resolver.chunk_exists(chunk_id)?;
            record_reference_check(
                report,
                RuntimeReferenceKind::Chunk,
                chunk_id,
                node_id,
                field_name,
                exists,
            );
        }
        PortValue::ArtifactRef { artifact_id } => {
            let exists = resolver.artifact_exists(artifact_id)?;
            record_reference_check(
                report,
                RuntimeReferenceKind::Artifact,
                artifact_id,
                node_id,
                field_name,
                exists,
            );
        }
    }
    Ok(())
}

/// 记录单个引用检查结果，缺失时写入恢复报告。
fn record_reference_check(
    report: &mut RuntimeRecoveryReport,
    kind: RuntimeReferenceKind,
    id: &str,
    node_id: &NodeId,
    field_name: &str,
    exists: bool,
) {
    report.checked_reference_count += 1;
    if !exists {
        report.missing_references.push(RuntimeReferenceCheck {
            kind,
            id: id.to_owned(),
            node_id: node_id.clone(),
            field_name: field_name.to_owned(),
            exists,
            message: format!(
                "missing {:?} reference {} for node {} field {}",
                kind,
                id,
                node_id.as_str(),
                field_name
            ),
        });
    }
}

/// 重置涉及指定节点集合的 communication 边运行状态，使返修循环能正确重新触发。
///
/// Loop 重跑和路径 A 注入时，被清理节点的 communication 边可能仍保留上一轮的
/// `completed = true`、旧消息列表和 `message_count`，导致重跑后
/// `advance_communication` 跳过通信。此函数将这些边重置为初始状态。
fn reset_communication_edges_for_nodes(
    state: &mut WorkflowRunState,
    affected_nodes: &[NodeId],
    workflow: &WorkflowDefinition,
) {
    let affected_set: HashSet<&NodeId> = affected_nodes.iter().collect();
    for edge in &workflow.edges {
        if edge.kind != WorkflowEdgeKind::Communication {
            continue;
        }
        // 仅当边的两端节点至少有一个在 affected 集合中时才重置
        if !affected_set.contains(&edge.from.node_id) && !affected_set.contains(&edge.to.node_id) {
            continue;
        }
        if let Some(comm) = state.communication_edges.get_mut(&edge.id) {
            let initiator = edge
                .communication
                .as_ref()
                .map(|config| config.initiator_for_edge(edge).clone())
                .unwrap_or_else(|| comm.initiator_node_id.clone());
            comm.initiator_node_id = initiator.clone();
            comm.next_sender_node_id = initiator;
            comm.completed = false;
            comm.completed_reason = None;
            comm.pause_reason = None;
            comm.message_count = 0;
            comm.last_message_hash = None;
            comm.messages.clear();
            if let Some(config) = edge.communication.as_ref() {
                comm.max_message_count = config.max_communication_count;
            }
        }
    }
}

/// communication 控制字段的 serde 默认值。
fn default_continue_communication() -> bool {
    true
}

/// run control 字段的 serde 默认值。
fn default_run_control() -> RunControl {
    RunControl::Continue
}
