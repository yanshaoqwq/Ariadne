use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::core::{AutoModeState, CoreError, CoreResult};
use crate::knowledge::models::{
    ApprovalStatus, FactProposal, KnowledgeConflict, KnowledgeFact, KnowledgeFactKey,
    KnowledgeHealthReport, KnowledgeRebuildReason, KnowledgeRebuildReport, KnowledgeRebuildStatus,
    LayeredSummary, ProposalDecision, TwoStepApproval,
};
use crate::knowledge::service::{
    decide_proposal, make_conflict, rebuild_report_from_counts, KnowledgeApprovalPolicy,
};
use crate::knowledge::traits::KnowledgeRepository;

/// 内存知识库状态。
#[derive(Debug, Default)]
struct MemoryKnowledgeState {
    summaries: BTreeMap<String, LayeredSummary>,
    facts_by_id: BTreeMap<String, KnowledgeFact>,
    fact_ids_by_key: BTreeMap<KnowledgeFactKey, String>,
    conflicts: BTreeMap<String, KnowledgeConflict>,
    rebuild_status: KnowledgeRebuildStatus,
    rebuild_reason: Option<KnowledgeRebuildReason>,
    rebuild_message: Option<String>,
}

/// 内存知识库，供测试和早期集成使用。
#[derive(Debug)]
pub struct MemoryKnowledgeBase {
    state: Mutex<MemoryKnowledgeState>,
    approval_policy: KnowledgeApprovalPolicy,
    auto_mode: AutoModeState,
}

impl MemoryKnowledgeBase {
    /// 创建内存知识库。
    pub fn new(approval_policy: KnowledgeApprovalPolicy, auto_mode: AutoModeState) -> Self {
        Self {
            state: Mutex::new(MemoryKnowledgeState {
                rebuild_status: KnowledgeRebuildStatus::NotRequired,
                ..MemoryKnowledgeState::default()
            }),
            approval_policy,
            auto_mode,
        }
    }

    /// 获取内部状态锁。
    fn state(&self) -> CoreResult<std::sync::MutexGuard<'_, MemoryKnowledgeState>> {
        self.state
            .lock()
            .map_err(|_| CoreError::validation("knowledge state lock poisoned"))
    }
}

impl KnowledgeRepository for MemoryKnowledgeBase {
    /// 写入或更新分层摘要。
    fn upsert_summary(&self, summary: LayeredSummary) -> CoreResult<()> {
        summary.validate()?;
        self.state()?
            .summaries
            .insert(summary.summary_id.clone(), summary);
        Ok(())
    }

    /// 按 id 读取摘要。
    fn summary(&self, summary_id: &str) -> CoreResult<Option<LayeredSummary>> {
        Ok(self.state()?.summaries.get(summary_id).cloned())
    }

    /// 写入已确认事实。
    fn upsert_fact(&self, fact: KnowledgeFact) -> CoreResult<()> {
        fact.validate()?;
        let mut state = self.state()?;
        state
            .fact_ids_by_key
            .insert(fact.conflict_key(), fact.fact_id.clone());
        state.facts_by_id.insert(fact.fact_id.clone(), fact);
        Ok(())
    }

    /// 按冲突 key 查找事实。
    fn fact_by_key(&self, key: &KnowledgeFactKey) -> CoreResult<Option<KnowledgeFact>> {
        let state = self.state()?;
        let Some(fact_id) = state.fact_ids_by_key.get(key) else {
            return Ok(None);
        };
        Ok(state.facts_by_id.get(fact_id).cloned())
    }

    /// 按 id 读取事实。
    fn fact(&self, fact_id: &str) -> CoreResult<Option<KnowledgeFact>> {
        Ok(self.state()?.facts_by_id.get(fact_id).cloned())
    }

    /// 写入冲突队列项。
    fn enqueue_conflict(&self, conflict: KnowledgeConflict) -> CoreResult<()> {
        let mut state = self.state()?;
        state
            .conflicts
            .insert(conflict.conflict_id.clone(), conflict);
        Ok(())
    }

    /// 列出冲突队列。
    fn list_conflicts(&self) -> CoreResult<Vec<KnowledgeConflict>> {
        Ok(self.state()?.conflicts.values().cloned().collect())
    }

    /// 处理 AI 抽取候选事实。
    fn apply_proposal(
        &self,
        proposal: FactProposal,
        approval: Option<TwoStepApproval>,
    ) -> CoreResult<ProposalDecision> {
        proposal.validate()?;
        if let Some(approval) = &approval {
            approval.validate()?;
        }

        let mut state = self.state()?;
        let existing = state
            .fact_ids_by_key
            .get(&proposal.candidate.conflict_key())
            .and_then(|fact_id| state.facts_by_id.get(fact_id))
            .cloned();
        let decision = decide_proposal(
            &proposal,
            existing.as_ref(),
            approval.as_ref(),
            &self.approval_policy,
            &self.auto_mode,
        )?;

        match decision.status {
            ApprovalStatus::Approved => {
                state.fact_ids_by_key.insert(
                    proposal.candidate.conflict_key(),
                    proposal.candidate.fact_id.clone(),
                );
                state
                    .facts_by_id
                    .insert(proposal.candidate.fact_id.clone(), proposal.candidate);
            }
            ApprovalStatus::Conflict => {
                let existing = existing.ok_or_else(|| {
                    CoreError::validation("conflict decision requires an existing fact")
                })?;
                let conflict = make_conflict(&proposal, &existing, approval.as_ref())?;
                state
                    .conflicts
                    .insert(conflict.conflict_id.clone(), conflict);
            }
            ApprovalStatus::Pending | ApprovalStatus::Rejected => {}
        }

        Ok(decision)
    }

    /// 标记知识库需要重建。
    fn mark_rebuild_required(&self, reason: KnowledgeRebuildReason, message: impl Into<String>) {
        if let Ok(mut state) = self.state.lock() {
            state.rebuild_status = KnowledgeRebuildStatus::Required;
            state.rebuild_reason = Some(reason);
            state.rebuild_message = Some(message.into());
        }
    }

    /// 返回知识库健康状态。
    fn health_report(&self) -> KnowledgeHealthReport {
        let Ok(state) = self.state.lock() else {
            return KnowledgeHealthReport {
                status: KnowledgeRebuildStatus::Failed,
                metadata_rebuild_required: true,
                index_rebuild_required: true,
                reason: Some(KnowledgeRebuildReason::MetadataCorrupt),
                message: Some("knowledge state lock poisoned".to_owned()),
            };
        };

        KnowledgeHealthReport {
            status: state.rebuild_status,
            metadata_rebuild_required: matches!(
                state.rebuild_reason,
                Some(KnowledgeRebuildReason::MetadataCorrupt | KnowledgeRebuildReason::GitRestore)
            ),
            index_rebuild_required: matches!(
                state.rebuild_reason,
                Some(KnowledgeRebuildReason::IndexCorrupt | KnowledgeRebuildReason::GitRestore)
            ),
            reason: state.rebuild_reason,
            message: state.rebuild_message.clone(),
        }
    }

    /// 完成一次重建并返回报告。
    fn complete_rebuild(&self) -> CoreResult<KnowledgeRebuildReport> {
        let mut state = self.state()?;
        state.rebuild_status = KnowledgeRebuildStatus::Completed;
        state.rebuild_reason = None;
        state.rebuild_message = None;
        Ok(rebuild_report_from_counts(
            0,
            state.facts_by_id.len() as u64,
            state.summaries.len() as u64,
        ))
    }
}
