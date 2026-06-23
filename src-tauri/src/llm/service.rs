use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::AutoModeConfig;
use crate::core::{CancellationToken, CoreError, CoreResult, RunControl};
use crate::costs::{evaluate_budget, BudgetUsage, CostLedger, CostQuery, TokenUsage};
use crate::llm::models::{
    tool_result_message, LlmAuditEvent, LlmAuditKind, LlmCallMode, LlmRunReport, LlmRunRequest,
    LlmServiceConfig, LlmStreamEvent, ToolExecutionContext, ToolExecutor,
};
use crate::providers::{ContentPart, LlmProvider, LlmResponse, ProviderExecutor};

/// LLM 编排服务，负责 Provider 调用、tool-use 循环保护和审计。
pub struct LlmService<'a, L: CostLedger> {
    ledger: &'a L,
    auto_mode: AutoModeConfig,
}

impl<'a, L: CostLedger> LlmService<'a, L> {
    /// 创建 LLM 服务。
    pub fn new(ledger: &'a L, auto_mode: AutoModeConfig) -> Self {
        Self { ledger, auto_mode }
    }

    /// 执行一次基础 LLM 生成，不进入 tool-use 循环。
    pub fn complete_basic(
        &self,
        provider: &dyn LlmProvider,
        request: LlmRunRequest,
        cancellation: &CancellationToken,
    ) -> CoreResult<LlmRunReport> {
        validate_config(&request.config)?;
        cancellation.check()?;
        let started_at = Instant::now();
        let mut audit_log = Vec::new();
        audit_log.push(started_event(&request, LlmCallMode::Basic));

        let response = self.call_provider(provider, &request, None, false)?;
        self.check_after_provider_response(&request, &response, started_at, cancellation)?;
        audit_log.push(provider_response_event(&request, 0, &response));

        Ok(LlmRunReport {
            mode: LlmCallMode::Basic,
            response,
            rounds_completed: 1,
            run_control: RunControl::Continue,
            audit_log: finish_audit(audit_log, 1, RunControl::Continue),
        })
    }

    /// 执行带 tool-use 的 LLM 生成循环。
    pub fn complete_with_tools(
        &self,
        provider: &dyn LlmProvider,
        mut request: LlmRunRequest,
        tool_executor: &dyn ToolExecutor,
        cancellation: &CancellationToken,
    ) -> CoreResult<LlmRunReport> {
        validate_config(&request.config)?;
        let started_at = Instant::now();
        let mut audit_log = vec![started_event(&request, LlmCallMode::ToolUse)];
        let mut total_usage = TokenUsage::default();

        for round in 0..=request.config.max_tool_rounds {
            cancellation.check()?;
            check_timeout(started_at, request.config.timeout_ms)?;

            let response = self.call_provider(provider, &request, None, false)?;
            self.check_after_provider_response(&request, &response, started_at, cancellation)?;
            accumulate_usage(&mut total_usage, response.usage);
            audit_log.push(provider_response_event(&request, round, &response));
            self.check_token_limit(&request.config, &total_usage)?;

            if response.tool_calls.is_empty() {
                return Ok(LlmRunReport {
                    mode: LlmCallMode::ToolUse,
                    response,
                    rounds_completed: round + 1,
                    run_control: RunControl::Continue,
                    audit_log: finish_audit(audit_log, round + 1, RunControl::Continue),
                });
            }

            if round >= request.config.max_tool_rounds {
                return Err(CoreError::validation(
                    "tool-use max rounds exceeded before final answer",
                ));
            }

            // assistant 的 tool call 消息必须回填到上下文，让下一轮模型能关联工具结果。
            request.messages.push(response.message.clone());
            for call in &response.tool_calls {
                audit_log.push(tool_requested_event(&request, round, call));
                let tool_context = ToolExecutionContext {
                    provider_id: request.config.provider_id.clone(),
                    workflow_id: request.workflow_id.clone(),
                    run_id: request.run_id.clone(),
                    node_id: request.node_id.clone(),
                    round,
                };
                let output = tool_executor.execute(&tool_context, call)?;
                audit_log.push(tool_completed_event(
                    &request,
                    round,
                    call,
                    &output.audit_metadata,
                ));
                request.messages.push(tool_result_message(call, output));
            }
        }

        Err(CoreError::validation("tool-use loop ended unexpectedly"))
    }

