use std::collections::BTreeMap;
use std::sync::Arc;

use crate::contracts::{CoreError, CoreResult, ExecutionCancellation};
use crate::retrieval::memory::sort_and_limit;
use crate::retrieval::models::{
    FullTextSearchRequest, HybridSearchRequest, RerankInput, RetrievalResult, RetrievalSource,
    StoreHealth, VectorSearchRequest,
};
use crate::retrieval::traits::{FullTextStore, HybridSearch, ResultReranker, VectorStore};

/// 单次混合检索允许的最大最终返回数量。
pub const MAX_HYBRID_SEARCH_LIMIT: usize = 10_000;
/// 单次混合检索允许进入 reranker 的最大候选数量。
pub const MAX_HYBRID_CANDIDATE_LIMIT: usize = MAX_HYBRID_SEARCH_LIMIT * 3;
/// 产品 Search 允许内联到运行快照的最大结果数。
pub const MAX_PRODUCT_SEARCH_LIMIT: usize = 50;
/// 产品 Search 允许内联到运行快照的最大序列化字节数。
pub const MAX_PRODUCT_SEARCH_RESULT_BYTES: usize = 128 * 1024;
/// Reciprocal Rank Fusion 常量；只依赖后端排名，不混加不同量纲的原始分数。
const RRF_K: f32 = 60.0;

/// 混合检索引擎，组合向量检索、全文检索和可选 reranker。
pub struct HybridSearchEngine {
    vector_store: Arc<dyn VectorStore>,
    full_text_store: Arc<dyn FullTextStore>,
    reranker: Option<Arc<dyn ResultReranker>>,
}

/// 三路混合检索引擎：Qdrant 向量、Tantivy 全文和 SQLite FTS5 全文。
pub struct ThreeWayHybridSearchEngine {
    vector_store: Option<Arc<dyn VectorStore>>,
    tantivy_store: Arc<dyn FullTextStore>,
    sqlite_store: Arc<dyn FullTextStore>,
    reranker: Option<Arc<dyn ResultReranker>>,
}

impl ThreeWayHybridSearchEngine {
    /// 创建三路混合检索引擎。
    pub fn new(
        vector_store: Arc<dyn VectorStore>,
        tantivy_store: Arc<dyn FullTextStore>,
        sqlite_store: Arc<dyn FullTextStore>,
    ) -> Self {
        Self {
            vector_store: Some(vector_store),
            tantivy_store,
            sqlite_store,
            reranker: None,
        }
    }

    /// 创建明确的全文-only 双路检索；不注入伪向量后端，也不报告伪向量健康。
    pub fn without_vector(
        tantivy_store: Arc<dyn FullTextStore>,
        sqlite_store: Arc<dyn FullTextStore>,
    ) -> Self {
        Self {
            vector_store: None,
            tantivy_store,
            sqlite_store,
            reranker: None,
        }
    }

    /// 为三路混合检索挂载 reranker。
    pub fn with_reranker(mut self, reranker: Arc<dyn ResultReranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    fn execute_search(
        &self,
        request: HybridSearchRequest,
        cancellation: Option<&ExecutionCancellation>,
    ) -> CoreResult<Vec<RetrievalResult>> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }
        if let Some(cancellation) = cancellation {
            cancellation.check()?;
        }

        validate_limit(request.limit)?;
        validate_weights(request.vector_weight, request.full_text_weight)?;

        let candidate_limit = request
            .limit
            .checked_mul(3)
            .unwrap_or(MAX_HYBRID_CANDIDATE_LIMIT)
            .min(MAX_HYBRID_CANDIDATE_LIMIT)
            .max(request.limit);
        let mut combined: BTreeMap<String, RetrievalResult> = BTreeMap::new();

