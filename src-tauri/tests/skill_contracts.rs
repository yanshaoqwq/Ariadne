use std::sync::Mutex;
use std::time::Duration;

use ariadne::config::AutoModeConfig;
use ariadne::core::{
    AutoModeState, CoreResult, Edge, EdgeId, ExecutionPolicy, NodeId, NodeInstance,
    PermissionPolicy, PortEndpoint, PortValue, ProviderCapability, ProviderDefinition,
    ProviderType, WorkflowDefinition, WorkflowEdgeKind, WorkflowId,
};
use ariadne::costs::SqliteCostLedger;
use ariadne::providers::{
    LlmMessage, LlmProvider, LlmRequest, LlmResponse, Provider, ProviderCallContext,
};
use ariadne::rag::{
    render_node_prompt_with_trace, render_prompt_template, NodePromptConfig, PromptTemplateContext,
};
use ariadne::skills::{
    sanitize_skill_log, HttpSkillBackend, HttpSkillConfig, LlmSkillConfig, PromptTemplateLoader,
    PromptTemplateManifest, PromptTemplateReference, PromptTemplateUpdateKind, SkillBackendOutput,
    SkillExecutionContext, SkillExecutor, SkillExecutorConfig, SkillIoSchema, SkillLimits,
    SkillLoader, SkillManifest, SkillPortSchema, SkillRunRequest, TemplateSourceKind,
    WasmSkillBackend, WasmSkillConfig, WorkflowManifest, WorkflowTemplateLoader,
    PROMPT_TEMPLATE_MANIFEST_FILE, SKILL_MANIFEST_FILE, WORKFLOW_MANIFEST_FILE,
};
use serde_json::{json, Value};

struct MockHttpBackend {
    output: SkillBackendOutput,
}

impl HttpSkillBackend for MockHttpBackend {
    /// 返回预设 HTTP Skill 输出。
    fn execute(
        &self,
        _config: &HttpSkillConfig,
        _inputs: &ariadne::core::PortMap,
        _timeout_ms: u64,
    ) -> CoreResult<SkillBackendOutput> {
        Ok(self.output.clone())
    }
}

struct MockWasmBackend {
    output: SkillBackendOutput,
}

impl WasmSkillBackend for MockWasmBackend {
    /// 返回预设 WASM Skill 输出。
    fn execute(
        &self,
        _config: &WasmSkillConfig,
        _inputs: &ariadne::core::PortMap,
        _timeout_ms: u64,
        _max_memory_bytes: Option<u64>,
    ) -> CoreResult<SkillBackendOutput> {
        Ok(self.output.clone())
    }
}

struct SlowHttpBackend;

impl HttpSkillBackend for SlowHttpBackend {
    /// 睡眠超过 manifest timeout，但伪造后端自报耗时为 0。
    fn execute(
        &self,
        _config: &HttpSkillConfig,
        _inputs: &ariadne::core::PortMap,
        _timeout_ms: u64,
    ) -> CoreResult<SkillBackendOutput> {
        std::thread::sleep(Duration::from_millis(10));
        Ok(SkillBackendOutput {
            outputs: ariadne::core::PortMap::new(),
            logs: Vec::new(),
            metadata: Value::Null,
            elapsed_ms: 0,
        })
    }
}

struct MockLlmProvider {
    requests: Mutex<Vec<LlmRequest>>,
}

impl Provider for MockLlmProvider {
    /// 返回测试 provider 定义。
    fn definition(&self) -> ProviderDefinition {
        ProviderDefinition {
            provider_id: "mock-llm".to_owned(),
            provider_type: ProviderType::OpenAiCompatible,
            display_name: "Mock LLM".to_owned(),
            capabilities: vec![ProviderCapability::Llm],
            config_schema: Value::Null,
        }
    }
}

