//! 生产全链条用户旅程契约（2026-07-26）。
//!
//! 本文件模拟**真实用户从零到出稿的完整操作序列**，每个用例是一条端到端旅程，
//! 而非单个函数的单元测试。判据始终是「用户能不能靠这套流程写出小说」。
//!
//! 与 `production_flow_contracts.rs` 的分工：
//! - `production_flow_contracts.rs` —— 聚焦单个已知缺陷的最小复现
//! - 本文件 —— 覆盖**跨环节的用户旅程**，捕捉"每步都对但连起来不通"的问题
//!
//! 所有用例断言**期望的正确行为**。未修复项会失败，失败即缺陷存在。
//!
//! 分析见 `项目检验报告/发布前全量代码审查/13-配置项存在性与执行链路阻断审查.md`。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    create_project, open_project, run_workflow_impl, save_provider_settings_impl,
    save_workflow_graph_impl, start_workflow, AriadneAppState, CanvasEdge, CanvasNode,
    ProviderSettingsUpdate, RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{
    ConfigStore, MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretStore, SecretValue,
};
use ariadne::contracts::{ProviderCapability, ProviderType, RunId, WorkflowEdgeKind, WorkflowId};
use ariadne::workflow::{SqliteWorkflowRuntimeStore, WorkflowRuntimeStore};
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

/// 假 LLM：按轮次返回预设响应，并把每一轮收到的请求原文交回供断言。
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
            let read = match stream.read(&mut buffer) {
                Ok(n) => n,
                Err(_) => break,
            };
            seen.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
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

