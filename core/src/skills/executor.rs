use std::io::{Read, Write};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::AutoModeConfig;
use crate::contracts::{
    CancellationToken, CoreError, CoreResult, ExecutionPolicy, ExternalDispatchAuthorization,
    ExternalDispatchOutcome, PermissionRequest, PortValue, RunControl,
};
use crate::costs::{evaluate_budget, BudgetLimits, BudgetUsage, CostLedger, CostQuery};
use crate::llm::{LlmRunRequest, LlmService, LlmServiceConfig, ToolExecutorRouter};
use crate::providers::{
    ContentPart, LlmMessage, LlmProvider, ProviderCallContext, SearchProvider, ToolDefinition,
    WebSearchToolExecutor,
};
use crate::retrieval::{ProjectRetrievalRuntime, ProjectSearchToolExecutor};
use crate::skills::models::{
    HttpSkillConfig, SkillBackendOutput, SkillExecutorConfig, SkillManifest, SkillRunOutput,
    WasmSkillConfig,
};
use crate::skills::sanitizer::sanitize_skill_logs;

const MAX_HTTP_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WASM_WORKER_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const WASM_WORKER_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// HTTP Skill 后端接口，真实网络实现后续可替换接入。
pub trait HttpSkillBackend {
    /// 执行一次 HTTP Skill。
    fn execute(
        &self,
        config: &HttpSkillConfig,
        inputs: &crate::contracts::PortMap,
        idempotency_key: Option<&str>,
        timeout_ms: u64,
        cancellation: &CancellationToken,
    ) -> CoreResult<SkillBackendOutput>;
}

/// WASM Skill 后端接口，真实 WASM 运行时后续可替换接入。
pub trait WasmSkillBackend {
    /// 执行一次 WASM Skill。
    fn execute(
        &self,
        config: &WasmSkillConfig,
        inputs: &crate::contracts::PortMap,
        timeout_ms: u64,
        max_memory_bytes: Option<u64>,
        cancellation: &CancellationToken,
    ) -> CoreResult<SkillBackendOutput>;
}

/// wasmi WASM ExecutorAdapter 后端。
///
/// ABI: 模块导出 `memory` 和 `run() -> i32`。Host 将输入 `SkillRunRequest` JSON
/// 写入 memory[8..]，memory[0..4] 写入输入长度；模块返回 0 时，需要把输出 JSON
/// 写在 memory[8..]，memory[4..8] 写入输出长度。输出 JSON 反序列化为
/// `SkillBackendOutput`，或作为普通 `result` 端口返回。
#[derive(Debug, Clone, Default)]
pub struct NativeWasmSkillBackend {
    worker_executable: Option<PathBuf>,
}

impl NativeWasmSkillBackend {
    /// 覆盖隔离执行器路径，供嵌入式宿主和合同测试显式绑定同版本二进制。
    pub fn with_worker_executable(mut self, executable: impl Into<PathBuf>) -> Self {
        self.worker_executable = Some(executable.into());
        self
    }
}

impl WasmSkillBackend for NativeWasmSkillBackend {
    /// 通过 wasmi 执行本地 WASM ExecutorAdapter。
    fn execute(
        &self,
        config: &WasmSkillConfig,
        inputs: &crate::contracts::PortMap,
        timeout_ms: u64,
        max_memory_bytes: Option<u64>,
        cancellation: &CancellationToken,
    ) -> CoreResult<SkillBackendOutput> {
        execute_native_wasm_isolated(
            config,
            inputs,
            timeout_ms,
            max_memory_bytes,
            cancellation,
            self.worker_executable.as_deref(),
        )
    }
}

/// reqwest HTTP/HTTPS ExecutorAdapter 后端。
///
/// 权限、预算、超时和输出大小仍由 `SkillExecutor` 统一处理；TLS 使用 rustls。
#[derive(Debug, Clone, Default)]
pub struct NativeHttpSkillBackend;

impl HttpSkillBackend for NativeHttpSkillBackend {
    /// 通过 HTTP JSON 请求执行 ExecutorAdapter。
    fn execute(
        &self,
        config: &HttpSkillConfig,
        inputs: &crate::contracts::PortMap,
        idempotency_key: Option<&str>,
        timeout_ms: u64,
        cancellation: &CancellationToken,
    ) -> CoreResult<SkillBackendOutput> {
        execute_native_http(config, inputs, idempotency_key, timeout_ms, cancellation)
    }
}

/// Skill 执行上下文。
pub struct SkillExecutionContext<'a, L: CostLedger> {
    pub execution_policy: &'a ExecutionPolicy,
    pub auto_mode_config: &'a AutoModeConfig,
    pub budget_limits: BudgetLimits,
    pub ledger: &'a L,
    pub llm_provider: Option<&'a dyn LlmProvider>,
    pub http_backend: Option<&'a dyn HttpSkillBackend>,
    pub wasm_backend: Option<&'a dyn WasmSkillBackend>,
}

