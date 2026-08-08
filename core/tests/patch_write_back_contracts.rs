//! U108 阶段 3：写作 patch 的写回闭环。
//!
//! **缺陷形状比报告描述的更靠前一环。** 报告写的是「审批通过后正文仍不会写入」，
//! 复核后发现链条断得更早：patch **根本没有被保留下来等待审批**。
//!
//! 三条互相印证的证据（复核于 2026-08-08）：
//! - `PatchSession` 生产零实例化——`PatchSession::new` 只在测试里被调用；
//! - `patch_session_commit_id` 生产恒为 `None`——全仓只有测试给它赋过 `Some`；
//! - 于是 `record_node_output` 的门禁恒走 `patch_state = None` 分支，
//!   `PatchWriteBackState` 四个变体在生产中一个都到不了。
//!
//! 行号 patch 工具（`writer-insert-lines` 等）执行后只把 `DocumentPatch`
//! **当作工具返回值塞回对话**，既不落盘、也不持久化。节点一结束，patch 随
//! 内存一起消失——用户看到「工具调用成功」，磁盘上一个字都没多。
//!
//! 判据因此只能取**磁盘正文**：任何「确认项存在」「工具返回成功」「运行态是
//! Applied」的断言，在 patch 压根没被保留时都可能照样为真。这正是 U108/U114/
//! U117 那一类「实现完整 + 有测试覆盖 + 生产零调用者」缺陷最擅长骗过的判据。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    get_permissions_settings_impl, resolve_confirmation_impl, run_workflow_impl,
    save_permissions_settings_impl, save_provider_settings_impl, save_workflow_graph_impl,
    CanvasNode, ConfirmationDecision, ProviderSettingsUpdate, ResolveConfirmationRequest,
    RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType, RunId, WorkflowId};
use ariadne::workflow::{
    RuntimeConfirmationState, SqliteWorkflowRuntimeStore, WorkflowRuntimeStore,
};
use serde_json::{json, Value};

const PROVIDER_ID: &str = "primary";
const MODEL_ID: &str = "m";
const CHAPTER_PATH: &str = "chapters/chapter-01.md";

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

