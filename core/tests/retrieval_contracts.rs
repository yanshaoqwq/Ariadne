use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use ariadne::config::{
    ConfigStore, MemorySecretStore, ModelConfig, ProjectCredentialScope, ProviderConfig,
    SecretValue, VectorStoreBackend,
};
use ariadne::contracts::{
    CoreError, CoreResult, ExecutionCancellation, ExternalDispatchAuthorization,
    ExternalDispatchOutcome, ProviderCapability, ProviderType, SourceSpan, TextRange,
};
use ariadne::documents::IndexInvalidationOutbox;
use ariadne::providers::ProviderCallContext;
use ariadne::rag::{
    MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, StoryEvent, StoryEventStatus,
    StorySegment,
};
use ariadne::retrieval::{
    recover_retrieval_components, select_available_port, ChunkDocument, FullTextRecord,
    FullTextSearchRequest, HybridSearchEngine, HybridSearchRequest, IndexingWorker,
    KnowledgeIndexSynchronizer, MemoryFullTextStore, MemoryVectorStore, ProjectRetrievalRuntime,
    RebuildStatus, RetrievalRecoveryAction, RetrievalSource, SidecarState, SqliteFullTextStore,
    StoreHealth, StoreStatus, TantivyFullTextStore, TextEmbedder, ThreeWayHybridSearchEngine,
    VectorRecord, VectorSearchRequest, MAX_HYBRID_SEARCH_LIMIT,
};
use ariadne::retrieval::{
    FullTextStore, HybridSearch, QdrantSidecarConfig, QdrantSidecarSupervisor, QdrantVectorStore,
    SidecarProcessRunner, VectorStore,
};
use rusqlite::Connection;
use std::process::{Child, Command};

#[test]
fn indexing_worker_consumes_outbox_and_preserves_utf8_source_versions() {
    let temp = tempfile::tempdir().unwrap();
    let document = temp.path().join("chapter.md");
    let content = "第一幕银色线索。第二幕人物重逢。第三幕真相揭晓。";
    std::fs::write(&document, content).unwrap();
    let document_id = document
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let source_version = test_content_version(content.as_bytes());
    let outbox = IndexInvalidationOutbox::new(temp.path().join("outbox.db"));
    let event_id = outbox
        .prepare(&document_id, "document_saved", &source_version, false)
        .unwrap();
    outbox.activate(&event_id).unwrap();
    let tantivy = Arc::new(MemoryFullTextStore::new());
    let sqlite = Arc::new(MemoryFullTextStore::new());
    let worker =
        IndexingWorker::new(outbox.clone(), tantivy.clone(), sqlite.clone(), 8, 2).unwrap();

    let report = worker.process_next().unwrap().unwrap();

    assert!(report.indexed_chunks >= 3);
    assert_eq!(report.source_version, source_version);
    let results = tantivy
        .search(FullTextSearchRequest::new("人物", 10))
        .unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|result| {
        result
            .metadata
            .get("source_version")
            .and_then(|value| value.as_str())
            == Some(source_version.as_str())
            && result.spans.iter().all(|span| {
                span.version.as_deref() == Some(source_version.as_str())
                    && span.range.end as usize <= content.len()
            })
    }));
    assert!(outbox.pending().unwrap().is_empty());
}

#[test]
fn indexing_worker_supersedes_stale_save_without_blocking_latest_version() {
    let temp = tempfile::tempdir().unwrap();
    let document = temp.path().join("chapter.md");
    std::fs::write(&document, "旧版本").unwrap();
    let outbox = IndexInvalidationOutbox::new(temp.path().join("outbox.db"));
    let stale_id = outbox
        .prepare(
            document.to_str().unwrap(),
            "save",
            "0000000000000000",
            false,
        )
        .unwrap();
    outbox.activate(&stale_id).unwrap();

    std::fs::write(&document, "最新线索").unwrap();
    let latest_version = test_content_version("最新线索".as_bytes());
    let latest_id = outbox
        .prepare(document.to_str().unwrap(), "save", &latest_version, false)
        .unwrap();
    outbox.activate(&latest_id).unwrap();

    let tantivy = Arc::new(MemoryFullTextStore::new());
    let sqlite = Arc::new(MemoryFullTextStore::new());
    let worker = IndexingWorker::new(outbox.clone(), tantivy.clone(), sqlite, 8, 2).unwrap();
    let report = worker.process_next().unwrap().unwrap();

    assert_eq!(report.event_id, latest_id);
    assert_eq!(report.source_version, latest_version);
    assert!(outbox.pending().unwrap().is_empty());
    let results = tantivy
        .search(FullTextSearchRequest::new("线索", 10))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].snippet.contains("最新线索"));
}

#[test]
fn full_rebuild_enqueue_is_atomic_idempotent_and_recovers_legacy_prepared_event() {
    let temp = tempfile::tempdir().unwrap();
    let outbox = IndexInvalidationOutbox::new(temp.path().join("outbox.db"));
    let legacy = outbox
        .prepare(temp.path().to_str().unwrap(), "legacy_rebuild", "v1", true)
        .unwrap();

    let recovered = outbox
        .enqueue_full_rebuild_if_absent(
            temp.path().to_str().unwrap(),
            "retrieval_configuration_changed",
            "v2",
        )
        .unwrap();
    assert_eq!(recovered.as_deref(), Some(legacy.as_str()));
    let pending = outbox.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event_id, legacy);

    let duplicate = outbox
        .enqueue_full_rebuild_if_absent(
            temp.path().to_str().unwrap(),
            "retrieval_configuration_changed",
            "v2",
        )
        .unwrap();
    assert!(duplicate.is_none());
    assert_eq!(outbox.pending().unwrap().len(), 1);
}

#[test]
fn indexing_worker_executes_project_full_rebuild_event() {
    let temp = tempfile::tempdir().unwrap();
    let documents = temp.path().join("documents");
    std::fs::create_dir_all(&documents).unwrap();
    std::fs::write(documents.join("chapter.md"), "回档后的中文线索").unwrap();
    let outbox = IndexInvalidationOutbox::new(temp.path().join("outbox.db"));
    let event_id = outbox
        .prepare(
            temp.path().to_str().unwrap(),
            "git_restore_full_rebuild",
            "commit-1",
            true,
        )
        .unwrap();
    outbox.activate(&event_id).unwrap();
    let tantivy = Arc::new(MemoryFullTextStore::new());
    let sqlite = Arc::new(MemoryFullTextStore::new());
    let worker = IndexingWorker::new(outbox.clone(), tantivy.clone(), sqlite, 8, 2).unwrap();

    let report = worker.process_next().unwrap().unwrap();

    assert_eq!(report.event_id, event_id);
    assert!(report.indexed_chunks > 0);
    assert!(!report.superseded);
    assert!(outbox.pending().unwrap().is_empty());
    assert!(!tantivy
        .search(FullTextSearchRequest::new("线索", 10))
        .unwrap()
        .is_empty());
}

/// F1 测试夹具：显式模拟 provider embedding，不允许 worker 自行生成哈希向量。
struct TestTextEmbedder {
    calls: Arc<AtomicUsize>,
    dimensions: usize,
}

impl TextEmbedder for TestTextEmbedder {
    fn provider_id(&self) -> &str {
        "test-embedding"
    }

    fn model_id(&self) -> &str {
        "test-embedding-model"
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(
        &self,
        _context: ProviderCallContext,
        inputs: Vec<String>,
    ) -> CoreResult<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(inputs
            .into_iter()
            .map(|_| {
                let mut vector = vec![0.0; self.dimensions];
                vector[0] = 1.0;
                vector
            })
            .collect())
    }

    fn health_check(&self) -> CoreResult<StoreHealth> {
        Ok(StoreHealth::healthy("test_embedding"))
    }
}

