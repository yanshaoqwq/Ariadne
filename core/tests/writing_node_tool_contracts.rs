//! 写作节点工具下发矩阵契约（2026-07-29）。
//!
//! U108 的接线只被 writer 节点的用例覆盖过。本文件把**全部 8 个写作节点类型**
//! 逐个跑起来，断言每个节点真实收到的工具清单与 `expected_tool_names` 的设计
//! 声明一致，捕捉「writer 通了但 planner 没通」这类分节点类型的漏接线。
//!
//! 判据是 ariadne 发给 LLM 的**真实出站 HTTP 请求体**里的 `tools` 数组，
//! 而非任何内部函数返回值——工具没序列化进请求，模型就调用不到。
//!
//! 分析见 `项目检验报告/发布前全量代码审查/13-配置项存在性与执行链路阻断审查.md`。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    get_permissions_settings_impl, run_workflow_impl, save_permissions_settings_impl,
    save_provider_settings_impl, save_workflow_graph_impl, CanvasNode, ProviderSettingsUpdate,
    RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType};
use serde_json::{json, Value};

// ════════════════════════════════════════════════════════
// 测试基建
// ════════════════════════════════════════════════════════

fn accept_with_deadline(listener: &TcpListener, timeout: Duration) -> std::net::TcpStream {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "等待本地 HTTP 请求超时");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("接受本地 HTTP 请求失败：{error}"),
        }
    }
}

