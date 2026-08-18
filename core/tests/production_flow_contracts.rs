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
    run_workflow_impl, save_provider_settings_impl, save_workflow_graph_impl, CanvasEdge,
    CanvasNode, ProviderSettingsUpdate, RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType, WorkflowEdgeKind};
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
            variables: Default::default(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
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
            variables: Default::default(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
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
    // 写入类工具出厂即关（`global.write = false`）。用户要让写作节点动笔，
    // 必须先在权限页打开——这一步是产品设定，不是测试脚手架。
    enable_write_tools(temp.path());

    save_single_node_workflow(
        temp.path(),
        "writing-flow",
        "writer",
        json!({
            "provider_id": PRIMARY_PROVIDER_ID,
            "model_id": "writer-chat",
            "prompt_template": "把开头写进第一章",
            // 写作节点必须指名要编辑的文档：行号 patch 工具需要正文原文才能
            // 把行号换算成字节区间。未指名时只下发只读工具（见下面的姊妹用例）。
            // 路径相对文档根，此处文档根即项目根。
            "document_id": "documents/chapter-01.md"
        }),
    );

    let run = run_workflow_impl(
        temp.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "writing-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    );

    // 先暴露运行错误：否则装配阶段失败时，下面的工具断言会掩盖真实原因。
    let run = run.expect("writer 节点应能运行成功");
    assert_eq!(run.status, "succeeded", "writer 节点运行未成功");

    let requests = server.join().unwrap_or_default();
    let outbound_request = requests.first().cloned().unwrap_or_default();

    // U108 的核心断言：写作节点必须真的拿到写入工具。
    assert!(
        outbound_request.contains("writer-insert-lines")
            || outbound_request.contains("writer-replace-lines"),
        "U108：writer 节点发给 LLM 的工具声明中没有任何写入工具（writer-insert-lines / \
         writer-replace-lines），模型即便想写也无从下笔。实际出站请求片段：{}",
        outbound_request.chars().take(800).collect::<String>()
    );
}

/// 安全边界一：节点没有指名 `document_id` 时，不得下发行号 patch 工具。
///
/// 行号 patch 需要正文原文才能把 1-based 行号换算成字节区间；没有文档上下文
/// 就下发这些工具，模型只会调用失败。只读工具（find/search）仍应可用。
#[test]
fn production_flow_writer_without_document_gets_no_write_tools() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm(vec![chat_response("writer-chat", "先想想。")]);
    let secrets = MemorySecretStore::default();
    provision_project_with_llm(
        temp.path(),
        &secrets,
        base_url,
        "writer-chat",
        ProviderCapability::Llm,
    );
    enable_write_tools(temp.path());

    // 与主用例唯一的差别：没有 document_id。
    save_single_node_workflow(
        temp.path(),
        "no-doc-flow",
        "writer",
        json!({
            "provider_id": PRIMARY_PROVIDER_ID,
            "model_id": "writer-chat",
            "prompt_template": "想一下开头"
        }),
    );

    let run = run_workflow_impl(
        temp.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "no-doc-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    );
    let run = run.expect("未指名文档的写作节点仍应能做纯文本生成");
    assert_eq!(run.status, "succeeded");

    let outbound = server
        .join()
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default();

    assert!(
        !outbound.contains("writer-insert-lines") && !outbound.contains("writer-replace-lines"),
        "未指名 document_id 时不得下发行号 patch 工具（模型必然调用失败）：{}",
        outbound.chars().take(600).collect::<String>()
    );
    assert!(
        outbound.contains("writer-find"),
        "只读工具不受文档缺失影响，应照常下发：{}",
        outbound.chars().take(600).collect::<String>()
    );
}

/// 安全边界二：权限页把写入工具关掉时（出厂默认），即使指名了文档也不得下发。
///
/// 这是权限开关对写作工具真正生效的证据——修复前那批 `write` 开关是死配置。
#[test]
fn production_flow_disabled_write_permission_removes_write_tools() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let chapter_path = temp.path().join("documents/chapter-01.md");
    std::fs::create_dir_all(chapter_path.parent().unwrap()).unwrap();
    std::fs::write(&chapter_path, "# 第一章\n").unwrap();

    let (base_url, server) = spawn_fake_llm(vec![chat_response("writer-chat", "写好了。")]);
    let secrets = MemorySecretStore::default();
    provision_project_with_llm(
        temp.path(),
        &secrets,
        base_url,
        "writer-chat",
        ProviderCapability::Llm,
    );
    // 注意：这里**不**调用 enable_write_tools，保持出厂的 write = false。

    save_single_node_workflow(
        temp.path(),
        "locked-flow",
        "writer",
        json!({
            "provider_id": PRIMARY_PROVIDER_ID,
            "model_id": "writer-chat",
            "prompt_template": "把开头写进第一章",
            "document_id": "documents/chapter-01.md"
        }),
    );

    let run = run_workflow_impl(
        temp.path(),
        &secrets,
        RunWorkflowRequest {
            workflow_id: "locked-flow".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    );
    let run = run.expect("权限关闭只影响工具清单，不应让节点运行失败");
    assert_eq!(run.status, "succeeded");

    let outbound = server
        .join()
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default();

    assert!(
        !outbound.contains("writer-insert-lines") && !outbound.contains("writer-replace-lines"),
        "权限页关闭写入工具时不得下发给模型：{}",
        outbound.chars().take(600).collect::<String>()
    );
}

/// 打开「修改项目文件」这一类工具开关，模拟用户在权限页的勾选。
fn enable_write_tools(project_root: &std::path::Path) {
    let mut settings = ariadne::commands::get_permissions_settings_impl(project_root).unwrap();
    settings
        .tool_controls
        .entry("global".to_owned())
        .or_default()
        .insert("write".to_owned(), Some(true));
    ariadne::commands::save_permissions_settings_impl(project_root, settings).unwrap();
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

    // loop 节点的图合同（`workflow/integration.rs` 的 `validate_workflow_execution_contracts`）
    // 要求 stop_condition.input_alias 必须有上游数据边喂值，因此需要一个 start 节点
    // 提供 `approved` 数据；这与本用例要验证的缺陷（全局轮次上限不生效）无关，
    // 只是满足图结构合同的最小必要脚手架。
    //
    // 工作流里的 loop 节点声明了 50 轮，远超全局上限 2。
    // `LoopNodeConfig` 字段是平铺的，不嵌套在 "policy" 键下。
    let saved = save_workflow_graph_impl(
        temp.path(),
        WorkflowGraphData {
            workflow_id: "runaway".to_owned(),
            name: "runaway".to_owned(),
            nodes: vec![
                CanvasNode {
                    id: "start".to_owned(),
                    r#type: "start".to_owned(),
                    label: None,
                    data: json!({"initial_inputs": {"approved": true}}),
                    position: Value::Null,
                },
                CanvasNode {
                    id: "loop-node".to_owned(),
                    r#type: "loop".to_owned(),
                    label: None,
                    data: json!({
                        "max_iterations": 50,
                        "timeout_ms": 60_000,
                        "stop_condition": {"input_alias": "approved", "equals": true},
                        "rerun_node_ids": []
                    }),
                    position: Value::Null,
                },
                // 图合同要求 loop 节点要么有 rerun_node_ids，要么有出向控制边；
                // 用一个无外部依赖的 export 节点（无 sink 时安全空操作）满足该合同，
                // 与本用例要验证的缺陷本身无关。
                CanvasNode {
                    id: "sink".to_owned(),
                    r#type: "export".to_owned(),
                    label: None,
                    data: json!({"artifact_id": "runaway-export", "format": "json"}),
                    position: Value::Null,
                },
            ],
            edges: vec![
                CanvasEdge {
                    id: "start-loop-exec".to_owned(),
                    source: "start".to_owned(),
                    target: "loop-node".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "start-loop-data".to_owned(),
                    source: "start".to_owned(),
                    target: "loop-node".to_owned(),
                    source_handle: "approved".to_owned(),
                    target_handle: "input".to_owned(),
                    kind: WorkflowEdgeKind::Data,
                    label: Some("approved".to_owned()),
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "loop-sink-exec".to_owned(),
                    source: "loop-node".to_owned(),
                    target: "sink".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
            ],
            metadata: Value::Null,
            content_revision: None,
            expected_revision: None,
        },
    );

    // 全局上限生效后，越界的 loop 在**保存边界**就应被拒绝——这比等到运行时更早，
    // 用户在画布上按下保存的当下就能看到「超出全局上限」，不必先烧掉一次运行。
    if let Err(error) = saved {
        let diagnostic = error.diagnostic_text();
        assert!(
            diagnostic.contains("exceeds workflow limit"),
            "U113：拒绝原因应指明越过了全局循环上限，实际：{diagnostic}"
        );
        return;
    }

    // 若保存边界放行（例如历史数据绕过保存直接落盘），运行前预检必须兜住。
    let run = run_workflow_impl(
        temp.path(),
        &MemorySecretStore::default(),
        RunWorkflowRequest {
            workflow_id: "runaway".to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
            // U165：变量来源取 Default（= ProjectAi，宽松那一侧）。
            // 显式写 ExecutionPage 会把 hidden 变量的拒绝行为拉进这些用例，
            // 让它们各自的判据受一个无关开关影响。
            variable_source: Default::default(),
        },
    );

    assert!(
        run.is_err(),
        "U113：全局最大循环轮次设为 2，工作流声明 50 轮却被放行，全局成本护栏未接线"
    );
}