/// F1：配置 VectorStore 时 worker 必须调用 TextEmbedder 并 upsert 真实向量。
#[test]
fn indexing_worker_upserts_vector_store_when_configured() {
    let temp = tempfile::tempdir().unwrap();
    let document = temp.path().join("chapter.md");
    let content = "可检索的中文线索段落。";
    std::fs::write(&document, content).unwrap();
    let document_id = document
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let source_version = test_content_version(content.as_bytes());
    let outbox = IndexInvalidationOutbox::new(temp.path().join("outbox.db"));
    let event_id = outbox
        .prepare(&document_id, "document_saved", &source_version, false)
        .unwrap();
    outbox.activate(&event_id).unwrap();

    let tantivy = Arc::new(MemoryFullTextStore::new());
    let sqlite = Arc::new(MemoryFullTextStore::new());
    let vector = Arc::new(MemoryVectorStore::new());
    let embedding_calls = Arc::new(AtomicUsize::new(0));
    let embedder = Arc::new(TestTextEmbedder {
        calls: Arc::clone(&embedding_calls),
        dimensions: 8,
    });
    let worker = IndexingWorker::with_vector_store(
        outbox.clone(),
        tantivy.clone(),
        sqlite,
        vector.clone(),
        embedder,
        32,
        4,
    )
    .unwrap();

    let report = worker.process_next().unwrap().unwrap();
    assert!(report.vector_indexed, "vector path must report indexed");
    assert!(report.indexed_chunks > 0);
    assert_eq!(embedding_calls.load(Ordering::SeqCst), 1);
    assert!(!vector
        .search(VectorSearchRequest::new(
            vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            10,
        ))
        .unwrap()
        .is_empty());
    let health = vector.health_check().unwrap();
    assert_eq!(health.status, StoreStatus::Healthy);
    // 删除文档后向量侧同步清空
    let _ = vector.delete_document(&document_id).unwrap();

    // 未配置向量时不写向量、不报 vector_indexed
    let event_id2 = outbox
        .prepare(&document_id, "document_saved", &source_version, false)
        .unwrap();
    outbox.activate(&event_id2).unwrap();
    let worker_ft =
        IndexingWorker::new(outbox, tantivy, Arc::new(MemoryFullTextStore::new()), 32, 4).unwrap();
    let report_ft = worker_ft.process_next().unwrap().unwrap();
    assert!(!report_ft.vector_indexed);
}

#[test]
fn confirmed_four_layer_knowledge_is_versioned_into_full_text_and_vector_indexes() {
    let project = tempfile::tempdir().unwrap();
    let chapter_path = project.path().join("documents").join("chapter.md");
    std::fs::create_dir_all(chapter_path.parent().unwrap()).unwrap();
    std::fs::write(&chapter_path, "角色在旧城听见异常声响").unwrap();
    let chapter_document_id = chapter_path.to_string_lossy().into_owned();
    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: "segment-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-1".to_owned(),
            summary: "角色听见量子回声".to_owned(),
            source: SourceSpan {
                document_id: chapter_document_id,
                range: TextRange::new(0, 6).unwrap(),
                version: Some("chapter-v1".to_owned()),
            },
            metadata: serde_json::Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_event(StoryEvent {
            event_id: "event-1".to_owned(),
            summary: "量子回声揭示旧城真相".to_owned(),
            status: StoryEventStatus::Ongoing,
            segment_ids: vec!["segment-1".to_owned()],
            chapter_ids: vec!["chapter-1".to_owned()],
            metadata: serde_json::Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_chapter_summary("chapter-1", "本章围绕量子回声推进")
        .unwrap();
    knowledge
        .upsert_stage_summary("stage-1", "本阶段解开量子回声来源")
        .unwrap();
    knowledge
        .link_chapter_stage("chapter-1", "stage-1")
        .unwrap();
    SqliteWritingKnowledgeStore::open(project.path())
        .unwrap()
        .save_knowledge(&knowledge)
        .unwrap();

    let tantivy: Arc<dyn FullTextStore> = Arc::new(MemoryFullTextStore::new());
    let sqlite: Arc<dyn FullTextStore> = Arc::new(MemoryFullTextStore::new());
    let vector: Arc<dyn VectorStore> = Arc::new(MemoryVectorStore::new());
    let embedder: Arc<dyn TextEmbedder> = Arc::new(TestTextEmbedder {
        calls: Arc::new(AtomicUsize::new(0)),
        dimensions: 8,
    });
    let synchronizer = KnowledgeIndexSynchronizer::new(project.path()).unwrap();

    let report = synchronizer
        .sync(
            &tantivy,
            &sqlite,
            Some(&vector),
            Some(&embedder),
            Some("test-vector-v1"),
            None,
        )
        .unwrap();

    assert!(report.changed);
    assert_eq!(report.indexed_records, 4);
    let text_results = tantivy
        .search(FullTextSearchRequest::new("量子回声", 10))
        .unwrap();
    assert_eq!(text_results.len(), 4);
    assert!(text_results.iter().all(|result| {
        result.metadata["confirmed"] == serde_json::json!(true)
            && result.metadata["knowledge_revision"] == serde_json::json!(report.revision)
    }));
    let vector_results = vector
        .search(VectorSearchRequest::new(
            vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            10,
        ))
        .unwrap();
    assert_eq!(vector_results.len(), 4);

    let unchanged = synchronizer
        .sync(
            &tantivy,
            &sqlite,
            Some(&vector),
            Some(&embedder),
            Some("test-vector-v1"),
            None,
        )
        .unwrap();
    assert!(!unchanged.changed);
}

#[test]
fn project_runtime_searches_confirmed_knowledge_when_original_text_lacks_the_term() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let chapter_path = project.path().join("documents").join("chapter.md");
    std::fs::write(&chapter_path, "角色在旧城听见异常声响").unwrap();
    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: "segment-product".to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-product".to_owned(),
            summary: "角色确认异常声响是量子回声".to_owned(),
            source: SourceSpan {
                document_id: chapter_path.to_string_lossy().into_owned(),
                range: TextRange::new(0, 6).unwrap(),
                version: Some("chapter-product-v1".to_owned()),
            },
            metadata: serde_json::Value::Null,
        })
        .unwrap();
    SqliteWritingKnowledgeStore::open(project.path())
        .unwrap()
        .save_knowledge(&knowledge)
        .unwrap();

    let runtime =
        ProjectRetrievalRuntime::open(project.path(), &MemorySecretStore::default()).unwrap();
    let results = runtime
        .search(
            "量子回声".to_owned(),
            10,
            ProviderCallContext::new("project_retrieval"),
        )
        .unwrap();

    assert!(!results.is_empty());
    assert!(results.iter().all(|result| {
        result.document_id.starts_with("ariadne-knowledge://")
            && result.metadata["confirmed"] == serde_json::json!(true)
            && result.metadata["ariadne_retrieval"]["source_kind"] == serde_json::json!("knowledge")
    }));
}

#[test]
fn retrieval_configuration_change_blocks_search_until_full_rebuild_completes() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    std::fs::write(
        project.path().join("documents").join("chapter.md"),
        "配置重建后的线索",
    )
    .unwrap();
    let secrets = MemorySecretStore::default();
    let config = ConfigStore::new(project.path()).load_or_create().unwrap();
    let original =
        ProjectRetrievalRuntime::from_config(project.path(), &secrets, &config, None).unwrap();
    let mut changed = config.clone();
    changed.rag.chunk_size_chars = 512;
    changed.rag.chunk_overlap_chars = 64;
    assert!(ProjectRetrievalRuntime::index_configuration_changed(
        &config, &changed
    ));
    let runtime =
        ProjectRetrievalRuntime::from_config(project.path(), &secrets, &changed, Some(&original))
            .unwrap();

    assert!(runtime.enqueue_configuration_rebuild().unwrap().is_some());
    let error = runtime
        .search(
            "线索".to_owned(),
            10,
            ProviderCallContext::new("project_retrieval"),
        )
        .unwrap_err();
    assert!(error.to_string().contains("indexing_not_ready"));

    assert_eq!(runtime.process_outbox().unwrap(), 1);
    let results = runtime
        .search(
            "线索".to_owned(),
            10,
            ProviderCallContext::new("project_retrieval"),
        )
        .unwrap();
    assert!(!results.is_empty());
}

#[test]
fn project_runtime_health_reports_index_dead_letters_as_unavailable() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let outbox = IndexInvalidationOutbox::new(
        project
            .path()
            .join(".runtime")
            .join("index_invalidation.db"),
    );
    let event_id = outbox
        .enqueue(
            project.path().join("missing.md").to_str().unwrap(),
            "document_saved",
            "missing-version",
            false,
        )
        .unwrap();
    for attempt in 0..5 {
        assert_eq!(outbox.claim_next().unwrap().unwrap().event_id, event_id);
        let dead_letter = outbox.retry(&event_id).unwrap();
        if attempt < 4 {
            assert!(!dead_letter);
            outbox.clear_backoff(&event_id).unwrap();
        } else {
            assert!(dead_letter);
        }
    }
    let runtime =
        ProjectRetrievalRuntime::open(project.path(), &MemorySecretStore::default()).unwrap();

    let health = runtime.health_check().unwrap();

    assert!(health.iter().any(|item| {
        item.component == "retrieval_index_outbox"
            && item.status == StoreStatus::Unavailable
            && item.reason.as_deref() == Some("diagnostics.retrieval.outbox.dead_letter")
    }));
}