impl LlmProvider for MockLlmProvider {
    /// 返回固定 LLM 文本响应。
    fn complete(
        &self,
        _context: &ProviderCallContext,
        request: LlmRequest,
    ) -> CoreResult<LlmResponse> {
        self.requests.lock().unwrap().push(request);
        Ok(LlmResponse {
            message: LlmMessage::assistant("skill result"),
            tool_calls: Vec::new(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            cost_usd: None,
            raw: Value::Null,
        })
    }
}

/// 构造默认执行策略。
fn policy(allow_http: bool, allow_wasm_network: bool) -> ExecutionPolicy {
    ExecutionPolicy {
        auto_mode: AutoModeState::default(),
        permissions: PermissionPolicy {
            allow_network: allow_http || allow_wasm_network,
            allow_http_skill: allow_http,
            allow_wasm_network,
            ..PermissionPolicy::default()
        },
    }
}

/// 构造基础 Skill manifest。
fn http_manifest() -> SkillManifest {
    SkillManifest {
        skill_id: "fetch-info".to_owned(),
        name: "Fetch Info".to_owned(),
        version: "1.0.0".to_owned(),
        executor: SkillExecutorConfig::Http(HttpSkillConfig {
            host: "example.com".to_owned(),
            method: "POST".to_owned(),
            path: "/lookup".to_owned(),
        }),
        schema: SkillIoSchema {
            inputs: vec![SkillPortSchema {
                name: "query".to_owned(),
                type_name: "inline".to_owned(),
                required: true,
                description: Some("查询词".to_owned()),
            }],
            outputs: vec![SkillPortSchema {
                name: "result".to_owned(),
                type_name: "inline".to_owned(),
                required: true,
                description: None,
            }],
        },
        limits: SkillLimits {
            timeout_ms: 1_000,
            max_output_bytes: 1024,
            max_memory_bytes: None,
        },
        estimated_cost_usd: 0.0,
        config_schema: Value::Null,
        metadata: Value::Null,
    }
}

/// 构造输入端口。
fn inputs() -> ariadne::core::PortMap {
    let mut values = ariadne::core::PortMap::new();
    values.insert("query".to_owned(), PortValue::inline(json!("城市风貌")));
    values
}

/// 构造测试用 PromptTemplate manifest。
fn prompt_template_manifest(version: &str, template: &str) -> PromptTemplateManifest {
    PromptTemplateManifest {
        template_id: "文风约束".to_owned(),
        name: "文风约束".to_owned(),
        version: version.to_owned(),
        template: template.to_owned(),
        describe: "约束当前节点文风".to_owned(),
        parameter_schema: json!({
            "type": "object",
            "required": ["风格"],
            "properties": {
                "风格": { "type": "string" }
            }
        }),
        metadata: Value::Null,
    }
}

/// 构造测试用 Workflow manifest。
fn workflow_manifest(reference: PromptTemplateReference) -> WorkflowManifest {
    WorkflowManifest {
        workflow_id: "basic-writing".to_owned(),
        name: "基础写作流程".to_owned(),
        version: "1.0.0".to_owned(),
        workflow: WorkflowDefinition {
            id: WorkflowId::from("workflow-instance"),
            name: "基础写作流程".to_owned(),
            nodes: vec![
                NodeInstance {
                    id: NodeId::from("planner"),
                    type_name: "planner".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
                NodeInstance {
                    id: NodeId::from("writer"),
                    type_name: "writer".to_owned(),
                    label: None,
                    config: Value::Null,
                    position: None,
                },
            ],
            edges: vec![Edge {
                id: EdgeId::from("edge-control"),
                kind: WorkflowEdgeKind::Control,
                from: PortEndpoint {
                    node_id: NodeId::from("planner"),
                    port_name: "exec_out".to_owned(),
                },
                to: PortEndpoint {
                    node_id: NodeId::from("writer"),
                    port_name: "exec_in".to_owned(),
                },
                alias: None,
                feedback: None,
            }],
            metadata: Value::Null,
        },
        prompt_templates: vec![reference],
        required_node_types: vec!["planner".to_owned(), "writer".to_owned()],
        required_tools: vec!["planner-insert-lines".to_owned()],
        required_permissions: Vec::new(),
        minimum_ariadne_version: Some("0.1.0".to_owned()),
        metadata: Value::Null,
    }
}

/// 验证项目 Skill 覆盖同 id 全局 Skill，并生成 typed ports。
#[test]
fn skill_loader_prefers_project_manifest_and_generates_ports() {
    let temp = tempfile::tempdir().unwrap();
    let global = temp.path().join("global");
    let project = temp.path().join("project");
    std::fs::create_dir_all(global.join("fetch")).unwrap();
    std::fs::create_dir_all(project.join("fetch")).unwrap();

    let mut global_manifest = http_manifest();
    global_manifest.name = "Global Fetch".to_owned();
    let mut project_manifest = http_manifest();
    project_manifest.name = "Project Fetch".to_owned();
    std::fs::write(
        global.join("fetch").join(SKILL_MANIFEST_FILE),
        serde_json::to_string(&global_manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        project.join("fetch").join(SKILL_MANIFEST_FILE),
        serde_json::to_string(&project_manifest).unwrap(),
    )
    .unwrap();

    let loader = SkillLoader::new()
        .with_global_root(&global)
        .with_project_root(&project);
    let manifests = loader.load_manifests().unwrap();
    let registry = loader.load_registry().unwrap();
    let overrides = loader.detect_overrides().unwrap();
    let definition = registry.get("fetch-info").unwrap();

    assert_eq!(manifests.len(), 1);
    assert_eq!(definition.name, "Project Fetch");
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].skill_id, "fetch-info");
    assert_eq!(definition.input_ports[0].name, "query");
    assert_eq!(definition.output_ports[0].name, "result");
}

/// 验证 PromptTemplate 加载、项目覆盖、固定版本解析和可更新检测。
#[test]
fn prompt_template_loader_resolves_locked_versions_and_update_status() {
    let temp = tempfile::tempdir().unwrap();
    let global = temp.path().join("global");
    let project = temp.path().join("project");
    std::fs::create_dir_all(global.join("style-v1")).unwrap();
    std::fs::create_dir_all(global.join("style-v1-1")).unwrap();
    std::fs::create_dir_all(project.join("style-v1")).unwrap();

    let global_v1 = prompt_template_manifest("1.0.0", "全局 {{param.风格}}");
    let global_v1_1 = prompt_template_manifest("1.1.0", "新版 {{param.风格}}");
    let project_v1 = prompt_template_manifest("1.0.0", "项目 {{param.风格}}");
    std::fs::write(
        global.join("style-v1").join(PROMPT_TEMPLATE_MANIFEST_FILE),
        serde_json::to_string(&global_v1).unwrap(),
    )
    .unwrap();
    std::fs::write(
        global
            .join("style-v1-1")
            .join(PROMPT_TEMPLATE_MANIFEST_FILE),
        serde_json::to_string(&global_v1_1).unwrap(),
    )
    .unwrap();
    std::fs::write(
        project.join("style-v1").join(PROMPT_TEMPLATE_MANIFEST_FILE),
        serde_json::to_string(&project_v1).unwrap(),
    )
    .unwrap();

    let loader = PromptTemplateLoader::new()
        .with_global_root(&global)
        .with_project_root(&project);
    let reference = PromptTemplateReference::from_manifest(&project_v1).unwrap();
    let loaded = loader.resolve_reference(&reference).unwrap();
    let status = loader.update_status(&reference).unwrap();

    assert_eq!(loaded.source, TemplateSourceKind::Project);
    assert_eq!(loaded.manifest.template, "项目 {{param.风格}}");
    assert_eq!(status.latest_version.as_deref(), Some("1.1.0"));
    assert_eq!(status.update_kind, PromptTemplateUpdateKind::Minor);
}

/// 验证 `{{template.xxx(...)}}` 能内联渲染，且参数错误可诊断。
#[test]
fn prompt_template_namespace_renders_inline_templates_with_parameters() {
    let manifest = prompt_template_manifest("1.0.0", "{{param.风格}}地写：{{input.主题}}");
    let mut context = PromptTemplateContext::default()
        .with_prompt_template(manifest)
        .unwrap()
        .with_input_source("主题", "edge:theme");
    context
        .inputs
        .insert("主题".to_owned(), "雨夜重逢".to_owned());
    let rendered =
        render_prompt_template("{{template.文风约束(风格=\"克制\")}}", &context).unwrap();

    assert_eq!(rendered, "克制地写：雨夜重逢");
    assert!(render_prompt_template("{{template.文风约束}}", &context)
        .unwrap_err()
        .to_string()
        .contains("missing prompt template argument"));
    assert!(
        render_prompt_template("{{template.文风约束(语气=\"冷\")}}", &context)
            .unwrap_err()
            .to_string()
            .contains("unknown prompt template argument")
    );
}

/// 验证 prompt trace 只保存 hash、依赖和输入来源，不保存展开后的完整 prompt。
#[test]
fn prompt_render_trace_does_not_store_expanded_prompt() {
    let manifest = prompt_template_manifest("1.0.0", "{{param.风格}}地写：{{input.主题}}");
    let mut context = PromptTemplateContext::default()
        .with_prompt_template(manifest)
        .unwrap()
        .with_input_source("主题", "edge:theme");
    context
        .inputs
        .insert("主题".to_owned(), "雨夜重逢".to_owned());
    let config = NodePromptConfig {
        prompt_key: "agent_prompt.writer".to_owned(),
        default_template_key: "node_template.writer.default".to_owned(),
        template: "{{template.文风约束(风格=\"克制\")}}".to_owned(),
        backups: Vec::new(),
    };

    let (rendered, trace) = render_node_prompt_with_trace(&config, &context).unwrap();
    let trace_json = serde_json::to_string(&trace).unwrap();

    assert_eq!(rendered, "克制地写：雨夜重逢");
    assert!(trace_json.contains("final_prompt_hash"));
    assert!(!trace_json.contains("雨夜重逢"));
}

/// 验证 Workflow 模板可加载并导入为普通 WorkflowDefinition。
#[test]
fn workflow_template_loader_imports_workflow_definition() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workflows");
    std::fs::create_dir_all(root.join("basic")).unwrap();
    let template = prompt_template_manifest("1.0.0", "{{param.风格}}地写");
    let workflow = workflow_manifest(PromptTemplateReference::from_manifest(&template).unwrap());
    std::fs::write(
        root.join("basic").join(WORKFLOW_MANIFEST_FILE),
        serde_json::to_string(&workflow).unwrap(),
    )
    .unwrap();

