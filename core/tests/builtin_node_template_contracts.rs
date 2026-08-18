//! U175：产品自带的 9 个节点提示词模板，拖到画布上必须能跑通（2026-08-18）。
//!
//! **本文件回答一个问题**：用户从工具箱拖一个写作节点到画布上，提示词框里
//! 是产品预填的 `node_template.{agent}.default`（前端 `PromptCatalog.ResolveNodePrompt`
//! 按同一套 key 取），什么都不改直接点运行——能不能跑起来？
//!
//! 判据必须是**真实运行**：跑单节点工作流，读运行终态 + 假 LLM 收到的
//! 出站 HTTP 请求原文。理由是这条链上的失败点在渲染器里，**发生在发请求之前**：
//! 渲染失败 ⇒ 零次出站请求 ⇒ 任何「请求里不含 `{{`」的断言都对空串恒真。
//! 只做字符串比对（比如「模板里的占位符都在 known_section_aliases 里」）测不到这个：
//! 别名表有这个名字，不等于**这次运行**的 context bundle 里有对应 section。
//!
//! 分析见 `项目检验报告/发布前全量代码审查/U175-自带节点模板必然运行失败.md`。

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
use ariadne::contracts::{ProviderCapability, ProviderType, RunId, WorkflowId};
use ariadne::workflow::{SqliteWorkflowRuntimeStore, WorkflowRuntimeStore};
use serde_json::{json, Value};

const PROVIDER_ID: &str = "primary";
const MODEL_ID: &str = "m";

// ════════════════════════════════════════════════════════
// 测试基建
// ════════════════════════════════════════════════════════

