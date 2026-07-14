use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;

use crate::config::{
    ConfigStore, ProjectConfig, ProjectCredentialScope, ProviderConfig, SecretStore,
    VectorStoreBackend, VectorStoreConfig,
};
use crate::contracts::{
    ensure_path_under_root, CoreError, CoreResult, ExternalDispatchOutcome, ProviderCapability,
};
use crate::costs::{CostLedger, SqliteCostLedger};
use crate::documents::IndexInvalidationOutbox;
use crate::providers::{
    HttpEmbeddingProvider, HttpRerankerProvider, ProviderCallContext, ProviderExecutor,
    ProviderHealth, RerankRequest, RerankerProvider,
};
use crate::retrieval::reranker::apply_rerank_results;
use crate::retrieval::{
    ensure_search_not_blocked_by_pending_index,
    filter_fresh_retrieval_results_with_knowledge_revision, validate_product_search_limit,
    validate_product_search_result_budget, FullTextStore, HybridSearch, HybridSearchRequest,
    IndexingWorker, KnowledgeIndexSyncReport, KnowledgeIndexSynchronizer, QdrantSidecarConfig,
    QdrantSidecarSupervisor, QdrantVectorStore, RetrievalResult, SidecarState, SqliteFullTextStore,
    StoreHealth, TantivyFullTextStore, TextEmbedder, ThreeWayHybridSearchEngine, VectorStore,
    MAX_HYBRID_SEARCH_LIMIT,
};

struct ProjectReranker {
    provider: Arc<dyn RerankerProvider>,
    ledger: Arc<dyn CostLedger>,
    provider_id: String,
    model_id: String,
}

/// 单个已打开项目的检索组合根。所有生产搜索、索引、诊断和 sidecar 生命周期共用它。
pub struct ProjectRetrievalRuntime {
    project_root: PathBuf,
    config: ProjectConfig,
    outbox: IndexInvalidationOutbox,
    tantivy_path: PathBuf,
    sqlite_path: PathBuf,
    tantivy: Arc<dyn FullTextStore>,
    sqlite: Arc<dyn FullTextStore>,
    vector: Option<Arc<dyn VectorStore>>,
    embedder: Option<Arc<dyn TextEmbedder>>,
    reranker: Option<ProjectReranker>,
    knowledge_index: KnowledgeIndexSynchronizer,
    vector_signature: Option<String>,
    sidecar: Option<Arc<QdrantSidecarSupervisor>>,
    chunk_size_chars: usize,
    chunk_overlap_chars: usize,
}

impl ProjectRetrievalRuntime {
    /// 从项目配置和可信的项目凭据作用域构造完整运行时。
    pub fn open(project_root: &Path, secrets: &dyn SecretStore) -> CoreResult<Self> {
        let project_root = project_root.canonicalize()?;
        let config = ConfigStore::new(&project_root).load_or_create()?;
        Self::from_config(&project_root, secrets, &config, None)
    }

