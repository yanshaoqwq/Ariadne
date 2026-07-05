use serde::{Deserialize, Serialize};

use crate::contracts::{CoreResult, ProviderDefinition};
use crate::providers::models::{
    EmbeddingRequest, EmbeddingResponse, LlmRequest, LlmResponse, ProviderCallContext,
    RerankRequest, RerankResponse, SearchProviderRequest, SearchProviderResponse,
};

/// Provider 健康状态，用于诊断、恢复和前端展示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Healthy,
    Degraded { reason: String },
    Unhealthy { reason: String },
}

/// 所有 provider 的公共生命周期接口。
pub trait Provider: Send + Sync {
    /// 返回 provider 静态定义。
    fn definition(&self) -> ProviderDefinition;

    /// 初始化 provider，默认无操作。
    fn initialize(&self) -> CoreResult<()> {
        Ok(())
    }

    /// 检查 provider 健康状态，默认认为健康。
    fn health_check(&self) -> CoreResult<ProviderHealth> {
        Ok(ProviderHealth::Healthy)
    }

    /// 关闭 provider，默认无操作。
    fn shutdown(&self) -> CoreResult<()> {
        Ok(())
    }
}

/// LLM provider 接口。
pub trait LlmProvider: Provider {
    /// 执行一次 LLM 生成调用。
    fn complete(
        &self,
        context: &ProviderCallContext,
        request: LlmRequest,
    ) -> CoreResult<LlmResponse>;
}

/// Embedding provider 接口。
pub trait EmbeddingProvider: Provider {
    /// 执行 embedding 调用。
    fn embed(
        &self,
        context: &ProviderCallContext,
        request: EmbeddingRequest,
    ) -> CoreResult<EmbeddingResponse>;
}

/// Reranker provider 接口。
pub trait RerankerProvider: Provider {
    /// 执行重排调用。
    fn rerank(
        &self,
        context: &ProviderCallContext,
        request: RerankRequest,
    ) -> CoreResult<RerankResponse>;
}

/// Search provider 接口。
pub trait SearchProvider: Provider {
    /// 执行外部搜索调用。
    fn search(
        &self,
        context: &ProviderCallContext,
        request: SearchProviderRequest,
    ) -> CoreResult<SearchProviderResponse>;
}
