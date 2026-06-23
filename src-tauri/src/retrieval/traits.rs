use crate::core::CoreResult;
use crate::retrieval::models::{
    FullTextRecord, FullTextSearchRequest, HybridSearchRequest, RebuildReport, RerankInput,
    RetrievalResult, StoreHealth, VectorRecord, VectorSearchRequest,
};

pub trait VectorStore: Send + Sync {
    fn upsert(&self, records: Vec<VectorRecord>) -> CoreResult<()>;

    fn delete_document(&self, document_id: &str) -> CoreResult<usize>;

    fn search(&self, request: VectorSearchRequest) -> CoreResult<Vec<RetrievalResult>>;

    fn health_check(&self) -> CoreResult<StoreHealth>;

    fn mark_rebuild_required(&self, reason: &str) -> CoreResult<()>;

    fn rebuild_from_records(&self, records: Vec<VectorRecord>) -> CoreResult<RebuildReport>;
}

pub trait FullTextStore: Send + Sync {
    fn upsert(&self, records: Vec<FullTextRecord>) -> CoreResult<()>;

    fn delete_document(&self, document_id: &str) -> CoreResult<usize>;

    fn search(&self, request: FullTextSearchRequest) -> CoreResult<Vec<RetrievalResult>>;

    fn health_check(&self) -> CoreResult<StoreHealth>;

    fn mark_rebuild_required(&self, reason: &str) -> CoreResult<()>;

    fn rebuild_from_records(&self, records: Vec<FullTextRecord>) -> CoreResult<RebuildReport>;
}

pub trait ResultReranker: Send + Sync {
    fn rerank(&self, input: RerankInput) -> CoreResult<Vec<RetrievalResult>>;
}

pub trait HybridSearch {
    fn search(&self, request: HybridSearchRequest) -> CoreResult<Vec<RetrievalResult>>;

    fn health_check(&self) -> CoreResult<Vec<StoreHealth>>;
}
