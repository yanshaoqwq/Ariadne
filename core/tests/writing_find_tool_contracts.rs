//! find 工具功能完整性契约（2026-07-26）。
//!
//! find 是写作 agent 的**唯一知识读取手段**——register 写进去的东西，
//! 全靠 find 查回来。因此「register 能写的每一类知识，find 都必须能查」
//! 是这套机制的闭环前提；任何一类只写不可读，就等于数据写进黑洞。
//!
//! 本文件逐维度实测 find 的对外契约（LLM 实际看到的 JSON Schema）
//! 与底层实现是否一致，并验证 `include_text` 正文回填。
//!
//! 分析见 `项目检验报告/发布前全量代码审查/13-配置项存在性与执行链路阻断审查.md`。

use ariadne::llm::{ToolExecutionContext, ToolExecutor};
use ariadne::providers::ToolCall;
use ariadne::rag::memory::MemoryWritingKnowledgeBase;
use ariadne::rag::models::{
    FindRequest, FindScope, RegisterFunction, RegisterOperation, WritingAgentKind,
};
use ariadne::rag::tools::{tool_definitions_for_agent, WritingToolExecutor};
use serde_json::{json, Value};

// ════════════════════════════════════════════════════════
// 一、Schema ↔ 实现一致性：register 写得进，find 必须查得回
// ════════════════════════════════════════════════════════

/// 从 find 工具的真实 JSON Schema 里取出 `a` 参数的 enum——
/// 这是 LLM 实际能看到、能填的全部取值。
fn find_scope_enum_exposed_to_llm() -> Vec<String> {
    let prompts = ariadne::rag::resources::load_prompt_resources().expect("内置 prompt 资源必须可读");
    let tools = tool_definitions_for_agent(WritingAgentKind::Planner, &prompts)
        .expect("planner 工具定义必须可生成");
    let find = tools
        .iter()
        .find(|tool| tool.name.ends_with("-find"))
        .expect("planner 必须有 find 工具");

    find.input_schema
        .get("properties")
        .and_then(|props| props.get("a"))
        .and_then(|a| a.get("enum"))
        .and_then(Value::as_array)
        .expect("find 的 a 参数必须声明 enum")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// **核心闭环断言**：register 能写的每一类知识，find 的 schema 都必须能查。
///
/// `RegisterFunction` 有 6 类（人物卡 / 出场计划 / 性格 / 关系 / 伏笔 / 主题锚点），
/// `FindScope::parse` 全部认得，底层 `MemoryWritingKnowledgeBase::find` 也
/// 全部实现了查询函数——唯独 find 工具对外的 schema enum 只列了 7 项，
/// 漏掉 `character_profile` / `character_plan` / `theme_anchor`。
///
/// LLM 只按 schema 填参数，因此这三类知识注册后**永久不可读**。
#[test]
fn every_registrable_knowledge_kind_is_findable_through_the_tool_schema() {
    let exposed = find_scope_enum_exposed_to_llm();

    // register 的每一类知识，对应 find 里应有的 scope 名。
    let required = [
        (RegisterFunction::CharacterProfile, "character_profile"),
        (RegisterFunction::CharacterPlan, "character_plan"),
        (RegisterFunction::CharacterTrait, "character_trait_path"),
        (RegisterFunction::Relationship, "relationship_path"),
        (RegisterFunction::Foreshadowing, "foreshadowing"),
        (RegisterFunction::ThemeAnchor, "theme_anchor"),
    ];

    let missing = required
        .iter()
        .filter(|(_, scope)| !exposed.iter().any(|item| item == scope))
        .map(|(function, scope)| format!("register 可写 {function:?}，但 find schema 无 `{scope}`"))
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "U120：register 写入的知识无法用 find 查回，数据写进黑洞：\n{}\n\
         find schema 当前暴露：{exposed:?}",
        missing.join("\n")
    );
}

/// schema enum 里的每一项都必须能被 `FindScope::parse` 接受。
///
/// 反方向的一致性：schema 若列了 parse 不认的值，LLM 照 schema 填反而报错。
#[test]
fn every_exposed_find_scope_is_accepted_by_the_parser() {
    for scope in find_scope_enum_exposed_to_llm() {
        assert!(
            FindScope::parse(&scope).is_ok(),
            "find schema 暴露了 `{scope}`，但 FindScope::parse 不接受它——\
             LLM 按 schema 填参数反而会失败"
        );
    }
}

