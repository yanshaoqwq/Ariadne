use crate::core::CoreResult;
use crate::providers::{ProviderCallContext, RerankRequest, RerankerProvider};
use crate::retrieval::memory::sort_and_limit;
use crate::retrieval::models::{RerankInput, RetrievalResult};
use crate::retrieval::traits::ResultReranker;

#[derive(Debug, Default)]
pub struct ScoreReranker;

impl ScoreReranker {
    pub fn new() -> Self {
        Self
    }
}

impl ResultReranker for ScoreReranker {
    fn rerank(&self, input: RerankInput) -> CoreResult<Vec<RetrievalResult>> {
        let mut results = input.results;
        sort_and_limit(&mut results, input.limit);
        Ok(results)
    }
}

pub struct ProviderResultReranker<'a> {
    provider: &'a dyn RerankerProvider,
    context: ProviderCallContext,
    model_id: String,
}

impl<'a> ProviderResultReranker<'a> {
    pub fn new(
        provider: &'a dyn RerankerProvider,
        context: ProviderCallContext,
        model_id: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            context,
            model_id: model_id.into(),
        }
    }
}

impl ResultReranker for ProviderResultReranker<'_> {
    fn rerank(&self, input: RerankInput) -> CoreResult<Vec<RetrievalResult>> {
        let response = self.provider.rerank(
            &self.context,
            RerankRequest {
                model_id: self.model_id.clone(),
                query: input.query,
                documents: input
                    .results
                    .iter()
                    .map(|result| result.snippet.clone())
                    .collect(),
                top_n: Some(input.limit),
                metadata: serde_json::Value::Null,
            },
        )?;

        let mut reranked = Vec::new();
        for item in response.results {
            if let Some(mut result) = input.results.get(item.index).cloned() {
                result.score = item.score;
                reranked.push(result);
            }
        }

        sort_and_limit(&mut reranked, input.limit);
        Ok(reranked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::models::{RetrievalResult, RetrievalSource};

    #[test]
    fn score_reranker_applies_limit() {
        let reranker = ScoreReranker::new();
        let results = reranker
            .rerank(RerankInput {
                query: "needle".to_owned(),
                limit: 1,
                results: vec![
                    RetrievalResult {
                        chunk_id: "low".to_owned(),
                        document_id: "doc".to_owned(),
                        snippet: "low".to_owned(),
                        score: 0.1,
                        source: RetrievalSource::FullText,
                        spans: Vec::new(),
                        metadata: serde_json::Value::Null,
                    },
                    RetrievalResult {
                        chunk_id: "high".to_owned(),
                        document_id: "doc".to_owned(),
                        snippet: "high".to_owned(),
                        score: 0.9,
                        source: RetrievalSource::Vector,
                        spans: Vec::new(),
                        metadata: serde_json::Value::Null,
                    },
                ],
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "high");
    }
}
