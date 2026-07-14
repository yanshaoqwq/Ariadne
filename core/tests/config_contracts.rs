use std::fs;

use ariadne::config::{
    ConfigStore, MemorySecretStore, ModelConfig, ProjectConfig, ProjectCredentialScope,
    ProviderConfig, SecretRef, SecretValue, AUTO_MODE_CONFIG_FILE, PERMISSIONS_CONFIG_FILE,
    RAG_CONFIG_FILE,
};
use ariadne::contracts::{ProviderCapability, ProviderType};

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
fn openai_compatible_provider_supports_base_url_without_project_secret_ref() {
    let mut config = ProjectConfig::default();
    config.providers.providers.push(ProviderConfig {
        provider_id: "ollama".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Ollama".to_owned(),
        enabled: true,
        base_url: Some("http://127.0.0.1:11434/v1".to_owned()),
        api_key: None,
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

#[test]
fn retrieval_defaults_do_not_enable_remote_or_fake_vector_features() {
    let config = ProjectConfig::default();

    assert!(!config.rag.vector_store.enabled);
    assert!(!config.rag.reranker_enabled);
    assert_eq!(config.rag.vector_store.collection, "ariadne_chunks");
    assert_eq!(config.rag.vector_store.vector_dimensions, 1536);
}

#[test]
fn legacy_rag_yaml_missing_new_fields_migrates_to_full_text_only() {
    let temp = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp.path());
    store.load_or_create().unwrap();
    fs::write(
        store.config_dir().join(RAG_CONFIG_FILE),
        r#"schema_version: 1
vector_store:
  backend: qdrant_sidecar
  sidecar:
    host: 127.0.0.1
    port: 6333
    data_dir: .indexes/qdrant
full_text_store:
  backend: tantivy
  index_dir: .indexes/tantivy
chunk_size_chars: 2000
chunk_overlap_chars: 200
"#,
    )
    .unwrap();

    let migrated = store.load_or_create().unwrap();

    assert!(!migrated.rag.vector_store.enabled);
    assert!(!migrated.rag.reranker_enabled);
    assert_eq!(migrated.rag.vector_store.collection, "ariadne_chunks");
    assert_eq!(migrated.rag.vector_store.vector_dimensions, 1536);
    assert_eq!(migrated.rag.vector_store.sidecar.binary_path, "qdrant");
    assert_eq!(migrated.rag.vector_store.sidecar.startup_timeout_ms, 10_000);
}

#[test]
fn local_provider_requires_explicit_base_url() {
    let mut config = ProjectConfig::default();
    config.providers.providers.push(ProviderConfig {
        provider_id: "local".to_owned(),
        provider_type: ProviderType::Local,
        display_name: "Local".to_owned(),
        enabled: true,
        base_url: None,
        api_key: None,
        models: Vec::new(),
    });

    let error = config.validate().unwrap_err().to_string();

    assert!(error.contains("local providers require base_url"));
}

#[test]
fn external_qdrant_does_not_require_sidecar_data_directory() {
    let mut config = ProjectConfig::default();
    config.rag.vector_store.enabled = true;
    config.rag.vector_store.backend = ariadne::config::VectorStoreBackend::ExternalQdrant;
    config.rag.vector_store.sidecar.data_dir.clear();

    assert!(config.rag.validate().is_ok());
}

#[test]
fn project_config_rejects_untrusted_provider_secret_ref() {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp_dir.path());
    let mut config = store.load_or_create().unwrap();
    config.providers.providers.push(ProviderConfig {
        provider_id: "attacker".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Attacker".to_owned(),
        enabled: true,
        base_url: Some("https://attacker.invalid/v1".to_owned()),
        api_key: Some(SecretRef::new("global-secret-chosen-by-project")),
        models: Vec::new(),
    });

    let error = store.save(&config).unwrap_err().to_string();
    assert!(error.contains("untrusted project SecretRef"));
}

#[test]
fn credential_rebind_load_strips_all_untrusted_provider_secret_refs() {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(temp_dir.path());
    let mut config = store.load_or_create().unwrap();
    config.providers.providers = ["openai", "anthropic"]
        .into_iter()
        .map(|provider_id| ProviderConfig {
            provider_id: provider_id.to_owned(),
            provider_type: if provider_id == "openai" {
                ProviderType::OpenAi
            } else {
                ProviderType::Anthropic
            },
            display_name: provider_id.to_owned(),
            enabled: true,
            base_url: None,
            api_key: Some(SecretRef::new(format!("legacy-{provider_id}-secret"))),
            models: Vec::new(),
        })
        .collect();
    config.providers.default_llm_provider_id = Some("openai".to_owned());
    let raw = yaml_serde::to_string(&yaml_serde::to_value(&config.providers).unwrap()).unwrap();
    fs::write(store.config_dir().join("providers.yaml"), raw).unwrap();

    let sanitized = store.load_or_create_for_credential_rebind().unwrap();
    assert!(sanitized
        .providers
        .providers
        .iter()
        .all(|provider| provider.api_key.is_none()));
    store.save(&sanitized).unwrap();
    assert!(store.load().is_ok());
}

#[test]
fn project_credential_scope_isolates_projects_and_provider_ids() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();
    let scope_a = ProjectCredentialScope::new(project_a.path(), &secrets).unwrap();
    let scope_b = ProjectCredentialScope::new(project_b.path(), &secrets).unwrap();

    scope_a
        .set_provider_secret("openai", SecretValue::new("sk-project-a"))
        .unwrap();
    assert_eq!(
        scope_a
            .get_provider_secret("openai")
            .unwrap()
            .unwrap()
            .expose_secret(),
        "sk-project-a"
    );
    assert!(scope_a.get_provider_secret("anthropic").unwrap().is_none());
    assert!(scope_b.get_provider_secret("openai").unwrap().is_none());

    scope_a.delete_provider_secret("openai").unwrap();
    assert!(scope_a.get_provider_secret("openai").unwrap().is_none());
}

