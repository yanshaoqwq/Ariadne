use std::sync::Arc;

use serde_json::Value;

use crate::contracts::{CoreError, CoreResult};
use crate::costs::CostLedger;
use crate::providers::{
    EmbeddingProvider, EmbeddingRequest, ProviderCallContext, ProviderExecutor, ProviderHealth,
};
use crate::retrieval::{StoreHealth, TextEmbedder};

/// 将标准 EmbeddingProvider 适配成检索层的批量文本向量化端口。
pub struct ProviderTextEmbedder {
    provider: Arc<dyn EmbeddingProvider>,
    ledger: Arc<dyn CostLedger>,
    provider_id: String,
    model_id: String,
    dimensions: usize,
}

impl ProviderTextEmbedder {
    pub fn new(
        provider: Arc<dyn EmbeddingProvider>,
        ledger: Arc<dyn CostLedger>,
        model_id: impl Into<String>,
        dimensions: usize,
    ) -> CoreResult<Self> {
        let provider_id = provider.definition().provider_id;
        if provider_id.trim().is_empty() {
            return Err(CoreError::validation(
                "embedding provider id cannot be empty",
            ));
        }
        let model_id = model_id.into();
        if model_id.trim().is_empty() {
            return Err(CoreError::validation("embedding model id cannot be empty"));
        }
        if dimensions == 0 {
            return Err(CoreError::validation(
                "embedding dimensions must be positive",
            ));
        }
        Ok(Self {
            provider,
            ledger,
            provider_id,
            model_id,
            dimensions,
        })
    }
}

impl TextEmbedder for ProviderTextEmbedder {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(
        &self,
        mut context: ProviderCallContext,
        inputs: Vec<String>,
    ) -> CoreResult<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        context.provider_id = self.provider_id.clone();
        let expected_count = inputs.len();
        let response = ProviderExecutor::new(self.ledger.as_ref()).embed(
            self.provider.as_ref(),
            &context,
            EmbeddingRequest {
                model_id: self.model_id.clone(),
                inputs,
                metadata: Value::Null,
            },
        )?;
        validate_embeddings(expected_count, self.dimensions, &response.embeddings)?;
        Ok(response.embeddings)
    }

    fn health_check(&self) -> CoreResult<StoreHealth> {
        match self.provider.health_check()? {
            // 通用 HTTP provider 没有可靠的无副作用探针；明确标为 degraded，真实可用性
            // 由首次 embedding 调用验证，不能把“配置存在”误报成远端健康。
            ProviderHealth::Healthy => Ok(StoreHealth::degraded(
                "embedding_provider",
                format!(
                    "provider {} is configured; remote endpoint is verified on embedding calls",
                    self.provider_id
                ),
            )),
            ProviderHealth::Degraded { reason } => {
                Ok(StoreHealth::degraded("embedding_provider", reason))
            }
            ProviderHealth::Unhealthy { reason } => {
                Ok(StoreHealth::unavailable("embedding_provider", reason))
            }
        }
    }
}

fn validate_embeddings(
    expected_count: usize,
    expected_dimensions: usize,
    embeddings: &[Vec<f32>],
) -> CoreResult<()> {
    if embeddings.len() != expected_count {
        return Err(CoreError::validation(format!(
            "embedding provider returned {} vectors for {expected_count} inputs",
            embeddings.len()
        )));
    }
    for embedding in embeddings {
        if embedding.len() != expected_dimensions {
            return Err(CoreError::validation(format!(
                "embedding dimension {} does not match configured dimension {expected_dimensions}",
                embedding.len()
            )));
        }
        if embedding.iter().any(|value| !value.is_finite()) {
            return Err(CoreError::validation(
                "embedding contains a non-finite value",
            ));
        }
    }
    Ok(())
}
