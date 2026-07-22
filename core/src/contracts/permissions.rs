use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contracts::errors::{CoreError, CoreResult};

/// Auto Mode 的运行态配置，只影响普通人工确认，不影响硬权限。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoModeState {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preauthorized_budget_usd: Option<f64>,
}

impl Default for AutoModeState {
    /// 默认关闭 Auto Mode，且不预授权预算。
    fn default() -> Self {
        Self {
            enabled: false,
            preauthorized_budget_usd: None,
        }
    }
}

/// 单个节点的审批策略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    pub allow_auto_approval: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_prompt_id: Option<String>,
    pub require_human_on_conflict: bool,
}

impl Default for ApprovalPolicy {
    /// 默认需要人工审批，冲突必须人工处理。
    fn default() -> Self {
        Self {
            allow_auto_approval: false,
            approval_prompt_id: None,
            require_human_on_conflict: true,
        }
    }
}

impl ApprovalPolicy {
    /// 判断在当前 Auto Mode 状态下是否允许跳过人工审批。
    pub fn should_auto_approve(&self, auto_mode: &AutoModeState, has_conflict: bool) -> bool {
        auto_mode.enabled
            && self.allow_auto_approval
            && !(has_conflict && self.require_human_on_conflict)
    }
}

/// 执行时的硬权限策略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionPolicy {
    pub allow_network: bool,
    pub allow_web_search: bool,
    pub allow_http_skill: bool,
    pub allow_wasm_network: bool,
    pub allow_secret_read: bool,
    #[serde(default)]
    pub writable_file_roots: Vec<PathBuf>,
    #[serde(default)]
    pub readable_file_roots: Vec<PathBuf>,
}

impl Default for PermissionPolicy {
    /// 默认拒绝所有敏感能力，由配置显式开放。
    fn default() -> Self {
        Self {
            allow_network: false,
            allow_web_search: false,
            allow_http_skill: false,
            allow_wasm_network: false,
            allow_secret_read: false,
            writable_file_roots: Vec::new(),
            readable_file_roots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionRequest {
    /// 普通网络访问。
    Network { host: String },
    /// Web 搜索能力。
    WebSearch,
    /// HTTP Skill 网络访问。
    HttpSkill { host: String },
    /// WASM 内部网络访问。
    WasmNetwork { host: String },
    /// 文件读取。
    FileRead { path: PathBuf },
    /// 文件写入。
    FileWrite { path: PathBuf },
    /// 直接读取 secret。
    SecretRead { key_id: String },
}

/// 权限判断结果，保留拒绝原因用于 UI 和日志。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PermissionDecision {
    /// 返回允许结果。
    pub fn allow() -> Self {
        Self {
            allowed: true,
            reason: None,
        }
    }

    /// 返回拒绝结果并记录原因。
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: Some(reason.into()),
        }
    }
}

impl PermissionPolicy {
    /// 校验权限层级关系和文件根目录，防止持久化永远不生效的配置。
    pub fn validate(&self) -> CoreResult<()> {
        if !self.allow_network
            && (self.allow_web_search || self.allow_http_skill || self.allow_wasm_network)
        {
            return Err(CoreError::validation(
                "network child permissions require allow_network",
            ));
        }

        for roots in [&self.readable_file_roots, &self.writable_file_roots] {
            let mut unique = std::collections::BTreeSet::new();
            for root in roots {
                if root
                    .components()
                    .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
                {
                    return Err(CoreError::validation(format!(
                        "permission file root cannot contain dot components: {}",
                        root.display()
                    )));
                }
                let normalized = normalize_absolute_path(root).ok_or_else(|| {
                    CoreError::validation(format!(
                        "permission file root must be absolute: {}",
                        root.display()
                    ))
                })?;
                if !unique.insert(normalized.clone()) {
                    return Err(CoreError::validation(format!(
                        "duplicate permission file root: {}",
                        normalized.display()
                    )));
                }
            }
        }
        Ok(())
    }