/// 假 LLM：收一个请求、回一段固定文本，并把请求原文交回供断言。
fn spawn_fake_llm(reply: &str) -> (String, thread::JoinHandle<Vec<String>>) {
    let body = json!({
        "model": "m",
        "choices": [{"message": {"content": reply, "tool_calls": []}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 4}
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        let mut stream = accept_with_deadline(&listener, Duration::from_secs(10));
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0u8; 262_144];
        if let Ok(read) = stream.read(&mut buffer) {
            seen.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        seen
    });
    (base_url, handle)
}

const PROVIDER_ID: &str = "primary";
const MODEL_ID: &str = "m";

fn provision(project_root: &std::path::Path, secrets: &MemorySecretStore, base_url: String) {
    ariadne::frontend::initialize_project(project_root).unwrap();
    save_provider_settings_impl(
        project_root,
        ProviderSettingsUpdate {
            provider_id: PROVIDER_ID.to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Primary".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: MODEL_ID.to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    )
    .unwrap();
    ProjectCredentialScope::new(project_root, secrets)
        .unwrap()
        .set_provider_secret(PROVIDER_ID, SecretValue::new("sk-test"))
        .unwrap();
}

/// 打开写入与注册两类工具的出厂开关。
///
/// 二者默认关闭（`global.write=false` / `global.register=false`），是产品的
/// 安全默认；要验证工具能否下发，必须先模拟用户在权限页打开它们。
fn enable_write_and_register(project_root: &std::path::Path) {
    let mut settings = get_permissions_settings_impl(project_root).unwrap();
    let global = settings
        .tool_controls
        .entry("global".to_owned())
        .or_default();
    global.insert("write".to_owned(), Some(true));
    global.insert("register".to_owned(), Some(true));
    save_permissions_settings_impl(project_root, settings).unwrap();
}

/// 跑一个单节点工作流，返回 (运行结果, 出站请求原文)。
fn run_single_writing_node(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    node_type: &str,
    data: Value,
    server: thread::JoinHandle<Vec<String>>,
) -> (Result<String, String>, String) {
    // 注意：返回的 run_id 用于判定**暂停原因**。U117 接线后写作节点产出即待审，
    // 终态是 paused 而非 succeeded；不看原因就无法区分「待审」与「真失败」。
    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: node_type.to_owned(),
            name: node_type.to_owned(),
            nodes: vec![CanvasNode {
                id: "node-1".to_owned(),
                r#type: node_type.to_owned(),
                label: None,
                data,
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .expect("保存工作流应当成功");

    let started = run_workflow_impl(
        project_root,
        secrets,
        RunWorkflowRequest {
            workflow_id: node_type.to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    )
    .map_err(|error| format!("{error:?}"));
    let run_id = started.as_ref().map(|s| s.run_id.clone()).unwrap_or_default();
    let run = started.map(|started| started.status);
    LAST_RUN_ID.with(|slot| {
        *slot.borrow_mut() = (project_root.to_path_buf(), node_type.to_owned(), run_id)
    });

    let outbound = server
        .join()
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default();
    (run, outbound)
}

thread_local! {
    /// 最近一次 `run_single_writing_node` 的 (workflow_id, run_id)。
    /// 用线程局部而非改 12 个调用点的返回解构：这些用例真正要断言的是
    /// **工具下发内容**，终态只是前置条件。
    static LAST_RUN_ID: std::cell::RefCell<(std::path::PathBuf, String, String)> =
        std::cell::RefCell::new((std::path::PathBuf::new(), String::new(), String::new()));
}

/// 断言节点已跑到「产出可用」的终态。
///
/// U117 接线后，写作节点产出正文即挂待审确认项，运行**正确地**停在 `paused`
/// ——正文不经审阅落盘才是缺陷。所以这里放行 `paused`，但**必须证明它是
/// 待审导致的**：读运行态确认项，有 Pending 才算数。
/// 若只放宽成「不是 error 就行」，真实的执行失败会被一并放过。
fn assert_output_ready(run: &Result<String, String>, label: &str) {
    let status = run
        .as_ref()
        .unwrap_or_else(|error| panic!("{label} 节点必须能运行：{error}"));
    if status == "succeeded" {
        return;
    }
    assert_eq!(status, "paused", "{label} 节点终态既非 succeeded 也非 paused");

    let (project_root, workflow_id, run_id) = LAST_RUN_ID.with(|slot| slot.borrow().clone());
    let pending = ariadne::workflow::SqliteWorkflowRuntimeStore::open(&project_root)
        .ok()
        .and_then(|store| {
            ariadne::workflow::WorkflowRuntimeStore::load_state(
                &store,
                &ariadne::contracts::WorkflowId::from(workflow_id),
                &ariadne::contracts::RunId::from(run_id),
            )
            .ok()
            .flatten()
        })
        .map(|state| {
            state
                .confirmations
                .values()
                .any(|item| item.state == ariadne::workflow::RuntimeConfirmationState::Pending)
        })
        .unwrap_or(false);
    assert!(
        pending,
        "{label} 停在 paused 但没有任何 Pending 确认项——\
         这不是待审，是真的执行失败"
    );
}

/// 为写文件类 agent 准备好目标文档，返回其相对 document_id。
fn seed_document(project_root: &std::path::Path, relative: &str, body: &str) -> String {
    let path = project_root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    relative.to_owned()
}

// ════════════════════════════════════════════════════════
// 规划类节点：outliner / designer / planner
// 这三个 agent 都有 register + 行号 patch，是覆盖面最广的一类
// ════════════════════════════════════════════════════════

/// **本文件的主用例**：planner 节点配好章节大纲文件后，
/// 必须同时拿到 register、find 与行号 patch 三类工具。
///
/// U108 的既有用例只覆盖 writer；planner 的工具集与 writer 不同
/// （多 register、作用域是 ChapterOutline 而非 ChapterBody），
/// 需要独立验证，否则「writer 通了 planner 没通」不会被发现。
#[test]
fn planner_node_receives_register_find_and_line_patch_tools() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("本章大纲已拟好。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    let document_id = seed_document(
        temp.path(),
        "planning/chapters/chapter-01.md",
        "# 第一章大纲\n1. 开场\n",
    );

    let (run, outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "planner",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "拟定本章大纲",
            "document_id": document_id,
        }),
        server,
    );

    assert_output_ready(&run, "planner");

    for tool in [
        "planner-register",
        "planner-find",
        "planner-insert-lines",
        "planner-replace-lines",
    ] {
        assert!(
            outbound.contains(tool),
            "planner 节点未收到工具 `{tool}`——设计声明它应当具备（见 rag/models.rs \
             expected_tool_names）。实际出站请求：{}",
            outbound.chars().take(1200).collect::<String>()
        );
    }
}

/// outliner 作用于全局总纲（`planning/global.md`）。
#[test]
fn outliner_node_receives_its_full_tool_set() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("全文总纲已拟好。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    let document_id = seed_document(temp.path(), "planning/global.md", "# 总纲\n");

    let (run, outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "outliner",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "拟定全文总纲",
            "document_id": document_id,
        }),
        server,
    );

    assert_output_ready(&run, "outliner");
    for tool in ["outliner-register", "outliner-find", "outliner-insert-lines"] {
        assert!(
            outbound.contains(tool),
            "outliner 节点未收到工具 `{tool}`。实际出站请求：{}",
            outbound.chars().take(1200).collect::<String>()
        );
    }
}

/// designer 作用于阶段总纲（`planning/stages/*.md`）。
#[test]
fn designer_node_receives_its_full_tool_set() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("阶段总纲已拟好。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    let document_id = seed_document(temp.path(), "planning/stages/stage-01.md", "# 阶段一\n");

    let (run, outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "designer",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "拟定阶段总纲",
            "document_id": document_id,
        }),
        server,
    );

    assert_output_ready(&run, "designer");
    for tool in ["designer-register", "designer-find", "designer-insert-lines"] {
        assert!(
            outbound.contains(tool),
            "designer 节点未收到工具 `{tool}`。实际出站请求：{}",
            outbound.chars().take(1200).collect::<String>()
        );
    }
}

