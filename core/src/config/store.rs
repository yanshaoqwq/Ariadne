use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs4::FileExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use yaml_serde::Value;

use crate::config::migration::{migrate_yaml_value, MigrationReport};
use crate::config::models::{
    AppConfig, AutoModeConfig, GitConfig, PermissionsConfig, ProjectConfig, ProvidersConfig,
    RagConfig, WorkflowConfig, APP_CONFIG_FILE, AUTO_MODE_CONFIG_FILE, CONFIG_DIR_NAME,
    GIT_CONFIG_FILE, PERMISSIONS_CONFIG_FILE, PROVIDERS_CONFIG_FILE, RAG_CONFIG_FILE,
    WORKFLOW_CONFIG_FILE,
};
use crate::config::{AppPermissionsStore, ProviderCatalogStore};
use crate::contracts::{CoreError, CoreResult};

/// 项目配置文件存储，负责分文件读写和迁移。
#[derive(Debug, Clone)]
pub struct ConfigStore {
    project_root: PathBuf,
    app_state_root: PathBuf,
}

impl ConfigStore {
    /// 创建配置存储。
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let project_root = project_root.into();
        Self {
            app_state_root: crate::config::trusted_app_state_for_project(&project_root),
            project_root,
        }
    }

    /// 使用调用方可信应用状态目录构造配置存储。
    pub fn with_app_state(
        project_root: impl Into<PathBuf>,
        app_state_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            project_root: project_root.into(),
            app_state_root: app_state_root.into(),
        }
    }

    /// 返回项目根目录。
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// 返回 `.config` 目录路径。
    pub fn config_dir(&self) -> PathBuf {
        self.project_root.join(CONFIG_DIR_NAME)
    }

    /// 加载配置；缺失时先创建默认文件，再执行迁移。
    pub fn load_or_create(&self) -> CoreResult<ProjectConfig> {
        self.ensure_config_dir()?;
        self.recover_pending_commit()?;
        self.create_missing_defaults()?;
        self.migrate_all()?;
        self.load_internal(true, true, true)
    }

    /// 只供“用户重新输入 Provider 密钥”流程读取旧配置。
    /// 该入口会一次性移除全部旧 `api_key` 引用；调用方只能写入可信项目作用域。
    pub fn load_or_create_for_credential_rebind(&self) -> CoreResult<ProjectConfig> {
        self.ensure_config_dir()?;
        self.recover_pending_commit()?;
        self.create_missing_defaults()?;
        self.migrate_all()?;
        let mut config = self.load_internal(false, true, false)?;
        clear_project_owned_secret_refs(&mut config);
        Ok(config)
    }

    /// 加载所有配置文件并执行整体验证。
    pub fn load(&self) -> CoreResult<ProjectConfig> {
        self.recover_pending_commit()?;
        self.load_internal(true, true, true)
    }

    /// 严格只读加载既有配置，不创建默认值、不迁移，也不恢复待提交事务。
    ///
    /// 仅供 maintenance 期间仍需可读的状态查询使用；普通业务读取仍应走
    /// `load`/`load_or_create`，并在调用侧持有项目 mutation fence。
    pub fn load_read_only(&self) -> CoreResult<ProjectConfig> {
        self.load_internal(true, false, true)
    }

    /// 只读加载项目显示所需的 app 配置。
    ///
    /// maintenance 期间允许 app.yaml 暂时缺失并由调用方使用目录名回退；但文件一旦
    /// 存在，IO/YAML 错误必须向上传播，不能把损坏配置伪装成健康项目。
    pub fn load_app_read_only_optional(&self) -> CoreResult<Option<AppConfig>> {
        let path = self.config_dir().join(APP_CONFIG_FILE);
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        Ok(Some(yaml_serde::from_str(&raw)?))
    }

    fn load_internal(
        &self,
        reject_project_secret_refs: bool,
        migrate_app_permissions: bool,
        validate_config: bool,
    ) -> CoreResult<ProjectConfig> {
        let mut config = ProjectConfig {
            app: self.read_config(APP_CONFIG_FILE)?,
            providers: self.read_config(PROVIDERS_CONFIG_FILE)?,
            permissions: self.read_config(PERMISSIONS_CONFIG_FILE)?,
            rag: self.read_config(RAG_CONFIG_FILE)?,
            workflow: self.read_config(WORKFLOW_CONFIG_FILE)?,
            git: self.read_config(GIT_CONFIG_FILE)?,
            auto_mode: self.read_config(AUTO_MODE_CONFIG_FILE)?,
        };

        ProviderCatalogStore::default_for_app(&self.app_state_root)
            .read()?
            .merge_authorized(&mut config.providers)?;
        config.permissions = if migrate_app_permissions {
            AppPermissionsStore::read_global_or_migrate(
                &self.app_state_root,
                Some(&self.project_root),
            )?
        } else {
            AppPermissionsStore::read_global_or_legacy(
                &self.app_state_root,
                Some(&self.project_root),
            )?
        };

        if reject_project_secret_refs {
            reject_project_owned_secret_refs(&config)?;
        }
        if validate_config {
            config.validate()?;
        }
        Ok(config)
    }

    /// 保存完整项目配置。
    ///
    /// D4-a：经 `atomic_commit::commit_files` 写入——唯一 transaction stage、fsync、
    /// 相对路径 containment；不再使用固定 `.stage-save` 目录（并发同 target 会互踩）。
    pub fn save(&self, config: &ProjectConfig) -> CoreResult<()> {
        reject_project_owned_secret_refs(config)?;
        config.validate()?;
        self.ensure_config_dir()?;
        let provider_catalog =
            ProviderCatalogStore::default_for_app(&self.app_state_root).read()?;
        let project_providers = provider_catalog.project_projection(&config.providers);
        let pairs: [(&str, String); 7] = [
            (
                APP_CONFIG_FILE,
                yaml_serde::to_string(&yaml_serde::to_value(&config.app)?)?,
            ),
            (
                PROVIDERS_CONFIG_FILE,
                yaml_serde::to_string(&yaml_serde::to_value(&project_providers)?)?,
            ),
            (
                PERMISSIONS_CONFIG_FILE,
                yaml_serde::to_string(&yaml_serde::to_value(PermissionsConfig::default())?)?,
            ),
            (
                RAG_CONFIG_FILE,
                yaml_serde::to_string(&yaml_serde::to_value(&config.rag)?)?,
            ),
            (
                WORKFLOW_CONFIG_FILE,
                yaml_serde::to_string(&yaml_serde::to_value(&config.workflow)?)?,
            ),
            (
                GIT_CONFIG_FILE,
                yaml_serde::to_string(&yaml_serde::to_value(&config.git)?)?,
            ),
            (
                AUTO_MODE_CONFIG_FILE,
                yaml_serde::to_string(&yaml_serde::to_value(&config.auto_mode)?)?,
            ),
        ];
        let files: Vec<(crate::config::AtomicCommitTarget, Vec<u8>)> = pairs
            .iter()
            .zip([
                crate::config::AtomicCommitTarget::App,
                crate::config::AtomicCommitTarget::Providers,
                crate::config::AtomicCommitTarget::Permissions,
                crate::config::AtomicCommitTarget::Rag,
                crate::config::AtomicCommitTarget::Workflow,
                crate::config::AtomicCommitTarget::Git,
                crate::config::AtomicCommitTarget::AutoMode,
            ])
            .map(|((_, raw), target)| (target, raw.as_bytes().to_vec()))
            .collect();
        crate::config::atomic_commit::commit_files(
            &self.project_root,
            &self.app_state_root,
            crate::config::AtomicCommitProfile::ProjectConfig,
            &files,
        )?;
        Ok(())
    }

    fn recover_pending_commit(&self) -> CoreResult<()> {
        crate::config::atomic_commit::recover_pending_commit(
            &self.project_root,
            &self.app_state_root,
        )
    }

    /// 对所有已存在配置文件执行幂等迁移。
    pub fn migrate_all(&self) -> CoreResult<Vec<MigrationReport>> {
        let mut reports = Vec::new();
        for file_name in config_file_names() {
            let path = self.config_dir().join(file_name);
            if !path.exists() {
                continue;
            }

            let raw = fs::read_to_string(&path)?;
            let value = if raw.trim().is_empty() {
                Value::Null
            } else {
                yaml_serde::from_str::<Value>(&raw)?
            };
            let (migrated, report) = migrate_yaml_value(file_name, value)?;
            if report.changed {
                // 只有确实变更时才回写，避免无意义修改用户配置文件。
                self.write_yaml_value(file_name, &migrated)?;
            }
            reports.push(report);
        }

        Ok(reports)
    }

    /// 确保配置目录存在。
    fn ensure_config_dir(&self) -> CoreResult<()> {
        fs::create_dir_all(self.config_dir())?;
        Ok(())
    }

    /// 为缺失的配置文件写入默认内容。
    fn create_missing_defaults(&self) -> CoreResult<()> {
        self.write_default_if_missing(APP_CONFIG_FILE, &AppConfig::default())?;
        self.write_default_if_missing(PROVIDERS_CONFIG_FILE, &ProvidersConfig::default())?;
        self.write_default_if_missing(PERMISSIONS_CONFIG_FILE, &PermissionsConfig::default())?;
        self.write_default_if_missing(RAG_CONFIG_FILE, &RagConfig::default())?;
        self.write_default_if_missing(WORKFLOW_CONFIG_FILE, &WorkflowConfig::default())?;
        self.write_default_if_missing(GIT_CONFIG_FILE, &GitConfig::default())?;
        self.write_default_if_missing(AUTO_MODE_CONFIG_FILE, &AutoModeConfig::default())?;
        Ok(())
    }

    /// 文件不存在时写入默认配置。
    fn write_default_if_missing<T: Serialize>(&self, file_name: &str, value: &T) -> CoreResult<()> {
        let path = self.config_dir().join(file_name);
        if !path.exists() {
            self.write_config(file_name, value)?;
        }
        Ok(())
    }

    /// 读取并反序列化单个配置文件。
    fn read_config<T: DeserializeOwned>(&self, file_name: &str) -> CoreResult<T> {
        let path = self.config_dir().join(file_name);
        if !path.exists() {
            return Err(CoreError::validation(format!(
                "missing config file: {}",
                path.display()
            )));
        }

        let raw = fs::read_to_string(path)?;
        Ok(yaml_serde::from_str(&raw)?)
    }

    /// 序列化并写入单个配置文件。
    fn write_config<T: Serialize>(&self, file_name: &str, value: &T) -> CoreResult<()> {
        let value = yaml_serde::to_value(value)?;
        self.write_yaml_value(file_name, &value)
    }

    /// 直接写入 YAML Value，用于迁移后的回写。
    fn write_yaml_value(&self, file_name: &str, value: &Value) -> CoreResult<()> {
        let path = self.config_dir().join(file_name);
        let raw = yaml_serde::to_string(value)?;
        atomic_write(&path, raw.as_bytes())?;
        Ok(())
    }
}

