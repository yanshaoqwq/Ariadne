use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;

use crate::contracts::{CoreError, CoreResult};
use crate::llm::{ToolExecutionContext, ToolExecutionOutput, ToolExecutor};
pub use crate::node_capabilities::{
    CRITIC_SEARCH_TOOL as TOOL_CRITIC_SEARCH, CRITIC_WEB_SEARCH_TOOL as TOOL_CRITIC_WEB_SEARCH,
    DESIGNER_SEARCH_TOOL as TOOL_DESIGNER_SEARCH,
    DESIGNER_WEB_SEARCH_TOOL as TOOL_DESIGNER_WEB_SEARCH, DETAIL_SEARCH_TOOL as TOOL_DETAIL_SEARCH,
    DETAIL_WEB_SEARCH_TOOL as TOOL_DETAIL_WEB_SEARCH, OUTLINER_SEARCH_TOOL as TOOL_OUTLINER_SEARCH,
    OUTLINER_WEB_SEARCH_TOOL as TOOL_OUTLINER_WEB_SEARCH,
    PLANNER_SEARCH_TOOL as TOOL_PLANNER_SEARCH, PLANNER_WEB_SEARCH_TOOL as TOOL_PLANNER_WEB_SEARCH,
    POLISHER_SEARCH_TOOL as TOOL_POLISHER_SEARCH,
    POLISHER_WEB_SEARCH_TOOL as TOOL_POLISHER_WEB_SEARCH,
    PRUDENT_SEARCH_TOOL as TOOL_PRUDENT_SEARCH, PRUDENT_WEB_SEARCH_TOOL as TOOL_PRUDENT_WEB_SEARCH,
    SUMMARIZER_SEARCH_TOOL as TOOL_SUMMARIZER_SEARCH,
    SUMMARIZER_WEB_SEARCH_TOOL as TOOL_SUMMARIZER_WEB_SEARCH,
    WRITER_SEARCH_TOOL as TOOL_WRITER_SEARCH, WRITER_WEB_SEARCH_TOOL as TOOL_WRITER_WEB_SEARCH,
};
use crate::providers::{
    ProviderCallContext, SearchProvider, SearchProviderRequest, SearchProviderResponse, ToolCall,
    ToolDefinition,
};
use crate::rag::line_patch::{
    insert_lines_to_patch, replace_lines_to_patch, rewrite_file_to_patch, PatchSession,
    PatchSessionCommit, WriterInsertLines, WriterReplaceLines,
};
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    FindRequest, FindResult, FindScope, RegisterContent, RegisterFunction, RegisterOperation,
    WritingAgentKind, WritingSearchResponse,
};
use crate::rag::resources::PromptResources;

pub const TOOL_OUTLINER_REGISTER: &str = "outliner-register";
pub const TOOL_OUTLINER_FIND: &str = "outliner-find";
pub const TOOL_OUTLINER_INSERT_LINES: &str = "outliner-insert-lines";
pub const TOOL_OUTLINER_REPLACE_LINES: &str = "outliner-replace-lines";
pub const TOOL_OUTLINER_REWRITE_FILE: &str = "outliner-rewrite-file";
pub const TOOL_DESIGNER_REGISTER: &str = "designer-register";
pub const TOOL_DESIGNER_FIND: &str = "designer-find";
pub const TOOL_DESIGNER_INSERT_LINES: &str = "designer-insert-lines";
pub const TOOL_DESIGNER_REPLACE_LINES: &str = "designer-replace-lines";
pub const TOOL_DESIGNER_REWRITE_FILE: &str = "designer-rewrite-file";
pub const TOOL_PLANNER_REGISTER: &str = "planner-register";
pub const TOOL_PLANNER_FIND: &str = "planner-find";
pub const TOOL_PLANNER_INSERT_LINES: &str = "planner-insert-lines";
pub const TOOL_PLANNER_REPLACE_LINES: &str = "planner-replace-lines";
pub const TOOL_PLANNER_REWRITE_FILE: &str = "planner-rewrite-file";
pub const TOOL_DETAIL_FIND: &str = "detail-find";
pub const TOOL_WRITER_FIND: &str = "writer-find";
pub const TOOL_WRITER_INSERT_LINES: &str = "writer-insert-lines";
pub const TOOL_WRITER_REPLACE_LINES: &str = "writer-replace-lines";
pub const TOOL_WRITER_REWRITE_FILE: &str = "writer-rewrite-file";
pub const TOOL_CRITIC_FIND: &str = "critic-find";
pub const TOOL_PRUDENT_FIND: &str = "prudent-find";
pub const TOOL_POLISHER_FIND: &str = "polisher-find";
pub const TOOL_POLISHER_INSERT_LINES: &str = "polisher-insert-lines";
pub const TOOL_POLISHER_REPLACE_LINES: &str = "polisher-replace-lines";
pub const TOOL_POLISHER_REWRITE_FILE: &str = "polisher-rewrite-file";

