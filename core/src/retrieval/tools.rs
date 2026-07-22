use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult};
use crate::llm::{ToolExecutionContext, ToolExecutionOutput, ToolExecutor};
use crate::providers::{ProviderCallContext, ToolCall, ToolDefinition};
use crate::retrieval::ProjectRetrievalRuntime;

pub use crate::node_capabilities::{
    EXECUTOR_ADAPTER_SEARCH_TOOL, GENERIC_LLM_SEARCH_TOOL, PROJECT_AI_SEARCH_TOOL,
    SUMMARIZER_SEARCH_TOOL,
};
pub const DEFAULT_PROJECT_SEARCH_LIMIT: usize = 10;

/// 构造项目内检索工具定义。工具只读项目索引，不会修改正文或知识库。
pub fn project_search_tool_definition(
    name: impl Into<String>,
    description: impl Into<String>,
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "要在当前项目文档与已确认知识中检索的问题或关键词"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "default": DEFAULT_PROJECT_SEARCH_LIMIT
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

/// 把任意 AI 节点的 search 工具统一路由到项目级混合检索运行时。
pub struct ProjectSearchToolExecutor<'a> {
    runtime: &'a ProjectRetrievalRuntime,
    base_context: ProviderCallContext,
    allowed_tools: BTreeSet<String>,
}

impl<'a> ProjectSearchToolExecutor<'a> {
    pub fn new(
        runtime: &'a ProjectRetrievalRuntime,
        base_context: ProviderCallContext,
        allowed_tools: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            runtime,
            base_context,
            allowed_tools: allowed_tools.into_iter().collect(),
        }
    }
}

impl ToolExecutor for ProjectSearchToolExecutor<'_> {
    fn execute(
        &self,
        context: &ToolExecutionContext,
        call: &ToolCall,
    ) -> CoreResult<ToolExecutionOutput> {
        if !self.allowed_tools.contains(&call.name) {
            return Err(CoreError::validation(format!(
                "project search tool is not allowed in this scope: {}",
                call.name
            )));
        }
        let query = call
            .arguments
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| CoreError::validation("project search query is required"))?
            .to_owned();
        let limit = call
            .arguments
            .get("limit")
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| CoreError::validation("project search limit must be an integer"))
                    .and_then(|value| {
                        usize::try_from(value).map_err(|_| {
                            CoreError::validation("project search limit exceeds platform range")
                        })
                    })
            })
            .transpose()?
            .unwrap_or(DEFAULT_PROJECT_SEARCH_LIMIT);

        let mut provider_context = self.base_context.clone();
        provider_context.provider_id = "project_retrieval".to_owned();
        provider_context.workflow_id = context.workflow_id.clone().or(provider_context.workflow_id);
        provider_context.run_id = context.run_id.clone().or(provider_context.run_id);
        provider_context.node_id = context.node_id.clone().or(provider_context.node_id);
        provider_context.tool_call_id = Some(call.tool_call_id.clone());
        provider_context.operation_id = provider_context.operation_id.map(|base| {
            format!(
                "{base}:search-round-{}:{}",
                context.round,
                stable_operation_component(&call.tool_call_id)
            )
        });
        provider_context.metadata = json!({
            "tool": call.name,
            "tool_call_id": call.tool_call_id,
            "round": context.round,
        });

        let results = self
            .runtime
            .search(query.clone(), limit, provider_context)?;
        let result_count = results.len();
        Ok(ToolExecutionOutput {
            value: json!({
                "query": query,
                "count": result_count,
                "results": results,
            }),
            audit_metadata: json!({
                "tool": call.name,
                "retrieval_scope": "project",
                "result_count": result_count,
                "vector_enabled": self.runtime.vector_enabled(),
            }),
        })
    }
}

fn stable_operation_component(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if normalized.is_empty() {
        "tool-call".to_owned()
    } else {
        normalized
    }
}