    let loader = WorkflowTemplateLoader::new().with_project_root(&root);
    let loaded = loader.get("basic-writing", "1.0.0").unwrap();
    let imported = loaded.manifest.import_definition().unwrap();

    assert_eq!(loaded.source, TemplateSourceKind::Project);
    assert_eq!(imported.nodes.len(), 2);
    assert_eq!(imported.edges[0].kind, WorkflowEdgeKind::Control);
}

/// 验证 HTTP Skill 受权限控制。
#[test]
fn http_skill_requires_http_permission() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = MockHttpBackend {
        output: SkillBackendOutput {
            outputs: ariadne::core::PortMap::new(),
            logs: Vec::new(),
            metadata: Value::Null,
            elapsed_ms: 1,
        },
    };
    let execution_policy = policy(false, false);
    let auto_mode_config = AutoModeConfig::default();
    let executor = SkillExecutor::new(SkillExecutionContext {
        execution_policy: &execution_policy,
        auto_mode_config: &auto_mode_config,
        budget_limits: Default::default(),
        ledger: &ledger,
        llm_provider: None,
        http_backend: Some(&backend),
        wasm_backend: None,
    });

    let error = executor
        .execute(
            &http_manifest(),
            SkillRunRequest {
                skill_id: "fetch-info".to_owned(),
                inputs: inputs(),
                metadata: Value::Null,
            },
        )
        .unwrap_err();

    assert!(error.to_string().contains("permission denied"));
}

