//! Structured command/IPC errors (U1).
//!
//! Command code consumes [`CommandErrorCode`] instead of inferring product
//! behavior from diagnostic text. Free-form diagnostics are secondary only.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::contracts::CoreError;

/// Exhaustive product-facing failure identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandErrorCode {
    Validation,
    Conflict,
    NotFound,
    Permission,
    Budget,
    ResourceLimit,
    Cancelled,
    Paused,
    Stopped,
    Network,
    External,
    ExternalOutcomeUnknown,
    Io,
    Serialization,
    Ipc,
    LegacyRun,
    Internal,
    Unknown,
}

impl CommandErrorCode {
    pub const ALL: [Self; 18] = [
        Self::Validation,
        Self::Conflict,
        Self::NotFound,
        Self::Permission,
        Self::Budget,
        Self::ResourceLimit,
        Self::Cancelled,
        Self::Paused,
        Self::Stopped,
        Self::Network,
        Self::External,
        Self::ExternalOutcomeUnknown,
        Self::Io,
        Self::Serialization,
        Self::Ipc,
        Self::LegacyRun,
        Self::Internal,
        Self::Unknown,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Conflict => "conflict",
            Self::NotFound => "not_found",
            Self::Permission => "permission",
            Self::Budget => "budget",
            Self::ResourceLimit => "resource_limit",
            Self::Cancelled => "cancelled",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
            Self::Network => "network",
            Self::External => "external",
            Self::ExternalOutcomeUnknown => "external_outcome_unknown",
            Self::Io => "io",
            Self::Serialization => "serialization",
            Self::Ipc => "ipc",
            Self::LegacyRun => "legacy_run",
            Self::Internal => "internal",
            Self::Unknown => "unknown",
        }
    }

    pub fn message_key(self) -> String {
        format!("ui.error.{}", self.as_str())
    }
}

impl std::fmt::Display for CommandErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable failure identity carried on command and `ok:false` IPC responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandError {
    pub code: CommandErrorCode,
    pub message_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

impl CommandError {
    pub fn new(code: CommandErrorCode, diagnostic: impl Into<String>) -> Self {
        Self {
            code,
            message_key: code.message_key(),
            diagnostic: Some(diagnostic.into()),
            params: BTreeMap::new(),
        }
    }

    pub fn with_key(
        code: CommandErrorCode,
        message_key: impl Into<String>,
        diagnostic: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message_key: message_key.into(),
            diagnostic: Some(diagnostic.into()),
            params: BTreeMap::new(),
        }
    }

