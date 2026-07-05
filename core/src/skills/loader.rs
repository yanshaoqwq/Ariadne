use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::contracts::{CoreError, CoreResult, SkillRegistry};
use crate::skills::models::{
    PromptTemplateManifest, PromptTemplateReference, PromptTemplateUpdateKind,
    PromptTemplateUpdateStatus, PromptTemplateVersion, SkillManifest, WorkflowManifest,
    PROMPT_TEMPLATE_MANIFEST_FILE, SKILL_MANIFEST_FILE, WORKFLOW_MANIFEST_FILE,
};

/// Skill 来源位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSourceKind {
    Global,
    Project,
}

/// PromptTemplate 与 Workflow 来源位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateSourceKind {
    Global,
    Project,
}

/// 带来源路径的 Skill manifest。
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedSkillManifest {
    pub manifest: SkillManifest,
    pub source: SkillSourceKind,
    pub manifest_path: PathBuf,
}

/// 项目 ExecutorAdapter 覆盖全局同 id manifest 的诊断信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillManifestOverride {
    pub skill_id: String,
    pub global_manifest_path: PathBuf,
    pub project_manifest_path: PathBuf,
}

/// 带来源路径的 PromptTemplate manifest。
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedPromptTemplateManifest {
    pub manifest: PromptTemplateManifest,
    pub source: TemplateSourceKind,
    pub manifest_path: PathBuf,
}

/// 带来源路径的 Workflow manifest。
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedWorkflowManifest {
    pub manifest: WorkflowManifest,
    pub source: TemplateSourceKind,
    pub manifest_path: PathBuf,
}

/// Skill 加载器，项目目录优先覆盖全局目录。
#[derive(Debug, Clone, Default)]
pub struct SkillLoader {
    global_roots: Vec<PathBuf>,
    project_roots: Vec<PathBuf>,
}

impl SkillLoader {
    /// 创建空加载器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 增加全局 Skill 根目录。
    pub fn with_global_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.global_roots.push(root.into());
        self
    }

    /// 增加项目 Skill 根目录。
    pub fn with_project_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.project_roots.push(root.into());
        self
    }

    /// 加载所有 Skill manifest，项目 Skill 覆盖同 id 全局 Skill。
    pub fn load_manifests(&self) -> CoreResult<Vec<LoadedSkillManifest>> {
        let mut manifests = BTreeMap::new();
        for root in &self.global_roots {
            load_root(root, SkillSourceKind::Global, &mut manifests)?;
        }
        for root in &self.project_roots {
            load_root(root, SkillSourceKind::Project, &mut manifests)?;
        }
        Ok(manifests.into_values().collect())
    }

    /// 加载并生成核心 SkillRegistry。
    pub fn load_registry(&self) -> CoreResult<SkillRegistry> {
        let mut registry = SkillRegistry::default();
        for loaded in self.load_manifests()? {
            registry.register(loaded.manifest.to_core_definition()?)?;
        }
        Ok(registry)
    }

    /// 返回项目 Skill 覆盖全局 Skill 的诊断列表，加载语义仍保持项目优先。
    pub fn detect_overrides(&self) -> CoreResult<Vec<SkillManifestOverride>> {
        let mut global_paths = BTreeMap::new();
        for root in &self.global_roots {
            collect_skill_manifest_paths(root, &mut global_paths)?;
        }

        let mut overrides = BTreeMap::new();
        for root in &self.project_roots {
            let mut project_paths = BTreeMap::new();
            collect_skill_manifest_paths(root, &mut project_paths)?;
            for (skill_id, project_manifest_path) in project_paths {
                if let Some(global_manifest_path) = global_paths.get(&skill_id) {
                    overrides.insert(
                        skill_id.clone(),
                        SkillManifestOverride {
                            skill_id,
                            global_manifest_path: global_manifest_path.clone(),
                            project_manifest_path,
                        },
                    );
                }
            }
        }

        Ok(overrides.into_values().collect())
    }
}

/// PromptTemplate 加载器，按 template_id + version 管理固定版本。
#[derive(Debug, Clone, Default)]
pub struct PromptTemplateLoader {
    global_roots: Vec<PathBuf>,
    project_roots: Vec<PathBuf>,
}

