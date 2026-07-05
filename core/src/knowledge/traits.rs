use crate::contracts::CoreResult;
use crate::knowledge::models::{
    FactProposal, KnowledgeConflict, KnowledgeFact, KnowledgeFactKey, KnowledgeHealthReport,
    KnowledgeRebuildReason, KnowledgeRebuildReport, LayeredSummary, ProposalDecision,
    TwoStepApproval,
};

/// 知识库仓储抽象，后续可替换为 SQLite 持久化实现。
pub trait KnowledgeRepository {
    /// 写入或更新分层摘要。
    fn upsert_summary(&self, summary: LayeredSummary) -> CoreResult<()>;

    /// 按 id 读取摘要。
    fn summary(&self, summary_id: &str) -> CoreResult<Option<LayeredSummary>>;

    /// 写入已确认事实。
    fn upsert_fact(&self, fact: KnowledgeFact) -> CoreResult<()>;

    /// 按冲突 key 查找事实。
    fn fact_by_key(&self, key: &KnowledgeFactKey) -> CoreResult<Option<KnowledgeFact>>;

    /// 按 id 读取事实。
    fn fact(&self, fact_id: &str) -> CoreResult<Option<KnowledgeFact>>;

    /// 写入冲突队列项。
    fn enqueue_conflict(&self, conflict: KnowledgeConflict) -> CoreResult<()>;

    /// 列出冲突队列。
    fn list_conflicts(&self) -> CoreResult<Vec<KnowledgeConflict>>;

    /// 处理 AI 抽取候选事实。
    fn apply_proposal(
        &self,
        proposal: FactProposal,
        approval: Option<TwoStepApproval>,
    ) -> CoreResult<ProposalDecision>;

    /// 标记知识库需要重建。
    fn mark_rebuild_required(&self, reason: KnowledgeRebuildReason, message: impl Into<String>);

    /// 返回知识库健康状态。
    fn health_report(&self) -> KnowledgeHealthReport;

    /// 完成一次重建并返回报告。
    fn complete_rebuild(&self) -> CoreResult<KnowledgeRebuildReport>;
}
