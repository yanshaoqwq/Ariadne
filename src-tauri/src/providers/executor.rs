use serde_json::Value;

use crate::core::{CoreError, CoreResult};
use crate::costs::{CostCategory, CostLedger, CostRecord, NewCostRecord};
use crate::providers::models::{
    EmbeddingRequest, EmbeddingResponse, LlmRequest, LlmResponse, ProviderCallContext,
    RerankRequest, RerankResponse, SearchProviderRequest, SearchProviderResponse,
};
use crate::providers::traits::{EmbeddingProvider, LlmProvider, RerankerProvider, SearchProvider};

pub struct ProviderExecutor<'a, L: CostLedger> {
    ledger: &'a L,
}

impl<'a, L: CostLedger> ProviderExecutor<'a, L> {
    pub fn new(ledger: &'a L) -> Self {
        Self { ledger }
    }

    pub fn complete_llm(
        &self,
        provider: &dyn LlmProvider,
        context: &ProviderCallContext,
        request: LlmRequest,
    ) -> CoreResult<LlmResponse> {
        let response = provider.complete(context, request)?;
        self.record_optional_cost(
            CostCategory::Llm,
            context,
            response.cost_usd,
            response.usage,
            response.raw.clone(),
        )?;
        Ok(response)
    }

    pub fn embed(
        &self,
        provider: &dyn EmbeddingProvider,
        context: &ProviderCallContext,
        request: EmbeddingRequest,
    ) -> CoreResult<EmbeddingResponse> {
        let response = provider.embed(context, request)?;
        self.record_optional_cost(
            CostCategory::Embedding,
            context,
            response.cost_usd,
            response.usage,
            response.raw.clone(),
        )?;
        Ok(response)
    }

    pub fn rerank(
        &self,
        provider: &dyn RerankerProvider,
        context: &ProviderCallContext,
        request: RerankRequest,
    ) -> CoreResult<RerankResponse> {
        let response = provider.rerank(context, request)?;
        self.record_optional_cost(
            CostCategory::Reranker,
            context,
            response.cost_usd,
            None,
            response.raw.clone(),
        )?;
        Ok(response)
    }

    pub fn search(
        &self,
        provider: &dyn SearchProvider,
        context: &ProviderCallContext,
        request: SearchProviderRequest,
    ) -> CoreResult<SearchProviderResponse> {
        let response = provider.search(context, request)?;
        self.record_optional_cost(
            CostCategory::SearchApi,
            context,
            response.cost_usd,
            None,
            response.raw.clone(),
        )?;
        Ok(response)
    }

    fn record_optional_cost(
        &self,
        category: CostCategory,
        context: &ProviderCallContext,
        amount_usd: Option<f64>,
        usage: Option<crate::costs::TokenUsage>,
        raw: Value,
    ) -> CoreResult<Option<CostRecord>> {
        let Some(amount_usd) = amount_usd else {
            return Ok(None);
        };

        if !amount_usd.is_finite() || amount_usd < 0.0 {
            return Err(CoreError::validation(
                "provider response cost_usd must be finite and non-negative",
            ));
        }

        self.ledger
            .record_cost(NewCostRecord {
                occurred_at_ms: unix_timestamp_ms()?,
                category,
                provider_id: Some(context.provider_id.clone()),
                model_id: None,
                workflow_id: context.workflow_id.clone(),
                run_id: context.run_id.clone(),
                node_id: context.node_id.clone(),
                tool_call_id: context.tool_call_id.clone(),
                input_tokens: usage.map(|usage| usage.input_tokens),
                output_tokens: usage.map(|usage| usage.output_tokens),
                amount_usd,
                metadata: raw,
            })
            .map(Some)
    }
}

fn unix_timestamp_ms() -> CoreResult<u64> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            CoreError::validation(format!("system time before unix epoch: {error}"))
        })?;
    u64::try_from(duration.as_millis())
        .map_err(|_| CoreError::validation("timestamp exceeds u64 range"))
}
