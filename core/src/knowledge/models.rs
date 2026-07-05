use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::SourceSpan;

/// 分层摘要的粒度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryLevel {
    Chunk,
    Scene,
    Chapter,
    Volume,
    Book,
}

/// 结构化事实类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    Character,
    Alias,
    Relationship,
    Worldbuilding,
    Timeline,
    ChapterSummary,
    Other,
}

/// 审批状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Conflict,
}

/// 重建原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeRebuildReason {
    MetadataCorrupt,
    IndexCorrupt,
    GitRestore,
}

/// 知识库重建状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeRebuildStatus {
    #[default]
    NotRequired,
    Required,
    Running,
    Completed,
    Failed,
}

/// 带来源和版本的事实值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionedFactValue {
    pub value: Value,
    pub source_version: String,
    #[serde(default)]
    pub sources: Vec<SourceSpan>,
}

impl VersionedFactValue {
    /// 校验事实值必须可追溯到来源。
    pub fn validate_sources(&self) -> crate::contracts::CoreResult<()> {
        if self.source_version.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "knowledge source_version cannot be empty",
            ));
        }
        if self.sources.is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "knowledge fact requires at least one source span",
            ));
        }

        Ok(())
    }
}

/// 结构化事实记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeFact {
    pub fact_id: String,
    pub kind: FactKind,
    pub entity: String,
    pub attribute: String,
    pub fact: VersionedFactValue,
    #[serde(default)]
    pub metadata: Value,
}

impl KnowledgeFact {
    /// 返回用于冲突检查的稳定 key。
    pub fn conflict_key(&self) -> KnowledgeFactKey {
        KnowledgeFactKey {
            kind: self.kind,
            entity: self.entity.clone(),
            attribute: self.attribute.clone(),
        }
    }

    /// 校验事实记录的必填字段和来源。
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        validate_id("fact_id", &self.fact_id)?;
        validate_id("entity", &self.entity)?;
        validate_id("attribute", &self.attribute)?;
        self.fact.validate_sources()
    }
}

/// 事实冲突 key。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KnowledgeFactKey {
    pub kind: FactKind,
    pub entity: String,
    pub attribute: String,
}

/// 分层摘要记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayeredSummary {
    pub summary_id: String,
    pub level: SummaryLevel,
    pub subject_id: String,
    pub text: String,
    pub source_version: String,
    #[serde(default)]
    pub sources: Vec<SourceSpan>,
    #[serde(default)]
    pub metadata: Value,
}

impl LayeredSummary {
    /// 校验摘要必须保留来源和版本。
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        validate_id("summary_id", &self.summary_id)?;
        validate_id("subject_id", &self.subject_id)?;
        if self.text.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "summary text cannot be empty",
            ));
        }
        if self.source_version.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "summary source_version cannot be empty",
            ));
        }
        if self.sources.is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "summary requires at least one source span",
            ));
        }

        Ok(())
    }
}

/// AI 抽取出的候选事实。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactProposal {
    pub proposal_id: String,
    pub candidate: KnowledgeFact,
    pub extraction_reason: String,
    #[serde(default)]
    pub metadata: Value,
}

impl FactProposal {
    /// 校验候选事实和抽取理由。
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        validate_id("proposal_id", &self.proposal_id)?;
        if self.extraction_reason.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "proposal extraction_reason cannot be empty",
            ));
        }
        self.candidate.validate()
    }
}

/// 两步 LLM 审批结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwoStepApproval {
    pub writing_reason: String,
    pub judge_reason: String,
    pub approved: bool,
    #[serde(default)]
    pub metadata: Value,
}

impl TwoStepApproval {
    /// 校验两步审批理由都存在。
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        if self.writing_reason.trim().is_empty() || self.judge_reason.trim().is_empty() {
            return Err(crate::contracts::CoreError::validation(
                "two-step approval requires both reasons",
            ));
        }

        Ok(())
    }
}

/// 冲突队列记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeConflict {
    pub conflict_id: String,
    pub key: KnowledgeFactKey,
    pub existing_fact_id: String,
    pub proposed_fact: KnowledgeFact,
    pub status: ApprovalStatus,
    pub writing_reason: String,
    pub judge_reason: String,
    #[serde(default)]
    pub sources: Vec<SourceSpan>,
}

/// 提案处理结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalDecision {
    pub proposal_id: String,
    pub status: ApprovalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_id: Option<String>,
    pub requires_human_review: bool,
    pub reason: String,
}

/// 知识库健康报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeHealthReport {
    pub status: KnowledgeRebuildStatus,
    pub metadata_rebuild_required: bool,
    pub index_rebuild_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<KnowledgeRebuildReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 知识库重建报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeRebuildReport {
    pub status: KnowledgeRebuildStatus,
    pub processed_documents: u64,
    pub processed_facts: u64,
    pub processed_summaries: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 校验非空 id。
fn validate_id(field: &str, value: &str) -> crate::contracts::CoreResult<()> {
    if value.trim().is_empty() {
        return Err(crate::contracts::CoreError::validation(format!(
            "{field} cannot be empty"
        )));
    }

    Ok(())
}
