use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use ariadne::commands::{
    create_checkpoint_impl, fetch_provider_models, fetch_provider_models_impl,
    get_app_settings_impl, get_automation_settings_impl, get_backend_diagnostics,
    get_budget_status_impl, get_display_name_language_pack_template, get_document_content_impl,
    get_document_tree_impl, get_git_history_impl, get_git_settings_impl,
    get_node_preset_settings_impl, get_permissions_settings_impl, get_provider_config_impl,
    get_rag_settings_impl, get_template_repository_settings_impl, get_workflow_settings_impl,
    load_workflow_graph_impl, pack_workflow_selection_impl, project_ai_chat, project_ai_chat_impl,
    resolve_confirmation_impl, resolve_project_references, run_workflow, run_workflow_impl,
    save_app_settings_impl, save_automation_settings_impl, save_document_content_impl,
    save_git_settings_impl, save_node_preset_settings_impl, save_permissions_settings_impl,
    save_provider_key_impl, save_provider_settings_impl, save_rag_settings_impl,
    save_template_repository_settings_impl, save_workflow_graph_impl, save_workflow_settings_impl,
    update_budget_config_impl, validate_display_name_language_pack, AppSettings, AriadneAppState,
    AutomationSettings, CanvasEdge, CanvasNode, ConfirmationAutoModePolicy, ConfirmationDecision,
    ConfirmationNormalPolicy, ConfirmationPolicySetting, GitSettings, NodePresetSettings,
    PermissionsSettings, ProjectAiChatMessage, ProjectAiChatRole, ProjectAiRequest,
    ProviderSettingsUpdate, RagSettings, ResolveConfirmationRequest, TemplateRepositorySettings,
    WorkflowGraphData, WorkflowSettings,
};
use ariadne::config::{ConfigStore, MemorySecretStore, ModelConfig, SecretStore};
use ariadne::contracts::{
    NodeId, PermissionPolicy, PortValue, ProviderCapability, ProviderType, RunId, RunStatus,
    WorkflowDefinition, WorkflowEdgeKind, WorkflowId,
};
use ariadne::diagnostics::DiagnosticStatus;
use ariadne::frontend::{ConfirmationLogEntry, ConfirmationLogState, FileConfirmationLogStore};
use ariadne::workflow::{
    RuntimeConfirmation, RuntimeConfirmationState, SqliteWorkflowRuntimeStore, WorkflowRunState,
    WorkflowRuntime, WorkflowRuntimeStore,
};
use serde_json::{json, Value};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn wait_for_terminal_workflow_state(
    store: &SqliteWorkflowRuntimeStore,
    workflow_id: &WorkflowId,
    run_id: &RunId,
) -> WorkflowRunState {
    let mut last = None;
    for _ in 0..50 {
        last = store.load_state(workflow_id, run_id).unwrap();
        if last
            .as_ref()
            .is_some_and(|state| state.status.is_terminal())
        {
            return last.unwrap();
        }
        thread::sleep(Duration::from_millis(20));
    }
    last.expect("workflow state should be persisted by background worker")
}

#[test]
fn document_commands_read_tree_and_round_trip_content() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("documents")).unwrap();

    save_document_content_impl(
        temp.path(),
        "documents/chapter.md".to_owned(),
        "正文".to_owned(),
    )
    .unwrap();
    let tree = get_document_tree_impl(temp.path()).unwrap();
    let content =
        get_document_content_impl(temp.path(), Some("documents/chapter.md".to_owned()), None)
            .unwrap();

    assert_eq!(content, "正文");
    assert!(format!("{tree:?}").contains("chapter.md"));
}

#[test]
fn app_state_root_can_be_separated_from_project_root_env() {
    let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    std::env::set_var("ARIADNE_PROJECT_ROOT", project.path());
    std::env::set_var("ARIADNE_APP_STATE_ROOT", app_state.path());

    let resolved_app_state = ariadne::commands::default_app_state_root();
    let resolved_project = ariadne::commands::default_project_root();

    std::env::remove_var("ARIADNE_PROJECT_ROOT");
    std::env::remove_var("ARIADNE_APP_STATE_ROOT");

    assert_eq!(resolved_project, project.path());
    assert_eq!(resolved_app_state, app_state.path());
}

#[test]
fn app_state_rejects_missing_or_uninitialized_project_root() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new(
        project.path().to_path_buf(),
        app_state.path().to_path_buf(),
        Arc::new(MemorySecretStore::default()),
    );

    let missing = project.path().join("missing");
    let missing_error = state.set_project_root(&missing).unwrap_err();
    assert!(missing_error.contains("does not exist"));

    let uninitialized = project.path().join("plain");
    std::fs::create_dir_all(&uninitialized).unwrap();
    let uninitialized_error = state.set_project_root(&uninitialized).unwrap_err();
    assert!(uninitialized_error.contains("not initialized"));

    std::fs::create_dir_all(uninitialized.join(".config")).unwrap();
    state.set_project_root(&uninitialized).unwrap();
    assert_eq!(state.project_root().unwrap(), uninitialized);
}

#[test]
fn command_impls_reject_missing_project_root_without_creating_it() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-project");

    let error = get_app_settings_impl(&missing).unwrap_err();
    assert!(error.contains("project root does not exist"));
    assert!(!missing.exists());
}

