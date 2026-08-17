//! 13-B：Planner 在大纲里写的正文引用 `{{ref:文档ID#L起始-L结束}}`，
//! 必须在**送进 LLM 请求体之前**就地展开成原文。
//!
//! **为什么判据只能取「真实出站请求体」**：
//! 这一族缺陷（U108/U114/U117 与本条）都是「契约层完整 + 有测试覆盖 + 生产零调用者」。
//! 任何「能构造出一个 `ExpandedOutline`」「展开器单测通过」「装配器挂上了来源」
//! 的断言，在**装配器压根没被生产路径调用**时都照样为真。
//! 所以这里起一个真实 HTTP 端当 LLM，**捕获它收到的请求原文**，
//! 断言里面既含被引章节的正文、又不含 `{{ref:` 字面量。
//!
//! ⚠️ `{{ref:...}}` 字面量进请求体不只是「少了点上下文」，而是**安全缺口**：
//! Auto Mode 的审计 LLM 会在「审的是占位符」的前提下给出虚假通过。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    run_workflow_impl, save_provider_settings_impl, save_workflow_graph_impl, CanvasEdge,
    CanvasNode, ProviderSettingsUpdate, RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType};
use serde_json::{json, Value};

const PROVIDER_ID: &str = "primary";
const MODEL_ID: &str = "m";

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

