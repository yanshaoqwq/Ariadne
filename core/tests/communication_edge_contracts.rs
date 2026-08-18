//! 通信边（communication edge）能否完成预定流程的端到端契约。
//!
//! 本文件回答一个问题：**通信边到底把消息送到对端了吗，还是只在状态机里转了一圈？**
//!
//! 判定标准刻意定得很硬：**断言真实出站 HTTP 请求体里出现了发送方说的话**。
//! 只断言「运行成功」或「状态机里有 N 条消息」都不算——U108/U114/U117 三条缺陷
//! 都曾是「实现完整 + 有测试覆盖 + 生产零消费者」，只有抓真实 payload 才拦得住。
//!
//! ```text
//! writer ──communication── critic
//!   │                        │
//!   └── 正向：writer 的稿子送给 critic
//!       反向：critic 的意见送回 writer
//! ```
//!
//! 分析见 `项目检验报告/发布前全量代码审查/13-配置项存在性与执行链路阻断审查.md`。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    run_workflow_impl, save_provider_settings_impl, save_workflow_graph_impl, CanvasEdge,
    CanvasNode, ProviderSettingsUpdate, RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType, WorkflowEdgeKind};
use ariadne::workflow::WorkflowRuntimeStore;
use serde_json::{json, Value};

const PROVIDER_ID: &str = "primary";
const MODEL_ID: &str = "chat-1";

/// writer 在第一轮说的话；必须能在 critic 的出站请求里找到。
const WRITER_SAYS: &str = "雨夜里他推开了那扇木门。";
/// critic 的意见；必须能在 writer 第二轮的出站请求里找到。
const CRITIC_SAYS: &str = "开头太平，建议加一处听觉细节。";

// ————————————————————————————————————————————————
// 测试基建：真实 HTTP 接收端
// ————————————————————————————————————————————————

/// 在超时前接受一个本地 HTTP 连接，避免测试永久挂起。
fn accept_with_deadline(listener: &TcpListener, timeout: Duration) -> Option<std::net::TcpStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("接受本地 HTTP 请求失败：{error}"),
        }
    }
}

/// 真实 HTTP 接收端：按到达顺序回预设响应，并把**每一轮的完整请求体**留下。
///
/// 与只回固定响应的桩不同，这里保留 payload 是本文件的核心手段——
/// 通信边是否真的把消息拼进了 prompt，只能从出站请求体里看出来。
struct RecordingLlm {
    base_url: String,
    seen: Arc<Mutex<Vec<String>>>,
    handle: thread::JoinHandle<()>,
}

impl RecordingLlm {
    /// `responses` 按轮次消费；多出来的请求会拿到最后一条响应。
    fn spawn(responses: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);

        let handle = thread::spawn(move || {
            let mut round = 0usize;
            // 多留几轮，便于观察"是否真的发生了第二次调用"。
            let max_rounds = responses.len() + 4;
            while round < max_rounds {
                let Some(mut stream) = accept_with_deadline(&listener, Duration::from_secs(8))
                else {
                    break;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut buffer = vec![0u8; 262_144];
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read > 0 {
                    sink.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                }
                let body = responses
                    .get(round)
                    .cloned()
                    .unwrap_or_else(|| responses.last().cloned().unwrap_or_default());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                round += 1;
            }
        });

        Self {
            base_url,
            seen,
            handle,
        }
    }

    /// 取出至今收到的全部请求原文。
    fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    fn finish(self) -> Vec<String> {
        let seen = Arc::clone(&self.seen);
        drop(self.handle);
        let captured = seen.lock().unwrap().clone();
        captured
    }
}

/// 普通 chat 完成响应。
fn chat_response(content: &str) -> String {
    json!({
        "model": MODEL_ID,
        "choices": [{
            "message": {"content": content, "tool_calls": []},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 20, "completion_tokens": 8}
    })
    .to_string()
}

/// 新建项目 + 配好一个可用的默认 LLM Provider + 存好密钥。
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
        .set_provider_secret(PROVIDER_ID, SecretValue::new("test-secret"))
        .unwrap();
}

