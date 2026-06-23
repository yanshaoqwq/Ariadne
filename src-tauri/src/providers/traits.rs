use serde::{Deserialize, Serialize};

use crate::core::{CoreResult, ProviderDefinition};
use crate::providers::models::{
    EmbeddingRequest, EmbeddingResponse, LlmRequest, LlmResponse, ProviderCallContext,
    RerankRequest, RerankResponse, SearchProviderRequest, SearchProviderResponse,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Healthy,
    Degraded { reason: String },
    Unhealthy { reason: String },
}

pub trait Provider: Send + Sync {
    fn definition(&self) -> ProviderDefinition;

    fn initialize(&self) -> CoreResult<()> {
        Ok(())
    }

    fn health_check(&self) -> CoreResult<ProviderHealth> {
        Ok(ProviderHealth::Healthy)
    }

    fn shutdown(&self) -> CoreResult<()> {
        Ok(())
    }
}

pub trait LlmProvider: Provider {
    fn complete(
        &self,
        context: &ProviderCallContext,
        request: LlmRequest,
    ) -> CoreResult<LlmResponse>;
}

pub trait EmbeddingProvider: Provider {
    fn embed(
        &self,
        context: &ProviderCallContext,
        request: EmbeddingRequest,
    ) -> CoreResult<EmbeddingResponse>;
}

pub trait RerankerProvider: Provider {
    fn rerank(
        &self,
        context: &ProviderCallContext,
        request: RerankRequest,
    ) -> CoreResult<RerankResponse>;
}

pub trait SearchProvider: Provider {
    fn search(
        &self,
        context: &ProviderCallContext,
        request: SearchProviderRequest,
    ) -> CoreResult<SearchProviderResponse>;
}
