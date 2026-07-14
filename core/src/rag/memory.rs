use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult};
use crate::rag::models::{
    BidirectionalIndex, CharacterPlanContent, CharacterProfileContent, CharacterTraitContent,
    ConfirmationItem, ConfirmationState, FindRequest, FindResponse, FindResult, FindScope,
    ForeshadowingRecord, ForeshadowingStatus, ForeshadowingUpdate, PlannerIssue, RegisterContent,
    RegisterFunction, RegisterOperation, RegisteredChange, RegisteredChangeStatus,
    RelationshipContent, StoryEvent, StorySegment, ThemeAnchorContent,
};

/// 内存版创作知识库，Module 9 先固定总结机制的数据契约和索引行为。
#[derive(Debug, Default)]
pub struct MemoryWritingKnowledgeBase {
    state: Mutex<WritingKnowledgeState>,
}

#[derive(Debug, Clone, Default)]
struct WritingKnowledgeState {
    segments: BTreeMap<String, StorySegment>,
    events: BTreeMap<String, StoryEvent>,
    changes: BTreeMap<String, RegisteredChange>,
    foreshadowing: BTreeMap<String, ForeshadowingRecord>,
    issues: BTreeMap<String, PlannerIssue>,
    confirmations: BTreeMap<String, ConfirmationItem>,
    chapter_summaries: BTreeMap<String, String>,
    stage_summaries: BTreeMap<String, String>,
    next_change_sequence: u64,
    index: BidirectionalIndex,
}

impl MemoryWritingKnowledgeBase {
    /// 创建空的内存创作知识库。
    pub fn new() -> Self {
        Self::default()
    }

    /// 写入或更新故事段，并重建故事段到章节的双向索引。
    pub fn upsert_segment(&self, segment: StorySegment) -> CoreResult<()> {
        segment.validate()?;
        let mut state = self.lock_state()?;
        state.insert_segment(segment);
        Ok(())
    }

    /// 删除故事段源记录，并清理章节、事件、注册项和伏笔的双向索引引用。
    pub fn delete_segment(&self, segment_id: &str) -> CoreResult<Option<StorySegment>> {
        validate_non_empty_local("segment_id", segment_id)?;
        let mut state = self.lock_state()?;
        let removed = state.segments.remove(segment_id);
        if removed.is_some() {
            state.remove_segment_links(segment_id);
        }
        Ok(removed)
    }

    /// 写入或更新事件，并同步事件与故事段、章节的双向索引。
    pub fn upsert_event(&self, event: StoryEvent) -> CoreResult<()> {
        event.validate()?;
        let mut state = self.lock_state()?;
        validate_event_references(&state, &event)?;
        state.insert_event(event);
        Ok(())
    }

    /// 原子替换某章节当前 revision 的故事段和事件事实。
    pub fn replace_chapter_summary_entities(
        &self,
        chapter_id: &str,
        segments: Vec<StorySegment>,
        events: Vec<StoryEvent>,
    ) -> CoreResult<()> {
        let mut state = self.lock_state()?;
        replace_chapter_summary_entities_on_state(&mut state, chapter_id, segments, events)
    }

    /// F21/C2/F14：章节总结 draft 的单一内存事务。
    /// `segments`/`events`/`chapter_summary`/`stage` 为 `None` 时跳过该步写入（确认前不激活）。
    /// 先在克隆态上完整应用，成功后再提交到锁内状态；任一步失败不修改库。
    #[allow(clippy::too_many_arguments)]
    pub fn apply_summary_pipeline_transaction(
        &self,
        chapter_id: &str,
        segments: Option<Vec<StorySegment>>,
        events: Option<Vec<StoryEvent>>,
        realized_changes: Vec<crate::rag::models::RealizedChangeLink>,
        foreshadowing_updates: Vec<ForeshadowingUpdate>,
        confirmations: Vec<ConfirmationItem>,
        chapter_summary: Option<String>,
        stage: Option<(String, String)>,
        cancellation: &crate::contracts::CancellationToken,
    ) -> CoreResult<Vec<PlannerIssue>> {
        validate_non_empty_local("chapter_id", chapter_id)?;
        if let Some(ref summary) = chapter_summary {
            validate_non_empty_local("chapter_summary", summary)?;
        }
        if let Some((ref stage_id, ref stage_summary)) = stage {
            validate_non_empty_local("stage_id", stage_id)?;
            validate_non_empty_local("stage_summary", stage_summary)?;
        }
        cancellation.check()?;

        let mut guard = self.lock_state()?;
        let mut working = (*guard).clone();
        let wrote_entities = segments.is_some() || events.is_some();
        let wrote_chapter = chapter_summary.is_some();
        if wrote_entities {
            let segs = segments.unwrap_or_else(|| {
                working
                    .segments
                    .values()
                    .filter(|s| s.chapter_id == chapter_id)
                    .cloned()
                    .collect()
            });
            let evs = events.unwrap_or_else(|| {
                working
                    .events
                    .values()
                    .filter(|e| e.chapter_ids.iter().any(|id| id == chapter_id))
                    .cloned()
                    .collect()
            });
            replace_chapter_summary_entities_on_state(&mut working, chapter_id, segs, evs)?;
        }
        cancellation.check()?;
        for link in realized_changes {
            mark_change_realized_on_state(&mut working, &link.change_id, &link.segment_id)?;
        }
        for update in foreshadowing_updates {
            apply_foreshadowing_update_on_state(&mut working, update)?;
        }
        for item in confirmations {
            validate_non_empty_local("confirmation_id", &item.confirmation_id)?;
            validate_non_empty_local("prompt_key", &item.prompt_key)?;
            working
                .confirmations
                .insert(item.confirmation_id.clone(), item);
        }
        if let Some(chapter_summary) = chapter_summary {
            working
                .chapter_summaries
                .insert(chapter_id.to_owned(), chapter_summary);
        }
        if let Some((stage_id, stage_summary)) = stage {
            if let Some(previous_stage_id) = working.index.chapter_stage.remove(chapter_id) {
                unlink_value(
                    &mut working.index.stage_chapters,
                    &previous_stage_id,
                    chapter_id,
                );
            }
            link_unique(
                &mut working.index.stage_chapters,
                &stage_id,
                chapter_id.to_owned(),
            );
            working
                .index
                .chapter_stage
                .insert(chapter_id.to_owned(), stage_id.clone());
            working.stage_summaries.insert(stage_id, stage_summary);
        }

        let issues = if wrote_entities || wrote_chapter {
            queue_unrealized_changes_on_state(&mut working, chapter_id)?
        } else {
            Vec::new()
        };
        cancellation.check()?;
        *guard = working;
        Ok(issues)
    }

