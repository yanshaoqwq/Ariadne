use std::collections::BTreeSet;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::ProviderConfig;
use crate::contracts::{
    CoreError, CoreResult, ExternalDispatchOutcome, ProviderCapability, ProviderDefinition,
};
use crate::costs::{estimate_model_config_cost, TokenUsage};
use crate::providers::{
    ContentPart, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse, LlmMessage, LlmProvider,
    LlmRequest, LlmResponse, LlmRole, Provider, ProviderCallContext, ProviderHealth,
    ProviderProtocol, RerankRequest, RerankResponse, RerankResult, RerankerProvider,
    SearchProvider, SearchProviderRequest, SearchProviderResponse, SearchProviderResult, ToolCall,
};

const MAX_PROVIDER_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// HTTP LLM provider；按 ProviderType 分派 OpenAI/Anthropic/Gemini 请求格式。
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleLlmProvider {
    config: ProviderConfig,
    base_url: String,
    api_key: Option<String>,
    client: Client,
}

impl OpenAiCompatibleLlmProvider {
    /// Build a blocking HTTP provider from project config and resolved secret text.
    pub fn new(config: ProviderConfig, api_key: Option<String>) -> CoreResult<Self> {
        config.validate()?;
        let base_url = crate::providers::resolve_base_url(&config)?
            .trim_end_matches('/')
            .to_owned();
        let client = Client::builder().build().map_err(http_provider_error)?;
        Ok(Self {
            config,
            base_url,
            api_key,
            client,
        })
    }
}

/// 复用项目 Provider 配置的原生 Web 搜索 provider。
///
/// OpenAI/OpenAI-compatible 使用 Responses API `web_search`，Anthropic 使用服务端
/// `web_search_20250305`，Gemini 使用 `google_search` grounding。搜索结果只返回给
/// 调用节点，不自动写入项目知识库。
#[derive(Debug, Clone)]
pub struct HttpWebSearchProvider {
    config: ProviderConfig,
    base_url: String,
    api_key: Option<String>,
    model_id: String,
    client: Client,
}

impl HttpWebSearchProvider {
    pub fn new(config: ProviderConfig, api_key: Option<String>) -> CoreResult<Self> {
        config.validate()?;
        let base_url = crate::providers::resolve_base_url(&config)?
            .trim_end_matches('/')
            .to_owned();
        let model_id = config
            .models
            .iter()
            .find(|model| model.capability == ProviderCapability::Search)
            .or_else(|| {
                config
                    .models
                    .iter()
                    .find(|model| model.capability == ProviderCapability::Llm)
            })
            .map(|model| model.model_id.trim().to_owned())
            .filter(|model| !model.is_empty())
            .ok_or_else(|| {
                CoreError::validation(
                    "web search provider requires a search or llm capability model",
                )
            })?;
        let client = Client::builder().build().map_err(http_provider_error)?;
        Ok(Self {
            config,
            base_url,
            api_key,
            model_id,
            client,
        })
    }
}

impl Provider for HttpWebSearchProvider {
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: self.config.provider_id.clone(),
            provider_type: self.config.provider_type.clone(),
            display_name: self.config.display_name.clone(),
            capabilities: vec![ProviderCapability::Search],
            config_schema: Value::Null,
        }
    }

    fn health_check(&self) -> CoreResult<ProviderHealth> {
        provider_config_health(&self.config)
    }
}

impl SearchProvider for HttpWebSearchProvider {
    fn search(
        &self,
        context: &ProviderCallContext,
        request: SearchProviderRequest,
    ) -> CoreResult<SearchProviderResponse> {
        let query = request.query.trim();
        if query.is_empty() {
            return Err(CoreError::validation("web search query cannot be empty"));
        }
        let limit = request.limit.unwrap_or(8);
        if !(1..=20).contains(&limit) {
            return Err(CoreError::validation(
                "web search limit must be between 1 and 20",
            ));
        }
        let protocol = ProviderProtocol::from_provider_type(&self.config.provider_type)?;
        let (endpoint, payload) = match protocol {
            ProviderProtocol::OpenAi => (
                format!("{}/responses", self.base_url),
                json!({
                    "model": self.model_id,
                    "input": query,
                    "tools": [{ "type": "web_search" }],
                    "tool_choice": "auto",
                }),
            ),
            ProviderProtocol::Anthropic => (
                format!("{}/messages", self.base_url),
                json!({
                    "model": self.model_id,
                    "max_tokens": 2048,
                    "messages": [{ "role": "user", "content": query }],
                    "tools": [{
                        "type": "web_search_20250305",
                        "name": "web_search",
                        "max_uses": 1
                    }],
                }),
            ),
            ProviderProtocol::Gemini => (
                format!(
                    "{}/{}:generateContent",
                    self.base_url,
                    gemini_model_path(&self.model_id)
                ),
                json!({
                    "contents": [{ "role": "user", "parts": [{ "text": query }] }],
                    "tools": [{ "google_search": {} }],
                }),
            ),
        };
        let mut http_request = self.client.post(endpoint).json(&payload);
        if matches!(protocol, ProviderProtocol::Anthropic) {
            http_request = http_request.header("anthropic-beta", "web-search-2025-03-05");
        }
        let http_request =
            authorize_provider_request(protocol, self.api_key.as_deref(), http_request);
        let raw =
            execute_provider_json(http_request, context, &format!("{protocol:?} web search"))?;
        let results = match protocol {
            ProviderProtocol::OpenAi => parse_openai_web_search_results(&raw, limit),
            ProviderProtocol::Anthropic => parse_anthropic_web_search_results(&raw, limit),
            ProviderProtocol::Gemini => parse_gemini_web_search_results(&raw, limit),
        }?;
        Ok(SearchProviderResponse {
            results,
            cost_usd: None,
            raw,
        })
    }
}