#[test]
fn display_name_language_pack_template_supports_arbitrary_language_codes() {
    let template = get_display_name_language_pack_template(Some("ZH_Hant".to_owned())).unwrap();

    assert_eq!(template.target_language, "zh-hant");
    assert_eq!(template.base_language, "zh");
    assert_eq!(template.fallback_language, "zh");
    assert_eq!(template.output_file_name, "display_name.zh-hant.json");
    assert_eq!(template.source_file_name, "display_name.json");
    assert!(template.entries.contains_key("ui.settings.misc.language"));
    assert!(template
        .instructions
        .iter()
        .any(|item| item.contains("Keep every JSON key unchanged")));
}

#[test]
fn display_name_language_pack_validation_reports_coverage() {
    let template = get_display_name_language_pack_template(Some("fr".to_owned())).unwrap();
    let mut keys = template.entries.keys().cloned();
    let translated_key = keys.next().unwrap();
    let empty_key = keys.next().unwrap();
    let mut overlay = BTreeMap::new();
    overlay.insert("_comment".to_owned(), "metadata is allowed".to_owned());
    overlay.insert(translated_key.clone(), "traduit".to_owned());
    overlay.insert(empty_key.clone(), "  ".to_owned());
    overlay.insert("ui.unknown".to_owned(), "extra".to_owned());

    let report = validate_display_name_language_pack(Some("FR".to_owned()), overlay).unwrap();

    assert_eq!(report.target_language, "fr");
    assert_eq!(report.output_file_name, "display_name.fr.json");
    assert_eq!(report.total_keys, template.entries.len());
    assert_eq!(report.translated_keys, 1);
    assert!(report.empty_keys.contains(&empty_key));
    assert!(report.extra_keys.contains(&"ui.unknown".to_owned()));
    assert_eq!(report.missing_keys.len(), template.entries.len() - 2);
    assert!(!report.complete);
}

#[test]
fn workflow_graph_commands_save_and_load_canvas_shape() {
    let temp = tempfile::tempdir().unwrap();
    let graph = WorkflowGraphData {
        workflow_id: "draft-flow".to_owned(),
        name: "Draft Flow".to_owned(),
        nodes: vec![CanvasNode {
            id: "writer".to_owned(),
            r#type: "writer".to_owned(),
            label: Some("Writer".to_owned()),
            data: json!({ "prompt_template": "writer.default" }),
            position: json!({ "x": 10.0, "y": 20.0 }),
        }],
        edges: Vec::new(),
        metadata: Value::Null,
    };

    save_workflow_graph_impl(temp.path(), graph).unwrap();
    let loaded = load_workflow_graph_impl(temp.path(), Some("draft-flow".to_owned())).unwrap();

    assert_eq!(loaded.workflow_id, "draft-flow");
    assert_eq!(loaded.nodes[0].id, "writer");
    assert_eq!(loaded.nodes[0].data["prompt_template"], "writer.default");
}

#[cfg(unix)]
#[test]
fn workflow_graph_save_rejects_symlink_escape_from_workflows_root() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("workflows")).unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join("workflows").join("escape"))
        .unwrap();
    let graph = WorkflowGraphData {
        workflow_id: "escape/owned".to_owned(),
        name: "Escaped".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
    };

    let error = save_workflow_graph_impl(temp.path(), graph).unwrap_err();

    assert!(error.contains("outside allowed root"));
    assert!(!outside.path().join("owned.json").exists());
}

#[test]
fn pack_workflow_selection_command_persists_subworkflow_graph() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "pack-flow".to_owned(),
            name: "Pack Flow".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "source".to_owned(),
                    r#type: "document_read".to_owned(),
                    label: None,
                    data: Value::Null,
                    position: json!({ "x": 0.0, "y": 0.0 }),
                },
                CanvasNode {
                    id: "writer".to_owned(),
                    r#type: "writer".to_owned(),
                    label: None,
                    data: Value::Null,
                    position: json!({ "x": 100.0, "y": 0.0 }),
                },
                CanvasNode {
                    id: "reviewer".to_owned(),
                    r#type: "critic".to_owned(),
                    label: None,
                    data: Value::Null,
                    position: json!({ "x": 200.0, "y": 0.0 }),
                },
                CanvasNode {
                    id: "sink".to_owned(),
                    r#type: "export".to_owned(),
                    label: None,
                    data: Value::Null,
                    position: json!({ "x": 300.0, "y": 0.0 }),
                },
            ],
            edges: vec![
                CanvasEdge {
                    id: "source-writer".to_owned(),
                    source: "source".to_owned(),
                    target: "writer".to_owned(),
                    source_handle: "out-draft".to_owned(),
                    target_handle: "in-draft".to_owned(),
                    kind: WorkflowEdgeKind::Data,
                    label: Some("draft".to_owned()),
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "writer-reviewer".to_owned(),
                    source: "writer".to_owned(),
                    target: "reviewer".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "reviewer-sink".to_owned(),
                    source: "reviewer".to_owned(),
                    target: "sink".to_owned(),
                    source_handle: "out-review".to_owned(),
                    target_handle: "in-review".to_owned(),
                    kind: WorkflowEdgeKind::Data,
                    label: Some("review".to_owned()),
                    data: Value::Null,
                },
            ],
            metadata: Value::Null,
        },
    )
    .unwrap();

    let report = pack_workflow_selection_impl(
        temp.path(),
        "pack-flow".to_owned(),
        vec!["writer".to_owned(), "reviewer".to_owned()],
        Some("sub-review".to_owned()),
        Some("Review Subflow".to_owned()),
    )
    .unwrap();
    let loaded = load_workflow_graph_impl(temp.path(), Some("pack-flow".to_owned())).unwrap();

    assert_eq!(report.subworkflow_node_id, NodeId::from("sub-review"));
    assert_eq!(loaded.nodes.len(), 3);
    assert!(loaded.nodes.iter().any(|node| {
        node.id == "sub-review"
            && node.r#type == "subworkflow"
            && node.data.get("embedded_workflow").is_some()
    }));
    assert!(loaded
        .edges
        .iter()
        .any(|edge| edge.source == "source" && edge.target == "sub-review"));
    assert!(loaded
        .edges
        .iter()
        .any(|edge| edge.source == "sub-review" && edge.target == "sink"));
}