    /// F14：将确认项中的 pending_payload 物化为 active 知识（单步）。
    pub fn materialize_summary_confirmation_payload(
        &self,
        item: &ConfirmationItem,
    ) -> CoreResult<()> {
        use crate::rag::models::ConfirmationKind;
        let payload = item
            .metadata
            .get("pending_payload")
            .cloned()
            .ok_or_else(|| {
                CoreError::validation(format!(
                    "summary confirmation missing pending_payload: {}",
                    item.confirmation_id
                ))
            })?;
        let chapter_id = item
            .metadata
            .get("chapter_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::validation("confirmation missing chapter_id"))?
            .to_owned();
        let cancellation = crate::contracts::CancellationToken::new();
        match item.kind {
            ConfirmationKind::SegmentSummary => {
                let segments: Vec<StorySegment> = serde_json::from_value(
                    payload
                        .get("segments")
                        .cloned()
                        .unwrap_or(Value::Array(vec![])),
                )
                .map_err(|e| CoreError::validation(format!("pending segments: {e}")))?;
                let realized: Vec<crate::rag::models::RealizedChangeLink> = serde_json::from_value(
                    payload
                        .get("realized_changes")
                        .cloned()
                        .unwrap_or(Value::Array(vec![])),
                )
                .unwrap_or_default();
                let foreshadowing: Vec<ForeshadowingUpdate> = serde_json::from_value(
                    payload
                        .get("foreshadowing_updates")
                        .cloned()
                        .unwrap_or(Value::Array(vec![])),
                )
                .unwrap_or_default();
                self.apply_summary_pipeline_transaction(
                    &chapter_id,
                    Some(segments),
                    None,
                    realized,
                    foreshadowing,
                    Vec::new(),
                    None,
                    None,
                    &cancellation,
                )?;
            }
            ConfirmationKind::EventSummary => {
                let events: Vec<StoryEvent> = serde_json::from_value(
                    payload
                        .get("events")
                        .cloned()
                        .unwrap_or(Value::Array(vec![])),
                )
                .map_err(|e| CoreError::validation(format!("pending events: {e}")))?;
                self.apply_summary_pipeline_transaction(
                    &chapter_id,
                    None,
                    Some(events),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    &cancellation,
                )?;
            }
            ConfirmationKind::ChapterSummary => {
                let summary = payload
                    .get("chapter_summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                self.apply_summary_pipeline_transaction(
                    &chapter_id,
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Some(summary),
                    None,
                    &cancellation,
                )?;
            }
            ConfirmationKind::StageSummary => {
                let stage_id = payload
                    .get("stage_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let stage_summary = payload
                    .get("stage_summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                self.apply_summary_pipeline_transaction(
                    &chapter_id,
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    Some((stage_id, stage_summary)),
                    &cancellation,
                )?;
            }
            _ => {
                return Err(CoreError::validation(
                    "materialize_summary_confirmation_payload only supports summary kinds",
                ));
            }
        }
        Ok(())
    }

    /// F25：stage_id 是否已作为确认过的阶段存在。
    pub fn has_stage(&self, stage_id: &str) -> CoreResult<bool> {
        Ok(self.lock_state()?.stage_summaries.contains_key(stage_id))
    }

    /// 写入或更新注册项，注册项默认表示 Planner 计划变化。
    pub fn upsert_registered_change(&self, change: RegisteredChange) -> CoreResult<()> {
        change.validate()?;
        let mut state = self.lock_state()?;
        state.remove_change_links(&change.change_id);
        for segment_id in &change.linked_segment_ids {
            link_unique(
                &mut state.index.change_segments,
                &change.change_id,
                segment_id.clone(),
            );
            link_unique(
                &mut state.index.segment_changes,
                segment_id,
                change.change_id.clone(),
            );
        }
        state.link_structured_register(&change.change_id, &change.content);
        state.changes.insert(change.change_id.clone(), change);
        Ok(())
    }

    /// 写入或更新伏笔记录，并维护伏笔与故事段的双向索引。
    pub fn upsert_foreshadowing(&self, record: ForeshadowingRecord) -> CoreResult<()> {
        validate_non_empty_local("foreshadowing_id", &record.foreshadowing_id)?;
        validate_non_empty_local("title", &record.title)?;
        validate_non_empty_local("description", &record.description)?;

        let mut state = self.lock_state()?;
        state.remove_foreshadowing_links(&record.foreshadowing_id);
        for segment_id in record
            .planted_segment_ids
            .iter()
            .chain(record.recovered_segment_ids.iter())
        {
            link_unique(
                &mut state.index.foreshadowing_segments,
                &record.foreshadowing_id,
                segment_id.clone(),
            );
            link_unique(
                &mut state.index.segment_foreshadowing,
                segment_id,
                record.foreshadowing_id.clone(),
            );
        }
        state
            .foreshadowing
            .insert(record.foreshadowing_id.clone(), record);
        Ok(())
    }

    /// Planner register 的统一入口，保留机制文档中的 a/b/c 三段式参数。
    pub fn apply_register_operation(
        &self,
        function: RegisterFunction,
        operation: RegisterOperation,
        content: Option<RegisterContent>,
        change_id: Option<String>,
    ) -> CoreResult<Vec<RegisteredChange>> {
        match operation {
            RegisterOperation::List => self.list_registered_changes(function),
            RegisterOperation::New => {
                let content = content.ok_or_else(|| {
                    CoreError::validation("planner-register new requires c content")
                })?;
                let change = self.create_registered_change(function, content, change_id)?;
                Ok(vec![change])
            }
            RegisterOperation::Update => {
                if function == RegisterFunction::Foreshadowing {
                    return Err(CoreError::validation(
                        "planner-register update is only allowed for character traits and relationships",
                    ));
                }
                let content = content.ok_or_else(|| {
                    CoreError::validation("planner-register update requires c content")
                })?;
                let change = self.create_registered_change(function, content, change_id)?;
                Ok(vec![change])
            }
            RegisterOperation::Delete => {
                let change_id = change_id.ok_or_else(|| {
                    CoreError::validation("planner-register delete requires change_id")
                })?;
                let mut state = self.lock_state()?;
                let change = state.changes.get_mut(&change_id).ok_or_else(|| {
                    CoreError::validation(format!("registered change not found: {change_id}"))
                })?;
                if change.function != function {
                    return Err(CoreError::validation(
                        "registered change function does not match delete request",
                    ));
                }
                change.status = RegisteredChangeStatus::Deleted;
                Ok(vec![change.clone()])
            }
        }
    }

    /// 标记 Planner 注册项已经在正文故事段中落地。
    pub fn mark_change_realized(&self, change_id: &str, segment_id: &str) -> CoreResult<()> {
        validate_non_empty_local("change_id", change_id)?;
        validate_non_empty_local("segment_id", segment_id)?;
        let mut state = self.lock_state()?;
        if !state.segments.contains_key(segment_id) {
            return Err(CoreError::validation(format!(
                "story segment not found: {segment_id}"
            )));
        }
        {
            let change = state.changes.get_mut(change_id).ok_or_else(|| {
                CoreError::validation(format!("registered change not found: {change_id}"))
            })?;
            change.status = RegisteredChangeStatus::Realized;
            push_unique(&mut change.linked_segment_ids, segment_id.to_owned());
        }
        link_unique(
            &mut state.index.change_segments,
            change_id,
            segment_id.to_owned(),
        );
        link_unique(
            &mut state.index.segment_changes,
            segment_id,
            change_id.to_owned(),
        );
        Ok(())
    }

    /// 标记伏笔已经在故事段中种植、回收或废弃，并维护双向索引。
    pub fn apply_foreshadowing_update(&self, update: ForeshadowingUpdate) -> CoreResult<()> {
        validate_non_empty_local("foreshadowing_id", &update.foreshadowing_id)?;
        validate_non_empty_local("segment_id", &update.segment_id)?;
        let mut state = self.lock_state()?;
        if !state.segments.contains_key(&update.segment_id) {
            return Err(CoreError::validation(format!(
                "story segment not found: {}",
                update.segment_id
            )));
        }
        let record = state
            .foreshadowing
            .get_mut(&update.foreshadowing_id)
            .ok_or_else(|| {
                CoreError::validation(format!(
                    "foreshadowing not found: {}",
                    update.foreshadowing_id
                ))
            })?;
        if !matches!(
            update.status,
            ForeshadowingStatus::Planted | ForeshadowingStatus::Recovered
        ) {
            return Err(CoreError::validation(
                "summarizer foreshadowing update only supports planted or recovered",
            ));
        }
        record.status = update.status;
        match update.status {
            ForeshadowingStatus::Planted => {
                push_unique(&mut record.planted_segment_ids, update.segment_id.clone());
            }
            ForeshadowingStatus::Recovered => {
                push_unique(&mut record.recovered_segment_ids, update.segment_id.clone());
            }
            ForeshadowingStatus::Planned | ForeshadowingStatus::Abandoned => unreachable!(),
        }
        link_unique(
            &mut state.index.foreshadowing_segments,
            &update.foreshadowing_id,
            update.segment_id.clone(),
        );
        link_unique(
            &mut state.index.segment_foreshadowing,
            &update.segment_id,
            update.foreshadowing_id,
        );
        Ok(())
    }

    /// 将章节归入阶段，表达“阶段概括使章节成为某阶段子项”。
    pub fn link_chapter_stage(&self, chapter_id: &str, stage_id: &str) -> CoreResult<()> {
        validate_non_empty_local("chapter_id", chapter_id)?;
        validate_non_empty_local("stage_id", stage_id)?;
        let mut state = self.lock_state()?;
        if let Some(previous_stage_id) = state.index.chapter_stage.remove(chapter_id) {
            unlink_value(
                &mut state.index.stage_chapters,
                &previous_stage_id,
                chapter_id,
            );
        }
        link_unique(
            &mut state.index.stage_chapters,
            stage_id,
            chapter_id.to_owned(),
        );
        state
            .index
            .chapter_stage
            .insert(chapter_id.to_owned(), stage_id.to_owned());
        Ok(())
    }

    /// 写入 Summarizer 发现的 Planner 未落地问题。
    pub fn add_planner_issue(&self, issue: PlannerIssue) -> CoreResult<()> {
        validate_non_empty_local("issue_id", &issue.issue_id)?;
        validate_non_empty_local("change_id", &issue.change_id)?;
        validate_non_empty_local("chapter_id", &issue.chapter_id)?;
        validate_non_empty_local("reason", &issue.reason)?;

        let mut state = self.lock_state()?;
        state.issues.insert(issue.issue_id.clone(), issue);
        Ok(())
    }

    /// 返回指定章节的问题队列；空章节 id 表示返回全部问题。
    pub fn planner_issues(&self, chapter_id: &str) -> CoreResult<Vec<PlannerIssue>> {
        let state = self.lock_state()?;
        Ok(state
            .issues
            .values()
            .filter(|issue| chapter_id.is_empty() || issue.chapter_id == chapter_id)
            .cloned()
            .collect())
    }

    /// 根据当前注册项生成未落地问题，避免 Summarizer 静默忽略计划变化。
    pub fn queue_unrealized_changes_for_chapter(
        &self,
        chapter_id: &str,
    ) -> CoreResult<Vec<PlannerIssue>> {
        validate_non_empty_local("chapter_id", chapter_id)?;
        let mut state = self.lock_state()?;
        let changes: Vec<RegisteredChange> = state
            .changes
            .values()
            .filter(|change| {
                change.status == RegisteredChangeStatus::Planned
                    && change.applies_to_chapter(chapter_id)
            })
            .cloned()
            .collect();
        let mut issues = Vec::new();
        for change in changes {
            let issue_id = format!("{chapter_id}::{}", change.change_id);
            if let Some(existing) = state.issues.get(&issue_id).cloned() {
                issues.push(existing);
                continue;
            }
            let issue = PlannerIssue {
                issue_id: issue_id.clone(),
                change_id: change.change_id,
                chapter_id: chapter_id.to_owned(),
                reason: "registered change was not matched to any realized story segment"
                    .to_owned(),
                related_sources: Vec::new(),
                planner_explanation: None,
                correction_patch: None,
            };
            state.issues.insert(issue_id, issue.clone());
            issues.push(issue);
        }
        Ok(issues)
    }

    /// 写入确认项。
    pub fn upsert_confirmation(&self, item: ConfirmationItem) -> CoreResult<()> {
        validate_non_empty_local("confirmation_id", &item.confirmation_id)?;
        validate_non_empty_local("prompt_key", &item.prompt_key)?;
        self.lock_state()?
            .confirmations
            .insert(item.confirmation_id.clone(), item);
        Ok(())
    }

    /// 更新确认项状态。
    pub fn update_confirmation_state(
        &self,
        confirmation_id: &str,
        state: ConfirmationState,
    ) -> CoreResult<ConfirmationItem> {
        let mut store = self.lock_state()?;
        let item = store
            .confirmations
            .get_mut(confirmation_id)
            .ok_or_else(|| {
                CoreError::validation(format!("confirmation item not found: {confirmation_id}"))
            })?;
        item.state = state;
        Ok(item.clone())
    }

    /// 返回指定状态的确认项；None 表示返回全部。
    pub fn confirmations(
        &self,
        state_filter: Option<ConfirmationState>,
    ) -> CoreResult<Vec<ConfirmationItem>> {
        let state = self.lock_state()?;
        Ok(state
            .confirmations
            .values()
            .filter(|item| {
                state_filter
                    .map(|state| item.state == state)
                    .unwrap_or(true)
            })
            .cloned()
            .collect())
    }

    /// 写入章节总结文本。
    pub fn upsert_chapter_summary(
        &self,
        chapter_id: impl Into<String>,
        summary: impl Into<String>,
    ) -> CoreResult<()> {
        let chapter_id = chapter_id.into();
        let summary = summary.into();
        validate_non_empty_local("chapter_id", &chapter_id)?;
        validate_non_empty_local("summary", &summary)?;

        self.lock_state()?
            .chapter_summaries
            .insert(chapter_id, summary);
        Ok(())
    }

    /// 读取章节总结。
    pub fn chapter_summary(&self, chapter_id: &str) -> CoreResult<Option<String>> {
        Ok(self
            .lock_state()?
            .chapter_summaries
            .get(chapter_id)
            .cloned())
    }

    /// 写入阶段总结文本。
    pub fn upsert_stage_summary(
        &self,
        stage_id: impl Into<String>,
        summary: impl Into<String>,
    ) -> CoreResult<()> {
        let stage_id = stage_id.into();
        let summary = summary.into();
        validate_non_empty_local("stage_id", &stage_id)?;
        validate_non_empty_local("summary", &summary)?;

        self.lock_state()?.stage_summaries.insert(stage_id, summary);
        Ok(())
    }

    /// 读取阶段总结。
    pub fn stage_summary(&self, stage_id: &str) -> CoreResult<Option<String>> {
        Ok(self.lock_state()?.stage_summaries.get(stage_id).cloned())
    }

    /// 返回全部章节总结，供 Planner 按当前阶段策略组装前文上下文。
    pub fn chapter_summaries(&self) -> CoreResult<BTreeMap<String, String>> {
        Ok(self.lock_state()?.chapter_summaries.clone())
    }

    /// 返回全部阶段总结。
    pub fn stage_summaries(&self) -> CoreResult<BTreeMap<String, String>> {
        Ok(self.lock_state()?.stage_summaries.clone())
    }

    /// 返回全部注册项。
    pub fn registered_changes(&self) -> CoreResult<Vec<RegisteredChange>> {
        Ok(self.lock_state()?.changes.values().cloned().collect())
    }

    /// 查询故事段，默认只返回摘要和来源，不复制正文。
    pub fn segment(&self, segment_id: &str) -> CoreResult<Option<StorySegment>> {
        Ok(self.lock_state()?.segments.get(segment_id).cloned())
    }

    /// 查询事件。
    pub fn event(&self, event_id: &str) -> CoreResult<Option<StoryEvent>> {
        Ok(self.lock_state()?.events.get(event_id).cloned())
    }

    /// 查询注册项。
    pub fn registered_change(&self, change_id: &str) -> CoreResult<Option<RegisteredChange>> {
        Ok(self.lock_state()?.changes.get(change_id).cloned())
    }

    /// 查询伏笔。
    pub fn foreshadowing(&self, foreshadowing_id: &str) -> CoreResult<Option<ForeshadowingRecord>> {
        Ok(self
            .lock_state()?
            .foreshadowing
            .get(foreshadowing_id)
            .cloned())
    }

    /// 返回未回收伏笔，Planner 上下文会默认使用这一视图。
    pub fn unresolved_foreshadowing(&self) -> CoreResult<Vec<ForeshadowingRecord>> {
        let state = self.lock_state()?;
        Ok(state
            .foreshadowing
            .values()
            .filter(|record| record.status != ForeshadowingStatus::Recovered)
            .cloned()
            .collect())
    }

    /// 返回全部故事段（含任意章节），供持久化层完整落库。
    pub fn all_segments(&self) -> CoreResult<Vec<StorySegment>> {
        Ok(self.lock_state()?.segments.values().cloned().collect())
    }

    /// 返回全部事件，供持久化层完整落库。
    pub fn all_events(&self) -> CoreResult<Vec<StoryEvent>> {
        Ok(self.lock_state()?.events.values().cloned().collect())
    }

    /// 返回全部伏笔记录（含已回收和已废弃），供持久化层完整落库。
    pub fn all_foreshadowing(&self) -> CoreResult<Vec<ForeshadowingRecord>> {
        Ok(self.lock_state()?.foreshadowing.values().cloned().collect())
    }

    /// 返回当前双向索引快照，便于测试和后续持久化重建。
    pub fn index_snapshot(&self) -> CoreResult<BidirectionalIndex> {
        Ok(self.lock_state()?.index.clone())
    }

    /// 执行 find 查询，默认返回轻量摘要、来源、评分和元数据。
    pub fn find(&self, request: FindRequest) -> CoreResult<FindResponse> {
        let state = self.lock_state()?;
        let mut results = match request.scope {
            FindScope::CharacterProfile => find_character_profiles(&state, &request.query),
            FindScope::CharacterPlan => find_character_plans(&state, &request.query),
            FindScope::CharacterTraitPath => find_character_traits(&state, &request.query),
            FindScope::RelationshipPath => find_relationships(&state, &request.query),
            FindScope::EventSegments => find_event_segments(&state, &request.query),
            FindScope::SegmentText => find_segments(&state, &request.query, request.include_text),
            FindScope::Foreshadowing => find_foreshadowing(&state, &request.query),
            FindScope::ThemeAnchor => find_theme_anchors(&state, &request.query),
            FindScope::ChapterSummary => {
                find_summaries(&state.chapter_summaries, "chapter_summary", &request.query)
            }
            FindScope::StageSummary => {
                find_summaries(&state.stage_summaries, "stage_summary", &request.query)
            }
        };

        results.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.result_id.cmp(&right.result_id))
        });
        Ok(FindResponse { results })
    }

    fn create_registered_change(
        &self,
        function: RegisterFunction,
        content: RegisterContent,
        change_id: Option<String>,
    ) -> CoreResult<RegisteredChange> {
        let mut state = self.lock_state()?;
        if !matches_register_content(function, &content) {
            return Err(CoreError::validation(
                "register content kind does not match function",
            ));
        }
        content.validate()?;

        let change_id = match change_id {
            Some(value) => value,
            None => next_change_id(&mut state, function),
        };
        validate_non_empty_local("change_id", &change_id)?;
        if state.changes.contains_key(&change_id) {
            return Err(CoreError::validation(format!(
                "registered change already exists: {change_id}"
            )));
        }

        let change = RegisteredChange {
            change_id: change_id.clone(),
            function,
            status: RegisteredChangeStatus::Planned,
            content,
            linked_segment_ids: Vec::new(),
            metadata: Value::Null,
        };
        state.link_structured_register(&change_id, &change.content);
        state.changes.insert(change_id, change.clone());
        Ok(change)
    }

    fn list_registered_changes(
        &self,
        function: RegisterFunction,
    ) -> CoreResult<Vec<RegisteredChange>> {
        let state = self.lock_state()?;
        Ok(state
            .changes
            .values()
            .filter(|change| change.function == function)
            .cloned()
            .collect())
    }

    fn lock_state(&self) -> CoreResult<std::sync::MutexGuard<'_, WritingKnowledgeState>> {
        self.state
            .lock()
            .map_err(|_| CoreError::validation("writing knowledge base lock poisoned"))
    }
}

