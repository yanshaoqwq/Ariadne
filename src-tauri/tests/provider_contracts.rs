use std::sync::Arc;

use ariadne::config::ProviderConfig;
use ariadne::core::{CoreResult, ProviderCapability, ProviderDefinition, ProviderType, RunId};
use ariadne::costs::{CostLedger, CostQuery, SqliteCostLedger, TokenUsage};
use ariadne::providers::{
    resolve_base_url, LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider,
    ProviderCallContext, ProviderExecutor, ProviderHealth, ProviderKind, ProviderProtocol,
    ProviderRuntimeRegistry, ToolUseEnvelope,
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
