use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::{CoreError, CoreResult};
use crate::retrieval::memory::sort_and_limit;
use crate::retrieval::models::{
    FullTextSearchRequest, HybridSearchRequest, RerankInput, RetrievalResult, RetrievalSource,
    StoreHealth, VectorSearchRequest,
};
use crate::retrieval::traits::{FullTextStore, HybridSearch, ResultReranker, VectorStore};

/// 混合检索引擎，组合向量检索、全文检索和可选 reranker。
pub struct HybridSearchEngine {
    vector_store: Arc<dyn VectorStore>,
    full_text_store: Arc<dyn FullTextStore>,
    reranker: Option<Arc<dyn ResultReranker>>,
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

        validate_weights(request.vector_weight, request.full_text_weight)?;

        // 先多召回一批候选，再交给 reranker 或最终裁剪，避免过早丢掉可重排结果。
        let candidate_limit = request.limit.saturating_mul(3).max(request.limit);
        let mut combined: BTreeMap<String, RetrievalResult> = BTreeMap::new();

        if let Some(query_embedding) = request.query_embedding.clone() {
            let vector_results = self.vector_store.search(VectorSearchRequest {
                query_embedding,
                limit: candidate_limit,
                filters: request.filters.clone(),
            })?;
            merge_results(&mut combined, vector_results, request.vector_weight);
        }

        let full_text_results = self.full_text_store.search(FullTextSearchRequest {
            query: request.query.clone(),
            limit: candidate_limit,
            filters: request.filters,
        })?;
        merge_results(&mut combined, full_text_results, request.full_text_weight);

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

/// 将一组检索结果合并进 combined，同一 chunk 的分数按权重累加。
fn merge_results(
    combined: &mut BTreeMap<String, RetrievalResult>,
    results: Vec<RetrievalResult>,
    weight: f32,
) {
    for mut result in results {
        result.score *= weight;
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
}
