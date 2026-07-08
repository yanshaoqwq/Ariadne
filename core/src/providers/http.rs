use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::ProviderConfig;
use crate::contracts::{CoreError, CoreResult, ProviderCapability, ProviderDefinition};
use crate::costs::{estimate_model_config_cost, TokenUsage};
use crate::providers::{
    ContentPart, LlmMessage, LlmProvider, LlmRequest, LlmResponse, LlmRole, Provider,
    ProviderCallContext, ProviderHealth, ProviderProtocol, ToolCall,
};

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
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(http_provider_error)?;
        Ok(Self {
            config,
            base_url,
            api_key,
            client,
        })
    }
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
        let mut http_request = self.authorize(protocol, self.client.post(endpoint).json(&payload));
        // per-call 超时优先生效；context.timeout_ms 为 0 时沿用 client 的默认超时。
        if context.timeout_ms > 0 {
            http_request = http_request.timeout(Duration::from_millis(context.timeout_ms));
        }
        let response = http_request.send().map_err(http_provider_error)?;
        let status = response.status();
        // 先取原始文本再解析：错误响应体可能不是 JSON（如网关返回 HTML），
        // 若先 .json() 会用解码错误掩盖真实的 4xx/5xx 状态码。
        let body = response.text().map_err(http_provider_error)?;
        if !status.is_success() {
            return Err(CoreError::External {
                service: context.provider_id.clone(),
                message: format!("{protocol:?} provider returned {status}: {body}"),
            });
        }
        let raw = serde_json::from_str::<Value>(&body).map_err(http_provider_error)?;
        match protocol {
            ProviderProtocol::OpenAi => openai_chat_response(&self.config, &request.model_id, raw),
            ProviderProtocol::Anthropic => {
                anthropic_messages_response(&self.config, &request.model_id, raw)
            }
            ProviderProtocol::Gemini => {
                gemini_generate_content_response(&self.config, &request.model_id, raw)
            }
        }
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

    fn authorize(
        &self,
        protocol: ProviderProtocol,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        let Some(api_key) = self.api_key.as_deref().filter(|value| !value.is_empty()) else {
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
            // ⚠️ Gemini API 要求将 key 作为 URL query parameter 传递，
            // 这意味着 API Key 可能被记录在服务器访问日志、代理日志等中。
            // 建议通过反向代理中转，或在网络层面限制日志记录。
            ProviderProtocol::Gemini => request.query(&[("key", api_key)]),
        }
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
    let model_path = if model_id.starts_with("models/") {
        model_id.to_owned()
    } else {
        format!("models/{model_id}")
    };
    format!("{base_url}/{model_path}:generateContent")
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

fn http_provider_error(message: impl std::fmt::Display) -> CoreError {
    CoreError::External {
        service: "openai_compatible_provider".to_owned(),
        message: message.to_string(),
    }
}
