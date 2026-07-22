use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contracts::{ArtifactKind, CoreError, CoreResult, NodeId, PortMap, PortValue};
use crate::node_capabilities::workflow_node_catalog_entry;
use crate::skills::{stable_json_hash, stable_text_hash};
use crate::workflow::{
    RuntimeConfirmation, RuntimeConfirmationState, RuntimeLoopControl, WorkflowNodeExecutionOutput,
    WorkflowNodeExecutionRequest, WorkflowNodeExecutor,
};

/// 外部节点适配器，负责 LLM、Document、ExecutorAdapter、Search 和写作节点。
pub trait WorkflowExternalNodeExecutor {
    fn operation_policy(
        &self,
        _request: &WorkflowNodeExecutionRequest,
    ) -> CoreResult<crate::workflow::WorkflowOperationPolicy> {
        Ok(crate::workflow::WorkflowOperationPolicy::Untracked)
    }

    fn reconcile_operation(
        &mut self,
        _request: &WorkflowNodeExecutionRequest,
    ) -> CoreResult<Option<WorkflowNodeExecutionOutput>> {
        Ok(None)
    }

    /// 执行非内建节点。
    fn execute_external(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput>;
}

/// Export 节点 artifact 后端。
pub trait WorkflowExportSink {
    fn operation_policy(&self) -> crate::workflow::WorkflowOperationPolicy {
        crate::workflow::WorkflowOperationPolicy::Untracked
    }

    /// 写入导出 artifact，并返回 artifact id。
    fn export_artifact(
        &mut self,
        request: &WorkflowNodeExecutionRequest,
        export: WorkflowExportRequest,
    ) -> CoreResult<String>;
}

/// 内建节点执行器，处理 Module 11 自身语义。
pub struct BuiltinWorkflowNodeExecutor<'a> {
    external: &'a mut dyn WorkflowExternalNodeExecutor,
    export_sink: Option<&'a mut dyn WorkflowExportSink>,
}

impl<'a> BuiltinWorkflowNodeExecutor<'a> {
    /// 创建内建节点执行器。
    pub fn new(external: &'a mut dyn WorkflowExternalNodeExecutor) -> Self {
        Self {
            external,
            export_sink: None,
        }
    }

    /// 注入 Export artifact 后端。
    pub fn with_export_sink(mut self, export_sink: &'a mut dyn WorkflowExportSink) -> Self {
        self.export_sink = Some(export_sink);
        self
    }
}

impl WorkflowNodeExecutor for BuiltinWorkflowNodeExecutor<'_> {
    fn operation_policy(
        &self,
        request: &WorkflowNodeExecutionRequest,
    ) -> CoreResult<crate::workflow::WorkflowOperationPolicy> {
        match canonical_node_type(&request.type_name) {
            "start" | "condition" | "loop" | "approval" => {
                Ok(crate::workflow::WorkflowOperationPolicy::Untracked)
            }
            "export" => Ok(self
                .export_sink
                .as_ref()
                .map(|sink| sink.operation_policy())
                .unwrap_or(crate::workflow::WorkflowOperationPolicy::Untracked)),
            _ => self.external.operation_policy(request),
        }
    }

    fn reconcile_operation(
        &mut self,
        request: &WorkflowNodeExecutionRequest,
    ) -> CoreResult<Option<WorkflowNodeExecutionOutput>> {
        match canonical_node_type(&request.type_name) {
            "start" | "condition" | "loop" | "approval" | "export" => Ok(None),
            _ => self.external.reconcile_operation(request),
        }
    }

    /// 分发内建节点和外部节点。
    fn execute(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput> {
        if request.cancellation.is_cancelled() {
            return Err(CoreError::external_cancelled(
                "workflow_node",
                crate::contracts::ExternalDispatchOutcome::NotDispatched,
            ));
        }
        // Module 11 只内置控制语义节点；LLM、Document、Search、写作节点都
        // 通过 external 适配器接入，避免 runtime 直接依赖具体服务。
        match canonical_node_type(&request.type_name) {
            "start" => {
                let outputs = start_node_initial_outputs(&request.config);
                Ok(WorkflowNodeExecutionOutput {
                    outputs,
                    metadata: json!({
                        "name": request.config.get("name").cloned().unwrap_or(Value::Null),
                        "work_dir": request.config.get("work_dir").cloned().unwrap_or(Value::Null),
                        "expose_as_tool": request
                            .config
                            .get("expose_as_tool")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    }),
                    ..WorkflowNodeExecutionOutput::default()
                })
            }
            "condition" => execute_condition(request),
            "loop" => execute_loop(request),
            "approval" => execute_approval(request),
            "export" => {
                if let Some(export_sink) = self.export_sink.as_mut() {
                    execute_export_with_sink(request, &mut **export_sink)
                } else {
                    execute_export_without_sink(request)
                }
            }
            _ => self.external.execute_external(request),
        }
    }
}

