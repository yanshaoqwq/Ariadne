use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use sha2::{Digest, Sha256};

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

/// 为项目建立位于可信 app-state 内的隔离 authority 目录。
///
/// 项目目录是不可信输入：任何会驱动恢复或继续写入的 journal/operation 均应放在此处，
/// 并按 canonical project 的无损平台路径身份隔离，不能从项目文件推导可执行授权。
pub fn project_authority_dir(
    project_root: &Path,
    app_state_root: &Path,
    namespace: &str,
) -> CoreResult<PathBuf> {
    project_authority_dir_with_identity(
        project_root,
        app_state_root,
        namespace,
        &canonical_project_identity(&project_root.canonicalize()?),
    )
}

pub(crate) fn project_authority_dir_with_identity(
    project_root: &Path,
    app_state_root: &Path,
    namespace: &str,
    project_identity: &str,
) -> CoreResult<PathBuf> {
    if namespace.is_empty()
        || !namespace
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(CoreError::validation(
            "app-state authority namespace must contain only ASCII letters, digits or '-'",
        ));
    }
    let project = project_root.canonicalize()?;
    if !project.is_dir() {
        return Err(CoreError::validation("project root must be a directory"));
    }
    let app_state = stable_absolute(app_state_root)?;
    if app_state.starts_with(&project) {
        return Err(CoreError::validation(
            "trusted app state must be outside the project tree",
        ));
    }
    fs::create_dir_all(&app_state)?;
    let app_state = app_state.canonicalize()?;
    if app_state.starts_with(&project) {
        return Err(CoreError::validation(
            "trusted app state resolves inside the project tree",
        ));
    }
    let namespace_dir = app_state.join(namespace);
    create_real_directory(&namespace_dir, "app-state authority namespace")?;
    if project_identity.len() != 64
        || !project_identity
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(CoreError::validation(
            "project authority identity must be a SHA-256 hex digest",
        ));
    }
    let authority = namespace_dir.join(project_identity);
    create_real_directory(&authority, "project authority")?;
    let authority = authority.canonicalize()?;
    if !authority.starts_with(&app_state) || authority.starts_with(&project) {
        return Err(CoreError::validation(
            "project authority escaped trusted app state",
        ));
    }
    let metadata = fs::symlink_metadata(&authority)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::validation(
            "project authority must be a real directory",
        ));
    }
    Ok(authority)
}

pub fn canonical_project_identity(project_root: &Path) -> String {
    let mut hasher = Sha256::new();
    // 与 atomic_commit v3 已发布 identity 完全兼容，避免升级后遗失待恢复 journal。
    hasher.update(b"ariadne-project-atomic-authority-v1\0");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(project_root.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in project_root.as_os_str().encode_wide() {
            hasher.update(unit.to_le_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

fn create_real_directory(path: &Path, label: &str) -> CoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(CoreError::validation(format!(
                "{label} must be a real directory"
            )));
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::validation(format!(
            "{label} must be a real directory"
        )));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    #[test]
    fn concurrent_project_authority_creation_is_idempotent() {
        let project = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();

        for _ in 0..8 {
            let barrier = Arc::clone(&barrier);
            let project = project.path().to_path_buf();
            let app_state = app_state.path().to_path_buf();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                project_authority_dir(&project, &app_state, "concurrent-authority")
            }));
        }

        let authorities = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert!(authorities.iter().all(|path| path == &authorities[0]));
        assert!(authorities[0].is_dir());
        assert!(!authorities[0].starts_with(project.path()));
    }
}
