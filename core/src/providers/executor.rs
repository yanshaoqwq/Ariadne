use serde_json::Value;

use crate::contracts::{CoreError, CoreResult};
use crate::costs::{CostCategory, CostLedger, CostRecord, NewCostRecord};
use crate::providers::models::{
    EmbeddingRequest, EmbeddingResponse, LlmRequest, LlmResponse, ProviderCallContext,
    RerankRequest, RerankResponse, SearchProviderRequest, SearchProviderResponse,
};
use crate::providers::traits::{EmbeddingProvider, LlmProvider, RerankerProvider, SearchProvider};

/// Provider 调用执行器，负责把 provider 返回的费用写入成本账本。
pub struct ProviderExecutor<'a, L: CostLedger + ?Sized> {
    ledger: &'a L,
}

impl<'a, L: CostLedger + ?Sized> ProviderExecutor<'a, L> {
    /// 创建 provider 执行器。
    pub fn new(ledger: &'a L) -> Self {
        Self { ledger }
    }

    /// 执行 LLM 调用并记录可选费用。
    pub fn complete_llm(
        &self,
        provider: &dyn LlmProvider,
        context: &ProviderCallContext,
        request: LlmRequest,
    ) -> CoreResult<LlmResponse> {
        self.complete_llm_with_response_observer(provider, context, request, |_| Ok(()))
    }

    /// 在远端响应返回后、成本落账前先执行持久化观察器。需要 response receipt 的
    /// 调用方可用它关闭“响应已到达但本地成本写入失败”的丢失窗口。
    pub fn complete_llm_with_response_observer(
        &self,
        provider: &dyn LlmProvider,
        context: &ProviderCallContext,
        request: LlmRequest,
        observe_response: impl FnOnce(&LlmResponse) -> CoreResult<()>,
    ) -> CoreResult<LlmResponse> {
        // 调用前捕获 model_id，保证成本记录能按模型归因；provider 消费 request 后就取不到了。
        let model_id = request.model_id.clone();
        validate_provider_identity(provider.definition().provider_id.as_str(), context)?;
        authorize_provider_dispatch(context)?;
        let response = provider.complete(context, request)?;
        observe_response(&response)?;
        self.record_llm_response_cost(context, &model_id, &response)?;
        Ok(response)
    }

    /// 根据已持久化的 LLM response 补写成本。operation_id 唯一索引保证重放幂等。
    pub fn record_llm_response_cost(
        &self,
        context: &ProviderCallContext,
        model_id: &str,
        response: &LlmResponse,
    ) -> CoreResult<()> {
        self.record_optional_cost(
            CostCategory::Llm,
            context,
            Some(model_id.to_owned()),
            response.cost_usd,
            response.usage,
            response.raw.clone(),
        )
        .map(|_| ())
    }

    /// 执行 embedding 调用并记录可选费用。
    pub fn embed(
        &self,
        provider: &dyn EmbeddingProvider,
        context: &ProviderCallContext,
        request: EmbeddingRequest,
    ) -> CoreResult<EmbeddingResponse> {
        // 调用前捕获 model_id，供 embedding 成本按模型归因。
        let model_id = request.model_id.clone();
        validate_provider_identity(provider.definition().provider_id.as_str(), context)?;
        authorize_provider_dispatch(context)?;
        let response = provider.embed(context, request)?;
        self.record_optional_cost(
            CostCategory::Embedding,
            context,
            Some(model_id),
            response.cost_usd,
            response.usage,
            response.raw.clone(),
        )?;
        Ok(response)
    }

    /// 执行 reranker 调用并记录可选费用。
    pub fn rerank(
        &self,
        provider: &dyn RerankerProvider,
        context: &ProviderCallContext,
        request: RerankRequest,
    ) -> CoreResult<RerankResponse> {
        // reranker 请求也带 model_id，一并归因到成本记录。
        let model_id = request.model_id.clone();
        validate_provider_identity(provider.definition().provider_id.as_str(), context)?;
        authorize_provider_dispatch(context)?;
        let response = provider.rerank(context, request)?;
        self.record_optional_cost(
            CostCategory::Reranker,
            context,
            Some(model_id),
            response.cost_usd,
            None,
            response.raw.clone(),
        )?;
        Ok(response)
    }

    /// 执行 search provider 调用并记录可选费用。
    pub fn search(
        &self,
        provider: &dyn SearchProvider,
        context: &ProviderCallContext,
        request: SearchProviderRequest,
    ) -> CoreResult<SearchProviderResponse> {
        validate_provider_identity(provider.definition().provider_id.as_str(), context)?;
        authorize_provider_dispatch(context)?;
        let response = provider.search(context, request)?;
        self.record_optional_cost(
            CostCategory::SearchApi,
            context,
            None,
            response.cost_usd,
            None,
            response.raw.clone(),
        )?;
        Ok(response)
    }

    /// provider 未返回费用时不写账；返回费用时统一校验并写入成本账本。
    fn record_optional_cost(
        &self,
        category: CostCategory,
        context: &ProviderCallContext,
        model_id: Option<String>,
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
                operation_id: context.operation_id.clone(),
                category,
                provider_id: Some(context.provider_id.clone()),
                model_id,
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

fn validate_provider_identity(
    actual_provider_id: &str,
    context: &ProviderCallContext,
) -> CoreResult<()> {
    if actual_provider_id == context.provider_id {
        return Ok(());
    }
    Err(CoreError::validation(format!(
        "provider dispatch identity mismatch: request declares '{}', actual provider is '{}'",
        context.provider_id, actual_provider_id
    )))
}

fn authorize_provider_dispatch(context: &ProviderCallContext) -> CoreResult<()> {
    if context.cancellation.is_cancelled() {
        return Err(CoreError::external_cancelled(
            "provider",
            crate::contracts::ExternalDispatchOutcome::NotDispatched,
        ));
    }
    context.dispatch_authorization.authorize_dispatch()
}

/// 返回当前 Unix 毫秒时间戳。
fn unix_timestamp_ms() -> CoreResult<u64> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            CoreError::validation(format!("system time before unix epoch: {error}"))
        })?;
    u64::try_from(duration.as_millis())
        .map_err(|_| CoreError::validation("timestamp exceeds u64 range"))
}
