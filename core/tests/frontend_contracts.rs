use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;

use ariadne::config::ConfigStore;
use ariadne::contracts::{
    ArtifactKind, Edge, EdgeId, ExecutionCancellation, NodeId, NodeInstance, PatchHunk,
    PermissionPolicy, PortEndpoint, PortValue, ProviderCapability, ProviderDefinition,
    ProviderType, RunId, SourceSpan, TextRange, WorkflowDefinition, WorkflowEdgeKind, WorkflowId,
    EXECUTION_INPUT_PORT, EXECUTION_OUTPUT_PORT,
};
use ariadne::costs::{BudgetLimits, SqliteCostLedger};
use ariadne::diagnostics::{BackendDiagnosticsReport, DiagnosticStatus};
use ariadne::documents::{
    ChapterDocumentEntry, ChapterDocumentIndex, ChapterDocumentKind, DocumentReadRequest,
    DocumentRepository, DocumentWriteRequest, FileDocumentService,
};
use ariadne::frontend::{
    apply_node_detail_patch, apply_quick_edit_patch, build_works_tree, export_chapters_combined,
    export_chapters_markdown, export_workflow_selection, extract_project_reference_tokens,
    import_chapter_document, initialize_project, initialize_project_with_app_state,
    install_workflow_template_manifest, node_has_breakpoint, now_timestamp_ms,
    pack_workflow_selection, project_ai_context_window, project_document_permission,
    quick_edit_to_patch, set_node_breakpoint, upsert_canvas_annotation, ArtifactReferenceEntry,
    CanvasAnnotation, ChapterExportFormat, ChapterImportRequest, ConfirmationLogEntry,
    ConfirmationLogState, ConfirmationLogStore, FileConfirmationLogStore, NodeDetailPatch,
    ProjectAiAppendOutcome, ProjectAiChatMessage, ProjectAiChatRole, ProjectAiConversationStore,
    ProjectMemoryStore, ProjectReferenceKind, ProjectReferenceResolver, ProjectRegistryStore,
    QuickEditService, TemplateRepositoryClient, UiPreferences, UiPreferencesStore, UiRunLogEntry,
    UiRunLogFilter, UiRunLogKind, UiRunLogLevel, UiRunLogStore, OFFICIAL_TEMPLATE_REPOSITORY_URL,
};
use ariadne::llm::{LlmService, LlmServiceConfig};
use ariadne::providers::{
    LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext,
};
use ariadne::skills::{WorkflowManifest, WorkflowTemplateLoader};
use ariadne::workflow::{WorkflowNodeRuntimeState, WorkflowRunState};
use serde_json::{json, Value};

fn allow_local_template_repository_for_test() {
    std::env::set_var("ARIADNE_ALLOW_LOCAL_TEMPLATE_REPOSITORY", "1");
}

struct MockQuickEditProvider;

struct MockLongQuickEditProvider {
    response: String,
}

impl Provider for MockQuickEditProvider {
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: "mock".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Mock".to_owned(),
            capabilities: vec![ProviderCapability::Llm],
            config_schema: Value::Null,
        }
    }
}

impl LlmProvider for MockQuickEditProvider {
    fn complete(
        &self,
        _context: &ProviderCallContext,
        _request: LlmRequest,
    ) -> ariadne::contracts::CoreResult<LlmResponse> {
        Ok(LlmResponse {
            message: LlmMessage::assistant("改写后文本"),
            tool_calls: Vec::new(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            cost_usd: None,
            raw: Value::Null,
        })
    }
}

impl Provider for MockLongQuickEditProvider {
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: "mock-long".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Mock Long".to_owned(),
            capabilities: vec![ProviderCapability::Llm],
            config_schema: Value::Null,
        }
    }
}

impl LlmProvider for MockLongQuickEditProvider {
    fn complete(
        &self,
        _context: &ProviderCallContext,
        _request: LlmRequest,
    ) -> ariadne::contracts::CoreResult<LlmResponse> {
        Ok(LlmResponse {
            message: LlmMessage::assistant(self.response.clone()),
            tool_calls: Vec::new(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            cost_usd: None,
            raw: Value::Null,
        })
    }
}

#[test]
fn project_memory_supports_read_append_and_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let store = ProjectMemoryStore::default_for_project(temp.path());

    assert_eq!(store.read_all().unwrap(), "");
    store.append("第一条记忆").unwrap();
    store.append("第二条记忆").unwrap();
    assert!(store.read_all().unwrap().contains("第一条记忆\n第二条记忆"));
    store.write_all("覆盖记忆").unwrap();
    assert_eq!(store.read_all().unwrap(), "覆盖记忆");
}

#[test]
fn confirmation_log_resolves_at_reference_to_diff_and_state() {
    let store = ConfirmationLogStore::default();
    store
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-1".to_owned(),
            kind: "writer_patch".to_owned(),
            node_id: "writer".to_owned(),
            timestamp_ms: now_timestamp_ms(),
            state: ConfirmationLogState::Approved,
            handling_method: "human".to_owned(),
            summary: "改写第一段".to_owned(),
            diff: "- old\n+ new".to_owned(),
            workflow_id: "wf".to_owned(),
            run_id: "run-1".to_owned(),
        })
        .unwrap();

    let reference = store.resolve_reference("@确认项:confirm-1").unwrap();
    assert_eq!(reference.state, ConfirmationLogState::Approved);
    assert_eq!(reference.diff, "- old\n+ new");
}

#[test]
fn file_confirmation_log_persists_reference_resolution() {
    let temp = tempfile::tempdir().unwrap();
    let store = FileConfirmationLogStore::default_for_project(temp.path());
    store
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-2".to_owned(),
            kind: "writer_patch".to_owned(),
            node_id: "writer".to_owned(),
            timestamp_ms: now_timestamp_ms(),
            state: ConfirmationLogState::Pending,
            handling_method: "pending".to_owned(),
            summary: "等待确认".to_owned(),
            diff: "- a\n+ b".to_owned(),
            workflow_id: "wf".to_owned(),
            run_id: "run-1".to_owned(),
        })
        .unwrap();

    let reopened = FileConfirmationLogStore::default_for_project(temp.path());
    let reference = reopened.resolve_reference("@确认项:confirm-2").unwrap();

    assert_eq!(reference.state, ConfirmationLogState::Pending);
    assert_eq!(reference.summary, "等待确认");
    assert_eq!(reference.diff, "- a\n+ b");
}

#[test]
fn workflow_selection_exports_internal_edges_and_boundary_pins() {
    let workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Workflow".to_owned(),
        nodes: vec![node("a"), node("b"), node("c")],
        edges: vec![control_edge("a-b", "a", "b"), control_edge("b-c", "b", "c")],
        metadata: Value::Null,
    };

    let exported = export_workflow_selection(&workflow, &["b".to_owned()]).unwrap();

    assert_eq!(exported.workflow.nodes.len(), 1);
    assert!(exported.workflow.edges.is_empty());
    assert_eq!(exported.boundary_inputs[0].node_id, NodeId::from("b"));
    assert_eq!(exported.boundary_outputs[0].node_id, NodeId::from("b"));
}

