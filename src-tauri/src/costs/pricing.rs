use crate::config::ModelConfig;
use crate::core::{CoreError, CoreResult};
use crate::costs::models::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenPricing {
    pub input_cost_per_million_tokens: f64,
    pub output_cost_per_million_tokens: f64,
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
}
