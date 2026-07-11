use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::contracts::{
    ArtifactKind, CoreError, CoreResult, DocumentPatch, NodeId, PortMap, PortValue,
};
use crate::costs::CostLedger;
use crate::documents::{
    ArtifactWriteRequest, DocumentReadRequest, DocumentRepository, FileDocumentService,
    PatchApplyReport, PatchCheckpointRequest,
};
use crate::git::GitService;
use crate::providers::{
    LlmProvider, LlmRequest, LlmResponse, ProviderCallContext, ProviderExecutor, SearchProvider,
    SearchProviderRequest,
};
use crate::retrieval::{HybridSearch, HybridSearchRequest};
use crate::skills::{SkillExecutor, SkillManifest, SkillRunRequest};
use crate::workflow::{
    PatchWriteBackState, RuntimeReferenceResolver, WorkflowExportRequest, WorkflowExportSink,
    WorkflowExternalNodeExecutor, WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest,
    WorkflowRuntime,
};

/// 工作流外部节点处理函数签名。
pub type ExternalNodeHandler =
    Box<dyn FnMut(WorkflowNodeExecutionRequest) -> CoreResult<WorkflowNodeExecutionOutput>>;

/// 简单外部节点路由器，用于把具体节点类型挂到 Module 11 runtime。
pub struct RoutedExternalNodeExecutor {
    handlers: BTreeMap<String, ExternalNodeHandler>,
}

impl RoutedExternalNodeExecutor {
    /// 创建空外部节点路由器。
    pub fn new() -> Self {
        Self {
            handlers: BTreeMap::new(),
        }
    }

    /// 注册一个节点类型处理器。
    pub fn register_handler(
        &mut self,
        type_name: impl Into<String>,
        handler: ExternalNodeHandler,
    ) -> CoreResult<()> {
        let type_name = type_name.into();
        if type_name.trim().is_empty() {
            return Err(CoreError::validation(
                "workflow node handler type_name cannot be empty",
            ));
        }
        // 重复注册必须在插入前拦截。否则即使返回 Err，也会把原 handler
        // 替换掉，导致外部节点路由表进入半失败状态。
        if self.handlers.contains_key(&type_name) {
            return Err(CoreError::validation(format!(
                "duplicate workflow external handler: {type_name}"
            )));
        }
        self.handlers.insert(type_name, handler);
        Ok(())
    }
}

