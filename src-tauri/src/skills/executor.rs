use serde_json::{json, Value};

use crate::config::AutoModeConfig;
use crate::core::{
    CoreError, CoreResult, ExecutionPolicy, PermissionRequest, PortValue, RunControl,
};
use crate::costs::{evaluate_budget, BudgetLimits, BudgetUsage, CostLedger, CostQuery};
use crate::llm::{LlmRunRequest, LlmService, LlmServiceConfig};
use crate::providers::{ContentPart, LlmMessage, LlmProvider};
use crate::skills::models::{
    HttpSkillConfig, SkillBackendOutput, SkillExecutorConfig, SkillManifest, SkillRunOutput,
    WasmSkillConfig,
};
use crate::skills::sanitizer::sanitize_skill_logs;

/// HTTP Skill 后端接口，真实网络实现后续可替换接入。
pub trait HttpSkillBackend {
    /// 执行一次 HTTP Skill。
    fn execute(
        &self,
        config: &HttpSkillConfig,
        inputs: &crate::core::PortMap,
        timeout_ms: u64,
    ) -> CoreResult<SkillBackendOutput>;
}

/// WASM Skill 后端接口，真实 WASM 运行时后续可替换接入。
pub trait WasmSkillBackend {
    /// 执行一次 WASM Skill。
    fn execute(
        &self,
        config: &WasmSkillConfig,
        inputs: &crate::core::PortMap,
        timeout_ms: u64,
        max_memory_bytes: Option<u64>,
    ) -> CoreResult<SkillBackendOutput>;
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
}

impl<'a, L: CostLedger> SkillExecutor<'a, L> {
    /// 创建 Skill 执行器。
    pub fn new(context: SkillExecutionContext<'a, L>) -> Self {
        Self { context }
    }

    /// 执行 Skill manifest。
    pub fn execute(
        &self,
        manifest: &SkillManifest,
        request: crate::skills::models::SkillRunRequest,
    ) -> CoreResult<SkillRunOutput> {
        if manifest.skill_id != request.skill_id {
            return Err(CoreError::validation(
                "skill run request skill_id does not match manifest",
            ));
        }
        manifest.validate()?;
        crate::core::validate_required_ports(&manifest.schema.input_ports()?, &request.inputs)?;
        self.check_budget(manifest.estimated_cost_usd)?;

        let output =
            match &manifest.executor {
                SkillExecutorConfig::Llm(config) => {
                    let provider = self.context.llm_provider.ok_or_else(|| {
                        CoreError::validation("llm skill requires an LLM provider")
                    })?;
                    let service =
                        LlmService::new(self.context.ledger, self.context.auto_mode_config.clone());
                    let report = service.complete_basic(
                        provider,
                        LlmRunRequest {
                            config: LlmServiceConfig {
                                provider_id: config.provider_id.clone(),
                                model_id: config.model_id.clone(),
                                max_tool_rounds: 0,
                                timeout_ms: manifest.limits.timeout_ms,
                                max_total_tokens: None,
                                budget_limits: self.context.budget_limits.clone(),
                            },
                            messages: vec![LlmMessage::user(render_prompt(
                                &config.prompt_template,
                                &request.inputs,
                            ))],
                            tools: Vec::new(),
                            workflow_id: None,
                            run_id: None,
                            node_id: None,
                            metadata: request.metadata.clone(),
                        },
                        &crate::core::CancellationToken::new(),
                    )?;
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
                    backend.execute(config, &request.inputs, manifest.limits.timeout_ms)?
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
                    backend.execute(
                        config,
                        &request.inputs,
                        manifest.limits.timeout_ms,
                        manifest.limits.max_memory_bytes,
                    )?
                }
            };

        validate_backend_output(
            &output,
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
    timeout_ms: u64,
    max_output_bytes: u64,
) -> CoreResult<()> {
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

/// 简单 prompt 渲染，把输入端口 JSON 放入模板末尾，避免隐式字符串替换误伤。
fn render_prompt(template: &str, inputs: &crate::core::PortMap) -> String {
    format!(
        "{template}\n\nInputs:\n{}",
        serde_json::to_string_pretty(inputs).unwrap_or_else(|_| "{}".to_owned())
    )
}

/// 从 LLM 消息中提取文本输出端口。
fn output_text_port(content: Vec<ContentPart>) -> crate::core::PortMap {
    let text = content
        .into_iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut outputs = crate::core::PortMap::new();
    outputs.insert("text".to_owned(), PortValue::inline(Value::String(text)));
    outputs
}
