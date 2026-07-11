use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::contracts::{CoreError, CoreResult};

static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 持久化索引失效事件；prepared 事件不会被 worker 消费。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexInvalidationEvent {
    pub event_id: String,
    pub document_id: String,
    pub reason: String,
    pub source_version: String,
    pub full_rebuild_required: bool,
    pub attempt_count: u32,
    pub status: String,
}

/// 项目级维护状态；active/failed 都会阻止普通写入和工作流继续执行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMaintenanceState {
    pub kind: String,
    pub status: String,
    pub phase: String,
    pub error: Option<String>,
}

const MAX_ATTEMPTS: u32 = 5;

/// 项目级索引失效 outbox，独立数据库避免依赖 UI 转发通知。
#[derive(Debug, Clone)]
pub struct IndexInvalidationOutbox {
    path: PathBuf,
}

impl IndexInvalidationOutbox {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn begin_maintenance(&self, kind: &str, phase: &str) -> CoreResult<()> {
        let connection = self.open()?;
        let existing = self.maintenance_state_with_connection(&connection)?;
        if matches!(
            existing.as_ref().map(|state| state.status.as_str()),
            Some("active")
        ) {
            return Err(CoreError::validation(
                "project maintenance is already active",
            ));
        }
        connection
            .execute(
                "INSERT INTO project_maintenance(id, kind, status, phase, error)
                 VALUES(1, ?1, 'active', ?2, NULL)
                 ON CONFLICT(id) DO UPDATE SET
                    kind = excluded.kind,
                    status = excluded.status,
                    phase = excluded.phase,
                    error = NULL",
                params![kind, phase],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    pub fn update_maintenance_phase(&self, phase: &str) -> CoreResult<()> {
        self.update_maintenance("active", phase, None)
    }

    pub fn complete_maintenance(&self, phase: &str) -> CoreResult<()> {
        self.update_maintenance("completed", phase, None)
    }

    pub fn fail_maintenance(&self, phase: &str, error: &str) -> CoreResult<()> {
        self.update_maintenance("failed", phase, Some(error))
    }

    pub fn maintenance_state(&self) -> CoreResult<Option<ProjectMaintenanceState>> {
        let connection = self.open()?;
        self.maintenance_state_with_connection(&connection)
    }

    pub fn ensure_available(&self) -> CoreResult<()> {
        if let Some(state) = self.maintenance_state()? {
            if state.status == "active" || state.status == "failed" {
                return Err(CoreError::validation(format!(
                    "project maintenance blocks writes: kind={}, status={}, phase={}",
                    state.kind, state.status, state.phase
                )));
            }
        }
        Ok(())
    }

    pub fn prepare(
        &self,
        document_id: &str,
        reason: &str,
        source_version: &str,
        full_rebuild_required: bool,
    ) -> CoreResult<String> {
        let event_id = next_event_id();
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO index_invalidation_events
                 (event_id, document_id, reason, source_version, full_rebuild_required, status, attempt_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'prepared', 0)",
                params![
                    event_id,
                    document_id,
                    reason,
                    source_version,
                    full_rebuild_required
                ],
            )
            .map_err(sqlite_error)?;
        Ok(event_id)
    }

    pub fn activate(&self, event_id: &str) -> CoreResult<()> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let document_id = transaction
            .query_row(
                "SELECT document_id FROM index_invalidation_events WHERE event_id = ?1",
                params![event_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?
            .ok_or_else(|| CoreError::validation("index invalidation event not found"))?;
        transaction
            .execute(
                "UPDATE index_invalidation_events
                 SET status = 'superseded'
                 WHERE document_id = ?1 AND event_id <> ?2 AND status = 'pending'",
                params![document_id, event_id],
            )
            .map_err(sqlite_error)?;
        let changed = transaction
            .execute(
                "UPDATE index_invalidation_events SET status = 'pending'
                 WHERE event_id = ?1 AND status = 'prepared'",
                params![event_id],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(
                "index invalidation event is not prepared",
            ));
        }
        transaction.commit().map_err(sqlite_error)
    }

    pub fn cancel(&self, event_id: &str) -> CoreResult<()> {
        self.set_status(event_id, "cancelled")
    }