fn parse_openai_web_search_results(
    raw: &Value,
    limit: usize,
) -> CoreResult<Vec<SearchProviderResult>> {
    let mut results = Vec::new();
    let mut seen = BTreeSet::new();
    let output = raw
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::validation("OpenAI web search response missing output"))?;
    for item in output {
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for part in content {
            let snippet = part.get("text").and_then(Value::as_str).unwrap_or_default();
            let Some(annotations) = part.get("annotations").and_then(Value::as_array) else {
                continue;
            };
            for annotation in annotations {
                let citation = annotation.get("url_citation").unwrap_or(annotation);
                let Some(url) = citation.get("url").and_then(Value::as_str) else {
                    continue;
                };
                let title = citation.get("title").and_then(Value::as_str).unwrap_or(url);
                push_web_search_result(
                    &mut results,
                    &mut seen,
                    title,
                    url,
                    snippet,
                    json!({ "provider": "openai" }),
                    limit,
                );
                if results.len() >= limit {
                    return Ok(results);
                }
            }
        }
    }
    Ok(results)
}

fn parse_anthropic_web_search_results(
    raw: &Value,
    limit: usize,
) -> CoreResult<Vec<SearchProviderResult>> {
    let content = raw
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::validation("Anthropic web search response missing content"))?;
    let summary = content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let mut results = Vec::new();
    let mut seen = BTreeSet::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("web_search_tool_result") {
            continue;
        }
        let Some(items) = block.get("content").and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(url) = item.get("url").and_then(Value::as_str) else {
                continue;
            };
            let title = item.get("title").and_then(Value::as_str).unwrap_or(url);
            push_web_search_result(
                &mut results,
                &mut seen,
                title,
                url,
                &summary,
                json!({
                    "provider": "anthropic",
                    "page_age": item.get("page_age").cloned().unwrap_or(Value::Null),
                }),
                limit,
            );
            if results.len() >= limit {
                return Ok(results);
            }
        }
    }
    Ok(results)
}

fn parse_gemini_web_search_results(
    raw: &Value,
    limit: usize,
) -> CoreResult<Vec<SearchProviderResult>> {
    let candidates = raw
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::validation("Gemini web search response missing candidates"))?;
    let mut results = Vec::new();
    let mut seen = BTreeSet::new();
    for candidate in candidates {
        let snippet = candidate
            .get("content")
            .and_then(|content| content.get("parts"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = candidate
            .get("groundingMetadata")
            .or_else(|| candidate.get("grounding_metadata"))
            .and_then(|metadata| {
                metadata
                    .get("groundingChunks")
                    .or_else(|| metadata.get("grounding_chunks"))
            })
            .and_then(Value::as_array);
        let Some(chunks) = chunks else {
            continue;
        };
        for chunk in chunks {
            let Some(web) = chunk.get("web") else {
                continue;
            };
            let Some(url) = web
                .get("uri")
                .or_else(|| web.get("url"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let title = web.get("title").and_then(Value::as_str).unwrap_or(url);
            push_web_search_result(
                &mut results,
                &mut seen,
                title,
                url,
                &snippet,
                json!({ "provider": "gemini" }),
                limit,
            );
            if results.len() >= limit {
                return Ok(results);
            }
        }
    }
    Ok(results)
}

fn push_web_search_result(
    results: &mut Vec<SearchProviderResult>,
    seen: &mut BTreeSet<String>,
    title: &str,
    url: &str,
    snippet: &str,
    metadata: Value,
    limit: usize,
) {
    if results.len() >= limit || url.trim().is_empty() || !seen.insert(url.to_owned()) {
        return;
    }
    results.push(SearchProviderResult {
        title: title.to_owned(),
        url: url.to_owned(),
        snippet: snippet.to_owned(),
        score: 1.0 / (results.len() as f32 + 1.0),
        metadata,
    });
}

impl Provider for OpenAiCompatibleLlmProvider {
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: self.config.provider_id.clone(),
            provider_type: self.config.provider_type.clone(),
            display_name: self.config.display_name.clone(),
            capabilities: vec![ProviderCapability::Llm],
            config_schema: Value::Null,
        }
    }

    fn health_check(&self) -> CoreResult<ProviderHealth> {
        if self.config.enabled {
            Ok(ProviderHealth::Healthy)
        } else {
            Ok(ProviderHealth::Unhealthy {
                reason: "provider is disabled".to_owned(),
            })
        }
    }
}

impl LlmProvider for OpenAiCompatibleLlmProvider {
    fn complete(
        &self,
        context: &ProviderCallContext,
        request: LlmRequest,
    ) -> CoreResult<LlmResponse> {
        let protocol = ProviderProtocol::from_provider_type(&self.config.provider_type)?;
        let (endpoint, payload) = self.request_envelope(protocol, &request)?;
        let http_request = authorize_provider_request(
            protocol,
            self.api_key.as_deref(),
            self.client.post(endpoint).json(&payload),
        );
        let raw = execute_provider_json(http_request, context, &format!("{protocol:?}"))?;
        let parsed = match protocol {
            ProviderProtocol::OpenAi => openai_chat_response(&self.config, &request.model_id, raw),
            ProviderProtocol::Anthropic => {
                anthropic_messages_response(&self.config, &request.model_id, raw)
            }
            ProviderProtocol::Gemini => {
                gemini_generate_content_response(&self.config, &request.model_id, raw)
            }
        };
        parsed.map_err(|error| {
            provider_request_error(
                &context.provider_id,
                ExternalDispatchOutcome::DispatchedUnknown,
                error,
            )
        })
    }
}

impl OpenAiCompatibleLlmProvider {
    fn request_envelope(
        &self,
        protocol: ProviderProtocol,
        request: &LlmRequest,
    ) -> CoreResult<(String, Value)> {
        match protocol {
            ProviderProtocol::OpenAi => Ok((
                format!("{}/chat/completions", self.base_url),
                openai_chat_request(request)?,
            )),
            ProviderProtocol::Anthropic => Ok((
                format!("{}/messages", self.base_url),
                anthropic_messages_request(request)?,
            )),
            ProviderProtocol::Gemini => Ok((
                gemini_generate_content_url(&self.base_url, &request.model_id),
                gemini_generate_content_request(request)?,
            )),
        }
    }
}

/// OpenAI-compatible / Gemini HTTP embedding provider。
#[derive(Debug, Clone)]
pub struct HttpEmbeddingProvider {
    config: ProviderConfig,
    base_url: String,
    api_key: Option<String>,
    client: Client,
}

impl HttpEmbeddingProvider {
    /// 从项目 provider 配置和已解析密钥创建 embedding provider。
    pub fn new(config: ProviderConfig, api_key: Option<String>) -> CoreResult<Self> {
        config.validate()?;
        let protocol = ProviderProtocol::from_provider_type(&config.provider_type)?;
        if matches!(protocol, ProviderProtocol::Anthropic) {
            return Err(CoreError::validation(
                "anthropic protocol does not define an embedding endpoint",
            ));
        }
        let base_url = crate::providers::resolve_base_url(&config)?
            .trim_end_matches('/')
            .to_owned();
        let client = Client::builder().build().map_err(http_provider_error)?;
        Ok(Self {
            config,
            base_url,
            api_key,
            client,
        })
    }
}

impl Provider for HttpEmbeddingProvider {
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: self.config.provider_id.clone(),
            provider_type: self.config.provider_type.clone(),
            display_name: self.config.display_name.clone(),
            capabilities: vec![ProviderCapability::Embedding],
            config_schema: Value::Null,
        }
    }

    fn health_check(&self) -> CoreResult<ProviderHealth> {
        provider_config_health(&self.config)
    }
}