/// 假 LLM：接受**任意多**轮请求直到超时，把每一轮的请求原文交回。
///
/// 为什么不是「收固定条数」：本文件要测的正是「一次都没发出去」这个状态，
/// 固定条数的 mock 会在 accept 上阻塞到超时后 panic，把「零次请求」这个
/// **待测结论**变成测试基建的崩溃。所以这里只等第一条，等不到就返回空表。
fn spawn_fake_llm(reply: &str) -> (String, thread::JoinHandle<Vec<String>>) {
    let body = json!({
        "model": MODEL_ID,
        "choices": [{"message": {"content": reply, "tool_calls": []}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 4}
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen: Vec<String> = Vec::new();
        // 只尝试接一条；渲染失败时这里会空手而归，这正是要观测的事实。
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                    let mut buffer = [0u8; 262_144];
                    if let Ok(read) = stream.read(&mut buffer) {
                        seen.push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    return seen;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return seen;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return seen,
            }
        }
    });
    (base_url, handle)
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

/// 打开写入与注册两类工具的出厂开关（默认关闭，是产品的安全默认）。
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

/// 取产品自带的节点默认模板原文——**从资源文件读，不在测试里抄一份**。
///
/// 抄一份会让「模板改了但没人跑过」这个缺陷（U175 的成因之一）继续隐身：
/// 用例里那份是旧的，照样全绿。
fn builtin_node_template(agent: ariadne::rag::models::WritingAgentKind) -> String {
    let prompts =
        ariadne::rag::resources::load_prompt_resources().expect("内置提示词资源必须可加载");
    prompts
        .get(agent.default_template_key())
        .map(|resource| resource.prompt.clone())
        .unwrap_or_else(|| panic!("缺少内置模板 {}", agent.default_template_key()))
}

/// 一次探针的观测结果。
struct TemplateProbe {
    /// 运行终态（`succeeded` / `paused` / `failed`），或命令层错误。
    status: Result<String, String>,
    /// 节点自身的错误信息（渲染失败时未解析的变量名在这里）。
    node_error: Option<String>,
    /// 假 LLM 收到的出站请求原文；渲染在发请求前失败时为空串。
    outbound: String,
}

impl TemplateProbe {
    /// 该模板是否「拖上画布就能跑」：跑到非 failed 终态，且真的发出了请求。
    fn is_runnable(&self) -> bool {
        matches!(self.status.as_deref(), Ok("succeeded") | Ok("paused"))
            && !self.outbound.is_empty()
    }

    /// 供失败信息使用的一行诊断。
    fn diagnosis(&self) -> String {
        match (&self.status, &self.node_error) {
            (Ok(status), Some(error)) => format!("终态 {status}，节点错误：{error}"),
            (Ok(status), None) if self.outbound.is_empty() => {
                format!("终态 {status}，但零次出站请求")
            }
            (Ok(status), None) => format!("终态 {status}"),
            (Err(error), _) => format!("运行命令被拒：{error}"),
        }
    }
}

/// 跑一个单节点工作流并采集探针结果。
fn probe_single_node(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    node_type: &str,
    data: Value,
    server: thread::JoinHandle<Vec<String>>,
) -> TemplateProbe {
    let workflow_id = format!("probe-{node_type}");
    probe_single_node_with_id(project_root, secrets, node_type, &workflow_id, data, server)
}

/// 同上，但工作流 id 由调用方给定。
///
/// 逐变量探针要在同一个项目里跑很多次，每次必须换 id：沿用同一个 id 会让
/// 上一轮的运行态快照留在 runtime.db 里，读到的「节点错误」可能是**上一轮**的，
/// 那样清单会记错（我差点就这么记）。
fn probe_single_node_with_id(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    node_type: &str,
    workflow_id: &str,
    data: Value,
    server: thread::JoinHandle<Vec<String>>,
) -> TemplateProbe {
    let workflow_id = workflow_id.to_owned();
    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: workflow_id.clone(),
            name: workflow_id.clone(),
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
            workflow_id: workflow_id.clone(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
            variable_source: Default::default(),
        },
    )
    .map_err(|error| format!("{error:?}"));

    let run_id = started
        .as_ref()
        .map(|started| started.run_id.clone())
        .unwrap_or_default();
    let status = started.map(|started| started.status);

    // 节点错误要从运行态快照读：`run_workflow_impl` 把「节点执行失败」当作
    // 运行结果而非命令错误返回，所以 Ok("failed") 里才藏着真正的原因。
    let node_error = SqliteWorkflowRuntimeStore::open(project_root)
        .ok()
        .and_then(|store| {
            store
                .load_state(
                    &WorkflowId::from(workflow_id.as_str()),
                    &RunId::from(run_id.as_str()),
                )
                .ok()
                .flatten()
        })
        .and_then(|state| {
            state
                .nodes
                .values()
                .find_map(|node| node.error.clone())
                .or_else(|| state.failure.map(|failure| format!("{failure:?}")))
        });

    let outbound = server
        .join()
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default();

    TemplateProbe {
        status,
        node_error,
        outbound,
    }
}