#[test]
fn workflow_selection_packs_nodes_into_subworkflow_and_rewires_boundary_edges() {
    let mut workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Workflow".to_owned(),
        nodes: vec![
            node("source"),
            node("writer"),
            node("reviewer"),
            node("sink"),
        ],
        edges: vec![
            data_edge("source-writer", "source", "writer", "draft"),
            control_edge("writer-reviewer", "writer", "reviewer"),
            data_edge("reviewer-sink", "reviewer", "sink", "review"),
        ],
        metadata: Value::Null,
    };

    let report = pack_workflow_selection(
        &mut workflow,
        &["writer".to_owned(), "reviewer".to_owned()],
        Some("packed-review".to_owned()),
        Some("Review Subflow".to_owned()),
    )
    .unwrap();

    assert_eq!(report.subworkflow_node_id, NodeId::from("packed-review"));
    assert_eq!(report.embedded_workflow.nodes.len(), 2);
    assert_eq!(report.embedded_workflow.edges.len(), 1);
    assert_eq!(report.boundary_inputs[0].node_id, NodeId::from("writer"));
    assert_eq!(report.boundary_outputs[0].node_id, NodeId::from("reviewer"));
    assert!(workflow
        .nodes
        .iter()
        .all(|node| node.id != NodeId::from("writer")));
    assert!(workflow.nodes.iter().any(|node| {
        node.id == NodeId::from("packed-review")
            && node.type_name == "subworkflow"
            && node.config.get("embedded_workflow").is_some()
    }));
    assert!(workflow.edges.iter().any(|edge| {
        edge.from.node_id == NodeId::from("source")
            && edge.to.node_id == NodeId::from("packed-review")
    }));
    assert!(workflow.edges.iter().any(|edge| {
        edge.from.node_id == NodeId::from("packed-review")
            && edge.to.node_id == NodeId::from("sink")
    }));
}

#[test]
fn workflow_breakpoint_helper_updates_node_config() {
    let mut workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Workflow".to_owned(),
        nodes: vec![node("a")],
        edges: Vec::new(),
        metadata: Value::Null,
    };

    set_node_breakpoint(&mut workflow, "a", true).unwrap();

    assert!(node_has_breakpoint(&workflow.nodes[0]));
}

#[test]
fn template_repository_client_uses_search_detail_and_download_endpoints() {
    allow_local_template_repository_for_test();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            let body = if request.starts_with("GET /templates/search") {
                json!([{
                    "id": "basic",
                    "name": "Basic",
                    "tags": ["writing"],
                    "requires_permissions": true
                }])
            } else if request.starts_with("GET /templates/basic/download") {
                json!({ "workflow_id": "basic" })
            } else {
                json!({
                    "id": "basic",
                    "name": "Basic",
                    "version": "1.0.0",
                    "manifest": { "workflow_id": "basic" },
                    "requires_permissions": true
                })
            };
            let body = serde_json::to_vec(&body).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        }
    });

    let client =
        TemplateRepositoryClient::new(format!("http://127.0.0.1:{}", addr.port())).unwrap();
    let results = client.search("basic", &["writing".to_owned()], 1).unwrap();
    let detail = client.detail("basic").unwrap();
    let manifest = client.download("basic").unwrap();

    server.join().unwrap();
    assert_eq!(results[0].id, "basic");
    assert!(detail.requires_permissions);
    assert_eq!(manifest["workflow_id"], "basic");
}

#[test]
fn official_template_repository_is_versioned_offline_and_installable() {
    let client = TemplateRepositoryClient::new(OFFICIAL_TEMPLATE_REPOSITORY_URL).unwrap();

    let all = client.search("", &[], 0).unwrap();
    let worldbuilding = client.search("世界观", &[], 0).unwrap();
    let detail = client.detail("official-novel-starter").unwrap();
    let manifest = client.download("official-novel-starter").unwrap();

    assert_eq!(all.len(), 3);
    assert_eq!(worldbuilding.len(), 1);
    assert_eq!(worldbuilding[0].id, "official-worldbuilding");
    assert_eq!(detail.version, "1.0.0");
    assert_eq!(detail.name, "ui.template.builtin.novel_starter.name");
    assert_eq!(manifest["minimum_ariadne_version"], "0.1.0");

    let temp = tempfile::tempdir().unwrap();
    let report = client
        .download_to_workflows("official-novel-starter", temp.path())
        .unwrap();
    let loaded = WorkflowTemplateLoader::new()
        .with_project_root(temp.path())
        .get("official-novel-starter", "1.0.0")
        .unwrap();
    assert_eq!(report.workflow_id, "official-novel-starter");
    assert_eq!(loaded.manifest.workflow.nodes.len(), 2);
}

#[test]
fn template_repository_rejects_unrecognized_internal_scheme_and_future_manifest() {
    let invalid = TemplateRepositoryClient::new("ariadne://other/v1").unwrap_err();
    assert!(invalid.to_string().contains("official v1 URL"));

    let temp = tempfile::tempdir().unwrap();
    let mut manifest = workflow_manifest("future-template", "1.0.0");
    manifest.minimum_ariadne_version = Some("999.0.0".to_owned());
    let error = install_workflow_template_manifest(
        serde_json::to_value(manifest).unwrap(),
        temp.path(),
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("requires Ariadne 999.0.0"));
    assert!(!temp.path().join("future-template").exists());
}

#[test]
fn template_repository_client_rejects_oversized_streaming_response() {
    allow_local_template_repository_for_test();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        stream.write_all(&vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
        stream.flush().unwrap();
    });

    let client =
        TemplateRepositoryClient::new(format!("http://127.0.0.1:{}", addr.port())).unwrap();
    let error = client.search("basic", &[], 0).unwrap_err().to_string();

    server.join().unwrap();
    assert!(error.contains("template_repository_response"));
    assert!(error.contains("response exceeds"));
}

#[test]
fn c9_template_repository_request_can_cancel_stalled_response() {
    allow_local_template_repository_for_test();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_accepted = std::sync::Arc::clone(&accepted);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        let _ = stream.read(&mut request).unwrap();
        server_accepted.store(true, std::sync::atomic::Ordering::Release);
        std::thread::sleep(std::time::Duration::from_millis(500));
    });

    let cancellation = ExecutionCancellation::new();
    let cancel_from_thread = cancellation.clone();
    let canceller = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        while !accepted.load(std::sync::atomic::Ordering::Acquire)
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        cancel_from_thread.cancel();
    });
    let client = TemplateRepositoryClient::new_with_cancellation(
        format!("http://127.0.0.1:{}", addr.port()),
        cancellation,
    )
    .unwrap();
    let started = std::time::Instant::now();
    let error = client.search("basic", &[], 0).unwrap_err();
    let request_elapsed = started.elapsed();

    canceller.join().unwrap();
    server.join().unwrap();
    assert!(matches!(
        error,
        ariadne::contracts::CoreError::ExternalCancellation { .. }
    ));
    assert!(request_elapsed < std::time::Duration::from_millis(300));
}

#[test]
fn downloaded_template_manifest_installs_into_workflows_loader_path() {
    let temp = tempfile::tempdir().unwrap();
    let workflows_root = temp.path().join("workflows");
    let manifest = workflow_manifest("market-basic", "1.0.0");

    let report = install_workflow_template_manifest(
        serde_json::to_value(&manifest).unwrap(),
        &workflows_root,
        true,
    )
    .unwrap();
    let loader = WorkflowTemplateLoader::new().with_project_root(&workflows_root);
    let loaded = loader.get("market-basic", "1.0.0").unwrap();

    assert!(report.requires_permissions);
    assert!(report.manifest_path.ends_with("workflow.json"));
    assert_eq!(loaded.manifest.workflow_id, "market-basic");
    assert_eq!(loaded.manifest.import_definition().unwrap().nodes.len(), 1);
}