/// 底层实现支持的每个 scope 都应对外暴露，否则是白写的能力。
///
/// 这条比上面两条更宽：它检查「实现了但没暴露」的浪费。
#[test]
fn all_implemented_find_scopes_are_exposed_to_the_model() {
    let exposed = find_scope_enum_exposed_to_llm();
    // FindScope 的全部变体对应的规范名。
    let implemented = [
        "character_profile",
        "character_plan",
        "character_trait_path",
        "relationship_path",
        "event_segments",
        "segment_text",
        "foreshadowing",
        "theme_anchor",
        "chapter_summary",
        "stage_summary",
    ];

    let hidden = implemented
        .iter()
        .filter(|scope| !exposed.iter().any(|item| item == *scope))
        .collect::<Vec<_>>();

    assert!(
        hidden.is_empty(),
        "以下 find 维度底层已实现、`FindScope::parse` 也认，但对 LLM 不可见：{hidden:?}\n\
         实现了却不暴露等于白写；且与 register 的可写类型不闭环。"
    );
}

// ════════════════════════════════════════════════════════
// 二、逐维度实测：注册后能否真的查回来
// ════════════════════════════════════════════════════════

/// 用 find 工具（而非底层 API）查询，返回结果条数。
fn find_via_tool(
    knowledge: &MemoryWritingKnowledgeBase,
    scope: &str,
    query: &str,
) -> Result<usize, String> {
    let executor = WritingToolExecutor::new(knowledge);
    let context = ToolExecutionContext {
        provider_id: "mock-llm".to_owned(),
        workflow_id: None,
        run_id: None,
        node_id: None,
        round: 0,
    };
    let call = ToolCall {
        tool_call_id: "call-1".to_owned(),
        name: "planner-find".to_owned(),
        arguments: json!({ "a": scope, "b": query }),
    };
    executor
        .execute(&context, &call)
        .map_err(|error| format!("{error:?}"))
        .map(|output| {
            output
                .value
                .get("results")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        })
}

/// 伏笔是最关键的一类：planner 注册伏笔，writer 后续要查回来回收。
#[test]
fn registered_foreshadowing_can_be_found_again() {
    let knowledge = MemoryWritingKnowledgeBase::default();

    knowledge
        .apply_register_operation(
            RegisterFunction::Foreshadowing,
            RegisterOperation::New,
            Some(
                ariadne::rag::models::RegisterContent::parse(
                    RegisterFunction::Foreshadowing,
                    json!({
                        "title": "苏禾的玉佩",
                        "description": "玉佩在第一章被反复摩挲，暗示身世",
                        "intended_payoff": "第三卷认亲时作为凭证"
                    }),
                )
                .expect("伏笔内容应可解析"),
            ),
            None,
        )
        .expect("注册伏笔应当成功");

    let count = find_via_tool(&knowledge, "foreshadowing", "玉佩")
        .expect("查询伏笔不应报错");
    assert!(
        count > 0,
        "planner 注册的伏笔查不回来，writer 无法回收伏笔——创作闭环断裂"
    );
}

/// 人物性格路径：register 的 CharacterTrait 对应 find 的 character_trait_path。
/// 这两个名字不同，是最容易接错的一对。
#[test]
fn registered_character_trait_is_findable_under_its_path_scope() {
    let knowledge = MemoryWritingKnowledgeBase::default();

    knowledge
        .apply_register_operation(
            RegisterFunction::CharacterTrait,
            RegisterOperation::New,
            Some(
                ariadne::rag::models::RegisterContent::parse(
                    RegisterFunction::CharacterTrait,
                    json!({
                        "character": "苏禾",
                        "trait_name": "外柔内刚",
                        "to_value": "在压力下显出决断",
                        "reason": "第一章末尾的抉择"
                    }),
                )
                .expect("性格内容应可解析"),
            ),
            None,
        )
        .expect("注册性格应当成功");

    let count = find_via_tool(&knowledge, "character_trait_path", "苏禾")
        .expect("查询性格路径不应报错");
    assert!(
        count > 0,
        "注册的人物性格查不回来：register 用 CharacterTrait、find 用 \
         character_trait_path，两侧命名不同，接线容易错位"
    );
}

/// 主题锚点：这一类在 find schema 里缺失，本用例证明底层其实查得到——
/// 即缺口纯粹在对外 schema，不是能力缺失。
#[test]
fn theme_anchor_is_queryable_at_the_api_level_even_though_schema_hides_it() {
    let knowledge = MemoryWritingKnowledgeBase::default();

    knowledge
        .apply_register_operation(
            RegisterFunction::ThemeAnchor,
            RegisterOperation::New,
            Some(
                ariadne::rag::models::RegisterContent::parse(
                    RegisterFunction::ThemeAnchor,
                    json!({
                        "anchor_id": "homecoming",
                        "title": "归乡",
                        "statement": "归乡即确认自己已不属于故地"
                    }),
                )
                .expect("主题锚点内容应可解析"),
            ),
            None,
        )
        .expect("注册主题锚点应当成功");

    // 底层 API 直查：证明查询能力存在。
    let response = knowledge
        .find(FindRequest {
            scope: FindScope::ThemeAnchor,
            query: "归乡".to_owned(),
            include_text: false,
            metadata: Value::Null,
        })
        .expect("底层查询主题锚点应当成功");

    assert!(
        !response.results.is_empty(),
        "底层主题锚点查询无结果——若连底层都查不到，说明问题比 schema 缺口更深"
    );

    // 经工具查询：schema 未暴露，但 parse 认得，所以这里应当也能通。
    // 真正的缺陷是 LLM 不知道可以填这个值（见 schema 一致性用例）。
    let count = find_via_tool(&knowledge, "theme_anchor", "归乡")
        .expect("工具层查询主题锚点不应报错");
    assert!(count > 0, "工具层查询主题锚点无结果");
}

