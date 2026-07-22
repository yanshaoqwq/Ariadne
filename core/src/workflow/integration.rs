use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::contracts::{
    ArtifactKind, CoreError, CoreResult, DocumentPatch, LoopPolicy, NodeId, PermissionPolicy,
    PortMap, PortValue, WorkflowDefinition, WorkflowEdgeKind,
};
use crate::costs::CostLedger;
use crate::documents::{
    ArtifactWriteRequest, DocumentReadRequest, DocumentRepository, FileDocumentService,
    PatchApplyReport, PatchCheckpointRequest,
};
use crate::frontend::service::{
    render_chapters_epub, render_chapters_markdown, render_chapters_pdf,
};
use crate::git::GitService;
use crate::llm::{tool_result_message, ToolExecutionContext, ToolExecutor, ToolExecutorRouter};
use crate::providers::{
    LlmProvider, LlmRequest, LlmResponse, ProviderCallContext, ProviderExecutor, SearchProvider,
    SearchProviderRequest, ToolDefinition, WebSearchToolExecutor,
};
use crate::retrieval::{
    validate_product_search_limit, validate_product_search_result_budget, HybridSearch,
    HybridSearchRequest, ProjectRetrievalRuntime, ProjectSearchToolExecutor,
};
use crate::skills::{SkillExecutor, SkillManifest, SkillRunRequest};
use crate::workflow::{
    ApprovalNodeConfig, ConditionNodeConfig, ExportNodeConfig, LoopNodeConfig, PatchWriteBackState,
    RuntimeReferenceResolver, WorkflowExportRequest, WorkflowExportSink,
    WorkflowExternalNodeExecutor, WorkflowNodeExecutionOutput, WorkflowNodeExecutionRequest,
    WorkflowRuntime,
};

/// 工作流外部节点处理函数签名。
pub type ExternalNodeHandler =
    Box<dyn FnMut(WorkflowNodeExecutionRequest) -> CoreResult<WorkflowNodeExecutionOutput>>;

pub type ExternalOperationReconciler = Box<
    dyn FnMut(&WorkflowNodeExecutionRequest) -> CoreResult<Option<WorkflowNodeExecutionOutput>>,
>;

#[derive(Default)]
pub struct WorkflowLlmSearchOptions<'a> {
    pub default_provider_id: Option<&'a str>,
    pub default_model_id: Option<&'a str>,
    pub project_search: Option<(&'a ProjectRetrievalRuntime, ToolDefinition)>,
    pub web_search: Option<(&'a dyn SearchProvider, &'a PermissionPolicy, ToolDefinition)>,
    pub max_tool_rounds: u32,
}

struct RoutedExternalNodeHandler {
    policy: crate::workflow::WorkflowOperationPolicy,
    execute: ExternalNodeHandler,
    reconcile: Option<ExternalOperationReconciler>,
}

/// 简单外部节点路由器，用于把具体节点类型挂到 Module 11 runtime。
pub struct RoutedExternalNodeExecutor {
    handlers: BTreeMap<String, RoutedExternalNodeHandler>,
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
        self.register_handler_with_policy(
            type_name,
            crate::workflow::WorkflowOperationPolicy::Untracked,
            handler,
        )
    }

    pub fn register_handler_with_policy(
        &mut self,
        type_name: impl Into<String>,
        policy: crate::workflow::WorkflowOperationPolicy,
        handler: ExternalNodeHandler,
    ) -> CoreResult<()> {
        self.register_handler_entry(type_name, policy, handler, None)
    }

    pub fn register_reconcilable_handler(
        &mut self,
        type_name: impl Into<String>,
        handler: ExternalNodeHandler,
        reconciler: ExternalOperationReconciler,
    ) -> CoreResult<()> {
        self.register_handler_entry(
            type_name,
            crate::workflow::WorkflowOperationPolicy::reconcilable_receipt(),
            handler,
            Some(reconciler),
        )
    }

    fn register_handler_entry(
        &mut self,
        type_name: impl Into<String>,
        policy: crate::workflow::WorkflowOperationPolicy,
        handler: ExternalNodeHandler,
        reconciler: Option<ExternalOperationReconciler>,
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
        if matches!(
            policy,
            crate::workflow::WorkflowOperationPolicy::Journaled {
                recovery: crate::workflow::WorkflowOperationRecoveryPolicy::ReconcileReceipt,
                ..
            }
        ) != reconciler.is_some()
        {
            return Err(CoreError::validation(
                "reconcile_receipt workflow handler requires exactly one reconciler",
            ));
        }
        self.handlers.insert(
            type_name,
            RoutedExternalNodeHandler {
                policy,
                execute: handler,
                reconcile: reconciler,
            },
        );
        Ok(())
    }

    /// 已注册外部节点 type_name 列表（产品路径与合同测试共用）。
    pub fn registered_type_names(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    /// 是否已注册指定 type_name。
    pub fn has_handler(&self, type_name: &str) -> bool {
        self.handlers.contains_key(type_name)
    }
}

