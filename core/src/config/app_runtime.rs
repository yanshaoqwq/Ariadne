use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{RagConfig, SidecarConfig};
use crate::contracts::{CoreError, CoreResult};

pub const APP_RUNTIME_SETTINGS_FILE: &str = "app_runtime_settings.json";
const APP_RUNTIME_SETTINGS_LOCK_FILE: &str = ".app-runtime-settings.lock";
const APP_RUNTIME_SETTINGS_TRANSACTION_LOCK_FILE: &str = ".app-runtime-settings-transaction.lock";

/// 设备/应用级运行环境配置。项目索引身份、数据目录和远端端点不进入这里。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRuntimeSettings {
    #[serde(default = "default_qdrant_binary_path")]
    pub qdrant_binary_path: String,
    #[serde(default = "default_qdrant_startup_timeout_ms")]
    pub qdrant_startup_timeout_ms: u64,
}

impl Default for AppRuntimeSettings {
    fn default() -> Self {
        let sidecar = SidecarConfig::default();
        Self {
            qdrant_binary_path: sidecar.binary_path,
            qdrant_startup_timeout_ms: sidecar.startup_timeout_ms,
        }
    }
}

impl AppRuntimeSettings {
    pub fn validate(&self) -> CoreResult<()> {
        if self.qdrant_binary_path.trim().is_empty() {
            return Err(CoreError::validation("qdrant binary path cannot be empty"));
        }
        if self.qdrant_startup_timeout_ms == 0 {
            return Err(CoreError::validation(
                "qdrant startup timeout must be positive",
            ));
        }
        Ok(())
    }

    pub fn apply_to_sidecar(&self, sidecar: &mut SidecarConfig) {
        sidecar.binary_path = self.qdrant_binary_path.clone();
        sidecar.startup_timeout_ms = self.qdrant_startup_timeout_ms;
    }
}

#[derive(Debug, Clone)]
pub struct AppRuntimeSettingsStore {
    app_state_root: PathBuf,
    path: PathBuf,
}

