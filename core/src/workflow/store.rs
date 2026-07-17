use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::{
    CoreError, CoreResult, ExternalDispatchAuthorization, ExternalDispatchOutcome, NodeId,
    RunControl, RunId, RunStatus, WorkflowDefinition, WorkflowId,
};
use crate::workflow::{WorkflowRunState, WorkflowRuntimeStore};

pub const RUNTIME_DB_FILE: &str = "runtime.db";
pub const RUNTIME_SCHEMA_VERSION: i64 = 11;

/// 可恢复外部副作用的 operation journal 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOperationStatus {
    Prepared,
    Dispatched,
    Completed,
    InDoubt,
    Aborted,
    Committed,
}

/// 执行器返回响应后，存储层对 operation journal 的原子结算结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowOperationCompletionOutcome {
    /// 派发授权已被消费，响应已从 Dispatched 原子推进到 Completed。
    Completed,
    /// 执行器未消费派发授权；Prepared 已原子隔离为 InDoubt。
    DispatchAuthorizationMissing,
}

/// operation 已进入 dispatched 但没有 runtime 响应时的恢复策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOperationRecoveryPolicy {
    /// 远端结果可能未知，必须进入 in_doubt 由作者显式处理。
    ManualResolution,
    /// 执行后端能以同一 operation ID 幂等执行；runtime 可直接重新调用执行器。
    ReplayExecutor,
    /// 执行后端可查询最终 receipt；存在则自动对账，不存在则进入 in_doubt。
    ReconcileReceipt,
}

/// in_doubt operation 是否允许用外部响应替代执行器结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOperationResponsePolicy {
    /// Provider 或远端工具结果可由作者提供完整响应继续。
    AllowExternalResponse,
    /// 必须由执行器自己的 receipt 恢复，禁止绕过本地副作用事务。
    RequireExecutorReceipt,
}

/// 节点执行器声明给 runtime 的副作用恢复能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkflowOperationPolicy {
    #[default]
    Untracked,
    Journaled {
        recovery: WorkflowOperationRecoveryPolicy,
        response: WorkflowOperationResponsePolicy,
    },
}

impl WorkflowOperationPolicy {
    pub const fn remote_response() -> Self {
        Self::Journaled {
            recovery: WorkflowOperationRecoveryPolicy::ManualResolution,
            response: WorkflowOperationResponsePolicy::AllowExternalResponse,
        }
    }

    pub const fn replayable_receipt() -> Self {
        Self::Journaled {
            recovery: WorkflowOperationRecoveryPolicy::ReplayExecutor,
            response: WorkflowOperationResponsePolicy::RequireExecutorReceipt,
        }
    }

    /// 远端调用没有可证明的幂等或对账协议。未知结果时保留审计记录、自动终止
    /// 运行且永不重发；不要求作者猜测远端结果。
    pub const fn at_most_once() -> Self {
        Self::Journaled {
            recovery: WorkflowOperationRecoveryPolicy::ManualResolution,
            response: WorkflowOperationResponsePolicy::RequireExecutorReceipt,
        }
    }

    /// 只读或携带稳定幂等键的远端调用，可用同一 operation ID 自动重放。
    pub const fn replayable_remote() -> Self {
        Self::Journaled {
            recovery: WorkflowOperationRecoveryPolicy::ReplayExecutor,
            response: WorkflowOperationResponsePolicy::AllowExternalResponse,
        }
    }

    pub const fn is_at_most_once(self) -> bool {
        matches!(
            self,
            Self::Journaled {
                recovery: WorkflowOperationRecoveryPolicy::ManualResolution,
                response: WorkflowOperationResponsePolicy::RequireExecutorReceipt,
            }
        )
    }

    /// 只有远端已声明稳定幂等键或天然只读时，未知结果才允许由 scheduler
    /// 按原 operation identity 自动退避重放。依赖本地 receipt 的执行器即使可
    /// 重入，在 receipt 仍未知时也必须暂停，避免无意义空转到失败。
    pub const fn is_automatically_replayable(self) -> bool {
        matches!(
            self,
            Self::Journaled {
                recovery: WorkflowOperationRecoveryPolicy::ReplayExecutor,
                response: WorkflowOperationResponsePolicy::AllowExternalResponse,
            }
        )
    }

    pub const fn reconcilable_receipt() -> Self {
        Self::Journaled {
            recovery: WorkflowOperationRecoveryPolicy::ReconcileReceipt,
            response: WorkflowOperationResponsePolicy::RequireExecutorReceipt,
        }
    }
}

/// workflow 节点的一次稳定外部操作记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowOperation {
    pub operation_id: String,
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub node_id: NodeId,
    pub attempt: u32,
    pub kind: String,
    pub provider: String,
    pub request_hash: String,
    pub lease_generation: u64,
    pub recovery_policy: WorkflowOperationRecoveryPolicy,
    pub response_policy: WorkflowOperationResponsePolicy,
    pub status: WorkflowOperationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_json: Option<Value>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_doubt_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_at_ms: Option<u64>,
}

/// 创建 prepared operation 所需的不可变标识与请求摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkflowOperation {
    pub operation_id: String,
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub node_id: NodeId,
    pub attempt: u32,
    pub kind: String,
    pub provider: String,
    pub request_hash: String,
    pub lease_generation: u64,
    pub recovery_policy: WorkflowOperationRecoveryPolicy,
    pub response_policy: WorkflowOperationResponsePolicy,
}

/// 单个 workflow run 的持久化 worker lease，并携带单调 fencing generation。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowWorkerLease {
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub owner_id: String,
    pub generation: u64,
    pub acquired_at_ms: u64,
    pub heartbeat_at_ms: u64,
    pub expires_at_ms: u64,
}

/// 原子 Resume claim 的结果。调用方可区分不存在、不可恢复和已有活跃 worker，
/// 且只有 `Claimed` 会修改运行快照与 lease。
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum WorkflowResumeClaimResult {
    Claimed {
        state: WorkflowRunState,
        lease: WorkflowWorkerLease,
    },
    NotFound,
    NotResumable {
        status: RunStatus,
    },
    Busy {
        lease: WorkflowWorkerLease,
    },
}

/// F9：调度器在同一事务中选取到期 runnable 并领取唯一 worker lease。
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum WorkflowRunnableClaimResult {
    Claimed {
        state: WorkflowRunState,
        lease: WorkflowWorkerLease,
    },
    Stopped {
        state: WorkflowRunState,
    },
    Empty,
}

/// 原子运行态变更与可选 worker claim 的结果。
///
/// 变更后的状态为 Queued/Running 时，`Saved` 必然携带 lease；其他状态不会
/// 获取 lease。若已有活跃 worker，返回 `Busy` 且闭包产生的全部状态变更回滚。
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum WorkflowMutationClaimResult {
    Saved {
        state: WorkflowRunState,
        lease: Option<WorkflowWorkerLease>,
    },
    NotFound,
    Busy {
        lease: WorkflowWorkerLease,
    },
}

/// F14：跨 runtime/knowledge 确认决策的持久协调状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationResolutionStatus {
    Prepared,
    KnowledgeCommitted,
    Committed,
}

/// F14：确认决策使用稳定领域值，不把 UI 字符串写入协调协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationResolutionDecision {
    Approve,
    Reject,
}

/// F14：runtime.db 中的 durable saga；knowledge receipt 是第二数据库的提交证明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationResolutionOperation {
    pub operation_id: String,
    pub workflow_id: WorkflowId,
    pub run_id: RunId,
    pub confirmation_id: String,
    pub decision: ConfirmationResolutionDecision,
    pub review_reason: Option<String>,
    pub request_hash: String,
    pub knowledge_required: bool,
    pub status: ConfirmationResolutionStatus,
    pub projected: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ConfirmationResolutionCommitResult {
    Saved { state: WorkflowRunState },
    AlreadyCommitted { state: WorkflowRunState },
    NotFound,
}

/// 原子 Stop 的结果。活跃 worker 存在时先进入 Stopping 并保留 lease；
/// 没有执行者时直接收敛为 Stopped。
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum WorkflowStopRequestResult {
    Saved { state: WorkflowRunState },
    NotFound,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InDoubtResolution {
    Retry,
    UseResponse { response: Value },
    Stop { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum InDoubtResolutionResult {
    Saved {
        state: WorkflowRunState,
        lease: Option<WorkflowWorkerLease>,
    },
    NotFound,
    NotInDoubt {
        status: Option<WorkflowOperationStatus>,
    },
    Busy {
        lease: WorkflowWorkerLease,
    },
}

#[derive(Serialize)]
struct PersistedWorkflowRunState<'a> {
    workflow_id: &'a WorkflowId,
    run_id: &'a RunId,
    prepared_workflow: &'a Option<WorkflowDefinition>,
    start_node_id: &'a Option<crate::contracts::NodeId>,
    status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_retry_at_ms: Option<u64>,
    control: crate::contracts::RunControl,
    pause_reason: &'a Option<String>,
    stop_reason: &'a Option<String>,
    failure: &'a Option<crate::workflow::WorkflowRunFailure>,
    nodes: &'a std::collections::BTreeMap<
        crate::contracts::NodeId,
        crate::workflow::WorkflowNodeRuntimeState,
    >,
    node_operation_sequences: &'a std::collections::BTreeMap<crate::contracts::NodeId, u32>,
    communication_edges: &'a std::collections::BTreeMap<
        crate::contracts::EdgeId,
        crate::workflow::CommunicationRuntimeState,
    >,
    loop_iterations: &'a std::collections::BTreeMap<crate::contracts::NodeId, u32>,
    rerun_queue: &'a Vec<crate::contracts::NodeId>,
    confirmations: &'a std::collections::BTreeMap<String, crate::workflow::RuntimeConfirmation>,
    next_event_sequence: u64,
    retry_policy: crate::workflow::NodeRetryPolicy,
}

impl<'a> From<&'a WorkflowRunState> for PersistedWorkflowRunState<'a> {
    fn from(state: &'a WorkflowRunState) -> Self {
        Self {
            workflow_id: &state.workflow_id,
            run_id: &state.run_id,
            prepared_workflow: &state.prepared_workflow,
            start_node_id: &state.start_node_id,
            status: state.status,
            next_retry_at_ms: state.next_retry_at_ms,
            control: state.control,
            pause_reason: &state.pause_reason,
            stop_reason: &state.stop_reason,
            failure: &state.failure,
            nodes: &state.nodes,
            node_operation_sequences: &state.node_operation_sequences,
            communication_edges: &state.communication_edges,
            loop_iterations: &state.loop_iterations,
            rerun_queue: &state.rerun_queue,
            confirmations: &state.confirmations,
            next_event_sequence: state.next_event_sequence,
            retry_policy: state.retry_policy,
        }
    }
}

/// runtime.db 健康状态，用于恢复诊断和前端提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStoreHealth {
    Healthy,
    Missing,
    Corrupt { message: String },
}

/// SQLite 工作流运行状态存储。
#[derive(Debug, Clone)]
pub struct SqliteWorkflowRuntimeStore {
    db_path: Option<PathBuf>,
    connection: Arc<Mutex<Connection>>,
    worker_lease: Option<WorkflowWorkerLease>,
}

impl SqliteWorkflowRuntimeStore {
    /// 在项目根目录打开 runtime.db。
    pub fn open(project_root: impl AsRef<Path>) -> CoreResult<Self> {
        let db_path = project_root.as_ref().join(RUNTIME_DB_FILE);
        let connection = Connection::open(&db_path).map_err(sqlite_error)?;
        configure_connection(&connection, true)?;
        let store = Self {
            db_path: Some(db_path),
            connection: Arc::new(Mutex::new(connection)),
            worker_lease: None,
        };
        store.migrate()?;
        Ok(store)
    }

