use std::collections::BTreeMap;
use std::sync::Arc;

use crate::core::{CoreError, CoreResult};
use crate::providers::traits::{
    EmbeddingProvider, LlmProvider, Provider, ProviderHealth, RerankerProvider, SearchProvider,
};

/// 运行时 provider 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Llm,
    Embedding,
    Reranker,
    Search,
}

/// Provider 初始化或关闭报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLifecycleReport {
    pub provider_id: String,
    pub kind: ProviderKind,
    pub success: bool,
    pub reason: Option<String>,
}

/// Provider 健康检查报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHealthReport {
    pub provider_id: String,
    pub kind: ProviderKind,
    pub health: ProviderHealth,
}

/// 运行时 provider 注册表，按能力类型分别索引 provider。
#[derive(Default)]
pub struct ProviderRuntimeRegistry {
    llm: BTreeMap<String, Arc<dyn LlmProvider>>,
    embedding: BTreeMap<String, Arc<dyn EmbeddingProvider>>,
    reranker: BTreeMap<String, Arc<dyn RerankerProvider>>,
    search: BTreeMap<String, Arc<dyn SearchProvider>>,
}

impl ProviderRuntimeRegistry {
    /// 注册 LLM provider。
    pub fn register_llm(
        &mut self,
        provider_id: impl Into<String>,
        provider: Arc<dyn LlmProvider>,
    ) -> CoreResult<()> {
        register(&mut self.llm, "llm_provider", provider_id.into(), provider)
    }

    /// 注册 embedding provider。
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

    /// 注册 reranker provider。
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

    /// 注册 search provider。
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

    /// 读取 LLM provider。
    pub fn llm(&self, provider_id: &str) -> CoreResult<Arc<dyn LlmProvider>> {
        get(&self.llm, "llm_provider", provider_id)
    }

    /// 读取 embedding provider。
    pub fn embedding(&self, provider_id: &str) -> CoreResult<Arc<dyn EmbeddingProvider>> {
        get(&self.embedding, "embedding_provider", provider_id)
    }

    /// 读取 reranker provider。
    pub fn reranker(&self, provider_id: &str) -> CoreResult<Arc<dyn RerankerProvider>> {
        get(&self.reranker, "reranker_provider", provider_id)
    }

    /// 读取 search provider。
    pub fn search(&self, provider_id: &str) -> CoreResult<Arc<dyn SearchProvider>> {
        get(&self.search, "search_provider", provider_id)
    }

    /// 初始化所有已注册 provider，并收集每个 provider 的结果。
    pub fn initialize_all(&self) -> Vec<ProviderLifecycleReport> {
        let mut reports = Vec::new();
        collect_lifecycle_reports(&mut reports, ProviderKind::Llm, &self.llm, |provider| {
            provider.initialize()
        });
        collect_lifecycle_reports(
            &mut reports,
            ProviderKind::Embedding,
            &self.embedding,
            |provider| provider.initialize(),
        );
        collect_lifecycle_reports(
            &mut reports,
            ProviderKind::Reranker,
            &self.reranker,
            |provider| provider.initialize(),
        );
        collect_lifecycle_reports(
            &mut reports,
            ProviderKind::Search,
            &self.search,
            |provider| provider.initialize(),
        );
        reports
    }

    /// 检查所有已注册 provider 的健康状态。
    pub fn health_check_all(&self) -> Vec<ProviderHealthReport> {
        let mut reports = Vec::new();
        collect_health_reports(&mut reports, ProviderKind::Llm, &self.llm);
        collect_health_reports(&mut reports, ProviderKind::Embedding, &self.embedding);
        collect_health_reports(&mut reports, ProviderKind::Reranker, &self.reranker);
        collect_health_reports(&mut reports, ProviderKind::Search, &self.search);
        reports
    }

    /// 关闭所有已注册 provider，并收集每个 provider 的结果。
    pub fn shutdown_all(&self) -> Vec<ProviderLifecycleReport> {
        let mut reports = Vec::new();
        collect_lifecycle_reports(&mut reports, ProviderKind::Llm, &self.llm, |provider| {
            provider.shutdown()
        });
        collect_lifecycle_reports(
            &mut reports,
            ProviderKind::Embedding,
            &self.embedding,
            |provider| provider.shutdown(),
        );
        collect_lifecycle_reports(
            &mut reports,
            ProviderKind::Reranker,
            &self.reranker,
            |provider| provider.shutdown(),
        );
        collect_lifecycle_reports(
            &mut reports,
            ProviderKind::Search,
            &self.search,
            |provider| provider.shutdown(),
        );
        reports
    }
}

/// 注册 provider，统一处理空 id 和重复 id。
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

/// 读取 provider，统一处理缺失错误。
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

/// 对一类 provider 执行生命周期动作并收集报告。
fn collect_lifecycle_reports<T, F>(
    reports: &mut Vec<ProviderLifecycleReport>,
    kind: ProviderKind,
    registry: &BTreeMap<String, Arc<T>>,
    action: F,
) where
    T: Provider + ?Sized,
    F: Fn(&T) -> CoreResult<()>,
{
    for (provider_id, provider) in registry {
        match action(provider.as_ref()) {
            Ok(()) => reports.push(ProviderLifecycleReport {
                provider_id: provider_id.clone(),
                kind,
                success: true,
                reason: None,
            }),
            Err(error) => reports.push(ProviderLifecycleReport {
                provider_id: provider_id.clone(),
                kind,
                success: false,
                reason: Some(error.to_string()),
            }),
        }
    }
}

/// 对一类 provider 执行健康检查并收集报告。
fn collect_health_reports<T>(
    reports: &mut Vec<ProviderHealthReport>,
    kind: ProviderKind,
    registry: &BTreeMap<String, Arc<T>>,
) where
    T: Provider + ?Sized,
{
    for (provider_id, provider) in registry {
        let health = provider
            .health_check()
            .unwrap_or_else(|error| ProviderHealth::Unhealthy {
                // 健康检查本身失败也要转成报告，避免诊断接口整体失败。
                reason: error.to_string(),
            });
        reports.push(ProviderHealthReport {
            provider_id: provider_id.clone(),
            kind,
            health,
        });
    }
}
