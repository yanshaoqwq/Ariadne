use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{CoreError, CoreResult, NodeId, RunId, WorkflowId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostCategory {
    Llm,
    Embedding,
    Reranker,
    SearchApi,
    HttpSkill,
    Approval,
    Other,
}

impl CostCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Embedding => "embedding",
            Self::Reranker => "reranker",
            Self::SearchApi => "search_api",
            Self::HttpSkill => "http_skill",
            Self::Approval => "approval",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value {
            "llm" => Ok(Self::Llm),
            "embedding" => Ok(Self::Embedding),
            "reranker" => Ok(Self::Reranker),
            "search_api" => Ok(Self::SearchApi),
            "http_skill" => Ok(Self::HttpSkill),
            "approval" => Ok(Self::Approval),
            "other" => Ok(Self::Other),
            other => Err(CoreError::validation(format!(
                "unknown cost category: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewCostRecord {
    pub occurred_at_ms: u64,
    pub category: CostCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<WorkflowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    pub amount_usd: f64,
    #[serde(default)]
    pub metadata: Value,
}

impl NewCostRecord {
    pub fn validate(&self) -> CoreResult<()> {
        if !self.amount_usd.is_finite() || self.amount_usd < 0.0 {
            return Err(CoreError::validation(
                "amount_usd must be finite and non-negative",
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostRecord {
    pub cost_id: i64,
    #[serde(flatten)]
    pub record: NewCostRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CostQuery {
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub workflow_id: Option<WorkflowId>,
    pub run_id: Option<RunId>,
    pub node_id: Option<NodeId>,
    pub category: Option<CostCategory>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}
