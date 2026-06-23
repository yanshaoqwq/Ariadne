use crate::config::ModelConfig;
use crate::core::{CoreError, CoreResult};
use crate::costs::models::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenPricing {
    pub input_cost_per_million_tokens: f64,
    pub output_cost_per_million_tokens: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostEstimate {
    pub expected_usd: f64,
    pub min_usd: f64,
    pub max_usd: f64,
    pub confidence: f32,
}

impl TokenPricing {
    pub fn validate(&self) -> CoreResult<()> {
        for (field, value) in [
            (
                "input_cost_per_million_tokens",
                self.input_cost_per_million_tokens,
            ),
            (
                "output_cost_per_million_tokens",
                self.output_cost_per_million_tokens,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(CoreError::validation(format!(
                    "{field} must be finite and non-negative"
                )));
            }
        }

        Ok(())
    }
}

pub fn estimate_token_cost(usage: TokenUsage, pricing: TokenPricing) -> CoreResult<f64> {
    pricing.validate()?;
    let input_cost =
        usage.input_tokens as f64 * pricing.input_cost_per_million_tokens / 1_000_000.0;
    let output_cost =
        usage.output_tokens as f64 * pricing.output_cost_per_million_tokens / 1_000_000.0;
    Ok(input_cost + output_cost)
}

pub fn estimate_model_config_cost(model: &ModelConfig, usage: TokenUsage) -> CoreResult<f64> {
    estimate_token_cost(
        usage,
        TokenPricing {
            input_cost_per_million_tokens: model.input_cost_per_million_tokens.unwrap_or(0.0),
            output_cost_per_million_tokens: model.output_cost_per_million_tokens.unwrap_or(0.0),
        },
    )
}

pub fn estimate_token_cost_range(
    usage: TokenUsage,
    pricing: TokenPricing,
    tool_use_rounds: Option<u32>,
) -> CoreResult<CostEstimate> {
    let base_cost = estimate_token_cost(usage, pricing)?;
    let estimate = match tool_use_rounds {
        Some(rounds) => {
            let multiplier = f64::from(rounds.max(1));
            let expected = base_cost * multiplier;
            CostEstimate {
                expected_usd: expected,
                min_usd: expected * 0.90,
                max_usd: expected * 1.10,
                confidence: 0.90,
            }
        }
        None => CostEstimate {
            expected_usd: base_cost * 2.0,
            min_usd: base_cost,
            max_usd: base_cost * 5.0,
            confidence: 0.50,
        },
    };

    Ok(estimate)
}

pub fn estimate_model_config_cost_range(
    model: &ModelConfig,
    usage: TokenUsage,
    tool_use_rounds: Option<u32>,
) -> CoreResult<CostEstimate> {
    estimate_token_cost_range(
        usage,
        TokenPricing {
            input_cost_per_million_tokens: model.input_cost_per_million_tokens.unwrap_or(0.0),
            output_cost_per_million_tokens: model.output_cost_per_million_tokens.unwrap_or(0.0),
        },
        tool_use_rounds,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_cost_uses_per_million_rates() {
        let cost = estimate_token_cost(
            TokenUsage {
                input_tokens: 1_000_000,
                output_tokens: 500_000,
            },
            TokenPricing {
                input_cost_per_million_tokens: 2.0,
                output_cost_per_million_tokens: 6.0,
            },
        )
        .unwrap();

        assert_eq!(cost, 5.0);
    }

    #[test]
    fn token_cost_range_is_conservative_when_tool_rounds_are_unknown() {
        let estimate = estimate_token_cost_range(
            TokenUsage {
                input_tokens: 1_000_000,
                output_tokens: 0,
            },
            TokenPricing {
                input_cost_per_million_tokens: 1.0,
                output_cost_per_million_tokens: 1.0,
            },
            None,
        )
        .unwrap();

        assert_eq!(estimate.min_usd, 1.0);
        assert_eq!(estimate.expected_usd, 2.0);
        assert_eq!(estimate.max_usd, 5.0);
        assert!(estimate.confidence < 1.0);
    }
}