// ════════════════════════════════════════════════════════
// 关键边界：register 不该被 document_id 绑架
// ════════════════════════════════════════════════════════

/// `*-register` 写的是知识库，不碰文件（`execute_register` 只用 `self.knowledge`）。
///
/// 所以 planner 没指名 `document_id` 时——例如用户只想让它注册伏笔、
/// 还没建大纲文件——register 与 find 仍必须可用，只是行号 patch 不下发。
///
/// 若此用例失败，说明知识库类工具被文档上下文错误地绑架了：
/// 用户「先注册设定、后写大纲」这条正常工作方式会被堵死。
#[test]
fn planner_without_document_id_still_gets_knowledge_tools() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("已注册伏笔。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    // 与主用例唯一的差别：没有 document_id。
    let (run, outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "planner",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "注册一个伏笔",
        }),
        server,
    );

    assert_output_ready(&run, "未指名 document_id 的 planner");

    assert!(
        outbound.contains("planner-register"),
        "未指名 document_id 时 `planner-register` 也被撤下了。register 只写知识库、\
         不碰文件，不应被文档上下文绑架——否则「先注册设定再写大纲」这条正常\
         工作方式被堵死。实际出站请求：{}",
        outbound.chars().take(1200).collect::<String>()
    );
    assert!(
        outbound.contains("planner-find"),
        "未指名 document_id 时 `planner-find` 也被撤下了。实际出站请求：{}",
        outbound.chars().take(1200).collect::<String>()
    );

    // 行号 patch 缺正文必然调用失败，此时不下发才是正确的。
    assert!(
        !outbound.contains("planner-insert-lines"),
        "未指名 document_id 却下发了行号 patch 工具，模型必然调用失败"
    );
}

// ════════════════════════════════════════════════════════
// 只读类节点：detail / critic / prudent
// 这三个 agent 没有写入作用域，是最容易在装配时被判空的一类
// ════════════════════════════════════════════════════════

/// 只读 agent（`writing_document_scope_for_agent` 返回 `None`）必须仍能拿到
/// `*-find`，且节点本身不能因为「没有写入作用域」而运行失败。
#[test]
fn read_only_writing_nodes_still_receive_find_tool() {
    for (node_type, expected_tool) in [
        ("detail", "detail-find"),
        ("critic", "critic-find"),
        ("prudent", "prudent-find"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

        let (base_url, server) = spawn_fake_llm("已完成。");
        let secrets = MemorySecretStore::default();
        provision(temp.path(), &secrets, base_url);
        enable_write_and_register(temp.path());

        let (run, outbound) = run_single_writing_node(
            temp.path(),
            &secrets,
            node_type,
            json!({
                "provider_id": PROVIDER_ID,
                "model_id": MODEL_ID,
                "prompt_template": "执行任务",
            }),
            server,
        );

        assert_output_ready(&run, node_type);
        assert!(
            outbound.contains(expected_tool),
            "只读 agent `{node_type}` 未收到 `{expected_tool}`——它没有写入作用域，\
             但仍应具备查询本地创作知识的能力。实际出站请求：{}",
            outbound.chars().take(1000).collect::<String>()
        );
    }
}

/// 只读 agent 若被误配了 `document_id`，必须给出可诊断的错误，
/// 而不是静默忽略或抛出与文档无关的内部错误。
///
/// `load_workflow_writing_context` 对这种情况有明确的 fail-loud 分支
/// （"does not support line patch tools"），本用例锁定该行为。
#[test]
fn read_only_node_with_document_id_fails_with_actionable_error() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("不该被调用。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());
    let document_id = seed_document(temp.path(), "documents/chapter-01.md", "正文\n");

    let (run, _) = run_single_writing_node(
        temp.path(),
        &secrets,
        "critic",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "评价",
            "document_id": document_id,
        }),
        server,
    );

    // 运行可能返回 Err（预检拒绝），也可能是 failed 状态（执行期拒绝）；
    // 两者都算 fail-loud，静默 succeeded 才是缺陷。
    match run {
        Ok(status) => assert_ne!(
            status, "succeeded",
            "只读 agent 配了 document_id 却静默成功了——用户不会知道自己配错"
        ),
        Err(_) => {}
    }
}

