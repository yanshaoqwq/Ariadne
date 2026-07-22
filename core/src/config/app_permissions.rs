use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{PermissionsConfig, CONFIG_DIR_NAME, PERMISSIONS_CONFIG_FILE};
use crate::contracts::CoreResult;

pub const APP_PERMISSIONS_SETTINGS_FILE: &str = "permissions_settings.json";
const APP_PERMISSIONS_LOCK_FILE: &str = ".permissions-settings.lock";

/// 应用级硬权限与工具默认。项目文件只能作为旧版迁移输入，不能扩大或覆盖这里的边界。
#[derive(Debug, Clone)]
pub struct AppPermissionsStore {
    app_state_root: PathBuf,
    path: PathBuf,
}

impl AppPermissionsStore {
    pub fn default_for_app(app_state_root: impl AsRef<Path>) -> Self {
        let app_state_root = app_state_root.as_ref().to_path_buf();
        Self {
            path: app_state_root.join(APP_PERMISSIONS_SETTINGS_FILE),
            app_state_root,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self) -> CoreResult<PermissionsConfig> {
        match fs::read_to_string(&self.path) {
            Ok(raw) => {
                let settings: PermissionsConfig = serde_json::from_str(&raw)?;
                settings.validate()?;
                Ok(settings)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(PermissionsConfig::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// 普通加载首次建立全局文件时迁移当前项目旧权限；锁内复验避免两个项目竞速决定初值。
    pub fn read_global_or_migrate(
        app_state_root: impl AsRef<Path>,
        project_root: Option<&Path>,
    ) -> CoreResult<PermissionsConfig> {
        let store = Self::default_for_app(app_state_root);
        let lock = store.lock_exclusive()?;
        let result = if store.path.is_file() {
            store.read()
        } else {
            let settings = project_root
                .map(read_legacy_project_permissions)
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

    /// maintenance 只读路径不得创建或迁移文件；全局文件缺失时仅以内存方式兼容旧项目。
    pub fn read_global_or_legacy(
        app_state_root: impl AsRef<Path>,
        project_root: Option<&Path>,
    ) -> CoreResult<PermissionsConfig> {
        let store = Self::default_for_app(app_state_root);
        if store.path.is_file() {
            return store.read();
        }
        let settings = project_root
            .map(read_legacy_project_permissions)
            .transpose()?
            .flatten()
            .unwrap_or_default();
        settings.validate()?;
        Ok(settings)
    }

    pub fn write(&self, settings: &PermissionsConfig) -> CoreResult<()> {
        settings.validate()?;
        let lock = self.lock_exclusive()?;
        let result = self.write_unlocked(settings);
        drop(lock);
        result
    }

    fn write_unlocked(&self, settings: &PermissionsConfig) -> CoreResult<()> {
        settings.validate()?;
        let body = serde_json::to_vec_pretty(settings)?;
        crate::config::store::atomic_write(&self.path, &body)
    }

    fn lock_exclusive(&self) -> CoreResult<std::fs::File> {
        crate::config::store::acquire_app_state_lock(
            &self.app_state_root,
            APP_PERMISSIONS_LOCK_FILE,
            "app_permissions_lock",
        )
    }
}

fn read_legacy_project_permissions(project_root: &Path) -> CoreResult<Option<PermissionsConfig>> {
    let path = project_root
        .join(CONFIG_DIR_NAME)
        .join(PERMISSIONS_CONFIG_FILE);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(yaml_serde::from_str(&raw)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_project_permissions_migrate_only_once() {
        let project_a = tempfile::tempdir().unwrap();
        let project_b = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        for project in [&project_a, &project_b] {
            fs::create_dir_all(project.path().join(CONFIG_DIR_NAME)).unwrap();
        }
        let mut project_a_permissions = PermissionsConfig::default();
        project_a_permissions.policy.allow_network = true;
        let mut project_b_permissions = PermissionsConfig::default();
        project_b_permissions.policy.allow_network = false;
        fs::write(
            project_a
                .path()
                .join(CONFIG_DIR_NAME)
                .join(PERMISSIONS_CONFIG_FILE),
            yaml_serde::to_string(&project_a_permissions).unwrap(),
        )
        .unwrap();
        fs::write(
            project_b
                .path()
                .join(CONFIG_DIR_NAME)
                .join(PERMISSIONS_CONFIG_FILE),
            yaml_serde::to_string(&project_b_permissions).unwrap(),
        )
        .unwrap();

        let migrated =
            AppPermissionsStore::read_global_or_migrate(app_state.path(), Some(project_a.path()))
                .unwrap();
        assert!(migrated.policy.allow_network);

        let reread =
            AppPermissionsStore::read_global_or_migrate(app_state.path(), Some(project_b.path()))
                .unwrap();
        assert!(reread.policy.allow_network);
    }

    #[test]
    fn malformed_legacy_permissions_are_not_replaced_with_global_defaults() {
        let project = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        fs::create_dir_all(project.path().join(CONFIG_DIR_NAME)).unwrap();
        fs::write(
            project
                .path()
                .join(CONFIG_DIR_NAME)
                .join(PERMISSIONS_CONFIG_FILE),
            "policy: [not-valid",
        )
        .unwrap();

        AppPermissionsStore::read_global_or_migrate(app_state.path(), Some(project.path()))
            .unwrap_err();

        assert!(!app_state
            .path()
            .join(APP_PERMISSIONS_SETTINGS_FILE)
            .exists());
    }
}
