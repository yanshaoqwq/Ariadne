use serde::{Deserialize, Serialize};

use crate::config::AutoModeConfig;
use crate::core::RunControl;

/// 预算上限配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_call_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_cost_confirmation_usd: Option<f64>,
}

impl Default for BudgetLimits {
    /// 创建默认预算限制，默认仅对高成本操作要求确认。
    fn default() -> Self {
        Self {
            single_call_usd: None,
            daily_usd: None,
            monthly_usd: None,
            high_cost_confirmation_usd: Some(1.0),
        }
    }
}

/// 预算评估后的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAction {
    Allow,
    RequireConfirmation,
    Pause,
}

/// 预算评估结果，同时包含工作流控制语义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetDecision {
    pub action: BudgetAction,
    pub run_control: RunControl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl BudgetDecision {
    /// 允许继续执行。
    fn allow() -> Self {
        Self {
            action: BudgetAction::Allow,
            run_control: RunControl::Continue,
            reason: None,
        }
    }

    /// 需要人工确认，工作流进入 Pause。
    fn require_confirmation(reason: impl Into<String>) -> Self {
        Self {
            action: BudgetAction::RequireConfirmation,
            run_control: RunControl::Pause,
            reason: Some(reason.into()),
        }
    }

    /// 超限或非法费用，工作流进入 Pause。
    fn pause(reason: impl Into<String>) -> Self {
        Self {
            action: BudgetAction::Pause,
            run_control: RunControl::Pause,
            reason: Some(reason.into()),
        }
    }
}

/// 当前调用和周期累计成本。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetUsage {
    pub requested_usd: f64,
    pub spent_today_usd: f64,
    pub spent_this_month_usd: f64,
}

/// 根据预算、Auto Mode 和当前用量决定是否继续、确认或暂停。
pub fn evaluate_budget(
    limits: &BudgetLimits,
    auto_mode: &AutoModeConfig,
    usage: BudgetUsage,
) -> BudgetDecision {
    if !usage.requested_usd.is_finite() || usage.requested_usd < 0.0 {
        return BudgetDecision::pause("requested cost must be finite and non-negative");
    }

    if exceeds(limits.single_call_usd, usage.requested_usd) {
        return BudgetDecision::pause("single-call budget limit exceeded");
    }

    if exceeds(
        limits.daily_usd,
        usage.spent_today_usd + usage.requested_usd,
    ) {
        return BudgetDecision::pause("daily budget limit exceeded");
    }

    if exceeds(
        limits.monthly_usd,
        usage.spent_this_month_usd + usage.requested_usd,
    ) {
        return BudgetDecision::pause("monthly budget limit exceeded");
    }

    if auto_mode.enabled_by_default {
        if exceeds(auto_mode.preauthorized_budget_usd, usage.requested_usd) {
            return BudgetDecision::pause("auto mode preauthorized budget exceeded");
        }
        // Auto Mode 只跳过普通确认，前面的硬预算限制仍然已经执行。
        return BudgetDecision::allow();
    }

    if exceeds(limits.high_cost_confirmation_usd, usage.requested_usd) {
        return BudgetDecision::require_confirmation("high-cost operation requires confirmation");
    }

    BudgetDecision::allow()
}

/// 判断 value 是否超过可选上限。
fn exceeds(limit: Option<f64>, value: f64) -> bool {
    limit.is_some_and(|limit| value > limit)
}

#[cfg(test)]
mod tests {
    use crate::config::AutoModeConfig;

    use super::*;

    #[test]
    fn auto_mode_over_preauthorized_budget_pauses() {
        let decision = evaluate_budget(
            &BudgetLimits::default(),
            &AutoModeConfig {
                enabled_by_default: true,
                preauthorized_budget_usd: Some(0.50),
                ..AutoModeConfig::default()
            },
            BudgetUsage {
                requested_usd: 0.75,
                spent_today_usd: 0.0,
                spent_this_month_usd: 0.0,
            },
        );

        assert_eq!(decision.action, BudgetAction::Pause);
        assert_eq!(decision.run_control, RunControl::Pause);
    }

    #[test]
    fn normal_mode_high_cost_requires_confirmation() {
        let decision = evaluate_budget(
            &BudgetLimits {
                high_cost_confirmation_usd: Some(0.25),
                ..BudgetLimits::default()
            },
            &AutoModeConfig::default(),
            BudgetUsage {
                requested_usd: 0.30,
                spent_today_usd: 0.0,
                spent_this_month_usd: 0.0,
            },
        );

        assert_eq!(decision.action, BudgetAction::RequireConfirmation);
        assert_eq!(decision.run_control, RunControl::Pause);
    }
}
