use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contracts::CoreResult;

use super::CONFIG_DIR_NAME;

pub const CONFIRMATION_POLICY_SETTINGS_FILE: &str = "confirmation_policy_settings.json";

/// 设置页与执行层共用的单项确认策略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfirmationPolicySetting {
    pub confirmation_kind: String,
    #[serde(default)]
    pub normal_policy: ConfirmationNormalPolicy,
    #[serde(default)]
    pub auto_mode_policy: ConfirmationAutoModePolicy,
}

impl<'de> Deserialize<'de> for ConfirmationPolicySetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawConfirmationPolicySetting {
            confirmation_kind: String,
            #[serde(default)]
            normal_policy: ConfirmationNormalPolicy,
            #[serde(default)]
            auto_mode_policy: ConfirmationAutoModePolicy,
            #[serde(default, rename = "policy")]
            policy_code: String,
        }

        let raw = RawConfirmationPolicySetting::deserialize(deserializer)?;
        let (normal_policy, auto_mode_policy) = if raw.policy_code.trim().is_empty() {
            (raw.normal_policy, raw.auto_mode_policy)
        } else {
            policies_from_policy_code(&raw.policy_code)
        };

        Ok(Self {
            confirmation_kind: raw.confirmation_kind,
            normal_policy,
            auto_mode_policy,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationNormalPolicy {
    #[default]
    ManualReview,
    AllowByDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationAutoModePolicy {
    #[default]
    AllowByDefault,
    AutoApproval,
}

pub fn confirmation_policy_settings_path(project_root: &Path) -> PathBuf {
    project_root
        .join(CONFIG_DIR_NAME)
        .join(CONFIRMATION_POLICY_SETTINGS_FILE)
}

/// 读取项目保存的确认策略。文件缺失表示使用领域默认值；其它错误必须 fail-loud。
pub fn read_confirmation_policy_settings(
    project_root: &Path,
) -> CoreResult<Option<Vec<ConfirmationPolicySetting>>> {
    let path = confirmation_policy_settings_path(project_root);
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(serde_json::from_str(&content)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// 兼容旧单字段 policy code，并统一供设置页迁移与执行层解析使用。
pub fn policies_from_policy_code(
    policy: &str,
) -> (ConfirmationNormalPolicy, ConfirmationAutoModePolicy) {
    match policy {
        "auto_skip" => (
            ConfirmationNormalPolicy::AllowByDefault,
            ConfirmationAutoModePolicy::AllowByDefault,
        ),
        "auto_audit" => (
            ConfirmationNormalPolicy::ManualReview,
            ConfirmationAutoModePolicy::AutoApproval,
        ),
        "manual_skip" | "manual" => (
            ConfirmationNormalPolicy::ManualReview,
            ConfirmationAutoModePolicy::AllowByDefault,
        ),
        "auto_approve" => (
            ConfirmationNormalPolicy::AllowByDefault,
            ConfirmationAutoModePolicy::AutoApproval,
        ),
        _ => (
            ConfirmationNormalPolicy::ManualReview,
            ConfirmationAutoModePolicy::AllowByDefault,
        ),
    }
}

pub fn policy_code_from_dual_policy(
    normal_policy: ConfirmationNormalPolicy,
    auto_mode_policy: ConfirmationAutoModePolicy,
) -> String {
    match (normal_policy, auto_mode_policy) {
        (ConfirmationNormalPolicy::AllowByDefault, ConfirmationAutoModePolicy::AllowByDefault) => {
            "auto_skip".to_owned()
        }
        (ConfirmationNormalPolicy::ManualReview, ConfirmationAutoModePolicy::AutoApproval) => {
            "auto_audit".to_owned()
        }
        (ConfirmationNormalPolicy::ManualReview, ConfirmationAutoModePolicy::AllowByDefault) => {
            "manual_skip".to_owned()
        }
        (ConfirmationNormalPolicy::AllowByDefault, ConfirmationAutoModePolicy::AutoApproval) => {
            "auto_approve".to_owned()
        }
    }
}