#[cfg(unix)]
#[test]
fn template_install_rejects_symlink_escape_from_workflows_root() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let workflows_root = temp.path().join("workflows");
    std::fs::create_dir_all(&workflows_root).unwrap();
    std::os::unix::fs::symlink(outside.path(), workflows_root.join("market-basic")).unwrap();
    let manifest = workflow_manifest("market-basic", "1.0.0");

    let error = install_workflow_template_manifest(
        serde_json::to_value(&manifest).unwrap(),
        &workflows_root,
        true,
    )
    .unwrap_err();

    assert!(error.to_string().contains("outside allowed root"));
    assert!(!outside.path().join("workflow.json").exists());
}

#[test]
fn quick_edit_generates_result_and_patch() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let llm = LlmService::new(&ledger, Default::default());
    let provider = MockQuickEditProvider;
    let service = QuickEditService::new(
        llm,
        &provider,
        LlmServiceConfig {
            provider_id: "mock".to_owned(),
            model_id: "model".to_owned(),
            max_tool_rounds: 0,
            timeout_ms: 1_000,
            max_total_tokens: None,
            budget_limits: BudgetLimits::default(),
            input_cost_per_million_tokens: None,
            output_cost_per_million_tokens: None,
            max_output_tokens: None,
            max_context_tokens: None,
        },
    );

    let result = service
        .quick_edit("原文", "改得更直接", Some("chapter-1"))
        .unwrap();
    let patch = quick_edit_to_patch(
        "原文\n下一行\n",
        "doc-1",
        Some("v1".to_owned()),
        TextRange { start: 0, end: 6 },
        &result,
    )
    .unwrap();

    assert_eq!(result.suggested, "改写后文本");
    assert!(!result.diff.is_empty());
    assert_eq!(patch.document_id, "doc-1");
}

#[test]
fn quick_edit_diff_preview_is_bounded_for_long_chapters() {
    let original = "旧".repeat(20_000);
    let suggested = "新".repeat(20_000);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let llm = LlmService::new(&ledger, Default::default());
    let provider = MockLongQuickEditProvider {
        response: suggested,
    };
    let result = QuickEditService::new(
        llm,
        &provider,
        LlmServiceConfig {
            provider_id: "mock-long".to_owned(),
            model_id: "model".to_owned(),
            max_tool_rounds: 0,
            timeout_ms: 1_000,
            max_total_tokens: None,
            budget_limits: BudgetLimits::default(),
            input_cost_per_million_tokens: None,
            output_cost_per_million_tokens: None,
            max_output_tokens: None,
            max_context_tokens: None,
        },
    )
    .quick_edit(&original, "改写", None)
    .unwrap();

    assert!(result.diff.len() <= 16 * 1024);
    assert!(result.diff.starts_with("- 旧"));
    assert!(result.diff.contains("\n+ 新"));
}

#[test]
fn quick_edit_diff_marks_changed_lines_and_folds_unchanged_runs() {
    let original = "共同一\n共同二\n共同三\n旧句\n共同四\n共同五\n共同六";
    let suggested = "共同一\n共同二\n共同三\n新句\n共同四\n共同五\n共同六";
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let llm = LlmService::new(&ledger, Default::default());
    let provider = MockLongQuickEditProvider {
        response: suggested.to_owned(),
    };
    let result = QuickEditService::new(
        llm,
        &provider,
        LlmServiceConfig {
            provider_id: "mock-diff".to_owned(),
            model_id: "model".to_owned(),
            max_tool_rounds: 0,
            timeout_ms: 1_000,
            max_total_tokens: None,
            budget_limits: BudgetLimits::default(),
            input_cost_per_million_tokens: None,
            output_cost_per_million_tokens: None,
            max_output_tokens: None,
            max_context_tokens: None,
        },
    )
    .quick_edit(original, "替换中间一句", None)
    .unwrap();

    assert_eq!(result.suggested, suggested);
    assert_eq!(
        result.diff,
        "  ... (3 unchanged lines)\n- 旧句\n+ 新句\n  ... (3 unchanged lines)\n"
    );
}

#[test]
fn quick_edit_patch_replaces_only_selected_span() {
    let document = "第一行：旧词，保留前后\n第二行不动\n";
    let start = document.find("旧词").unwrap();
    let end = start + "旧词".len();
    let range = TextRange {
        start: start as u64,
        end: end as u64,
    };
    let result = ariadne::frontend::QuickEditResult {
        original: "旧词".to_owned(),
        suggested: "新词".to_owned(),
        diff: "- 旧词\n+ 新词".to_owned(),
    };

    let patch = quick_edit_to_patch(document, "doc-1", None, range, &result).unwrap();

    assert_eq!(
        patch.hunks,
        vec![PatchHunk {
            range,
            replacement: "新词".to_owned(),
        }]
    );
}

#[test]
fn quick_edit_patch_can_apply_through_document_service() {
    let temp = tempfile::tempdir().unwrap();
    let artifact_root = temp.path().join(".runtime").join("artifacts");
    let service = FileDocumentService::new(
        PermissionPolicy {
            readable_file_roots: vec![temp.path().to_path_buf()],
            writable_file_roots: vec![temp.path().to_path_buf()],
            ..PermissionPolicy::default()
        },
        artifact_root,
    );
    let path = temp.path().join("doc.md");
    service
        .save_document(DocumentWriteRequest {
            path: path.clone(),
            content: "原文\n".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap();
    let result = ariadne::frontend::QuickEditResult {
        original: "原文".to_owned(),
        suggested: "改写后文本".to_owned(),
        diff: "- 原文\n+ 改写后文本".to_owned(),
    };

    let report = apply_quick_edit_patch(
        &service,
        &path.to_string_lossy(),
        None,
        "原文\n",
        TextRange { start: 0, end: 6 },
        &result,
    )
    .unwrap();

    assert_eq!(report.index_invalidation.reason, "patch_applied");
    assert_eq!(
        service
            .open_document(DocumentReadRequest { path, format: None })
            .unwrap()
            .content,
        "改写后文本\n"
    );
}

#[test]
fn project_registry_initializes_project_and_tracks_recent_projects() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("novel");
    let report = initialize_project(&project).unwrap();
    let registry = ProjectRegistryStore::default_for_project(&project);
    let recent = registry.record_opened("Novel", &project).unwrap();

    assert!(report.ready);
    assert!(report.git_initialized);
    assert_eq!(report.project_root, project);
    assert_eq!(report.project_name, "Untitled Literature Project");
    assert_eq!(report.created_config_files.len(), 7);
    for file_name in [
        "app.yaml",
        "providers.yaml",
        "permissions.yaml",
        "rag.yaml",
        "workflow.yaml",
        "git.yaml",
        "auto_mode.yaml",
    ] {
        assert!(report
            .created_config_files
            .contains(&project.join(".config").join(file_name)));
    }
    for directory in [
        ".config",
        ".runtime",
        "planning",
        "planning/stages",
        "planning/chapters",
        "documents",
        "workflows",
        "skills",
        "exports",
    ] {
        assert!(report.created_dirs.contains(&project.join(directory)));
        assert!(project.join(directory).is_dir());
    }
    assert!(project.join(".config").is_dir());
    assert!(project.join(".config/app.yaml").exists());
    assert!(project.join(".config/providers.yaml").exists());
    assert!(project.join(".config/auto_mode.yaml").exists());
    assert!(project.join("planning/chapters").is_dir());
    assert!(project.join(".git").is_dir());
    assert_eq!(recent[0].name, "Novel");
    assert_eq!(registry.read_all().unwrap()[0].path, project);
}

#[test]
fn project_initialization_binds_the_explicit_app_state_authority() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();

    initialize_project_with_app_state(project.path(), app_state.path(), None).unwrap();

    assert_eq!(
        ariadne::config::trusted_app_state_for_project(project.path()),
        app_state.path().canonicalize().unwrap()
    );
    ConfigStore::new(project.path()).load().unwrap();
}

