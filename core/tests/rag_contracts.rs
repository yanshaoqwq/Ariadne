use ariadne::contracts::{
    AutoModeState, ProviderCapability, ProviderDefinition, ProviderType, SourceSpan, TextRange,
};
use ariadne::llm::{ToolExecutionContext, ToolExecutor};
use ariadne::providers::{
    Provider, ProviderCallContext, SearchProvider, SearchProviderRequest, SearchProviderResponse,
    SearchProviderResult, ToolCall,
};
use ariadne::rag::{
    insert_lines_to_patch, load_display_name_resources, load_prompt_resources,
    midpoint_segment_number, render_node_prompt, render_prompt_template, replace_lines_to_patch,
    search_response_to_writing_response, tool_definitions_for_agent, CharacterTraitContent,
    ConfirmationItem, ConfirmationKind, ConfirmationState, ForeshadowingContent,
    ForeshadowingRecord, ForeshadowingStatus, ForeshadowingUpdate, MemoryWritingKnowledgeBase,
    NodePromptConfig, PatchSession, PromptTemplateContext, RealizedChangeLink, RegisterContent,
    RegisterFunction, RegisterOperation, RegisteredChange, RegisteredChangeStatus,
    SqliteWritingKnowledgeStore, StoryEvent, StoryEventStatus, StorySegment, SummaryPipelineDraft,
    SummaryPipelineExecutor, SummaryPipelineStep, WriterDocumentContext, WriterInsertLines,
    WriterReplaceLines, WritingAgentKind, WritingConfirmationPolicy, WritingContextAssembler,
    WritingContextRequest, WritingDocumentScope, WritingNodeDefinition, WritingToolExecutor,
    TOOL_CRITIC_FIND, TOOL_CRITIC_SEARCH, TOOL_DESIGNER_FIND, TOOL_DESIGNER_INSERT_LINES,
    TOOL_DESIGNER_REGISTER, TOOL_DESIGNER_REPLACE_LINES, TOOL_DESIGNER_SEARCH, TOOL_DETAIL_FIND,
    TOOL_DETAIL_SEARCH, TOOL_OUTLINER_FIND, TOOL_OUTLINER_INSERT_LINES, TOOL_OUTLINER_REGISTER,
    TOOL_OUTLINER_REPLACE_LINES, TOOL_OUTLINER_SEARCH, TOOL_PLANNER_FIND,
    TOOL_PLANNER_INSERT_LINES, TOOL_PLANNER_REGISTER, TOOL_PLANNER_REPLACE_LINES,
    TOOL_PLANNER_SEARCH, TOOL_PRUDENT_FIND, TOOL_PRUDENT_SEARCH, TOOL_WRITER_FIND,
    TOOL_WRITER_INSERT_LINES, TOOL_WRITER_REPLACE_LINES, TOOL_WRITER_SEARCH,
};
use serde_json::{json, Value};

struct MockSearchProvider;

impl Provider for MockSearchProvider {
    /// 返回测试 search provider 定义。
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: "mock-search".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Mock Search".to_owned(),
            capabilities: vec![ProviderCapability::Search],
            config_schema: Value::Null,
        }
    }
}

impl SearchProvider for MockSearchProvider {
    /// 返回固定搜索结果，便于验证 search 不会自动入库。
    fn search(
        &self,
        _context: &ProviderCallContext,
        request: SearchProviderRequest,
    ) -> ariadne::contracts::CoreResult<SearchProviderResponse> {
        Ok(SearchProviderResponse {
            results: vec![SearchProviderResult {
                title: format!("资料: {}", request.query),
                url: "https://example.test/source".to_owned(),
                snippet: "外部资料摘要".to_owned(),
                score: 0.9,
                metadata: json!({ "limit": request.limit }),
            }],
            cost_usd: None,
            raw: Value::Null,
        })
    }
}

/// 构造测试用来源片段。
fn source_span() -> SourceSpan {
    SourceSpan {
        document_id: "doc-1".to_owned(),
        range: TextRange { start: 0, end: 9 },
        version: Some("v1".to_owned()),
    }
}

/// 构造标准 tool 执行上下文。
fn tool_context() -> ToolExecutionContext {
    ToolExecutionContext {
        provider_id: "mock-llm".to_owned(),
        workflow_id: None,
        run_id: None,
        node_id: None,
        round: 0,
    }
}

/// 验证内置提示词和显示名称资源完整可加载。
#[test]
fn rag_resources_validate_required_prompt_and_display_keys() {
    let prompts = load_prompt_resources().unwrap();
    let display_names = load_display_name_resources().unwrap();

    assert!(prompts.contains_key("agent_prompt.outliner"));
    assert!(prompts.contains_key("node_template.writer.default"));
    assert!(prompts.contains_key("tool.planner_register"));
    assert!(prompts["tool.writer_search"]
        .prompt
        .contains("现实中的情况"));
    assert_eq!(display_names["agent.designer"], "阶段设计");
    assert_eq!(display_names["tool.writer-replace-lines"], "按行替换正文");
}

/// 验证不同 agent 暴露的工具集合符合总结机制契约。
#[test]
fn writing_agents_expose_expected_tools_from_prompt_resources() {
    let prompts = load_prompt_resources().unwrap();
    let outliner = tool_definitions_for_agent(WritingAgentKind::Outliner, &prompts).unwrap();
    let designer = tool_definitions_for_agent(WritingAgentKind::Designer, &prompts).unwrap();
    let planner = tool_definitions_for_agent(WritingAgentKind::Planner, &prompts).unwrap();
    let detail = tool_definitions_for_agent(WritingAgentKind::Detail, &prompts).unwrap();
    let writer = tool_definitions_for_agent(WritingAgentKind::Writer, &prompts).unwrap();
    let critic = tool_definitions_for_agent(WritingAgentKind::Critic, &prompts).unwrap();
    let prudent = tool_definitions_for_agent(WritingAgentKind::Prudent, &prompts).unwrap();

    assert_eq!(
        outliner
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            TOOL_OUTLINER_REGISTER,
            TOOL_OUTLINER_FIND,
            TOOL_OUTLINER_SEARCH,
            TOOL_OUTLINER_INSERT_LINES,
            TOOL_OUTLINER_REPLACE_LINES
        ]
    );
    assert_eq!(
        designer
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            TOOL_DESIGNER_REGISTER,
            TOOL_DESIGNER_FIND,
            TOOL_DESIGNER_SEARCH,
            TOOL_DESIGNER_INSERT_LINES,
            TOOL_DESIGNER_REPLACE_LINES
        ]
    );
    assert_eq!(
        planner
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            TOOL_PLANNER_REGISTER,
            TOOL_PLANNER_FIND,
            TOOL_PLANNER_SEARCH,
            TOOL_PLANNER_INSERT_LINES,
            TOOL_PLANNER_REPLACE_LINES
        ]
    );
    assert_eq!(
        detail
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec![TOOL_DETAIL_FIND, TOOL_DETAIL_SEARCH]
    );
    assert_eq!(
        writer
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            TOOL_WRITER_FIND,
            TOOL_WRITER_SEARCH,
            TOOL_WRITER_INSERT_LINES,
            TOOL_WRITER_REPLACE_LINES
        ]
    );
    assert!(!writer.iter().any(|tool| tool.name == TOOL_PLANNER_REGISTER));
    assert_eq!(
        critic
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec![TOOL_CRITIC_FIND, TOOL_CRITIC_SEARCH]
    );
    assert_eq!(
        prudent
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec![TOOL_PRUDENT_FIND, TOOL_PRUDENT_SEARCH]
    );
    assert!(!critic.iter().any(|tool| tool.name == TOOL_PLANNER_REGISTER));
    assert!(!prudent
        .iter()
        .any(|tool| tool.name == TOOL_PLANNER_REGISTER));
    assert_eq!(
        planner[0].description,
        prompts["tool.planner_register"].describe
    );
}

/// 验证一个写作节点就是一个 agent，且内置节点边界固定。
#[test]
fn writing_nodes_are_one_to_one_with_agents() {
    let prompts = load_prompt_resources().unwrap();
    let display_names = load_display_name_resources().unwrap();
    let nodes = WritingNodeDefinition::built_in_nodes();

    assert_eq!(
        nodes.iter().map(|node| node.agent).collect::<Vec<_>>(),
        vec![
            WritingAgentKind::Outliner,
            WritingAgentKind::Designer,
            WritingAgentKind::Planner,
            WritingAgentKind::Detail,
            WritingAgentKind::Writer,
            WritingAgentKind::Critic,
            WritingAgentKind::Prudent,
            WritingAgentKind::Polisher,
            WritingAgentKind::Summarizer,
        ]
    );
    for node in &nodes {
        node.validate(&prompts, &display_names).unwrap();
        assert!(node
            .prompt_keys
            .iter()
            .any(|key| key == node.agent.prompt_key()));
        assert!(node
            .prompt_keys
            .iter()
            .any(|key| key == node.agent.default_template_key()));
    }
    assert_eq!(
        nodes[4].tool_names,
        vec![
            TOOL_WRITER_FIND,
            TOOL_WRITER_SEARCH,
            TOOL_WRITER_INSERT_LINES,
            TOOL_WRITER_REPLACE_LINES
        ]
    );
    assert!(nodes
        .iter()
        .find(|node| node.agent == WritingAgentKind::Summarizer)
        .unwrap()
        .tool_names
        .is_empty());
}

/// 验证节点提示词模板可内联节点提示词、上下文区块和上游数据边 alias。
#[test]
fn node_prompt_template_renders_inline_context_and_alias_inputs() {
    let prompts = load_prompt_resources().unwrap();
    let kb = MemoryWritingKnowledgeBase::new();
    let assembler = WritingContextAssembler::new(&kb);
    let mut template_inputs = std::collections::BTreeMap::new();
    template_inputs.insert("上游补充".to_owned(), "来自上游节点的内容".to_owned());

    let bundle = assembler
        .assemble(WritingContextRequest {
            agent: WritingAgentKind::Writer,
            chapter_id: "chapter-1".to_owned(),
            stage_id: None,
            user_intent: None,
            global_outline: None,
            stage_outline: None,
            previous_stage_outline: None,
            chapter_summaries: None,
            outline: Some("本章大纲".to_owned()),
            details: Some("本章细节".to_owned()),
            previous_chapter_text: Some("上一章原文".to_owned()),
            current_draft_text: None,
            target_text: None,
            critic_outputs: None,
            revision_context: Some("返修上下文".to_owned()),
            template_inputs,
            metadata: Value::Null,
        })
        .unwrap();
    let mut config =
        NodePromptConfig::default_for_agent(WritingAgentKind::Writer, &prompts).unwrap();
    let context =
        PromptTemplateContext::from_bundle(WritingAgentKind::Writer, &prompts, &bundle).unwrap();
    let rendered = render_node_prompt(&config, &context).unwrap();

    assert!(rendered.contains("正式写作"));
    assert!(rendered.contains("上一章原文"));
    assert!(rendered.contains("本章大纲"));
    assert!(rendered.contains("返修上下文"));

    config
        .replace_template(
            "{{prompt.节点提示词}}\n{{input.上游补充}}\n{{system.当前章节号}}",
            Some("用户编辑".to_owned()),
        )
        .unwrap();
    let rendered = render_node_prompt(&config, &context).unwrap();
    assert!(rendered.contains("来自上游节点的内容"));
    assert!(rendered.contains("chapter-1"));
    assert_eq!(config.backups.len(), 1);
}

/// 验证缺失变量会返回可诊断错误，不会静默替换为空字符串。
#[test]
fn prompt_template_rejects_unresolved_variables() {
    let error =
        render_prompt_template("{{input.不存在}}", &PromptTemplateContext::default()).unwrap_err();

    assert!(error.to_string().contains("unresolved"));
}

/// 验证十进制故事段编号不用浮点数也能生成中点。
#[test]
fn segment_number_midpoint_uses_decimal_strings() {
    assert_eq!(midpoint_segment_number("1", "2").unwrap(), "1.5");
    assert_eq!(midpoint_segment_number("1", "1.5").unwrap(), "1.25");
}