impl Default for RoutedExternalNodeExecutor {
    /// 创建默认外部节点路由器。
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowExternalNodeExecutor for RoutedExternalNodeExecutor {
    /// 按节点 type_name 分发到注册处理器。
    fn execute_external(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput> {
        let type_name = request.type_name.clone();
        let handler = self.handlers.get_mut(&type_name).ok_or_else(|| {
            CoreError::validation(format!("workflow external handler not found: {type_name}"))
        })?;
        handler(request)
    }
}

/// 基于文件系统和 Git 的引用解析器。
pub struct FilesystemRuntimeReferenceResolver {
    artifact_root: PathBuf,
    checkpoint_ids: BTreeSet<String>,
    patch_commit_ids: BTreeSet<String>,
}

impl FilesystemRuntimeReferenceResolver {
    /// 创建文件系统引用解析器。
    pub fn new(artifact_root: impl Into<PathBuf>) -> Self {
        Self {
            artifact_root: artifact_root.into(),
            checkpoint_ids: BTreeSet::new(),
            patch_commit_ids: BTreeSet::new(),
        }
    }

    /// 记录一个可解析的 checkpoint id。
    pub fn with_checkpoint(mut self, checkpoint_id: impl Into<String>) -> Self {
        self.checkpoint_ids.insert(checkpoint_id.into());
        self
    }

    /// 记录一个可解析的 patch session commit id。
    pub fn with_patch_commit(mut self, commit_id: impl Into<String>) -> Self {
        self.patch_commit_ids.insert(commit_id.into());
        self
    }

    /// 判断文件路径是否存在。
    fn path_exists(path: &str) -> bool {
        Path::new(path).exists()
    }
}

impl RuntimeReferenceResolver for FilesystemRuntimeReferenceResolver {
    /// document_id 当前由 Documents 模块使用规范化路径生成，因此按路径存在性检查。
    fn document_exists(&self, document_id: &str) -> CoreResult<bool> {
        Ok(Self::path_exists(document_id))
    }

    /// chunk 引用属于可重建索引内容；当前没有统一 chunk store 时按保守缺失处理。
    fn chunk_exists(&self, _chunk_id: &str) -> CoreResult<bool> {
        Ok(false)
    }

    /// artifact_id 按 artifact_root 下相对路径检查。
    fn artifact_exists(&self, artifact_id: &str) -> CoreResult<bool> {
        Ok(self.artifact_root.join(artifact_id).exists())
    }

    /// patch commit id 由运行时显式登记，避免猜测文件名。
    fn patch_session_commit_exists(&self, patch_session_commit_id: &str) -> CoreResult<bool> {
        Ok(self.patch_commit_ids.contains(patch_session_commit_id))
    }

    /// checkpoint id 由运行时显式登记，避免直接 shell 查询 Git。
    fn checkpoint_exists(&self, checkpoint_id: &str) -> CoreResult<bool> {
        Ok(self.checkpoint_ids.contains(checkpoint_id))
    }
}

/// 基于 Documents 模块的 Export sink。
pub struct DocumentWorkflowExportSink<'a> {
    documents: &'a FileDocumentService,
}

impl<'a> DocumentWorkflowExportSink<'a> {
    /// 创建 Documents Export sink。
    pub fn new(documents: &'a FileDocumentService) -> Self {
        Self { documents }
    }
}

impl WorkflowExportSink for DocumentWorkflowExportSink<'_> {
    /// 将 Export 节点输入序列化为 artifact。
    fn export_artifact(
        &mut self,
        request: &WorkflowNodeExecutionRequest,
        export: WorkflowExportRequest,
    ) -> CoreResult<String> {
        let payload = json!({
            "workflow_id": request.workflow_id,
            "run_id": request.run_id,
            "node_id": request.node_id,
            "format": export.format,
            "title": export.title,
            "inputs": export.inputs,
        });
        let bytes = serde_json::to_vec_pretty(&payload)?;
        let report = self.documents.write_artifact(ArtifactWriteRequest {
            artifact_id: export.artifact_id.clone(),
            kind: ArtifactKind::Export,
            media_type: export_media_type(&export.format).to_owned(),
            bytes,
            metadata: json!({
                "workflow_id": request.workflow_id,
                "run_id": request.run_id,
                "node_id": request.node_id,
            }),
        })?;
        Ok(report.descriptor.artifact_id)
    }
}

/// patch 写回执行结果。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowPatchApplyOutcome {
    pub report: PatchApplyReport,
    pub checkpoint_id: Option<String>,
}

/// 执行确认后的 patch 写回，并同步 runtime 写回状态。
pub fn apply_confirmed_patch(
    runtime: &mut WorkflowRuntime,
    documents: &FileDocumentService,
    git: Option<&GitService>,
    node_id: &NodeId,
    patch: &DocumentPatch,
    checkpoint_message: Option<&str>,
) -> CoreResult<WorkflowPatchApplyOutcome> {
    // 写回分成两步：先在 runtime 上做只读校验，再调用 DocumentService
    // 修改文件。只有真实文件写入和 checkpoint 都成功后，才把运行态置为
    // Applied，避免 I/O 失败时留下“已写回”的错误快照。
    runtime.ensure_patch_write_back_can_start(node_id)?;
    let report = documents.apply_patch(
        patch,
        git,
        Some(&PatchCheckpointRequest {
            node_id: node_id.as_str().to_owned(),
            message: checkpoint_message.map(str::to_owned),
        }),
    )?;
    runtime.mark_patch_write_back_state(node_id, PatchWriteBackState::Applied)?;
    let checkpoint_id = report
        .checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.checkpoint_id.clone());
    if let Some(node) = runtime.state.nodes.get_mut(node_id) {
        node.checkpoint_id = checkpoint_id.clone();
    }
    Ok(WorkflowPatchApplyOutcome {
        report,
        checkpoint_id,
    })
}

/// LLM 节点配置。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct WorkflowLlmNodeConfig {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// 执行单次 LLM 节点。
pub fn execute_llm_node<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_llm_node_with_defaults(request, provider, ledger, None, None)
}