#[test]
fn vector_index_revision_tracks_embedding_semantics_not_unrelated_provider_fields() {
    let mut original = ariadne::config::ProjectConfig::default();
    original.rag.vector_store.enabled = true;
    original.rag.vector_store.backend = VectorStoreBackend::ExternalQdrant;
    original.providers.default_embedding_provider_id = Some("embedding".to_owned());
    original.providers.providers.push(ProviderConfig {
        provider_id: "embedding".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Embedding Provider".to_owned(),
        enabled: true,
        base_url: Some("http://127.0.0.1:18080/v1".to_owned()),
        api_key: None,
        models: vec![ModelConfig {
            model_id: "embed-v1".to_owned(),
            capability: ProviderCapability::Embedding,
            max_context_tokens: None,
            input_cost_per_million_tokens: Some(0.1),
            output_cost_per_million_tokens: None,
        }],
    });

    let mut renamed = original.clone();
    renamed.providers.providers[0].display_name = "仅修改显示名".to_owned();
    renamed.providers.providers[0].models[0].input_cost_per_million_tokens = Some(0.2);
    assert!(!ProjectRetrievalRuntime::index_configuration_changed(
        &original, &renamed
    ));

    let mut changed_model = original.clone();
    changed_model.providers.providers[0].models[0].model_id = "embed-v2".to_owned();
    assert!(ProjectRetrievalRuntime::index_configuration_changed(
        &original,
        &changed_model
    ));

    let mut changed_endpoint = original.clone();
    changed_endpoint.providers.providers[0].base_url = Some("http://127.0.0.1:18081/v1".to_owned());
    assert!(ProjectRetrievalRuntime::index_configuration_changed(
        &original,
        &changed_endpoint
    ));
}

/// U116：换 embedding 模型/端点必须与换 vector_store 一样被判定为「换了向量空间」。
/// 两者的判据必须同源，否则重开 runtime 的判定会漏掉只改 embedding 的那一半，
/// 让抱着旧 embedder 的 runtime 继续服务新索引。
#[test]
fn vector_pipeline_reuse_boundary_covers_embedding_identity_not_only_vector_store() {
    let mut original = ariadne::config::ProjectConfig::default();
    original.rag.vector_store.enabled = true;
    original.rag.vector_store.backend = VectorStoreBackend::ExternalQdrant;
    original.providers.default_embedding_provider_id = Some("embedding".to_owned());
    original.providers.providers.push(ProviderConfig {
        provider_id: "embedding".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Embedding Provider".to_owned(),
        enabled: true,
        base_url: Some("http://127.0.0.1:18080/v1".to_owned()),
        api_key: None,
        models: vec![ModelConfig {
            model_id: "embed-v1".to_owned(),
            capability: ProviderCapability::Embedding,
            max_context_tokens: None,
            input_cost_per_million_tokens: Some(0.1),
            output_cost_per_million_tokens: None,
        }],
    });

    // 只改 embedding 模型：vector_store 完全没动，但向量语义空间已经变了。
    let mut changed_model = original.clone();
    changed_model.providers.providers[0].models[0].model_id = "embed-v2".to_owned();
    assert_eq!(
        original.rag.vector_store, changed_model.rag.vector_store,
        "本用例的前提是 vector_store 逐字段相同，只有 embedding 身份变了"
    );
    assert!(
        ProjectRetrievalRuntime::index_configuration_changed(&original, &changed_model),
        "换 embedding 模型必须触发重建索引"
    );

    // 与重建判据同源：复用边界也必须认为这是不同的向量流水线。
    assert!(
        ProjectRetrievalRuntime::vector_pipeline_configuration_changed(&original, &changed_model),
        "复用边界漏掉了 embedding 身份，只改模型时旧 runtime 会被错误复用"
    );

    // 无关字段（显示名、单价）不得让复用边界失效，否则每次改价都白重开一次向量库。
    let mut renamed = original.clone();
    renamed.providers.providers[0].display_name = "仅修改显示名".to_owned();
    renamed.providers.providers[0].models[0].input_cost_per_million_tokens = Some(0.2);
    assert!(
        !ProjectRetrievalRuntime::vector_pipeline_configuration_changed(&original, &renamed),
        "无关字段变化不应判定为换了向量流水线"
    );
}