impl Default for RoutedExternalNodeExecutor {
    /// 创建默认外部节点路由器。
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowExternalNodeExecutor for RoutedExternalNodeExecutor {
    fn operation_policy(
        &self,
        request: &WorkflowNodeExecutionRequest,
    ) -> CoreResult<crate::workflow::WorkflowOperationPolicy> {
        self.handlers
            .get(&request.type_name)
            .map(|handler| handler.policy)
            .ok_or_else(|| {
                CoreError::validation(format!(
                    "workflow external handler not found: {}",
                    request.type_name
                ))
            })
    }

    fn reconcile_operation(
        &mut self,
        request: &WorkflowNodeExecutionRequest,
    ) -> CoreResult<Option<WorkflowNodeExecutionOutput>> {
        let handler = self.handlers.get_mut(&request.type_name).ok_or_else(|| {
            CoreError::validation(format!(
                "workflow external handler not found: {}",
                request.type_name
            ))
        })?;
        match handler.reconcile.as_mut() {
            Some(reconcile) => reconcile(request),
            None => Ok(None),
        }
    }

    /// 按节点 type_name 分发到注册处理器。
    fn execute_external(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput> {
        if request.cancellation.is_cancelled() {
            return Err(CoreError::external_cancelled(
                "workflow_external_node",
                crate::contracts::ExternalDispatchOutcome::NotDispatched,
            ));
        }
        let type_name = request.type_name.clone();
        let handler = self.handlers.get_mut(&type_name).ok_or_else(|| {
            CoreError::validation(format!("workflow external handler not found: {type_name}"))
        })?;
        (handler.execute)(request)
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
    fn operation_policy(&self) -> crate::workflow::WorkflowOperationPolicy {
        crate::workflow::WorkflowOperationPolicy::replayable_receipt()
    }

    /// 将 Export 节点输入序列化为 artifact。
    fn export_artifact(
        &mut self,
        request: &WorkflowNodeExecutionRequest,
        export: WorkflowExportRequest,
    ) -> CoreResult<String> {
        let format = export.format.trim().to_ascii_lowercase();
        let title = export
            .title
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Export".to_owned());
        let bytes = if format == "json" {
            let payload = json!({
                "operation_id": request.operation_id,
                "workflow_id": request.workflow_id,
                "run_id": request.run_id,
                "node_id": request.node_id,
                "format": format,
                "title": export.title,
                "inputs": export.inputs,
            });
            serde_json::to_vec_pretty(&payload)?
        } else {
            let chapters = export_chapters_from_inputs(&export.inputs, &title)?;
            match format.as_str() {
                "markdown" | "md" => render_chapters_markdown(&chapters).into_bytes(),
                "epub" => render_chapters_epub(&chapters)?,
                "pdf" => render_chapters_pdf(&chapters),
                other => {
                    return Err(CoreError::validation(format!(
                        "unsupported workflow export format: {other}"
                    )))
                }
            }
        };
        request.dispatch_authorization.authorize_dispatch()?;
        let report = self.documents.write_artifact_with_cancellation(
            ArtifactWriteRequest {
                artifact_id: export.artifact_id.clone(),
                kind: ArtifactKind::Export,
                media_type: export_media_type(&format).to_owned(),
                bytes,
                operation_id: Some(request.operation_id.clone()),
                metadata: json!({
                    "operation_id": request.operation_id,
                    "workflow_id": request.workflow_id,
                    "run_id": request.run_id,
                    "node_id": request.node_id,
                }),
            },
            &request.cancellation,
        )?;
        Ok(report.descriptor.artifact_id)
    }
}

fn export_chapters_from_inputs(
    inputs: &PortMap,
    default_title: &str,
) -> CoreResult<Vec<(String, String)>> {
    if inputs.is_empty() {
        return Ok(vec![(default_title.to_owned(), String::new())]);
    }
    inputs
        .iter()
        .map(|(alias, value)| {
            let content = match value {
                PortValue::Inline { value } => value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
                _ => serde_json::to_string_pretty(value)?,
            };
            Ok((alias.clone(), content))
        })
        .collect()
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
    let report = documents.apply_patch_with_cancellation(
        patch,
        git,
        Some(&PatchCheckpointRequest {
            node_id: node_id.as_str().to_owned(),
            message: checkpoint_message.map(str::to_owned),
        }),
        runtime.cancellation(),
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
    /// F13：画布/预设写入的节点超时（ms）；未设或 0 时回退默认 120s。
    /// 兼容桌面历史字符串写入（`"7500"`）与正确 number。
    #[serde(
        default,
        deserialize_with = "deserialize_opt_u64_lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_ms: Option<u64>,
    /// F13：画布节点单次调用预算（USD）；与 `single_call_budget_usd` 二选一。
    #[serde(
        default,
        deserialize_with = "deserialize_opt_f64_lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub budget_usd: Option<f64>,
    /// F13：设置页预设字段名兼容。
    #[serde(
        default,
        deserialize_with = "deserialize_opt_f64_lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub single_call_budget_usd: Option<f64>,
}

fn deserialize_opt_u64_lenient<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptU64;
    impl<'de> Visitor<'de> for OptU64 {
        type Value = Option<u64>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("u64, number, or decimal string")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            if v < 0 {
                return Err(E::custom("timeout_ms cannot be negative"));
            }
            Ok(Some(v as u64))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            if !v.is_finite() || v < 0.0 {
                return Err(E::custom("timeout_ms must be finite non-negative"));
            }
            Ok(Some(v as u64))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed.parse::<u64>().map(Some).or_else(|_| {
                trimmed
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .map(|n| Some(n as u64))
                    .ok_or_else(|| E::custom(format!("invalid timeout_ms string: {v}")))
            })
        }
    }

    deserializer.deserialize_any(OptU64)
}

fn deserialize_opt_f64_lenient<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct OptF64;
    impl<'de> Visitor<'de> for OptF64 {
        type Value = Option<f64>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("f64 number or decimal string")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v as f64))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v as f64))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<f64>()
                .map(Some)
                .map_err(|_| E::custom(format!("invalid budget string: {v}")))
        }
    }

    deserializer.deserialize_any(OptF64)
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
    execute_llm_node_with_optional_search_tools(
        request,
        provider,
        ledger,
        WorkflowLlmSearchOptions {
            default_provider_id,
            default_model_id,
            ..WorkflowLlmSearchOptions::default()
        },
    )
}