impl EmbeddingProvider for HttpEmbeddingProvider {
    fn embed(
        &self,
        context: &ProviderCallContext,
        request: EmbeddingRequest,
    ) -> CoreResult<EmbeddingResponse> {
        validate_embedding_request(&request)?;
        let protocol = ProviderProtocol::from_provider_type(&self.config.provider_type)?;
        let (endpoint, payload) = match protocol {
            ProviderProtocol::OpenAi => (
                format!("{}/embeddings", self.base_url),
                json!({
                    "model": request.model_id,
                    "input": request.inputs,
                }),
            ),
            ProviderProtocol::Gemini => {
                let model = gemini_model_path(&request.model_id);
                let requests = request
                    .inputs
                    .iter()
                    .map(|input| {
                        json!({
                            "model": model,
                            "content": { "parts": [{ "text": input }] },
                        })
                    })
                    .collect::<Vec<_>>();
                (
                    format!("{}/{}:batchEmbedContents", self.base_url, model),
                    json!({ "requests": requests }),
                )
            }
            ProviderProtocol::Anthropic => {
                return Err(CoreError::validation(
                    "anthropic protocol does not define an embedding endpoint",
                ));
            }
        };
        let http_request = authorize_provider_request(
            protocol,
            self.api_key.as_deref(),
            self.client.post(endpoint).json(&payload),
        );
        let raw = execute_provider_json(http_request, context, &format!("{protocol:?}"))?;
        match protocol {
            ProviderProtocol::OpenAi => openai_embedding_response(
                &self.config,
                &request.model_id,
                request.inputs.len(),
                raw,
            ),
            ProviderProtocol::Gemini => gemini_embedding_response(
                &self.config,
                &request.model_id,
                request.inputs.len(),
                raw,
            ),
            ProviderProtocol::Anthropic => unreachable!("validated above"),
        }
        .map_err(|error| {
            provider_request_error(
                &context.provider_id,
                ExternalDispatchOutcome::DispatchedUnknown,
                error,
            )
        })
    }
}

/// OpenAI-compatible `/rerank` HTTP provider。
#[derive(Debug, Clone)]
pub struct HttpRerankerProvider {
    config: ProviderConfig,
    base_url: String,
    api_key: Option<String>,
    client: Client,
}

impl HttpRerankerProvider {
    /// 原生 OpenAI/Anthropic/Gemini 均没有本项目可依赖的统一 rerank 契约。
    pub fn new(config: ProviderConfig, api_key: Option<String>) -> CoreResult<Self> {
        config.validate()?;
        if !matches!(
            config.provider_type,
            crate::contracts::ProviderType::OpenAiCompatible
                | crate::contracts::ProviderType::Local
        ) {
            return Err(CoreError::validation(
                "reranker requires an open_ai_compatible or local provider",
            ));
        }
        let base_url = crate::providers::resolve_base_url(&config)?
            .trim_end_matches('/')
            .to_owned();
        let client = Client::builder().build().map_err(http_provider_error)?;
        Ok(Self {
            config,
            base_url,
            api_key,
            client,
        })
    }
}