/// 为指定写作 agent 生成工具定义，描述文本来自 prompt_list.json。
pub fn tool_definitions_for_agent(
    agent: WritingAgentKind,
    prompts: &PromptResources,
) -> CoreResult<Vec<ToolDefinition>> {
    match agent {
        WritingAgentKind::Outliner => Ok(vec![
            tool_definition(
                TOOL_OUTLINER_REGISTER,
                "tool.outliner_register",
                prompts,
                planner_register_schema(),
            )?,
            tool_definition(
                TOOL_OUTLINER_FIND,
                "tool.outliner_find",
                prompts,
                find_schema(),
            )?,
            tool_definition(
                TOOL_OUTLINER_SEARCH,
                "tool.outliner_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_OUTLINER_WEB_SEARCH,
                "tool.outliner_web_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_OUTLINER_INSERT_LINES,
                "tool.outliner_insert_lines",
                prompts,
                writer_insert_schema(),
            )?,
            tool_definition(
                TOOL_OUTLINER_REPLACE_LINES,
                "tool.outliner_replace_lines",
                prompts,
                writer_replace_schema(),
            )?,
            tool_definition(
                TOOL_OUTLINER_REWRITE_FILE,
                "tool.outliner_rewrite_file",
                prompts,
                rewrite_file_schema(),
            )?,
        ]),
        WritingAgentKind::Designer => Ok(vec![
            tool_definition(
                TOOL_DESIGNER_REGISTER,
                "tool.designer_register",
                prompts,
                planner_register_schema(),
            )?,
            tool_definition(
                TOOL_DESIGNER_FIND,
                "tool.designer_find",
                prompts,
                find_schema(),
            )?,
            tool_definition(
                TOOL_DESIGNER_SEARCH,
                "tool.designer_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_DESIGNER_WEB_SEARCH,
                "tool.designer_web_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_DESIGNER_INSERT_LINES,
                "tool.designer_insert_lines",
                prompts,
                writer_insert_schema(),
            )?,
            tool_definition(
                TOOL_DESIGNER_REPLACE_LINES,
                "tool.designer_replace_lines",
                prompts,
                writer_replace_schema(),
            )?,
            tool_definition(
                TOOL_DESIGNER_REWRITE_FILE,
                "tool.designer_rewrite_file",
                prompts,
                rewrite_file_schema(),
            )?,
        ]),
        WritingAgentKind::Planner => Ok(vec![
            tool_definition(
                TOOL_PLANNER_REGISTER,
                "tool.planner_register",
                prompts,
                planner_register_schema(),
            )?,
            tool_definition(
                TOOL_PLANNER_FIND,
                "tool.planner_find",
                prompts,
                find_schema(),
            )?,
            tool_definition(
                TOOL_PLANNER_SEARCH,
                "tool.planner_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_PLANNER_WEB_SEARCH,
                "tool.planner_web_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_PLANNER_INSERT_LINES,
                "tool.planner_insert_lines",
                prompts,
                writer_insert_schema(),
            )?,
            tool_definition(
                TOOL_PLANNER_REPLACE_LINES,
                "tool.planner_replace_lines",
                prompts,
                writer_replace_schema(),
            )?,
            tool_definition(
                TOOL_PLANNER_REWRITE_FILE,
                "tool.planner_rewrite_file",
                prompts,
                rewrite_file_schema(),
            )?,
        ]),
        WritingAgentKind::Detail => Ok(vec![
            tool_definition(TOOL_DETAIL_FIND, "tool.detail_find", prompts, find_schema())?,
            tool_definition(
                TOOL_DETAIL_SEARCH,
                "tool.detail_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_DETAIL_WEB_SEARCH,
                "tool.detail_web_search",
                prompts,
                search_schema(),
            )?,
        ]),
        WritingAgentKind::Writer => Ok(vec![
            tool_definition(TOOL_WRITER_FIND, "tool.writer_find", prompts, find_schema())?,
            tool_definition(
                TOOL_WRITER_SEARCH,
                "tool.writer_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_WRITER_WEB_SEARCH,
                "tool.writer_web_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_WRITER_INSERT_LINES,
                "tool.writer_insert_lines",
                prompts,
                writer_insert_schema(),
            )?,
            tool_definition(
                TOOL_WRITER_REPLACE_LINES,
                "tool.writer_replace_lines",
                prompts,
                writer_replace_schema(),
            )?,
            tool_definition(
                TOOL_WRITER_REWRITE_FILE,
                "tool.writer_rewrite_file",
                prompts,
                rewrite_file_schema(),
            )?,
        ]),
        WritingAgentKind::Critic => Ok(vec![
            tool_definition(TOOL_CRITIC_FIND, "tool.critic_find", prompts, find_schema())?,
            tool_definition(
                TOOL_CRITIC_SEARCH,
                "tool.critic_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_CRITIC_WEB_SEARCH,
                "tool.critic_web_search",
                prompts,
                search_schema(),
            )?,
        ]),
        WritingAgentKind::Prudent => Ok(vec![
            tool_definition(
                TOOL_PRUDENT_FIND,
                "tool.prudent_find",
                prompts,
                find_schema(),
            )?,
            tool_definition(
                TOOL_PRUDENT_SEARCH,
                "tool.prudent_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_PRUDENT_WEB_SEARCH,
                "tool.prudent_web_search",
                prompts,
                search_schema(),
            )?,
        ]),
        WritingAgentKind::Polisher => Ok(vec![
            tool_definition(
                TOOL_POLISHER_FIND,
                "tool.polisher_find",
                prompts,
                find_schema(),
            )?,
            tool_definition(
                TOOL_POLISHER_SEARCH,
                "tool.polisher_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_POLISHER_WEB_SEARCH,
                "tool.polisher_web_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_POLISHER_INSERT_LINES,
                "tool.polisher_insert_lines",
                prompts,
                writer_insert_schema(),
            )?,
            tool_definition(
                TOOL_POLISHER_REPLACE_LINES,
                "tool.polisher_replace_lines",
                prompts,
                writer_replace_schema(),
            )?,
            tool_definition(
                TOOL_POLISHER_REWRITE_FILE,
                "tool.polisher_rewrite_file",
                prompts,
                rewrite_file_schema(),
            )?,
        ]),
        WritingAgentKind::Summarizer => Ok(vec![
            tool_definition(
                TOOL_SUMMARIZER_SEARCH,
                "tool.summarizer_search",
                prompts,
                search_schema(),
            )?,
            tool_definition(
                TOOL_SUMMARIZER_WEB_SEARCH,
                "tool.summarizer_web_search",
                prompts,
                search_schema(),
            )?,
        ]),
    }
}