#[test]
fn project_reference_resolver_handles_confirmation_document_chapter_artifact_and_node_output() {
    let temp = tempfile::tempdir().unwrap();
    let service = test_document_service(temp.path());
    let doc_path = temp.path().join("documents").join("chapter.md");
    service
        .save_document(DocumentWriteRequest {
            path: doc_path.clone(),
            content: "正文".to_owned(),
            format: None,
            base_version: None,
        })
        .unwrap();
    let doc = service
        .open_document(DocumentReadRequest {
            path: doc_path.clone(),
            format: None,
        })
        .unwrap();
    let index = ChapterDocumentIndex::new(
        "v1",
        vec![ChapterDocumentEntry {
            chapter_id: "stage1:chapter1".to_owned(),
            document_id: doc.metadata.document_id.clone(),
            path: doc_path.clone(),
            title: "第一章".to_owned(),
            order: 1,
            kind: ChapterDocumentKind::ChapterBody,
            version: doc.metadata.version.clone(),
            word_count: Some(1),
            outline_ref: Some(SourceSpan {
                document_id: "outline".to_owned(),
                range: TextRange { start: 0, end: 2 },
                version: None,
            }),
        }],
    )
    .unwrap();
    let confirmations = FileConfirmationLogStore::default_for_project(temp.path());
    confirmations
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-3".to_owned(),
            kind: "writer_patch".to_owned(),
            node_id: "writer".to_owned(),
            timestamp_ms: now_timestamp_ms(),
            state: ConfirmationLogState::Approved,
            handling_method: "human".to_owned(),
            summary: "批准".to_owned(),
            diff: "- a\n+ b".to_owned(),
            workflow_id: "wf".to_owned(),
            run_id: "run-1".to_owned(),
        })
        .unwrap();
    let mut runtime = WorkflowRunState::new(WorkflowId::from("wf"), RunId::from("run"));
    let mut outputs = ariadne::contracts::PortMap::new();
    outputs.insert("draft".to_owned(), PortValue::inline("正文"));
    runtime.nodes.insert(
        NodeId::from("writer"),
        WorkflowNodeRuntimeState {
            node_id: NodeId::from("writer"),
            status: ariadne::contracts::RunStatus::Succeeded,
            outputs,
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
    let resolver = ProjectReferenceResolver::new()
        .with_confirmations(&confirmations)
        .with_documents(&service)
        .with_chapter_index(&index)
        .with_runtime(&runtime)
        .with_artifacts(vec![ArtifactReferenceEntry {
            artifact_id: "artifact-1".to_owned(),
            kind: ArtifactKind::Export,
            storage_uri: "file:///tmp/export.md".to_owned(),
            summary: Some("导出文件".to_owned()),
        }]);

    assert_eq!(
        resolver.resolve("@确认项:confirm-3").unwrap().kind,
        ProjectReferenceKind::Confirmation
    );
    let document = resolver
        .resolve(&format!("@文档/{}", doc_path.display()))
        .unwrap();
    assert_eq!(document.kind, ProjectReferenceKind::Document);
    let fragment = &document.payload["fragments"][0];
    assert_eq!(fragment["text"], "正文");
    assert_eq!(fragment["start_line"], 1);
    assert_eq!(fragment["source_version"], document.payload["version"]);
    assert_eq!(
        resolver.resolve("@章节/stage1:chapter1").unwrap().summary,
        "第一章"
    );
    assert_eq!(
        resolver.resolve("@artifact/artifact-1").unwrap().summary,
        "导出文件"
    );
    assert_eq!(
        resolver.resolve("@节点/writer/输出/draft").unwrap().kind,
        ProjectReferenceKind::NodeOutput
    );
}

#[test]
fn project_reference_tokens_are_extracted_and_deduplicated_from_message() {
    assert_eq!(
        extract_project_reference_tokens(
            "比较 @文档/documents/a.md，引用 @知识/story-segment-1；再看 @文档/documents/a.md。"
        ),
        vec![
            "@文档/documents/a.md".to_owned(),
            "@知识/story-segment-1".to_owned(),
        ]
    );
}

#[test]
fn project_document_reference_uses_bounded_query_centered_fragments() {
    let temp = tempfile::tempdir().unwrap();
    let service = test_document_service(temp.path());
    let path = temp.path().join("documents/large.md");
    let content = format!(
        "{}MIDDLE-DOCUMENT-SENTINEL{}",
        "a".repeat(20_000),
        "b".repeat(20_000)
    );
    service
        .save_document(DocumentWriteRequest {
            path: path.clone(),
            content,
            format: None,
            base_version: None,
        })
        .unwrap();
    let resolver = ProjectReferenceResolver::new()
        .with_documents(&service)
        .with_document_root(temp.path())
        .with_query("MIDDLE-DOCUMENT-SENTINEL");
    let reference = resolver.resolve("@文档/documents/large.md").unwrap();
    let fragments = reference.payload["fragments"].as_array().unwrap();
    let chars = fragments
        .iter()
        .filter_map(|fragment| fragment["text"].as_str())
        .map(|text| text.chars().count())
        .sum::<usize>();
    assert!(chars <= 32 * 1024);
    assert!(fragments.iter().any(|fragment| {
        fragment["text"]
            .as_str()
            .is_some_and(|text| text.contains("MIDDLE-DOCUMENT-SENTINEL"))
    }));
    assert_eq!(reference.payload["content_truncated"], true);
}

#[test]
fn run_log_store_generates_toasts_filters_logs_and_badges() {
    let temp = tempfile::tempdir().unwrap();
    let store = UiRunLogStore::default_for_project(temp.path());
    let confirmations = FileConfirmationLogStore::default_for_project(temp.path());
    confirmations
        .record(ConfirmationLogEntry {
            confirmation_id: "confirm-4".to_owned(),
            kind: "writer_patch".to_owned(),
            node_id: "writer".to_owned(),
            timestamp_ms: now_timestamp_ms(),
            state: ConfirmationLogState::Pending,
            handling_method: "pending".to_owned(),
            summary: "等待".to_owned(),
            diff: "diff".to_owned(),
            workflow_id: "wf".to_owned(),
            run_id: "run-1".to_owned(),
        })
        .unwrap();
    let toast = store
        .append(UiRunLogEntry {
            log_id: "log-1".to_owned(),
            timestamp_ms: 0,
            kind: UiRunLogKind::Error,
            level: UiRunLogLevel::Error,
            message: "provider failed".to_owned(),
            workflow_id: None,
            run_id: None,
            node_id: Some(NodeId::from("writer")),
            unread: false,
            metadata: Value::Null,
        })
        .unwrap();
    let diagnostics = BackendDiagnosticsReport {
        status: DiagnosticStatus::Degraded,
        items: Vec::new(),
    };

    assert_eq!(toast.target.as_deref(), Some("run_log"));
    assert_eq!(
        store
            .query(UiRunLogFilter {
                query: Some("provider".to_owned()),
                ..UiRunLogFilter::default()
            })
            .unwrap()
            .len(),
        1
    );
    let badges = store
        .badge_counts(Some(&confirmations), Some(&diagnostics))
        .unwrap();
    assert_eq!(badges.run_logs, 1);
    assert_eq!(badges.confirmations, 1);
    assert_eq!(badges.diagnostics, 1);
    store.mark_all_read().unwrap();
    assert_eq!(
        store
            .badge_counts(Some(&confirmations), None)
            .unwrap()
            .run_logs,
        0
    );
}

#[test]
fn ui_log_stores_migrate_legacy_json_once_and_use_sqlite_indexes() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = temp.path().join(".runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    let timestamp_ms = now_timestamp_ms();
    let confirmation = ConfirmationLogEntry {
        confirmation_id: "legacy-confirm".to_owned(),
        kind: "writer_patch".to_owned(),
        node_id: "writer".to_owned(),
        timestamp_ms,
        state: ConfirmationLogState::Pending,
        handling_method: "pending".to_owned(),
        summary: "旧确认".to_owned(),
        diff: "diff".to_owned(),
        workflow_id: "wf".to_owned(),
        run_id: "run".to_owned(),
    };
    let run_log = UiRunLogEntry {
        log_id: "legacy-log".to_owned(),
        timestamp_ms: timestamp_ms + 1,
        kind: UiRunLogKind::Error,
        level: UiRunLogLevel::Error,
        message: "legacy failure".to_owned(),
        workflow_id: Some(WorkflowId::from("wf")),
        run_id: Some(RunId::from("run")),
        node_id: Some(NodeId::from("writer")),
        unread: true,
        metadata: Value::Null,
    };
    std::fs::write(
        runtime.join("confirmation_log.json"),
        serde_json::to_string(&vec![confirmation]).unwrap(),
    )
    .unwrap();
    std::fs::write(
        runtime.join("run_log.json"),
        serde_json::to_string(&vec![run_log]).unwrap(),
    )
    .unwrap();

    let confirmations = FileConfirmationLogStore::default_for_project(temp.path());
    let logs = UiRunLogStore::default_for_project(temp.path());
    assert_eq!(confirmations.pending_count().unwrap(), 1);
    assert_eq!(logs.read_all().unwrap().len(), 1);
    assert!(runtime.join("ui_logs.db").exists());

    // 再次打开只读取迁移后的 SQLite，不重复导入旧 JSON。
    assert_eq!(confirmations.read_all().unwrap().len(), 1);
    assert_eq!(logs.read_all().unwrap().len(), 1);
    logs.mark_all_read().unwrap();
    assert!(!logs.read_all().unwrap()[0].unread);
}

#[test]
fn run_log_query_supports_stable_cursor_and_limit() {
    let temp = tempfile::tempdir().unwrap();
    let store = UiRunLogStore::default_for_project(temp.path());
    let timestamp_ms = now_timestamp_ms();
    for (log_id, timestamp_ms) in [
        ("a", timestamp_ms),
        ("b", timestamp_ms),
        ("c", timestamp_ms + 1),
    ] {
        store
            .append(UiRunLogEntry {
                log_id: log_id.to_owned(),
                timestamp_ms,
                kind: UiRunLogKind::Node,
                level: UiRunLogLevel::Info,
                message: format!("log {log_id}"),
                workflow_id: Some(WorkflowId::from("wf")),
                run_id: Some(RunId::from("run")),
                node_id: None,
                unread: false,
                metadata: Value::Null,
            })
            .unwrap();
    }

    let first = store
        .query(UiRunLogFilter {
            limit: Some(2),
            ..UiRunLogFilter::default()
        })
        .unwrap();
    assert_eq!(
        first
            .iter()
            .map(|entry| entry.log_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    let second = store
        .query(UiRunLogFilter {
            after_timestamp_ms: Some(first[1].timestamp_ms),
            after_log_id: Some(first[1].log_id.clone()),
            limit: Some(2),
            ..UiRunLogFilter::default()
        })
        .unwrap();
    assert_eq!(
        second
            .iter()
            .map(|entry| entry.log_id.as_str())
            .collect::<Vec<_>>(),
        vec!["c"]
    );

    let newest = store
        .query(UiRunLogFilter {
            limit: Some(2),
            descending: true,
            ..UiRunLogFilter::default()
        })
        .unwrap();
    assert_eq!(
        newest
            .iter()
            .map(|entry| entry.log_id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "b"]
    );
    let older = store
        .query(UiRunLogFilter {
            after_timestamp_ms: Some(newest[1].timestamp_ms),
            after_log_id: Some(newest[1].log_id.clone()),
            limit: Some(2),
            descending: true,
            ..UiRunLogFilter::default()
        })
        .unwrap();
    assert_eq!(
        older
            .iter()
            .map(|entry| entry.log_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a"]
    );
}

#[test]
fn run_log_mark_read_respects_visible_filter_scope() {
    let temp = tempfile::tempdir().unwrap();
    let store = UiRunLogStore::default_for_project(temp.path());
    let timestamp_ms = now_timestamp_ms();
    for (log_id, run_id, level) in [
        ("run-a-error", "run-a", UiRunLogLevel::Error),
        ("run-a-info", "run-a", UiRunLogLevel::Info),
        ("run-b-error", "run-b", UiRunLogLevel::Error),
    ] {
        store
            .append(UiRunLogEntry {
                log_id: log_id.to_owned(),
                timestamp_ms,
                kind: UiRunLogKind::Node,
                level,
                message: log_id.to_owned(),
                workflow_id: Some(WorkflowId::from("wf")),
                run_id: Some(RunId::from(run_id)),
                node_id: Some(NodeId::from("writer")),
                unread: false,
                metadata: Value::Null,
            })
            .unwrap();
    }

    let updated = store
        .mark_read(UiRunLogFilter {
            level: Some(UiRunLogLevel::Error),
            run_id: Some(RunId::from("run-a")),
            ..UiRunLogFilter::default()
        })
        .unwrap();
    assert_eq!(updated, 1);

    let entries = store.read_all().unwrap();
    assert!(
        !entries
            .iter()
            .find(|entry| entry.log_id == "run-a-error")
            .unwrap()
            .unread
    );
    assert!(
        entries
            .iter()
            .find(|entry| entry.log_id == "run-a-info")
            .unwrap()
            .unread
    );
    assert!(
        entries
            .iter()
            .find(|entry| entry.log_id == "run-b-error")
            .unwrap()
            .unread
    );
}

#[test]
fn works_service_builds_tree_imports_chapters_and_exports_selected_markdown() {
    let temp = tempfile::tempdir().unwrap();
    let service = test_document_service(temp.path());
    let source = temp.path().join("source.md");
    std::fs::write(&source, "第一章正文").unwrap();
    let target = temp.path().join("documents").join("chapter1.md");
    let import = import_chapter_document(
        &service,
        ChapterImportRequest {
            chapter_id: "stage1:chapter1".to_owned(),
            title: "第一章".to_owned(),
            order: 1,
            source_path: source,
            target_path: target,
            overwrite: false,
            outline_ref: None,
        },
    )
    .unwrap();
    let index = ChapterDocumentIndex::new("v1", vec![import.entry.clone()]).unwrap();
    let chapter_stage = BTreeMap::from([("stage1:chapter1".to_owned(), "stage1".to_owned())]);
    let tree = build_works_tree(&index, &chapter_stage, temp.path().join("planning")).unwrap();
    let export = export_chapters_markdown(
        &service,
        &index,
        &["stage1:chapter1".to_owned()],
        "exports/book.md",
    )
    .unwrap();

    assert_eq!(tree.children[0].children[0].title, "第一章");
    assert_eq!(export.exported_chapter_ids, vec!["stage1:chapter1"]);
    assert!(export.storage_uri.ends_with("exports/book.md"));
}

/// F20：作品树只接受 metadata.db 的正式阶段关系，不再从 chapter_id 文本猜测。
#[test]
fn f20_works_tree_uses_official_stage_relation_and_preserves_unassigned_chapters() {
    let index = ChapterDocumentIndex::new(
        "v1",
        vec![
            ChapterDocumentEntry {
                chapter_id: "misleading-prefix:chapter-1".to_owned(),
                document_id: "documents/chapter-1.md".to_owned(),
                path: "documents/chapter-1.md".into(),
                title: "第一章".to_owned(),
                order: 1,
                kind: ChapterDocumentKind::ChapterBody,
                version: "v1".to_owned(),
                word_count: None,
                outline_ref: None,
            },
            ChapterDocumentEntry {
                chapter_id: "chapter-2".to_owned(),
                document_id: "documents/chapter-2.md".to_owned(),
                path: "documents/chapter-2.md".into(),
                title: "第二章".to_owned(),
                order: 2,
                kind: ChapterDocumentKind::ChapterBody,
                version: "v2".to_owned(),
                word_count: None,
                outline_ref: None,
            },
        ],
    )
    .unwrap();
    let official = BTreeMap::from([(
        "misleading-prefix:chapter-1".to_owned(),
        "official-stage".to_owned(),
    )]);

    let tree = build_works_tree(&index, &official, "planning").unwrap();
    let official_stage = tree
        .children
        .iter()
        .find(|node| node.stage_id.as_deref() == Some("official-stage"))
        .unwrap();
    assert_eq!(official_stage.node_id, "stage:official-stage");
    assert_eq!(official_stage.title, "official-stage");
    assert_eq!(
        official_stage.children[0].chapter_id.as_deref(),
        Some("misleading-prefix:chapter-1")
    );
    assert_eq!(
        official_stage.children[0].document_id.as_deref(),
        Some("documents/chapter-1.md")
    );
    assert_eq!(
        official_stage.children[0].stage_id.as_deref(),
        Some("official-stage")
    );
    assert!(!tree
        .children
        .iter()
        .any(|node| node.stage_id.as_deref() == Some("misleading-prefix")));

    let unassigned = tree
        .children
        .iter()
        .find(|node| node.node_id == "stage:__unassigned__")
        .unwrap();
    assert_eq!(unassigned.title, "ui.works.unassigned_stage");
    assert!(unassigned.path.as_os_str().is_empty());
    assert_eq!(
        unassigned.children[0].chapter_id.as_deref(),
        Some("chapter-2")
    );
    assert_eq!(
        unassigned.children[0].document_id.as_deref(),
        Some("documents/chapter-2.md")
    );

    let error = build_works_tree(
        &index,
        &BTreeMap::from([("missing-chapter".to_owned(), "stage-x".to_owned())]),
        "planning",
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("references missing chapter index entry"));
}

#[test]
fn import_chapter_rejects_source_outside_project_root() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let service = test_document_service(temp.path());
    let source = outside.path().join("source.md");
    std::fs::write(&source, "泄露正文").unwrap();
    let target = temp.path().join("documents").join("chapter1.md");

    let error = import_chapter_document(
        &service,
        ChapterImportRequest {
            chapter_id: "stage1:chapter1".to_owned(),
            title: "第一章".to_owned(),
            order: 1,
            source_path: source,
            target_path: target.clone(),
            overwrite: false,
            outline_ref: None,
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("permission denied"));
    assert!(!target.exists());
}

#[test]
fn import_chapter_requires_explicit_overwrite_and_preserves_existing_target() {
    let temp = tempfile::tempdir().unwrap();
    let service = test_document_service(temp.path());
    let source = temp.path().join("source.md");
    let target = temp.path().join("documents").join("chapter1.md");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&source, "新正文").unwrap();
    std::fs::write(&target, "旧正文").unwrap();

    let request = ChapterImportRequest {
        chapter_id: "chapter1".to_owned(),
        title: "第一章".to_owned(),
        order: 1,
        source_path: source,
        target_path: target.clone(),
        overwrite: false,
        outline_ref: None,
    };
    let error = import_chapter_document(&service, request.clone()).unwrap_err();
    assert!(matches!(
        error,
        ariadne::contracts::CoreError::DocumentAlreadyExists { .. }
    ));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "旧正文");

    let imported = import_chapter_document(
        &service,
        ChapterImportRequest {
            overwrite: true,
            ..request
        },
    )
    .unwrap();
    assert_eq!(imported.entry.chapter_id, "chapter1");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "新正文");
}

#[test]
fn works_service_exports_epub_and_pdf_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let service = test_document_service(temp.path());
    let chapter_path = temp.path().join("documents").join("chapter1.md");
    let chapter_body = std::iter::once("第一章正文".to_owned())
        .chain((1..=80).map(|line| format!("第{line:02}行中文内容")))
        .collect::<Vec<_>>()
        .join("\n");
    service
        .save_document(DocumentWriteRequest {
            path: chapter_path.clone(),
            content: chapter_body,
            format: None,
            base_version: None,
        })
        .unwrap();
    let index = ChapterDocumentIndex::new(
        "v1",
        vec![ChapterDocumentEntry {
            chapter_id: "stage1:chapter1".to_owned(),
            document_id: chapter_path.to_string_lossy().to_string(),
            path: chapter_path,
            title: "第一章".to_owned(),
            order: 1,
            kind: ChapterDocumentKind::ChapterBody,
            version: "v1".to_owned(),
            word_count: Some(6),
            outline_ref: None,
        }],
    )
    .unwrap();

    let epub = export_chapters_combined(
        &service,
        &index,
        &["stage1:chapter1".to_owned()],
        "exports/book.epub",
        ChapterExportFormat::Epub,
    )
    .unwrap();
    let pdf = export_chapters_combined(
        &service,
        &index,
        &["stage1:chapter1".to_owned()],
        "exports/book.pdf",
        ChapterExportFormat::Pdf,
    )
    .unwrap();
    let epub_bytes =
        std::fs::read(temp.path().join(".runtime/artifacts/exports/book.epub")).unwrap();
    let pdf_bytes = std::fs::read(temp.path().join(".runtime/artifacts/exports/book.pdf")).unwrap();

    assert_eq!(epub.format, ChapterExportFormat::Epub);
    assert_eq!(pdf.format, ChapterExportFormat::Pdf);
    assert!(epub_bytes.starts_with(b"PK\x03\x04"));
    assert!(epub_bytes
        .windows("OEBPS/content.opf".len())
        .any(|window| window == b"OEBPS/content.opf"));
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF\n"));
    let pdf_text = String::from_utf8_lossy(&pdf_bytes);
    assert!(pdf_text.contains("/Subtype /Type0"));
    assert!(pdf_text.contains("/BaseFont /STSong-Light"));
    assert!(pdf_text.contains("/Count 2"));
    assert!(pdf_text.contains(&utf16be_hex("第一章正文")));
}

