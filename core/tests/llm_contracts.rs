use std::sync::Mutex;

use ariadne::config::AutoModeConfig;
use ariadne::contracts::{
    CancellationToken, CoreError, CoreResult, ProviderCapability, ProviderDefinition, ProviderType,
    RunId,
};
use ariadne::costs::{BudgetLimits, CostLedger, CostQuery, SqliteCostLedger, TokenUsage};
use ariadne::llm::{
    LlmAuditKind, LlmRunRequest, LlmService, LlmServiceConfig, ToolExecutionContext,
    ToolExecutionOutput, ToolExecutor,
};
use ariadne::providers::{
    ContentPart, LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext,
    ToolCall, ToolDefinition,
};
use serde_json::{json, Value};

struct ScriptedLlmProvider {
    provider_id: String,
    responses: Mutex<Vec<LlmResponse>>,
    requests: Mutex<Vec<LlmRequest>>,
}

impl ScriptedLlmProvider {
    /// 创建按顺序返回响应的测试 provider。
    fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            provider_id: "mock-llm".to_owned(),
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// 返回 provider 收到的请求中是否启用了 stream。
    fn saw_stream_request(&self) -> bool {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.stream)
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Provider for ScriptedLlmProvider {
    /// 返回测试 provider 定义。
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: self.provider_id.clone(),
            provider_type: ProviderType::OpenAi,
            display_name: self.provider_id.clone(),
            capabilities: vec![ProviderCapability::Llm, ProviderCapability::ToolUse],
            config_schema: Value::Null,
        }
    }
}

impl LlmProvider for ScriptedLlmProvider {
    /// 逐次弹出预设响应。
    fn complete(
        &self,
        _context: &ProviderCallContext,
        request: LlmRequest,
    ) -> CoreResult<LlmResponse> {
        self.requests.lock().unwrap().push(request);
        let mut responses = self.responses.lock().unwrap();
        Ok(responses.remove(0))
    }
}

struct EchoToolExecutor;

impl ToolExecutor for EchoToolExecutor {
    /// 返回 tool 名称和参数，便于断言审计链路。
    fn execute(
        &self,
        context: &ToolExecutionContext,
        call: &ToolCall,
    ) -> CoreResult<ToolExecutionOutput> {
        Ok(ToolExecutionOutput {
            value: json!({
                "round": context.round,
                "tool": call.name,
                "arguments": call.arguments
            }),
            audit_metadata: json!({ "ok": true }),
        })
    }
}

/// 构造基础 LLM 运行请求。
fn run_request() -> LlmRunRequest {
    LlmRunRequest {
        config: LlmServiceConfig::new("mock-llm", "model-a"),
        messages: vec![LlmMessage::user("写一段场景")],
        tools: vec![ToolDefinition {
            name: "lookup".to_owned(),
            description: "Lookup context".to_owned(),
            input_schema: json!({ "type": "object" }),
        }],
        workflow_id: None,
        run_id: Some(RunId::new("run-llm")),
        node_id: None,
        metadata: Value::Null,
    }
}

/// 构造带文本的 LLM 响应。
fn text_response(text: &str, cost_usd: Option<f64>, usage: TokenUsage) -> LlmResponse {
    LlmResponse {
        message: LlmMessage::assistant(text),
        tool_calls: Vec::new(),
        usage: Some(usage),
        finish_reason: Some("stop".to_owned()),
        cost_usd,
        raw: json!({ "text": text }),
    }
}

/// 构造带 tool call 的 LLM 响应。
fn tool_response() -> LlmResponse {
    LlmResponse {
        message: LlmMessage {
            role: ariadne::providers::LlmRole::Assistant,
            content: vec![ContentPart::text("need lookup")],
            name: None,
            tool_call_id: None,
        },
        tool_calls: vec![ToolCall {
            tool_call_id: "tool-1".to_owned(),
            name: "lookup".to_owned(),
            arguments: json!({ "query": "人物设定" }),
        }],
        usage: Some(TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
        }),
        finish_reason: Some("tool_calls".to_owned()),
        cost_usd: Some(0.01),
        raw: json!({ "tool": true }),
    }
}

/// 验证基础生成会调用 ProviderExecutor 并写入成本账本。
#[test]
fn llm_service_basic_generation_records_cost() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let service = LlmService::new(&ledger, AutoModeConfig::default());
    let provider = ScriptedLlmProvider::new(vec![text_response(
        "完成",
        Some(0.05),
        TokenUsage {
            input_tokens: 30,
            output_tokens: 20,
        },
    )]);

    let report = service
        .complete_basic(&provider, run_request(), &CancellationToken::new())
        .unwrap();
    let total = ledger
        .total_cost(&CostQuery {
            run_id: Some(RunId::new("run-llm")),
            ..CostQuery::default()
        })
        .unwrap();

    assert_eq!(report.response.message, LlmMessage::assistant("完成"));
    assert_eq!(total, 0.05);
    assert!(report
        .audit_log
        .iter()
        .any(|event| event.kind == LlmAuditKind::ProviderResponse));
}