/// Module 9 的写作工具执行器，承接本地 find/register 和 Writer 行号 patch。
pub struct WritingToolExecutor<'a> {
    knowledge: &'a MemoryWritingKnowledgeBase,
    current_document: Option<WriterDocumentContext<'a>>,
    search_provider: Option<&'a dyn SearchProvider>,
    search_context: Option<ProviderCallContext>,
    /// U108 阶段 3：本节点内累积的行号 patch 会话。
    ///
    /// **为什么必须累积而不是每次独立成 patch**：同一节点内第 2 次插入的行号，
    /// 是模型对着「第 1 次插入之后」的正文数出来的。各自独立算 patch 会让第 2 次
    /// 插错位置，且症状随插入点变化，事后极难定位。`PatchSession.simulated`
    /// 就是为此存在的。
    ///
    /// **为什么是 `Mutex` 而不是 `RefCell`**：`ToolExecutor` 要求 `Send + Sync`
    /// 且 `execute` 只收 `&self`（`ToolExecutorRouter` 存的是 `&dyn ToolExecutor`
    /// 共享引用，且同一个 executor 会注册到多个工具名下）。`RefCell` 不是 `Sync`，
    /// 挂上去会让本类型不再实现 `ToolExecutor`。
    ///
    /// 为 `None` 时行号工具行为与接线前完全一致（只返回 patch，不累积、不落盘），
    /// 因此既有调用方无需改动。
    patch_session: Option<Mutex<PatchSession>>,
}

/// 行号 patch 工具的文档作用域；用于约束“每类节点只能修改自己负责的文件”。
///
/// 对应 `创作总结机制(不可删除).md`：
/// - Outliner -> 全局总纲 `planning/global.md`
/// - Designer -> 阶段总纲 `planning/stages/*.md`
/// - Planner  -> 章节大纲 `planning/chapters/*.md`
/// - Writer/Polisher -> 章节正文
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingDocumentScope {
    /// 全局总纲（Outliner，对应 global outline）。
    GlobalOutline,
    /// 阶段总纲（Designer，对应 stage outline）。
    StageOutline,
    /// 章节大纲（Planner，对应 chapter outline）。
    ChapterOutline,
    /// 章节正文（Writer/Polisher，对应 chapter body）。
    ChapterBody,
}

impl WritingDocumentScope {
    /// 返回该作用域的显示用英文名，便于错误信息和审计。
    fn label(self) -> &'static str {
        match self {
            // global outline
            Self::GlobalOutline => "global_outline",
            // stage outline
            Self::StageOutline => "stage_outline",
            // chapter outline
            Self::ChapterOutline => "chapter_outline",
            // chapter body
            Self::ChapterBody => "chapter_body",
        }
    }
}

/// 返回行号 patch 工具被允许写入的文档作用域；非行号工具返回 None。
fn line_tool_required_scope(tool_name: &str) -> Option<WritingDocumentScope> {
    match tool_name {
        TOOL_OUTLINER_INSERT_LINES
        | TOOL_OUTLINER_REPLACE_LINES
        | TOOL_OUTLINER_REWRITE_FILE => Some(WritingDocumentScope::GlobalOutline),
        TOOL_DESIGNER_INSERT_LINES
        | TOOL_DESIGNER_REPLACE_LINES
        | TOOL_DESIGNER_REWRITE_FILE => Some(WritingDocumentScope::StageOutline),
        TOOL_PLANNER_INSERT_LINES | TOOL_PLANNER_REPLACE_LINES | TOOL_PLANNER_REWRITE_FILE => {
            Some(WritingDocumentScope::ChapterOutline)
        }
        TOOL_WRITER_INSERT_LINES
        | TOOL_WRITER_REPLACE_LINES
        | TOOL_WRITER_REWRITE_FILE
        | TOOL_POLISHER_INSERT_LINES
        | TOOL_POLISHER_REPLACE_LINES
        | TOOL_POLISHER_REWRITE_FILE => Some(WritingDocumentScope::ChapterBody),
        _ => None,
    }
}

/// Writer 当前可编辑正文上下文。
#[derive(Debug, Clone, Copy)]
pub struct WriterDocumentContext<'a> {
    pub document_id: &'a str,
    pub base_version: Option<&'a str>,
    pub text: &'a str,
    /// 当前文档所属作用域；行号 patch 工具据此校验节点写作边界。
    pub scope: WritingDocumentScope,
}

impl<'a> WritingToolExecutor<'a> {
    /// 创建只支持本地知识工具的执行器。
    pub fn new(knowledge: &'a MemoryWritingKnowledgeBase) -> Self {
        Self {
            knowledge,
            current_document: None,
            search_provider: None,
            search_context: None,
            patch_session: None,
        }
    }

    /// 创建带 Writer 正文上下文的执行器。
    pub fn with_document(
        knowledge: &'a MemoryWritingKnowledgeBase,
        current_document: WriterDocumentContext<'a>,
    ) -> Self {
        Self {
            knowledge,
            current_document: Some(current_document),
            search_provider: None,
            search_context: None,
            patch_session: None,
        }
    }

    /// U108 阶段 3：挂上 patch 会话，使本节点内的行号改动累积成一个最终 patch。
    ///
    /// 不挂会话时行号工具只返回 patch 给模型看，不会有任何东西落盘——
    /// 这正是接线前的缺陷状态，因此**写作节点必须挂**。
    pub fn with_patch_session(mut self, session: PatchSession) -> Self {
        self.patch_session = Some(Mutex::new(session));
        self
    }

