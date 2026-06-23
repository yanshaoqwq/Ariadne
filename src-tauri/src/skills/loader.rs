use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::{CoreError, CoreResult, SkillRegistry};
use crate::skills::models::{SkillManifest, SKILL_MANIFEST_FILE};

/// Skill 来源位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSourceKind {
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
