use serde_json::json;

use crate::core::{AutoModeState, CoreError, CoreResult};
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    ConfirmationItem, ConfirmationKind, ConfirmationState, SummaryPipelineDraft,
    SummaryPipelineReport, SummaryPipelineStep, WritingConfirmationPolicy,
};

/// Summarizer 流水线执行器，消费结构化总结草稿并更新创作知识库。
pub struct SummaryPipelineExecutor<'a> {
    knowledge: &'a MemoryWritingKnowledgeBase,
    confirmation_policy: WritingConfirmationPolicy,
    auto_mode: AutoModeState,
}

impl<'a> SummaryPipelineExecutor<'a> {
    /// 创建流水线执行器。
    pub fn new(
        knowledge: &'a MemoryWritingKnowledgeBase,
        confirmation_policy: WritingConfirmationPolicy,
        auto_mode: AutoModeState,
    ) -> Self {
        Self {
            knowledge,
            confirmation_policy,
            auto_mode,
        }
    }

    /// 执行故事段 -> 事件 -> 章节 -> 阶段的写入流程。
    pub fn apply_draft(&self, draft: SummaryPipelineDraft) -> CoreResult<SummaryPipelineReport> {
        if draft.chapter_id.trim().is_empty() {
            return Err(CoreError::validation("chapter_id cannot be empty"));
        }

        let mut completed_steps = Vec::new();
        for segment in draft.segments {
            self.knowledge.upsert_segment(segment)?;
        }
        self.enqueue_confirmation(
            ConfirmationKind::SegmentSummary,
            &draft.chapter_id,
            json!({ "step": "segment" }),
        )?;
        completed_steps.push(SummaryPipelineStep::Segment);

        for event in draft.events {
            self.knowledge.upsert_event(event)?;
        }
        self.enqueue_confirmation(
            ConfirmationKind::EventSummary,
            &draft.chapter_id,
            json!({ "step": "event" }),
        )?;
        completed_steps.push(SummaryPipelineStep::Event);

        if let Some(summary) = draft.chapter_summary {
            self.knowledge
                .upsert_chapter_summary(&draft.chapter_id, summary)?;
            self.enqueue_confirmation(
                ConfirmationKind::ChapterSummary,
                &draft.chapter_id,
                json!({ "step": "chapter" }),
            )?;
            completed_steps.push(SummaryPipelineStep::Chapter);
        }

        if let (Some(stage_id), Some(stage_summary)) = (draft.stage_id, draft.stage_summary) {
            self.knowledge
                .upsert_stage_summary(stage_id, stage_summary)?;
            self.enqueue_confirmation(
                ConfirmationKind::StageSummary,
                &draft.chapter_id,
                json!({ "step": "stage" }),
            )?;
            completed_steps.push(SummaryPipelineStep::Stage);
        }

        for realized in draft.realized_changes {
            self.knowledge
                .mark_change_realized(&realized.change_id, &realized.segment_id)?;
        }
        let issues = self
            .knowledge
            .queue_unrealized_changes_for_chapter(&draft.chapter_id)?;

        let pending_confirmations = has_pending_confirmations(self.knowledge)?;
        let has_unrealized_issues = !issues.is_empty();
        Ok(SummaryPipelineReport {
            chapter_id: draft.chapter_id,
            completed_steps,
            paused: pending_confirmations || has_unrealized_issues,
            pause_reason: if has_unrealized_issues {
                Some(format!("{} planner changes are not realized", issues.len()))
            } else if pending_confirmations {
                Some("pending confirmation items".to_owned())
            } else {
                None
            },
        })
    }

    /// 根据确认策略创建确认项。
    fn enqueue_confirmation(
        &self,
        kind: ConfirmationKind,
        chapter_id: &str,
        metadata: serde_json::Value,
    ) -> CoreResult<ConfirmationItem> {
        let state = self
            .confirmation_policy
            .initial_state(kind, &self.auto_mode);
        let item = ConfirmationItem::new(confirmation_id(kind, chapter_id), kind, state, metadata);
        self.knowledge.upsert_confirmation(item.clone())?;
        Ok(item)
    }
}

/// 把待确认项状态推进为已通过。
pub fn approve_confirmation(
    knowledge: &MemoryWritingKnowledgeBase,
    confirmation_id: &str,
) -> CoreResult<ConfirmationItem> {
    knowledge.update_confirmation_state(confirmation_id, ConfirmationState::Approved)
}

/// 把待确认项状态推进为已拒绝。
pub fn reject_confirmation(
    knowledge: &MemoryWritingKnowledgeBase,
    confirmation_id: &str,
) -> CoreResult<ConfirmationItem> {
    knowledge.update_confirmation_state(confirmation_id, ConfirmationState::Rejected)
}

/// 判断当前是否仍有待人工确认项。
pub fn has_pending_confirmations(knowledge: &MemoryWritingKnowledgeBase) -> CoreResult<bool> {
    Ok(!knowledge
        .confirmations(Some(ConfirmationState::Pending))?
        .is_empty())
}

/// 生成稳定确认项 id。
fn confirmation_id(kind: ConfirmationKind, chapter_id: &str) -> String {
    let name = match kind {
        ConfirmationKind::OutlinerOutput => "outliner-output",
        ConfirmationKind::DesignerOutput => "designer-output",
        ConfirmationKind::PlannerOutput => "planner-output",
        ConfirmationKind::PlannerRegister => "planner-register",
        ConfirmationKind::CriticReview => "critic-review",
        ConfirmationKind::PrudentReview => "prudent-review",
        ConfirmationKind::SegmentSummary => "segment-summary",
        ConfirmationKind::EventSummary => "event-summary",
        ConfirmationKind::ChapterSummary => "chapter-summary",
        ConfirmationKind::StageSummary => "stage-summary",
        ConfirmationKind::WriterCorrectionPatch => "writer-correction-patch",
    };
    format!("{chapter_id}::{name}")
}