    /// 接入外部 SearchProvider；搜索结果仍不会自动写入知识库。
    pub fn with_search_provider(
        mut self,
        provider: &'a dyn SearchProvider,
        context: ProviderCallContext,
    ) -> Self {
        self.search_provider = Some(provider);
        self.search_context = Some(context);
        self
    }

    /// 执行外部搜索，不自动写入创作知识库。
    pub fn execute_search(
        &self,
        provider: &dyn SearchProvider,
        context: &ProviderCallContext,
        query: impl Into<String>,
        limit: Option<usize>,
        metadata: Value,
    ) -> CoreResult<WritingSearchResponse> {
        let response = provider.search(
            context,
            SearchProviderRequest {
                query: query.into(),
                limit,
                metadata,
            },
        )?;
        Ok(search_response_to_writing_response(response))
    }

    fn execute_register(
        &self,
        tool_name: &str,
        arguments: &Value,
    ) -> CoreResult<ToolExecutionOutput> {
        let function = RegisterFunction::parse(required_str(arguments, "a")?)?;
        let operation = RegisterOperation::parse(required_str(arguments, "b")?)?;
        let change_id = optional_str(arguments, "change_id")
            .or_else(|| optional_str_path(arguments, &["c", "change_id"]));
        let content = match operation {
            RegisterOperation::List | RegisterOperation::Delete => None,
            RegisterOperation::New | RegisterOperation::Update => {
                let value = required_value(arguments, "c")?;
                Some(RegisterContent::parse(function, value.clone())?)
            }
        };

        let changes = self
            .knowledge
            .apply_register_operation(function, operation, content, change_id)?;
        Ok(ToolExecutionOutput {
            value: json!({ "changes": changes }),
            audit_metadata: json!({
                "tool": tool_name,
                "function": function,
                "operation": operation,
                "count": changes.len(),
            }),
        })
    }

    fn execute_find(&self, tool_name: &str, arguments: &Value) -> CoreResult<ToolExecutionOutput> {
        let request = parse_find_request(arguments)?;
        let include_text = request.include_text;
        let mut response = self.knowledge.find(request)?;
        if include_text {
            self.attach_document_text(&mut response.results)?;
        }
        Ok(ToolExecutionOutput {
            value: serde_json::to_value(&response)?,
            audit_metadata: json!({
                "tool": tool_name,
                "result_count": response.results.len(),
            }),
        })
    }

    fn execute_search_tool(
        &self,
        tool_name: &str,
        arguments: &Value,
    ) -> CoreResult<ToolExecutionOutput> {
        let provider = self.search_provider.ok_or_else(|| {
            CoreError::validation("search tools require a SearchProvider adapter")
        })?;
        let context = self.search_context.as_ref().ok_or_else(|| {
            CoreError::validation("search tools require a SearchProvider context")
        })?;
        let response = self.execute_search(
            provider,
            context,
            required_str(arguments, "query")?,
            optional_u64(arguments, "limit").map(|value| value as usize),
            arguments.get("metadata").cloned().unwrap_or(Value::Null),
        )?;
        Ok(ToolExecutionOutput {
            value: serde_json::to_value(&response)?,
            audit_metadata: json!({
                "tool": tool_name,
                "result_count": response.results.len(),
                "persisted_to_knowledge": response.persisted_to_knowledge,
            }),
        })
    }

    fn execute_line_insert(
        &self,
        tool_name: &str,
        arguments: &Value,
    ) -> CoreResult<ToolExecutionOutput> {
        let document = self.require_document_context()?;
        // 先校验工具与当前文档作用域匹配，再校验参数 document_id 未越界到其它文件。
        let document_id = self.resolve_line_patch_target(tool_name, &document, arguments)?;
        let request = WriterInsertLines {
            document_id,
            base_version: base_version_from_args(arguments, document.base_version),
            after_line: required_u64(arguments, "after_line")?,
            text: required_str(arguments, "text")?.to_owned(),
        };
        // U108 阶段 3：行号一律对着**会话模拟文本**换算，而不是节点开始时的原始正文。
        // 模型数的行号是「上一次改动之后」的行号；用原始正文换算会插错位置。
        let basis = self.line_patch_basis(document.text)?;
        let patch = insert_lines_to_patch(&basis, request.clone())?;
        self.record_insert(&request)?;
        Ok(ToolExecutionOutput {
            value: serde_json::to_value(&patch)?,
            audit_metadata: json!({
                "tool": tool_name,
                "hunks": patch.hunks.len(),
            }),
        })
    }

    fn execute_line_replace(
        &self,
        tool_name: &str,
        arguments: &Value,
    ) -> CoreResult<ToolExecutionOutput> {
        let document = self.require_document_context()?;
        // 先校验工具与当前文档作用域匹配，再校验参数 document_id 未越界到其它文件。
        let document_id = self.resolve_line_patch_target(tool_name, &document, arguments)?;
        let request = WriterReplaceLines {
            document_id,
            base_version: base_version_from_args(arguments, document.base_version),
            start_line: required_u64(arguments, "start_line")?,
            end_line: required_u64(arguments, "end_line")?,
            text: required_str(arguments, "text")?.to_owned(),
        };
        // 同 `execute_line_insert`：行号对着会话模拟文本换算。
        let basis = self.line_patch_basis(document.text)?;
        let patch = replace_lines_to_patch(&basis, request.clone())?;
        self.record_replace(&request)?;
        Ok(ToolExecutionOutput {
            value: serde_json::to_value(&patch)?,
            audit_metadata: json!({
                "tool": tool_name,
                "hunks": patch.hunks.len(),
            }),
        })
    }