/// 使用项目级 Search tool 执行 LLM/写作节点；模型可按需多轮检索后再给出最终输出。
pub fn execute_llm_node_with_project_search<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    options: WorkflowLlmSearchOptions<'_>,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_llm_node_with_optional_search_tools(request, provider, ledger, options)
}

/// 同时为 LLM/写作节点提供项目 Search 与外部 Web Search。
pub fn execute_llm_node_with_search_tools<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    options: WorkflowLlmSearchOptions<'_>,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_llm_node_with_optional_search_tools(request, provider, ledger, options)
}

fn execute_llm_node_with_optional_search_tools<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    options: WorkflowLlmSearchOptions<'_>,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<WorkflowLlmNodeConfig>(request.config.clone())?;
    let provider_id = config
        .provider_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(options.default_provider_id)
        .ok_or_else(|| CoreError::validation("LLM node provider_id is not configured"))?;
    let model_id = config
        .model_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(options.default_model_id)
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

    let timeout_ms = resolve_node_timeout_ms(config.timeout_ms);
    let single_call_budget_usd =
        resolve_node_single_call_budget_usd(config.budget_usd, config.single_call_budget_usd);
    let mut call_metadata = request.metadata.clone();
    if let Some(object) = call_metadata.as_object_mut() {
        object.insert("node_timeout_ms".to_owned(), json!(timeout_ms));
        if let Some(budget) = single_call_budget_usd {
            object.insert("node_single_call_budget_usd".to_owned(), json!(budget));
        }
    } else if call_metadata.is_null() {
        call_metadata = json!({
            "node_timeout_ms": timeout_ms,
            "node_single_call_budget_usd": single_call_budget_usd,
        });
    }

    let executor = ProviderExecutor::new(ledger);
    let base_context = ProviderCallContext {
        provider_id: provider_id.to_owned(),
        operation_id: Some(request.operation_id.clone()),
        workflow_id: Some(request.workflow_id.clone()),
        run_id: Some(request.run_id.clone()),
        node_id: Some(request.node_id.clone()),
        tool_call_id: None,
        timeout_ms,
        max_retries: 0,
        metadata: call_metadata.clone(),
        cancellation: request.cancellation.clone(),
        // F12-b：把 runtime 注入的派发栅栏传到 provider 与检索真实副作用边界。
        dispatch_authorization: request.dispatch_authorization.clone(),
    };

    if options.project_search.is_none() && options.web_search.is_none() {
        let response = executor.complete_llm(
            provider,
            &base_context,
            LlmRequest {
                model_id: model_id.to_owned(),
                messages,
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: call_metadata,
            },
        )?;
        enforce_single_call_budget(single_call_budget_usd, response.cost_usd)?;
        return llm_response_to_output(response);
    }

    if options.max_tool_rounds == 0 || options.max_tool_rounds > 32 {
        return Err(CoreError::validation(
            "search tool max_tool_rounds must be between 1 and 32",
        ));
    }
    let project_tool_executor = options.project_search.as_ref().map(|(retrieval, tool)| {
        ProjectSearchToolExecutor::new(retrieval, base_context.clone(), [tool.name.clone()])
    });
    let web_tool_executor = options
        .web_search
        .as_ref()
        .map(|(search_provider, policy, tool)| {
            WebSearchToolExecutor::new(
                *search_provider,
                ledger,
                policy,
                base_context.clone(),
                [tool.name.clone()],
            )
        });
    let mut tool_router = ToolExecutorRouter::new();
    if let (Some((_, tool)), Some(tool_executor)) = (
        options.project_search.as_ref(),
        project_tool_executor.as_ref(),
    ) {
        tool_router.register(tool.name.clone(), tool_executor)?;
    }
    if let (Some((_, _, tool)), Some(tool_executor)) =
        (options.web_search.as_ref(), web_tool_executor.as_ref())
    {
        tool_router.register(tool.name.clone(), tool_executor)?;
    }
    let tools = options
        .project_search
        .iter()
        .map(|(_, tool)| tool.clone())
        .chain(options.web_search.iter().map(|(_, _, tool)| tool.clone()))
        .collect::<Vec<_>>();
    let tool_names = tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    for round in 0..=options.max_tool_rounds {
        request.cancellation.check()?;
        let mut round_context = base_context.clone();
        round_context.operation_id = Some(format!("{}:llm-round-{round}", request.operation_id));
        round_context.metadata = json!({
            "node_metadata": call_metadata,
            "tool_round": round,
            "search_tools": tool_names,
        });
        let response = executor.complete_llm(
            provider,
            &round_context,
            LlmRequest {
                model_id: model_id.to_owned(),
                messages: messages.clone(),
                tools: tools.clone(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: round_context.metadata.clone(),
            },
        )?;
        enforce_single_call_budget(single_call_budget_usd, response.cost_usd)?;
        if response.tool_calls.is_empty() {
            return llm_response_to_output(response);
        }
        if round >= options.max_tool_rounds {
            return Err(CoreError::validation(
                "LLM node search tool max rounds exceeded before final answer",
            ));
        }
        messages.push(response.message.clone());
        for call in &response.tool_calls {
            let output = tool_router.execute(
                &ToolExecutionContext {
                    provider_id: provider_id.to_owned(),
                    workflow_id: Some(request.workflow_id.clone()),
                    run_id: Some(request.run_id.clone()),
                    node_id: Some(request.node_id.clone()),
                    round,
                },
                call,
            )?;
            messages.push(tool_result_message(call, output));
        }
    }
    Err(CoreError::validation(
        "LLM node search tool loop ended unexpectedly",
    ))
}