        if let Some(query_embedding) = request.query_embedding.clone() {
            let vector_store = self.vector_store.as_ref().ok_or_else(|| {
                CoreError::validation(
                    "query_embedding was provided but vector retrieval is not configured",
                )
            })?;
            let vector_request = VectorSearchRequest {
                query_embedding,
                limit: candidate_limit,
                filters: request.filters.clone(),
            };
            let vector_results = match cancellation {
                Some(cancellation) => {
                    vector_store.search_with_cancellation(vector_request, cancellation)?
                }
                None => vector_store.search(vector_request)?,
            };
            merge_ranked_results(&mut combined, vector_results, request.vector_weight)?;
        }

        if let Some(cancellation) = cancellation {
            cancellation.check()?;
        }
        let text_weight = request.full_text_weight / 2.0;
        let tantivy_results = self.tantivy_store.search(FullTextSearchRequest {
            query: request.query.clone(),
            limit: candidate_limit,
            filters: request.filters.clone(),
        })?;
        merge_ranked_results(&mut combined, tantivy_results, text_weight)?;

        if let Some(cancellation) = cancellation {
            cancellation.check()?;
        }
        let sqlite_results = self.sqlite_store.search(FullTextSearchRequest {
            query: request.query.clone(),
            limit: candidate_limit,
            filters: request.filters,
        })?;
        merge_ranked_results(&mut combined, sqlite_results, text_weight)?;

        let mut results = combined.into_values().collect::<Vec<_>>();
        sort_and_limit(&mut results, candidate_limit);

        if let Some(cancellation) = cancellation {
            cancellation.check()?;
        }
        if let Some(reranker) = &self.reranker {
            return reranker.rerank(RerankInput {
                query: request.query,
                results,
                limit: request.limit,
            });
        }

        sort_and_limit(&mut results, request.limit);
        Ok(results)
    }
}

impl HybridSearchEngine {
    /// 创建不带 reranker 的混合检索引擎。
    pub fn new(
        vector_store: Arc<dyn VectorStore>,
        full_text_store: Arc<dyn FullTextStore>,
    ) -> Self {
        Self {
            vector_store,
            full_text_store,
            reranker: None,
        }
    }

    /// 为混合检索引擎挂载 reranker。
    pub fn with_reranker(mut self, reranker: Arc<dyn ResultReranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }
}

impl HybridSearch for HybridSearchEngine {
    /// 执行混合检索，并按 chunk_id 合并重复结果。
    fn search(&self, request: HybridSearchRequest) -> CoreResult<Vec<RetrievalResult>> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }

        validate_limit(request.limit)?;
        validate_weights(request.vector_weight, request.full_text_weight)?;

        // 先多召回一批候选，再交给 reranker 或最终裁剪，避免过早丢掉可重排结果。
        let candidate_limit = request
            .limit
            .checked_mul(3)
            .unwrap_or(MAX_HYBRID_CANDIDATE_LIMIT)
            .min(MAX_HYBRID_CANDIDATE_LIMIT)
            .max(request.limit);
        let mut combined: BTreeMap<String, RetrievalResult> = BTreeMap::new();

        if let Some(query_embedding) = request.query_embedding.clone() {
            let vector_results = self.vector_store.search(VectorSearchRequest {
                query_embedding,
                limit: candidate_limit,
                filters: request.filters.clone(),
            })?;
            merge_ranked_results(&mut combined, vector_results, request.vector_weight)?;
        }

        let full_text_results = self.full_text_store.search(FullTextSearchRequest {
            query: request.query.clone(),
            limit: candidate_limit,
            filters: request.filters,
        })?;
        merge_ranked_results(&mut combined, full_text_results, request.full_text_weight)?;

        let mut results = combined.into_values().collect::<Vec<_>>();
        sort_and_limit(&mut results, candidate_limit);

        if let Some(reranker) = &self.reranker {
            // reranker 接收合并后的候选集，负责最终排序和裁剪。
            return reranker.rerank(RerankInput {
                query: request.query,
                results,
                limit: request.limit,
            });
        }

        sort_and_limit(&mut results, request.limit);
        Ok(results)
    }

    /// 返回向量和全文两个底层组件的健康状态。
    fn health_check(&self) -> CoreResult<Vec<StoreHealth>> {
        Ok(vec![
            self.vector_store.health_check()?,
            self.full_text_store.health_check()?,
        ])
    }
}