    /// 校验行号 patch 工具的写作边界，返回最终允许写入的 document_id。
    ///
    /// 两道约束（对应 `创作总结机制(不可删除).md`“每类节点只能修改自己负责的纲领文件”）：
    /// 1. 工具种类对应的作用域必须与当前正文上下文的作用域一致，
    ///    例如 outliner-* 只能作用于全局总纲，不能改章节正文。
    /// 2. 调用参数中的 `document_id` 不能偏离当前上下文文档，
    ///    避免 LLM 通过参数把 patch 指向其它文件绕过沙箱。
    /// U119：整文件重写。复用行号工具的作用域与目标校验，避免规划类 agent
    /// 借该工具整章覆写正文；产出同样只是 `DocumentPatch`，落盘仍走确认流。
    fn execute_rewrite_file(
        &self,
        tool_name: &str,
        arguments: &Value,
    ) -> CoreResult<ToolExecutionOutput> {
        let document = self.require_document_context()?;
        let document_id = self.resolve_line_patch_target(tool_name, &document, arguments)?;
        let replacement = required_str(arguments, "text")?.to_owned();
        // 整文件重写同样要对着会话模拟文本：本节点内先插了几行再整章重写时，
        // 覆盖的应该是「插入之后」的正文。
        let basis = self.line_patch_basis(document.text)?;
        let patch = rewrite_file_to_patch(
            document_id,
            base_version_from_args(arguments, document.base_version),
            &basis,
            replacement.clone(),
        )?;
        self.record_rewrite(&basis, replacement)?;
        Ok(ToolExecutionOutput {
            value: serde_json::to_value(&patch)?,
            audit_metadata: json!({
                "tool": tool_name,
                "hunks": patch.hunks.len(),
            }),
        })
    }

    /// U108 阶段 3：行号换算的基准文本。
    ///
    /// 有会话时取模拟文本（含本节点此前的全部改动），无会话时退回原始正文，
    /// 后者即接线前的行为。
    fn line_patch_basis(&self, document_text: &str) -> CoreResult<String> {
        let Some(session) = &self.patch_session else {
            return Ok(document_text.to_owned());
        };
        let session = session.lock().map_err(|_| Self::patch_session_poisoned())?;
        Ok(session.simulated.clone())
    }

    /// 把一次插入记进会话；无会话时不记账（行为与接线前一致）。
    fn record_insert(&self, request: &WriterInsertLines) -> CoreResult<()> {
        let Some(session) = &self.patch_session else {
            return Ok(());
        };
        let mut session = session.lock().map_err(|_| Self::patch_session_poisoned())?;
        session.insert_lines(request.after_line, request.text.clone())
    }

    /// 把一次替换记进会话；无会话时不记账。
    fn record_replace(&self, request: &WriterReplaceLines) -> CoreResult<()> {
        let Some(session) = &self.patch_session else {
            return Ok(());
        };
        let mut session = session.lock().map_err(|_| Self::patch_session_poisoned())?;
        session.replace_lines(request.start_line, request.end_line, request.text.clone())
    }

    /// 把一次整文件重写记进会话：等价于「替换掉当前全部行」。
    ///
    /// 空文件用 `(0, 0)`——`validate_replace_lines` 对空文档只接受这一个区间。
    fn record_rewrite(&self, basis: &str, replacement: String) -> CoreResult<()> {
        let Some(session) = &self.patch_session else {
            return Ok(());
        };
        let mut session = session.lock().map_err(|_| Self::patch_session_poisoned())?;
        let line_count = basis.split_inclusive('\n').count() as u64;
        if line_count == 0 {
            session.replace_lines(0, 0, replacement)
        } else {
            session.replace_lines(1, line_count, replacement)
        }
    }

    /// 锁中毒即前一次记账 panic 了，会话内容不可信。
    /// fail-loud 而不是跳过：跳过等于悄悄丢掉用户的正文改动。
    fn patch_session_poisoned() -> CoreError {
        CoreError::validation("writing patch session lock is poisoned; node output is unreliable")
    }

    /// U108 阶段 3：提交本节点累积的 patch 会话。
    ///
    /// 没有会话、或会话里一次改动都没有时返回 `None`——与
    /// 「副作用类确认项无证据则不产出」同一语义，避免造出一条永远待审的空确认项
    /// 把工作流卡死在 `PendingConfirmation`。
    pub fn commit_patch_session(&self) -> CoreResult<Option<PatchSessionCommit>> {
        let Some(session) = &self.patch_session else {
            return Ok(None);
        };
        let session = session.lock().map_err(|_| Self::patch_session_poisoned())?;
        if session.pending_ops.is_empty() {
            return Ok(None);
        }
        session.commit().map(Some)
    }

    fn resolve_line_patch_target(
        &self,
        tool_name: &str,
        document: &WriterDocumentContext<'a>,
        arguments: &Value,
    ) -> CoreResult<String> {
        let required_scope = line_tool_required_scope(tool_name).ok_or_else(|| {
            CoreError::validation(format!("{tool_name} is not a line patch tool"))
        })?;
        if document.scope != required_scope {
            return Err(CoreError::PermissionDenied {
                action: tool_name.to_owned(),
                reason: format!(
                    "tool requires {} document scope but current document is {}",
                    required_scope.label(),
                    document.scope.label()
                ),
            });
        }

        // document_id 参数缺省时回退到上下文文档；显式给出时必须与上下文一致。
        let target = document_id_from_args(arguments, document.document_id);
        if target != document.document_id {
            return Err(CoreError::PermissionDenied {
                action: tool_name.to_owned(),
                reason: format!(
                    "tool may only edit current document {} but requested {target}",
                    document.document_id
                ),
            });
        }
        Ok(target)
    }

