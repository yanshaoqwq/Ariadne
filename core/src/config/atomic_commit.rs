//! 可信应用状态驱动的项目配置多文件提交（N2 / D4-a / S4）。
//!
//! 项目目录是不可信输入，不能携带可执行的恢复路径或恢复授权。恢复 journal 与
//! writer serialization 均位于项目树之外的 app-state；项目内 stage 只保存固定目标
//! 的 payload，并由外部 journal 的事务身份、目标枚举、长度和摘要逐项校验。

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::models::{
    APP_CONFIG_FILE, AUTO_MODE_CONFIG_FILE, GIT_CONFIG_FILE, PERMISSIONS_CONFIG_FILE,
    PROVIDERS_CONFIG_FILE, RAG_CONFIG_FILE, WORKFLOW_CONFIG_FILE,
};
use crate::contracts::{CoreError, CoreResult};

const JOURNAL_VERSION: u32 = 3;
const STAGE_OWNER_VERSION: u32 = 1;
const AUTHORITY_DIR: &str = "atomic-commits";
const AUTHORITY_JOURNAL: &str = "journal.json";
const AUTHORITY_LOCK_DB: &str = "writer-lock.db";
const LEGACY_PROJECT_JOURNAL: &str = "atomic-commit.journal.json";
const STAGE_PREFIX: &str = ".atomic-stage-";
const STAGE_OWNER_FILE: &str = "owner.json";
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;
const MAX_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024;

/// 多文件提交唯一允许触达的配置目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomicCommitTarget {
    App,
    Providers,
    Permissions,
    Rag,
    Workflow,
    Git,
    AutoMode,
    Budget,
    ConfirmationPolicies,
}

impl AtomicCommitTarget {
    fn file_name(self) -> &'static str {
        match self {
            Self::App => APP_CONFIG_FILE,
            Self::Providers => PROVIDERS_CONFIG_FILE,
            Self::Permissions => PERMISSIONS_CONFIG_FILE,
            Self::Rag => RAG_CONFIG_FILE,
            Self::Workflow => WORKFLOW_CONFIG_FILE,
            Self::Git => GIT_CONFIG_FILE,
            Self::AutoMode => AUTO_MODE_CONFIG_FILE,
            Self::Budget => "budget.json",
            Self::ConfirmationPolicies => "confirmation_policy_settings.json",
        }
    }

    fn stage_name(self) -> String {
        format!("{}.payload", self.file_name())
    }
}

const PROJECT_CONFIG_TARGETS: [AtomicCommitTarget; 7] = [
    AtomicCommitTarget::App,
    AtomicCommitTarget::Providers,
    AtomicCommitTarget::Permissions,
    AtomicCommitTarget::Rag,
    AtomicCommitTarget::Workflow,
    AtomicCommitTarget::Git,
    AtomicCommitTarget::AutoMode,
];

const AUTOMATION_SETTINGS_TARGETS: [AtomicCommitTarget; 9] = [
    AtomicCommitTarget::App,
    AtomicCommitTarget::Providers,
    AtomicCommitTarget::Permissions,
    AtomicCommitTarget::Rag,
    AtomicCommitTarget::Workflow,
    AtomicCommitTarget::Git,
    AtomicCommitTarget::AutoMode,
    AtomicCommitTarget::Budget,
    AtomicCommitTarget::ConfirmationPolicies,
];

/// 每类命令拥有固定目标集合；journal 无法自行扩权。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomicCommitProfile {
    ProjectConfig,
    AutomationSettings,
}