impl Provider for HttpRerankerProvider {
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: self.config.provider_id.clone(),
            provider_type: self.config.provider_type.clone(),
            display_name: self.config.display_name.clone(),
            capabilities: vec![ProviderCapability::Reranker],
            config_schema: Value::Null,
        }
    }

    fn health_check(&self) -> CoreResult<ProviderHealth> {
        provider_config_health(&self.config)
    }
}

impl RerankerProvider for HttpRerankerProvider {
    fn rerank(
        &self,
        context: &ProviderCallContext,
        request: RerankRequest,
    ) -> CoreResult<RerankResponse> {
        validate_rerank_request(&request)?;
        let payload = json!({
            "model": request.model_id,
            "query": request.query,
            "documents": request.documents,
            "top_n": request.top_n,
        });
        let http_request = authorize_provider_request(
            ProviderProtocol::OpenAi,
            self.api_key.as_deref(),
            self.client
                .post(format!("{}/rerank", self.base_url))
                .json(&payload),
        );
        let raw = execute_provider_json(http_request, context, "OpenAiCompatibleRerank")?;
        openai_compatible_rerank_response(
            &self.config,
            &request.model_id,
            request.documents.len(),
            request.top_n,
            raw,
        )
        .map_err(|error| {
            provider_request_error(
                &context.provider_id,
                ExternalDispatchOutcome::DispatchedUnknown,
                error,
            )
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiChatMessage {
    role: &'static str,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    // assistant 消息回发时必须带上上一轮的 tool_calls，否则后续 role:"tool"
    // 消息会因为“没有对应的 assistant tool_calls”被 OpenAI 拒绝（HTTP 400）。
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChatChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatChoice {
    message: OpenAiAssistantMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiAssistantMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCall>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(default)]
    function: OpenAiFunctionCall,
}

#[derive(Debug, Default, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

fn openai_chat_request(request: &LlmRequest) -> CoreResult<Value> {
    let messages = request
        .messages
        .iter()
        .map(openai_message)
        .collect::<CoreResult<Vec<_>>>()?;
    let mut payload = json!({
        "model": request.model_id,
        "messages": messages,
        "stream": request.stream,
    });
    if let Some(temperature) = request.temperature {
        payload["temperature"] = json!(temperature);
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        payload["max_tokens"] = json!(max_output_tokens);
    }
    if !request.tools.is_empty() {
        payload["tools"] = json!(request
            .tools
            .iter()
            .map(|tool| json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                }
            }))
            .collect::<Vec<_>>());
    }
    Ok(payload)
}

fn openai_message(message: &LlmMessage) -> CoreResult<OpenAiChatMessage> {
    // assistant 的 tool_calls 从 content 里的 ToolUse 片段还原成 OpenAI 结构。
    let tool_calls = openai_tool_calls(message);
    Ok(OpenAiChatMessage {
        role: match message.role {
            LlmRole::System => "system",
            LlmRole::User => "user",
            LlmRole::Assistant => "assistant",
            LlmRole::Tool => "tool",
        },
        content: message_text(message)?,
        name: message.name.clone(),
        tool_call_id: message.tool_call_id.clone(),
        tool_calls,
    })
}

/// 把消息里的 ToolUse 片段转换成 OpenAI chat 的 tool_calls 数组。
fn openai_tool_calls(message: &LlmMessage) -> Option<Vec<Value>> {
    let calls = message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolUse {
                tool_call_id,
                name,
                arguments,
            } => Some(json!({
                "id": tool_call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_owned()),
                }
            })),
            _ => None,
        })
        .collect::<Vec<_>>();
    (!calls.is_empty()).then_some(calls)
}

fn message_text(message: &LlmMessage) -> CoreResult<String> {
    let mut parts = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => parts.push(text.clone()),
            ContentPart::Json { value } | ContentPart::ToolResult { value, .. } => {
                parts.push(serde_json::to_string(value)?);
            }
            // ToolUse 通过 tool_calls 字段单独回发，不再拼进文本内容。
            ContentPart::ToolUse { .. } => {}
        }
    }
    Ok(parts.join("\n"))
}

fn openai_chat_response(
    config: &ProviderConfig,
    requested_model_id: &str,
    raw: Value,
) -> CoreResult<LlmResponse> {
    let parsed: OpenAiChatResponse = serde_json::from_value(raw.clone())?;
    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::validation("provider returned no choices"))?;
    let text = choice.message.content.unwrap_or_default();
    let usage = parsed.usage.map(|usage| TokenUsage {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
    });
    let response_model_id = raw["model"].as_str().unwrap_or(requested_model_id);
    let cost_usd = response_cost_usd(config, response_model_id, usage);
    let tool_calls = choice
        .message
        .tool_calls
        .into_iter()
        .map(|call| {
            let arguments = serde_json::from_str(&call.function.arguments).map_err(|error| {
                CoreError::validation(format!(
                    "openai tool call {} ({}) returned invalid JSON arguments: {error}",
                    call.id, call.function.name
                ))
            })?;
            Ok(ToolCall {
                tool_call_id: call.id,
                name: call.function.name,
                arguments,
            })
        })
        .collect::<CoreResult<Vec<_>>>()?;
    Ok(LlmResponse {
        // assistant 消息同时携带文本和 tool_calls，保证多轮 tool-use 回填上下文时
        // 后续的 tool 结果消息能对应到前一条 assistant 的 tool_calls。
        message: LlmMessage::assistant_with_tool_calls(text, &tool_calls),
        tool_calls,
        usage,
        finish_reason: choice.finish_reason,
        cost_usd,
        raw,
    })
}

