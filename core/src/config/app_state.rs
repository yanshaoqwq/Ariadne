use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use crate::contracts::{CoreError, CoreResult};

pub const APP_STATE_ENV: &str = "ARIADNE_APP_STATE_ROOT";

static FALLBACK_APP_STATE_ROOT: OnceLock<PathBuf> = OnceLock::new();
static PROJECT_APP_STATE_ROOTS: OnceLock<RwLock<BTreeMap<PathBuf, PathBuf>>> = OnceLock::new();

/// 返回项目树之外的默认应用状态目录。
pub fn default_app_state_root() -> PathBuf {
    if let Some(path) = std::env::var_os(APP_STATE_ENV) {
        return PathBuf::from(path);
    }

    platform_app_state_root()
}

fn platform_app_state_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(path) = std::env::var_os("APPDATA") {
        return PathBuf::from(path).join("Ariadne");
    }

    #[cfg(target_os = "macos")]
    if let Some(path) = std::env::var_os("HOME") {
        return PathBuf::from(path)
            .join("Library")
            .join("Application Support")
            .join("Ariadne");
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(path).join("Ariadne");
        }
        if let Some(path) = std::env::var_os("HOME") {
            return PathBuf::from(path)
                .join(".local")
                .join("share")
                .join("Ariadne");
        }
    }

    std::env::temp_dir().join("Ariadne-app-state")
}

/// 将一个项目绑定到进程内可信 app-state authority。
///
/// 同一 canonical project 只能绑定一个 authority，避免桌面、CLI 和后台 worker
/// 分别恢复不同 journal。绑定表仅驻留进程内，不读取项目文件。
pub fn bind_project_app_state(project_root: &Path, app_state_root: &Path) -> CoreResult<()> {
    if project_root.as_os_str().is_empty() {
        return Ok(());
    }
    let project = stable_absolute(project_root)?;
    let app_state = stable_absolute(app_state_root)?;
    if app_state.starts_with(&project) {
        return Err(CoreError::validation(
            "trusted app state must be outside the project tree",
        ));
    }

    let registry = PROJECT_APP_STATE_ROOTS.get_or_init(|| RwLock::new(BTreeMap::new()));
    let mut registry = registry
        .write()
        .map_err(|_| CoreError::validation("project app-state registry lock poisoned"))?;
    if let Some(existing) = registry.get(&project) {
        if existing != &app_state {
            return Err(CoreError::validation(format!(
                "project is already bound to a different trusted app-state authority: {}",
                existing.display()
            )));
        }
        return Ok(());
    }
    registry.insert(project, app_state);
    Ok(())
}

/// 返回已绑定 authority；没有显式 AriadneAppState 时使用进程启动默认值。
pub fn trusted_app_state_for_project(project_root: &Path) -> PathBuf {
    let key = stable_absolute(project_root).ok();
    if let (Some(registry), Some(key)) = (PROJECT_APP_STATE_ROOTS.get(), key) {
        if let Ok(registry) = registry.read() {
            if let Some(root) = registry.get(&key) {
                return root.clone();
            }
        }
    }
    FALLBACK_APP_STATE_ROOT
        .get_or_init(unbound_process_app_state_root)
        .clone()
}

fn unbound_process_app_state_root() -> PathBuf {
    let mut random = [0u8; 16];
    let suffix = if getrandom::getrandom(&mut random).is_ok() {
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    } else {
        format!("pid-{}", std::process::id())
    };
    std::env::temp_dir().join(format!("Ariadne-unbound-{suffix}"))
}

fn stable_absolute(path: &Path) -> CoreResult<PathBuf> {
    if path.exists() {
        return path.canonicalize().map_err(CoreError::from);
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}