    /// 执行流式调用入口；当前同步 provider 返回后拆成事件，后续可替换为真实增量流。
    pub fn stream_basic_events(
        &self,
        provider: &dyn LlmProvider,
        request: LlmRunRequest,
        cancellation: &CancellationToken,
    ) -> CoreResult<Vec<LlmStreamEvent>> {
        validate_config(&request.config)?;
        cancellation.check()?;
        let started_at = Instant::now();
        let mut events = vec![LlmStreamEvent::Started {
            provider_id: request.config.provider_id.clone(),
            model_id: request.config.model_id.clone(),
        }];

        let response = self.call_provider(provider, &request, None, true)?;
        if let Err(error) =
            self.check_after_provider_response(&request, &response, started_at, cancellation)
        {
            events.push(LlmStreamEvent::Failed {
                error: error.to_string(),
            });
            return Err(error);
        }

        // 当前 Provider trait 仍是同步返回；这里先把完整响应拆成前端可消费的流式事件。
        for part in &response.message.content {
            if let ContentPart::Text { text } = part {
                if !text.is_empty() {
                    events.push(LlmStreamEvent::Delta { text: text.clone() });
                }
            }
        }
        for call in &response.tool_calls {
            events.push(LlmStreamEvent::ToolCall { call: call.clone() });
        }
        events.push(LlmStreamEvent::Finished { response });
        Ok(events)
    }

    /// 调用 ProviderExecutor，让费用继续写入 Module 2 成本账本。
    fn call_provider(
        &self,
        provider: &dyn LlmProvider,
        request: &LlmRunRequest,
        tool_call_id: Option<String>,
        stream: bool,
    ) -> CoreResult<LlmResponse> {
        let executor = ProviderExecutor::new(self.ledger);
        let context = request.provider_context(tool_call_id);
        executor.complete_llm(provider, &context, request.to_llm_request(stream))
    }

    /// Provider 返回后检查取消、超时和预算。
    fn check_after_provider_response(
        &self,
        request: &LlmRunRequest,
        response: &LlmResponse,
        started_at: Instant,
        cancellation: &CancellationToken,
    ) -> CoreResult<()> {
        cancellation.check()?;
        check_timeout(started_at, request.config.timeout_ms)?;
        self.check_budget(request, response.cost_usd.unwrap_or_default())?;
        Ok(())
    }

    /// 按 Module 2 预算策略决定是否暂停或继续。
    fn check_budget(&self, request: &LlmRunRequest, requested_usd: f64) -> CoreResult<()> {
        let spent_after_record = self.spent_for_run(request)?;
        let spent_before_current = (spent_after_record - requested_usd).max(0.0);
        let decision = evaluate_budget(
            &request.config.budget_limits,
            &self.auto_mode,
            BudgetUsage {
                requested_usd,
                // ProviderExecutor 已经把本次调用写入账本；预算评估需要“本次之前”的累计值。
                spent_today_usd: spent_before_current,
                spent_this_month_usd: spent_before_current,
            },
        );

        match decision.run_control {
            RunControl::Continue => Ok(()),
            RunControl::Pause => Err(CoreError::Paused {
                reason: decision
                    .reason
                    .unwrap_or_else(|| "llm budget requires pause".to_owned()),
            }),
            RunControl::Stop => Err(CoreError::Stopped {
                reason: decision
                    .reason
                    .unwrap_or_else(|| "llm budget requested stop".to_owned()),
            }),
        }
    }

    /// 当前先按 run 维度统计成本，后续 Module 11 再接真实日期窗口。
    fn spent_for_run(&self, request: &LlmRunRequest) -> CoreResult<f64> {
        let Some(run_id) = &request.run_id else {
            return Ok(0.0);
        };
        self.ledger.total_cost(&CostQuery {
            run_id: Some(run_id.clone()),
            ..CostQuery::default()
        })
    }

    /// 检查累计 token 是否超过配置上限。
    fn check_token_limit(&self, config: &LlmServiceConfig, usage: &TokenUsage) -> CoreResult<()> {
        if let Some(limit) = config.max_total_tokens {
            let total = usage.total_tokens();
            if total > limit {
                return Err(CoreError::ResourceLimitExceeded {
                    resource: "tokens".to_owned(),
                    reason: format!("usage {total} exceeds limit {limit}"),
                });
            }
        }

        Ok(())
    }
}