fn anthropic_messages_request(request: &LlmRequest) -> CoreResult<Value> {
    let mut messages = Vec::new();
    let mut system_parts = Vec::new();
    for message in &request.messages {
        match message.role {
            LlmRole::System => system_parts.push(message_text(message)?),
            LlmRole::User => messages.push(json!({
                "role": "user",
                "content": anthropic_content_blocks(message)?,
            })),
            LlmRole::Assistant => messages.push(json!({
                "role": "assistant",
                "content": anthropic_content_blocks(message)?,
            })),
            LlmRole::Tool => messages.push(json!({
                "role": "user",
                "content": anthropic_content_blocks(message)?,
            })),
        }
    }

    let mut payload = json!({
        "model": request.model_id,
        "messages": messages,
        "max_tokens": request.max_output_tokens.unwrap_or(4096),
    });
    if !system_parts.is_empty() {
        payload["system"] = json!(system_parts.join("\n\n"));
    }
    if let Some(temperature) = request.temperature {
        payload["temperature"] = json!(temperature);
    }
    if !request.tools.is_empty() {
        payload["tools"] = json!(request
            .tools
            .iter()
            .map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            }))
            .collect::<Vec<_>>());
    }
    Ok(payload)
}

fn anthropic_content_blocks(message: &LlmMessage) -> CoreResult<Vec<Value>> {
    let mut blocks = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => {
                if !text.is_empty() {
                    blocks.push(json!({ "type": "text", "text": text }));
                }
            }
            ContentPart::Json { value } => {
                blocks.push(json!({ "type": "text", "text": value.to_string() }));
            }
            ContentPart::ToolResult {
                tool_call_id,
                value,
            } => {
                blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": value.to_string(),
                }));
            }
            // assistant 的 tool_use 块必须原样回发，Anthropic 才能把后续 user 里的
            // tool_result 关联到本次工具调用；缺失会被判成孤儿 tool_result。
            ContentPart::ToolUse {
                tool_call_id,
                name,
                arguments,
            } => {
                blocks.push(json!({
                    "type": "tool_use",
                    "id": tool_call_id,
                    "name": name,
                    "input": arguments,
                }));
            }
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({ "type": "text", "text": "" }));
    }
    Ok(blocks)
}

fn anthropic_messages_response(
    config: &ProviderConfig,
    requested_model_id: &str,
    raw: Value,
) -> CoreResult<LlmResponse> {
    let content = raw["content"]
        .as_array()
        .ok_or_else(|| CoreError::validation("anthropic provider returned no content"))?;
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    for block in content {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(text) = block["text"].as_str() {
                    text_parts.push(text.to_owned());
                }
            }
            Some("tool_use") => {
                let Some(id) = block["id"].as_str() else {
                    continue;
                };
                let Some(name) = block["name"].as_str() else {
                    continue;
                };
                tool_calls.push(ToolCall {
                    tool_call_id: id.to_owned(),
                    name: name.to_owned(),
                    arguments: block["input"].clone(),
                });
            }
            _ => {}
        }
    }
    let usage = anthropic_usage(&raw);
    let response_model_id = raw["model"].as_str().unwrap_or(requested_model_id);
    Ok(LlmResponse {
        // 保留 tool_use 块到 assistant 消息，多轮 tool-use 回填时才能关联 tool_result。
        message: LlmMessage::assistant_with_tool_calls(text_parts.join("\n"), &tool_calls),
        tool_calls,
        usage,
        finish_reason: raw["stop_reason"].as_str().map(str::to_owned),
        cost_usd: response_cost_usd(config, response_model_id, usage),
        raw,
    })
}

fn anthropic_usage(raw: &Value) -> Option<TokenUsage> {
    let usage = raw["usage"].as_object()?;
    Some(TokenUsage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        output_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    })
}

fn gemini_generate_content_url(base_url: &str, model_id: &str) -> String {
    let model_path = gemini_model_path(model_id);
    format!("{base_url}/{model_path}:generateContent")
}

fn gemini_model_path(model_id: &str) -> String {
    if model_id.starts_with("models/") {
        model_id.to_owned()
    } else {
        format!("models/{model_id}")
    }
}

fn gemini_generate_content_request(request: &LlmRequest) -> CoreResult<Value> {
    let mut contents = Vec::new();
    let mut system_parts = Vec::new();
    for message in &request.messages {
        match message.role {
            LlmRole::System => system_parts.push(message_text(message)?),
            LlmRole::User => contents.push(json!({
                "role": "user",
                "parts": gemini_parts(message)?,
            })),
            LlmRole::Assistant => contents.push(json!({
                "role": "model",
                "parts": gemini_parts(message)?,
            })),
            LlmRole::Tool => contents.push(json!({
                "role": "user",
                "parts": gemini_parts(message)?,
            })),
        }
    }

    let mut payload = json!({
        "contents": contents,
    });
    if !system_parts.is_empty() {
        payload["systemInstruction"] = json!({
            "parts": [{ "text": system_parts.join("\n\n") }]
        });
    }

    let mut generation_config = serde_json::Map::new();
    if let Some(temperature) = request.temperature {
        generation_config.insert("temperature".to_owned(), json!(temperature));
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        generation_config.insert("maxOutputTokens".to_owned(), json!(max_output_tokens));
    }
    if !generation_config.is_empty() {
        payload["generationConfig"] = Value::Object(generation_config);
    }
    if !request.tools.is_empty() {
        payload["tools"] = json!([{
            "functionDeclarations": request
                .tools
                .iter()
                .map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": gemini_schema(&tool.input_schema),
                }))
                .collect::<Vec<_>>()
        }]);
    }
    Ok(payload)
}

