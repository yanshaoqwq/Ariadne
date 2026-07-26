//! 生产流程端到端契约（2026-07-26 配置项审查）。
//!
//! 本文件不做逐字段单测，而是**完整模拟用户的真实生产流程**：
//!
//! ```text
//! 新建项目 → 配置 Provider（真实 HTTP mock）→ 设为默认路由 → 建工作流
//!   → 点运行 → 断言真的产出了内容
//! ```
//!
//! 判定标准是「用户能不能靠这套配置把稿子写出来」，而非某个函数返回值是否正确。
//! 用例失败即表示生产链路在该环节断裂。
//!
//! 分析见 `项目检验报告/发布前全量代码审查/13-配置项存在性与执行链路阻断审查.md`。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    run_workflow_impl, save_provider_settings_impl, save_workflow_graph_impl, CanvasNode,
    ProviderSettingsUpdate, RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType};
use serde_json::{json, Value};

// ————————————————————————————————————————————————
// 测试基建
// ————————————————————————————————————————————————

/// 在超时前接受一个本地 HTTP 连接，避免测试永久挂起。
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

/// 启动一个假 LLM：按轮次返回预设响应，并把收到的请求原文交回给调用方断言。
///
/// 返回 (base_url, join_handle)，handle 结束后可取出每一轮的请求原文。
fn spawn_fake_llm(responses: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for body in responses {
            let mut stream = accept_with_deadline(&listener, Duration::from_secs(10));
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0u8; 262_144];
            let read = stream.read(&mut buffer).unwrap();
            seen.push(String::from_utf8_lossy(&buffer[..read]).into_owned());

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        }
        seen
    });
    (base_url, handle)
}

/// 构造一个普通的 chat 完成响应（无工具调用）。
fn chat_response(model: &str, content: &str) -> String {
    json!({
        "model": model,
        "choices": [{
            "message": {"content": content, "tool_calls": []},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 20, "completion_tokens": 8}
    })
    .to_string()
}

/// Provider 在生产流程中的固定身份，供节点 `data` 显式声明 `provider_id`/`model_id`
/// （画布节点必须指名路由，不能只填 prompt 就指望落到某个默认模型）。
const PRIMARY_PROVIDER_ID: &str = "primary";

/// 完成「新建项目 + 配好一个可用的默认 LLM Provider + 存好密钥」这几步。
///
/// `secrets` 必须与调用方后续传给 `run_workflow_impl` 的是**同一个实例**——
/// `MemorySecretStore` 只存在于内存，两个独立实例互不可见。
fn provision_project_with_llm(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    base_url: String,
    model_id: &str,
    capability: ProviderCapability,
) {
    ariadne::frontend::initialize_project(project_root).unwrap();

    save_provider_settings_impl(
        project_root,
        ProviderSettingsUpdate {
            provider_id: PRIMARY_PROVIDER_ID.to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Primary".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: model_id.to_owned(),
                capability,
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
        .set_provider_secret(PRIMARY_PROVIDER_ID, SecretValue::new("test-secret"))
        .unwrap();
}

/// 保存一张只含单个节点的工作流图。
fn save_single_node_workflow(
    project_root: &std::path::Path,
    workflow_id: &str,
    node_type: &str,
    data: Value,
) {
    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: workflow_id.to_owned(),
            name: workflow_id.to_owned(),
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
    .unwrap();
}

// ————————————————————————————————————————————————
// 生产流程 1：最小可用链路——配 Provider 后能跑通一个 LLM 节点
// ————————————————————————————————————————————————

/// 这是整个产品的地基：用户配好一个 Provider，建一个 LLM 节点，点运行，应当成功。
/// 若此用例失败，说明连最基本的生产链路都不通。
#[test]
fn production_flow_minimal_llm_node_runs_to_success() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm(vec![chat_response("chat-1", "第一章：风起于青萍之末。")]);
    let secrets = MemorySecretStore::default();
    provision_project_with_llm(
        temp.path(),
        &secrets,
        base_url,
        "chat-1",
        ProviderCapability::Llm,
    );

    save_single_node_workflow(
        temp.path(),
        "minimal",
        "llm",
        json!({
            "provider_id": PRIMARY_PROVIDER_ID,
            "model_id": "chat-1",
            "prompt_template": "写第一章的开头"
        }),
    );

    let run = run_workflow_impl(
        temp.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "minimal".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
        },
    );
    let _ = server.join();

    let run = run.expect("最小生产链路：配好 Provider 的 LLM 节点必须能运行");
    assert_eq!(
        run.status, "succeeded",
        "最小生产链路运行未成功：{:?}",
        run.status
    );
}

// ————————————————————————————————————————————————
// 生产流程 2：U107——用户按 UI 选择 tool_use 模型后能否走完全程
// ————————————————————————————————————————————————

/// 设置页能力下拉框第 2 项是「工具调用」(`tool_use`)，且写作类节点本就需要工具调用。
/// 用户照此配置后，必须能保存并跑通工作流。
///
/// 当前保存边界 `validate_provider_model_role` 拒绝 tool_use，
/// 因此本用例会在**第一步配置**就失败——用户根本走不到运行。
#[test]
fn production_flow_tool_use_model_completes_whole_journey() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm(vec![chat_response("tool-chat", "已按工具调用完成。")]);

    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let save_result = save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: "tooling".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Tooling".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            // 用户在 UI 上选了「工具调用」这一项。
            models: vec![ModelConfig {
                model_id: "tool-chat".to_owned(),
                capability: ProviderCapability::ToolUse,
                max_context_tokens: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
            }],
            make_default_llm: true,
            make_default_embedding: false,
            make_default_reranker: false,
            make_default_search: false,
        },
    );

    assert!(
        save_result.is_ok(),
        "U107：用户在能力下拉框选择「工具调用」后无法保存 Provider，\
         生产流程在第一步即中断：{:?}",
        save_result.err()
    );

    let secrets = MemorySecretStore::default();
    ProjectCredentialScope::new(temp.path(), &secrets)
        .unwrap()
        .set_provider_secret("tooling", SecretValue::new("test-secret"))
        .unwrap();

    save_single_node_workflow(
        temp.path(),
        "tooling-flow",
        "llm",
        json!({
            "provider_id": "tooling",
            "model_id": "tool-chat",
            "prompt_template": "写一段"
        }),
    );
    let run = run_workflow_impl(
        temp.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "tooling-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
        },
    );
    let _ = server.join();

    let run = run.expect("tool_use 模型应能驱动工作流");
    assert_eq!(run.status, "succeeded");
}

