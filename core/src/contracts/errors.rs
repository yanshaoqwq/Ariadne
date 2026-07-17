use std::path::PathBuf;

use thiserror::Error;

/// 项目内部统一 Result 类型。
pub type CoreResult<T> = Result<T, CoreError>;

/// 外部调用失败时请求相对远端副作用边界的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalDispatchOutcome {
    /// 构建请求或建立连接前失败，远端确定未接收。
    NotDispatched,
    /// 请求可能已经发送，但没有取得可判定响应。
    DispatchedUnknown,
    /// 已收到 HTTP/协议响应，远端结果明确为失败。
    ResponseReceived,
}

/// 跨模块共享错误类型。
#[derive(Debug, Error)]
pub enum CoreError {
    /// 参数、配置或状态校验失败。
    #[error("validation failed: {message}")]
    Validation { message: String },

    /// 注册表中已存在相同 key。
    #[error("registry entry already exists in {registry}: {key}")]
    RegistryDuplicate { registry: &'static str, key: String },

    /// 注册表中找不到指定 key。
    #[error("registry entry not found in {registry}: {key}")]
    RegistryMissing { registry: &'static str, key: String },

    /// 必填端口缺失。
    #[error("missing required port: {port}")]
    PortMissing { port: String },

    /// 端口值类型不符合定义。
    #[error("port type mismatch for {port}: expected {expected}, got {actual}")]
    PortTypeMismatch {
        port: String,
        expected: String,
        actual: String,
    },

    /// 权限硬限制拒绝。
    #[error("permission denied for {action}: {reason}")]
    PermissionDenied { action: String, reason: String },

    /// 预算硬限制超限。
    #[error("budget exceeded: limit ${limit_usd:.4}, requested ${requested_usd:.4}")]
    BudgetExceeded { limit_usd: f64, requested_usd: f64 },

    /// 运行时资源限制超限。
    #[error("resource limit exceeded for {resource}: {reason}")]
    ResourceLimitExceeded { resource: String, reason: String },

    /// 外部服务错误。
    #[error("external service error from {service}: {message}")]
    External { service: String, message: String },

    /// 带明确发送阶段的 Provider 传输/协议错误，保留 provider 专用诊断语义。
    #[error("provider request error from {service} ({outcome:?}): {message}")]
    ProviderRequest {
        service: String,
        outcome: ExternalDispatchOutcome,
        message: String,
    },

    /// 非 Provider 外部操作（HTTP Skill 等）的发送阶段错误。
    #[error("external operation error from {service} ({outcome:?}): {message}")]
    ExternalOperation {
        service: String,
        outcome: ExternalDispatchOutcome,
        message: String,
    },

    /// 外部操作取消，显式携带取消发生在 dispatch 边界的哪一侧。
    #[error("external operation cancelled in {service} ({outcome:?})")]
    ExternalCancellation {
        service: String,
        outcome: ExternalDispatchOutcome,
    },

    /// 远端可能已经执行，但该操作声明为 at-most-once，运行时已禁止重发并自动终止。
    #[error("external operation outcome is unknown for at-most-once operation {operation_id}: {message}")]
    ExternalOutcomeUnknown {
        operation_id: String,
        message: String,
    },

    /// 本地操作被取消；不隐含外部请求是否 dispatch。外部适配器必须返回带
    /// `ExternalDispatchOutcome` 的错误，journal 才能判定安全重试或 in_doubt。
    #[error("operation cancelled")]
    Cancelled,

    /// 运行已暂停。
    #[error("run is paused: {reason}")]
    Paused { reason: String },

    /// 运行已停止。
    #[error("run is stopped: {reason}")]
    Stopped { reason: String },

    /// 工作流快照已被其它命令或 worker 更新，调用方必须重载后重放意图。
    #[error(
        "workflow state revision conflict for {workflow_id}/{run_id}: expected {expected}, actual {actual}"
    )]
    WorkflowStateRevisionConflict {
        workflow_id: String,
        run_id: String,
        expected: u64,
        actual: u64,
    },

    /// create-only 文档写入发现目标已经存在；调用方必须显式声明覆盖意图。
    #[error("document already exists: {}", path.display())]
    DocumentAlreadyExists { path: PathBuf },

    /// 指定工作流运行不存在，save 不得隐式复活已删除记录。
    #[error("workflow run not found: {workflow_id}/{run_id}")]
    WorkflowRunNotFound { workflow_id: String, run_id: String },

    /// journaled 节点执行器违反外部副作用协议；operation 已被隔离，禁止静默重试。
    #[error("workflow executor contract violation for operation {operation_id}: {message}")]
    WorkflowExecutorContractViolation {
        operation_id: String,
        message: String,
    },

    /// IO 错误。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON 错误。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// YAML 错误。
    #[error("yaml error: {0}")]
    Yaml(#[from] yaml_serde::Error),
}

impl CoreError {
    /// 创建校验错误。
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    pub fn external_cancelled(
        service: impl Into<String>,
        outcome: ExternalDispatchOutcome,
    ) -> Self {
        Self::ExternalCancellation {
            service: service.into(),
            outcome,
        }
    }

    pub fn external_dispatch_outcome(&self) -> Option<ExternalDispatchOutcome> {
        match self {
            Self::ProviderRequest { outcome, .. }
            | Self::ExternalOperation { outcome, .. }
            | Self::ExternalCancellation { outcome, .. } => Some(*outcome),
            _ => None,
        }
    }
}
