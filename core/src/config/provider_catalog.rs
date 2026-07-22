use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{current_schema_version, ProviderConfig, ProvidersConfig};
use crate::contracts::{CoreError, CoreResult};

pub const PROVIDER_CATALOG_FILE: &str = "provider_catalog.json";
const PROVIDER_CATALOG_LOCK_FILE: &str = ".provider-catalog.lock";

/// 应用级 Provider 连接目录。项目只保存授权、默认角色和项目凭据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderCatalog {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

impl Default for ProviderCatalog {
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            providers: Vec::new(),
        }
    }
}

impl ProviderCatalog {
    pub fn validate(&self) -> CoreResult<()> {
        let mut ids = BTreeSet::new();
        for provider in &self.providers {
            provider.validate()?;
            if provider.api_key.is_some() {
                return Err(CoreError::validation(format!(
                    "global provider '{}' must not contain a SecretRef",
                    provider.provider_id
                )));
            }
            if !ids.insert(provider.provider_id.as_str()) {
                return Err(CoreError::validation(format!(
                    "duplicate global provider_id: {}",
                    provider.provider_id
                )));
            }
        }
        Ok(())
    }

    pub fn upsert(&mut self, mut provider: ProviderConfig) -> CoreResult<()> {
        provider.api_key = None;
        provider.validate()?;
        if let Some(existing) = self
            .providers
            .iter_mut()
            .find(|existing| existing.provider_id == provider.provider_id)
        {
            *existing = provider;
        } else {
            self.providers.push(provider);
        }
        self.providers
            .sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        self.validate()
    }

    /// 将项目明确授权的全局条目覆盖到有效配置；未授权条目不可进入运行时。
    pub fn merge_authorized(&self, project: &mut ProvidersConfig) -> CoreResult<()> {
        self.validate()?;
        let catalog = self
            .providers
            .iter()
            .map(|provider| (provider.provider_id.as_str(), provider))
            .collect::<HashMap<_, _>>();

        for provider_id in &project.authorized_provider_ids {
            let Some(global) = catalog.get(provider_id.as_str()) else {
                if project
                    .providers
                    .iter()
                    .any(|provider| provider.provider_id == *provider_id)
                {
                    continue;
                }
                return Err(CoreError::validation(format!(
                    "project authorizes missing global provider: {provider_id}"
                )));
            };
            if let Some(existing) = project
                .providers
                .iter_mut()
                .find(|provider| provider.provider_id == *provider_id)
            {
                *existing = (*global).clone();
            } else {
                project.providers.push((*global).clone());
            }
        }
        project
            .providers
            .sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        Ok(())
    }

