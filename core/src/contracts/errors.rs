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
    ///
    /// `reason` 说的是**哪一道门拦的**（日限额 / 月限额 / Auto Mode 预授权耗尽 /
    /// 单次调用上限）。三者数字可能相同而下一步动作完全不同：
    /// 日限额要改设置、Auto Mode 额度耗尽要重新授权。缺了它，
    /// 界面只能说「预算已达上限」而说不出该去改哪里。
    #[error("budget exceeded: limit ${limit_usd:.4}, requested ${requested_usd:.4}{}", reason.as_ref().map(|r| format!(" ({r})")).unwrap_or_default())]
    BudgetExceeded {
        limit_usd: f64,
        requested_usd: f64,
        reason: Option<String>,
    },

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

    /// U196-A：文档写入的乐观并发（CAS）校验失败 —— 手上的 base_version
    /// 已经不是磁盘上的当前版本。
    ///
    /// **不要用 `validation` 表达这件事**：它会让作者读到「输入内容不符合要求，
    /// 请检查后重试」，而他的输入完全合法，真实原因是正文在别处被改过
    /// （另一个页面、工作流写回、或上一次保存尚未落盘完成）。
    /// 正确文案 `ui.error.conflict`（「内容已被其它操作更新」）**早就存在**。
    ///
    /// 与 `DocumentAlreadyExists` 的区别：那条是「不该有的东西已经有了」
    /// （create-only 语义），这条是「有的东西变了」（update 语义）。
    /// 两者的下一步动作不同：前者要改名或声明覆盖，后者要刷新后重做。
    #[error(
        "document version conflict: {} expected version {expected_version}, found {actual_version}",
        path.display()
    )]
    DocumentVersionConflict {
        path: PathBuf,
        // ⚠️ `String` 而非数字：文档版本是内容哈希一类的不透明标识
        // （`documents/models.rs:51` 的 `pub version: String`），
        // 按数字建模会在这里编译不过，也会误导后人以为它单调递增、可比大小。
        expected_version: String,
        actual_version: String,
    },

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