/// 假 LLM：第 1 轮发起若干次写作工具调用，第 2 轮给终答。
///
/// 支持多个工具调用是必需的——单次调用测不出「同一节点内多次 patch
/// 必须基于前一次的模拟结果算行号」这条，而那正是 `PatchSession` 存在的理由。
fn spawn_patch_tool_llm(calls: Vec<(String, String)>) -> String {
    let tool_calls: Vec<Value> = calls
        .iter()
        .enumerate()
        .map(|(index, (name, arguments))| {
            json!({
                "id": format!("call-{index}"),
                "type": "function",
                "function": {"name": name, "arguments": arguments}
            })
        })
        .collect();
    let first = json!({
        "model": MODEL_ID,
        "choices": [{
            "message": {"content": "", "tool_calls": tool_calls},
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2}
    })
    .to_string();
    let second = json!({
        "model": MODEL_ID,
        "choices": [{"message": {"content": "写好了。", "tool_calls": []}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 20, "completion_tokens": 3}
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for body in [first, second] {
            let mut stream = accept_with_deadline(&listener, Duration::from_secs(10));
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buffer = [0u8; 262_144];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    base_url
}

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

    let mut settings = get_permissions_settings_impl(project_root).unwrap();
    let global = settings
        .tool_controls
        .entry("global".to_owned())
        .or_default();
    global.insert("write".to_owned(), Some(true));
    global.insert("register".to_owned(), Some(true));
    save_permissions_settings_impl(project_root, settings).unwrap();
}

fn seed_document(project_root: &std::path::Path, relative: &str, body: &str) -> String {
    let path = project_root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    relative.to_owned()
}

fn read_document(project_root: &std::path::Path, relative: &str) -> String {
    std::fs::read_to_string(project_root.join(relative)).unwrap_or_default()
}

/// 取该次运行的第一条 Pending 确认项 id。
///
/// 直接读运行态库而不走 `list_pending_confirmations_impl`：后者在 `commands.rs`
/// 里是私有的，而那个文件当前有并发改动，不值得为一个测试去动它的可见性。
/// 读的是同一张表，判据强度不变。
fn pending_confirmation_id(
    project_root: &std::path::Path,
    workflow_id: &str,
    run_id: &str,
) -> String {
    let store = SqliteWorkflowRuntimeStore::open(project_root).unwrap();
    let state = store
        .load_state(&WorkflowId::from(workflow_id), &RunId::from(run_id))
        .unwrap()
        .unwrap_or_else(|| panic!("运行 {run_id} 没有落库运行态"));
    let pending: Vec<_> = state
        .confirmations
        .values()
        .filter(|item| item.state == RuntimeConfirmationState::Pending)
        .collect();
    let first = pending.first().unwrap_or_else(|| {
        panic!(
            "运行 {run_id} 没有 Pending 确认项，实际确认项：{:?}",
            state.confirmations
        )
    });
    first.confirmation_id.clone()
}

/// 跑一个 writer 单节点工作流，返回 run_id。
fn run_writer_node(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    document_id: &str,
) -> String {
    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: "writer".to_owned(),
            name: "writer".to_owned(),
            nodes: vec![CanvasNode {
                id: "node-1".to_owned(),
                r#type: "writer".to_owned(),
                label: None,
                data: json!({
                    "provider_id": PROVIDER_ID,
                    "model_id": MODEL_ID,
                    "prompt_template": "写一段",
                    "document_id": document_id,
                    "chapter_id": "chapter-01",
                }),
                position: Value::Null,
            }],
            edges: Vec::new(),
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .expect("保存工作流应当成功");

    run_workflow_impl(
        project_root,
        secrets,
        RunWorkflowRequest {
            workflow_id: "writer".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
        },
    )
    .map(|started| started.run_id)
    .unwrap_or_else(|error| panic!("运行 writer 工作流失败：{error:?}"))
}

// ════════════════════════════════════════════════════════
// 主用例：审批通过后，正文必须真的写进磁盘
// ════════════════════════════════════════════════════════

/// **U108 阶段 3 主用例**：writer 改的正文，在用户点「同意」之后必须落到磁盘。
///
/// 判据取磁盘正文而非运行态/确认项状态——这是唯一无法被「实现完整但没接线」
/// 骗过的判据。缺陷未修时，`chapter-01.md` 在整个流程结束后与初始内容一字不差。
#[test]
fn approved_writer_patch_is_written_to_disk() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let base_url = spawn_patch_tool_llm(vec![(
        "writer-insert-lines".to_owned(),
        json!({
            "document_id": CHAPTER_PATH,
            "after_line": 1,
            "text": "夜里下起了雨。"
        })
        .to_string(),
    )]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);

    let document_id = seed_document(temp.path(), CHAPTER_PATH, "第一章\n");
    let run_id = run_writer_node(temp.path(), &secrets, &document_id);

    // 审批前：正文不应被改动（U117 的门禁语义，这里顺带钉住）。
    assert!(
        !read_document(temp.path(), CHAPTER_PATH).contains("夜里下起了雨"),
        "审批之前正文就已落盘——写作产出绕过了审批门禁"
    );

    // 找出待审确认项并同意。
    let confirmation_id = pending_confirmation_id(temp.path(), "writer", &run_id);
    resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: "writer".to_owned(),
            run_id: run_id.clone(),
            confirmation_id,
            decision: ConfirmationDecision::Approve,
            review_reason: None,
        },
    )
    .expect("同意确认项应当成功");

    let body = read_document(temp.path(), CHAPTER_PATH);
    assert!(
        body.contains("夜里下起了雨"),
        "U108 阶段 3：用户点了「同意」，但 writer 写的正文没有落到磁盘。\
         patch 在节点结束时就已随内存消失（PatchSession 生产零实例化、\
         patch_session_commit_id 生产恒为 None），审批通过时已经无物可写。\
         磁盘实际内容：{body:?}"
    );
}

/// 拒绝路径：用户点「拒绝」后，正文必须**保持原样**。
///
/// 与主用例是同一枚硬币的两面。只测同意路径的话，一个「无论决议如何都写盘」
/// 的实现会照样通过——那等于把审批做成了一道纯装饰的确认框。
#[test]
fn rejected_writer_patch_leaves_document_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let base_url = spawn_patch_tool_llm(vec![(
        "writer-insert-lines".to_owned(),
        json!({
            "document_id": CHAPTER_PATH,
            "after_line": 1,
            "text": "这段不该出现。"
        })
        .to_string(),
    )]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);

    let original = "第一章\n";
    let document_id = seed_document(temp.path(), CHAPTER_PATH, original);
    let run_id = run_writer_node(temp.path(), &secrets, &document_id);

    let confirmation_id = pending_confirmation_id(temp.path(), "writer", &run_id);
    resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: "writer".to_owned(),
            run_id,
            confirmation_id,
            decision: ConfirmationDecision::Reject,
            review_reason: Some("不要这段".to_owned()),
        },
    )
    .expect("拒绝确认项应当成功");

    assert_eq!(
        read_document(temp.path(), CHAPTER_PATH),
        original,
        "用户点了「拒绝」，正文却被改了——审批成了纯装饰"
    );
}