// ════════════════════════════════════════════════════════
// polisher：与 writer 同作用域，但工具集不同（无 register）
// ════════════════════════════════════════════════════════

/// polisher 在 ChapterBody 作用域做有限修改，必须拿到行号 patch。
#[test]
fn polisher_node_receives_line_patch_tools() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("已润色。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    let document_id = seed_document(
        temp.path(),
        "documents/chapter-01.md",
        "# 第一章\n夜色沉沉。\n",
    );

    let (run, outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "polisher",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "润色本章",
            "document_id": document_id,
        }),
        server,
    );

    assert_output_ready(&run, "polisher");
    for tool in ["polisher-insert-lines", "polisher-replace-lines"] {
        assert!(
            outbound.contains(tool),
            "polisher 节点未收到 `{tool}`。实际出站请求：{}",
            outbound.chars().take(1200).collect::<String>()
        );
    }

    // polisher 按设计不具备 register（只改正文，不注册设定）。
    assert!(
        !outbound.contains("polisher-register"),
        "polisher 不应具备 register 工具——设计上它只做有限的正文修改"
    );
}

// ════════════════════════════════════════════════════════
// 契约一致性：声明清单 ↔ 实际生成
// ════════════════════════════════════════════════════════

/// `expected_tool_names` 声明了 `*-rewrite-file`（outliner/designer/planner 三个），
/// 但 `tool_definitions_for_agent` 不生成它、`WritingToolExecutor` 不处理它、
/// `rewrite_file_to_patch` 零调用——三方断裂。
///
/// 该断裂现有测试发现不了：`rag_contracts.rs` 只断言实现输出，
/// 不与声明清单交叉验证。本用例把两者对齐，锁死这个缺口。
///
/// 修复方向二选一：要么实现这三个工具，要么从声明清单与权限开关中移除。
/// 无论哪条，声明与实现都不该继续背离。
#[test]
fn declared_tool_names_match_generated_tool_definitions() {
    let prompts = ariadne::rag::resources::load_prompt_resources()
        .expect("内置 prompt 资源必须可读");

    let definitions = [
        ariadne::rag::models::WritingNodeDefinition::outliner(),
        ariadne::rag::models::WritingNodeDefinition::designer(),
        ariadne::rag::models::WritingNodeDefinition::planner(),
        ariadne::rag::models::WritingNodeDefinition::detail(),
        ariadne::rag::models::WritingNodeDefinition::writer(),
        ariadne::rag::models::WritingNodeDefinition::critic(),
        ariadne::rag::models::WritingNodeDefinition::prudent(),
        ariadne::rag::models::WritingNodeDefinition::polisher(),
        ariadne::rag::models::WritingNodeDefinition::summarizer(),
    ];

    let mut mismatches = Vec::new();
    for definition in definitions {
        let generated =
            ariadne::rag::tools::tool_definitions_for_agent(definition.agent, &prompts)
                .unwrap_or_default()
                .into_iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>();

        for declared in &definition.tool_names {
            if !generated.contains(declared) {
                mismatches.push(format!(
                    "{}: 声明了 `{declared}` 但 tool_definitions_for_agent 不生成它",
                    definition.agent.node_type()
                ));
            }
        }
    }

    assert!(
        mismatches.is_empty(),
        "声明清单与实际生成的工具定义不一致，权限页会出现控制不到任何行为的开关：\n{}",
        mismatches.join("\n")
    );
}

// ════════════════════════════════════════════════════════
// U121：register 工具的产出必须落盘
// ════════════════════════════════════════════════════════