/// Skill 执行器，统一处理权限、预算、超时、输出大小和日志脱敏。
pub struct SkillExecutor<'a, L: CostLedger> {
    context: SkillExecutionContext<'a, L>,
    project_search: Option<SkillProjectSearch<'a>>,
    web_search: Option<SkillWebSearch<'a>>,
}

struct SkillProjectSearch<'a> {
    runtime: &'a ProjectRetrievalRuntime,
    tool: ToolDefinition,
    max_tool_rounds: u32,
}

struct SkillWebSearch<'a> {
    provider: &'a dyn SearchProvider,
    tool: ToolDefinition,
    max_tool_rounds: u32,
}

impl<'a, L: CostLedger> SkillExecutor<'a, L> {
    /// 创建 Skill 执行器。
    pub fn new(context: SkillExecutionContext<'a, L>) -> Self {
        Self {
            context,
            project_search: None,
            web_search: None,
        }
    }

    pub fn with_project_search(
        mut self,
        runtime: &'a ProjectRetrievalRuntime,
        tool: ToolDefinition,
        max_tool_rounds: u32,
    ) -> Self {
        self.project_search = Some(SkillProjectSearch {
            runtime,
            tool,
            max_tool_rounds,
        });
        self
    }

    pub fn with_web_search(
        mut self,
        provider: &'a dyn SearchProvider,
        tool: ToolDefinition,
        max_tool_rounds: u32,
    ) -> Self {
        self.web_search = Some(SkillWebSearch {
            provider,
            tool,
            max_tool_rounds,
        });
        self
    }

    /// 执行 Skill manifest。
    pub fn execute(
        &self,
        manifest: &SkillManifest,
        request: crate::skills::models::SkillRunRequest,
    ) -> CoreResult<SkillRunOutput> {
        self.execute_with_cancellation(manifest, request, &CancellationToken::new())
    }

    /// 使用调用方提供的共享 token 执行 Skill。
    pub fn execute_with_cancellation(
        &self,
        manifest: &SkillManifest,
        request: crate::skills::models::SkillRunRequest,
        cancellation: &CancellationToken,
    ) -> CoreResult<SkillRunOutput> {
        self.execute_with_control(
            manifest,
            request,
            cancellation,
            &ExternalDispatchAuthorization::default(),
        )
    }

