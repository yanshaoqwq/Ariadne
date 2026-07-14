use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::config::{ModelConfig, ProviderConfig};
use ariadne::contracts::{
    CoreError, CoreResult, ExternalDispatchAuthorization, ExternalDispatchOutcome,
    ProviderCapability, ProviderDefinition, ProviderType, RunId,
};
use ariadne::costs::{CostLedger, CostQuery, SqliteCostLedger, TokenUsage};
use ariadne::providers::{
    resolve_base_url, EmbeddingProvider, EmbeddingRequest, HttpEmbeddingProvider,
    HttpRerankerProvider, LlmMessage, LlmProvider, LlmRequest, LlmResponse,
    OpenAiCompatibleLlmProvider, Provider, ProviderCallContext, ProviderExecutor, ProviderHealth,
    ProviderKind, ProviderProtocol, ProviderRuntimeRegistry, RerankRequest, RerankerProvider,
    ToolUseEnvelope,
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

struct CountingLlmProvider {
    calls: Arc<AtomicUsize>,
}

impl Provider for CountingLlmProvider {
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: "counting".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Counting".to_owned(),
            capabilities: vec![ProviderCapability::Llm],
            config_schema: Value::Null,
        }
    }
}

impl LlmProvider for CountingLlmProvider {
    fn complete(
        &self,
        _context: &ProviderCallContext,
        _request: LlmRequest,
    ) -> CoreResult<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            message: LlmMessage::assistant("unexpected"),
            tool_calls: Vec::new(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            cost_usd: None,
            raw: Value::Null,
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
fn provider_executor_does_not_call_backend_when_dispatch_authorization_is_denied() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let executor = ProviderExecutor::new(&ledger);
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = CountingLlmProvider {
        calls: calls.clone(),
    };
    let mut context = ProviderCallContext::new("counting");
    context.dispatch_authorization = ExternalDispatchAuthorization::new(|dispatch| {
        assert!(dispatch);
        Err(CoreError::external_cancelled(
            "provider_dispatch_test",
            ExternalDispatchOutcome::NotDispatched,
        ))
    });

    let error = executor
        .complete_llm(
            &provider,
            &context,
            LlmRequest {
                model_id: "model".to_owned(),
                messages: vec![LlmMessage::user("must not dispatch")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap_err();

    assert_eq!(
        error.external_dispatch_outcome(),
        Some(ExternalDispatchOutcome::NotDispatched)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
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
fn openai_compatible_embedding_provider_batches_inputs_and_orders_vectors() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /embeddings "));
        assert!(request.contains("authorization: Bearer embedding-key"));
        assert!(request.contains("\"model\":\"embed-test\""));
        assert!(request.contains("\"input\":[\"第一段\",\"第二段\"]"));
        let response_body = r#"{
          "model":"embed-test",
          "data":[
            {"index":1,"embedding":[0.3,0.4]},
            {"index":0,"embedding":[0.1,0.2]}
          ],
          "usage":{"prompt_tokens":4,"total_tokens":4}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let provider = HttpEmbeddingProvider::new(
        ProviderConfig {
            provider_id: "embedding".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Embedding".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "embed-test".to_owned(),
                capability: ProviderCapability::Embedding,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(2.0),
                output_cost_per_million_tokens: None,
            }],
        },
        Some("embedding-key".to_owned()),
    )
    .unwrap();
    let response = provider
        .embed(
            &ProviderCallContext::new("embedding"),
            EmbeddingRequest {
                model_id: "embed-test".to_owned(),
                inputs: vec!["第一段".to_owned(), "第二段".to_owned()],
                metadata: Value::Null,
            },
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(response.embeddings, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);
    assert_eq!(response.usage.unwrap().input_tokens, 4);
    assert_eq!(response.cost_usd, Some(0.000008));
}

#[test]
fn gemini_embedding_provider_uses_batch_embed_contents_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request
            .starts_with("POST /models/text-embedding-test:batchEmbedContents?key=gemini-key "));
        assert!(request.contains("\"model\":\"models/text-embedding-test\""));
        assert!(request.contains("\"text\":\"章节内容\""));
        let response_body = r#"{
          "embeddings":[{"values":[0.25,0.75]}]
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let provider = HttpEmbeddingProvider::new(
        ProviderConfig {
            provider_id: "gemini-embedding".to_owned(),
            provider_type: ProviderType::Gemini,
            display_name: "Gemini embedding".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "text-embedding-test".to_owned(),
                capability: ProviderCapability::Embedding,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
        },
        Some("gemini-key".to_owned()),
    )
    .unwrap();
    let response = provider
        .embed(
            &ProviderCallContext::new("gemini-embedding"),
            EmbeddingRequest {
                model_id: "text-embedding-test".to_owned(),
                inputs: vec!["章节内容".to_owned()],
                metadata: Value::Null,
            },
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(response.embeddings, vec![vec![0.25, 0.75]]);
}

#[test]
fn openai_compatible_reranker_validates_and_maps_document_indices() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /rerank "));
        assert!(request.contains("authorization: Bearer rerank-key"));
        assert!(request.contains("\"query\":\"角色动机\""));
        assert!(request.contains("\"documents\":[\"文档甲\",\"文档乙\"]"));
        assert!(request.contains("\"top_n\":2"));
        let response_body = r#"{
          "model":"rerank-test",
          "results":[
            {"index":1,"relevance_score":0.95},
            {"index":0,"score":0.4}
          ],
          "usage":{"total_tokens":5}
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let provider = HttpRerankerProvider::new(
        ProviderConfig {
            provider_id: "reranker".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Reranker".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "rerank-test".to_owned(),
                capability: ProviderCapability::Reranker,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(1.0),
                output_cost_per_million_tokens: None,
            }],
        },
        Some("rerank-key".to_owned()),
    )
    .unwrap();
    let response = provider
        .rerank(
            &ProviderCallContext::new("reranker"),
            RerankRequest {
                model_id: "rerank-test".to_owned(),
                query: "角色动机".to_owned(),
                documents: vec!["文档甲".to_owned(), "文档乙".to_owned()],
                top_n: Some(2),
                metadata: Value::Null,
            },
        )
        .unwrap();
    server.join().unwrap();

