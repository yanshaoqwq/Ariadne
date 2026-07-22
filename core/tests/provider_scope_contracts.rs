use std::sync::Arc;

use ariadne::commands::{
    get_permissions_settings, get_provider_config, save_permissions_settings, save_provider_key,
    save_provider_settings, AriadneAppState, ProviderSettingsUpdate,
};
use ariadne::config::{ConfigStore, MemorySecretStore, ModelConfig, SecretStore};
use ariadne::contracts::{ProviderCapability, ProviderType};

fn provider_update(
    display_name: &str,
    model_id: &str,
    make_default_llm: bool,
) -> ProviderSettingsUpdate {
    ProviderSettingsUpdate {
        provider_id: "shared-openai".to_owned(),
        provider_type: ProviderType::OpenAi,
        display_name: display_name.to_owned(),
        enabled: true,
        base_url: None,
        models: vec![ModelConfig {
            model_id: model_id.to_owned(),
            capability: ProviderCapability::Llm,
            max_context_tokens: Some(32_000),
            input_cost_per_million_tokens: None,
            output_cost_per_million_tokens: None,
        }],
        make_default_llm,
        make_default_embedding: false,
        make_default_reranker: false,
        make_default_search: false,
    }
}

#[test]
fn provider_profiles_are_global_while_authorization_defaults_and_keys_remain_project_scoped() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project_a.path()).unwrap();
    ariadne::frontend::initialize_project(project_b.path()).unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    let state = AriadneAppState::new(project_a.path(), app_state.path(), Arc::clone(&secrets));
    let canonical_provider_id = "shared_openai";

    save_provider_settings(
        &state,
        provider_update("Shared OpenAI", "gpt-global-v1", true),
    )
    .unwrap();
    save_provider_key(
        &state,
        canonical_provider_id.to_owned(),
        "project-a-key".to_owned(),
    )
    .unwrap();

    let project_a_yaml =
        std::fs::read_to_string(project_a.path().join(".config/providers.yaml")).unwrap();
    assert!(project_a_yaml.contains("authorized_provider_ids"));
    assert!(!project_a_yaml.contains("Shared OpenAI"));
    assert!(!project_a_yaml.contains("gpt-global-v1"));
    let catalog = std::fs::read_to_string(app_state.path().join("provider_catalog.json")).unwrap();
    assert!(catalog.contains("Shared OpenAI"));
    assert!(catalog.contains("gpt-global-v1"));

    state.set_project_root(project_b.path()).unwrap();
    let before_authorization = get_provider_config(&state).unwrap();
    let available = before_authorization
        .providers
        .iter()
        .find(|provider| provider.provider == canonical_provider_id)
        .unwrap();
    assert!(!available.configured);
    assert!(!available.has_key);
    assert_eq!(before_authorization.default_llm_provider_id, None);

    save_provider_settings(
        &state,
        provider_update("Shared OpenAI v2", "gpt-global-v2", false),
    )
    .unwrap();
    let project_b_status = get_provider_config(&state).unwrap();
    assert!(project_b_status
        .providers
        .iter()
        .any(|provider| provider.provider == canonical_provider_id && provider.configured));
    assert_eq!(project_b_status.default_llm_provider_id, None);

    state.set_project_root(project_a.path()).unwrap();
    let project_a_status = get_provider_config(&state).unwrap();
    let shared = project_a_status
        .providers
        .iter()
        .find(|provider| provider.provider == canonical_provider_id)
        .unwrap();
    assert!(shared.configured);
    assert!(shared.has_key);
    assert_eq!(shared.display_name, "Shared OpenAI v2");
    assert_eq!(shared.models[0].model_id, "gpt-global-v2");
    assert_eq!(
        project_a_status.default_llm_provider_id.as_deref(),
        Some(canonical_provider_id)
    );
}

#[test]
fn failed_project_provider_write_rolls_back_global_catalog() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    save_provider_settings(
        &state,
        provider_update("Stable Provider", "stable-model", true),
    )
    .unwrap();
    let catalog_path = app_state.path().join("provider_catalog.json");
    let before = std::fs::read_to_string(&catalog_path).unwrap();

    let provider_path = project.path().join(".config/providers.yaml");
    std::fs::remove_file(&provider_path).unwrap();
    std::fs::create_dir(&provider_path).unwrap();
    let error = save_provider_settings(
        &state,
        provider_update("Broken Update", "broken-model", true),
    )
    .unwrap_err();

    assert!(error.to_string().contains("global provider update failed"));
    assert_eq!(std::fs::read_to_string(catalog_path).unwrap(), before);
}

#[test]
fn permission_baseline_is_app_global_and_unrelated_project_save_cannot_roll_it_back() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project_a.path()).unwrap();
    ariadne::frontend::initialize_project(project_b.path()).unwrap();
    let state = AriadneAppState::new(
        project_a.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let mut permissions = get_permissions_settings(&state).unwrap();
    permissions.policy.allow_network = true;
    permissions.policy.allow_web_search = true;
    save_permissions_settings(&state, permissions).unwrap();

    let global_json =
        std::fs::read_to_string(app_state.path().join("permissions_settings.json")).unwrap();
    assert!(global_json.contains("\"allow_network\": true"));
    let project_yaml =
        std::fs::read_to_string(project_a.path().join(".config/permissions.yaml")).unwrap();
    assert!(!project_yaml.contains("allow_network: true"));

    state.set_project_root(project_b.path()).unwrap();
    assert!(
        get_permissions_settings(&state)
            .unwrap()
            .policy
            .allow_network
    );

    let store_b = ConfigStore::with_app_state(project_b.path(), app_state.path());
    let mut config_b = store_b.load().unwrap();
    config_b.app.project_name = "Renamed B".to_owned();
    store_b.save(&config_b).unwrap();
    assert!(
        get_permissions_settings(&state)
            .unwrap()
            .policy
            .allow_network
    );

    let mut updated = get_permissions_settings(&state).unwrap();
    updated.policy.allow_network = false;
    updated.policy.allow_web_search = false;
    save_permissions_settings(&state, updated).unwrap();
    state.set_project_root(project_a.path()).unwrap();
    assert!(
        !get_permissions_settings(&state)
            .unwrap()
            .policy
            .allow_network
    );
}

#[test]
fn global_permissions_can_be_managed_without_an_open_project() {
    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new("", app_state.path(), Arc::new(MemorySecretStore::default()));

    let mut permissions = get_permissions_settings(&state).unwrap();
    permissions.policy.allow_network = true;
    let saved = save_permissions_settings(&state, permissions).unwrap();

    assert!(saved.policy.allow_network);
    assert!(
        get_permissions_settings(&state)
            .unwrap()
            .policy
            .allow_network
    );
    assert!(app_state.path().join("permissions_settings.json").is_file());
}