impl WritingKnowledgeState {
    fn insert_segment(&mut self, segment: StorySegment) {
        self.remove_segment_chapter_link(&segment.segment_id);
        link_unique(
            &mut self.index.chapter_segments,
            &segment.chapter_id,
            segment.segment_id.clone(),
        );
        self.index
            .segment_chapter
            .insert(segment.segment_id.clone(), segment.chapter_id.clone());
        self.segments.insert(segment.segment_id.clone(), segment);
    }

    fn insert_event(&mut self, event: StoryEvent) {
        self.remove_event_links(&event.event_id);
        for segment_id in &event.segment_ids {
            link_unique(
                &mut self.index.event_segments,
                &event.event_id,
                segment_id.clone(),
            );
            link_unique(
                &mut self.index.segment_events,
                segment_id,
                event.event_id.clone(),
            );
        }
        for chapter_id in &event.chapter_ids {
            link_unique(
                &mut self.index.event_chapters,
                &event.event_id,
                chapter_id.clone(),
            );
            link_unique(
                &mut self.index.chapter_events,
                chapter_id,
                event.event_id.clone(),
            );
        }
        self.events.insert(event.event_id.clone(), event);
    }

    /// 仅移除故事段与章节的索引；普通 upsert 不应破坏事件/注册项等长期链接。
    fn remove_segment_chapter_link(&mut self, segment_id: &str) {
        if let Some(chapter_id) = self.index.segment_chapter.remove(segment_id) {
            unlink_value(&mut self.index.chapter_segments, &chapter_id, segment_id);
        }
    }

