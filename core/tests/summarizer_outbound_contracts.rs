//! 总结链（Summarizer）真实出站请求契约（2026-08-17）。
//!
//! **本文件回答一个问题**：设置页只暴露了一份 summarizer 角色提示词，
//! 那么「三层架构、形式化划分段落、分次总结」是真的做到了，还是只是文档里写着？
//!
//! 判据刻意取**真实 HTTP 出站请求体**，不取返回值、不取 draft 结构：
//!
//! - 用 mock provider 断言「调了 4 次」证不了链路真通——CLAUDE.md 记着
//!   「mock 会掩盖一整类缺陷」，IPC 的 BOM 那条只在真实进程管道上才复现。
//!   这里自建 HTTP 接收端，捕获**真正发出去的字节**。
//! - 断言 draft 里有 4 层数据也不够：一个把 4 步拼进 1 次调用、
//!   让模型一口气返回全部层级的实现，draft 结构完全一样。
//!   **只有数出站请求条数、并逐条比对提示词文本，才能区分「分次」与「一次」。**
//!
//! 已有的 `rag_contracts.rs::f15_f16_...` 断言了 `requests.len() == 4`，
//! 但那是进程内 mock provider。本文件是它在真实 HTTP 边界上的复核。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::config::{ModelConfig, ProviderConfig};
use ariadne::contracts::{ProviderCapability, ProviderType};
use ariadne::costs::SqliteCostLedger;
use ariadne::providers::OpenAiCompatibleLlmProvider;
use ariadne::rag::resources::load_prompt_resources;
use ariadne::rag::summarizer::{SummarizerConfig, SummarizerExecutor};
use ariadne::rag::SummaryGenerationContext;
use serde_json::{json, Value};

// ════════════════════════════════════════════════════════
// 真实 HTTP 接收端
// ════════════════════════════════════════════════════════

/// 读满一个 HTTP 请求：先读到头结束，再按 Content-Length 补齐 body。
///
/// 不能只 `read` 一次就当拿到全部——中文正文一个字 3 字节，
/// 四步指令里塞了正文与历史上下文，单次 read 很可能只拿到前半段，
/// 于是「提示词里没有某段文本」这种断言会**因为读少了而假失败**。
fn read_full_request(stream: &mut std::net::TcpStream) -> String {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 65_536];
    loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        raw.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&raw);
        if let Some(head_end) = text.find("\r\n\r\n") {
            let want: usize = text[..head_end]
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse().ok())
                })
                .unwrap_or(0);
            // 用字节数比，不用字符数：中文 body 的 Content-Length 是字节口径，
            // 按 char 数比较永远凑不满，会把正常请求读成「还没收完」而卡死。
            if raw.len() - (head_end + 4) >= want {
                break;
            }
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

