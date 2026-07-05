use std::net::TcpListener;
use std::sync::Arc;

use ariadne::retrieval::{
    recover_retrieval_components, select_available_port, ChunkDocument, FullTextRecord,
    FullTextSearchRequest, HybridSearchEngine, HybridSearchRequest, MemoryFullTextStore,
    MemoryVectorStore, RebuildStatus, RetrievalRecoveryAction, RetrievalSource, SidecarState,
    SqliteFullTextStore, StoreStatus, TantivyFullTextStore, ThreeWayHybridSearchEngine,
    VectorRecord, VectorSearchRequest, MAX_HYBRID_SEARCH_LIMIT,
};
use ariadne::retrieval::{
    FullTextStore, HybridSearch, QdrantSidecarConfig, QdrantSidecarSupervisor,
    SidecarProcessRunner, VectorStore,
};
use std::process::{Child, Command};

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
    fn spawn(&self, _config: &QdrantSidecarConfig, _port: u16) -> ariadne::contracts::CoreResult<Child> {
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