/// 校验 LLM 服务配置，避免无边界循环。
fn validate_config(config: &LlmServiceConfig) -> CoreResult<()> {
    if config.provider_id.trim().is_empty() {
        return Err(CoreError::validation("provider_id cannot be empty"));
    }
    if config.model_id.trim().is_empty() {
        return Err(CoreError::validation("model_id cannot be empty"));
    }
    if config.timeout_ms == 0 {
        return Err(CoreError::validation(
            "llm timeout_ms must be greater than zero",
        ));
    }
    if config.max_tool_rounds > 32 {
        return Err(CoreError::validation("max_tool_rounds cannot exceed 32"));
    }

    Ok(())
}

/// 检查整体调用耗时是否超过配置。
fn check_timeout(started_at: Instant, timeout_ms: u64) -> CoreResult<()> {
    if started_at.elapsed() > Duration::from_millis(timeout_ms) {
        return Err(CoreError::Stopped {
            reason: "llm call timeout exceeded".to_owned(),
        });
    }

    Ok(())
}

/// 累加 token 用量。
fn accumulate_usage(total: &mut TokenUsage, usage: Option<TokenUsage>) {
    if let Some(usage) = usage {
        total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
        total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
    }
}

/// 创建调用开始审计事件。
fn started_event(request: &LlmRunRequest, mode: LlmCallMode) -> LlmAuditEvent {
    LlmAuditEvent {
        kind: LlmAuditKind::RequestStarted,
        round: 0,
        tool_call_id: None,
        provider_id: Some(request.config.provider_id.clone()),
        model_id: Some(request.config.model_id.clone()),
        usage: None,
        cost_usd: None,
        run_control: None,
        metadata: json!({ "mode": mode }),
    }
}

/// 创建 provider 响应审计事件。
fn provider_response_event(
    request: &LlmRunRequest,
    round: u32,
    response: &LlmResponse,
) -> LlmAuditEvent {
    LlmAuditEvent {
        kind: LlmAuditKind::ProviderResponse,
        round,
        tool_call_id: None,
        provider_id: Some(request.config.provider_id.clone()),
        model_id: Some(request.config.model_id.clone()),
        usage: response.usage,
        cost_usd: response.cost_usd,
        run_control: None,
        metadata: json!({
            "finish_reason": response.finish_reason,
            "tool_call_count": response.tool_calls.len()
        }),
    }
}

/// 创建 tool 请求审计事件。
fn tool_requested_event(
    request: &LlmRunRequest,
    round: u32,
    call: &crate::providers::ToolCall,
) -> LlmAuditEvent {
    LlmAuditEvent {
        kind: LlmAuditKind::ToolCallRequested,
        round,
        tool_call_id: Some(call.tool_call_id.clone()),
        provider_id: Some(request.config.provider_id.clone()),
        model_id: Some(request.config.model_id.clone()),
        usage: None,
        cost_usd: None,
        run_control: None,
        metadata: json!({ "tool_name": call.name }),
    }
}

/// 创建 tool 完成审计事件。
fn tool_completed_event(
    request: &LlmRunRequest,
    round: u32,
    call: &crate::providers::ToolCall,
    audit_metadata: &Value,
) -> LlmAuditEvent {
    LlmAuditEvent {
        kind: LlmAuditKind::ToolCallCompleted,
        round,
        tool_call_id: Some(call.tool_call_id.clone()),
        provider_id: Some(request.config.provider_id.clone()),
        model_id: Some(request.config.model_id.clone()),
        usage: None,
        cost_usd: None,
        run_control: None,
        metadata: json!({
            "tool_name": call.name,
            "tool": audit_metadata
        }),
    }
}

/// 添加运行结束审计事件。
fn finish_audit(
    mut audit_log: Vec<LlmAuditEvent>,
    round: u32,
    run_control: RunControl,
) -> Vec<LlmAuditEvent> {
    audit_log.push(LlmAuditEvent {
        kind: LlmAuditKind::RunFinished,
        round,
        tool_call_id: None,
        provider_id: None,
        model_id: None,
        usage: None,
        cost_usd: None,
        run_control: Some(run_control),
        metadata: Value::Null,
    });
    audit_log
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_empty_provider() {
        let mut config = LlmServiceConfig::new("", "model");
        config.max_tool_rounds = 1;

        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn usage_accumulates_safely() {
        let mut total = TokenUsage::default();
        accumulate_usage(
            &mut total,
            Some(TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
            }),
        );

        assert_eq!(total.total_tokens(), 5);
    }
}
