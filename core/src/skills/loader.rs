use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contracts::{CoreError, CoreResult, SkillRegistry};
use crate::skills::models::{
    PromptTemplateManifest, PromptTemplateReference, PromptTemplateUpdateKind,
    PromptTemplateUpdateStatus, PromptTemplateVersion, SkillManifest, WorkflowManifest,
    PROMPT_TEMPLATE_MANIFEST_FILE, SKILL_MANIFEST_FILE, WORKFLOW_MANIFEST_FILE,
};

/// Skill 来源位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    /// 只加载当前执行闭包要求的 Skill manifest。
    ///
    /// 未引用 Skill 的损坏配置不得阻断其它工作流；被引用 Skill 缺失、损坏或项目
    /// 覆盖身份不一致时仍 fail-loud。
    pub fn load_required_manifests(
        &self,
        required_skill_ids: &std::collections::BTreeSet<String>,
    ) -> CoreResult<Vec<LoadedSkillManifest>> {
        if required_skill_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut manifests = BTreeMap::new();
        for root in &self.global_roots {
            load_required_global_root(root, required_skill_ids, &mut manifests)?;
        }
        load_required_project_roots(&self.project_roots, required_skill_ids, &mut manifests)?;
        let missing = required_skill_ids
            .iter()
            .filter(|skill_id| !manifests.contains_key(*skill_id))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(CoreError::validation(format!(
                "required skill manifest not found: {}",
                missing.join(", ")
            )));
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

        let mut project_paths = BTreeMap::new();
        for root in &self.project_roots {
            collect_skill_manifest_paths(root, &mut project_paths)?;
        }

        let mut overrides = BTreeMap::new();
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

        Ok(overrides.into_values().collect())
    }
}

/// 建立根目录下一层 manifest 的稳定路径索引；所有 Skill 发现入口共用该顺序。
fn skill_manifest_paths(root: &Path) -> CoreResult<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        return Err(CoreError::validation(format!(
            "skill root is not a directory: {}",
            root.display()
        )));
    }

    let mut paths = Vec::new();
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(SKILL_MANIFEST_FILE);
        if manifest_path.is_file() {
            paths.push(manifest_path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn parse_skill_manifest(manifest_path: &Path) -> CoreResult<SkillManifest> {
    let text = fs::read_to_string(manifest_path)?;
    let manifest: SkillManifest = serde_json::from_str(&text)?;
    manifest.validate()?;
    Ok(manifest)
}

fn load_required_global_root(
    root: &Path,
    required_skill_ids: &std::collections::BTreeSet<String>,
    manifests: &mut BTreeMap<String, LoadedSkillManifest>,
) -> CoreResult<()> {
    for manifest_path in skill_manifest_paths(root)? {
        let directory_skill_id = manifest_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str());
        let directory_is_required =
            directory_skill_id.is_some_and(|skill_id| required_skill_ids.contains(skill_id));
        let text = match fs::read_to_string(&manifest_path) {
            Ok(text) => text,
            Err(_) if !directory_is_required => continue,
            Err(error) => return Err(error.into()),
        };
        let raw = match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(raw) => raw,
            Err(_) if !directory_is_required => continue,
            Err(error) => return Err(error.into()),
        };
        let declared_skill_id = raw
            .get("skill_id")
            .and_then(serde_json::Value::as_str)
            .filter(|skill_id| !skill_id.is_empty());
        let Some(declared_skill_id) = declared_skill_id else {
            if directory_is_required {
                return Err(CoreError::validation(format!(
                    "required global skill manifest has no skill_id: {}",
                    manifest_path.display()
                )));
            }
            continue;
        };
        if !required_skill_ids.contains(declared_skill_id) {
            if directory_is_required {
                return Err(CoreError::validation(format!(
                    "required skill directory '{}' declares different skill_id '{}': {}",
                    directory_skill_id.unwrap_or_default(),
                    declared_skill_id,
                    manifest_path.display()
                )));
            }
            continue;
        }

        let manifest: SkillManifest = serde_json::from_value(raw)?;
        manifest.validate()?;
        if manifests
            .get(&manifest.skill_id)
            .is_some_and(|existing| existing.source == SkillSourceKind::Global)
        {
            return Err(CoreError::validation(format!(
                "duplicate {:?} skill manifest for '{}': {}",
                SkillSourceKind::Global,
                manifest.skill_id,
                manifest_path.display()
            )));
        }
        manifests.insert(
            manifest.skill_id.clone(),
            LoadedSkillManifest {
                manifest,
                source: SkillSourceKind::Global,
                manifest_path,
            },
        );
    }
    Ok(())
}