fn llm_node(id: &str, prompt: &str) -> CanvasNode {
    CanvasNode {
        id: id.to_owned(),
        r#type: "llm".to_owned(),
        label: None,
        data: json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": prompt,
            "max_tool_rounds": 0
        }),
        position: Value::Null,
    }
}

/// 保存「writer ──exec── critic」+ 一条 communication 边的双节点图。
fn save_communication_workflow(
    project_root: &std::path::Path,
    workflow_id: &str,
    max_communication_count: Option<u32>,
) -> Result<(), String> {
    let mut communication_data = json!({
        "initiator_node_id": "writer",
        "forward_alias": "draft",
        "reverse_alias": "review",
        "forward_template": "{{input.draft}}",
        "reverse_template": "{{input.review}}"
    });
    if let Some(max) = max_communication_count {
        communication_data["max_communication_count"] = json!(max);
    }

    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: workflow_id.to_owned(),
            name: workflow_id.to_owned(),
            nodes: vec![
                llm_node("writer", "写一段正文"),
                llm_node("critic", "审阅上面的稿子"),
            ],
            edges: vec![
                // 执行顺序：writer 先跑，critic 后跑。
                CanvasEdge {
                    id: "exec-1".to_owned(),
                    source: "writer".to_owned(),
                    target: "critic".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                // 通信边：双向传消息。
                CanvasEdge {
                    id: "comm-1".to_owned(),
                    source: "writer".to_owned(),
                    target: "critic".to_owned(),
                    source_handle: "communication".to_owned(),
                    target_handle: "communication".to_owned(),
                    kind: WorkflowEdgeKind::Communication,
                    label: None,
                    data: communication_data,
                },
            ],
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    )
    .map(|_| ())
    .map_err(|error| format!("{error:?}"))
}