/// 逐个变量探针：把节点提示词换成**只含一个占位符**的模板跑一次真实运行。
///
/// **为什么必须逐个变量、不能一次跑整份模板**：`render_prompt_template` 在
/// **第一个**未解析变量上就 `return Err`。整份模板只能告诉你「首个坏变量是谁」，
/// 补好它之后才会暴露第二个——那样要修多少处永远算不清。一次一个变量，
/// 一轮就能拿到该 agent 的完整缺口清单。
///
/// 返回 `Ok(替换后的正文片段)` 或 `Err(诊断)`。
fn probe_variable(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    node_type: &str,
    variable: &str,
    index: usize,
    extra_config: &Value,
) -> Result<String, String> {
    let (base_url, server) = spawn_fake_llm("ok");
    // provider 的 base_url 每轮都变（假 LLM 每轮换端口），必须重新保存。
    provision_provider_only(project_root, secrets, base_url);

    let mut data = json!({
        "provider_id": PROVIDER_ID,
        "model_id": MODEL_ID,
        // 前后加锚点，便于从出站请求里切出该变量替换后的正文。
        "prompt_template": format!("<<{index}>>{{{{{variable}}}}}<<END>>"),
    });
    if let (Some(target), Some(extra)) = (data.as_object_mut(), extra_config.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }

    let probe = probe_single_node_with_id(
        project_root,
        secrets,
        node_type,
        &format!("probe-{node_type}-{index}"),
        data,
        server,
    );
    if !probe.is_runnable() {
        return Err(probe.diagnosis());
    }
    // 出站请求里占位符必须已被替换掉，且替换成的不能是空串——
    // 静默置空比留字面量更糟：模型会以为「这一章本来就没有大纲」。
    let marker = format!("<<{index}>>");
    let body = &probe.outbound;
    let start = body
        .find(&marker)
        .map(|at| at + marker.len())
        .ok_or_else(|| format!("出站请求里找不到锚点 {marker}"))?;
    let end = body[start..]
        .find("<<END>>")
        .map(|at| start + at)
        .ok_or_else(|| "出站请求里找不到结束锚点".to_owned())?;
    let rendered = body[start..end].trim().to_owned();
    // ⚠️ `{{ref:` 要排除：`{{角色设定}}` 展开成 agent_prompt 正文，而 planner 那份
    // 正文里有一句教模型引用语法的 `{{ref:文档ID#L起始-L结束}}`——那是**刻意的语法
    // 示例**（「文档ID」是占位说明，不是真实 document_id），不是待展开的引用。
    // 引用展开跑在渲染**之前**，看不到代入后的正文，所以它留在这里是正确行为。
    // 不排除就会把产品的正确行为报成缺陷（我第一版探针就误报了一条）。
    if rendered.replace("{{ref:", "").contains("{{") {
        return Err(format!("占位符以字面量出站：{rendered}"));
    }
    if rendered.is_empty() {
        return Err("解析成了空串（模型会以为这项资料本来就不存在）".to_owned());
    }
    Ok(rendered)
}

/// 只重存 provider（每轮假 LLM 换端口，base_url 必须跟着更新）。
fn provision_provider_only(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    base_url: String,
) {
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

// ════════════════════════════════════════════════════════
// 探针：writer
// ════════════════════════════════════════════════════════

/// **U175 的最小复现**：writer 节点用产品自带模板必须能跑。
///
/// `node_template.writer.default` 引用 `{{上一章原文}}` `{{本章大纲}}`
/// `{{本章细节}}` `{{返修上下文}}` 四个变量，而生产装配处
/// （`integration.rs` 的 `render_writing_node_prompt`）只填 `current_draft_text`
/// ⇒ 对应 section 全部缺席 ⇒ 渲染器 fail-loud ⇒ 节点 failed、零次出站请求。
///
/// 用户看到的后果：**从工具箱拖一个「执笔」节点、什么都不改、点运行，必然报错。**
/// 这是产品可用性的地基——预填的默认值不能是一个必然失败的配置。
#[test]
fn u175_builtin_writer_template_runs_on_a_fresh_node() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

    let (base_url, server) = spawn_fake_llm("雨落下来。");
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, base_url);
    enable_write_and_register(temp.path());

    // writer 的作用域是章节正文，节点要指名可编辑文档才拿得到行号 patch 工具。
    let chapter = temp.path().join("chapter-02.md");
    std::fs::write(&chapter, "第二章\n\n她把伞收进门廊。\n").unwrap();

    let probe = probe_single_node(
        temp.path(),
        &secrets,
        "writer",
        json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": builtin_node_template(
                ariadne::rag::models::WritingAgentKind::Writer,
            ),
            "document_id": "chapter-02.md",
            "chapter_id": "chapter-02",
        }),
        server,
    );

    assert!(
        probe.is_runnable(),
        "U175：拖一个执笔节点上画布、用产品预填的提示词直接运行——失败了。\n{}\n\
         产品自带的默认配置必须是可运行的配置：预填一份必然报错的模板，\n\
         等于让每个新用户的第一次运行都撞墙。",
        probe.diagnosis()
    );
    assert!(
        !probe.outbound.contains("{{"),
        "U175：占位符以字面量出站——模板语法被当正文喂给了模型。\n出站请求：{}",
        probe.outbound.chars().take(600).collect::<String>()
    );
}

