use serde_json::json;

use crate::contracts::{AutoModeState, CoreError, CoreResult};
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    ConfirmationItem, ConfirmationKind, ConfirmationState, SummaryPipelineDraft,
    SummaryPipelineReport, SummaryPipelineStep, SummaryRerunPlan, WritingConfirmationPolicy,
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
        validate_complete_draft(&draft)?;

        let chapter_id = draft.chapter_id.clone();
        let mut completed_steps = Vec::new();
        let mut confirmation_ids = Vec::new();
        for segment in draft.segments {
            self.knowledge.upsert_segment(segment)?;
        }
        confirmation_ids.push(
            self.enqueue_confirmation(
                ConfirmationKind::SegmentSummary,
                &chapter_id,
                json!({
                    "step": "segment",
                    "chapter_id": chapter_id.clone(),
                    "segment_count": self
                        .knowledge
                        .index_snapshot()?
                        .chapter_segments
                        .get(&chapter_id)
                        .map(|segments| segments.len())
                        .unwrap_or_default()
                }),
            )?
            .confirmation_id,
        );
        completed_steps.push(SummaryPipelineStep::Segment);

        for realized in draft.realized_changes {
            self.knowledge
                .mark_change_realized(&realized.change_id, &realized.segment_id)?;
        }
        for update in draft.foreshadowing_updates {
            self.knowledge.apply_foreshadowing_update(update)?;
        }

        for event in draft.events {
            self.knowledge.upsert_event(event)?;
        }
        confirmation_ids.push(
            self.enqueue_confirmation(
                ConfirmationKind::EventSummary,
                &chapter_id,
                json!({ "step": "event", "chapter_id": chapter_id.clone() }),
            )?
            .confirmation_id,
        );
        completed_steps.push(SummaryPipelineStep::Event);

        let chapter_summary = draft.chapter_summary.expect("validated chapter summary");
        self.knowledge
            .upsert_chapter_summary(&chapter_id, chapter_summary)?;
        confirmation_ids.push(
            self.enqueue_confirmation(
                ConfirmationKind::ChapterSummary,
                &chapter_id,
                json!({ "step": "chapter", "chapter_id": chapter_id.clone() }),
            )?
            .confirmation_id,
        );
        completed_steps.push(SummaryPipelineStep::Chapter);

        let stage_id = draft.stage_id.expect("validated stage id");
        let stage_summary = draft.stage_summary.expect("validated stage summary");
        self.knowledge.link_chapter_stage(&chapter_id, &stage_id)?;
        self.knowledge
            .upsert_stage_summary(&stage_id, stage_summary)?;
        confirmation_ids.push(
            self.enqueue_confirmation(
                ConfirmationKind::StageSummary,
                &chapter_id,
                json!({
                    "step": "stage",
                    "chapter_id": chapter_id.clone(),
                    "stage_id": stage_id
                }),
            )?
            .confirmation_id,
        );
        completed_steps.push(SummaryPipelineStep::Stage);

        let issues = self
            .knowledge
            .queue_unrealized_changes_for_chapter(&chapter_id)?;
        let planner_issue_ids = issues
            .iter()
            .map(|issue| issue.issue_id.clone())
            .collect::<Vec<_>>();

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
            planner_issue_ids,
            confirmation_ids,
        })
    }

    /// Writer 补写 patch 写回已审核正文后，从故事段步骤重跑受影响流水线。
    pub fn plan_rerun_after_patch_write_back(
        &self,
        chapter_id: &str,
        reason: &str,
    ) -> CoreResult<SummaryRerunPlan> {
        SummaryRerunPlan::new(chapter_id, SummaryPipelineStep::Segment, reason)
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
        ConfirmationKind::PolisherCorrectionPatch => "polisher-correction-patch",
    };
    format!("{chapter_id}::{name}")
}

/// 校验正式章节总结草稿必须完整覆盖四个步骤。
fn validate_complete_draft(draft: &SummaryPipelineDraft) -> CoreResult<()> {
    if draft.chapter_id.trim().is_empty() {
        return Err(CoreError::validation("chapter_id cannot be empty"));
    }
    if draft.segments.is_empty() {
        return Err(CoreError::validation(
            "summary pipeline requires at least one story segment",
        ));
    }
    for segment in &draft.segments {
        if segment.chapter_id != draft.chapter_id {
            return Err(CoreError::validation(
                "story segment chapter_id must match pipeline chapter_id",
            ));
        }
    }
    if draft.events.is_empty() {
        return Err(CoreError::validation(
            "summary pipeline requires at least one changed event",
        ));
    }
    for event in &draft.events {
        if !event.chapter_ids.iter().any(|id| id == &draft.chapter_id) {
            return Err(CoreError::validation(
                "changed event must link back to pipeline chapter_id",
            ));
        }
    }
    validate_required_text("chapter_summary", draft.chapter_summary.as_deref())?;
    validate_required_text("stage_id", draft.stage_id.as_deref())?;
    validate_required_text("stage_summary", draft.stage_summary.as_deref())?;
    Ok(())
}

fn validate_required_text(field: &str, value: Option<&str>) -> CoreResult<()> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(()),
        _ => Err(CoreError::validation(format!(
            "summary pipeline requires {field}"
        ))),
    }
}