fn reject_project_owned_secret_refs(config: &ProjectConfig) -> CoreResult<()> {
    if let Some(provider) = config
        .providers
        .providers
        .iter()
        .find(|provider| provider.api_key.is_some())
    {
        return Err(CoreError::validation(format!(
            "provider '{}' contains an untrusted project SecretRef; re-enter the credential to bind it in trusted app state",
            provider.provider_id
        )));
    }
    Ok(())
}

fn clear_project_owned_secret_refs(config: &mut ProjectConfig) {
    for provider in &mut config.providers.providers {
        provider.api_key = None;
    }
}

/// 为单个应用状态域取得跨进程 advisory lock。
pub(crate) fn acquire_app_state_lock(
    app_state_root: &Path,
    lock_file_name: &str,
    service: &str,
) -> CoreResult<File> {
    fs::create_dir_all(app_state_root)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(app_state_root.join(lock_file_name))?;
    file.lock_exclusive().map_err(|error| CoreError::External {
        service: service.to_owned(),
        message: error.to_string(),
    })?;
    Ok(file)
}

/// Write via temp file + rename (best-effort atomic replace on POSIX).
///
/// D4-a：临时名含 PID + 纳秒，避免同进程并发写同一固定 temp；写后尽力 fsync。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        nanos
    ));
    {
        use std::io::Write;
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        let _ = file.sync_all();
    }
    fs::rename(&tmp, path)?;
    // Best-effort directory fsync so rename is durable on crash.
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Exclusive write lock for a document path (D1-a CAS window / D1-b lifecycle).
///
/// 锁文件位于项目树外并保持稳定；所有权由内核 advisory lock 管理，进程退出自动释放，
/// 避免 PID/TTL 猜测以及删除旧 inode 后两个写者分别持锁。
pub struct PathWriteLock {
    lock_file: File,
    lock_path: PathBuf,
}