#[test]
fn project_runtime_executes_configured_embedding_qdrant_and_reranker_chain() {
    let qdrant_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let qdrant_address = qdrant_listener.local_addr().unwrap();
    let qdrant_call_count = Arc::new(AtomicUsize::new(0));
    let qdrant_server_call_count = Arc::clone(&qdrant_call_count);
    let qdrant_server = thread::spawn(move || {
        let mut requests = Vec::new();
        let mut indexed_payload = serde_json::Value::Null;
        for step in 0..9 {
            let (mut stream, _) = qdrant_listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            qdrant_server_call_count.fetch_add(1, Ordering::SeqCst);
            requests.push(request.clone());
            match step {
                0 => {
                    assert!(request.starts_with("GET /collections/product_chunks "));
                    write_json_response(&mut stream, 404, r#"{"status":"not found"}"#);
                }
                1 => {
                    assert!(request.starts_with("PUT /collections/product_chunks "));
                    assert!(request.contains("\"size\":2"));
                    write_json_response(&mut stream, 200, r#"{"result":true}"#);
                }
                2 => {
                    assert!(request
                        .starts_with("POST /collections/product_chunks/points/delete?wait=true "));
                    write_json_response(&mut stream, 200, r#"{"result":{"status":"completed"}}"#);
                }
                3 => {
                    assert!(request.starts_with("GET /collections/product_chunks "));
                    write_json_response(
                        &mut stream,
                        200,
                        r#"{"result":{"config":{"params":{"vectors":{"size":2,"distance":"Cosine"}}}}}"#,
                    );
                }
                4 => {
                    assert!(
                        request.starts_with("PUT /collections/product_chunks/points?wait=true ")
                    );
                    let body: serde_json::Value =
                        serde_json::from_str(http_request_body(&request)).unwrap();
                    indexed_payload = body["points"][0]["payload"].clone();
                    write_json_response(&mut stream, 200, r#"{"result":{"status":"completed"}}"#);
                }
                5 => {
                    assert!(request
                        .starts_with("POST /collections/product_chunks/points/delete?wait=true "));
                    write_json_response(&mut stream, 200, r#"{"result":{"status":"completed"}}"#);
                }
                6 => {
                    assert!(request.starts_with("GET /collections/product_chunks "));
                    write_json_response(
                        &mut stream,
                        200,
                        r#"{"result":{"config":{"params":{"vectors":{"size":2,"distance":"Cosine"}}}}}"#,
                    );
                }
                7 => {
                    assert!(
                        request.starts_with("PUT /collections/product_chunks/points?wait=true ")
                    );
                    write_json_response(&mut stream, 200, r#"{"result":{"status":"completed"}}"#);
                }
                8 => {
                    assert!(request.starts_with("POST /collections/product_chunks/points/search "));
                    let response = serde_json::json!({
                        "result": [{ "score": 0.88, "payload": indexed_payload }]
                    });
                    write_json_response(&mut stream, 200, &response.to_string());
                }
                _ => unreachable!(),
            }
        }
        requests
    });

    let provider_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let provider_base_url = format!("http://{}", provider_listener.local_addr().unwrap());
    let provider_call_count = Arc::new(AtomicUsize::new(0));
    let provider_server_call_count = Arc::clone(&provider_call_count);
    let provider_server = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..4 {
            let (mut stream, _) = provider_listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            provider_server_call_count.fetch_add(1, Ordering::SeqCst);
            requests.push(request.clone());
            if request.starts_with("POST /embeddings ") {
                write_json_response(
                    &mut stream,
                    200,
                    r#"{"model":"embed-product","data":[{"index":0,"embedding":[1.0,0.0]}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#,
                );
            } else {
                assert!(request.starts_with("POST /rerank "));
                write_json_response(
                    &mut stream,
                    200,
                    r#"{"model":"rerank-product","results":[{"index":0,"relevance_score":0.99}],"usage":{"total_tokens":1}}"#,
                );
            }
        }
        requests
    });

    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let mut config = ConfigStore::new(project.path()).load_or_create().unwrap();
    config.providers.providers = vec![ProviderConfig {
        provider_id: "retrieval-product".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Retrieval Product".to_owned(),
        enabled: true,
        base_url: Some(provider_base_url),
        api_key: None,
        models: vec![
            ModelConfig {
                model_id: "embed-product".to_owned(),
                capability: ProviderCapability::Embedding,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            },
            ModelConfig {
                model_id: "rerank-product".to_owned(),
                capability: ProviderCapability::Reranker,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            },
        ],
    }];
    config.providers.default_embedding_provider_id = Some("retrieval-product".to_owned());
    config.providers.default_reranker_provider_id = Some("retrieval-product".to_owned());
    config.rag.vector_store.enabled = true;
    config.rag.vector_store.backend = VectorStoreBackend::ExternalQdrant;
    config.rag.vector_store.collection = "product_chunks".to_owned();
    config.rag.vector_store.vector_dimensions = 2;
    config.rag.vector_store.sidecar.host = qdrant_address.ip().to_string();
    config.rag.vector_store.sidecar.port = qdrant_address.port();
    config.rag.reranker_enabled = true;
    ConfigStore::new(project.path()).save(&config).unwrap();
    let secrets = MemorySecretStore::default();
    ProjectCredentialScope::new(project.path(), &secrets)
        .unwrap()
        .set_provider_secret(
            "retrieval-product",
            SecretValue::new("retrieval-product-key"),
        )
        .unwrap();

    let chapter = project.path().join("documents").join("chapter.md");
    std::fs::create_dir_all(chapter.parent().unwrap()).unwrap();
    let content = "银色线索藏在旧钟楼";
    std::fs::write(&chapter, content).unwrap();
    let document_id = chapter
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let source_version = test_content_version(content.as_bytes());
    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: "segment-dispatch-fence".to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-dispatch-fence".to_owned(),
            summary: "旧钟楼线索已由知识层确认".to_owned(),
            source: SourceSpan {
                document_id: document_id.clone(),
                range: TextRange::new(0, content.len() as u64).unwrap(),
                version: Some(source_version.clone()),
            },
            metadata: serde_json::Value::Null,
        })
        .unwrap();
    SqliteWritingKnowledgeStore::open(project.path())
        .unwrap()
        .save_knowledge(&knowledge)
        .unwrap();
    let outbox = IndexInvalidationOutbox::new(
        project
            .path()
            .join(".runtime")
            .join("index_invalidation.db"),
    );
    let event_id = outbox
        .prepare(&document_id, "document_saved", &source_version, false)
        .unwrap();
    outbox.activate(&event_id).unwrap();

    let runtime = ProjectRetrievalRuntime::open(project.path(), &secrets).unwrap();
    assert_eq!(runtime.process_outbox().unwrap(), 1);
    assert_eq!(provider_call_count.load(Ordering::SeqCst), 1);
    assert_eq!(qdrant_call_count.load(Ordering::SeqCst), 5);

    let mut denied_context = ProviderCallContext::new("project_retrieval");
    denied_context.dispatch_authorization = ExternalDispatchAuthorization::new(|dispatch| {
        if dispatch {
            Err(CoreError::validation("dispatch denied"))
        } else {
            Ok(())
        }
    });
    let denied = runtime
        .search("钟楼线索".to_owned(), 5, denied_context)
        .unwrap_err();
    assert!(denied.to_string().contains("dispatch denied"));
    assert_eq!(provider_call_count.load(Ordering::SeqCst), 1);
    assert_eq!(qdrant_call_count.load(Ordering::SeqCst), 5);
    assert!(!project
        .path()
        .join(".indexes")
        .join("knowledge-index-manifest.json")
        .exists());
    assert!(!project
        .path()
        .join(".indexes")
        .join("knowledge-index-rebuild-required.json")
        .exists());

    let results = runtime
        .search(
            "钟楼线索".to_owned(),
            5,
            ProviderCallContext::new("project_retrieval"),
        )
        .unwrap();

    let qdrant_requests = qdrant_server.join().unwrap();
    let provider_requests = provider_server.join().unwrap();
    assert_eq!(qdrant_requests.len(), 9);
    assert_eq!(
        provider_requests
            .iter()
            .filter(|request| request.starts_with("POST /embeddings "))
            .count(),
        3
    );
    assert!(provider_requests
        .iter()
        .any(|request| request.starts_with("POST /rerank ")));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].document_id, document_id);
    assert_eq!(results[0].score, 0.99);
}

fn test_content_version(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[test]
fn vector_and_full_text_stores_return_referenced_results() {
    let vector = MemoryVectorStore::new();
    let full_text = MemoryFullTextStore::new();
    let chunk = ChunkDocument::new("chunk-1", "doc-1", "Ariadne follows a silver thread");

    vector
        .upsert(vec![VectorRecord {
            chunk: chunk.clone(),
            embedding: vec![1.0, 0.0, 0.0],
        }])
        .unwrap();
    full_text
        .upsert(vec![FullTextRecord {
            chunk: chunk.clone(),
        }])
        .unwrap();

    let vector_results = vector
        .search(VectorSearchRequest::new(vec![1.0, 0.0, 0.0], 5))
        .unwrap();
    let text_results = full_text
        .search(FullTextSearchRequest::new("silver thread", 5))
        .unwrap();

    assert_eq!(vector_results[0].chunk_id, "chunk-1");
    assert_eq!(vector_results[0].document_id, "doc-1");
    assert_eq!(vector_results[0].source, RetrievalSource::Vector);
    assert_eq!(text_results[0].source, RetrievalSource::FullText);
}

#[test]
fn hybrid_search_merges_vector_and_full_text_results() {
    let vector = Arc::new(MemoryVectorStore::new());
    let full_text = Arc::new(MemoryFullTextStore::new());
    let chunk = ChunkDocument::new("chunk-1", "doc-1", "thread memory");

    vector
        .upsert(vec![VectorRecord {
            chunk: chunk.clone(),
            embedding: vec![1.0, 0.0],
        }])
        .unwrap();
    full_text.upsert(vec![FullTextRecord { chunk }]).unwrap();

    let engine = HybridSearchEngine::new(vector, full_text);
    let results = engine
        .search(HybridSearchRequest::new("thread", Some(vec![1.0, 0.0]), 10))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, RetrievalSource::Hybrid);
}

#[test]
fn hybrid_search_rejects_unbounded_candidate_limits() {
    let vector = Arc::new(MemoryVectorStore::new());
    let full_text = Arc::new(MemoryFullTextStore::new());
    let engine = HybridSearchEngine::new(vector, full_text);
    let error = engine
        .search(HybridSearchRequest::new(
            "thread",
            Some(vec![1.0, 0.0]),
            MAX_HYBRID_SEARCH_LIMIT + 1,
        ))
        .unwrap_err();

    assert!(error.to_string().contains("hybrid search limit"));
}

#[test]
fn stores_report_rebuild_required_and_clear_after_rebuild() {
    let vector = MemoryVectorStore::new();
    vector
        .mark_rebuild_required("index checksum mismatch")
        .unwrap();

    let health = vector.health_check().unwrap();
    assert_eq!(health.status, StoreStatus::RebuildRequired);

    let report = vector
        .rebuild_from_records(vec![VectorRecord {
            chunk: ChunkDocument::new("chunk-1", "doc-1", "rebuilt"),
            embedding: vec![0.5, 0.5],
        }])
        .unwrap();

    assert_eq!(report.status, RebuildStatus::Completed);
    assert_eq!(report.processed_items, 1);
    assert_eq!(vector.health_check().unwrap().status, StoreStatus::Healthy);
}