    /// 完整移除故事段相关索引，供删除故事段时避免留下孤儿引用。
    fn remove_segment_links(&mut self, segment_id: &str) {
        self.remove_segment_chapter_link(segment_id);
        if let Some(event_ids) = self.index.segment_events.remove(segment_id) {
            for event_id in event_ids {
                unlink_value(&mut self.index.event_segments, &event_id, segment_id);
                if let Some(event) = self.events.get_mut(&event_id) {
                    event.segment_ids.retain(|id| id != segment_id);
                }
            }
        }
        if let Some(change_ids) = self.index.segment_changes.remove(segment_id) {
            for change_id in change_ids {
                unlink_value(&mut self.index.change_segments, &change_id, segment_id);
                if let Some(change) = self.changes.get_mut(&change_id) {
                    change.linked_segment_ids.retain(|id| id != segment_id);
                }
            }
        }
        if let Some(foreshadowing_ids) = self.index.segment_foreshadowing.remove(segment_id) {
            for foreshadowing_id in foreshadowing_ids {
                unlink_value(
                    &mut self.index.foreshadowing_segments,
                    &foreshadowing_id,
                    segment_id,
                );
                if let Some(record) = self.foreshadowing.get_mut(&foreshadowing_id) {
                    record.planted_segment_ids.retain(|id| id != segment_id);
                    record.recovered_segment_ids.retain(|id| id != segment_id);
                }
            }
        }
    }