#[test]
fn run_workflow_executes_document_nodes_with_real_document_service() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("documents")).unwrap();
    std::fs::write(temp.path().join("documents/source.md"), "正文").unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "doc-flow".to_owned(),
            name: "Doc Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "read".to_owned(),
                r#type: "document_read".to_owned(),
                label: None,
                data: json!({
                    "path": temp.path().join("documents/source.md"),
                    "include_content": true
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        },
    )
    .unwrap();

    let run = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "doc-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap();

    assert_eq!(run.status, "succeeded");
}

#[test]
fn run_workflow_command_starts_background_run() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "async-run".to_owned(),
            name: "Async Run".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-main".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Start".to_owned()),
                data: json!({
                    "work_dir": "main"
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        },
    )
    .unwrap();

    let state = AriadneAppState::new(
        temp.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );
    let run = run_workflow(
        &state,
        "async-run".to_owned(),
        Some("start-main".to_owned()),
    )
    .unwrap();

    assert_eq!(run.status, "queued");
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let run_id = RunId::from(run.run_id);
    let state = wait_for_terminal_workflow_state(&store, &WorkflowId::from("async-run"), &run_id);

    assert_eq!(state.status, RunStatus::Succeeded);
    assert!(state.nodes.contains_key(&NodeId::from("start-main")));
}

#[test]
fn run_workflow_from_start_node_executes_only_that_branch() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("main/documents")).unwrap();
    std::fs::create_dir_all(temp.path().join("extra/documents")).unwrap();
    std::fs::write(temp.path().join("main/documents/source.md"), "正篇").unwrap();
    std::fs::write(temp.path().join("extra/documents/source.md"), "番外").unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "multi-start".to_owned(),
            name: "Multi Start".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "start-main".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("Main".to_owned()),
                    data: json!({
                        "name": "正篇",
                        "work_dir": "main",
                        "expose_as_tool": true
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "read-main".to_owned(),
                    r#type: "document_read".to_owned(),
                    label: None,
                    data: json!({
                        "path": "documents/source.md",
                        "include_content": true
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "start-extra".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("Extra".to_owned()),
                    data: json!({
                        "name": "番外",
                        "work_dir": "extra",
                        "expose_as_tool": false
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "read-extra".to_owned(),
                    r#type: "document_read".to_owned(),
                    label: None,
                    data: json!({
                        "path": "documents/source.md",
                        "include_content": true
                    }),
                    position: Value::Null,
                },
            ],
            edges: vec![
                CanvasEdge {
                    id: "main-edge".to_owned(),
                    source: "start-main".to_owned(),
                    target: "read-main".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "extra-edge".to_owned(),
                    source: "start-extra".to_owned(),
                    target: "read-extra".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
            ],
            metadata: Value::Null,
        },
    )
    .unwrap();

    let main = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "multi-start".to_owned(),
            start_node_id: Some("start-main".to_owned()),
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap();
    let extra = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "multi-start".to_owned(),
            start_node_id: Some("start-extra".to_owned()),
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap();

    assert_eq!(main.status, "succeeded");
    assert_eq!(extra.status, "succeeded");
    assert_ne!(main.run_id, extra.run_id);

    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let main_state = store
        .load_state(&WorkflowId::from("multi-start"), &RunId::from(main.run_id))
        .unwrap()
        .unwrap();
    let extra_state = store
        .load_state(&WorkflowId::from("multi-start"), &RunId::from(extra.run_id))
        .unwrap()
        .unwrap();

    assert!(main_state.nodes.contains_key(&NodeId::from("start-main")));
    assert!(main_state.nodes.contains_key(&NodeId::from("read-main")));
    assert_eq!(
        main_state.nodes[&NodeId::from("read-main")].outputs["content"],
        PortValue::inline("正篇")
    );
    assert!(!main_state.nodes.contains_key(&NodeId::from("start-extra")));
    assert!(!main_state.nodes.contains_key(&NodeId::from("read-extra")));
    assert!(extra_state.nodes.contains_key(&NodeId::from("start-extra")));
    assert!(extra_state.nodes.contains_key(&NodeId::from("read-extra")));
    assert_eq!(
        extra_state.nodes[&NodeId::from("read-extra")].outputs["content"],
        PortValue::inline("番外")
    );
    assert!(!extra_state.nodes.contains_key(&NodeId::from("start-main")));
    assert!(!extra_state.nodes.contains_key(&NodeId::from("read-main")));
}

#[test]
fn run_workflow_from_start_node_injects_tool_arguments_as_outputs() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "tool-start".to_owned(),
            name: "Tool Start".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "start-main".to_owned(),
                    r#type: "start".to_owned(),
                    label: Some("Start Main".to_owned()),
                    data: json!({
                        "expose_as_tool": true,
                        "input_schema": {
                            "type": "object",
                            "properties": {
                                "topic": { "type": "string" }
                            },
                            "required": ["topic"],
                            "additionalProperties": false
                        }
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "check-topic".to_owned(),
                    r#type: "condition".to_owned(),
                    label: None,
                    data: json!({
                        "input_alias": "topic",
                        "operator": "equals",
                        "expected": "长夜行"
                    }),
                    position: Value::Null,
                },
            ],
            edges: vec![
                CanvasEdge {
                    id: "start-to-check".to_owned(),
                    source: "start-main".to_owned(),
                    target: "check-topic".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "topic-to-check".to_owned(),
                    source: "start-main".to_owned(),
                    target: "check-topic".to_owned(),
                    source_handle: "topic".to_owned(),
                    target_handle: "input".to_owned(),
                    kind: WorkflowEdgeKind::Data,
                    label: Some("topic".to_owned()),
                    data: Value::Null,
                },
            ],
            metadata: Value::Null,
        },
    )
    .unwrap();

    let mut initial_inputs = BTreeMap::new();
    initial_inputs.insert("topic".to_owned(), json!("长夜行"));
    let run = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "tool-start".to_owned(),
            start_node_id: Some("start-main".to_owned()),
            initial_inputs,
        },
    )
    .unwrap();

    assert_eq!(run.status, "succeeded");
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let state = store
        .load_state(&WorkflowId::from("tool-start"), &RunId::from(run.run_id))
        .unwrap()
        .unwrap();
    let check = state.nodes.get(&NodeId::from("check-topic")).unwrap();
    assert_eq!(check.outputs.get("passed"), Some(&PortValue::inline(true)));
}