    /// 生成可写回项目的投影，避免把全局端点和模型目录复制进项目树。
    pub fn project_projection(&self, effective: &ProvidersConfig) -> ProvidersConfig {
        let global_ids = self
            .providers
            .iter()
            .map(|provider| provider.provider_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut projected = effective.clone();
        let authorized = projected.authorized_provider_ids.clone();
        projected.providers.retain(|provider| {
            !authorized.contains(&provider.provider_id)
                || !global_ids.contains(provider.provider_id.as_str())
        });
        projected
    }
}

#[derive(Debug, Clone)]
pub struct ProviderCatalogStore {
    app_state_root: PathBuf,
    path: PathBuf,
}

impl ProviderCatalogStore {
    pub fn default_for_app(app_state_root: impl AsRef<Path>) -> Self {
        let app_state_root = app_state_root.as_ref().to_path_buf();
        Self {
            path: app_state_root.join(PROVIDER_CATALOG_FILE),
            app_state_root,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self) -> CoreResult<ProviderCatalog> {
        self.read_unlocked()
    }

    pub(crate) fn read_unlocked(&self) -> CoreResult<ProviderCatalog> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => {
                let catalog: ProviderCatalog = serde_json::from_str(&raw)?;
                catalog.validate()?;
                Ok(catalog)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ProviderCatalog::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn write(&self, catalog: &ProviderCatalog) -> CoreResult<()> {
        let lock = self.lock_exclusive()?;
        let result = self.write_unlocked(catalog);
        drop(lock);
        result
    }

    pub(crate) fn write_unlocked(&self, catalog: &ProviderCatalog) -> CoreResult<()> {
        catalog.validate()?;
        let body = serde_json::to_vec_pretty(catalog)?;
        crate::config::store::atomic_write(&self.path, &body)
    }

    pub(crate) fn lock_exclusive(&self) -> CoreResult<std::fs::File> {
        crate::config::store::acquire_app_state_lock(
            &self.app_state_root,
            PROVIDER_CATALOG_LOCK_FILE,
            "provider_catalog_lock",
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::contracts::ProviderType;

    use super::*;

    fn provider(id: &str, url: &str) -> ProviderConfig {
        ProviderConfig {
            provider_id: id.to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: id.to_owned(),
            enabled: true,
            base_url: Some(url.to_owned()),
            api_key: None,
            models: Vec::new(),
        }
    }

    #[test]
    fn merges_only_explicitly_authorized_global_providers() {
        let catalog = ProviderCatalog {
            schema_version: current_schema_version(),
            providers: vec![provider("shared", "https://global.example/v1")],
        };
        let mut project = ProvidersConfig::default();
        catalog.merge_authorized(&mut project).unwrap();
        assert!(project.providers.is_empty());

        project.authorized_provider_ids.insert("shared".to_owned());
        catalog.merge_authorized(&mut project).unwrap();
        assert_eq!(project.providers[0].provider_id, "shared");
    }

    #[test]
    fn project_projection_drops_global_profile_but_keeps_authorization() {
        let catalog = ProviderCatalog {
            schema_version: current_schema_version(),
            providers: vec![provider("shared", "https://global.example/v1")],
        };
        let mut effective = ProvidersConfig::default();
        effective
            .authorized_provider_ids
            .insert("shared".to_owned());
        effective.providers = catalog.providers.clone();

        let projected = catalog.project_projection(&effective);
        assert!(projected.providers.is_empty());
        assert!(projected.authorized_provider_ids.contains("shared"));
    }

    #[test]
    fn independent_store_transactions_serialize_read_modify_write() {
        let app_state = tempfile::tempdir().unwrap();
        let first_store = ProviderCatalogStore::default_for_app(app_state.path());
        let second_store = ProviderCatalogStore::default_for_app(app_state.path());
        let (first_locked_tx, first_locked_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let (second_started_tx, second_started_rx) = mpsc::channel();
        let (second_done_tx, second_done_rx) = mpsc::channel();

        let first = thread::spawn(move || {
            let _lock = first_store.lock_exclusive().unwrap();
            let mut catalog = first_store.read_unlocked().unwrap();
            catalog
                .upsert(provider("first", "https://first.example/v1"))
                .unwrap();
            first_locked_tx.send(()).unwrap();
            release_first_rx.recv().unwrap();
            first_store.write_unlocked(&catalog).unwrap();
        });
        first_locked_rx.recv().unwrap();

        let second = thread::spawn(move || {
            second_started_tx.send(()).unwrap();
            let _lock = second_store.lock_exclusive().unwrap();
            let mut catalog = second_store.read_unlocked().unwrap();
            catalog
                .upsert(provider("second", "https://second.example/v1"))
                .unwrap();
            second_store.write_unlocked(&catalog).unwrap();
            second_done_tx.send(()).unwrap();
        });
        second_started_rx.recv().unwrap();
        assert!(second_done_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err());

        release_first_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();

        let catalog = ProviderCatalogStore::default_for_app(app_state.path())
            .read()
            .unwrap();
        assert_eq!(
            catalog
                .providers
                .iter()
                .map(|provider| provider.provider_id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }
}