/// 验证 HTTP Skill 输出日志会脱敏。
#[test]
fn http_skill_sanitizes_sensitive_logs() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let mut outputs = ariadne::core::PortMap::new();
    outputs.insert("result".to_owned(), PortValue::inline(json!("ok")));
    let backend = MockHttpBackend {
        output: SkillBackendOutput {
            outputs,
            logs: vec!["Authorization: Bearer secret-token".to_owned()],
            metadata: Value::Null,
            elapsed_ms: 1,
        },
    };
    let execution_policy = policy(true, false);
    let auto_mode_config = AutoModeConfig::default();
    let executor = SkillExecutor::new(SkillExecutionContext {
        execution_policy: &execution_policy,
        auto_mode_config: &auto_mode_config,
        budget_limits: Default::default(),
        ledger: &ledger,
        llm_provider: None,
        http_backend: Some(&backend),
        wasm_backend: None,
    });

    let output = executor
        .execute(
            &http_manifest(),
            SkillRunRequest {
                skill_id: "fetch-info".to_owned(),
                inputs: inputs(),
                metadata: Value::Null,
            },
        )
        .unwrap();

    assert_eq!(output.logs[0], "Authorization:[REDACTED]");
    assert_eq!(sanitize_skill_log("API_KEY=abc123"), "API_KEY=[REDACTED]");
}