    fn require_document_context(&self) -> CoreResult<WriterDocumentContext<'a>> {
        self.current_document.ok_or_else(|| {
            CoreError::validation("line patch tools require current document context")
        })
    }

    /// 对 find 结果按 SourceSpan 回填正文；只有显式 include_text 时调用。
    fn attach_document_text(&self, results: &mut [FindResult]) -> CoreResult<()> {
        let Some(document) = self.current_document else {
            return Ok(());
        };
        for result in results {
            let Some(span) = result
                .spans
                .iter()
                .find(|span| span.document_id == document.document_id)
            else {
                continue;
            };
            let start = usize::try_from(span.range.start)
                .map_err(|_| CoreError::validation("source span start exceeds usize range"))?;
            let end = usize::try_from(span.range.end)
                .map_err(|_| CoreError::validation("source span end exceeds usize range"))?;
            let Some(text) = document.text.get(start..end) else {
                return Err(CoreError::validation(
                    "source span is not aligned to UTF-8 character boundaries",
                ));
            };
            result.text = Some(text.to_owned());
        }
        Ok(())
    }
}

impl WritingToolExecutor<'_> {
    /// 本执行器**自己**能承接的工具名。
    ///
    /// 刻意不含 `*-search`：项目检索由 `ProjectSearchToolExecutor` 承接，
    /// 二者必须经 `ToolExecutorRouter` 分流。调用方据此注册路由，
    /// 就不会把 `writer-search` 错误地交给本执行器而收到
    /// "unsupported writing tool"（U108 接线时的第一个坑）。
    pub fn handles_tool(tool_name: &str) -> bool {
        matches!(
            tool_name,
            TOOL_OUTLINER_REGISTER
                | TOOL_DESIGNER_REGISTER
                | TOOL_PLANNER_REGISTER
                | TOOL_OUTLINER_FIND
                | TOOL_DESIGNER_FIND
                | TOOL_PLANNER_FIND
                | TOOL_DETAIL_FIND
                | TOOL_WRITER_FIND
                | TOOL_CRITIC_FIND
                | TOOL_PRUDENT_FIND
                | TOOL_POLISHER_FIND
                | TOOL_OUTLINER_INSERT_LINES
                | TOOL_DESIGNER_INSERT_LINES
                | TOOL_PLANNER_INSERT_LINES
                | TOOL_WRITER_INSERT_LINES
                | TOOL_POLISHER_INSERT_LINES
                | TOOL_OUTLINER_REPLACE_LINES
                | TOOL_DESIGNER_REPLACE_LINES
                | TOOL_PLANNER_REPLACE_LINES
                | TOOL_WRITER_REPLACE_LINES
                | TOOL_POLISHER_REPLACE_LINES
                | TOOL_OUTLINER_REWRITE_FILE
                | TOOL_DESIGNER_REWRITE_FILE
                | TOOL_PLANNER_REWRITE_FILE
                | TOOL_WRITER_REWRITE_FILE
                | TOOL_POLISHER_REWRITE_FILE
                | TOOL_OUTLINER_WEB_SEARCH
                | TOOL_DESIGNER_WEB_SEARCH
                | TOOL_PLANNER_WEB_SEARCH
                | TOOL_DETAIL_WEB_SEARCH
                | TOOL_WRITER_WEB_SEARCH
                | TOOL_CRITIC_WEB_SEARCH
                | TOOL_PRUDENT_WEB_SEARCH
                | TOOL_POLISHER_WEB_SEARCH
        )
    }

    /// 判断工具是否有副作用（写盘或改知识库）：行号 patch 类 + 三个 `*-register`。
    ///
    /// ⚠️ **U116 复核（2026-08-10）：生产零调用者，且刻意不接线。**
    ///
    /// 它原本要服务的判断是「有副作用的节点需要更强的幂等保证」。但现状是
    /// **所有写作节点一律注册为 `WorkflowOperationPolicy::at_most_once()`**
    /// （`commands.rs` 的模型节点 handler），那已经是最保守的策略：
    /// 恢复走 `ManualResolution`，不自动重放。
    ///
    /// 也就是说这个区分只可能用来**放宽**（把只读节点降级成可自动重放），
    /// 属于性能优化而非安全需求，眼下没有任何调用方需要它。
    /// 接线它不会让系统更安全，只会多一条要维护的分支。
    ///
    /// 真正在用的是它的下位判定 `is_line_patch_tool`——
    /// U108 用后者统计 patch 证据（`integration.rs` 的工具循环）。
    pub fn is_mutating_tool(tool_name: &str) -> bool {
        Self::is_line_patch_tool(tool_name)
            || matches!(
                tool_name,
                TOOL_OUTLINER_REGISTER | TOOL_DESIGNER_REGISTER | TOOL_PLANNER_REGISTER
            )
    }

    /// 判断工具是否是行号 patch 类。
    ///
    /// 这类工具必须有当前文档正文才能把行号换算成字节区间，
    /// 因此节点未指名 `document_id` 时不应下发——否则模型拿到的是必然失败的工具。
    pub fn is_line_patch_tool(tool_name: &str) -> bool {
        line_tool_required_scope(tool_name).is_some()
    }
}

