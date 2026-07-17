use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult, PermissionPolicy, PermissionRequest};
use crate::costs::CostLedger;
use crate::llm::{ToolExecutionContext, ToolExecutionOutput, ToolExecutor};
use crate::providers::{
    ProviderCallContext, ProviderExecutor, SearchProvider, SearchProviderRequest, ToolCall,
    ToolDefinition,
};

pub const PROJECT_AI_WEB_SEARCH_TOOL: &str = "project-ai-web-search";
pub const GENERIC_LLM_WEB_SEARCH_TOOL: &str = "llm-web-search";
pub const SUMMARIZER_WEB_SEARCH_TOOL: &str = "summarizer-web-search";
pub const EXECUTOR_ADAPTER_WEB_SEARCH_TOOL: &str = "executor-adapter-web-search";
pub const DEFAULT_WEB_SEARCH_LIMIT: usize = 8;

/// 构造外部 Web 搜索工具定义；与当前项目的本地 Search 明确分离。
pub fn web_search_tool_definition(
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
                    "description": "要在公开互联网中检索的问题或关键词"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "default": DEFAULT_WEB_SEARCH_LIMIT
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

/// 通用 Web 搜索工具执行器：硬权限、派发栅栏和成本归因都在真实网络边界复核。
pub struct WebSearchToolExecutor<'a, L: CostLedger> {
    provider: &'a dyn SearchProvider,
    ledger: &'a L,
    policy: &'a PermissionPolicy,
    base_context: ProviderCallContext,
    allowed_tools: BTreeSet<String>,
}

impl<'a, L: CostLedger> WebSearchToolExecutor<'a, L> {
    pub fn new(
        provider: &'a dyn SearchProvider,
        ledger: &'a L,
        policy: &'a PermissionPolicy,
        base_context: ProviderCallContext,
        allowed_tools: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            provider,
            ledger,
            policy,
            base_context,
            allowed_tools: allowed_tools.into_iter().collect(),
        }
    }
}

impl<L: CostLedger> ToolExecutor for WebSearchToolExecutor<'_, L> {
    fn execute(
        &self,
        context: &ToolExecutionContext,
        call: &ToolCall,
    ) -> CoreResult<ToolExecutionOutput> {
        if !self.allowed_tools.contains(&call.name) {
            return Err(CoreError::validation(format!(
                "web search tool is not allowed in this scope: {}",
                call.name
            )));
        }
        self.policy.ensure(&PermissionRequest::WebSearch)?;
        let query = call
            .arguments
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| CoreError::validation("web search query is required"))?
            .to_owned();
        let limit = call
            .arguments
            .get("limit")
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| CoreError::validation("web search limit must be an integer"))
                    .and_then(|value| {
                        usize::try_from(value).map_err(|_| {
                            CoreError::validation("web search limit exceeds platform range")
                        })
                    })
            })
            .transpose()?
            .unwrap_or(DEFAULT_WEB_SEARCH_LIMIT);
        if !(1..=20).contains(&limit) {
            return Err(CoreError::validation(
                "web search limit must be between 1 and 20",
            ));
        }

        let mut provider_context = self.base_context.clone();
        provider_context.provider_id = self.provider.definition().provider_id;
        provider_context.workflow_id = context.workflow_id.clone().or(provider_context.workflow_id);
        provider_context.run_id = context.run_id.clone().or(provider_context.run_id);
        provider_context.node_id = context.node_id.clone().or(provider_context.node_id);
        provider_context.tool_call_id = Some(call.tool_call_id.clone());
        provider_context.operation_id = provider_context.operation_id.map(|base| {
            format!(
                "{base}:web-search-round-{}:{}",
                context.round,
                stable_operation_component(&call.tool_call_id)
            )
        });
        provider_context.metadata = json!({
            "tool": call.name,
            "tool_call_id": call.tool_call_id,
            "round": context.round,
            "search_scope": "web",
        });

        let response = ProviderExecutor::new(self.ledger).search(
            self.provider,
            &provider_context,
            SearchProviderRequest {
                query: query.clone(),
                limit: Some(limit),
                metadata: provider_context.metadata.clone(),
            },
        )?;
        let result_count = response.results.len();
        Ok(ToolExecutionOutput {
            value: json!({
                "query": query,
                "count": result_count,
                "results": response.results,
                "persisted_to_knowledge": false,
            }),
            audit_metadata: json!({
                "tool": call.name,
                "search_scope": "web",
                "provider_id": provider_context.provider_id,
                "result_count": result_count,
                "persisted_to_knowledge": false,
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