#[test]
fn run_workflow_start_node_id_must_reference_start_node() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "bad-start".to_owned(),
            name: "Bad Start".to_owned(),
            nodes: vec![CanvasNode {
                id: "read".to_owned(),
                r#type: "document_read".to_owned(),
                label: None,
                data: Value::Null,
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        },
    )
    .unwrap();

    let error = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "bad-start".to_owned(),
            start_node_id: Some("read".to_owned()),
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap_err();

    assert!(error.contains("must reference a start node"));
}

#[test]
fn run_workflow_llm_node_requires_configured_provider_instead_of_noop() {
    let temp = tempfile::tempdir().unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "llm-flow".to_owned(),
            name: "LLM Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "ask".to_owned(),
                r#type: "llm".to_owned(),
                label: None,
                data: json!({
                    "provider_id": "openai",
                    "model_id": "gpt-test",
                    "prompt_alias": "prompt"
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        },
    )
    .unwrap();

    let error = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ariadne::commands::RunWorkflowRequest {
            workflow_id: "llm-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
        },
    )
    .unwrap_err();

    assert!(error.contains("LLM provider"));
}

#[test]
fn budget_and_provider_commands_do_not_return_secret_values() {
    let temp = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();
    update_budget_config_impl(temp.path(), 25.0, 3.5).unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: "gpt-test".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: true,
            make_default_reranker: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "openai".to_owned(),
        "sk-secret".to_owned(),
    )
    .unwrap();

    let budget = get_budget_status_impl(temp.path()).unwrap();
    let provider = get_provider_config_impl(temp.path(), &secrets).unwrap();

    assert_eq!(budget.budget_usd, 25.0);
    assert_eq!(budget.preauthorized_usd, 3.5);
    assert!(provider.has_openai_key);
    assert_eq!(provider.default_llm_provider_id.as_deref(), Some("openai"));
    assert_eq!(
        provider.default_embedding_provider_id.as_deref(),
        Some("openai")
    );
    assert_eq!(provider.providers[0].provider, "openai");
    assert_eq!(provider.providers[0].models[0].model_id, "gpt-test");
    let config = ConfigStore::new(temp.path()).load_or_create().unwrap();
    let key_id = config.providers.providers[0]
        .api_key
        .as_ref()
        .unwrap()
        .key_id
        .clone();
    assert!(key_id.starts_with("project."));
    assert!(key_id.ends_with(".provider.openai"));
    assert_eq!(
        secrets
            .get_secret(&key_id)
            .unwrap()
            .unwrap()
            .expose_secret(),
        "sk-secret"
    );
    assert!(secrets.get_secret("provider.openai").unwrap().is_none());
}

#[test]
fn provider_key_status_is_namespaced_by_project_root() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();

    save_provider_key_impl(
        project_a.path(),
        &secrets,
        "openai".to_owned(),
        "sk-project-a".to_owned(),
    )
    .unwrap();

    let status_a = get_provider_config_impl(project_a.path(), &secrets).unwrap();
    let status_b = get_provider_config_impl(project_b.path(), &secrets).unwrap();

    assert!(status_a.has_openai_key);
    assert!(!status_b.has_openai_key);
}

#[test]
fn provider_model_fetch_returns_configured_and_embedding_models() {
    let temp = tempfile::tempdir().unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: "gpt-test".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(4096),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: true,
            make_default_reranker: false,
        },
    )
    .unwrap();

    let models = fetch_provider_models_impl(temp.path(), Some("openai".to_owned())).unwrap();

    assert_eq!(models.provider_id, "openai");
    assert!(models
        .models
        .iter()
        .any(|model| model.model_id == "gpt-test" && model.capability == ProviderCapability::Llm));
    assert!(models
        .models
        .iter()
        .any(|model| model.model_id == "text-embedding-3-small"
            && model.capability == ProviderCapability::Embedding));
}

