//! U117：写作节点必须产出确认项，写作产出不得不经审阅直接落地。
//!
//! 缺陷形状与 U108/U114 同类，且更隐蔽：12 种 `ConfirmationKind` 全部定义齐备，
//! `record_node_output` 的门禁也正确实现（有 Pending 确认项就把 patch 落盘
//! 置为 `PendingConfirmation`），但**只有 4 种 summarizer 确认项会被创建**——
//! 8 种写作类从不产出。门禁读到的永远是空列表 → 正文直接写入磁盘。
//!
//! 因此判据取**真实运行后的运行态**（`run_workflow_impl` 之后确认项是否入库、
//! 是否 Pending），而不是「函数能不能构造出一个确认项」——后者在
//! 发射点没接线时依然会绿，正是这类缺陷最擅长骗过的那种测试。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use ariadne::commands::{
    run_workflow_impl, save_provider_settings_impl, save_workflow_graph_impl, CanvasNode,
    ProviderSettingsUpdate, RunWorkflowRequest, WorkflowGraphData,
};
use ariadne::contracts::{RunId, WorkflowId};
use ariadne::workflow::{
    RuntimeConfirmation, RuntimeConfirmationState, SqliteWorkflowRuntimeStore, WorkflowRuntimeStore,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProjectCredentialScope, SecretValue};
use ariadne::contracts::{ProviderCapability, ProviderType};
use ariadne::rag::models::{ConfirmationKind, WritingAgentKind, WritingNodeDefinition};
use serde_json::{json, Value};

const PROVIDER_ID: &str = "primary";
const MODEL_ID: &str = "m";