/// F13：节点超时；未配置或 0 时保持历史默认 120s。
fn resolve_node_timeout_ms(timeout_ms: Option<u64>) -> u64 {
    timeout_ms.filter(|value| *value > 0).unwrap_or(120_000)
}

/// F13：节点单次预算（画布 `budget_usd` 或预设 `single_call_budget_usd`）。
fn resolve_node_single_call_budget_usd(
    budget_usd: Option<f64>,
    single_call_budget_usd: Option<f64>,
) -> Option<f64> {
    budget_usd
        .or(single_call_budget_usd)
        .filter(|value| value.is_finite() && *value > 0.0)
}

/// F13：响应成本超过节点单次预算时 fail-loud，禁止当作成功节点完成。
fn enforce_single_call_budget(limit_usd: Option<f64>, cost_usd: Option<f64>) -> CoreResult<()> {
    let Some(limit) = limit_usd else {
        return Ok(());
    };
    let Some(cost) = cost_usd else {
        return Ok(());
    };
    if !cost.is_finite() || cost < 0.0 {
        return Err(CoreError::validation(format!(
            "LLM node cost_usd is invalid under single-call budget {limit}"
        )));
    }
    if cost > limit {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "node_single_call_budget_usd".to_owned(),
            reason: format!("single-call cost {cost} exceeds node budget {limit}"),
        });
    }
    Ok(())
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
    /// F24：节点 prompt_template；非空时并入四步 LLM 指令。
    #[serde(default)]
    pub prompt_template: Option<String>,
    /// F24：兼容 agent_prompt.summarizer / 项目级 agent prompt 字段。
    #[serde(default)]
    pub agent_prompt: Option<String>,
    /// F13：节点超时（ms）；未设或 0 时回退 120s。
    #[serde(default, deserialize_with = "deserialize_opt_u64_lenient")]
    pub timeout_ms: Option<u64>,
    /// F13：单次调用预算（USD）。
    #[serde(default, deserialize_with = "deserialize_opt_f64_lenient")]
    pub budget_usd: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_lenient")]
    pub single_call_budget_usd: Option<f64>,
}

impl WorkflowSummarizerNodeConfig {
    /// 从持久化节点配置解析并执行与产品保存、运行预检相同的业务校验。
    pub fn from_value(value: serde_json::Value) -> CoreResult<Self> {
        let mut config = serde_json::from_value::<Self>(value).map_err(|error| {
            CoreError::validation(format!("summarizer node config is invalid: {error}"))
        })?;
        config.provider_id = config.provider_id.trim().to_owned();
        config.model_id = config.model_id.trim().to_owned();
        config.chapter_id = config.chapter_id.trim().to_owned();
        config.chapter_document_id = config.chapter_document_id.trim().to_owned();
        config.chapter_text_alias = config.chapter_text_alias.trim().to_owned();
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> CoreResult<()> {
        validate_summarizer_required_field("provider_id", &self.provider_id)?;
        validate_summarizer_required_field("model_id", &self.model_id)?;
        validate_summarizer_required_field("chapter_id", &self.chapter_id)?;
        validate_summarizer_required_field("chapter_document_id", &self.chapter_document_id)?;
        validate_summarizer_required_field("chapter_text_alias", &self.chapter_text_alias)?;
        Ok(())
    }
}

fn validate_summarizer_required_field(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!(
            "summarizer node {field} cannot be empty"
        )));
    }
    Ok(())
}

