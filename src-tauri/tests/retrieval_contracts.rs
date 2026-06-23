use std::net::TcpListener;
use std::sync::Arc;

use ariadne::retrieval::{
    select_available_port, ChunkDocument, FullTextRecord, FullTextSearchRequest,
    HybridSearchEngine, HybridSearchRequest, MemoryFullTextStore, MemoryVectorStore, RebuildStatus,
    RetrievalSource, SidecarState, StoreStatus, VectorRecord, VectorSearchRequest,
};
use ariadne::retrieval::{FullTextStore, HybridSearch, QdrantSidecarSupervisor, VectorStore};

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