/// D4：本地密钥文件必须 atomic_write，避免直接覆盖写半文件。
#[test]
fn d4_local_secret_store_uses_atomic_write() {
    use ariadne::config::{LocalFileSecretStore, SecretStore, SecretValue};
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("secrets.json");
    let store =
        LocalFileSecretStore::with_master_password(&path, SecretValue::new("test-master-password"))
            .unwrap();
    store
        .set_secret("providers.a", SecretValue::new("sk-first"))
        .unwrap();
    assert!(path.is_file());
    let ino1 = fs::metadata(&path).unwrap().ino();
    store
        .set_secret("providers.a", SecretValue::new("sk-second"))
        .unwrap();
    let ino2 = fs::metadata(&path).unwrap().ino();
    assert_ne!(ino1, ino2, "secret file should be replaced via rename");
    let got = store
        .get_secret("providers.a")
        .unwrap()
        .expect("secret present");
    assert_eq!(got.expose_secret(), "sk-second");
}

/// D4-a：ConfigStore::save 不得再使用固定 `.stage-save`；经 atomic_commit 唯一 stage。
#[test]
fn d4a_config_store_save_does_not_use_fixed_stage_save_dir() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let app_state = temp.path().join("app-state");
    fs::create_dir_all(project.join(".config")).unwrap();
    fs::create_dir_all(&app_state).unwrap();
    let store = ConfigStore::with_app_state(&project, &app_state);
    let mut config = store.load_or_create().unwrap();
    config.app.project_name = "d4a-unique-stage".to_owned();
    store.save(&config).unwrap();

    // Fixed residual path must not remain after successful save.
    let fixed = store.config_dir().join(".stage-save");
    assert!(
        !fixed.exists(),
        "fixed .stage-save dir must not be used by ConfigStore::save"
    );
    // No orphaned .atomic-stage-* dirs after successful commit either.
    let orphans: Vec<_> = fs::read_dir(store.config_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".atomic-stage-")
        })
        .collect();
    assert!(
        orphans.is_empty(),
        "successful save must clear unique stage dirs: {orphans:?}"
    );
    let reloaded = store.load().unwrap();
    assert_eq!(reloaded.app.project_name, "d4a-unique-stage");

    // Source contract: save must call commit_files, not fixed stage-save write loop.
    let src = include_str!("../src/config/store.rs");
    assert!(
        src.contains("atomic_commit::commit_files"),
        "ConfigStore::save must use atomic_commit::commit_files"
    );
    assert!(
        !src.contains("join(\".stage-save\")"),
        "ConfigStore::save must not hardcode .stage-save"
    );
    // Authority journal must not live as project-owned executable recovery file.
    assert!(!store
        .config_dir()
        .join("atomic-commit.journal.json")
        .exists());
}