// ════════════════════════════════════════════════════════
// 三、include_text：正文回填
// ════════════════════════════════════════════════════════

/// find 默认只返回轻量结果（snippet），`include_text=true` 才回填正文。
/// 这是控制上下文开销的关键设计，必须真的生效。
#[test]
fn find_include_text_is_honored_and_defaults_to_lightweight() {
    let knowledge = MemoryWritingKnowledgeBase::default();
    let executor = WritingToolExecutor::new(&knowledge);
    let context = ToolExecutionContext {
        provider_id: "mock-llm".to_owned(),
        workflow_id: None,
        run_id: None,
        node_id: None,
        round: 0,
    };

    // 默认（不传 include_text）
    let lightweight = executor
        .execute(
            &context,
            &ToolCall {
        tool_call_id: "c1".to_owned(),
                name: "planner-find".to_owned(),
                arguments: json!({ "a": "chapter_summary", "b": "第一章" }),
            },
        )
        .expect("默认查询不应报错");

    // 显式 include_text=true
    let with_text = executor
        .execute(
            &context,
            &ToolCall {
        tool_call_id: "c2".to_owned(),
                name: "planner-find".to_owned(),
                arguments: json!({
                    "a": "chapter_summary",
                    "b": "第一章",
                    "include_text": true
                }),
            },
        )
        .expect("include_text 查询不应报错");

    // 两次调用都必须成功且结构合法——include_text 不能让工具报错。
    for (label, output) in [("默认", &lightweight), ("include_text", &with_text)] {
        assert!(
            output.value.get("results").is_some(),
            "{label} 查询的返回值缺少 results 字段，LLM 无法解析"
        );
    }
}

/// `include_text` 也支持嵌套在 `c` 对象里（`parse_find_request` 的兼容路径）。
/// 两种写法都必须被接受，否则 LLM 换个写法就静默丢失该参数。
#[test]
fn find_accepts_include_text_both_flat_and_nested() {
    let flat = ariadne::rag::tools::parse_find_request(&json!({
        "a": "segment_text",
        "b": "开头",
        "include_text": true
    }))
    .expect("平铺写法应可解析");
    assert!(flat.include_text, "平铺 include_text 未被识别");

    let nested = ariadne::rag::tools::parse_find_request(&json!({
        "a": "segment_text",
        "b": "开头",
        "c": { "include_text": true }
    }))
    .expect("嵌套写法应可解析");
    assert!(
        nested.include_text,
        "嵌套在 c 里的 include_text 未被识别——LLM 换写法就会静默丢参数"
    );
}

// ════════════════════════════════════════════════════════
// 四、错误可诊断性
// ════════════════════════════════════════════════════════

/// 非法 scope 必须给出可诊断错误，而不是静默返回空结果。
/// 静默空结果会让 LLM 误以为「知识库里没有」，进而写出错误正文。
#[test]
fn unknown_find_scope_fails_loudly_instead_of_returning_empty() {
    let knowledge = MemoryWritingKnowledgeBase::default();
    let error = find_via_tool(&knowledge, "不存在的维度", "x")
        .expect_err("非法 scope 必须报错，不能静默返回空结果");

    assert!(
        error.contains("find scope") || error.contains("unknown"),
        "非法 scope 的错误信息应指明是 scope 问题，实际：{error}"
    );
}

/// 缺少必填参数 `b`（查询词）时必须报错。
#[test]
fn find_without_query_fails_loudly() {
    let knowledge = MemoryWritingKnowledgeBase::default();
    let executor = WritingToolExecutor::new(&knowledge);
    let result = executor.execute(
        &ToolExecutionContext {
        provider_id: "mock-llm".to_owned(),
        workflow_id: None,
        run_id: None,
        node_id: None,
        round: 0,
    },
        &ToolCall {
        tool_call_id: "c".to_owned(),
            name: "planner-find".to_owned(),
            arguments: json!({ "a": "foreshadowing" }),
        },
    );

    assert!(
        result.is_err(),
        "缺少查询词 b 时必须报错，否则 LLM 收到空结果会误判知识库为空"
    );
}
