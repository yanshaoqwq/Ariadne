use std::fs;

use ariadne::config::{
    ConfigStore, ModelConfig, ProjectConfig, ProviderConfig, SecretRef, AUTO_MODE_CONFIG_FILE,
    PERMISSIONS_CONFIG_FILE,
};
use ariadne::core::{ProviderCapability, ProviderType};

#[test]
fn default_project_config_is_split_across_yaml_files() {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp_dir.path());
    let config = store.load_or_create().unwrap();

    assert_eq!(config.app.schema_version, 1);
    assert!(store.config_dir().join("app.yaml").exists());
    assert!(store.config_dir().join("providers.yaml").exists());
    assert!(store.config_dir().join("auto_mode.yaml").exists());
}

#[test]
fn auto_mode_config_does_not_change_permission_switches() {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp_dir.path());
    let mut config = store.load_or_create().unwrap();
    config.auto_mode.enabled_by_default = true;
    config.auto_mode.preauthorized_budget_usd = Some(5.0);
    store.save(&config).unwrap();

    let permissions_raw = fs::read_to_string(store.config_dir().join(PERMISSIONS_CONFIG_FILE))
        .expect("permissions config should exist");
    let auto_mode_raw = fs::read_to_string(store.config_dir().join(AUTO_MODE_CONFIG_FILE))
        .expect("auto mode config should exist");

    assert!(auto_mode_raw.contains("enabled_by_default: true"));
    assert!(permissions_raw.contains("allow_network: false"));
    assert!(permissions_raw.contains("allow_web_search: false"));
}

#[test]
fn openai_compatible_provider_supports_base_url_and_secret_ref() {
    let mut config = ProjectConfig::default();
    config.providers.providers.push(ProviderConfig {
        provider_id: "ollama".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Ollama".to_owned(),
        enabled: true,
        base_url: Some("http://127.0.0.1:11434/v1".to_owned()),
        api_key: Some(SecretRef::new("providers.ollama.api_key")),
        models: vec![ModelConfig {
            model_id: "qwen3".to_owned(),
            capability: ProviderCapability::Llm,
            max_context_tokens: Some(32_768),
            input_cost_per_million_tokens: Some(0.0),
            output_cost_per_million_tokens: Some(0.0),
        }],
    });
    config.providers.default_llm_provider_id = Some("ollama".to_owned());

    assert!(config.validate().is_ok());
}