    /// 从候选配置构造新 generation；未变的索引/sidecar 与旧 generation 共享。
    pub fn from_config(
        project_root: &Path,
        secrets: &dyn SecretStore,
        config: &ProjectConfig,
        previous: Option<&Self>,
    ) -> CoreResult<Self> {
        let project_root = project_root.canonicalize()?;
        config.validate()?;

        let tantivy_path = resolve_project_path(
            &project_root,
            Path::new(&config.rag.full_text_store.index_dir),
        )?;
        let sqlite_path = project_root.join(".indexes").join("full_text.db");
        if let Some(parent) = sqlite_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let reusable = previous.filter(|runtime| runtime.project_root == project_root);
        let tantivy: Arc<dyn FullTextStore> =
            match reusable.filter(|runtime| runtime.tantivy_path == tantivy_path) {
                Some(runtime) => Arc::clone(&runtime.tantivy),
                None => Arc::new(TantivyFullTextStore::open(&tantivy_path)?),
            };
        let sqlite: Arc<dyn FullTextStore> =
            match reusable.filter(|runtime| runtime.sqlite_path == sqlite_path) {
                Some(runtime) => Arc::clone(&runtime.sqlite),
                None => Arc::new(SqliteFullTextStore::open(&sqlite_path)?),
            };
        let ledger: Arc<dyn CostLedger> = Arc::new(SqliteCostLedger::open(&project_root)?);
        let credentials = ProjectCredentialScope::new(&project_root, secrets)?;

        let embedder = if config.rag.vector_store.enabled {
            let (provider_config, model_id) = select_capability_provider(
                &config.providers,
                config.providers.default_embedding_provider_id.as_deref(),
                ProviderCapability::Embedding,
                "embedding",
            )?;
            let api_key = resolve_provider_secret(&credentials, &provider_config, true)?;
            let provider: Arc<dyn crate::providers::EmbeddingProvider> =
                Arc::new(HttpEmbeddingProvider::new(provider_config, api_key)?);
            Some(Arc::new(crate::retrieval::ProviderTextEmbedder::new(
                provider,
                Arc::clone(&ledger),
                model_id,
                config.rag.vector_store.vector_dimensions as usize,
            )?) as Arc<dyn TextEmbedder>)
        } else {
            None
        };

        let reranker = if config.rag.reranker_enabled {
            let (provider_config, model_id) = select_capability_provider(
                &config.providers,
                config.providers.default_reranker_provider_id.as_deref(),
                ProviderCapability::Reranker,
                "reranker",
            )?;
            let api_key = resolve_provider_secret(&credentials, &provider_config, false)?;
            let provider: Arc<dyn RerankerProvider> =
                Arc::new(HttpRerankerProvider::new(provider_config, api_key)?);
            let provider_id = provider.definition().provider_id;
            Some(ProjectReranker {
                provider,
                ledger: Arc::clone(&ledger),
                provider_id,
                model_id,
            })
        } else {
            None
        };

        let (vector, sidecar) = if reusable
            .is_some_and(|runtime| runtime.config.rag.vector_store == config.rag.vector_store)
        {
            let runtime = reusable.expect("reusable runtime checked above");
            (runtime.vector.clone(), runtime.sidecar.clone())
        } else if config.rag.vector_store.enabled {
            let mut sidecar = None;
            let vector_config = &config.rag.vector_store;
            let endpoint = match vector_config.backend {
                VectorStoreBackend::QdrantSidecar => {
                    let data_dir = resolve_project_path(
                        &project_root,
                        Path::new(&vector_config.sidecar.data_dir),
                    )?;
                    let log_dir = project_root.join(".runtime").join("logs").join("qdrant");
                    let supervisor = Arc::new(QdrantSidecarSupervisor::new(QdrantSidecarConfig {
                        binary_path: PathBuf::from(&vector_config.sidecar.binary_path),
                        host: vector_config.sidecar.host.clone(),
                        requested_port: vector_config.sidecar.port,
                        data_dir,
                        log_dir,
                        startup_timeout_ms: vector_config.sidecar.startup_timeout_ms,
                    }));
                    let status = supervisor.start()?;
                    if matches!(
                        status.state,
                        SidecarState::Stopped | SidecarState::Unavailable
                    ) {
                        let _ = supervisor.stop();
                        return Err(CoreError::External {
                            service: "qdrant_sidecar".to_owned(),
                            message: status
                                .reason
                                .unwrap_or_else(|| "sidecar did not start".to_owned()),
                        });
                    }
                    let endpoint = status.endpoint.ok_or_else(|| {
                        CoreError::validation("running qdrant sidecar did not expose an endpoint")
                    })?;
                    sidecar = Some(supervisor);
                    endpoint
                }
                VectorStoreBackend::ExternalQdrant => format!(
                    "http://{}:{}",
                    vector_config.sidecar.host.trim(),
                    vector_config.sidecar.port
                ),
            };
            let store = QdrantVectorStore::new(
                endpoint,
                vector_config.collection.clone(),
                vector_config.vector_dimensions as usize,
            )?
            .with_rebuild_marker(
                project_root
                    .join(".indexes")
                    .join("qdrant-rebuild-required.json"),
            )?;
            if let Err(error) = store.initialize() {
                if let Some(supervisor) = &sidecar {
                    let _ = supervisor.stop();
                }
                return Err(error);
            }
            (Some(Arc::new(store) as Arc<dyn VectorStore>), sidecar)
        } else {
            (None, None)
        };

        if vector.is_some() != embedder.is_some() {
            if let Some(supervisor) = &sidecar {
                let _ = supervisor.stop();
            }
            return Err(CoreError::validation(
                "vector store and embedding provider must be configured together",
            ));
        }

        let vector_signature = match (&vector, &embedder) {
            (Some(_), Some(embedder)) => Some(vector_index_signature(config, embedder.as_ref())?),
            (None, None) => None,
            _ => unreachable!("partial vector configuration rejected above"),
        };
        let knowledge_index = KnowledgeIndexSynchronizer::new(&project_root)?;

        Ok(Self {
            project_root: project_root.clone(),
            config: config.clone(),
            outbox: IndexInvalidationOutbox::new(
                project_root.join(".runtime").join("index_invalidation.db"),
            ),
            tantivy_path,
            sqlite_path,
            tantivy,
            sqlite,
            vector,
            embedder,
            reranker,
            knowledge_index,
            vector_signature,
            sidecar,
            chunk_size_chars: config.rag.chunk_size_chars as usize,
            chunk_overlap_chars: config.rag.chunk_overlap_chars as usize,
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn vector_enabled(&self) -> bool {
        self.vector.is_some()
    }

    pub fn config(&self) -> &ProjectConfig {
        &self.config
    }

    pub fn uses_vector_config(&self, config: &VectorStoreConfig) -> bool {
        &self.config.rag.vector_store == config
    }

    /// 判断候选配置是否仍使用同一向量空间与后端；embedding provider/model 也在边界内。
    pub fn uses_vector_pipeline_config(&self, config: &ProjectConfig) -> bool {
        vector_pipeline_config_matches(&self.config, config)
    }

    /// chunk、全文目录或任一向量空间变化都要求完整重建，禁止新查询打旧索引。
    pub fn index_configuration_changed(
        previous: &ProjectConfig,
        candidate: &ProjectConfig,
    ) -> bool {
        previous.rag.chunk_size_chars != candidate.rag.chunk_size_chars
            || previous.rag.chunk_overlap_chars != candidate.rag.chunk_overlap_chars
            || previous.rag.full_text_store != candidate.rag.full_text_store
            || !vector_pipeline_config_matches(previous, candidate)
    }

    pub fn index_configuration_revision(config: &ProjectConfig) -> CoreResult<String> {
        let bytes = serde_json::to_vec(&(
            &config.rag.chunk_size_chars,
            &config.rag.chunk_overlap_chars,
            &config.rag.full_text_store,
            vector_pipeline_descriptor(config),
        ))?;
        Ok(crate::contracts::content_version_for_bytes(&bytes))
    }

    /// 配置 generation 提交后幂等入队完整重建；pending 事件本身就是搜索门禁。
    pub fn enqueue_configuration_rebuild(&self) -> CoreResult<Option<String>> {
        if self.outbox.has_incomplete_full_rebuild()? {
            return Ok(None);
        }
        let revision = Self::index_configuration_revision(&self.config)?;
        let document_id = self.project_root.to_string_lossy().into_owned();
        let event_id = self.outbox.prepare(
            &document_id,
            "retrieval_configuration_changed",
            &revision,
            true,
        )?;
        self.outbox.activate(&event_id)?;
        Ok(Some(event_id))
    }

    /// 创建与本运行时共享后端和 provider 的 outbox worker。
    pub fn indexing_worker(&self) -> CoreResult<IndexingWorker> {
        match (&self.vector, &self.embedder) {
            (Some(vector), Some(embedder)) => IndexingWorker::with_vector_store(
                self.outbox.clone(),
                Arc::clone(&self.tantivy),
                Arc::clone(&self.sqlite),
                Arc::clone(vector),
                Arc::clone(embedder),
                self.chunk_size_chars,
                self.chunk_overlap_chars,
            ),
            (None, None) => IndexingWorker::new(
                self.outbox.clone(),
                Arc::clone(&self.tantivy),
                Arc::clone(&self.sqlite),
                self.chunk_size_chars,
                self.chunk_overlap_chars,
            ),
            _ => Err(CoreError::validation(
                "retrieval runtime has a partial vector configuration",
            )),
        }
    }

    /// 同步排空 outbox；任一失败保持事件可重试并向调用方 fail-loud。
    pub fn process_outbox(&self) -> CoreResult<usize> {
        let worker = self.indexing_worker()?;
        let mut processed = 0usize;
        loop {
            match worker.process_next()? {
                Some(_) => processed = processed.saturating_add(1),
                None => return Ok(processed),
            }
        }
    }

    /// 将 metadata.db 的四层已确认知识同步到正式全文/向量索引。
    pub fn sync_knowledge_index(
        &self,
        cancellation: Option<&crate::contracts::ExecutionCancellation>,
    ) -> CoreResult<KnowledgeIndexSyncReport> {
        self.knowledge_index.sync(
            &self.tantivy,
            &self.sqlite,
            self.vector.as_ref(),
            self.embedder.as_ref(),
            self.vector_signature.as_deref(),
            cancellation,
        )
    }

    /// 产品搜索入口：一次授权后生成查询向量、三路召回、磁盘新鲜度过滤和可选 rerank。
    pub fn search(
        &self,
        query: String,
        limit: usize,
        context: ProviderCallContext,
    ) -> CoreResult<Vec<RetrievalResult>> {
        if query.trim().is_empty() {
            return Err(CoreError::validation("retrieval query cannot be empty"));
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        validate_product_search_limit(limit)?;
        ensure_search_not_blocked_by_pending_index(&self.outbox)?;
        if context.cancellation.is_cancelled() {
            return Err(CoreError::external_cancelled(
                "project_retrieval",
                ExternalDispatchOutcome::NotDispatched,
            ));
        }
        let knowledge = self.sync_knowledge_index(Some(&context.cancellation))?;
        if context.cancellation.is_cancelled() {
            return Err(CoreError::external_cancelled(
                "project_retrieval",
                ExternalDispatchOutcome::NotDispatched,
            ));
        }
        // 工作流控制/lease 只在产品检索边界消费一次；子 provider 调用继承身份和取消，
        // 但使用独立默认派发授权，避免同一 operation 栅栏被重复 CAS。
        context.dispatch_authorization.authorize_dispatch()?;
        let operation_base = context
            .operation_id
            .clone()
            .unwrap_or_else(new_retrieval_operation_id);
        let query_embedding = if let Some(embedder) = &self.embedder {
            let child = child_provider_context(
                &context,
                embedder.provider_id(),
                format!("{operation_base}:query-embedding"),
            );
            let mut vectors = embedder.embed(child, vec![query.clone()])?;
            Some(vectors.pop().ok_or_else(|| {
                CoreError::validation("embedding provider returned no query vector")
            })?)
        } else {
            None
        };

        let candidate_limit = if self.reranker.is_some() {
            limit
                .checked_mul(3)
                .unwrap_or(MAX_HYBRID_SEARCH_LIMIT)
                .min(MAX_HYBRID_SEARCH_LIMIT)
        } else {
            limit
        };
        let retrieval = match &self.vector {
            Some(vector) => ThreeWayHybridSearchEngine::new(
                Arc::clone(vector),
                Arc::clone(&self.tantivy),
                Arc::clone(&self.sqlite),
            ),
            None => ThreeWayHybridSearchEngine::without_vector(
                Arc::clone(&self.tantivy),
                Arc::clone(&self.sqlite),
            ),
        };
        let mut results = retrieval.search(HybridSearchRequest::new(
            query.clone(),
            query_embedding,
            candidate_limit,
        ))?;
        results = filter_fresh_retrieval_results_with_knowledge_revision(
            results,
            Some(&knowledge.revision),
        )?;

        if let Some(reranker) = &self.reranker {
            if !results.is_empty() {
                let child = child_provider_context(
                    &context,
                    &reranker.provider_id,
                    format!("{operation_base}:rerank"),
                );
                let response = ProviderExecutor::new(reranker.ledger.as_ref()).rerank(
                    reranker.provider.as_ref(),
                    &child,
                    RerankRequest {
                        model_id: reranker.model_id.clone(),
                        query,
                        documents: results
                            .iter()
                            .map(|result| result.snippet.clone())
                            .collect(),
                        top_n: Some(limit.min(results.len())),
                        metadata: Value::Null,
                    },
                )?;
                results = apply_rerank_results(&results, response.results, limit)?;
            }
        }
        validate_product_search_result_budget(&results)?;
        Ok(results)
    }

    /// 诊断复用真实运行时组件；未配置向量时只报告两路全文组件。
    pub fn health_check(&self) -> CoreResult<Vec<StoreHealth>> {
        let mut health = Vec::new();
        if let Some(sidecar) = &self.sidecar {
            health.push(sidecar.health_check()?);
        }
        if let Some(vector) = &self.vector {
            health.push(vector.health_check()?);
        }
        if let Some(embedder) = &self.embedder {
            health.push(embedder.health_check()?);
        }
        if let Some(reranker) = &self.reranker {
            health.push(provider_health(
                "reranker_provider",
                &reranker.provider_id,
                reranker.provider.health_check()?,
            ));
        }
        health.push(
            self.knowledge_index
                .health_check(self.vector_signature.as_deref())?,
        );
        health.push(self.tantivy.health_check()?);
        health.push(self.sqlite.health_check()?);
        Ok(health)
    }

    pub fn shutdown(&self) -> CoreResult<()> {
        if let Some(sidecar) = &self.sidecar {
            if Arc::strong_count(sidecar) == 1 {
                sidecar.stop()?;
            }
        }
        Ok(())
    }
}

fn select_capability_provider(
    providers: &crate::config::ProvidersConfig,
    default_provider_id: Option<&str>,
    capability: ProviderCapability,
    label: &str,
) -> CoreResult<(ProviderConfig, String)> {
    let provider_id = default_provider_id.ok_or_else(|| {
        CoreError::validation(format!(
            "{label} is enabled but default_{label}_provider_id is not configured"
        ))
    })?;
    let provider = providers
        .providers
        .iter()
        .find(|provider| provider.provider_id == provider_id)
        .filter(|provider| provider.enabled)
        .cloned()
        .ok_or_else(|| {
            CoreError::validation(format!(
                "default {label} provider is missing or disabled: {provider_id}"
            ))
        })?;
    if provider.api_key.is_some() {
        return Err(CoreError::validation(format!(
            "provider '{}' contains an untrusted project SecretRef; re-enter the credential before {label} use",
            provider.provider_id
        )));
    }
    let model_id = provider
        .models
        .iter()
        .find(|model| model.capability == capability)
        .map(|model| model.model_id.clone())
        .ok_or_else(|| {
            CoreError::validation(format!(
                "provider '{}' has no {label} model configured",
                provider.provider_id
            ))
        })?;
    Ok((provider, model_id))
}

fn resolve_provider_secret(
    credentials: &ProjectCredentialScope<'_>,
    provider: &ProviderConfig,
    require_hosted_secret: bool,
) -> CoreResult<Option<String>> {
    let secret = credentials
        .get_provider_secret(&provider.provider_id)?
        .map(|value| value.expose_secret().to_owned());
    if require_hosted_secret
        && matches!(
            provider.provider_type,
            crate::contracts::ProviderType::OpenAi | crate::contracts::ProviderType::Gemini
        )
        && secret
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(CoreError::validation(format!(
            "provider '{}' requires a project-scoped credential",
            provider.provider_id
        )));
    }
    Ok(secret)
}

fn child_provider_context(
    parent: &ProviderCallContext,
    provider_id: &str,
    operation_id: String,
) -> ProviderCallContext {
    ProviderCallContext {
        provider_id: provider_id.to_owned(),
        operation_id: Some(operation_id),
        workflow_id: parent.workflow_id.clone(),
        run_id: parent.run_id.clone(),
        node_id: parent.node_id.clone(),
        tool_call_id: parent.tool_call_id.clone(),
        timeout_ms: parent.timeout_ms,
        max_retries: parent.max_retries,
        metadata: parent.metadata.clone(),
        cancellation: parent.cancellation.clone(),
        dispatch_authorization: Default::default(),
    }
}

fn provider_health(component: &str, provider_id: &str, health: ProviderHealth) -> StoreHealth {
    match health {
        ProviderHealth::Healthy => StoreHealth::degraded(
            component,
            format!("provider {provider_id} is configured; remote endpoint is verified on calls"),
        ),
        ProviderHealth::Degraded { reason } => StoreHealth::degraded(component, reason),
        ProviderHealth::Unhealthy { reason } => StoreHealth::unavailable(component, reason),
    }
}

fn vector_index_signature(
    config: &ProjectConfig,
    embedder: &dyn TextEmbedder,
) -> CoreResult<String> {
    let bytes = serde_json::to_vec(&(
        &config.rag.vector_store,
        embedder.provider_id(),
        embedder.model_id(),
        embedder.dimensions(),
    ))?;
    Ok(crate::contracts::content_version_for_bytes(&bytes))
}

fn vector_pipeline_config_matches(left: &ProjectConfig, right: &ProjectConfig) -> bool {
    vector_pipeline_descriptor(left) == vector_pipeline_descriptor(right)
}

fn vector_pipeline_descriptor(
    config: &ProjectConfig,
) -> Option<(&VectorStoreConfig, Option<&str>, Option<&ProviderConfig>)> {
    if !config.rag.vector_store.enabled {
        return None;
    }
    let provider_id = config.providers.default_embedding_provider_id.as_deref();
    let provider = provider_id.and_then(|provider_id| {
        config
            .providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == provider_id)
    });
    Some((&config.rag.vector_store, provider_id, provider))
}

fn resolve_project_path(project_root: &Path, configured: &Path) -> CoreResult<PathBuf> {
    if configured
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CoreError::validation(
            "retrieval path cannot contain parent traversal",
        ));
    }
    let path = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        project_root.join(configured)
    };
    ensure_path_under_root(project_root, &path)?;
    Ok(path)
}

fn new_retrieval_operation_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let timestamp_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "project-retrieval-{}-{timestamp_ns}-{sequence}",
        std::process::id()
    )
}
