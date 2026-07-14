use std::collections::BTreeSet;

use crate::contracts::{CoreError, CoreResult};
use crate::providers::{ProviderCallContext, RerankRequest, RerankResult, RerankerProvider};
use crate::retrieval::memory::sort_and_limit;
use crate::retrieval::models::{RerankInput, RetrievalResult};
use crate::retrieval::traits::ResultReranker;

/// 默认 reranker，仅按已有 score 排序。
#[derive(Debug, Default)]
pub struct ScoreReranker;

impl ScoreReranker {
    /// 创建默认 score reranker。
    pub fn new() -> Self {
        Self
    }
}

impl ResultReranker for ScoreReranker {
    /// 按 score 排序并裁剪到指定 limit。
    fn rerank(&self, input: RerankInput) -> CoreResult<Vec<RetrievalResult>> {
        let mut results = input.results;
        sort_and_limit(&mut results, input.limit);
        Ok(results)
    }
}

/// 将 Module 3 的 RerankerProvider 适配成 Module 4 的结果重排器。
pub struct ProviderResultReranker<'a> {
    provider: &'a dyn RerankerProvider,
    context: ProviderCallContext,
    model_id: String,
}

impl<'a> ProviderResultReranker<'a> {
    /// 创建 provider-backed reranker。
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
    /// 调用 provider 重排，并把返回的 index/score 映射回原始检索结果。
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

        apply_rerank_results(&input.results, response.results, input.limit)
    }
}

/// 校验 provider 返回的候选引用并映射回原始结果。
///
/// 即使调用方注入了非 HTTP 实现，越界、重复或非有限分数也必须 fail-loud，
/// 不能在组合层静默丢弃候选。
pub(crate) fn apply_rerank_results(
    candidates: &[RetrievalResult],
    items: Vec<RerankResult>,
    limit: usize,
) -> CoreResult<Vec<RetrievalResult>> {
    let maximum_results = limit.min(candidates.len());
    if items.len() > maximum_results {
        return Err(CoreError::validation(format!(
            "reranker returned {} results, exceeding requested maximum {maximum_results}",
            items.len()
        )));
    }

    let mut seen = BTreeSet::new();
    let mut reranked = Vec::with_capacity(items.len());
    for item in items {
        let mut result = candidates.get(item.index).cloned().ok_or_else(|| {
            CoreError::validation(format!(
                "reranker returned out-of-range document index {}",
                item.index
            ))
        })?;
        if !seen.insert(item.index) {
            return Err(CoreError::validation(format!(
                "reranker returned duplicate document index {}",
                item.index
            )));
        }
        if !item.score.is_finite() {
            return Err(CoreError::validation(
                "reranker returned a non-finite relevance score",
            ));
        }
        result.score = item.score;
        reranked.push(result);
    }

    sort_and_limit(&mut reranked, limit);
    Ok(reranked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::RerankResult;
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

    #[test]
    fn provider_result_mapping_rejects_unknown_candidate_index() {
        let candidates = vec![RetrievalResult {
            chunk_id: "known".to_owned(),
            document_id: "doc".to_owned(),
            snippet: "known".to_owned(),
            score: 0.1,
            source: RetrievalSource::FullText,
            spans: Vec::new(),
            metadata: serde_json::Value::Null,
        }];

        let error = apply_rerank_results(
            &candidates,
            vec![RerankResult {
                index: 1,
                score: 0.9,
            }],
            1,
        )
        .unwrap_err();

        assert!(error.to_string().contains("out-of-range document index 1"));
    }
}