impl AtomicCommitProfile {
    fn expected_targets(self) -> &'static [AtomicCommitTarget] {
        match self {
            Self::ProjectConfig => &PROJECT_CONFIG_TARGETS,
            Self::AutomationSettings => &AUTOMATION_SETTINGS_TARGETS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AtomicCommitEntry {
    target: AtomicCommitTarget,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AtomicCommitJournal {
    version: u32,
    transaction_id: String,
    project_identity: String,
    profile: AtomicCommitProfile,
    entries: Vec<AtomicCommitEntry>,
    #[serde(default)]
    completed: Vec<AtomicCommitTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StageOwner {
    version: u32,
    transaction_id: String,
    project_identity: String,
    profile: AtomicCommitProfile,
    manifest_sha256: String,
}

struct AtomicCommitCoordinator {
    config_dir: PathBuf,
    authority_dir: PathBuf,
    project_identity: String,
}

impl AtomicCommitCoordinator {
    fn new(project_root: &Path, app_state_root: &Path) -> CoreResult<Self> {
        let project_root = project_root.canonicalize()?;
        if !project_root.is_dir() {
            return Err(CoreError::validation("project root must be a directory"));
        }

        let config_dir = project_root.join(".config");
        validate_real_directory(&config_dir, "project config directory")?;

        let app_state_absolute = absolute_lexical(app_state_root)?;
        if app_state_absolute.starts_with(&project_root) {
            return Err(CoreError::validation(
                "atomic commit authority must be outside the project tree",
            ));
        }
        fs::create_dir_all(&app_state_absolute)?;
        let app_state_root = app_state_absolute.canonicalize()?;
        if app_state_root.starts_with(&project_root) {
            return Err(CoreError::validation(
                "atomic commit authority resolves inside the project tree",
            ));
        }

        let project_identity = project_identity(&project_root);
        let authority_dir = app_state_root.join(AUTHORITY_DIR).join(&project_identity);
        fs::create_dir_all(&authority_dir)?;
        validate_real_directory(&authority_dir, "atomic commit authority directory")?;
        let authority_dir = authority_dir.canonicalize()?;
        if !authority_dir.starts_with(&app_state_root) || authority_dir.starts_with(&project_root) {
            return Err(CoreError::validation(
                "atomic commit authority escaped trusted app state",
            ));
        }

        Ok(Self {
            config_dir,
            authority_dir,
            project_identity,
        })
    }

    fn journal_path(&self) -> PathBuf {
        self.authority_dir.join(AUTHORITY_JOURNAL)
    }

    fn stage_dir(&self, transaction_id: &str) -> PathBuf {
        self.config_dir
            .join(format!("{STAGE_PREFIX}{transaction_id}"))
    }

    fn acquire_writer(&self) -> CoreResult<WriterGuard> {
        WriterGuard::acquire(&self.authority_dir.join(AUTHORITY_LOCK_DB))
    }
}

struct WriterGuard {
    connection: Connection,
    finished: bool,
}

impl WriterGuard {
    fn acquire(path: &Path) -> CoreResult<Self> {
        let connection = Connection::open(path).map_err(atomic_lock_error)?;
        connection
            .busy_timeout(Duration::from_secs(30))
            .map_err(atomic_lock_error)?;
        connection
            .execute_batch("BEGIN IMMEDIATE;")
            .map_err(atomic_lock_error)?;
        Ok(Self {
            connection,
            finished: false,
        })
    }

    fn commit(mut self) -> CoreResult<()> {
        self.connection
            .execute_batch("COMMIT;")
            .map_err(atomic_lock_error)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for WriterGuard {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.connection.execute_batch("ROLLBACK;");
        }
    }
}

/// 在可信 app-state authority 下提交固定项目配置目标。
pub fn commit_files(
    project_root: &Path,
    app_state_root: &Path,
    profile: AtomicCommitProfile,
    files: &[(AtomicCommitTarget, Vec<u8>)],
) -> CoreResult<()> {
    commit_files_with_fail_after(project_root, app_state_root, profile, files, None)
}

/// 测试钩子：完成指定数量 rename 后返回错误，保留可恢复 journal。
pub fn commit_files_with_fail_after(
    project_root: &Path,
    app_state_root: &Path,
    profile: AtomicCommitProfile,
    files: &[(AtomicCommitTarget, Vec<u8>)],
    fail_after: Option<usize>,
) -> CoreResult<()> {
    let coordinator = AtomicCommitCoordinator::new(project_root, app_state_root)?;
    let guard = coordinator.acquire_writer()?;
    let trusted_transaction = trusted_transaction_id(&coordinator)?;
    reject_project_owned_recovery_artifacts(&coordinator, trusted_transaction)?;
    recover_locked(&coordinator)?;
    validate_payload_set(profile, files)?;

    let transaction_id = random_hex_id()?;
    let stage_dir = coordinator.stage_dir(&transaction_id);
    fs::create_dir(&stage_dir)?;

    let mut entries = Vec::with_capacity(files.len());
    for (target, bytes) in files {
        if bytes.len() as u64 > MAX_PAYLOAD_BYTES {
            return Err(CoreError::validation(format!(
                "atomic commit payload {} exceeds {} bytes",
                target.file_name(),
                MAX_PAYLOAD_BYTES
            )));
        }
        let staged = stage_dir.join(target.stage_name());
        write_new_fsynced(&staged, bytes)?;
        entries.push(AtomicCommitEntry {
            target: *target,
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        });
    }

    let journal = AtomicCommitJournal {
        version: JOURNAL_VERSION,
        transaction_id: transaction_id.clone(),
        project_identity: coordinator.project_identity.clone(),
        profile,
        entries,
        completed: Vec::new(),
    };
    let owner = StageOwner {
        version: STAGE_OWNER_VERSION,
        transaction_id,
        project_identity: coordinator.project_identity.clone(),
        profile,
        manifest_sha256: manifest_sha256(&journal)?,
    };
    write_new_fsynced(
        &stage_dir.join(STAGE_OWNER_FILE),
        &serde_json::to_vec_pretty(&owner)?,
    )?;
    sync_directory(&stage_dir)?;
    write_json_fsynced(&coordinator.journal_path(), &journal)?;
    sync_directory(&coordinator.authority_dir)?;

    apply_trusted_journal(&coordinator, journal, fail_after)?;
    guard.commit()
}

/// 恢复 app-state 中由 Ariadne 创建的提交；项目内 legacy/孤儿恢复数据只隔离不执行。
pub fn recover_pending_commit(project_root: &Path, app_state_root: &Path) -> CoreResult<()> {
    let coordinator = AtomicCommitCoordinator::new(project_root, app_state_root)?;
    let guard = coordinator.acquire_writer()?;
    reject_project_owned_recovery_artifacts(&coordinator, trusted_transaction_id(&coordinator)?)?;
    recover_locked(&coordinator)?;
    guard.commit()
}

/// True if a trusted app-state authority journal exists for this project.
pub fn has_pending_journal(project_root: &Path, app_state_root: &Path) -> bool {
    match AtomicCommitCoordinator::new(project_root, app_state_root) {
        Ok(coordinator) => coordinator.journal_path().exists(),
        Err(_) => false,
    }
}

fn recover_locked(coordinator: &AtomicCommitCoordinator) -> CoreResult<()> {
    let journal_path = coordinator.journal_path();
    if !journal_path.exists() {
        return Ok(());
    }
    let journal: AtomicCommitJournal = read_bounded_json(&journal_path, MAX_JOURNAL_BYTES)?;
    validate_journal(coordinator, &journal)?;
    apply_trusted_journal(coordinator, journal, None)
}

fn apply_trusted_journal(
    coordinator: &AtomicCommitCoordinator,
    mut journal: AtomicCommitJournal,
    fail_after: Option<usize>,
) -> CoreResult<()> {
    validate_journal(coordinator, &journal)?;
    let stage_dir = coordinator.stage_dir(&journal.transaction_id);
    validate_stage_tree(coordinator, &journal, &stage_dir)?;

    let mut completed: BTreeSet<_> = journal.completed.iter().copied().collect();
    let mut renamed = 0usize;
    for entry in &journal.entries {
        if completed.contains(&entry.target) {
            continue;
        }
        let staged = stage_dir.join(entry.target.stage_name());
        let final_path = coordinator.config_dir.join(entry.target.file_name());

        if !path_exists_without_following(&staged)? {
            validate_file_digest(&final_path, entry)?;
            completed.insert(entry.target);
            journal.completed = completed.iter().copied().collect();
            write_json_fsynced(&coordinator.journal_path(), &journal)?;
            continue;
        }
        if fail_after.is_some_and(|limit| renamed >= limit) {
            return Err(CoreError::validation(format!(
                "injected atomic commit failure after {renamed} renames"
            )));
        }

        replace_file(&staged, &final_path)?;
        sync_directory(&coordinator.config_dir)?;
        completed.insert(entry.target);
        journal.completed = completed.iter().copied().collect();
        write_json_fsynced(&coordinator.journal_path(), &journal)?;
        renamed += 1;
    }

    clear_commit_artifacts(coordinator, &journal)?;
    Ok(())
}

fn validate_payload_set(
    profile: AtomicCommitProfile,
    files: &[(AtomicCommitTarget, Vec<u8>)],
) -> CoreResult<()> {
    let actual: Vec<_> = files.iter().map(|(target, _)| *target).collect();
    if actual != profile.expected_targets() {
        return Err(CoreError::validation(format!(
            "atomic commit target set does not match {:?} profile",
            profile
        )));
    }
    Ok(())
}

fn validate_journal(
    coordinator: &AtomicCommitCoordinator,
    journal: &AtomicCommitJournal,
) -> CoreResult<()> {
    if journal.version != JOURNAL_VERSION {
        return Err(CoreError::validation(format!(
            "unsupported atomic commit journal version {}",
            journal.version
        )));
    }
    validate_transaction_id(&journal.transaction_id)?;
    if journal.project_identity != coordinator.project_identity {
        return Err(CoreError::validation(
            "atomic commit journal belongs to a different project identity",
        ));
    }
    let targets: Vec<_> = journal.entries.iter().map(|entry| entry.target).collect();
    if targets != journal.profile.expected_targets() {
        return Err(CoreError::validation(
            "atomic commit journal target set is not the fixed profile allowlist",
        ));
    }
    for entry in &journal.entries {
        if entry.size > MAX_PAYLOAD_BYTES || !is_sha256_hex(&entry.sha256) {
            return Err(CoreError::validation(format!(
                "invalid atomic commit manifest for {}",
                entry.target.file_name()
            )));
        }
    }
    let completed: BTreeSet<_> = journal.completed.iter().copied().collect();
    if completed.len() != journal.completed.len()
        || !completed
            .iter()
            .all(|target| journal.profile.expected_targets().contains(target))
    {
        return Err(CoreError::validation(
            "atomic commit journal contains invalid completion progress",
        ));
    }
    Ok(())
}

fn validate_stage_tree(
    coordinator: &AtomicCommitCoordinator,
    journal: &AtomicCommitJournal,
    stage_dir: &Path,
) -> CoreResult<()> {
    let all_completed = journal.completed.len() == journal.entries.len();
    if !path_exists_without_following(stage_dir)? {
        if all_completed {
            for entry in &journal.entries {
                validate_file_digest(
                    &coordinator.config_dir.join(entry.target.file_name()),
                    entry,
                )?;
            }
            return Ok(());
        }
        return Err(CoreError::validation(
            "trusted atomic commit stage directory is missing",
        ));
    }
    validate_real_directory(stage_dir, "atomic commit stage directory")?;

    let owner_path = stage_dir.join(STAGE_OWNER_FILE);
    reject_symlink(&owner_path, "atomic commit stage owner")?;
    let owner: StageOwner = read_bounded_json(&owner_path, MAX_JOURNAL_BYTES)?;
    let expected_owner = StageOwner {
        version: STAGE_OWNER_VERSION,
        transaction_id: journal.transaction_id.clone(),
        project_identity: coordinator.project_identity.clone(),
        profile: journal.profile,
        manifest_sha256: manifest_sha256(journal)?,
    };
    if owner != expected_owner {
        return Err(CoreError::validation(
            "atomic commit stage ownership manifest does not match trusted authority",
        ));
    }

    let allowed_names: BTreeSet<_> = journal
        .entries
        .iter()
        .map(|entry| entry.target.stage_name())
        .chain(std::iter::once(STAGE_OWNER_FILE.to_owned()))
        .collect();
    let actual_names: BTreeSet<_> = fs::read_dir(stage_dir)?
        .map(|entry| {
            let entry = entry?;
            entry.file_name().into_string().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "non-UTF8 atomic commit stage entry",
                )
            })
        })
        .collect::<Result<_, _>>()?;
    if !actual_names.contains(STAGE_OWNER_FILE)
        || !actual_names.iter().all(|name| allowed_names.contains(name))
    {
        return Err(CoreError::validation(
            "atomic commit stage contains an unexpected entry or lacks its owner manifest",
        ));
    }

    for entry in &journal.entries {
        let staged = stage_dir.join(entry.target.stage_name());
        let final_path = coordinator.config_dir.join(entry.target.file_name());
        reject_symlink_if_exists(&final_path, "atomic commit final target")?;
        if journal.completed.contains(&entry.target) {
            if path_exists_without_following(&staged)? {
                return Err(CoreError::validation(
                    "completed atomic commit target still has a staged payload",
                ));
            }
            validate_file_digest(&final_path, entry)?;
        } else if path_exists_without_following(&staged)? {
            reject_symlink(&staged, "atomic commit staged payload")?;
            validate_file_digest(&staged, entry)?;
        } else {
            validate_file_digest(&final_path, entry)?;
        }
    }
    Ok(())
}

fn clear_commit_artifacts(
    coordinator: &AtomicCommitCoordinator,
    journal: &AtomicCommitJournal,
) -> CoreResult<()> {
    let stage_dir = coordinator.stage_dir(&journal.transaction_id);
    if path_exists_without_following(&stage_dir)? {
        validate_real_directory(&stage_dir, "atomic commit stage directory")?;
        let owner = stage_dir.join(STAGE_OWNER_FILE);
        if path_exists_without_following(&owner)? {
            reject_symlink(&owner, "atomic commit stage owner")?;
            fs::remove_file(owner)?;
        }
        fs::remove_dir(&stage_dir)?;
        sync_directory(&coordinator.config_dir)?;
    }

    let journal_path = coordinator.journal_path();
    if path_exists_without_following(&journal_path)? {
        reject_symlink(&journal_path, "atomic commit authority journal")?;
        fs::remove_file(journal_path)?;
        sync_directory(&coordinator.authority_dir)?;
    }
    Ok(())
}

fn reject_project_owned_recovery_artifacts(
    coordinator: &AtomicCommitCoordinator,
    trusted_transaction: Option<String>,
) -> CoreResult<()> {
    let legacy = coordinator.config_dir.join(LEGACY_PROJECT_JOURNAL);
    if path_exists_without_following(&legacy)? {
        quarantine_project_entry(&coordinator.config_dir, &legacy, "legacy-journal")?;
        return Err(CoreError::validation(
            "project-owned atomic commit journal was quarantined without execution",
        ));
    }

    let mut quarantined = Vec::new();
    for entry in fs::read_dir(&coordinator.config_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(STAGE_PREFIX) {
            continue;
        }
        let transaction = name.strip_prefix(STAGE_PREFIX).unwrap_or_default();
        if trusted_transaction.as_deref() == Some(transaction) {
            continue;
        }
        quarantine_project_entry(&coordinator.config_dir, &entry.path(), "orphan-stage")?;
        quarantined.push(name);
    }
    if !quarantined.is_empty() {
        return Err(CoreError::validation(format!(
            "untrusted atomic commit stages quarantined: {}",
            quarantined.join(", ")
        )));
    }
    Ok(())
}

fn trusted_transaction_id(coordinator: &AtomicCommitCoordinator) -> CoreResult<Option<String>> {
    let journal_path = coordinator.journal_path();
    if !journal_path.exists() {
        return Ok(None);
    }
    let journal: AtomicCommitJournal = read_bounded_json(&journal_path, MAX_JOURNAL_BYTES)?;
    validate_journal(coordinator, &journal)?;
    Ok(Some(journal.transaction_id))
}

fn quarantine_project_entry(config_dir: &Path, source: &Path, kind: &str) -> CoreResult<()> {
    let id = random_hex_id()?;
    let destination = config_dir.join(format!(".atomic-quarantine-{kind}-{id}"));
    fs::rename(source, destination)?;
    sync_directory(config_dir)
}

fn validate_real_directory(path: &Path, label: &str) -> CoreResult<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::validation(format!(
            "{label} must be a real directory, not a symlink"
        )));
    }
    Ok(())
}

