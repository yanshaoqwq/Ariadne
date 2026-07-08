use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::Duration;

use ariadne::config::AutoModeConfig;
use ariadne::contracts::{
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
    sanitize_skill_log, HttpSkillBackend, HttpSkillConfig, LlmSkillConfig, NativeHttpSkillBackend,
    NativeWasmSkillBackend, PromptTemplateLoader, PromptTemplateManifest, PromptTemplateReference,
    PromptTemplateUpdateKind, SkillBackendOutput, SkillExecutionContext, SkillExecutor,
    SkillExecutorConfig, SkillIoSchema, SkillLimits, SkillLoader, SkillManifest, SkillPortSchema,
    SkillRunRequest, TemplateSourceKind, WasmSkillBackend, WasmSkillConfig, WorkflowManifest,
    WorkflowTemplateLoader, PROMPT_TEMPLATE_MANIFEST_FILE, SKILL_MANIFEST_FILE,
    WORKFLOW_MANIFEST_FILE,
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
        _inputs: &ariadne::contracts::PortMap,
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
        _inputs: &ariadne::contracts::PortMap,
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
        _inputs: &ariadne::contracts::PortMap,
        _timeout_ms: u64,
    ) -> CoreResult<SkillBackendOutput> {
        std::thread::sleep(Duration::from_millis(10));
        Ok(SkillBackendOutput {
            outputs: ariadne::contracts::PortMap::new(),
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
fn inputs() -> ariadne::contracts::PortMap {
    let mut values = ariadne::contracts::PortMap::new();
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
                communication: None,
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
            outputs: ariadne::contracts::PortMap::new(),
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
    let mut outputs = ariadne::contracts::PortMap::new();
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

    assert_eq!(output.logs[0], "Authorization: [REDACTED]");
    assert_eq!(sanitize_skill_log("API_KEY=abc123"), "API_KEY=[REDACTED]");
    assert_eq!(
        sanitize_skill_log(r#"payload {"api_key":"abc123","ok":true}"#),
        r#"payload {"api_key":"[REDACTED]","ok":true}"#
    );
    assert_eq!(
        sanitize_skill_log("url /v1?token=abc123&safe=true"),
        "url /v1?token=[REDACTED]&safe=true"
    );
    assert_eq!(
        sanitize_skill_log("bearer secret-token request_id=42"),
        "bearer [REDACTED] request_id=42"
    );
}

/// 验证标准库真实 HTTP 后端能发送 JSON 输入并解析 SkillBackendOutput。
#[test]
fn native_http_backend_executes_against_local_http_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&request).unwrap();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim())
                    })
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let header_end = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .unwrap()
                    + 4;
                while request.len() < header_end + content_length {
                    let read = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..read]);
                }
                break;
            }
        }
        let body = serde_json::to_vec(&SkillBackendOutput {
            outputs: {
                let mut outputs = ariadne::contracts::PortMap::new();
                outputs.insert("result".to_owned(), PortValue::inline(json!("native-ok")));
                outputs
            },
            logs: vec!["http backend completed".to_owned()],
            metadata: json!({ "server": "local" }),
            elapsed_ms: 1,
        })
        .unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        stream.flush().unwrap();
        String::from_utf8(request).unwrap()
    });

    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = NativeHttpSkillBackend;
    let mut manifest = http_manifest();
    manifest.executor = SkillExecutorConfig::Http(HttpSkillConfig {
        host: format!("http://127.0.0.1:{}", addr.port()),
        method: "POST".to_owned(),
        path: "/run".to_owned(),
    });
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

    std::env::set_var("ARIADNE_ALLOW_LOCAL_HTTP_SKILL", "1");
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
    std::env::remove_var("ARIADNE_ALLOW_LOCAL_HTTP_SKILL");

    let request = server.join().unwrap();
    assert!(request.starts_with("POST /run HTTP/1.1"));
    assert!(request.contains("\"query\""));
    assert_eq!(
        output.outputs["result"],
        PortValue::inline(json!("native-ok"))
    );
    assert_eq!(output.logs, vec!["http backend completed".to_owned()]);
}

/// 验证真实 HTTP 后端对未声明 Content-Length 的超大响应也会流式截断。
#[test]
fn native_http_backend_rejects_oversized_streaming_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        stream.write_all(&vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
        stream.flush().unwrap();
    });

    let backend = NativeHttpSkillBackend;
    let config = HttpSkillConfig {
        host: format!("http://127.0.0.1:{}", addr.port()),
        method: "GET".to_owned(),
        path: "/large".to_owned(),
    };

    std::env::set_var("ARIADNE_ALLOW_LOCAL_HTTP_SKILL", "1");
    let result = backend.execute(&config, &inputs(), 1_000);
    std::env::remove_var("ARIADNE_ALLOW_LOCAL_HTTP_SKILL");
    server.join().unwrap();

    let error = result.unwrap_err().to_string();
    assert!(error.contains("http_skill_response"));
    assert!(error.contains("response exceeds"));
}