/// 同一节点内的**多次** patch 必须叠加，且后一次的行号基于前一次的结果。
///
/// 这是 `PatchSession` 存在的全部理由（`simulated` 字段）。若实现改成
/// 「每个工具调用各自产出一个独立 patch、依次应用」，两次插入的行号都基于
/// 原始快照，第二次就会插错位置——正文顺序被打乱，且症状随插入点位置变化，
/// 属于极难在事后定位的一类。
#[test]
fn multiple_patches_in_one_node_compose_by_simulated_line_numbers() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    // 原文三行：甲/乙/丙。
    // 第 1 次：在第 1 行后插入「A\n」  → 甲/A/乙/丙
    // 第 2 次：在第 3 行后插入「B\n」  → 甲/A/乙/B/丙
    //
    // 第 2 次的「第 3 行」必须指**第 1 次插入之后**的第 3 行（即「乙」）。
    // 若行号基于原始快照，「第 3 行」是「丙」，结果就成了 甲/A/乙/丙/B。
    // 两种结果不同，本用例即可区分——这正是 PatchSession.simulated 的判据。
    //
    // 插入文本自带 `\n`：行号 patch 按 `split_inclusive('\n')` 定位，插入点是
    // 「该行连同换行符的末尾」，不自带换行的文本会粘到下一行开头（"A乙"），
    // 那样行数不变，本用例反而丧失区分能力。
    let base_url = spawn_patch_tool_llm(vec![
        (
            "writer-insert-lines".to_owned(),
            json!({"document_id": CHAPTER_PATH, "after_line": 1, "text": "A\n"}).to_string(),
        ),
        (
            "writer-insert-lines".to_owned(),
            json!({"document_id": CHAPTER_PATH, "after_line": 3, "text": "B\n"}).to_string(),
        ),
    ]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);

    let document_id = seed_document(temp.path(), CHAPTER_PATH, "甲\n乙\n丙\n");
    let run_id = run_writer_node(temp.path(), &secrets, &document_id);

    let confirmation_id = pending_confirmation_id(temp.path(), "writer", &run_id);
    resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: "writer".to_owned(),
            run_id,
            confirmation_id,
            decision: ConfirmationDecision::Approve,
            review_reason: None,
        },
    )
    .expect("同意确认项应当成功");

    let body = read_document(temp.path(), CHAPTER_PATH);
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines,
        vec!["甲", "A", "乙", "B", "丙"],
        "同一节点内两次插入没有按模拟行号叠加。\
         期望 甲/A/乙/B/丙；若得到 甲/A/乙/丙/B，说明第二次插入的行号\
         基于原始快照而非前一次的结果（PatchSession.simulated 没有生效）。\
         磁盘实际内容：{body:?}"
    );
}

