use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::{CoreError, CoreResult};
use crate::skills::{LoadedSkillManifest, SkillExecutorConfig, SkillLoader};

/// 当前工作流实际引用的 ExecutorAdapter 依赖计划。
///
/// manifest、LLM Provider 和 model 身份在运行组合前冻结；未引用 Skill 不会进入
/// Provider、权限、工具或 handler 注册链。
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutorAdapterExecutionPlan {
    manifests: Vec<LoadedSkillManifest>,
    llm_provider_models: BTreeMap<String, BTreeSet<String>>,
}

impl ExecutorAdapterExecutionPlan {
    pub fn compile(
        loader: &SkillLoader,
        required_skill_ids: &BTreeSet<String>,
    ) -> CoreResult<Self> {
        let manifests = loader.load_required_manifests(required_skill_ids)?;
        let mut llm_provider_models = BTreeMap::<String, BTreeSet<String>>::new();
        for loaded in &manifests {
            if let SkillExecutorConfig::Llm(config) = &loaded.manifest.executor {
                llm_provider_models
                    .entry(config.provider_id.clone())
                    .or_default()
                    .insert(config.model_id.clone());
            }
        }
        Ok(Self {
            manifests,
            llm_provider_models,
        })
    }

    pub fn empty() -> Self {
        Self {
            manifests: Vec::new(),
            llm_provider_models: BTreeMap::new(),
        }
    }

    /// 从运行快照中已冻结的 manifest 重建执行计划，不再访问当前磁盘。
    pub fn from_frozen_manifests(manifests: Vec<LoadedSkillManifest>) -> CoreResult<Self> {
        let mut skill_ids = BTreeSet::new();
        let mut llm_provider_models = BTreeMap::<String, BTreeSet<String>>::new();
        for loaded in &manifests {
            loaded.manifest.validate()?;
            if !skill_ids.insert(loaded.manifest.skill_id.clone()) {
                return Err(CoreError::validation(format!(
                    "duplicate frozen skill manifest: {}",
                    loaded.manifest.skill_id
                )));
            }
            if let SkillExecutorConfig::Llm(config) = &loaded.manifest.executor {
                llm_provider_models
                    .entry(config.provider_id.clone())
                    .or_default()
                    .insert(config.model_id.clone());
            }
        }
        Ok(Self {
            manifests,
            llm_provider_models,
        })
    }

    pub fn manifests(&self) -> &[LoadedSkillManifest] {
        &self.manifests
    }

    pub fn into_manifests(self) -> Vec<LoadedSkillManifest> {
        self.manifests
    }

    pub fn llm_provider_models(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.llm_provider_models
    }

    pub fn uses_llm(&self) -> bool {
        !self.llm_provider_models.is_empty()
    }
}