    assert_eq!(response.results[0].index, 1);
    assert_eq!(response.results[0].score, 0.95);
    assert_eq!(response.results[1].index, 0);
    assert_eq!(response.cost_usd, Some(0.000005));
}

#[test]
fn unsupported_embedding_and_reranker_protocols_fail_before_network_dispatch() {
    let anthropic_embedding = HttpEmbeddingProvider::new(
        ProviderConfig {
            provider_id: "anthropic".to_owned(),
            provider_type: ProviderType::Anthropic,
            display_name: "Anthropic".to_owned(),
            enabled: true,
            base_url: Some("http://127.0.0.1:1".to_owned()),
            api_key: None,
            models: Vec::new(),
        },
        None,
    )
    .unwrap_err();
    assert!(anthropic_embedding
        .to_string()
        .contains("does not define an embedding endpoint"));

    let native_openai_reranker = HttpRerankerProvider::new(
        ProviderConfig {
            provider_id: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            display_name: "OpenAI".to_owned(),
            enabled: true,
            base_url: Some("http://127.0.0.1:1".to_owned()),
            api_key: None,
            models: Vec::new(),
        },
        None,
    )
    .unwrap_err();
    assert!(native_openai_reranker
        .to_string()
        .contains("open_ai_compatible or local"));
}

#[test]
fn http_provider_connect_failure_is_confirmed_not_dispatched() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "connect-failure".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Connect failure".to_owned(),
            enabled: true,
            base_url: Some(format!("http://{address}")),
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
            &ProviderCallContext::new("connect-failure"),
            LlmRequest {
                model_id: "test-model".to_owned(),
                messages: vec![LlmMessage::user("request must not be dispatched")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap_err();

    assert_eq!(
        error.external_dispatch_outcome(),
        Some(ExternalDispatchOutcome::NotDispatched)
    );
}

#[test]
fn http_provider_disconnect_after_request_is_dispatched_unknown() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("request may have reached the server"));
        // 请求已读取，但在任何 HTTP 响应之前断开，客户端不能确认远端副作用。
    });
    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "disconnect-after-send".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Disconnect after send".to_owned(),
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
            &ProviderCallContext::new("disconnect-after-send"),
            LlmRequest {
                model_id: "test-model".to_owned(),
                messages: vec![LlmMessage::user("request may have reached the server")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
        .unwrap_err();
    server.join().unwrap();

    assert_eq!(
        error.external_dispatch_outcome(),
        Some(ExternalDispatchOutcome::DispatchedUnknown)
    );
}

#[test]
fn http_provider_cancellation_aborts_in_flight_request_without_waiting_for_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (request_seen_tx, request_seen_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 8192];
        let _ = stream.read(&mut buffer).unwrap();
        request_seen_tx.send(()).unwrap();
        thread::sleep(Duration::from_secs(2));
    });
    let provider = OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "cancel-in-flight".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Cancel in flight".to_owned(),
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
    let mut context = ProviderCallContext::new("cancel-in-flight");
    context.timeout_ms = 30_000;
    let cancellation = context.cancellation.clone();
    let started = Instant::now();
    let request_thread = thread::spawn(move || {
        provider.complete(
            &context,
            LlmRequest {
                model_id: "test-model".to_owned(),
                messages: vec![LlmMessage::user("等待取消")],
                tools: Vec::new(),
                temperature: None,
                max_output_tokens: None,
                stream: false,
                metadata: Value::Null,
            },
        )
    });
    request_seen_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("server must receive the request before cancellation");
    cancellation.cancel();
    let error = request_thread.join().unwrap().unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "cancellation must not wait for provider timeout"
    );
    assert_eq!(
        error.external_dispatch_outcome(),
        Some(ExternalDispatchOutcome::DispatchedUnknown)
    );
    server.join().unwrap();
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
    assert_eq!(
        error.external_dispatch_outcome(),
        Some(ExternalDispatchOutcome::DispatchedUnknown)
    );
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
    assert_eq!(
        error.external_dispatch_outcome(),
        Some(ExternalDispatchOutcome::DispatchedUnknown)
    );
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