    pub fn pending(&self) -> CoreResult<Vec<IndexInvalidationEvent>> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT event_id, document_id, reason, source_version,
                        full_rebuild_required, attempt_count, status
                 FROM index_invalidation_events
                 WHERE status = 'pending'
                 ORDER BY rowid",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(IndexInvalidationEvent {
                    event_id: row.get(0)?,
                    document_id: row.get(1)?,
                    reason: row.get(2)?,
                    source_version: row.get(3)?,
                    full_rebuild_required: row.get(4)?,
                    attempt_count: row.get(5)?,
                    status: row.get(6)?,
                })
            })
            .map_err(sqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
    }

    /// 原子领取最早的 pending 事件；同一事件只会被一个 worker 置为 processing。
    pub fn claim_next(&self) -> CoreResult<Option<IndexInvalidationEvent>> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let event_id = transaction
            .query_row(
                "SELECT event_id FROM index_invalidation_events
                 WHERE status = 'pending' ORDER BY rowid LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        let Some(event_id) = event_id else {
            transaction.commit().map_err(sqlite_error)?;
            return Ok(None);
        };
        let changed = transaction
            .execute(
                "UPDATE index_invalidation_events
                 SET status = 'processing'
                 WHERE event_id = ?1 AND status = 'pending'",
                params![event_id],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            transaction.commit().map_err(sqlite_error)?;
            return Ok(None);
        }
        let event = transaction
            .query_row(
                "SELECT event_id, document_id, reason, source_version,
                        full_rebuild_required, attempt_count, status
                 FROM index_invalidation_events WHERE event_id = ?1",
                params![event_id],
                row_to_event,
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(Some(event))
    }

    pub fn complete(&self, event_id: &str) -> CoreResult<()> {
        self.set_status(event_id, "completed")
    }

    /// 可重试错误达到上限后转入 dead_letter，避免单条坏事件永久阻塞队列。
    pub fn retry(&self, event_id: &str) -> CoreResult<bool> {
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE index_invalidation_events
                 SET status = CASE WHEN attempt_count + 1 >= ?2 THEN 'dead_letter' ELSE 'pending' END,
                     attempt_count = attempt_count + 1
                 WHERE event_id = ?1 AND status = 'processing'",
                params![event_id, MAX_ATTEMPTS],
            )
            .map_err(sqlite_error)?;
        if changed == 0 {
            return Err(CoreError::validation(
                "index invalidation event is not processing",
            ));
        }
        let status = connection
            .query_row(
                "SELECT status FROM index_invalidation_events WHERE event_id = ?1",
                params![event_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(sqlite_error)?;
        Ok(status == "dead_letter")
    }

    pub fn supersede(&self, event_id: &str) -> CoreResult<()> {
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE index_invalidation_events SET status = 'superseded'
                 WHERE event_id = ?1 AND status = 'processing'",
                params![event_id],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(
                "index invalidation event is not processing",
            ));
        }
        Ok(())
    }

    /// 应用进程异常退出后，把未完成的 processing 事件放回队列。
    ///
    /// 仅应在确认当前进程没有该项目的活动 worker 时调用；索引写入按版本幂等，重放比永久丢失安全。
    pub fn requeue_interrupted(&self) -> CoreResult<usize> {
        let connection = self.open()?;
        connection
            .execute(
                "UPDATE index_invalidation_events
                 SET status = 'pending', attempt_count = attempt_count + 1
                 WHERE status = 'processing'",
                [],
            )
            .map_err(sqlite_error)
    }

    fn set_status(&self, event_id: &str, status: &str) -> CoreResult<()> {
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE index_invalidation_events SET status = ?2 WHERE event_id = ?1",
                params![event_id, status],
            )
            .map_err(sqlite_error)?;
        if changed == 0 {
            return Err(CoreError::validation("index invalidation event not found"));
        }
        Ok(())
    }

    fn update_maintenance(&self, status: &str, phase: &str, error: Option<&str>) -> CoreResult<()> {
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE project_maintenance SET status = ?1, phase = ?2, error = ?3
                 WHERE id = 1 AND status = 'active'",
                params![status, phase, error],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation("project maintenance is not active"));
        }
        Ok(())
    }

    fn maintenance_state_with_connection(
        &self,
        connection: &Connection,
    ) -> CoreResult<Option<ProjectMaintenanceState>> {
        connection
            .query_row(
                "SELECT kind, status, phase, error FROM project_maintenance WHERE id = 1",
                [],
                |row| {
                    Ok(ProjectMaintenanceState {
                        kind: row.get(0)?,
                        status: row.get(1)?,
                        phase: row.get(2)?,
                        error: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(sqlite_error)
    }

    fn open(&self) -> CoreResult<Connection> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&self.path).map_err(sqlite_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(sqlite_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS index_invalidation_events (
                    event_id TEXT PRIMARY KEY,
                    document_id TEXT NOT NULL,
                    reason TEXT NOT NULL,
                    source_version TEXT NOT NULL,
                    full_rebuild_required INTEGER NOT NULL,
                    status TEXT NOT NULL,
                    attempt_count INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE IF NOT EXISTS project_maintenance (
                    id INTEGER PRIMARY KEY CHECK(id = 1),
                    kind TEXT NOT NULL,
                    status TEXT NOT NULL,
                    phase TEXT NOT NULL,
                    error TEXT
                 );",
            )
            .map_err(sqlite_error)?;
        Ok(connection)
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexInvalidationEvent> {
    Ok(IndexInvalidationEvent {
        event_id: row.get(0)?,
        document_id: row.get(1)?,
        reason: row.get(2)?,
        source_version: row.get(3)?,
        full_rebuild_required: row.get(4)?,
        attempt_count: row.get(5)?,
        status: row.get(6)?,
    })
}

fn next_event_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = EVENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("idx-{nanos:x}-{sequence:x}")
}

fn sqlite_error(error: rusqlite::Error) -> CoreError {
    CoreError::External {
        service: "index_invalidation_outbox".to_owned(),
        message: error.to_string(),
    }
}
