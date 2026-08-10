use std::collections::BTreeMap;
use std::sync::Arc;

use crate::config::ProviderConfig;
use crate::contracts::{CoreError, CoreResult};
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
///
/// **U116 接线（2026-07-31）**：此前生产每条链路都现场 `new` 一个 provider，
/// 本注册表零调用；`initialize_all` / `health_check_all` / `shutdown_all`
/// 三项能力在产品里完全不可用。现已接入 `AriadneAppState`。
///
/// ## 缓存为什么按「指纹」而不是按 provider_id
///
/// provider 实例内含用 `ProviderConfig` + API key 构造好的 HTTP 客户端。
/// 只按 id 缓存的话，用户改了 base_url 或换了 key，后端仍会拿旧实例发请求——
/// 而这是**静默**的：请求照发，只是发去了旧地址 / 用了旧凭据。
///
/// 手工在每个「改配置」命令里挂失效（`save_provider_key` / `revoke_provider_key` /
/// `rebind_project_provider_key` / `save_provider_settings` /
/// `save_provider_section_settings` / `remove_provider`，共 6 处）能解决，但
/// **漏一处就是上面那个静默缺陷，且以后新增第七个命令还会再漏**。
///
/// 所以改为**缓存自校验**：键里带上配置与凭据的指纹，配置一变指纹就变，
/// 旧条目自然不再命中。新增改配置的入口无需记得做任何事。
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

/// provider 实例缓存键：provider id + 配置/凭据指纹。
///
/// 指纹一变即视为不同条目，旧实例自然失效——这是「缓存自校验」的落点，
/// 使得**新增改配置的命令无需记得手工失效**（详见 `ProviderRuntimeRegistry` 的说明）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProviderCacheKey {
    pub provider_id: String,
    pub fingerprint: String,
}

impl ProviderCacheKey {
    /// 由 provider 配置与已解析的 API key 计算缓存键。
    ///
    /// 凭据必须参与指纹：它不存在 `ProviderConfig` 里（在 keychain / 密钥文件中），
    /// 只按配置算的话，用户「换了 key 但没改配置」时指纹不变、旧实例继续被命中，
    /// 后端就会拿作废的凭据发请求。
    ///
    /// 这里对 key 取哈希而**不是**存明文：缓存键会出现在错误信息与调试输出里，
    /// 明文凭据不得进入任何可打印结构。
    pub fn new(config: &ProviderConfig, api_key: Option<&str>) -> CoreResult<Self> {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        // 配置侧：整体序列化，字段增减自动纳入指纹，不必逐个列举维护。
        let config_json = serde_json::to_vec(config)
            .map_err(|error| CoreError::validation(format!("provider config is not serializable: {error}")))?;
        hasher.update(&config_json);
        // 凭据侧：用分隔符隔开，避免「空 key + 配置尾部」与「非空 key」拼接后撞哈希。
        hasher.update(b"\x00credential\x00");
        // 用**标签字节**区分「无凭据」与「有凭据」，而不是用一个哨兵字符串：
        // 若 None 直接哈希 `<none>` 字面量，那么 key 恰好等于 `<none>` 的情形
        // 会与「已撤销凭据」撞出同一指纹，撤销后旧实例仍被复用、撤销形同无效。
        // 回归见 `provider_cache_key_distinguishes_missing_credential_from_present_one`。
        match api_key {
            Some(key) => {
                hasher.update([1u8]);
                hasher.update(key.as_bytes());
            }
            None => hasher.update([0u8]),
        }
        Ok(Self {
            provider_id: config.provider_id.clone(),
            fingerprint: format!("{:x}", hasher.finalize()),
        })
    }
}