/// D4-a：并发 ConfigStore::save 同一项目不得留下损坏/混合配置；
/// 多文件（app + workflow + auto_mode）整组属于单一 writer 世代。
#[test]
fn d4a_config_store_concurrent_saves_leave_readable_consistent_config() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let app_state = temp.path().join("app-state");
    fs::create_dir_all(project.join(".config")).unwrap();
    fs::create_dir_all(&app_state).unwrap();
    let store = ConfigStore::with_app_state(&project, &app_state);
    let baseline = store.load_or_create().unwrap();
    store.save(&baseline).unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for (name, locale, docs_dir, budget) in [
        ("writer-A", "zh-CN-A", "docs-A", 1.11_f64),
        ("writer-B", "en-US-B", "docs-B", 2.22_f64),
    ] {
        let barrier = Arc::clone(&barrier);
        let project = project.clone();
        let app_state = app_state.clone();
        let name = name.to_owned();
        let locale = locale.to_owned();
        let docs_dir = docs_dir.to_owned();
        handles.push(thread::spawn(move || {
            let store = ConfigStore::with_app_state(&project, &app_state);
            let mut config = store.load().unwrap();
            config.app.project_name = name.clone();
            config.app.locale = locale;
            config.app.documents_dir = docs_dir;
            config.workflow.max_loop_iterations = if name.ends_with('A') { 11 } else { 22 };
            config.workflow.max_tool_rounds = if name.ends_with('A') { 3 } else { 7 };
            config.auto_mode.enabled_by_default = name.ends_with('A');
            config.auto_mode.preauthorized_budget_usd = Some(budget);
            barrier.wait();
            store.save(&config).map(|_| name).map_err(|e| e.to_string())
        }));
    }
    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("thread"))
        .collect();
    // Exclusive app-state writer lock serializes whole multi-file payloads.
    assert!(
        results.iter().all(|r| r.is_ok()),
        "exclusive multi-file commit lock should serialize both saves: {results:?}"
    );
    let reloaded = ConfigStore::with_app_state(&project, &app_state)
        .load()
        .unwrap();
    let winner = reloaded.app.project_name.clone();
    assert!(
        winner == "writer-A" || winner == "writer-B",
        "final project_name must be one complete writer payload, got {winner:?}"
    );
    if winner == "writer-A" {
        assert_eq!(reloaded.app.locale, "zh-CN-A");
        assert_eq!(reloaded.app.documents_dir, "docs-A");
        assert_eq!(reloaded.workflow.max_loop_iterations, 11);
        assert_eq!(reloaded.workflow.max_tool_rounds, 3);
        assert!(reloaded.auto_mode.enabled_by_default);
        assert_eq!(reloaded.auto_mode.preauthorized_budget_usd, Some(1.11));
    } else {
        assert_eq!(reloaded.app.locale, "en-US-B");
        assert_eq!(reloaded.app.documents_dir, "docs-B");
        assert_eq!(reloaded.workflow.max_loop_iterations, 22);
        assert_eq!(reloaded.workflow.max_tool_rounds, 7);
        assert!(!reloaded.auto_mode.enabled_by_default);
        assert_eq!(reloaded.auto_mode.preauthorized_budget_usd, Some(2.22));
    }
    let cfg = project.join(".config");
    assert!(!cfg.join(".stage-save").exists());
    assert!(!cfg.join("atomic-commit.journal.json").exists());
    assert!(!ariadne::config::atomic_commit::has_pending_journal(
        &project, &app_state
    ));
}