impl ToolExecutor for WritingToolExecutor<'_> {
    /// 执行 Module 9 写作工具。
    fn execute(
        &self,
        _context: &ToolExecutionContext,
        call: &ToolCall,
    ) -> CoreResult<ToolExecutionOutput> {
        match call.name.as_str() {
            TOOL_OUTLINER_REGISTER | TOOL_DESIGNER_REGISTER | TOOL_PLANNER_REGISTER => {
                self.execute_register(&call.name, &call.arguments)
            }
            TOOL_OUTLINER_FIND | TOOL_DESIGNER_FIND | TOOL_PLANNER_FIND | TOOL_DETAIL_FIND
            | TOOL_WRITER_FIND | TOOL_CRITIC_FIND | TOOL_PRUDENT_FIND | TOOL_POLISHER_FIND => {
                self.execute_find(&call.name, &call.arguments)
            }
            TOOL_OUTLINER_INSERT_LINES
            | TOOL_DESIGNER_INSERT_LINES
            | TOOL_PLANNER_INSERT_LINES
            | TOOL_WRITER_INSERT_LINES
            | TOOL_POLISHER_INSERT_LINES => self.execute_line_insert(&call.name, &call.arguments),
            TOOL_OUTLINER_REPLACE_LINES
            | TOOL_DESIGNER_REPLACE_LINES
            | TOOL_PLANNER_REPLACE_LINES
            | TOOL_WRITER_REPLACE_LINES
            | TOOL_POLISHER_REPLACE_LINES => self.execute_line_replace(&call.name, &call.arguments),
            // U119：整文件重写只对规划类短纲领开放；writer/polisher 的正文
            // 必须走行级修改（见 指导性文件/项目总计划-架构版:416），因此这里
            // 刻意不含 writer/polisher，且作用域校验会再拦一次越权覆写。
            // U123：writer/polisher 也开放整文件重写。设计原先要求「除非用户显式
            // 确认 Writer 重写模式」，但该确认条件在产品里从不存在（无开关、无确认
            // 项）；而分章节写作下一个文件就是一章，整章重写是自然操作。
            // 作用域校验仍拦住越权——writer 只能重写 ChapterBody，改不到纲领。
            TOOL_OUTLINER_REWRITE_FILE
            | TOOL_DESIGNER_REWRITE_FILE
            | TOOL_PLANNER_REWRITE_FILE
            | TOOL_WRITER_REWRITE_FILE
            | TOOL_POLISHER_REWRITE_FILE => {
                self.execute_rewrite_file(&call.name, &call.arguments)
            }
            TOOL_OUTLINER_WEB_SEARCH
            | TOOL_DESIGNER_WEB_SEARCH
            | TOOL_PLANNER_WEB_SEARCH
            | TOOL_DETAIL_WEB_SEARCH
            | TOOL_WRITER_WEB_SEARCH
            | TOOL_CRITIC_WEB_SEARCH
            | TOOL_PRUDENT_WEB_SEARCH
            | TOOL_POLISHER_WEB_SEARCH => self.execute_search_tool(&call.name, &call.arguments),
            // `*-search` 是项目检索，归 ProjectSearchToolExecutor；能走到这里
            // 说明调用方没有按 `handles_tool` 分流，属于装配错误而非模型错误。
            TOOL_OUTLINER_SEARCH
            | TOOL_DESIGNER_SEARCH
            | TOOL_PLANNER_SEARCH
            | TOOL_DETAIL_SEARCH
            | TOOL_WRITER_SEARCH
            | TOOL_CRITIC_SEARCH
            | TOOL_PRUDENT_SEARCH
            | TOOL_POLISHER_SEARCH
            | TOOL_SUMMARIZER_SEARCH => Err(CoreError::validation(format!(
                "project search tool '{}' must be routed to the project search executor, \
                 not the writing tool executor",
                call.name
            ))),
            other => Err(CoreError::validation(format!(
                "unsupported writing tool: {other}"
            ))),
        }
    }
}

/// 将 SearchProvider 响应转换为写作搜索响应，并明确不自动入库。
pub fn search_response_to_writing_response(
    response: SearchProviderResponse,
) -> WritingSearchResponse {
    WritingSearchResponse {
        results: response
            .results
            .into_iter()
            .enumerate()
            .map(|(index, result)| FindResult {
                result_id: format!("search-{}", index + 1),
                title: result.title,
                snippet: result.snippet,
                score: result.score,
                source: result.url,
                spans: Vec::new(),
                text: None,
                metadata: result.metadata,
            })
            .collect(),
        persisted_to_knowledge: false,
    }
}