/// 验证 tool-use 循环会记录 tool 请求、tool 完成和最终响应。
#[test]
fn llm_service_tool_use_is_auditable() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let service = LlmService::new(&ledger, AutoModeConfig::default());
    let provider = ScriptedLlmProvider::new(vec![
        tool_response(),
        text_response(
            "根据检索结果继续",
            Some(0.02),
            TokenUsage {
                input_tokens: 20,
                output_tokens: 10,
            },
        ),
    ]);

    let report = service
        .complete_with_tools(
            &provider,
            run_request(),
            &EchoToolExecutor,
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(report.rounds_completed, 2);
    assert!(report
        .audit_log
        .iter()
        .any(|event| event.kind == LlmAuditKind::ToolCallRequested));
    assert!(report
        .audit_log
        .iter()
        .any(|event| event.kind == LlmAuditKind::ToolCallCompleted));
}

/// 验证超过最大 tool-use 轮次时停止。
#[test]
fn llm_service_rejects_excess_tool_rounds() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let service = LlmService::new(&ledger, AutoModeConfig::default());
    let provider = ScriptedLlmProvider::new(vec![tool_response()]);
    let mut request = run_request();
    request.config.max_tool_rounds = 0;

    let error = service
        .complete_with_tools(
            &provider,
            request,
            &EchoToolExecutor,
            &CancellationToken::new(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("max rounds"));
}

/// 验证 token 上限能阻断后续循环。
#[test]
fn llm_service_enforces_token_limit() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let service = LlmService::new(&ledger, AutoModeConfig::default());
    let provider = ScriptedLlmProvider::new(vec![tool_response()]);
    let mut request = run_request();
    request.config.max_total_tokens = Some(1);

    let error = service
        .complete_with_tools(
            &provider,
            request,
            &EchoToolExecutor,
            &CancellationToken::new(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("tokens"));
}

/// 验证取消信号会在调用前阻断生成。
#[test]
fn llm_service_honors_cancellation_before_call() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let service = LlmService::new(&ledger, AutoModeConfig::default());
    let provider = ScriptedLlmProvider::new(vec![text_response(
        "不会调用",
        None,
        TokenUsage {
            input_tokens: 1,
            output_tokens: 1,
        },
    )]);
    let token = CancellationToken::new();
    token.cancel();

    let error = service
        .complete_basic(&provider, run_request(), &token)
        .unwrap_err();

    assert!(error.to_string().contains("operation cancelled"));
}

/// 验证普通模式高成本调用会暂停等待确认。
#[test]
fn llm_service_pauses_for_high_cost_confirmation() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let service = LlmService::new(&ledger, AutoModeConfig::default());
    let provider = ScriptedLlmProvider::new(vec![text_response(
        "昂贵响应",
        Some(0.50),
        TokenUsage {
            input_tokens: 100,
            output_tokens: 100,
        },
    )]);
    let mut request = run_request();
    request.config.budget_limits = BudgetLimits {
        high_cost_confirmation_usd: Some(0.10),
        ..BudgetLimits::default()
    };

    let error = service
        .complete_basic(&provider, request, &CancellationToken::new())
        .unwrap_err();

    assert!(matches!(error, CoreError::Paused { .. }));
}

/// 验证配置了模型价格时，预算会在 provider 调用前预检。
#[test]
fn llm_service_preflights_budget_before_provider_call() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let service = LlmService::new(&ledger, AutoModeConfig::default());
    let provider = ScriptedLlmProvider::new(vec![text_response(
        "不应被调用",
        Some(1.0),
        TokenUsage {
            input_tokens: 1,
            output_tokens: 1,
        },
    )]);
    let mut request = run_request();
    request.config.budget_limits = BudgetLimits {
        single_call_usd: Some(0.01),
        high_cost_confirmation_usd: None,
        ..BudgetLimits::default()
    };
    request.config.input_cost_per_million_tokens = Some(0.0);
    request.config.output_cost_per_million_tokens = Some(100.0);
    request.config.max_output_tokens = Some(4096);

    let error = service
        .complete_basic(&provider, request, &CancellationToken::new())
        .unwrap_err();

    assert!(matches!(error, CoreError::Paused { .. }));
    assert_eq!(provider.request_count(), 0);
}

/// 验证流式入口会用 stream=true 调用 provider，并生成前端可消费事件。
#[test]
fn llm_service_stream_basic_emits_events() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let service = LlmService::new(&ledger, AutoModeConfig::default());
    let provider = ScriptedLlmProvider::new(vec![text_response(
        "流式文本",
        None,
        TokenUsage {
            input_tokens: 5,
            output_tokens: 3,
        },
    )]);

    let events = service
        .stream_basic_events(&provider, run_request(), &CancellationToken::new())
        .unwrap();

    assert!(provider.saw_stream_request());
    assert!(events.iter().any(
        |event| matches!(event, ariadne::llm::LlmStreamEvent::Delta { text } if text == "流式文本")
    ));
    assert!(events
        .iter()
        .any(|event| matches!(event, ariadne::llm::LlmStreamEvent::Finished { .. })));
}
