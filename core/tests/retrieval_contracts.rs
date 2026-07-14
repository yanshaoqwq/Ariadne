use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use ariadne::config::{
    ConfigStore, MemorySecretStore, ModelConfig, ProjectCredentialScope, ProviderConfig,
    SecretValue, VectorStoreBackend,
};
use ariadne::contracts::{
    CoreError, CoreResult, ExternalDispatchAuthorization, ProviderCapability, ProviderType,
    SourceSpan, TextRange,
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