impl HybridSearch for ThreeWayHybridSearchEngine {
    /// 执行 Qdrant + Tantivy + SQLite 三路混合召回。
    fn search(&self, request: HybridSearchRequest) -> CoreResult<Vec<RetrievalResult>> {
        self.execute_search(request, None)
    }

    fn search_with_cancellation(
        &self,
        request: HybridSearchRequest,
        cancellation: &ExecutionCancellation,
    ) -> CoreResult<Vec<RetrievalResult>> {
        self.execute_search(request, Some(cancellation))
    }

    /// 返回三路底层组件健康状态。
    fn health_check(&self) -> CoreResult<Vec<StoreHealth>> {
        let mut health = Vec::with_capacity(3);
        if let Some(vector_store) = &self.vector_store {
            health.push(vector_store.health_check()?);
        }
        health.push(self.tantivy_store.health_check()?);
        health.push(self.sqlite_store.health_check()?);
        Ok(health)
    }
}

/// 校验请求数量上限，避免极端 limit 透传到底层索引。
fn validate_limit(limit: usize) -> CoreResult<()> {
    if limit > MAX_HYBRID_SEARCH_LIMIT {
        return Err(CoreError::validation(format!(
            "hybrid search limit {limit} exceeds maximum {MAX_HYBRID_SEARCH_LIMIT}"
        )));
    }
    Ok(())
}

/// 产品边界使用远低于底层组件防御值的结果上限，避免大数组进入 runtime.db。
pub fn validate_product_search_limit(limit: usize) -> CoreResult<()> {
    if limit > MAX_PRODUCT_SEARCH_LIMIT {
        return Err(CoreError::validation(format!(
            "project search limit {limit} exceeds product maximum {MAX_PRODUCT_SEARCH_LIMIT}"
        )));
    }
    Ok(())
}

/// 在写入 PortValue::Inline 或 IPC 响应前校验完整 JSON 字节预算。
pub fn validate_product_search_result_budget(results: &[RetrievalResult]) -> CoreResult<()> {
    let bytes = serde_json::to_vec(results)?.len();
    if bytes > MAX_PRODUCT_SEARCH_RESULT_BYTES {
        return Err(CoreError::validation(format!(
            "project search results require {bytes} bytes, exceeding inline budget {MAX_PRODUCT_SEARCH_RESULT_BYTES}; reduce limit or chunk size"
        )));
    }
    Ok(())
}

/// 校验混合检索权重，避免 NaN 或全零权重污染排序。
fn validate_weights(vector_weight: f32, full_text_weight: f32) -> CoreResult<()> {
    if !vector_weight.is_finite() || vector_weight < 0.0 {
        return Err(CoreError::validation(
            "vector_weight must be finite and non-negative",
        ));
    }

    if !full_text_weight.is_finite() || full_text_weight < 0.0 {
        return Err(CoreError::validation(
            "full_text_weight must be finite and non-negative",
        ));
    }

    if vector_weight == 0.0 && full_text_weight == 0.0 {
        return Err(CoreError::validation(
            "at least one hybrid search weight must be greater than zero",
        ));
    }

    Ok(())
}

/// 用 RRF 将一组有序结果合并进 combined；后端原始 score 不进入跨源计算。
fn merge_ranked_results(
    combined: &mut BTreeMap<String, RetrievalResult>,
    results: Vec<RetrievalResult>,
    weight: f32,
) -> CoreResult<()> {
    for (rank, mut result) in results.into_iter().enumerate() {
        let layer_weight = retrieval_layer_weight(&result)?;
        result.score = weight * layer_weight / (RRF_K + rank as f32 + 1.0);
        match combined.get_mut(&result.chunk_id) {
            Some(existing) => {
                // 同一 chunk 同时被向量和全文命中时，保留一个结果并累加信号。
                existing.score += result.score;
                existing.source = RetrievalSource::Hybrid;
                if existing.spans.is_empty() {
                    existing.spans = result.spans;
                }
            }
            None => {
                combined.insert(result.chunk_id.clone(), result);
            }
        }
    }
    Ok(())
}

