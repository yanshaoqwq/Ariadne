use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use yaml_serde::Value;

use crate::config::migration::{migrate_yaml_value, MigrationReport};
use crate::config::models::{
    AppConfig, AutoModeConfig, GitConfig, PermissionsConfig, ProjectConfig, ProvidersConfig,
    RagConfig, WorkflowConfig, APP_CONFIG_FILE, AUTO_MODE_CONFIG_FILE, CONFIG_DIR_NAME,
    GIT_CONFIG_FILE, PERMISSIONS_CONFIG_FILE, PROVIDERS_CONFIG_FILE, RAG_CONFIG_FILE,
    WORKFLOW_CONFIG_FILE,
};
use crate::core::{CoreError, CoreResult};

#[derive(Debug, Clone)]
pub struct ConfigStore {
    project_root: PathBuf,
}

impl ConfigStore {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn config_dir(&self) -> PathBuf {
        self.project_root.join(CONFIG_DIR_NAME)
    }

    pub fn load_or_create(&self) -> CoreResult<ProjectConfig> {
        self.ensure_config_dir()?;
        self.create_missing_defaults()?;
        self.migrate_all()?;
        self.load()
    }

    pub fn load(&self) -> CoreResult<ProjectConfig> {
        let config = ProjectConfig {
            app: self.read_config(APP_CONFIG_FILE)?,
            providers: self.read_config(PROVIDERS_CONFIG_FILE)?,
            permissions: self.read_config(PERMISSIONS_CONFIG_FILE)?,
            rag: self.read_config(RAG_CONFIG_FILE)?,
            workflow: self.read_config(WORKFLOW_CONFIG_FILE)?,
            git: self.read_config(GIT_CONFIG_FILE)?,
            auto_mode: self.read_config(AUTO_MODE_CONFIG_FILE)?,
        };

        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, config: &ProjectConfig) -> CoreResult<()> {
        config.validate()?;
        self.ensure_config_dir()?;
        self.write_config(APP_CONFIG_FILE, &config.app)?;
        self.write_config(PROVIDERS_CONFIG_FILE, &config.providers)?;
        self.write_config(PERMISSIONS_CONFIG_FILE, &config.permissions)?;
        self.write_config(RAG_CONFIG_FILE, &config.rag)?;
        self.write_config(WORKFLOW_CONFIG_FILE, &config.workflow)?;
        self.write_config(GIT_CONFIG_FILE, &config.git)?;
        self.write_config(AUTO_MODE_CONFIG_FILE, &config.auto_mode)?;
        Ok(())
    }

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
                self.write_yaml_value(file_name, &migrated)?;
            }
            reports.push(report);
        }

        Ok(reports)
    }

    fn ensure_config_dir(&self) -> CoreResult<()> {
        fs::create_dir_all(self.config_dir())?;
        Ok(())
    }

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

    fn write_default_if_missing<T: Serialize>(&self, file_name: &str, value: &T) -> CoreResult<()> {
        let path = self.config_dir().join(file_name);
        if !path.exists() {
            self.write_config(file_name, value)?;
        }
        Ok(())
    }

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

    fn write_config<T: Serialize>(&self, file_name: &str, value: &T) -> CoreResult<()> {
        let value = yaml_serde::to_value(value)?;
        self.write_yaml_value(file_name, &value)
    }

    fn write_yaml_value(&self, file_name: &str, value: &Value) -> CoreResult<()> {
        let path = self.config_dir().join(file_name);
        let raw = yaml_serde::to_string(value)?;
        fs::write(path, raw)?;
        Ok(())
    }
}

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
    use crate::core::{ProviderCapability, ProviderType};

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
    fn provider_config_serializes_key_id_not_secret_value() {
        let temp_dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(temp_dir.path());
        let mut config = ProjectConfig::default();
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

        store.save(&config).unwrap();
        let raw = fs::read_to_string(store.config_dir().join(PROVIDERS_CONFIG_FILE)).unwrap();

        assert!(raw.contains("provider.local-openai"));
        assert!(!raw.contains("sk-"));
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
