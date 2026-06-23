use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::{CoreError, CoreResult};
use crate::retrieval::memory::sort_and_limit;
use crate::retrieval::models::{
    FullTextSearchRequest, HybridSearchRequest, RerankInput, RetrievalResult, RetrievalSource,
    StoreHealth, VectorSearchRequest,
};
use crate::retrieval::traits::{FullTextStore, HybridSearch, ResultReranker, VectorStore};

pub struct HybridSearchEngine {
    vector_store: Arc<dyn VectorStore>,
    full_text_store: Arc<dyn FullTextStore>,
    reranker: Option<Arc<dyn ResultReranker>>,
}

impl HybridSearchEngine {
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

    pub fn with_reranker(mut self, reranker: Arc<dyn ResultReranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }
}

impl HybridSearch for HybridSearchEngine {
    fn search(&self, request: HybridSearchRequest) -> CoreResult<Vec<RetrievalResult>> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }

        validate_weights(request.vector_weight, request.full_text_weight)?;

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
            return reranker.rerank(RerankInput {
                query: request.query,
                results,
                limit: request.limit,
            });
        }

        sort_and_limit(&mut results, request.limit);
        Ok(results)
    }

    fn health_check(&self) -> CoreResult<Vec<StoreHealth>> {
        Ok(vec![
            self.vector_store.health_check()?,
            self.full_text_store.health_check()?,
        ])
    }
}

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

fn merge_results(
    combined: &mut BTreeMap<String, RetrievalResult>,
    results: Vec<RetrievalResult>,
    weight: f32,
) {
    for mut result in results {
        result.score *= weight;
        match combined.get_mut(&result.chunk_id) {
            Some(existing) => {
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
