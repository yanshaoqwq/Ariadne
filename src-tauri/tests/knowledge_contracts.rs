use ariadne::core::{ApprovalPolicy, AutoModeState, SourceSpan, TextRange};
use ariadne::knowledge::{
    ApprovalStatus, FactKind, FactProposal, KnowledgeApprovalPolicy, KnowledgeFact,
    KnowledgeRebuildReason, KnowledgeRebuildStatus, KnowledgeRepository, LayeredSummary,
    MemoryKnowledgeBase, SummaryLevel, TwoStepApproval, VersionedFactValue,
};
use serde_json::{json, Value};

/// 构造测试用来源片段。
fn source_span() -> SourceSpan {
    SourceSpan {
        document_id: "doc-1".to_owned(),
        range: TextRange { start: 0, end: 12 },
        version: Some("v1".to_owned()),
    }
}

/// 构造测试事实。
fn fact(fact_id: &str, entity: &str, attribute: &str, value: Value) -> KnowledgeFact {
    KnowledgeFact {
        fact_id: fact_id.to_owned(),
        kind: FactKind::Character,
        entity: entity.to_owned(),
        attribute: attribute.to_owned(),
        fact: VersionedFactValue {
            value,
            source_version: "v1".to_owned(),
            sources: vec![source_span()],
        },
        metadata: Value::Null,
    }
}

/// 构造测试提案。
fn proposal(proposal_id: &str, candidate: KnowledgeFact) -> FactProposal {
    FactProposal {
        proposal_id: proposal_id.to_owned(),
        candidate,
        extraction_reason: "从章节中抽取到明确描述".to_owned(),
        metadata: Value::Null,
    }
}

/// 构造两步审批结果。
fn approval(approved: bool) -> TwoStepApproval {
    TwoStepApproval {
        writing_reason: "写作模型认为上下文支持该设定".to_owned(),
        judge_reason: "审批模型判断该候选需要进入待审队列".to_owned(),
        approved,
        metadata: Value::Null,
    }
}

/// 创建普通模式知识库。
fn normal_kb() -> MemoryKnowledgeBase {
    MemoryKnowledgeBase::new(
        KnowledgeApprovalPolicy::default(),
        AutoModeState {
            enabled: false,
            preauthorized_budget_usd: None,
        },
    )
}

/// 创建 Auto Mode 知识库。
fn auto_kb() -> MemoryKnowledgeBase {
    MemoryKnowledgeBase::new(
        KnowledgeApprovalPolicy {
            require_human_confirmation: false,
            node_approval: ApprovalPolicy {
                allow_auto_approval: true,
                approval_prompt_id: None,
                require_human_on_conflict: true,
            },
        },
        AutoModeState {
            enabled: true,
            preauthorized_budget_usd: None,
        },
    )
}

/// 验证摘要必须保留来源和版本。
#[test]
fn summaries_require_sources_and_versions() {
    let kb = normal_kb();
    let summary = LayeredSummary {
        summary_id: "sum-1".to_owned(),
        level: SummaryLevel::Chapter,
        subject_id: "chapter-1".to_owned(),
        text: "章节摘要".to_owned(),
        source_version: "v1".to_owned(),
        sources: vec![source_span()],
        metadata: Value::Null,
    };

    kb.upsert_summary(summary.clone()).unwrap();
    assert_eq!(kb.summary("sum-1").unwrap(), Some(summary.clone()));

    let invalid = LayeredSummary {
        summary_id: "sum-2".to_owned(),
        sources: Vec::new(),
        ..summary
    };
    assert!(kb.upsert_summary(invalid).is_err());
}

/// 验证普通模式下无审批的候选会进入 pending。
#[test]
fn normal_mode_keeps_proposals_pending() {
    let kb = normal_kb();
    let decision = kb
        .apply_proposal(
            proposal("proposal-1", fact("fact-1", "阿宁", "性格", json!("谨慎"))),
            None,
        )
        .unwrap();

    assert_eq!(decision.status, ApprovalStatus::Pending);
    assert!(decision.requires_human_review);
    assert!(kb.fact("fact-1").unwrap().is_none());
}

/// 验证 Auto Mode 会自动接受无冲突候选。
#[test]
fn auto_mode_approves_non_conflicting_proposals() {
    let kb = auto_kb();
    let decision = kb
        .apply_proposal(
            proposal("proposal-1", fact("fact-1", "阿宁", "性格", json!("谨慎"))),
            None,
        )
        .unwrap();

    assert_eq!(decision.status, ApprovalStatus::Approved);
    assert_eq!(kb.fact("fact-1").unwrap().unwrap().entity, "阿宁");
}

/// 验证冲突不会静默覆盖旧事实，而是进入冲突队列。
#[test]
fn conflicts_are_queued_without_overwriting_existing_fact() {
    let kb = auto_kb();
    let original = fact("fact-1", "阿宁", "性格", json!("谨慎"));
    kb.upsert_fact(original.clone()).unwrap();

    let decision = kb
        .apply_proposal(
            proposal("proposal-2", fact("fact-2", "阿宁", "性格", json!("冲动"))),
            Some(approval(false)),
        )
        .unwrap();
    let conflicts = kb.list_conflicts().unwrap();

    assert_eq!(decision.status, ApprovalStatus::Conflict);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(kb.fact("fact-1").unwrap(), Some(original));
    assert!(kb.fact("fact-2").unwrap().is_none());
}

/// 验证 Auto Mode 下冲突即使尚无两步审批也会先入队。
#[test]
fn auto_mode_queues_conflict_without_prior_approval() {
    let kb = auto_kb();
    kb.upsert_fact(fact("fact-1", "阿宁", "性格", json!("谨慎")))
        .unwrap();

    let decision = kb
        .apply_proposal(
            proposal("proposal-2", fact("fact-2", "阿宁", "性格", json!("冲动"))),
            None,
        )
        .unwrap();
    let conflicts = kb.list_conflicts().unwrap();

    assert_eq!(decision.status, ApprovalStatus::Conflict);
    assert!(decision.requires_human_review);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].writing_reason, "从章节中抽取到明确描述");
    assert_eq!(
        conflicts[0].judge_reason,
        "awaiting independent LLM judgment"
    );
    assert!(kb.fact("fact-2").unwrap().is_none());
}

/// 验证两步审批通过后候选事实入库。
#[test]
fn two_step_approval_can_accept_proposal() {
    let kb = normal_kb();
    let decision = kb
        .apply_proposal(
            proposal("proposal-3", fact("fact-3", "阿宁", "别名", json!("小宁"))),
            Some(approval(true)),
        )
        .unwrap();

    assert_eq!(decision.status, ApprovalStatus::Approved);
    assert!(kb.fact("fact-3").unwrap().is_some());
}

/// 验证 metadata、索引或 Git 回档后的重建状态可诊断。
#[test]
fn rebuild_status_reports_recovery_requirements() {
    let kb = normal_kb();

    kb.mark_rebuild_required(
        KnowledgeRebuildReason::GitRestore,
        "restore branch checked out",
    );
    let health = kb.health_report();

    assert_eq!(health.status, KnowledgeRebuildStatus::Required);
    assert!(health.metadata_rebuild_required);
    assert!(health.index_rebuild_required);

    let report = kb.complete_rebuild().unwrap();
    assert_eq!(report.status, KnowledgeRebuildStatus::Completed);
    assert_eq!(kb.health_report().status, KnowledgeRebuildStatus::Completed);
}
