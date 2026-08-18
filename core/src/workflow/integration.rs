use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::contracts::{
    condition_branch_for_port, ArtifactKind, CoreError, CoreResult, DocumentPatch, LoopPolicy,
    NodeId, PermissionPolicy, PortMap, PortValue, WorkflowDefinition, WorkflowEdgeKind,
    WorkflowExecutionLimits, EXECUTION_OUTPUT_PORT_FALSE, EXECUTION_OUTPUT_PORT_TRUE,
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
    ToolDefinition, WebSearchToolExecutor,
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
    /// U108：Module 9 写作工具（find/register/行号 patch）的装配参数。
    /// 为 `None` 时该节点只拿到检索类工具，行为与接线前一致。
    pub writing_tools: Option<WorkflowWritingToolOptions<'a>>,
    /// U113：节点超时回落与 tool-use 轮次上限统一从项目全局限制读取，
    /// 不再由调用方各自传裸值。
    pub limits: WorkflowExecutionLimits,
}

/// U108：写作节点把 Module 9 工具下发给模型所需的全部依赖。
///
/// 设计约束（对应审查报告列出的三个非重构性阻碍）：
/// - **正文来源**：由节点 config 的 `document_id` 指名，执行时从磁盘读，
///   不依赖上游端口传大文本（符合「引用式数据流」原则）。
/// - **副作用边界**：行号 patch 只产出 `DocumentPatch` 作为工具结果，
///   由节点输出走确认流落盘，executor 内不直写文件。
/// - **知识库**：register 类工具写入内存知识库，节点结束后由调用方
///   在 operation receipt 保护下持久化。
pub struct WorkflowWritingToolOptions<'a> {
    /// 该节点对应的写作 agent。
    pub agent: crate::rag::models::WritingAgentKind,
    /// 经权限过滤后允许下发的工具名；空集表示不下发任何写作工具。
    pub allowed_tools: BTreeSet<String>,
    /// 项目写作知识库（find/register 的数据源）。
    pub knowledge: &'a crate::rag::MemoryWritingKnowledgeBase,
    /// 节点声明的可编辑文档；缺省时写入类工具会被自动剔除。
    pub document: Option<crate::rag::tools::WriterDocumentContext<'a>>,
    /// U114：本次写作针对的章节 id，上下文装配的归属键。
    /// 缺省时退化为「不装配上下文」，节点行为与接线前一致。
    pub chapter_id: Option<&'a str>,
    /// U117：确认项策略。写作节点产出的确认项据此决定初始状态
    /// （人工 / 跳过 / Auto Mode 审计）。
    pub confirmation_policy: crate::rag::models::WritingConfirmationPolicy,
    /// U117：Auto Mode 状态，决定 `AutoAudit` 是否真的生效。
    pub auto_mode: crate::contracts::AutoModeState,
    /// U108 阶段 3：节点运行开始时的正文快照会话。
    ///
    /// 挂上它，行号 patch 工具才会把改动累积下来并随确认项持久化；
    /// 为 `None` 时行号工具只把 patch 返回给模型看，节点结束即丢失——
    /// 那正是「用户点同意后正文纹丝不动」的缺陷状态。
    ///
    /// `document_id` 必须是**绝对**路径：`DocumentService::apply_patch` 直接
    /// `PathBuf::from(patch.document_id)`，而路径沙箱对相对路径一律拒绝。
    pub patch_session: Option<crate::rag::line_patch::PatchSession>,
    /// 13-B：正文引用 `{{ref:文档ID#L起始-L结束}}` 的展开来源。
    ///
    /// 大纲类文本（`outline` / `details` / `global_outline` …）里可以写引用，
    /// 装配层会**就地展开成原文**再交给模型——`{{ref:...}}` 字面量进请求体是安全缺口
    /// （Auto Mode 的审计 LLM 会在「审的是占位符」的前提下给出虚假通过）。
    ///
    /// **为什么是一张预读好的表、而不是一个文件系统句柄**：
    /// 引用可指向**任意** document_id（不限当前节点那一篇），读盘就需要项目根
    /// 与路径沙箱（`ensure_path_under_root`）。那两样都在 `commands` 层，
    /// **沙箱责任必须留在已经持有项目根的那一层**——把句柄传进 `rag` 会让
    /// 越权检查散落到一个不该管这件事的模块里。
    ///
    /// 为 `None`（或表里没有被引文档）时，展开器把占位符换成可诊断标记并发警告，
    /// 不静默放过、也不 panic。
    pub reference_documents: Option<crate::rag::reference::InMemoryReferenceDocuments>,
    /// U175：本节点可用的规划正文（全局总纲 / 本章大纲），由调用方预读。
    ///
    /// **为什么是预读好的值、而不是一个文件系统句柄**：与 `reference_documents`
    /// 同一个理由——解析这些路径需要项目文档根与路径沙箱
    /// （`ensure_path_under_root`），那两样都在 `commands` 层。
    /// 把句柄传进 `rag` 会让越权检查散落到一个不该管这件事的模块里。
    ///
    /// 为 `None`（或字段为 `None`）时装配层给空态文本，节点仍可运行。
    pub planning: Option<WorkflowWritingPlanningContext>,
}