// ════════════════════════════════════════════════════════
// 全量清单：9 个 agent × 各自模板的每一个占位符
// ════════════════════════════════════════════════════════

/// 一个 agent 的探针场景：节点类型 + 它需要的节点 config + 模板占位符清单。
struct AgentScenario {
    agent: ariadne::rag::models::WritingAgentKind,
    /// 该 agent 的作用域文档（相对项目根），None = 只读 agent 不指名文档。
    document: Option<(&'static str, &'static str)>,
}

/// 9 个写作 agent 的探针场景。
///
/// `document_id` 必须落在该 agent 的作用域内（见 `writing_document_scope_for_agent`）：
/// outliner → `planning/global.md`、designer → `planning/stages/*`、
/// planner → `planning/chapters/*`、writer/polisher → 章节正文。
/// 只读 agent（detail/critic/prudent/summarizer）配了 document_id 会被 fail-loud 拒绝。
fn agent_scenarios() -> Vec<AgentScenario> {
    use ariadne::rag::models::WritingAgentKind as Kind;
    vec![
        AgentScenario {
            agent: Kind::Outliner,
            document: Some(("planning/global.md", "# 全局总纲\n1. 玉佩线\n")),
        },
        AgentScenario {
            agent: Kind::Designer,
            document: Some((
                "planning/stages/stage-01.md",
                "# 第一阶段\n本阶段收束玉佩线。\n",
            )),
        },
        AgentScenario {
            agent: Kind::Planner,
            document: Some((
                "planning/chapters/chapter-02.md",
                "# 第二章大纲\n苏禾归还玉佩。\n",
            )),
        },
        AgentScenario {
            agent: Kind::Detail,
            document: None,
        },
        AgentScenario {
            agent: Kind::Writer,
            document: Some(("chapter-02.md", "第二章\n\n她把伞收进门廊。\n")),
        },
        AgentScenario {
            agent: Kind::Critic,
            document: None,
        },
        AgentScenario {
            agent: Kind::Prudent,
            document: None,
        },
        AgentScenario {
            agent: Kind::Polisher,
            document: Some(("chapter-02.md", "第二章\n\n她把伞收进门廊。\n")),
        },
        // ⚠️ Summarizer **刻意不在这张表里**：它的节点执行走
        // `execute_summarizer_node_with_optional_search_tools`（四步总结生产链），
        // 根本不经过 `render_writing_node_prompt`，且节点 config 要求
        // `chapter_document_id` 与一条 `chapter_text` 数据边。
        // 它单独由 `u175_builtin_summarizer_template_is_rendered_not_sent_literally` 覆盖。
    ]
}

/// 解析模板里的占位符名（顺序保留、去重），跳过 `ref:` 正文引用。
///
/// `{{ref:...}}` 不是上下文变量：它由 `expand_prompt_content_references` 在渲染
/// **之前**展开，走的是完全不同的一条路。把它混进变量清单会得出一条假缺口。
fn template_placeholders(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = after[..end].trim();
        if !name.starts_with("ref:") && !names.iter().any(|seen| seen == name) {
            names.push(name.to_owned());
        }
        rest = &after[end + 2..];
    }
    names
}

