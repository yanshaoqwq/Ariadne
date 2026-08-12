use serde::{Deserialize, Serialize};

use crate::config::AutoModeConfig;
use crate::contracts::RunControl;

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

/// 将用户配置的日预算（美元）映射为执行侧 `BudgetLimits`。
///
/// - `budget_usd <= 0`：视为未设置限额，日/月硬上限均为 `None`（与 UI「0 = 不限制」一致）。
/// - `budget_usd > 0`：作为**日限额** `daily_usd` 生效，调用 `evaluate_budget` 时今日累计超限会 Pause。
/// - 保留默认 `high_cost_confirmation_usd = 1.0`，非 Auto Mode 下高成本仍可触发确认。
/// - 单次/月限额不由该字段映射（保持 `None`，除非他处写入）。
pub fn budget_limits_from_global_budget(budget_usd: f64) -> BudgetLimits {
    let mut limits = BudgetLimits::default();
    limits.daily_usd = normalize_budget_limit(Some(budget_usd));
    limits
}

/// 把「日预算」这类**单值输入**规整成可选上限：非正或非有限值表示不设该上限。
///
/// 仅供 `budget_limits_from_global_budget` 这类「单个 f64 → Option 上限」的入口使用。
/// U112 注意：**不要**把它用到 Auto Mode 预授权额度上——那里的 `Some(0.0)`
/// 是用户显式设定的零额度，规整成 None 会静默解除限制，属安全性倒退。
/// 两个字段对 `0` 的解释不同是有意的：日预算 0 = 未设上限；预授权 0 = 零额度。
fn normalize_budget_limit(value: Option<f64>) -> Option<f64> {
    value.filter(|limit| limit.is_finite() && *limit > 0.0)
}

/// U126：返回该毫秒时间戳所在**本地自然日**的 00:00:00（毫秒）。
///
/// 为什么不用 UTC：日预算文案写的是「今日累计花费上限」。按 UTC 切日意味着
/// UTC+8 的用户在**北京时间早上 8 点**额度重置——熬夜写作时额度会突然刷新，
/// 与用户对「今天」的认知不符。
///
/// 实现约束：项目不引 chrono / time（`core/Cargo.toml` 保持零时间库依赖），
/// 所以这里只做「偏移 → UTC 切日 → 偏移回」，不需要民用历算法。
///
/// 时区偏移由 `local_utc_offset_ms()` 提供；取不到时退化为 UTC，
/// 宁可切日时刻不合预期，也不要因时区探测失败而让预算判定整体失效。
pub fn start_of_local_day_ms(now_ms: u64) -> u64 {
    const MS_PER_DAY: i64 = 86_400_000;
    let offset = crate::contracts::local_utc_offset_ms();
    // 先把时间轴平移到「本地墙钟」，切日，再平移回 UTC。
    let local = now_ms as i64 + offset;
    let local_day_start = local - local.rem_euclid(MS_PER_DAY);
    (local_day_start - offset).max(0) as u64
}

// 时区偏移已上移到 `contracts::clock::local_utc_offset_ms`——`llm/service.rs` 的月度
// 用量切月也要用同一份偏移，两处各持一份会让「今天花了多少」与「这个月花了多少」
// 按不同规则切分。

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

impl BudgetUsage {
    /// 返回本次调用后当天累计成本，供硬预算和 Auto Mode 预授权共用。
    fn spent_today_after_request(self) -> f64 {
        self.spent_today_usd + self.requested_usd
    }

    /// 返回本次调用后当月累计成本。
    fn spent_this_month_after_request(self) -> f64 {
        self.spent_this_month_usd + self.requested_usd
    }
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

    if exceeds(limits.daily_usd, usage.spent_today_after_request()) {
        return BudgetDecision::pause("daily budget limit exceeded");
    }

    if exceeds(limits.monthly_usd, usage.spent_this_month_after_request()) {
        return BudgetDecision::pause("monthly budget limit exceeded");
    }

    if auto_mode.enabled_by_default {
        // Auto Mode 预授权是累计放行额度，不是单次调用额度；否则多次小额调用会绕过上限。
        // U112：`Some(0.0)` 是显式的零额度（任何有成本的调用都暂停），不可当成不限制——
        // 那会把用户刻意设的零额度静默解除。「不限制」只由 `None` 表达。
        if exceeds(
            auto_mode.preauthorized_budget_usd,
            usage.spent_today_after_request(),
        ) {
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
///
/// 注意：这里**不**做 0 值规整。`high_cost_confirmation_usd` 也走本函数，
/// 而它的 `0` 表示「任何有成本的调用都要确认」，是合法且有用的意图。
/// 「0 = 不限制」只适用于会触发 Pause 的预算上限，由 `normalize_budget_limit`
/// 在各上限的入口处显式应用（U112）。
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
    fn auto_mode_uses_cumulative_preauthorized_budget() {
        let decision = evaluate_budget(
            &BudgetLimits::default(),
            &AutoModeConfig {
                enabled_by_default: true,
                preauthorized_budget_usd: Some(1.0),
                ..AutoModeConfig::default()
            },
            BudgetUsage {
                requested_usd: 0.25,
                spent_today_usd: 0.90,
                spent_this_month_usd: 0.90,
            },
        );

        assert_eq!(decision.action, BudgetAction::Pause);
        assert_eq!(
            decision.reason.as_deref(),
            Some("auto mode preauthorized budget exceeded")
        );
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

    #[test]
    fn global_budget_zero_maps_to_unlimited_daily() {
        let limits = budget_limits_from_global_budget(0.0);
        assert_eq!(limits.daily_usd, None);
        let decision = evaluate_budget(
            &limits,
            &AutoModeConfig::default(),
            BudgetUsage {
                requested_usd: 50.0,
                spent_today_usd: 0.0,
                spent_this_month_usd: 0.0,
            },
        );
        // 无日限额时，高成本仍走默认 $1 确认阈值。
        assert_eq!(decision.action, BudgetAction::RequireConfirmation);
    }

    #[test]
    fn global_budget_positive_enforced_as_daily_limit_on_evaluate_budget() {
        let limits = budget_limits_from_global_budget(1.0);
        assert_eq!(limits.daily_usd, Some(1.0));
        let decision = evaluate_budget(
            &limits,
            &AutoModeConfig::default(),
            BudgetUsage {
                requested_usd: 0.40,
                spent_today_usd: 0.70,
                spent_this_month_usd: 0.70,
            },
        );
        assert_eq!(decision.action, BudgetAction::Pause);
        assert_eq!(
            decision.reason.as_deref(),
            Some("daily budget limit exceeded")
        );
    }

    #[test]
    fn global_budget_under_limit_allows_when_not_high_cost() {
        let limits = budget_limits_from_global_budget(5.0);
        let decision = evaluate_budget(
            &limits,
            &AutoModeConfig::default(),
            BudgetUsage {
                requested_usd: 0.50,
                spent_today_usd: 1.0,
                spent_this_month_usd: 1.0,
            },
        );
        assert_eq!(decision.action, BudgetAction::Allow);
    }
}