    /// 根据权限请求计算允许或拒绝结果。
    pub fn evaluate(&self, request: &PermissionRequest) -> PermissionDecision {
        match request {
            PermissionRequest::Network { .. } => {
                if self.allow_network {
                    PermissionDecision::allow()
                } else {
                    PermissionDecision::deny("network access is disabled")
                }
            }
            PermissionRequest::WebSearch => {
                if self.allow_network && self.allow_web_search {
                    PermissionDecision::allow()
                } else {
                    PermissionDecision::deny("web search is disabled")
                }
            }
            PermissionRequest::HttpSkill { .. } => {
                if self.allow_network && self.allow_http_skill {
                    PermissionDecision::allow()
                } else {
                    PermissionDecision::deny("http skill access is disabled")
                }
            }
            PermissionRequest::WasmNetwork { .. } => {
                if self.allow_network && self.allow_wasm_network {
                    PermissionDecision::allow()
                } else {
                    PermissionDecision::deny("wasm network access is disabled")
                }
            }
            PermissionRequest::FileRead { path } => {
                if is_under_any_root(path, &self.readable_file_roots)
                    || is_under_any_root(path, &self.writable_file_roots)
                {
                    PermissionDecision::allow()
                } else {
                    PermissionDecision::deny("file read path is outside allowed roots")
                }
            }
            PermissionRequest::FileWrite { path } => {
                if is_under_any_root(path, &self.writable_file_roots) {
                    PermissionDecision::allow()
                } else {
                    PermissionDecision::deny("file write path is outside writable roots")
                }
            }
            PermissionRequest::SecretRead { .. } => {
                if self.allow_secret_read {
                    PermissionDecision::allow()
                } else {
                    PermissionDecision::deny("direct secret reads are disabled")
                }
            }
        }
    }

    /// 权限拒绝时转成统一 CoreError。
    pub fn ensure(&self, request: &PermissionRequest) -> CoreResult<()> {
        let decision = self.evaluate(request);
        if decision.allowed {
            return Ok(());
        }

        Err(CoreError::PermissionDenied {
            action: permission_action(request),
            reason: decision.reason.unwrap_or_else(|| "denied".to_owned()),
        })
    }
}

/// 确认目标路径位于指定根目录内，且已存在前缀不能通过符号链接逃逸。
pub fn ensure_path_under_root(root: &Path, path: &Path) -> CoreResult<()> {
    let normalized_path = normalize_absolute_path(path).ok_or_else(|| {
        CoreError::validation(format!("path must be absolute: {}", path.display()))
    })?;
    let normalized_root = normalize_absolute_path(root).ok_or_else(|| {
        CoreError::validation(format!("root must be absolute: {}", root.display()))
    })?;

    let canonicalish_path = canonicalize_existing_prefix(&normalized_path).ok_or_else(|| {
        CoreError::validation(format!("path cannot be resolved: {}", path.display()))
    })?;
    let canonicalish_root = canonicalize_existing_prefix(&normalized_root).ok_or_else(|| {
        CoreError::validation(format!("root cannot be resolved: {}", root.display()))
    })?;

    if canonicalish_path.starts_with(&canonicalish_root) {
        return Ok(());
    }

    Err(CoreError::PermissionDenied {
        action: format!("path:{}", path.display()),
        reason: format!(
            "path is outside allowed root {} after resolving existing prefixes",
            root.display()
        ),
    })
}

/// 判断 path 是否在任一允许根目录下，包含父目录和符号链接逃逸防护。
fn is_under_any_root(path: &Path, roots: &[PathBuf]) -> bool {
    let Some(normalized_path) = normalize_absolute_path(path) else {
        return false;
    };

    let Some(canonicalish_path) = canonicalize_existing_prefix(&normalized_path) else {
        return false;
    };

    roots.iter().any(|root| {
        let Some(normalized_root) = normalize_absolute_path(root) else {
            return false;
        };

        let Some(canonicalish_root) = canonicalize_existing_prefix(&normalized_root) else {
            return false;
        };

        canonicalish_path.starts_with(canonicalish_root)
    })
}