fn gemini_parts(message: &LlmMessage) -> CoreResult<Vec<Value>> {
    let mut parts = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text { text } => {
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
            }
            ContentPart::Json { value } => {
                parts.push(json!({ "text": value.to_string() }));
            }
            ContentPart::ToolUse {
                name, arguments, ..
            } => {
                // Gemini 用 functionCall 表达模型发起的工具调用；多轮回填时按函数名匹配，
                // 没有独立的 tool_call_id 概念，因此这里只回发 name 和 args。
                parts.push(json!({
                    "functionCall": {
                        "name": name,
                        "args": arguments,
                    }
                }));
            }
            ContentPart::ToolResult { value, .. } => {
                parts.push(json!({
                    "functionResponse": {
                        "name": message.name.as_deref().unwrap_or("tool"),
                        "response": value,
                    }
                }));
            }
        }
    }
    if parts.is_empty() {
        parts.push(json!({ "text": "" }));
    }
    Ok(parts)
}

fn gemini_generate_content_response(
    config: &ProviderConfig,
    requested_model_id: &str,
    raw: Value,
) -> CoreResult<LlmResponse> {
    let candidate = raw["candidates"]
        .as_array()
        .and_then(|candidates| candidates.first())
        .ok_or_else(|| CoreError::validation("gemini provider returned no candidates"))?;
    let parts = candidate["content"]["parts"]
        .as_array()
        .ok_or_else(|| CoreError::validation("gemini provider returned no content parts"))?;
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if let Some(text) = part["text"].as_str() {
            text_parts.push(text.to_owned());
        }
        if let Some(call) = part["functionCall"].as_object() {
            let Some(name) = call.get("name").and_then(Value::as_str) else {
                continue;
            };
            tool_calls.push(ToolCall {
                tool_call_id: format!("gemini-tool-{index}"),
                name: name.to_owned(),
                arguments: call.get("args").cloned().unwrap_or(Value::Null),
            });
        }
    }
    let usage = gemini_usage(&raw);
    Ok(LlmResponse {
        // 携带 tool_calls，Gemini 多轮 tool-use 回填时才能重建 functionCall parts。
        message: LlmMessage::assistant_with_tool_calls(text_parts.join("\n"), &tool_calls),
        tool_calls,
        usage,
        finish_reason: candidate["finishReason"].as_str().map(str::to_owned),
        cost_usd: response_cost_usd(config, requested_model_id, usage),
        raw,
    })
}

fn gemini_usage(raw: &Value) -> Option<TokenUsage> {
    let usage = raw["usageMetadata"].as_object()?;
    Some(TokenUsage {
        input_tokens: usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        output_tokens: usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    })
}

