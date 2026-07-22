use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::contracts::{AutoModeState, CoreError, CoreResult};
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    confirmation_state_activates_knowledge, ConfirmationAuditDecision, ConfirmationItem,
    ConfirmationKind, ConfirmationState, SummaryPipelineDraft, SummaryPipelineReport,
    SummaryPipelineStep, SummaryRerunPlan, WritingConfirmationPolicy,
};

/// Summarizer 流水线执行器，消费结构化总结草稿并更新创作知识库。
pub struct SummaryPipelineExecutor<'a> {
    knowledge: &'a MemoryWritingKnowledgeBase,
    confirmation_policy: WritingConfirmationPolicy,
    auto_mode: AutoModeState,
    cancellation: crate::contracts::CancellationToken,
    auto_audit_decisions: Option<BTreeMap<ConfirmationKind, ConfirmationAuditDecision>>,
}

impl<'a> SummaryPipelineExecutor<'a> {
    /// 创建流水线执行器。
    pub fn new(
        knowledge: &'a MemoryWritingKnowledgeBase,
        confirmation_policy: WritingConfirmationPolicy,
        auto_mode: AutoModeState,
    ) -> Self {
        Self::with_cancellation(
            knowledge,
            confirmation_policy,
            auto_mode,
            crate::contracts::CancellationToken::new(),
        )
    }

    /// F11：workflow 执行路径注入共享取消 token。
    pub fn with_cancellation(
        knowledge: &'a MemoryWritingKnowledgeBase,
        confirmation_policy: WritingConfirmationPolicy,
        auto_mode: AutoModeState,
        cancellation: crate::contracts::CancellationToken,
    ) -> Self {
        Self {
            knowledge,
            confirmation_policy,
            auto_mode,
            cancellation,
            auto_audit_decisions: None,
        }
    }

    /// 生产执行路径必须显式提供每个 AutoAudit 确认项的模型审计决定。
    pub fn with_auto_audit_decisions(
        mut self,
        decisions: BTreeMap<ConfirmationKind, ConfirmationAuditDecision>,
    ) -> Self {
        self.auto_audit_decisions = Some(decisions);
        self
    }