#[test]
fn sqlite_full_text_store_persists_search_and_rebuild_state() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("retrieval.sqlite");
    let store = SqliteFullTextStore::open(&db_path).unwrap();
    let mut hot = ChunkDocument::new("chunk-hot", "doc-1", "silver thread in the maze");
    hot.metadata = serde_json::json!({ "layer": "hot" });
    let mut cold = ChunkDocument::new("chunk-cold", "doc-2", "silver thread archived");
    cold.metadata = serde_json::json!({ "layer": "cold" });
    store
        .upsert(vec![
            FullTextRecord { chunk: hot },
            FullTextRecord { chunk: cold },
        ])
        .unwrap();

    let reopened = SqliteFullTextStore::open(&db_path).unwrap();
    let mut request = FullTextSearchRequest::new("silver", 10);
    request.filters.insert("layer".to_owned(), "hot".to_owned());
    let results = reopened.search(request).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, "chunk-hot");
    assert_eq!(results[0].source, RetrievalSource::FullText);

    reopened
        .mark_rebuild_required("sqlite checksum mismatch")
        .unwrap();
    assert_eq!(
        reopened.health_check().unwrap().status,
        StoreStatus::RebuildRequired
    );
    let report = reopened
        .rebuild_from_records(vec![FullTextRecord {
            chunk: ChunkDocument::new("chunk-new", "doc-3", "rebuilt silver"),
        }])
        .unwrap();
    assert_eq!(report.status, RebuildStatus::Completed);
    assert_eq!(
        reopened.health_check().unwrap().status,
        StoreStatus::Healthy
    );
    assert_eq!(
        reopened.delete_document("doc-3").unwrap(),
        1,
        "delete_document returns deleted chunk count"
    );
}

#[test]
fn sqlite_full_text_store_migrates_v1_rows_to_natural_language_ngrams() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("retrieval-v1.sqlite");
    let connection = Connection::open(&db_path).unwrap();
    connection
        .execute_batch(
            "
            CREATE TABLE schema_migrations (
                name TEXT PRIMARY KEY,
                version INTEGER NOT NULL
            );
            INSERT INTO schema_migrations(name, version)
            VALUES('sqlite_full_text_store', 1);

            CREATE TABLE full_text_chunks (
                chunk_id TEXT PRIMARY KEY,
                document_id TEXT NOT NULL,
                text TEXT NOT NULL,
                sources_json TEXT NOT NULL,
                metadata_json TEXT NOT NULL
            );
            CREATE VIRTUAL TABLE full_text_chunks_fts
                USING fts5(chunk_id UNINDEXED, text);
            CREATE TABLE retrieval_store_state (
                component TEXT PRIMARY KEY,
                rebuild_reason TEXT
            );

            INSERT INTO full_text_chunks(
                chunk_id, document_id, text, sources_json, metadata_json
            ) VALUES(
                'chunk-v1',
                'doc-v1',
                '角色：张三（旧城）留下未闭合的线索',
                '[]',
                '{}'
            );
            INSERT INTO full_text_chunks_fts(chunk_id, text)
            VALUES('chunk-v1', '角色：张三（旧城）留下未闭合的线索');
            ",
        )
        .unwrap();
    drop(connection);

    let store = SqliteFullTextStore::open(&db_path).unwrap();
    let results = store
        .search(FullTextSearchRequest::new("未闭合 \"线索", 10))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, "chunk-v1");
    drop(store);

    let connection = Connection::open(&db_path).unwrap();
    let version = connection
        .query_row(
            "SELECT version FROM schema_migrations WHERE name = 'sqlite_full_text_store'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(version, 2);
}

#[test]
fn tantivy_full_text_store_searches_and_rebuilds() {
    let store = TantivyFullTextStore::open_in_memory().unwrap();
    let mut keep = ChunkDocument::new("chunk-hot", "doc-1", "silver thread in the maze");
    keep.metadata = serde_json::json!({ "layer": "hot" });
    let mut skip = ChunkDocument::new("chunk-cold", "doc-2", "silver thread archived");
    skip.metadata = serde_json::json!({ "layer": "cold" });
    store
        .upsert(vec![
            FullTextRecord { chunk: keep },
            FullTextRecord { chunk: skip },
        ])
        .unwrap();

    let mut request = FullTextSearchRequest::new("silver", 10);
    request.filters.insert("layer".to_owned(), "hot".to_owned());
    let results = store.search(request).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, "chunk-hot");
    store
        .mark_rebuild_required("tantivy checksum mismatch")
        .unwrap();
    assert_eq!(
        store.health_check().unwrap().status,
        StoreStatus::RebuildRequired
    );
    let report = store
        .rebuild_from_records(vec![FullTextRecord {
            chunk: ChunkDocument::new("chunk-new", "doc-3", "rebuilt silver"),
        }])
        .unwrap();
    assert_eq!(report.status, RebuildStatus::Completed);
}

#[test]
fn full_text_backends_treat_author_punctuation_as_literal_natural_language() {
    let tantivy = TantivyFullTextStore::open_in_memory().unwrap();
    let sqlite = SqliteFullTextStore::open_in_memory().unwrap();
    let record = FullTextRecord {
        chunk: ChunkDocument::new(
            "chunk-natural-language",
            "doc-natural-language",
            "角色：张三（旧城）留下未闭合的线索",
        ),
    };
    tantivy.upsert(vec![record.clone()]).unwrap();
    sqlite.upsert(vec![record]).unwrap();

    for query in ["角色:张三", "张三（旧城）", "未闭合 \"线索"] {
        let tantivy_results = tantivy
            .search(FullTextSearchRequest::new(query, 10))
            .unwrap();
        let sqlite_results = sqlite
            .search(FullTextSearchRequest::new(query, 10))
            .unwrap();
        assert!(
            !tantivy_results.is_empty(),
            "Tantivy should accept natural query: {query}"
        );
        assert!(
            !sqlite_results.is_empty(),
            "SQLite FTS should accept natural query: {query}"
        );
    }
}

#[test]
fn three_way_hybrid_search_merges_vector_tantivy_and_sqlite() {
    let vector = Arc::new(MemoryVectorStore::new());
    let tantivy = Arc::new(TantivyFullTextStore::open_in_memory().unwrap());
    let sqlite = Arc::new(SqliteFullTextStore::open_in_memory().unwrap());
    let chunk = ChunkDocument::new("chunk-1", "doc-1", "silver thread memory");
    vector
        .upsert(vec![VectorRecord {
            chunk: chunk.clone(),
            embedding: vec![1.0, 0.0],
        }])
        .unwrap();
    tantivy
        .upsert(vec![FullTextRecord {
            chunk: chunk.clone(),
        }])
        .unwrap();
    sqlite.upsert(vec![FullTextRecord { chunk }]).unwrap();

    let engine = ThreeWayHybridSearchEngine::new(vector, tantivy, sqlite);
    let results = engine
        .search(HybridSearchRequest::new("silver", Some(vec![1.0, 0.0]), 5))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, RetrievalSource::Hybrid);
    assert_eq!(engine.health_check().unwrap().len(), 3);
}

#[test]
fn sidecar_port_selection_handles_conflicts() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let taken = listener.local_addr().unwrap().port();

    let selection = select_available_port("127.0.0.1", taken).unwrap();

    assert_ne!(selection.port, taken);
    assert!(!selection.reused_requested_port);
}

#[test]
fn sidecar_supervisor_reports_crash_as_unavailable() {
    let temp_dir = tempfile::tempdir().unwrap();
    let supervisor = QdrantSidecarSupervisor::new(ariadne::retrieval::QdrantSidecarConfig {
        binary_path: temp_dir.path().join("qdrant"),
        host: "127.0.0.1".to_owned(),
        requested_port: 6333,
        data_dir: temp_dir.path().join("data"),
        log_dir: temp_dir.path().join("logs"),
        startup_timeout_ms: 5_000,
        max_restarts_per_window: 3,
        restart_window_ms: 60_000,
    });

    let status = supervisor.mark_crashed("process exited").unwrap();
    let health = supervisor.health_check().unwrap();

    assert_eq!(status.state, SidecarState::Unavailable);
    assert_eq!(health.status, StoreStatus::Unavailable);
}

#[derive(Debug)]
struct NoopSidecarRunner;

impl SidecarProcessRunner for NoopSidecarRunner {
    fn spawn(
        &self,
        _config: &QdrantSidecarConfig,
        _port: u16,
    ) -> ariadne::contracts::CoreResult<Child> {
        Command::new("sh")
            .arg("-c")
            .arg("sleep 1")
            .spawn()
            .map_err(Into::into)
    }
}

/// 只 spawn 一个立刻退出的进程，用于模拟 sidecar 被 OOM/外部 kill 掉。
#[derive(Debug)]
struct ImmediatelyExitingSidecarRunner;

impl SidecarProcessRunner for ImmediatelyExitingSidecarRunner {
    fn spawn(
        &self,
        _config: &QdrantSidecarConfig,
        _port: u16,
    ) -> ariadne::contracts::CoreResult<Child> {
        Command::new("sh")
            .arg("-c")
            .arg("exit 137")
            .spawn()
            .map_err(Into::into)
    }
}