fn default_chapter_text_alias() -> String {
    "chapter_text".to_owned()
}

/// 产品级工作流校验：拓扑与节点业务配置只走这一入口，保存、显式校验和运行预检共用。
pub fn validate_workflow_execution_contracts(workflow: &WorkflowDefinition) -> CoreResult<()> {
    workflow.validate_topology()?;
    let node_ids = workflow
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let mut approval_ids = BTreeSet::new();
    for node in &workflow.nodes {
        match node.type_name.as_str() {
            "summarizer" => {
                let config = WorkflowSummarizerNodeConfig::from_value(node.config.clone())
                    .map_err(|error| {
                        CoreError::validation(format!(
                            "summarizer node {} failed configuration validation: {error}",
                            node.id.as_str()
                        ))
                    })?;
                require_incoming_data_alias(workflow, node, &config.chapter_text_alias)?;
            }
            "condition" | "eval" => {
                let config = serde_json::from_value::<ConditionNodeConfig>(node.config.clone())
                    .map_err(|error| node_configuration_error(node, error))?;
                require_non_empty_node_field(node, "input_alias", &config.input_alias)?;
                require_non_empty_node_field(node, "operator", &config.operator)?;
                if !matches!(config.operator.as_str(), "truthy" | "equals" | "not_equals") {
                    return Err(CoreError::validation(format!(
                        "{} node {} has unsupported operator {}",
                        node.type_name,
                        node.id.as_str(),
                        config.operator
                    )));
                }
                require_incoming_data_alias(workflow, node, &config.input_alias)?;
                for edge in workflow.edges.iter().filter(|edge| {
                    edge.kind == WorkflowEdgeKind::Control
                        && edge.from.node_id == node.id
                        && edge.alias.is_some()
                }) {
                    let selector = edge.alias.as_deref().map(str::trim).unwrap_or_default();
                    if !matches!(selector, "true" | "false") {
                        return Err(CoreError::validation(format!(
                            "condition control edge {} requires branch selector true or false",
                            edge.id.as_str()
                        )));
                    }
                }
            }
            "search" | "project_search" => {
                let config =
                    serde_json::from_value::<WorkflowProjectSearchNodeConfig>(node.config.clone())
                        .map_err(|error| node_configuration_error(node, error))?;
                require_non_empty_node_field(node, "query_alias", &config.query_alias)?;
                require_incoming_data_alias(workflow, node, &config.query_alias)?;
            }
            "document" | "document_read" => {
                let config =
                    serde_json::from_value::<WorkflowDocumentReadConfig>(node.config.clone())
                        .map_err(|error| node_configuration_error(node, error))?;
                if config.path.as_os_str().is_empty() {
                    return Err(CoreError::validation(format!(
                        "{} node {} path cannot be empty",
                        node.type_name,
                        node.id.as_str()
                    )));
                }
            }
            "loop" => {
                let config = serde_json::from_value::<LoopNodeConfig>(node.config.clone())
                    .map_err(|error| node_configuration_error(node, error))?;
                LoopPolicy {
                    max_iterations: config.max_iterations,
                    timeout_ms: config.timeout_ms,
                    budget_limit_usd: config.budget_limit_usd,
                    stop_condition: config.stop_condition.clone(),
                }
                .validate()?;
                let stop = config.stop_condition.as_object().ok_or_else(|| {
                    CoreError::validation(format!(
                        "loop node {} stop_condition must be an object",
                        node.id.as_str()
                    ))
                })?;
                let input_alias = stop
                    .get("input_alias")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                require_non_empty_node_field(node, "stop_condition.input_alias", input_alias)?;
                if !stop.contains_key("equals") {
                    return Err(CoreError::validation(format!(
                        "loop node {} stop_condition requires equals",
                        node.id.as_str()
                    )));
                }
                require_incoming_data_alias(workflow, node, input_alias)?;
                for rerun_node_id in &config.rerun_node_ids {
                    if !node_ids.contains(rerun_node_id) {
                        return Err(CoreError::validation(format!(
                            "loop node {} references missing rerun node {}",
                            node.id.as_str(),
                            rerun_node_id.as_str()
                        )));
                    }
                }
                if config.rerun_node_ids.is_empty()
                    && !workflow.edges.iter().any(|edge| {
                        edge.kind == WorkflowEdgeKind::Control && edge.from.node_id == node.id
                    })
                {
                    return Err(CoreError::validation(format!(
                        "loop node {} requires rerun_node_ids or an outgoing control edge",
                        node.id.as_str()
                    )));
                }
            }
            "approval" => {
                let config = serde_json::from_value::<ApprovalNodeConfig>(node.config.clone())
                    .map_err(|error| node_configuration_error(node, error))?;
                require_non_empty_node_field(node, "approval_id", &config.approval_id)?;
                if !approval_ids.insert(config.approval_id.trim().to_owned()) {
                    return Err(CoreError::validation(format!(
                        "duplicate workflow approval_id: {}",
                        config.approval_id.trim()
                    )));
                }
            }
            "export" => {
                let config = serde_json::from_value::<ExportNodeConfig>(node.config.clone())
                    .map_err(|error| node_configuration_error(node, error))?;
                require_non_empty_node_field(node, "artifact_id", &config.artifact_id)?;
                let format = config.format.trim().to_ascii_lowercase();
                if !matches!(format.as_str(), "json" | "markdown" | "md" | "epub" | "pdf") {
                    return Err(CoreError::validation(format!(
                        "export node {} has unsupported format {}",
                        node.id.as_str(),
                        config.format
                    )));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn node_configuration_error(
    node: &crate::contracts::NodeInstance,
    error: serde_json::Error,
) -> CoreError {
    CoreError::validation(format!(
        "{} node {} failed configuration validation: {error}",
        node.type_name,
        node.id.as_str()
    ))
}

fn require_non_empty_node_field(
    node: &crate::contracts::NodeInstance,
    field: &str,
    value: &str,
) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!(
            "{} node {} {field} cannot be empty",
            node.type_name,
            node.id.as_str()
        )));
    }
    Ok(())
}

fn require_incoming_data_alias(
    workflow: &WorkflowDefinition,
    node: &crate::contracts::NodeInstance,
    alias: &str,
) -> CoreResult<()> {
    let alias = alias.trim();
    let has_edge = workflow.edges.iter().any(|edge| {
        edge.kind == WorkflowEdgeKind::Data
            && edge.to.node_id == node.id
            && edge.alias.as_deref().map(str::trim) == Some(alias)
    });
    if !has_edge {
        return Err(CoreError::validation(format!(
            "{} node {} requires an incoming data edge with alias {alias}",
            node.type_name,
            node.id.as_str()
        )));
    }
    Ok(())
}

/// F17：把项目保存的双模式策略解析为 Summarizer 四步领域策略。
/// 未出现的键保留领域默认值；文件存在但无法读取/解析时必须在 provider dispatch 前失败。
fn load_summarizer_confirmation_policy(
    project_root: &Path,
    auto_mode: bool,
) -> CoreResult<(
    crate::rag::models::WritingConfirmationPolicy,
    BTreeMap<crate::rag::models::ConfirmationKind, String>,
)> {
    use crate::config::{ConfirmationAutoModePolicy, ConfirmationNormalPolicy};
    use crate::rag::models::{
        confirmation_prompt_key, ConfirmationKind, ConfirmationMode, WritingConfirmationPolicy,
    };

    let mut policy = if auto_mode {
        WritingConfirmationPolicy::auto_audit_default()
    } else {
        WritingConfirmationPolicy::normal_default()
    };
    let resources = crate::rag::resources::load_prompt_resources()?;
    let mut approval_prompts = BTreeMap::new();
    for kind in [
        ConfirmationKind::SegmentSummary,
        ConfirmationKind::EventSummary,
        ConfirmationKind::ChapterSummary,
        ConfirmationKind::StageSummary,
    ] {
        let key = confirmation_prompt_key(kind);
        let prompt = resources
            .get(key)
            .ok_or_else(|| CoreError::validation(format!("missing prompt resource: {key}")))?
            .prompt
            .trim()
            .to_owned();
        approval_prompts.insert(kind, prompt);
    }
    let Some(settings) = crate::config::read_confirmation_policy_settings(project_root)? else {
        return Ok((policy, approval_prompts));
    };
    for setting in settings {
        let kind = match setting.confirmation_kind.as_str() {
            "segment_summary" => ConfirmationKind::SegmentSummary,
            "event_summary" => ConfirmationKind::EventSummary,
            "chapter_summary" => ConfirmationKind::ChapterSummary,
            "stage_summary" => ConfirmationKind::StageSummary,
            _ => continue,
        };
        if !setting.approval_prompt.trim().is_empty() {
            approval_prompts.insert(kind, setting.approval_prompt.trim().to_owned());
        }
        let mode = if auto_mode {
            match setting.auto_mode_policy {
                ConfirmationAutoModePolicy::AllowByDefault => ConfirmationMode::Skip,
                ConfirmationAutoModePolicy::AutoApproval => ConfirmationMode::AutoAudit,
            }
        } else {
            match setting.normal_policy {
                ConfirmationNormalPolicy::ManualReview => ConfirmationMode::RequireHuman,
                ConfirmationNormalPolicy::AllowByDefault => ConfirmationMode::Skip,
            }
        };
        policy.set_mode(kind, mode);
    }
    Ok((policy, approval_prompts))
}

/// 执行 Summarizer 节点：加载写作知识库 → 四步总结 → 落库建索引 → 生成四层确认项。
/// 这是「故事段划分并概括」等总结机制的真实生产入口，取代把 summarizer 降级为普通 LLM 节点。
pub fn execute_summarizer_node<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    project_root: &Path,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_summarizer_node_with_optional_search_tools(
        request,
        provider,
        ledger,
        project_root,
        None,
        None,
        0,
    )
}

