use ariadne::core::{
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
    search_response_to_writing_response, tool_definitions_for_agent, ForeshadowingContent,
    ForeshadowingRecord, ForeshadowingStatus, MemoryWritingKnowledgeBase, NodePromptConfig,
    PatchSession, PromptTemplateContext, RealizedChangeLink, RegisterContent, RegisterFunction,
    RegisterOperation, RegisteredChange, RegisteredChangeStatus, StoryEvent, StoryEventStatus,
    StorySegment, SummaryPipelineDraft, SummaryPipelineExecutor, WriterDocumentContext,
    WriterInsertLines, WriterReplaceLines, WritingAgentKind, WritingConfirmationPolicy,
    WritingContextAssembler, WritingContextRequest, WritingNodeDefinition, WritingToolExecutor,
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
    ) -> ariadne::core::CoreResult<SearchProviderResponse> {
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

    assert_eq!(nodes.len(), 8);
    assert_eq!(nodes[0].agent, WritingAgentKind::Outliner);
    assert_eq!(nodes[1].agent, WritingAgentKind::Designer);
    assert_eq!(nodes[2].agent, WritingAgentKind::Planner);
    assert_eq!(nodes[3].agent, WritingAgentKind::Detail);
    assert_eq!(nodes[4].agent, WritingAgentKind::Writer);
    assert_eq!(nodes[5].agent, WritingAgentKind::Critic);
    assert_eq!(nodes[6].agent, WritingAgentKind::Prudent);
    assert_eq!(nodes[7].agent, WritingAgentKind::Summarizer);
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
    assert!(nodes[7].tool_names.is_empty());
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
            realized_changes: Vec::new(),
            metadata: Value::Null,
        })
        .unwrap();

    assert!(report.paused);
    assert_eq!(report.completed_steps.len(), 4);
    assert_eq!(
        kb.chapter_summary("chapter-1").unwrap(),
        Some("章节总结".to_owned())
    );
    assert_eq!(kb.confirmations(None).unwrap().len(), 4);
    assert_eq!(kb.planner_issues("chapter-1").unwrap().len(), 1);
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
            events: Vec::new(),
            chapter_summary: None,
            stage_id: None,
            stage_summary: None,
            realized_changes: vec![RealizedChangeLink {
                change_id: "trait-1".to_owned(),
                segment_id: "seg-1".to_owned(),
            }],
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
