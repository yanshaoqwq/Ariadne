//! U201-C：节点默认提示词占位符 —— 三种语言写法必须解析到同一份全文（2026-08-18）。
//!
//! # 本文件回答什么
//!
//! 新建写作节点时，`node.PromptTemplate` 里存的是**一行占位符**
//! （`{{outliner 默认提示词}}`）而不是 300~470 字全文。运行时由后端展开成
//! `prompt_list.json` 的 `agent_prompt.{agent}`。
//!
//! 占位符是**给人手打**的（这条功能的意义就是让作者看见并能照抄语法），
//! 而作者照抄的是**屏幕上那一行**：中文界面抄中文、英文界面抄英文。所以：
//!
//! - `{{outliner 默认提示词}}`（zh）
//! - `{{outliner default prompt}}`（en）
//! - `{{outliner デフォルトプロンプト}}`（ja）
//!
//! **三种都必须解析到同一份 `agent_prompt.outliner`**。
//!
//! # 判据为什么落在出站请求原文上
//!
//! 与 U175 同一个理由（见 `builtin_node_template_contracts.rs` 头注）：这条链上的
//! 失败点在渲染器里、**发生在发请求之前**。展开失败 ⇒ 零次出站请求 ⇒ 任何
//! 「请求里不含 `{{`」的断言都对空串恒真。断言「解析函数返回 Ok」更弱：
//! 它连「展开出来的东西有没有进请求体」都不问。
//!
//! 所以每条用例都跑真实单节点工作流，读**假 LLM 收到的 HTTP 请求原文**。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    run_workflow_impl, save_provider_settings_impl, save_workflow_graph_impl, CanvasNode,
    ProviderSettingsUpdate, RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType, RunId, WorkflowId};
use ariadne::rag::models::WritingAgentKind;
use ariadne::workflow::{SqliteWorkflowRuntimeStore, WorkflowRuntimeStore};
use serde_json::{json, Value};

const PROVIDER_ID: &str = "primary";
const MODEL_ID: &str = "m";

// ════════════════════════════════════════════════════════
// 测试基建（与 builtin_node_template_contracts.rs 同款）
// ════════════════════════════════════════════════════════

/// 假 LLM：接一条请求就交回原文；接不到（渲染在发请求前失败）则返回空表。
///
/// 不用「收固定条数」的 mock：本文件要观测的正是「一次都没发出去」这个状态，
/// 固定条数会在 accept 上阻塞到超时后 panic，把待测结论变成基建崩溃。
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

/// 只写 provider（每轮假 LLM 换端口，必须重存 base_url）。
fn provision_provider(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    base_url: String,
    first_time: bool,
) {
    if first_time {
        ariadne::frontend::initialize_project(project_root).unwrap();
    }
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
    if first_time {
        ProjectCredentialScope::new(project_root, secrets)
            .unwrap()
            .set_provider_secret(PROVIDER_ID, SecretValue::new("sk-test"))
            .unwrap();
    }
}

/// 一次探针：拿指定 prompt_template 跑一个真实单节点工作流。
struct Probe {
    status: Result<String, String>,
    node_error: Option<String>,
    /// 假 LLM 收到的出站请求原文；展开在发请求前失败时为空串。
    outbound: String,
}