/// **U175 的主用例 / 第一步产物**：9 个自带模板的每一个占位符都必须解析得出。
///
/// 判据落在**真实运行**上：每个变量单独跑一次单节点工作流，观测
/// (终态, 节点错误, 出站请求原文) 三元组。一个变量算「通」需要同时满足：
/// 运行没 failed、真的发出了请求、锚点之间的正文非空且不含 `{{`。
///
/// **为什么一次一个变量**：渲染器在第一个坏变量上就 `return Err`，
/// 整份模板跑一次只能暴露一个。逐变量跑才拿得到完整缺口清单。
///
/// 失败信息按 agent 分组打印**全部**缺口，而不是在第一个上 panic——
/// 「还要修多少处」是这条 P0 最需要先回答的问题。
#[test]
fn u175_every_builtin_template_placeholder_resolves_on_a_real_run() {
    let mut failures: Vec<String> = Vec::new();

    for scenario in agent_scenarios() {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

        let (base_url, warmup) = spawn_fake_llm("ok");
        let secrets = MemorySecretStore::default();
        provision(temp.path(), &secrets, base_url);
        enable_write_and_register(temp.path());
        drop(warmup);

        let mut extra = json!({ "chapter_id": "chapter-02" });
        if let Some((relative, body)) = scenario.document {
            let path = temp.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, body).unwrap();
            if let Some(object) = extra.as_object_mut() {
                object.insert("document_id".to_owned(), json!(relative));
            }
        }

        let template = builtin_node_template(scenario.agent);
        let node_type = scenario.agent.node_type();
        let mut unresolved: Vec<String> = Vec::new();

        for (index, variable) in template_placeholders(&template).iter().enumerate() {
            match probe_variable(temp.path(), &secrets, node_type, variable, index, &extra) {
                Ok(_) => {}
                Err(diagnosis) => unresolved.push(format!("    {variable} —— {diagnosis}")),
            }
        }

        if !unresolved.is_empty() {
            failures.push(format!(
                "  {} ({}) 共 {} 个变量解析不出来：\n{}",
                node_type,
                scenario.agent.default_template_key(),
                unresolved.len(),
                unresolved.join("\n")
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "U175：产品自带的节点提示词模板拖到画布上运行必然失败。\n\
         下面每一条都是「用户拖一个该类型节点、用预填提示词点运行」时会撞上的错误：\n{}\n\
         产品预填的默认值必须是可运行的配置。",
        failures.join("\n")
    );
}

// ════════════════════════════════════════════════════════
// 第 9 个模板：summarizer 走的是另一条路
// ════════════════════════════════════════════════════════

/// **U175 · summarizer**：拖一个「章节归档」节点，预填的提示词里不得有占位符。
///
/// summarizer 是 9 个写作节点里**唯一不走 `render_writing_node_prompt`** 的：
/// 它由 `execute_summarizer_node_with_optional_search_tools` 承接（四步总结生产链），
/// 节点 `prompt_template` 经 `SummarizerExecutor::author_template_prefix()`
/// **原文 `format!` 拼进**每一步指令，从不过 `render_prompt_template`。
/// 也就是说这条路上**没有渲染器**，占位符写进去就会字面量出站——
/// 且运行照样成功、总结照样产出，没有任何东西会报错（比 A 类的节点失败更隐蔽）。
///
/// 好消息是**默认拖拽路径是安全的**：前端 `PromptCatalog.ResolveNodePrompt`
/// 优先取 `agent_prompt.summarizer`（非空、无占位符），所以用户拖一个
/// 章节归档节点拿到的是那份，不是 `node_template.summarizer.default`。
/// 本用例钉住的就是这个保证——它是唯一挡在「占位符字面量出站」前面的东西，
/// 哪天有人给 `agent_prompt.summarizer` 加个 `{{本章大纲}}`，这里当场变红。
///
/// ⚠️ `node_template.summarizer.default` 里那两个占位符
/// （`{{角色设定}}` `{{当前章节正文}}`）**是一处遗留不一致**：它在默认路径上
/// 取不到（agent_prompt 非空即优先），但用户手动粘贴或预设引用它时会字面量出站。
/// 本轮不修——修它要动 `rag/summarizer.rs`（让它走渲染器），那个文件本轮由他人负责。
/// 已记进 U175 报告的「遗留待办」一节。
#[test]
fn u175_dragged_summarizer_node_has_no_unrenderable_placeholders() {
    let agent = ariadne::rag::models::WritingAgentKind::Summarizer;
    let prefilled = desktop_prefilled_prompt(agent);
    let placeholders = template_placeholders(&prefilled);

    assert!(
        placeholders.is_empty(),
        "U175 · summarizer：拖一个章节归档节点，预填提示词里有占位符 {:?}。\n\
         summarizer 的执行路径（execute_summarizer_node_* → author_template_prefix）\n\
         是**原文 format! 拼接**，不过 render_prompt_template ⇒ 这些占位符会以\n\
         字面量进入 4 次出站 LLM 请求，而运行照样「成功」、没有任何报错。\n\
         要么把占位符从预填提示词里去掉，要么让 summarizer 走渲染器。",
        placeholders,
    );
}

// ════════════════════════════════════════════════════════
// 真实拖拽路径：前端优先取 agent_prompt.{type}
// ════════════════════════════════════════════════════════

/// 取**前端实际预填**的提示词，复刻 `PromptCatalog.ResolveNodePrompt` 的优先级。
///
/// ⚠️ 这与 `builtin_node_template()` 取的**不是同一份**：前端先试
/// `agent_prompt.{type}`，非空就用它，`node_template.{type}.default` 只是回落。
/// 而 9 个 `agent_prompt.*` 全都非空 ⇒ **用户拖节点拿到的是 agent_prompt**，
/// `node_template.*` 在默认拖拽路径上其实取不到。
/// 两条路都要测：前者是用户实际撞上的，后者是产品文档承诺的「默认模板」。
fn desktop_prefilled_prompt(agent: ariadne::rag::models::WritingAgentKind) -> String {
    let prompts =
        ariadne::rag::resources::load_prompt_resources().expect("内置提示词资源必须可加载");
    let agent_prompt = prompts
        .get(agent.prompt_key())
        .map(|resource| resource.prompt.clone())
        .unwrap_or_default();
    if !agent_prompt.trim().is_empty() {
        return agent_prompt;
    }
    builtin_node_template(agent)
}

/// **U175 · 真实拖拽路径**：拖任一写作节点上画布、什么都不改、点运行，必须能跑。
///
/// 这条比 `u175_every_builtin_template_placeholder_resolves_on_a_real_run` 更贴近
/// 用户：它用的是前端**真正预填**进提示词框的那份文本（`agent_prompt.{type}` 优先）。
///
/// summarizer 除外——它的节点 config 要求 `chapter_document_id` 与一条
/// `chapter_text` 数据边，单节点跑不起来，属另一条执行路径。
#[test]
fn u175_dragging_any_writing_node_and_running_it_works() {
    let mut failures: Vec<String> = Vec::new();

    for scenario in agent_scenarios() {
        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();

        let (base_url, server) = spawn_fake_llm("好。");
        let secrets = MemorySecretStore::default();
        provision(temp.path(), &secrets, base_url);
        enable_write_and_register(temp.path());

        let mut data = json!({
            "provider_id": PROVIDER_ID,
            "model_id": MODEL_ID,
            "prompt_template": desktop_prefilled_prompt(scenario.agent),
            "chapter_id": "chapter-02",
        });
        if let Some((relative, body)) = scenario.document {
            let path = temp.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, body).unwrap();
            if let Some(object) = data.as_object_mut() {
                object.insert("document_id".to_owned(), json!(relative));
            }
        }

        let node_type = scenario.agent.node_type();
        let probe = probe_single_node(temp.path(), &secrets, node_type, data, server);
        if !probe.is_runnable() {
            failures.push(format!("  {node_type} —— {}", probe.diagnosis()));
        }
    }

    assert!(
        failures.is_empty(),
        "U175：从工具箱拖一个写作节点到画布上、用产品预填的提示词直接点运行——失败了：\n{}\n\
         产品预填的默认值必须是可运行的配置。",
        failures.join("\n")
    );
}
