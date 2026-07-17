use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::ModelConfig;
use crate::contracts::{NodeId, RunControl, RunId, WorkflowId};
use crate::costs::{BudgetLimits, TokenUsage};
use crate::providers::{
    ContentPart, LlmMessage, LlmRequest, LlmResponse, ProviderCallContext, ToolCall,
};

/// LLM 调用模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmCallMode {
    Basic,
    ToolUse,
}

/// LLM 服务调用配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmServiceConfig {
    pub provider_id: String,
    pub model_id: String,
    pub max_tool_rounds: u32,
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
    #[serde(default)]
    pub budget_limits: BudgetLimits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_million_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_million_tokens: Option<f64>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<u32>,
}

impl LlmServiceConfig {
    /// 创建默认 LLM 服务配置。
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            max_tool_rounds: 4,
            timeout_ms: 120_000,
            max_total_tokens: None,
            budget_limits: BudgetLimits::default(),
            input_cost_per_million_tokens: None,
            output_cost_per_million_tokens: None,
            max_output_tokens: None,
            max_context_tokens: None,
        }
    }

    pub fn with_model_config(mut self, model: &ModelConfig) -> Self {
        self.input_cost_per_million_tokens = model.input_cost_per_million_tokens;
        self.output_cost_per_million_tokens = model.output_cost_per_million_tokens;
        self.max_context_tokens = model.max_context_tokens;
        self
    }
}

/// 单次 LLM 运行请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRunRequest {
    pub config: LlmServiceConfig,
    #[serde(default)]
    pub messages: Vec<LlmMessage>,
    #[serde(default)]
    pub tools: Vec<crate::providers::ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<WorkflowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(skip, default)]
    pub dispatch_authorization: crate::contracts::ExternalDispatchAuthorization,
}

impl LlmRunRequest {
    /// 转成 Module 3 的 provider 请求。
    pub fn to_llm_request(&self, stream: bool) -> LlmRequest {
        LlmRequest {
            model_id: self.config.model_id.clone(),
            messages: self.messages.clone(),
            tools: self.tools.clone(),
            temperature: None,
            max_output_tokens: self.config.max_output_tokens,
            stream,
            metadata: self.metadata.clone(),
        }
    }

    /// 构造 Provider 调用上下文。
    pub fn provider_context(
        &self,
        tool_call_id: Option<String>,
        cancellation: &crate::contracts::CancellationToken,
    ) -> ProviderCallContext {
        ProviderCallContext {
            provider_id: self.config.provider_id.clone(),
            operation_id: None,
            workflow_id: self.workflow_id.clone(),
            run_id: self.run_id.clone(),
            node_id: self.node_id.clone(),
            tool_call_id,
            timeout_ms: self.config.timeout_ms,
            max_retries: 0,
            metadata: self.metadata.clone(),
            cancellation: cancellation.clone(),
            dispatch_authorization: self.dispatch_authorization.clone(),
        }
    }
}

/// Tool 执行上下文，供上层 tool router 记录调用来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExecutionContext {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<WorkflowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    pub round: u32,
}

/// Tool 执行结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExecutionOutput {
    pub value: Value,
    #[serde(default)]
    pub audit_metadata: Value,
}

/// Tool 执行器抽象，Module 10 Skill 和 Module 9 RAG 后续可实现它。
pub trait ToolExecutor: Send + Sync {
    /// 执行一次 tool call。
    fn execute(
        &self,
        context: &ToolExecutionContext,
        call: &ToolCall,
    ) -> crate::contracts::CoreResult<ToolExecutionOutput>;
}

/// 按工具名把同一轮 LLM tool call 路由到不同执行器。
///
/// 项目检索、Web 搜索和后续通用工具可在同一节点同时出现，调用方无需把多个
/// 完全不同的副作用边界塞进单一特殊执行器。
#[derive(Default)]
pub struct ToolExecutorRouter<'a> {
    routes: BTreeMap<String, &'a dyn ToolExecutor>,
}

impl<'a> ToolExecutorRouter<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        tool_name: impl Into<String>,
        executor: &'a dyn ToolExecutor,
    ) -> crate::contracts::CoreResult<()> {
        let tool_name = tool_name.into();
        if tool_name.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "tool route name cannot be empty",
            ));
        }
        if self.routes.insert(tool_name.clone(), executor).is_some() {
            return Err(crate::contracts::CoreError::validation(format!(
                "duplicate tool executor route: {tool_name}"
            )));
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

impl ToolExecutor for ToolExecutorRouter<'_> {
    fn execute(
        &self,
        context: &ToolExecutionContext,
        call: &ToolCall,
    ) -> crate::contracts::CoreResult<ToolExecutionOutput> {
        let executor = self.routes.get(&call.name).ok_or_else(|| {
            crate::contracts::CoreError::validation(format!(
                "tool is not routed in this scope: {}",
                call.name
            ))
        })?;
        executor.execute(context, call)
    }
}

/// LLM 服务审计事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmAuditKind {
    RequestStarted,
    ProviderResponse,
    ToolCallRequested,
    ToolCallCompleted,
    BudgetDecision,
    ControlSignal,
    RunFinished,
}

/// LLM 服务审计事件，保存可审计元数据，不保存密钥或大段上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmAuditEvent {
    pub kind: LlmAuditKind,
    pub round: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_control: Option<RunControl>,
    #[serde(default)]
    pub metadata: Value,
}

/// LLM 服务运行报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRunReport {
    pub mode: LlmCallMode,
    pub response: LlmResponse,
    pub rounds_completed: u32,
    pub run_control: RunControl,
    #[serde(default)]
    pub audit_log: Vec<LlmAuditEvent>,
}

/// LLM 流式事件；当前先建立稳定契约，真实增量输出后续接入。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmStreamEvent {
    Started {
        provider_id: String,
        model_id: String,
    },
    Delta {
        text: String,
    },
    ToolCall {
        call: ToolCall,
    },
    Finished {
        response: LlmResponse,
    },
    Failed {
        error: String,
    },
}

/// 将 tool 输出包装成 provider 可继续消费的消息。
pub fn tool_result_message(call: &ToolCall, output: ToolExecutionOutput) -> LlmMessage {
    LlmMessage {
        role: crate::providers::LlmRole::Tool,
        content: vec![ContentPart::ToolResult {
            tool_call_id: call.tool_call_id.clone(),
            value: output.value,
        }],
        name: Some(call.name.clone()),
        tool_call_id: Some(call.tool_call_id.clone()),
    }
}
