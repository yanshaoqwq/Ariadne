use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::{CoreError, CoreResult, NodeId, RunId, WorkflowId};

/// 成本记录类别。
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
    /// 返回数据库中使用的稳定字符串。
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

    /// 从数据库字符串解析成本类别。
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

/// 即将写入账本的新成本记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewCostRecord {
    pub occurred_at_ms: u64,
    /// 外部副作用的稳定操作 ID；存在时用于成本写入幂等去重。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
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
    /// 校验成本金额合法。
    pub fn validate(&self) -> CoreResult<()> {
        if self
            .operation_id
            .as_deref()
            .is_some_and(|operation_id| operation_id.trim().is_empty())
        {
            return Err(CoreError::validation("cost operation_id cannot be blank"));
        }
        if !self.amount_usd.is_finite() || self.amount_usd < 0.0 {
            return Err(CoreError::validation(
                "amount_usd must be finite and non-negative",
            ));
        }

        Ok(())
    }
}

/// 已写入账本并带有数据库 id 的成本记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostRecord {
    pub cost_id: i64,
    #[serde(flatten)]
    pub record: NewCostRecord,
}

/// 成本查询条件。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CostQuery {
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub operation_id: Option<String>,
    pub workflow_id: Option<WorkflowId>,
    pub run_id: Option<RunId>,
    pub node_id: Option<NodeId>,
    pub category: Option<CostCategory>,
}

/// 模型 token 使用量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenUsage {
    /// 返回输入和输出 token 的总量。
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}