/// 使用项目默认 provider/model 执行 UI 创建的 LLM/写作节点。
pub fn execute_llm_node_with_defaults<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    default_provider_id: Option<&str>,
    default_model_id: Option<&str>,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<WorkflowLlmNodeConfig>(request.config.clone())?;
    let provider_id = config
        .provider_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(default_provider_id)
        .ok_or_else(|| CoreError::validation("LLM node provider_id is not configured"))?;
    let model_id = config
        .model_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(default_model_id)
        .ok_or_else(|| CoreError::validation("LLM node model_id is not configured"))?;
    let input_prompt = resolve_llm_input_prompt(&request.inputs, config.prompt_alias.as_deref())?;
    let prompt_template = config
        .prompt_template
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let prompt = match (prompt_template, input_prompt) {
        (Some(template), Some(input)) => format!("{template}\n\n{input}"),
        (Some(template), None) => template.to_owned(),
        (None, Some(input)) => input,
        (None, None) => {
            return Err(CoreError::validation(
                "LLM node requires prompt_template or a text input",
            ))
        }
    };
    let mut messages = Vec::new();
    if let Some(system_prompt) = &config.system_prompt {
        messages.push(crate::providers::LlmMessage {
            role: crate::providers::LlmRole::System,
            content: vec![crate::providers::ContentPart::text(system_prompt.clone())],
            name: None,
            tool_call_id: None,
        });
    }
    messages.push(crate::providers::LlmMessage::user(prompt));

    let executor = ProviderExecutor::new(ledger);
    let response = executor.complete_llm(
        provider,
        &ProviderCallContext {
            provider_id: provider_id.to_owned(),
            workflow_id: Some(request.workflow_id.clone()),
            run_id: Some(request.run_id.clone()),
            node_id: Some(request.node_id.clone()),
            tool_call_id: None,
            timeout_ms: 120_000,
            max_retries: 0,
            metadata: request.metadata.clone(),
        },
        LlmRequest {
            model_id: model_id.to_owned(),
            messages,
            tools: Vec::new(),
            temperature: None,
            max_output_tokens: None,
            stream: false,
            metadata: request.metadata,
        },
    )?;
    llm_response_to_output(response)
}

fn resolve_llm_input_prompt(
    inputs: &PortMap,
    prompt_alias: Option<&str>,
) -> CoreResult<Option<String>> {
    if let Some(alias) = prompt_alias
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
    {
        return input_text(inputs, alias).map(Some);
    }
    for alias in ["prompt", "input", "text"] {
        if inputs.contains_key(alias) {
            return input_text(inputs, alias).map(Some);
        }
    }
    if inputs.len() == 1 {
        if let Some(alias) = inputs.keys().next() {
            return input_text(inputs, alias).map(Some);
        }
    }
    Ok(None)
}

/// Summarizer 节点配置：四步总结生产链的接线参数。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct WorkflowSummarizerNodeConfig {
    pub provider_id: String,
    pub model_id: String,
    /// 章节 id，作为故事段/事件/总结的归属键。
    pub chapter_id: String,
    /// 章节正文所属文档 id，给故事段构造 source span（不复制正文）。
    pub chapter_document_id: String,
    /// 从哪个输入 alias 取章节正文。
    #[serde(default = "default_chapter_text_alias")]
    pub chapter_text_alias: String,
    /// 是否走 Auto Mode 的确认策略（自动审计）。
    #[serde(default)]
    pub auto_mode: bool,
}

fn default_chapter_text_alias() -> String {
    "chapter_text".to_owned()
}