fn utf16be_hex(value: &str) -> String {
    let mut encoded = String::new();
    for unit in value.encode_utf16() {
        encoded.push_str(&format!("{unit:04X}"));
    }
    encoded
}

#[test]
fn node_detail_patch_annotations_and_preferences_are_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let mut workflow = WorkflowDefinition {
        id: WorkflowId::from("wf"),
        name: "Workflow".to_owned(),
        nodes: vec![node("source"), node("writer")],
        edges: vec![Edge {
            id: EdgeId::from("source-writer"),
            kind: ariadne::contracts::WorkflowEdgeKind::Data,
            from: PortEndpoint {
                node_id: NodeId::from("source"),
                port_name: "data-out".to_owned(),
            },
            to: PortEndpoint {
                node_id: NodeId::from("writer"),
                port_name: "data-in-in1".to_owned(),
            },
            alias: Some("in1".to_owned()),
            communication: None,
        }],
        metadata: Value::Null,
    };
    apply_node_detail_patch(
        &mut workflow,
        NodeDetailPatch {
            node_id: NodeId::from("writer"),
            prompt_template: Some("template-a".to_owned()),
            input_aliases: BTreeMap::from([("in1".to_owned(), "上一章".to_owned())]),
            tool_enabled: BTreeMap::from([("writer-search".to_owned(), false)]),
            approval_policy: BTreeMap::from([("writer_patch".to_owned(), "auto_audit".to_owned())]),
            model_id: Some("model-a".to_owned()),
            budget_usd: Some(1.0),
            timeout_ms: Some(1_000),
        },
    )
    .unwrap();
    upsert_canvas_annotation(
        &mut workflow,
        CanvasAnnotation {
            annotation_id: "group-1".to_owned(),
            title: "第一组".to_owned(),
            node_ids: vec![NodeId::from("writer")],
            metadata: Value::Null,
        },
    )
    .unwrap();
    let prefs = UiPreferences {
        theme: "dark".to_owned(),
        git_manual_color: "#ff9900".to_owned(),
        panel_states: [("workspace.right_panel".to_owned(), false)]
            .into_iter()
            .collect(),
        ..UiPreferences::default()
    };
    let prefs_store = UiPreferencesStore::default_for_project(temp.path());
    prefs_store.write(&prefs).unwrap();

    let writer = workflow
        .nodes
        .iter()
        .find(|node| node.id == NodeId::from("writer"))
        .unwrap();
    assert_eq!(writer.config["prompt_template"], "template-a");
    assert_eq!(workflow.edges[0].alias.as_deref(), Some("上一章"));
    assert_eq!(workflow.edges[0].to.port_name, "data-in-上一章");
    assert!(workflow.metadata["canvas_annotations"].is_array());
    assert_eq!(prefs_store.read().unwrap().theme, "dark");
    assert!(!prefs_store.read().unwrap().panel_states["workspace.right_panel"]);
}

