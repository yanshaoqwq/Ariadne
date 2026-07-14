//! Structured command/IPC errors (U1).
//!
//! Author-facing localization uses `code` / `message_key`.
//! Free-form text is diagnostic only. Legacy string errors are adapted **only**
//! in [`from_legacy_message`] at the IPC boundary — not re-implemented on the desktop.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable failure identity carried on `ok:false` IPC responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandError {
    pub code: String,
    pub message_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

impl CommandError {
    pub fn new(code: impl Into<String>, diagnostic: impl Into<String>) -> Self {
        let code = code.into();
        let message_key = format!("ui.error.{code}");
        Self {
            code,
            message_key,
            diagnostic: Some(diagnostic.into()),
            params: BTreeMap::new(),
        }
    }

    pub fn with_key(
        code: impl Into<String>,
        message_key: impl Into<String>,
        diagnostic: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message_key: message_key.into(),
            diagnostic: Some(diagnostic.into()),
            params: BTreeMap::new(),
        }
    }

    pub fn conflict(diagnostic: impl Into<String>) -> Self {
        Self::new("conflict", diagnostic)
    }

    pub fn validation(diagnostic: impl Into<String>) -> Self {
        Self::new("validation", diagnostic)
    }

    pub fn not_found(diagnostic: impl Into<String>) -> Self {
        Self::new("not_found", diagnostic)
    }

    pub fn io(diagnostic: impl Into<String>) -> Self {
        Self::new("io", diagnostic)
    }

    /// Legacy protocol adapter (v1): map free-form command `String` errors to a code.
    /// Desktop must not re-implement this table — only consume `error_code` from IPC.
    pub fn from_legacy_message(message: impl Into<String>) -> Self {
        let diagnostic = message.into();
        let code = classify_legacy_message(&diagnostic).to_owned();
        Self::new(code, diagnostic)
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.diagnostic {
            Some(d) => write!(f, "{d}"),
            None => write!(f, "{}", self.code),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<String> for CommandError {
    fn from(value: String) -> Self {
        Self::from_legacy_message(value)
    }
}

impl From<&str> for CommandError {
    fn from(value: &str) -> Self {
        Self::from_legacy_message(value)
    }
}

/// Single legacy classifier — used only when converting historical `String` errors.
pub fn classify_legacy_message(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    if contains_any(
        &m,
        &[
            "permission denied",
            "access is denied",
            "unauthorized",
            "forbidden",
            "eacces",
        ],
    ) {
        return "permission";
    }
    if contains_any(
        &m,
        &[
            "not found",
            "no such file",
            "does not exist",
            "missing required",
            "404",
        ],
    ) {
        return "not_found";
    }
    if contains_any(
        &m,
        &[
            "revision conflict",
            "content revision",
            "expected_revision",
            "already exists",
            "conflict",
        ],
    ) {
        return "conflict";
    }
    if contains_any(
        &m,
        &[
            "validation failed",
            "validation",
            "invalid",
            "must be",
            "parse error",
            "cannot be empty",
            "schema",
        ],
    ) {
        return "validation";
    }
    if contains_any(&m, &["budget exceeded", "preauthorized"])
        || (m.contains("budget") && (m.contains("exceed") || m.contains("limit")))
    {
        return "budget";
    }
    if contains_any(&m, &["cancelled", "canceled", "operation cancelled"]) {
        return "cancelled";
    }
    if contains_any(
        &m,
        &[
            "connection refused",
            "actively refused",
            "timed out",
            "timeout",
            "could not connect",
            "broken pipe",
            "connection reset",
            "network unreachable",
        ],
    ) {
        return "network";
    }
    if contains_any(&m, &["external service"]) || (m.contains("http ") && m.contains("error")) {
        return "external";
    }
    if contains_any(&m, &["io error", "i/o", "disk", "filesystem", "os error"]) {
        return "io";
    }
    if contains_any(
        &m,
        &[
            "backend ipc",
            "ipc command",
            "ipc process",
            "ipc returned",
            "not connected",
        ],
    ) {
        return "ipc";
    }
    if m.contains("legacy") && m.contains("prepared_workflow") {
        return "legacy_run";
    }
    "unknown"
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_permission_before_network_word() {
        assert_eq!(
            classify_legacy_message("permission denied for tool: network"),
            "permission"
        );
    }

    #[test]
    fn revision_conflict_maps_conflict() {
        assert_eq!(
            classify_legacy_message("workflow content revision conflict: expected a actual b"),
            "conflict"
        );
    }
}