fn load_required_project_roots(
    roots: &[PathBuf],
    required_skill_ids: &std::collections::BTreeSet<String>,
    manifests: &mut BTreeMap<String, LoadedSkillManifest>,
) -> CoreResult<()> {
    let mut project_manifests = BTreeMap::new();
    let mut ambiguous_manifests = Vec::new();

    for root in roots {
        for manifest_path in skill_manifest_paths(root)? {
            let directory_skill_id = manifest_path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let directory_is_required =
                directory_skill_id.is_some_and(|skill_id| required_skill_ids.contains(skill_id));
            let text = match fs::read_to_string(&manifest_path) {
                Ok(text) => text,
                Err(error) if !directory_is_required => {
                    ambiguous_manifests.push((manifest_path, error.to_string()));
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let raw = match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(raw) => raw,
                Err(error) if !directory_is_required => {
                    ambiguous_manifests.push((manifest_path, format!("invalid JSON: {error}")));
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let declared_skill_id = raw
                .get("skill_id")
                .and_then(serde_json::Value::as_str)
                .filter(|skill_id| !skill_id.is_empty());
            let Some(declared_skill_id) = declared_skill_id else {
                if directory_is_required {
                    return Err(CoreError::validation(format!(
                        "required project skill manifest has no skill_id: {}",
                        manifest_path.display()
                    )));
                }
                ambiguous_manifests.push((
                    manifest_path,
                    "manifest has no declared skill_id".to_owned(),
                ));
                continue;
            };
            if !required_skill_ids.contains(declared_skill_id) {
                if directory_is_required {
                    return Err(CoreError::validation(format!(
                        "required skill directory '{}' declares different skill_id '{}': {}",
                        directory_skill_id.unwrap_or_default(),
                        declared_skill_id,
                        manifest_path.display()
                    )));
                }
                continue;
            }

            let manifest: SkillManifest = serde_json::from_value(raw)?;
            manifest.validate()?;
            if project_manifests.contains_key(&manifest.skill_id) {
                return Err(CoreError::validation(format!(
                    "duplicate {:?} skill manifest for '{}': {}",
                    SkillSourceKind::Project,
                    manifest.skill_id,
                    manifest_path.display()
                )));
            }
            project_manifests.insert(
                manifest.skill_id.clone(),
                LoadedSkillManifest {
                    manifest,
                    source: SkillSourceKind::Project,
                    manifest_path,
                },
            );
        }
    }

    let unresolved = required_skill_ids
        .iter()
        .filter(|skill_id| !project_manifests.contains_key(*skill_id))
        .cloned()
        .collect::<Vec<_>>();
    if !unresolved.is_empty() && !ambiguous_manifests.is_empty() {
        let (path, reason) = &ambiguous_manifests[0];
        return Err(CoreError::validation(format!(
            "ambiguous project Skill manifest may override required skill(s) {}: {} ({reason})",
            unresolved.join(", "),
            path.display()
        )));
    }

    manifests.extend(project_manifests);
    Ok(())
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
    for manifest_path in skill_manifest_paths(root)? {
        let manifest = parse_skill_manifest(&manifest_path)?;
        if manifests
            .get(&manifest.skill_id)
            .is_some_and(|existing| existing.source == source)
        {
            return Err(CoreError::validation(format!(
                "duplicate {:?} skill manifest for '{}': {}",
                source,
                manifest.skill_id,
                manifest_path.display()
            )));
        }
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
    for manifest_path in skill_manifest_paths(root)? {
        let manifest = parse_skill_manifest(&manifest_path)?;
        if let Some(existing) = paths.insert(manifest.skill_id.clone(), manifest_path.clone()) {
            return Err(CoreError::validation(format!(
                "duplicate skill manifest for '{}': {} and {}",
                manifest.skill_id,
                existing.display(),
                manifest_path.display()
            )));
        }
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
