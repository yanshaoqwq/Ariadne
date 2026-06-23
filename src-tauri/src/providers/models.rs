use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{NodeId, RunId, WorkflowId};
use crate::costs::TokenUsage;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Json { value: Value },
    ToolResult { tool_call_id: String, value: Value },
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

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
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: LlmRole::User,
            content: vec![ContentPart::text(text)],
            name: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: LlmRole::Assistant,
            content: vec![ContentPart::text(text)],
            name: None,
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_call_id: String,
    pub name: String,
    pub arguments: Value,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model_id: String,
    pub inputs: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankResult {
    pub index: usize,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankResponse {
    pub results: Vec<RerankResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchProviderRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchProviderResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub score: f32,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchProviderResponse {
    pub results: Vec<SearchProviderResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub raw: Value,
}