// ————————————————————————————————————————————————
// 生产流程 3：U108——写作节点能否真的把内容写进文档
// ————————————————————————————————————————————————

/// 这是本产品的核心价值主张：跑一个 writer 节点，稿子要真的落到 documents/ 里。
///
/// 当前模型节点的生产装配（`commands.rs` 的 `workflow_node_search_bindings`）
/// 只会给节点接上 `project-search` / `web-search` 两种工具，
/// Module 9 的 `writer-insert-lines` 等写入工具从未下发给 LLM，
/// 因此模型即便想写也没有可用的写入手段。
///
/// 断言方式：检查 ariadne 发给 LLM 的**出站请求**里，工具声明列表是否包含
/// `writer-insert-lines`。这与假 LLM 如何应答无关——只要产品代码把工具清单
/// 序列化进了请求体，这里就能测到；反之则证明该工具从未被提供给模型。
#[test]
fn production_flow_writer_node_offers_write_tool_to_the_model() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let chapter_path = temp.path().join("documents/chapter-01.md");
    std::fs::create_dir_all(chapter_path.parent().unwrap()).unwrap();
    std::fs::write(&chapter_path, "# 第一章\n").unwrap();

    let (base_url, server) =
        spawn_fake_llm(vec![chat_response("writer-chat", "第一章开头写好了。")]);
    let secrets = MemorySecretStore::default();
    provision_project_with_llm(
        temp.path(),
        &secrets,
        base_url,
        "writer-chat",
        ProviderCapability::Llm,
    );

    save_single_node_workflow(
        temp.path(),
        "writing-flow",
        "writer",
        json!({
            "provider_id": PRIMARY_PROVIDER_ID,
            "model_id": "writer-chat",
            "prompt_template": "把开头写进第一章"
        }),
    );

    let run = run_workflow_impl(
        temp.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "writing-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
        },
    );
    let requests = server.join().unwrap_or_default();
    let outbound_request = requests.first().cloned().unwrap_or_default();

    // U108 的核心断言：写作节点连一个写入工具都没有下发给模型。
    assert!(
        outbound_request.contains("writer-insert-lines")
            || outbound_request.contains("writer-replace-lines"),
        "U108：writer 节点发给 LLM 的工具声明中没有任何写入工具（writer-insert-lines / \
         writer-replace-lines），模型即便想写也无从下笔。实际出站请求片段：{}",
        outbound_request.chars().take(800).collect::<String>()
    );

    run.expect("writer 节点本身（不含写入工具时的纯文本生成）应能运行成功");
}

// ————————————————————————————————————————————————
// 生产流程 4：U113——全局超时/循环上限对真实运行是否有约束力
// ————————————————————————————————————————————————

/// 用户把全局「最大循环轮次」设为 2，工作流里的 loop 节点却声明 50 轮。
/// 全局上限若有约束力，运行应当被限制或直接拒绝。
///
/// 当前 `max_loop_iterations` 的唯一可达路径 `validate_loop_policy` 零调用者，
/// 运行时只认节点自带的 `policy.max_iterations`，故全局护栏形同虚设——
/// 对按次计费的 LLM 应用，这是成本失控风险。
#[test]
fn production_flow_global_loop_limit_constrains_runaway_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    // 用户在设置页把全局循环上限收紧到 2。
    let store = ariadne::config::ConfigStore::new(temp.path());
    let mut config = store.load_or_create().unwrap();
    config.workflow.max_loop_iterations = 2;
    store.save(&config).unwrap();

    // 工作流里的 loop 节点却要求 50 轮（`LoopNodeConfig` 字段是平铺的，
    // 不是嵌套在 "policy" 键下——见 workflow/nodes.rs 的 LoopNodeConfig）。
    save_single_node_workflow(
        temp.path(),
        "runaway",
        "loop",
        json!({
            "max_iterations": 50,
            "timeout_ms": 60_000,
            "stop_condition": {"kind": "manual"}
        }),
    );

    let run = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        RunWorkflowRequest {
            workflow_id: "runaway".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
        },
    );

    // 全局上限若生效，越界的 loop policy 应在预检或运行时被拒绝。
    assert!(
        run.is_err(),
        "U113：全局最大循环轮次设为 2，工作流声明 50 轮却被放行，\
         全局成本护栏未接线（`validate_loop_policy` 零调用者）"
    );
}