#[test]
fn provider_model_fetch_calls_remote_models_endpoint() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("GET /models "));
        assert!(request.contains("authorization: Bearer local-key"));
        let response_body = r#"{
          "data": [
            {"id": "chat-alpha"},
            {"id": "text-embedding-3-small"}
          ]
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_models".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Models".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "chat-alpha".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(8192),
                input_cost_per_million_tokens: Some(0.25),
                output_cost_per_million_tokens: Some(0.5),
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        secrets.as_ref(),
        "local_models".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let state = AriadneAppState::new(
        temp.path().to_path_buf(),
        temp.path().join("app-state"),
        Arc::clone(&secrets),
    );
    let models = fetch_provider_models(&state, Some("local_models".to_owned())).unwrap();
    server.join().unwrap();

    assert_eq!(models.provider_id, "local_models");
    let chat_model = models
        .models
        .iter()
        .find(|model| model.model_id == "chat-alpha")
        .unwrap();
    assert_eq!(chat_model.capability, ProviderCapability::Llm);
    assert_eq!(chat_model.max_context_tokens, Some(8192));
    assert_eq!(chat_model.input_cost_per_million_tokens, Some(0.25));
    assert!(models
        .models
        .iter()
        .any(|model| model.model_id == "text-embedding-3-small"
            && model.capability == ProviderCapability::Embedding));
}

#[test]
fn provider_model_fetch_rejects_oversized_streaming_response() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("GET /models "));
        let response_header =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
        stream.write_all(response_header.as_bytes()).unwrap();
        stream.write_all(&vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
    });

    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_models".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Models".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "chat-alpha".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: Some(8192),
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
        },
    )
    .unwrap();
    let state = AriadneAppState::new(
        temp.path().to_path_buf(),
        temp.path().join("app-state"),
        Arc::clone(&secrets),
    );

    let error = fetch_provider_models(&state, Some("local_models".to_owned())).unwrap_err();
    server.join().unwrap();

    assert!(error.contains("model list response exceeds"));
}

#[test]
fn provider_settings_reject_non_http_base_url() {
    let temp = tempfile::tempdir().unwrap();
    let error = save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_file".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local File".to_owned(),
            enabled: true,
            base_url: Some("file:///tmp/provider".to_owned()),
            models: vec![ModelConfig {
                model_id: "local".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
        },
    )
    .unwrap_err();

    assert!(error.contains("provider base_url must use http or https"));
}

#[test]
fn node_preset_settings_reject_unknown_configured_model() {
    let temp = tempfile::tempdir().unwrap();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: None,
            models: vec![ModelConfig {
                model_id: "gpt-configured".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
        },
    )
    .unwrap();
    let mut settings = NodePresetSettings::default();
    settings.default_model_id = "missing-model".to_owned();
    for preset in &mut settings.presets {
        preset.model_id = "gpt-configured".to_owned();
    }

    let error = save_node_preset_settings_impl(temp.path(), settings).unwrap_err();

    assert!(error.contains("default_model_id references a model that is not configured"));
}

#[test]
fn backend_diagnostics_reports_provider_configuration_gaps() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let state = AriadneAppState::new(
        project.path(),
        app_state.path(),
        Arc::new(MemorySecretStore::default()),
    );

    let report = get_backend_diagnostics(&state).unwrap();

    assert_ne!(report.status, DiagnosticStatus::Healthy);
    assert!(report
        .items
        .iter()
        .any(|item| item.component == "providers.llm.default"
            && item.status == DiagnosticStatus::Degraded));
    assert!(report
        .items
        .iter()
        .any(|item| item.component == "providers.embedding.default"
            && item.status == DiagnosticStatus::Degraded));
}

#[test]
fn node_preset_settings_are_per_node_type() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = NodePresetSettings::default();
    assert!(settings
        .presets
        .iter()
        .any(|preset| preset.node_type == "writer"));

    let writer = settings
        .presets
        .iter_mut()
        .find(|preset| preset.node_type == "writer")
        .unwrap();
    writer.model_id = "gpt-writer".to_owned();
    writer.timeout_ms = 600_000;
    writer.budget_usd = 0.25;

    save_node_preset_settings_impl(temp.path(), settings).unwrap();
    let loaded = get_node_preset_settings_impl(temp.path()).unwrap();
    let writer = loaded
        .presets
        .iter()
        .find(|preset| preset.node_type == "writer")
        .unwrap();

    assert_eq!(writer.model_id, "gpt-writer");
    assert_eq!(writer.timeout_ms, 600_000);
    assert_eq!(writer.budget_usd, 0.25);
}