fn canonical_node_type(type_name: &str) -> &str {
    workflow_node_catalog_entry(type_name)
        .map(|entry| entry.node_type.as_str())
        .unwrap_or(type_name)
}

fn start_node_initial_outputs(config: &Value) -> PortMap {
    let mut outputs = PortMap::new();
    let Some(inputs) = config.get("initial_inputs").and_then(Value::as_object) else {
        return outputs;
    };
    for (key, value) in inputs {
        if !key.trim().is_empty() {
            outputs.insert(key.clone(), PortValue::inline(value.clone()));
        }
    }
    outputs
}

/// Condition/Eval 节点配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionNodeConfig {
    pub input_alias: String,
    #[serde(default)]
    pub expected: Value,
    #[serde(default = "default_condition_operator")]
    pub operator: String,
}

/// Loop 节点配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopNodeConfig {
    pub max_iterations: u32,
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_limit_usd: Option<f64>,
    pub stop_condition: Value,
    #[serde(default)]
    pub rerun_node_ids: Vec<NodeId>,
}

/// Approval 节点配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalNodeConfig {
    pub approval_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(default)]
    pub auto_approve: bool,
}

/// Export 节点配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportNodeConfig {
    pub artifact_id: String,
    #[serde(default = "default_export_format")]
    pub format: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// 交给 Export 后端的导出请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowExportRequest {
    pub artifact_id: String,
    pub format: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub inputs: PortMap,
}