/// 中文正文必须按**字符**而非字节切分行。
///
/// 与 CLAUDE.md §3 同源：正文全是中文，多字节字符占 3 字节。若行号→字节区间的
/// 换算切在字符中间，写出来的就是乱码（或直接 panic）。这条走完整链路，
/// 而非只测 `line_ranges` 这一个函数——换算正确但拼接处理错了照样出乱码。
#[test]
fn chinese_body_survives_the_patch_write_back_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let base_url = spawn_patch_tool_llm(vec![(
        "writer-replace-lines".to_owned(),
        json!({
            "document_id": CHAPTER_PATH,
            "start_line": 2,
            "end_line": 2,
            "text": "她把伞收进门厅，雨水顺着伞骨淌了一地。"
        })
        .to_string(),
    )]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);

    let document_id = seed_document(
        temp.path(),
        CHAPTER_PATH,
        "第一章 雨夜\n旧的一行——将被替换。\n第三行保持不变。\n",
    );
    let run_id = run_writer_node(temp.path(), &secrets, &document_id);

    let confirmation_id = pending_confirmation_id(temp.path(), "writer", &run_id);
    resolve_confirmation_impl(
        temp.path(),
        ResolveConfirmationRequest {
            workflow_id: "writer".to_owned(),
            run_id,
            confirmation_id,
            decision: ConfirmationDecision::Approve,
            review_reason: None,
        },
    )
    .expect("同意确认项应当成功");

    let body = read_document(temp.path(), CHAPTER_PATH);
    assert!(
        body.contains("她把伞收进门厅，雨水顺着伞骨淌了一地。"),
        "中文正文替换后内容不完整——行号→字节换算可能切在多字节字符中间。\
         磁盘实际内容：{body:?}"
    );
    assert!(
        body.contains("第一章 雨夜") && body.contains("第三行保持不变。"),
        "替换操作破坏了相邻行。磁盘实际内容：{body:?}"
    );
}

/// 同一确认项审批两次，正文只能被改一次。
///
/// 钉住的是一个很容易漏的持久化环节：`apply_confirmed_patch` 在写盘成功后
/// 把节点标为 `Applied`，但那发生在 `commit_confirmation_resolution` **已经
/// 落库之后**，是纯内存改动。若不再存一次运行态，`Applied` 标记就丢了，
/// 而重复写回的判重恰恰依赖它（`ensure_patch_write_back_can_start`）。
///
/// 判据取磁盘正文里插入文本出现的**次数**：写两次的话会出现两遍。
/// 只断言「第二次调用返回 Err」是不够的——那不能区分「没写」和「写了但报错」。
#[test]
fn approving_the_same_confirmation_twice_writes_the_patch_once() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let base_url = spawn_patch_tool_llm(vec![(
        "writer-insert-lines".to_owned(),
        json!({
            "document_id": CHAPTER_PATH,
            "after_line": 1,
            "text": "只应出现一次的段落。\n"
        })
        .to_string(),
    )]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);

    let document_id = seed_document(temp.path(), CHAPTER_PATH, "第一章\n");
    let run_id = run_writer_node(temp.path(), &secrets, &document_id);
    let confirmation_id = pending_confirmation_id(temp.path(), "writer", &run_id);

    let request = |confirmation_id: String| ResolveConfirmationRequest {
        workflow_id: "writer".to_owned(),
        run_id: run_id.clone(),
        confirmation_id,
        decision: ConfirmationDecision::Approve,
        review_reason: None,
    };

    resolve_confirmation_impl(temp.path(), request(confirmation_id.clone()))
        .expect("第一次同意应当成功");
    // 第二次的返回值不做断言：无论它是幂等成功还是明确报错，
    // 用户可见的正确性判据都是「正文没被改第二遍」。
    let _ = resolve_confirmation_impl(temp.path(), request(confirmation_id));

    let body = read_document(temp.path(), CHAPTER_PATH);
    assert_eq!(
        body.matches("只应出现一次的段落。").count(),
        1,
        "同一确认项审批两次，patch 被重复写回。\
         多半是写盘后的 `Applied` 标记没落库——判重全靠它。\
         磁盘实际内容：{body:?}"
    );
}
