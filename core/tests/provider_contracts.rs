use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use ariadne::config::{ModelConfig, ProviderConfig};
use ariadne::contracts::{CoreResult, ProviderCapability, ProviderDefinition, ProviderType, RunId};
use ariadne::costs::{CostLedger, CostQuery, SqliteCostLedger, TokenUsage};
use ariadne::providers::{
    resolve_base_url, LlmMessage, LlmProvider, LlmRequest, LlmResponse,
    OpenAiCompatibleLlmProvider, Provider, ProviderCallContext, ProviderExecutor, ProviderHealth,
    ProviderKind, ProviderProtocol, ProviderRuntimeRegistry, ToolUseEnvelope,
};
use serde_json::{json, Value};

#[derive(Clone)]
struct MockLlmProvider {
    provider_id: String,
    response_text: String,
    cost_usd: Option<f64>,
}

impl Provider for MockLlmProvider {
    /// 返回测试 provider 的基础定义。
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: self.provider_id.clone(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: self.provider_id.clone(),
            capabilities: vec![ProviderCapability::Llm, ProviderCapability::ToolUse],
            config_schema: Value::Null,
        }
    }

    /// 测试 provider 始终报告健康。
    fn health_check(&self) -> CoreResult<ProviderHealth> {
        Ok(ProviderHealth::Healthy)
    }
}

impl LlmProvider for MockLlmProvider {
    /// 返回预设 LLM 响应并附带可选成本。
    fn complete(
        &self,
        _context: &ProviderCallContext,
        _request: LlmRequest,
    ) -> CoreResult<LlmResponse> {
        Ok(LlmResponse {
            message: LlmMessage::assistant(self.response_text.clone()),
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 20,
            }),
            finish_reason: Some("stop".to_owned()),
            cost_usd: self.cost_usd,
            raw: json!({ "mock": true }),
        })
    }
}

