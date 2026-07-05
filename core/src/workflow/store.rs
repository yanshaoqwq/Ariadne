use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::contracts::{CoreError, CoreResult, RunId, RunStatus, WorkflowId};
use crate::workflow::{WorkflowRunState, WorkflowRuntimeStore};

pub const RUNTIME_DB_FILE: &str = "runtime.db";
const SCHEMA_VERSION: i64 = 1;

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
        let connection = self.connection.lock().map_err(lock_error)?;
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
                ",
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

impl WorkflowRuntimeStore for SqliteWorkflowRuntimeStore {
    /// 保存整份运行快照；节点级索引后续可在不破坏 JSON 快照的情况下追加。
    fn save_state(&self, state: &WorkflowRunState) -> CoreResult<()> {
        let state_json = serde_json::to_string(state)?;
        let connection = self.connection.lock().map_err(lock_error)?;
        // 同一 workflow/run 反复保存时覆盖整份快照；updated_at_ms 用于前端
        // 展示最后保存时间，也方便后续清理陈旧运行记录。
        connection
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

        state_json
            .map(|value| serde_json::from_str::<WorkflowRunState>(&value).map_err(CoreError::from))
            .transpose()
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