fn reject_symlink(path: &Path, label: &str) -> CoreResult<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::validation(format!(
            "{label} must be a regular file, not a symlink"
        )));
    }
    Ok(())
}

fn reject_symlink_if_exists(path: &Path, label: &str) -> CoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            CoreError::validation(format!("{label} must be a regular file, not a symlink")),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn path_exists_without_following(path: &Path) -> CoreResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn validate_file_digest(path: &Path, entry: &AtomicCommitEntry) -> CoreResult<()> {
    reject_symlink(path, "atomic commit payload")?;
    let metadata = fs::metadata(path)?;
    if metadata.len() != entry.size || entry.size > MAX_PAYLOAD_BYTES {
        return Err(CoreError::validation(format!(
            "atomic commit payload size mismatch for {}",
            entry.target.file_name()
        )));
    }
    let digest = sha256_file(path, MAX_PAYLOAD_BYTES)?;
    if digest != entry.sha256 {
        return Err(CoreError::validation(format!(
            "atomic commit payload digest mismatch for {}",
            entry.target.file_name()
        )));
    }
    Ok(())
}

fn replace_file(source: &Path, destination: &Path) -> CoreResult<()> {
    reject_symlink(source, "atomic commit staged payload")?;
    reject_symlink_if_exists(destination, "atomic commit final target")?;
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

fn write_new_fsynced(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_json_fsynced<T: Serialize>(path: &Path, value: &T) -> CoreResult<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(CoreError::validation(
            "atomic commit authority record exceeds size limit",
        ));
    }
    let tmp = path.with_file_name(format!(".journal-{}.tmp", random_hex_id()?));
    write_new_fsynced(&tmp, &bytes)?;
    reject_symlink_if_exists(path, "atomic commit authority journal")?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn read_bounded_json<T: for<'de> Deserialize<'de>>(path: &Path, limit: u64) -> CoreResult<T> {
    reject_symlink(path, "atomic commit metadata")?;
    let metadata = fs::metadata(path)?;
    if metadata.len() > limit {
        return Err(CoreError::validation(
            "atomic commit metadata exceeds size limit",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(CoreError::validation(
            "atomic commit metadata exceeds size limit",
        ));
    }
    serde_json::from_slice(&bytes).map_err(CoreError::from)
}

fn sync_directory(path: &Path) -> CoreResult<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn manifest_sha256(journal: &AtomicCommitJournal) -> CoreResult<String> {
    let manifest = serde_json::to_vec(&(
        journal.version,
        &journal.transaction_id,
        &journal.project_identity,
        journal.profile,
        &journal.entries,
    ))?;
    Ok(sha256_hex(&manifest))
}

fn project_identity(project_root: &Path) -> String {
    let mut hasher = Sha256::new();
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
    hex_encode(&hasher.finalize())
}

fn absolute_lexical(path: &Path) -> CoreResult<PathBuf> {
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

fn validate_transaction_id(value: &str) -> CoreResult<()> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CoreError::validation(
            "atomic commit transaction id is not a trusted random identifier",
        ));
    }
    Ok(())
}

fn random_hex_id() -> CoreResult<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| CoreError::External {
        service: "atomic_commit_random".to_owned(),
        message: error.to_string(),
    })?;
    Ok(hex_encode(&bytes))
}