    /// 移除事件相关索引，随后由 upsert 重新建立。
    fn remove_event_links(&mut self, event_id: &str) {
        if let Some(segment_ids) = self.index.event_segments.remove(event_id) {
            for segment_id in segment_ids {
                unlink_value(&mut self.index.segment_events, &segment_id, event_id);
            }
        }
        if let Some(chapter_ids) = self.index.event_chapters.remove(event_id) {
            for chapter_id in chapter_ids {
                unlink_value(&mut self.index.chapter_events, &chapter_id, event_id);
            }
        }
    }

    /// 移除注册项相关索引，随后由 upsert 或 realized 标记重新建立。
    fn remove_change_links(&mut self, change_id: &str) {
        if let Some(change) = self.changes.get(change_id).cloned() {
            self.remove_structured_register_links(change_id, &change.content);
        }
        if let Some(segment_ids) = self.index.change_segments.remove(change_id) {
            for segment_id in segment_ids {
                unlink_value(&mut self.index.segment_changes, &segment_id, change_id);
            }
        }
    }

    /// 移除结构化 register 派生索引，随后由 upsert 重新建立。
    fn remove_structured_register_links(&mut self, change_id: &str, content: &RegisterContent) {
        match content {
            RegisterContent::CharacterPlan(plan) => {
                unlink_value(
                    &mut self.index.character_profile_plans,
                    &plan.character_id,
                    change_id,
                );
                self.index.character_plan_profile.remove(change_id);
                if let Some(stage_id) = &plan.stage_id {
                    unlink_value(&mut self.index.stage_character_plans, stage_id, change_id);
                    self.index.character_plan_stage.remove(change_id);
                }
                if let Some(chapter_id) = &plan.chapter_id {
                    unlink_value(
                        &mut self.index.chapter_character_plans,
                        chapter_id,
                        change_id,
                    );
                    self.index.character_plan_chapter.remove(change_id);
                }
            }
            RegisterContent::ThemeAnchor(anchor) => {
                for stage_id in &anchor.stage_ids {
                    unlink_value(
                        &mut self.index.theme_stage_links,
                        &anchor.anchor_id,
                        stage_id,
                    );
                    unlink_value(
                        &mut self.index.stage_theme_links,
                        stage_id,
                        &anchor.anchor_id,
                    );
                }
                for chapter_id in &anchor.chapter_ids {
                    unlink_value(
                        &mut self.index.theme_chapter_links,
                        &anchor.anchor_id,
                        chapter_id,
                    );
                    unlink_value(
                        &mut self.index.chapter_theme_links,
                        chapter_id,
                        &anchor.anchor_id,
                    );
                }
            }
            _ => {}
        }
    }

    /// 建立结构化 register 派生索引。
    fn link_structured_register(&mut self, change_id: &str, content: &RegisterContent) {
        match content {
            RegisterContent::CharacterPlan(plan) => {
                link_unique(
                    &mut self.index.character_profile_plans,
                    &plan.character_id,
                    change_id.to_owned(),
                );
                self.index
                    .character_plan_profile
                    .insert(change_id.to_owned(), plan.character_id.clone());
                if let Some(stage_id) = &plan.stage_id {
                    link_unique(
                        &mut self.index.stage_character_plans,
                        stage_id,
                        change_id.to_owned(),
                    );
                    self.index
                        .character_plan_stage
                        .insert(change_id.to_owned(), stage_id.clone());
                }
                if let Some(chapter_id) = &plan.chapter_id {
                    link_unique(
                        &mut self.index.chapter_character_plans,
                        chapter_id,
                        change_id.to_owned(),
                    );
                    self.index
                        .character_plan_chapter
                        .insert(change_id.to_owned(), chapter_id.clone());
                }
            }
            RegisterContent::ThemeAnchor(anchor) => {
                for stage_id in &anchor.stage_ids {
                    link_unique(
                        &mut self.index.theme_stage_links,
                        &anchor.anchor_id,
                        stage_id.clone(),
                    );
                    link_unique(
                        &mut self.index.stage_theme_links,
                        stage_id,
                        anchor.anchor_id.clone(),
                    );
                }
                for chapter_id in &anchor.chapter_ids {
                    link_unique(
                        &mut self.index.theme_chapter_links,
                        &anchor.anchor_id,
                        chapter_id.clone(),
                    );
                    link_unique(
                        &mut self.index.chapter_theme_links,
                        chapter_id,
                        anchor.anchor_id.clone(),
                    );
                }
            }
            _ => {}
        }
    }