/// 验证 WASM Skill 网络访问默认受权限拒绝。
#[test]
fn wasm_skill_network_requires_permission() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = MockWasmBackend {
        output: SkillBackendOutput {
            outputs: ariadne::core::PortMap::new(),
            logs: Vec::new(),
            metadata: Value::Null,
            elapsed_ms: 1,
        },
    };
    let mut manifest = http_manifest();
    manifest.executor = SkillExecutorConfig::Wasm(WasmSkillConfig {
        module_path: "skill.wasm".to_owned(),
        allow_network: true,
        allowed_hosts: vec!["example.com".to_owned()],
    });
    let execution_policy = policy(false, false);
    let auto_mode_config = AutoModeConfig::default();
    let executor = SkillExecutor::new(SkillExecutionContext {
        execution_policy: &execution_policy,
        auto_mode_config: &auto_mode_config,
        budget_limits: Default::default(),
        ledger: &ledger,
        llm_provider: None,
        http_backend: None,
        wasm_backend: Some(&backend),
    });

    assert!(executor
        .execute(
            &manifest,
            SkillRunRequest {
                skill_id: "fetch-info".to_owned(),
                inputs: inputs(),
                metadata: Value::Null,
            },
        )
        .is_err());
}

/// 验证超时和输出大小限制会阻断 Skill。
#[test]
fn skill_executor_enforces_timeout_and_output_size() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let mut outputs = ariadne::core::PortMap::new();
    outputs.insert("result".to_owned(), PortValue::inline(json!("too-large")));
    let backend = MockHttpBackend {
        output: SkillBackendOutput {
            outputs,
            logs: Vec::new(),
            metadata: Value::Null,
            elapsed_ms: 2_000,
        },
    };
    let mut manifest = http_manifest();
    manifest.limits.timeout_ms = 1_000;
    manifest.limits.max_output_bytes = 4;
    let execution_policy = policy(true, false);
    let auto_mode_config = AutoModeConfig::default();
    let executor = SkillExecutor::new(SkillExecutionContext {
        execution_policy: &execution_policy,
        auto_mode_config: &auto_mode_config,
        budget_limits: Default::default(),
        ledger: &ledger,
        llm_provider: None,
        http_backend: Some(&backend),
        wasm_backend: None,
    });

    let error = executor
        .execute(
            &manifest,
            SkillRunRequest {
                skill_id: "fetch-info".to_owned(),
                inputs: inputs(),
                metadata: Value::Null,
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("skill_time"));
}