/// 验证故事段、事件和注册项会建立双向索引。
#[test]
fn memory_writing_knowledge_maintains_bidirectional_indexes() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.upsert_segment(StorySegment {
        segment_id: "seg-1".to_owned(),
        number: "1".to_owned(),
        chapter_id: "chapter-1".to_owned(),
        summary: "阿宁决定进入废城".to_owned(),
        source: source_span(),
        metadata: Value::Null,
    })
    .unwrap();
    kb.upsert_event(StoryEvent {
        event_id: "event-1".to_owned(),
        summary: "进入废城".to_owned(),
        status: StoryEventStatus::Ongoing,
        segment_ids: vec!["seg-1".to_owned()],
        chapter_ids: vec!["chapter-1".to_owned()],
        metadata: Value::Null,
    })
    .unwrap();
    kb.upsert_registered_change(RegisteredChange {
        change_id: "change-1".to_owned(),
        function: RegisterFunction::Foreshadowing,
        status: RegisteredChangeStatus::Planned,
        content: RegisterContent::Foreshadowing(ForeshadowingContent {
            title: "旧钥匙".to_owned(),
            description: "阿宁在门缝里看见旧钥匙".to_owned(),
            intended_payoff: "第三章打开地下室".to_owned(),
        }),
        linked_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();
    kb.mark_change_realized("change-1", "seg-1").unwrap();

    let index = kb.index_snapshot().unwrap();
    assert_eq!(index.chapter_segments["chapter-1"], vec!["seg-1"]);
    assert_eq!(index.segment_events["seg-1"], vec!["event-1"]);
    assert_eq!(index.segment_changes["seg-1"], vec!["change-1"]);
}

/// 验证 planner-register 使用强类型 c 参数，并拒绝伏笔 update。
#[test]
fn planner_register_enforces_typed_content_and_lifecycle_rules() {
    let kb = MemoryWritingKnowledgeBase::new();
    let changes = kb
        .apply_register_operation(
            RegisterFunction::CharacterTrait,
            RegisterOperation::New,
            Some(RegisterContent::CharacterTrait(
                ariadne::rag::CharacterTraitContent {
                    character: "阿宁".to_owned(),
                    trait_name: "戒备心".to_owned(),
                    from_value: None,
                    to_value: "开始信任队友".to_owned(),
                    reason: "废城事件后产生变化".to_owned(),
                },
            )),
            Some("trait-1".to_owned()),
        )
        .unwrap();
    assert_eq!(changes[0].status, RegisteredChangeStatus::Planned);

    let error = kb
        .apply_register_operation(
            RegisterFunction::Foreshadowing,
            RegisterOperation::Update,
            Some(RegisterContent::Foreshadowing(ForeshadowingContent {
                title: "旧钥匙".to_owned(),
                description: "调整伏笔".to_owned(),
                intended_payoff: String::new(),
            })),
            None,
        )
        .unwrap_err();
    assert!(error.to_string().contains("update is only allowed"));
}

/// 验证自动生成的 register id 会避开显式指定过的 id。
#[test]
fn planner_register_generated_ids_skip_existing_explicit_ids() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.apply_register_operation(
        RegisterFunction::CharacterTrait,
        RegisterOperation::New,
        Some(RegisterContent::CharacterTrait(
            ariadne::rag::CharacterTraitContent {
                character: "阿宁".to_owned(),
                trait_name: "戒备心".to_owned(),
                from_value: None,
                to_value: "警觉".to_owned(),
                reason: "初始状态".to_owned(),
            },
        )),
        Some("register-character-trait-1".to_owned()),
    )
    .unwrap();

    let generated = kb
        .apply_register_operation(
            RegisterFunction::CharacterTrait,
            RegisterOperation::New,
            Some(RegisterContent::CharacterTrait(
                ariadne::rag::CharacterTraitContent {
                    character: "阿宁".to_owned(),
                    trait_name: "信任".to_owned(),
                    from_value: None,
                    to_value: "开始松动".to_owned(),
                    reason: "队友救援".to_owned(),
                },
            )),
            None,
        )
        .unwrap();

    assert_ne!(generated[0].change_id, "register-character-trait-1");
}

/// 验证删除故事段时不会留下事件、注册项和伏笔的反向索引孤儿。
#[test]
fn deleting_segment_cleans_all_reverse_indexes() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.upsert_segment(StorySegment {
        segment_id: "seg-1".to_owned(),
        number: "1".to_owned(),
        chapter_id: "chapter-1".to_owned(),
        summary: "阿宁发现旧钥匙".to_owned(),
        source: source_span(),
        metadata: Value::Null,
    })
    .unwrap();
    kb.upsert_event(StoryEvent {
        event_id: "event-1".to_owned(),
        summary: "发现钥匙".to_owned(),
        status: StoryEventStatus::Ongoing,
        segment_ids: vec!["seg-1".to_owned()],
        chapter_ids: vec!["chapter-1".to_owned()],
        metadata: Value::Null,
    })
    .unwrap();
    kb.upsert_registered_change(RegisteredChange {
        change_id: "change-1".to_owned(),
        function: RegisterFunction::CharacterTrait,
        status: RegisteredChangeStatus::Realized,
        content: RegisterContent::CharacterTrait(ariadne::rag::CharacterTraitContent {
            character: "阿宁".to_owned(),
            trait_name: "好奇心".to_owned(),
            from_value: None,
            to_value: "主动探索".to_owned(),
            reason: "发现旧钥匙".to_owned(),
        }),
        linked_segment_ids: vec!["seg-1".to_owned()],
        metadata: Value::Null,
    })
    .unwrap();
    kb.upsert_foreshadowing(ForeshadowingRecord {
        foreshadowing_id: "f-1".to_owned(),
        title: "旧钥匙".to_owned(),
        description: "门缝里的钥匙".to_owned(),
        status: ForeshadowingStatus::Planted,
        planted_segment_ids: vec!["seg-1".to_owned()],
        recovered_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();

    assert!(kb.delete_segment("seg-1").unwrap().is_some());
    let index = kb.index_snapshot().unwrap();

    assert!(!index.segment_chapter.contains_key("seg-1"));
    assert!(!index.segment_events.contains_key("seg-1"));
    assert!(!index.segment_changes.contains_key("seg-1"));
    assert!(!index.segment_foreshadowing.contains_key("seg-1"));
    assert!(!index
        .event_segments
        .get("event-1")
        .is_some_and(|values| values.contains(&"seg-1".to_owned())));
    assert!(!index
        .change_segments
        .get("change-1")
        .is_some_and(|values| values.contains(&"seg-1".to_owned())));
    assert!(!index
        .foreshadowing_segments
        .get("f-1")
        .is_some_and(|values| values.contains(&"seg-1".to_owned())));
}

/// 验证 find 默认不返回正文，显式 include_text 且有文档上下文才回填正文。
#[test]
fn find_defaults_to_lightweight_results_and_can_attach_text() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.upsert_segment(StorySegment {
        segment_id: "seg-1".to_owned(),
        number: "1".to_owned(),
        chapter_id: "chapter-1".to_owned(),
        summary: "阿宁走进废城".to_owned(),
        source: source_span(),
        metadata: Value::Null,
    })
    .unwrap();
    let executor = WritingToolExecutor::with_document(
        &kb,
        WriterDocumentContext {
            document_id: "doc-1",
            base_version: Some("v1"),
            text: "阿宁走\n进废城",
            scope: WritingDocumentScope::ChapterBody,
        },
    );

    let light = executor
        .execute(
            &tool_context(),
            &ToolCall {
                tool_call_id: "find-1".to_owned(),
                name: TOOL_WRITER_FIND.to_owned(),
                arguments: json!({
                    "a": "segment_text",
                    "b": "seg-1"
                }),
            },
        )
        .unwrap();
    assert!(light.value["results"][0].get("text").is_none());

    let with_text = executor
        .execute(
            &tool_context(),
            &ToolCall {
                tool_call_id: "find-2".to_owned(),
                name: TOOL_WRITER_FIND.to_owned(),
                arguments: json!({
                    "a": "segment_text",
                    "b": "seg-1",
                    "include_text": true
                }),
            },
        )
        .unwrap();
    assert_eq!(with_text.value["results"][0]["text"], "阿宁走");
}

/// 验证 SearchProvider 结果不会自动写入创作知识库。
#[test]
fn search_results_are_not_persisted_to_writing_knowledge() {
    let response = search_response_to_writing_response(SearchProviderResponse {
        results: vec![SearchProviderResult {
            title: "资料".to_owned(),
            url: "https://example.test".to_owned(),
            snippet: "摘要".to_owned(),
            score: 1.0,
            metadata: Value::Null,
        }],
        cost_usd: None,
        raw: Value::Null,
    });
    assert!(!response.persisted_to_knowledge);

    let provider = MockSearchProvider;
    let kb = MemoryWritingKnowledgeBase::new();
    let executor = WritingToolExecutor::new(&kb)
        .with_search_provider(&provider, ProviderCallContext::new("mock-search"));
    let output = executor
        .execute(
            &tool_context(),
            &ToolCall {
                tool_call_id: "search-1".to_owned(),
                name: TOOL_WRITER_SEARCH.to_owned(),
                arguments: json!({
                    "query": "宋代城市",
                    "limit": 1
                }),
            },
        )
        .unwrap();
    assert_eq!(output.value["persisted_to_knowledge"], false);
}