/// 默认拒绝 HTTP Skill 访问本机/内网地址，避免 SSRF。
#[test]
fn native_http_backend_rejects_local_addresses_by_default() {
    let backend = NativeHttpSkillBackend;
    let config = HttpSkillConfig {
        host: "http://127.0.0.1:12345".to_owned(),
        method: "POST".to_owned(),
        path: "/run".to_owned(),
    };

    assert!(backend.execute(&config, &inputs(), 1_000).is_err());
}

/// 验证真实 WASM 后端按固定 ABI 执行并返回 SkillBackendOutput。
#[test]
fn native_wasm_backend_executes_exported_run_abi() {
    let temp = tempfile::tempdir().unwrap();
    let wasm_path = temp.path().join("echo.wasm");
    let wat = r#"
    (module
      (memory (export "memory") 1)
      (func (export "run") (result i32)
        (local $i i32)
        (local $len i32)
        (local $base i32)
        (local.set $base (i32.const 8))
        (local.set $len (i32.const 94))
        (i32.store (i32.const 4) (local.get $len))
        (loop $copy
          (i32.store8
            (i32.add (local.get $base) (local.get $i))
            (i32.load8_u (i32.add (i32.const 256) (local.get $i))))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br_if $copy (i32.lt_u (local.get $i) (local.get $len))))
        (i32.const 0))
      (data (i32.const 256) "{\"outputs\":{\"result\":{\"kind\":\"inline\",\"value\":\"wasm-ok\"}},\"logs\":[\"wasm done\"],\"elapsed_ms\":1}")
    )
    "#;
    std::fs::write(&wasm_path, wat::parse_str(wat).unwrap()).unwrap();

    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = NativeWasmSkillBackend;
    let mut manifest = http_manifest();
    manifest.executor = SkillExecutorConfig::Wasm(WasmSkillConfig {
        module_path: wasm_path.to_string_lossy().into_owned(),
        allow_network: false,
        allowed_hosts: Vec::new(),
    });
    manifest.limits.max_memory_bytes = Some(64 * 1024);
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
        output.outputs["result"],
        PortValue::inline(json!("wasm-ok"))
    );
    assert_eq!(output.logs, vec!["wasm done".to_owned()]);
}

/// 验证真实 WASM 后端会中断无限循环，而不是等 run() 返回后再检查墙钟。
#[test]
fn native_wasm_backend_interrupts_infinite_loop_with_fuel() {
    let temp = tempfile::tempdir().unwrap();
    let wasm_path = temp.path().join("spin.wasm");
    let wat = r#"
    (module
      (memory (export "memory") 1)
      (func (export "run") (result i32)
        (loop $spin
          br $spin)
        (i32.const 0))
    )
    "#;
    std::fs::write(&wasm_path, wat::parse_str(wat).unwrap()).unwrap();

    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = NativeWasmSkillBackend;
    let mut manifest = http_manifest();
    manifest.executor = SkillExecutorConfig::Wasm(WasmSkillConfig {
        module_path: wasm_path.to_string_lossy().into_owned(),
        allow_network: false,
        allowed_hosts: Vec::new(),
    });
    manifest.limits.timeout_ms = 1;
    manifest.limits.max_memory_bytes = Some(64 * 1024);
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

    assert!(error.to_string().contains("wasm_time"));
    assert!(error.to_string().contains("fuel exhausted"));
}

/// 验证 WASM 在 memory.grow 时受 Store limiter 约束，而不是运行结束后才检查。
#[test]
fn native_wasm_backend_blocks_memory_growth_beyond_limit() {
    let temp = tempfile::tempdir().unwrap();
    let wasm_path = temp.path().join("grow.wasm");
    let wat = r#"
    (module
      (memory (export "memory") 1)
      (func (export "run") (result i32)
        (drop (memory.grow (i32.const 1)))
        (i32.const 0))
    )
    "#;
    std::fs::write(&wasm_path, wat::parse_str(wat).unwrap()).unwrap();

    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = NativeWasmSkillBackend;
    let mut manifest = http_manifest();
    manifest.executor = SkillExecutorConfig::Wasm(WasmSkillConfig {
        module_path: wasm_path.to_string_lossy().into_owned(),
        allow_network: false,
        allowed_hosts: Vec::new(),
    });
    manifest.limits.max_memory_bytes = Some(64 * 1024);
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

    let message = error.to_string();
    assert!(
        message.contains("wasm run failed") || message.contains("wasm_memory"),
        "{message}"
    );
}

/// 验证 WASM Skill 网络访问默认受权限拒绝。
#[test]
fn wasm_skill_network_requires_permission() {
    let ledger = SqliteCostLedger::open_in_memory().unwrap();
    let backend = MockWasmBackend {
        output: SkillBackendOutput {
            outputs: ariadne::contracts::PortMap::new(),
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
    let mut outputs = ariadne::contracts::PortMap::new();
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
            outputs: ariadne::contracts::PortMap::new(),
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