fn gemini_schema(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut converted = serde_json::Map::new();
            for (key, value) in map {
                if key == "type" {
                    if let Some(kind) = value.as_str() {
                        converted.insert(key.clone(), Value::String(kind.to_ascii_uppercase()));
                        continue;
                    }
                }
                converted.insert(key.clone(), gemini_schema(value));
            }
            Value::Object(converted)
        }
        Value::Array(values) => Value::Array(values.iter().map(gemini_schema).collect()),
        _ => value.clone(),
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingEnvelope {
    data: Vec<OpenAiEmbeddingDatum>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingDatum {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct EmbeddingUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct GeminiEmbeddingEnvelope {
    embeddings: Vec<GeminiEmbeddingDatum>,
}

#[derive(Debug, Deserialize)]
struct GeminiEmbeddingDatum {
    values: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct CompatibleRerankEnvelope {
    results: Vec<CompatibleRerankResult>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
struct CompatibleRerankResult {
    index: usize,
    #[serde(alias = "score")]
    relevance_score: f32,
}

fn validate_embedding_request(request: &EmbeddingRequest) -> CoreResult<()> {
    if request.model_id.trim().is_empty() {
        return Err(CoreError::validation(
            "embedding request model_id cannot be empty",
        ));
    }
    if request.inputs.is_empty() {
        return Err(CoreError::validation(
            "embedding request inputs cannot be empty",
        ));
    }
    if request.inputs.iter().any(|input| input.trim().is_empty()) {
        return Err(CoreError::validation(
            "embedding request inputs cannot contain empty text",
        ));
    }
    Ok(())
}

fn validate_rerank_request(request: &RerankRequest) -> CoreResult<()> {
    if request.model_id.trim().is_empty() {
        return Err(CoreError::validation(
            "rerank request model_id cannot be empty",
        ));
    }
    if request.query.trim().is_empty() {
        return Err(CoreError::validation("rerank query cannot be empty"));
    }
    if request.documents.is_empty() {
        return Err(CoreError::validation("rerank documents cannot be empty"));
    }
    if request
        .documents
        .iter()
        .any(|document| document.trim().is_empty())
    {
        return Err(CoreError::validation(
            "rerank documents cannot contain empty text",
        ));
    }
    if request
        .top_n
        .is_some_and(|top_n| top_n == 0 || top_n > request.documents.len())
    {
        return Err(CoreError::validation(
            "rerank top_n must be between one and the document count",
        ));
    }
    Ok(())
}

fn openai_embedding_response(
    config: &ProviderConfig,
    requested_model_id: &str,
    expected_count: usize,
    raw: Value,
) -> CoreResult<EmbeddingResponse> {
    let mut parsed: OpenAiEmbeddingEnvelope = serde_json::from_value(raw.clone())?;
    parsed.data.sort_by_key(|item| item.index);
    for (expected_index, item) in parsed.data.iter().enumerate() {
        if item.index != expected_index {
            return Err(CoreError::validation(format!(
                "embedding provider returned non-contiguous index {}; expected {expected_index}",
                item.index
            )));
        }
    }
    let embeddings = parsed
        .data
        .into_iter()
        .map(|item| item.embedding)
        .collect::<Vec<_>>();
    validate_embedding_vectors(expected_count, &embeddings)?;
    let usage = parsed.usage.map(embedding_token_usage);
    let response_model_id = parsed.model.as_deref().unwrap_or(requested_model_id);
    Ok(EmbeddingResponse {
        embeddings,
        usage,
        cost_usd: response_cost_usd(config, response_model_id, usage),
        raw,
    })
}

fn gemini_embedding_response(
    config: &ProviderConfig,
    requested_model_id: &str,
    expected_count: usize,
    raw: Value,
) -> CoreResult<EmbeddingResponse> {
    let parsed: GeminiEmbeddingEnvelope = serde_json::from_value(raw.clone())?;
    let embeddings = parsed
        .embeddings
        .into_iter()
        .map(|item| item.values)
        .collect::<Vec<_>>();
    validate_embedding_vectors(expected_count, &embeddings)?;
    let usage = raw
        .get("usageMetadata")
        .and_then(|value| serde_json::from_value::<EmbeddingUsage>(value.clone()).ok())
        .map(embedding_token_usage);
    Ok(EmbeddingResponse {
        embeddings,
        usage,
        cost_usd: response_cost_usd(config, requested_model_id, usage),
        raw,
    })
}

fn validate_embedding_vectors(expected_count: usize, embeddings: &[Vec<f32>]) -> CoreResult<()> {
    if embeddings.len() != expected_count {
        return Err(CoreError::validation(format!(
            "embedding provider returned {} vectors for {expected_count} inputs",
            embeddings.len()
        )));
    }
    let dimensions = embeddings
        .first()
        .map(Vec::len)
        .ok_or_else(|| CoreError::validation("embedding provider returned no vectors"))?;
    if dimensions == 0 {
        return Err(CoreError::validation(
            "embedding provider returned an empty vector",
        ));
    }
    for embedding in embeddings {
        if embedding.len() != dimensions {
            return Err(CoreError::validation(
                "embedding provider returned inconsistent vector dimensions",
            ));
        }
        if embedding.iter().any(|value| !value.is_finite()) {
            return Err(CoreError::validation(
                "embedding provider returned a non-finite vector value",
            ));
        }
    }
    Ok(())
}

fn openai_compatible_rerank_response(
    config: &ProviderConfig,
    requested_model_id: &str,
    document_count: usize,
    top_n: Option<usize>,
    raw: Value,
) -> CoreResult<RerankResponse> {
    let parsed: CompatibleRerankEnvelope = serde_json::from_value(raw.clone())?;
    let maximum_results = top_n.unwrap_or(document_count);
    if parsed.results.len() > maximum_results {
        return Err(CoreError::validation(format!(
            "reranker returned {} results, exceeding requested maximum {maximum_results}",
            parsed.results.len()
        )));
    }
    let mut seen = BTreeSet::new();
    let mut results = Vec::with_capacity(parsed.results.len());
    for result in parsed.results {
        if result.index >= document_count {
            return Err(CoreError::validation(format!(
                "reranker returned out-of-range document index {}",
                result.index
            )));
        }
        if !seen.insert(result.index) {
            return Err(CoreError::validation(format!(
                "reranker returned duplicate document index {}",
                result.index
            )));
        }
        if !result.relevance_score.is_finite() {
            return Err(CoreError::validation(
                "reranker returned a non-finite relevance score",
            ));
        }
        results.push(RerankResult {
            index: result.index,
            score: result.relevance_score,
        });
    }
    let usage = parsed.usage.map(embedding_token_usage);
    let response_model_id = parsed.model.as_deref().unwrap_or(requested_model_id);
    Ok(RerankResponse {
        results,
        cost_usd: response_cost_usd(config, response_model_id, usage),
        raw,
    })
}

fn embedding_token_usage(usage: EmbeddingUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: if usage.prompt_tokens > 0 {
            usage.prompt_tokens
        } else {
            usage.total_tokens
        },
        output_tokens: 0,
    }
}

fn provider_config_health(config: &ProviderConfig) -> CoreResult<ProviderHealth> {
    if config.enabled {
        Ok(ProviderHealth::Healthy)
    } else {
        Ok(ProviderHealth::Unhealthy {
            reason: "provider is disabled".to_owned(),
        })
    }
}

fn authorize_provider_request(
    protocol: ProviderProtocol,
    api_key: Option<&str>,
    request: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) else {
        return match protocol {
            ProviderProtocol::Anthropic => request.header("anthropic-version", "2023-06-01"),
            _ => request,
        };
    };

    match protocol {
        ProviderProtocol::OpenAi => request.bearer_auth(api_key),
        ProviderProtocol::Anthropic => request
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
        // Gemini REST API 以 query parameter 接收 key；生产部署应避免代理记录完整 URL。
        ProviderProtocol::Gemini => request.query(&[("key", api_key)]),
    }
}

fn execute_provider_json(
    mut request: reqwest::RequestBuilder,
    context: &ProviderCallContext,
    protocol_name: &str,
) -> CoreResult<Value> {
    // per-call 超时优先生效；context.timeout_ms 为 0 时沿用 client 默认值。
    if context.timeout_ms > 0 {
        request = request.timeout(Duration::from_millis(context.timeout_ms));
    }
    if context.cancellation.is_cancelled() {
        return Err(CoreError::external_cancelled(
            &context.provider_id,
            ExternalDispatchOutcome::NotDispatched,
        ));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(http_provider_error)?;
    let response = runtime.block_on(send_provider_request(
        request,
        context,
        &context.provider_id,
    ))?;
    let status = response.status();
    // 错误响应体可能是 HTML；先保留状态码和原始文本，避免 JSON 解码掩盖原因。
    if !status.is_success() {
        let body = runtime.block_on(read_provider_response_body(
            response,
            &context.provider_id,
            &context.cancellation,
            ExternalDispatchOutcome::ResponseReceived,
        ))?;
        return Err(provider_request_error(
            &context.provider_id,
            ExternalDispatchOutcome::ResponseReceived,
            format!("{protocol_name} provider returned {status}: {body}"),
        ));
    }
    let body = runtime.block_on(read_provider_response_body(
        response,
        &context.provider_id,
        &context.cancellation,
        ExternalDispatchOutcome::DispatchedUnknown,
    ))?;
    serde_json::from_str::<Value>(&body).map_err(|error| {
        provider_request_error(
            &context.provider_id,
            ExternalDispatchOutcome::DispatchedUnknown,
            error,
        )
    })
}

fn response_cost_usd(
    config: &ProviderConfig,
    response_model_id: &str,
    usage: Option<TokenUsage>,
) -> Option<f64> {
    usage.and_then(|usage| {
        config
            .models
            .iter()
            .find(|model| model.model_id == response_model_id)
            .and_then(|model| estimate_model_config_cost(model, usage).ok())
    })
}

async fn send_provider_request(
    request: reqwest::RequestBuilder,
    context: &ProviderCallContext,
    service: &str,
) -> CoreResult<reqwest::Response> {
    let cancellation = context.cancellation.clone();
    let mut send = Box::pin(request.send());
    loop {
        match tokio::time::timeout(Duration::from_millis(25), &mut send).await {
            Ok(result) => {
                return result.map_err(|error| {
                    let outcome = if error.is_builder() || error.is_connect() {
                        ExternalDispatchOutcome::NotDispatched
                    } else {
                        ExternalDispatchOutcome::DispatchedUnknown
                    };
                    provider_request_error(service, outcome, error)
                });
            }
            Err(_) => {
                if cancellation.is_cancelled() {
                    return Err(CoreError::external_cancelled(
                        service,
                        ExternalDispatchOutcome::DispatchedUnknown,
                    ));
                }
            }
        }
    }
}

async fn read_provider_response_body(
    mut response: reqwest::Response,
    service: &str,
    cancellation: &crate::contracts::ExecutionCancellation,
    failure_outcome: ExternalDispatchOutcome,
) -> CoreResult<String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES)
    {
        return Err(provider_request_error(
            service,
            failure_outcome,
            format!("provider_response exceeds {MAX_PROVIDER_RESPONSE_BYTES} bytes"),
        ));
    }

    let mut bytes = Vec::new();
    loop {
        let chunk = match tokio::time::timeout(Duration::from_millis(25), response.chunk()).await {
            Ok(result) => {
                result.map_err(|error| provider_request_error(service, failure_outcome, error))?
            }
            Err(_) => {
                if cancellation.is_cancelled() {
                    return Err(CoreError::external_cancelled(service, failure_outcome));
                }
                continue;
            }
        };
        let Some(chunk) = chunk else { break };
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_PROVIDER_RESPONSE_BYTES {
            return Err(provider_request_error(
                service,
                failure_outcome,
                format!("provider_response exceeds {MAX_PROVIDER_RESPONSE_BYTES} bytes"),
            ));
        }
    }
    if bytes.len() as u64 > MAX_PROVIDER_RESPONSE_BYTES {
        return Err(provider_request_error(
            service,
            failure_outcome,
            format!("provider_response exceeds {MAX_PROVIDER_RESPONSE_BYTES} bytes"),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| provider_request_error(service, failure_outcome, error))
}

fn http_provider_error(message: impl std::fmt::Display) -> CoreError {
    CoreError::External {
        service: "openai_compatible_provider".to_owned(),
        message: message.to_string(),
    }
}

fn provider_request_error(
    service: &str,
    outcome: ExternalDispatchOutcome,
    message: impl std::fmt::Display,
) -> CoreError {
    CoreError::ProviderRequest {
        service: service.to_owned(),
        outcome,
        message: message.to_string(),
    }
}