/// 假 LLM：按轮次回放固定回答，并把**每一轮收到的请求原文**送回测试线程。
///
/// 捕获请求体是本文件的全部意义：判据落在「模型实际看到了什么」，
/// 而不是「我们相信装配器做了什么」。
fn spawn_capturing_llm(rounds: usize) -> (String, mpsc::Receiver<String>) {
    let (sender, receiver) = mpsc::channel();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    thread::spawn(move || {
        for _ in 0..rounds {
            let mut stream = accept_with_deadline(&listener, Duration::from_secs(15));
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            // 引用展开后正文会被塞进请求体，缓冲区要够大——
            // 太小会截断请求，让「请求体里没有那段正文」变成假阴性。
            let mut buffer = vec![0u8; 1_048_576];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let _ = sender.send(String::from_utf8_lossy(&buffer[..read]).to_string());

            let body = json!({
                "model": MODEL_ID,
                "choices": [{
                    "message": {"content": "好。", "tool_calls": []},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 2}
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    (base_url, receiver)
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
}

fn seed_document(project_root: &std::path::Path, relative: &str, body: &str) {
    let path = project_root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
}

/// 建一个「上游产出大纲 → writer 消费」的两节点工作流。
///
/// **必须是两节点**：引用的真实来源是**上游节点的输出**（Planner 写的大纲），
/// 它经 `resolve_llm_input_prompt` 拼进 writer 的 prompt。
/// 只用单节点把引用写死在 `prompt_template` 里也能测到展开，
/// 但那覆盖不到真实形态——而真实形态才是这条缺陷发生的地方。
fn save_two_node_workflow(project_root: &std::path::Path, upstream_outline: &str) {
    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: "chain".to_owned(),
            name: "chain".to_owned(),
            nodes: vec![
                // 上游用 `start` 节点直接产出大纲文本：不调模型，
                // 于是假 LLM 收到的第一条请求就一定是 writer 的，无需按轮次筛。
                CanvasNode {
                    id: "outline".to_owned(),
                    r#type: "start".to_owned(),
                    label: None,
                    data: json!({
                        "work_dir": "main",
                        // 字段名是 `initial_inputs`（见 nodes.rs 的
                        // `start_node_initial_outputs`）——start 节点把它逐键
                        // 转成自己的输出端口值。
                        "initial_inputs": { "prompt": upstream_outline }
                    }),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "writer".to_owned(),
                    r#type: "writer".to_owned(),
                    label: None,
                    data: json!({
                        "provider_id": PROVIDER_ID,
                        "model_id": MODEL_ID,
                        "prompt_template": "按下面的大纲写：",
                        "chapter_id": "chapter-02",
                    }),
                    position: Value::Null,
                },
            ],
            edges: vec![CanvasEdge {
                id: "e1".to_owned(),
                source: "outline".to_owned(),
                target: "writer".to_owned(),
                source_handle: "prompt".to_owned(),
                target_handle: "prompt".to_owned(),
                kind: ariadne::contracts::WorkflowEdgeKind::Data,
                label: None,
                data: Value::Null,
            }],
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .expect("保存工作流应当成功");
}

fn run_chain(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
) -> Result<String, String> {
    run_workflow_impl(
        project_root,
        secrets,
        RunWorkflowRequest {
            workflow_id: "chain".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
        },
    )
    .map(|started| started.status)
    .map_err(|error| format!("{error:?}"))
}

// ════════════════════════════════════════════════════════
// 主用例：引用必须在进请求体之前变成原文
// ════════════════════════════════════════════════════════

/// **13-B 主用例**：上游大纲里的 `{{ref:...}}` 在 LLM 请求体里已是原文。
///
/// 三条断言各自不可省：
/// - 请求体含被引章节的**原文特征串** ⇒ 展开真的发生了
/// - 请求体**不含** `{{ref:` ⇒ 没有任何占位符漏进去
/// - 请求体含上游大纲的其余文字 ⇒ 展开是**就地替换**而不是把大纲整段丢了
///
/// 缺陷版本（装配层完整但生产没接线）下第一条与第二条同时失败：
/// prompt 里 `{{ref:...}}` 原样存在，而 `render_prompt_template` 随后
/// 会把它当未知变量报错 ⇒ 节点直接失败。
#[test]
fn upstream_outline_references_reach_the_model_as_real_prose() {
    let temp = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();
    let (base_url, requests) = spawn_capturing_llm(1);
    provision(temp.path(), &secrets, base_url);

    // 被引章节：正文里放一个不会偶然出现的特征串。
    seed_document(
        temp.path(),
        "chapters/chapter-01.md",
        "第一行\n阿宁在雨里站了很久，伞早就丢了。\n第三行\n",
    );

    // 上游大纲：引用第 2 行，前后都有别的文字。
    save_two_node_workflow(
        temp.path(),
        "本章要接住上一章的雨。\n{{ref:chapters/chapter-01.md#L2-L2}}\n注意保持同样的克制。",
    );

    let status = run_chain(temp.path(), &secrets).expect("运行应当成功");
    assert_eq!(status, "succeeded", "运行未成功，无法判断请求体内容");

    let body = requests
        .recv_timeout(Duration::from_secs(15))
        .expect("假 LLM 没有收到任何请求——引用展开之前节点就失败了");

    assert!(
        body.contains("伞早就丢了"),
        "请求体里没有被引章节的原文 ⇒ 引用没有被展开（13-B 生产未接线）。\n请求体：{body}"
    );
    assert!(
        !body.contains("{{ref:"),
        "请求体里仍有 {{{{ref:}} 字面量 ⇒ 占位符漏进了 LLM 请求。\
         这不只是少了上下文：Auto Mode 的审计 LLM 会在「审的是占位符」的前提下\
         给出虚假通过。\n请求体：{body}"
    );
    assert!(
        body.contains("注意保持同样的克制"),
        "上游大纲的其余文字不见了 ⇒ 展开不是就地替换。\
         引用服务于大纲里的具体指示，必须与指示保持相邻。\n请求体：{body}"
    );
}

/// 引用指向**不存在的文档**时，不能静默、也不能把占位符漏出去。
///
/// 这是最常见的失效形态（大纲写错了章节名）。三种处理里只有一种可接受：
/// - ❌ 原样放过 → 占位符进请求体
/// - ❌ 静默删掉 → 写作者以为那条指示后面本来就没有原文，人也看不出少了什么
/// - ✅ 换成可诊断标记 → 模型知道「这里本该有原文但取不到」
///
/// 判据取「请求体里没有 `{{ref:` 且有可辨识的缺失标记」，
/// 而不是「运行失败」——一条引用写错不该让整章写作停摆。
#[test]
fn missing_referenced_document_becomes_a_diagnosable_marker() {
    let temp = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();
    let (base_url, requests) = spawn_capturing_llm(1);
    provision(temp.path(), &secrets, base_url);

    save_two_node_workflow(
        temp.path(),
        "参考这段：\n{{ref:chapters/does-not-exist.md#L1-L5}}\n照它的语气写。",
    );

    let status = run_chain(temp.path(), &secrets).expect("引用失效不应让整次运行报错");
    assert_eq!(status, "succeeded");

    let body = requests
        .recv_timeout(Duration::from_secs(15))
        .expect("假 LLM 没有收到请求");

    assert!(
        !body.contains("{{ref:"),
        "引用失效时占位符仍漏进请求体：{body}"
    );
    assert!(
        body.contains("照它的语气写"),
        "大纲其余部分必须保留：{body}"
    );
}

/// 路径逃出文档根 ⇒ **整次执行 fail-loud**，不降级成「文档不存在」。
///
/// ⚠️ 这两种情形必须区别对待：
/// - 文档不存在 = 大纲写错了章节名，属正常失效 ⇒ 标记 + 继续
/// - 路径越权 = 有人在尝试读项目外的文件 ⇒ **安全事件，必须 fail-loud**
///
/// 把越权降级成「文档不存在」会让越权尝试看起来只是一条无效引用，
/// 掩盖真实的安全事件。
///
/// ⚠️ **判据必须是「恰好等于 failed」，不能写成 `assert_ne!(status, "succeeded")`。**
/// 首版就是后者，而变异测试证明它是**空测**：把沙箱校验降级成
/// `if ...is_err() { continue; }` 后，运行返回 `queued`（引用被跳过、
/// 节点转入异步执行）——`assert_ne!` 对 `queued` 同样成立，用例照样绿。
/// 「不是成功」有两种：一种是被安全检查拒了，一种是压根还没跑完。
/// 只有前者是本用例要证明的性质。
#[test]
fn reference_escaping_the_document_root_fails_loudly() {
    let temp = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();
    // 越权应当在读盘阶段就被拒，模型压根不会被调用 ⇒ 不需要假 LLM 回任何东西。
    let (base_url, _requests) = spawn_capturing_llm(0);
    provision(temp.path(), &secrets, base_url);

    save_two_node_workflow(
        temp.path(),
        "参考这段：\n{{ref:../../../etc/passwd}}\n照它的语气写。",
    );

    let outcome = run_chain(temp.path(), &secrets);
    match outcome {
        // run_workflow_impl 对失败运行返回 Ok(status="failed") 而非 Err，两者都算 fail-loud。
        Ok(status) => assert_eq!(
            status, "failed",
            "引用逃出文档根必须让本次执行**当场失败**。\n\
             拿到 `{status}` 说明沙箱没有 fail-loud：`queued` 意味着越权引用被\
             静默跳过、节点照常转入异步执行，那正是「把安全事件降级成一条无效引用」。"
        ),
        Err(_) => {}
    }

    // 对照组：**合法但不存在**的文档不该让运行失败。
    // 没有这一条，上面那个 `assert_eq!(failed)` 可能只是因为「引用一律让运行失败」，
    // 而不是因为「越权被拦住了」——那两者的产品含义完全不同。
    let plain = tempfile::tempdir().unwrap();
    let plain_secrets = MemorySecretStore::default();
    let (plain_url, _plain_requests) = spawn_capturing_llm(1);
    provision(plain.path(), &plain_secrets, plain_url);
    save_two_node_workflow(
        plain.path(),
        "参考这段：\n{{ref:chapters/never-written.md}}\n照它的语气写。",
    );
    let plain_outcome = run_chain(plain.path(), &plain_secrets).expect("引用失效不该报 Err");
    assert_ne!(
        plain_outcome, "failed",
        "合法但不存在的引用被当成了错误——那是大纲写错章节名的正常情形，\
         应当标记为失效并继续，不该让整章写作停摆"
    );
}
