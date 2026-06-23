use serde::{Deserialize, Serialize};

use crate::config::ProviderConfig;
use crate::core::{CoreError, CoreResult, ProviderType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    OpenAi,
    Anthropic,
    Gemini,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolUseEnvelope {
    OpenAiTools,
    AnthropicTools,
    GeminiFunctionDeclarations,
}

impl ProviderProtocol {
    pub fn from_provider_type(provider_type: &ProviderType) -> CoreResult<Self> {
        match provider_type {
            ProviderType::OpenAi | ProviderType::OpenAiCompatible | ProviderType::Local => {
                Ok(Self::OpenAi)
            }
            ProviderType::Anthropic => Ok(Self::Anthropic),
            ProviderType::Gemini => Ok(Self::Gemini),
            ProviderType::Other => Err(CoreError::validation(
                "provider_type other does not define a protocol",
            )),
        }
    }

    pub fn tool_use_envelope(self) -> ToolUseEnvelope {
        match self {
            Self::OpenAi => ToolUseEnvelope::OpenAiTools,
            Self::Anthropic => ToolUseEnvelope::AnthropicTools,
            Self::Gemini => ToolUseEnvelope::GeminiFunctionDeclarations,
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta",
        }
    }
}

pub fn resolve_base_url(config: &ProviderConfig) -> CoreResult<String> {
    let protocol = ProviderProtocol::from_provider_type(&config.provider_type)?;
    if matches!(config.provider_type, ProviderType::OpenAiCompatible) {
        return config
            .base_url
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CoreError::validation("open_ai_compatible provider requires base_url"));
    }

    Ok(config
        .base_url
        .clone()
        .unwrap_or_else(|| protocol.default_base_url().to_owned()))
}

#[cfg(test)]
mod tests {
    use crate::config::ProviderConfig;

    use super::*;

    #[test]
    fn openai_compatible_uses_openai_protocol_with_custom_base_url() {
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

    #[test]
    fn anthropic_and_gemini_tool_envelopes_are_distinct() {
        assert_eq!(
            ProviderProtocol::Anthropic.tool_use_envelope(),
            ToolUseEnvelope::AnthropicTools
        );
        assert_eq!(
            ProviderProtocol::Gemini.tool_use_envelope(),
            ToolUseEnvelope::GeminiFunctionDeclarations
        );
    }
}
