use thiserror::Error;

/// 项目内部统一 Result 类型。
pub type CoreResult<T> = Result<T, CoreError>;

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

    /// 操作被取消。
    #[error("operation cancelled")]
    Cancelled,

    /// 运行已暂停。
    #[error("run is paused: {reason}")]
    Paused { reason: String },

    /// 运行已停止。
    #[error("run is stopped: {reason}")]
    Stopped { reason: String },

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
}