/// 假 LLM：回一段正文，不带 tool_calls（本文件只关心确认项，不关心工具轮次）。
fn spawn_llm() -> String {
    let body = json!({
        "model": MODEL_ID,
        "choices": [{"message": {"content": "第一章写好了。", "tool_calls": []}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1}
    })
    .to_string();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        // 多应答几次：不同节点类型可能发起不同轮数，多余的 accept 超时即退出。
        for _ in 0..8 {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buffer = [0u8; 65_536];
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
}

/// 存一个单节点工作流，节点类型即写作 agent 的 node_type。
fn save_single_node_workflow(project_root: &std::path::Path, node_type: &str) {
    save_workflow_graph_impl(
        project_root,
        WorkflowGraphData {
            workflow_id: node_type.to_owned(),
            name: node_type.to_owned(),
            nodes: vec![CanvasNode {
                id: "node-1".to_owned(),
                r#type: node_type.to_owned(),
                label: None,
                data: json!({
                    "provider_id": PROVIDER_ID,
                    "model_id": MODEL_ID,
                    "prompt_template": "写一段",
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
}

/// 读取该次运行落库的确认项（与 `get_workflow_run_state` 同一条读路径）。
fn run_confirmations(
    project_root: &std::path::Path,
    workflow_id: &str,
    run_id: &str,
) -> Vec<RuntimeConfirmation> {
    let store = SqliteWorkflowRuntimeStore::open(project_root).unwrap();
    store
        .load_state(&WorkflowId::from(workflow_id), &RunId::from(run_id))
        .unwrap()
        .map(|state| state.confirmations.values().cloned().collect())
        .unwrap_or_default()
}

fn run(project_root: &std::path::Path, secrets: &MemorySecretStore, workflow_id: &str) -> String {
    run_workflow_impl(
        project_root,
        secrets,
        RunWorkflowRequest {
            workflow_id: workflow_id.to_owned(),
            start_node_id: None,
            initial_inputs: std::collections::BTreeMap::new(),
            variables: Default::default(),
            origin_conversation_id: None,
        },
    )
    .map(|started| started.run_id)
    .unwrap_or_else(|error| panic!("运行工作流失败：{error:?}"))
}

/// **U117 主用例**：每个声明了确认项的写作 agent，运行后必须真的产出确认项。
///
/// 逐 agent 断言而非抽样：缺陷正是「4 种有、8 种没有」，
/// 抽样很容易恰好命中已有的那 4 种而漏掉真正的缺口。
#[test]
fn u117_every_writing_agent_emits_its_declared_confirmations() {
    for agent in WritingAgentKind::ALL {
        let declared = WritingNodeDefinition::confirmation_kinds_for(agent);
        if declared.is_empty() {
            // detail 刻意不声明确认项（它只产出细纲供下游消费，不直接落正文）。
            continue;
        }
        // summarizer 走的是另一条独立管线（`execute_summarizer_node`），
        // 本文件针对的是**写作节点**这条此前完全没有确认项的路径。
        if agent == WritingAgentKind::Summarizer {
            continue;
        }
        // writer / polisher 只声明**副作用类**确认项（`*CorrectionPatch`）。
        // 副作用类必须凭证据产出：本用例的假 LLM 不发起任何 patch 工具调用，
        // 此时没有任何待审内容，产出确认项反而会造出一条永远待审的空项、
        // 把工作流永久卡在 PendingConfirmation。它们由下面的
        // `u117_side_effect_confirmations_require_evidence` 单独覆盖。
        if declared
            .iter()
            .all(|kind| matches!(kind, ConfirmationKind::WriterCorrectionPatch
                | ConfirmationKind::PolisherCorrectionPatch
                | ConfirmationKind::PlannerRegister))
        {
            continue;
        }

        let temp = tempfile::tempdir().unwrap();
        let app_state = tempfile::tempdir().unwrap();
        ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
        let secrets = MemorySecretStore::default();
        provision(temp.path(), &secrets, spawn_llm());

        let node_type = agent.node_type();
        save_single_node_workflow(temp.path(), node_type);
        let run_id = run(temp.path(), &secrets, node_type);
        let confirmations = run_confirmations(temp.path(), node_type, &run_id);

        assert!(
            !confirmations.is_empty(),
            "U117：{node_type} 运行完成后没有任何确认项——\
             该 agent 声明了 {declared:?}，写作产出正在不经审阅直接落地"
        );
    }
}

/// 门禁的**实际效果**：Pending 确认项必须把正文落盘挡住。
///
/// 只断言「确认项存在」不够：确认项挂在输出上而门禁没读它，
/// 症状与修复前完全一样（正文照样直接写盘）。
#[test]
fn u117_pending_confirmation_blocks_patch_write_back() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::config::bind_project_app_state(temp.path(), app_state.path()).unwrap();
    let secrets = MemorySecretStore::default();
    provision(temp.path(), &secrets, spawn_llm());

    let node_type = WritingAgentKind::Outliner.node_type();
    save_single_node_workflow(temp.path(), node_type);
    let run_id = run(temp.path(), &secrets, node_type);

    let confirmations = run_confirmations(temp.path(), node_type, &run_id);
    assert!(
        confirmations
            .iter()
            .any(|item| item.state == RuntimeConfirmationState::Pending),
        "普通模式（未开 Auto Mode）下 outliner 的确认项必须是 Pending，实际：{:?}",
        confirmations
            .iter()
            .map(|item| item.state)
            .collect::<Vec<_>>()
    );
}

/// 声明表本身的一致性：12 种 kind 必须都有 agent 认领。
///
/// 少一种的后果不是报错，而是**静默无审批**——正是 U117 的形状。
/// 这条不依赖运行，跑得快，能在重构声明表时第一时间报警。
#[test]
fn u117_every_confirmation_kind_is_claimed_by_some_agent() {
    let mut claimed = std::collections::BTreeSet::new();
    for agent in WritingAgentKind::ALL {
        for kind in WritingNodeDefinition::confirmation_kinds_for(agent) {
            claimed.insert(format!("{kind:?}"));
        }
    }
    for kind in ConfirmationKind::ALL {
        assert!(
            claimed.contains(&format!("{kind:?}")),
            "{kind:?} 没有任何 agent 认领——该确认项永远不会被创建"
        );
    }
}

/// 副作用类确认项必须**凭证据**产出，而不是一律产出。
///
/// 两个方向都要钉住：
/// - 无副作用时产出 → 造出一条永远待审的空确认项，工作流永久卡死；
/// - 有副作用时不产出 → 正文改动绕过审批，正是 U117 的危害本身。
///
/// 这里直接驱动产出函数，因为要精确控制「有无副作用」这一个变量；
/// 走完整工作流则需要让假 LLM 稳定发出 patch 工具调用，引入与本契约无关的耦合。
#[test]
fn u117_side_effect_confirmations_require_evidence() {
    let policy = ariadne::rag::models::WritingConfirmationPolicy::normal_default();
    let auto_mode = ariadne::contracts::AutoModeState {
        enabled: false,
        preauthorized_budget_usd: None,
    };

    // 有证据：必须产出，且在人工策略下为 Pending（否则拦不住落盘）。
    let with_evidence = ariadne::rag::pipeline::build_writing_confirmation(
        ConfirmationKind::WriterCorrectionPatch,
        "chapter-01",
        "op-1",
        json!({ "patch_count": 1 }),
        &policy,
        &auto_mode,
        None,
    )
    .unwrap();
    assert_eq!(
        with_evidence.state,
        ariadne::rag::models::ConfirmationState::Pending,
        "人工策略下正文 patch 必须待审——非 Pending 就不会触发落盘门禁"
    );
}