/// U175：写作节点的规划类上下文，从磁盘预读。
///
/// **只放路径映射无歧义的那几项**。判据是「由 id 到文件是不是一对一的直接映射」：
/// - `global_outline` ← `planning/global.md`：**固定路径**，无歧义。
/// - `outline` ← `planning/chapters/{chapter_id}.md`：由 `chapter_id` **直接**成名，
///   无歧义（作用域约定见 CLAUDE.md「Tool Execution & Document Scope」）。
///
/// 刻意**不含** `stage_outline` 与 `previous_chapter_text`：
/// - 阶段总纲在 `planning/stages/*.md`，但没有任何配置提供 `stage_id`
///   ⇒ 要靠猜是哪一个文件；
/// - 「上一章原文」要先知道**哪一章是上一章**，那需要一套章节排序约定
///   （补零？分卷跨卷？），本项目尚未确立。
///
/// 两者的共同点是**猜错的代价是把错的材料当对的喂给模型**，比没有更糟：
/// 作者要到成稿里才发现承接错了对象。所以它们保持空态文本，
/// 由作者用数据边显式传入。
#[derive(Debug, Clone, Default)]
pub struct WorkflowWritingPlanningContext {
    /// `planning/global.md` 的正文。
    pub global_outline: Option<String>,
    /// `planning/chapters/{chapter_id}.md` 的正文。
    pub outline: Option<String>,
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

/// 生产用运行态引用解析器：checkpoint / patch commit 直接问 Git。
///
/// 与 `FilesystemRuntimeReferenceResolver` 的分工是**信息来源**，不是新旧：
/// 后者的 commit id 靠 `with_checkpoint`/`with_patch_commit` 由调用方灌入，
/// 适合测试构造确定场景；诊断跑在真实项目上，没人能预先给出「应该存在哪些 commit」，
/// 只能反过来拿运行快照里记的 id 去问仓库。
pub struct GitRuntimeReferenceResolver<'a> {
    artifact_root: PathBuf,
    git: &'a GitService,
}

impl<'a> GitRuntimeReferenceResolver<'a> {
    /// 创建 Git 引用解析器。
    pub fn new(artifact_root: impl Into<PathBuf>, git: &'a GitService) -> Self {
        Self {
            artifact_root: artifact_root.into(),
            git,
        }
    }
}

