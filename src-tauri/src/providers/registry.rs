use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::{CoreError, CoreResult};
use crate::providers::traits::{EmbeddingProvider, LlmProvider, RerankerProvider, SearchProvider};

#[derive(Default)]
pub struct ProviderRuntimeRegistry {
    llm: BTreeMap<String, Arc<dyn LlmProvider>>,
    embedding: BTreeMap<String, Arc<dyn EmbeddingProvider>>,
    reranker: BTreeMap<String, Arc<dyn RerankerProvider>>,
    search: BTreeMap<String, Arc<dyn SearchProvider>>,
}

impl ProviderRuntimeRegistry {
    pub fn register_llm(
        &mut self,
        provider_id: impl Into<String>,
        provider: Arc<dyn LlmProvider>,
    ) -> CoreResult<()> {
        register(&mut self.llm, "llm_provider", provider_id.into(), provider)
    }

    pub fn register_embedding(
        &mut self,
        provider_id: impl Into<String>,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> CoreResult<()> {
        register(
            &mut self.embedding,
            "embedding_provider",
            provider_id.into(),
            provider,
        )
    }

    pub fn register_reranker(
        &mut self,
        provider_id: impl Into<String>,
        provider: Arc<dyn RerankerProvider>,
    ) -> CoreResult<()> {
        register(
            &mut self.reranker,
            "reranker_provider",
            provider_id.into(),
            provider,
        )
    }

    pub fn register_search(
        &mut self,
        provider_id: impl Into<String>,
        provider: Arc<dyn SearchProvider>,
    ) -> CoreResult<()> {
        register(
            &mut self.search,
            "search_provider",
            provider_id.into(),
            provider,
        )
    }

    pub fn llm(&self, provider_id: &str) -> CoreResult<Arc<dyn LlmProvider>> {
        get(&self.llm, "llm_provider", provider_id)
    }

    pub fn embedding(&self, provider_id: &str) -> CoreResult<Arc<dyn EmbeddingProvider>> {
        get(&self.embedding, "embedding_provider", provider_id)
    }

    pub fn reranker(&self, provider_id: &str) -> CoreResult<Arc<dyn RerankerProvider>> {
        get(&self.reranker, "reranker_provider", provider_id)
    }

    pub fn search(&self, provider_id: &str) -> CoreResult<Arc<dyn SearchProvider>> {
        get(&self.search, "search_provider", provider_id)
    }
}

fn register<T>(
    registry: &mut BTreeMap<String, Arc<T>>,
    registry_name: &'static str,
    provider_id: String,
    provider: Arc<T>,
) -> CoreResult<()>
where
    T: ?Sized,
{
    if provider_id.trim().is_empty() {
        return Err(CoreError::validation("provider_id cannot be empty"));
    }

    if registry.contains_key(&provider_id) {
        return Err(CoreError::RegistryDuplicate {
            registry: registry_name,
            key: provider_id,
        });
    }

    registry.insert(provider_id, provider);
    Ok(())
}

fn get<T>(
    registry: &BTreeMap<String, Arc<T>>,
    registry_name: &'static str,
    provider_id: &str,
) -> CoreResult<Arc<T>>
where
    T: ?Sized,
{
    registry
        .get(provider_id)
        .cloned()
        .ok_or_else(|| CoreError::RegistryMissing {
            registry: registry_name,
            key: provider_id.to_owned(),
        })
}