/// 执行 Condition/Eval 节点。
fn execute_condition(
    request: WorkflowNodeExecutionRequest,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<ConditionNodeConfig>(request.config)?;
    let input = request.inputs.get(&config.input_alias).ok_or_else(|| {
        CoreError::validation(format!(
            "condition input alias missing: {}",
            config.input_alias
        ))
    })?;
    let input_value = port_value_as_json(input);
    // 先把 PortValue 统一转成 JSON 再判断，保证 inline/ref 类型都能进入
    // 相同的比较路径，后续扩展更多 operator 也不会分散处理。
    let passed = match config.operator.as_str() {
        "truthy" => value_truthy(&input_value),
        "equals" => input_value == config.expected,
        "not_equals" => input_value != config.expected,
        other => {
            return Err(CoreError::validation(format!(
                "unsupported condition operator: {other}"
            )))
        }
    };

    let mut outputs = PortMap::new();
    outputs.insert("passed".to_owned(), PortValue::inline(passed));
    outputs.insert(
        "reason".to_owned(),
        PortValue::inline(condition_reason(passed)),
    );
    outputs.insert(
        "branch".to_owned(),
        PortValue::inline(if passed { "true" } else { "false" }),
    );

    Ok(WorkflowNodeExecutionOutput {
        outputs,
        metadata: json!({
            "input_hash": stable_json_hash(&input_value)?,
            "operator": config.operator,
        }),
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// 执行 Loop 节点。
fn execute_loop(request: WorkflowNodeExecutionRequest) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<LoopNodeConfig>(request.config)?;
    // Loop 节点配置要先转换成核心 LoopPolicy 并校验边界，防止节点输出
    // loop_control 时绕过 max_iterations / timeout / stop_condition 约束。
    let policy = crate::contracts::LoopPolicy {
        max_iterations: config.max_iterations,
        timeout_ms: config.timeout_ms,
        budget_limit_usd: config.budget_limit_usd,
        stop_condition: config.stop_condition.clone(),
    };
    policy.validate()?;

    let continue_loop = loop_condition_requests_continue(&request.inputs, &config.stop_condition);
    let mut outputs = PortMap::new();
    outputs.insert("continue_loop".to_owned(), PortValue::inline(continue_loop));
    outputs.insert(
        "termination_reason".to_owned(),
        PortValue::inline(if continue_loop {
            "condition_not_satisfied"
        } else {
            "stop_condition_satisfied"
        }),
    );

    Ok(WorkflowNodeExecutionOutput {
        outputs,
        loop_control: Some(RuntimeLoopControl {
            continue_loop,
            rerun_node_ids: config.rerun_node_ids,
            reason: Some(if continue_loop {
                "condition_not_satisfied".to_owned()
            } else {
                "stop_condition_satisfied".to_owned()
            }),
        }),
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// 执行 Approval 节点。
fn execute_approval(
    request: WorkflowNodeExecutionRequest,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<ApprovalNodeConfig>(request.config)?;
    let mut outputs = PortMap::new();
    outputs.insert(
        "approved".to_owned(),
        PortValue::inline(config.auto_approve),
    );
    outputs.insert("rejected".to_owned(), PortValue::inline(false));
    outputs.insert(
        "review_reason".to_owned(),
        PortValue::inline(if config.auto_approve {
            "auto approved"
        } else {
            "pending approval"
        }),
    );

    let state = if config.auto_approve {
        RuntimeConfirmationState::AutoAudited
    } else {
        RuntimeConfirmationState::Pending
    };
    let confirmation = RuntimeConfirmation {
        confirmation_id: config.approval_id.clone(),
        node_id: request.node_id.clone(),
        state,
        artifact_id: None,
        patch_session_commit_id: None,
        metadata: json!({
            "kind": "approval",
            "rejection_behavior": "stop_workflow",
            "prompt_id": config.prompt_id,
            "reason": if config.auto_approve { "auto approved" } else { "pending approval" },
        }),
    };

    // 待确认项由 runtime 的统一 pending-confirmation 门禁暂停运行。
    // 节点本身先记录为成功，审批解决后才能复用已有输出推进下游，
    // 避免 Resume 重新执行审批节点并再次生成同一个待审项。
    Ok(WorkflowNodeExecutionOutput {
        outputs,
        confirmations: vec![confirmation],
        run_control: None,
        metadata: json!({ "approval_id": config.approval_id }),
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// 执行带真实导出后端的 Export 节点。
fn execute_export_with_sink(
    request: WorkflowNodeExecutionRequest,
    export_sink: &mut dyn WorkflowExportSink,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<ExportNodeConfig>(request.config.clone())?;
    let export = WorkflowExportRequest {
        artifact_id: config.artifact_id.clone(),
        format: config.format.clone(),
        title: config.title.clone(),
        inputs: request.inputs.clone(),
    };
    let artifact_id = export_sink.export_artifact(&request, export)?;
    export_output(config, artifact_id)
}

/// 执行没有真实导出后端的 Export 节点。
fn execute_export_without_sink(
    request: WorkflowNodeExecutionRequest,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let config = serde_json::from_value::<ExportNodeConfig>(request.config)?;
    let artifact_id = config.artifact_id.clone();
    export_output(config, artifact_id)
}

/// 构造 Export 节点标准输出。
fn export_output(
    config: ExportNodeConfig,
    artifact_id: String,
) -> CoreResult<WorkflowNodeExecutionOutput> {
    let mut outputs = PortMap::new();
    outputs.insert(
        "artifact".to_owned(),
        PortValue::artifact_ref(artifact_id.clone()),
    );
    outputs.insert("format".to_owned(), PortValue::inline(config.format));

    Ok(WorkflowNodeExecutionOutput {
        outputs,
        metadata: json!({
            "artifact_id": artifact_id,
            "artifact_kind": ArtifactKind::Export,
        }),
        ..WorkflowNodeExecutionOutput::default()
    })
}

/// 判断 Loop 停止条件是否还未满足。
fn loop_condition_requests_continue(inputs: &PortMap, stop_condition: &Value) -> bool {
    // 当前 stop_condition 使用最小稳定形态：
    // { "input_alias": "...", "equals": value }。缺少输入时保守地继续循环，
    // 由 runtime 的最大轮次兜底，避免条件缺失时误判成功。
    let Some(alias) = stop_condition.get("input_alias").and_then(Value::as_str) else {
        return true;
    };
    let expected = stop_condition
        .get("equals")
        .cloned()
        .unwrap_or(Value::Bool(true));
    let Some(value) = inputs.get(alias).map(port_value_as_json) else {
        return true;
    };
    value != expected
}

/// 将 PortValue 转成可比较 JSON。
fn port_value_as_json(value: &PortValue) -> Value {
    match value {
        PortValue::Inline { value } => value.clone(),
        PortValue::DocumentRef { document_id, range } => json!({
            "kind": "document_ref",
            "document_id": document_id,
            "range": range,
        }),
        PortValue::ChunkRef { chunk_id } => json!({
            "kind": "chunk_ref",
            "chunk_id": chunk_id,
        }),
        PortValue::ArtifactRef { artifact_id } => json!({
            "kind": "artifact_ref",
            "artifact_id": artifact_id,
        }),
    }
}

/// 判断 JSON 值是否为真值。
fn value_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

/// 生成 Condition 节点的人类可读原因。
fn condition_reason(passed: bool) -> &'static str {
    if passed {
        "condition passed"
    } else {
        "condition failed"
    }
}

/// Condition operator 的 serde 默认值。
fn default_condition_operator() -> String {
    "truthy".to_owned()
}

/// Export format 的 serde 默认值。
fn default_export_format() -> String {
    "markdown".to_owned()
}

/// 测试用外部节点执行器。
#[derive(Default)]
pub struct NoopExternalNodeExecutor {
    pub calls: BTreeMap<String, usize>,
}

impl WorkflowExternalNodeExecutor for NoopExternalNodeExecutor {
    /// 默认外部节点返回空输出，并记录调用次数。
    fn execute_external(
        &mut self,
        request: WorkflowNodeExecutionRequest,
    ) -> CoreResult<WorkflowNodeExecutionOutput> {
        *self
            .calls
            .entry(request.node_id.as_str().to_owned())
            .or_insert(0) += 1;
        Ok(WorkflowNodeExecutionOutput {
            metadata: json!({
                "external_node_type": request.type_name,
                "request_hash": stable_text_hash(request.node_id.as_str()),
            }),
            ..WorkflowNodeExecutionOutput::default()
        })
    }
}