/// 纯词法规范化绝对路径，先消解 `.` 和 `..` 防止父目录逃逸。
fn normalize_absolute_path(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                // 词法层面先处理 `..`，阻止 `/allowed/../secret` 这类逃逸。
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    Some(normalized)
}

/// canonicalize 已存在的前缀，允许目标文件本身尚未创建。
fn canonicalize_existing_prefix(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Some(canonical);
    }

    let mut missing_suffix = Vec::new();
    let mut current = path;
    loop {
        if let Ok(canonical_parent) = current.canonicalize() {
            let mut combined = canonical_parent;
            // 目标文件可以尚未存在，所以只 canonicalize 已存在前缀，再拼回缺失后缀。
            for component in missing_suffix.iter().rev() {
                combined.push(component);
            }
            return Some(combined);
        }

        let file_name = current.file_name()?.to_owned();
        missing_suffix.push(file_name);
        current = current.parent()?;
    }
}

/// 为权限错误生成稳定的动作描述。
fn permission_action(request: &PermissionRequest) -> String {
    match request {
        PermissionRequest::Network { host } => format!("network:{host}"),
        PermissionRequest::WebSearch => "web_search".to_owned(),
        PermissionRequest::HttpSkill { host } => format!("http_skill:{host}"),
        PermissionRequest::WasmNetwork { host } => format!("wasm_network:{host}"),
        PermissionRequest::FileRead { path } => format!("file_read:{}", path.display()),
        PermissionRequest::FileWrite { path } => format!("file_write:{}", path.display()),
        PermissionRequest::SecretRead { key_id } => format!("secret_read:{key_id}"),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub auto_mode: AutoModeState,
    pub permissions: PermissionPolicy,
}

impl ExecutionPolicy {
    /// 判断当前执行策略是否跳过普通人工确认。
    pub fn should_skip_human_confirmation(
        &self,
        approval_policy: &ApprovalPolicy,
        has_conflict: bool,
    ) -> bool {
        approval_policy.should_auto_approve(&self.auto_mode, has_conflict)
    }

    /// 执行硬权限检查。
    pub fn ensure_permission(&self, request: &PermissionRequest) -> CoreResult<()> {
        self.permissions.ensure(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_mode_does_not_bypass_network_permission() {
        let policy = ExecutionPolicy {
            auto_mode: AutoModeState {
                enabled: true,
                preauthorized_budget_usd: Some(10.0),
            },
            permissions: PermissionPolicy::default(),
        };

        assert!(policy
            .ensure_permission(&PermissionRequest::Network {
                host: "example.com".to_owned()
            })
            .is_err());
    }

    #[test]
    fn approval_policy_requires_node_level_auto_approval() {
        let auto_mode = AutoModeState {
            enabled: true,
            preauthorized_budget_usd: None,
        };

        assert!(!ApprovalPolicy::default().should_auto_approve(&auto_mode, false));
    }

    #[test]
    fn file_permission_rejects_parent_directory_escape() {
        let temp_dir = tempfile::tempdir().unwrap();
        let allowed = temp_dir.path().join("documents");
        std::fs::create_dir_all(&allowed).unwrap();
        let escaped = allowed.join("../secrets.txt");
        let policy = PermissionPolicy {
            readable_file_roots: vec![allowed],
            ..PermissionPolicy::default()
        };

        assert!(policy
            .ensure(&PermissionRequest::FileRead { path: escaped })
            .is_err());
    }

    #[test]
    #[cfg(unix)]
    fn file_permission_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let allowed = temp_dir.path().join("documents");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let link = allowed.join("link");
        symlink(&outside, &link).unwrap();

        let policy = PermissionPolicy {
            readable_file_roots: vec![allowed],
            ..PermissionPolicy::default()
        };

        assert!(policy
            .ensure(&PermissionRequest::FileRead {
                path: link.join("secret.txt")
            })
            .is_err());
    }
}