#[test]
fn automation_and_permission_settings_round_trip_config_files() {
    let temp = tempfile::tempdir().unwrap();
    update_budget_config_impl(temp.path(), 10.0, 1.0).unwrap();
    let current = get_automation_settings_impl(temp.path()).unwrap();
    save_automation_settings_impl(
        temp.path(),
        AutomationSettings {
            budget: ariadne::commands::BudgetStatus {
                budget_usd: 20.0,
                spent_usd: current.budget.spent_usd,
                preauthorized_usd: 4.0,
                auto_mode_enabled: true,
            },
            confirmation_policies: vec![
                ConfirmationPolicySetting {
                    confirmation_kind: "chapter_write".to_owned(),
                    normal_policy: ConfirmationNormalPolicy::ManualReview,
                    auto_mode_policy: ConfirmationAutoModePolicy::AutoApproval,
                },
                ConfirmationPolicySetting {
                    confirmation_kind: "summary_write".to_owned(),
                    normal_policy: ConfirmationNormalPolicy::AllowByDefault,
                    auto_mode_policy: ConfirmationAutoModePolicy::AllowByDefault,
                },
            ],
        },
    )
    .unwrap();
    let automation = get_automation_settings_impl(temp.path()).unwrap();

    assert_eq!(automation.budget.budget_usd, 20.0);
    assert_eq!(automation.budget.preauthorized_usd, 4.0);
    assert!(automation.budget.auto_mode_enabled);
    assert!(automation
        .confirmation_policies
        .iter()
        .any(|item| item.confirmation_kind == "chapter_write"
            && item.normal_policy == ConfirmationNormalPolicy::ManualReview
            && item.auto_mode_policy == ConfirmationAutoModePolicy::AutoApproval));
    assert!(automation
        .confirmation_policies
        .iter()
        .any(|item| item.confirmation_kind == "summary_write"
            && item.normal_policy == ConfirmationNormalPolicy::AllowByDefault
            && item.auto_mode_policy == ConfirmationAutoModePolicy::AllowByDefault));

    let mut policy = PermissionPolicy::default();
    policy.allow_network = true;
    policy.allow_http_skill = true;
    policy
        .readable_file_roots
        .push(temp.path().join("documents"));
    save_permissions_settings_impl(
        temp.path(),
        PermissionsSettings {
            policy: policy.clone(),
            tool_controls: BTreeMap::from([(
                "project_ai".to_owned(),
                BTreeMap::from([("project-ai-workflow-tools".to_owned(), false)]),
            )]),
        },
    )
    .unwrap();
    let permissions = get_permissions_settings_impl(temp.path()).unwrap();

    assert_eq!(permissions.policy, policy);
    assert_eq!(
        permissions
            .tool_controls
            .get("project_ai")
            .and_then(|scope| scope.get("project-ai-workflow-tools")),
        Some(&false)
    );
    assert!(permissions.tool_controls.contains_key("writer"));
    assert_eq!(
        permissions
            .tool_controls
            .get("writer")
            .and_then(|scope| scope.get("writer-insert-lines")),
        Some(&false)
    );
    assert_eq!(
        permissions
            .tool_controls
            .get("writer")
            .and_then(|scope| scope.get("writer-find")),
        Some(&true)
    );
}

#[test]
fn automation_settings_read_old_policy_code_but_write_dual_policies_only() {
    let temp = tempfile::tempdir().unwrap();
    let settings_path = temp
        .path()
        .join(".config/confirmation_policy_settings.json");
    std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&json!([
            {
                "confirmation_kind": "chapter_write",
                "policy": "auto_approve"
            }
        ]))
        .unwrap(),
    )
    .unwrap();

    let automation = get_automation_settings_impl(temp.path()).unwrap();
    let chapter = automation
        .confirmation_policies
        .iter()
        .find(|item| item.confirmation_kind == "chapter_write")
        .unwrap();
    assert_eq!(
        chapter.normal_policy,
        ConfirmationNormalPolicy::AllowByDefault
    );
    assert_eq!(
        chapter.auto_mode_policy,
        ConfirmationAutoModePolicy::AutoApproval
    );

    save_automation_settings_impl(temp.path(), automation).unwrap();
    let saved = std::fs::read_to_string(settings_path).unwrap();
    assert!(saved.contains("\"normal_policy\""));
    assert!(saved.contains("\"auto_mode_policy\""));
    assert!(!saved.contains("\"policy\""));
}

#[test]
fn module_settings_round_trip_config_files() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let mut app = get_app_settings_impl(temp.path()).unwrap().app;
    app.project_name = "模块设置项目".to_owned();
    app.locale = "zh-CN".to_owned();
    save_app_settings_impl(temp.path(), AppSettings { app }).unwrap();
    assert_eq!(
        get_app_settings_impl(temp.path()).unwrap().app.project_name,
        "模块设置项目"
    );

    let mut rag = get_rag_settings_impl(temp.path()).unwrap().rag;
    rag.chunk_size_chars = 4096;
    rag.chunk_overlap_chars = 256;
    save_rag_settings_impl(temp.path(), RagSettings { rag }).unwrap();
    assert_eq!(
        get_rag_settings_impl(temp.path())
            .unwrap()
            .rag
            .chunk_size_chars,
        4096
    );

    let mut workflow = get_workflow_settings_impl(temp.path()).unwrap().workflow;
    workflow.max_tool_rounds = 12;
    workflow.runtime_autosave_ms = 2500;
    save_workflow_settings_impl(temp.path(), WorkflowSettings { workflow }).unwrap();
    assert_eq!(
        get_workflow_settings_impl(temp.path())
            .unwrap()
            .workflow
            .max_tool_rounds,
        12
    );

    let mut git = get_git_settings_impl(temp.path()).unwrap().git;
    git.track_skills = false;
    git.ignored_paths.push("scratch/".to_owned());
    save_git_settings_impl(temp.path(), GitSettings { git }).unwrap();
    assert!(!get_git_settings_impl(temp.path()).unwrap().git.track_skills);

    save_template_repository_settings_impl(
        temp.path(),
        &TemplateRepositorySettings {
            base_url: "http://127.0.0.1:8080/templates".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        get_template_repository_settings_impl(temp.path())
            .unwrap()
            .base_url,
        "http://127.0.0.1:8080/templates"
    );
}

