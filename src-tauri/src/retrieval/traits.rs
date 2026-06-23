use crate::core::CoreResult;
use crate::retrieval::models::{
    FullTextRecord, FullTextSearchRequest, HybridSearchRequest, RebuildReport, RerankInput,
    RetrievalResult, StoreHealth, VectorRecord, VectorSearchRequest,
};

/// 向量索引后端契约，真实 Qdrant 和测试内存后端都实现它。
pub trait VectorStore: Send + Sync {
    /// 写入或覆盖向量记录。
    fn upsert(&self, records: Vec<VectorRecord>) -> CoreResult<()>;

    /// 删除某个文档下的所有向量记录，返回删除数量。
    fn delete_document(&self, document_id: &str) -> CoreResult<usize>;

    /// 按 query embedding 检索相似 chunk。
    fn search(&self, request: VectorSearchRequest) -> CoreResult<Vec<RetrievalResult>>;

    /// 返回当前后端健康状态。
    fn health_check(&self) -> CoreResult<StoreHealth>;

    /// 标记索引需要重建。
    fn mark_rebuild_required(&self, reason: &str) -> CoreResult<()>;

    /// 使用源记录重建整个索引。
    fn rebuild_from_records(&self, records: Vec<VectorRecord>) -> CoreResult<RebuildReport>;
}

/// 全文索引后端契约，真实 Tantivy 和测试内存后端都实现它。
pub trait FullTextStore: Send + Sync {
    /// 写入或覆盖全文记录。
    fn upsert(&self, records: Vec<FullTextRecord>) -> CoreResult<()>;

    /// 删除某个文档下的所有全文记录，返回删除数量。
    fn delete_document(&self, document_id: &str) -> CoreResult<usize>;

    /// 按文本查询检索 chunk。
    fn search(&self, request: FullTextSearchRequest) -> CoreResult<Vec<RetrievalResult>>;

    /// 返回当前后端健康状态。
    fn health_check(&self) -> CoreResult<StoreHealth>;

    /// 标记索引需要重建。
    fn mark_rebuild_required(&self, reason: &str) -> CoreResult<()>;

    /// 使用源记录重建整个索引。
    fn rebuild_from_records(&self, records: Vec<FullTextRecord>) -> CoreResult<RebuildReport>;
}

/// 检索结果重排契约。
pub trait ResultReranker: Send + Sync {
    /// 对候选结果重新排序并裁剪。
    fn rerank(&self, input: RerankInput) -> CoreResult<Vec<RetrievalResult>>;
}

/// 混合检索契约，组合多个底层检索后端。
pub trait HybridSearch {
    /// 执行混合检索。
    fn search(&self, request: HybridSearchRequest) -> CoreResult<Vec<RetrievalResult>>;

    /// 返回底层组件健康状态。
    fn health_check(&self) -> CoreResult<Vec<StoreHealth>>;
}