    /// 打开内存 runtime 存储，主要用于契约测试。
    pub fn open_in_memory() -> CoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(sqlite_error)?;
        configure_connection(&connection, false)?;
        let store = Self {
            db_path: None,
            connection: Arc::new(Mutex::new(connection)),
            worker_lease: None,
        };
        store.migrate()?;
        Ok(store)
    }

    /// 返回数据库路径；内存模式下为 None。
    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    /// 将本连接限制为指定 fencing lease。之后每次保存都在同一 IMMEDIATE
    /// 事务内验证 owner+generation+有效期，过期 worker 无法覆盖接管者状态。
    pub fn with_worker_lease(mut self, lease: WorkflowWorkerLease) -> Self {
        self.worker_lease = Some(lease);
        self
    }

    /// F12-b：dispatch 门禁读取当前 fencing lease。
    pub fn worker_lease(&self) -> Option<&WorkflowWorkerLease> {
        self.worker_lease.as_ref()
    }

    /// 执行 runtime.db 幂等迁移。
    pub fn migrate(&self) -> CoreResult<()> {
        let mut connection = self.connection.lock().map_err(lock_error)?;
        // runtime.db 当前以整份 JSON 快照为权威状态，workflow_runs 只索引
        // workflow/run/status。这样后续追加节点级查询表时，不会破坏已有快照。
        connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    name TEXT PRIMARY KEY,
                    version INTEGER NOT NULL,
                    applied_at_ms INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS workflow_runs (
                    workflow_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    control TEXT NOT NULL DEFAULT 'continue' CHECK(control IN (
                        'continue', 'pause', 'stop'
                    )),
                    updated_at_ms INTEGER NOT NULL,
                    state_revision INTEGER NOT NULL DEFAULT 0,
                    state_json TEXT NOT NULL,
                    -- F9：可重试运行的下一次 runnable 时间，避免调度器反序列化全量快照扫描。
                    next_retry_at_ms INTEGER,
                    -- C10：执行定义单独落库一次；revision>0 的 state_json 不再重复整图。
                    prepared_workflow_json TEXT,
                    PRIMARY KEY(workflow_id, run_id)
                );

                CREATE INDEX IF NOT EXISTS idx_workflow_runs_status
                    ON workflow_runs(status);

                CREATE TABLE IF NOT EXISTS workflow_run_events (
                    workflow_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    sequence INTEGER NOT NULL,
                    event_json TEXT NOT NULL,
                    PRIMARY KEY(workflow_id, run_id, sequence)
                );

                CREATE TABLE IF NOT EXISTS workflow_run_legacy_events (
                    workflow_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    event_index INTEGER NOT NULL,
                    event_text TEXT NOT NULL,
                    PRIMARY KEY(workflow_id, run_id, event_index)
                );

                CREATE TABLE IF NOT EXISTS workflow_run_worker_leases (
                    workflow_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    owner_id TEXT,
                    generation INTEGER NOT NULL,
                    acquired_at_ms INTEGER NOT NULL,
                    heartbeat_at_ms INTEGER NOT NULL,
                    expires_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(workflow_id, run_id),
                    FOREIGN KEY(workflow_id, run_id)
                        REFERENCES workflow_runs(workflow_id, run_id)
                        ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_workflow_run_worker_leases_expiry
                    ON workflow_run_worker_leases(expires_at_ms);

                CREATE TABLE IF NOT EXISTS workflow_scheduler_leases (
                    scheduler_key TEXT PRIMARY KEY,
                    owner_id TEXT NOT NULL,
                    generation INTEGER NOT NULL,
                    acquired_at_ms INTEGER NOT NULL,
                    heartbeat_at_ms INTEGER NOT NULL,
                    expires_at_ms INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS workflow_operations (
                    operation_id TEXT PRIMARY KEY,
                    workflow_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    node_id TEXT NOT NULL,
                    attempt INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    request_hash TEXT NOT NULL,
                    lease_generation INTEGER NOT NULL,
                    recovery_policy TEXT NOT NULL CHECK(recovery_policy IN (
                        'manual_resolution', 'replay_executor', 'reconcile_receipt'
                    )),
                    response_policy TEXT NOT NULL CHECK(response_policy IN (
                        'allow_external_response', 'require_executor_receipt'
                    )),
                    status TEXT NOT NULL CHECK(status IN (
                        'prepared', 'dispatched', 'completed', 'in_doubt', 'aborted', 'committed'
                    )),
                    response_json TEXT,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    dispatched_at_ms INTEGER,
                    completed_at_ms INTEGER,
                    in_doubt_at_ms INTEGER,
                    committed_at_ms INTEGER,
                    FOREIGN KEY(workflow_id, run_id)
                        REFERENCES workflow_runs(workflow_id, run_id)
                        ON DELETE CASCADE
                );

                CREATE INDEX IF NOT EXISTS idx_workflow_operations_run
                    ON workflow_operations(workflow_id, run_id, node_id, attempt);
                CREATE INDEX IF NOT EXISTS idx_workflow_operations_status
                    ON workflow_operations(status, updated_at_ms);

                CREATE TABLE IF NOT EXISTS confirmation_resolution_operations (
                    operation_id TEXT PRIMARY KEY,
                    workflow_id TEXT NOT NULL,
                    run_id TEXT NOT NULL,
                    confirmation_id TEXT NOT NULL,
                    decision TEXT NOT NULL CHECK(decision IN ('approve', 'reject')),
                    review_reason TEXT,
                    request_hash TEXT NOT NULL,
                    knowledge_required INTEGER NOT NULL CHECK(knowledge_required IN (0, 1)),
                    status TEXT NOT NULL CHECK(status IN (
                        'prepared', 'knowledge_committed', 'committed'
                    )),
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    projected_at_ms INTEGER,
                    UNIQUE(workflow_id, run_id, confirmation_id),
                    FOREIGN KEY(workflow_id, run_id)
                        REFERENCES workflow_runs(workflow_id, run_id)
                        ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_confirmation_resolution_recovery
                    ON confirmation_resolution_operations(status, projected_at_ms, updated_at_ms);
                ",
            )
            .map_err(sqlite_error)?;

        migrate_workflow_operations_v6(&mut connection)?;
        migrate_workflow_operations_v7(&mut connection)?;

        let previous_version = connection
            .query_row(
                "SELECT version FROM schema_migrations WHERE name = 'workflow_runtime'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(sqlite_error)?
            .unwrap_or(0);
        if previous_version < 2 {
            migrate_embedded_events(&mut connection)?;
        }
        if previous_version < 4
            && !sqlite_column_exists(&connection, "workflow_runs", "state_revision")?
        {
            connection
                .execute(
                    "ALTER TABLE workflow_runs ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(sqlite_error)?;
        }
        // C10：为既有 runtime.db 追加 prepared_workflow 独立列并回填。
        if previous_version < 8 {
            migrate_prepared_workflow_column_v8(&mut connection)?;
        }
        if previous_version < 9 || !sqlite_column_exists(&connection, "workflow_runs", "control")? {
            migrate_workflow_run_control_v9(&mut connection)?;
        }
        connection
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_workflow_runs_control_status
                 ON workflow_runs(control, status)",
                [],
            )
            .map_err(sqlite_error)?;
        if previous_version < 11
            || !sqlite_column_exists(&connection, "workflow_runs", "next_retry_at_ms")?
        {
            migrate_workflow_run_retry_v11(&mut connection)?;
        }
        connection
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_workflow_runs_runnable
                 ON workflow_runs(status, next_retry_at_ms, updated_at_ms)",
                [],
            )
            .map_err(sqlite_error)?;

        connection
            .execute(
                "
                INSERT INTO schema_migrations(name, version, applied_at_ms)
                VALUES('workflow_runtime', ?1, ?2)
                ON CONFLICT(name) DO UPDATE SET
                    version = excluded.version,
                    applied_at_ms = excluded.applied_at_ms
                ",
                params![RUNTIME_SCHEMA_VERSION, unix_timestamp_ms_i64()?],
            )
            .map_err(sqlite_error)?;

        Ok(())
    }

    /// 读取 runtime.db 当前 schema version。
    pub fn schema_version(&self) -> CoreResult<Option<i64>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .query_row(
                "SELECT version FROM schema_migrations WHERE name = 'workflow_runtime'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sqlite_error)
    }

    /// 创建 prepared operation。稳定 ID 冲突时拒绝覆盖，避免不同请求复用同一 ID。
    pub fn create_operation(
        &self,
        operation: &NewWorkflowOperation,
        now_ms: u64,
    ) -> CoreResult<()> {
        validate_operation(operation)?;
        let now_ms = sqlite_millis(now_ms, "workflow operation timestamp")?;
        let attempt = i64::from(operation.attempt);
        let generation = sqlite_millis(
            operation.lease_generation,
            "workflow operation lease generation",
        )?;
        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .execute(
                "
                INSERT INTO workflow_operations(
                    operation_id, workflow_id, run_id, node_id, attempt, kind,
                    provider, request_hash, lease_generation, recovery_policy,
                    response_policy, status, created_at_ms, updated_at_ms
                ) VALUES(
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    'prepared', ?12, ?12
                )
                ",
                params![
                    operation.operation_id,
                    operation.workflow_id.as_str(),
                    operation.run_id.as_str(),
                    operation.node_id.as_str(),
                    attempt,
                    operation.kind,
                    operation.provider,
                    operation.request_hash,
                    generation,
                    operation_recovery_policy_name(operation.recovery_policy),
                    operation_response_policy_name(operation.response_policy),
                    now_ms,
                ],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    /// 按稳定 ID 读取 operation。
    pub fn load_operation(&self, operation_id: &str) -> CoreResult<Option<WorkflowOperation>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .query_row(
                "SELECT operation_id, workflow_id, run_id, node_id, attempt, kind,
                        provider, request_hash, lease_generation, recovery_policy,
                        response_policy, status, response_json,
                        created_at_ms, updated_at_ms, dispatched_at_ms, completed_at_ms,
                        in_doubt_at_ms, committed_at_ms
                 FROM workflow_operations WHERE operation_id = ?1",
                params![operation_id],
                read_operation,
            )
            .optional()
            .map_err(sqlite_error)?
            .map(parse_operation)
            .transpose()
    }

    /// 按 run 列出 operation，稳定地按创建时间和 ID 排序。
    pub fn list_operations(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
    ) -> CoreResult<Vec<WorkflowOperation>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "SELECT operation_id, workflow_id, run_id, node_id, attempt, kind,
                        provider, request_hash, lease_generation, recovery_policy,
                        response_policy, status, response_json,
                        created_at_ms, updated_at_ms, dispatched_at_ms, completed_at_ms,
                        in_doubt_at_ms, committed_at_ms
                 FROM workflow_operations
                 WHERE workflow_id = ?1 AND run_id = ?2
                 ORDER BY created_at_ms, operation_id",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map(
                params![workflow_id.as_str(), run_id.as_str()],
                read_operation,
            )
            .map_err(sqlite_error)?;
        rows.map(|row| row.map_err(sqlite_error).and_then(parse_operation))
            .collect()
    }

    /// 删除尚未 dispatch 的 prepared operation。已产生外部副作用的记录不可删除。
    pub fn delete_prepared_operation(&self, operation_id: &str) -> CoreResult<bool> {
        let connection = self.connection.lock().map_err(lock_error)?;
        Ok(connection
            .execute(
                "DELETE FROM workflow_operations WHERE operation_id = ?1 AND status = 'prepared'",
                params![operation_id],
            )
            .map_err(sqlite_error)?
            == 1)
    }

    /// 以 expected status 做 CAS 迁移。状态不匹配返回 false，不覆盖并发结果。
    pub fn transition_operation(
        &self,
        operation_id: &str,
        expected: WorkflowOperationStatus,
        next: WorkflowOperationStatus,
        response_json: Option<&Value>,
        now_ms: u64,
    ) -> CoreResult<bool> {
        validate_operation_transition(expected, next, response_json)?;
        let now_ms = sqlite_millis(now_ms, "workflow operation timestamp")?;
        let response_json = response_json.map(serde_json::to_string).transpose()?;
        let timestamp_column = match next {
            WorkflowOperationStatus::Dispatched => "dispatched_at_ms",
            WorkflowOperationStatus::Completed => "completed_at_ms",
            WorkflowOperationStatus::InDoubt => "in_doubt_at_ms",
            WorkflowOperationStatus::Aborted => "updated_at_ms",
            WorkflowOperationStatus::Committed => "committed_at_ms",
            WorkflowOperationStatus::Prepared => unreachable!("validated transition"),
        };
        let sql = format!(
            "UPDATE workflow_operations
             SET status = ?1, updated_at_ms = ?2, response_json = COALESCE(?3, response_json),
                 {timestamp_column} = ?2
             WHERE operation_id = ?4 AND status = ?5"
        );
        let connection = self.connection.lock().map_err(lock_error)?;
        Ok(connection
            .execute(
                &sql,
                params![
                    operation_status_name(next),
                    now_ms,
                    response_json,
                    operation_id,
                    operation_status_name(expected),
                ],
            )
            .map_err(sqlite_error)?
            == 1)
    }

    /// 原子结算执行器成功响应。该事务与 `authorize_operation_dispatch` 使用相同的
    /// IMMEDIATE 写入线性化域：授权先赢则完成 Dispatched→Completed；响应先赢且
    /// 仍为 Prepared 时，说明执行器绕过了授权，必须隔离为 InDoubt。
    pub fn complete_operation_response(
        &self,
        operation_id: &str,
        response_json: &Value,
        now_ms: u64,
    ) -> CoreResult<WorkflowOperationCompletionOutcome> {
        let now_ms = sqlite_millis(now_ms, "workflow operation completion timestamp")?;
        let response_json = serde_json::to_string(response_json)?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let status = transaction
            .query_row(
                "SELECT status FROM workflow_operations WHERE operation_id = ?1",
                params![operation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?
            .ok_or_else(|| {
                CoreError::validation(format!(
                    "workflow operation not found before response completion: {operation_id}"
                ))
            })?;
        let status = parse_operation_status(&status)?;
        let outcome = match status {
            WorkflowOperationStatus::Dispatched => {
                let changed = transaction
                    .execute(
                        "UPDATE workflow_operations
                         SET status = 'completed', updated_at_ms = ?1,
                             response_json = ?2, completed_at_ms = ?1
                         WHERE operation_id = ?3 AND status = 'dispatched'",
                        params![now_ms, response_json, operation_id],
                    )
                    .map_err(sqlite_error)?;
                if changed != 1 {
                    return Err(CoreError::validation(
                        "workflow operation changed during response completion",
                    ));
                }
                WorkflowOperationCompletionOutcome::Completed
            }
            WorkflowOperationStatus::Prepared => {
                let changed = transaction
                    .execute(
                        "UPDATE workflow_operations
                         SET status = 'in_doubt', updated_at_ms = ?1,
                             in_doubt_at_ms = ?1
                         WHERE operation_id = ?2 AND status = 'prepared'",
                        params![now_ms, operation_id],
                    )
                    .map_err(sqlite_error)?;
                if changed != 1 {
                    return Err(CoreError::validation(
                        "workflow operation changed while isolating an unfenced response",
                    ));
                }
                WorkflowOperationCompletionOutcome::DispatchAuthorizationMissing
            }
            WorkflowOperationStatus::InDoubt => {
                WorkflowOperationCompletionOutcome::DispatchAuthorizationMissing
            }
            _ => {
                return Err(CoreError::validation(format!(
                    "workflow operation cannot complete from status {}",
                    operation_status_name(status)
                )))
            }
        };
        transaction.commit().map_err(sqlite_error)?;
        Ok(outcome)
    }

    /// 在一个 IMMEDIATE 事务中复核运行控制、worker fencing 与 operation 状态。
    /// `dispatch=true` 时同时把 Prepared（或可重放的 InDoubt）推进到 Dispatched。
    pub fn authorize_operation_dispatch(
        &self,
        operation_id: &str,
        dispatch: bool,
        now_ms: u64,
    ) -> CoreResult<()> {
        let now_ms_sql = sqlite_millis(now_ms, "workflow dispatch timestamp")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let row = transaction
            .query_row(
                "
                SELECT o.workflow_id, o.run_id, o.lease_generation, o.status,
                       o.recovery_policy, r.status, r.control,
                       l.owner_id, l.generation, l.expires_at_ms
                FROM workflow_operations o
                JOIN workflow_runs r
                  ON r.workflow_id = o.workflow_id AND r.run_id = o.run_id
                LEFT JOIN workflow_run_worker_leases l
                  ON l.workflow_id = o.workflow_id AND l.run_id = o.run_id
                WHERE o.operation_id = ?1
                ",
                params![operation_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?;
        let Some((
            workflow_id,
            run_id,
            lease_generation,
            operation_status,
            recovery_policy,
            run_status,
            control,
            lease_owner,
            current_generation,
            lease_expires_at_ms,
        )) = row
        else {
            return Err(CoreError::validation(format!(
                "workflow operation not found before dispatch: {operation_id}"
            )));
        };
        let operation_status = parse_operation_status(&operation_status)?;
        let denied = || dispatch_denied_error(operation_status);
        if run_status != "running" || control != "continue" {
            return Err(denied());
        }
        if lease_generation < 0 {
            return Err(CoreError::validation(
                "workflow operation lease generation cannot be negative",
            ));
        }
        if lease_generation > 0 {
            let Some(expected_lease) = self.worker_lease.as_ref() else {
                return Err(denied());
            };
            let expected_generation = sqlite_millis(
                expected_lease.generation,
                "workflow dispatch lease generation",
            )?;
            let lease_is_current = expected_lease.workflow_id.as_str() == workflow_id
                && expected_lease.run_id.as_str() == run_id
                && expected_generation == lease_generation
                && lease_owner.as_deref() == Some(expected_lease.owner_id.as_str())
                && current_generation == Some(expected_generation)
                && lease_expires_at_ms.is_some_and(|expires| expires > now_ms_sql);
            if !lease_is_current {
                return Err(denied());
            }
        }

        if dispatch {
            let expected = match operation_status {
                WorkflowOperationStatus::Prepared => WorkflowOperationStatus::Prepared,
                WorkflowOperationStatus::InDoubt if recovery_policy == "replay_executor" => {
                    WorkflowOperationStatus::InDoubt
                }
                WorkflowOperationStatus::Dispatched => {
                    transaction.commit().map_err(sqlite_error)?;
                    return Ok(());
                }
                WorkflowOperationStatus::InDoubt => return Err(denied()),
                _ => {
                    return Err(CoreError::validation(format!(
                        "workflow operation cannot dispatch from status {}",
                        operation_status_name(operation_status)
                    )))
                }
            };
            let changed = transaction
                .execute(
                    "UPDATE workflow_operations
                     SET status = 'dispatched', updated_at_ms = ?1,
                         dispatched_at_ms = COALESCE(dispatched_at_ms, ?1)
                     WHERE operation_id = ?2 AND status = ?3",
                    params![now_ms_sql, operation_id, operation_status_name(expected)],
                )
                .map_err(sqlite_error)?;
            if changed != 1 {
                return Err(CoreError::validation(
                    "workflow operation changed during dispatch authorization",
                ));
            }
        } else if !matches!(
            operation_status,
            WorkflowOperationStatus::Prepared
                | WorkflowOperationStatus::Dispatched
                | WorkflowOperationStatus::InDoubt
        ) {
            return Err(CoreError::validation(format!(
                "workflow operation cannot execute from status {}",
                operation_status_name(operation_status)
            )));
        }
        transaction.commit().map_err(sqlite_error)
    }

    /// 原子请求 Stop。活跃 worker 仍持有 lease 时进入 Stopping；否则直接 Stopped。
    pub fn request_stop(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        reason: &str,
        now_ms: u64,
    ) -> CoreResult<WorkflowStopRequestResult> {
        let now_ms_sql = sqlite_millis(now_ms, "workflow stop timestamp")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let Some(mut state) = load_state_for_mutation(&transaction, workflow_id, run_id)? else {
            return Ok(WorkflowStopRequestResult::NotFound);
        };
        if state.status.is_terminal() {
            return Ok(WorkflowStopRequestResult::Saved { state });
        }
        let has_live_worker =
            load_live_worker_lease(&transaction, workflow_id, run_id, now_ms_sql)?.is_some();
        if state.status == RunStatus::Stopping && has_live_worker {
            return Ok(WorkflowStopRequestResult::Saved { state });
        }
        let was_stopping = state.status == RunStatus::Stopping;
        state.control = RunControl::Stop;
        state.stop_reason = Some(reason.to_owned());
        state.pause_reason = None;
        state.next_retry_at_ms = None;
        state.status = if has_live_worker {
            RunStatus::Stopping
        } else {
            RunStatus::Stopped
        };
        let event_type = if has_live_worker {
            crate::workflow::WorkflowRuntimeEventType::RunStopRequested
        } else {
            crate::workflow::WorkflowRuntimeEventType::RunStopped
        };
        let message = if has_live_worker {
            format!("run stop requested: {reason}")
        } else {
            reason.to_owned()
        };
        if !was_stopping || !has_live_worker {
            state.events.push(message.clone());
        }
        let sequence = state.next_event_sequence;
        if !was_stopping || !has_live_worker {
            state.next_event_sequence = state.next_event_sequence.saturating_add(1);
            state
                .structured_events
                .push(crate::workflow::WorkflowRuntimeEvent {
                    sequence,
                    event_type,
                    node_id: None,
                    message,
                    metadata: Value::Null,
                });
        }
        persist_mutated_state(
            &transaction,
            &mut state,
            now_ms_sql,
            "workflow state changed during atomic stop request",
        )?;
        if state.status == RunStatus::Stopped {
            transaction
                .execute(
                    "UPDATE workflow_run_worker_leases
                     SET owner_id = NULL, heartbeat_at_ms = ?1, expires_at_ms = 0
                     WHERE workflow_id = ?2 AND run_id = ?3",
                    params![now_ms_sql, workflow_id.as_str(), run_id.as_str()],
                )
                .map_err(sqlite_error)?;
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(WorkflowStopRequestResult::Saved { state })
    }

    /// F14：在任何 knowledge 副作用前持久化确认决策意图。
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_confirmation_resolution(
        &self,
        operation_id: &str,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        confirmation_id: &str,
        decision: ConfirmationResolutionDecision,
        review_reason: Option<&str>,
        request_hash: &str,
        knowledge_required: bool,
        now_ms: u64,
    ) -> CoreResult<ConfirmationResolutionOperation> {
        validate_non_empty_operation_field("operation_id", operation_id)?;
        validate_non_empty_operation_field("confirmation_id", confirmation_id)?;
        validate_non_empty_operation_field("request_hash", request_hash)?;
        let now_ms = sqlite_millis(now_ms, "confirmation resolution timestamp")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        if let Some(existing) = load_confirmation_resolution_by_identity(
            &transaction,
            workflow_id,
            run_id,
            confirmation_id,
        )? {
            if existing.operation_id != operation_id
                || existing.request_hash != request_hash
                || existing.decision != decision
                || existing.review_reason.as_deref() != review_reason
            {
                return Err(CoreError::validation(format!(
                    "confirmation resolution identity was reused with different input: {confirmation_id}"
                )));
            }
            transaction.commit().map_err(sqlite_error)?;
            return Ok(existing);
        }
        let state =
            load_state_for_mutation(&transaction, workflow_id, run_id)?.ok_or_else(|| {
                CoreError::validation(format!(
                    "workflow run not found: {}/{}",
                    workflow_id.as_str(),
                    run_id.as_str()
                ))
            })?;
        let confirmation = state.confirmations.get(confirmation_id).ok_or_else(|| {
            CoreError::validation(format!("confirmation item not found: {confirmation_id}"))
        })?;
        if confirmation.state != crate::workflow::RuntimeConfirmationState::Pending {
            return Err(CoreError::validation(format!(
                "confirmation resolution requires pending state: {confirmation_id}"
            )));
        }
        if let Some(lease) = load_live_worker_lease(&transaction, workflow_id, run_id, now_ms)? {
            return Err(CoreError::validation(format!(
                "workflow run is busy during confirmation resolution: {}",
                lease.owner_id
            )));
        }
        let status = if knowledge_required {
            ConfirmationResolutionStatus::Prepared
        } else {
            ConfirmationResolutionStatus::KnowledgeCommitted
        };
        transaction
            .execute(
                "INSERT INTO confirmation_resolution_operations(
                    operation_id, workflow_id, run_id, confirmation_id, decision,
                    review_reason, request_hash, knowledge_required, status,
                    created_at_ms, updated_at_ms, projected_at_ms
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,NULL)",
                params![
                    operation_id,
                    workflow_id.as_str(),
                    run_id.as_str(),
                    confirmation_id,
                    confirmation_resolution_decision_name(decision),
                    review_reason,
                    request_hash,
                    knowledge_required,
                    confirmation_resolution_status_name(status),
                    now_ms,
                ],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(ConfirmationResolutionOperation {
            operation_id: operation_id.to_owned(),
            workflow_id: workflow_id.clone(),
            run_id: run_id.clone(),
            confirmation_id: confirmation_id.to_owned(),
            decision,
            review_reason: review_reason.map(str::to_owned),
            request_hash: request_hash.to_owned(),
            knowledge_required,
            status,
            projected: false,
        })
    }

    /// F14：knowledge receipt 已与实体变更同事务提交后，推进 saga。
    pub fn mark_confirmation_knowledge_committed(
        &self,
        operation_id: &str,
        request_hash: &str,
        now_ms: u64,
    ) -> CoreResult<ConfirmationResolutionOperation> {
        let now_ms = sqlite_millis(now_ms, "confirmation knowledge timestamp")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let operation =
            load_confirmation_resolution(&transaction, operation_id)?.ok_or_else(|| {
                CoreError::validation(format!(
                    "confirmation resolution operation not found: {operation_id}"
                ))
            })?;
        if operation.request_hash != request_hash {
            return Err(CoreError::validation(
                "confirmation resolution request hash mismatch",
            ));
        }
        if operation.status == ConfirmationResolutionStatus::Prepared {
            transaction
                .execute(
                    "UPDATE confirmation_resolution_operations
                     SET status='knowledge_committed', updated_at_ms=?1
                     WHERE operation_id=?2 AND status='prepared'",
                    params![now_ms, operation_id],
                )
                .map_err(sqlite_error)?;
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(ConfirmationResolutionOperation {
            status: if operation.status == ConfirmationResolutionStatus::Prepared {
                ConfirmationResolutionStatus::KnowledgeCommitted
            } else {
                operation.status
            },
            ..operation
        })
    }

    /// F14：runtime 决策、事件、状态与 saga committed 同事务提交。
    /// worker 由统一 claim_resume 在提交后领取；进程在两步间退出时 open 可立即接管。
    pub fn commit_confirmation_resolution(
        &self,
        operation_id: &str,
        now_ms: u64,
    ) -> CoreResult<ConfirmationResolutionCommitResult> {
        let now_ms_sql = sqlite_millis(now_ms, "confirmation runtime timestamp")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let operation =
            load_confirmation_resolution(&transaction, operation_id)?.ok_or_else(|| {
                CoreError::validation(format!(
                    "confirmation resolution operation not found: {operation_id}"
                ))
            })?;
        if operation.status == ConfirmationResolutionStatus::Prepared {
            return Err(CoreError::validation(
                "confirmation knowledge commit is not durable yet",
            ));
        }
        let Some(mut state) =
            load_state_for_mutation(&transaction, &operation.workflow_id, &operation.run_id)?
        else {
            return Ok(ConfirmationResolutionCommitResult::NotFound);
        };
        if operation.status == ConfirmationResolutionStatus::Committed {
            transaction.commit().map_err(sqlite_error)?;
            return Ok(ConfirmationResolutionCommitResult::AlreadyCommitted { state });
        }
        let mut runtime = crate::workflow::WorkflowRuntime::from_state(state);
        if let Some(reason) = operation
            .review_reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
        {
            let confirmation = runtime
                .state
                .confirmations
                .get_mut(&operation.confirmation_id)
                .ok_or_else(|| {
                    CoreError::validation(format!(
                        "confirmation item not found: {}",
                        operation.confirmation_id
                    ))
                })?;
            if !confirmation.metadata.is_object() {
                confirmation.metadata = serde_json::json!({});
            }
            confirmation
                .metadata
                .as_object_mut()
                .expect("metadata normalized to object")
                .insert("reason".to_owned(), Value::String(reason.to_owned()));
        }
        runtime.update_confirmation_state(
            &operation.confirmation_id,
            match operation.decision {
                ConfirmationResolutionDecision::Approve => {
                    crate::workflow::RuntimeConfirmationState::Approved
                }
                ConfirmationResolutionDecision::Reject => {
                    crate::workflow::RuntimeConfirmationState::Rejected
                }
            },
        )?;
        state = runtime.state;
        persist_mutated_state(
            &transaction,
            &mut state,
            now_ms_sql,
            "workflow state changed during confirmation resolution",
        )?;
        let changed = transaction
            .execute(
                "UPDATE confirmation_resolution_operations
                 SET status='committed', updated_at_ms=?1
                 WHERE operation_id=?2 AND status='knowledge_committed'",
                params![now_ms_sql, operation_id],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(
                "confirmation resolution changed during runtime commit",
            ));
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(ConfirmationResolutionCommitResult::Saved { state })
    }

    /// F14-b：投影是可重建派生数据；成功后仅标记 outbox 已消费。
    pub fn mark_confirmation_resolution_projected(
        &self,
        operation_id: &str,
        now_ms: u64,
    ) -> CoreResult<()> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let changed = connection
            .execute(
                "UPDATE confirmation_resolution_operations
                 SET projected_at_ms=COALESCE(projected_at_ms, ?1), updated_at_ms=?1
                 WHERE operation_id=?2 AND status='committed'",
                params![
                    sqlite_millis(now_ms, "confirmation projection timestamp")?,
                    operation_id
                ],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(
                "confirmation resolution is not committed for projection",
            ));
        }
        Ok(())
    }

    /// F14：列出未完成 saga 与未消费投影，供 open/retry 前向恢复。
    pub fn list_recoverable_confirmation_resolutions(
        &self,
    ) -> CoreResult<Vec<ConfirmationResolutionOperation>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "SELECT operation_id, workflow_id, run_id, confirmation_id, decision,
                        review_reason, request_hash, knowledge_required, status,
                        projected_at_ms
                 FROM confirmation_resolution_operations
                 WHERE status != 'committed' OR projected_at_ms IS NULL
                 ORDER BY created_at_ms, operation_id",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], read_confirmation_resolution)
            .map_err(sqlite_error)?;
        let mut operations = Vec::new();
        for row in rows {
            operations.push(parse_confirmation_resolution(row.map_err(sqlite_error)?)?);
        }
        Ok(operations)
    }

    /// 检查 runtime.db 是否可打开、可迁移、可查询。
    pub fn health(project_root: impl AsRef<Path>) -> RuntimeStoreHealth {
        let db_path = project_root.as_ref().join(RUNTIME_DB_FILE);
        if !db_path.exists() {
            return RuntimeStoreHealth::Missing;
        }
        // health 走真实 open+migrate+schema 查询路径。这样能同时发现文件损坏、
        // SQLite 打不开和 schema_migrations 异常。
        match Self::open(project_root) {
            Ok(store) => match store.schema_version() {
                Ok(Some(_)) => RuntimeStoreHealth::Healthy,
                Ok(None) => RuntimeStoreHealth::Corrupt {
                    message: "workflow runtime schema version is missing".to_owned(),
                },
                Err(error) => RuntimeStoreHealth::Corrupt {
                    message: error.to_string(),
                },
            },
            Err(error) => RuntimeStoreHealth::Corrupt {
                message: error.to_string(),
            },
        }
    }

    /// 原子获取单个 run 的唯一 worker lease。
    ///
    /// 未过期 lease 不允许覆盖；到期后由单条 SQLite UPSERT 完成接管，避免
    /// 多进程之间出现先查询、后写入的竞态。
    pub fn acquire_worker_lease(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        owner_id: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> CoreResult<Option<WorkflowWorkerLease>> {
        validate_lease_owner(owner_id)?;
        let expires_at_ms = lease_expiry(now_ms, ttl_ms)?;
        let now_ms_sql = sqlite_millis(now_ms, "worker lease timestamp")?;
        let expires_at_ms_sql = sqlite_millis(expires_at_ms, "worker lease expiry")?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let changed = connection
            .query_row(
                "
                INSERT INTO workflow_run_worker_leases(
                    workflow_id, run_id, owner_id, generation,
                    acquired_at_ms, heartbeat_at_ms, expires_at_ms
                )
                SELECT ?1, ?2, ?3, 1, ?4, ?4, ?5
                FROM workflow_runs
                WHERE workflow_id = ?1 AND run_id = ?2
                  AND status IN ('queued', 'running')
                  AND NOT EXISTS (
                      SELECT 1 FROM confirmation_resolution_operations
                      WHERE workflow_id = ?1 AND run_id = ?2
                        AND status != 'committed'
                  )
                ON CONFLICT(workflow_id, run_id) DO UPDATE SET
                    owner_id = excluded.owner_id,
                    generation = workflow_run_worker_leases.generation + 1,
                    acquired_at_ms = excluded.acquired_at_ms,
                    heartbeat_at_ms = excluded.heartbeat_at_ms,
                    expires_at_ms = excluded.expires_at_ms
                WHERE workflow_run_worker_leases.expires_at_ms <= excluded.acquired_at_ms
                  AND workflow_run_worker_leases.generation < 9223372036854775807
                RETURNING owner_id, generation, acquired_at_ms,
                          heartbeat_at_ms, expires_at_ms
                ",
                params![
                    workflow_id.as_str(),
                    run_id.as_str(),
                    owner_id,
                    now_ms_sql,
                    expires_at_ms_sql
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?;
        let Some((owner_id, generation, acquired_at_ms, heartbeat_at_ms, expires_at_ms)) = changed
        else {
            return Ok(None);
        };
        Ok(Some(WorkflowWorkerLease {
            workflow_id: workflow_id.clone(),
            run_id: run_id.clone(),
            owner_id,
            generation: sqlite_u64(generation, "worker lease generation")?,
            acquired_at_ms: sqlite_u64(acquired_at_ms, "worker lease acquired_at_ms")?,
            heartbeat_at_ms: sqlite_u64(heartbeat_at_ms, "worker lease heartbeat_at_ms")?,
            expires_at_ms: sqlite_u64(expires_at_ms, "worker lease expires_at_ms")?,
        }))
    }

    /// F9：获取或续租项目级 runnable 调度器 lease。
    ///
    /// 同一 runtime.db 只有一个未过期 owner 能扫描并领取到期任务；进程崩溃后
    /// lease 到期即可由另一实例接管。最终重复防护仍由每个 run 的 worker lease
    /// 与 generation fencing 提供。
    pub fn acquire_scheduler_lease(
        &self,
        owner_id: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> CoreResult<bool> {
        validate_scheduler_owner(owner_id)?;
        let expires_at_ms = lease_expiry(now_ms, ttl_ms)?;
        let now_ms_sql = sqlite_millis(now_ms, "workflow scheduler lease timestamp")?;
        let expires_at_ms_sql = sqlite_millis(expires_at_ms, "workflow scheduler lease expiry")?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let owner = connection
            .query_row(
                "
                INSERT INTO workflow_scheduler_leases(
                    scheduler_key, owner_id, generation,
                    acquired_at_ms, heartbeat_at_ms, expires_at_ms
                ) VALUES('workflow-runnable', ?1, 1, ?2, ?2, ?3)
                ON CONFLICT(scheduler_key) DO UPDATE SET
                    owner_id = CASE
                        WHEN workflow_scheduler_leases.owner_id = excluded.owner_id
                        THEN workflow_scheduler_leases.owner_id
                        ELSE excluded.owner_id
                    END,
                    generation = CASE
                        WHEN workflow_scheduler_leases.owner_id = excluded.owner_id
                        THEN workflow_scheduler_leases.generation
                        ELSE workflow_scheduler_leases.generation + 1
                    END,
                    acquired_at_ms = CASE
                        WHEN workflow_scheduler_leases.owner_id = excluded.owner_id
                        THEN workflow_scheduler_leases.acquired_at_ms
                        ELSE excluded.acquired_at_ms
                    END,
                    heartbeat_at_ms = excluded.heartbeat_at_ms,
                    expires_at_ms = excluded.expires_at_ms
                WHERE workflow_scheduler_leases.owner_id = excluded.owner_id
                   OR workflow_scheduler_leases.expires_at_ms <= excluded.acquired_at_ms
                RETURNING owner_id
                ",
                params![owner_id, now_ms_sql, expires_at_ms_sql],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        Ok(owner.as_deref() == Some(owner_id))
    }

    /// 当前 owner 主动退出时立即让出项目级调度器租约。
    pub fn release_scheduler_lease(&self, owner_id: &str) -> CoreResult<bool> {
        validate_scheduler_owner(owner_id)?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let changed = connection
            .execute(
                "DELETE FROM workflow_scheduler_leases
                 WHERE scheduler_key = 'workflow-runnable' AND owner_id = ?1",
                params![owner_id],
            )
            .map_err(sqlite_error)?;
        Ok(changed == 1)
    }

    /// F9：在一个 IMMEDIATE 事务内选择并领取一条当前可运行任务。
    ///
    /// next_retry_at_ms 为 NULL 表示初始 Start handoff 或崩溃恢复，可立即领取；
    /// 有时间的 Queued 任务必须到期。Running orphan 同样可接管，Paused/Stopping
    /// 不从此入口恢复。候选若被另一实例先领取，事务内继续尝试后续候选。
    pub fn claim_next_runnable(
        &self,
        scheduler_owner_id: &str,
        worker_owner_id: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> CoreResult<WorkflowRunnableClaimResult> {
        validate_scheduler_owner(scheduler_owner_id)?;
        validate_lease_owner(worker_owner_id)?;
        let expires_at_ms = lease_expiry(now_ms, ttl_ms)?;
        let now_ms_sql = sqlite_millis(now_ms, "workflow runnable claim timestamp")?;
        let expires_at_ms_sql = sqlite_millis(expires_at_ms, "workflow runnable lease expiry")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let mut statement = transaction
            .prepare(
                "
                SELECT r.workflow_id, r.run_id
                FROM workflow_runs r
                LEFT JOIN workflow_run_worker_leases l
                  ON l.workflow_id = r.workflow_id AND l.run_id = r.run_id
                WHERE (
                    (
                      r.control = 'continue'
                      AND (
                        r.status = 'running'
                        OR (
                          r.status = 'queued'
                          AND (r.next_retry_at_ms IS NULL OR r.next_retry_at_ms <= ?1)
                        )
                      )
                    )
                    OR (r.status = 'stopping' AND r.control = 'stop')
                  )
                  AND (
                    l.workflow_id IS NULL
                    OR l.owner_id IS NULL
                    OR l.expires_at_ms <= ?1
                  )
                  AND NOT EXISTS (
                    SELECT 1 FROM confirmation_resolution_operations c
                    WHERE c.workflow_id = r.workflow_id AND c.run_id = r.run_id
                      AND c.status != 'committed'
                  )
                  AND EXISTS (
                    SELECT 1 FROM workflow_scheduler_leases s
                    WHERE s.scheduler_key = 'workflow-runnable'
                      AND s.owner_id = ?2
                      AND s.expires_at_ms > ?1
                  )
                ORDER BY
                  CASE WHEN r.status = 'running' THEN 0 ELSE 1 END,
                  COALESCE(r.next_retry_at_ms, r.updated_at_ms),
                  r.updated_at_ms
                ",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map(params![now_ms_sql, scheduler_owner_id], |row| {
                Ok((
                    WorkflowId::from(row.get::<_, String>(0)?),
                    RunId::from(row.get::<_, String>(1)?),
                ))
            })
            .map_err(sqlite_error)?;
        let mut candidates = Vec::new();
        for row in rows {
            candidates.push(row.map_err(sqlite_error)?);
        }
        drop(statement);

        for (workflow_id, run_id) in candidates {
            let Some(mut state) = load_state_for_mutation(&transaction, &workflow_id, &run_id)?
            else {
                continue;
            };
            if state.status == RunStatus::Stopping {
                let reason = state
                    .stop_reason
                    .clone()
                    .unwrap_or_else(|| "stopped after worker lease expired".to_owned());
                state.status = RunStatus::Stopped;
                state.control = RunControl::Stop;
                state.pause_reason = None;
                state.next_retry_at_ms = None;
                state.events.push(reason.clone());
                let sequence = state.next_event_sequence;
                state.next_event_sequence = state.next_event_sequence.saturating_add(1);
                state
                    .structured_events
                    .push(crate::workflow::WorkflowRuntimeEvent {
                        sequence,
                        event_type: crate::workflow::WorkflowRuntimeEventType::RunStopped,
                        node_id: None,
                        message: reason,
                        metadata: Value::Null,
                    });
                persist_mutated_state(
                    &transaction,
                    &mut state,
                    now_ms_sql,
                    "workflow state changed during scheduler stop convergence",
                )?;
                transaction
                    .execute(
                        "DELETE FROM workflow_run_worker_leases
                         WHERE workflow_id = ?1 AND run_id = ?2",
                        params![workflow_id.as_str(), run_id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                transaction.commit().map_err(sqlite_error)?;
                return Ok(WorkflowRunnableClaimResult::Stopped { state });
            }
            if state.has_pending_confirmations() {
                continue;
            }
            let lease = claim_worker_lease(
                &transaction,
                &workflow_id,
                &run_id,
                worker_owner_id,
                now_ms_sql,
                expires_at_ms_sql,
            )?;
            transaction.commit().map_err(sqlite_error)?;
            return Ok(WorkflowRunnableClaimResult::Claimed { state, lease });
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(WorkflowRunnableClaimResult::Empty)
    }

    /// 仅允许当前 owner 续租；已过期 lease 不可被心跳复活，必须重新 acquire。
    pub fn heartbeat_worker_lease(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        owner_id: &str,
        generation: u64,
        now_ms: u64,
        ttl_ms: u64,
    ) -> CoreResult<bool> {
        validate_lease_owner(owner_id)?;
        let expires_at_ms = lease_expiry(now_ms, ttl_ms)?;
        let generation = sqlite_millis(generation, "worker lease generation")?;
        let now_ms_sql = sqlite_millis(now_ms, "worker lease heartbeat")?;
        let expires_at_ms_sql = sqlite_millis(expires_at_ms, "worker lease expiry")?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let changed = connection
            .execute(
                "
                UPDATE workflow_run_worker_leases
                SET heartbeat_at_ms = ?1, expires_at_ms = ?2
                WHERE workflow_id = ?3 AND run_id = ?4 AND owner_id = ?5
                  AND generation = ?6
                  AND expires_at_ms > ?1
                ",
                params![
                    now_ms_sql,
                    expires_at_ms_sql,
                    workflow_id.as_str(),
                    run_id.as_str(),
                    owner_id,
                    generation,
                ],
            )
            .map_err(sqlite_error)?;
        Ok(changed == 1)
    }

    /// 判断当前 worker 是否必须停止产生新副作用。
    ///
    /// 该查询只读取索引状态与 lease 行，不反序列化运行快照。Pause/Stop/终态、
    /// owner 接管、generation 变化和 lease 过期统一映射为 cancellation。
    pub fn execution_should_cancel(
        &self,
        lease: &WorkflowWorkerLease,
        now_ms: u64,
    ) -> CoreResult<bool> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let row = connection
            .query_row(
                "
                SELECT r.status, r.control, l.owner_id, l.generation, l.expires_at_ms
                FROM workflow_runs r
                LEFT JOIN workflow_run_worker_leases l
                  ON l.workflow_id = r.workflow_id AND l.run_id = r.run_id
                WHERE r.workflow_id = ?1 AND r.run_id = ?2
                ",
                params![lease.workflow_id.as_str(), lease.run_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?;
        let Some((status, control, owner_id, generation, expires_at_ms)) = row else {
            return Ok(true);
        };
        let lease_current = owner_id.as_deref() == Some(lease.owner_id.as_str())
            && generation == Some(sqlite_millis(lease.generation, "worker lease generation")?)
            && expires_at_ms
                .is_some_and(|expires| expires > i64::try_from(now_ms).unwrap_or(i64::MAX));
        Ok(!lease_current
            || control != "continue"
            || matches!(
                status.as_str(),
                "paused" | "stopping" | "stopped" | "failed" | "succeeded"
            ))
    }

    /// 仅当前 owner+generation 可以释放 lease；保留 generation tombstone 防止 fencing token 回退。
    pub fn release_worker_lease(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        owner_id: &str,
        generation: u64,
    ) -> CoreResult<bool> {
        validate_lease_owner(owner_id)?;
        let generation = sqlite_millis(generation, "worker lease generation")?;
        let released_at_ms = unix_timestamp_ms_i64()?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let changed = connection
            .execute(
                "
                UPDATE workflow_run_worker_leases
                SET owner_id = NULL, heartbeat_at_ms = ?1, expires_at_ms = 0
                WHERE workflow_id = ?2 AND run_id = ?3 AND owner_id = ?4
                  AND generation = ?5
                ",
                params![
                    released_at_ms,
                    workflow_id.as_str(),
                    run_id.as_str(),
                    owner_id,
                    generation,
                ],
            )
            .map_err(sqlite_error)?;
        Ok(changed == 1)
    }

    /// 读取当前 lease，供命令层诊断和 fencing 测试使用。
    pub fn load_worker_lease(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
    ) -> CoreResult<Option<WorkflowWorkerLease>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let lease = connection
            .query_row(
                "
                SELECT owner_id, generation, acquired_at_ms,
                       heartbeat_at_ms, expires_at_ms
                FROM workflow_run_worker_leases
                WHERE workflow_id = ?1 AND run_id = ?2
                  AND owner_id IS NOT NULL
                ",
                params![workflow_id.as_str(), run_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?;
        let Some((owner_id, generation, acquired_at_ms, heartbeat_at_ms, expires_at_ms)) = lease
        else {
            return Ok(None);
        };
        Ok(Some(WorkflowWorkerLease {
            workflow_id: workflow_id.clone(),
            run_id: run_id.clone(),
            owner_id,
            generation: sqlite_u64(generation, "worker lease generation")?,
            acquired_at_ms: sqlite_u64(acquired_at_ms, "worker lease acquired_at_ms")?,
            heartbeat_at_ms: sqlite_u64(heartbeat_at_ms, "worker lease heartbeat_at_ms")?,
            expires_at_ms: sqlite_u64(expires_at_ms, "worker lease expires_at_ms")?,
        }))
    }

    /// 在同一个 IMMEDIATE 事务中加载最新快照、执行变更并按结果状态决定是否 claim worker。
    ///
    /// 闭包仅修改事务内刚加载的状态。若变更后为 Queued/Running，则必须同时取得
    /// worker lease 才会提交；已有活跃 lease 时返回 `Busy`，快照、事件与 lease
    /// 均保持原样。其他状态直接提交快照，并返回 `lease: None`。
    pub fn mutate_state_and_claim<F>(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        owner_id: &str,
        now_ms: u64,
        ttl_ms: u64,
        mutate: F,
    ) -> CoreResult<WorkflowMutationClaimResult>
    where
        F: FnOnce(&mut WorkflowRunState) -> CoreResult<()>,
    {
        let now_ms_sql = sqlite_millis(now_ms, "workflow mutation timestamp")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let Some(mut state) = load_state_for_mutation(&transaction, workflow_id, run_id)? else {
            return Ok(WorkflowMutationClaimResult::NotFound);
        };
        let loaded_revision = state.state_revision;

        mutate(&mut state)?;
        if state.workflow_id != *workflow_id || state.run_id != *run_id {
            return Err(CoreError::validation(
                "workflow mutation cannot change workflow_id or run_id",
            ));
        }
        if state.state_revision != loaded_revision {
            return Err(CoreError::validation(
                "workflow mutation cannot change state_revision",
            ));
        }

        let lease = if matches!(state.status, RunStatus::Queued | RunStatus::Running) {
            validate_lease_owner(owner_id)?;
            let expires_at_ms = lease_expiry(now_ms, ttl_ms)?;
            let expires_at_ms_sql = sqlite_millis(expires_at_ms, "worker lease expiry")?;
            if let Some(lease) =
                load_live_worker_lease(&transaction, workflow_id, run_id, now_ms_sql)?
            {
                return Ok(WorkflowMutationClaimResult::Busy { lease });
            }
            Some(claim_worker_lease(
                &transaction,
                workflow_id,
                run_id,
                owner_id,
                now_ms_sql,
                expires_at_ms_sql,
            )?)
        } else {
            None
        };

        persist_mutated_state(
            &transaction,
            &mut state,
            now_ms_sql,
            "workflow state changed during atomic mutation claim",
        )?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(WorkflowMutationClaimResult::Saved { state, lease })
    }

    /// 原子处理 in_doubt operation，并同步运行状态、事件、revision 与可选 lease。
    pub fn resolve_in_doubt_operation(
        &self,
        operation_id: &str,
        resolution: InDoubtResolution,
        owner_id: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> CoreResult<InDoubtResolutionResult> {
        let now_ms_sql = sqlite_millis(now_ms, "in_doubt resolution timestamp")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let operation = transaction
            .query_row(
                "SELECT operation_id, workflow_id, run_id, node_id, attempt, kind,
                        provider, request_hash, lease_generation, recovery_policy,
                        response_policy, status, response_json,
                        created_at_ms, updated_at_ms, dispatched_at_ms, completed_at_ms,
                        in_doubt_at_ms, committed_at_ms
                 FROM workflow_operations WHERE operation_id = ?1",
                params![operation_id],
                read_operation,
            )
            .optional()
            .map_err(sqlite_error)?
            .map(parse_operation)
            .transpose()?;
        let Some(operation) = operation else {
            return Ok(InDoubtResolutionResult::NotFound);
        };
        if operation.status != WorkflowOperationStatus::InDoubt {
            return Ok(InDoubtResolutionResult::NotInDoubt {
                status: Some(operation.status),
            });
        }
        if matches!(&resolution, InDoubtResolution::UseResponse { .. })
            && operation.response_policy == WorkflowOperationResponsePolicy::RequireExecutorReceipt
        {
            return Err(CoreError::validation(
                "workflow operation requires an executor receipt; external response is not allowed",
            ));
        }
        let Some(mut state) =
            load_state_for_mutation(&transaction, &operation.workflow_id, &operation.run_id)?
        else {
            return Ok(InDoubtResolutionResult::NotFound);
        };
        if state.status.is_terminal() && !matches!(&resolution, InDoubtResolution::Stop { .. }) {
            return Err(CoreError::validation(
                "terminal workflow run cannot retry or consume an in_doubt response",
            ));
        }
        let requires_worker = !matches!(&resolution, InDoubtResolution::Stop { .. });
        let lease = if requires_worker {
            validate_lease_owner(owner_id)?;
            if let Some(lease) = load_live_worker_lease(
                &transaction,
                &operation.workflow_id,
                &operation.run_id,
                now_ms_sql,
            )? {
                return Ok(InDoubtResolutionResult::Busy { lease });
            }
            Some(claim_worker_lease(
                &transaction,
                &operation.workflow_id,
                &operation.run_id,
                owner_id,
                now_ms_sql,
                sqlite_millis(lease_expiry(now_ms, ttl_ms)?, "worker lease expiry")?,
            )?)
        } else {
            None
        };

        let (next_operation_status, response_json, event_type, message) = match resolution {
            InDoubtResolution::Retry => {
                state
                    .node_operation_sequences
                    .entry(operation.node_id.clone())
                    .and_modify(|sequence| *sequence = (*sequence).max(operation.attempt))
                    .or_insert(operation.attempt);
                if let Some(node) = state.nodes.get_mut(&operation.node_id) {
                    node.status = RunStatus::Queued;
                    node.error = None;
                    node.error_state = None;
                }
                state.status = RunStatus::Queued;
                state.control = crate::contracts::RunControl::Continue;
                state.pause_reason = None;
                state.next_retry_at_ms = None;
                (
                    WorkflowOperationStatus::Aborted,
                    None,
                    crate::workflow::WorkflowRuntimeEventType::RunQueued,
                    format!("in_doubt operation {operation_id} approved for retry"),
                )
            }
            InDoubtResolution::UseResponse { response } => {
                let _: crate::workflow::WorkflowNodeExecutionOutput =
                    serde_json::from_value(response.clone())?;
                state.status = RunStatus::Queued;
                state.control = crate::contracts::RunControl::Continue;
                state.pause_reason = None;
                state.next_retry_at_ms = None;
                (
                    WorkflowOperationStatus::Completed,
                    Some(response),
                    crate::workflow::WorkflowRuntimeEventType::RunQueued,
                    format!("in_doubt operation {operation_id} supplied a response"),
                )
            }
            InDoubtResolution::Stop { reason } => {
                state.status = RunStatus::Stopped;
                state.control = crate::contracts::RunControl::Stop;
                state.stop_reason = Some(reason.clone());
                state.pause_reason = None;
                state.next_retry_at_ms = None;
                (
                    WorkflowOperationStatus::Aborted,
                    None,
                    crate::workflow::WorkflowRuntimeEventType::RunStopped,
                    reason,
                )
            }
        };
        let sequence = state.next_event_sequence;
        state.next_event_sequence = state.next_event_sequence.saturating_add(1);
        state
            .structured_events
            .push(crate::workflow::WorkflowRuntimeEvent {
                sequence,
                event_type,
                node_id: Some(operation.node_id.clone()),
                message,
                metadata: serde_json::json!({
                    "operation_id": operation_id,
                    "resolution": operation_status_name(next_operation_status),
                }),
            });
        persist_mutated_state(
            &transaction,
            &mut state,
            now_ms_sql,
            "workflow state changed during in_doubt resolution",
        )?;
        let changed = transaction
            .execute(
                "UPDATE workflow_operations
                 SET status = ?1, response_json = COALESCE(?2, response_json),
                     updated_at_ms = ?3,
                     completed_at_ms = CASE WHEN ?1 = 'completed' THEN ?3 ELSE completed_at_ms END
                 WHERE operation_id = ?4 AND status = 'in_doubt'",
                params![
                    operation_status_name(next_operation_status),
                    response_json
                        .map(|value| serde_json::to_string(&value))
                        .transpose()?,
                    now_ms_sql,
                    operation_id,
                ],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(
                "workflow operation changed during in_doubt resolution",
            ));
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(InDoubtResolutionResult::Saved { state, lease })
    }

    /// 在同一个 IMMEDIATE 事务中恢复运行并获取 worker lease。
    ///
    /// 活跃 lease、终态或 Stopping 状态均不会修改快照；到期 lease 可由新
    /// owner 接管。成功时 Resume 状态迁移、revision、事件表与 lease 一起提交。
    pub fn claim_resume(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        owner_id: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> CoreResult<WorkflowResumeClaimResult> {
        validate_lease_owner(owner_id)?;
        let expires_at_ms = lease_expiry(now_ms, ttl_ms)?;
        let now_ms_sql = sqlite_millis(now_ms, "worker lease timestamp")?;
        let expires_at_ms_sql = sqlite_millis(expires_at_ms, "worker lease expiry")?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let Some(mut state) = load_state_for_mutation(&transaction, workflow_id, run_id)? else {
            return Ok(WorkflowResumeClaimResult::NotFound);
        };
        if !matches!(
            state.status,
            RunStatus::Paused | RunStatus::Queued | RunStatus::Running
        ) {
            return Ok(WorkflowResumeClaimResult::NotResumable {
                status: state.status,
            });
        }
        let confirmation_resolution_pending = transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM confirmation_resolution_operations
                     WHERE workflow_id=?1 AND run_id=?2 AND status != 'committed'
                 )",
                params![workflow_id.as_str(), run_id.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(sqlite_error)?;
        if confirmation_resolution_pending {
            return Err(CoreError::validation(
                "workflow confirmation resolution is still committing",
            ));
        }
        if state.has_pending_confirmations() {
            return Ok(WorkflowResumeClaimResult::NotResumable {
                status: state.status,
            });
        }

        if let Some(lease) = load_live_worker_lease(&transaction, workflow_id, run_id, now_ms_sql)?
        {
            return Ok(WorkflowResumeClaimResult::Busy { lease });
        }

        let lease = claim_worker_lease(
            &transaction,
            workflow_id,
            run_id,
            owner_id,
            now_ms_sql,
            expires_at_ms_sql,
        )?;

        state.control = crate::contracts::RunControl::Continue;
        state.pause_reason = None;
        state.next_retry_at_ms = None;
        if state.status == RunStatus::Paused {
            state.status = RunStatus::Queued;
        }
        persist_mutated_state(
            &transaction,
            &mut state,
            now_ms_sql,
            "workflow state changed during atomic resume claim",
        )?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(WorkflowResumeClaimResult::Claimed { state, lease })
    }
}

fn load_state_for_mutation(
    transaction: &rusqlite::Transaction<'_>,
    workflow_id: &WorkflowId,
    run_id: &RunId,
) -> CoreResult<Option<WorkflowRunState>> {
    let snapshot = transaction
        .query_row(
            "SELECT state_revision, state_json, prepared_workflow_json FROM workflow_runs
             WHERE workflow_id = ?1 AND run_id = ?2",
            params![workflow_id.as_str(), run_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some((revision, state_json, prepared_workflow_json)) = snapshot else {
        return Ok(None);
    };
    let mut state = serde_json::from_str::<WorkflowRunState>(&state_json)?;
    state.state_revision = sqlite_u64(revision, "workflow state revision")?;
    rehydrate_prepared_workflow(&mut state, prepared_workflow_json.as_deref())?;
    state.structured_events = load_structured_events(transaction, workflow_id, run_id)?;
    state.events = load_legacy_events(transaction, workflow_id, run_id)?;
    Ok(Some(state))
}

/// C10：把 state_json 中的 prepared_workflow 瘦身；独立列在 create 时已固化。
fn serialize_state_json_for_persist(
    state: &WorkflowRunState,
    slim_prepared_workflow: bool,
) -> CoreResult<String> {
    if slim_prepared_workflow {
        let mut slim = state.clone();
        slim.prepared_workflow = None;
        Ok(serde_json::to_string(&PersistedWorkflowRunState::from(
            &slim,
        ))?)
    } else {
        Ok(serde_json::to_string(&PersistedWorkflowRunState::from(
            state,
        ))?)
    }
}

/// 返回运行快照声明的唯一持久化 runnable 时间。
fn workflow_next_retry_at_ms(state: &WorkflowRunState) -> CoreResult<Option<i64>> {
    if state.status != RunStatus::Queued {
        return Ok(None);
    }
    state
        .next_retry_at_ms
        .map(|value| sqlite_millis(value, "workflow next_retry_at_ms"))
        .transpose()
}

/// v10 及更早快照只有节点级退避时间，迁移时取最早值提升到运行级字段。
fn legacy_workflow_next_retry_at_ms(state: &WorkflowRunState) -> Option<u64> {
    if state.status != RunStatus::Queued {
        return None;
    }
    state
        .nodes
        .values()
        .filter(|node| node.status == RunStatus::Queued)
        .filter_map(|node| {
            node.error_state
                .as_ref()
                .and_then(|error| error.next_retry_at_ms)
        })
        .min()
}

/// C10：load 时若 state_json 已瘦身，从独立列补回冻结执行定义。
fn rehydrate_prepared_workflow(
    state: &mut WorkflowRunState,
    prepared_workflow_json: Option<&str>,
) -> CoreResult<()> {
    if state.prepared_workflow.is_some() {
        return Ok(());
    }
    let Some(raw) = prepared_workflow_json
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    state.prepared_workflow = Some(serde_json::from_str(raw)?);
    Ok(())
}

fn load_live_worker_lease(
    transaction: &rusqlite::Transaction<'_>,
    workflow_id: &WorkflowId,
    run_id: &RunId,
    now_ms_sql: i64,
) -> CoreResult<Option<WorkflowWorkerLease>> {
    let lease = transaction
        .query_row(
            "SELECT owner_id, generation, acquired_at_ms, heartbeat_at_ms, expires_at_ms
             FROM workflow_run_worker_leases
             WHERE workflow_id = ?1 AND run_id = ?2
               AND owner_id IS NOT NULL AND expires_at_ms > ?3",
            params![workflow_id.as_str(), run_id.as_str(), now_ms_sql],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some((owner_id, generation, acquired_at_ms, heartbeat_at_ms, expires_at_ms)) = lease else {
        return Ok(None);
    };
    Ok(Some(WorkflowWorkerLease {
        workflow_id: workflow_id.clone(),
        run_id: run_id.clone(),
        owner_id,
        generation: sqlite_u64(generation, "worker lease generation")?,
        acquired_at_ms: sqlite_u64(acquired_at_ms, "worker lease acquired_at_ms")?,
        heartbeat_at_ms: sqlite_u64(heartbeat_at_ms, "worker lease heartbeat_at_ms")?,
        expires_at_ms: sqlite_u64(expires_at_ms, "worker lease expires_at_ms")?,
    }))
}

fn claim_worker_lease(
    transaction: &rusqlite::Transaction<'_>,
    workflow_id: &WorkflowId,
    run_id: &RunId,
    owner_id: &str,
    now_ms_sql: i64,
    expires_at_ms_sql: i64,
) -> CoreResult<WorkflowWorkerLease> {
    let confirmation_resolution_pending = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM confirmation_resolution_operations
                 WHERE workflow_id=?1 AND run_id=?2 AND status != 'committed'
             )",
            params![workflow_id.as_str(), run_id.as_str()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(sqlite_error)?;
    if confirmation_resolution_pending {
        return Err(CoreError::validation(
            "workflow confirmation resolution is still committing",
        ));
    }
    let lease = transaction
        .query_row(
            "INSERT INTO workflow_run_worker_leases(
                 workflow_id, run_id, owner_id, generation,
                 acquired_at_ms, heartbeat_at_ms, expires_at_ms
             ) VALUES(?1, ?2, ?3, 1, ?4, ?4, ?5)
             ON CONFLICT(workflow_id, run_id) DO UPDATE SET
                 owner_id = excluded.owner_id,
                 generation = workflow_run_worker_leases.generation + 1,
                 acquired_at_ms = excluded.acquired_at_ms,
                 heartbeat_at_ms = excluded.heartbeat_at_ms,
                 expires_at_ms = excluded.expires_at_ms
             WHERE workflow_run_worker_leases.expires_at_ms <= excluded.acquired_at_ms
               AND workflow_run_worker_leases.generation < 9223372036854775807
             RETURNING owner_id, generation, acquired_at_ms,
                       heartbeat_at_ms, expires_at_ms",
            params![
                workflow_id.as_str(),
                run_id.as_str(),
                owner_id,
                now_ms_sql,
                expires_at_ms_sql,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_error)?
        .ok_or_else(|| CoreError::validation("workflow worker lease generation exhausted"))?;
    Ok(WorkflowWorkerLease {
        workflow_id: workflow_id.clone(),
        run_id: run_id.clone(),
        owner_id: lease.0,
        generation: sqlite_u64(lease.1, "worker lease generation")?,
        acquired_at_ms: sqlite_u64(lease.2, "worker lease acquired_at_ms")?,
        heartbeat_at_ms: sqlite_u64(lease.3, "worker lease heartbeat_at_ms")?,
        expires_at_ms: sqlite_u64(lease.4, "worker lease expires_at_ms")?,
    })
}

fn persist_mutated_state(
    transaction: &rusqlite::Transaction<'_>,
    state: &mut WorkflowRunState,
    updated_at_ms: i64,
    conflict_message: &str,
) -> CoreResult<()> {
    let expected_revision = state.state_revision;
    let next_revision = expected_revision
        .checked_add(1)
        .ok_or_else(|| CoreError::validation("workflow state revision overflow"))?;
    // C10：CAS 路径同样瘦身 state_json，避免 rehydrate 后把整图写回放大。
    let state_json = serialize_state_json_for_persist(state, expected_revision > 0)?;
    let next_retry_at_ms = workflow_next_retry_at_ms(state)?;
    // 若 create 时尚未写入独立列（极旧路径），在首次有 definition 的 mutate 时补写。
    if let Some(workflow) = &state.prepared_workflow {
        let prepared_json = serde_json::to_string(workflow)?;
        transaction
            .execute(
                "
                UPDATE workflow_runs
                SET prepared_workflow_json = COALESCE(prepared_workflow_json, ?1)
                WHERE workflow_id = ?2 AND run_id = ?3
                ",
                params![
                    prepared_json,
                    state.workflow_id.as_str(),
                    state.run_id.as_str()
                ],
            )
            .map_err(sqlite_error)?;
    }
    let changed = transaction
        .execute(
            "UPDATE workflow_runs
             SET status = ?1, control = ?2, updated_at_ms = ?3,
                 state_revision = ?4, state_json = ?5, next_retry_at_ms = ?6
             WHERE workflow_id = ?7 AND run_id = ?8
               AND state_revision = ?9",
            params![
                run_status_name(state.status),
                run_control_name(state.control),
                updated_at_ms,
                sqlite_millis(next_revision, "workflow state revision")?,
                state_json,
                next_retry_at_ms,
                state.workflow_id.as_str(),
                state.run_id.as_str(),
                sqlite_millis(expected_revision, "workflow state revision")?,
            ],
        )
        .map_err(sqlite_error)?;
    if changed != 1 {
        return Err(CoreError::validation(conflict_message));
    }
    append_state_events(transaction, state)?;
    state.state_revision = next_revision;
    Ok(())
}

fn validate_lease_owner(owner_id: &str) -> CoreResult<()> {
    if owner_id.trim().is_empty() {
        return Err(CoreError::validation(
            "workflow worker lease owner_id cannot be empty",
        ));
    }
    Ok(())
}

fn validate_scheduler_owner(owner_id: &str) -> CoreResult<()> {
    if owner_id.trim().is_empty() {
        return Err(CoreError::validation(
            "workflow scheduler lease owner_id cannot be empty",
        ));
    }
    Ok(())
}

fn lease_expiry(now_ms: u64, ttl_ms: u64) -> CoreResult<u64> {
    if ttl_ms == 0 {
        return Err(CoreError::validation(
            "workflow worker lease ttl_ms must be greater than zero",
        ));
    }
    now_ms
        .checked_add(ttl_ms)
        .ok_or_else(|| CoreError::validation("workflow worker lease expiry overflow"))
}

fn sqlite_millis(value: u64, field: &str) -> CoreResult<i64> {
    i64::try_from(value).map_err(|_| CoreError::validation(format!("{field} exceeds SQLite i64")))
}

fn sqlite_u64(value: i64, field: &str) -> CoreResult<u64> {
    u64::try_from(value).map_err(|_| CoreError::validation(format!("{field} cannot be negative")))
}

fn migrate_embedded_events(connection: &mut Connection) -> CoreResult<()> {
    let transaction = connection.transaction().map_err(sqlite_error)?;
    let mut statement = transaction
        .prepare("SELECT workflow_id, run_id, state_json FROM workflow_runs")
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(sqlite_error)?;
    let mut snapshots = Vec::new();
    for row in rows {
        snapshots.push(row.map_err(sqlite_error)?);
    }
    drop(statement);
    for (workflow_id, run_id, state_json) in snapshots {
        let state = serde_json::from_str::<WorkflowRunState>(&state_json)?;
        for event in &state.structured_events {
            let sequence = i64::try_from(event.sequence)
                .map_err(|_| CoreError::validation("workflow event sequence exceeds SQLite i64"))?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO workflow_run_events(workflow_id, run_id, sequence, event_json) VALUES(?1, ?2, ?3, ?4)",
                    params![workflow_id, run_id, sequence, serde_json::to_string(event)?],
                )
                .map_err(sqlite_error)?;
        }
        for (index, event) in state.events.iter().enumerate() {
            let index = i64::try_from(index).map_err(|_| {
                CoreError::validation("legacy workflow event index exceeds SQLite i64")
            })?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO workflow_run_legacy_events(workflow_id, run_id, event_index, event_text) VALUES(?1, ?2, ?3, ?4)",
                    params![workflow_id, run_id, index, event],
                )
                .map_err(sqlite_error)?;
        }
    }
    transaction.commit().map_err(sqlite_error)
}

fn migrate_workflow_operations_v6(connection: &mut Connection) -> CoreResult<()> {
    let table_sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'workflow_operations'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    if table_sql
        .as_deref()
        .is_some_and(|sql| sql.contains("'aborted'"))
    {
        return Ok(());
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    transaction
        .execute_batch(
            "DROP INDEX IF EXISTS idx_workflow_operations_run;
             DROP INDEX IF EXISTS idx_workflow_operations_status;
             ALTER TABLE workflow_operations RENAME TO workflow_operations_v5;
             CREATE TABLE workflow_operations (
                 operation_id TEXT PRIMARY KEY,
                 workflow_id TEXT NOT NULL,
                 run_id TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 attempt INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 request_hash TEXT NOT NULL,
                 lease_generation INTEGER NOT NULL,
                 status TEXT NOT NULL CHECK(status IN (
                     'prepared', 'dispatched', 'completed', 'in_doubt', 'aborted', 'committed'
                 )),
                 response_json TEXT,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL,
                 dispatched_at_ms INTEGER,
                 completed_at_ms INTEGER,
                 in_doubt_at_ms INTEGER,
                 committed_at_ms INTEGER,
                 FOREIGN KEY(workflow_id, run_id)
                     REFERENCES workflow_runs(workflow_id, run_id) ON DELETE CASCADE
             );
             INSERT INTO workflow_operations SELECT * FROM workflow_operations_v5;
             DROP TABLE workflow_operations_v5;
             CREATE INDEX idx_workflow_operations_run
                 ON workflow_operations(workflow_id, run_id, node_id, attempt);
             CREATE INDEX idx_workflow_operations_status
                 ON workflow_operations(status, updated_at_ms);",
        )
        .map_err(sqlite_error)?;
    transaction.commit().map_err(sqlite_error)
}

/// C10：独立列保存冻结执行定义；从旧 state_json 回填，后续 save 可瘦身 state_json。
fn migrate_prepared_workflow_column_v8(connection: &mut Connection) -> CoreResult<()> {
    if !sqlite_column_exists(connection, "workflow_runs", "prepared_workflow_json")? {
        connection
            .execute(
                "ALTER TABLE workflow_runs ADD COLUMN prepared_workflow_json TEXT",
                [],
            )
            .map_err(sqlite_error)?;
    }
    let mut statement = connection
        .prepare(
            "
            SELECT workflow_id, run_id, state_json, prepared_workflow_json
            FROM workflow_runs
            ",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(sqlite_error)?;
    let mut backfills = Vec::new();
    for row in rows {
        let (workflow_id, run_id, state_json, existing) = row.map_err(sqlite_error)?;
        if existing
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            continue;
        }
        let state: WorkflowRunState = serde_json::from_str(&state_json)?;
        if let Some(workflow) = state.prepared_workflow {
            backfills.push((workflow_id, run_id, serde_json::to_string(&workflow)?));
        }
    }
    drop(statement);
    for (workflow_id, run_id, prepared_json) in backfills {
        connection
            .execute(
                "
                UPDATE workflow_runs
                SET prepared_workflow_json = ?1
                WHERE workflow_id = ?2 AND run_id = ?3
                  AND (prepared_workflow_json IS NULL OR prepared_workflow_json = '')
                ",
                params![prepared_json, workflow_id, run_id],
            )
            .map_err(sqlite_error)?;
    }
    Ok(())
}

/// F12：把控制态提升为可索引列，Stop/Pause 与 dispatch 门禁不再依赖反序列化快照。
fn migrate_workflow_run_control_v9(connection: &mut Connection) -> CoreResult<()> {
    if !sqlite_column_exists(connection, "workflow_runs", "control")? {
        connection
            .execute(
                "ALTER TABLE workflow_runs ADD COLUMN control TEXT NOT NULL DEFAULT 'continue'
                 CHECK(control IN ('continue', 'pause', 'stop'))",
                [],
            )
            .map_err(sqlite_error)?;
    }
    let mut statement = connection
        .prepare("SELECT workflow_id, run_id, state_json FROM workflow_runs")
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(sqlite_error)?;
    let mut backfills = Vec::new();
    for row in rows {
        let (workflow_id, run_id, state_json) = row.map_err(sqlite_error)?;
        let state: WorkflowRunState = serde_json::from_str(&state_json)?;
        backfills.push((workflow_id, run_id, run_control_name(state.control)));
    }
    drop(statement);
    for (workflow_id, run_id, control) in backfills {
        connection
            .execute(
                "UPDATE workflow_runs SET control = ?1
                 WHERE workflow_id = ?2 AND run_id = ?3",
                params![control, workflow_id, run_id],
            )
            .map_err(sqlite_error)?;
    }
    Ok(())
}

/// F9：把旧快照中的节点退避时间提升为运行级字段和可索引 runnable 投影。
fn migrate_workflow_run_retry_v11(connection: &mut Connection) -> CoreResult<()> {
    if !sqlite_column_exists(connection, "workflow_runs", "next_retry_at_ms")? {
        connection
            .execute(
                "ALTER TABLE workflow_runs ADD COLUMN next_retry_at_ms INTEGER",
                [],
            )
            .map_err(sqlite_error)?;
    }
    let mut statement = connection
        .prepare("SELECT workflow_id, run_id, state_json FROM workflow_runs")
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(sqlite_error)?;
    let mut backfills = Vec::new();
    for row in rows {
        let (workflow_id, run_id, state_json) = row.map_err(sqlite_error)?;
        let mut state = serde_json::from_str::<WorkflowRunState>(&state_json)?;
        state.next_retry_at_ms = if state.status == RunStatus::Queued {
            state
                .next_retry_at_ms
                .or_else(|| legacy_workflow_next_retry_at_ms(&state))
        } else {
            None
        };
        let next_retry_at_ms = workflow_next_retry_at_ms(&state)?;
        let mut state_value = serde_json::from_str::<Value>(&state_json)?;
        if let Some(object) = state_value.as_object_mut() {
            if let Some(next_retry_at_ms) = state.next_retry_at_ms {
                object.insert(
                    "next_retry_at_ms".to_owned(),
                    Value::Number(next_retry_at_ms.into()),
                );
            } else {
                object.remove("next_retry_at_ms");
            }
        }
        backfills.push((
            workflow_id,
            run_id,
            next_retry_at_ms,
            serde_json::to_string(&state_value)?,
        ));
    }
    drop(statement);
    for (workflow_id, run_id, next_retry_at_ms, state_json) in backfills {
        connection
            .execute(
                "UPDATE workflow_runs SET next_retry_at_ms = ?1, state_json = ?2
                 WHERE workflow_id = ?3 AND run_id = ?4",
                params![next_retry_at_ms, state_json, workflow_id, run_id],
            )
            .map_err(sqlite_error)?;
    }
    Ok(())
}

fn migrate_workflow_operations_v7(connection: &mut Connection) -> CoreResult<()> {
    let has_recovery_policy =
        sqlite_column_exists(connection, "workflow_operations", "recovery_policy")?;
    let has_response_policy =
        sqlite_column_exists(connection, "workflow_operations", "response_policy")?;
    if has_recovery_policy && has_response_policy {
        return Ok(());
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    if !has_recovery_policy {
        transaction
            .execute(
                "ALTER TABLE workflow_operations ADD COLUMN recovery_policy TEXT NOT NULL
                     DEFAULT 'manual_resolution' CHECK(recovery_policy IN (
                         'manual_resolution', 'replay_executor', 'reconcile_receipt'
                     ))",
                [],
            )
            .map_err(sqlite_error)?;
    }
    if !has_response_policy {
        transaction
            .execute(
                "ALTER TABLE workflow_operations ADD COLUMN response_policy TEXT NOT NULL
                     DEFAULT 'allow_external_response' CHECK(response_policy IN (
                         'allow_external_response', 'require_executor_receipt'
                     ))",
                [],
            )
            .map_err(sqlite_error)?;
    }
    transaction.commit().map_err(sqlite_error)
}

fn append_state_events(
    transaction: &rusqlite::Transaction<'_>,
    state: &WorkflowRunState,
) -> CoreResult<()> {
    let last_sequence = transaction
        .query_row(
            "SELECT MAX(sequence) FROM workflow_run_events WHERE workflow_id = ?1 AND run_id = ?2",
            params![state.workflow_id.as_str(), state.run_id.as_str()],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(sqlite_error)?;
    for event in state.structured_events.iter().filter(|event| {
        last_sequence.is_none_or(|sequence| event.sequence > sequence.max(0) as u64)
    }) {
        let sequence = i64::try_from(event.sequence)
            .map_err(|_| CoreError::validation("workflow event sequence exceeds SQLite i64"))?;
        transaction
            .execute(
                "INSERT INTO workflow_run_events(workflow_id, run_id, sequence, event_json) VALUES(?1, ?2, ?3, ?4)",
                params![state.workflow_id.as_str(), state.run_id.as_str(), sequence, serde_json::to_string(event)?],
            )
            .map_err(sqlite_error)?;
    }
    let legacy_count = transaction
        .query_row(
            "SELECT COUNT(*) FROM workflow_run_legacy_events WHERE workflow_id = ?1 AND run_id = ?2",
            params![state.workflow_id.as_str(), state.run_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_error)?;
    let legacy_count = usize::try_from(legacy_count.max(0))
        .map_err(|_| CoreError::validation("legacy workflow event count exceeds usize"))?;
    for (index, event) in state.events.iter().enumerate().skip(legacy_count) {
        let index = i64::try_from(index)
            .map_err(|_| CoreError::validation("legacy workflow event index exceeds SQLite i64"))?;
        transaction
            .execute(
                "INSERT INTO workflow_run_legacy_events(workflow_id, run_id, event_index, event_text) VALUES(?1, ?2, ?3, ?4)",
                params![state.workflow_id.as_str(), state.run_id.as_str(), index, event],
            )
            .map_err(sqlite_error)?;
    }
    Ok(())
}

impl WorkflowRuntimeStore for SqliteWorkflowRuntimeStore {
    fn create_state(&self, state: &WorkflowRunState) -> CoreResult<()> {
        if state.state_revision != 0 {
            return Err(CoreError::validation(
                "new workflow state revision must be zero",
            ));
        }
        // C10：create 时把冻结定义写入独立列；state_json 可含首次快照（便于旧读者）。
        let state_json = serde_json::to_string(&PersistedWorkflowRunState::from(state))?;
        let prepared_workflow_json = state
            .prepared_workflow
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let next_retry_at_ms = workflow_next_retry_at_ms(state)?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        for event in &state.structured_events {
            let sequence = i64::try_from(event.sequence)
                .map_err(|_| CoreError::validation("workflow event sequence exceeds SQLite i64"))?;
            transaction
                .execute(
                    "INSERT INTO workflow_run_events(workflow_id, run_id, sequence, event_json) VALUES(?1, ?2, ?3, ?4)",
                    params![state.workflow_id.as_str(), state.run_id.as_str(), sequence, serde_json::to_string(event)?],
                )
                .map_err(sqlite_error)?;
        }
        transaction
            .execute(
                "INSERT INTO workflow_runs(
                    workflow_id, run_id, status, control, updated_at_ms, state_revision,
                    state_json, next_retry_at_ms, prepared_workflow_json
                 ) VALUES(?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
                params![
                    state.workflow_id.as_str(),
                    state.run_id.as_str(),
                    run_status_name(state.status),
                    run_control_name(state.control),
                    unix_timestamp_ms_i64()?,
                    state_json,
                    next_retry_at_ms,
                    prepared_workflow_json,
                ],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)
    }

    /// 保存整份运行快照；节点级索引后续可在不破坏 JSON 快照的情况下追加。
    fn save_state(
        &self,
        state: &mut WorkflowRunState,
        commit_operation_id: Option<&str>,
    ) -> CoreResult<()> {
        let expected_revision = state.state_revision;
        // C10：revision>0 后不再把完整 prepared_workflow 图重复写入 state_json（写放大）。
        // 内存仍保留 definition；磁盘独立列在 create_state 时已固化。
        let state_json = serialize_state_json_for_persist(state, expected_revision > 0)?;
        let next_retry_at_ms = workflow_next_retry_at_ms(state)?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        // 防御：若独立列为空而内存仍有 definition，补写一次（不覆盖已有冻结定义）。
        if let Some(workflow) = &state.prepared_workflow {
            let prepared_json = serde_json::to_string(workflow)?;
            transaction
                .execute(
                    "
                    UPDATE workflow_runs
                    SET prepared_workflow_json = COALESCE(prepared_workflow_json, ?1)
                    WHERE workflow_id = ?2 AND run_id = ?3
                    ",
                    params![
                        prepared_json,
                        state.workflow_id.as_str(),
                        state.run_id.as_str()
                    ],
                )
                .map_err(sqlite_error)?;
        }
        if let Some(lease) = &self.worker_lease {
            let now_ms = unix_timestamp_ms_i64()?;
            let generation = sqlite_millis(lease.generation, "worker lease generation")?;
            let lease_is_current = transaction
                .query_row(
                    "
                    SELECT EXISTS(
                        SELECT 1 FROM workflow_run_worker_leases
                        WHERE workflow_id = ?1 AND run_id = ?2
                          AND owner_id = ?3 AND generation = ?4
                          AND expires_at_ms > ?5
                    )
                    ",
                    params![
                        state.workflow_id.as_str(),
                        state.run_id.as_str(),
                        lease.owner_id.as_str(),
                        generation,
                        now_ms,
                    ],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(sqlite_error)?;
            if !lease_is_current {
                return Err(CoreError::validation(
                    "workflow worker lease lost before state save",
                ));
            }
        }
        let expected_revision_sql = sqlite_millis(expected_revision, "workflow state revision")?;
        let next_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| CoreError::validation("workflow state revision overflow"))?;
        let next_revision_sql = sqlite_millis(next_revision, "workflow state revision")?;
        let changed = transaction
            .execute(
                "
                UPDATE workflow_runs
                SET status = ?1, control = ?2, updated_at_ms = ?3,
                    state_revision = ?4, state_json = ?5, next_retry_at_ms = ?6
                WHERE workflow_id = ?7 AND run_id = ?8
                  AND state_revision = ?9
                ",
                params![
                    run_status_name(state.status),
                    run_control_name(state.control),
                    unix_timestamp_ms_i64()?,
                    next_revision_sql,
                    state_json,
                    next_retry_at_ms,
                    state.workflow_id.as_str(),
                    state.run_id.as_str(),
                    expected_revision_sql,
                ],
            )
            .map_err(sqlite_error)?;
        if changed == 0 {
            let actual_revision = transaction
                .query_row(
                    "SELECT state_revision FROM workflow_runs WHERE workflow_id = ?1 AND run_id = ?2",
                    params![state.workflow_id.as_str(), state.run_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(sqlite_error)?;
            let Some(actual_revision) = actual_revision else {
                return Err(CoreError::WorkflowRunNotFound {
                    workflow_id: state.workflow_id.as_str().to_owned(),
                    run_id: state.run_id.as_str().to_owned(),
                });
            };
            return Err(CoreError::WorkflowStateRevisionConflict {
                workflow_id: state.workflow_id.as_str().to_owned(),
                run_id: state.run_id.as_str().to_owned(),
                expected: expected_revision,
                actual: sqlite_u64(actual_revision, "workflow state revision")?,
            });
        }
        append_state_events(&transaction, state)?;
        if let Some(operation_id) = commit_operation_id {
            let now_ms = unix_timestamp_ms_i64()?;
            let committed = transaction
                .execute(
                    "UPDATE workflow_operations
                     SET status = 'committed', updated_at_ms = ?1, committed_at_ms = ?1
                     WHERE operation_id = ?2 AND workflow_id = ?3 AND run_id = ?4
                       AND status = 'completed'",
                    params![
                        now_ms,
                        operation_id,
                        state.workflow_id.as_str(),
                        state.run_id.as_str(),
                    ],
                )
                .map_err(sqlite_error)?;
            if committed != 1 {
                return Err(CoreError::validation(
                    "workflow operation changed before atomic node commit",
                ));
            }
        }
        if let Some(lease) = &self.worker_lease {
            if worker_yields_lease(state.status) {
                let released = transaction
                    .execute(
                        "
                        UPDATE workflow_run_worker_leases
                        SET owner_id = NULL, heartbeat_at_ms = ?1, expires_at_ms = 0
                        WHERE workflow_id = ?2 AND run_id = ?3 AND owner_id = ?4
                          AND generation = ?5
                        ",
                        params![
                            unix_timestamp_ms_i64()?,
                            state.workflow_id.as_str(),
                            state.run_id.as_str(),
                            lease.owner_id.as_str(),
                            sqlite_millis(lease.generation, "worker lease generation")?,
                        ],
                    )
                    .map_err(sqlite_error)?;
                if released != 1 {
                    return Err(CoreError::validation(
                        "workflow worker lease changed before yield release",
                    ));
                }
            }
        }
        transaction.commit().map_err(sqlite_error)?;
        state.state_revision = next_revision;
        Ok(())
    }

    /// 加载运行快照；JSON 损坏会显式转为 CoreError::Json。
    fn load_state(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
    ) -> CoreResult<Option<WorkflowRunState>> {
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let snapshot = transaction
            .query_row(
                "
                SELECT state_revision, state_json, prepared_workflow_json FROM workflow_runs
                WHERE workflow_id = ?1 AND run_id = ?2
                ",
                params![workflow_id.as_str(), run_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?;

        let Some((state_revision, state_json, prepared_workflow_json)) = snapshot else {
            return Ok(None);
        };
        let mut state = serde_json::from_str::<WorkflowRunState>(&state_json)?;
        state.state_revision = sqlite_u64(state_revision, "workflow state revision")?;
        rehydrate_prepared_workflow(&mut state, prepared_workflow_json.as_deref())?;
        let structured_events = load_structured_events(&transaction, workflow_id, run_id)?;
        if !structured_events.is_empty() {
            state.structured_events = structured_events;
        }
        let legacy_events = load_legacy_events(&transaction, workflow_id, run_id)?;
        if !legacy_events.is_empty() {
            state.events = legacy_events;
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(Some(state))
    }

    fn operation_lease_generation(&self) -> u64 {
        self.worker_lease
            .as_ref()
            .map(|lease| lease.generation)
            .unwrap_or(0)
    }

    fn load_operation(&self, operation_id: &str) -> CoreResult<Option<WorkflowOperation>> {
        SqliteWorkflowRuntimeStore::load_operation(self, operation_id)
    }

    fn create_operation(&self, operation: &NewWorkflowOperation, now_ms: u64) -> CoreResult<()> {
        SqliteWorkflowRuntimeStore::create_operation(self, operation, now_ms)
    }

    fn transition_operation(
        &self,
        operation_id: &str,
        expected: WorkflowOperationStatus,
        next: WorkflowOperationStatus,
        response_json: Option<&Value>,
        now_ms: u64,
    ) -> CoreResult<bool> {
        SqliteWorkflowRuntimeStore::transition_operation(
            self,
            operation_id,
            expected,
            next,
            response_json,
            now_ms,
        )
    }

    fn complete_operation_response(
        &self,
        operation_id: &str,
        response_json: &Value,
        now_ms: u64,
    ) -> CoreResult<WorkflowOperationCompletionOutcome> {
        SqliteWorkflowRuntimeStore::complete_operation_response(
            self,
            operation_id,
            response_json,
            now_ms,
        )
    }

    fn operation_dispatch_authorization(
        &self,
        operation_id: &str,
    ) -> ExternalDispatchAuthorization {
        let store = self.clone();
        let operation_id = operation_id.to_owned();
        ExternalDispatchAuthorization::new(move |dispatch| {
            let now_ms = sqlite_u64(
                unix_timestamp_ms_i64()?,
                "workflow dispatch authorization timestamp",
            )?;
            store.authorize_operation_dispatch(&operation_id, dispatch, now_ms)
        })
    }

    fn delete_prepared_operation(&self, operation_id: &str) -> CoreResult<bool> {
        SqliteWorkflowRuntimeStore::delete_prepared_operation(self, operation_id)
    }
}

fn load_structured_events(
    connection: &Connection,
    workflow_id: &WorkflowId,
    run_id: &RunId,
) -> CoreResult<Vec<crate::workflow::WorkflowRuntimeEvent>> {
    let mut statement = connection
        .prepare(
            "SELECT event_json FROM workflow_run_events WHERE workflow_id = ?1 AND run_id = ?2 ORDER BY sequence",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map(params![workflow_id.as_str(), run_id.as_str()], |row| {
            row.get::<_, String>(0)
        })
        .map_err(sqlite_error)?;
    let mut events = Vec::new();
    for row in rows {
        events.push(serde_json::from_str(&row.map_err(sqlite_error)?)?);
    }
    Ok(events)
}

fn load_legacy_events(
    connection: &Connection,
    workflow_id: &WorkflowId,
    run_id: &RunId,
) -> CoreResult<Vec<String>> {
    let mut statement = connection
        .prepare(
            "SELECT event_text FROM workflow_run_legacy_events WHERE workflow_id = ?1 AND run_id = ?2 ORDER BY event_index",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map(params![workflow_id.as_str(), run_id.as_str()], |row| {
            row.get::<_, String>(0)
        })
        .map_err(sqlite_error)?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row.map_err(sqlite_error)?);
    }
    Ok(events)
}

impl SqliteWorkflowRuntimeStore {
    /// 直接从追加事件表读取增量事件，轮询无需反序列化完整运行快照。
    pub fn list_events_since(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
        after_sequence: u64,
        limit: Option<usize>,
    ) -> CoreResult<Option<(RunStatus, Vec<crate::workflow::WorkflowRuntimeEvent>)>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let status = connection
            .query_row(
                "SELECT status FROM workflow_runs WHERE workflow_id = ?1 AND run_id = ?2",
                params![workflow_id.as_str(), run_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        let Some(status) = status else {
            return Ok(None);
        };
        let status = parse_run_status(&status)?;
        let after_sequence = i64::try_from(after_sequence)
            .map_err(|_| CoreError::validation("workflow event cursor exceeds SQLite i64"))?;
        let limit = i64::try_from(limit.unwrap_or(usize::MAX).min(i64::MAX as usize))
            .map_err(|_| CoreError::validation("workflow event limit exceeds SQLite i64"))?;
        let mut statement = connection
            .prepare(
                "SELECT event_json FROM workflow_run_events
                 WHERE workflow_id = ?1 AND run_id = ?2 AND sequence >= ?3
                 ORDER BY sequence LIMIT ?4",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map(
                params![workflow_id.as_str(), run_id.as_str(), after_sequence, limit],
                |row| row.get::<_, String>(0),
            )
            .map_err(sqlite_error)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(serde_json::from_str(&row.map_err(sqlite_error)?)?);
        }
        Ok(Some((status, events)))
    }

    /// C1：终态 run 超过保留窗口后删除追加事件与 legacy 事件，避免历史无限膨胀。
    /// 主 state_json 快照保留以便列表/审计。
    pub fn prune_terminal_run_events(&self, max_age_ms: u64) -> CoreResult<usize> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let cutoff = now_ms.saturating_sub(max_age_ms) as i64;
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut deleted = 0usize;
        deleted += connection
            .execute(
                "DELETE FROM workflow_run_events
                 WHERE (workflow_id, run_id) IN (
                     SELECT workflow_id, run_id FROM workflow_runs
                     WHERE status IN ('succeeded', 'failed', 'cancelled', 'stopped')
                       AND COALESCE(updated_at_ms, 0) < ?1
                 )",
                params![cutoff],
            )
            .map_err(sqlite_error)?;
        deleted += connection
            .execute(
                "DELETE FROM workflow_run_legacy_events
                 WHERE (workflow_id, run_id) IN (
                     SELECT workflow_id, run_id FROM workflow_runs
                     WHERE status IN ('succeeded', 'failed', 'cancelled', 'stopped')
                       AND COALESCE(updated_at_ms, 0) < ?1
                 )",
                params![cutoff],
            )
            .map_err(sqlite_error)?;
        Ok(deleted)
    }

    /// 列出尚未终态的运行快照（用于待审确认项聚合）。
    pub fn list_non_terminal_states(&self) -> CoreResult<Vec<WorkflowRunState>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "
                SELECT state_revision, state_json, prepared_workflow_json FROM workflow_runs
                WHERE status NOT IN ('stopped', 'succeeded', 'failed')
                ORDER BY updated_at_ms DESC
                ",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(sqlite_error)?;
        let mut states = Vec::new();
        for row in rows {
            let (revision, state_json, prepared_workflow_json) = row.map_err(sqlite_error)?;
            let mut state = serde_json::from_str::<WorkflowRunState>(&state_json)?;
            state.state_revision = sqlite_u64(revision, "workflow state revision")?;
            rehydrate_prepared_workflow(&mut state, prepared_workflow_json.as_deref())?;
            states.push(state);
        }
        Ok(states)
    }

    /// F9/F10-d/F12：列出当前已可运行且无存活 worker lease 的 run。
    ///
    /// 初始 Queued 没有 retry 时间，create/claim 崩溃后可立即恢复；自动重试
    /// 只有在 next_retry_at_ms 到期后才进入结果，不能由 open 提前绕过退避。
    pub fn list_orphaned_runnable_states(&self, now_ms: u64) -> CoreResult<Vec<WorkflowRunState>> {
        let now_ms_sql = sqlite_millis(now_ms, "orphan recovery now_ms")?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "
                SELECT r.state_revision, r.state_json, r.prepared_workflow_json
                FROM workflow_runs r
                LEFT JOIN workflow_run_worker_leases l
                  ON l.workflow_id = r.workflow_id AND l.run_id = r.run_id
                WHERE (
                    r.status IN ('running', 'stopping')
                    OR (
                        r.status = 'queued'
                        AND (r.next_retry_at_ms IS NULL OR r.next_retry_at_ms <= ?1)
                    )
                  )
                  AND r.control != 'pause'
                  AND (
                    l.workflow_id IS NULL
                    OR l.owner_id IS NULL
                    OR l.expires_at_ms <= ?1
                  )
                ORDER BY r.updated_at_ms ASC
                ",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map(params![now_ms_sql], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(sqlite_error)?;
        let mut states = Vec::new();
        for row in rows {
            let (revision, state_json, prepared_workflow_json) = row.map_err(sqlite_error)?;
            let mut state = serde_json::from_str::<WorkflowRunState>(&state_json)?;
            state.state_revision = sqlite_u64(revision, "workflow state revision")?;
            rehydrate_prepared_workflow(&mut state, prepared_workflow_json.as_deref())?;
            states.push(state);
        }
        Ok(states)
    }

    /// 返回下一次需要检查 runnable run 的 unix 毫秒时间。
    ///
    /// 查询只访问状态、退避投影和 worker lease 索引，不反序列化运行快照。到期的
    /// retry、无 lease 的初始 Queued 以及 lease 已过期的 Running/Stopping 返回 now。
    pub fn next_runnable_at_ms(&self, now_ms: u64) -> CoreResult<Option<u64>> {
        let now_ms_sql = sqlite_millis(now_ms, "workflow scheduler now_ms")?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let deadline = connection
            .query_row(
                "
                SELECT MIN(MAX(
                    ?1,
                    COALESCE(r.next_retry_at_ms, ?1),
                    COALESCE(l.expires_at_ms, ?1)
                ))
                FROM workflow_runs r
                LEFT JOIN workflow_run_worker_leases l
                  ON l.workflow_id = r.workflow_id AND l.run_id = r.run_id
                WHERE r.status IN ('queued', 'running', 'stopping')
                  AND r.control != 'pause'
                  AND NOT EXISTS (
                    SELECT 1 FROM confirmation_resolution_operations c
                    WHERE c.workflow_id = r.workflow_id AND c.run_id = r.run_id
                      AND c.status != 'committed'
                  )
                ",
                params![now_ms_sql],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(sqlite_error)?;
        deadline
            .map(|value| sqlite_u64(value, "workflow runnable deadline"))
            .transpose()
    }

    /// Git 回档前把所有持久化非终态运行置为 stopped，避免旧分支运行被恢复入口继续调度。
    pub fn stop_non_terminal_for_restore(&self, reason: &str) -> CoreResult<usize> {
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let mut statement = transaction
            .prepare(
                "
                SELECT state_revision, state_json, prepared_workflow_json FROM workflow_runs
                WHERE status NOT IN ('stopped', 'succeeded', 'failed')
                ORDER BY updated_at_ms DESC
                ",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(sqlite_error)?;
        let mut states = Vec::new();
        for row in rows {
            let (revision, state_json, prepared_workflow_json) = row.map_err(sqlite_error)?;
            let mut state = serde_json::from_str::<WorkflowRunState>(&state_json)?;
            state.state_revision = sqlite_u64(revision, "workflow state revision")?;
            rehydrate_prepared_workflow(&mut state, prepared_workflow_json.as_deref())?;
            states.push(state);
        }
        drop(statement);
        let count = states.len();
        for state in &mut states {
            let expected_revision = state.state_revision;
            let next_revision = expected_revision
                .checked_add(1)
                .ok_or_else(|| CoreError::validation("workflow state revision overflow"))?;
            state.status = RunStatus::Stopped;
            state.control = crate::contracts::RunControl::Stop;
            state.stop_reason = Some(reason.to_owned());
            state.pause_reason = None;
            state.next_retry_at_ms = None;
            let sequence = state.next_event_sequence;
            state.next_event_sequence = state.next_event_sequence.saturating_add(1);
            state
                .structured_events
                .push(crate::workflow::WorkflowRuntimeEvent {
                    sequence,
                    event_type: crate::workflow::WorkflowRuntimeEventType::RunStopped,
                    node_id: None,
                    message: reason.to_owned(),
                    metadata: serde_json::Value::Null,
                });
            let state_json = serialize_state_json_for_persist(state, expected_revision > 0)?;
            let changed = transaction
                .execute(
                    "
                    UPDATE workflow_runs
                    SET status = 'stopped', control = 'stop', updated_at_ms = ?1,
                        state_revision = ?2, state_json = ?3, next_retry_at_ms = NULL
                    WHERE workflow_id = ?4 AND run_id = ?5
                      AND state_revision = ?6
                    ",
                    params![
                        unix_timestamp_ms_i64()?,
                        sqlite_millis(next_revision, "workflow state revision")?,
                        state_json,
                        state.workflow_id.as_str(),
                        state.run_id.as_str(),
                        sqlite_millis(expected_revision, "workflow state revision")?,
                    ],
                )
                .map_err(sqlite_error)?;
            if changed != 1 {
                return Err(CoreError::validation(
                    "workflow state changed during Git restore fencing",
                ));
            }
            append_state_events(&transaction, state)?;
            transaction
                .execute(
                    "
                    UPDATE workflow_run_worker_leases
                    SET owner_id = NULL,
                        generation = generation + 1,
                        heartbeat_at_ms = ?1,
                        expires_at_ms = 0
                    WHERE workflow_id = ?2 AND run_id = ?3
                      AND generation < 9223372036854775807
                    ",
                    params![
                        unix_timestamp_ms_i64()?,
                        state.workflow_id.as_str(),
                        state.run_id.as_str(),
                    ],
                )
                .map_err(sqlite_error)?;
            state.state_revision = next_revision;
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(count)
    }
}

/// 将运行状态转成数据库索引用字符串。
fn run_status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::Stopping => "stopping",
        RunStatus::Stopped => "stopped",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

fn run_control_name(control: RunControl) -> &'static str {
    match control {
        RunControl::Continue => "continue",
        RunControl::Pause => "pause",
        RunControl::Stop => "stop",
    }
}

fn dispatch_denied_error(_status: WorkflowOperationStatus) -> CoreError {
    // 此错误描述的是“本次即将发生的调用”尚未越过边界；父 operation 之前是否
    // 有已完成调用由 journal status/子 receipt 单独结算，不能混入本次 outcome。
    CoreError::external_cancelled("workflow_dispatch", ExternalDispatchOutcome::NotDispatched)
}

/// worker 保存这些状态后已不再继续占用执行权。Stopping 仍需当前 worker
/// 完成协作式停止并写回 Stopped，因此与 Running 一样保留 lease。
fn worker_yields_lease(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Queued
            | RunStatus::Paused
            | RunStatus::Stopped
            | RunStatus::Succeeded
            | RunStatus::Failed
    )
}

fn parse_run_status(status: &str) -> CoreResult<RunStatus> {
    match status {
        "queued" => Ok(RunStatus::Queued),
        "running" => Ok(RunStatus::Running),
        "paused" => Ok(RunStatus::Paused),
        "stopping" => Ok(RunStatus::Stopping),
        "stopped" => Ok(RunStatus::Stopped),
        "succeeded" => Ok(RunStatus::Succeeded),
        "failed" => Ok(RunStatus::Failed),
        _ => Err(CoreError::validation(format!(
            "unknown workflow run status in runtime store: {status}"
        ))),
    }
}

type RawWorkflowOperation = (
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

fn read_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawWorkflowOperation> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
        row.get(17)?,
        row.get(18)?,
    ))
}

fn parse_operation(raw: RawWorkflowOperation) -> CoreResult<WorkflowOperation> {
    let (
        operation_id,
        workflow_id,
        run_id,
        node_id,
        attempt,
        kind,
        provider,
        request_hash,
        lease_generation,
        recovery_policy,
        response_policy,
        status,
        response_json,
        created_at_ms,
        updated_at_ms,
        dispatched_at_ms,
        completed_at_ms,
        in_doubt_at_ms,
        committed_at_ms,
    ) = raw;
    Ok(WorkflowOperation {
        operation_id,
        workflow_id: WorkflowId::from(workflow_id),
        run_id: RunId::from(run_id),
        node_id: NodeId::from(node_id),
        attempt: u32::try_from(attempt)
            .map_err(|_| CoreError::validation("workflow operation attempt out of range"))?,
        kind,
        provider,
        request_hash,
        lease_generation: sqlite_u64(lease_generation, "workflow operation lease generation")?,
        recovery_policy: parse_operation_recovery_policy(&recovery_policy)?,
        response_policy: parse_operation_response_policy(&response_policy)?,
        status: parse_operation_status(&status)?,
        response_json: response_json
            .map(|json| serde_json::from_str(&json))
            .transpose()?,
        created_at_ms: sqlite_u64(created_at_ms, "workflow operation created_at_ms")?,
        updated_at_ms: sqlite_u64(updated_at_ms, "workflow operation updated_at_ms")?,
        dispatched_at_ms: optional_sqlite_u64(
            dispatched_at_ms,
            "workflow operation dispatched_at_ms",
        )?,
        completed_at_ms: optional_sqlite_u64(
            completed_at_ms,
            "workflow operation completed_at_ms",
        )?,
        in_doubt_at_ms: optional_sqlite_u64(in_doubt_at_ms, "workflow operation in_doubt_at_ms")?,
        committed_at_ms: optional_sqlite_u64(
            committed_at_ms,
            "workflow operation committed_at_ms",
        )?,
    })
}

fn optional_sqlite_u64(value: Option<i64>, field: &str) -> CoreResult<Option<u64>> {
    value.map(|value| sqlite_u64(value, field)).transpose()
}

fn operation_status_name(status: WorkflowOperationStatus) -> &'static str {
    match status {
        WorkflowOperationStatus::Prepared => "prepared",
        WorkflowOperationStatus::Dispatched => "dispatched",
        WorkflowOperationStatus::Completed => "completed",
        WorkflowOperationStatus::InDoubt => "in_doubt",
        WorkflowOperationStatus::Aborted => "aborted",
        WorkflowOperationStatus::Committed => "committed",
    }
}

fn operation_recovery_policy_name(policy: WorkflowOperationRecoveryPolicy) -> &'static str {
    match policy {
        WorkflowOperationRecoveryPolicy::ManualResolution => "manual_resolution",
        WorkflowOperationRecoveryPolicy::ReplayExecutor => "replay_executor",
        WorkflowOperationRecoveryPolicy::ReconcileReceipt => "reconcile_receipt",
    }
}

fn parse_operation_recovery_policy(value: &str) -> CoreResult<WorkflowOperationRecoveryPolicy> {
    match value {
        "manual_resolution" => Ok(WorkflowOperationRecoveryPolicy::ManualResolution),
        "replay_executor" => Ok(WorkflowOperationRecoveryPolicy::ReplayExecutor),
        "reconcile_receipt" => Ok(WorkflowOperationRecoveryPolicy::ReconcileReceipt),
        _ => Err(CoreError::validation(format!(
            "unknown workflow operation recovery policy: {value}"
        ))),
    }
}

fn operation_response_policy_name(policy: WorkflowOperationResponsePolicy) -> &'static str {
    match policy {
        WorkflowOperationResponsePolicy::AllowExternalResponse => "allow_external_response",
        WorkflowOperationResponsePolicy::RequireExecutorReceipt => "require_executor_receipt",
    }
}

fn parse_operation_response_policy(value: &str) -> CoreResult<WorkflowOperationResponsePolicy> {
    match value {
        "allow_external_response" => Ok(WorkflowOperationResponsePolicy::AllowExternalResponse),
        "require_executor_receipt" => Ok(WorkflowOperationResponsePolicy::RequireExecutorReceipt),
        _ => Err(CoreError::validation(format!(
            "unknown workflow operation response policy: {value}"
        ))),
    }
}

fn parse_operation_status(status: &str) -> CoreResult<WorkflowOperationStatus> {
    match status {
        "prepared" => Ok(WorkflowOperationStatus::Prepared),
        "dispatched" => Ok(WorkflowOperationStatus::Dispatched),
        "completed" => Ok(WorkflowOperationStatus::Completed),
        "in_doubt" => Ok(WorkflowOperationStatus::InDoubt),
        "aborted" => Ok(WorkflowOperationStatus::Aborted),
        "committed" => Ok(WorkflowOperationStatus::Committed),
        _ => Err(CoreError::validation(format!(
            "unknown workflow operation status: {status}"
        ))),
    }
}

type RawConfirmationResolutionOperation = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    bool,
    String,
    Option<i64>,
);

fn read_confirmation_resolution(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RawConfirmationResolutionOperation> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

fn parse_confirmation_resolution(
    raw: RawConfirmationResolutionOperation,
) -> CoreResult<ConfirmationResolutionOperation> {
    Ok(ConfirmationResolutionOperation {
        operation_id: raw.0,
        workflow_id: WorkflowId::from(raw.1),
        run_id: RunId::from(raw.2),
        confirmation_id: raw.3,
        decision: parse_confirmation_resolution_decision(&raw.4)?,
        review_reason: raw.5,
        request_hash: raw.6,
        knowledge_required: raw.7,
        status: parse_confirmation_resolution_status(&raw.8)?,
        projected: raw.9.is_some(),
    })
}

fn load_confirmation_resolution(
    transaction: &rusqlite::Transaction<'_>,
    operation_id: &str,
) -> CoreResult<Option<ConfirmationResolutionOperation>> {
    transaction
        .query_row(
            "SELECT operation_id, workflow_id, run_id, confirmation_id, decision,
                    review_reason, request_hash, knowledge_required, status,
                    projected_at_ms
             FROM confirmation_resolution_operations WHERE operation_id=?1",
            params![operation_id],
            read_confirmation_resolution,
        )
        .optional()
        .map_err(sqlite_error)?
        .map(parse_confirmation_resolution)
        .transpose()
}

fn load_confirmation_resolution_by_identity(
    transaction: &rusqlite::Transaction<'_>,
    workflow_id: &WorkflowId,
    run_id: &RunId,
    confirmation_id: &str,
) -> CoreResult<Option<ConfirmationResolutionOperation>> {
    transaction
        .query_row(
            "SELECT operation_id, workflow_id, run_id, confirmation_id, decision,
                    review_reason, request_hash, knowledge_required, status,
                    projected_at_ms
             FROM confirmation_resolution_operations
             WHERE workflow_id=?1 AND run_id=?2 AND confirmation_id=?3",
            params![workflow_id.as_str(), run_id.as_str(), confirmation_id],
            read_confirmation_resolution,
        )
        .optional()
        .map_err(sqlite_error)?
        .map(parse_confirmation_resolution)
        .transpose()
}

fn confirmation_resolution_decision_name(decision: ConfirmationResolutionDecision) -> &'static str {
    match decision {
        ConfirmationResolutionDecision::Approve => "approve",
        ConfirmationResolutionDecision::Reject => "reject",
    }
}

fn parse_confirmation_resolution_decision(
    decision: &str,
) -> CoreResult<ConfirmationResolutionDecision> {
    match decision {
        "approve" => Ok(ConfirmationResolutionDecision::Approve),
        "reject" => Ok(ConfirmationResolutionDecision::Reject),
        _ => Err(CoreError::validation(format!(
            "unknown confirmation resolution decision: {decision}"
        ))),
    }
}

fn confirmation_resolution_status_name(status: ConfirmationResolutionStatus) -> &'static str {
    match status {
        ConfirmationResolutionStatus::Prepared => "prepared",
        ConfirmationResolutionStatus::KnowledgeCommitted => "knowledge_committed",
        ConfirmationResolutionStatus::Committed => "committed",
    }
}

fn parse_confirmation_resolution_status(status: &str) -> CoreResult<ConfirmationResolutionStatus> {
    match status {
        "prepared" => Ok(ConfirmationResolutionStatus::Prepared),
        "knowledge_committed" => Ok(ConfirmationResolutionStatus::KnowledgeCommitted),
        "committed" => Ok(ConfirmationResolutionStatus::Committed),
        _ => Err(CoreError::validation(format!(
            "unknown confirmation resolution status: {status}"
        ))),
    }
}

fn validate_non_empty_operation_field(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!(
            "confirmation resolution {field} cannot be blank"
        )));
    }
    Ok(())
}

fn validate_operation(operation: &NewWorkflowOperation) -> CoreResult<()> {
    for (field, value) in [
        ("operation_id", operation.operation_id.as_str()),
        ("kind", operation.kind.as_str()),
        ("provider", operation.provider.as_str()),
        ("request_hash", operation.request_hash.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(CoreError::validation(format!(
                "workflow operation {field} cannot be blank"
            )));
        }
    }
    Ok(())
}

fn validate_operation_transition(
    expected: WorkflowOperationStatus,
    next: WorkflowOperationStatus,
    response_json: Option<&Value>,
) -> CoreResult<()> {
    let allowed = matches!(
        (expected, next),
        (
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::Dispatched
        ) | (
            WorkflowOperationStatus::Prepared,
            WorkflowOperationStatus::InDoubt
        ) | (
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::Completed
        ) | (
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::InDoubt
        ) | (
            WorkflowOperationStatus::InDoubt,
            WorkflowOperationStatus::Completed
        ) | (
            WorkflowOperationStatus::InDoubt,
            WorkflowOperationStatus::Dispatched
        ) | (
            WorkflowOperationStatus::Dispatched,
            WorkflowOperationStatus::Aborted
        ) | (
            WorkflowOperationStatus::Completed,
            WorkflowOperationStatus::Committed
        )
    );
    if !allowed {
        return Err(CoreError::validation(format!(
            "invalid workflow operation transition: {} -> {}",
            operation_status_name(expected),
            operation_status_name(next)
        )));
    }
    if next == WorkflowOperationStatus::Completed && response_json.is_none() {
        return Err(CoreError::validation(
            "completed workflow operation requires response_json",
        ));
    }
    Ok(())
}

fn sqlite_column_exists(connection: &Connection, table: &str, column: &str) -> CoreResult<bool> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_error)?;
    for row in rows {
        if row.map_err(sqlite_error)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn configure_connection(connection: &Connection, persistent: bool) -> CoreResult<()> {
    connection
        .execute_batch("PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;")
        .map_err(sqlite_error)?;
    if persistent {
        connection
            .execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(sqlite_error)?;
    }
    Ok(())
}

/// 返回当前 Unix 毫秒时间戳，并转成 SQLite 友好的 i64。
fn unix_timestamp_ms_i64() -> CoreResult<i64> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            CoreError::validation(format!("system time before unix epoch: {error}"))
        })?;
    i64::try_from(duration.as_millis())
        .map_err(|_| CoreError::validation("timestamp_ms exceeds i64 range"))
}

/// 将锁中毒转换成统一错误。
fn lock_error<T>(_: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation("workflow runtime store lock poisoned")
}

/// 将 rusqlite 错误转换成统一外部服务错误。
fn sqlite_error(error: rusqlite::Error) -> CoreError {
    CoreError::External {
        service: "sqlite".to_owned(),
        message: error.to_string(),
    }
}