pub fn execute_summarizer_node_with_project_search<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    project_root: &Path,
    retrieval: &ProjectRetrievalRuntime,
    search_tool: ToolDefinition,
    max_tool_rounds: u32,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_summarizer_node_with_optional_search_tools(
        request,
        provider,
        ledger,
        project_root,
        Some((retrieval, search_tool)),
        None,
        max_tool_rounds,
    )
}

pub fn execute_summarizer_node_with_search_tools<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    project_root: &Path,
    project_search: Option<(&ProjectRetrievalRuntime, ToolDefinition)>,
    web_search: Option<(&dyn SearchProvider, &PermissionPolicy, ToolDefinition)>,
    max_tool_rounds: u32,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_summarizer_node_with_optional_search_tools(
        request,
        provider,
        ledger,
        project_root,
        project_search,
        web_search,
        max_tool_rounds,
    )
}

fn execute_summarizer_node_with_optional_search_tools<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    project_root: &Path,
    project_search: Option<(&ProjectRetrievalRuntime, ToolDefinition)>,
    web_search: Option<(&dyn SearchProvider, &PermissionPolicy, ToolDefinition)>,
    max_tool_rounds: u32,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    use crate::contracts::{AutoModeState, RunControl};
    use crate::rag::models::ConfirmationState;
    use crate::rag::pipeline::SummaryPipelineExecutor;
    use crate::rag::store::SqliteWritingKnowledgeStore;
    use crate::rag::summarizer::{
        SummarizerConfig, SummarizerExecutor, SummarizerWorkflowOperationContext,
    };
    use crate::workflow::{RuntimeConfirmation, RuntimeConfirmationState};

    let config = WorkflowSummarizerNodeConfig::from_value(request.config.clone())?;
    let chapter_text = input_text(&request.inputs, &config.chapter_text_alias)?;
    let prompts = crate::rag::resources::load_prompt_resources()?;

    // 先验证当前章节关系闭包可读取，避免相关数据损坏时仍发起昂贵外部调用。
    let store = SqliteWritingKnowledgeStore::open(project_root)?;
    if let Some(receipt) =
        store.load_operation_receipt(&request.operation_id, &request.request_hash)?
    {
        return serde_json::from_value(receipt.response_json).map_err(Into::into);
    }
    // 只有迁移后的空数据库才代表新项目；损坏、权限或 JSON 错误必须阻断总结，
    // 不能静默创建空知识库后覆盖已有作品事实。
    let generation_context = store.load_summary_generation_context(&config.chapter_id)?;
    store.load_summary_working_set(&config.chapter_id, None)?;
    let (policy, approval_prompts) =
        load_summarizer_confirmation_policy(project_root, config.auto_mode)?;

    // 四步总结 → 组装 draft。
    let author_prompt = config
        .prompt_template
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .or_else(|| {
            config
                .agent_prompt
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned()
        });
    let timeout_ms = resolve_node_timeout_ms(config.timeout_ms);
    let summarizer = SummarizerExecutor::new(
        provider,
        ledger,
        &prompts,
        SummarizerConfig {
            provider_id: config.provider_id.clone(),
            model_id: config.model_id.clone(),
            chapter_document_id: config.chapter_document_id.clone(),
            run_id: Some(request.run_id.as_str().to_owned()),
            timeout_ms,
            cancellation: request.cancellation.clone(),
            dispatch_authorization: request.dispatch_authorization.clone(),
            prompt_template: author_prompt,
            generation_context,
            workflow_operation: Some(SummarizerWorkflowOperationContext {
                project_root: project_root.to_path_buf(),
                workflow_id: request.workflow_id.clone(),
                run_id: request.run_id.clone(),
                node_id: request.node_id.clone(),
                operation_id: request.operation_id.clone(),
                operation_attempt: request.operation_attempt,
                request_hash: request.request_hash.clone(),
            }),
        },
    );
    let summarizer = match project_search {
        Some((retrieval, search_tool)) => {
            summarizer.with_project_search(retrieval, search_tool, max_tool_rounds)
        }
        None => summarizer,
    };
    let summarizer = match web_search {
        Some((search_provider, permission_policy, search_tool)) => summarizer.with_web_search(
            search_provider,
            permission_policy,
            search_tool,
            max_tool_rounds,
        ),
        None => summarizer,
    };
    let draft = summarizer.summarize_chapter(&config.chapter_id, &chapter_text)?;
    let mut audit_decisions = BTreeMap::new();
    if config.auto_mode {
        use crate::rag::models::{ConfirmationKind, ConfirmationMode};

        for kind in [
            ConfirmationKind::SegmentSummary,
            ConfirmationKind::EventSummary,
            ConfirmationKind::ChapterSummary,
            ConfirmationKind::StageSummary,
        ] {
            if policy.mode_for(kind) != ConfirmationMode::AutoAudit {
                continue;
            }
            let approval_prompt = approval_prompts.get(&kind).ok_or_else(|| {
                CoreError::validation(format!(
                    "missing Auto Mode approval prompt for confirmation kind {kind:?}"
                ))
            })?;
            audit_decisions.insert(
                kind,
                summarizer.audit_confirmation(kind, approval_prompt, &chapter_text, &draft)?,
            );
        }
    }

    // 外部计算不占写锁；提交前在统一写护栏内重放检查并重新读取最新快照，
    // 避免并发确认决策被长耗时总结器的旧快照覆盖。
    let _writer_lock = store.acquire_writer_lock()?;
    if let Some(receipt) =
        store.load_operation_receipt(&request.operation_id, &request.request_hash)?
    {
        return serde_json::from_value(receipt.response_json).map_err(Into::into);
    }
    let knowledge = store.load_summary_working_set(&config.chapter_id, Some(&draft))?;

    // 落库建索引 + 生成四层确认项。
    let auto_mode = if config.auto_mode {
        AutoModeState {
            enabled: true,
            preauthorized_budget_usd: None,
        }
    } else {
        AutoModeState::default()
    };
    let pipeline = SummaryPipelineExecutor::with_cancellation(
        &knowledge,
        policy,
        auto_mode,
        request.cancellation.clone(),
    )
    .with_auto_audit_decisions(audit_decisions);
    let report = pipeline.apply_draft(draft)?;

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

    let output = WorkflowNodeExecutionOutput {
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
    };
    // C2：章节作用域落盘，不 wipe 其它章的故事段/事件。
    store.save_chapter_knowledge_with_operation_locked(
        &knowledge,
        &config.chapter_id,
        &request.operation_id,
        &request.request_hash,
        &serde_json::to_value(&output)?,
        &request.cancellation,
        &_writer_lock,
    )?;
    Ok(output)
}

