use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fs4::FileExt;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    /// N4：指数退避截止时间（unix ms）；0 表示立即可领取。
    #[serde(default)]
    pub next_attempt_at_ms: u64,
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
const PROJECT_MAINTENANCE_DRAIN_TIMEOUT_MS: u64 = 30_000;

/// 项目普通写的 OS 共享锁；进程退出时由内核自动释放。
pub struct ProjectMutationGuard {
    _lock: File,
}

/// maintenance 持有的 OS 独占锁；覆盖 checkout 与索引重建整个临界区。
pub struct ProjectMaintenanceGuard {
    _lock: File,
}

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
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let existing = self.maintenance_state_with_connection(&transaction)?;
        if matches!(
            existing.as_ref().map(|state| state.status.as_str()),
            Some("active")
        ) {
            return Err(CoreError::validation(
                "project maintenance is already active",
            ));
        }
        transaction
            .execute(
                "INSERT INTO project_maintenance(id, kind, status, phase, error, generation)
                 VALUES(1, ?1, 'active', ?2, NULL, 1)
                 ON CONFLICT(id) DO UPDATE SET
                    kind = excluded.kind,
                    status = excluded.status,
                    phase = excluded.phase,
                    error = NULL,
                    generation = project_maintenance.generation + 1",
                params![kind, phase],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)
    }

    /// 普通写先快检 maintenance，再取得共享锁并复检；不会在 restore 后迟到执行。
    pub fn acquire_project_mutation(&self, _kind: &str) -> CoreResult<ProjectMutationGuard> {
        self.ensure_available()?;
        let lock = self.open_mutation_fence()?;
        match FileExt::try_lock_shared(&lock) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(CoreError::validation(
                    "project maintenance is draining active mutations",
                ));
            }
            Err(error) => return Err(error.into()),
        }
        if let Err(error) = self.ensure_available() {
            let _ = FileExt::unlock(&lock);
            return Err(error);
        }
        Ok(ProjectMutationGuard { _lock: lock })
    }

    /// active intent 已持久化后取得独占锁；成功即证明此前普通 mutation 已全部排空。
    pub fn acquire_maintenance_fence(&self) -> CoreResult<ProjectMaintenanceGuard> {
        self.acquire_maintenance_fence_with_timeout(Duration::from_millis(
            PROJECT_MAINTENANCE_DRAIN_TIMEOUT_MS,
        ))
    }

    pub fn acquire_maintenance_fence_with_timeout(
        &self,
        timeout: Duration,
    ) -> CoreResult<ProjectMaintenanceGuard> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(guard) = self.try_acquire_maintenance_fence()? {
                return Ok(guard);
            }
            if std::time::Instant::now() >= deadline {
                return Err(CoreError::validation(
                    "timed out draining active project mutations before maintenance",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 非阻塞尝试取得 maintenance 独占锁，供需要交替执行停机扫描的维护入口使用。
    pub fn try_acquire_maintenance_fence(&self) -> CoreResult<Option<ProjectMaintenanceGuard>> {
        let state = self.maintenance_state()?.ok_or_else(|| {
            CoreError::validation("project maintenance must be active before draining mutations")
        })?;
        if state.status != "active" {
            return Err(CoreError::validation(
                "project maintenance must be active before draining mutations",
            ));
        }
        let lock = self.open_mutation_fence()?;
        match FileExt::try_lock_exclusive(&lock) {
            Ok(()) => Ok(Some(ProjectMaintenanceGuard { _lock: lock })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
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
        // Serialize outbox mutators across threads/processes so concurrent document
        // writers surface CAS/version conflicts, not SQLite "database is locked".
        let _db_lock = self.acquire_outbox_write_lock()?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        transaction
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
        transaction.commit().map_err(sqlite_error)?;
        Ok(event_id)
    }

    pub fn activate(&self, event_id: &str) -> CoreResult<()> {
        let _db_lock = self.acquire_outbox_write_lock()?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
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
        let _db_lock = self.acquire_outbox_write_lock()?;
        self.set_status(event_id, "cancelled")
    }

    pub fn pending(&self) -> CoreResult<Vec<IndexInvalidationEvent>> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT event_id, document_id, reason, source_version,
                        full_rebuild_required, attempt_count, status,
                        COALESCE(next_attempt_at_ms, 0)
                 FROM index_invalidation_events
                 WHERE status = 'pending'
                 ORDER BY rowid",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], row_to_event)
            .map_err(sqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
    }

    /// F2-a：是否已有 pending/processing 的 full rebuild（避免重复入队）。
    pub fn has_incomplete_full_rebuild(&self) -> CoreResult<bool> {
        let connection = self.open()?;
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM index_invalidation_events
                 WHERE full_rebuild_required = 1
                   AND status IN ('pending', 'processing', 'prepared')",
                [],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        Ok(count > 0)
    }

    /// F2-b：是否仍有未完成的索引失效（pending/processing），搜索应 fail-loud。
    pub fn has_incomplete_invalidation(&self) -> CoreResult<bool> {
        let connection = self.open()?;
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM index_invalidation_events
                 WHERE status IN ('pending', 'processing')",
                [],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        Ok(count > 0)
    }

    /// 原子领取最早的 pending 事件；同一事件只会被一个 worker 置为 processing。
    pub fn claim_next(&self) -> CoreResult<Option<IndexInvalidationEvent>> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let event_id = transaction
            .query_row(
                "SELECT event_id FROM index_invalidation_events
                 WHERE status = 'pending'
                   AND COALESCE(next_attempt_at_ms, 0) <= ?1
                 ORDER BY rowid LIMIT 1",
                params![now_ms],
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
                        full_rebuild_required, attempt_count, status,
                        COALESCE(next_attempt_at_ms, 0)
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
    /// N4：未达上限时按 attempt_count 指数退避写入 next_attempt_at_ms。
    pub fn retry(&self, event_id: &str) -> CoreResult<bool> {
        let connection = self.open()?;
        let attempt_count: u32 = connection
            .query_row(
                "SELECT attempt_count FROM index_invalidation_events
                 WHERE event_id = ?1 AND status = 'processing'",
                params![event_id],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        let next_attempt = attempt_count.saturating_add(1);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // 1s, 2s, 4s, 8s… 上限 5 分钟
        let backoff_ms = 1_000i64
            .saturating_mul(1i64 << next_attempt.min(8))
            .min(300_000);
        let next_at = now_ms.saturating_add(backoff_ms);
        let changed = connection
            .execute(
                "UPDATE index_invalidation_events
                 SET status = CASE WHEN attempt_count + 1 >= ?2 THEN 'dead_letter' ELSE 'pending' END,
                     attempt_count = attempt_count + 1,
                     next_attempt_at_ms = CASE
                       WHEN attempt_count + 1 >= ?2 THEN 0
                       ELSE ?3
                     END
                 WHERE event_id = ?1 AND status = 'processing'",
                params![event_id, MAX_ATTEMPTS, next_at],
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

    /// 列出 dead_letter 事件，供诊断/手动恢复入口。
    pub fn list_dead_letters(&self) -> CoreResult<Vec<IndexInvalidationEvent>> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT event_id, document_id, reason, source_version,
                        full_rebuild_required, attempt_count, status,
                        COALESCE(next_attempt_at_ms, 0)
                 FROM index_invalidation_events
                 WHERE status = 'dead_letter'
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
                    next_attempt_at_ms: row.get::<_, i64>(7)?.max(0) as u64,
                })
            })
            .map_err(sqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
    }

    /// 清除退避时间，使 pending 事件立即可领取（测试与手动恢复共用）。
    pub fn clear_backoff(&self, event_id: &str) -> CoreResult<()> {
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE index_invalidation_events SET next_attempt_at_ms = 0
                 WHERE event_id = ?1 AND status = 'pending'",
                params![event_id],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(
                "index invalidation event is not pending",
            ));
        }
        Ok(())
    }

    /// 将 dead_letter 重新入队（手动恢复）。
    pub fn requeue_dead_letter(&self, event_id: &str) -> CoreResult<()> {
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE index_invalidation_events
                 SET status = 'pending', attempt_count = 0, next_attempt_at_ms = 0
                 WHERE event_id = ?1 AND status = 'dead_letter'",
                params![event_id],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(
                "index invalidation event is not a dead letter",
            ));
        }
        Ok(())
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
        self.ensure_schema()?;
        let connection = Connection::open(&self.path).map_err(sqlite_error)?;
        connection
            .busy_timeout(Duration::from_secs(30))
            .map_err(sqlite_error)?;
        // Also set via SQL for engines that honor pragma more reliably under contention.
        connection
            .pragma_update(None, "busy_timeout", 30_000i32)
            .map_err(sqlite_error)?;
        Ok(connection)
    }

    fn ensure_schema(&self) -> CoreResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Separate init lock so prepare/activate (which hold write lock) can call open().
        let _init_lock = self.acquire_named_lock(
            "index_invalidation.init.lock",
            "index invalidation outbox init lock",
        )?;
        let connection = Connection::open(&self.path).map_err(sqlite_error)?;
        connection
            .busy_timeout(Duration::from_secs(30))
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
                    attempt_count INTEGER NOT NULL DEFAULT 0,
                    next_attempt_at_ms INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE IF NOT EXISTS project_maintenance (
                    id INTEGER PRIMARY KEY CHECK(id = 1),
                    kind TEXT NOT NULL,
                    status TEXT NOT NULL,
                    phase TEXT NOT NULL,
                    error TEXT,
                    generation INTEGER NOT NULL DEFAULT 0
                 );",
            )
            .map_err(sqlite_error)?;
        let has_next: bool = connection
            .prepare("PRAGMA table_info(index_invalidation_events)")
            .and_then(|mut stmt| {
                let names = stmt
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(names.iter().any(|n| n == "next_attempt_at_ms"))
            })
            .unwrap_or(false);
        if !has_next {
            connection
                .execute(
                    "ALTER TABLE index_invalidation_events
                     ADD COLUMN next_attempt_at_ms INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(sqlite_error)?;
        }
        let has_generation: bool = connection
            .prepare("PRAGMA table_info(project_maintenance)")
            .and_then(|mut stmt| {
                let names = stmt
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(names.iter().any(|name| name == "generation"))
            })
            .unwrap_or(false);
        if !has_generation {
            connection
                .execute(
                    "ALTER TABLE project_maintenance
                     ADD COLUMN generation INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(sqlite_error)?;
        }
        Ok(())
    }

    fn open_mutation_fence(&self) -> CoreResult<File> {
        let path = self.mutation_fence_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(CoreError::from)
    }

    /// D3-a/D1-b：fence 位于项目树外，并以无损 canonical project identity 稳定派生。
    pub fn mutation_fence_path(&self) -> CoreResult<PathBuf> {
        let database_root = self.path.parent().unwrap_or(self.path.as_path());
        let project_root = if database_root
            .file_name()
            .is_some_and(|name| name == ".runtime")
        {
            database_root.parent().unwrap_or(database_root)
        } else {
            database_root
        };
        let canonical = project_root.canonicalize().or_else(|_| {
            if project_root.is_absolute() {
                Ok::<PathBuf, std::io::Error>(project_root.to_path_buf())
            } else {
                std::env::current_dir().map(|cwd| cwd.join(project_root))
            }
        })?;
        let mut digest = Sha256::new();
        digest.update(b"ariadne-project-mutation-v1\0");
        digest.update(project_path_identity_bytes(&canonical));
        let encoded = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(std::env::temp_dir()
            .join("ariadne-project-mutation-fences")
            .join(format!("{encoded}.lock")))
    }

    /// Exclusive OS lock for outbox mutators (prepare/activate/cancel).
    fn acquire_outbox_write_lock(&self) -> CoreResult<File> {
        self.acquire_named_lock(
            "index_invalidation.write.lock",
            "index invalidation outbox write lock",
        )
    }

    fn acquire_named_lock(&self, file_name: &str, label: &str) -> CoreResult<File> {
        let path = self.path.with_file_name(file_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(CoreError::from)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return Err(CoreError::validation(format!(
                            "timed out waiting for {label}"
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

#[cfg(unix)]
fn project_path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = b"unix\0".to_vec();
    bytes.extend_from_slice(path.as_os_str().as_bytes());
    bytes
}

#[cfg(windows)]
fn project_path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut bytes = b"windows\0".to_vec();
    for unit in path.as_os_str().encode_wide() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
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
        next_attempt_at_ms: row.get::<_, i64>(7).unwrap_or(0).max(0) as u64,
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