#[test]
fn llm_providers_can_be_switched_by_provider_id() {
    let mut registry = ProviderRuntimeRegistry::default();
    registry
        .register_llm(
            "fast",
            Arc::new(MockLlmProvider {
                provider_id: "fast".to_owned(),
                response_text: "fast-response".to_owned(),
                cost_usd: None,
            }),
        )
        .unwrap();
    registry
        .register_llm(
            "quality",
            Arc::new(MockLlmProvider {
                provider_id: "quality".to_owned(),
                response_text: "quality-response".to_owned(),
                cost_usd: None,
            }),
        )
        .unwrap();

    let provider = registry.llm("quality").unwrap();
    let response = provider
        .complete(
            &ProviderCallContext::new("quality"),
            LlmRequest {
                model_id: "model".to_owned(),
                messages: vec![LlmMessage::user("Draft")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap();

    assert_eq!(response.message, LlmMessage::assistant("quality-response"));
}

#[test]
fn provider_executor_records_llm_costs() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let executor = ProviderExecutor::new(&ledger);
    let provider = MockLlmProvider {
        provider_id: "mock".to_owned(),
        response_text: "ok".to_owned(),
        cost_usd: Some(0.12),
    };
    let mut context = ProviderCallContext::new("mock");
    context.run_id = Some(RunId::new("run-1"));

    executor
        .complete_llm(
            &provider,
            &context,
            LlmRequest {
                model_id: "model".to_owned(),
                messages: vec![LlmMessage::user("Hello")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap();

    let total = ledger
        .total_cost(&CostQuery {
            run_id: Some(RunId::new("run-1")),
            ..CostQuery::default()
        })
        .unwrap();

    assert_eq!(total, 0.12);
}

#[test]
fn openai_compatible_http_provider_posts_chat_completions() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("\"model\":\"test-model\""));
        assert!(request.contains("\"content\":\"改写这段\""));
        let response_body = r#"{
          "model":"test-model",
          "choices":[{"message":{"content":"改写完成"},"finish_reason":"stop"}],
          "usage":{"prompt_tokens":10,"completion_tokens":5}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "local".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "test-model".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(1.0),
                output_cost_per_million_tokens: Some(2.0),
            }],
        },
        None,
    )
    .unwrap();

    let response = provider
        .complete(
            &ProviderCallContext::new("local"),
            LlmRequest {
                model_id: "test-model".to_owned(),
                messages: vec![LlmMessage::user("改写这段")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(response.message, LlmMessage::assistant("改写完成"));
    assert_eq!(response.usage.unwrap().input_tokens, 10);
    assert_eq!(response.cost_usd, Some(0.00002));
}

#[test]
fn openai_compatible_http_provider_rejects_oversized_streaming_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        let response_header =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
        stream.write_all(response_header.as_bytes()).unwrap();
        stream.write_all(&vec![b' '; 16 * 1024 * 1024 + 1]).unwrap();
    });

    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "local".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Local".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "test-model".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
        },
        None,
    )
    .unwrap();

    let error = provider
        .complete(
            &ProviderCallContext::new("local"),
            LlmRequest {
                model_id: "test-model".to_owned(),
                messages: vec![LlmMessage::user("改写这段")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap_err();
    server.join().unwrap();

    let error_text = format!("{error:?}");
    assert!(error_text.contains("provider_response"));
    assert!(error_text.contains("response exceeds"));
}

#[test]
fn openai_http_provider_uses_chat_completions_and_bearer_auth() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("authorization: Bearer sk-test"));
        assert!(request.contains("\"model\":\"gpt-test\""));
        assert!(request.contains("\"content\":\"续写这段\""));
        let response_body = r#"{
          "model":"gpt-test",
          "choices":[{"message":{"content":"续写完成"},"finish_reason":"stop"}],
          "usage":{"prompt_tokens":8,"completion_tokens":4}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "gpt-test".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(1.0),
                output_cost_per_million_tokens: Some(2.0),
            }],
        },
        Some("sk-test".to_owned()),
    )
    .unwrap();

    let response = provider
        .complete(
            &ProviderCallContext::new("openai"),
            LlmRequest {
                model_id: "gpt-test".to_owned(),
                messages: vec![LlmMessage::user("续写这段")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(response.message, LlmMessage::assistant("续写完成"));
    assert_eq!(response.usage.unwrap().output_tokens, 4);
    assert_eq!(response.cost_usd, Some(0.000016));
}

#[test]
fn openai_http_provider_rejects_invalid_tool_call_arguments() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let _ = stream.read(&mut buffer).unwrap();
        let response_body = r#"{
          "model":"gpt-test",
          "choices":[{
            "message":{
              "content":"",
              "tool_calls":[{
                "id":"call-1",
                "type":"function",
                "function":{"name":"lookup","arguments":"{bad json"}
              }]
            },
            "finish_reason":"tool_calls"
          }]
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: Vec::new(),
        },
        Some("sk-test".to_owned()),
    )
    .unwrap();

    let error = provider
        .complete(
            &ProviderCallContext::new("openai"),
            LlmRequest {
                model_id: "gpt-test".to_owned(),
                messages: vec![LlmMessage::user("调用工具")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap_err();
    server.join().unwrap();

    assert!(error.to_string().contains("invalid JSON arguments"));
    assert!(error.to_string().contains("call-1"));
    assert!(error.to_string().contains("lookup"));
}

#[test]
fn anthropic_http_provider_posts_messages_format() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /messages "));
        assert!(request.contains("x-api-key: claude-key"));
        assert!(request.contains("anthropic-version: 2023-06-01"));
        assert!(request.contains("\"model\":\"claude-test\""));
        assert!(request.contains("\"role\":\"user\""));
        assert!(request.contains("\"text\":\"润色这段\""));
        assert!(request.contains("\"max_tokens\":4096"));
        let response_body = r#"{
          "model":"claude-test",
          "content":[{"type":"text","text":"润色完成"}],
          "stop_reason":"end_turn",
          "usage":{"input_tokens":12,"output_tokens":6}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "anthropic".to_owned(),
            provider_type: ProviderType::Anthropic,
            display_name: "Anthropic".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "claude-test".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(3.0),
                output_cost_per_million_tokens: Some(15.0),
            }],
        },
        Some("claude-key".to_owned()),
    )
    .unwrap();

    let response = provider
        .complete(
            &ProviderCallContext::new("anthropic"),
            LlmRequest {
                model_id: "claude-test".to_owned(),
                messages: vec![LlmMessage::user("润色这段")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(response.message, LlmMessage::assistant("润色完成"));
    assert_eq!(response.finish_reason, Some("end_turn".to_owned()));
    assert_eq!(response.usage.unwrap().input_tokens, 12);
    assert_eq!(response.cost_usd, Some(0.000126));
}

#[test]
fn gemini_http_provider_posts_generate_content_format() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /models/gemini-test:generateContent?key=gemini-key "));
        assert!(request.contains("\"contents\""));
        assert!(request.contains("\"role\":\"user\""));
        assert!(request.contains("\"text\":\"总结这段\""));
        let response_body = r#"{
          "candidates":[{
            "content":{"role":"model","parts":[{"text":"总结完成"}]},
            "finishReason":"STOP"
          }],
          "usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "gemini".to_owned(),
            provider_type: ProviderType::Gemini,
            display_name: "Gemini".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "gemini-test".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(0.5),
                output_cost_per_million_tokens: Some(1.5),
            }],
        },
        Some("gemini-key".to_owned()),
    )
    .unwrap();

    let response = provider
        .complete(
            &ProviderCallContext::new("gemini"),
            LlmRequest {
                model_id: "gemini-test".to_owned(),
                messages: vec![LlmMessage::user("总结这段")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(response.message, LlmMessage::assistant("总结完成"));
    assert_eq!(response.finish_reason, Some("STOP".to_owned()));
    assert_eq!(response.usage.unwrap().output_tokens, 3);
    assert_eq!(response.cost_usd, Some(0.000008));
}

#[test]
fn provider_protocol_encapsulates_tool_use_differences() {
    assert_eq!(
        ProviderProtocol::Anthropic.tool_use_envelope(),
        ToolUseEnvelope::AnthropicTools
    );
    assert_eq!(
        ProviderProtocol::Gemini.tool_use_envelope(),
        ToolUseEnvelope::GeminiFunctionDeclarations
    );
}

#[test]
fn provider_registry_exposes_lifecycle_reports() {
    let mut registry = ProviderRuntimeRegistry::default();
    registry
        .register_llm(
            "healthy",
            Arc::new(MockLlmProvider {
                provider_id: "healthy".to_owned(),
                response_text: "ok".to_owned(),
                cost_usd: None,
            }),
        )
        .unwrap();

    let init_reports = registry.initialize_all();
    let health_reports = registry.health_check_all();
    let shutdown_reports = registry.shutdown_all();

    assert_eq!(init_reports.len(), 1);
    assert!(init_reports[0].success);
    assert_eq!(health_reports[0].kind, ProviderKind::Llm);
    assert_eq!(health_reports[0].health, ProviderHealth::Healthy);
    assert!(shutdown_reports[0].success);
}

#[test]
fn openai_compatible_uses_custom_base_url_without_separate_code_path() {
    let config = ProviderConfig {
        provider_id: "local".to_owned(),
        provider_type: ProviderType::OpenAiCompatible,
        display_name: "Local".to_owned(),
        enabled: true,
        base_url: Some("http://127.0.0.1:11434/v1".to_owned()),
        api_key: None,
        models: Vec::new(),
    };

    assert_eq!(
        ProviderProtocol::from_provider_type(&config.provider_type).unwrap(),
        ProviderProtocol::OpenAi
    );
    assert_eq!(
        resolve_base_url(&config).unwrap(),
        "http://127.0.0.1:11434/v1"
    );
}