/// 支持两轮的假 LLM：第 1 轮发起一次工具调用，第 2 轮给出终答。
///
/// 单轮版 `spawn_fake_llm` 无法覆盖「模型真的调用了写作工具」这条路径——
/// 而工具**被调用之后**产出去哪了，才是本节要验证的问题。
fn spawn_tool_calling_llm(
    tool_name: &str,
    arguments_json: &str,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let first = json!({
        "model": MODEL_ID,
        "choices": [{
            "message": {
                "content": "",
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {"name": tool_name, "arguments": arguments_json}
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2}
    })
    .to_string();
    let second = json!({
        "model": MODEL_ID,
        "choices": [{"message": {"content": "已登记。", "tool_calls": []}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 20, "completion_tokens": 3}
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for body in [first, second] {
            let mut stream = accept_with_deadline(&listener, Duration::from_secs(10));
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0u8; 262_144];
            if let Ok(read) = stream.read(&mut buffer) {
                seen.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
        seen
    });
    (base_url, handle)
}

/// **U121**：planner 调用 `planner-register` 登记一条伏笔后，该伏笔必须真的进入
/// 项目写作知识库（`.runtime` 下的 sqlite），而不是随节点执行结束一起蒸发。
///
/// 这是 U108 接线留下的缺口：装配处每次执行都 `load_knowledge()` 出一个**内存**
/// 知识库交给 executor，register 工具确实写进了它，但节点结束后没有任何
/// `save_knowledge*` 调用——产出无声丢失。对照 summarizer 走的是
/// `save_chapter_knowledge_with_operation_locked`（`integration.rs:1571`）。
///
/// 判据是「用户下次能不能查到这条伏笔」，所以断言直接读库，不看工具返回值。
#[test]
fn register_tool_output_is_persisted_to_project_knowledge() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_tool_calling_llm(
        "planner-register",
        "{\"a\":\"foreshadowing\",\"b\":\"new\",\"c\":{\"title\":\"旧钥匙\",\
         \"description\":\"阿宁在门缝里看见旧钥匙\",\"intended_payoff\":\"第三章打开地下室\"}}",
    );
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    let document_id = seed_document(
        temp.path(),
        "planning/chapters/chapter-01.md",
        "# 第一章大纲\n1. 开场\n",
    );

    let (run, _outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "planner",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "登记一条伏笔",
            "document_id": document_id,
        }),
        server,
    );
    assert_output_ready(&run, "planner");

    // 判据：重新打开项目知识库，那条伏笔必须还在。
    let store =
        ariadne::rag::store::SqliteWritingKnowledgeStore::open(temp.path()).unwrap();
    let knowledge = store.load_knowledge().unwrap();
    let changes = knowledge.registered_changes().unwrap();

    assert!(
        changes.iter().any(|change| {
            serde_json::to_string(&change.content)
                .unwrap_or_default()
                .contains("旧钥匙")
        }),
        "U121：planner 调用 register 登记的伏笔没有落盘——节点结束即丢失。\
         装配处 load_knowledge() 出的是内存知识库，执行后无人保存。\
         实际库中的注册项：{changes:?}"
    );
}

// ════════════════════════════════════════════════════════
// U121：planner 埋的伏笔，find 必须查得回来
// ════════════════════════════════════════════════════════

/// **伏笔机制的闭环判据**：planner 用 `register` 埋一条伏笔后，
/// 任何 agent 用 `find(scope=foreshadowing)` 都必须能查到它。
///
/// 这是伏笔机制存在的全部意义——埋下去是为了后文回收。
/// 若查不回来，planner 埋的伏笔对 writer 不可见，伏笔功能整体失效。
///
/// 缺陷所在：`apply_register_operation` 把伏笔写进 `state.changes`，
/// 而 `find_foreshadowing` 读的是 `state.foreshadowing`——两个互不相通的容器。
/// 性格/关系类读的都是 `state.changes`，故唯独伏笔这一类错位。
#[test]
fn registered_foreshadowing_is_findable_by_find_tool() {
    use ariadne::rag::memory::MemoryWritingKnowledgeBase;
    use ariadne::rag::models::{
        FindRequest, FindScope, ForeshadowingContent, RegisterContent, RegisterFunction,
        RegisterOperation,
    };

    let knowledge = MemoryWritingKnowledgeBase::new();

    // planner 埋下一条伏笔（等价于 LLM 调用 planner-register）。
    knowledge
        .apply_register_operation(
            RegisterFunction::Foreshadowing,
            RegisterOperation::New,
            Some(RegisterContent::Foreshadowing(ForeshadowingContent {
                title: "旧钥匙".to_owned(),
                description: "阿宁在门缝里看见一把旧钥匙".to_owned(),
                intended_payoff: "第三章用它打开地下室".to_owned(),
            })),
            None,
        )
        .expect("planner 埋伏笔应当成功");

    // writer 后续检索伏笔（等价于 LLM 调用 writer-find(scope=foreshadowing)）。
    let found = knowledge
        .find(FindRequest {
            scope: FindScope::Foreshadowing,
            query: "旧钥匙".to_owned(),
            include_text: false,
            metadata: serde_json::Value::Null,
        })
        .expect("find 调用本身应当成功");

    assert!(
        !found.results.is_empty(),
        "U121：planner 埋下的伏笔「旧钥匙」用 find(scope=foreshadowing) 查不回来——\
         register 写 state.changes，find 读 state.foreshadowing，两个容器互不相通。\
         伏笔埋下去就是为了后文回收，查不回来等于伏笔机制整体失效。"
    );
}

// ════════════════════════════════════════════════════════
// U114 / U115：提示词占位符必须被替换，写作节点必须看到小说上下文
// ════════════════════════════════════════════════════════

/// **U115**：模板里的 `{{}}` 占位符不得以字面量进入 LLM 请求。
///
/// 用户按产品自带的 `node_template.writer.default` 形态写提示词（含
/// `{{带行号正文}}` 这类槽位）。接线前没有任何替换逻辑，字面量原样发给模型；
/// 接线后必须替换成实际内容。
#[test]
fn prompt_placeholders_are_substituted_not_sent_literally() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("续写完成。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    let document_id = seed_document(
        temp.path(),
        "chapter-01.md",
        "顾言把玉佩收进袖中。\n她没有回头。\n",
    );

    let (run, outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "writer",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "依据正文续写：\n{{带行号正文}}",
            "document_id": document_id,
            "chapter_id": "chapter-1",
        }),
        server,
    );

    let status = run.expect("writer 节点必须能运行成功");
    assert_eq!(status, "succeeded", "writer 节点运行未成功");

    assert!(
        !outbound.contains("{{"),
        "U115：占位符被原样发给 LLM，未做替换。实际出站请求：{}",
        outbound.chars().take(900).collect::<String>()
    );
}