impl PromptTemplateLoader {
    /// 创建空 PromptTemplate 加载器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 增加全局 PromptTemplate 根目录。
    pub fn with_global_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.global_roots.push(root.into());
        self
    }

    /// 增加项目 PromptTemplate 根目录。
    pub fn with_project_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.project_roots.push(root.into());
        self
    }

    /// 加载所有 PromptTemplate，项目同 id 同版本覆盖全局模板。
    pub fn load_manifests(&self) -> CoreResult<Vec<LoadedPromptTemplateManifest>> {
        let mut manifests = BTreeMap::new();
        for root in &self.global_roots {
            load_prompt_template_root(root, TemplateSourceKind::Global, &mut manifests)?;
        }
        for root in &self.project_roots {
            load_prompt_template_root(root, TemplateSourceKind::Project, &mut manifests)?;
        }
        Ok(manifests.into_values().collect())
    }

    /// 根据固定版本引用解析模板，并校验内容 hash 没有漂移。
    pub fn resolve_reference(
        &self,
        reference: &PromptTemplateReference,
    ) -> CoreResult<LoadedPromptTemplateManifest> {
        reference.validate()?;
        let key = prompt_template_key(&reference.template_id, &reference.version);
        let manifests = self
            .load_manifests()?
            .into_iter()
            .map(|loaded| (prompt_template_loaded_key(&loaded), loaded))
            .collect::<BTreeMap<_, _>>();
        let loaded = manifests
            .get(&key)
            .cloned()
            .ok_or_else(|| CoreError::validation(format!("prompt template not found: {key}")))?;
        let actual_hash = loaded.manifest.content_hash()?;
        if actual_hash != reference.content_hash {
            return Err(CoreError::validation(format!(
                "prompt template content hash mismatch for {key}"
            )));
        }
        Ok(loaded)
    }

    /// 检测锁定模板是否有新版本；只报告，不自动升级节点配置。
    pub fn update_status(
        &self,
        reference: &PromptTemplateReference,
    ) -> CoreResult<PromptTemplateUpdateStatus> {
        reference.validate()?;
        let locked_version = PromptTemplateVersion::parse(&reference.version)?;
        let latest = self
            .load_manifests()?
            .into_iter()
            .filter(|loaded| loaded.manifest.template_id == reference.template_id)
            .filter_map(|loaded| {
                PromptTemplateVersion::parse(&loaded.manifest.version)
                    .ok()
                    .map(|version| (version, loaded.manifest.version))
            })
            .max_by_key(|(version, _)| *version);

        let (latest_version, update_kind) = match latest {
            Some((version, raw)) => {
                let kind = locked_version.update_kind(version);
                (Some(raw), kind)
            }
            None => (None, PromptTemplateUpdateKind::None),
        };

        Ok(PromptTemplateUpdateStatus {
            template_id: reference.template_id.clone(),
            locked_version: reference.version.clone(),
            latest_version,
            update_kind,
        })
    }
}

/// Workflow 模板加载器。
#[derive(Debug, Clone, Default)]
pub struct WorkflowTemplateLoader {
    global_roots: Vec<PathBuf>,
    project_roots: Vec<PathBuf>,
}

impl WorkflowTemplateLoader {
    /// 创建空 Workflow 模板加载器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 增加全局 Workflow 模板根目录。
    pub fn with_global_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.global_roots.push(root.into());
        self
    }

    /// 增加项目 Workflow 模板根目录。
    pub fn with_project_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.project_roots.push(root.into());
        self
    }

    /// 加载所有 Workflow 模板，项目同 id 同版本覆盖全局模板。
    pub fn load_manifests(&self) -> CoreResult<Vec<LoadedWorkflowManifest>> {
        let mut manifests = BTreeMap::new();
        for root in &self.global_roots {
            load_workflow_root(root, TemplateSourceKind::Global, &mut manifests)?;
        }
        for root in &self.project_roots {
            load_workflow_root(root, TemplateSourceKind::Project, &mut manifests)?;
        }
        Ok(manifests.into_values().collect())
    }

    /// 按 id 和版本读取 Workflow 模板。
    pub fn get(&self, workflow_id: &str, version: &str) -> CoreResult<LoadedWorkflowManifest> {
        let key = workflow_key(workflow_id, version);
        self.load_manifests()?
            .into_iter()
            .find(|loaded| workflow_loaded_key(loaded) == key)
            .ok_or_else(|| CoreError::validation(format!("workflow template not found: {key}")))
    }
}