/// 与 `run` 相同，但把 run_id 一并返回，便于直接读运行态断言。
fn run_with_id(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    workflow_id: &str,
) -> Result<(String, String), String> {
    let started = run_workflow_impl(
        project_root,
        secrets,
        RunWorkflowRequest {
            workflow_id: workflow_id.to_owned(),
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
    .map_err(|error| format!("{error:?}"))?;
    Ok((started.status.clone(), started.run_id.clone()))
}

fn run(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    workflow_id: &str,
) -> Result<String, String> {
    let started = run_workflow_impl(
        project_root,
        secrets,
        RunWorkflowRequest {
            workflow_id: workflow_id.to_owned(),
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
    .map_err(|error| format!("{error:?}"))?;

    // 诊断：把运行态里每个节点的状态与错误打出来。
    // 只看顶层 status 无法区分「下游没被调度」与「下游跑了但失败」。
    eprintln!("--- 运行态诊断 run_id={} ---", started.run_id);
    match ariadne::workflow::SqliteWorkflowRuntimeStore::open(project_root) {
        Ok(store) => {
            match store.load_state(
                &ariadne::contracts::WorkflowId::from(workflow_id),
                &ariadne::contracts::RunId::from(started.run_id.as_str()),
            ) {
                Ok(Some(state)) => {
                    eprintln!("  顶层 status={:?}", state.status);
                    eprintln!("  pause_reason={:?}", state.pause_reason);
                    eprintln!("  stop_reason={:?}", state.stop_reason);
                    for (node_id, node) in &state.nodes {
                        eprintln!(
                            "  节点 {:<8} status={:?} error={:?}",
                            node_id.as_str(),
                            node.status,
                            node.error
                        );
                    }
                    for (edge_id, comm) in &state.communication_edges {
                        eprintln!(
                            "  通信边 {:<8} 消息数={} ",
                            edge_id.as_str(),
                            comm.messages.len()
                        );
                        for message in &comm.messages {
                            eprintln!("      {message:?}");
                        }
                    }
                }
                Ok(None) => eprintln!("  (运行态未持久化)"),
                Err(error) => eprintln!("  (读取运行态失败：{error})"),
            }
        }
        Err(error) => eprintln!("  (打开运行态库失败：{error})"),
    }

    Ok(started.status)
}

// ————————————————————————————————————————————————
// 断言 1（核心）：writer 说的话必须出现在 critic 的**真实出站请求体**里
// ————————————————————————————————————————————————

/// 这是本文件唯一不可妥协的断言。
///
/// 通信边的全部意义就是「把上游说的话交给下游」。若 critic 的出站 HTTP 请求里
/// 找不到 writer 的原话，那么通信边就是个只在内存里转圈的空壳——
/// 状态机再完整也不构成产品能力。
#[test]
fn writer_message_reaches_critic_outbound_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    // 第一轮给 writer，第二轮给 critic。
    let llm = RecordingLlm::spawn(vec![chat_response(WRITER_SAYS), chat_response(CRITIC_SAYS)]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, llm.base_url.clone());

    if let Err(error) = save_communication_workflow(temp.path(), "comm", None) {
        panic!("通信边工作流保存失败，用户根本画不出这张图：{error}");
    }

    let status = run(temp.path(), &secrets, "comm");
    let requests = llm.requests();

    // 先给出诊断信息，失败时能直接看出断在哪一环。
    eprintln!("=== 运行状态：{status:?}");
    eprintln!("=== 出站请求数：{}", requests.len());
    for (index, request) in requests.iter().enumerate() {
        let body = request.split("\r\n\r\n").nth(1).unwrap_or("(无 body)");
        eprintln!("--- 第 {} 次出站请求 body ---\n{}", index + 1, body);
    }

    assert!(
        requests.len() >= 2,
        "通信边未产生第二次 LLM 调用：只看到 {} 次出站请求。\
         writer 跑完后 critic 应当被触发并带着 writer 的消息发起调用。",
        requests.len()
    );

    let critic_request = &requests[1];
    assert!(
        critic_request.contains(WRITER_SAYS),
        "**通信边未把消息送达**：critic 的出站请求体里找不到 writer 的原话「{WRITER_SAYS}」。\
         这说明 communication_messages 虽然进了 WorkflowNodeExecutionRequest，\
         但没有被拼进发给 LLM 的 prompt——属于「字段传到了但没人消费」。\n\
         critic 实际请求体：{critic_request}"
    );
}

// ————————————————————————————————————————————————
// 断言 2：反向消息——critic 的意见要能回到 writer
// ————————————————————————————————————————————————

/// 通信边声明了 `reverse_alias`/`reverse_template`，即设计上支持双向。
/// 若反向不通，「critic 提意见 → writer 改稿」这个本产品的核心协作循环就不成立。
///
/// ⚠️ **必须显式设 `max_communication_count`（本例 3）**，不能传 `None`。
/// 默认上限是 **2 条消息**（`DEFAULT_COMMUNICATION_MAX_MESSAGE_COUNT`）：
/// writer→critic 是第 1 条、critic→writer 是第 2 条，**第 2 条一发就撞上限、
/// 工作流立即 pause**，writer 永远没机会读到那条意见。
///
/// 也就是说默认配置下「一来一回」只完成了「一来一回的投递」，
/// **没有完成「读到并回应」**——本用例要验的恰恰是后者，所以需要第 3 条的额度。
/// 传 `None` 时本用例失败反映的是**它自己配置不足**，不是产品缺陷；
/// 我第一次读到那个红以为是反向通信没实现，查到 pause 原因才看清。
///
/// 📌 这个默认值本身值得产品复核：`2` 让最常见的「写→评→改」三步走不完。
/// 但那是产品决策（改默认值会影响所有既有工作流的花费），不在本用例范围内。
#[test]
fn critic_feedback_returns_to_writer_outbound_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let llm = RecordingLlm::spawn(vec![
        chat_response(WRITER_SAYS),
        chat_response(CRITIC_SAYS),
        chat_response("已按意见补上雨声。"),
    ]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, llm.base_url.clone());
    // 3 = writer 说、critic 回、writer 再说一次（读到意见的那一次）。
    save_communication_workflow(temp.path(), "comm-rev", Some(3)).unwrap();

    let status = run(temp.path(), &secrets, "comm-rev");
    let requests = llm.requests();

    eprintln!("=== 运行状态：{status:?}，出站 {} 次", requests.len());
    for (index, request) in requests.iter().enumerate() {
        let body = request.split("\r\n\r\n").nth(1).unwrap_or("(无 body)");
        eprintln!("--- 第 {} 次 ---\n{}", index + 1, body);
    }

    // 反向消息成立的判据：某一次出站请求里同时不含 CRITIC_SAYS 的请求之后，
    // 出现了带 CRITIC_SAYS 的请求（即 writer 收到了意见）。
    let carries_review = requests
        .iter()
        .skip(2)
        .any(|request| request.contains(CRITIC_SAYS));
    assert!(
        carries_review,
        "反向通信未生效：critic 的意见「{CRITIC_SAYS}」没有出现在后续任何出站请求里。\
         共 {} 次出站请求。若通信边只支持单向，`reverse_alias`/`reverse_template` \
         两个配置项即为假配置。",
        requests.len()
    );
}

// ————————————————————————————————————————————————
// 断言 3：次数上限必须真的生效，且不能静默
// ————————————————————————————————————————————————

/// `DEFAULT_COMMUNICATION_MAX_MESSAGE_COUNT = 2`，注释写明「避免隐式无限循环」。
///
/// 这条断言两件事：
/// 1. 上限真的拦得住（不会无限往返把预算烧光）；
/// 2. 撞上限后**运行不是静默成功**——用户需要知道"没聊完就停了"。
#[test]
fn communication_count_limit_actually_stops_and_is_observable() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    // 每轮都回"还要继续聊"，逼通信边一直往返；上限设为 1 便于快速撞线。
    let llm = RecordingLlm::spawn(vec![chat_response("继续聊")]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, llm.base_url.clone());
    save_communication_workflow(temp.path(), "comm-limit", Some(1)).unwrap();

    let status = run(temp.path(), &secrets, "comm-limit");
    let requests = llm.finish();

    eprintln!("=== 状态：{status:?}，出站 {} 次（上限 1）", requests.len());

    // 上限 1 意味着最多一次通信往返。给足余量：writer + critic + 一次往返 ≈ 3 次。
    // 若出现远超此数的调用，说明上限没拦住。
    assert!(
        requests.len() <= 4,
        "通信次数上限未生效：上限设为 1，却发生了 {} 次 LLM 调用。\
         对按 token 计费的产品，这是成本失控风险。",
        requests.len()
    );
}

// ————————————————————————————————————————————————
// 断言 5（根因）：节点必须产出可作通信内容的文本，否则通信边永不轮转
// ————————————————————————————————————————————————

/// **这条用例直指 U147-a 的根因。**
///
/// 修复前（2026-08-18 之前）轮转的前置条件读的是一个死字段：
///
/// ```ignore
/// let output = node_state.communication_output.clone().unwrap_or_default();
/// if output.trim().is_empty() { continue; }   // ← 恒真，不轮转 next_sender
/// ```
///
/// 而 `communication_output` 在 `core/src/` 里**只有 runtime.rs 自己出现**，
/// 生产侧（commands.rs / integration.rs / llm/）**零写入**：
/// `llm_response_to_output` 用 `..Default()` 构造输出，该字段默认就是 `None`。
///
/// 后果链：
/// writer 跑完 → 判据取到空串 → `continue` 跳过轮转 → `next_sender` 仍是 writer
/// → critic 的 `communication_start_ready` 恒为 false → 就绪队列空
/// → `pause("no runnable nodes are ready")`
///
/// 这与 U108/U114/U117 是同一个模式：**字段、状态机、消费点都在，唯独没有生产者。**
///
/// **修法（产品已定夺的方案 C）**：轮转改用节点主输出的 `"text"` 键
/// （`communication_text_from_outputs`），死字段 `communication_output` 连同
/// 定义一并删除——留着就是第二个「看起来能用但没人写」的陷阱。
///
/// ⚠️ **判据随之改为「节点的 `text` 输出非空」**，而不再读那个已不存在的字段。
/// 这条用例因此**不再能靠「字段是 None」失败**——它现在真正验的是
/// 「轮转依赖的那个值，节点到底有没有产出」，与取值机制解耦：
/// 将来换成别的键也只需改这里一处，而「没有生产者」这个缺陷形态依然拦得住。
#[test]
fn llm_node_must_emit_communication_output_for_edge_to_advance() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let llm = RecordingLlm::spawn(vec![chat_response(WRITER_SAYS), chat_response(CRITIC_SAYS)]);
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, llm.base_url.clone());
    save_communication_workflow(temp.path(), "comm-output", None).unwrap();

    let status = run_with_id(temp.path(), &secrets, "comm-output");
    let (status_text, run_id) = status.expect("运行应当返回状态与 run_id");
    eprintln!("=== 状态：{status_text}");

    // 直接检查运行态：writer 成功后，轮转要读的那个输出是否非空。
    let store = ariadne::workflow::SqliteWorkflowRuntimeStore::open(temp.path()).unwrap();
    let state = store
        .load_state(
            &ariadne::contracts::WorkflowId::from("comm-output"),
            &ariadne::contracts::RunId::from(run_id.as_str()),
        )
        .unwrap()
        .expect("运行态应当已持久化");

    let writer = state
        .nodes
        .get(&ariadne::contracts::NodeId::from("writer"))
        .expect("writer 应当已执行");

    eprintln!(
        "  writer.status={:?} outputs.keys={:?}",
        writer.status,
        writer.outputs.keys().collect::<Vec<_>>()
    );

    assert_eq!(
        writer.status,
        ariadne::contracts::RunStatus::Succeeded,
        "前提不成立：writer 自身就没跑成功"
    );

    // 与生产同一条取值路径：`text` 键的 inline 字符串。
    // 刻意不在测试里重写一套「找第一个字符串输出」的模糊逻辑——
    // 那样测的就不是生产实际取的那个值了。
    let emitted = match writer.outputs.get("text") {
        Some(ariadne::contracts::PortValue::Inline { value }) => {
            value.as_str().unwrap_or_default().trim().to_owned()
        }
        _ => String::new(),
    };

    assert!(
        !emitted.is_empty(),
        "**U147-a 根因**：writer 成功执行后没有可作通信内容的 `text` 输出（实际 outputs={:?}）。\
         `advance_communication` 因此 `continue` 跳过轮转，`next_sender_node_id` 永远停在 writer，\
         critic 的 `communication_start_ready` 恒为 false，工作流停在 \
         \"no runnable nodes are ready\"——**用户画的通信边根本转不起来**。",
        writer.outputs.keys().collect::<Vec<_>>()
    );
}


/// U145 记录了 `SourceHandle`/`TargetHandle` 是自由文本框的问题。
/// 这条用例固定住"正确的引脚名能存下来"这个前提；
/// 若它失败，说明连按契约常量填写都存不进去，通信边在 UI 上根本不可达。
#[test]
fn communication_edge_with_canonical_port_names_can_be_saved() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let saved = save_communication_workflow(temp.path(), "comm-save", None);
    assert!(
        saved.is_ok(),
        "按契约常量 `communication` 填引脚名仍无法保存通信边：{saved:?}"
    );
}