    /// 使用共享取消信号与 workflow 持久化派发栅栏执行 Skill。
    pub fn execute_with_control(
        &self,
        manifest: &SkillManifest,
        request: crate::skills::models::SkillRunRequest,
        cancellation: &CancellationToken,
        dispatch_authorization: &ExternalDispatchAuthorization,
    ) -> CoreResult<SkillRunOutput> {
        cancellation.check()?;
        if manifest.skill_id != request.skill_id {
            return Err(CoreError::validation(
                "skill run request skill_id does not match manifest",
            ));
        }
        manifest.validate()?;
        crate::contracts::validate_required_ports(
            &manifest.schema.input_ports()?,
            &request.inputs,
        )?;
        self.check_budget(manifest.estimated_cost_usd)?;

        let started_at = Instant::now();
        let output =
            match &manifest.executor {
                SkillExecutorConfig::Llm(config) => {
                    let provider = self.context.llm_provider.ok_or_else(|| {
                        CoreError::validation("llm skill requires an LLM provider")
                    })?;
                    let service =
                        LlmService::new(self.context.ledger, self.context.auto_mode_config.clone());
                    let llm_request = LlmRunRequest {
                        config: LlmServiceConfig {
                            provider_id: config.provider_id.clone(),
                            model_id: config.model_id.clone(),
                            max_tool_rounds: self
                                .project_search
                                .as_ref()
                                .map(|search| search.max_tool_rounds)
                                .unwrap_or(0),
                            timeout_ms: manifest.limits.timeout_ms,
                            max_total_tokens: None,
                            budget_limits: self.context.budget_limits.clone(),
                            input_cost_per_million_tokens: None,
                            output_cost_per_million_tokens: None,
                            max_output_tokens: None,
                            max_context_tokens: None,
                        },
                        messages: vec![LlmMessage::user(render_prompt(
                            &config.prompt_template,
                            &request.inputs,
                        ))],
                        tools: self
                            .project_search
                            .iter()
                            .map(|search| search.tool.clone())
                            .chain(self.web_search.iter().map(|search| search.tool.clone()))
                            .collect(),
                        workflow_id: None,
                        run_id: None,
                        node_id: None,
                        metadata: request.metadata.clone(),
                        dispatch_authorization: dispatch_authorization.clone(),
                    };
                    let report = if self.project_search.is_some() || self.web_search.is_some() {
                        let max_tool_rounds = self
                            .project_search
                            .iter()
                            .map(|search| search.max_tool_rounds)
                            .chain(self.web_search.iter().map(|search| search.max_tool_rounds))
                            .max()
                            .unwrap_or(0);
                        let mut llm_request = llm_request;
                        llm_request.config.max_tool_rounds = max_tool_rounds;
                        let project_tool_executor = self.project_search.as_ref().map(|search| {
                            let mut search_context = ProviderCallContext::new("project_retrieval");
                            search_context.operation_id = request.operation_id.clone();
                            search_context.timeout_ms = manifest.limits.timeout_ms;
                            search_context.cancellation = cancellation.clone();
                            search_context.dispatch_authorization = dispatch_authorization.clone();
                            ProjectSearchToolExecutor::new(
                                search.runtime,
                                search_context,
                                [search.tool.name.clone()],
                            )
                        });
                        let web_tool_executor = self.web_search.as_ref().map(|search| {
                            let mut search_context =
                                ProviderCallContext::new(search.provider.definition().provider_id);
                            search_context.operation_id = request.operation_id.clone();
                            search_context.timeout_ms = manifest.limits.timeout_ms;
                            search_context.cancellation = cancellation.clone();
                            search_context.dispatch_authorization = dispatch_authorization.clone();
                            WebSearchToolExecutor::new(
                                search.provider,
                                self.context.ledger,
                                &self.context.execution_policy.permissions,
                                search_context,
                                [search.tool.name.clone()],
                            )
                        });
                        let mut tool_router = ToolExecutorRouter::new();
                        if let (Some(search), Some(executor)) =
                            (self.project_search.as_ref(), project_tool_executor.as_ref())
                        {
                            tool_router.register(search.tool.name.clone(), executor)?;
                        }
                        if let (Some(search), Some(executor)) =
                            (self.web_search.as_ref(), web_tool_executor.as_ref())
                        {
                            tool_router.register(search.tool.name.clone(), executor)?;
                        }
                        service.complete_with_tools(
                            provider,
                            llm_request,
                            &tool_router,
                            cancellation,
                        )?
                    } else {
                        service.complete_basic(provider, llm_request, cancellation)?
                    };
                    SkillBackendOutput {
                        outputs: output_text_port(report.response.message.content),
                        logs: vec!["llm skill completed".to_owned()],
                        metadata: json!({ "rounds_completed": report.rounds_completed }),
                        elapsed_ms: 0,
                    }
                }
                SkillExecutorConfig::Http(config) => {
                    self.context.execution_policy.ensure_permission(
                        &PermissionRequest::HttpSkill {
                            host: config.host.clone(),
                        },
                    )?;
                    let backend = self.context.http_backend.ok_or_else(|| {
                        CoreError::validation("http skill requires an HTTP backend")
                    })?;
                    dispatch_authorization.authorize_dispatch()?;
                    backend.execute(
                        config,
                        &request.inputs,
                        request.operation_id.as_deref(),
                        manifest.limits.timeout_ms,
                        cancellation,
                    )?
                }
                SkillExecutorConfig::Wasm(config) => {
                    if config.allow_network {
                        for host in &config.allowed_hosts {
                            self.context.execution_policy.ensure_permission(
                                &PermissionRequest::WasmNetwork { host: host.clone() },
                            )?;
                        }
                    }
                    let backend = self.context.wasm_backend.ok_or_else(|| {
                        CoreError::validation("wasm skill requires a WASM backend")
                    })?;
                    dispatch_authorization.authorize_dispatch()?;
                    backend.execute(
                        config,
                        &request.inputs,
                        manifest.limits.timeout_ms,
                        manifest.limits.max_memory_bytes,
                        cancellation,
                    )?
                }
            };
        let actual_elapsed_ms = elapsed_millis(started_at);
        cancellation.check()?;

        validate_backend_output(
            &output,
            actual_elapsed_ms,
            manifest.limits.timeout_ms,
            manifest.limits.max_output_bytes,
        )?;
        Ok(SkillRunOutput {
            outputs: output.outputs,
            logs: sanitize_skill_logs(&output.logs),
            metadata: output.metadata,
        })
    }

    /// 评估 Skill 执行预算。
    fn check_budget(&self, requested_usd: f64) -> CoreResult<()> {
        let spent = self.context.ledger.total_cost(&CostQuery::default())?;
        let decision = evaluate_budget(
            &self.context.budget_limits,
            self.context.auto_mode_config,
            BudgetUsage {
                requested_usd,
                spent_today_usd: spent,
                spent_this_month_usd: spent,
            },
        );
        match decision.run_control {
            RunControl::Continue => Ok(()),
            RunControl::Pause => Err(CoreError::Paused {
                reason: decision
                    .reason
                    .unwrap_or_else(|| "skill budget requires pause".to_owned()),
            }),
            RunControl::Stop => Err(CoreError::Stopped {
                reason: decision
                    .reason
                    .unwrap_or_else(|| "skill budget requested stop".to_owned()),
            }),
        }
    }
}

/// 检查后端输出是否超过限制。
fn validate_backend_output(
    output: &SkillBackendOutput,
    actual_elapsed_ms: u64,
    timeout_ms: u64,
    max_output_bytes: u64,
) -> CoreResult<()> {
    if actual_elapsed_ms > timeout_ms {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "skill_time".to_owned(),
            reason: format!("elapsed {actual_elapsed_ms}ms exceeds {timeout_ms}ms"),
        });
    }
    if output.elapsed_ms > timeout_ms {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "skill_time".to_owned(),
            reason: format!("elapsed {}ms exceeds {}ms", output.elapsed_ms, timeout_ms),
        });
    }
    let output_bytes = serde_json::to_vec(&output.outputs)?.len() as u64;
    if output_bytes > max_output_bytes {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "skill_output".to_owned(),
            reason: format!("output {output_bytes} bytes exceeds {max_output_bytes} bytes"),
        });
    }
    Ok(())
}