    /// 移除伏笔相关索引，随后由 upsert 重新建立。
    fn remove_foreshadowing_links(&mut self, foreshadowing_id: &str) {
        if let Some(segment_ids) = self.index.foreshadowing_segments.remove(foreshadowing_id) {
            for segment_id in segment_ids {
                unlink_value(
                    &mut self.index.segment_foreshadowing,
                    &segment_id,
                    foreshadowing_id,
                );
            }
        }
    }
}

fn replace_chapter_summary_entities_on_state(
    state: &mut WritingKnowledgeState,
    chapter_id: &str,
    segments: Vec<StorySegment>,
    events: Vec<StoryEvent>,
) -> CoreResult<()> {
    validate_non_empty_local("chapter_id", chapter_id)?;
    let mut segment_ids = BTreeSet::new();
    for segment in &segments {
        segment.validate()?;
        if segment.chapter_id != chapter_id {
            return Err(CoreError::validation(
                "replacement story segment belongs to another chapter",
            ));
        }
        if !segment_ids.insert(segment.segment_id.clone()) {
            return Err(CoreError::validation(
                "replacement contains duplicate story segment id",
            ));
        }
    }
    let mut event_ids = BTreeSet::new();
    for event in &events {
        event.validate()?;
        if !event.chapter_ids.iter().any(|id| id == chapter_id) {
            return Err(CoreError::validation(
                "replacement story event does not reference chapter",
            ));
        }
        if !event_ids.insert(event.event_id.clone()) {
            return Err(CoreError::validation(
                "replacement contains duplicate story event id",
            ));
        }
        if let Some(missing) = event
            .segment_ids
            .iter()
            .find(|segment_id| !segment_ids.contains(*segment_id))
        {
            return Err(CoreError::validation(format!(
                "story event references missing replacement segment: {missing}"
            )));
        }
    }

    let old_segment_ids = state
        .index
        .chapter_segments
        .get(chapter_id)
        .cloned()
        .unwrap_or_default();
    let old_segment_set = old_segment_ids.iter().cloned().collect::<BTreeSet<_>>();
    let old_event_ids = state
        .index
        .chapter_events
        .get(chapter_id)
        .cloned()
        .unwrap_or_default();
    let mut preserved_events = BTreeMap::new();
    for event_id in old_event_ids {
        if let Some(mut event) = state.events.remove(&event_id) {
            state.remove_event_links(&event_id);
            event.chapter_ids.retain(|id| id != chapter_id);
            event.segment_ids.retain(|id| !old_segment_set.contains(id));
            if !event.chapter_ids.is_empty() && !event.segment_ids.is_empty() {
                preserved_events.insert(event_id, event);
            }
        }
    }
    for segment_id in old_segment_ids {
        state.segments.remove(&segment_id);
        state.remove_segment_links(&segment_id);
    }
    for segment in segments {
        state.insert_segment(segment);
    }
    for mut event in events {
        if let Some(existing) = state.events.remove(&event.event_id) {
            state.remove_event_links(&event.event_id);
            merge_unique(&mut event.segment_ids, existing.segment_ids);
            merge_unique(&mut event.chapter_ids, existing.chapter_ids);
        }
        if let Some(existing) = preserved_events.remove(&event.event_id) {
            merge_unique(&mut event.segment_ids, existing.segment_ids);
            merge_unique(&mut event.chapter_ids, existing.chapter_ids);
        }
        state.insert_event(event);
    }
    for (_, event) in preserved_events {
        state.insert_event(event);
    }
    Ok(())
}

fn mark_change_realized_on_state(
    state: &mut WritingKnowledgeState,
    change_id: &str,
    segment_id: &str,
) -> CoreResult<()> {
    validate_non_empty_local("change_id", change_id)?;
    validate_non_empty_local("segment_id", segment_id)?;
    if !state.segments.contains_key(segment_id) {
        return Err(CoreError::validation(format!(
            "story segment not found: {segment_id}"
        )));
    }
    {
        let change = state.changes.get_mut(change_id).ok_or_else(|| {
            CoreError::validation(format!("registered change not found: {change_id}"))
        })?;
        change.status = RegisteredChangeStatus::Realized;
        push_unique(&mut change.linked_segment_ids, segment_id.to_owned());
    }
    link_unique(
        &mut state.index.change_segments,
        change_id,
        segment_id.to_owned(),
    );
    link_unique(
        &mut state.index.segment_changes,
        segment_id,
        change_id.to_owned(),
    );
    Ok(())
}

fn apply_foreshadowing_update_on_state(
    state: &mut WritingKnowledgeState,
    update: ForeshadowingUpdate,
) -> CoreResult<()> {
    validate_non_empty_local("foreshadowing_id", &update.foreshadowing_id)?;
    validate_non_empty_local("segment_id", &update.segment_id)?;
    if !state.segments.contains_key(&update.segment_id) {
        return Err(CoreError::validation(format!(
            "story segment not found: {}",
            update.segment_id
        )));
    }
    let record = state
        .foreshadowing
        .get_mut(&update.foreshadowing_id)
        .ok_or_else(|| {
            CoreError::validation(format!(
                "foreshadowing not found: {}",
                update.foreshadowing_id
            ))
        })?;
    if !matches!(
        update.status,
        ForeshadowingStatus::Planted | ForeshadowingStatus::Recovered
    ) {
        return Err(CoreError::validation(
            "summarizer foreshadowing update only supports planted or recovered",
        ));
    }
    record.status = update.status;
    match update.status {
        ForeshadowingStatus::Planted => {
            push_unique(&mut record.planted_segment_ids, update.segment_id.clone());
        }
        ForeshadowingStatus::Recovered => {
            push_unique(&mut record.recovered_segment_ids, update.segment_id.clone());
        }
        ForeshadowingStatus::Planned | ForeshadowingStatus::Abandoned => unreachable!(),
    }
    link_unique(
        &mut state.index.foreshadowing_segments,
        &update.foreshadowing_id,
        update.segment_id.clone(),
    );
    link_unique(
        &mut state.index.segment_foreshadowing,
        &update.segment_id,
        update.foreshadowing_id,
    );
    Ok(())
}