/// 读取根目录下一层 Skill manifest。
fn load_root(
    root: &Path,
    source: SkillSourceKind,
    manifests: &mut BTreeMap<String, LoadedSkillManifest>,
) -> CoreResult<()> {
    if !root.exists() {
        return Ok(());
    }
    if !root.is_dir() {
        return Err(CoreError::validation(format!(
            "skill root is not a directory: {}",
            root.display()
        )));
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(SKILL_MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }
        let text = fs::read_to_string(&manifest_path)?;
        let manifest: SkillManifest = serde_json::from_str(&text)?;
        manifest.validate()?;
        manifests.insert(
            manifest.skill_id.clone(),
            LoadedSkillManifest {
                manifest,
                source,
                manifest_path,
            },
        );
    }
    Ok(())
}

/// 收集根目录下一层 Skill manifest 路径，用于覆盖诊断。
fn collect_skill_manifest_paths(
    root: &Path,
    paths: &mut BTreeMap<String, PathBuf>,
) -> CoreResult<()> {
    if !root.exists() {
        return Ok(());
    }
    if !root.is_dir() {
        return Err(CoreError::validation(format!(
            "skill root is not a directory: {}",
            root.display()
        )));
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(SKILL_MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }
        let text = fs::read_to_string(&manifest_path)?;
        let manifest: SkillManifest = serde_json::from_str(&text)?;
        manifest.validate()?;
        paths.insert(manifest.skill_id, manifest_path);
    }
    Ok(())
}

/// 读取根目录下一层 PromptTemplate manifest。
fn load_prompt_template_root(
    root: &Path,
    source: TemplateSourceKind,
    manifests: &mut BTreeMap<String, LoadedPromptTemplateManifest>,
) -> CoreResult<()> {
    if !root.exists() {
        return Ok(());
    }
    if !root.is_dir() {
        return Err(CoreError::validation(format!(
            "prompt template root is not a directory: {}",
            root.display()
        )));
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(PROMPT_TEMPLATE_MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }
        let text = fs::read_to_string(&manifest_path)?;
        let manifest: PromptTemplateManifest = serde_json::from_str(&text)?;
        manifest.validate()?;
        let key = prompt_template_key(&manifest.template_id, &manifest.version);
        manifests.insert(
            key,
            LoadedPromptTemplateManifest {
                manifest,
                source,
                manifest_path,
            },
        );
    }
    Ok(())
}

/// 读取根目录下一层 Workflow manifest。
fn load_workflow_root(
    root: &Path,
    source: TemplateSourceKind,
    manifests: &mut BTreeMap<String, LoadedWorkflowManifest>,
) -> CoreResult<()> {
    if !root.exists() {
        return Ok(());
    }
    if !root.is_dir() {
        return Err(CoreError::validation(format!(
            "workflow root is not a directory: {}",
            root.display()
        )));
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(WORKFLOW_MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }
        let text = fs::read_to_string(&manifest_path)?;
        let manifest: WorkflowManifest = serde_json::from_str(&text)?;
        manifest.validate()?;
        let key = workflow_key(&manifest.workflow_id, &manifest.version);
        manifests.insert(
            key,
            LoadedWorkflowManifest {
                manifest,
                source,
                manifest_path,
            },
        );
    }
    Ok(())
}

/// 构造 PromptTemplate 固定版本 key。
fn prompt_template_key(template_id: &str, version: &str) -> String {
    format!("{template_id}@{version}")
}

/// 构造已加载 PromptTemplate 的固定版本 key。
fn prompt_template_loaded_key(loaded: &LoadedPromptTemplateManifest) -> String {
    prompt_template_key(&loaded.manifest.template_id, &loaded.manifest.version)
}

/// 构造 Workflow 固定版本 key。
fn workflow_key(workflow_id: &str, version: &str) -> String {
    format!("{workflow_id}@{version}")
}

/// 构造已加载 Workflow 的固定版本 key。
fn workflow_loaded_key(loaded: &LoadedWorkflowManifest) -> String {
    workflow_key(&loaded.manifest.workflow_id, &loaded.manifest.version)
}