/// 进程被外部杀掉后，诊断必须报不健康。
///
/// 回归的是一条会**骗人**的缺陷：`health_check` 曾只读缓存的 status 字段，
/// 而写入点只有 start/stop/mark_crashed，其中 mark_crashed 生产无人调用。
/// 于是 sidecar 被 OOM 杀掉后诊断永远显示 healthy，向量检索却已全部静默失败。
#[test]
fn sidecar_health_detects_externally_killed_process_instead_of_reporting_cached_running() {
    let temp_dir = tempfile::tempdir().unwrap();
    let supervisor = QdrantSidecarSupervisor::with_runner(
        QdrantSidecarConfig {
            binary_path: temp_dir.path().join("qdrant"),
            host: "127.0.0.1".to_owned(),
            requested_port: 0,
            data_dir: temp_dir.path().join("data"),
            log_dir: temp_dir.path().join("logs"),
            startup_timeout_ms: 1,
            max_restarts_per_window: 3,
            restart_window_ms: 60_000,
        },
        ImmediatelyExitingSidecarRunner,
    );

    // start 会拿到一个已经/即将退出的进程；TCP 探测失败使其落在 Degraded。
    let started = supervisor.start().unwrap();
    assert_ne!(
        started.state,
        SidecarState::Stopped,
        "start 之后不应是 Stopped，否则本用例没有覆盖到崩溃探测"
    );

    // 等待子进程确实退出，让 try_wait 能观察到退出码。
    std::thread::sleep(std::time::Duration::from_millis(200));

    let health = supervisor.health_check().unwrap();
    assert_eq!(
        health.status,
        StoreStatus::Unavailable,
        "进程已退出时诊断必须报 Unavailable，报 healthy 会让用户看不到检索已失效"
    );
    assert_eq!(
        supervisor.status().unwrap().state,
        SidecarState::Unavailable
    );
    assert!(
        health
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("exited")),
        "失败原因要能指出是进程退出，实际为 {:?}",
        health.reason
    );
}

/// spawn 一个真正监听并持续 accept 的进程，使 start() 能判定为 Running。
/// 用它才能复现最恶劣的场景：先健康运行、缓存写入 Running，之后进程才被杀掉。
///
/// **必须持续 accept**：只 listen 不 accept 的话，连接会堆在 backlog 里，
/// 塞满后触发内核 SYN 重传退避，单次连接要 ~300ms。那是夹具产物而非探活开销，
/// 会让性能用例给出完全错误的结论（我第一版就踩了这个坑）。
#[derive(Debug)]
struct ListeningSidecarRunner;

impl SidecarProcessRunner for ListeningSidecarRunner {
    fn spawn(
        &self,
        _config: &QdrantSidecarConfig,
        port: u16,
    ) -> ariadne::contracts::CoreResult<Child> {
        Command::new("python3")
            .arg("-c")
            .arg(format!(
                "import socket\n\
                 s=socket.socket()\n\
                 s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)\n\
                 s.bind(('127.0.0.1',{port}))\n\
                 s.listen(64)\n\
                 while True:\n\
                 \x20   c,_=s.accept()\n\
                 \x20   c.close()"
            ))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(Into::into)
    }
}

/// 最恶劣场景：sidecar 先健康运行（缓存写入 Running），之后被 OOM/外部 kill 掉。
///
/// 这才是那条会**骗人**的缺陷的真实形态——缓存停在 Running，
/// `health_check` 只读缓存就会一直报 healthy，而向量检索早已全部失败。
/// 上面那个 `..._instead_of_reporting_cached_running` 用例里进程从未成功监听过，
/// 缓存是 Degraded，掩盖了 healthy 这一最坏结果。
#[test]
fn sidecar_health_detects_kill_after_healthy_start_when_cache_says_running() {
    let temp_dir = tempfile::tempdir().unwrap();
    // 必须指定具体端口：requested_port=0 会走系统分配路径，
    // start() 据此判定「端口回退」而置为 Degraded，就覆盖不到 Running 缓存了。
    // 先探一个空闲端口再立即释放，让 start() 自己去 bind。
    let free_port = {
        let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        probe.local_addr().unwrap().port()
    };
    let supervisor = QdrantSidecarSupervisor::with_runner(
        QdrantSidecarConfig {
            binary_path: temp_dir.path().join("qdrant"),
            host: "127.0.0.1".to_owned(),
            requested_port: free_port,
            data_dir: temp_dir.path().join("data"),
            log_dir: temp_dir.path().join("logs"),
            startup_timeout_ms: 5_000,
            max_restarts_per_window: 3,
            restart_window_ms: 60_000,
        },
        ListeningSidecarRunner,
    );

    let started = supervisor.start().unwrap();
    assert_eq!(
        started.state,
        SidecarState::Running,
        "前提：本用例要求 sidecar 先真正健康运行，否则覆盖不到 cached-Running 这一最坏情形"
    );
    assert_eq!(
        supervisor.health_check().unwrap().status,
        StoreStatus::Healthy,
        "健康运行时不应误报故障"
    );

    // 模拟 OOM killer：直接杀掉子进程，supervisor 不经手，缓存仍是 Running。
    let pid = started
        .process_id
        .expect("running sidecar must expose a pid");
    Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));

    let health = supervisor.health_check().unwrap();
    assert_eq!(
        health.status,
        StoreStatus::Unavailable,
        "被杀掉后必须报 Unavailable；报 healthy 正是那条骗人的缺陷"
    );
}

/// 探活开销必须留在用户无感范围内。
///
/// 诊断页每次刷新都会走 `health_check`，探活给它加了一次 `try_wait` 和一次 TCP 连接。
/// 本用例钉住两种情形的耗时上界，防止后人把探活换成阻塞轮询
/// （`wait_for_tcp_health` 就是 25ms 一轮直到 startup_timeout_ms，放这里会卡满几秒）。
#[test]
fn sidecar_probe_stays_cheap_enough_for_the_diagnostics_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let free_port = {
        let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        probe.local_addr().unwrap().port()
    };
    let supervisor = QdrantSidecarSupervisor::with_runner(
        QdrantSidecarConfig {
            binary_path: temp_dir.path().join("qdrant"),
            host: "127.0.0.1".to_owned(),
            requested_port: free_port,
            data_dir: temp_dir.path().join("data"),
            log_dir: temp_dir.path().join("logs"),
            startup_timeout_ms: 5_000,
            max_restarts_per_window: 3,
            restart_window_ms: 60_000,
        },
        ListeningSidecarRunner,
    );
    supervisor.start().unwrap();

    // 情形一：健康运行。这是最常见的路径，每次诊断刷新都会走。
    let healthy_start = std::time::Instant::now();
    for _ in 0..20 {
        supervisor.health_check().unwrap();
    }
    let healthy_each = healthy_start.elapsed() / 20;
    eprintln!("[timing] healthy probe each: {healthy_each:?}");
    // 实测：try_wait ~1µs + 本地 TCP 连接 ~150µs，合计远在 1ms 内。
    // 5ms 上界留足 CI 抖动余量，抓的是「退回阻塞轮询」那种数量级退化
    // （wait_for_tcp_health 是 25ms 一轮直到 startup_timeout_ms，必然撞线）。
    assert!(
        healthy_each < std::time::Duration::from_millis(5),
        "健康态单次探活耗时 {healthy_each:?}，说明退回了阻塞轮询"
    );

    // 情形二：进程已死。try_wait 立即给出结论，不该等 TCP 超时。
    let pid = supervisor.status().unwrap().process_id.unwrap();
    Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));

    let dead_start = std::time::Instant::now();
    supervisor.health_check().unwrap();
    let dead_elapsed = dead_start.elapsed();
    assert!(
        dead_elapsed < std::time::Duration::from_millis(50),
        "进程已死时探活耗时 {dead_elapsed:?}；应由 try_wait 立即判定，不该等 TCP 连接超时"
    );
}

/// 长命但从不监听端口的进程：用于构造「进程活着 + 端口不通」这一 Degraded 场景。
/// `NoopSidecarRunner` 的 `sleep 1` 会在探活前就退出，那样只能测到崩溃分支。
#[derive(Debug)]
/// 每次 spawn 都立刻退出，并统计被调用次数——用来验证重启上限真的拦住了 fork 风暴。
struct CountingFailingSidecarRunner {
    spawns: Arc<AtomicUsize>,
}