fn queue_unrealized_changes_on_state(
    state: &mut WritingKnowledgeState,
    chapter_id: &str,
) -> CoreResult<Vec<PlannerIssue>> {
    let changes: Vec<RegisteredChange> = state
        .changes
        .values()
        .filter(|change| {
            change.status == RegisteredChangeStatus::Planned
                && change.applies_to_chapter(chapter_id)
        })
        .cloned()
        .collect();
    let mut issues = Vec::new();
    for change in changes {
        let issue_id = format!("{chapter_id}::{}", change.change_id);
        if let Some(existing) = state.issues.get(&issue_id).cloned() {
            issues.push(existing);
            continue;
        }
        let issue = PlannerIssue {
            issue_id: issue_id.clone(),
            change_id: change.change_id,
            chapter_id: chapter_id.to_owned(),
            reason: "registered change was not matched to any realized story segment".to_owned(),
            related_sources: Vec::new(),
            planner_explanation: None,
            correction_patch: None,
        };
        state.issues.insert(issue_id, issue.clone());
        issues.push(issue);
    }
    Ok(issues)
}

/// 判断 register 内容类型是否与功能一致。
fn matches_register_content(function: RegisterFunction, content: &RegisterContent) -> bool {
    matches!(
        (function, content),
        (
            RegisterFunction::CharacterProfile,
            RegisterContent::CharacterProfile(_)
        ) | (
            RegisterFunction::CharacterPlan,
            RegisterContent::CharacterPlan(_)
        ) | (
            RegisterFunction::ThemeAnchor,
            RegisterContent::ThemeAnchor(_)
        ) | (
            RegisterFunction::CharacterTrait,
            RegisterContent::CharacterTrait(_)
        ) | (
            RegisterFunction::Relationship,
            RegisterContent::Relationship(_)
        ) | (
            RegisterFunction::Foreshadowing,
            RegisterContent::Foreshadowing(_)
        )
    )
}

/// 生成稳定且单调递增的注册项 id，避免显式 id 或失败重试造成 `len()+1` 撞号。
fn next_change_id(state: &mut WritingKnowledgeState, function: RegisterFunction) -> String {
    let prefix = match function {
        RegisterFunction::CharacterProfile => "character-profile",
        RegisterFunction::CharacterPlan => "character-plan",
        RegisterFunction::CharacterTrait => "character-trait",
        RegisterFunction::Relationship => "relationship",
        RegisterFunction::Foreshadowing => "foreshadowing",
        RegisterFunction::ThemeAnchor => "theme-anchor",
    };
    loop {
        state.next_change_sequence = state.next_change_sequence.saturating_add(1);
        let candidate = format!("register-{prefix}-{}", state.next_change_sequence);
        if !state.changes.contains_key(&candidate) {
            return candidate;
        }
    }
}

/// 人物实体查询。
fn find_character_profiles(state: &WritingKnowledgeState, query: &str) -> Vec<FindResult> {
    state
        .changes
        .values()
        .filter_map(|change| match &change.content {
            RegisterContent::CharacterProfile(content)
                if query_matches(
                    query,
                    &[
                        &content.character_id,
                        &content.name,
                        &content.narrative_role,
                    ],
                ) || content
                    .aliases
                    .iter()
                    .any(|alias| query_matches(query, &[alias])) =>
            {
                Some(change_result(
                    change,
                    &format!("{} ({})", content.name, content.character_id),
                    character_profile_snippet(content),
                ))
            }
            _ => None,
        })
        .collect()
}

/// 人物出场计划查询。
fn find_character_plans(state: &WritingKnowledgeState, query: &str) -> Vec<FindResult> {
    state
        .changes
        .values()
        .filter_map(|change| match &change.content {
            RegisterContent::CharacterPlan(content)
                if query_matches(
                    query,
                    &[
                        &content.plan_id,
                        &content.character_id,
                        content.stage_id.as_deref().unwrap_or_default(),
                        content.chapter_id.as_deref().unwrap_or_default(),
                        &content.narrative_function,
                    ],
                ) =>
            {
                Some(change_result(
                    change,
                    &format!("{}: {}", content.character_id, content.plan_id),
                    character_plan_snippet(content),
                ))
            }
            _ => None,
        })
        .collect()
}

/// 人物性格路径查询。
fn find_character_traits(state: &WritingKnowledgeState, query: &str) -> Vec<FindResult> {
    state
        .changes
        .values()
        .filter_map(|change| match &change.content {
            RegisterContent::CharacterTrait(content)
                if query_matches(query, &[&content.character, &content.trait_name]) =>
            {
                Some(change_result(
                    change,
                    &format!("{}: {}", content.character, content.trait_name),
                    character_trait_snippet(content),
                ))
            }
            _ => None,
        })
        .collect()
}

/// 人物关系路径查询。
fn find_relationships(state: &WritingKnowledgeState, query: &str) -> Vec<FindResult> {
    state
        .changes
        .values()
        .filter_map(|change| match &change.content {
            RegisterContent::Relationship(content)
                if query_matches(
                    query,
                    &[
                        &content.character_a,
                        &content.character_b,
                        &content.relationship_name,
                    ],
                ) =>
            {
                Some(change_result(
                    change,
                    &format!(
                        "{} / {}: {}",
                        content.character_a, content.character_b, content.relationship_name
                    ),
                    relationship_snippet(content),
                ))
            }
            _ => None,
        })
        .collect()
}

/// 事件到故事段查询。
fn find_event_segments(state: &WritingKnowledgeState, query: &str) -> Vec<FindResult> {
    let mut results = Vec::new();
    for event in state.events.values() {
        if !query_matches(query, &[&event.event_id, &event.summary]) {
            continue;
        }
        for segment_id in &event.segment_ids {
            if let Some(segment) = state.segments.get(segment_id) {
                results.push(segment_result(segment, None));
            }
        }
    }
    results
}

/// 故事段查询；正文文本只有显式要求且后续接入 resolver 时才填充。
fn find_segments(
    state: &WritingKnowledgeState,
    query: &str,
    _include_text: bool,
) -> Vec<FindResult> {
    state
        .segments
        .values()
        .filter(|segment| {
            query_matches(
                query,
                &[&segment.segment_id, &segment.number, &segment.summary],
            )
        })
        .map(|segment| {
            // 正文回填依赖当前文档上下文，由工具执行器在显式 include_text 时处理。
            segment_result(segment, None)
        })
        .collect()
}

/// 伏笔查询，空查询默认返回全部未回收伏笔。
fn find_foreshadowing(state: &WritingKnowledgeState, query: &str) -> Vec<FindResult> {
    state
        .foreshadowing
        .values()
        .filter(|record| {
            record.status != ForeshadowingStatus::Recovered
                && query_matches(
                    query,
                    &[&record.foreshadowing_id, &record.title, &record.description],
                )
        })
        .map(|record| {
            let mut spans = Vec::new();
            for segment_id in record
                .planted_segment_ids
                .iter()
                .chain(record.recovered_segment_ids.iter())
            {
                if let Some(segment) = state.segments.get(segment_id) {
                    spans.push(segment.source.clone());
                }
            }
            FindResult {
                result_id: record.foreshadowing_id.clone(),
                title: record.title.clone(),
                snippet: record.description.clone(),
                score: 1.0,
                source: "writing_knowledge.foreshadowing".to_owned(),
                spans,
                text: None,
                metadata: json!({
                    "status": record.status,
                    "planted_segment_ids": &record.planted_segment_ids,
                    "recovered_segment_ids": &record.recovered_segment_ids,
                }),
            }
        })
        .collect()
}