/// 验证 Writer 行号工具转换为 byte-range patch。
#[test]
fn writer_line_tools_convert_one_based_lines_to_document_patches() {
    let insert = insert_lines_to_patch(
        "甲\n乙",
        WriterInsertLines {
            document_id: "doc-1".to_owned(),
            base_version: Some("v1".to_owned()),
            after_line: 1,
            text: "插入\n".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(insert.hunks[0].range, TextRange { start: 4, end: 4 });

    let replace = replace_lines_to_patch(
        "甲\n乙\n丙",
        WriterReplaceLines {
            document_id: "doc-1".to_owned(),
            base_version: Some("v1".to_owned()),
            start_line: 2,
            end_line: 2,
            text: "新乙\n".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(replace.hunks[0].range, TextRange { start: 4, end: 8 });
}

/// 验证行号 patch 会话内的多次操作基于模拟文本连续应用，并最终提交一个 patch。
#[test]
fn patch_session_commits_multiple_line_ops_as_one_patch() {
    let mut session = PatchSession::new("doc-1", Some("v1".to_owned()), "甲\n乙\n丙").unwrap();

    session.insert_lines(1, "新行\n").unwrap();
    session.replace_lines(2, 2, "替换\n").unwrap();
    let commit = session.commit().unwrap();

    assert_eq!(session.simulated, "甲\n替换\n乙\n丙");
    assert_eq!(commit.operations.len(), 2);
    assert_eq!(commit.patch.hunks.len(), 1);
    assert_eq!(commit.patch.base_version.as_deref(), Some("v1"));
}

/// 验证未回收伏笔会进入 Planner 默认可查询集合。
#[test]
fn unresolved_foreshadowing_excludes_recovered_items() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.upsert_foreshadowing(ForeshadowingRecord {
        foreshadowing_id: "f-1".to_owned(),
        title: "旧钥匙".to_owned(),
        description: "门缝里的钥匙".to_owned(),
        status: ForeshadowingStatus::Planted,
        planted_segment_ids: Vec::new(),
        recovered_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();
    kb.upsert_foreshadowing(ForeshadowingRecord {
        foreshadowing_id: "f-2".to_owned(),
        title: "暗号".to_owned(),
        description: "已经回收".to_owned(),
        status: ForeshadowingStatus::Recovered,
        planted_segment_ids: Vec::new(),
        recovered_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();

    let unresolved = kb.unresolved_foreshadowing().unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].foreshadowing_id, "f-1");
}

/// 验证写作上下文按独立节点/agent 组装，Writer 不默认接收未回收伏笔。
#[test]
fn writing_contexts_are_specialized_by_node_agent() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.upsert_chapter_summary("chapter-0", "上一章总结")
        .unwrap();
    kb.upsert_registered_change(RegisteredChange {
        change_id: "trait-1".to_owned(),
        function: RegisterFunction::CharacterTrait,
        status: RegisteredChangeStatus::Realized,
        content: RegisterContent::CharacterTrait(ariadne::rag::CharacterTraitContent {
            character: "阿宁".to_owned(),
            trait_name: "戒备心".to_owned(),
            from_value: None,
            to_value: "开始信任队友".to_owned(),
            reason: "废城事件后产生变化".to_owned(),
        }),
        linked_segment_ids: vec!["seg-1".to_owned()],
        metadata: Value::Null,
    })
    .unwrap();
    kb.upsert_foreshadowing(ForeshadowingRecord {
        foreshadowing_id: "f-1".to_owned(),
        title: "旧钥匙".to_owned(),
        description: "门缝里的钥匙".to_owned(),
        status: ForeshadowingStatus::Planted,
        planted_segment_ids: Vec::new(),
        recovered_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();
    let assembler = WritingContextAssembler::new(&kb);

    let planner = assembler
        .assemble(WritingContextRequest {
            agent: WritingAgentKind::Planner,
            chapter_id: "chapter-1".to_owned(),
            stage_id: None,
            user_intent: None,
            global_outline: None,
            stage_outline: None,
            previous_stage_outline: None,
            chapter_summaries: None,
            outline: None,
            details: None,
            previous_chapter_text: Some("上一章全文".to_owned()),
            current_draft_text: None,
            target_text: None,
            critic_outputs: None,
            revision_context: None,
            template_inputs: Default::default(),
            metadata: Value::Null,
        })
        .unwrap();
    assert!(planner
        .sections
        .iter()
        .any(|section| section.section_id == "unresolved_foreshadowing"));

    let writer = assembler
        .assemble(WritingContextRequest {
            agent: WritingAgentKind::Writer,
            chapter_id: "chapter-1".to_owned(),
            stage_id: None,
            user_intent: None,
            global_outline: None,
            stage_outline: None,
            previous_stage_outline: None,
            chapter_summaries: None,
            outline: Some("本章大纲".to_owned()),
            details: Some("细节材料".to_owned()),
            previous_chapter_text: None,
            current_draft_text: Some("甲\n乙".to_owned()),
            target_text: None,
            critic_outputs: None,
            revision_context: None,
            template_inputs: Default::default(),
            metadata: Value::Null,
        })
        .unwrap();
    assert!(writer
        .sections
        .iter()
        .any(|section| section.section_id == "line_numbered_draft"));
    assert!(!writer
        .sections
        .iter()
        .any(|section| section.section_id == "unresolved_foreshadowing"));
}

/// 验证 Summarizer 流水线按顺序写入摘要并在普通模式下生成待确认项和未落地问题。
#[test]
fn summary_pipeline_applies_draft_and_tracks_confirmations_and_issues() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.upsert_registered_change(RegisteredChange {
        change_id: "trait-1".to_owned(),
        function: RegisterFunction::CharacterTrait,
        status: RegisteredChangeStatus::Planned,
        content: RegisterContent::CharacterTrait(ariadne::rag::CharacterTraitContent {
            character: "阿宁".to_owned(),
            trait_name: "戒备心".to_owned(),
            from_value: None,
            to_value: "开始信任队友".to_owned(),
            reason: "废城事件后产生变化".to_owned(),
        }),
        linked_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::normal_default(),
        AutoModeState::default(),
    );

    let report = executor
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "chapter-1".to_owned(),
            segments: vec![StorySegment {
                segment_id: "seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "chapter-1".to_owned(),
                summary: "阿宁进入废城".to_owned(),
                source: source_span(),
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "event-1".to_owned(),
                summary: "进入废城".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["seg-1".to_owned()],
                chapter_ids: vec!["chapter-1".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("章节总结".to_owned()),
            stage_id: Some("stage-1".to_owned()),
            stage_summary: Some("阶段总结".to_owned()),
            is_new_stage: None,
            realized_changes: Vec::new(),
            foreshadowing_updates: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();

    assert!(report.paused);
    // F14：normal policy 下步骤未确认前不写入 active 知识
    assert!(report.completed_steps.is_empty());
    assert_eq!(report.confirmation_ids.len(), 4);
    assert_eq!(report.planner_issue_ids.len(), 0);
    assert_eq!(kb.chapter_summary("chapter-1").unwrap(), None);
    assert!(kb.all_segments().unwrap().is_empty());
    assert_eq!(kb.confirmations(None).unwrap().len(), 4);
    // 批准全部四步后知识才激活
    for id in &report.confirmation_ids {
        ariadne::rag::approve_confirmation(&kb, id).unwrap();
    }
    assert_eq!(
        kb.chapter_summary("chapter-1").unwrap(),
        Some("章节总结".to_owned())
    );
    assert_eq!(kb.all_segments().unwrap().len(), 1);
    assert_eq!(kb.planner_issues("chapter-1").unwrap().len(), 1);
}

#[test]
fn summary_pipeline_rejects_event_with_missing_segment_before_mutation() {
    let kb = MemoryWritingKnowledgeBase::new();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::normal_default(),
        AutoModeState::default(),
    );
    let error = executor
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "chapter-invalid".to_owned(),
            segments: vec![StorySegment {
                segment_id: "chapter-invalid::seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "chapter-invalid".to_owned(),
                summary: "有效故事段".to_owned(),
                source: source_span(),
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "event-invalid".to_owned(),
                summary: "引用了不存在的故事段".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["chapter-invalid::seg-missing".to_owned()],
                chapter_ids: vec!["chapter-invalid".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("不会写入".to_owned()),
            stage_id: Some("stage-invalid".to_owned()),
            stage_summary: Some("不会写入".to_owned()),
            is_new_stage: None,
            realized_changes: Vec::new(),
            foreshadowing_updates: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap_err();

    assert!(error.to_string().contains("missing story segment"));
    assert!(kb.all_segments().unwrap().is_empty());
    assert!(kb.all_events().unwrap().is_empty());
    assert!(kb.confirmations(None).unwrap().is_empty());
}

#[test]
fn summary_pipeline_rerun_replaces_active_entities_and_keeps_revision_history() {
    let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
    let kb = MemoryWritingKnowledgeBase::new();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::auto_audit_default(),
        AutoModeState {
            enabled: true,
            preauthorized_budget_usd: None,
        },
    );
    let first = executor
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "chapter-rerun".to_owned(),
            segments: vec![
                StorySegment {
                    segment_id: "chapter-rerun::seg-1".to_owned(),
                    number: "1".to_owned(),
                    chapter_id: "chapter-rerun".to_owned(),
                    summary: "第一段".to_owned(),
                    source: source_span(),
                    metadata: Value::Null,
                },
                StorySegment {
                    segment_id: "chapter-rerun::seg-2".to_owned(),
                    number: "2".to_owned(),
                    chapter_id: "chapter-rerun".to_owned(),
                    summary: "将被新 revision 删除的第二段".to_owned(),
                    source: source_span(),
                    metadata: Value::Null,
                },
            ],
            events: vec![StoryEvent {
                event_id: "event-old".to_owned(),
                summary: "旧事件".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["chapter-rerun::seg-2".to_owned()],
                chapter_ids: vec!["chapter-rerun".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("旧章节总结".to_owned()),
            stage_id: Some("stage-1".to_owned()),
            stage_summary: Some("旧阶段总结".to_owned()),
            is_new_stage: None,
            realized_changes: Vec::new(),
            foreshadowing_updates: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();
    store.save_knowledge(&kb).unwrap();

    let second = executor
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "chapter-rerun".to_owned(),
            segments: vec![StorySegment {
                segment_id: "chapter-rerun::seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "chapter-rerun".to_owned(),
                summary: "合并后的唯一故事段".to_owned(),
                source: source_span(),
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "event-new".to_owned(),
                summary: "新事件".to_owned(),
                status: StoryEventStatus::Completed,
                segment_ids: vec!["chapter-rerun::seg-1".to_owned()],
                chapter_ids: vec!["chapter-rerun".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("新章节总结".to_owned()),
            stage_id: Some("stage-1".to_owned()),
            stage_summary: Some("新阶段总结".to_owned()),
            is_new_stage: None,
            realized_changes: Vec::new(),
            foreshadowing_updates: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();
    store.save_knowledge(&kb).unwrap();

    assert_ne!(first.revision_id, second.revision_id);
    assert_eq!(kb.all_segments().unwrap().len(), 1);
    assert_eq!(kb.all_events().unwrap()[0].event_id, "event-new");
    assert_eq!(kb.confirmations(None).unwrap().len(), 8);

    let loaded = store.load_knowledge().unwrap();
    assert_eq!(loaded.all_segments().unwrap().len(), 1);
    assert_eq!(loaded.all_events().unwrap()[0].event_id, "event-new");
    assert_eq!(loaded.confirmations(None).unwrap().len(), 8);
    assert!(loaded
        .confirmations(None)
        .unwrap()
        .iter()
        .all(|item| item.metadata.get("revision_id").is_some()));
}

/// 验证 Auto Mode 下确认项默认进入自动审计状态，且已落地注册项不会进问题队列。
#[test]
fn summary_pipeline_auto_mode_auto_audits_confirmations() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.upsert_registered_change(RegisteredChange {
        change_id: "trait-1".to_owned(),
        function: RegisterFunction::CharacterTrait,
        status: RegisteredChangeStatus::Planned,
        content: RegisterContent::CharacterTrait(ariadne::rag::CharacterTraitContent {
            character: "阿宁".to_owned(),
            trait_name: "戒备心".to_owned(),
            from_value: None,
            to_value: "开始信任队友".to_owned(),
            reason: "废城事件后产生变化".to_owned(),
        }),
        linked_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::auto_audit_default(),
        AutoModeState {
            enabled: true,
            preauthorized_budget_usd: None,
        },
    );

    let report = executor
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "chapter-1".to_owned(),
            segments: vec![StorySegment {
                segment_id: "seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "chapter-1".to_owned(),
                summary: "阿宁进入废城并信任队友".to_owned(),
                source: source_span(),
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "event-1".to_owned(),
                summary: "废城信任事件继续推进".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["seg-1".to_owned()],
                chapter_ids: vec!["chapter-1".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("阿宁在废城中开始信任队友".to_owned()),
            stage_id: Some("stage-1".to_owned()),
            stage_summary: Some("废城阶段推进到团队互信".to_owned()),
            is_new_stage: None,
            realized_changes: vec![RealizedChangeLink {
                change_id: "trait-1".to_owned(),
                segment_id: "seg-1".to_owned(),
            }],
            foreshadowing_updates: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();

    assert!(!report.paused);
    assert_eq!(kb.planner_issues("chapter-1").unwrap().len(), 0);
    assert!(kb
        .confirmations(Some(ariadne::rag::ConfirmationState::AutoAudited))
        .unwrap()
        .iter()
        .all(|item| item.state == ariadne::rag::ConfirmationState::AutoAudited));
}

/// 验证 Summarizer 章节总结完整复刻故事段、事件、章节总结、阶段概括四步及派生索引。
#[test]
fn summary_pipeline_links_segments_foreshadowing_stage_and_rerun_plan() {
    let kb = MemoryWritingKnowledgeBase::new();
    kb.upsert_registered_change(RegisteredChange {
        change_id: "trait-1".to_owned(),
        function: RegisterFunction::CharacterTrait,
        status: RegisteredChangeStatus::Planned,
        content: RegisterContent::CharacterTrait(ariadne::rag::CharacterTraitContent {
            character: "阿宁".to_owned(),
            trait_name: "戒备心".to_owned(),
            from_value: None,
            to_value: "主动交出钥匙".to_owned(),
            reason: "废城入口的共同选择".to_owned(),
        }),
        linked_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();
    kb.upsert_foreshadowing(ForeshadowingRecord {
        foreshadowing_id: "f-1".to_owned(),
        title: "旧钥匙".to_owned(),
        description: "门缝里的旧钥匙".to_owned(),
        status: ForeshadowingStatus::Planned,
        planted_segment_ids: Vec::new(),
        recovered_segment_ids: Vec::new(),
        metadata: Value::Null,
    })
    .unwrap();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::auto_audit_default(),
        AutoModeState {
            enabled: true,
            preauthorized_budget_usd: None,
        },
    );

    let report = executor
        .apply_draft(SummaryPipelineDraft {
            chapter_id: "chapter-1".to_owned(),
            segments: vec![StorySegment {
                segment_id: "seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "chapter-1".to_owned(),
                summary: "阿宁在废城入口交出旧钥匙".to_owned(),
                source: source_span(),
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "event-1".to_owned(),
                summary: "废城入口事件进行中".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["seg-1".to_owned()],
                chapter_ids: vec!["chapter-1".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("本章推进废城入口事件，并体现阿宁交出钥匙。".to_owned()),
            stage_id: Some("stage-1".to_owned()),
            stage_summary: Some("当前阶段围绕废城入口和团队互信推进。".to_owned()),
            is_new_stage: None,
            realized_changes: vec![RealizedChangeLink {
                change_id: "trait-1".to_owned(),
                segment_id: "seg-1".to_owned(),
            }],
            foreshadowing_updates: vec![ForeshadowingUpdate {
                foreshadowing_id: "f-1".to_owned(),
                status: ForeshadowingStatus::Planted,
                segment_id: "seg-1".to_owned(),
            }],
            metadata: Value::Null,
        })
        .unwrap();

    assert!(!report.paused);
    assert_eq!(
        report.completed_steps,
        vec![
            SummaryPipelineStep::Segment,
            SummaryPipelineStep::Event,
            SummaryPipelineStep::Chapter,
            SummaryPipelineStep::Stage,
        ]
    );
    let index = kb.index_snapshot().unwrap();
    assert_eq!(index.chapter_segments["chapter-1"], vec!["seg-1"]);
    assert_eq!(index.segment_events["seg-1"], vec!["event-1"]);
    assert_eq!(index.segment_changes["seg-1"], vec!["trait-1"]);
    assert_eq!(index.segment_foreshadowing["seg-1"], vec!["f-1"]);
    assert_eq!(index.stage_chapters["stage-1"], vec!["chapter-1"]);
    assert_eq!(index.chapter_stage["chapter-1"], "stage-1");
    assert_eq!(
        kb.foreshadowing("f-1").unwrap().unwrap().status,
        ForeshadowingStatus::Planted
    );

    let rerun = executor
        .plan_rerun_after_patch_write_back("chapter-1", "writer correction patch applied")
        .unwrap();
    assert_eq!(rerun.start_step, SummaryPipelineStep::Segment);
    assert_eq!(
        rerun.affected_steps,
        vec![
            SummaryPipelineStep::Segment,
            SummaryPipelineStep::Event,
            SummaryPipelineStep::Chapter,
            SummaryPipelineStep::Stage,
        ]
    );
}

/// 验证行号 patch 工具的作用域校验：Outliner 工具不能修改章节正文文档。
#[test]
fn line_patch_tool_rejects_cross_scope_document() {
    let kb = MemoryWritingKnowledgeBase::new();
    // 当前正文上下文是章节正文，但调用的是 outliner-insert-lines（要求全局总纲作用域）。
    let executor = WritingToolExecutor::with_document(
        &kb,
        WriterDocumentContext {
            document_id: "planning/chapters/chapter-1.md",
            base_version: Some("v1"),
            text: "第一行\n第二行",
            scope: WritingDocumentScope::ChapterBody,
        },
    );

    let error = executor
        .execute(
            &tool_context(),
            &ToolCall {
                tool_call_id: "insert-1".to_owned(),
                name: TOOL_OUTLINER_INSERT_LINES.to_owned(),
                arguments: json!({
                    "after_line": 1,
                    "text": "越权写入"
                }),
            },
        )
        .unwrap_err();

    // 作用域不匹配必须拒绝，而不是静默写入。
    assert!(error.to_string().contains("permission denied"));
}

/// 验证行号 patch 工具拒绝参数 document_id 偏离当前上下文文档。
#[test]
fn line_patch_tool_rejects_foreign_document_id() {
    let kb = MemoryWritingKnowledgeBase::new();
    let executor = WritingToolExecutor::with_document(
        &kb,
        WriterDocumentContext {
            document_id: "chapter-1-body.md",
            base_version: Some("v1"),
            text: "第一行\n第二行",
            scope: WritingDocumentScope::ChapterBody,
        },
    );

    let error = executor
        .execute(
            &tool_context(),
            &ToolCall {
                tool_call_id: "replace-1".to_owned(),
                name: TOOL_WRITER_REPLACE_LINES.to_owned(),
                arguments: json!({
                    "document_id": "another-chapter.md",
                    "start_line": 1,
                    "end_line": 1,
                    "text": "改到别的文件"
                }),
            },
        )
        .unwrap_err();

    assert!(error.to_string().contains("permission denied"));
}

/// 验证作用域匹配且 document_id 一致时行号 patch 正常生成。
#[test]
fn line_patch_tool_allows_matching_scope_and_document() {
    let kb = MemoryWritingKnowledgeBase::new();
    let executor = WritingToolExecutor::with_document(
        &kb,
        WriterDocumentContext {
            document_id: "planning/global.md",
            base_version: Some("v1"),
            text: "总纲第一行\n总纲第二行",
            scope: WritingDocumentScope::GlobalOutline,
        },
    );

    let output = executor
        .execute(
            &tool_context(),
            &ToolCall {
                tool_call_id: "insert-ok".to_owned(),
                name: TOOL_OUTLINER_INSERT_LINES.to_owned(),
                arguments: json!({
                    "after_line": 1,
                    "text": "新增总纲内容"
                }),
            },
        )
        .unwrap();

    assert!(output.value.get("hunks").is_some());
}

// ─── SqliteWritingKnowledgeStore 往返等价测试 ─────────────────────────────

#[cfg(test)]
mod store_contracts {
    use ariadne::contracts::AutoModeState;
    use ariadne::contracts::{SourceSpan, TextRange};
    use ariadne::rag::memory::MemoryWritingKnowledgeBase;
    use ariadne::rag::models::{
        ForeshadowingContent, ForeshadowingRecord, ForeshadowingStatus, RealizedChangeLink,
        RegisterContent, RegisterFunction, RegisteredChange, RegisteredChangeStatus, StoryEvent,
        StoryEventStatus, StorySegment, WritingConfirmationPolicy,
    };
    use ariadne::rag::pipeline::SummaryPipelineExecutor;
    use ariadne::rag::store::SqliteWritingKnowledgeStore;
    use ariadne::rag::SummaryPipelineDraft;
    use serde_json::{json, Value};

    fn source_span() -> SourceSpan {
        SourceSpan {
            document_id: "chapters/ch-1.md".to_owned(),
            range: TextRange { start: 0, end: 100 },
            version: None,
        }
    }

    fn make_draft() -> SummaryPipelineDraft {
        SummaryPipelineDraft {
            chapter_id: "ch-1".to_owned(),
            segments: vec![StorySegment {
                segment_id: "ch-1::seg-1".to_owned(),
                number: "1".to_owned(),
                chapter_id: "ch-1".to_owned(),
                summary: "主角进入废城".to_owned(),
                source: source_span(),
                metadata: Value::Null,
            }],
            events: vec![StoryEvent {
                event_id: "event-1".to_owned(),
                summary: "进城事件".to_owned(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["ch-1::seg-1".to_owned()],
                chapter_ids: vec!["ch-1".to_owned()],
                metadata: Value::Null,
            }],
            chapter_summary: Some("第一章总结".to_owned()),
            stage_id: Some("stage-1".to_owned()),
            stage_summary: Some("第一阶段开篇".to_owned()),
            is_new_stage: None,
            realized_changes: vec![],
            foreshadowing_updates: vec![],
            metadata: json!({ "generated_by": "test" }),
        }
    }

    /// C2：章节 SQLite delta 成功只替换本章，失败不污染库。
    #[test]
    fn chapter_sqlite_delta_replaces_only_chapter_and_is_fail_atomic() {
        use ariadne::rag::models::{StoryEvent, StoryEventStatus, StorySegment};
        use ariadne::rag::store::SqliteWritingKnowledgeStore;
        use serde_json::json;

        let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
        let kb = MemoryWritingKnowledgeBase::new();
        // 先写入两章
        kb.upsert_segment(StorySegment {
            segment_id: "ch-a::s1".into(),
            number: "1".into(),
            chapter_id: "ch-a".into(),
            summary: "A旧".into(),
            source: source_span(),
            metadata: Value::Null,
        })
        .unwrap();
        kb.upsert_segment(StorySegment {
            segment_id: "ch-b::s1".into(),
            number: "1".into(),
            chapter_id: "ch-b".into(),
            summary: "B保留".into(),
            source: source_span(),
            metadata: Value::Null,
        })
        .unwrap();
        kb.upsert_event(StoryEvent {
            event_id: "ev-a".into(),
            summary: "A事件".into(),
            status: StoryEventStatus::Ongoing,
            segment_ids: vec!["ch-a::s1".into()],
            chapter_ids: vec!["ch-a".into()],
            metadata: Value::Null,
        })
        .unwrap();
        store.save_knowledge(&kb).unwrap();

        // 本章换成新 segment
        let kb2 = store.load_knowledge().unwrap();
        kb2.replace_chapter_summary_entities(
            "ch-a",
            vec![StorySegment {
                segment_id: "ch-a::s2".into(),
                number: "1".into(),
                chapter_id: "ch-a".into(),
                summary: "A新".into(),
                source: source_span(),
                metadata: Value::Null,
            }],
            vec![StoryEvent {
                event_id: "ev-a2".into(),
                summary: "A新事件".into(),
                status: StoryEventStatus::Ongoing,
                segment_ids: vec!["ch-a::s2".into()],
                chapter_ids: vec!["ch-a".into()],
                metadata: Value::Null,
            }],
        )
        .unwrap();
        kb2.upsert_chapter_summary("ch-a", "章A总结").unwrap();
        store
            .save_chapter_knowledge_with_operation(
                &kb2,
                "ch-a",
                "op-chapter-a",
                "hash-a",
                &json!({"ok": true}),
                &ariadne::contracts::CancellationToken::new(),
            )
            .unwrap();

        let reloaded = store.load_knowledge().unwrap();
        let segs = reloaded.all_segments().unwrap();
        assert!(segs
            .iter()
            .any(|s| s.segment_id == "ch-a::s2" && s.summary == "A新"));
        assert!(!segs.iter().any(|s| s.segment_id == "ch-a::s1"));
        assert!(
            segs.iter()
                .any(|s| s.segment_id == "ch-b::s1" && s.summary == "B保留"),
            "foreign chapter must survive chapter-scoped save"
        );

        // fail-after 不得留下半写入（foreign 章仍在；本章不半成功）
        let before = store.load_knowledge().unwrap();
        let before_a: Vec<_> = before
            .all_segments()
            .unwrap()
            .into_iter()
            .filter(|s| s.chapter_id == "ch-a")
            .map(|s| s.segment_id)
            .collect();
        let kb3 = store.load_knowledge().unwrap();
        kb3.replace_chapter_summary_entities(
            "ch-a",
            vec![StorySegment {
                segment_id: "ch-a::s3".into(),
                number: "1".into(),
                chapter_id: "ch-a".into(),
                summary: "应回滚".into(),
                source: source_span(),
                metadata: Value::Null,
            }],
            vec![],
        )
        .unwrap();
        let err = store
            .save_chapter_knowledge_with_operation_fail_after(
                &kb3,
                "ch-a",
                "op-fail",
                "hash-fail",
                &json!({"ok": false}),
                2,
            )
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("injected knowledge chapter save failure"));
        let after = store.load_knowledge().unwrap();
        let after_a: Vec<_> = after
            .all_segments()
            .unwrap()
            .into_iter()
            .filter(|s| s.chapter_id == "ch-a")
            .map(|s| s.segment_id)
            .collect();
        assert_eq!(
            before_a, after_a,
            "failed mid-write must leave chapter unchanged"
        );
        assert!(after
            .all_segments()
            .unwrap()
            .iter()
            .any(|s| s.segment_id == "ch-b::s1"));
    }

    /// C2：生产 Summarizer 只装配章节关系闭包；无关章不进入工作集，
    /// 但跨章事件、Planner 变化与伏笔必须在同一章节事务中完整保留/写回。
    #[test]
    fn summary_working_set_is_chapter_scoped_and_persists_related_updates() {
        let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
        let seed = MemoryWritingKnowledgeBase::new();
        for (segment_id, chapter_id, summary) in [
            ("ch-a::old", "ch-a", "A旧段"),
            ("ch-b::linked", "ch-b", "B跨章段"),
            ("ch-c::unrelated", "ch-c", "C无关段"),
        ] {
            seed.upsert_segment(StorySegment {
                segment_id: segment_id.to_owned(),
                number: "1".to_owned(),
                chapter_id: chapter_id.to_owned(),
                summary: summary.to_owned(),
                source: source_span(),
                metadata: Value::Null,
            })
            .unwrap();
        }
        seed.upsert_event(StoryEvent {
            event_id: "cross-event".to_owned(),
            summary: "跨章事件".to_owned(),
            status: StoryEventStatus::Ongoing,
            segment_ids: vec!["ch-a::old".to_owned(), "ch-b::linked".to_owned()],
            chapter_ids: vec!["ch-a".to_owned(), "ch-b".to_owned()],
            metadata: Value::Null,
        })
        .unwrap();
        seed.upsert_registered_change(RegisteredChange {
            change_id: "change-a".to_owned(),
            function: RegisterFunction::CharacterTrait,
            status: RegisteredChangeStatus::Planned,
            content: RegisterContent::CharacterTrait(ariadne::rag::models::CharacterTraitContent {
                character: "阿宁".to_owned(),
                trait_name: "信任".to_owned(),
                from_value: None,
                to_value: "交出钥匙".to_owned(),
                reason: "测试".to_owned(),
            }),
            linked_segment_ids: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();
        seed.upsert_foreshadowing(ForeshadowingRecord {
            foreshadowing_id: "fore-a".to_owned(),
            title: "钥匙".to_owned(),
            description: "旧钥匙".to_owned(),
            status: ForeshadowingStatus::Planned,
            planted_segment_ids: Vec::new(),
            recovered_segment_ids: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();
        store.save_knowledge(&seed).unwrap();

        let mut draft = make_draft();
        draft.chapter_id = "ch-a".to_owned();
        draft.segments[0].segment_id = "ch-a::new".to_owned();
        draft.segments[0].chapter_id = "ch-a".to_owned();
        draft.events[0].event_id = "cross-event".to_owned();
        draft.events[0].segment_ids = vec!["ch-a::new".to_owned()];
        draft.events[0].chapter_ids = vec!["ch-a".to_owned()];
        draft.realized_changes = vec![RealizedChangeLink {
            change_id: "change-a".to_owned(),
            segment_id: "ch-a::new".to_owned(),
        }];
        draft.foreshadowing_updates = vec![ariadne::rag::models::ForeshadowingUpdate {
            foreshadowing_id: "fore-a".to_owned(),
            status: ForeshadowingStatus::Planted,
            segment_id: "ch-a::new".to_owned(),
        }];

        let working = store
            .load_summary_working_set("ch-a", Some(&draft))
            .unwrap();
        let working_segments = working.all_segments().unwrap();
        assert!(working_segments
            .iter()
            .any(|segment| segment.segment_id == "ch-a::old"));
        assert!(working_segments
            .iter()
            .any(|segment| segment.segment_id == "ch-b::linked"));
        assert!(!working_segments
            .iter()
            .any(|segment| segment.segment_id == "ch-c::unrelated"));

        SummaryPipelineExecutor::new(
            &working,
            WritingConfirmationPolicy::auto_audit_default(),
            AutoModeState {
                enabled: true,
                preauthorized_budget_usd: None,
            },
        )
        .apply_draft(draft)
        .unwrap();
        store
            .save_chapter_knowledge_with_operation(
                &working,
                "ch-a",
                "op-scoped-a",
                "hash-scoped-a",
                &json!({"ok": true}),
                &ariadne::contracts::CancellationToken::new(),
            )
            .unwrap();

        let reloaded = store.load_knowledge().unwrap();
        assert!(reloaded
            .all_segments()
            .unwrap()
            .iter()
            .any(|segment| segment.segment_id == "ch-c::unrelated"));
        let cross = reloaded.event("cross-event").unwrap().unwrap();
        assert!(cross.chapter_ids.contains(&"ch-b".to_owned()));
        assert!(cross.segment_ids.contains(&"ch-b::linked".to_owned()));
        let change = reloaded.registered_change("change-a").unwrap().unwrap();
        assert_eq!(change.status, RegisteredChangeStatus::Realized);
        assert!(change.linked_segment_ids.contains(&"ch-a::new".to_owned()));
        let foreshadowing = reloaded.foreshadowing("fore-a").unwrap().unwrap();
        assert_eq!(foreshadowing.status, ForeshadowingStatus::Planted);
        assert!(foreshadowing
            .planted_segment_ids
            .contains(&"ch-a::new".to_owned()));
        assert!(store
            .load_operation_receipt("op-scoped-a", "hash-scoped-a")
            .unwrap()
            .is_some());
    }

    /// F21：事务中途失败时不得留下半写入确认项/总结。
    #[test]
    fn apply_draft_failed_mid_transaction_leaves_knowledge_unchanged() {
        let kb = MemoryWritingKnowledgeBase::new();
        // auto_audit 立即激活 segment 步，从而执行 foreshadowing 更新并触发失败
        let executor = SummaryPipelineExecutor::new(
            &kb,
            WritingConfirmationPolicy::auto_audit_default(),
            AutoModeState {
                enabled: true,
                preauthorized_budget_usd: None,
            },
        );
        // 引用不存在的 foreshadowing → 事务失败
        let mut draft = make_draft();
        draft.foreshadowing_updates = vec![ariadne::rag::models::ForeshadowingUpdate {
            foreshadowing_id: "missing-fore".to_owned(),
            status: ForeshadowingStatus::Planted,
            segment_id: "ch-1::seg-1".to_owned(),
        }];
        assert!(executor.apply_draft(draft).is_err());
        assert!(
            kb.confirmations(None).unwrap().is_empty(),
            "failed transaction must not enqueue confirmations"
        );
        assert!(
            kb.chapter_summary("ch-1").unwrap().is_none(),
            "failed transaction must not write chapter summary"
        );
        assert!(
            kb.all_segments().unwrap().is_empty(),
            "failed transaction must not replace chapter segments"
        );
    }

    #[test]
    fn store_roundtrip_preserves_all_entities() {
        let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
        let kb = MemoryWritingKnowledgeBase::new();

        // 填充数据（auto_audit 立即激活四步知识，便于往返断言）
        let executor = SummaryPipelineExecutor::new(
            &kb,
            WritingConfirmationPolicy::auto_audit_default(),
            AutoModeState {
                enabled: true,
                preauthorized_budget_usd: None,
            },
        );
        executor.apply_draft(make_draft()).unwrap();

        // 写入伏笔
        kb.upsert_foreshadowing(ForeshadowingRecord {
            foreshadowing_id: "fore-1".to_owned(),
            title: "神秘地图".to_owned(),
            description: "主角在废城发现残破地图".to_owned(),
            status: ForeshadowingStatus::Planted,
            planted_segment_ids: vec!["ch-1::seg-1".to_owned()],
            recovered_segment_ids: vec![],
            metadata: Value::Null,
        })
        .unwrap();

        // 写入注册项
        kb.apply_register_operation(
            RegisterFunction::Foreshadowing,
            ariadne::rag::models::RegisterOperation::New,
            Some(RegisterContent::Foreshadowing(ForeshadowingContent {
                title: "地图".to_owned(),
                description: "见伏笔".to_owned(),
                intended_payoff: "揭示地下城".to_owned(),
            })),
            None,
        )
        .unwrap();

        // 落库
        store.save_knowledge(&kb).unwrap();

        // 重新加载
        let kb2 = store.load_knowledge().unwrap();

        // 源实体往返等价
        assert_eq!(kb2.all_segments().unwrap().len(), 1, "故事段数量");
        assert_eq!(kb2.all_events().unwrap().len(), 1, "事件数量");
        assert_eq!(
            kb2.chapter_summary("ch-1").unwrap(),
            Some("第一章总结".to_owned()),
            "章节总结"
        );
        assert_eq!(
            kb2.stage_summary("stage-1").unwrap(),
            Some("第一阶段开篇".to_owned()),
            "阶段总结"
        );
        assert_eq!(kb2.all_foreshadowing().unwrap().len(), 1, "伏笔数量");
        assert_eq!(kb2.registered_changes().unwrap().len(), 1, "注册项数量");

        // 确认项 4 个（segment/event/chapter/stage）
        assert_eq!(kb2.confirmations(None).unwrap().len(), 4, "确认项数量");

        // 双向索引在重放后正确重建
        let idx = kb2.index_snapshot().unwrap();
        assert!(idx.chapter_segments.contains_key("ch-1"), "章节-故事段索引");
        assert!(
            idx.event_segments.contains_key("event-1"),
            "事件-故事段索引"
        );
    }

    #[test]
    fn store_incremental_save_upserts_correctly() {
        let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
        let kb = MemoryWritingKnowledgeBase::new();

        // 第一次保存
        let executor = SummaryPipelineExecutor::new(
            &kb,
            WritingConfirmationPolicy::auto_audit_default(),
            AutoModeState {
                enabled: true,
                preauthorized_budget_usd: None,
            },
        );
        executor.apply_draft(make_draft()).unwrap();
        store.save_knowledge(&kb).unwrap();

        // 追加阶段总结后再保存
        kb.upsert_stage_summary("stage-2", "第二阶段").unwrap();
        store.save_knowledge(&kb).unwrap();

        let kb2 = store.load_knowledge().unwrap();
        assert_eq!(
            kb2.stage_summary("stage-2").unwrap(),
            Some("第二阶段".to_owned()),
            "增量保存后阶段总结应存在"
        );
        // 原有实体未丢失
        assert_eq!(kb2.all_segments().unwrap().len(), 1);
    }

    #[test]
    fn knowledge_operation_receipt_is_atomic_idempotent_and_cancellable() {
        let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
        let kb = MemoryWritingKnowledgeBase::new();
        let executor = SummaryPipelineExecutor::new(
            &kb,
            WritingConfirmationPolicy::auto_audit_default(),
            AutoModeState {
                enabled: true,
                preauthorized_budget_usd: None,
            },
        );
        executor.apply_draft(make_draft()).unwrap();
        let response = json!({"outputs":{"chapter_id":"ch-1"}});
        let cancellation = ariadne::contracts::CancellationToken::new();

        store
            .save_knowledge_with_operation(
                &kb,
                "knowledge-op-1",
                "request-hash-1",
                &response,
                &cancellation,
            )
            .unwrap();
        let receipt = store
            .load_operation_receipt("knowledge-op-1", "request-hash-1")
            .unwrap()
            .unwrap();
        assert_eq!(receipt.response_json, response);
        assert_eq!(
            store
                .load_knowledge()
                .unwrap()
                .all_segments()
                .unwrap()
                .len(),
            1
        );

        store
            .save_knowledge_with_operation(
                &kb,
                "knowledge-op-1",
                "request-hash-1",
                &response,
                &cancellation,
            )
            .unwrap();
        assert!(store
            .save_knowledge_with_operation(
                &kb,
                "knowledge-op-1",
                "request-hash-1",
                &json!({"different":true}),
                &cancellation,
            )
            .is_err());
        assert!(store
            .load_operation_receipt("knowledge-op-1", "different-request")
            .is_err());

        let cancelled = ariadne::contracts::CancellationToken::new();
        cancelled.cancel();
        assert!(store
            .save_knowledge_with_operation(
                &kb,
                "knowledge-op-cancelled",
                "request-hash-cancelled",
                &json!({}),
                &cancelled,
            )
            .is_err());
        assert!(store
            .load_operation_receipt("knowledge-op-cancelled", "request-hash-cancelled")
            .unwrap()
            .is_none());
    }

    #[test]
    fn knowledge_receipt_failure_rolls_back_entities_confirmations_and_indexes() {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWritingKnowledgeStore::open(temp.path()).unwrap();
        let kb = MemoryWritingKnowledgeBase::new();
        SummaryPipelineExecutor::new(
            &kb,
            WritingConfirmationPolicy::normal_default(),
            AutoModeState::default(),
        )
        .apply_draft(make_draft())
        .unwrap();
        let connection = rusqlite::Connection::open(temp.path().join("metadata.db")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_knowledge_receipt
                 BEFORE INSERT ON knowledge_operations
                 BEGIN SELECT RAISE(ABORT, 'forced receipt failure'); END;",
            )
            .unwrap();

        assert!(store
            .save_knowledge_with_operation(
                &kb,
                "knowledge-op-fail",
                "request-hash-fail",
                &json!({"result":"must rollback"}),
                &ariadne::contracts::CancellationToken::new(),
            )
            .is_err());

        let loaded = store.load_knowledge().unwrap();
        assert!(loaded.all_segments().unwrap().is_empty());
        assert!(loaded.all_events().unwrap().is_empty());
        assert!(loaded.confirmations(None).unwrap().is_empty());
        assert!(store
            .load_operation_receipt("knowledge-op-fail", "request-hash-fail")
            .unwrap()
            .is_none());
    }
}

// ── F14 / F24 / F25 ──────────────────────────────────────────────────────────

fn f14_draft(chapter: &str, stage: &str) -> SummaryPipelineDraft {
    SummaryPipelineDraft {
        chapter_id: chapter.to_owned(),
        segments: vec![StorySegment {
            segment_id: format!("{chapter}::seg-1"),
            number: "1".to_owned(),
            chapter_id: chapter.to_owned(),
            summary: "段摘要".to_owned(),
            source: SourceSpan {
                document_id: "doc.md".to_owned(),
                range: TextRange { start: 0, end: 10 },
                version: None,
            },
            metadata: Value::Null,
        }],
        events: vec![StoryEvent {
            event_id: format!("{chapter}-event"),
            summary: "事件摘要".to_owned(),
            status: StoryEventStatus::Ongoing,
            segment_ids: vec![format!("{chapter}::seg-1")],
            chapter_ids: vec![chapter.to_owned()],
            metadata: Value::Null,
        }],
        chapter_summary: Some("章总结".to_owned()),
        stage_id: Some(stage.to_owned()),
        stage_summary: Some("阶段总结".to_owned()),
        is_new_stage: Some(true),
        realized_changes: vec![],
        foreshadowing_updates: vec![],
        metadata: Value::Null,
    }
}

/// F14：未确认步骤不得进入 active 知识；拒绝后仍不激活；批准后物化。
#[test]
fn f14_stepwise_gate_pending_not_active_reject_stays_inactive_approve_materializes() {
    let kb = MemoryWritingKnowledgeBase::new();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::normal_default(),
        AutoModeState::default(),
    );
    let report = executor
        .apply_draft(f14_draft("ch-f14", "stage-f14"))
        .unwrap();
    assert!(report.paused);
    assert!(report.completed_steps.is_empty());
    assert!(kb.chapter_summary("ch-f14").unwrap().is_none());
    assert!(kb.all_segments().unwrap().is_empty());
    assert!(kb.all_events().unwrap().is_empty());
    assert!(kb.stage_summary("stage-f14").unwrap().is_none());

    // 拒绝事件步：不得出现 active 事件
    let event_id = report
        .confirmation_ids
        .iter()
        .find(|id| id.ends_with("event-summary"))
        .unwrap();
    ariadne::rag::reject_confirmation(&kb, event_id).unwrap();
    assert!(kb.all_events().unwrap().is_empty());

    // 批准故事段与章节与阶段
    for id in &report.confirmation_ids {
        if id.ends_with("event-summary") {
            continue;
        }
        ariadne::rag::approve_confirmation(&kb, id).unwrap();
    }
    assert_eq!(kb.all_segments().unwrap().len(), 1);
    assert_eq!(
        kb.chapter_summary("ch-f14").unwrap(),
        Some("章总结".to_owned())
    );
    assert_eq!(
        kb.stage_summary("stage-f14").unwrap(),
        Some("阶段总结".to_owned())
    );
    // 被拒绝的事件步仍无 active 事件
    assert!(kb.all_events().unwrap().is_empty());
}

#[test]
fn f14_persistent_receipts_cover_all_four_summary_kinds_idempotently() {
    use ariadne::rag::ConfirmationState;

    let temp = tempfile::tempdir().unwrap();
    let kb = MemoryWritingKnowledgeBase::new();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::normal_default(),
        AutoModeState::default(),
    );
    let report = executor
        .apply_draft(f14_draft("ch-f14-persist", "stage-f14-persist"))
        .unwrap();
    let store = SqliteWritingKnowledgeStore::open(temp.path()).unwrap();
    store.save_knowledge(&kb).unwrap();

    for suffix in [
        "segment-summary",
        "event-summary",
        "chapter-summary",
        "stage-summary",
    ] {
        let confirmation_id = report
            .confirmation_ids
            .iter()
            .find(|confirmation_id| confirmation_id.ends_with(suffix))
            .unwrap();
        let operation_id = format!("f14-operation-{suffix}");
        let request_hash = format!("f14-request-{suffix}");
        let response = json!({
            "confirmation_id": confirmation_id,
            "decision": "approve",
        });
        assert!(store
            .resolve_confirmation_with_operation(
                confirmation_id,
                ConfirmationState::Approved,
                &operation_id,
                &request_hash,
                &response,
            )
            .unwrap());
        assert!(store
            .resolve_confirmation_with_operation(
                confirmation_id,
                ConfirmationState::Approved,
                &operation_id,
                &request_hash,
                &response,
            )
            .unwrap());
        assert_eq!(
            store
                .load_operation_receipt(&operation_id, &request_hash)
                .unwrap()
                .unwrap()
                .response_json,
            response
        );
    }

    let loaded = store.load_knowledge().unwrap();
    assert_eq!(loaded.all_segments().unwrap().len(), 1);
    assert_eq!(loaded.all_events().unwrap().len(), 1);
    assert_eq!(
        loaded.chapter_summary("ch-f14-persist").unwrap(),
        Some("章总结".to_owned())
    );
    assert_eq!(
        loaded.stage_summary("stage-f14-persist").unwrap(),
        Some("阶段总结".to_owned())
    );
    assert!(loaded
        .confirmations(None)
        .unwrap()
        .iter()
        .all(|item| item.state == ConfirmationState::Approved));
}

/// F25：is_new_stage=false 且未知 stage 拒绝；提议新阶段经确认后落库；附着已有阶段成功。
#[test]
fn f25_stage_identity_rejects_orphan_invent_and_accepts_proposal() {
    let kb = MemoryWritingKnowledgeBase::new();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::auto_audit_default(),
        AutoModeState {
            enabled: true,
            preauthorized_budget_usd: None,
        },
    );

    let mut orphan = f14_draft("ch-orphan", "stage-ghost");
    orphan.is_new_stage = Some(false);
    let err = executor.apply_draft(orphan).unwrap_err();
    assert!(
        err.to_string().contains("unknown stage_id") || err.to_string().contains("is_new_stage"),
        "unexpected: {err}"
    );

    // 新阶段提议（auto 立即激活）
    let mut propose = f14_draft("ch-new", "stage-born");
    propose.is_new_stage = Some(true);
    executor.apply_draft(propose).unwrap();
    assert!(kb.has_stage("stage-born").unwrap());

    // 附着已有阶段
    let mut attach = f14_draft("ch-attach", "stage-born");
    attach.is_new_stage = Some(false);
    executor.apply_draft(attach).unwrap();
    assert_eq!(
        kb.stage_summary("stage-born").unwrap(),
        Some("阶段总结".to_owned())
    );
}

/// F22：调用方在 draft.metadata 中伪造 revision_id 不得覆盖历史确认项。
#[test]
fn f22_forged_revision_id_does_not_overwrite_confirmation_history() {
    use ariadne::contracts::{AutoModeState, SourceSpan, TextRange};
    use ariadne::rag::{
        ConfirmationState, MemoryWritingKnowledgeBase, SummaryPipelineDraft,
        SummaryPipelineExecutor, WritingConfirmationPolicy,
    };

    let kb = MemoryWritingKnowledgeBase::new();
    let executor = SummaryPipelineExecutor::new(
        &kb,
        WritingConfirmationPolicy::auto_audit_default(),
        AutoModeState {
            enabled: true,
            preauthorized_budget_usd: None,
        },
    );
    let mut draft = SummaryPipelineDraft {
        chapter_id: "ch-f22".to_owned(),
        segments: vec![ariadne::rag::StorySegment {
            segment_id: "ch-f22::seg-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "ch-f22".to_owned(),
            summary: "段".to_owned(),
            source: SourceSpan {
                document_id: "d.md".to_owned(),
                range: TextRange { start: 0, end: 1 },
                version: None,
            },
            metadata: Value::Null,
        }],
        events: vec![ariadne::rag::StoryEvent {
            event_id: "e1".to_owned(),
            summary: "事".to_owned(),
            status: ariadne::rag::StoryEventStatus::Ongoing,
            segment_ids: vec!["ch-f22::seg-1".to_owned()],
            chapter_ids: vec!["ch-f22".to_owned()],
            metadata: Value::Null,
        }],
        chapter_summary: Some("章".to_owned()),
        stage_id: Some("st".to_owned()),
        stage_summary: Some("阶".to_owned()),
        is_new_stage: Some(true),
        realized_changes: vec![],
        foreshadowing_updates: vec![],
        metadata: json!({ "revision_id": "forged-same-rev" }),
    };
    let first = executor.apply_draft(draft.clone()).unwrap();
    draft.chapter_summary = Some("章-第二轮".to_owned());
    // Same forged revision_id must NOT collapse history.
    draft.metadata = json!({ "revision_id": "forged-same-rev" });
    let second = executor.apply_draft(draft).unwrap();
    assert_ne!(
        first.revision_id, second.revision_id,
        "pipeline must mint unique revisions ignoring forged metadata"
    );
    assert_ne!(first.revision_id, "forged-same-rev");
    assert_ne!(second.revision_id, "forged-same-rev");
    // 两轮 × 四步 = 8 条确认；不得因同 revision 覆盖成 4 条
    let confs = kb.confirmations(None).unwrap();
    assert_eq!(
        confs.len(),
        8,
        "forged duplicate revision_id must not overwrite prior confirmations"
    );
    assert!(confs
        .iter()
        .all(|c| matches!(c.state, ConfirmationState::AutoAudited)));
}

/// F11：分步 LLM 使用 stable operation_id，重入时命中 step receipt 不再次调用 provider。
#[test]
fn f11_summarizer_step_receipt_skips_provider_on_reentry() {
    use ariadne::contracts::{ProviderCapability, ProviderDefinition, ProviderType};
    use ariadne::costs::SqliteCostLedger;
    use ariadne::providers::{
        LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext,
        ProviderHealth,
    };
    use ariadne::rag::load_prompt_resources;
    use ariadne::rag::summarizer::{
        SummarizerConfig, SummarizerExecutor, SummarizerWorkflowOperationContext,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingLlm {
        calls: AtomicUsize,
    }
    impl Provider for CountingLlm {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "count".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "count".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for CountingLlm {
        fn complete(
            &self,
            _ctx: &ProviderCallContext,
            request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let step = request
                .metadata
                .get("summarizer_step")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let body = match step {
                "segments" => {
                    r#"{"segments":[{"number":"1","summary":"s","start_line":1,"end_line":2}]}"#
                }
                "events" => {
                    r#"{"events":[{"event_id":"e1","summary":"ev","status":"ongoing","segment_ids":["ch::seg-1"]}]}"#
                }
                "chapter" => r#"{"summary":"章节总结"}"#,
                "stage" => r#"{"stage_id":"stage-1","stage_summary":"阶段","is_new_stage":true}"#,
                _ => r#"{"summary":"x"}"#,
            };
            Ok(LlmResponse {
                message: LlmMessage::assistant(body),
                tool_calls: Vec::new(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                cost_usd: Some(0.01),
                raw: json!({}),
            })
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let provider = CountingLlm {
        calls: AtomicUsize::new(0),
    };
    let exec = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        SummarizerConfig {
            provider_id: "count".to_owned(),
            model_id: "m".to_owned(),
            chapter_document_id: "doc".to_owned(),
            run_id: Some("run-1".to_owned()),
            timeout_ms: 5_000,
            cancellation: ariadne::contracts::CancellationToken::new(),
            dispatch_authorization: Default::default(),
            prompt_template: None,
            generation_context: Default::default(),
            workflow_operation: Some(SummarizerWorkflowOperationContext {
                project_root: temp.path().to_path_buf(),
                workflow_id: ariadne::contracts::WorkflowId::from("wf-f11-step-receipt"),
                run_id: ariadne::contracts::RunId::from("run-1"),
                node_id: ariadne::contracts::NodeId::from("summarizer"),
                operation_id: "wf-op-f11-step-receipt".to_owned(),
                operation_attempt: 1,
                request_hash: "wf-request-f11-step-receipt".to_owned(),
            }),
        },
    );
    let draft1 = exec.summarize_chapter("ch", "line1\nline2").unwrap();
    assert_eq!(draft1.segments.len(), 1);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
    let draft2 = exec.summarize_chapter("ch", "line1\nline2").unwrap();
    assert_eq!(draft2.segments.len(), 1);
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        4,
        "second run must not call provider again (all four steps cached)"
    );
}

#[test]
fn f11_summarizer_stage_journal_replays_receipt_and_rejects_identity_drift() {
    use ariadne::rag::{SummarizerStageOperationStatus, SummarizerStagePreparation};

    let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
    let preparation = store
        .prepare_summarizer_stage_operation(
            "parent-op",
            "parent-op",
            1,
            "parent-hash",
            "segments",
            "segments-hash",
        )
        .unwrap();
    let SummarizerStagePreparation::Execute { operation_id } = preparation else {
        panic!("expected a new stage operation");
    };
    store
        .mark_summarizer_stage_dispatched(&operation_id)
        .unwrap();
    let response = json!({"message": "segments-response"});
    store
        .complete_summarizer_stage_operation(&operation_id, &response)
        .unwrap();

    assert_eq!(
        store
            .prepare_summarizer_stage_operation(
                "parent-op",
                "parent-op",
                1,
                "parent-hash",
                "segments",
                "segments-hash",
            )
            .unwrap(),
        SummarizerStagePreparation::Replay {
            operation_id: operation_id.clone(),
            response_json: response,
        }
    );
    assert!(store
        .prepare_summarizer_stage_operation(
            "parent-op",
            "parent-op",
            1,
            "changed-parent-hash",
            "segments",
            "segments-hash",
        )
        .unwrap_err()
        .to_string()
        .contains("parent operation identity mismatch"));
    assert!(store
        .prepare_summarizer_stage_operation(
            "parent-op",
            "parent-op",
            1,
            "parent-hash",
            "segments",
            "changed-segments-hash",
        )
        .unwrap_err()
        .to_string()
        .contains("request changed"));
    assert_eq!(
        store.list_summarizer_stage_operations("parent-op").unwrap()[0].status,
        SummarizerStageOperationStatus::Completed
    );
}

#[test]
fn f11_summarizer_stage_journal_does_not_redispatch_unknown_response() {
    use ariadne::contracts::{CoreError, ExternalDispatchOutcome, NodeId, RunId, WorkflowId};
    use ariadne::costs::SqliteCostLedger;
    use ariadne::providers::{
        LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext, ProviderHealth,
    };
    use ariadne::rag::summarizer::{
        SummarizerConfig, SummarizerExecutor, SummarizerWorkflowOperationContext,
    };
    use ariadne::rag::SummarizerStageOperationStatus;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct UnknownAfterDispatch {
        calls: AtomicUsize,
    }
    impl Provider for UnknownAfterDispatch {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "unknown-after-dispatch".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "unknown-after-dispatch".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for UnknownAfterDispatch {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            _request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(CoreError::ProviderRequest {
                service: "unknown-after-dispatch".to_owned(),
                outcome: ExternalDispatchOutcome::DispatchedUnknown,
                message: "response lost".to_owned(),
            })
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let provider = UnknownAfterDispatch {
        calls: AtomicUsize::new(0),
    };
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        SummarizerConfig {
            provider_id: "unknown-after-dispatch".to_owned(),
            model_id: "m".to_owned(),
            chapter_document_id: "doc".to_owned(),
            run_id: Some("run-1".to_owned()),
            timeout_ms: 5_000,
            cancellation: ariadne::contracts::CancellationToken::new(),
            dispatch_authorization: Default::default(),
            prompt_template: None,
            generation_context: Default::default(),
            workflow_operation: Some(SummarizerWorkflowOperationContext {
                project_root: temp.path().to_path_buf(),
                workflow_id: WorkflowId::from("wf-stage-unknown"),
                run_id: RunId::from("run-1"),
                node_id: NodeId::from("summarizer"),
                operation_id: "parent-stage-unknown".to_owned(),
                operation_attempt: 1,
                request_hash: "parent-stage-unknown-hash".to_owned(),
            }),
        },
    );

    assert!(executor
        .summarize_chapter("chapter", "line 1")
        .unwrap_err()
        .to_string()
        .contains("response lost"));
    assert!(executor
        .summarize_chapter("chapter", "line 1")
        .unwrap_err()
        .to_string()
        .contains("no durable response receipt"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let stages = SqliteWritingKnowledgeStore::open(temp.path())
        .unwrap()
        .list_summarizer_stage_operations("parent-stage-unknown")
        .unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0].status, SummarizerStageOperationStatus::InDoubt);
}

#[test]
fn f11_summarizer_response_receipt_precedes_cost_write_and_recovers_without_recall() {
    use ariadne::contracts::{NodeId, RunId, WorkflowId};
    use ariadne::costs::{CostLedger, CostQuery, SqliteCostLedger};
    use ariadne::providers::{
        LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext,
        ProviderHealth,
    };
    use ariadne::rag::summarizer::{
        SummarizerConfig, SummarizerExecutor, SummarizerWorkflowOperationContext,
    };
    use ariadne::rag::SummarizerStageOperationStatus;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StageResponses {
        calls: AtomicUsize,
    }
    impl Provider for StageResponses {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "stage-responses".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "stage-responses".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for StageResponses {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let step = request.metadata["summarizer_step"].as_str().unwrap();
            let text = match step {
                "segments" => {
                    r#"{"segments":[{"number":"1","summary":"s","start_line":1,"end_line":1}]}"#
                }
                "events" => {
                    r#"{"events":[{"event_id":"e","summary":"event","status":"ongoing","segment_ids":["chapter::seg-1"]}]}"#
                }
                "chapter" => r#"{"summary":"chapter summary"}"#,
                "stage" => {
                    r#"{"stage_id":"stage-1","stage_summary":"stage summary","is_new_stage":true}"#
                }
                other => panic!("unexpected summarizer step {other}"),
            };
            Ok(LlmResponse {
                message: LlmMessage::assistant(text),
                tool_calls: Vec::new(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                cost_usd: Some(0.01),
                raw: json!({"step": step}),
            })
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("costs.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER abort_first_stage_cost
             BEFORE INSERT ON cost_events
             BEGIN SELECT RAISE(ABORT, 'forced stage cost failure'); END;",
        )
        .unwrap();
    let provider = StageResponses {
        calls: AtomicUsize::new(0),
    };
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        SummarizerConfig {
            provider_id: "stage-responses".to_owned(),
            model_id: "m".to_owned(),
            chapter_document_id: "doc".to_owned(),
            run_id: Some("run-1".to_owned()),
            timeout_ms: 5_000,
            cancellation: ariadne::contracts::CancellationToken::new(),
            dispatch_authorization: Default::default(),
            prompt_template: None,
            generation_context: Default::default(),
            workflow_operation: Some(SummarizerWorkflowOperationContext {
                project_root: temp.path().to_path_buf(),
                workflow_id: WorkflowId::from("wf-stage-cost"),
                run_id: RunId::from("run-1"),
                node_id: NodeId::from("summarizer"),
                operation_id: "parent-stage-cost".to_owned(),
                operation_attempt: 1,
                request_hash: "parent-stage-cost-hash".to_owned(),
            }),
        },
    );

    assert!(executor
        .summarize_chapter("chapter", "line 1")
        .unwrap_err()
        .to_string()
        .contains("forced stage cost failure"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let first_stage = SqliteWritingKnowledgeStore::open(temp.path())
        .unwrap()
        .list_summarizer_stage_operations("parent-stage-cost")
        .unwrap()
        .remove(0);
    assert_eq!(
        first_stage.status,
        SummarizerStageOperationStatus::Completed
    );

    connection
        .execute_batch("DROP TRIGGER abort_first_stage_cost;")
        .unwrap();
    let draft = executor.summarize_chapter("chapter", "line 1").unwrap();
    assert_eq!(draft.segments.len(), 1);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
    assert_eq!(ledger.list_costs(&CostQuery::default()).unwrap().len(), 4);
}

#[test]
fn f11_summarizer_pre_cancel_aborts_prepared_stage_without_provider_call() {
    use ariadne::contracts::{NodeId, RunId, WorkflowId};
    use ariadne::costs::SqliteCostLedger;
    use ariadne::providers::{
        LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext, ProviderHealth,
    };
    use ariadne::rag::summarizer::{
        SummarizerConfig, SummarizerExecutor, SummarizerWorkflowOperationContext,
    };
    use ariadne::rag::SummarizerStageOperationStatus;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MustNotCall {
        calls: AtomicUsize,
    }
    impl Provider for MustNotCall {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "must-not-call".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "must-not-call".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for MustNotCall {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            _request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("cancelled summarizer must not call provider")
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let cancellation = ariadne::contracts::CancellationToken::new();
    cancellation.cancel();
    let provider = MustNotCall {
        calls: AtomicUsize::new(0),
    };
    let ledger = SqliteCostLedger::open(temp.path()).unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        SummarizerConfig {
            provider_id: "must-not-call".to_owned(),
            model_id: "m".to_owned(),
            chapter_document_id: "doc".to_owned(),
            run_id: Some("run-1".to_owned()),
            timeout_ms: 5_000,
            cancellation,
            dispatch_authorization: Default::default(),
            prompt_template: None,
            generation_context: Default::default(),
            workflow_operation: Some(SummarizerWorkflowOperationContext {
                project_root: temp.path().to_path_buf(),
                workflow_id: WorkflowId::from("wf-stage-cancel"),
                run_id: RunId::from("run-1"),
                node_id: NodeId::from("summarizer"),
                operation_id: "parent-stage-cancel".to_owned(),
                operation_attempt: 1,
                request_hash: "parent-stage-cancel-hash".to_owned(),
            }),
        },
    );

    assert_eq!(
        executor
            .summarize_chapter("chapter", "line 1")
            .unwrap_err()
            .external_dispatch_outcome(),
        Some(ariadne::contracts::ExternalDispatchOutcome::NotDispatched)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let stages = SqliteWritingKnowledgeStore::open(temp.path())
        .unwrap()
        .list_summarizer_stage_operations("parent-stage-cancel")
        .unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0].status, SummarizerStageOperationStatus::Aborted);
}

/// F24：非空 prompt_template 进入 summarizer 指令构造，并进入实际 LLM 请求。
#[test]
fn f24_author_prompt_template_is_included_in_step_instruction() {
    use ariadne::contracts::{ProviderCapability, ProviderDefinition, ProviderType};
    use ariadne::costs::SqliteCostLedger;
    use ariadne::providers::{
        ContentPart, LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider,
        ProviderCallContext, ProviderHealth,
    };
    use ariadne::rag::load_prompt_resources;
    use ariadne::rag::summarizer::{SummarizerConfig, SummarizerExecutor};
    use std::sync::Mutex;

    struct CaptureLlm {
        last: Mutex<String>,
    }
    impl Provider for CaptureLlm {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "cap".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "cap".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for CaptureLlm {
        fn complete(
            &self,
            _ctx: &ProviderCallContext,
            request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            let text = request
                .messages
                .iter()
                .flat_map(|m| m.content.iter())
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            *self.last.lock().unwrap() = text;
            Ok(LlmResponse {
                message: LlmMessage::assistant(
                    r#"{"segments":[{"number":"1","summary":"s","start_line":1,"end_line":2}]}"#,
                ),
                tool_calls: Vec::new(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                cost_usd: None,
                raw: json!({}),
            })
        }
    }

    let marker = "AUTHOR_TEMPLATE_MARKER_F24_XYZ";
    let prompts = load_prompt_resources().unwrap();
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let provider = CaptureLlm {
        last: Mutex::new(String::new()),
    };
    let exec = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        SummarizerConfig {
            provider_id: "cap".to_owned(),
            model_id: "m".to_owned(),
            chapter_document_id: "doc".to_owned(),
            run_id: None,
            timeout_ms: 5_000,
            cancellation: ariadne::contracts::CancellationToken::new(),
            dispatch_authorization: Default::default(),
            prompt_template: Some(marker.to_owned()),
            generation_context: Default::default(),
            workflow_operation: None,
        },
    );
    let built = exec
        .build_step_instruction_for_test("summarizer.segments", "BODY")
        .unwrap();
    assert!(
        built.contains(marker),
        "template missing from instruction: {built}"
    );
    // 真实 LLM 路径（仅完成 step1 也会捕获 user instruction）
    let _ = exec.summarize_chapter("ch", "line1\nline2");
    let captured = provider.last.lock().unwrap().clone();
    assert!(
        captured.contains(marker),
        "template missing from LLM request: {captured}"
    );
}

/// F15/F16：四步请求消费历史上下文，章节输出真实变化链接，SourceSpan 使用
/// UTF-8 byte offset + 正文版本，并完整覆盖正文。
#[test]
fn f15_f16_summarizer_uses_history_links_changes_and_utf8_source_spans() {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use ariadne::contracts::{
        content_version_for_bytes, ProviderCapability, ProviderDefinition, ProviderType,
    };
    use ariadne::costs::SqliteCostLedger;
    use ariadne::providers::{
        ContentPart, LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider,
        ProviderCallContext, ProviderHealth,
    };
    use ariadne::rag::models::{CharacterTraitContent, SummaryStageContext};
    use ariadne::rag::summarizer::{SummarizerConfig, SummarizerExecutor};
    use ariadne::rag::SummaryGenerationContext;

    struct ContextProvider {
        requests: Mutex<Vec<LlmRequest>>,
    }
    impl Provider for ContextProvider {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "context-provider".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "context-provider".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for ContextProvider {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            let step = request.metadata["summarizer_step"]
                .as_str()
                .unwrap()
                .to_owned();
            self.requests.lock().unwrap().push(request);
            let body = match step.as_str() {
                "segments" => {
                    r#"{"segments":[{"number":"1","summary":"前两行","start_line":1,"end_line":2},{"number":"2","summary":"末行","start_line":3,"end_line":3}]}"#
                }
                "events" => {
                    r#"{"events":[{"event_id":"event-old","summary":"旧事件在本章推进","status":"completed","segment_ids":["chapter::seg-1","chapter::seg-2"]}]}"#
                }
                "chapter" => {
                    r#"{"summary":"本章完成交接并种下钥匙伏笔","realized_changes":[{"change_id":"trait-1","segment_id":"chapter::seg-2"}],"foreshadowing_updates":[{"foreshadowing_id":"fore-1","status":"planted","segment_id":"chapter::seg-1"}]}"#
                }
                "stage" => {
                    r#"{"stage_id":"stage-old","stage_summary":"旧阶段在本章完成","is_new_stage":false}"#
                }
                other => panic!("unexpected summarizer step {other}"),
            };
            Ok(LlmResponse {
                message: LlmMessage::assistant(body),
                tool_calls: Vec::new(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                cost_usd: None,
                raw: Value::Null,
            })
        }
    }

    let context = SummaryGenerationContext {
        existing_events: vec![StoryEvent {
            event_id: "event-old".to_owned(),
            summary: "跨章旧事件".to_owned(),
            status: StoryEventStatus::Ongoing,
            segment_ids: vec!["previous::seg-1".to_owned()],
            chapter_ids: vec!["previous".to_owned()],
            metadata: Value::Null,
        }],
        planned_changes: vec![RegisteredChange {
            change_id: "trait-1".to_owned(),
            function: RegisterFunction::CharacterTrait,
            status: RegisteredChangeStatus::Planned,
            content: RegisterContent::CharacterTrait(CharacterTraitContent {
                character: "阿宁".to_owned(),
                trait_name: "信任".to_owned(),
                from_value: Some("戒备".to_owned()),
                to_value: "交出钥匙".to_owned(),
                reason: "关系推进".to_owned(),
            }),
            linked_segment_ids: Vec::new(),
            metadata: Value::Null,
        }],
        foreshadowing: vec![ForeshadowingRecord {
            foreshadowing_id: "fore-1".to_owned(),
            title: "旧钥匙".to_owned(),
            description: "将在后续回收".to_owned(),
            status: ForeshadowingStatus::Planned,
            planted_segment_ids: Vec::new(),
            recovered_segment_ids: Vec::new(),
            metadata: Value::Null,
        }],
        stages: vec![SummaryStageContext {
            stage_id: "stage-old".to_owned(),
            stage_summary: Some("旧阶段总结".to_owned()),
            chapter_summaries: BTreeMap::from([(
                "previous".to_owned(),
                "上一章正式总结".to_owned(),
            )]),
        }],
        current_stage_id: Some("stage-old".to_owned()),
    };
    let prompts = load_prompt_resources().unwrap();
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let provider = ContextProvider {
        requests: Mutex::new(Vec::new()),
    };
    let executor = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        SummarizerConfig {
            provider_id: "context-provider".to_owned(),
            model_id: "m".to_owned(),
            chapter_document_id: "documents/chapter.md".to_owned(),
            run_id: None,
            timeout_ms: 5_000,
            cancellation: ariadne::contracts::CancellationToken::new(),
            dispatch_authorization: Default::default(),
            prompt_template: None,
            generation_context: context,
            workflow_operation: None,
        },
    );
    let chapter_text = "甲\n乙\n丙";
    let draft = executor.summarize_chapter("chapter", chapter_text).unwrap();

    assert_eq!(draft.segments.len(), 2);
    assert_eq!(
        draft.segments[0].source.range,
        TextRange {
            start: 0,
            end: "甲\n乙\n".len() as u64,
        }
    );
    assert_eq!(
        draft.segments[1].source.range,
        TextRange {
            start: "甲\n乙\n".len() as u64,
            end: chapter_text.len() as u64,
        }
    );
    let expected_version = content_version_for_bytes(chapter_text.as_bytes());
    assert_eq!(
        draft.segments[0].source.version.as_deref(),
        Some(expected_version.as_str())
    );
    assert_eq!(draft.realized_changes[0].change_id, "trait-1");
    assert_eq!(
        draft.foreshadowing_updates[0].status,
        ForeshadowingStatus::Planted
    );

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    let request_text = |step: &str| {
        requests
            .iter()
            .find(|request| request.metadata["summarizer_step"] == step)
            .unwrap()
            .messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(request_text("events").contains("跨章旧事件"));
    assert!(request_text("chapter").contains("trait-1"));
    assert!(request_text("chapter").contains("旧钥匙"));
    assert!(request_text("stage").contains("上一章正式总结"));
    assert!(request_text("stage").contains(chapter_text));
}

/// F16：分段出现空洞时必须在第一步后阻断，不能继续消费后续 LLM 调用。
#[test]
fn f16_summarizer_rejects_segment_gaps_before_later_steps() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ariadne::contracts::{ProviderCapability, ProviderDefinition, ProviderType};
    use ariadne::costs::SqliteCostLedger;
    use ariadne::providers::{
        LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext,
        ProviderHealth,
    };
    use ariadne::rag::summarizer::{SummarizerConfig, SummarizerExecutor};

    struct GapProvider(AtomicUsize);
    impl Provider for GapProvider {
        fn definition(&self) -> ProviderDefinition {
            ProviderDefinition {
                provider_id: "gap".to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "gap".to_owned(),
                capabilities: vec![ProviderCapability::Llm],
                config_schema: Value::Null,
            }
        }
        fn health_check(&self) -> ariadne::contracts::CoreResult<ProviderHealth> {
            Ok(ProviderHealth::Healthy)
        }
    }
    impl LlmProvider for GapProvider {
        fn complete(
            &self,
            _context: &ProviderCallContext,
            _request: LlmRequest,
        ) -> ariadne::contracts::CoreResult<LlmResponse> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(LlmResponse {
                message: LlmMessage::assistant(
                    r#"{"segments":[{"number":"1","summary":"首行","start_line":1,"end_line":1},{"number":"2","summary":"末行","start_line":3,"end_line":3}]}"#,
                ),
                tool_calls: Vec::new(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                cost_usd: None,
                raw: Value::Null,
            })
        }
    }

    let provider = GapProvider(AtomicUsize::new(0));
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        SummarizerConfig {
            provider_id: "gap".to_owned(),
            model_id: "m".to_owned(),
            chapter_document_id: "doc".to_owned(),
            run_id: None,
            timeout_ms: 5_000,
            cancellation: ariadne::contracts::CancellationToken::new(),
            dispatch_authorization: Default::default(),
            prompt_template: None,
            generation_context: Default::default(),
            workflow_operation: None,
        },
    );

    let error = executor
        .summarize_chapter("chapter", "第一行\n第二行\n第三行")
        .unwrap_err();
    assert!(error.to_string().contains("gap-free"));
    assert_eq!(provider.0.load(Ordering::SeqCst), 1);
}

/// F15/F18：持久化层以固定批量查询构造生成上下文，并保留正式阶段关系。
#[test]
fn f15_generation_context_loads_events_changes_foreshadowing_and_stage_history() {
    use ariadne::rag::models::CharacterTraitContent;

    let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: "chapter-old::seg-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-old".to_owned(),
            summary: "旧段".to_owned(),
            source: source_span(),
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_event(StoryEvent {
            event_id: "event-cross".to_owned(),
            summary: "跨章事件".to_owned(),
            status: StoryEventStatus::Ongoing,
            segment_ids: vec!["chapter-old::seg-1".to_owned()],
            chapter_ids: vec!["chapter-old".to_owned()],
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_registered_change(RegisteredChange {
            change_id: "change-context".to_owned(),
            function: RegisterFunction::CharacterTrait,
            status: RegisteredChangeStatus::Planned,
            content: RegisterContent::CharacterTrait(CharacterTraitContent {
                character: "阿宁".to_owned(),
                trait_name: "勇气".to_owned(),
                from_value: Some("退缩".to_owned()),
                to_value: "行动".to_owned(),
                reason: "计划".to_owned(),
            }),
            linked_segment_ids: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_foreshadowing(ForeshadowingRecord {
            foreshadowing_id: "fore-context".to_owned(),
            title: "钥匙".to_owned(),
            description: "后续回收".to_owned(),
            status: ForeshadowingStatus::Planned,
            planted_segment_ids: Vec::new(),
            recovered_segment_ids: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_chapter_summary("chapter-old", "旧章总结")
        .unwrap();
    knowledge
        .upsert_stage_summary("stage-context", "旧阶段总结")
        .unwrap();
    knowledge
        .link_chapter_stage("chapter-old", "stage-context")
        .unwrap();
    store.save_knowledge(&knowledge).unwrap();

    let context = store
        .load_summary_generation_context("chapter-old")
        .unwrap();
    assert_eq!(context.existing_events[0].event_id, "event-cross");
    assert_eq!(context.planned_changes[0].change_id, "change-context");
    assert_eq!(context.foreshadowing[0].foreshadowing_id, "fore-context");
    assert_eq!(context.current_stage_id.as_deref(), Some("stage-context"));
    assert_eq!(
        context.stages[0]
            .chapter_summaries
            .get("chapter-old")
            .map(String::as_str),
        Some("旧章总结")
    );
}

/// F19：作品页从 metadata.db 读取唯一正式投影，并把确认历史与 active 知识分开。
#[test]
fn f19_chapter_summary_view_projects_formal_knowledge_and_confirmation_history() {
    let store = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
    let knowledge = MemoryWritingKnowledgeBase::new();
    for segment in [
        StorySegment {
            segment_id: "chapter-view::seg-2".to_owned(),
            number: "2".to_owned(),
            chapter_id: "chapter-view".to_owned(),
            summary: "第二段".to_owned(),
            source: SourceSpan {
                document_id: "documents/chapter-view.md".to_owned(),
                range: TextRange { start: 3, end: 7 },
                version: Some("v1".to_owned()),
            },
            metadata: Value::Null,
        },
        StorySegment {
            segment_id: "chapter-view::seg-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-view".to_owned(),
            summary: "第一段".to_owned(),
            source: SourceSpan {
                document_id: "documents/chapter-view.md".to_owned(),
                range: TextRange { start: 0, end: 3 },
                version: Some("v1".to_owned()),
            },
            metadata: Value::Null,
        },
    ] {
        knowledge.upsert_segment(segment).unwrap();
    }
    knowledge
        .upsert_event(StoryEvent {
            event_id: "event-view".to_owned(),
            summary: "跨越两段的事件".to_owned(),
            status: StoryEventStatus::Ongoing,
            segment_ids: vec![
                "chapter-view::seg-1".to_owned(),
                "chapter-view::seg-2".to_owned(),
            ],
            chapter_ids: vec!["chapter-view".to_owned()],
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_registered_change(RegisteredChange {
            change_id: "change-view".to_owned(),
            function: RegisterFunction::CharacterTrait,
            status: RegisteredChangeStatus::Realized,
            content: RegisterContent::CharacterTrait(CharacterTraitContent {
                character: "阿青".to_owned(),
                trait_name: "勇气".to_owned(),
                from_value: Some("犹疑".to_owned()),
                to_value: "坚定".to_owned(),
                reason: "作出选择".to_owned(),
            }),
            linked_segment_ids: vec!["chapter-view::seg-2".to_owned()],
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_foreshadowing(ForeshadowingRecord {
            foreshadowing_id: "fore-view".to_owned(),
            title: "旧钥匙".to_owned(),
            description: "本章完成回收".to_owned(),
            status: ForeshadowingStatus::Recovered,
            planted_segment_ids: vec!["chapter-view::seg-1".to_owned()],
            recovered_segment_ids: vec!["chapter-view::seg-2".to_owned()],
            metadata: Value::Null,
        })
        .unwrap();
    knowledge
        .upsert_chapter_summary("chapter-view", "正式章节总结")
        .unwrap();
    knowledge
        .upsert_stage_summary("stage-official", "正式阶段总结")
        .unwrap();
    knowledge
        .link_chapter_stage("chapter-view", "stage-official")
        .unwrap();

    for (index, (kind, state)) in [
        (ConfirmationKind::SegmentSummary, ConfirmationState::Pending),
        (ConfirmationKind::EventSummary, ConfirmationState::Skipped),
        (
            ConfirmationKind::ChapterSummary,
            ConfirmationState::AutoAudited,
        ),
        (ConfirmationKind::StageSummary, ConfirmationState::Approved),
    ]
    .into_iter()
    .enumerate()
    {
        knowledge
            .upsert_confirmation(ConfirmationItem {
                confirmation_id: format!("confirm-{index}"),
                kind,
                state,
                prompt_key: "confirmation.test".to_owned(),
                metadata: json!({
                    "chapter_id": "chapter-view",
                    "revision_id": format!("rev-{index}"),
                    "pending_payload": { "must_not_be_projected": true },
                }),
            })
            .unwrap();
    }
    store.save_knowledge(&knowledge).unwrap();

    let view = store.load_chapter_summary_view("chapter-view").unwrap();
    assert_eq!(view.chapter_summary.as_deref(), Some("正式章节总结"));
    let stage = view.stage.unwrap();
    assert_eq!(stage.stage_id, "stage-official");
    assert_eq!(stage.summary.as_deref(), Some("正式阶段总结"));
    assert_eq!(stage.chapter_ids, vec!["chapter-view"]);
    assert_eq!(
        view.segments
            .iter()
            .map(|segment| segment.number.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "2"]
    );
    assert_eq!(view.events[0].event_id, "event-view");
    assert_eq!(view.realized_changes[0].change_id, "change-view");
    assert_eq!(view.foreshadowing[0].foreshadowing_id, "fore-view");
    assert_eq!(view.confirmations.len(), 4);
    assert_eq!(view.confirmations[0].state, ConfirmationState::Pending);
    assert_eq!(view.confirmations[1].state, ConfirmationState::Skipped);
    assert_eq!(view.confirmations[2].state, ConfirmationState::AutoAudited);
    assert_eq!(view.confirmations[3].state, ConfirmationState::Approved);
}

/// F19：SourceSpan 版本和关系闭包损坏时 fail loud，不把坏数据伪装成空总结。
#[test]
fn f19_chapter_summary_view_rejects_missing_source_version_and_dangling_relation() {
    let missing_version = SqliteWritingKnowledgeStore::open_in_memory().unwrap();
    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: "chapter-bad::seg-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-bad".to_owned(),
            summary: "缺版本".to_owned(),
            source: SourceSpan {
                document_id: "documents/chapter-bad.md".to_owned(),
                range: TextRange { start: 0, end: 3 },
                version: None,
            },
            metadata: Value::Null,
        })
        .unwrap();
    missing_version.save_knowledge(&knowledge).unwrap();
    let error = missing_version
        .load_chapter_summary_view("chapter-bad")
        .unwrap_err();
    assert!(error.to_string().contains("invalid source span"));

    let temp = tempfile::tempdir().unwrap();
    let store = SqliteWritingKnowledgeStore::open(temp.path()).unwrap();
    let valid = MemoryWritingKnowledgeBase::new();
    valid
        .upsert_segment(StorySegment {
            segment_id: "chapter-bad::seg-1".to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-bad".to_owned(),
            summary: "有效来源".to_owned(),
            source: SourceSpan {
                document_id: "documents/chapter-bad.md".to_owned(),
                range: TextRange { start: 0, end: 3 },
                version: Some("v1".to_owned()),
            },
            metadata: Value::Null,
        })
        .unwrap();
    store.save_knowledge(&valid).unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(temp.path().join("metadata.db")).unwrap();
    connection
        .execute(
            "INSERT INTO event_chapter_links(event_id, chapter_id) VALUES (?1, ?2)",
            ("missing-event", "chapter-bad"),
        )
        .unwrap();
    drop(connection);

    let reopened = SqliteWritingKnowledgeStore::open(temp.path()).unwrap();
    let error = reopened
        .load_chapter_summary_view("chapter-bad")
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("event relation references missing entity"));
}