/// 按顺序返回预设响应，并交回每一轮收到的**完整请求原文**。
///
/// 多返回一条 `extra_capacity` 的余量：若被测代码发出了**多于预期**的调用，
/// 这里能接住并记录，从而让「调用次数」断言看到真实数字，
/// 而不是让被测代码卡在连不上的第 5 次请求上超时——
/// **超时的报错信息说不出「多调了一次」，而这正是本文件要区分的事**。
fn spawn_recording_llm(
    responses: Vec<String>,
    extra_capacity: usize,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let total = responses.len() + extra_capacity;
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for index in 0..total {
            // 预期内的轮次给足等待；余量轮次只短等一下——没有多余请求是正常情况，
            // 不能让它把测试拖成超时失败。
            let wait = if index < responses.len() {
                Duration::from_secs(15)
            } else {
                Duration::from_millis(600)
            };
            let deadline = Instant::now() + wait;
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break Some(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            break None;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break None,
                }
            };
            let Some(stream) = stream.as_mut() else {
                if index < responses.len() {
                    panic!("等待第 {} 次 LLM 请求超时", index + 1);
                }
                break;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            seen.push(read_full_request(stream));
            let body = responses
                .get(index)
                .cloned()
                .unwrap_or_else(|| chat_response(&json!({})));
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

fn chat_response(content: &Value) -> String {
    json!({
        "model": "m",
        "choices": [{
            "message": {"content": content.to_string(), "tool_calls": []},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 20, "completion_tokens": 8}
    })
    .to_string()
}

/// 章节正文：6 行中文。刻意用中文——UTF-8 下一个汉字 3 字节，
/// 能同时检出「按字节切行」和「按 char 索引当字节偏移」两类缺陷。
const CHAPTER_TEXT: &str =
    "李砚在渡口等了一夜。\n沈昀来时天刚亮。\n两人没有说话。\n船工催了第三遍。\n沈昀转身走了。\n李砚独自登船。\n";

/// 四步各自的合法 JSON 响应。行范围必须**有序、无缝、覆盖全文**，
/// 否则 `summarize_segments` 会拒绝——这正是「形式化划分」的判据之一。
fn four_step_responses() -> Vec<String> {
    vec![
        chat_response(&json!({
            "segments": [
                {"number": "1", "summary": "李砚在渡口等沈昀", "start_line": 1, "end_line": 3},
                {"number": "2", "summary": "沈昀离去，李砚登船", "start_line": 4, "end_line": 6},
            ]
        })),
        chat_response(&json!({
            "events": [{
                "event_id": "evt-1",
                "summary": "李砚与沈昀在渡口分别",
                "status": "ongoing",
                "segment_ids": ["chapter-1::seg-1", "chapter-1::seg-2"]
            }]
        })),
        chat_response(&json!({
            "summary": "本章李砚与沈昀在渡口分别，李砚独自登船。",
            "realized_changes": [],
            "foreshadowing_updates": []
        })),
        chat_response(&json!({
            "stage_id": "stage-1",
            "stage_summary": "第一阶段：李砚离乡南下。",
            // 必须是 true：`SummaryGenerationContext::default()` 的 stages 为空，
            // 后端会双向校验（已有阶段报成新的、或选了不存在的阶段，都拒绝）。
            // 填 false 会得到 "selected an unknown existing stage" —— 那是**校验在正常工作**，
            // 不是缺陷；第一次总结本来就是新阶段。
            "is_new_stage": true
        })),
    ]
}

fn provider_for(base_url: String) -> OpenAiCompatibleLlmProvider {
    OpenAiCompatibleLlmProvider::new(
        ProviderConfig {
            provider_id: "primary".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "本地录制端".to_owned(),
            enabled: true,
            base_url: Some(base_url),
            api_key: None,
            models: vec![ModelConfig {
                model_id: "m".to_owned(),
                capability: ProviderCapability::Llm,
                max_context_tokens: None,
                input_cost_per_million_tokens: Some(1.0),
                output_cost_per_million_tokens: Some(2.0),
            }],
        },
        None,
    )
    .expect("构造真实 HTTP provider 应当成功")
}

fn summarizer_config(prompt_template: Option<String>) -> SummarizerConfig {
    SummarizerConfig {
        provider_id: "primary".to_owned(),
        model_id: "m".to_owned(),
        chapter_document_id: "/tmp/chapter-1.md".to_owned(),
        run_id: None,
        timeout_ms: 30_000,
        cancellation: ariadne::contracts::CancellationToken::new(),
        dispatch_authorization: Default::default(),
        prompt_template,
        generation_context: SummaryGenerationContext::default(),
        workflow_operation: None,
    }
}

/// 从捕获到的 HTTP 请求原文里取出 body 中的全部文本内容。
fn request_body(raw: &str) -> String {
    raw.find("\r\n\r\n")
        .map(|i| raw[i + 4..].to_owned())
        .unwrap_or_default()
}

// ════════════════════════════════════════════════════════
// 用例
// ════════════════════════════════════════════════════════

/// 判据一：**真实 HTTP 上确实发出 4 次独立请求**，不是 1 次拼完。
///
/// 这是「分次总结」的核心判据。变异测试点：若把四步合并成一次调用，
/// 本用例必须失败——`seen.len()` 会变成 1。
#[test]
fn summarizer_emits_exactly_four_separate_http_calls() {
    let (base_url, server) = spawn_recording_llm(four_step_responses(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, summarizer_config(None));

    let draft = executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");

    let seen = server.join().unwrap();
    assert_eq!(
        seen.len(),
        4,
        "总结链必须在真实 HTTP 上发出恰好 4 次独立请求（分次总结），实际 {} 次。\
         若为 1 次，说明四步被拼成了一次调用——draft 结构看不出这个区别，只有请求条数能",
        seen.len()
    );

    // 四层产物都要有，否则「三层架构」只是把一次结果切开摆放。
    assert_eq!(draft.segments.len(), 2, "故事段层必须有产物");
    assert_eq!(draft.events.len(), 1, "事件层必须有产物");
    assert!(draft.chapter_summary.is_some(), "章节层必须有产物");
    assert!(draft.stage_summary.is_some(), "阶段层必须有产物");
}

/// 判据二：**四次请求携带的是四份不同的分步提示词**，不是同一份重复四遍。
///
/// 只数「4 次」不够——四次都发同一个通用提示词、靠模型自己猜该做哪一步，
/// 请求条数一样是 4。必须逐条比对提示词正文。
#[test]
fn summarizer_four_calls_carry_four_distinct_step_prompts() {
    let (base_url, server) = spawn_recording_llm(four_step_responses(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, summarizer_config(None));

    executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");

    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 4, "应当有 4 次请求");
    let bodies: Vec<String> = seen.iter().map(|raw| request_body(raw)).collect();

    // 每一步的**特征句**取自 prompt_list.json 里该步提示词的独有措辞。
    // 用独有句而不是 key 名：key 名不会出现在出站请求里，出站的是提示词正文。
    let markers = [
        ("segments", "切点落在场景、时间或视角真正转换的地方"),
        ("events", "同一件事跨章续写时必须并入原有事件"),
        ("chapter_summary", "这份总结会在几十章之后代替正文被读到"),
        ("stage_summary", "只有故事的处境、地点或阶段目标确实换了"),
    ];

    for (step, marker) in markers {
        let hits = bodies.iter().filter(|body| body.contains(marker)).count();
        assert_eq!(
            hits, 1,
            "步骤 {step} 的专属提示词必须恰好出现在 1 次请求里（实际 {hits} 次）。\
             0 次 = 该步没用自己的提示词；>1 次 = 提示词被重复塞进多步，分工失效"
        );
    }

    // 反向断言：四份 body 两两不同。若实现退化成「同一提示词发四遍」，
    // 上面的 marker 检查可能仍过（各 marker 各命中一次），但 body 会重复。
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            assert_ne!(
                bodies[i], bodies[j],
                "第 {} 与第 {} 次请求的内容完全相同——分步总结退化成了重复调用",
                i + 1,
                j + 1
            );
        }
    }
}

/// 判据三：**段落划分是形式化的**——行范围必须落在真实 UTF-8 字符边界上，
/// 且覆盖全文、互不重叠。
///
/// 「形式化划分」的实质不是「模型返回了 start_line/end_line」，
/// 而是**后端会拿这些数字去正文里真的定位，并拒绝不合法的划分**。
/// 判据取 `SourceSpan` 的字节偏移能否切出原文对应片段。
#[test]
fn summarizer_segment_ranges_are_real_utf8_byte_spans_covering_whole_chapter() {
    let (base_url, server) = spawn_recording_llm(four_step_responses(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, summarizer_config(None));

    let draft = executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");
    let _ = server.join();

    assert_eq!(draft.segments.len(), 2);
    let total = CHAPTER_TEXT.len() as u64;

    // 1. 首段从 0 开始、末段到正文结尾——即完整覆盖。
    assert_eq!(draft.segments[0].source.range.start, 0, "首段必须从正文开头起");
    assert_eq!(
        draft.segments[1].source.range.end, total,
        "末段必须到正文结尾，否则尾部内容不属于任何故事段"
    );

    // 2. 段间无缝隙、无重叠。
    assert_eq!(
        draft.segments[0].source.range.end, draft.segments[1].source.range.start,
        "相邻故事段之间不得有缝隙或重叠"
    );

    for (index, segment) in draft.segments.iter().enumerate() {
        let start = segment.source.range.start as usize;
        let end = segment.source.range.end as usize;

        // 3. 偏移必须落在 UTF-8 字符边界上。若实现按字节找 '\n' 或把
        //    char 索引当字节偏移，这里会 panic —— 这正是要钉住的那类缺陷。
        assert!(
            CHAPTER_TEXT.is_char_boundary(start) && CHAPTER_TEXT.is_char_boundary(end),
            "故事段 {} 的偏移 [{start},{end}) 落在非 UTF-8 字符边界上",
            index + 1
        );

        // 4. 切出来的必须是真正的正文片段（形式化定位的最终判据）。
        let slice = &CHAPTER_TEXT[start..end];
        assert!(!slice.trim().is_empty(), "故事段 {} 切出了空片段", index + 1);
        assert!(
            slice.ends_with('\n'),
            "按行划分的片段应当以换行结尾，实际 {slice:?}"
        );
    }

    // 5. 两段拼起来必须**逐字节等于**原文。这一条是覆盖性的终极判据：
    //    任何缝隙、重叠、越界都会让拼接结果与原文不同。
    let joined = format!(
        "{}{}",
        &CHAPTER_TEXT[draft.segments[0].source.range.start as usize
            ..draft.segments[0].source.range.end as usize],
        &CHAPTER_TEXT[draft.segments[1].source.range.start as usize
            ..draft.segments[1].source.range.end as usize],
    );
    assert_eq!(joined, CHAPTER_TEXT, "故事段拼接后必须逐字节还原正文");
}

/// 判据四：**不合法的段落划分必须被拒绝**（形式化 = 有强制力）。
///
/// 若后端照单全收模型给的行号，「形式化划分」就只是个说法。
/// 这里让模型返回有缝隙的划分（跳过第 3 行），断言总结失败。
#[test]
fn summarizer_rejects_segment_ranges_with_gaps() {
    let mut responses = four_step_responses();
    // 第 1 步改成「1-2 行 + 4-6 行」，第 3 行无人认领。
    responses[0] = chat_response(&json!({
        "segments": [
            {"number": "1", "summary": "前段", "start_line": 1, "end_line": 2},
            {"number": "2", "summary": "后段", "start_line": 4, "end_line": 6},
        ]
    }));
    let (base_url, server) = spawn_recording_llm(responses, 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, summarizer_config(None));

    let result = executor.summarize_chapter("chapter-1", CHAPTER_TEXT);
    let _ = server.join();

    let error = result.expect_err("有缝隙的段落划分必须被拒绝，否则第 3 行的内容会静默丢失");
    let message = error.to_string();
    assert!(
        message.contains("ordered") || message.contains("gap") || message.contains("expected"),
        "拒绝理由应当指明行范围问题，实际：{message}"
    );
}

/// 判据五之二：**作者模板里的占位符在四步的出站请求体里都已展开**。
///
/// U175 遗留（2026-08-18 修）：`author_template_prefix` 此前原文直传。
/// 而产品自带的 `node_template.summarizer.default` 就是
/// `{{角色设定}}\n\n本章定稿正文：\n{{当前章节正文}}` ⇒ 手动粘贴该模板时
/// 占位符字面量进请求体，模型读到「本章定稿正文：{{当前章节正文}}」。
///
/// ⚠️ **这条比 U114 那条更隐蔽**：总结链不校验模板 ⇒ 节点**照样跑完、
/// 照样报成功**，四层总结全部基于一份空洞指令产出。用户拿到的是
/// 「看起来正常」的总结，而它没读过正文。所以判据必须取**出站字节**，
/// 取「运行是否成功」在缺陷版本下照样绿。
///
/// 四步都查：前缀在四步都拼一次，只在第一步展开等于后三步仍然发出字面量。
#[test]
fn author_template_placeholders_are_expanded_in_all_four_steps() {
    let (base_url, server) = spawn_recording_llm(four_step_responses(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    // 与产品自带 node_template.summarizer.default 同形（含两个占位符）。
    let executor = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        summarizer_config(Some(
            "【作者口径】人名一律用全名。\n\n本章定稿正文：\n{{当前章节正文}}".to_owned(),
        )),
    );

    executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("含占位符的作者模板应当能跑完");

    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 4, "应当有 4 次请求");
    for (index, raw) in seen.iter().enumerate() {
        let body = request_body(raw);
        assert!(
            !body.contains("{{当前章节正文}}"),
            "第 {} 步的出站请求里占位符仍是**字面量**——模型读到的是\
             「本章定稿正文：{{{{当前章节正文}}}}」，而节点会照样报成功（U175 遗留）",
            index + 1
        );
        // 只断言「不含 {{}}」不够：静默替换成空串同样能过那一条，
        // 而那比留字面量更糟（模型会以为这章本来就没有正文）。
        assert!(
            body.contains("李砚在渡口等了一夜"),
            "第 {} 步占位符被消掉了却没换上真实正文——静默置空",
            index + 1
        );
    }
}

/// 判据五之三：**作者模板里拼错的变量名当场报错，不静默发出去**。
///
/// 与 `audit_prompt_with_an_unknown_variable_fails_loudly` 同一条契约，
/// 只是落在节点主模板这条路上。缺了这条，作者把 `{{当前章节正文}}`
/// 打成 `{{章节正文}}` 时会得到一份「基于空洞指令的成功总结」。
#[test]
fn author_template_with_an_unknown_variable_fails_loudly() {
    // 预置**零个**响应：本用例期望一次请求都不发出。
    // 传 four_step_responses() 会让录制端在「等第 1 次请求」处 panic
    // （它对预期内的轮次等 15 秒然后 `panic!("等待第 N 次 LLM 请求超时")`），
    // 那个 panic 会盖住真正要验的断言——我第一版就踩了这个。
    // `extra_capacity: 2` 仍保留：若被测代码**真的**发了请求，
    // 这里能接住并让下面的 `seen.len() == 0` 报出真实数字。
    let (base_url, server) = spawn_recording_llm(Vec::new(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        summarizer_config(Some("对照 {{章节正文}} 做总结".to_owned())),
    );

    let error = executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect_err("未知变量必须报错，不能静默把字面量发出去");
    assert!(
        error.to_string().contains("章节正文"),
        "报错信息里没指名是哪个变量拼错了：{error}"
    );

    // 一次请求都不该发出：错在模板上，发出去只是白花钱换一份错总结。
    let seen = server.join().unwrap();
    assert_eq!(
        seen.len(),
        0,
        "渲染失败后仍发出了请求——那些调用注定基于错的指令，还要计费"
    );
}

/// 判据五：**作者在设置页填的那一份角色提示词，会进入全部四步**。
///
/// 这解释了「只看见一个提示词」为什么不等于「只有一层」：
/// 用户可编辑的是角色设定，四份分步指令是内置的，两者拼接后发出。
/// 若作者提示词只进第一步，后三步就会丢失作者定下的口径。
#[test]
fn author_prompt_template_reaches_every_one_of_the_four_steps() {
    let marker = "【作者口径】人名一律用全名，不要用小名。";
    let (base_url, server) = spawn_recording_llm(four_step_responses(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(
        &provider,
        &ledger,
        &prompts,
        summarizer_config(Some(marker.to_owned())),
    );

    executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");

    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 4, "应当有 4 次请求");
    for (index, raw) in seen.iter().enumerate() {
        let body = request_body(raw);
        assert!(
            body.contains("人名一律用全名"),
            "第 {} 步的出站请求里没有作者提示词——该步会丢失作者定下的口径",
            index + 1
        );
    }
}

/// 判据六：**三层是真的三层**——故事段、事件、章节/阶段各自独立成层，
/// 且层与层之间有真实引用关系（事件引用故事段 id、故事段引用章节 id）。
///
/// 若只是把一份总结复制三遍换个字段名，引用关系会对不上。
#[test]
fn summary_layers_reference_each_other_by_real_ids() {
    let (base_url, server) = spawn_recording_llm(four_step_responses(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, summarizer_config(None));

    let draft = executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");
    let _ = server.join();

    // 第一层 → 章节：每个故事段都挂在本章上。
    for segment in &draft.segments {
        assert_eq!(
            segment.chapter_id, "chapter-1",
            "故事段必须归属到本章，否则跨章检索会串台"
        );
        assert!(
            segment.segment_id.starts_with("chapter-1::"),
            "故事段 id 应当带章节前缀，实际 {}",
            segment.segment_id
        );
    }

    // 第二层 → 第一层：事件引用的 segment_id 必须真实存在。
    let known: Vec<&str> = draft
        .segments
        .iter()
        .map(|s| s.segment_id.as_str())
        .collect();
    let mut linked = 0usize;
    for event in &draft.events {
        for segment_id in &event.segment_ids {
            assert!(
                known.contains(&segment_id.as_str()),
                "事件引用了不存在的故事段 {segment_id}——层间引用断裂，\
                 这条线在后文就串不起来了"
            );
            linked += 1;
        }
    }
    assert!(linked > 0, "事件层必须至少引用一个故事段，否则两层没有真实关系");

    // 第三层：章节总结与阶段概括各自存在且非空。
    let chapter = draft.chapter_summary.as_deref().unwrap_or_default();
    let stage = draft.stage_summary.as_deref().unwrap_or_default();
    assert!(!chapter.trim().is_empty(), "章节总结不得为空");
    assert!(!stage.trim().is_empty(), "阶段概括不得为空");
    assert_ne!(
        chapter, stage,
        "章节总结与阶段概括不得完全相同——那说明阶段层没有真正独立总结"
    );
}

/// 判据七：**后一步真的消费前一步的产物**（分次的意义在于递进）。
///
/// 若每步都只拿原始正文、彼此不传递，那就是「四次平行调用」而非「四步流水线」。
/// 判据：第 2 步请求里必须出现第 1 步产出的段落概括文本。
#[test]
fn later_steps_consume_earlier_step_outputs() {
    let (base_url, server) = spawn_recording_llm(four_step_responses(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, summarizer_config(None));

    executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");

    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 4, "应当有 4 次请求");
    let bodies: Vec<String> = seen.iter().map(|raw| request_body(raw)).collect();

    // 第 1 步返回的段落概括（见 four_step_responses 第一条）。
    let segment_summary = "李砚在渡口等沈昀";
    let downstream = bodies[1..]
        .iter()
        .filter(|body| body.contains(segment_summary))
        .count();
    assert!(
        downstream >= 1,
        "第 1 步产出的段落概括没有出现在任何后续步骤的请求里——\
         四步是平行的，不是流水线；那样分次调用只多花钱不增效果"
    );

    // 第 3 步（章节总结）应当能看到事件层的信息。
    // 用事件概括文本作探针——`EventDto` 没有 title 字段，事件的可见内容就是 summary。
    let event_summary = "李砚与沈昀在渡口分别";
    assert!(
        bodies[2].contains(event_summary) || bodies[3].contains(event_summary),
        "事件层产物没有进入章节或阶段总结的请求——上层总结看不到下层结论"
    );
}

// ════════════════════════════════════════════════════════
// Auto Mode 自动审阅：U114 的遗漏路径（2026-08-18）
// ════════════════════════════════════════════════════════
//
// U114 只接了**写作节点**那条渲染路径（`render_writing_node_prompt`）。
// 自动审阅走的是 `audit_confirmation` 里一条独立的 `format!` 拼串，
// 审批提示词原文直传、**不过 `render_prompt_template`**。
//
// 判据同样取真实出站 HTTP 请求体（报告的验收要求，与本文件其余用例同一标准）：
// 断言「`{{}}` 已被替换成真实内容」必须看**发出去的字节**——
// 看返回值看不出提示词是什么样，看 draft 结构更看不出。

/// 跑完四步再审一项，返回审阅那一次请求的 body。
///
/// 走完整的 `summarize_chapter` 而不是手搓 draft：审阅的候选内容来自 draft，
/// 手搓的 draft 可能和真实产物形状不同，那样测的就不是生产路径。
fn audit_request_body(
    approval_prompt: &str,
    kind: ariadne::rag::models::ConfirmationKind,
    context: SummaryGenerationContext,
) -> String {
    // 阶段那一步的响应必须与传入的 `stages` 一致：后端双向校验
    // 「已有阶段报成新的」和「选了不存在的阶段」都拒绝（见 `summarize_stage`）。
    // 上下文里已有 stage-1 时就得 is_new_stage=false，否则被合法地拒掉——
    // 那是校验在正常工作，不是缺陷。
    let has_known_stage = !context.stages.is_empty();
    let mut responses = four_step_responses();
    if has_known_stage {
        let stage_id = context.stages[0].stage_id.clone();
        responses.pop();
        responses.push(chat_response(&json!({
            "stage_id": stage_id,
            "stage_summary": "第一阶段：李砚离乡南下。",
            "is_new_stage": false
        })));
    }
    responses.push(chat_response(&json!({
        "approved": true,
        "reason": "总结与正文一致"
    })));
    let (base_url, server) = spawn_recording_llm(responses, 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let mut config = summarizer_config(None);
    config.generation_context = context;
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, config);

    let draft = executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");
    executor
        .audit_confirmation(kind, approval_prompt, CHAPTER_TEXT, &draft)
        .expect("自动审阅应当成功");

    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 5, "四步总结 + 一次审阅 = 5 次请求");
    request_body(&seen[4])
}

/// 判据七：**审批提示词里的占位符在出站请求体里已被替换成真实内容**。
///
/// 这是 U114 遗漏路径的核心判据。缺陷版本下审阅模型收到的是字面量
/// `{{当前章节正文}}`——它会在「审的是占位符」的前提下给出通过判定，
/// 属安全缺口而非文案瑕疵。
///
/// 变异测试点：摘掉 `render_approval_prompt` 调用后，
/// 出站 body 里会出现 `{{当前章节正文}}` 字面量 ⇒ 本用例失败。
#[test]
fn audit_prompt_placeholders_are_expanded_in_the_real_outbound_request() {
    let body = audit_request_body(
        "审这份总结前先读一遍正文：\n{{当前章节正文}}",
        ariadne::rag::models::ConfirmationKind::SegmentSummary,
        SummaryGenerationContext::default(),
    );

    assert!(
        !body.contains("{{当前章节正文}}"),
        "审批提示词里的占位符以**字面量**发给了审阅模型——\
         它会在「审的是 {{{{}}}} 而不是正文」的前提下给出虚假通过（U114 遗漏路径）"
    );
    // 正文里的独特词组：确认替换进去的是**真内容**，不是空串。
    // 只断言「不含 {{}}」是不够的——静默替换成空串同样能通过那一条。
    assert!(
        body.contains("李砚在渡口等了一夜"),
        "占位符被消掉了却没换上真实正文——静默置空比留着字面量更糟：\
         审阅模型会以为这一段本来就没有内容"
    );
}

/// 判据八：**拼错的变量名当场报错，不静默发出去**。
///
/// 作者把 `{{当前章节正文}}` 打成 `{{章节正文}}` 时必须 fail-loud 指名变量，
/// 否则那次审阅是在「少了一段上下文」的情况下做的，而没人会知道。
#[test]
fn audit_prompt_with_an_unknown_variable_fails_loudly() {
    let (base_url, server) = spawn_recording_llm(four_step_responses(), 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, summarizer_config(None));

    let draft = executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");
    let error = executor
        .audit_confirmation(
            ariadne::rag::models::ConfirmationKind::SegmentSummary,
            "对照 {{章节正文}} 审这份总结",
            CHAPTER_TEXT,
            &draft,
        )
        .expect_err("未知变量必须报错，不能静默把字面量发出去");

    let message = error.to_string();
    assert!(
        message.contains("章节正文"),
        "报错信息里没指名是哪个变量拼错了，作者无从修：{message}"
    );

    // 审阅那一次请求**根本不该发出去**：错在提示词上，发出去只是白花钱。
    let seen = server.join().unwrap();
    assert_eq!(
        seen.len(),
        4,
        "渲染失败后仍发出了审阅请求——那次调用注定审的是错东西，还要计费"
    );
}

/// 判据九：**不同确认类型拿到的上下文不同**（报告验收第 3 条）。
///
/// 审「阶段总结」要能对照阶段总纲；审「故事段」不需要它。
/// 无差别塞全部上下文会撑爆审阅调用的 token，并让模型在噪声里
/// 找不到该对照的那一条——所以「都给」和「都不给」一样是错的。
#[test]
fn audit_context_differs_by_confirmation_kind() {
    use ariadne::rag::models::{ConfirmationKind, SummaryStageContext};

    let stage_marker = "第一阶段总纲：李砚离乡，沈昀留守。";
    let context = SummaryGenerationContext {
        stages: vec![SummaryStageContext {
            stage_id: "stage-1".to_owned(),
            stage_summary: Some(stage_marker.to_owned()),
            chapter_summaries: Default::default(),
        }],
        current_stage_id: Some("stage-1".to_owned()),
        ..Default::default()
    };

    // 阶段总结：`{{阶段总纲}}` 必须解析得出。
    let stage_body = audit_request_body(
        "对照阶段总纲审这份阶段总结：\n{{阶段总纲}}",
        ConfirmationKind::StageSummary,
        context.clone(),
    );
    assert!(
        stage_body.contains(stage_marker),
        "审阶段总结时拿不到阶段总纲——模型只能看它是否自洽，判不了它与规划是否一致"
    );

    // 故事段：同一个变量**不该**可用。审段落划分不需要阶段总纲，
    // 给了只是噪声；这里用 fail-loud 反证「上下文确实按类型区分」，
    // 而不是所有类型都拿到同一个大包。
    let mut responses = four_step_responses();
    responses.pop();
    responses.push(chat_response(&json!({
        "stage_id": "stage-1",
        "stage_summary": "第一阶段：李砚离乡南下。",
        "is_new_stage": false
    })));
    let (base_url, server) = spawn_recording_llm(responses, 2);
    let provider = provider_for(base_url);
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let prompts = load_prompt_resources().unwrap();
    let mut config = summarizer_config(None);
    config.generation_context = context;
    let executor = SummarizerExecutor::new(&provider, &ledger, &prompts, config);
    let draft = executor
        .summarize_chapter("chapter-1", CHAPTER_TEXT)
        .expect("四步总结应当成功");
    let result = executor.audit_confirmation(
        ConfirmationKind::SegmentSummary,
        "对照阶段总纲审这份段落划分：\n{{阶段总纲}}",
        CHAPTER_TEXT,
        &draft,
    );
    assert!(
        result.is_err(),
        "段落划分的审阅也拿到了阶段总纲——说明上下文没按确认类型区分，\
         是无差别塞了全部（撑 token 且降判准）"
    );
    let _ = server.join();
}

/// 判据十：**不含占位符的审批提示词原样发出**，不因「上下文凑不齐」而失败。
///
/// 内置那 10 条审批提示词（`prompt_list.json` 的 `auto_audit.*`）都不带占位符。
/// 若把渲染做成无条件必经之路，它们会一并受装配失败牵连——
/// 那是拿一条安全修复换掉整个 Auto Mode 的可用性。
#[test]
fn audit_prompt_without_placeholders_passes_through_untouched() {
    let plain = "你在代替作者过一遍这份总结。只看写的是不是正文里确实发生的。";
    let body = audit_request_body(
        plain,
        ariadne::rag::models::ConfirmationKind::ChapterSummary,
        SummaryGenerationContext::default(),
    );
    assert!(
        body.contains("只看写的是不是正文里确实发生的"),
        "不含占位符的审批提示词没原样进入请求体"
    );
}