/// 主题锚点查询。
fn find_theme_anchors(state: &WritingKnowledgeState, query: &str) -> Vec<FindResult> {
    state
        .changes
        .values()
        .filter_map(|change| match &change.content {
            RegisterContent::ThemeAnchor(content)
                if query_matches(
                    query,
                    &[&content.anchor_id, &content.title, &content.statement],
                ) || content
                    .motifs
                    .iter()
                    .any(|motif| query_matches(query, &[motif])) =>
            {
                Some(change_result(
                    change,
                    &format!("{}: {}", content.anchor_id, content.title),
                    theme_anchor_snippet(content),
                ))
            }
            _ => None,
        })
        .collect()
}

/// 总结查询。
fn find_summaries(
    summaries: &BTreeMap<String, String>,
    source: &str,
    query: &str,
) -> Vec<FindResult> {
    summaries
        .iter()
        .filter(|(id, summary)| query_matches(query, &[id.as_str(), summary.as_str()]))
        .map(|(id, summary)| FindResult {
            result_id: id.clone(),
            title: id.clone(),
            snippet: summary.clone(),
            score: 1.0,
            source: format!("writing_knowledge.{source}"),
            spans: Vec::new(),
            text: None,
            metadata: Value::Null,
        })
        .collect()
}

/// 把注册项转换成 find 结果。
fn change_result(change: &RegisteredChange, title: &str, snippet: String) -> FindResult {
    FindResult {
        result_id: change.change_id.clone(),
        title: title.to_owned(),
        snippet,
        score: match change.status {
            RegisteredChangeStatus::Deleted => 0.1,
            RegisteredChangeStatus::Planned => 0.8,
            RegisteredChangeStatus::Realized => 1.0,
        },
        source: "writing_knowledge.register".to_owned(),
        spans: Vec::new(),
        text: None,
        metadata: json!({
            "function": change.function,
            "status": change.status,
            "linked_segment_ids": &change.linked_segment_ids,
        }),
    }
}

/// 把故事段转换成 find 结果。
fn segment_result(segment: &StorySegment, text: Option<String>) -> FindResult {
    FindResult {
        result_id: segment.segment_id.clone(),
        title: format!("{}#{}", segment.chapter_id, segment.number),
        snippet: segment.summary.clone(),
        score: 1.0,
        source: "writing_knowledge.segment".to_owned(),
        spans: vec![segment.source.clone()],
        text,
        metadata: json!({
            "chapter_id": &segment.chapter_id,
            "number": &segment.number,
        }),
    }
}

/// 生成人物实体摘要。
fn character_profile_snippet(content: &CharacterProfileContent) -> String {
    let aliases = if content.aliases.is_empty() {
        String::new()
    } else {
        format!("别名：{}。", content.aliases.join("、"))
    };
    let new_marker = if content.is_new_character {
        "新人物。"
    } else {
        ""
    };
    format!(
        "{}{}叙事定位：{}。初始状态：{}。",
        new_marker, aliases, content.narrative_role, content.initial_state
    )
}

/// 生成人物出场计划摘要。
fn character_plan_snippet(content: &CharacterPlanContent) -> String {
    let scope = match (&content.stage_id, &content.chapter_id) {
        (Some(stage_id), Some(chapter_id)) => format!("阶段 {stage_id} / 章节 {chapter_id}"),
        (Some(stage_id), None) => format!("阶段 {stage_id}"),
        (None, Some(chapter_id)) => format!("章节 {chapter_id}"),
        (None, None) => "未指定范围".to_owned(),
    };
    format!(
        "{} 出场于{}，承担{}；目标：{}。{}",
        content.character_id,
        scope,
        content.narrative_function,
        content.appearance_goal,
        content.relation_to_theme
    )
}

/// 生成人物性格变化摘要。
fn character_trait_snippet(content: &CharacterTraitContent) -> String {
    match &content.from_value {
        Some(from_value) => format!(
            "{} 从 {} 变化为 {}。{}",
            content.trait_name, from_value, content.to_value, content.reason
        ),
        None => format!(
            "{} 设定为 {}。{}",
            content.trait_name, content.to_value, content.reason
        ),
    }
}

/// 生成人物关系变化摘要。
fn relationship_snippet(content: &RelationshipContent) -> String {
    match &content.from_value {
        Some(from_value) => format!(
            "{} 从 {} 变化为 {}。{}",
            content.relationship_name, from_value, content.to_value, content.reason
        ),
        None => format!(
            "{} 设定为 {}。{}",
            content.relationship_name, content.to_value, content.reason
        ),
    }
}

/// 生成主题锚点摘要。
fn theme_anchor_snippet(content: &ThemeAnchorContent) -> String {
    let motifs = if content.motifs.is_empty() {
        String::new()
    } else {
        format!("母题：{}。", content.motifs.join("、"))
    };
    format!("{}{}", content.statement, motifs)
}

/// 空查询匹配全部；非空查询做大小写无关的子串匹配。
fn query_matches(query: &str, fields: &[&str]) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    fields
        .iter()
        .any(|field| field.to_lowercase().contains(&query))
}

/// 向双向索引中写入唯一链接。
fn link_unique(map: &mut BTreeMap<String, Vec<String>>, key: &str, value: String) {
    let values = map.entry(key.to_owned()).or_default();
    push_unique(values, value);
}

/// 向 Vec 写入唯一值。
fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn merge_unique(values: &mut Vec<String>, additional: Vec<String>) {
    for value in additional {
        push_unique(values, value);
    }
}

fn validate_event_references(state: &WritingKnowledgeState, event: &StoryEvent) -> CoreResult<()> {
    for segment_id in &event.segment_ids {
        let segment = state.segments.get(segment_id).ok_or_else(|| {
            CoreError::validation(format!(
                "story event references missing story segment: {segment_id}"
            ))
        })?;
        if !event.chapter_ids.iter().any(|id| id == &segment.chapter_id) {
            return Err(CoreError::validation(format!(
                "story event segment {segment_id} belongs to unreferenced chapter {}",
                segment.chapter_id
            )));
        }
    }
    Ok(())
}

/// 从索引 Vec 中移除指定值，空 Vec 会被清理。
fn unlink_value(map: &mut BTreeMap<String, Vec<String>>, key: &str, value: &str) {
    if let Some(values) = map.get_mut(key) {
        values.retain(|existing| existing != value);
        if values.is_empty() {
            map.remove(key);
        }
    }
}

/// 本模块内部的非空校验，避免把私有模型函数暴露给存储实现。
fn validate_non_empty_local(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }
    Ok(())
}