fn chat_response(model: &str, content: &str) -> String {
    json!({
        "model": model,
        "choices": [{"message": {"content": content, "tool_calls": []}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 20, "completion_tokens": 8}
    })
    .to_string()
}

const PROVIDER_ID: &str = "primary";

/// 模拟「用户在设置页配好一个 Provider 并保存密钥」。
fn user_configures_provider(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    base_url: String,
    model_id: &str,
) {
    save_provider_settings_impl(
        project_root,
        ProviderSettingsUpdate {
            provider_id: PROVIDER_ID.to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "我的模型服务".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            models: vec![ModelConfig {
                model_id: model_id.to_owned(),
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
    .expect("用户配置 Provider 应当成功");

    ProjectCredentialScope::new(project_root, secrets)
        .unwrap()
        .set_provider_secret(PROVIDER_ID, SecretValue::new("sk-user-key"))
        .expect("保存 API Key 应当成功");
}

/// 模拟「用户在画布上拖一个 AI 节点并配好模型」。
fn user_builds_single_node_workflow(
    project_root: &std::path::Path,
    workflow_id: &str,
    node_type: &str,
    model_id: &str,
    prompt: &str,
) {
    user_builds_single_node_workflow_with(
        project_root,
        workflow_id,
        node_type,
        model_id,
        prompt,
        json!({}),
    );
}

/// 同上，但允许追加节点 config 键（如 `chapter_id`、`document_id`）。
///
/// 上下文装配需要 `chapter_id` 作为归属键（知识库按章查总结），
/// 缺它时 `render_writing_node_prompt` 会 fail-loud。所以任何要验
/// 「上下文真的进了请求」的用例都必须配它——否则测到的是报错路径，
/// 而报错路径下出站请求根本不存在，`!outbound.contains("{{")` 之类的
/// 断言会因为 outbound 是空串而**恒真**。
fn user_builds_single_node_workflow_with(
    project_root: &std::path::Path,
    workflow_id: &str,
    node_type: &str,
    model_id: &str,
    prompt: &str,
    extra_config: Value,
) {
    let mut data = json!({
        "provider_id": PROVIDER_ID,
        "model_id": model_id,
        "prompt_template": prompt
    });
    if let (Some(target), Some(extra)) = (data.as_object_mut(), extra_config.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
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
    .expect("用户保存工作流应当成功");
}

/// 模拟「用户在界面上点运行」——走 `AriadneAppState` 的真实命令入口。
///
/// 与直接调 `run_workflow_impl` 的关键区别：`start_workflow` 会传入
/// `state.retrieval_runtime()`（AppState 缓存的实例），复用同一个 tantivy
/// IndexWriter。绕过 state 直接调 `run_workflow_impl` 会重开索引并触发
/// `LockBusy`——那是测试用法问题，不是产品缺陷。
fn user_clicks_run_via_app(
    app: &AriadneAppState,
    workflow_id: &str,
) -> Result<ariadne::commands::WorkflowRunStarted, String> {
    start_workflow(app, workflow_id.to_owned(), None).map_err(|error| format!("{error:?}"))
}

/// 无 AppState 场景下的直跑入口（仅用于不涉及索引的用例）。
fn user_clicks_run(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    workflow_id: &str,
) -> Result<ariadne::commands::WorkflowRunStarted, String> {
    run_workflow_impl(
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
    .map_err(|error| format!("{error:?}"))
}

/// `start_workflow` 是后台调度：返回 `queued` 后由 worker 推进。
/// 轮询运行状态直到终态，模拟用户在执行页看到运行结束。
fn wait_until_finished(
    project_root: &std::path::Path,
    workflow_id: &str,
    run_id: &str,
) -> String {
    let store = SqliteWorkflowRuntimeStore::open(project_root).expect("打开运行状态库");
    let workflow_id = WorkflowId::from(workflow_id);
    let run_id = RunId::from(run_id);
    for _ in 0..250 {
        if let Ok(Some(state)) = store.load_state(&workflow_id, &run_id) {
            if state.status.is_terminal() {
                return format!("{:?}", state.status).to_lowercase();
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("等待工作流终态超时");
}

// ════════════════════════════════════════════════════════
// 旅程 1：全新用户的第一次成功
// ════════════════════════════════════════════════════════

/// **最重要的一条**：一个全新用户，从「新建项目」到「跑出第一段文字」。
///
/// 用真实的 `create_project` 命令（而非底层 `initialize_project`），
/// 因为这是用户在欢迎页实际点的那个按钮。
#[test]
fn journey_new_user_from_empty_to_first_output() {
    let workspace = tempfile::tempdir().unwrap();
    let app_state_dir = tempfile::tempdir().unwrap();
    let project_root = workspace.path().join("我的第一本小说");

    let secrets = Arc::new(MemorySecretStore::default());
    let store: Arc<dyn SecretStore> = Arc::clone(&secrets) as Arc<dyn SecretStore>;
    let app = AriadneAppState::new(workspace.path(), app_state_dir.path(), store);

    // 第 1 步：用户在欢迎页点「新建项目」
    let report = create_project(
        &app,
        project_root.to_string_lossy().into_owned(),
        Some("我的第一本小说".to_owned()),
    );
    let report = report.expect("新建项目应当成功");
    assert!(
        project_root.exists(),
        "新建项目后目录必须存在：{report:?}"
    );

    // 第 2 步：用户在设置页配 Provider
    let (base_url, server) = spawn_fake_llm(vec![chat_response("my-model", "夜色像一封没写完的信。")]);
    user_configures_provider(&project_root, &secrets, base_url, "my-model");
    // 真实 UI 里保存 Provider 走 state 命令并同步刷新检索运行时；
    // 测试为控制 mock 时序直接调 impl 保存，这里补上同样的刷新语义。
    app.reload_retrieval_runtime()
        .expect("保存 Provider 后刷新检索运行时应当成功");

    // 第 3 步：用户在画布拖节点、填提示词
    user_builds_single_node_workflow(
        &project_root,
        "first-flow",
        "llm",
        "my-model",
        "写一句开场",
    );

    // 第 4 步：用户点运行
    let run = user_clicks_run_via_app(&app, "first-flow");
    let _ = server.join();

    let run = run.expect("全新用户的第一次运行必须成功——这是产品可用性的地基");
    let final_status = wait_until_finished(&project_root, "first-flow", &run.run_id);
    assert_eq!(
        final_status, "succeeded",
        "第一次运行未成功，新用户会直接流失"
    );
}

/// 用户关掉应用、重新打开项目后，之前配好的 Provider 与工作流仍在。
///
/// 覆盖"配置持久化"这条容易在重构中被打破的链路。
#[test]
fn journey_reopen_project_preserves_provider_and_workflow() {
    let workspace = tempfile::tempdir().unwrap();
    let app_state_dir = tempfile::tempdir().unwrap();
    let project_root = workspace.path().join("续写项目");

    let secrets = Arc::new(MemorySecretStore::default());
    let store: Arc<dyn SecretStore> = Arc::clone(&secrets) as Arc<dyn SecretStore>;
    let app = AriadneAppState::new(workspace.path(), app_state_dir.path(), store);

    create_project(
        &app,
        project_root.to_string_lossy().into_owned(),
        Some("续写项目".to_owned()),
    )
    .expect("新建项目应当成功");

    let (base_url, server) = spawn_fake_llm(vec![chat_response("m", "ok")]);
    user_configures_provider(&project_root, &secrets, base_url, "m");
    user_builds_single_node_workflow(&project_root, "keep", "llm", "m", "继续");

    // 用户关闭应用（丢弃 state），重新打开同一项目
    drop(app);
    let store2: Arc<dyn SecretStore> = Arc::clone(&secrets) as Arc<dyn SecretStore>;
    let app2 = AriadneAppState::new(workspace.path(), app_state_dir.path(), store2);
    open_project(&app2, project_root.to_string_lossy().into_owned(), None)
        .expect("重新打开项目应当成功");
    app2.reload_retrieval_runtime()
        .expect("重开项目后检索运行时应当可用");

    // 配置必须还在
    let config = ConfigStore::new(&project_root).load_or_create().unwrap();
    assert!(
        config
            .providers
            .providers
            .iter()
            .any(|p| p.provider_id == PROVIDER_ID),
        "重开项目后 Provider 配置丢失"
    );
    assert_eq!(
        config.providers.default_llm_provider_id.as_deref(),
        Some(PROVIDER_ID),
        "重开项目后默认 LLM 路由丢失"
    );

    // 工作流必须还能跑
    let run = user_clicks_run_via_app(&app2, "keep");
    let _ = server.join();
    run.expect("重开项目后工作流应当仍可运行");
}

// ════════════════════════════════════════════════════════
// 旅程 2：文学 agent 的核心——写作节点要真的懂这本小说
// ════════════════════════════════════════════════════════

/// **U114**：writer 节点拿上下文的**触发条件是占位符**，不是「节点类型是 writer」。
///
/// ⚠️ 本用例曾长期是红的，而**没人看见过**——它所在的文件因缺 `variable_source`
/// 字段编译不过（2026-08-18 补齐时才暴露）。红的原因不是产品退回，
/// 是用例自己的前提站不住：它给节点配的 `prompt_template` 是裸文本「写第二章」，
/// 既无 `{{}}` 占位符、也无 `chapter_id`，然后期望上下文被自动注入。
///
/// **产品的设计是占位符驱动注入**（`render_writing_node_prompt`）：
/// 模板不含 `{{` 就原样发出，省一次知识库遍历。这是刻意的取舍——
/// 「凡是 writer 节点就无条件塞进全部大纲/前文/设定」在百万字长篇里
/// 每次调用都要拖一整本书，而作者对**这一次**要不要前文是有判断的，
/// 占位符就是他表达判断的手段。
///
/// 所以本用例改为断言那条真实契约：**同一个节点，加了占位符就拿到材料**。
/// 前后对照写在一条用例里，是为了让「注入由什么驱动」这件事在读用例时
/// 就看得见——分成两条会让人以为裸文本那条是缺陷。
///
/// U114 真正的主判据在邻居 `journey_prompt_placeholders_are_never_sent_literally`
/// （占位符不得以字面量出站），那条一直是绿的。
#[test]
fn journey_writer_context_is_driven_by_placeholders_not_node_type() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    // 用户已有的创作材料
    let planning = temp.path().join("planning/chapters");
    std::fs::create_dir_all(&planning).unwrap();
    std::fs::write(
        planning.join("chapter-02.md"),
        "本章大纲：苏禾在雨夜归还玉佩，语气须克制。",
    )
    .unwrap();

    let documents = temp.path().join("documents");
    std::fs::create_dir_all(&documents).unwrap();
    std::fs::write(
        documents.join("chapter-01.md"),
        "第一章\n\n顾言把玉佩收进袖中，她没有回头。",
    )
    .unwrap();

    // ① 裸文本模板：原样发出，不拖知识库。这是**产品承诺的行为**，不是缺陷。
    let (base_url, server) = spawn_fake_llm(vec![chat_response("w", "雨落下来。")]);
    let secrets = MemorySecretStore::default();
    user_configures_provider(temp.path(), &secrets, base_url, "w");
    user_builds_single_node_workflow(temp.path(), "write-plain", "writer", "w", "写第二章");

    let run = user_clicks_run(temp.path(), &secrets, "write-plain");
    let plain = server
        .join()
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default();
    run.expect("裸文本提示词的 writer 节点应当能运行");
    assert!(
        plain.contains("写第二章"),
        "裸文本提示词没原样出站，说明连基本发送都断了：{}",
        plain.chars().take(400).collect::<String>()
    );
    assert!(
        !plain.contains("苏禾"),
        "裸文本模板也被塞进了大纲——那意味着每次调用都拖一整本书，\
         而作者没有任何手段说「这次不要前文」"
    );

    // ② 同一个节点类型、同样的项目材料，只把占位符加上：材料必须进请求。
    //    这才是 U114 的真实契约。
    let (base_url2, server2) = spawn_fake_llm(vec![chat_response("w", "雨落下来。")]);
    user_configures_provider(temp.path(), &secrets, base_url2, "w");
    user_builds_single_node_workflow_with(
        temp.path(),
        "write-ctx",
        "writer",
        "w",
        "本章大纲：{{本章大纲}}\n\n照它写第二章。",
        // chapter_id 是上下文装配的归属键，缺它会 fail-loud（见 helper 注释）。
        json!({ "chapter_id": "chapter-02" }),
    );

    let run2 = user_clicks_run(temp.path(), &secrets, "write-ctx");
    let contextual = server2
        .join()
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default();

    // 先判运行本身：装配失败时出站请求是空的，下面的断言会因为
    // 「空串不含 {{」而恒真——那是空测，必须先把这条路堵掉。
    run2.expect("配了 chapter_id 与占位符的 writer 节点应当能运行");
    assert!(
        !contextual.is_empty(),
        "带占位符的节点没发出任何请求——装配或渲染在发射前就失败了，\
         此时任何「请求里没有 {{{{}}}}」的断言都是恒真的空测"
    );
    assert!(
        !contextual.contains("{{"),
        "U115：占位符以字面量出站。\n实际出站请求：{}",
        contextual.chars().take(900).collect::<String>()
    );
    assert!(
        contextual.contains("苏禾") || contextual.contains("玉佩"),
        "U114：占位符被替换掉了，但换上去的不是真实大纲内容——\
         静默置空比留着字面量更糟：模型会以为这一章本来就没有大纲。\n实际出站请求：{}",
        contextual.chars().take(900).collect::<String>()
    );
}

/// **U114 + U115**：提示词模板里的 `{{ }}` 占位符必须被替换，不能字面量发给 LLM。
///
/// 用户按 `prompt_list.json` 里 `node_template.writer.default` 的形态写提示词，
/// 里面含 `{{上一章原文}}` 这类槽位。
#[test]
fn journey_prompt_placeholders_are_never_sent_literally() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let (base_url, server) = spawn_fake_llm(vec![chat_response("w", "写好了。")]);
    let secrets = MemorySecretStore::default();
    user_configures_provider(temp.path(), &secrets, base_url, "w");

    // 用户使用了含占位符的模板（这是产品自带的 node_template 形态）
    user_builds_single_node_workflow_with(
        temp.path(),
        "tpl-flow",
        "writer",
        "w",
        "根据上一章继续写：{{上一章原文}}\n本章大纲：{{本章大纲}}",
        // 必须配 chapter_id：缺它时装配 fail-loud、请求根本不发出，
        // 而 `!outbound.contains("{{")` 对空串恒真 ⇒ 这条用例会变成空测。
        // 它此前正是这个状态（还 `let _ = run` 把运行结果丢掉了）。
        json!({ "chapter_id": "chapter-02" }),
    );

    let run = user_clicks_run(temp.path(), &secrets, "tpl-flow");
    let requests = server.join().unwrap_or_default();
    let outbound = requests.first().cloned().unwrap_or_default();

    run.expect("含占位符的模板配齐 chapter_id 后应当能运行");
    assert!(
        !outbound.is_empty(),
        "没发出任何请求 ⇒ 下面那条断言是恒真的空测，先修这里"
    );

    assert!(
        !outbound.contains("{{"),
        "U115：提示词占位符 `{{{{...}}}}` 被原样发给 LLM。\
         要么替换为实际内容，要么在保存时就报错——不能把模板语法当正文喂给模型。\n\
         实际出站请求：{}",
        outbound.chars().take(900).collect::<String>()
    );
}

// ════════════════════════════════════════════════════════
// 旅程 3：钱与安全的护栏在真实运行中生效
// ════════════════════════════════════════════════════════

/// **U112**：用户打开 Auto Mode 但从未设置预授权预算，运行不应被无故暂停。
///
/// 新项目后端默认 `preauthorized_budget_usd = None`（不限制）。
/// 若 UI 把它显示为 `0` 且保存回 `Some(0.0)`，Auto Mode 会每次调用都暂停。
#[test]
fn journey_auto_mode_without_explicit_budget_still_runs() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    // 用户在顶栏打开 Auto Mode，但没碰预算输入框
    let store = ConfigStore::new(temp.path());
    let mut config = store.load_or_create().unwrap();
    config.auto_mode.enabled_by_default = true;
    store.save(&config).unwrap();

    let (base_url, server) = spawn_fake_llm(vec![chat_response("m", "自动跑完了。")]);
    let secrets = MemorySecretStore::default();
    user_configures_provider(temp.path(), &secrets, base_url, "m");
    user_builds_single_node_workflow(temp.path(), "auto-flow", "llm", "m", "写一段");

    let run = user_clicks_run(temp.path(), &secrets, "auto-flow");
    let _ = server.join();

    let run = run.expect("开了 Auto Mode 但未设预算，运行不应失败");
    assert_ne!(
        run.status, "paused",
        "U112：用户只是打开 Auto Mode、从未设置预授权预算，\
         运行却被暂停——`None`（不限制）被误当作 `0`（零额度）"
    );
}

/// **U113**：用户在设置页收紧全局循环上限后，越界的工作流不得放行。
///
/// 对按次计费的 LLM 应用，这是成本护栏。
#[test]
fn journey_tightened_loop_limit_blocks_runaway_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    // 用户在设置页把全局最大循环轮次改成 2
    let store = ConfigStore::new(temp.path());
    let mut config = store.load_or_create().unwrap();
    config.workflow.max_loop_iterations = 2;
    store.save(&config).unwrap();

    // 但导入的工作流声明了 50 轮
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
                CanvasNode {
                    id: "sink".to_owned(),
                    r#type: "export".to_owned(),
                    label: None,
                    data: json!({"artifact_id": "runaway.json", "format": "json"}),
                    position: Value::Null,
                },
            ],
            edges: vec![
                CanvasEdge {
                    id: "e1".to_owned(),
                    source: "start".to_owned(),
                    target: "loop-node".to_owned(),
                    source_handle: "exec_out".to_owned(),
                    target_handle: "exec_in".to_owned(),
                    kind: WorkflowEdgeKind::Control,
                    label: None,
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "e2".to_owned(),
                    source: "start".to_owned(),
                    target: "loop-node".to_owned(),
                    source_handle: "approved".to_owned(),
                    target_handle: "input".to_owned(),
                    kind: WorkflowEdgeKind::Data,
                    label: Some("approved".to_owned()),
                    data: Value::Null,
                },
                CanvasEdge {
                    id: "e3".to_owned(),
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

    // 护栏可以落在保存边界或运行预检，两者都算生效。
    let secrets = MemorySecretStore::default();
    let blocked_at_save = saved.is_err();
    let blocked_at_run = if saved.is_ok() {
        user_clicks_run(temp.path(), &secrets, "runaway").is_err()
    } else {
        false
    };

    assert!(
        blocked_at_save || blocked_at_run,
        "U113：用户把全局循环上限收紧到 2，声明 50 轮的工作流却被放行，\
         全局成本护栏未接线"
    );
}

/// API Key 绝不能出现在工作流运行的产物或日志里。
#[test]
fn journey_api_key_never_leaks_into_run_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let (base_url, server) = spawn_fake_llm(vec![chat_response("m", "ok")]);
    let secrets = MemorySecretStore::default();
    user_configures_provider(temp.path(), &secrets, base_url, "m");
    user_builds_single_node_workflow(temp.path(), "leak-check", "llm", "m", "写一段");

    let run = user_clicks_run(temp.path(), &secrets, "leak-check");
    let _ = server.join();
    let _ = run;

    // 扫描项目目录下所有文件，确认密钥没落盘
    let mut leaked = Vec::new();
    for entry in walk(temp.path()) {
        if let Ok(text) = std::fs::read_to_string(&entry) {
            if text.contains("sk-user-key") {
                leaked.push(entry.display().to_string());
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "API Key 泄漏到以下文件：{leaked:?}"
    );
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
    }
    out
}

// ════════════════════════════════════════════════════════
// 旅程 4：错误路径——用户配错了，产品要说人话
// ════════════════════════════════════════════════════════

/// 用户没配 Provider 就点运行：必须给出可操作的错误，而不是崩溃或沉默。
#[test]
fn journey_running_without_provider_gives_actionable_error() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    user_builds_single_node_workflow(temp.path(), "no-provider", "llm", "nonexistent", "写一段");

    let secrets = MemorySecretStore::default();
    let run = user_clicks_run(temp.path(), &secrets, "no-provider");

    let error = run.expect_err("没配 Provider 就运行必须失败");
    let lowered = error.to_lowercase();
    assert!(
        lowered.contains("provider") || lowered.contains("model") || error.contains("模型"),
        "错误信息必须指向「Provider / 模型未配置」，用户才知道去哪修。实际：{error}"
    );
}

/// 用户配了 Provider 但漏填 API Key：错误必须指向密钥，而不是笼统的网络失败。
#[test]
fn journey_missing_api_key_error_points_at_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    // 配了 Provider，但不存密钥
    save_provider_settings_impl(
        temp.path(),
        ProviderSettingsUpdate {
            provider_id: PROVIDER_ID.to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "无密钥服务".to_owned(),
            enabled: true,
            base_url: Some("http://127.0.0.1:1".to_owned()),
            models: vec![ModelConfig {
                model_id: "m".to_owned(),
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

    user_builds_single_node_workflow(temp.path(), "no-key", "llm", "m", "写一段");

    let secrets = MemorySecretStore::default();
    let run = user_clicks_run(temp.path(), &secrets, "no-key");

    // 这里不强断言错误措辞（对 OpenAiCompatible 缺密钥是合法配置，
    // 失败发生在连接层），只要求 fail-loud 且不 panic——静默成功才是真问题。
    // 注意 run_workflow_impl 对失败的运行返回 Ok(status="failed") 而非 Err。
    match run {
        Err(_) => {}
        Ok(started) => assert_ne!(
            started.status, "succeeded",
            "指向不可达端点且无密钥的 Provider，运行绝不应报告成功"
        ),
    }
}

/// 用户把项目目录设成保留目录名：必须在保存时就被拒绝。
#[test]
fn journey_reserved_directory_name_is_rejected_at_save() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let store = ConfigStore::new(temp.path());
    let mut config = store.load_or_create().unwrap();
    config.app.documents_dir = ".git".to_owned();

    assert!(
        config.validate().is_err(),
        "把作品目录设成 .git 必须被拒绝，否则会破坏版本库"
    );
}