pub fn reconcile_summarizer_operation(
    request: &WorkflowNodeExecutionRequest,
    project_root: &Path,
) -> CoreResult<Option<WorkflowNodeExecutionOutput>> {
    let store = crate::rag::store::SqliteWritingKnowledgeStore::open(project_root)?;
    store
        .load_operation_receipt(&request.operation_id, &request.request_hash)?
        .map(|receipt| serde_json::from_value(receipt.response_json).map_err(Into::into))
        .transpose()
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
            operation_id: Some(request.operation_id.clone()),
            workflow_id: Some(request.workflow_id.clone()),
            run_id: Some(request.run_id.clone()),
            node_id: Some(request.node_id.clone()),
            tool_call_id: None,
            timeout_ms: 60_000,
            max_retries: 0,
            metadata: request.metadata.clone(),
            cancellation: request.cancellation.clone(),
            // F12-b：搜索节点同样在真实副作用边界复核 control/lease。
            dispatch_authorization: request.dispatch_authorization.clone(),
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

/// 仅供内存合同夹具使用；正式产品路径必须调用 `execute_project_retrieval_node_for_project`。
#[doc(hidden)]
pub fn execute_project_search_node_for_test_fixture(
    request: WorkflowNodeExecutionRequest,
    retrieval: &dyn HybridSearch,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<WorkflowProjectSearchNodeConfig>(request.config)?;
    let query = input_text(&request.inputs, &config.query_alias)?;
    let limit = config.limit.unwrap_or(10);
    validate_product_search_limit(limit)?;
    request.dispatch_authorization.authorize_dispatch()?;
    let results = retrieval.search(HybridSearchRequest::new(query, None, limit))?;
    validate_product_search_result_budget(&results)?;
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

/// F1/F2 产品组合根入口：将 workflow 身份、取消和 dispatch 栅栏传入项目级检索运行时。
pub fn execute_project_retrieval_node_for_project(
    project_root: &std::path::Path,
    request: WorkflowNodeExecutionRequest,
    retrieval: &crate::retrieval::ProjectRetrievalRuntime,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<WorkflowProjectSearchNodeConfig>(request.config.clone())?;
    let query = input_text(&request.inputs, &config.query_alias)?;
    let limit = config.limit.unwrap_or(10);
    validate_product_search_limit(limit)?;
    let context = ProviderCallContext {
        provider_id: "project_retrieval".to_owned(),
        operation_id: Some(request.operation_id.clone()),
        workflow_id: Some(request.workflow_id.clone()),
        run_id: Some(request.run_id.clone()),
        node_id: Some(request.node_id.clone()),
        tool_call_id: None,
        timeout_ms: 60_000,
        max_retries: 0,
        metadata: request.metadata,
        cancellation: request.cancellation,
        dispatch_authorization: request.dispatch_authorization,
    };
    let results = retrieval.search(query, limit, context)?;
    let mut outputs = PortMap::new();
    outputs.insert("results".to_owned(), PortValue::inline(json!(results)));
    Ok(WorkflowNodeExecutionOutput {
        outputs,
        metadata: json!({
            "retrieval_scope": "project",
            "result_count": results.len(),
            "vector_enabled": retrieval.vector_enabled(),
            "project_root": project_root,
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
    let output = executor.execute_with_control(
        manifest,
        SkillRunRequest {
            skill_id: config.skill_id,
            operation_id: Some(request.operation_id),
            inputs: request.inputs,
            metadata: request.metadata,
        },
        &request.cancellation,
        &request.dispatch_authorization,
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
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}