impl SidecarProcessRunner for CountingFailingSidecarRunner {
    fn spawn(
        &self,
        _config: &QdrantSidecarConfig,
        _port: u16,
    ) -> ariadne::contracts::CoreResult<Child> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        Command::new("sh")
            .arg("-c")
            .arg("exit 137")
            .spawn()
            .map_err(Into::into)
    }
}

/// 持久故障下，自动恢复必须被滑动窗口上限截住。
///
/// 钉的是安全性质而非功能：sidecar 二进制缺失/端口被占这类故障不会自愈，
/// 若每次检索都拉起一个必然失败的进程，一次故障就被放大成 fork 风暴。
#[test]
fn sidecar_recovery_stops_spawning_after_restart_window_limit() {
    let temp_dir = tempfile::tempdir().unwrap();
    let spawns = Arc::new(AtomicUsize::new(0));
    let supervisor = QdrantSidecarSupervisor::with_runner(
        QdrantSidecarConfig {
            binary_path: temp_dir.path().join("qdrant"),
            host: "127.0.0.1".to_owned(),
            requested_port: 0,
            data_dir: temp_dir.path().join("data"),
            log_dir: temp_dir.path().join("logs"),
            startup_timeout_ms: 200,
            max_restarts_per_window: 2,
            restart_window_ms: 60_000,
        },
        CountingFailingSidecarRunner {
            spawns: spawns.clone(),
        },
    );

    // 先让它进入「启动过但已死」的状态，恢复路径才会被触发。
    let _ = supervisor.start();
    let spawns_after_start = spawns.load(Ordering::SeqCst);

    // 反复恢复，次数远超上限。每轮都要清节流窗口，否则测的是节流不是重启上限。
    for _ in 0..10 {
        supervisor.reset_recovery_probe_throttle_for_tests();
        let _ = supervisor.recover_if_unavailable();
    }

    let extra = spawns.load(Ordering::SeqCst) - spawns_after_start;
    assert!(
        extra <= 2,
        "持久故障下恢复应被窗口上限截住，实际额外 spawn {extra} 次"
    );
}

/// 稳态下恢复路径不能每次检索都做 TCP 探活。
///
/// 钉的是性能性质：sidecar 正常时探活恒定返回健康，高频检索反复连接是纯浪费。
#[test]
fn sidecar_recovery_probe_is_throttled_between_consecutive_calls() {
    let temp_dir = tempfile::tempdir().unwrap();
    let supervisor = QdrantSidecarSupervisor::with_runner(
        QdrantSidecarConfig {
            binary_path: temp_dir.path().join("qdrant"),
            host: "127.0.0.1".to_owned(),
            requested_port: 0,
            data_dir: temp_dir.path().join("data"),
            log_dir: temp_dir.path().join("logs"),
            startup_timeout_ms: 200,
            max_restarts_per_window: 3,
            restart_window_ms: 60_000,
        },
        ImmediatelyExitingSidecarRunner,
    );
    let _ = supervisor.start();

    // 第一次探活到期后立刻再调；窗口内必须直接返回 None，不进 probe。
    supervisor.reset_recovery_probe_throttle_for_tests();
    let _ = supervisor.recover_if_unavailable();
    let started = std::time::Instant::now();
    let throttled = supervisor.recover_if_unavailable().unwrap();
    let elapsed = started.elapsed();

    assert!(throttled.is_none(), "节流窗口内不应返回恢复结果");
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "节流命中时不应付探活代价，实际耗时 {elapsed:?}"
    );
}

struct LongLivedSilentSidecarRunner;

impl SidecarProcessRunner for LongLivedSilentSidecarRunner {
    fn spawn(
        &self,
        _config: &QdrantSidecarConfig,
        _port: u16,
    ) -> ariadne::contracts::CoreResult<Child> {
        Command::new("sh")
            .arg("-c")
            .arg("sleep 30")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(Into::into)
    }
}

/// 进程活着但端口不通时，探活必须快速返回而非等满超时。
///
/// 这是唯一能暴露「探活退回阻塞轮询」的场景：
/// 进程已死走 try_wait 立即返回、端口正常第一次连接就成功，两条路径都绕过了轮询代价。
/// 只有「进程活着 + 端口不通」会真的进到 TCP 失败路径。
#[test]
fn sidecar_probe_returns_fast_when_process_lives_but_port_is_unreachable() {
    let temp_dir = tempfile::tempdir().unwrap();
    let supervisor = QdrantSidecarSupervisor::with_runner(
        QdrantSidecarConfig {
            binary_path: temp_dir.path().join("qdrant"),
            host: "127.0.0.1".to_owned(),
            requested_port: 0,
            data_dir: temp_dir.path().join("data"),
            log_dir: temp_dir.path().join("logs"),
            // start() 内部的 wait_for_tcp_health 会等满这个值，故取小；
            // 探活若退回阻塞轮询，用的也是这个值——下面用倍数关系把两者区分开。
            startup_timeout_ms: 300,
            max_restarts_per_window: 3,
            restart_window_ms: 60_000,
        },
        // sleep 进程一直活着，但从不监听端口 → try_wait 说活着、TCP 连不上。
        LongLivedSilentSidecarRunner,
    );

    // start 会因端口不通落在 Degraded，但进程是活的、缓存里有端口号。
    supervisor.start().unwrap();

    let started = std::time::Instant::now();
    let health = supervisor.health_check().unwrap();
    let elapsed = started.elapsed();

    assert_eq!(
        health.status,
        StoreStatus::Degraded,
        "进程活着但端口不通应报 Degraded（可能只是负载高），报 Unavailable 会诱发不必要的重启"
    );
    // 单次 connect_timeout 到本地未监听端口会立刻收到 RST（µs 级）；
    // 阻塞轮询则要磨满 startup_timeout_ms=300ms。150ms 卡在两者中间。
    assert!(
        elapsed < std::time::Duration::from_millis(150),
        "探活耗时 {elapsed:?}；退回阻塞轮询会一直磨到 startup_timeout_ms"
    );
}

/// 用户主动 stop 之后探活不得把「已停止」误报成「崩溃」。
#[test]
fn sidecar_probe_keeps_user_requested_stop_distinct_from_crash() {
    let temp_dir = tempfile::tempdir().unwrap();
    let supervisor = QdrantSidecarSupervisor::with_runner(
        QdrantSidecarConfig {
            binary_path: temp_dir.path().join("qdrant"),
            host: "127.0.0.1".to_owned(),
            requested_port: 0,
            data_dir: temp_dir.path().join("data"),
            log_dir: temp_dir.path().join("logs"),
            startup_timeout_ms: 1,
            max_restarts_per_window: 3,
            restart_window_ms: 60_000,
        },
        NoopSidecarRunner,
    );

    supervisor.start().unwrap();
    supervisor.stop().unwrap();

    assert_eq!(
        supervisor.status().unwrap().state,
        SidecarState::Stopped,
        "stop 之后应保持 Stopped"
    );
    // 反复探活也不能把主动停止改写成 Unavailable。
    supervisor.health_check().unwrap();
    assert_eq!(supervisor.status().unwrap().state, SidecarState::Stopped);
}

#[test]
fn retrieval_recovery_restarts_sidecar_and_rebuilds_indexes() {
    let temp_dir = tempfile::tempdir().unwrap();
    let supervisor = QdrantSidecarSupervisor::with_runner(
        QdrantSidecarConfig {
            binary_path: temp_dir.path().join("qdrant"),
            host: "127.0.0.1".to_owned(),
            requested_port: 0,
            data_dir: temp_dir.path().join("data"),
            log_dir: temp_dir.path().join("logs"),
            startup_timeout_ms: 1,
            max_restarts_per_window: 3,
            restart_window_ms: 60_000,
        },
        NoopSidecarRunner,
    );
    supervisor.mark_crashed("process exited").unwrap();
    let vector = MemoryVectorStore::new();
    let text = MemoryFullTextStore::new();
    vector.mark_rebuild_required("vector stale").unwrap();
    text.mark_rebuild_required("text stale").unwrap();

    let report = recover_retrieval_components(
        &supervisor,
        &vector,
        vec![VectorRecord {
            chunk: ChunkDocument::new("chunk-v", "doc-v", "vector rebuilt"),
            embedding: vec![1.0],
        }],
        &text,
        vec![FullTextRecord {
            chunk: ChunkDocument::new("chunk-t", "doc-t", "text rebuilt"),
        }],
    )
    .unwrap();

    assert!(report
        .actions
        .contains(&RetrievalRecoveryAction::RestartSidecar));
    assert!(report
        .actions
        .contains(&RetrievalRecoveryAction::RebuildVectorIndex));
    assert!(report
        .actions
        .contains(&RetrievalRecoveryAction::RebuildFullTextIndex));
    assert_eq!(report.rebuild_reports.len(), 2);
}