impl RuntimeReferenceResolver for GitRuntimeReferenceResolver<'_> {
    /// document_id 是规范化绝对路径，按存在性检查。
    fn document_exists(&self, document_id: &str) -> CoreResult<bool> {
        Ok(Path::new(document_id).exists())
    }

    /// chunk 是可重建的索引产物，不构成需要人工恢复的悬空引用。
    ///
    /// 返回 `true` 而非 `FilesystemRuntimeReferenceResolver` 的 `false`：那边保守报缺失
    /// 是为了让测试能断言「未登记即缺失」，而诊断面向用户——把可自动重建的东西
    /// 报成「引用缺失，需人工恢复」是假警报。当前生产也不产出 `ChunkRef`
    /// （`PortValue::ChunkRef` 全仓库只有匹配分支、无构造点），这里属防御性分支。
    fn chunk_exists(&self, _chunk_id: &str) -> CoreResult<bool> {
        Ok(true)
    }

    /// artifact_id 按 artifact_root 下相对路径检查。
    fn artifact_exists(&self, artifact_id: &str) -> CoreResult<bool> {
        Ok(self.artifact_root.join(artifact_id).exists())
    }

    /// patch session commit 落在项目仓库里，问 Git 是否还能解析。
    fn patch_session_commit_exists(&self, patch_session_commit_id: &str) -> CoreResult<bool> {
        self.git.commit_exists(patch_session_commit_id)
    }

    /// checkpoint 就是一个 commit，同样问 Git。
    fn checkpoint_exists(&self, checkpoint_id: &str) -> CoreResult<bool> {
        self.git.commit_exists(checkpoint_id)
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
///
/// 建 Git 检查点，等价于 `apply_confirmed_patch_with_checkpoint(.., true)`。
pub fn apply_confirmed_patch(
    runtime: &mut WorkflowRuntime,
    documents: &FileDocumentService,
    git: Option<&GitService>,
    node_id: &NodeId,
    patch: &DocumentPatch,
    checkpoint_message: Option<&str>,
) -> CoreResult<WorkflowPatchApplyOutcome> {
    apply_confirmed_patch_with_checkpoint(
        runtime,
        documents,
        git,
        node_id,
        patch,
        checkpoint_message,
        true,
    )
}

/// U111：按 `checkpoint_enabled` 决定 patch 写回时建不建 Git 检查点。
///
/// `checkpoint_enabled = false` 时**只跳过检查点，正文照常落盘**——
/// 该开关的语义是「要不要留一条可回滚的 Git 记录」，不是「要不要写正文」。
/// 把它做成后者等于用一个设置项静默阉割核心功能。
///
/// 实现上传 `None` 作为 `PatchCheckpointRequest`：`apply_patch_with_cancellation`
/// 内部按 `(git, checkpoint_request)` 双 `Some` 才建检查点，任一为 `None` 即跳过。
/// 这里刻意不改传 `git: None` ——`git` 句柄将来可能有检查点之外的用途，
/// 用「不给检查点请求」表达意图比「不给 git」更贴合这个开关。
pub fn apply_confirmed_patch_with_checkpoint(
    runtime: &mut WorkflowRuntime,
    documents: &FileDocumentService,
    git: Option<&GitService>,
    node_id: &NodeId,
    patch: &DocumentPatch,
    checkpoint_message: Option<&str>,
    checkpoint_enabled: bool,
) -> CoreResult<WorkflowPatchApplyOutcome> {
    // 写回分成两步：先在 runtime 上做只读校验，再调用 DocumentService
    // 修改文件。只有真实文件写入和 checkpoint 都成功后，才把运行态置为
    // Applied，避免 I/O 失败时留下“已写回”的错误快照。
    runtime.ensure_patch_write_back_can_start(node_id)?;
    let checkpoint_request = checkpoint_enabled.then(|| PatchCheckpointRequest {
        node_id: node_id.as_str().to_owned(),
        message: checkpoint_message.map(str::to_owned),
    });
    let report = documents.apply_patch_with_cancellation(
        patch,
        git,
        checkpoint_request.as_ref(),
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

/// U108 阶段 3：按确认项决议把已审批的 patch 写回磁盘。
///
/// 这是「用户点了同意，正文却纹丝不动」的修复点。此前 `apply_confirmed_patch`
/// 生产零调用者，`resolve_confirmation_impl_with_claim` 只写知识库决议、提交运行态、
/// 取续跑租约，从不落盘。
///
/// 返回 `Ok(None)` 的四种情况都**不是**错误：
/// - 确认项不存在（并发解决过了）；
/// - 决议不是通过（拒绝路径必须一个字节都不写）；
/// - 确认项不带 patch（产出类确认项、summarizer 确认项）；
/// - patch 为空。
pub fn apply_approved_patch_for_confirmation(
    runtime: &mut WorkflowRuntime,
    documents: &FileDocumentService,
    git: Option<&GitService>,
    confirmation_id: &str,
    checkpoint_enabled: bool,
) -> CoreResult<Option<WorkflowPatchApplyOutcome>> {
    let Some(confirmation) = runtime.state.confirmations.get(confirmation_id) else {
        return Ok(None);
    };
    // 只有已通过/已自动审计才写盘。这是 U117 门禁语义的第一道防线；
    // `apply_confirmed_patch` 内部的 `ensure_patch_write_back_can_start`
    // 会再查一次（按 node_id + commit_id 配对），刻意冗余——
    // 「审批通过才落盘」是不可绕过点。
    if !matches!(
        confirmation.state,
        crate::workflow::RuntimeConfirmationState::Approved
            | crate::workflow::RuntimeConfirmationState::AutoAudited
    ) {
        return Ok(None);
    }
    let node_id = confirmation.node_id.clone();
    let Some(commit) = patch_commit_from_confirmation(&confirmation.metadata)? else {
        return Ok(None);
    };
    if commit.patch.is_empty() {
        return Ok(None);
    }
    let message = format!("writing patch approved via {confirmation_id}");
    apply_confirmed_patch_with_checkpoint(
        runtime,
        documents,
        git,
        &node_id,
        &commit.patch,
        Some(&message),
        checkpoint_enabled,
    )
    .map(Some)
}

/// 从确认项 metadata 取回 patch commit。
///
/// 键不存在返回 `Ok(None)`（该确认项本来就不带 patch）；
/// 键存在但解不出来则 **fail-loud**——那说明本该有 patch，
/// 静默跳过等于回到「同意了也不落盘」的缺陷状态。
fn patch_commit_from_confirmation(
    metadata: &Value,
) -> CoreResult<Option<crate::rag::line_patch::PatchSessionCommit>> {
    let Some(raw) = metadata.get(PATCH_SESSION_COMMIT_METADATA_KEY) else {
        return Ok(None);
    };
    serde_json::from_value(raw.clone())
        .map(Some)
        .map_err(|error| {
            CoreError::validation(format!(
                "confirmation carries an unreadable patch session commit: {error}"
            ))
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
    /// F13：画布/预设写入的节点超时（ms）；未设或 0 时回退到项目配置的
    /// `workflow.default_timeout_ms`（U113）。
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
    /// U114：本节点所写章节的 id，用于从写作知识库装配上下文
    /// （章节概括、人物与关系当前状态、未回收伏笔等）。
    ///
    /// 缺省时退化为「只用节点自己的 prompt_template」——与接线前行为一致，
    /// 不会让历史工作流因缺字段而失败。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    /// U108：写作节点要编辑的文档，相对项目文档根的路径（如 `chapter-01.md`）。
    ///
    /// 行号 patch 工具（`*-insert-lines` / `*-replace-lines`）需要正文原文才能
    /// 把行号换算成字节区间，因此节点必须显式指名文档；未指定时该节点只会拿到
    /// 只读工具（find/search/web-search），不会下发写入工具。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
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
    // U114/U115：把 Module 9 的上下文装配与提示词渲染接进生产路径。
    //
    // 接线前这里直接把 `prompt_template` 原文发给 LLM，于是：
    //   1. 模板里的 `{{上一章原文}}` 等占位符**以字面量**进入请求（U115）；
    //   2. 写作节点对这本小说一无所知——没有大纲、没有前文、没有人物设定（U114）；
    //   3. 行号工具连带不可用：LLM 看不到带行号正文，无从调用
    //      `writer-insert-lines(after_line=N)`，U108 只接工具是接不通的。
    //
    // 装配所需的数据在 U108 里已经全部到手：知识库（人物状态/章节概括/
    // 未回收伏笔由 assembler 自行派生）与节点指名文档的正文。
    let prompt = render_writing_node_prompt(&prompt, options.writing_tools.as_ref())?;

    // U147-c（2026-08-18）：把通信边送来的入站消息拼进 prompt。
    //
    // 这是**同一形态的第三个缺口**（前两个是 U147-a 的 `communication_output`
    // 无生产者、U114 的上下文装配零调用）：`communication_messages` 一路从
    // `collect_inbound_messages`（`runtime.rs:1424`）传进
    // `WorkflowNodeExecutionRequest`，而 `integration.rs` **零处消费它** ⇒
    // 轮转修好之后，critic 仍然读不到 writer 的原话，
    // 用户画的通信边看起来转了（消息计数在涨）却完全不起作用。
    //
    // 拼在**最后**而不是最前：作者的角色设定与本章材料是长期语境，
    // 对话方刚说的话是当下要回应的东西——放在末尾更接近「对话轮次」的语义，
    // 也避免长语境把这句挤出模型的注意范围。
    //
    // `content` 是 runtime 里 `render_communication_template` 按**用户在边上配的
    // 模板**渲染好的文本（含 `{{input.别名}}` 替换），所以这里只做拼接、
    // 不再二次加工——加工两次会让边上那份模板配置形同虚设。
    let prompt = match request.communication_messages.as_slice() {
        [] => prompt,
        messages => {
            let inbound = messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            format!("{prompt}\n\n{inbound}")
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

    let timeout_ms = resolve_node_timeout_ms(config.timeout_ms, &options.limits);
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

    // U108：写作工具也算「有工具」，否则装配了写作工具的节点仍走无工具快路径。
    let writing_tool_definitions = match options.writing_tools.as_ref() {
        Some(writing) => resolve_writing_tool_definitions(writing)?,
        None => Vec::new(),
    };
    if options.project_search.is_none()
        && options.web_search.is_none()
        && writing_tool_definitions.is_empty()
    {
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
        let mut output = llm_response_to_output(response)?;
        // 这条路径没下发任何工具，故无副作用证据；只补产出类确认项。
        attach_writing_confirmations(
            &mut output,
            options.writing_tools.as_ref(),
            &request.node_id,
            &request.operation_id,
            &[],
            0,
            None,
        )?;
        return Ok(output);
    }

    // U117：记录进入工具循环前的登记项基线。
    // 副作用确认项必须只反映**本次节点**造成的改动——知识库是跨节点复用的，
    // 拿全量登记项当证据会把别人早先登记的内容也算到本节点头上。
    let registered_baseline: BTreeSet<String> = match options.writing_tools.as_ref() {
        Some(writing) => writing
            .knowledge
            .registered_changes()?
            .into_iter()
            .map(|change| change.change_id)
            .collect(),
        None => BTreeSet::new(),
    };
    let mut patch_count = 0usize;

    let max_tool_rounds = options.limits.max_tool_rounds;
    if max_tool_rounds == 0 || max_tool_rounds > 32 {
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
    // U108：写作工具与检索工具共用同一个 router——`*-search` 归 ProjectSearchToolExecutor，
    // find/register/行号 patch 归 WritingToolExecutor，按工具名分流。
    let writing_tool_executor = options.writing_tools.as_ref().map(|writing| {
        let executor = match writing.document {
            Some(document) => {
                crate::rag::tools::WritingToolExecutor::with_document(writing.knowledge, document)
            }
            None => crate::rag::tools::WritingToolExecutor::new(writing.knowledge),
        };
        // U108 阶段 3：挂上 patch 会话，行号改动才会被累积下来等待审批。
        let executor = match writing.patch_session.clone() {
            Some(session) => executor.with_patch_session(session),
            None => executor,
        };
        match options.web_search.as_ref() {
            // 写作 agent 的 `*-web-search` 由写作执行器自己承接，沿用其
            // 「搜索结果不自动入库」语义。
            Some((search_provider, _, _)) => {
                executor.with_search_provider(*search_provider, base_context.clone())
            }
            None => executor,
        }
    });
    if let Some(tool_executor) = writing_tool_executor.as_ref() {
        for tool in &writing_tool_definitions {
            tool_router.register(tool.name.clone(), tool_executor)?;
        }
    }
    let tools = options
        .project_search
        .iter()
        .map(|(_, tool)| tool.clone())
        .chain(options.web_search.iter().map(|(_, _, tool)| tool.clone()))
        .chain(writing_tool_definitions.iter().cloned())
        .collect::<Vec<_>>();
    let tool_names = tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    for round in 0..=max_tool_rounds {
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
            let mut output = llm_response_to_output(response)?;
            // U108 阶段 3：取出本节点累积的 patch。它必须随确认项一起持久化——
            // executor 一出作用域，内存里的会话就没了，审批时将无物可写。
            let patch_commit = match writing_tool_executor.as_ref() {
                Some(executor) => executor.commit_patch_session()?,
                None => None,
            };
            // 只取基线之后新增的登记项——理由见 registered_baseline 处注释。
            let registered_new: Vec<String> = match options.writing_tools.as_ref() {
                Some(writing) => writing
                    .knowledge
                    .registered_changes()?
                    .into_iter()
                    .map(|change| change.change_id)
                    .filter(|id| !registered_baseline.contains(id))
                    .collect(),
                None => Vec::new(),
            };
            attach_writing_confirmations(
                &mut output,
                options.writing_tools.as_ref(),
                &request.node_id,
                &request.operation_id,
                &registered_new,
                patch_count,
                patch_commit.as_ref(),
            )?;
            return Ok(output);
        }
        if round >= max_tool_rounds {
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
            // U117：行号 patch 类工具的产出即副作用证据。
            // 以工具名判定而非解析返回值：返回值 schema 属工具内部约定，
            // 在这里解析会让两处实现悄悄漂移。
            if crate::rag::tools::WritingToolExecutor::is_line_patch_tool(&call.name) {
                patch_count += 1;
            }
            messages.push(tool_result_message(call, output));
        }
    }
    Err(CoreError::validation(
        "LLM node search tool loop ended unexpectedly",
    ))
}

/// U108：解析该写作节点最终下发给模型的工具定义。
///
/// 三层过滤，任一不满足即不下发：
/// 1. agent 本身声明了该工具（`tool_definitions_for_agent`）；
/// 2. 权限页的 tool_controls 允许（调用方预先算好放进 `allowed_tools`）；
/// 3. 写入类工具还要求节点已指名可编辑文档——否则模型会拿到一个必然失败的工具。
///
/// `*-search` 一律排除：它由 `ProjectSearchToolExecutor` 承接，
/// 是否下发取决于 `options.project_search` 是否装配，不走这里。
/// U114/U115：为写作节点装配小说上下文并渲染提示词模板。
///
/// 三条设计约束：
/// - **非写作节点原样返回**。普通 `llm` 节点没有 agent 身份，也没有知识库，
///   不能凭空给它装配上下文；`writing_tools` 为 `None` 即直接返回原 prompt，
///   行为与接线前完全一致。
/// - **不静默吞掉渲染失败**。`render_prompt_template` 对未知变量报错而非替换成
///   空串（其 doc 注释明确写了这一点）。但模板里**不含**任何 `{{}}` 时也走
///   渲染器是安全的——没有占位符就没有未知变量。因此只要模板含占位符且解析
///   不出来，就 fail-loud 让用户知道哪个变量拼错了，而不是把 `{{上一章原文}}`
///   当正文喂给模型。
/// - **上下文来源限于已在手的数据**。知识库（assembler 自行派生人物状态、
///   章节概括、未回收伏笔）与节点 `document_id` 指名的正文。跨章节的
///   「上一章原文」需要一套章节→文档的目录约定，尚未确立，故本次不猜测。
fn render_writing_node_prompt(
    prompt: &str,
    writing: Option<&WorkflowWritingToolOptions<'_>>,
) -> CoreResult<String> {
    let Some(writing) = writing else {
        return Ok(prompt.to_owned());
    };
    // 13-B：正文引用先展开，**必须在模板渲染之前**。
    //
    // 引用的真实来源是**上游 Planner 的输出**：它经 `resolve_llm_input_prompt`
    // 拼进这里收到的 `prompt`（格式 `{template}\n\n{input}`），
    // 而不是任何 context section——`WritingContextRequest` 的 `outline` / `details`
    // 等字段在生产里全是 `None`（见下方装配处），所以 13B 提案里
    // 「在 writer_sections 的 outline 段就地替换」那条路在当前实现下走不通。
    //
    // ⚠️ **顺序不能反**：`render_prompt_template` 对未知变量 fail-loud，
    // 而它不认识 `ref:` 前缀（`resolve_variable` 里没有这一支）⇒
    // 先渲染会让每一条引用都变成「变量无法解析」，节点直接失败。
    // 先展开则把 `{{ref:...}}` 换成原文，渲染器再也看不到它。
    //
    // ⚠️ 装配层的 `expand_section_references` 收口管的是 **context section**，
    // 与这里互补而非重复：那条防的是「大纲值里带引用」（将来上游数据边接通后会有），
    // 这条防的是「上游正文里带引用」（现在就有）。两处都必要。
    let prompt = expand_prompt_content_references(prompt, writing)?;
    let prompt = prompt.as_str();

    // 没有占位符的模板不必装配上下文，省一次知识库遍历。
    if !prompt.contains("{{") {
        return Ok(prompt.to_owned());
    }

    // 章节 id 是上下文装配的归属键（知识库按章查总结）。模板既然含占位符，
    // 缺 chapter_id 就无法装配——此时必须 fail-loud 指名该补什么，
    // 绝不能把 `{{上一章原文}}` 原样喂给模型。
    let chapter_id = writing
        .chapter_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CoreError::validation(
                "writing node prompt template contains {{...}} placeholders but the node has no \
             chapter_id; set chapter_id on the node, or remove the placeholders from the template",
            )
        })?;

    let prompts = crate::rag::resources::load_prompt_resources()?;
    let mut context_request = crate::rag::models::WritingContextRequest {
        agent: writing.agent,
        chapter_id: chapter_id.to_owned(),
        stage_id: None,
        user_intent: None,
        global_outline: None,
        stage_outline: None,
        previous_stage_outline: None,
        chapter_summaries: None,
        outline: None,
        details: None,
        previous_chapter_text: None,
        current_draft_text: None,
        target_text: None,
        critic_outputs: None,
        revision_context: None,
        template_inputs: BTreeMap::new(),
        metadata: json!({}),
    };
    // 带行号正文是行号 patch 工具的前提：LLM 必须先看到行号，
    // 才可能调用 `*-insert-lines(after_line=N)`。
    if let Some(document) = writing.document {
        context_request.current_draft_text = Some(document.text.to_owned());
    }
    // U175：把预读好的规划正文填进装配请求。
    //
    // 此前这里除 `current_draft_text` 外**全部写死 `None`**，而产品自带的
    // `node_template.*` 无条件引用 `{{本章大纲}}` `{{全局总纲}}` 等变量 ⇒
    // 区块不产出 ⇒ 别名不登记 ⇒ 渲染器 fail-loud ⇒ **拖一个写作节点上画布
    // 用预填提示词点运行必然失败**（U175）。
    //
    // 只填路径映射无歧义的那两项，理由见 `WorkflowWritingPlanningContext` 的
    // doc 注释；其余仍由装配层给空态文本，节点可运行但明确告知模型缺什么。
    if let Some(planning) = writing.planning.as_ref() {
        context_request.global_outline = planning.global_outline.clone();
        context_request.outline = planning.outline.clone();
    }

    // 13-B：挂上正文引用来源，让大纲里的 `{{ref:...}}` 就地展开成原文。
    //
    // 没挂来源时装配器对含引用的区块 **fail-loud**（不是静默放过）——
    // `{{ref:...}}` 字面量进 LLM 请求体是安全缺口：Auto Mode 的审计 LLM
    // 会在「审的是占位符」的前提下给出虚假通过。
    // 但**没有引用**的项目完全不受影响：`expand_section_references` 先用
    // `contains_content_reference` 便宜地筛一遍，不含引用的区块直接跳过。
    let assembler = crate::rag::context::WritingContextAssembler::new(writing.knowledge);
    let assembler = match writing.reference_documents.as_ref() {
        Some(documents) => assembler.with_reference_documents(documents),
        None => assembler,
    };
    let bundle = assembler.assemble(context_request)?;
    let context = crate::rag::prompt_template::PromptTemplateContext::from_bundle(
        writing.agent,
        &prompts,
        &bundle,
    )?;
    crate::rag::prompt_template::render_prompt_template(prompt, &context)
}

/// 13-B：把 prompt 里的正文引用 `{{ref:文档ID#L起始-L结束}}` 展开成原文。
///
/// **没有引用就原样返回**（`contains_content_reference` 是一次便宜的 substring 检查），
/// 所以不写引用的项目完全不受影响、也不会因为没挂来源而失败。
///
/// **有引用但没挂来源 ⇒ fail-loud。** 三个候选做法里只有这个是对的：
/// - 原样放过 → `{{ref:...}}` 进 LLM 请求体，随后 `render_prompt_template`
///   会把它当未知变量报错（信息含混：用户以为是自己拼错了变量名），
///   更糟的是若将来渲染器放宽，占位符会直接进请求体，
///   而 Auto Mode 的审计 LLM 会在「审的是占位符」的前提下给出虚假通过；
/// - 静默删掉 → 写作者以为那条指示后面本来就没有原文，人也看不出少了什么；
/// - fail-loud → 报错点名缺什么，用户当场知道该配什么。
fn expand_prompt_content_references(
    prompt: &str,
    writing: &WorkflowWritingToolOptions<'_>,
) -> CoreResult<String> {
    if !crate::rag::reference::contains_content_reference(prompt) {
        return Ok(prompt.to_owned());
    }

    let Some(documents) = writing.reference_documents.as_ref() else {
        return Err(CoreError::validation(
            "writing node prompt contains {{ref:...}} content references but no reference \
             document source is available; the upstream node must be re-run so the referenced \
             chapters get preloaded, or remove the references from the outline",
        ));
    };

    let expansion = crate::rag::reference::expand_content_references(
        prompt,
        documents,
        &crate::rag::reference::ReferenceExpansionLimits::default(),
    )?;

    // 展开后置条件：不能再有 `{{ref:` 残留。
    //
    // 展开器内部有若干「无法展开」的分支（文档缺失、越权、超条数、嵌套），
    // 每一条都必须把占位符换成可诊断标记；哪天有人新加一条分支忘了替换，
    // 占位符就会一路进到请求体。在这里 fail-loud 比在生产里静默批准好得多。
    if crate::rag::reference::contains_content_reference(&expansion.text) {
        return Err(CoreError::validation(
            "writing node prompt still contains {{ref:...}} after expansion; refusing to hand \
             unexpanded content references to a model",
        ));
    }

    Ok(expansion.text)
}

/// U117：把确认项挂到节点输出上；非写作节点原样放过。
///
/// 普通 `llm` 节点没有 agent 身份，凭空给它造确认项会把每个模型节点都变成
/// 待审状态——所以 `writing` 为 `None` 时必须什么都不做，行为与接线前一致。
fn attach_writing_confirmations(
    output: &mut WorkflowNodeExecutionOutput,
    writing: Option<&WorkflowWritingToolOptions<'_>>,
    node_id: &NodeId,
    revision_id: &str,
    registered_change_ids: &[String],
    patch_count: usize,
    patch_commit: Option<&crate::rag::line_patch::PatchSessionCommit>,
) -> CoreResult<()> {
    let Some(writing) = writing else {
        return Ok(());
    };
    let items =
        writing_node_confirmations(writing, revision_id, registered_change_ids, patch_count)?;
    let commit_id = patch_commit.map(patch_session_commit_id);
    // U108 阶段 3：节点自身也要带 commit id。`ensure_patch_write_back_can_start`
    // 读的是**节点**上的这个字段；不设的话 `record_node_output` 走不进
    // `PendingConfirmation` 分支，门禁形同虚设。
    output.patch_session_commit_id = commit_id.clone();
    output.confirmations = items
        .into_iter()
        .map(|item| {
            // patch 只挂在正文改动类确认项上。同节点的产出类确认项（如
            // `PlannerOutput`）不该带 patch——`ensure_patch_confirmation_allows_apply`
            // 按 (node_id, commit_id) 配对，多条确认项共用一个 commit 会让
            // 任意一条的决议都能左右落盘。
            let carries_patch = matches!(
                item.kind,
                crate::rag::models::ConfirmationKind::WriterCorrectionPatch
                    | crate::rag::models::ConfirmationKind::PolisherCorrectionPatch
            );
            let (patch_session_commit_id, metadata) = match (carries_patch, patch_commit) {
                (true, Some(commit)) => (
                    commit_id.clone(),
                    merge_patch_commit_metadata(item.metadata, commit)?,
                ),
                _ => (None, item.metadata),
            };
            Ok(crate::workflow::RuntimeConfirmation {
                confirmation_id: item.confirmation_id,
                node_id: node_id.clone(),
                state: match item.state {
                    crate::rag::models::ConfirmationState::Pending => {
                        crate::workflow::RuntimeConfirmationState::Pending
                    }
                    crate::rag::models::ConfirmationState::Approved => {
                        crate::workflow::RuntimeConfirmationState::Approved
                    }
                    crate::rag::models::ConfirmationState::Rejected => {
                        crate::workflow::RuntimeConfirmationState::Rejected
                    }
                    crate::rag::models::ConfirmationState::Skipped
                    | crate::rag::models::ConfirmationState::AutoAudited => {
                        crate::workflow::RuntimeConfirmationState::AutoAudited
                    }
                },
                artifact_id: None,
                patch_session_commit_id,
                metadata,
            })
        })
        .collect::<CoreResult<Vec<_>>>()?;
    Ok(())
}

/// U108 阶段 3：patch 会话提交 id。
///
/// 用内容 hash 而非随机数：同一节点重放产生相同 id（幂等），
/// 而内容不同必然 id 不同——避免门禁把两次不同的 patch 当成同一个。
fn patch_session_commit_id(commit: &crate::rag::line_patch::PatchSessionCommit) -> String {
    format!(
        "patch-{}-{}",
        commit.base_content_hash, commit.final_content_hash
    )
}

/// U108 阶段 3：把 patch commit 塞进确认项 metadata。
///
/// 存这里而不是另开一张表：metadata 已随运行态快照落在**同一个事务**里，
/// 不会出现「运行态说已审批、patch 却不见了」的跨存储不一致。
/// 体积可控——`commit()` 已把全文压成最小 hunk，存的是改动不是全文。
fn merge_patch_commit_metadata(
    metadata: Value,
    commit: &crate::rag::line_patch::PatchSessionCommit,
) -> CoreResult<Value> {
    let mut metadata = match metadata {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    metadata.insert(
        PATCH_SESSION_COMMIT_METADATA_KEY.to_owned(),
        serde_json::to_value(commit)?,
    );
    Ok(Value::Object(metadata))
}

/// 确认项 metadata 里存放 patch commit 的键名。
pub(crate) const PATCH_SESSION_COMMIT_METADATA_KEY: &str = "patch_session_commit";

/// U117：为写作节点的产出补上确认项，使写作结果进入审批门禁。
///
/// 修复前这里返回的 `confirmations` 恒为空，而 `PatchWriteBackState` 正是靠
/// 「有无 Pending 确认项」决定是否拦住落盘（`NotRequested` → 直接写入）。
/// 于是 8 种写作类确认项从不产出，**写作产出不经审阅直接落地**。
///
/// 两类确认项，来源不同：
/// - **产出类**（outliner/designer/planner/critic/prudent 的 `*Output` / `*Review`）：
///   只要节点跑完就该产出，与是否调用工具无关；
/// - **副作用类**（`PlannerRegister` / `WriterCorrectionPatch` /
///   `PolisherCorrectionPatch`）：只在**确有副作用**时产出，否则会凭空造出
///   一条永远待审的空确认项，把工作流永久卡在 `PendingConfirmation`。
///   故以可观测证据为准——register 看知识库新增的登记项，patch 看本轮产生的补丁数。
fn writing_node_confirmations(
    writing: &WorkflowWritingToolOptions<'_>,
    revision_id: &str,
    registered_change_ids: &[String],
    patch_count: usize,
) -> CoreResult<Vec<crate::rag::models::ConfirmationItem>> {
    use crate::rag::models::ConfirmationKind;

    // 章节 id 是确认项的归属键。写作节点未声明时退化为 agent 名，
    // 以免因缺一个可选字段就完全不产出确认项——那等于回到缺陷状态。
    let chapter_id = writing
        .chapter_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| writing.agent.node_type());

    let mut items = Vec::new();
    for kind in crate::rag::models::WritingNodeDefinition::confirmation_kinds_for(writing.agent) {
        // 副作用类：无证据则不产出（理由见函数文档）。
        let metadata = match kind {
            ConfirmationKind::PlannerRegister => {
                if registered_change_ids.is_empty() {
                    continue;
                }
                json!({ "registered_change_ids": registered_change_ids })
            }
            ConfirmationKind::WriterCorrectionPatch | ConfirmationKind::PolisherCorrectionPatch => {
                if patch_count == 0 {
                    continue;
                }
                json!({ "patch_count": patch_count })
            }
            _ => json!({ "agent": writing.agent.node_type() }),
        };
        items.push(crate::rag::pipeline::build_writing_confirmation(
            kind,
            chapter_id,
            revision_id,
            metadata,
            &writing.confirmation_policy,
            &writing.auto_mode,
            None,
        )?);
    }
    Ok(items)
}

fn resolve_writing_tool_definitions(
    writing: &WorkflowWritingToolOptions<'_>,
) -> CoreResult<Vec<ToolDefinition>> {
    if writing.allowed_tools.is_empty() {
        return Ok(Vec::new());
    }
    let prompts = crate::rag::resources::load_prompt_resources()?;
    let definitions = crate::rag::tools::tool_definitions_for_agent(writing.agent, &prompts)?;
    Ok(definitions
        .into_iter()
        .filter(|tool| crate::rag::tools::WritingToolExecutor::handles_tool(&tool.name))
        .filter(|tool| writing.allowed_tools.contains(&tool.name))
        .filter(|tool| {
            writing.document.is_some()
                || !crate::rag::tools::WritingToolExecutor::is_line_patch_tool(&tool.name)
        })
        .collect())
}

/// F13/U113：节点未声明超时时回落到项目配置的 `default_timeout_ms`，
/// 不再硬编码 120s——否则设置页展示的默认值与运行时真实行为长期不一致。
fn resolve_node_timeout_ms(timeout_ms: Option<u64>, limits: &WorkflowExecutionLimits) -> u64 {
    limits.resolve_node_timeout_ms(timeout_ms)
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
/// 校验工作流的执行契约。
///
/// U113：全局限制是执行契约的一部分，因此必须走同一个入口。过去 `max_loop_iterations`
/// 只存在于零调用者的 `validate_loop_policy` 里，导致越界的 loop 节点在任何路径上
/// 都不会被拒绝。现在限制随图结构一起校验，新增调用点不可能「忘记」带上它。
pub fn validate_workflow_execution_contracts(
    workflow: &WorkflowDefinition,
    limits: &WorkflowExecutionLimits,
) -> CoreResult<()> {
    limits.validate()?;
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
                // U125：condition 的控制出边必须从两个分支引脚之一拉出，且同一引脚
                // 不得重复连出。旧实现用 `.filter(|edge| edge.alias.is_some())`
                // 只校验「已填标签」的边——留空的边被整体跳过，而留空正是最常见的
                // 画法（用户无从知道要填 `true`），于是 condition 恒放行下游。
                let mut used_branch_ports = BTreeSet::new();
                for edge in workflow.edges.iter().filter(|edge| {
                    edge.kind == WorkflowEdgeKind::Control && edge.from.node_id == node.id
                }) {
                    let port_name = edge.from.port_name.trim();
                    if condition_branch_for_port(port_name).is_none() {
                        return Err(CoreError::validation(format!(
                            "{} node {} control out edge {} must leave from {} or {}, got {}",
                            node.type_name,
                            node.id.as_str(),
                            edge.id.as_str(),
                            EXECUTION_OUTPUT_PORT_TRUE,
                            EXECUTION_OUTPUT_PORT_FALSE,
                            port_name
                        )));
                    }
                    if !used_branch_ports.insert(port_name.to_owned()) {
                        return Err(CoreError::validation(format!(
                            "{} node {} has more than one control edge on branch port {}",
                            node.type_name,
                            node.id.as_str(),
                            port_name
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
                // U113：loop 节点必须同时满足自身合法性与项目全局上限，
                // 否则用户在设置页收紧的轮次/超时护栏对真实运行没有约束力。
                LoopPolicy {
                    max_iterations: config.max_iterations,
                    timeout_ms: config.timeout_ms,
                    budget_limit_usd: config.budget_limit_usd,
                    stop_condition: config.stop_condition.clone(),
                }
                .validate_within(limits)
                .map_err(|error| {
                    CoreError::validation(format!(
                        "loop node {} violates workflow limits: {error}",
                        node.id.as_str()
                    ))
                })?;
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
    validate_branch_ports_limited_to_condition_nodes(workflow)?;
    Ok(())
}

/// U125：分支引脚是 condition/eval 专属。
///
/// `validate_edge_kind`（`contracts/workflow.rs`）为了放开 condition 的两个分支引脚，
/// 把控制边源引脚从「必须是 `exec_out`」松成「三者之一」。那一层拿不到节点类型，
/// 所以「普通节点不得使用分支引脚」这条只能在这里补上——否则放开校验的同时
/// 也放开了「任意节点挂 `exec_out_true` 却永远没有 `branch` 输出」这种画法，
/// 其下游会永久停在 `Waiting`，表现为工作流静默卡住。
fn validate_branch_ports_limited_to_condition_nodes(
    workflow: &WorkflowDefinition,
) -> CoreResult<()> {
    let condition_node_ids = workflow
        .nodes
        .iter()
        .filter(|node| matches!(node.type_name.as_str(), "condition" | "eval"))
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    for edge in &workflow.edges {
        if edge.kind != WorkflowEdgeKind::Control {
            continue;
        }
        if condition_branch_for_port(edge.from.port_name.trim()).is_none() {
            continue;
        }
        if !condition_node_ids.contains(&edge.from.node_id) {
            return Err(CoreError::validation(format!(
                "control edge {} leaves from branch port {} but source node {} is not a condition node",
                edge.id.as_str(),
                edge.from.port_name.trim(),
                edge.from.node_id.as_str()
            )));
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

/// F17 / U117：把项目保存的双模式策略解析为写作领域确认策略。
/// 未出现的键保留领域默认值；文件存在但无法读取/解析时必须在 provider dispatch 前失败。
///
/// U117：原实现只认四种总结类确认项（其余 `_ => continue` 丢弃），于是设置页里
/// 「章节大纲确认」「伏笔注册确认」「润色修正文档确认」等 8 项配了也不生效——
/// 用户在为**永远不会发生的事件**配置策略。现改为覆盖全部 12 种。
pub(crate) fn load_writing_confirmation_policy(
    project_root: &Path,
    auto_mode: bool,
) -> CoreResult<(
    crate::rag::models::WritingConfirmationPolicy,
    BTreeMap<crate::rag::models::ConfirmationKind, String>,
)> {
    use crate::config::{ConfirmationAutoModePolicy, ConfirmationNormalPolicy};
    use crate::rag::models::{
        confirmation_kind_from_policy_key, confirmation_prompt_key, ConfirmationKind,
        ConfirmationMode, WritingConfirmationPolicy,
    };

    let mut policy = if auto_mode {
        WritingConfirmationPolicy::auto_audit_default()
    } else {
        WritingConfirmationPolicy::normal_default()
    };
    let resources = crate::rag::resources::load_prompt_resources()?;
    let mut approval_prompts = BTreeMap::new();
    // U117：为**全部** 12 种确认项预载审计 prompt。缺任何一条都会让 Auto Mode
    // 在真正需要审计时才炸——那时节点已经花掉了 LLM 调用的钱。
    for kind in ConfirmationKind::ALL {
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
        let Some(kind) = confirmation_kind_from_policy_key(&setting.confirmation_kind) else {
            continue;
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
    limits: &WorkflowExecutionLimits,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_summarizer_node_with_optional_search_tools(
        request,
        provider,
        ledger,
        project_root,
        None,
        None,
        limits,
    )
}

pub fn execute_summarizer_node_with_project_search<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    project_root: &Path,
    retrieval: &ProjectRetrievalRuntime,
    search_tool: ToolDefinition,
    limits: &WorkflowExecutionLimits,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_summarizer_node_with_optional_search_tools(
        request,
        provider,
        ledger,
        project_root,
        Some((retrieval, search_tool)),
        None,
        limits,
    )
}

pub fn execute_summarizer_node_with_search_tools<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    project_root: &Path,
    project_search: Option<(&ProjectRetrievalRuntime, ToolDefinition)>,
    web_search: Option<(&dyn SearchProvider, &PermissionPolicy, ToolDefinition)>,
    limits: &WorkflowExecutionLimits,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    execute_summarizer_node_with_optional_search_tools(
        request,
        provider,
        ledger,
        project_root,
        project_search,
        web_search,
        limits,
    )
}

fn execute_summarizer_node_with_optional_search_tools<L: CostLedger>(
    request: WorkflowNodeExecutionRequest,
    provider: &dyn LlmProvider,
    ledger: &L,
    project_root: &Path,
    project_search: Option<(&ProjectRetrievalRuntime, ToolDefinition)>,
    web_search: Option<(&dyn SearchProvider, &PermissionPolicy, ToolDefinition)>,
    limits: &WorkflowExecutionLimits,
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
        load_writing_confirmation_policy(project_root, config.auto_mode)?;

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
    let timeout_ms = resolve_node_timeout_ms(config.timeout_ms, limits);
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
            summarizer.with_project_search(retrieval, search_tool, limits.max_tool_rounds)
        }
        None => summarizer,
    };
    let summarizer = match web_search {
        Some((search_provider, permission_policy, search_tool)) => summarizer.with_web_search(
            search_provider,
            permission_policy,
            search_tool,
            limits.max_tool_rounds,
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

// U116：原有 `WorkflowSearchNodeConfig` + `execute_search_node`（对接外部 SearchProvider）
// 已删除——两者互相引用、外界零调用，是个闭合的死簇。
//
// 留着有害而不只是冗余：节点目录里只有**一个** `search` 节点类型，而它的配置形状是
// 下面的 `WorkflowProjectSearchNodeConfig`（项目内 RAG 检索 + 新鲜度门禁）。
// 生产注册的执行器是 `execute_project_retrieval_node_for_project`（`commands.rs:6665`）。
// 保留一套「配置字段对不上唯一可用节点类型」的旧实现，只会让接线者选错。

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

/// 执行文档读取节点，并把相对路径锚定到指定工作目录。
///
/// U116：曾有一个不带 root 的 `execute_document_read_node` 薄包装（root 传 `None`
/// = **不设读取边界**），生产从不走它，已删。新调用方一律用本函数并显式给出
/// `work_root`——「忘记传边界」不该是一个能通过编译的选项。
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