impl AppRuntimeSettingsStore {
    pub fn default_for_app(app_state_root: impl AsRef<Path>) -> Self {
        let app_state_root = app_state_root.as_ref().to_path_buf();
        Self {
            path: app_state_root.join(APP_RUNTIME_SETTINGS_FILE),
            app_state_root,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self) -> CoreResult<AppRuntimeSettings> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => {
                let settings: AppRuntimeSettings = serde_json::from_str(&content)?;
                settings.validate()?;
                Ok(settings)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(AppRuntimeSettings::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// 全局文件首次建立时，从当前项目旧版 rag.yaml 迁移二进制/超时一次。
    pub fn read_global_or_migrate(
        app_state_root: impl AsRef<Path>,
        project_root: Option<&Path>,
    ) -> CoreResult<AppRuntimeSettings> {
        let store = Self::default_for_app(app_state_root);
        let lock = store.lock_exclusive()?;
        let result = if store.path.is_file() {
            store.read()
        } else {
            let settings = project_root
                .map(read_legacy_project_runtime_settings)
                .transpose()?
                .flatten()
                .unwrap_or_default();
            settings.validate()?;
            store.write_unlocked(&settings)?;
            Ok(settings)
        };
        drop(lock);
        result
    }

    pub fn write(&self, settings: &AppRuntimeSettings) -> CoreResult<()> {
        let lock = self.lock_exclusive()?;
        let result = self.write_unlocked(settings);
        drop(lock);
        result
    }

    fn write_unlocked(&self, settings: &AppRuntimeSettings) -> CoreResult<()> {
        settings.validate()?;
        let body = serde_json::to_vec_pretty(settings)?;
        crate::config::store::atomic_write(&self.path, &body)
    }

    fn lock_exclusive(&self) -> CoreResult<std::fs::File> {
        crate::config::store::acquire_app_state_lock(
            &self.app_state_root,
            APP_RUNTIME_SETTINGS_LOCK_FILE,
            "app_runtime_settings_lock",
        )
    }

    /// 串行化跨进程的 write + runtime reload + rollback 完整事务。
    pub(crate) fn lock_transaction_exclusive(&self) -> CoreResult<std::fs::File> {
        crate::config::store::acquire_app_state_lock(
            &self.app_state_root,
            APP_RUNTIME_SETTINGS_TRANSACTION_LOCK_FILE,
            "app_runtime_settings_transaction_lock",
        )
    }
}

fn read_legacy_project_runtime_settings(
    project_root: &Path,
) -> CoreResult<Option<AppRuntimeSettings>> {
    let path = project_root
        .join(".config")
        .join(crate::config::RAG_CONFIG_FILE);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let rag: RagConfig = yaml_serde::from_str(&raw)?;
    let settings = AppRuntimeSettings {
        qdrant_binary_path: rag.vector_store.sidecar.binary_path,
        qdrant_startup_timeout_ms: rag.vector_store.sidecar.startup_timeout_ms,
    };
    settings.validate()?;
    Ok(Some(settings))
}

fn default_qdrant_binary_path() -> String {
    SidecarConfig::default().binary_path
}

fn default_qdrant_startup_timeout_ms() -> u64 {
    SidecarConfig::default().startup_timeout_ms
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn migrates_legacy_project_runtime_fields_only_once() {
        let project = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".config")).unwrap();
        std::fs::write(
            project.path().join(".config").join(crate::config::RAG_CONFIG_FILE),
            "vector_store:\n  sidecar:\n    binary_path: /opt/qdrant-a\n    startup_timeout_ms: 42000\n",
        )
        .unwrap();

        let migrated =
            AppRuntimeSettingsStore::read_global_or_migrate(app_state.path(), Some(project.path()))
                .unwrap();
        assert_eq!(migrated.qdrant_binary_path, "/opt/qdrant-a");
        assert_eq!(migrated.qdrant_startup_timeout_ms, 42_000);

        std::fs::write(
            project.path().join(".config").join(crate::config::RAG_CONFIG_FILE),
            "vector_store:\n  sidecar:\n    binary_path: /opt/qdrant-b\n    startup_timeout_ms: 99000\n",
        )
        .unwrap();
        let reread =
            AppRuntimeSettingsStore::read_global_or_migrate(app_state.path(), Some(project.path()))
                .unwrap();
        assert_eq!(reread, migrated);
    }

    #[test]
    fn malformed_legacy_runtime_settings_do_not_create_global_defaults() {
        let project = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        fs::create_dir_all(project.path().join(".config")).unwrap();
        fs::write(
            project
                .path()
                .join(".config")
                .join(crate::config::RAG_CONFIG_FILE),
            "vector_store: [not-valid",
        )
        .unwrap();

        AppRuntimeSettingsStore::read_global_or_migrate(app_state.path(), Some(project.path()))
            .unwrap_err();

        assert!(!app_state.path().join(APP_RUNTIME_SETTINGS_FILE).exists());
    }

    #[test]
    fn global_runtime_migration_waits_for_domain_lock() {
        let project = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        fs::create_dir_all(project.path().join(".config")).unwrap();
        fs::write(
            project.path().join(".config").join(crate::config::RAG_CONFIG_FILE),
            "vector_store:\n  sidecar:\n    binary_path: /opt/qdrant-locked\n    startup_timeout_ms: 42000\n",
        )
        .unwrap();

        let store = AppRuntimeSettingsStore::default_for_app(app_state.path());
        let lock = store.lock_exclusive().unwrap();
        let app_state_root = app_state.path().to_path_buf();
        let project_root = project.path().to_path_buf();
        let (started_sender, started_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_sender.send(()).unwrap();
            result_sender
                .send(AppRuntimeSettingsStore::read_global_or_migrate(
                    app_state_root,
                    Some(&project_root),
                ))
                .unwrap();
        });

        started_receiver.recv().unwrap();
        assert!(result_receiver
            .recv_timeout(Duration::from_millis(150))
            .is_err());
        drop(lock);
        let migrated = result_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        worker.join().unwrap();
        assert_eq!(migrated.qdrant_binary_path, "/opt/qdrant-locked");
    }
}
