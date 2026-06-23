use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("validation failed: {message}")]
    Validation { message: String },

    #[error("registry entry already exists in {registry}: {key}")]
    RegistryDuplicate { registry: &'static str, key: String },

    #[error("registry entry not found in {registry}: {key}")]
    RegistryMissing { registry: &'static str, key: String },

    #[error("missing required port: {port}")]
    PortMissing { port: String },

    #[error("port type mismatch for {port}: expected {expected}, got {actual}")]
    PortTypeMismatch {
        port: String,
        expected: String,
        actual: String,
    },

    #[error("permission denied for {action}: {reason}")]
    PermissionDenied { action: String, reason: String },

    #[error("budget exceeded: limit ${limit_usd:.4}, requested ${requested_usd:.4}")]
    BudgetExceeded { limit_usd: f64, requested_usd: f64 },

    #[error("resource limit exceeded for {resource}: {reason}")]
    ResourceLimitExceeded { resource: String, reason: String },

    #[error("external service error from {service}: {message}")]
    External { service: String, message: String },

    #[error("operation cancelled")]
    Cancelled,

    #[error("run is paused: {reason}")]
    Paused { reason: String },

    #[error("run is stopped: {reason}")]
    Stopped { reason: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("yaml error: {0}")]
    Yaml(#[from] yaml_serde::Error),
}

impl CoreError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }
}
