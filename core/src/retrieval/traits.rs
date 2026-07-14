use crate::contracts::CoreResult;
use crate::providers::ProviderCallContext;
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

/// 文本向量化端口；生产实现必须调用已配置的 EmbeddingProvider。
pub trait TextEmbedder: Send + Sync {
    /// 稳定 provider id，用于诊断与成本归因。
    fn provider_id(&self) -> &str;

    /// 稳定模型 id；provider/model 任一变化都代表向量空间可能变化。
    fn model_id(&self) -> &str;

    /// 配置要求的向量维度。
    fn dimensions(&self) -> usize;

    /// 批量生成向量，并校验数量、维度和有限值。
    fn embed(&self, context: ProviderCallContext, inputs: Vec<String>)
        -> CoreResult<Vec<Vec<f32>>>;

    /// 返回 provider 配置健康状态；不把未探测的远端伪装成已验证健康。
    fn health_check(&self) -> CoreResult<StoreHealth>;
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