impl PathWriteLock {
    pub fn acquire(target: &Path) -> CoreResult<Self> {
        let lock_path = document_write_lock_path(target)?;
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        for attempt in 0..400 {
            match FileExt::try_lock_exclusive(&lock_file) {
                Ok(()) => {
                    return Ok(Self {
                        lock_file,
                        lock_path,
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if attempt == 399 {
                        return Err(crate::contracts::CoreError::validation(format!(
                            "timed out waiting for document write lock: {}",
                            lock_path.display()
                        )));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(crate::contracts::CoreError::validation(
            "document write lock acquisition failed",
        ))
    }

    /// Test / diagnostics: lock file is never under the document directory.
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

impl Drop for PathWriteLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock_file);
    }
}

fn document_write_lock_path(target: &Path) -> CoreResult<PathBuf> {
    let canonical = canonical_document_identity(target)?;
    let mut digest = Sha256::new();
    digest.update(b"ariadne-document-write-v2\0");
    digest.update(document_path_identity_bytes(&canonical));
    let encoded = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(std::env::temp_dir()
        .join("ariadne-document-write-locks")
        .join(format!("{encoded}.lock")))
}

fn canonical_document_identity(target: &Path) -> CoreResult<PathBuf> {
    if let Ok(canonical) = target.canonicalize() {
        return Ok(canonical);
    }
    if let (Some(parent), Some(file_name)) = (target.parent(), target.file_name()) {
        if let Ok(canonical_parent) = parent.canonicalize() {
            return Ok(canonical_parent.join(file_name));
        }
    }
    if target.is_absolute() {
        Ok(target.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(target))
    }
}

#[cfg(unix)]
fn document_path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = b"unix\0".to_vec();
    bytes.extend_from_slice(path.as_os_str().as_bytes());
    bytes
}

#[cfg(windows)]
fn document_path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut bytes = b"windows\0".to_vec();
    for unit in path.as_os_str().encode_wide() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// 返回当前分文件配置清单。
fn config_file_names() -> [&'static str; 7] {
    [
        APP_CONFIG_FILE,
        PROVIDERS_CONFIG_FILE,
        PERMISSIONS_CONFIG_FILE,
        RAG_CONFIG_FILE,
        WORKFLOW_CONFIG_FILE,
        GIT_CONFIG_FILE,
        AUTO_MODE_CONFIG_FILE,
    ]
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::config::{ProviderConfig, SecretRef};
    use crate::contracts::{ProviderCapability, ProviderType};

    use super::*;

    #[test]
    fn load_or_create_writes_split_config_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp_dir.path());
        let config = store.load_or_create().unwrap();

        assert_eq!(config.app.schema_version, 1);
        for file_name in config_file_names() {
            assert!(store.config_dir().join(file_name).exists());
        }
    }

    #[test]
    fn provider_config_rejects_project_owned_secret_ref_without_writing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp_dir.path());
        let mut config = store.load_or_create().unwrap();
        config.providers.providers.push(ProviderConfig {
            provider_id: "local-openai".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local OpenAI-compatible".to_owned(),
            enabled: true,
            base_url: Some("http://127.0.0.1:11434/v1".to_owned()),
            api_key: Some(SecretRef::new("provider.local-openai")),
            models: vec![crate::config::ModelConfig {
                model_id: "local-model".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(16_384),
                input_cost_per_million_tokens: Some(0.0),
                output_cost_per_million_tokens: Some(0.0),
            }],
        });

        let error = store.save(&config).unwrap_err();
        let raw = fs::read_to_string(store.config_dir().join(PROVIDERS_CONFIG_FILE)).unwrap();

        assert!(error
            .to_string()
            .contains("contains an untrusted project SecretRef"));
        assert!(!raw.contains("provider.local-openai"));
    }

    #[test]
    fn migration_is_repeatable() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp_dir.path());
        store.ensure_config_dir().unwrap();
        fs::write(
            store.config_dir().join(APP_CONFIG_FILE),
            "project_name: Migrated\n",
        )
        .unwrap();

        let first = store.migrate_all().unwrap();
        let second = store.migrate_all().unwrap();

        assert!(first.iter().any(|report| report.changed));
        assert!(second.iter().all(|report| !report.changed));
    }
}