#[test]
fn ui_preferences_locale_migrates_to_app_scope_and_stays_global() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    initialize_project(project_a.path()).unwrap();
    initialize_project(project_b.path()).unwrap();

    let store_a = ConfigStore::with_app_state(project_a.path(), app_state.path());
    let mut config_a = store_a.load().unwrap();
    config_a.app.locale = "fr".to_owned();
    store_a.save(&config_a).unwrap();

    let migrated =
        UiPreferencesStore::read_global_or_migrate(app_state.path(), Some(project_a.path()))
            .unwrap();
    assert_eq!(migrated.locale, "fr");
    assert!(app_state.path().join("ui_preferences.json").is_file());

    let store_b = ConfigStore::with_app_state(project_b.path(), app_state.path());
    let mut config_b = store_b.load().unwrap();
    config_b.app.locale = "ja".to_owned();
    store_b.save(&config_b).unwrap();

    let from_other_project =
        UiPreferencesStore::read_global_or_migrate(app_state.path(), Some(project_b.path()))
            .unwrap();
    assert_eq!(from_other_project.locale, "fr");
    assert!(!project_a
        .path()
        .join(".runtime/ui_preferences.json")
        .exists());
    assert!(!project_b
        .path()
        .join(".runtime/ui_preferences.json")
        .exists());
}