/// 将墙钟耗时转换为 u64 毫秒，溢出时保守按最大值处理。
fn elapsed_millis(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// 简单 prompt 渲染，把输入端口 JSON 放入模板末尾，避免隐式字符串替换误伤。
fn render_prompt(template: &str, inputs: &crate::contracts::PortMap) -> String {
    format!(
        "{template}\n\nInputs:\n{}",
        serde_json::to_string_pretty(inputs).unwrap_or_else(|_| "{}".to_owned())
    )
}

/// 从 LLM 消息中提取文本输出端口。
fn output_text_port(content: Vec<ContentPart>) -> crate::contracts::PortMap {
    let text = content
        .into_iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut outputs = crate::contracts::PortMap::new();
    outputs.insert("text".to_owned(), PortValue::inline(Value::String(text)));
    outputs
}

fn execute_native_http(
    config: &HttpSkillConfig,
    inputs: &crate::contracts::PortMap,
    idempotency_key: Option<&str>,
    timeout_ms: u64,
    cancellation: &CancellationToken,
) -> CoreResult<SkillBackendOutput> {
    cancellation.check()?;
    let started_at = Instant::now();
    let endpoint = HttpEndpoint::parse(&config.host, &config.path)?;
    validate_http_endpoint(&endpoint)?;
    let method = config.method.to_ascii_uppercase();
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| CoreError::validation(format!("invalid HTTP method: {}", config.method)))?;
    if !matches!(method, reqwest::Method::GET | reqwest::Method::POST) {
        return Err(CoreError::validation(format!(
            "native HTTP backend only supports GET and POST, got {}",
            config.method
        )));
    }

    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|error| {
            http_skill_operation_error(
                ExternalDispatchOutcome::NotDispatched,
                format!("failed to build HTTP client: {error}"),
            )
        })?;
    let mut request = client
        .request(method.clone(), endpoint.url())
        .header(reqwest::header::ACCEPT, "application/json");
    if method == reqwest::Method::POST {
        request = request.json(inputs);
    }
    if let Some(header) = config.idempotency_header.as_deref() {
        let key = idempotency_key.ok_or_else(|| {
            CoreError::validation(
                "http skill idempotency_header requires a stable workflow operation_id",
            )
        })?;
        let header = reqwest::header::HeaderName::from_bytes(header.trim().as_bytes())
            .map_err(|_| CoreError::validation("invalid http skill idempotency_header"))?;
        let value = reqwest::header::HeaderValue::from_str(key)
            .map_err(|_| CoreError::validation("invalid http skill operation_id header value"))?;
        request = request.header(header, value);
    }
    request = request.timeout(Duration::from_millis(timeout_ms));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|error| {
            http_skill_operation_error(
                ExternalDispatchOutcome::NotDispatched,
                format!("failed to build HTTP runtime: {error}"),
            )
        })?;
    runtime.block_on(async {
        let mut send = Box::pin(request.send());
        let mut response = loop {
            match tokio::time::timeout(Duration::from_millis(25), &mut send).await {
                Ok(result) => {
                    break result.map_err(|error| {
                        let outcome = if error.is_builder() || error.is_connect() {
                            ExternalDispatchOutcome::NotDispatched
                        } else {
                            ExternalDispatchOutcome::DispatchedUnknown
                        };
                        http_skill_operation_error(
                            outcome,
                            format!("failed to execute HTTP backend: {error}"),
                        )
                    })?
                }
                Err(_) if cancellation.is_cancelled() => {
                    return Err(CoreError::external_cancelled(
                        "http_skill",
                        ExternalDispatchOutcome::DispatchedUnknown,
                    ));
                }
                Err(_) => {}
            }
        };
        parse_http_response_async(&mut response, elapsed_millis(started_at), cancellation).await
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpEndpoint {
    scheme: String,
    host: String,
    port: u16,
    path: String,
}

impl HttpEndpoint {
    fn parse(host: &str, path: &str) -> CoreResult<Self> {
        let host = host.trim();
        if host.is_empty() {
            return Err(CoreError::validation("http skill host cannot be empty"));
        }
        let (scheme, without_scheme) = if let Some(rest) = host.strip_prefix("http://") {
            ("http", rest)
        } else if let Some(rest) = host.strip_prefix("https://") {
            ("https", rest)
        } else {
            ("http", host)
        };
        let (authority, host_path) = without_scheme
            .split_once('/')
            .map(|(authority, rest)| (authority, format!("/{rest}")))
            .unwrap_or_else(|| (without_scheme, String::new()));
        let (hostname, port) = parse_authority(authority, scheme)?;
        let configured_path = if path.trim().is_empty() {
            "/"
        } else {
            path.trim()
        };
        let mut final_path = if configured_path.starts_with('/') {
            configured_path.to_owned()
        } else {
            format!("/{configured_path}")
        };
        if !host_path.is_empty() && final_path == "/" {
            final_path = host_path;
        }
        Ok(Self {
            scheme: scheme.to_owned(),
            host: hostname,
            port,
            path: final_path,
        })
    }

    fn url(&self) -> String {
        format!("{}://{}:{}{}", self.scheme, self.host, self.port, self.path)
    }
}

fn parse_authority(authority: &str, scheme: &str) -> CoreResult<(String, u16)> {
    let authority = authority.trim();
    if authority.is_empty() {
        return Err(CoreError::validation("http skill host cannot be empty"));
    }
    if authority.contains('@') {
        return Err(CoreError::validation(
            "http skill host cannot contain userinfo",
        ));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            let parsed_port = port
                .parse::<u16>()
                .map_err(|_| CoreError::validation(format!("invalid http skill port: {port}")))?;
            (host, parsed_port)
        }
        _ => (authority, default_port_for_scheme(scheme)?),
    };
    if host.trim().is_empty() {
        return Err(CoreError::validation("http skill host cannot be empty"));
    }
    Ok((host.to_owned(), port))
}

fn validate_http_endpoint(endpoint: &HttpEndpoint) -> CoreResult<()> {
    let host = endpoint
        .host
        .trim()
        .trim_matches(&['[', ']'][..])
        .to_ascii_lowercase();
    let allow_local = std::env::var_os("ARIADNE_ALLOW_LOCAL_HTTP_SKILL").is_some();
    if allow_local {
        return Ok(());
    }
    if matches!(host.as_str(), "localhost" | "0.0.0.0") {
        return Err(CoreError::validation(
            "http skill host cannot target local addresses",
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_is_private_or_local(ip) {
            return Err(CoreError::validation(
                "http skill host cannot target private or local addresses",
            ));
        }
    }
    if let Ok(addresses) = (host.as_str(), endpoint.port).to_socket_addrs() {
        for address in addresses {
            if ip_is_private_or_local(address.ip()) {
                return Err(CoreError::validation(
                    "http skill host cannot target private or local addresses",
                ));
            }
        }
    }
    Ok(())
}

fn ip_is_private_or_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

fn default_port_for_scheme(scheme: &str) -> CoreResult<u16> {
    match scheme {
        "http" => Ok(80),
        "https" => Ok(443),
        _ => Err(CoreError::validation(format!(
            "unsupported HTTP skill scheme: {scheme}"
        ))),
    }
}

async fn parse_http_response_async(
    response: &mut reqwest::Response,
    elapsed_ms: u64,
    cancellation: &CancellationToken,
) -> CoreResult<SkillBackendOutput> {
    let status = response.status();
    if !status.is_success() {
        return Err(http_skill_operation_error(
            ExternalDispatchOutcome::ResponseReceived,
            format!("HTTP backend returned status {status}"),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HTTP_RESPONSE_BYTES)
    {
        return Err(http_skill_operation_error(
            ExternalDispatchOutcome::DispatchedUnknown,
            format!("http_skill_response exceeds {MAX_HTTP_RESPONSE_BYTES} bytes"),
        ));
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = match tokio::time::timeout(Duration::from_millis(25), response.chunk()).await {
            Ok(result) => result.map_err(|error| {
                http_skill_operation_error(
                    ExternalDispatchOutcome::DispatchedUnknown,
                    format!("failed to read HTTP response: {error}"),
                )
            })?,
            Err(_) if cancellation.is_cancelled() => {
                return Err(CoreError::external_cancelled(
                    "http_skill",
                    ExternalDispatchOutcome::DispatchedUnknown,
                ));
            }
            Err(_) => continue,
        };
        let Some(chunk) = chunk else { break };
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_HTTP_RESPONSE_BYTES {
            return Err(http_skill_operation_error(
                ExternalDispatchOutcome::DispatchedUnknown,
                format!("http_skill_response exceeds {MAX_HTTP_RESPONSE_BYTES} bytes"),
            ));
        }
    }
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
        http_skill_operation_error(
            ExternalDispatchOutcome::DispatchedUnknown,
            format!("failed to parse HTTP JSON response: {error}"),
        )
    })?;
    let mut output = match serde_json::from_value::<SkillBackendOutput>(value.clone()) {
        Ok(output) if !output.outputs.is_empty() || !output.logs.is_empty() => output,
        _ => {
            let mut outputs = crate::contracts::PortMap::new();
            outputs.insert("result".to_owned(), PortValue::inline(value));
            SkillBackendOutput {
                outputs,
                logs: vec!["http skill completed".to_owned()],
                metadata: Value::Null,
                elapsed_ms,
            }
        }
    };
    if output.elapsed_ms == 0 {
        output.elapsed_ms = elapsed_ms;
    }
    Ok(output)
}

fn http_skill_operation_error(
    outcome: ExternalDispatchOutcome,
    message: impl Into<String>,
) -> CoreError {
    CoreError::ExternalOperation {
        service: "http_skill".to_owned(),
        outcome,
        message: message.into(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WasmWorkerRequest {
    config: WasmSkillConfig,
    inputs: crate::contracts::PortMap,
    timeout_ms: u64,
    max_memory_bytes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WasmWorkerResponse {
    output: Option<SkillBackendOutput>,
    error: Option<WasmWorkerError>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WasmWorkerError {
    ResourceLimit { resource: String, reason: String },
    Validation { message: String },
    Cancelled,
    External { message: String },
}

impl WasmWorkerResponse {
    fn from_result(result: CoreResult<SkillBackendOutput>) -> Self {
        match result {
            Ok(output) => Self {
                output: Some(output),
                error: None,
            },
            Err(error) => Self {
                output: None,
                error: Some(match error {
                    CoreError::ResourceLimitExceeded { resource, reason } => {
                        WasmWorkerError::ResourceLimit { resource, reason }
                    }
                    CoreError::Validation { message } => WasmWorkerError::Validation { message },
                    CoreError::Cancelled => WasmWorkerError::Cancelled,
                    error => WasmWorkerError::External {
                        message: error.to_string(),
                    },
                }),
            },
        }
    }

    fn into_result(self) -> CoreResult<SkillBackendOutput> {
        match (self.output, self.error) {
            (Some(output), None) => Ok(output),
            (None, Some(WasmWorkerError::ResourceLimit { resource, reason })) => {
                Err(CoreError::ResourceLimitExceeded { resource, reason })
            }
            (None, Some(WasmWorkerError::Validation { message })) => {
                Err(CoreError::validation(message))
            }
            (None, Some(WasmWorkerError::Cancelled)) => Err(CoreError::Cancelled),
            (None, Some(WasmWorkerError::External { message })) => Err(wasm_external(message)),
            _ => Err(wasm_external("wasm worker returned an invalid response")),
        }
    }
}

/// `ariadne-ipc wasm-worker` 的受限 stdio 入口。外层进程负责墙钟超时和抢占，
/// 子进程只执行 wasmi，并把类型化结果写回单个 JSON 消息。
pub fn run_wasm_worker_stdio() -> Result<(), String> {
    let request_bytes = read_limited(
        std::io::stdin().lock(),
        MAX_WASM_WORKER_MESSAGE_BYTES,
        "wasm worker request",
    )?;
    let request: WasmWorkerRequest = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("failed to parse wasm worker request: {error}"))?;
    let response = WasmWorkerResponse::from_result(execute_native_wasm_in_process(
        &request.config,
        &request.inputs,
        request.timeout_ms,
        request.max_memory_bytes,
        &CancellationToken::new(),
    ));
    let bytes = serde_json::to_vec(&response)
        .map_err(|error| format!("failed to serialize wasm worker response: {error}"))?;
    if bytes.len() > MAX_WASM_WORKER_MESSAGE_BYTES {
        return Err("wasm worker response exceeds message limit".to_owned());
    }
    std::io::stdout()
        .lock()
        .write_all(&bytes)
        .map_err(|error| format!("failed to write wasm worker response: {error}"))
}

fn execute_native_wasm_isolated(
    config: &WasmSkillConfig,
    inputs: &crate::contracts::PortMap,
    timeout_ms: u64,
    max_memory_bytes: Option<u64>,
    cancellation: &CancellationToken,
    configured_worker: Option<&Path>,
) -> CoreResult<SkillBackendOutput> {
    cancellation.check()?;
    let executable = resolve_wasm_worker_executable(configured_worker)?;
    let request = serde_json::to_vec(&WasmWorkerRequest {
        config: config.clone(),
        inputs: inputs.clone(),
        timeout_ms,
        max_memory_bytes,
    })?;
    if request.len() > MAX_WASM_WORKER_MESSAGE_BYTES {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "wasm_input".to_owned(),
            reason: format!(
                "worker request {} bytes exceeds {} bytes",
                request.len(),
                MAX_WASM_WORKER_MESSAGE_BYTES
            ),
        });
    }

    let mut child = Command::new(&executable)
        .arg("wasm-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            wasm_external(format!(
                "failed to start isolated wasm worker {}: {error}",
                executable.display()
            ))
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| wasm_external("wasm worker stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| wasm_external("wasm worker stderr was not piped"))?;
    let stdout_reader = thread::spawn(move || {
        read_limited(
            stdout,
            MAX_WASM_WORKER_MESSAGE_BYTES,
            "wasm worker response",
        )
    });
    let stderr_reader = thread::spawn(move || {
        read_limited(stderr, MAX_WASM_WORKER_MESSAGE_BYTES, "wasm worker stderr")
    });
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| wasm_external("wasm worker stdin was not piped"))?
        .write_all(&request);
    if let Err(error) = write_result {
        terminate_and_reap(&mut child);
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        return Err(wasm_external(format!(
            "failed to send isolated wasm request: {error}"
        )));
    }

    let started_at = Instant::now();
    let status = loop {
        if cancellation.is_cancelled() {
            terminate_and_reap(&mut child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(CoreError::Cancelled);
        }
        if elapsed_millis(started_at) >= timeout_ms {
            terminate_and_reap(&mut child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(CoreError::ResourceLimitExceeded {
                resource: "wasm_time".to_owned(),
                reason: format!("isolated wasm worker exceeded {timeout_ms}ms wall-clock timeout"),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(WASM_WORKER_POLL_INTERVAL),
            Err(error) => {
                terminate_and_reap(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(wasm_external(format!(
                    "failed to poll isolated wasm worker: {error}"
                )));
            }
        }
    };
    let stdout = join_reader(stdout_reader, "stdout")?;
    let stderr = join_reader(stderr_reader, "stderr")?;
    if !status.success() {
        return Err(wasm_external(format!(
            "isolated wasm worker exited with {status}: {}",
            String::from_utf8_lossy(&stderr)
        )));
    }
    let response: WasmWorkerResponse = serde_json::from_slice(&stdout)
        .map_err(|error| wasm_external(format!("invalid wasm worker response: {error}")))?;
    response.into_result()
}

fn resolve_wasm_worker_executable(configured: Option<&Path>) -> CoreResult<PathBuf> {
    if let Some(configured) = configured {
        if configured.is_file() {
            return Ok(configured.to_path_buf());
        }
        return Err(wasm_external(format!(
            "configured wasm worker does not exist: {}",
            configured.display()
        )));
    }
    let current = std::env::current_exe()
        .map_err(|error| wasm_external(format!("failed to locate current executable: {error}")))?;
    let executable_name = if cfg!(windows) {
        "ariadne-ipc.exe"
    } else {
        "ariadne-ipc"
    };
    if current
        .file_name()
        .is_some_and(|name| name == executable_name)
    {
        return Ok(current);
    }
    let parent = current
        .parent()
        .ok_or_else(|| wasm_external("current executable has no parent directory"))?;
    for candidate in [
        parent.join(executable_name),
        parent
            .parent()
            .map(|directory| directory.join(executable_name))
            .unwrap_or_default(),
        parent.join("Backend").join(executable_name),
        parent
            .parent()
            .map(|directory| directory.join("Backend").join(executable_name))
            .unwrap_or_default(),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(wasm_external(format!(
        "could not locate {executable_name} beside {}",
        current.display()
    )))
}

fn terminate_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn join_reader(
    reader: thread::JoinHandle<Result<Vec<u8>, String>>,
    stream: &str,
) -> CoreResult<Vec<u8>> {
    reader
        .join()
        .map_err(|_| wasm_external(format!("wasm worker {stream} reader panicked")))?
        .map_err(wasm_external)
}

fn read_limited(mut reader: impl Read, limit: usize, label: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {label}: {error}"))?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > limit {
            return Err(format!("{label} exceeds {limit} bytes"));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

fn execute_native_wasm_in_process(
    config: &WasmSkillConfig,
    inputs: &crate::contracts::PortMap,
    timeout_ms: u64,
    max_memory_bytes: Option<u64>,
    cancellation: &CancellationToken,
) -> CoreResult<SkillBackendOutput> {
    cancellation.check()?;
    let started_at = Instant::now();
    let module_path = Path::new(&config.module_path);
    let wasm = std::fs::read(module_path)?;
    let mut engine_config = wasmi::Config::default();
    engine_config.consume_fuel(true);
    let engine = wasmi::Engine::new(&engine_config);
    let module = wasmi::Module::new(&engine, &wasm[..])
        .map_err(|error| wasm_external(format!("failed to compile wasm module: {error}")))?;
    let mut store = wasmi::Store::new(&engine, WasmStoreState::new(max_memory_bytes)?);
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(wasm_fuel_for_timeout(timeout_ms))
        .map_err(|error| wasm_external(format!("failed to configure wasm fuel: {error}")))?;
    let instance = wasmi::Linker::new(&engine)
        .instantiate(&mut store, &module)
        .map_err(|error| wasm_external(format!("failed to instantiate wasm module: {error}")))?
        .start(&mut store)
        .map_err(|error| wasm_runtime_error("failed to start wasm module", error, timeout_ms))?;
    let memory = instance
        .get_export(&store, "memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| wasm_external("wasm module must export memory"))?;
    enforce_wasm_memory_limit(&memory, &store, max_memory_bytes)?;

    let input_json = serde_json::to_vec(inputs)?;
    let input_len = u32::try_from(input_json.len())
        .map_err(|_| CoreError::validation("wasm input json exceeds u32 length"))?;
    let required_len = 8usize
        .checked_add(input_json.len())
        .ok_or_else(|| CoreError::validation("wasm input length overflow"))?;
    if memory.data_size(&store) < required_len {
        return Err(wasm_external(format!(
            "wasm memory is too small: need {required_len} bytes"
        )));
    }
    write_u32_le(&mut store, &memory, 0, input_len)?;
    write_u32_le(&mut store, &memory, 4, 0)?;
    memory
        .write(&mut store, 8, &input_json)
        .map_err(|error| wasm_external(format!("failed to write wasm input: {error}")))?;

    let run = instance
        .get_typed_func::<(), i32>(&store, "run")
        .map_err(|error| wasm_external(format!("wasm module must export run() -> i32: {error}")))?;
    let status = run
        .call(&mut store, ())
        .map_err(|error| wasm_runtime_error("wasm run failed", error, timeout_ms))?;
    cancellation.check()?;
    if status != 0 {
        return Err(wasm_external(format!("wasm run returned status {status}")));
    }
    if elapsed_millis(started_at) > timeout_ms {
        return Err(CoreError::ResourceLimitExceeded {
            resource: "wasm_time".to_owned(),
            reason: format!(
                "elapsed {}ms exceeds {timeout_ms}ms",
                elapsed_millis(started_at)
            ),
        });
    }

    enforce_wasm_memory_limit(&memory, &store, max_memory_bytes)?;
    let output_len = read_u32_le(&store, &memory, 4)? as usize;
    let output_end = 8usize
        .checked_add(output_len)
        .ok_or_else(|| CoreError::validation("wasm output length overflow"))?;
    if output_len == 0 || output_end > memory.data_size(&store) {
        return Err(wasm_external("wasm output length is invalid"));
    }
    let mut output_json = vec![0u8; output_len];
    memory
        .read(&store, 8, &mut output_json)
        .map_err(|error| wasm_external(format!("failed to read wasm output: {error}")))?;
    parse_wasm_output(output_json, elapsed_millis(started_at))
}

fn parse_wasm_output(output_json: Vec<u8>, elapsed_ms: u64) -> CoreResult<SkillBackendOutput> {
    let value = serde_json::from_slice::<Value>(&output_json)?;
    let mut output = match serde_json::from_value::<SkillBackendOutput>(value.clone()) {
        Ok(output) if !output.outputs.is_empty() || !output.logs.is_empty() => output,
        _ => {
            let mut outputs = crate::contracts::PortMap::new();
            outputs.insert("result".to_owned(), PortValue::inline(value));
            SkillBackendOutput {
                outputs,
                logs: vec!["wasm skill completed".to_owned()],
                metadata: Value::Null,
                elapsed_ms,
            }
        }
    };
    if output.elapsed_ms == 0 {
        output.elapsed_ms = elapsed_ms;
    }
    Ok(output)
}

fn enforce_wasm_memory_limit(
    memory: &wasmi::Memory,
    store: &wasmi::Store<WasmStoreState>,
    max_memory_bytes: Option<u64>,
) -> CoreResult<()> {
    if let Some(max_memory_bytes) = max_memory_bytes {
        let actual = memory.data_size(store) as u64;
        if actual > max_memory_bytes {
            return Err(CoreError::ResourceLimitExceeded {
                resource: "wasm_memory".to_owned(),
                reason: format!("memory {actual} bytes exceeds {max_memory_bytes} bytes"),
            });
        }
    }
    Ok(())
}

struct WasmStoreState {
    limits: wasmi::StoreLimits,
}

impl WasmStoreState {
    fn new(max_memory_bytes: Option<u64>) -> CoreResult<Self> {
        let mut builder = wasmi::StoreLimitsBuilder::new()
            .instances(1)
            .memories(1)
            .trap_on_grow_failure(true);
        if let Some(max_memory_bytes) = max_memory_bytes {
            let limit = usize::try_from(max_memory_bytes).map_err(|_| {
                CoreError::validation("wasm max_memory_bytes exceeds platform usize")
            })?;
            builder = builder.memory_size(limit);
        }
        Ok(Self {
            limits: builder.build(),
        })
    }
}

fn write_u32_le<T>(
    store: &mut wasmi::Store<T>,
    memory: &wasmi::Memory,
    offset: usize,
    value: u32,
) -> CoreResult<()> {
    memory
        .write(store, offset, &value.to_le_bytes())
        .map_err(|error| wasm_external(format!("failed to write wasm memory: {error}")))
}

fn read_u32_le<T>(
    store: &wasmi::Store<T>,
    memory: &wasmi::Memory,
    offset: usize,
) -> CoreResult<u32> {
    let mut bytes = [0u8; 4];
    memory
        .read(store, offset, &mut bytes)
        .map_err(|error| wasm_external(format!("failed to read wasm memory: {error}")))?;
    Ok(u32::from_le_bytes(bytes))
}

fn wasm_external(message: impl Into<String>) -> CoreError {
    CoreError::External {
        service: "wasm_skill".to_owned(),
        message: message.into(),
    }
}

fn wasm_runtime_error(context: &str, error: wasmi::Error, timeout_ms: u64) -> CoreError {
    if error.as_trap_code() == Some(wasmi::core::TrapCode::OutOfFuel) {
        return CoreError::ResourceLimitExceeded {
            resource: "wasm_time".to_owned(),
            reason: format!("{context}: fuel exhausted before {timeout_ms}ms timeout"),
        };
    }
    wasm_external(format!("{context}: {error}"))
}

fn wasm_fuel_for_timeout(timeout_ms: u64) -> u64 {
    timeout_ms.saturating_mul(10_000).max(10_000)
}
