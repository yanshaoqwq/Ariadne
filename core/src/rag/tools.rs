use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult};
use crate::llm::{ToolExecutionContext, ToolExecutionOutput, ToolExecutor};
use crate::providers::{
    ProviderCallContext, SearchProvider, SearchProviderRequest, SearchProviderResponse, ToolCall,
    ToolDefinition,
};
use crate::rag::line_patch::{
    insert_lines_to_patch, replace_lines_to_patch, WriterInsertLines, WriterReplaceLines,
};
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    FindRequest, FindResult, FindScope, RegisterContent, RegisterFunction, RegisterOperation,
    WritingAgentKind, WritingSearchResponse,
};
use crate::rag::resources::PromptResources;

pub const TOOL_OUTLINER_REGISTER: &str = "outliner-register";
pub const TOOL_OUTLINER_FIND: &str = "outliner-find";
pub const TOOL_OUTLINER_SEARCH: &str = "outliner-search";
pub const TOOL_OUTLINER_WEB_SEARCH: &str = "outliner-web-search";
pub const TOOL_OUTLINER_INSERT_LINES: &str = "outliner-insert-lines";
pub const TOOL_OUTLINER_REPLACE_LINES: &str = "outliner-replace-lines";
pub const TOOL_DESIGNER_REGISTER: &str = "designer-register";
pub const TOOL_DESIGNER_FIND: &str = "designer-find";
pub const TOOL_DESIGNER_SEARCH: &str = "designer-search";
pub const TOOL_DESIGNER_WEB_SEARCH: &str = "designer-web-search";
pub const TOOL_DESIGNER_INSERT_LINES: &str = "designer-insert-lines";
pub const TOOL_DESIGNER_REPLACE_LINES: &str = "designer-replace-lines";
pub const TOOL_PLANNER_REGISTER: &str = "planner-register";
pub const TOOL_PLANNER_FIND: &str = "planner-find";
pub const TOOL_PLANNER_SEARCH: &str = "planner-search";
pub const TOOL_PLANNER_WEB_SEARCH: &str = "planner-web-search";
pub const TOOL_PLANNER_INSERT_LINES: &str = "planner-insert-lines";
pub const TOOL_PLANNER_REPLACE_LINES: &str = "planner-replace-lines";
pub const TOOL_DETAIL_FIND: &str = "detail-find";
pub const TOOL_DETAIL_SEARCH: &str = "detail-search";
pub const TOOL_DETAIL_WEB_SEARCH: &str = "detail-web-search";
pub const TOOL_WRITER_FIND: &str = "writer-find";
pub const TOOL_WRITER_SEARCH: &str = "writer-search";
pub const TOOL_WRITER_WEB_SEARCH: &str = "writer-web-search";
pub const TOOL_WRITER_INSERT_LINES: &str = "writer-insert-lines";
pub const TOOL_WRITER_REPLACE_LINES: &str = "writer-replace-lines";
pub const TOOL_CRITIC_FIND: &str = "critic-find";
pub const TOOL_CRITIC_SEARCH: &str = "critic-search";
pub const TOOL_CRITIC_WEB_SEARCH: &str = "critic-web-search";
pub const TOOL_PRUDENT_FIND: &str = "prudent-find";
pub const TOOL_PRUDENT_SEARCH: &str = "prudent-search";
pub const TOOL_PRUDENT_WEB_SEARCH: &str = "prudent-web-search";
pub const TOOL_POLISHER_FIND: &str = "polisher-find";
pub const TOOL_POLISHER_SEARCH: &str = "polisher-search";
pub const TOOL_POLISHER_WEB_SEARCH: &str = "polisher-web-search";
pub const TOOL_POLISHER_INSERT_LINES: &str = "polisher-insert-lines";
pub const TOOL_POLISHER_REPLACE_LINES: &str = "polisher-replace-lines";
pub const TOOL_SUMMARIZER_SEARCH: &str = "summarizer-search";
pub const TOOL_SUMMARIZER_WEB_SEARCH: &str = "summarizer-web-search";

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
        TOOL_OUTLINER_INSERT_LINES | TOOL_OUTLINER_REPLACE_LINES => {
            Some(WritingDocumentScope::GlobalOutline)
        }
        TOOL_DESIGNER_INSERT_LINES | TOOL_DESIGNER_REPLACE_LINES => {
            Some(WritingDocumentScope::StageOutline)
        }
        TOOL_PLANNER_INSERT_LINES | TOOL_PLANNER_REPLACE_LINES => {
            Some(WritingDocumentScope::ChapterOutline)
        }
        TOOL_WRITER_INSERT_LINES
        | TOOL_WRITER_REPLACE_LINES
        | TOOL_POLISHER_INSERT_LINES
        | TOOL_POLISHER_REPLACE_LINES => Some(WritingDocumentScope::ChapterBody),
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
        }
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
        let patch = insert_lines_to_patch(document.text, request)?;
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
        let patch = replace_lines_to_patch(document.text, request)?;
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
            TOOL_OUTLINER_WEB_SEARCH
            | TOOL_DESIGNER_WEB_SEARCH
            | TOOL_PLANNER_WEB_SEARCH
            | TOOL_DETAIL_WEB_SEARCH
            | TOOL_WRITER_WEB_SEARCH
            | TOOL_CRITIC_WEB_SEARCH
            | TOOL_PRUDENT_WEB_SEARCH
            | TOOL_POLISHER_WEB_SEARCH => self.execute_search_tool(&call.name, &call.arguments),
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

/// find 输入 schema。
fn find_schema() -> Value {
    json!({
        "type": "object",
        "required": ["a", "b"],
        "properties": {
            "a": {
                "type": "string",
                "enum": [
                    "character_trait_path",
                    "relationship_path",
                    "event_segments",
                    "segment_text",
                    "foreshadowing",
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
            "after_line": { "type": "integer", "minimum": 1 },
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
            "start_line": { "type": "integer", "minimum": 1 },
            "end_line": { "type": "integer", "minimum": 1 },
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