#[test]
fn ui_preferences_reject_invalid_global_locale() {
    let temp = tempfile::tempdir().unwrap();
    let store = UiPreferencesStore::new(temp.path().join("ui_preferences.json"));
    let error = store
        .write(&UiPreferences {
            locale: "../../project".to_owned(),
            ..UiPreferences::default()
        })
        .unwrap_err();
    assert!(error.to_string().contains("locale must be"));
}

fn node(id: &str) -> NodeInstance {
    NodeInstance {
        id: NodeId::from(id),
        type_name: "writer".to_owned(),
        label: None,
        config: Value::Null,
        position: None,
    }
}

fn control_edge(id: &str, from: &str, to: &str) -> Edge {
    Edge {
        id: EdgeId::from(id),
        kind: WorkflowEdgeKind::Control,
        from: PortEndpoint {
            node_id: NodeId::from(from),
            port_name: EXECUTION_OUTPUT_PORT.to_owned(),
        },
        to: PortEndpoint {
            node_id: NodeId::from(to),
            port_name: EXECUTION_INPUT_PORT.to_owned(),
        },
        alias: None,
        communication: None,
    }
}

fn data_edge(id: &str, from: &str, to: &str, alias: &str) -> Edge {
    Edge {
        id: EdgeId::from(id),
        kind: WorkflowEdgeKind::Data,
        from: PortEndpoint {
            node_id: NodeId::from(from),
            port_name: format!("out-{alias}"),
        },
        to: PortEndpoint {
            node_id: NodeId::from(to),
            port_name: format!("in-{alias}"),
        },
        alias: Some(alias.to_owned()),
        communication: None,
    }
}

fn workflow_manifest(id: &str, version: &str) -> WorkflowManifest {
    WorkflowManifest {
        workflow_id: id.to_owned(),
        name: "Market Basic".to_owned(),
        version: version.to_owned(),
        workflow: WorkflowDefinition {
            id: WorkflowId::from(id),
            name: "Market Basic".to_owned(),
            nodes: vec![node("writer")],
            edges: Vec::new(),
            metadata: Value::Null,
        },
        prompt_templates: Vec::new(),
        required_node_types: vec!["writer".to_owned()],
        required_tools: Vec::new(),
        required_permissions: vec!["http_skill".to_owned()],
        minimum_ariadne_version: None,
        metadata: Value::Null,
    }
}

fn test_document_service(root: &std::path::Path) -> FileDocumentService {
    FileDocumentService::new(
        project_document_permission(root),
        root.join(".runtime").join("artifacts"),
    )
}