    pub fn validation(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Validation, diagnostic)
    }

    pub fn conflict(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Conflict, diagnostic)
    }

    pub fn not_found(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::NotFound, diagnostic)
    }

    pub fn permission(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Permission, diagnostic)
    }

    pub fn network(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Network, diagnostic)
    }

    pub fn external(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::External, diagnostic)
    }

    pub fn io(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Io, diagnostic)
    }

    pub fn serialization(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Serialization, diagnostic)
    }

    pub fn ipc(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Ipc, diagnostic)
    }

    pub fn legacy_run(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::LegacyRun, diagnostic)
    }

    pub fn internal(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Internal, diagnostic)
    }

    pub fn unknown(diagnostic: impl Into<String>) -> Self {
        Self::new(CommandErrorCode::Unknown, diagnostic)
    }

    pub fn from_core(error: &CoreError) -> Self {
        let code = match error {
            CoreError::Validation { .. }
            | CoreError::PortMissing { .. }
            | CoreError::PortTypeMismatch { .. } => CommandErrorCode::Validation,
            CoreError::RegistryDuplicate { .. }
            | CoreError::WorkflowStateRevisionConflict { .. }
            | CoreError::DocumentAlreadyExists { .. } => CommandErrorCode::Conflict,
            CoreError::RegistryMissing { .. } | CoreError::WorkflowRunNotFound { .. } => {
                CommandErrorCode::NotFound
            }
            CoreError::PermissionDenied { .. } => CommandErrorCode::Permission,
            CoreError::BudgetExceeded { .. } => CommandErrorCode::Budget,
            CoreError::ResourceLimitExceeded { .. } => CommandErrorCode::ResourceLimit,
            CoreError::External { .. }
            | CoreError::ProviderRequest { .. }
            | CoreError::ExternalOperation { .. } => CommandErrorCode::External,
            CoreError::ExternalCancellation { .. } | CoreError::Cancelled => {
                CommandErrorCode::Cancelled
            }
            CoreError::ExternalOutcomeUnknown { .. } => CommandErrorCode::ExternalOutcomeUnknown,
            CoreError::Paused { .. } => CommandErrorCode::Paused,
            CoreError::Stopped { .. } => CommandErrorCode::Stopped,
            CoreError::WorkflowExecutorContractViolation { .. } => CommandErrorCode::Internal,
            CoreError::Io(_) => CommandErrorCode::Io,
            CoreError::Json(_) | CoreError::Yaml(_) => CommandErrorCode::Serialization,
        };
        Self::new(code, error.to_string())
    }

    pub fn contains(&self, pattern: &str) -> bool {
        self.diagnostic
            .as_deref()
            .is_some_and(|diagnostic| diagnostic.contains(pattern))
    }

    pub fn diagnostic_text(&self) -> &str {
        self.diagnostic
            .as_deref()
            .unwrap_or_else(|| self.code.as_str())
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.diagnostic {
            Some(diagnostic) => f.write_str(diagnostic),
            None => self.code.fmt(f),
        }
    }
}

impl std::error::Error for CommandError {}

impl std::ops::Deref for CommandError {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.diagnostic_text()
    }
}

impl From<CoreError> for CommandError {
    fn from(value: CoreError) -> Self {
        Self::from_core(&value)
    }
}

impl From<std::io::Error> for CommandError {
    fn from(value: std::io::Error) -> Self {
        Self::io(value.to_string())
    }
}

impl From<serde_json::Error> for CommandError {
    fn from(value: serde_json::Error) -> Self {
        Self::serialization(value.to_string())
    }
}

impl From<yaml_serde::Error> for CommandError {
    fn from(value: yaml_serde::Error) -> Self {
        Self::serialization(value.to_string())
    }
}

impl From<reqwest::Error> for CommandError {
    fn from(value: reqwest::Error) -> Self {
        if value.is_connect() || value.is_timeout() || value.is_request() {
            Self::network(value.to_string())
        } else {
            Self::external(value.to_string())
        }
    }
}

impl From<std::path::StripPrefixError> for CommandError {
    fn from(value: std::path::StripPrefixError) -> Self {
        Self::permission(value.to_string())
    }
}

impl From<std::time::SystemTimeError> for CommandError {
    fn from(value: std::time::SystemTimeError) -> Self {
        Self::internal(value.to_string())
    }
}

impl From<CommandError> for String {
    fn from(value: CommandError) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_mapping_is_variant_driven() {
        assert_eq!(
            CommandError::from(CoreError::PermissionDenied {
                action: "network".to_owned(),
                reason: "policy".to_owned(),
            })
            .code,
            CommandErrorCode::Permission
        );
        assert_eq!(
            CommandError::from(CoreError::WorkflowRunNotFound {
                workflow_id: "wf".to_owned(),
                run_id: "run".to_owned(),
            })
            .code,
            CommandErrorCode::NotFound
        );
    }

    #[test]
    fn explicit_error_conversions_are_variant_driven() {
        let yaml_error = yaml_serde::from_str::<serde_json::Value>("[unterminated")
            .expect_err("invalid YAML should fail");
        assert_eq!(
            CommandError::from(yaml_error).code,
            CommandErrorCode::Serialization
        );

        let strip_error = std::path::Path::new("outside")
            .strip_prefix("project")
            .expect_err("unrelated paths should fail");
        assert_eq!(
            CommandError::from(strip_error).code,
            CommandErrorCode::Permission
        );
    }
}