#[test]
fn git_commands_create_checkpoint_and_return_history() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    std::fs::write(temp.path().join("documents").join("chapter.md"), "正文").unwrap();

    let checkpoint = create_checkpoint_impl(temp.path(), "章节完成".to_owned()).unwrap();
    let history = get_git_history_impl(temp.path()).unwrap();

    assert_eq!(checkpoint.message, "章节完成");
    assert_eq!(history[0].summary, "章节完成");
}

#[test]
fn git_checkpoint_respects_tracking_and_ignored_settings() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );

    let mut git = get_git_settings_impl(temp.path()).unwrap().git;
    git.track_skills = false;
    git.ignored_paths.push("scratch".to_owned());
    save_git_settings_impl(temp.path(), GitSettings { git }).unwrap();

    std::fs::create_dir_all(temp.path().join("skills")).unwrap();
    std::fs::create_dir_all(temp.path().join("scratch")).unwrap();
    std::fs::write(temp.path().join("documents").join("chapter.md"), "正文").unwrap();
    std::fs::write(temp.path().join("skills").join("skill.md"), "skill").unwrap();
    std::fs::write(temp.path().join("scratch").join("draft.md"), "scratch").unwrap();

    create_checkpoint_impl(temp.path(), "受控存档".to_owned()).unwrap();

    let tree = git_stdout(temp.path(), ["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(tree.contains("documents/chapter.md"));
    assert!(tree.contains(".config/app.yaml"));
    assert!(!tree.contains("skills/skill.md"));
    assert!(!tree.contains("scratch/draft.md"));
}

#[test]
fn project_ai_resolves_references_and_updates_memory_without_llm() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    FileConfirmationLogStore::default_for_project(temp.path())
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-1".to_owned(),
            kind: "chapter_summary".to_owned(),
            node_id: "summarizer".to_owned(),
            timestamp_ms: 1,
            state: ConfirmationLogState::Pending,
            handling_method: "manual".to_owned(),
            summary: "章节总结待确认".to_owned(),
            diff: "- old\n+ new".to_owned(),
        })
        .unwrap();

    let resolved =
        resolve_project_references(temp.path(), &["@确认项/confirm-1".to_owned()]).unwrap();
    let response = project_ai_chat_impl(
        temp.path(),
        &MemorySecretStore::default(),
        ProjectAiRequest {
            message: String::new(),
            chat_history: Vec::new(),
            references: vec!["@确认项/confirm-1".to_owned()],
            workflow_id_to_run: None,
            append_memory: Some("长期偏好：保持第三人称。".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(resolved[0].summary, "章节总结待确认");
    assert!(response.project_memory.contains("第三人称"));
    assert_eq!(response.resolved_references[0].id, "confirm-1");
    assert_eq!(response.answer, "已处理项目记忆或工作流请求。");
}

#[test]
fn project_ai_chat_sends_chat_history_through_llm_provider() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("authorization: Bearer local-key"));
        assert!(request.contains("\"role\":\"system\""));
        assert!(request.contains("上一轮问题"));
        assert!(request.contains("上一轮回答"));
        assert!(request.contains("继续说明"));
        let response_body = r#"{
          "model":"local-chat",
          "choices":[{"message":{"content":"继续回答"},"finish_reason":"stop"}],
          "usage":{"prompt_tokens":16,"completion_tokens":4}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets = MemorySecretStore::default();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "local-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "local_chat".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let response = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "继续说明".to_owned(),
            chat_history: vec![
                ProjectAiChatMessage {
                    role: ProjectAiChatRole::User,
                    content: "上一轮问题".to_owned(),
                },
                ProjectAiChatMessage {
                    role: ProjectAiChatRole::Assistant,
                    content: "上一轮回答".to_owned(),
                },
            ],
            references: Vec::new(),
            workflow_id_to_run: None,
            append_memory: None,
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(response.answer, "继续回答");
    assert_eq!(response.chat_history.len(), 4);
    assert_eq!(response.chat_history[2].content, "继续说明");
    assert_eq!(response.chat_history[3].content, "继续回答");
}

#[test]
fn project_ai_chat_exposes_start_nodes_as_workflow_tools() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "tool-flow".to_owned(),
            name: "Tool Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-draft".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Draft Tool".to_owned()),
                data: json!({
                    "name": "Draft Tool",
                    "work_dir": "draft",
                    "expose_as_tool": true
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        },
    )
    .unwrap();
    save_permissions_settings_impl(
        temp.path(),
        PermissionsSettings {
            policy: PermissionPolicy::default(),
            tool_controls: BTreeMap::from([(
                "project_ai".to_owned(),
                BTreeMap::from([("project-ai-workflow-tools".to_owned(), true)]),
            )]),
        },
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("\"tools\""));
        assert!(request.contains("\"name\":\"draft_tool\""));
        let response_body = r#"{
          "model":"local-chat",
          "choices":[{
            "message":{
              "content":"",
              "tool_calls":[{
                "id":"call-1",
                "type":"function",
                "function":{"name":"draft_tool","arguments":"{}"}
              }]
            },
            "finish_reason":"tool_calls"
          }],
          "usage":{"prompt_tokens":16,"completion_tokens":1}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "local-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        secrets.as_ref(),
        "local_chat".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let app_state = tempfile::tempdir().unwrap();
    let state = AriadneAppState::new(temp.path(), app_state.path(), secrets);
    let response = project_ai_chat(
        &state,
        ProjectAiRequest {
            message: "启动草稿工具".to_owned(),
            chat_history: Vec::new(),
            references: Vec::new(),
            workflow_id_to_run: None,
            append_memory: None,
        },
    )
    .unwrap();
    server.join().unwrap();

    let workflow_run = response.workflow_run.unwrap();
    assert_eq!(workflow_run.status, "queued");
    assert_eq!(response.answer, "ui.project_ai.workflow_tool_started");
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let run_id = RunId::from(workflow_run.run_id);
    let state = wait_for_terminal_workflow_state(&store, &WorkflowId::from("tool-flow"), &run_id);
    assert_eq!(state.status, RunStatus::Succeeded);
    assert!(state.nodes.contains_key(&NodeId::from("start-draft")));
}