/// **U114**：writer 必须看到**带行号**的正文。
///
/// 这是 U108 行号工具真正可用的前提——LLM 看不到行号，就无法调用
/// `writer-insert-lines(after_line=N)`。断言行号标记与正文同时出现。
#[test]
fn writer_node_receives_line_numbered_draft_for_line_patch_tools() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("好。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    let document_id = seed_document(
        temp.path(),
        "chapter-01.md",
        "顾言把玉佩收进袖中。\n她没有回头。\n",
    );

    let (run, outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "writer",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "在合适位置插入一句：\n{{带行号正文}}",
            "document_id": document_id,
            "chapter_id": "chapter-1",
        }),
        server,
    );

    let status = run.expect("writer 节点必须能运行成功");
    assert_eq!(status, "succeeded", "writer 节点运行未成功");

    // 行号格式见 line_patch.rs 的 line_numbered_text：`1: 正文`。
    assert!(
        outbound.contains("1:") && outbound.contains("玉佩"),
        "U114：writer 没有收到带行号正文，行号 patch 工具无从下笔。\
         实际出站请求：{}",
        outbound.chars().take(900).collect::<String>()
    );
    assert!(
        outbound.contains("2:"),
        "U114：带行号正文只有第一行，行号装配不完整。实际出站请求：{}",
        outbound.chars().take(900).collect::<String>()
    );
}

