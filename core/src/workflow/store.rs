use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::contracts::{CoreError, CoreResult, RunId, RunStatus, WorkflowId};
use crate::workflow::{WorkflowRunState, WorkflowRuntimeStore};

pub const RUNTIME_DB_FILE: &str = "runtime.db";
const SCHEMA_VERSION: i64 = 2;

#[derive(Serialize)]
struct PersistedWorkflowRunState<'a> {
    workflow_id: &'a WorkflowId,
    run_id: &'a RunId,
    start_node_id: &'a Option<crate::contracts::NodeId>,
    status: RunStatus,
    control: crate::contracts::RunControl,
    pause_reason: &'a Option<String>,
    stop_reason: &'a Option<String>,
    nodes: &'a std::collections::BTreeMap<
        crate::contracts::NodeId,
        crate::workflow::WorkflowNodeRuntimeState,
    >,
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
            start_node_id: &state.start_node_id,
            status: state.status,
            control: state.control,
            pause_reason: &state.pause_reason,
            stop_reason: &state.stop_reason,
            nodes: &state.nodes,
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
#[derive(Debug)]
pub struct SqliteWorkflowRuntimeStore {
    db_path: Option<PathBuf>,
    connection: Mutex<Connection>,
}

impl SqliteWorkflowRuntimeStore {
    /// 在项目根目录打开 runtime.db。
    pub fn open(project_root: impl AsRef<Path>) -> CoreResult<Self> {
        let db_path = project_root.as_ref().join(RUNTIME_DB_FILE);
        let connection = Connection::open(&db_path).map_err(sqlite_error)?;
        configure_connection(&connection, true)?;
        let store = Self {
            db_path: Some(db_path),
            connection: Mutex::new(connection),
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
            connection: Mutex::new(connection),
        };
        store.migrate()?;
        Ok(store)
    }

    /// 返回数据库路径；内存模式下为 None。
    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
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
                    updated_at_ms INTEGER NOT NULL,
                    state_json TEXT NOT NULL,
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
                ",
            )
            .map_err(sqlite_error)?;

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

        connection
            .execute(
                "
                INSERT INTO schema_migrations(name, version, applied_at_ms)
                VALUES('workflow_runtime', ?1, ?2)
                ON CONFLICT(name) DO UPDATE SET
                    version = excluded.version,
                    applied_at_ms = excluded.applied_at_ms
                ",
                params![SCHEMA_VERSION, unix_timestamp_ms_i64()?],
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

impl WorkflowRuntimeStore for SqliteWorkflowRuntimeStore {
    /// 保存整份运行快照；节点级索引后续可在不破坏 JSON 快照的情况下追加。
    fn save_state(&self, state: &WorkflowRunState) -> CoreResult<()> {
        let state_json = serde_json::to_string(&PersistedWorkflowRunState::from(state))?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
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
            let index = i64::try_from(index).map_err(|_| {
                CoreError::validation("legacy workflow event index exceeds SQLite i64")
            })?;
            transaction
                .execute(
                    "INSERT INTO workflow_run_legacy_events(workflow_id, run_id, event_index, event_text) VALUES(?1, ?2, ?3, ?4)",
                    params![state.workflow_id.as_str(), state.run_id.as_str(), index, event],
                )
                .map_err(sqlite_error)?;
        }
        // 同一 workflow/run 反复保存时覆盖整份快照；updated_at_ms 用于前端
        // 展示最后保存时间，也方便后续清理陈旧运行记录。
        transaction
            .execute(
                "
                INSERT INTO workflow_runs(
                    workflow_id, run_id, status, updated_at_ms, state_json
                )
                VALUES(?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(workflow_id, run_id) DO UPDATE SET
                    status = excluded.status,
                    updated_at_ms = excluded.updated_at_ms,
                    state_json = excluded.state_json
                ",
                params![
                    state.workflow_id.as_str(),
                    state.run_id.as_str(),
                    run_status_name(state.status),
                    unix_timestamp_ms_i64()?,
                    state_json,
                ],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(())
    }

    /// 加载运行快照；JSON 损坏会显式转为 CoreError::Json。
    fn load_state(
        &self,
        workflow_id: &WorkflowId,
        run_id: &RunId,
    ) -> CoreResult<Option<WorkflowRunState>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let state_json = connection
            .query_row(
                "
                SELECT state_json FROM workflow_runs
                WHERE workflow_id = ?1 AND run_id = ?2
                ",
                params![workflow_id.as_str(), run_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;

        let Some(state_json) = state_json else {
            return Ok(None);
        };
        let mut state = serde_json::from_str::<WorkflowRunState>(&state_json)?;
        let structured_events = load_structured_events(&connection, workflow_id, run_id)?;
        if !structured_events.is_empty() {
            state.structured_events = structured_events;
        }
        let legacy_events = load_legacy_events(&connection, workflow_id, run_id)?;
        if !legacy_events.is_empty() {
            state.events = legacy_events;
        }
        Ok(Some(state))
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

    /// 列出尚未终态的运行快照（用于待审确认项聚合）。
    pub fn list_non_terminal_states(&self) -> CoreResult<Vec<WorkflowRunState>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "
                SELECT state_json FROM workflow_runs
                WHERE status NOT IN ('stopped', 'succeeded', 'failed')
                ORDER BY updated_at_ms DESC
                ",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite_error)?;
        let mut states = Vec::new();
        for row in rows {
            let state_json = row.map_err(sqlite_error)?;
            states.push(serde_json::from_str::<WorkflowRunState>(&state_json)?);
        }
        Ok(states)
    }

    /// Git 回档前把所有持久化非终态运行置为 stopped，避免旧分支运行被恢复入口继续调度。
    pub fn stop_non_terminal_for_restore(&self, reason: &str) -> CoreResult<usize> {
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let mut statement = transaction
            .prepare(
                "
                SELECT state_json FROM workflow_runs
                WHERE status NOT IN ('stopped', 'succeeded', 'failed')
                ORDER BY updated_at_ms DESC
                ",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite_error)?;
        let mut states = Vec::new();
        for row in rows {
            states.push(serde_json::from_str::<WorkflowRunState>(
                &row.map_err(sqlite_error)?,
            )?);
        }
        drop(statement);
        let count = states.len();
        for state in &mut states {
            state.status = RunStatus::Stopped;
            state.control = crate::contracts::RunControl::Stop;
            state.stop_reason = Some(reason.to_owned());
            state.pause_reason = None;
            transaction
                .execute(
                    "
                    UPDATE workflow_runs
                    SET status = 'stopped', updated_at_ms = ?1, state_json = ?2
                    WHERE workflow_id = ?3 AND run_id = ?4
                    ",
                    params![
                        unix_timestamp_ms_i64()?,
                        serde_json::to_string(&PersistedWorkflowRunState::from(&*state))?,
                        state.workflow_id.as_str(),
                        state.run_id.as_str(),
                    ],
                )
                .map_err(sqlite_error)?;
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