/// D4：偏好 / 项目记忆 / 最近项目 覆盖写必须走 atomic_write（临时文件 + rename）。
#[test]
fn d4_durable_frontend_writes_use_atomic_replace_not_bare_overwrite() {
    use ariadne::frontend::{
        ProjectMemoryStore, ProjectRegistryStore, UiPreferences, UiPreferencesStore,
    };
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    let temp = tempfile::tempdir().unwrap();

    // Preferences
    let prefs_store = UiPreferencesStore::default_for_project(temp.path());
    prefs_store
        .write(&UiPreferences {
            theme: "dark".to_owned(),
            ..UiPreferences::default()
        })
        .unwrap();
    let prefs_path = temp.path().join(".runtime/ui_preferences.json");
    assert!(prefs_path.is_file());
    assert!(fs::read_to_string(&prefs_path).unwrap().contains("dark"));
    // No leftover .tmp from successful atomic write
    let runtime = temp.path().join(".runtime");
    let leftovers: Vec<_> = fs::read_dir(&runtime)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "tmp leftovers: {leftovers:?}");

    // Project memory overwrite
    let memory = ProjectMemoryStore::default_for_project(temp.path());
    memory.write_all("记忆甲").unwrap();
    memory.write_all("记忆乙-覆盖").unwrap();
    assert_eq!(memory.read_all().unwrap(), "记忆乙-覆盖");

    // Recent projects
    let recent = ProjectRegistryStore::default_for_project(temp.path());
    recent
        .record_opened("demo", temp.path().join("proj-a"))
        .unwrap();
    let recent_path = temp.path().join(".runtime/recent_projects.json");
    assert!(fs::read_to_string(&recent_path).unwrap().contains("demo"));
    // inode change on second write proves replace (atomic rename) rather than in-place truncate
    let ino1 = fs::metadata(&recent_path).unwrap().ino();
    recent
        .record_opened("demo2", temp.path().join("proj-b"))
        .unwrap();
    let ino2 = fs::metadata(&recent_path).unwrap().ino();
    assert_ne!(ino1, ino2, "atomic rename should replace inode on POSIX");
}

/// D4 residual: product `append_project_memory` must not bare-append in place;
/// crash mid-write must not leave a half-line (read-modify-atomic_write).
#[test]
fn d4_project_memory_append_uses_atomic_replace_not_in_place() {
    use ariadne::frontend::ProjectMemoryStore;
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    let temp = tempfile::tempdir().unwrap();
    let memory = ProjectMemoryStore::default_for_project(temp.path());
    let path = temp.path().join(".runtime/project_memory.md");

    memory.write_all("第一行\n").unwrap();
    let ino1 = fs::metadata(&path).unwrap().ino();

    let after = memory.append("第二行").unwrap();
    assert!(after.contains("第一行\n第二行\n"), "got: {after:?}");
    let ino2 = fs::metadata(&path).unwrap().ino();
    assert_ne!(
        ino1, ino2,
        "append must replace via atomic rename, not OpenOptions in-place append"
    );

    // Re-entry: second append still preserves history and remains atomic
    let ino3 = fs::metadata(&path).unwrap().ino();
    let after2 = memory.append("第三行").unwrap();
    assert!(after2.contains("第二行\n第三行\n"), "got: {after2:?}");
    let ino4 = fs::metadata(&path).unwrap().ino();
    assert_ne!(ino3, ino4);

    // Failure path: oversize append must not destroy prior content (limit is 4 MiB)
    let huge = "x".repeat((4 * 1024 * 1024) + 64);
    let err = memory.append(&huge).expect_err("oversize append must fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("project_memory") || msg.contains("exceed"),
        "unexpected error: {msg}"
    );
    assert_eq!(
        memory.read_all().unwrap(),
        after2,
        "failed append must leave prior memory intact"
    );
}

#[test]
fn project_ai_conversation_store_compacts_with_revision_and_structured_memory() {
    let temp = tempfile::tempdir().unwrap();
    let store = ProjectAiConversationStore::open(temp.path()).unwrap();
    let seed = (0..60)
        .map(|index| {
            (
                if index % 2 == 0 {
                    "user".to_owned()
                } else {
                    "assistant".to_owned()
                },
                format!("历史消息 {index} C4-SUMMARY-SENTINEL"),
            )
        })
        .collect::<Vec<_>>();
    let snapshot = store.load_or_seed("works", &seed).unwrap();
    assert_eq!(snapshot.revision, 0);
    assert!(snapshot.messages.len() <= 48);
    assert!(!store
        .select_summary_chunks("works", "C4-SUMMARY-SENTINEL", 8)
        .unwrap()
        .is_empty());

    let saved = store
        .append_messages(
            "works",
            snapshot.revision,
            &[
                ("user".to_owned(), "新问题".to_owned()),
                ("assistant".to_owned(), "新回答".to_owned()),
            ],
        )
        .unwrap();
    let ProjectAiAppendOutcome::Saved {
        snapshot: saved_snapshot,
        appended,
    } = saved
    else {
        panic!("conversation append must save at matching revision");
    };
    assert_eq!(saved_snapshot.revision, 1);
    assert_eq!(appended.len(), 2);
    assert!(matches!(
        store.append_messages("works", 0, &[("user".to_owned(), "stale".to_owned())]),
        Ok(ProjectAiAppendOutcome::RevisionConflict { actual_revision: 1 })
    ));

    let memory_version = store
        .synchronize_project_memory("叙事视角：第三人称\n时代：近未来")
        .unwrap();
    let memory = store.select_project_memory("第三人称", 8).unwrap();
    assert_eq!(memory.len(), 1);
    assert_eq!(memory[0].logical_key, "叙事视角");
    assert_eq!(memory[0].source, "project_memory.md");
    assert_eq!(memory[0].source_version, memory_version);
    let entity_id = memory[0].entity_id.clone();
    let updated_memory_version = store
        .synchronize_project_memory("前言：保留\n叙事视角：第一人称\n时代：近未来")
        .unwrap();
    let updated_memory = store.select_project_memory("第一人称", 8).unwrap();
    assert_eq!(updated_memory.len(), 1);
    assert_eq!(updated_memory[0].entity_id, entity_id);
    assert_ne!(updated_memory[0].source_version, memory_version);
    assert_eq!(updated_memory[0].source_version, updated_memory_version);
    assert_eq!(updated_memory[0].source_line, 2);

    let reopened = ProjectAiConversationStore::open(temp.path()).unwrap();
    assert_eq!(reopened.load("works").unwrap().revision, 1);
    assert!(!reopened
        .select_summary_chunks("works", "C4-SUMMARY-SENTINEL", 8)
        .unwrap()
        .is_empty());
}

#[test]
fn project_ai_context_policy_bounds_history_without_orphan_assistant_turn() {
    let mut history = Vec::new();
    for index in 0..8 {
        history.push(ProjectAiChatMessage {
            role: ProjectAiChatRole::User,
            content: format!("user-{index}-{}", "u".repeat(400)),
        });
        history.push(ProjectAiChatMessage {
            role: ProjectAiChatRole::Assistant,
            content: format!("assistant-{index}-{}", "a".repeat(400)),
        });
    }

    let window = project_ai_context_window(
        &[],
        &[],
        &[],
        &history,
        "current question",
        Some(2_048),
        Some(256),
    )
    .unwrap();

    assert!(window.history_truncated);
    assert!(window.history.len() < history.len());
    assert_ne!(
        window.history.first().map(|message| message.role),
        Some(ProjectAiChatRole::Assistant)
    );
    assert!(window.estimated_input_tokens <= u64::from(window.context_limit_tokens));
}

#[test]
fn project_ai_conversation_guard_serializes_provider_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let first = ProjectAiConversationStore::try_acquire_conversation(temp.path(), "works")
        .unwrap()
        .expect("first caller must claim the conversation");
    assert!(
        ProjectAiConversationStore::try_acquire_conversation(temp.path(), "works")
            .unwrap()
            .is_none(),
        "a competing caller must not reach provider dispatch"
    );
    assert!(
        ProjectAiConversationStore::try_acquire_conversation(temp.path(), "workspace")
            .unwrap()
            .is_some(),
        "independent conversations must remain concurrent"
    );
    drop(first);
    assert!(
        ProjectAiConversationStore::try_acquire_conversation(temp.path(), "works")
            .unwrap()
            .is_some(),
        "the OS lock must be released with the request guard"
    );
}
