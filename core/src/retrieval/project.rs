use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;

use crate::config::{
    external_qdrant_endpoint, AppRuntimeSettingsStore, ConfigStore, ProjectConfig,
    ProjectCredentialScope, ProviderConfig, QdrantAuthMode, SecretStore, VectorStoreBackend,
    VectorStoreConfig,
};
use crate::contracts::{
    ensure_path_under_root, CoreError, CoreResult, ExternalDispatchOutcome, ProviderCapability,
    ProviderType,
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
    filter_fresh_retrieval_results_with_knowledge_revision, resolve_qdrant_binary_path,
    validate_product_search_limit, validate_product_search_result_budget, FullTextStore,
    HybridSearch, HybridSearchRequest, IndexingWorker, KnowledgeIndexSyncReport,
    KnowledgeIndexSynchronizer, QdrantSidecarConfig, QdrantSidecarSupervisor, QdrantVectorStore,
    RetrievalResult, SidecarState, SqliteFullTextStore, StoreHealth, TantivyFullTextStore,
    TextEmbedder, ThreeWayHybridSearchEngine, VectorStore, MAX_HYBRID_SEARCH_LIMIT,
};
use crate::retrieval::sidecar::{default_max_restarts_per_window, default_restart_window_ms};

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
    /// U109：`reranker_enabled` 为真但重排序装配失败时的原因。检索继续按无重排序运行，
    /// 该原因由 `health_check` 上报为 degraded，不静默丢弃。
    reranker_unavailable: Option<String>,
    knowledge_index: KnowledgeIndexSynchronizer,
    vector_signature: Option<String>,
    qdrant_credential_generation: Option<String>,
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

    /// 判断共享组合根是否由同一份项目配置构造。恢复旧工作流时若配置代次不同，
    /// 调度器必须 fail-loud，不能另开一个隐式检索组合根。
    pub fn matches_project_config(&self, config: &ProjectConfig) -> CoreResult<bool> {
        let mut expected = config.clone();
        let app_state_root = crate::config::trusted_app_state_for_project(&self.project_root);
        let runtime_settings = AppRuntimeSettingsStore::read_global_or_migrate(
            app_state_root,
            Some(&self.project_root),
        )?;
        runtime_settings.apply_to_sidecar(&mut expected.rag.vector_store.sidecar);
        expected.validate()?;
        Ok(self.config == expected)
    }

    /// 从候选配置构造新 generation；未变的索引/sidecar 与旧 generation 共享。
    pub fn from_config(
        project_root: &Path,
        secrets: &dyn SecretStore,
        config: &ProjectConfig,
        previous: Option<&Self>,
    ) -> CoreResult<Self> {
        let project_root = project_root.canonicalize()?;
        let mut config = config.clone();
        let app_state_root = crate::config::trusted_app_state_for_project(&project_root);
        let runtime_settings =
            AppRuntimeSettingsStore::read_global_or_migrate(app_state_root, Some(&project_root))?;
        runtime_settings.apply_to_sidecar(&mut config.rag.vector_store.sidecar);
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
        let qdrant_credential_generation = if config.rag.vector_store.enabled
            && config.rag.vector_store.backend == VectorStoreBackend::ExternalQdrant
            && config.rag.vector_store.sidecar.auth_mode == QdrantAuthMode::ApiKey
        {
            let endpoint = external_qdrant_endpoint(&config.rag.vector_store.sidecar)?;
            Some(credentials.external_qdrant_secret_generation(&endpoint)?)
        } else {
            None
        };

        let embedder = if config.rag.vector_store.enabled {
            let (provider_config, model_id) = select_capability_provider(
                &config.providers,
                config.providers.default_embedding_provider_id.as_deref(),
                config.providers.default_embedding_model_id.as_deref(),
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

        // U109：重排序只是检索结果的质量叠加层，不是检索能力本身。它的装配失败
        // 必须降级为「重排序不可用」，绝不能通过 `?` 把整个组合根（含全文与向量）
        // 一起击穿——否则一个质量开关会让全部检索停摆，且错误与「重排序」毫无表面关联。
        // 降级原因记入运行时状态，由 health_check 显式上报，不做静默吞掉。
        let mut reranker_unavailable = None;
        let reranker = if config.rag.reranker_enabled {
            match Self::build_reranker(&config, &credentials, &ledger) {
                Ok(reranker) => Some(reranker),
                Err(error) => {
                    reranker_unavailable = Some(error.to_string());
                    None
                }
            }
        } else {
            None
        };

        let (vector, sidecar) = if reusable.is_some_and(|runtime| {
            runtime.config.rag.vector_store == config.rag.vector_store
                && runtime.qdrant_credential_generation == qdrant_credential_generation
        }) {
            let runtime = reusable.expect("reusable runtime checked above");
            (runtime.vector.clone(), runtime.sidecar.clone())
        } else if config.rag.vector_store.enabled {
            let mut sidecar = None;
            let vector_config = &config.rag.vector_store;
            let (endpoint, qdrant_api_key) = match vector_config.backend {
                VectorStoreBackend::QdrantSidecar => {
                    let data_dir = resolve_project_path(
                        &project_root,
                        Path::new(&vector_config.sidecar.data_dir),
                    )?;
                    let log_dir = project_root.join(".runtime").join("logs").join("qdrant");
                    let binary_path =
                        resolve_qdrant_binary_path(Path::new(&vector_config.sidecar.binary_path))?;
                    let supervisor = Arc::new(QdrantSidecarSupervisor::new(QdrantSidecarConfig {
                        binary_path,
                        host: vector_config.sidecar.host.clone(),
                        requested_port: vector_config.sidecar.port,
                        data_dir,
                        log_dir,
                        startup_timeout_ms: vector_config.sidecar.startup_timeout_ms,
                        // 自动重启节流取默认（60s 窗口内最多 3 次）。不进 config schema：
                        // 这是防 fork 风暴的安全下限，不是需要用户调的旋钮，暴露出去
                        // 反而给了「设成 999 次」这种把自己打死的机会。
                        max_restarts_per_window: default_max_restarts_per_window(),
                        restart_window_ms: default_restart_window_ms(),
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
                    (endpoint, None)
                }
                VectorStoreBackend::ExternalQdrant => {
                    let endpoint = external_qdrant_endpoint(&vector_config.sidecar)?;
                    let api_key = match vector_config.sidecar.auth_mode {
                        QdrantAuthMode::None => None,
                        QdrantAuthMode::ApiKey => Some(
                            credentials
                                .get_external_qdrant_secret(&endpoint)?
                                .ok_or_else(|| {
                                    CoreError::validation(
                                        "external qdrant API key is not configured for this endpoint",
                                    )
                                })?,
                        ),
                    };
                    (endpoint, api_key)
                }
            };
            let store = QdrantVectorStore::new_with_api_key(
                endpoint,
                vector_config.collection.clone(),
                vector_config.vector_dimensions as usize,
                qdrant_api_key.as_ref().map(|value| value.expose_secret()),
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
            (Some(_), Some(embedder)) => Some(vector_index_signature(&config, embedder.as_ref())?),
            (None, None) => None,
            _ => unreachable!("partial vector configuration rejected above"),
        };
        let knowledge_index = KnowledgeIndexSynchronizer::new(&project_root)?;
        let chunk_size_chars = config.rag.chunk_size_chars as usize;
        let chunk_overlap_chars = config.rag.chunk_overlap_chars as usize;

        Ok(Self {
            project_root: project_root.clone(),
            config,
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
            reranker_unavailable,
            knowledge_index,
            vector_signature,
            qdrant_credential_generation,
            sidecar,
            chunk_size_chars,
            chunk_overlap_chars,
        })
    }

    /// U109：把重排序装配单独收成一个可失败的构造，让调用方能选择降级而不是击穿。
    fn build_reranker(
        config: &ProjectConfig,
        credentials: &ProjectCredentialScope<'_>,
        ledger: &Arc<dyn CostLedger>,
    ) -> CoreResult<ProjectReranker> {
        let (provider_config, model_id) = select_capability_provider(
            &config.providers,
            config.providers.default_reranker_provider_id.as_deref(),
            config.providers.default_reranker_model_id.as_deref(),
            ProviderCapability::Reranker,
            "reranker",
        )?;
        let api_key = resolve_provider_secret(credentials, &provider_config, false)?;
        let provider: Arc<dyn RerankerProvider> =
            Arc::new(HttpRerankerProvider::new(provider_config, api_key)?);
        let provider_id = provider.definition().provider_id;
        Ok(ProjectReranker {
            provider,
            ledger: Arc::clone(ledger),
            provider_id,
            model_id,
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// U109：重排序已启用但当前不可用时的原因；检索本身仍然可用。
    pub fn reranker_unavailable_reason(&self) -> Option<&str> {
        self.reranker_unavailable.as_deref()
    }

    pub fn vector_enabled(&self) -> bool {
        self.vector.is_some()
    }

    pub fn config(&self) -> &ProjectConfig {
        &self.config
    }

    /// 判断向量流水线身份是否变化：vector_store 之外，embedding provider/端点/模型
    /// 也在边界内——换了 embedding 就是换了向量语义空间，旧 embedder 不能再服务新索引。
    /// 与 `index_configuration_changed` 共用 `vector_pipeline_config_matches`，
    /// 避免「要重建索引」与「要重开 runtime」两处判据漂移（U116）。
    pub fn vector_pipeline_configuration_changed(
        previous: &ProjectConfig,
        candidate: &ProjectConfig,
    ) -> bool {
        !vector_pipeline_config_matches(previous, candidate)
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
        let revision = Self::index_configuration_revision(&self.config)?;
        let document_id = self.project_root.to_string_lossy().into_owned();
        self.outbox.enqueue_full_rebuild_if_absent(
            &document_id,
            "retrieval_configuration_changed",
            &revision,
        )
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
        self.process_outbox_with_cancellation(&crate::contracts::ExecutionCancellation::new())
    }

    pub fn process_outbox_with_cancellation(
        &self,
        cancellation: &crate::contracts::ExecutionCancellation,
    ) -> CoreResult<usize> {
        let worker = self.indexing_worker()?;
        let mut processed = 0usize;
        loop {
            cancellation.check()?;
            match worker.process_next_with_cancellation(cancellation)? {
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

    /// 向量检索前尝试恢复崩掉的 sidecar。抽成独立方法而非内联在 `search` 里，
    /// 是为了让「这条接线存在」本身可被断言：生产默认后端是 `QdrantSidecar`，
    /// 而集成测试只能用 `ExternalQdrant`（自托管分支要找真二进制、起不来直接 Err），
    /// 因此内联版本在测试环境里**不可达**——摘掉它没有一条测试会变红。
    ///
    /// 恢复放在这里而不是 `health_check` 里：
    /// - 诊断必须是只读观测，否则「看一眼设置页」就会重启后端服务；
    /// - 检索是 sidecar 真正被需要的时刻，此时恢复才有意义；
    /// - 放在 embedding 之前，避免「先付费嵌入、再发现向量库不可用」。
    ///
    /// 仅在启用向量检索时执行；纯全文检索不该为 sidecar 付探活成本。
    /// 探活自带节流（`RECOVERY_PROBE_MIN_INTERVAL_MS`），稳态下只读一个 Instant，
    /// 重启另有滑动窗口上限兜底，持久故障不会被高频检索放大成 fork 风暴。
    ///
    /// 返回值仅供测试断言恢复是否被真正触达，生产路径忽略它。
    pub(crate) fn recover_sidecar_before_vector_search(&self) -> bool {
        if self.vector.is_none() {
            return false;
        }
        let Some(sidecar) = &self.sidecar else {
            return false;
        };
        // 恢复失败不阻断检索：可能只是本次拉不起来，让后续真实请求去报错，
        // 比在这里把一次检索变成启动错误更贴近用户预期。
        let _ = sidecar.recover_if_unavailable();
        true
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
        // 知识同步在启用向量检索时会调用远端 embedding，并随后改写正式索引；
        // 因此父级 workflow/lease 授权必须在线性化点先消费，不能等同步完成后再补验。
        // 子 provider 调用继承身份和取消，但使用独立默认派发授权，避免重复 CAS。
        context.dispatch_authorization.authorize_dispatch()?;
        let knowledge = self.sync_knowledge_index(Some(&context.cancellation))?;
        if context.cancellation.is_cancelled() {
            return Err(CoreError::external_cancelled(
                "project_retrieval",
                ExternalDispatchOutcome::NotDispatched,
            ));
        }
        let operation_base = context
            .operation_id
            .clone()
            .unwrap_or_else(new_retrieval_operation_id);
        self.recover_sidecar_before_vector_search();

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
        let mut results = retrieval.search_with_cancellation(
            HybridSearchRequest::new(query.clone(), query_embedding, candidate_limit),
            &context.cancellation,
        )?;
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
        let dead_letters = self.outbox.list_dead_letters()?;
        if !dead_letters.is_empty() {
            health.push(StoreHealth::unavailable(
                "retrieval_index_outbox",
                "diagnostics.retrieval.outbox.dead_letter",
            ));
        } else if self.outbox.has_incomplete_invalidation()?
            || self.outbox.has_incomplete_full_rebuild()?
        {
            health.push(StoreHealth::degraded(
                "retrieval_index_outbox",
                "diagnostics.retrieval.outbox.pending",
            ));
        } else {
            health.push(StoreHealth::healthy("retrieval_index_outbox"));
        }
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
        } else if self.reranker_unavailable.is_some() {
            // U109：开关开着但装配失败——必须显式暴露为降级项，不能表现为「没配重排序」。
            // reason 保持纯 display_name key（前端 DiagnosticReasonLabel 只本地化 `diagnostics.` 前缀键），
            // 具体失败原因由 `reranker_unavailable_reason()` 提供给诊断详情。
            health.push(StoreHealth::degraded(
                "reranker_provider",
                "diagnostics.retrieval.reranker.unavailable",
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
    default_model_id: Option<&str>,
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
    let model_id = default_model_id
        .map(|model_id| {
            provider
                .models
                .iter()
                .find(|model| model.model_id == model_id && model.capability == capability)
                .ok_or_else(|| {
                    CoreError::validation(format!(
                        "default {label} model is missing or has the wrong capability: {provider_id}/{model_id}"
                    ))
                })
        })
        .transpose()?
        .or_else(|| provider.models.iter().find(|model| model.capability == capability))
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

#[derive(serde::Serialize, PartialEq)]
struct VectorPipelineDescriptor<'a> {
    vector_store: &'a VectorStoreConfig,
    provider_id: Option<&'a str>,
    provider_type: Option<&'a ProviderType>,
    provider_enabled: Option<bool>,
    base_url: Option<&'a str>,
    model_id: Option<&'a str>,
}

fn vector_pipeline_descriptor(config: &ProjectConfig) -> Option<VectorPipelineDescriptor<'_>> {
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
    let model_id = provider.and_then(|provider| {
        provider
            .models
            .iter()
            .find(|model| model.capability == ProviderCapability::Embedding)
            .map(|model| model.model_id.as_str())
    });
    Some(VectorPipelineDescriptor {
        vector_store: &config.rag.vector_store,
        provider_id,
        provider_type: provider.map(|provider| &provider.provider_type),
        provider_enabled: provider.map(|provider| provider.enabled),
        base_url: provider.and_then(|provider| provider.base_url.as_deref()),
        model_id,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::{MemoryFullTextStore, MemoryVectorStore};

    /// 直接构造组合根，绕开 `from_config`。
    ///
    /// 必须绕开：`from_config` 在 `QdrantSidecar` 后端下会去解析真实二进制并 `start()?`，
    /// 测试机上没有 Qdrant 就直接 `Err`；而唯一能装配成功的 `ExternalQdrant` 后端
    /// 根本不建 supervisor（`sidecar` 恒为 `None`）。两条路都到不了这条接线，
    /// 所以只能在同模块内按字段拼装。
    fn runtime_with_sidecar(
        root: &Path,
        vector: Option<Arc<dyn VectorStore>>,
        sidecar: Option<Arc<QdrantSidecarSupervisor>>,
    ) -> ProjectRetrievalRuntime {
        ProjectRetrievalRuntime {
            project_root: root.to_path_buf(),
            config: ProjectConfig::default(),
            outbox: IndexInvalidationOutbox::new(root.join("outbox.db")),
            tantivy_path: root.join(".indexes").join("tantivy"),
            sqlite_path: root.join(".indexes").join("full_text.db"),
            tantivy: Arc::new(MemoryFullTextStore::new()),
            sqlite: Arc::new(MemoryFullTextStore::new()),
            vector,
            embedder: None,
            reranker: None,
            reranker_unavailable: None,
            knowledge_index: KnowledgeIndexSynchronizer::new(root).unwrap(),
            vector_signature: None,
            qdrant_credential_generation: None,
            sidecar,
            chunk_size_chars: 512,
            chunk_overlap_chars: 64,
        }
    }

    /// 指向不存在的二进制：`spawn` 必然失败，正好用来验证恢复失败不会 panic、
    /// 也不会把失败冒泡成检索错误。
    fn unavailable_supervisor(root: &Path) -> Arc<QdrantSidecarSupervisor> {
        Arc::new(QdrantSidecarSupervisor::new(QdrantSidecarConfig {
            binary_path: root.join("no-such-qdrant-binary"),
            host: "127.0.0.1".to_owned(),
            requested_port: 0,
            data_dir: root.join("qdrant-data"),
            log_dir: root.join("qdrant-logs"),
            startup_timeout_ms: 50,
            max_restarts_per_window: default_max_restarts_per_window(),
            restart_window_ms: default_restart_window_ms(),
        }))
    }

    #[test]
    fn vector_search_path_attempts_sidecar_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let sidecar = unavailable_supervisor(&root);
        sidecar.mark_crashed("test induced crash").unwrap();
        let runtime = runtime_with_sidecar(
            &root,
            Some(Arc::new(MemoryVectorStore::new())),
            Some(Arc::clone(&sidecar)),
        );

        assert!(
            runtime.recover_sidecar_before_vector_search(),
            "启用向量检索且存在 supervisor 时必须尝试恢复；摘掉这条接线本用例应变红"
        );
    }

    #[test]
    fn full_text_only_search_does_not_touch_sidecar() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        // 纯全文检索不该为 sidecar 付探活成本：vector 为 None 时直接跳过。
        let runtime = runtime_with_sidecar(&root, None, Some(unavailable_supervisor(&root)));

        assert!(!runtime.recover_sidecar_before_vector_search());
    }

    #[test]
    fn external_qdrant_without_supervisor_is_not_an_error() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        // ExternalQdrant 后端没有本地进程可管，恢复应静默跳过而非报错。
        let runtime =
            runtime_with_sidecar(&root, Some(Arc::new(MemoryVectorStore::new())), None);

        assert!(!runtime.recover_sidecar_before_vector_search());
    }
}