impl Probe {
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

/// 跑一次探针。`workflow_id` 每次必须不同：沿用同一个 id 会读到**上一轮**的
/// 运行态快照，节点错误因此可能记错。
fn probe(
    project_root: &std::path::Path,
    secrets: &MemorySecretStore,
    node_type: &str,
    workflow_id: &str,
    prompt_template: &str,
    first_time: bool,
) -> Probe {
    let (base_url, server) = spawn_fake_llm("ok");
    provision_provider(project_root, secrets, base_url, first_time);

    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: workflow_id.to_owned(),
            name: workflow_id.to_owned(),
            nodes: vec![CanvasNode {
                id: "node-1".to_owned(),
                r#type: node_type.to_owned(),
                label: None,
                data: json!({
                    "provider_id": PROVIDER_ID,
                    "model_id": MODEL_ID,
                    "prompt_template": prompt_template,
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

    let started = run_workflow_impl(
        project_root,
        secrets,
        RunWorkflowRequest {
            workflow_id: workflow_id.to_owned(),
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

    // 节点执行失败被当作运行结果而非命令错误返回，所以 Ok("failed") 里才藏着原因。
    let node_error = SqliteWorkflowRuntimeStore::open(project_root)
        .ok()
        .and_then(|store| {
            store
                .load_state(
                    &WorkflowId::from(workflow_id),
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

    Probe {
        status,
        node_error,
        outbound,
    }
}

// ════════════════════════════════════════════════════════
// 字面量一律**运行时从语言包读**，源码里只出现 ASCII 的 key 名
// ════════════════════════════════════════════════════════

/// 取某 agent 的占位符字面量在**全部语言包**里的取值（去重，保持并入顺序）。
///
/// **刻意不在源码里硬写那几个字面量**，两个理由：
/// 1. 用例因此**跟着补译走**——语言包改了、补了新语言，用例自动覆盖到，
///    不必有人记得回来同步一张表（硬编码表的漏项形态是「某语言界面手打的
///    写法认不出」，静默且极难查，正是本功能最怕的那一种）；
/// 2. 它顺带验证了并集**真的是从资源现算的**：若实现改回硬编码，
///    这里读到的语言包新增写法就不会被接受，用例当场红。
fn placeholder_literals(agent: WritingAgentKind) -> Vec<String> {
    let key = ariadne::rag::default_prompt::default_prompt_placeholder_key(agent);
    let mut seen = std::collections::BTreeSet::new();
    let mut literals = Vec::new();
    for pack in display_name_packs_from_disk() {
        if let Some(value) = pack.get(&key) {
            if !value.trim().is_empty() && seen.insert(value.clone()) {
                literals.push(value.clone());
            }
        }
    }
    literals
}

/// 直接从磁盘读三份语言包。
///
/// ⚠️ **刻意不用生产的 `all_display_name_packs()`**：用它就等于让待测实现自己提供
/// 输入源。那样「并集退化成只读 zh」这个缺陷会让用例红在**前提断言**上
/// （「语言包里只有 1 种写法」），而不是红在**目标断言**上
/// （「en 写法没有展开成同一份全文」）——前者读起来像是语言包缺译，
/// 会把人引向错误的修复方向。输入源必须独立于被测实现。
fn display_name_packs_from_disk() -> Vec<std::collections::BTreeMap<String, String>> {
    let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    ["display_name.json", "display_name.en.json", "display_name.ja.json"]
        .into_iter()
        .map(|name| {
            let path = resources.join(name);
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("读不到语言包 {}：{error}", path.display()));
            serde_json::from_str(&raw)
                .unwrap_or_else(|error| panic!("语言包 {} 不是平坦映射：{error}", path.display()))
        })
        .collect()
}

/// 取某 agent 的默认提示词全文（`agent_prompt.{agent}`）。
fn agent_prompt_body(agent: WritingAgentKind) -> String {
    let prompts = ariadne::rag::resources::load_prompt_resources().expect("提示词资源必须可加载");
    prompts
        .get(agent.prompt_key())
        .map(|resource| resource.prompt.clone())
        .unwrap_or_else(|| panic!("缺少 {}", agent.prompt_key()))
}

/// 把一段文本转成它在 JSON 请求体里的样子（换行等会被转义）。
///
/// 出站请求体是 JSON，直接拿原文去 `contains` 会因为 `\n` 被转义而假阴性。
fn as_json_fragment(text: &str) -> String {
    let quoted = serde_json::to_string(text).expect("字符串一定可序列化");
    quoted[1..quoted.len() - 1].to_owned()
}

// ════════════════════════════════════════════════════════
// 判据 1：任意一种语言的写法都解析到同一份全文
// ════════════════════════════════════════════════════════

/// 三份语言包各自的写法，跑真实运行后**出站请求里出现的是同一份全文**。
///
/// 这是本功能最关键的一条约束（「解析宽容」那一半）。判据落在假 LLM 收到的
/// HTTP 请求原文上，而不是「解析函数返回 Ok」：后者在展开产物根本没进请求体时
/// 照样绿。
///
/// 用 outliner：它的 `agent_prompt` 正文**不含任何 `{{}}`**，
/// 于是展开产物直接就是最终 prompt，断言最干净（planner 那份带
/// `{{ref:...}}` 示例，另有一条用例专门管它）。
#[test]
fn every_language_spelling_expands_to_the_same_prompt_body() {
    let agent = WritingAgentKind::Outliner;
    let literals = placeholder_literals(agent);
    // 三份语言包都建了这个 key ⇒ 至少两种不同写法（en/ja 若同值则去重后为 2）。
    assert!(
        literals.len() >= 2,
        "{} 在语言包里只有 {} 种写法；\
         并集是这条功能的地基，各语言必须都有值（缺的那门语言里手打的写法会认不出）",
        ariadne::rag::default_prompt::default_prompt_placeholder_key(agent),
        literals.len()
    );

    let expected = as_json_fragment(&agent_prompt_body(agent));
    let project = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();

    for (index, literal) in literals.iter().enumerate() {
        let template = format!("{{{{{literal}}}}}");
        let result = probe(
            project.path(),
            &secrets,
            agent.node_type(),
            &format!("u201c-lang-{index}"),
            &template,
            index == 0,
        );
        // ⚠️ **正向断言放最前**：它就是本用例的目标判据。
        //
        // 先断言「出站非空」会让缺陷红在那条前置检查上，而它的信息
        // （「零次出站请求」）指不出原因；正向断言在两种失败形态下都会红——
        // 认不出该写法时出站为空（渲染器 fail-loud），认成别的 agent 时出站有内容
        // 但不含这份全文——且失败信息里带着节点错误原文。
        assert!(
            result.outbound.contains(&expected),
            "第 {index} 种写法没有展开成 {}；三种语言写法必须解析到同一份全文。\
             出站请求{}。诊断：{}",
            agent.prompt_key(),
            if result.outbound.is_empty() {
                "为空（展开在发请求前就失败了，说明这种写法没被接受集合认出）"
            } else {
                "非空但不含该全文（说明归一到了别的 agent）"
            },
            result.diagnosis()
        );
        // 占位符字面量绝不能原样进请求体：模型会把「XX 默认提示词」这行字
        // 当作自己的角色设定，而节点照样报成功——这正是本功能最坏的失败形态。
        assert!(
            !result.outbound.contains(&as_json_fragment(&template)),
            "第 {index} 种写法的占位符**原样进了请求体**，模型收到的是字面量而非角色设定"
        );
    }
}

// ════════════════════════════════════════════════════════
// 判据 2：认不出的占位符 fail-loud，绝不原样进请求体
// ════════════════════════════════════════════════════════

/// 拼错的占位符**不能**原样发给模型。
///
/// 作者少打一个字（或语言包缺了那门语言的 key）时有三条可能的出路，只有第三条对：
/// - 原样放过 ⇒ 模型把「XX 默认提示词」当角色设定，节点报成功，作者无从发现；
/// - 静默删掉 ⇒ 模型没有角色设定，输出变差但看不出原因；
/// - fail-loud ⇒ 报错点名，作者当场知道拼错了。
///
/// 这里的写法在**任何**语言包里都不存在，所以它落到渲染器的未知变量分支
/// （本步只认自己那一类占位符，其余原样留给后两步，见 `expand_...` 的注释）。
/// 判据是「零次出站请求」+「有节点错误」，而不是「请求里不含 `{{`」——
/// 后者对空串恒真，是个空断言。
#[test]
fn unknown_placeholder_never_reaches_the_request_body() {
    let project = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();
    // ASCII 且绝不可能是任何语言的译文；也不含 `.` 前缀，因此不会被
    // `input.` / `var.` 之类的命名空间分支接走。
    let template = "{{u201c-nonexistent-placeholder}}";

    let result = probe(
        project.path(),
        &secrets,
        WritingAgentKind::Outliner.node_type(),
        "u201c-unknown",
        template,
        true,
    );

    assert!(
        result.outbound.is_empty(),
        "认不出的占位符竟然发出了请求；请求体里含字面量 = 模型把占位符当角色设定。\
         出站原文：{}",
        result.outbound
    );
    assert!(
        result.node_error.is_some(),
        "认不出的占位符必须 fail-loud 点名，不能静默跑完。诊断：{}",
        result.diagnosis()
    );
}

// ════════════════════════════════════════════════════════
// 判据 3：展开产物自带的 `{{ref:...}}` 也要被处理（顺序 + 预读）
// ════════════════════════════════════════════════════════

/// planner 的占位符能跑通——它的全文里**自带** `{{ref:文档ID#L起始-L结束}}`。
///
/// 这条钉住两个**必须同时成立**的性质，缺一个都会让每个 planner 节点跑不起来：
///
/// 1. **展开顺序**：默认提示词占位符必须在 `{{ref:...}}` 展开**之前**处理。
///    反了的话，第 1 步引入的引用没人展开，会一路撞到渲染器的 fail-loud，
///    症状是「节点报某个变量无法解析，而作者的模板里根本没写过那个变量名」。
/// 2. **预读同步**（`commands.rs` 的 `preload_referenced_documents`）：预读必须
///    先展开占位符再扫引用。只扫占位符那一行 ⇒ 扫不到引用 ⇒ 引用来源为 `None`
///    ⇒ 第 2 步发现「有引用但没挂来源」⇒ fail-loud 报「上游节点需要重跑」，
///    而作者根本没写过任何引用。
///
/// 注意全文里那个引用是**教学示例**（字面写着「文档ID」），文档不存在，
/// 展开器会把它换成可诊断的失效标记并继续——这是既有的正确行为。
/// 判据因此是「跑通且占位符没原样进请求体」，不要求那条示例引用能取到正文。
#[test]
fn planner_placeholder_whose_body_contains_a_content_reference_still_runs() {
    let agent = WritingAgentKind::Planner;
    let body = agent_prompt_body(agent);
    assert!(
        body.contains("{{ref:"),
        "本用例的前提是 {} 正文里自带 `{{{{ref:`；\
         前提若不再成立（有人改了提示词），这条用例就不再守着「顺序 + 预读」那两条，\
         请改用另一个自带引用的 agent，而不是删掉它",
        agent.prompt_key()
    );

    let literals = placeholder_literals(agent);
    let project = tempfile::tempdir().unwrap();
    let secrets = MemorySecretStore::default();

    for (index, literal) in literals.iter().enumerate() {
        let template = format!("{{{{{literal}}}}}");
        let result = probe(
            project.path(),
            &secrets,
            agent.node_type(),
            &format!("u201c-planner-{index}"),
            &template,
            index == 0,
        );
        // 全文的第一句必须在请求体里；用首行而非整份，因为其中的 `{{ref:...}}`
        // 会被展开器换成失效标记，整份比对必然不等。
        //
        // 正向断言放最前（理由同前两条用例），并把两种失败形态各自的成因写进
        // 失败信息里——这条用例同时守着「顺序」与「预读」两件事，
        // 红的时候必须能一眼分清是哪一件。
        let first_line = body.lines().next().unwrap_or_default();
        assert!(
            result.outbound.contains(&as_json_fragment(first_line)),
            "planner 第 {index} 种写法没有展开成 {}。诊断：{}\n\
             —— 若节点错误提到「reference document source / 上游节点需要重跑」，\
             那是**预读**没有先展开占位符（commands.rs 的 preload_referenced_documents）；\
             若提到「变量无法解析」，那是**展开顺序**反了。",
            agent.prompt_key(),
            result.diagnosis()
        );
        assert!(
            !result.outbound.contains(&as_json_fragment(&template)),
            "planner 第 {index} 种写法的占位符原样进了请求体"
        );
        // 展开后不能再有未处理的引用占位符残留。
        assert!(
            !result.outbound.contains(&as_json_fragment("{{ref:")),
            "planner 全文自带的 `{{{{ref:` 原样进了请求体：\
             展开顺序反了（第 1 步排在了引用展开之后）"
        );
    }
}

// ════════════════════════════════════════════════════════
// 判据 4：summarizer 走另一条路，也必须展开
// ════════════════════════════════════════════════════════

/// summarizer 节点的占位符也要展开——它**不走** `render_writing_node_prompt`。
///
/// summarizer 是 9 个写作节点里唯一由 `execute_summarizer_node_*`（四步总结生产链）
/// 承接的，接线漏掉它的后果与 U175 那条同形、也同样隐蔽：**总结链不校验模板**，
/// 节点会照样跑完、照样报成功，而模型收到的「角色设定」是占位符字面量——
/// 作者拿到一份「看起来正常」的总结。
///
/// 判据落在第一次出站请求原文上。这里**不要求节点跑成功**：四步总结要求四段结构化
/// 响应与一套章节数据，凑齐它们与本用例要证的事无关；只要第一次请求已经发出，
/// 「占位符有没有被展开」就已成定局。
#[test]
fn summarizer_node_expands_the_placeholder_too() {
    use ariadne::contracts::{NodeId, RunId, WorkflowId};

    let agent = WritingAgentKind::Summarizer;
    let literals = placeholder_literals(agent);
    let expected_first_line = agent_prompt_body(agent)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();

    for (index, literal) in literals.iter().enumerate() {
        let (base_url, server) = spawn_fake_llm("{}");
        let project = tempfile::tempdir().unwrap();
        ariadne::frontend::initialize_project(project.path()).unwrap();
        let chapter = project.path().join("documents").join("chapter-1.md");
        std::fs::create_dir_all(chapter.parent().unwrap()).unwrap();
        std::fs::write(&chapter, "第一行正文。\n第二行正文。\n").unwrap();

        let provider = ariadne::providers::OpenAiCompatibleLlmProvider::new(
            ariadne::config::ProviderConfig {
                provider_id: PROVIDER_ID.to_owned(),
                provider_type: ProviderType::OpenAiCompatible,
                display_name: "Primary".to_owned(),
                enabled: true,
                base_url: Some(base_url),
                api_key: None,
                models: vec![ModelConfig {
                    model_id: MODEL_ID.to_owned(),
                    capability: ProviderCapability::Llm,
                    max_context_tokens: None,
                    input_cost_per_million_tokens: Some(1.0),
                    output_cost_per_million_tokens: Some(2.0),
                }],
            },
            None,
        )
        .expect("构造 HTTP provider 应当成功");
        let ledger = ariadne::costs::SqliteCostLedger::open_in_memory().unwrap();

        let mut inputs = ariadne::contracts::PortMap::new();
        inputs.insert(
            "chapter_text".to_owned(),
            ariadne::contracts::PortValue::Inline {
                value: json!("第一行正文。\n第二行正文。\n"),
            },
        );

        let request = ariadne::workflow::WorkflowNodeExecutionRequest {
            workflow_id: WorkflowId::from("u201c-sum"),
            run_id: RunId::from(format!("run-{index}").as_str()),
            node_id: NodeId::from("node-1"),
            operation_id: format!("op-{index}"),
            operation_attempt: 1,
            request_hash: format!("hash-{index}"),
            type_name: agent.node_type().to_owned(),
            config: json!({
                "provider_id": PROVIDER_ID,
                "model_id": MODEL_ID,
                "chapter_id": "chapter-1",
                "chapter_document_id": "documents/chapter-1.md",
                "chapter_text_alias": "chapter_text",
                "auto_mode": false,
                "prompt_template": format!("{{{{{literal}}}}}"),
            }),
            inputs,
            communication_messages: Vec::new(),
            variables: Default::default(),
            metadata: Value::Null,
            cancellation: Default::default(),
            dispatch_authorization: Default::default(),
        };

        // 跑不通没关系（四步响应是假的），要看的是第一次请求已经带上了什么。
        let _ = ariadne::workflow::execute_summarizer_node(
            request,
            &provider,
            &ledger,
            project.path(),
            &ariadne::contracts::WorkflowExecutionLimits::default(),
        );

        let outbound = server
            .join()
            .unwrap_or_default()
            .first()
            .cloned()
            .unwrap_or_default();
        // 正向断言放最前，理由同上一条用例。
        //
        // 接线被摘掉时症状是**零次出站请求**（占位符原样留下 ⇒
        // `author_template_prefix` 把它当未知变量 ⇒ 渲染器 fail-loud ⇒ 发请求前就失败），
        // 所以这条断言必须自己把「出站为空」这个形态解释清楚，
        // 否则红在「零次请求」上会让人以为是测试基建没接上。
        assert!(
            outbound.contains(&as_json_fragment(&expected_first_line)),
            "summarizer 第 {index} 种写法没有展开成 {}。出站请求{}。\
             summarizer **不走** render_writing_node_prompt（它有自己的四步分调用链），\
             展开必须在 execute_summarizer_node_with_optional_search_tools 里另接一次",
            agent.prompt_key(),
            if outbound.is_empty() {
                "为空（占位符没展开 ⇒ 被当成未知变量 ⇒ 渲染器在发请求前 fail-loud）"
            } else {
                "非空但不含该全文"
            }
        );
        assert!(
            !outbound.contains(&as_json_fragment(&format!("{{{{{literal}}}}}"))),
            "summarizer 第 {index} 种写法的占位符**原样进了请求体**：\
             模型把这行字当成了自己的角色设定，而节点照样会报成功"
        );
    }
}

// ════════════════════════════════════════════════════════
// 判据 5：覆盖面 —— 9 个 agent 逐一对应，不是「存在性」
// ════════════════════════════════════════════════════════

/// 9 个 agent **每一个**都要在**每一份**语言包里有占位符字面量，且互不重复。
///
/// 守的是逐一对应而非存在性：漏掉一个 agent 的形态是「那门语言界面里手打的写法
/// 认不出」，而 `DisplayNameService` 缺 key 会静默回落中文 ⇒ 界面照样显示、
/// 不报任何错，唯一发现途径就是这条用例。
///
/// 重复也必须拦：两个 agent 共用同一写法时解析不出该给谁，实现会 fail-loud
/// （`DefaultPromptPlaceholderTable::ambiguous`），但那时用户已经卡住了。
#[test]
fn every_agent_has_a_distinct_placeholder_in_every_language_pack() {
    // 输入源独立读盘（理由见 `display_name_packs_from_disk`），
    // 这样「实现少 include 一份」会红在下面的接受集合断言上，而不是红在这里。
    let packs = display_name_packs_from_disk();

    for (pack_index, pack) in packs.iter().enumerate() {
        let mut seen: std::collections::BTreeMap<String, &'static str> =
            std::collections::BTreeMap::new();
        for agent in WritingAgentKind::ALL {
            let key = ariadne::rag::default_prompt::default_prompt_placeholder_key(agent);
            let value = pack.get(&key).map(String::as_str).unwrap_or_default();
            assert!(
                !value.trim().is_empty(),
                "第 {pack_index} 份语言包缺 {key}；\
                 缺一个 agent = 那门语言界面里手打的写法认不出，而界面不会报错"
            );
            let normalized = ariadne::rag::default_prompt::normalize_placeholder_literal(value);
            if let Some(previous) = seen.insert(normalized, agent.node_type()) {
                panic!(
                    "第 {pack_index} 份语言包里 {} 与 {} 的占位符写法归一化后相同；\
                     解析时无法判断该给哪个 agent",
                    previous,
                    agent.node_type()
                );
            }
        }
    }

    // 逐语言逐 agent 地正向检查：磁盘上**每一条**写法都必须被生产的接受集合认出，
    // 且归一到正确的那个 agent。
    //
    // 这才是「并集」的真正判据。只比接受集合的**条数**是不够的：条数够也可能
    // 认错 agent（例如把 en 的 writer 归到 polisher），而那种缺陷的形态是
    // 「英文界面新建的写作节点拿到了别人的角色设定」。
    let table = ariadne::rag::default_prompt::placeholder_table()
        .expect("内联语言包必须能建出接受集合");
    for (pack_index, pack) in packs.iter().enumerate() {
        for agent in WritingAgentKind::ALL {
            let key = ariadne::rag::default_prompt::default_prompt_placeholder_key(agent);
            let Some(literal) = pack.get(&key) else {
                continue;
            };
            let resolved = table
                .resolve(literal)
                .unwrap_or_else(|error| panic!("解析 {key} 的写法出错：{error:?}"));
            assert_eq!(
                resolved,
                Some(agent),
                "第 {pack_index} 份语言包里 {key} 的写法没有归一到 {}；\
                 并集没把这门语言并进来（实现少 include 了一份？），\
                 后果是这门语言界面里手打的写法认不出、或认成了别的 agent",
                agent.node_type()
            );
        }
    }
}