/// 反向护栏：模板引用了一个**拼错**的变量时必须 fail-loud，
/// 不得静默替换成空串——否则「本章大纲」写成空白也能一路跑到底。
#[test]
fn unknown_prompt_placeholder_fails_loudly() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("不该被调用。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    let document_id = seed_document(temp.path(), "chapter-01.md", "正文。\n");

    let (run, _outbound) = run_single_writing_node(
        temp.path(),
        &secrets,
        "writer",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": "续写：{{这个变量根本不存在}}",
            "document_id": document_id,
            "chapter_id": "chapter-1",
        }),
        server,
    );

    // `run_workflow_impl` 把「节点执行失败」当作运行结果而非命令错误返回，
    // 所以判据是终态为 failed，而不是 Err。
    let status = run.expect("运行命令本身应当受理");
    assert_eq!(
        status, "failed",
        "U115：拼错的占位符必须让该节点失败，而不是把 `{{{{这个变量根本不存在}}}}` \
         当正文发给模型、或静默替换成空串"
    );

    // 更强的证据：假 LLM 一次都没被调用——渲染在发请求**之前**就失败了，
    // 因此不存在「先烧掉一次调用再报错」。
    assert!(
        _outbound.is_empty(),
        "渲染失败必须发生在调用 LLM 之前，实际却已发出请求：{}",
        _outbound.chars().take(300).collect::<String>()
    );
}

// ════════════════════════════════════════════════════════
// U123：空文件写入 / after_line=0 / CRLF 规范化
// ════════════════════════════════════════════════════════

/// **U123 主用例**：空文件必须写得进去。
///
/// 「每章一个文件」时新建章节文件为空，是分章节写作最常见的起点。
/// 修复前 `line_ranges("")` 为空，任何 `after_line >= 1` 都越界，
/// writer 在空章节上一个字都写不进去。
#[test]
fn u123_empty_file_accepts_insert_at_line_zero() {
    use ariadne::rag::line_patch::{insert_lines_to_patch, WriterInsertLines};

    let patch = insert_lines_to_patch(
        "",
        WriterInsertLines {
            document_id: "chapter-01.md".to_owned(),
            base_version: None,
            after_line: 0,
            text: "第一章\n\n夜色像一封没写完的信。\n".to_owned(),
        },
    )
    .expect("U123：空文件 after_line=0 必须能写入初始内容");

    assert_eq!(patch.hunks.len(), 1, "空文件写入应产出单个 hunk");
    assert!(
        patch.hunks[0].replacement.contains("夜色像一封没写完的信"),
        "写入内容未进入 patch：{:?}",
        patch.hunks[0]
    );
}

/// `after_line = 0` 在**非空**文件上表示插入到最开头（第 1 行之前）。
///
/// 修复前最小值是 1（插到第 1 行**之后**），想在开头补一段环境描写只能改用
/// `replace_lines(1, 1, "新段落\n原第一行")`——逼 LLM 重述原文，费 token 又易错。
#[test]
fn u123_insert_at_line_zero_prepends_to_non_empty_file() {
    use ariadne::rag::line_patch::{insert_lines_to_patch, WriterInsertLines};

    let original = "顾言把玉佩收进袖中。\n她没有回头。\n";
    let patch = insert_lines_to_patch(
        original,
        WriterInsertLines {
            document_id: "chapter-01.md".to_owned(),
            base_version: None,
            after_line: 0,
            text: "雨从檐角落下。\n".to_owned(),
        },
    )
    .expect("after_line=0 应插入到文件最开头");

    let hunk = &patch.hunks[0];
    assert_eq!(
        (hunk.range.start, hunk.range.end),
        (0, 0),
        "插入到开头的区间必须是 (0, 0)，实际 {:?}",
        hunk.range
    );
}

/// 空文件的 replace 必须能写入初始内容（唯一可表达的区间是 0..=0）。
#[test]
fn u123_empty_file_accepts_zero_line_replace() {
    use ariadne::rag::line_patch::{replace_lines_to_patch, WriterReplaceLines};

    let patch = replace_lines_to_patch(
        "",
        WriterReplaceLines {
            document_id: "chapter-01.md".to_owned(),
            base_version: None,
            start_line: 0,
            end_line: 0,
            text: "初稿。\n".to_owned(),
        },
    )
    .expect("U123：空文件 0 行 replace 必须能写入初始内容");

    assert_eq!(patch.hunks.len(), 1);
}