fn sha256_file(path: &Path, limit: u64) -> CoreResult<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut read = 0u64;
    let mut buffer = [0u8; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        read = read.saturating_add(count as u64);
        if read > limit {
            return Err(CoreError::validation(
                "atomic commit payload exceeds size limit",
            ));
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn atomic_lock_error(error: rusqlite::Error) -> CoreError {
    CoreError::External {
        service: "atomic_commit_writer_lock".to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn config_payloads(name: &str) -> Vec<(AtomicCommitTarget, Vec<u8>)> {
        PROJECT_CONFIG_TARGETS
            .iter()
            .map(|target| {
                (
                    *target,
                    format!("{name}-{}", target.file_name()).into_bytes(),
                )
            })
            .collect()
    }

    #[test]
    fn commit_writes_fixed_targets_and_clears_trusted_artifacts() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        let app_state = dir.path().join("app-state");
        fs::create_dir_all(project.join(".config")).unwrap();

        commit_files(
            &project,
            &app_state,
            AtomicCommitProfile::ProjectConfig,
            &config_payloads("one"),
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(project.join(".config").join(APP_CONFIG_FILE)).unwrap(),
            "one-app.yaml"
        );
        assert!(fs::read_dir(project.join(".config"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(STAGE_PREFIX)));
    }

    #[test]
    fn mid_fail_recovers_from_app_state_authority() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        let app_state = dir.path().join("app-state");
        fs::create_dir_all(project.join(".config")).unwrap();

        let error = commit_files_with_fail_after(
            &project,
            &app_state,
            AtomicCommitProfile::ProjectConfig,
            &config_payloads("resume"),
            Some(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("injected"));

        recover_pending_commit(&project, &app_state).unwrap();
        for target in PROJECT_CONFIG_TARGETS {
            assert_eq!(
                fs::read_to_string(project.join(".config").join(target.file_name())).unwrap(),
                format!("resume-{}", target.file_name())
            );
        }
    }

    #[test]
    fn project_owned_legacy_journal_is_quarantined_without_execution() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        let app_state = dir.path().join("app-state");
        let config = project.join(".config");
        fs::create_dir_all(&config).unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let victim = outside.join("victim.txt");
        fs::write(&victim, b"keep").unwrap();
        fs::write(
            config.join(LEGACY_PROJECT_JOURNAL),
            format!(r#"{{"stage_dir":"{}","entries":[]}}"#, outside.display()),
        )
        .unwrap();

        let error = recover_pending_commit(&project, &app_state).unwrap_err();
        assert!(error.to_string().contains("quarantined"));
        assert_eq!(fs::read_to_string(victim).unwrap(), "keep");
        assert!(!config.join(LEGACY_PROJECT_JOURNAL).exists());
    }

    #[cfg(unix)]
    #[test]
    fn stage_and_final_symlink_escape_are_rejected_before_replace() {
        use std::os::unix::fs::symlink;

        for attack_final in [false, true] {
            let dir = tempdir().unwrap();
            let project = dir.path().join("project");
            let app_state = dir.path().join("app-state");
            let config = project.join(".config");
            fs::create_dir_all(&config).unwrap();
            let outside = dir.path().join("outside.txt");
            fs::write(&outside, b"keep").unwrap();

            commit_files_with_fail_after(
                &project,
                &app_state,
                AtomicCommitProfile::ProjectConfig,
                &config_payloads("safe"),
                Some(0),
            )
            .unwrap_err();
            let coordinator = AtomicCommitCoordinator::new(&project, &app_state).unwrap();
            let journal: AtomicCommitJournal =
                read_bounded_json(&coordinator.journal_path(), MAX_JOURNAL_BYTES).unwrap();
            if attack_final {
                symlink(&outside, config.join(APP_CONFIG_FILE)).unwrap();
            } else {
                let staged = coordinator
                    .stage_dir(&journal.transaction_id)
                    .join(AtomicCommitTarget::App.stage_name());
                fs::remove_file(&staged).unwrap();
                symlink(&outside, staged).unwrap();
            }

            assert!(recover_pending_commit(&project, &app_state).is_err());
            assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
        }
    }

    #[test]
    fn forged_project_stage_has_no_recovery_authority() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        let app_state = dir.path().join("app-state");
        let config = project.join(".config");
        fs::create_dir_all(&config).unwrap();
        let forged = config.join(format!("{STAGE_PREFIX}{}", "0".repeat(32)));
        fs::create_dir_all(&forged).unwrap();
        fs::write(forged.join("app.yaml.payload"), b"forged").unwrap();

        let error = recover_pending_commit(&project, &app_state).unwrap_err();
        assert!(error.to_string().contains("quarantined"));
        assert!(!config.join(APP_CONFIG_FILE).exists());
    }
}