    /// 执行故事段 -> 事件 -> 章节 -> 阶段的写入流程。
    /// F14：仅将已激活（Approved/AutoAudited/Skipped）步骤写入 active 知识；
    /// Pending 步骤载荷放在 confirmation.metadata.pending_payload，批准后物化。
    /// F21：先完整校验并组装确认项，再经单一内存事务提交。
    pub fn apply_draft(
        &self,
        mut draft: SummaryPipelineDraft,
    ) -> CoreResult<SummaryPipelineReport> {
        let revision_id = summary_revision_id(&draft);
        attach_revision_metadata(&mut draft, &revision_id);
        validate_complete_draft(&draft)?;
        validate_stage_identity(self.knowledge, &draft)?;

        let chapter_id = draft.chapter_id.clone();
        let chapter_summary = draft
            .chapter_summary
            .clone()
            .expect("validated chapter summary");
        let stage_id = draft.stage_id.clone().expect("validated stage id");
        let stage_summary = draft
            .stage_summary
            .clone()
            .expect("validated stage summary");
        let is_new_stage = resolve_is_new_stage(self.knowledge, &stage_id, draft.is_new_stage)?;
        let segment_count = draft.segments.len();
        let segments_owned = draft.segments.clone();
        let events_owned = draft.events.clone();
        let realized_owned = draft.realized_changes.clone();
        let foreshadowing_owned = draft.foreshadowing_updates.clone();

        let mut seg_conf = self.build_confirmation(
            ConfirmationKind::SegmentSummary,
            &chapter_id,
            &revision_id,
            json!({
                "step": "segment",
                "chapter_id": chapter_id.clone(),
                "segment_count": segment_count,
                "pending_payload": {
                    "segments": segments_owned,
                    "realized_changes": realized_owned,
                    "foreshadowing_updates": foreshadowing_owned,
                }
            }),
        )?;
        let mut event_conf = self.build_confirmation(
            ConfirmationKind::EventSummary,
            &chapter_id,
            &revision_id,
            json!({
                "step": "event",
                "chapter_id": chapter_id.clone(),
                "pending_payload": { "events": events_owned }
            }),
        )?;
        let mut chapter_conf = self.build_confirmation(
            ConfirmationKind::ChapterSummary,
            &chapter_id,
            &revision_id,
            json!({
                "step": "chapter",
                "chapter_id": chapter_id.clone(),
                "pending_payload": { "chapter_summary": chapter_summary }
            }),
        )?;
        let mut stage_conf = self.build_confirmation(
            ConfirmationKind::StageSummary,
            &chapter_id,
            &revision_id,
            json!({
                "step": "stage",
                "chapter_id": chapter_id.clone(),
                "stage_id": stage_id.clone(),
                "is_new_stage": is_new_stage,
                "pending_payload": {
                    "stage_id": stage_id.clone(),
                    "stage_summary": stage_summary.clone(),
                    "is_new_stage": is_new_stage
                }
            }),
        )?;

        let activate_segment = confirmation_state_activates_knowledge(seg_conf.state);
        let activate_event = confirmation_state_activates_knowledge(event_conf.state);
        let activate_chapter = confirmation_state_activates_knowledge(chapter_conf.state);
        let activate_stage = confirmation_state_activates_knowledge(stage_conf.state);

        if activate_segment {
            clear_pending_payload(&mut seg_conf);
        }
        if activate_event {
            clear_pending_payload(&mut event_conf);
        }
        if activate_chapter {
            clear_pending_payload(&mut chapter_conf);
        }
        if activate_stage {
            clear_pending_payload(&mut stage_conf);
        }

        let confirmation_ids = vec![
            seg_conf.confirmation_id.clone(),
            event_conf.confirmation_id.clone(),
            chapter_conf.confirmation_id.clone(),
            stage_conf.confirmation_id.clone(),
        ];
        let confirmations = vec![seg_conf, event_conf, chapter_conf, stage_conf];

        let segments = if activate_segment {
            Some(draft.segments)
        } else {
            None
        };
        let events = if activate_event {
            Some(draft.events)
        } else {
            None
        };
        let realized = if activate_segment {
            draft.realized_changes
        } else {
            Vec::new()
        };
        let foreshadowing = if activate_segment {
            draft.foreshadowing_updates
        } else {
            Vec::new()
        };
        let chapter_summary_write = if activate_chapter {
            draft.chapter_summary
        } else {
            None
        };
        let stage_write = if activate_stage {
            Some((stage_id, stage_summary))
        } else {
            None
        };

        let issues = self.knowledge.apply_summary_pipeline_transaction(
            &chapter_id,
            segments,
            events,
            realized,
            foreshadowing,
            confirmations,
            chapter_summary_write,
            stage_write,
            &self.cancellation,
        )?;

        let planner_issue_ids = issues
            .iter()
            .map(|issue| issue.issue_id.clone())
            .collect::<Vec<_>>();

        // F14：completed_steps 仅包含已激活写入的步骤。
        let mut completed_steps = Vec::new();
        if activate_segment {
            completed_steps.push(SummaryPipelineStep::Segment);
        }
        if activate_event {
            completed_steps.push(SummaryPipelineStep::Event);
        }
        if activate_chapter {
            completed_steps.push(SummaryPipelineStep::Chapter);
        }
        if activate_stage {
            completed_steps.push(SummaryPipelineStep::Stage);
        }

        let pending_confirmations = has_pending_confirmations(self.knowledge)?;
        let has_unrealized_issues = !issues.is_empty();
        Ok(SummaryPipelineReport {
            chapter_id: draft.chapter_id,
            revision_id,
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

    /// 根据确认策略组装确认项（不落库；由单一事务统一写入）。
    fn build_confirmation(
        &self,
        kind: ConfirmationKind,
        chapter_id: &str,
        revision_id: &str,
        metadata: serde_json::Value,
    ) -> CoreResult<ConfirmationItem> {
        let mut state = self
            .confirmation_policy
            .initial_state(kind, &self.auto_mode);
        let mut metadata = merge_metadata(
            metadata,
            json!({ "revision_id": revision_id, "chapter_id": chapter_id }),
        );
        if state == ConfirmationState::AutoAudited {
            if let Some(decisions) = &self.auto_audit_decisions {
                let decision = decisions.get(&kind).ok_or_else(|| {
                    CoreError::validation(format!(
                        "missing Auto Mode audit decision for confirmation kind {kind:?}"
                    ))
                })?;
                if decision.reason.trim().is_empty() {
                    return Err(CoreError::validation(format!(
                        "empty Auto Mode audit reason for confirmation kind {kind:?}"
                    )));
                }
                if !decision.approved {
                    state = ConfirmationState::Pending;
                }
                metadata = merge_metadata(
                    metadata,
                    json!({
                        "auto_audit": {
                            "approved": decision.approved,
                            "reason": decision.reason,
                        }
                    }),
                );
            }
        }
        Ok(ConfirmationItem::new(
            confirmation_id(kind, chapter_id, revision_id),
            kind,
            state,
            metadata,
        ))
    }
}

/// 把待确认项状态推进为已通过，并物化 summary pending_payload（F14）。
pub fn approve_confirmation(
    knowledge: &MemoryWritingKnowledgeBase,
    confirmation_id: &str,
) -> CoreResult<ConfirmationItem> {
    let current = knowledge
        .confirmations(None)?
        .into_iter()
        .find(|c| c.confirmation_id == confirmation_id)
        .ok_or_else(|| {
            CoreError::validation(format!("confirmation item not found: {confirmation_id}"))
        })?;
    if is_summary_kind(current.kind)
        && current.metadata.get("pending_payload").is_some()
        && !confirmation_state_activates_knowledge(current.state)
    {
        knowledge.materialize_summary_confirmation_payload(&current)?;
    }
    let mut updated =
        knowledge.update_confirmation_state(confirmation_id, ConfirmationState::Approved)?;
    clear_pending_payload(&mut updated);
    knowledge.upsert_confirmation(updated.clone())?;
    Ok(updated)
}

/// 把待确认项状态推进为已拒绝；丢弃 pending_payload，不写入 active 知识（F14）。
pub fn reject_confirmation(
    knowledge: &MemoryWritingKnowledgeBase,
    confirmation_id: &str,
) -> CoreResult<ConfirmationItem> {
    let mut updated =
        knowledge.update_confirmation_state(confirmation_id, ConfirmationState::Rejected)?;
    clear_pending_payload(&mut updated);
    knowledge.upsert_confirmation(updated.clone())?;
    Ok(updated)
}

/// 判断当前是否仍有待人工确认项。
pub fn has_pending_confirmations(knowledge: &MemoryWritingKnowledgeBase) -> CoreResult<bool> {
    Ok(!knowledge
        .confirmations(Some(ConfirmationState::Pending))?
        .is_empty())
}

fn is_summary_kind(kind: ConfirmationKind) -> bool {
    matches!(
        kind,
        ConfirmationKind::SegmentSummary
            | ConfirmationKind::EventSummary
            | ConfirmationKind::ChapterSummary
            | ConfirmationKind::StageSummary
    )
}

fn clear_pending_payload(item: &mut ConfirmationItem) {
    if let Some(obj) = item.metadata.as_object_mut() {
        obj.remove("pending_payload");
    }
}

/// F25：校验阶段身份。
fn validate_stage_identity(
    knowledge: &MemoryWritingKnowledgeBase,
    draft: &SummaryPipelineDraft,
) -> CoreResult<()> {
    let stage_id = draft.stage_id.as_deref().unwrap_or("");
    let exists = knowledge.has_stage(stage_id)?;
    match draft.is_new_stage {
        Some(false) if !exists => Err(CoreError::validation(format!(
            "unknown stage_id '{stage_id}': set is_new_stage true to propose a new stage"
        ))),
        Some(true) | Some(false) | None => Ok(()),
    }
}

fn resolve_is_new_stage(
    knowledge: &MemoryWritingKnowledgeBase,
    stage_id: &str,
    flag: Option<bool>,
) -> CoreResult<bool> {
    match flag {
        Some(v) => Ok(v),
        None => Ok(!knowledge.has_stage(stage_id)?),
    }
}

/// 生成稳定确认项 id。
fn confirmation_id(kind: ConfirmationKind, chapter_id: &str, revision_id: &str) -> String {
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
    format!("{chapter_id}::{revision_id}::{name}")
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
    let mut segment_ids = BTreeSet::new();
    for segment in &draft.segments {
        if segment.chapter_id != draft.chapter_id {
            return Err(CoreError::validation(
                "story segment chapter_id must match pipeline chapter_id",
            ));
        }
        if !segment_ids.insert(segment.segment_id.as_str()) {
            return Err(CoreError::validation(
                "summary pipeline contains duplicate story segment id",
            ));
        }
    }
    if draft.events.is_empty() {
        return Err(CoreError::validation(
            "summary pipeline requires at least one changed event",
        ));
    }
    for event in &draft.events {
        event.validate()?;
        if !event.chapter_ids.iter().any(|id| id == &draft.chapter_id) {
            return Err(CoreError::validation(
                "changed event must link back to pipeline chapter_id",
            ));
        }
        for segment_id in &event.segment_ids {
            if !segment_ids.contains(segment_id.as_str()) {
                return Err(CoreError::validation(format!(
                    "changed event references missing story segment: {segment_id}"
                )));
            }
        }
    }
    validate_required_text("chapter_summary", draft.chapter_summary.as_deref())?;
    validate_required_text("stage_id", draft.stage_id.as_deref())?;
    validate_required_text("stage_summary", draft.stage_summary.as_deref())?;
    Ok(())
}

/// F22：revision 仅由流水线可信层生成；**忽略** draft.metadata 中调用方提供的 revision_id，
/// 防止伪造/重复 revision 覆盖既有 confirmation 历史。
fn summary_revision_id(_draft: &SummaryPipelineDraft) -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("summary-{nanos:x}-{sequence:x}")
}

fn attach_revision_metadata(draft: &mut SummaryPipelineDraft, revision_id: &str) {
    for segment in &mut draft.segments {
        segment.metadata = merge_metadata(
            std::mem::take(&mut segment.metadata),
            json!({ "revision_id": revision_id, "active_revision": true }),
        );
    }
    for event in &mut draft.events {
        event.metadata = merge_metadata(
            std::mem::take(&mut event.metadata),
            json!({ "revision_id": revision_id, "active_revision": true }),
        );
    }
}

fn merge_metadata(base: Value, extra: Value) -> Value {
    let mut object = base.as_object().cloned().unwrap_or_default();
    if let Some(extra) = extra.as_object() {
        object.extend(extra.clone());
    }
    Value::Object(object)
}

fn validate_required_text(field: &str, value: Option<&str>) -> CoreResult<()> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(()),
        _ => Err(CoreError::validation(format!(
            "summary pipeline requires {field}"
        ))),
    }
}