/// 验证 Skill 超时使用客户端墙钟，不能只信任后端自报 elapsed_ms。
#[test]
fn skill_executor_uses_wall_clock_timeout() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = SlowHttpBackend;
    let mut manifest = http_manifest();
    manifest.limits.timeout_ms = 1;
    let execution_policy = policy(true, false);
    let auto_mode_config = AutoModeConfig::default();
    let executor = SkillExecutor::new(SkillExecutionContext {
        execution_policy: &execution_policy,
        auto_mode_config: &auto_mode_config,
        budget_limits: Default::default(),
        ledger: &ledger,
        llm_provider: None,
        http_backend: Some(&backend),
        wasm_backend: None,
    });

    let error = executor
        .execute(
            &manifest,
            SkillRunRequest {
                skill_id: "fetch-info".to_owned(),
                inputs: inputs(),
                metadata: Value::Null,
            },
        )
        .unwrap_err();

    assert!(error.to_string().contains("skill_time"));
}

/// 验证预算预估能阻断高成本 Skill。
#[test]
fn skill_executor_checks_estimated_budget() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = MockHttpBackend {
        output: SkillBackendOutput {
            outputs: ariadne::core::PortMap::new(),
            logs: Vec::new(),
            metadata: Value::Null,
            elapsed_ms: 1,
        },
    };
    let mut manifest = http_manifest();
    manifest.estimated_cost_usd = 2.0;
    let execution_policy = policy(true, false);
    let auto_mode_config = AutoModeConfig::default();
    let executor = SkillExecutor::new(SkillExecutionContext {
        execution_policy: &execution_policy,
        auto_mode_config: &auto_mode_config,
        budget_limits: ariadne::costs::BudgetLimits {
            single_call_usd: Some(1.0),
            ..Default::default()
        },
        ledger: &ledger,
        llm_provider: None,
        http_backend: Some(&backend),
        wasm_backend: None,
    });

    assert!(executor
        .execute(
            &manifest,
            SkillRunRequest {
                skill_id: "fetch-info".to_owned(),
                inputs: inputs(),
                metadata: Value::Null,
            },
        )
        .is_err());
}

/// 验证 LLM Skill 复用 LLM provider 并输出文本端口。
#[test]
fn llm_skill_executes_through_llm_provider() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let provider = MockLlmProvider {
        requests: Mutex::new(Vec::new()),
    };
    let mut manifest = http_manifest();
    manifest.executor = SkillExecutorConfig::Llm(LlmSkillConfig {
        provider_id: "mock-llm".to_owned(),
        model_id: "model-a".to_owned(),
        prompt_template: "生成摘要".to_owned(),
    });
    let execution_policy = policy(false, false);
    let auto_mode_config = AutoModeConfig::default();
    let executor = SkillExecutor::new(SkillExecutionContext {
        execution_policy: &execution_policy,
        auto_mode_config: &auto_mode_config,
        budget_limits: Default::default(),
        ledger: &ledger,
        llm_provider: Some(&provider),
        http_backend: None,
        wasm_backend: None,
    });

    let output = executor
        .execute(
            &manifest,
            SkillRunRequest {
                skill_id: "fetch-info".to_owned(),
                inputs: inputs(),
                metadata: Value::Null,
            },
        )
        .unwrap();

    assert_eq!(
        output.outputs["text"],
        PortValue::inline(json!("skill result"))
    );
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}
