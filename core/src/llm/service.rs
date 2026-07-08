use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::AutoModeConfig;
use crate::contracts::{CancellationToken, CoreError, CoreResult, RunControl};
use crate::costs::{
    estimate_token_cost, evaluate_budget, BudgetUsage, CostLedger, CostQuery, TokenPricing,
    TokenUsage,
};
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
        self.check_estimated_budget(&request)?;

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
            self.check_estimated_budget(&request)?;

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
        self.check_estimated_budget(&request)?;

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

    fn check_estimated_budget(&self, request: &LlmRunRequest) -> CoreResult<()> {
        let Some(estimated_usd) = estimate_request_cost_usd(request)? else {
            return Ok(());
        };
        let now_ms = unix_timestamp_ms()?;
        let day_start_ms = start_of_utc_day_ms(now_ms);
        let month_start_ms = start_of_utc_month_ms(now_ms);
        self.check_budget_usage(
            request,
            estimated_usd,
            self.spent_in_window(request, Some(day_start_ms))?,
            self.spent_in_window(request, Some(month_start_ms))?,
        )
    }

    /// 按 Module 2 预算策略决定是否暂停或继续。
    ///
    /// ProviderExecutor 已经把本次调用写进账本，因此这里查出来的累计值包含本次；
    /// 预算评估需要“本次之前”的累计，用 spent_before_current 反推。日、月额度分别
    /// 按真实 UTC 时间窗口聚合，而不是共用同一个 run 级数字。
    fn check_budget(&self, request: &LlmRunRequest, requested_usd: f64) -> CoreResult<()> {
        let now_ms = unix_timestamp_ms()?;
        let day_start_ms = start_of_utc_day_ms(now_ms);
        let month_start_ms = start_of_utc_month_ms(now_ms);

        let spent_today_after = self.spent_in_window(request, Some(day_start_ms))?;
        let spent_month_after = self.spent_in_window(request, Some(month_start_ms))?;

        self.check_budget_usage(
            request,
            requested_usd,
            spent_before_current(spent_today_after, requested_usd)?,
            spent_before_current(spent_month_after, requested_usd)?,
        )
    }

    fn check_budget_usage(
        &self,
        request: &LlmRunRequest,
        requested_usd: f64,
        spent_today_before: f64,
        spent_month_before: f64,
    ) -> CoreResult<()> {
        let decision = evaluate_budget(
            &request.config.budget_limits,
            &self.auto_mode,
            BudgetUsage {
                requested_usd,
                spent_today_usd: spent_today_before,
                spent_this_month_usd: spent_month_before,
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

    /// 聚合指定时间窗口起点之后的累计成本。有 run_id 时限定到当前 run，
    /// 没有 run_id 时按全局账本累计——绝不能短路返回 0，否则本次已入账的费用
    /// 会让 spent_before_current 反推出负数并误报会计错误。
    fn spent_in_window(&self, request: &LlmRunRequest, start_ms: Option<u64>) -> CoreResult<f64> {
        self.ledger.total_cost(&CostQuery {
            start_ms,
            run_id: request.run_id.clone(),
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

fn estimate_request_cost_usd(request: &LlmRunRequest) -> CoreResult<Option<f64>> {
    let (Some(input_price), Some(output_price)) = (
        request.config.input_cost_per_million_tokens,
        request.config.output_cost_per_million_tokens,
    ) else {
        return Ok(None);
    };
    let prompt_tokens = estimate_prompt_tokens(&request.messages);
    let output_tokens = request.config.max_output_tokens.unwrap_or(4096) as u64;
    estimate_token_cost(
        TokenUsage {
            input_tokens: prompt_tokens,
            output_tokens,
        },
        TokenPricing {
            input_cost_per_million_tokens: input_price,
            output_cost_per_million_tokens: output_price,
        },
    )
    .map(Some)
}

fn estimate_prompt_tokens(messages: &[crate::providers::LlmMessage]) -> u64 {
    let chars = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .map(content_part_char_len)
        .sum::<usize>();
    u64::try_from(chars.div_ceil(4)).unwrap_or(u64::MAX).max(1)
}

fn content_part_char_len(part: &ContentPart) -> usize {
    match part {
        ContentPart::Text { text } => text.chars().count(),
        ContentPart::Json { value } | ContentPart::ToolResult { value, .. } => {
            value.to_string().chars().count()
        }
        ContentPart::ToolUse {
            name, arguments, ..
        } => name.chars().count() + arguments.to_string().chars().count(),
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
    for (field, value) in [
        (
            "input_cost_per_million_tokens",
            config.input_cost_per_million_tokens,
        ),
        (
            "output_cost_per_million_tokens",
            config.output_cost_per_million_tokens,
        ),
    ] {
        if let Some(value) = value {
            if !value.is_finite() || value < 0.0 {
                return Err(CoreError::validation(format!(
                    "{field} must be finite and non-negative"
                )));
            }
        }
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

/// ProviderExecutor 已经记录本次费用；这里反推出本次前累计，不能吞掉负数会计错误。
fn spent_before_current(spent_after_record: f64, requested_usd: f64) -> CoreResult<f64> {
    if !spent_after_record.is_finite() {
        return Err(CoreError::validation(
            "cost accounting error: spent total must be finite",
        ));
    }
    if !requested_usd.is_finite() || requested_usd < 0.0 {
        return Err(CoreError::validation(
            "requested cost must be finite and non-negative",
        ));
    }

    let spent_before_current = spent_after_record - requested_usd;
    if spent_before_current < -f64::EPSILON {
        return Err(CoreError::validation(
            "cost accounting error: current request exceeds recorded total",
        ));
    }

    Ok(spent_before_current.max(0.0))
}

/// 返回当前 Unix 毫秒时间戳。
fn unix_timestamp_ms() -> CoreResult<u64> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            CoreError::validation(format!("system time before unix epoch: {error}"))
        })?;
    u64::try_from(duration.as_millis())
        .map_err(|_| CoreError::validation("timestamp exceeds u64 range"))
}

/// 返回该毫秒时间戳所在 UTC 自然日的 00:00:00 毫秒。
fn start_of_utc_day_ms(now_ms: u64) -> u64 {
    const MS_PER_DAY: u64 = 86_400_000;
    now_ms - (now_ms % MS_PER_DAY)
}

/// 返回该毫秒时间戳所在 UTC 自然月的 1 号 00:00:00 毫秒。
/// 使用 Howard Hinnant 的民用历算法，只依赖 std，不引第三方时间库。
fn start_of_utc_month_ms(now_ms: u64) -> u64 {
    const MS_PER_DAY: i64 = 86_400_000;
    let days = (now_ms / MS_PER_DAY as u64) as i64;
    let (year, month, _day) = civil_from_days(days);
    let month_start_days = days_from_civil(year, month, 1);
    (month_start_days * MS_PER_DAY) as u64
}

/// days（自 1970-01-01 起的天数）转成 (year, month, day)（UTC）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // 以 0000-03-01 为纪元的 Hinnant 算法。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// (year, month, day)（UTC）转成自 1970-01-01 起的天数。
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64; // [0, 399]
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
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

    #[test]
    fn spent_before_current_rejects_negative_accounting() {
        let error = spent_before_current(0.10, 0.25).unwrap_err();

        assert!(error.to_string().contains("cost accounting error"));
    }
}
