use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::{NodeId, RunId, WorkflowId};
use crate::costs::TokenUsage;

/// Provider 调用上下文，携带运行、节点、超时和重试信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderCallContext {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<WorkflowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub timeout_ms: u64,
    pub max_retries: u32,
    #[serde(default)]
    pub metadata: Value,
}

impl ProviderCallContext {
    /// 创建默认调用上下文。
    pub fn new(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            workflow_id: None,
            run_id: None,
            node_id: None,
            tool_call_id: None,
            timeout_ms: 60_000,
            max_retries: 2,
            metadata: Value::Null,
        }
    }
}

/// LLM 消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmRole {
    System,
    User,
    Assistant,
    Tool,
}

/// LLM 消息内容片段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    Json {
        value: Value,
    },
    ToolResult {
        tool_call_id: String,
        value: Value,
    },
    /// assistant 发起的工具调用；多轮 tool-use 必须把它随 assistant 消息回填，
    /// 否则后续 tool_result 会成为没有前置 tool_calls 的孤儿消息，被 provider 拒绝。
    ToolUse {
        tool_call_id: String,
        name: String,
        arguments: Value,
    },
}

impl ContentPart {
    /// 创建纯文本内容片段。
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

/// 标准化 LLM 消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: LlmRole,
    #[serde(default)]
    pub content: Vec<ContentPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl LlmMessage {
    /// 创建 user 消息。
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: LlmRole::User,
            content: vec![ContentPart::text(text)],
            name: None,
            tool_call_id: None,
        }
    }

    /// 创建 assistant 消息。
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: LlmRole::Assistant,
            content: vec![ContentPart::text(text)],
            name: None,
            tool_call_id: None,
        }
    }

    /// 创建携带 tool_calls 的 assistant 消息，供多轮 tool-use 回填上下文。
    /// 文本为空时不放入 Text 片段，避免给 provider 发送空 assistant 文本块。
    pub fn assistant_with_tool_calls(text: impl Into<String>, tool_calls: &[ToolCall]) -> Self {
        let mut content = Vec::new();
        let text = text.into();
        if !text.is_empty() {
            content.push(ContentPart::text(text));
        }
        for call in tool_calls {
            content.push(ContentPart::ToolUse {
                tool_call_id: call.tool_call_id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            });
        }
        Self {
            role: LlmRole::Assistant,
            content,
            name: None,
            tool_call_id: None,
        }
    }
}

/// Tool 定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// LLM 返回的工具调用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_call_id: String,
    pub name: String,
    pub arguments: Value,
}

/// 标准化 LLM 请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model_id: String,
    #[serde(default)]
    pub messages: Vec<LlmMessage>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub metadata: Value,
}

/// 标准化 LLM 响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmResponse {
    pub message: LlmMessage,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub raw: Value,
}

/// Embedding 请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model_id: String,
    pub inputs: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// Embedding 响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    pub embeddings: Vec<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub raw: Value,
}

/// Reranker 请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankRequest {
    pub model_id: String,
    pub query: String,
    pub documents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<usize>,
    #[serde(default)]
    pub metadata: Value,
}

/// Reranker 单条结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankResult {
    pub index: usize,
    pub score: f32,
}

/// Reranker 响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankResponse {
    pub results: Vec<RerankResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub raw: Value,
}

/// SearchProvider 请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchProviderRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub metadata: Value,
}

/// SearchProvider 单条结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchProviderResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub score: f32,
    #[serde(default)]
    pub metadata: Value,
}

/// SearchProvider 响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchProviderResponse {
    pub results: Vec<SearchProviderResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub raw: Value,
}
