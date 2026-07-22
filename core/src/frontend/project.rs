use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::config::{
    ConfigStore, ProjectLayout, APP_CONFIG_FILE, AUTO_MODE_CONFIG_FILE, GIT_CONFIG_FILE,
    PERMISSIONS_CONFIG_FILE, PROVIDERS_CONFIG_FILE, RAG_CONFIG_FILE, WORKFLOW_CONFIG_FILE,
};
use crate::contracts::{CoreError, CoreResult};
use crate::git::GitService;

static PROJECT_CREATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 最近项目存储。命令层只负责选择应用状态根，不参与 JSON 持久化细节。
#[derive(Debug, Clone)]
pub struct ProjectRegistryStore {
    path: PathBuf,
}

/// 最近项目条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProjectEntry {
    pub name: String,
    pub path: PathBuf,
    pub last_opened_ms: u64,
}

/// 项目初始化报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInitReport {
    pub project_root: PathBuf,
    pub project_name: String,
    #[serde(default)]
    pub created_dirs: Vec<PathBuf>,
    #[serde(default)]
    pub created_config_files: Vec<PathBuf>,
    pub git_initialized: bool,
    pub ready: bool,
}

/// 已发布但尚未由命令层确认激活的项目目录。
///
/// 目录事务与运行时事务分离：命令层完成运行时绑定和最近项目登记后调用 `commit`；
/// 任一后续步骤失败则先清理运行时，再调用 `rollback` 删除本次新建目录。
#[derive(Debug)]
#[must_use = "published project creation must be committed or rolled back"]
pub struct PublishedProjectCreation {
    project_root: PathBuf,
    report: ProjectInitReport,
}

impl PublishedProjectCreation {
    pub fn commit(self) -> ProjectInitReport {
        self.report
    }

    pub fn rollback(self) -> CoreResult<()> {
        match std::fs::remove_dir_all(&self.project_root) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

impl ProjectRegistryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_for_project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".runtime/recent_projects.json"))
    }

    pub fn read_all(&self) -> CoreResult<Vec<RecentProjectEntry>> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => serde_json::from_str(&content).map_err(Into::into),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn write_all(&self, entries: &[RecentProjectEntry]) -> CoreResult<()> {
        let bytes = serde_json::to_vec_pretty(entries)?;
        crate::config::store::atomic_write(&self.path, &bytes)?;
        Ok(())
    }

    pub fn record_opened(
        &self,
        name: impl Into<String>,
        project_root: impl Into<PathBuf>,
    ) -> CoreResult<Vec<RecentProjectEntry>> {
        let project_root = project_root.into();
        let mut entries = self.read_all()?;
        entries.retain(|entry| entry.path != project_root);
        entries.insert(
            0,
            RecentProjectEntry {
                name: name.into(),
                path: project_root,
                last_opened_ms: super::service::now_timestamp_ms(),
            },
        );
        entries.truncate(20);
        self.write_all(&entries)?;
        Ok(entries)
    }
}

/// 初始化既有目录，供测试、迁移与非桌面入口使用。
pub fn initialize_project(project_root: impl AsRef<Path>) -> CoreResult<ProjectInitReport> {
    let project_root = project_root.as_ref();
    initialize_project_with_store(project_root, ConfigStore::new(project_root), None)
}

/// 使用可信应用状态目录初始化项目。
pub fn initialize_project_with_app_state(
    project_root: impl AsRef<Path>,
    app_state_root: impl AsRef<Path>,
    project_name: Option<&str>,
) -> CoreResult<ProjectInitReport> {
    let project_root = project_root.as_ref();
    let app_state_root = app_state_root.as_ref();
    crate::config::bind_project_app_state(project_root, app_state_root)?;
    initialize_project_with_store(
        project_root,
        ConfigStore::with_app_state(project_root, app_state_root),
        project_name,
    )
}