fn retrieval_layer_weight(result: &RetrievalResult) -> CoreResult<f32> {
    let Some(value) = result
        .metadata
        .get("ariadne_retrieval")
        .and_then(|value| value.get("layer_weight"))
    else {
        return Ok(1.0);
    };
    let weight = value.as_f64().ok_or_else(|| {
        CoreError::validation("retrieval layer_weight must be a finite positive number")
    })? as f32;
    if !weight.is_finite() || weight <= 0.0 || weight > 4.0 {
        return Err(CoreError::validation(
            "retrieval layer_weight must be finite and in (0,4]",
        ));
    }
    Ok(weight)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::retrieval::{
        ChunkDocument, FullTextRecord, MemoryFullTextStore, MemoryVectorStore, VectorRecord,
    };

    #[test]
    fn hybrid_search_merges_duplicate_chunks() {
        let vector = Arc::new(MemoryVectorStore::new());
        let full_text = Arc::new(MemoryFullTextStore::new());
        let chunk = ChunkDocument::new("chunk-a", "doc", "needle text");
        vector
            .upsert(vec![VectorRecord {
                chunk: chunk.clone(),
                embedding: vec![1.0, 0.0],
            }])
            .unwrap();
        full_text.upsert(vec![FullTextRecord { chunk }]).unwrap();

        let engine = HybridSearchEngine::new(vector, full_text);
        let results = engine
            .search(HybridSearchRequest::new("needle", Some(vec![1.0, 0.0]), 5))
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, RetrievalSource::Hybrid);
    }

    #[test]
    fn reciprocal_rank_fusion_ignores_incompatible_backend_score_scales() {
        let mut first_scale = BTreeMap::new();
        let mut second_scale = BTreeMap::new();
        let ranked = |first_score, second_score| {
            vec![
                RetrievalResult {
                    chunk_id: "first".to_owned(),
                    document_id: "doc".to_owned(),
                    snippet: "first".to_owned(),
                    score: first_score,
                    source: RetrievalSource::FullText,
                    spans: Vec::new(),
                    metadata: serde_json::Value::Null,
                },
                RetrievalResult {
                    chunk_id: "second".to_owned(),
                    document_id: "doc".to_owned(),
                    snippet: "second".to_owned(),
                    score: second_score,
                    source: RetrievalSource::FullText,
                    spans: Vec::new(),
                    metadata: serde_json::Value::Null,
                },
            ]
        };

        merge_ranked_results(&mut first_scale, ranked(10_000.0, 0.001), 1.0).unwrap();
        merge_ranked_results(&mut second_scale, ranked(0.2, -500.0), 1.0).unwrap();

        assert_eq!(
            first_scale
                .into_values()
                .map(|result| (result.chunk_id, result.score))
                .collect::<Vec<_>>(),
            second_scale
                .into_values()
                .map(|result| (result.chunk_id, result.score))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn product_search_budget_rejects_unbounded_inline_payload() {
        let results = vec![RetrievalResult {
            chunk_id: "oversized".to_owned(),
            document_id: "doc".to_owned(),
            snippet: "文".repeat(MAX_PRODUCT_SEARCH_RESULT_BYTES),
            score: 1.0,
            source: RetrievalSource::FullText,
            spans: Vec::new(),
            metadata: serde_json::Value::Null,
        }];

        let error = validate_product_search_result_budget(&results).unwrap_err();
        assert!(error.to_string().contains("exceeding inline budget"));
    }
}
