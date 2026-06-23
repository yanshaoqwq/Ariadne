use serde::{Deserialize, Serialize};

use crate::core::{ApprovalPolicy, AutoModeState, CoreResult};
use crate::knowledge::models::{
    ApprovalStatus, FactProposal, KnowledgeConflict, KnowledgeFact, KnowledgeRebuildReport,
    KnowledgeRebuildStatus, ProposalDecision, TwoStepApproval,
};

/// 知识库审批策略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeApprovalPolicy {
    pub require_human_confirmation: bool,
    pub node_approval: ApprovalPolicy,
}

impl Default for KnowledgeApprovalPolicy {
    /// 默认普通模式需要人工确认，并沿用节点审批策略。
    fn default() -> Self {
        Self {
            require_human_confirmation: true,
            node_approval: ApprovalPolicy::default(),
        }
    }
}

/// 根据候选事实、既有事实、审批和 Auto Mode 计算处理结果。
pub fn decide_proposal(
    proposal: &FactProposal,
    existing: Option<&KnowledgeFact>,
    approval: Option<&TwoStepApproval>,
    policy: &KnowledgeApprovalPolicy,
    auto_mode: &AutoModeState,
) -> CoreResult<ProposalDecision> {
    let has_conflict = existing
        .map(|fact| fact.fact.value != proposal.candidate.fact.value)
        .unwrap_or(false);

    if has_conflict {
        let existing = existing.expect("has_conflict requires existing fact");
        return Ok(ProposalDecision {
            proposal_id: proposal.proposal_id.clone(),
            status: ApprovalStatus::Conflict,
            fact_id: None,
            conflict_id: Some(conflict_id(&proposal.proposal_id, &existing.fact_id)),
            requires_human_review: true,
            reason: "proposal conflicts with existing fact and was queued for review".to_owned(),
        });
    }

    if let Some(approval) = approval {
        approval.validate()?;
        if approval.approved {
            return Ok(ProposalDecision {
                proposal_id: proposal.proposal_id.clone(),
                status: ApprovalStatus::Approved,
                fact_id: Some(proposal.candidate.fact_id.clone()),
                conflict_id: None,
                requires_human_review: false,
                reason: "proposal approved by two-step approval".to_owned(),
            });
        }
        return Ok(ProposalDecision {
            proposal_id: proposal.proposal_id.clone(),
            status: ApprovalStatus::Rejected,
            fact_id: None,
            conflict_id: None,
            requires_human_review: false,
            reason: "proposal rejected by two-step approval".to_owned(),
        });
    }

    let can_auto_approve = policy.node_approval.should_auto_approve(auto_mode, false)
        || (auto_mode.enabled && !policy.require_human_confirmation);
    if can_auto_approve {
        return Ok(ProposalDecision {
            proposal_id: proposal.proposal_id.clone(),
            status: ApprovalStatus::Approved,
            fact_id: Some(proposal.candidate.fact_id.clone()),
            conflict_id: None,
            requires_human_review: false,
            reason: "proposal auto-approved without conflict".to_owned(),
        });
    }

    Ok(ProposalDecision {
        proposal_id: proposal.proposal_id.clone(),
        status: ApprovalStatus::Pending,
        fact_id: None,
        conflict_id: None,
        requires_human_review: true,
        reason: "proposal requires human confirmation".to_owned(),
    })
}

/// 由冲突提案创建冲突队列项。
pub fn make_conflict(
    proposal: &FactProposal,
    existing: &KnowledgeFact,
    approval: Option<&TwoStepApproval>,
) -> CoreResult<KnowledgeConflict> {
    let (writing_reason, judge_reason) = if let Some(approval) = approval {
        approval.validate()?;
        (
            approval.writing_reason.clone(),
            approval.judge_reason.clone(),
        )
    } else {
        // 冲突不能因审批尚未完成而丢失：先入队，后续再补独立判断。
        (
            proposal.extraction_reason.clone(),
            "awaiting independent LLM judgment".to_owned(),
        )
    };

    Ok(KnowledgeConflict {
        conflict_id: conflict_id(&proposal.proposal_id, &existing.fact_id),
        key: proposal.candidate.conflict_key(),
        existing_fact_id: existing.fact_id.clone(),
        proposed_fact: proposal.candidate.clone(),
        status: ApprovalStatus::Conflict,
        writing_reason,
        judge_reason,
        sources: proposal.candidate.fact.sources.clone(),
    })
}

/// 生成稳定冲突 id。
fn conflict_id(proposal_id: &str, existing_fact_id: &str) -> String {
    format!("conflict::{existing_fact_id}::{proposal_id}")
}

/// 根据当前计数生成重建完成报告。
pub fn rebuild_report_from_counts(
    processed_documents: u64,
    processed_facts: u64,
    processed_summaries: u64,
) -> KnowledgeRebuildReport {
    KnowledgeRebuildReport {
        status: KnowledgeRebuildStatus::Completed,
        processed_documents,
        processed_facts,
        processed_summaries,
        error: None,
    }
}