/// 执行 Summarizer 节点：加载写作知识库 → 四步总结 → 落库建索引 → 生成四层确认项。
/// 这是「故事段划分并概括」等总结机制的真实生产入口，取代把 summarizer 降级为普通 LLM 节点。
pub fn execute_summarizer_node<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    project_root: &Path,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    use crate::contracts::{AutoModeState, RunControl};
    use crate::rag::models::{ConfirmationState, WritingConfirmationPolicy};
    use crate::rag::pipeline::SummaryPipelineExecutor;
    use crate::rag::store::SqliteWritingKnowledgeStore;
    use crate::rag::summarizer::{SummarizerConfig, SummarizerExecutor};
    use crate::workflow::{RuntimeConfirmation, RuntimeConfirmationState};

    let config = serde_json::from_value::<WorkflowSummarizerNodeConfig>(request.config.clone())?;
    let chapter_text = input_text(&request.inputs, &config.chapter_text_alias)?;
    let prompts = crate::rag::resources::load_prompt_resources()?;

    // 加载持久化知识库（跨章、跨运行存活）；首次运行则得到空库。
    let store = SqliteWritingKnowledgeStore::open(project_root)?;
    // 只有迁移后的空数据库才代表新项目；损坏、权限或 JSON 错误必须阻断总结，
    // 不能静默创建空知识库后覆盖已有作品事实。
    let knowledge = store.load_knowledge()?;

    // 四步总结 → 组装 draft。
    let summarizer = SummarizerExecutor::new(
        provider,
        ledger,
        &prompts,
        SummarizerConfig {
            provider_id: config.provider_id.clone(),
            model_id: config.model_id.clone(),
            chapter_document_id: config.chapter_document_id.clone(),
            run_id: Some(request.run_id.as_str().to_owned()),
            timeout_ms: 120_000,
        },
    );
    let draft = summarizer.summarize_chapter(&config.chapter_id, &chapter_text)?;

    // 落库建索引 + 生成四层确认项。
    let (policy, auto_mode) = if config.auto_mode {
        (
            WritingConfirmationPolicy::auto_audit_default(),
            AutoModeState {
                enabled: true,
                preauthorized_budget_usd: None,
            },
        )
    } else {
        (
            WritingConfirmationPolicy::normal_default(),
            AutoModeState::default(),
        )
    };
    let pipeline = SummaryPipelineExecutor::new(&knowledge, policy, auto_mode);
    let report = pipeline.apply_draft(draft)?;

    // 持久化更新后的知识库。
    store.save_knowledge(&knowledge)?;

    // 把知识库确认项映射成 runtime 确认项，使工作流按需暂停。
    let mut confirmations = Vec::new();
    for item in knowledge.confirmations(None)? {
        if !report.confirmation_ids.contains(&item.confirmation_id) {
            continue;
        }
        let state = match item.state {
            ConfirmationState::Pending => RuntimeConfirmationState::Pending,
            ConfirmationState::Approved => RuntimeConfirmationState::Approved,
            ConfirmationState::Rejected => RuntimeConfirmationState::Rejected,
            ConfirmationState::Skipped | ConfirmationState::AutoAudited => {
                RuntimeConfirmationState::AutoAudited
            }
        };
        confirmations.push(RuntimeConfirmation {
            confirmation_id: item.confirmation_id,
            node_id: request.node_id.clone(),
            state,
            artifact_id: None,
            patch_session_commit_id: None,
            metadata: item.metadata,
        });
    }

    let has_pending = confirmations
        .iter()
        .any(|c| c.state == RuntimeConfirmationState::Pending);

    let mut outputs = PortMap::new();
    outputs.insert(
        "confirmation_ids".to_owned(),
        PortValue::inline(json!(report.confirmation_ids)),
    );
    outputs.insert(
        "completed_steps".to_owned(),
        PortValue::inline(json!(report.completed_steps)),
    );
    outputs.insert(
        "chapter_id".to_owned(),
        PortValue::inline(json!(report.chapter_id)),
    );
    outputs.insert("paused".to_owned(), PortValue::inline(json!(report.paused)));

    Ok(WorkflowNodeExecutionOutput {
        outputs,
        run_control: if has_pending || report.paused {
            Some(RunControl::Pause)
        } else {
            None
        },
        confirmations,
        metadata: json!({
            "planner_issue_ids": report.planner_issue_ids,
            "pause_reason": report.pause_reason,
        }),
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// Search 节点配置。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct WorkflowSearchNodeConfig {
    pub provider_id: String,
    pub query_alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// 执行一次 SearchProvider 节点。
pub fn execute_search_node<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn SearchProvider,
    ledger: &L,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<WorkflowSearchNodeConfig>(request.config.clone())?;
    let query = input_text(&request.inputs, &config.query_alias)?;
    let executor = ProviderExecutor::new(ledger);
    let response = executor.search(
        provider,
        &ProviderCallContext {
            provider_id: config.provider_id,
            workflow_id: Some(request.workflow_id.clone()),
            run_id: Some(request.run_id.clone()),
            node_id: Some(request.node_id.clone()),
            tool_call_id: None,
            timeout_ms: 60_000,
            max_retries: 0,
            metadata: request.metadata.clone(),
        },
        SearchProviderRequest {
            query,
            limit: config.limit,
            metadata: request.metadata,
        },
    )?;
    let mut outputs = PortMap::new();
    outputs.insert(
        "results".to_owned(),
        PortValue::inline(json!(response.results)),
    );
    outputs.insert("raw".to_owned(), PortValue::inline(response.raw.clone()));
    Ok(WorkflowNodeExecutionOutput {
        outputs,
        metadata: json!({ "cost_usd": response.cost_usd }),
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// 项目内 RAG 搜索节点配置；与外部 Web SearchProvider 明确分离。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct WorkflowProjectSearchNodeConfig {
    pub query_alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// 使用项目混合索引执行 search 节点。
pub fn execute_project_search_node(
    request: WorkflowNodeExecutionRequest,
    retrieval: &dyn HybridSearch,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<WorkflowProjectSearchNodeConfig>(request.config)?;
    let query = input_text(&request.inputs, &config.query_alias)?;
    let limit = config.limit.unwrap_or(10);
    let results = retrieval.search(HybridSearchRequest::new(query, None, limit))?;
    let mut outputs = PortMap::new();
    outputs.insert("results".to_owned(), PortValue::inline(json!(results)));
    Ok(WorkflowNodeExecutionOutput {
        outputs,
        metadata: json!({
            "retrieval_scope": "project",
            "result_count": results.len(),
        }),
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// Document read 节点配置。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct WorkflowDocumentReadConfig {
    pub path: PathBuf,
    #[serde(default)]
    pub include_content: bool,
}

/// 执行文档读取节点。
pub fn execute_document_read_node(
    request: WorkflowNodeExecutionRequest,
    documents: &FileDocumentService,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_document_read_node_with_root(request, documents, None)
}

/// 执行文档读取节点，并把相对路径锚定到指定工作目录。
pub fn execute_document_read_node_with_root(
    request: WorkflowNodeExecutionRequest,
    documents: &FileDocumentService,
    work_root: Option<&Path>,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<WorkflowDocumentReadConfig>(request.config)?;
    let path = match (config.path.is_absolute(), work_root) {
        (false, Some(root)) => root.join(&config.path),
        _ => config.path,
    };
    let content = documents.open_document(DocumentReadRequest { path, format: None })?;
    let mut outputs = PortMap::new();
    outputs.insert(
        "document".to_owned(),
        PortValue::document_ref(content.metadata.document_id.clone(), None),
    );
    outputs.insert(
        "metadata".to_owned(),
        PortValue::inline(json!(content.metadata)),
    );
    if config.include_content {
        outputs.insert("content".to_owned(), PortValue::inline(content.content));
    }
    Ok(WorkflowNodeExecutionOutput {
        outputs,
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// ExecutorAdapter 节点配置。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct WorkflowExecutorAdapterConfig {
    pub skill_id: String,
}

/// 执行 ExecutorAdapter 节点。
pub fn execute_executor_adapter_node<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    manifest: &SkillManifest,
    executor: &SkillExecutor<'_, L>,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<WorkflowExecutorAdapterConfig>(request.config.clone())?;
    if manifest.skill_id != config.skill_id {
        return Err(CoreError::validation(format!(
            "executor adapter config skill_id {} does not match manifest {}",
            config.skill_id, manifest.skill_id
        )));
    }
    let output = executor.execute(
        manifest,
        SkillRunRequest {
            skill_id: config.skill_id,
            inputs: request.inputs,
            metadata: request.metadata,
        },
    )?;
    Ok(WorkflowNodeExecutionOutput {
        outputs: output.outputs,
        metadata: output.metadata,
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// 把 LLM response 转成标准节点输出。
fn llm_response_to_output(response: LlmResponse) -> CoreResult<WorkflowNodeExecutionOutput> {
    let mut outputs = PortMap::new();
    outputs.insert(
        "message".to_owned(),
        PortValue::inline(json!(response.message)),
    );
    outputs.insert(
        "text".to_owned(),
        PortValue::inline(llm_response_text(&response)),
    );
    outputs.insert(
        "tool_calls".to_owned(),
        PortValue::inline(json!(response.tool_calls)),
    );
    Ok(WorkflowNodeExecutionOutput {
        outputs,
        metadata: json!({
            "usage": response.usage,
            "finish_reason": response.finish_reason,
            "cost_usd": response.cost_usd,
            "raw": response.raw,
        }),
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// 从 LLM response 提取合并文本。
fn llm_response_text(response: &LlmResponse) -> String {
    response
        .message
        .content
        .iter()
        .filter_map(|part| match part {
            crate::providers::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// 从端口输入中读取字符串。
fn input_text(inputs: &PortMap, alias: &str) -> CoreResult<String> {
    let value = inputs
        .get(alias)
        .ok_or_else(|| CoreError::validation(format!("input alias missing: {alias}")))?;
    match value {
        PortValue::Inline { value } => value
            .as_str()
            .map(str::to_owned)
            .or_else(|| Some(value.to_string()))
            .ok_or_else(|| CoreError::validation(format!("input alias {alias} is not text"))),
        _ => Err(CoreError::validation(format!(
            "input alias {alias} must be inline text"
        ))),
    }
}

/// 根据导出格式返回 media type。
fn export_media_type(format: &str) -> &'static str {
    match format {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "markdown" | "md" => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    }
}