#[test]
fn qdrant_initialize_rejects_existing_collection_dimension_mismatch() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /collections/ariadne "));
        write_json_response(
            &mut stream,
            200,
            r#"{"result":{"config":{"params":{"vectors":{"size":3,"distance":"Cosine"}}}}}"#,
        );
    });
    let store = QdrantVectorStore::new(endpoint, "ariadne", 2).unwrap();

    let error = store.initialize().unwrap_err();
    server.join().unwrap();

    assert!(error.to_string().contains("vector dimension 3"));
    assert!(error.to_string().contains("configured dimension 2"));
}

#[test]
fn qdrant_api_key_is_sent_as_a_request_header() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        write_json_response(
            &mut stream,
            200,
            r#"{"result":{"config":{"params":{"vectors":{"size":2,"distance":"Cosine"}}}}}"#,
        );
        request
    });
    let store =
        QdrantVectorStore::new_with_api_key(endpoint, "ariadne", 2, Some("endpoint-secret"))
            .unwrap();

    store.initialize().unwrap();
    let request = server.join().unwrap();

    assert!(request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("api-key") && value.trim() == "endpoint-secret"
        })
    }));
}

#[test]
fn qdrant_health_detects_collection_dimension_drift() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /collections/ariadne "));
        write_json_response(
            &mut stream,
            200,
            r#"{"result":{"config":{"params":{"vectors":{"size":3,"distance":"Cosine"}}}}}"#,
        );
    });
    let store = QdrantVectorStore::new(endpoint, "ariadne", 2).unwrap();

    let health = store.health_check().unwrap();
    server.join().unwrap();

    assert_eq!(health.status, StoreStatus::Unavailable);
    assert!(health.reason.unwrap().contains("configured dimension 2"));
}

#[test]
fn c9_qdrant_search_can_cancel_stalled_response_after_dispatch() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicBool::new(false));
    let server_accepted = Arc::clone(&accepted);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /collections/ariadne/points/search "));
        server_accepted.store(true, Ordering::Release);
        thread::sleep(std::time::Duration::from_millis(500));
    });
    let store = QdrantVectorStore::new(endpoint, "ariadne", 2).unwrap();
    let cancellation = ExecutionCancellation::new();
    let cancel_from_thread = cancellation.clone();
    let canceller = thread::spawn(move || {
        let started = std::time::Instant::now();
        while !accepted.load(Ordering::Acquire)
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            thread::sleep(std::time::Duration::from_millis(5));
        }
        cancel_from_thread.cancel();
    });

    let started = std::time::Instant::now();
    let error = store
        .search_with_cancellation(VectorSearchRequest::new(vec![1.0, 0.0], 5), &cancellation)
        .unwrap_err();
    let request_elapsed = started.elapsed();

    canceller.join().unwrap();
    server.join().unwrap();
    assert!(matches!(
        error,
        CoreError::ExternalCancellation {
            outcome: ExternalDispatchOutcome::DispatchedUnknown,
            ..
        }
    ));
    assert!(request_elapsed < std::time::Duration::from_millis(300));
}

#[test]
fn qdrant_rebuild_deletes_stale_collection_before_recreate_and_upsert() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("qdrant-rebuild-required.json");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let responses = [
            (200, r#"{"result":true}"#),
            (404, r#"{"status":"not found"}"#),
            (200, r#"{"result":true}"#),
            (
                200,
                r#"{"result":{"config":{"params":{"vectors":{"size":2,"distance":"Cosine"}}}}}"#,
            ),
            (200, r#"{"result":{"status":"completed"}}"#),
        ];
        responses
            .into_iter()
            .map(|(status, body)| {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                write_json_response(&mut stream, status, body);
                request
            })
            .collect::<Vec<_>>()
    });
    let store = QdrantVectorStore::new(endpoint, "ariadne", 2)
        .unwrap()
        .with_rebuild_marker(&marker)
        .unwrap();
    store
        .mark_rebuild_required("old points may remain")
        .unwrap();
    assert_eq!(
        store.health_check().unwrap().status,
        StoreStatus::RebuildRequired
    );

    let report = store
        .rebuild_from_records(vec![VectorRecord {
            chunk: ChunkDocument::new("fresh-chunk", "fresh-document", "fresh text"),
            embedding: vec![0.25, 0.75],
        }])
        .unwrap();
    let requests = server.join().unwrap();

    assert_eq!(report.status, RebuildStatus::Completed);
    assert_eq!(report.processed_items, 1);
    assert!(!marker.exists(), "successful rebuild must clear marker");
    assert!(requests[0].starts_with("DELETE /collections/ariadne "));
    assert!(requests[1].starts_with("GET /collections/ariadne "));
    assert!(requests[2].starts_with("PUT /collections/ariadne "));
    assert!(requests[2].contains("\"size\":2"));
    assert!(requests[3].starts_with("GET /collections/ariadne "));
    assert!(requests[4].starts_with("PUT /collections/ariadne/points?wait=true "));
    assert!(requests[4].contains("fresh-chunk"));
    assert!(!requests[4].contains("old points may remain"));
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut expected_len = None;
    loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if expected_len.is_none() {
            if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_len = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                    })
                    .unwrap_or(0);
                expected_len = Some(header_end + 4 + content_len);
            }
        }
        if expected_len.is_some_and(|expected| bytes.len() >= expected) {
            break;
        }
    }
    String::from_utf8(bytes).unwrap()
}

fn http_request_body(request: &str) -> &str {
    request.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}

fn write_json_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}

/// U109：`reranker_enabled` 为真但重排序路由不可解析时，重排序必须单独降级，
/// **不得**连带击穿整个检索组合根——全文检索仍要能正常构造并返回结果。
///
/// 修复前 `ProjectRetrievalRuntime::from_config` 在 reranker 装配处直接 `?`，
/// 一个只影响排序质量的开关会让全部检索能力停摆，且错误与「重排序」毫无表面关联。
#[test]
fn enabled_but_unresolvable_reranker_degrades_without_breaking_retrieval() {
    let project = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();

    let store = ConfigStore::new(project.path());
    let mut config = store.load_or_create().unwrap();
    // 开着重排序，但既没有默认 reranker Provider，也没有任何 reranker 模型。
    config.rag.reranker_enabled = true;
    config.rag.vector_store.enabled = false;
    store.save(&config).unwrap();

    let secrets = MemorySecretStore::default();
    let runtime = ProjectRetrievalRuntime::open(project.path(), &secrets)
        .expect("U109：重排序不可用不得阻断检索组合根的构造");

    // 降级原因必须被显式记录，不能静默吞掉。
    let reason = runtime
        .reranker_unavailable_reason()
        .expect("U109：重排序装配失败必须留下可诊断的原因");
    assert!(
        reason.contains("reranker"),
        "降级原因应指明是重排序失败：{reason}"
    );

    // 且必须在健康检查里表现为 degraded 的 reranker_provider 项。
    let health = runtime.health_check().expect("健康检查本身不应失败");
    let reranker_health = health
        .iter()
        .find(|item| item.component == "reranker_provider")
        .expect("U109：开关开着时必须上报 reranker_provider 健康项，不能表现为『没配重排序』");
    assert_eq!(
        reranker_health.status,
        StoreStatus::Degraded,
        "重排序不可用应为 degraded 而非 unavailable/healthy"
    );
    assert_eq!(
        reranker_health.reason.as_deref(),
        Some("diagnostics.retrieval.reranker.unavailable"),
        "reason 必须是纯 display_name key，前端才能本地化"
    );

    // 关键断言：全文检索链路完全不受影响。
    let chapter = project.path().join("documents").join("chapter.md");
    std::fs::create_dir_all(chapter.parent().unwrap()).unwrap();
    std::fs::write(&chapter, "银色线索藏在旧钟楼的第三层").unwrap();
    runtime
        .health_check()
        .expect("U109：重排序降级后检索运行时仍须整体可用");
}