/// 反向护栏：**非空**文件上的 0 行 replace 仍须被拒。
///
/// 那里 `(0, 0)` 没有意义，放行只会掩盖 LLM 的行号计算错误。
#[test]
fn u123_non_empty_file_still_rejects_zero_line_replace() {
    use ariadne::rag::line_patch::{replace_lines_to_patch, WriterReplaceLines};

    let error = replace_lines_to_patch(
        "已有正文。\n",
        WriterReplaceLines {
            document_id: "chapter-01.md".to_owned(),
            base_version: None,
            start_line: 0,
            end_line: 0,
            text: "覆盖。\n".to_owned(),
        },
    )
    .expect_err("非空文件的 0 行 replace 必须被拒");

    let message = format!("{error:?}");
    assert!(
        message.contains("1-based") || message.contains("interval"),
        "错误信息应说明非空文件要求 1-based 闭区间，实际：{message}"
    );
}

/// **防两处逻辑漂移**：同一组操作经 `PatchSession`（模拟路径）与
/// `insert_lines_to_patch`（直接路径）必须给出一致结果。
///
/// 这两处各有一份 `after_line` 拒绝逻辑，修复时同步放行了；本用例锁住它们
/// 不再分叉——否则 PatchSession 的预览与实际落盘会不一致。
#[test]
fn u123_patch_session_and_direct_call_agree_on_empty_file() {
    use ariadne::rag::line_patch::{insert_lines_to_patch, PatchSession, WriterInsertLines};

    let text = "开篇第一句。\n";
    let mut session = PatchSession::new("chapter-01.md", None, "").unwrap();
    session
        .insert_lines(0, text)
        .expect("PatchSession 模拟路径也必须接受空文件 after_line=0");
    let via_session = session.simulated.clone();

    let patch = insert_lines_to_patch(
        "",
        WriterInsertLines {
            document_id: "chapter-01.md".to_owned(),
            base_version: None,
            after_line: 0,
            text: text.to_owned(),
        },
    )
    .expect("直接路径应成功");

    assert_eq!(
        via_session, text,
        "PatchSession 模拟结果与写入文本不一致"
    );
    assert_eq!(
        patch.hunks[0].replacement, text,
        "两条路径对同一操作给出的结果必须一致"
    );
}

/// **U123 CRLF 规范化**：带 `\r\n` 的正文经保存边界后，
/// 「带行号预览」的行数与「行号→字节区间」的段数必须一致。
///
/// 病根：`line_numbered_text` 用 `str::lines()`（剥掉 `\r`），而 `line_ranges`
/// 只按 `\n` 切分（`\r` 留在行内）。用户从 Word / 记事本导入 CRLF 正文时，
/// LLM 按预览算出的行号会落到错误的字节区间，replace 把 `\r` 算进区间、切坏正文。
#[test]
fn u123_crlf_is_normalized_at_write_boundary() {
    use ariadne::documents::{DocumentReadRequest, DocumentRepository, DocumentWriteRequest};
    use ariadne::rag::line_patch::line_numbered_text;

    let temp = tempfile::tempdir().unwrap();
    let documents = ariadne::documents::FileDocumentService::new(
        ariadne::contracts::PermissionPolicy {
            readable_file_roots: vec![temp.path().to_path_buf()],
            writable_file_roots: vec![temp.path().to_path_buf()],
            ..ariadne::contracts::PermissionPolicy::default()
        },
        temp.path().join(".runtime/artifacts"),
    );

    let path = temp.path().join("chapter-01.md");
    documents
        .save_document(DocumentWriteRequest {
            path: path.clone(),
            // 模拟从 Word / 记事本粘来的 CRLF 正文。
            content: "第一行\r\n第二行\r\n第三行\r\n".to_owned(),
            format: None,
            base_version: None,
        })
        .expect("保存 CRLF 正文应当成功");

    let stored = documents
        .open_document(DocumentReadRequest {
            path,
            format: None,
        })
        .expect("读回文档应当成功")
        .content;

    assert!(
        !stored.contains('\r'),
        "U123：保存边界未把 CRLF 规范化成 LF，实际存了：{stored:?}"
    );

    // 关键一致性：预览行数 == 行号区间段数。不一致就意味着 LLM 看到的行号
    // 与后端切分的字节区间对不上。
    let preview_lines = line_numbered_text(&stored).lines().count();
    let range_lines = stored.split_inclusive('\n').count();
    assert_eq!(
        preview_lines, range_lines,
        "带行号预览({preview_lines} 行)与行号区间({range_lines} 段)不一致——\
         行号工具会切到错误位置"
    );
}