/// 从工具参数解析 find 请求，兼容机制文档的 a/b/c 参数命名。
pub fn parse_find_request(arguments: &Value) -> CoreResult<FindRequest> {
    let scope = FindScope::parse(required_str(arguments, "a")?)?;
    let query = required_str(arguments, "b")?.to_owned();
    let include_text = arguments
        .get("include_text")
        .and_then(Value::as_bool)
        .or_else(|| {
            arguments
                .get("c")
                .and_then(|value| value.get("include_text"))
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);
    let metadata = arguments.get("c").cloned().unwrap_or(Value::Null);

    Ok(FindRequest {
        scope,
        query,
        include_text,
        metadata,
    })
}

/// 创建单个工具定义。
fn tool_definition(
    name: &str,
    prompt_key: &str,
    prompts: &PromptResources,
    input_schema: Value,
) -> CoreResult<ToolDefinition> {
    let resource = prompts.get(prompt_key).ok_or_else(|| {
        CoreError::validation(format!("missing prompt resource for tool: {prompt_key}"))
    })?;

    Ok(ToolDefinition {
        name: name.to_owned(),
        description: resource.describe.clone(),
        input_schema,
    })
}

/// planner-register 输入 schema。
fn planner_register_schema() -> Value {
    json!({
        "type": "object",
        "required": ["a", "b"],
        "properties": {
            "a": {
                "type": "string",
                "enum": ["character_trait", "relationship", "foreshadowing"]
            },
            "b": {
                "type": "string",
                "enum": ["list", "new", "update", "delete"]
            },
            "c": {
                "type": "object",
                "description": "按 a 的值使用人物性格、人物关系或伏笔强类型结构"
            },
            "change_id": {
                "type": "string",
                "description": "可选；delete 或指定注册项 id 时使用"
            }
        }
    })
}

/// 整文件重写输入 schema（U119）。
///
/// 只暴露 `text`：目标文件由执行时的文档上下文锚定，`document_id` 仅用于
/// 让模型显式确认写的是哪个文件（与上下文不一致时 `resolve_line_patch_target`
/// 会拒绝）。不接受行号——整文件重写本身就是「全量替换」语义。
fn rewrite_file_schema() -> Value {
    json!({
        "type": "object",
        "required": ["text"],
        "additionalProperties": false,
        "properties": {
            "document_id": {
                "type": "string",
                "description": "可选；给出时必须与当前节点负责的文档一致"
            },
            "base_version": {
                "type": "string",
                "description": "可选；乐观并发版本号，缺省取当前文档版本"
            },
            "text": {
                "type": "string",
                "description": "替换后的完整文件正文"
            }
        }
    })
}

/// find 输入 schema。
fn find_schema() -> Value {
    json!({
        "type": "object",
        "required": ["a", "b"],
        "properties": {
            // U120：这里必须与 `FindScope::parse`（rag/models.rs）保持同步。
            // 少列一个 scope，对应的知识就变成「register 写得进、find 查不回」的
            // 黑洞——底层查询早已实现，只是模型从 schema 里看不到这个入口。
            "a": {
                "type": "string",
                "enum": [
                    "character_profile",
                    "character_plan",
                    "character_trait_path",
                    "relationship_path",
                    "event_segments",
                    "segment_text",
                    "foreshadowing",
                    "theme_anchor",
                    "chapter_summary",
                    "stage_summary"
                ]
            },
            "b": { "type": "string" },
            "c": { "type": "object" },
            "include_text": { "type": "boolean" }
        }
    })
}

/// search 输入 schema。
fn search_schema() -> Value {
    json!({
        "type": "object",
        "required": ["query"],
        "properties": {
            "query": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1 },
            "metadata": { "type": "object" }
        }
    })
}

/// writer-insert-lines 输入 schema。
fn writer_insert_schema() -> Value {
    json!({
        "type": "object",
        "required": ["after_line", "text"],
        "properties": {
            "document_id": { "type": "string" },
            "base_version": { "type": "string" },
            // U123：0 = 插入到文件最开头（第 1 行之前）。schema 若仍写
            // `minimum: 1`，模型就不知道 0 可用，后端放行等于白做——空文件
            // 依旧一个字写不进去。
            "after_line": {
                "type": "integer",
                "minimum": 0,
                "description": "在第几行之后插入；0 表示插入到文件最开头（空文件必须用 0）"
            },
            "text": { "type": "string" }
        }
    })
}

/// writer-replace-lines 输入 schema。
fn writer_replace_schema() -> Value {
    json!({
        "type": "object",
        "required": ["start_line", "end_line", "text"],
        "properties": {
            "document_id": { "type": "string" },
            "base_version": { "type": "string" },
            // U123：空文件只能用 start_line = end_line = 0 写入初始内容；
            // 非空文件仍是 1-based 闭区间（后端会拒绝非空文件上的 0）。
            "start_line": {
                "type": "integer",
                "minimum": 0,
                "description": "起始行（1-based）；空文件写入初始内容时用 0"
            },
            "end_line": {
                "type": "integer",
                "minimum": 0,
                "description": "结束行（1-based，含）；空文件写入初始内容时用 0"
            },
            "text": { "type": "string" }
        }
    })
}

/// 读取必填字符串参数。
fn required_str<'a>(arguments: &'a Value, key: &str) -> CoreResult<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CoreError::validation(format!("tool argument {key} must be a string")))
}

/// 读取可选字符串参数。
fn optional_str(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

/// 读取嵌套路径中的可选字符串参数。
fn optional_str_path(arguments: &Value, path: &[&str]) -> Option<String> {
    let mut value = arguments;
    for key in path {
        value = value.get(*key)?;
    }
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

/// 读取必填 JSON 参数。
fn required_value<'a>(arguments: &'a Value, key: &str) -> CoreResult<&'a Value> {
    arguments
        .get(key)
        .ok_or_else(|| CoreError::validation(format!("tool argument {key} is required")))
}

/// 读取必填 u64 参数。
fn required_u64(arguments: &Value, key: &str) -> CoreResult<u64> {
    arguments.get(key).and_then(Value::as_u64).ok_or_else(|| {
        CoreError::validation(format!("tool argument {key} must be a positive integer"))
    })
}

/// 读取可选 u64 参数。
fn optional_u64(arguments: &Value, key: &str) -> Option<u64> {
    arguments.get(key).and_then(Value::as_u64)
}

/// Writer 参数可覆盖当前正文上下文中的文档 id。
fn document_id_from_args(arguments: &Value, fallback: &str) -> String {
    optional_str(arguments, "document_id").unwrap_or_else(|| fallback.to_owned())
}

/// Writer 参数可覆盖当前正文上下文中的基础版本。
fn base_version_from_args(arguments: &Value, fallback: Option<&str>) -> Option<String> {
    optional_str(arguments, "base_version").or_else(|| fallback.map(ToOwned::to_owned))
}
