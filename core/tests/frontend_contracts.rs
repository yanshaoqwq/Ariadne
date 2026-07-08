use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;

use ariadne::contracts::{
    ArtifactKind, Edge, EdgeId, NodeId, NodeInstance, PatchHunk, PermissionPolicy, PortEndpoint,
    PortValue, ProviderCapability, ProviderDefinition, ProviderType, RunId, SourceSpan, TextRange,
    WorkflowDefinition, WorkflowEdgeKind, WorkflowId, EXECUTION_INPUT_PORT, EXECUTION_OUTPUT_PORT,
};
use ariadne::costs::{BudgetLimits, SqliteCostLedger};
use ariadne::diagnostics::{BackendDiagnosticsReport, DiagnosticStatus};
use ariadne::documents::{
    ChapterDocumentEntry, ChapterDocumentIndex, ChapterDocumentKind, DocumentReadRequest,
    DocumentRepository, DocumentWriteRequest, FileDocumentService,
};
use ariadne::frontend::{
    apply_node_detail_patch, apply_quick_edit_patch, build_works_tree, export_chapters_combined,
    export_chapters_markdown, export_workflow_selection, import_chapter_document,
    initialize_project, install_workflow_template_manifest, node_has_breakpoint, now_timestamp_ms,
    pack_workflow_selection, project_document_permission, quick_edit_to_patch, set_node_breakpoint,
    upsert_canvas_annotation, ArtifactReferenceEntry, CanvasAnnotation, ChapterExportFormat,
    ChapterImportRequest, ConfirmationLogEntry, ConfirmationLogState, ConfirmationLogStore,
    FileConfirmationLogStore, NodeDetailPatch, ProjectMemoryStore, ProjectReferenceKind,
    ProjectReferenceResolver, ProjectRegistryStore, QuickEditService, TemplateRepositoryClient,
    UiPreferences, UiPreferencesStore, UiRunLogEntry, UiRunLogFilter, UiRunLogKind, UiRunLogLevel,
    UiRunLogStore,
};
use ariadne::llm::{LlmService, LlmServiceConfig};
use ariadne::providers::{
    LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext,
};
use ariadne::skills::{WorkflowManifest, WorkflowTemplateLoader};
use ariadne::workflow::{WorkflowNodeRuntimeState, WorkflowRunState};
use serde_json::{json, Value};

struct MockQuickEditProvider;

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

    assert!(report.git_initialized);
    assert!(project.join(".config").is_dir());
    assert!(project.join("planning/chapters").is_dir());
    assert!(project.join(".git").is_dir());
    assert_eq!(recent[0].name, "Novel");
    assert_eq!(registry.read_all().unwrap()[0].path, project);
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
    assert_eq!(
        resolver
            .resolve(&format!("@文档/{}", doc_path.display()))
            .unwrap()
            .kind,
        ProjectReferenceKind::Document
    );
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
            outline_ref: None,
        },
    )
    .unwrap();
    let index = ChapterDocumentIndex::new("v1", vec![import.entry.clone()]).unwrap();
    let tree = build_works_tree(&index, temp.path().join("planning")).unwrap();
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
            outline_ref: None,
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("permission denied"));
    assert!(!target.exists());
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
    assert_eq!(
        prefs_store.read().unwrap().panel_states["workspace.right_panel"],
        false
    );
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