/// 在同一父目录完成“临时初始化 → 校验 → 原子发布”，不混入运行时激活职责。
pub fn publish_initialized_project(
    project_root: impl AsRef<Path>,
    app_state_root: impl AsRef<Path>,
    project_name: Option<&str>,
) -> CoreResult<PublishedProjectCreation> {
    let project_root = project_root.as_ref();
    if project_root.exists() {
        return Err(CoreError::validation(format!(
            "project root already exists: {}",
            project_root.display()
        )));
    }
    let parent = project_root.parent().ok_or_else(|| {
        CoreError::validation("project root must have an existing parent directory")
    })?;
    if !parent.is_dir() {
        return Err(CoreError::validation(format!(
            "project parent is not a directory: {}",
            parent.display()
        )));
    }
    let stage_root = parent.join(format!(
        ".ariadne-create-{}-{}-{}",
        std::process::id(),
        super::service::now_timestamp_ms(),
        PROJECT_CREATION_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir(&stage_root)?;

    let mut report =
        match initialize_project_with_app_state(&stage_root, app_state_root, project_name) {
            Ok(report) => report,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&stage_root);
                return Err(error);
            }
        };
    if let Err(error) = std::fs::rename(&stage_root, project_root) {
        let _ = std::fs::remove_dir_all(&stage_root);
        return Err(error.into());
    }
    if let Err(error) = remap_project_init_report_paths(&mut report, &stage_root, project_root) {
        let _ = std::fs::remove_dir_all(project_root);
        return Err(error);
    }

    Ok(PublishedProjectCreation {
        project_root: project_root.to_path_buf(),
        report,
    })
}

fn initialize_project_with_store(
    project_root: &Path,
    config_store: ConfigStore,
    project_name: Option<&str>,
) -> CoreResult<ProjectInitReport> {
    if project_root.as_os_str().is_empty() {
        return Err(CoreError::validation("project_root cannot be empty"));
    }
    std::fs::create_dir_all(project_root)?;
    let fixed_dirs = [
        ".config",
        ".runtime",
        "planning",
        "planning/stages",
        "planning/chapters",
    ];
    let mut created_dirs = BTreeSet::new();
    for directory in fixed_dirs {
        let path = project_root.join(directory);
        std::fs::create_dir_all(&path)?;
        created_dirs.insert(path);
    }

    let mut config = config_store.load_or_create()?;
    if let Some(name) = project_name.map(str::trim).filter(|name| !name.is_empty()) {
        config.app.project_name = name.to_owned();
        config_store.save(&config)?;
    }

    let layout = ProjectLayout::from_app(project_root, &config.app)?;
    for path in [
        layout.documents,
        layout.workflows,
        layout.skills,
        layout.exports,
    ] {
        std::fs::create_dir_all(&path)?;
        created_dirs.insert(path);
    }

    let created_config_files = [
        APP_CONFIG_FILE,
        PROVIDERS_CONFIG_FILE,
        PERMISSIONS_CONFIG_FILE,
        RAG_CONFIG_FILE,
        WORKFLOW_CONFIG_FILE,
        GIT_CONFIG_FILE,
        AUTO_MODE_CONFIG_FILE,
    ]
    .into_iter()
    .map(|file_name| config_store.config_dir().join(file_name))
    .collect::<Vec<_>>();
    for path in &created_config_files {
        if !path.is_file() {
            return Err(CoreError::validation(format!(
                "project initialization did not create config file: {}",
                path.display()
            )));
        }
    }

    let git = GitService::new(project_root);
    git.init_repository()?;
    if !project_root.join(".git").is_dir() {
        return Err(CoreError::validation(
            "project initialization did not create Git metadata",
        ));
    }

    Ok(ProjectInitReport {
        project_root: project_root.to_path_buf(),
        project_name: config.app.project_name,
        created_dirs: created_dirs.into_iter().collect(),
        created_config_files,
        git_initialized: true,
        ready: true,
    })
}

fn remap_project_init_report_paths(
    report: &mut ProjectInitReport,
    stage_root: &Path,
    project_root: &Path,
) -> CoreResult<()> {
    fn remap(path: &Path, stage_root: &Path, project_root: &Path) -> CoreResult<PathBuf> {
        let relative = path.strip_prefix(stage_root).map_err(|_| {
            CoreError::validation(format!(
                "project initialization report escaped staging root: {}",
                path.display()
            ))
        })?;
        Ok(project_root.join(relative))
    }

    report.project_root = project_root.to_path_buf();
    report.created_dirs = report
        .created_dirs
        .iter()
        .map(|path| remap(path, stage_root, project_root))
        .collect::<CoreResult<Vec<_>>>()?;
    report.created_config_files = report
        .created_config_files
        .iter()
        .map(|path| remap(path, stage_root, project_root))
        .collect::<CoreResult<Vec<_>>>()?;
    Ok(())
}