#[test]
fn project_ai_chat_respects_disabled_workflow_tool_permission() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "tool-flow".to_owned(),
            name: "Tool Flow".to_owned(),
            nodes: vec![CanvasNode {
                id: "start-draft".to_owned(),
                r#type: "start".to_owned(),
                label: Some("Draft Tool".to_owned()),
                data: json!({
                    "name": "Draft Tool",
                    "work_dir": "draft",
                    "expose_as_tool": true
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
        },
    )
    .unwrap();
    save_permissions_settings_impl(
        temp.path(),
        PermissionsSettings {
            policy: PermissionPolicy::default(),
            tool_controls: BTreeMap::from([(
                "project_ai".to_owned(),
                BTreeMap::from([("project-ai-workflow-tools".to_owned(), false)]),
            )]),
        },
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 16384];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(!request.contains("\"tools\""));
        assert!(!request.contains("\"name\":\"draft_tool\""));
        let response_body = r#"{
          "model":"local-chat",
          "choices":[{
            "message":{"content":"工具已关闭"},
            "finish_reason":"stop"
          }],
          "usage":{"prompt_tokens":16,"completion_tokens":3}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let secrets = MemorySecretStore::default();
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "local_chat".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local Chat".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: "local-chat".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
        },
    )
    .unwrap();
    save_provider_key_impl(
        temp.path(),
        &secrets,
        "local_chat".to_owned(),
        "local-key".to_owned(),
    )
    .unwrap();

    let response = project_ai_chat_impl(
        temp.path(),
        &secrets,
        ProjectAiRequest {
            message: "启动草稿工具".to_owned(),
            chat_history: Vec::new(),
            references: Vec::new(),
            workflow_id_to_run: None,
            append_memory: None,
        },
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(response.answer, "工具已关闭");
    assert!(response.workflow_run.is_none());
}

#[test]
fn resolve_confirmation_command_updates_runtime_and_log_badges() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Confirm Flow".to_owned(),
        nodes: Vec::new(),
        edges: Vec::new(),
        metadata: Value::Null,
    };
    let mut runtime = WorkflowRuntime::new(&workflow, RunId::from("run-1")).unwrap();
    runtime.state.status = RunStatus::Paused;
    runtime.state.pause_reason = Some("pending confirmation items".to_owned());
    runtime.state.confirmations.insert(
        "confirm-1".to_owned(),
        RuntimeConfirmation {
            confirmation_id: "confirm-1".to_owned(),
            node_id: NodeId::from("approval"),
            state: RuntimeConfirmationState::Pending,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: json!({
                "kind": "approval",
                "summary": "待确认输出",
                "diff": "- old\n+ new",
                "reason": "pending",
            }),
        },
    );
    runtime.state.nodes.insert(
        NodeId::from("approval"),
        ariadne::workflow::WorkflowNodeRuntimeState {
            node_id: NodeId::from("approval"),
            status: RunStatus::Paused,
            outputs: Default::default(),
            communication_output: None,
            communication_control: Default::default(),
            prompt_trace_hash: None,
            patch_session_commit_id: None,
            checkpoint_id: None,
            patch_write_back_state: None,
            metadata: Value::Null,
            error: None,
            error_state: None,
            execution_attempts: 1,
        },
    );
    let store = SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    store.save_state(&runtime.state).unwrap();
    FileConfirmationLogStore::default_for_project(temp.path())
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-1".to_owned(),
            kind: "approval".to_owned(),
            node_id: "approval".to_owned(),
            timestamp_ms: 1,
            state: ConfirmationLogState::Pending,
            handling_method: "manual".to_owned(),
            summary: "待确认输出".to_owned(),
            diff: "- old\n+ new".to_owned(),
        })
        .unwrap();

    let result = resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: "wf".to_owned(),
            run_id: "run-1".to_owned(),
            confirmation_id: "confirm-1".to_owned(),
            decision: ConfirmationDecision::Approve,
            review_reason: Some("人工通过".to_owned()),
        },
    )
    .unwrap();
    let updated = store
        .load_state(&WorkflowId::from("wf"), &RunId::from("run-1"))
        .unwrap()
        .unwrap();
    let node = updated.nodes.get(&NodeId::from("approval")).unwrap();

    assert_eq!(result.confirmation.state, ConfirmationLogState::Approved);
    assert_eq!(result.badges.confirmations, 0);
    assert_eq!(result.workflow.status, "queued");
    assert!(matches!(
        node.outputs.get("approved"),
        Some(PortValue::Inline { value }) if value == &json!(true)
    ));
    assert!(matches!(
        node.outputs.get("review_reason"),
        Some(PortValue::Inline { value }) if value == &json!("人工通过")
    ));
}

fn run_git<const N: usize>(repo: &std::path::Path, args: [&str; N]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout<const N: usize>(repo: &std::path::Path, args: [&str; N]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
