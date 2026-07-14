use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use fs4::FileExt;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::contracts::{CancellationToken, CoreError, CoreResult, SourceSpan};
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    ChapterStageSummaryView, ChapterSummaryConfirmationView, ChapterSummaryView, ConfirmationItem,
    ConfirmationKind, ConfirmationState, ForeshadowingRecord, ForeshadowingStatus, PlannerIssue,
    RegisterContent, RegisterFunction, RegisteredChange, RegisteredChangeStatus, StoryEvent,
    StoryEventStatus, StorySegment, SummaryGenerationContext, SummaryPipelineDraft,
    SummaryStageContext,
};
use crate::rag::numbering::parse_segment_number;

pub const METADATA_DB_FILE: &str = "metadata.db";
const SCHEMA_VERSION: i64 = 3;

/// 已确认知识实体的可版本化检索快照；pending confirmation 不会进入此结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeRetrievalSnapshot {
    pub revision: String,
    pub entries: Vec<KnowledgeRetrievalEntry>,
}

/// 四层总结知识映射到检索域前的中立记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeRetrievalEntry {
    pub entity_id: String,
    pub layer: String,
    pub text: String,
    #[serde(default)]
    pub sources: Vec<SourceSpan>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeOperationReceipt {
    pub operation_id: String,
    pub request_hash: String,
    pub response_json: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummarizerStageOperationStatus {
    Prepared,
    Dispatched,
    Completed,
    InDoubt,
    Aborted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummarizerStageOperation {
    pub operation_id: String,
    pub scope_id: String,
    pub parent_operation_id: String,
    pub parent_operation_attempt: u32,
    pub parent_request_hash: String,
    pub step: String,
    pub stage_attempt: u32,
    pub request_hash: String,
    pub status: SummarizerStageOperationStatus,
    pub response_json: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SummarizerStagePreparation {
    Execute {
        operation_id: String,
    },
    Replay {
        operation_id: String,
        response_json: Value,
    },
    InDoubt {
        operation_id: String,
    },
}

/// 写作知识库 SQLite 持久化后端，使用完整关系表存储源实体。
/// 22 张双向索引表全部可从源实体派生，加载时重放 upsert 路径重建，零重复代码。
#[derive(Debug)]
pub struct SqliteWritingKnowledgeStore {
    db_path: Option<PathBuf>,
    connection: Mutex<Connection>,
    writer_lock: Mutex<()>,
}

pub(crate) struct KnowledgeWriterGuard<'a> {
    db_path: Option<PathBuf>,
    _process_guard: MutexGuard<'a, ()>,
    _file: Option<File>,
}

impl SqliteWritingKnowledgeStore {
    /// 在项目根目录打开 metadata.db。
    pub fn open(project_root: impl AsRef<Path>) -> CoreResult<Self> {
        let db_path = project_root.as_ref().join(METADATA_DB_FILE);
        let connection = Connection::open(&db_path).map_err(sqlite_error)?;
        configure_connection(&connection, true)?;
        let store = Self {
            db_path: Some(db_path),
            connection: Mutex::new(connection),
            writer_lock: Mutex::new(()),
        };
        store.migrate()?;
        Ok(store)
    }

    /// 打开内存数据库，主要用于测试。
    pub fn open_in_memory() -> CoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(sqlite_error)?;
        configure_connection(&connection, false)?;
        let store = Self {
            db_path: None,
            connection: Mutex::new(connection),
            writer_lock: Mutex::new(()),
        };
        store.migrate()?;
        Ok(store)
    }

    /// 返回数据库路径；内存模式下为 None。
    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    /// 执行幂等 schema 迁移。
    pub fn migrate(&self) -> CoreResult<()> {
        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS schema_migrations (
                name TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                applied_at_ms INTEGER NOT NULL
            );

            -- 故事段源实体
            CREATE TABLE IF NOT EXISTS story_segments (
                segment_id   TEXT PRIMARY KEY,
                number       TEXT NOT NULL,
                chapter_id   TEXT NOT NULL,
                summary      TEXT NOT NULL,
                source_json  TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_seg_chapter ON story_segments(chapter_id);

            -- 事件源实体
            CREATE TABLE IF NOT EXISTS story_events (
                event_id      TEXT PRIMARY KEY,
                summary       TEXT NOT NULL,
                status        TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT '{}'
            );
            -- 事件-故事段 多对多
            CREATE TABLE IF NOT EXISTS event_segment_links (
                event_id   TEXT NOT NULL,
                segment_id TEXT NOT NULL,
                PRIMARY KEY(event_id, segment_id)
            );
            -- 事件-章节 多对多
            CREATE TABLE IF NOT EXISTS event_chapter_links (
                event_id   TEXT NOT NULL,
                chapter_id TEXT NOT NULL,
                PRIMARY KEY(event_id, chapter_id)
            );

            -- 注册项源实体（content_json 含 kind + content 字段）
            CREATE TABLE IF NOT EXISTS registered_changes (
                change_id     TEXT PRIMARY KEY,
                function      TEXT NOT NULL,
                status        TEXT NOT NULL,
                content_json  TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT 'null'
            );
            -- 注册项-故事段 多对多（realized 后建立）
            CREATE TABLE IF NOT EXISTS change_segment_links (
                change_id  TEXT NOT NULL,
                segment_id TEXT NOT NULL,
                PRIMARY KEY(change_id, segment_id)
            );

            -- 伏笔源实体
            CREATE TABLE IF NOT EXISTS foreshadowing (
                foreshadowing_id TEXT PRIMARY KEY,
                title            TEXT NOT NULL,
                description      TEXT NOT NULL,
                status           TEXT NOT NULL,
                metadata_json    TEXT NOT NULL DEFAULT '{}'
            );
            -- 伏笔-种植故事段
            CREATE TABLE IF NOT EXISTS foreshadowing_planted (
                foreshadowing_id TEXT NOT NULL,
                segment_id       TEXT NOT NULL,
                PRIMARY KEY(foreshadowing_id, segment_id)
            );
            -- 伏笔-回收故事段
            CREATE TABLE IF NOT EXISTS foreshadowing_recovered (
                foreshadowing_id TEXT NOT NULL,
                segment_id       TEXT NOT NULL,
                PRIMARY KEY(foreshadowing_id, segment_id)
            );

            -- 总结文本（章节/阶段）
            CREATE TABLE IF NOT EXISTS chapter_summaries (
                chapter_id TEXT PRIMARY KEY,
                summary    TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS stage_summaries (
                stage_id TEXT PRIMARY KEY,
                summary  TEXT NOT NULL
            );
            -- 章节-阶段 归属
            CREATE TABLE IF NOT EXISTS chapter_stage (
                chapter_id TEXT PRIMARY KEY,
                stage_id   TEXT NOT NULL
            );

            -- Planner 未落地问题
            CREATE TABLE IF NOT EXISTS planner_issues (
                issue_id                TEXT PRIMARY KEY,
                change_id               TEXT NOT NULL,
                chapter_id              TEXT NOT NULL,
                reason                  TEXT NOT NULL,
                related_sources_json    TEXT NOT NULL DEFAULT '[]',
                planner_explanation     TEXT,
                correction_patch_json   TEXT
            );

            -- 确认项（由 SummaryPipeline 生成的四层确认 + 其他）
            CREATE TABLE IF NOT EXISTS writing_confirmations (
                confirmation_id TEXT PRIMARY KEY,
                kind            TEXT NOT NULL,
                state           TEXT NOT NULL,
                prompt_key      TEXT NOT NULL,
                metadata_json   TEXT NOT NULL DEFAULT 'null'
            );

            CREATE TABLE IF NOT EXISTS knowledge_operations (
                operation_id TEXT PRIMARY KEY,
                request_hash TEXT NOT NULL,
                response_json TEXT NOT NULL,
                committed_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS summarizer_stage_operations (
                operation_id TEXT PRIMARY KEY,
                scope_id TEXT NOT NULL,
                parent_operation_id TEXT NOT NULL,
                parent_operation_attempt INTEGER NOT NULL,
                parent_request_hash TEXT NOT NULL,
                step TEXT NOT NULL,
                stage_attempt INTEGER NOT NULL,
                request_hash TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN (
                    'prepared', 'dispatched', 'completed', 'in_doubt', 'aborted'
                )),
                response_json TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                UNIQUE(scope_id, step, stage_attempt)
            );
            CREATE INDEX IF NOT EXISTS idx_summarizer_stage_parent
                ON summarizer_stage_operations(scope_id, step, stage_attempt);
            ",
            )
            .map_err(sqlite_error)?;

        connection
            .execute(
                "INSERT INTO schema_migrations(name, version, applied_at_ms)
             VALUES('writing_knowledge', ?1, ?2)
             ON CONFLICT(name) DO UPDATE SET
                 version = excluded.version,
                 applied_at_ms = excluded.applied_at_ms",
                params![SCHEMA_VERSION, unix_timestamp_ms_i64()?],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    /// 把内存知识库的全部源实体落库（全量 upsert）。
    /// 不存储派生索引——load_knowledge 时重放 upsert 路径重建。
    pub fn save_knowledge(&self, kb: &MemoryWritingKnowledgeBase) -> CoreResult<()> {
        let _writer_lock = self.acquire_writer_lock()?;
        self.save_knowledge_transaction(kb, None, &CancellationToken::new())
    }

    pub fn save_knowledge_with_operation(
        &self,
        kb: &MemoryWritingKnowledgeBase,
        operation_id: &str,
        request_hash: &str,
        response_json: &Value,
        cancellation: &CancellationToken,
    ) -> CoreResult<()> {
        let writer_lock = self.acquire_writer_lock()?;
        self.save_knowledge_with_operation_for_chapter_locked(
            kb,
            None,
            operation_id,
            request_hash,
            response_json,
            cancellation,
            None,
            &writer_lock,
        )
    }

    /// C2：章节作用域落盘（Summarizer 生产路径）。
    /// 只替换该章的故事段/事件链接，其它章实体保留；单 SQLite 事务。
    pub fn save_chapter_knowledge_with_operation(
        &self,
        kb: &MemoryWritingKnowledgeBase,
        chapter_id: &str,
        operation_id: &str,
        request_hash: &str,
        response_json: &Value,
        cancellation: &CancellationToken,
    ) -> CoreResult<()> {
        let writer_lock = self.acquire_writer_lock()?;
        self.save_knowledge_with_operation_for_chapter_locked(
            kb,
            Some(chapter_id),
            operation_id,
            request_hash,
            response_json,
            cancellation,
            None,
            &writer_lock,
        )
    }

    /// Test/diagnostic: `fail_after` aborts after N successful phase checks (before commit).
    pub fn save_chapter_knowledge_with_operation_fail_after(
        &self,
        kb: &MemoryWritingKnowledgeBase,
        chapter_id: &str,
        operation_id: &str,
        request_hash: &str,
        response_json: &Value,
        fail_after: usize,
    ) -> CoreResult<()> {
        let writer_lock = self.acquire_writer_lock()?;
        self.save_knowledge_with_operation_for_chapter_locked(
            kb,
            Some(chapter_id),
            operation_id,
            request_hash,
            response_json,
            &CancellationToken::new(),
            Some(fail_after),
            &writer_lock,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn save_knowledge_with_operation_for_chapter_locked(
        &self,
        kb: &MemoryWritingKnowledgeBase,
        chapter_id: Option<&str>,
        operation_id: &str,
        request_hash: &str,
        response_json: &Value,
        cancellation: &CancellationToken,
        fail_after: Option<usize>,
        writer_lock: &KnowledgeWriterGuard<'_>,
    ) -> CoreResult<()> {
        self.validate_writer_lock(writer_lock)?;
        if operation_id.trim().is_empty() || request_hash.trim().is_empty() {
            return Err(CoreError::validation(
                "knowledge operation id and request hash cannot be empty",
            ));
        }
        if let Some(existing) = self.load_operation_receipt(operation_id, request_hash)? {
            if existing.response_json == *response_json {
                return Ok(());
            }
            return Err(CoreError::validation(format!(
                "knowledge operation id reused with a different response: {operation_id}"
            )));
        }
        if let Some(chapter_id) = chapter_id {
            self.save_chapter_knowledge_transaction(
                kb,
                chapter_id,
                Some((operation_id, request_hash, response_json)),
                cancellation,
                fail_after,
            )
        } else {
            self.save_knowledge_transaction(
                kb,
                Some((operation_id, request_hash, response_json)),
                cancellation,
            )
        }
    }

    pub fn load_operation_receipt(
        &self,
        operation_id: &str,
        request_hash: &str,
    ) -> CoreResult<Option<KnowledgeOperationReceipt>> {
        let conn = self.connection.lock().map_err(lock_error)?;
        let receipt = conn
            .query_row(
                "SELECT request_hash, response_json FROM knowledge_operations WHERE operation_id=?1",
                params![operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(sqlite_error)?;
        let Some((persisted_hash, response_json)) = receipt else {
            return Ok(None);
        };
        if persisted_hash != request_hash {
            return Err(CoreError::validation(format!(
                "knowledge operation id reused with a different request: {operation_id}"
            )));
        }
        Ok(Some(KnowledgeOperationReceipt {
            operation_id: operation_id.to_owned(),
            request_hash: persisted_hash,
            response_json: serde_json::from_str(&response_json)?,
        }))
    }

    /// F14：knowledge 侧确认决策、实体物化与 operation receipt 同事务提交。
    pub fn resolve_confirmation_with_operation(
        &self,
        confirmation_id: &str,
        target_state: ConfirmationState,
        operation_id: &str,
        request_hash: &str,
        response_json: &Value,
    ) -> CoreResult<bool> {
        if !matches!(
            target_state,
            ConfirmationState::Approved | ConfirmationState::Rejected
        ) {
            return Err(CoreError::validation(
                "knowledge confirmation resolution requires approved or rejected state",
            ));
        }
        let _writer_lock = self.acquire_writer_lock()?;
        if let Some(existing) = self.load_operation_receipt(operation_id, request_hash)? {
            if existing.response_json == *response_json {
                return Ok(true);
            }
            return Err(CoreError::validation(format!(
                "knowledge operation id reused with a different response: {operation_id}"
            )));
        }
        let knowledge = self.load_knowledge()?;
        let item = knowledge
            .confirmations(None)?
            .into_iter()
            .find(|item| item.confirmation_id == confirmation_id);
        let Some(item) = item else {
            return Ok(false);
        };
        match target_state {
            ConfirmationState::Approved => {
                crate::rag::approve_confirmation(&knowledge, confirmation_id)?;
            }
            ConfirmationState::Rejected => {
                crate::rag::reject_confirmation(&knowledge, confirmation_id)?;
            }
            _ => unreachable!("validated confirmation target"),
        }
        let chapter_id = item
            .metadata
            .get("chapter_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|chapter_id| !chapter_id.is_empty());
        if let Some(chapter_id) = chapter_id {
            self.save_knowledge_with_operation_for_chapter_locked(
                &knowledge,
                Some(chapter_id),
                operation_id,
                request_hash,
                response_json,
                &CancellationToken::new(),
                None,
                &_writer_lock,
            )?;
        } else {
            self.save_knowledge_with_operation_for_chapter_locked(
                &knowledge,
                None,
                operation_id,
                request_hash,
                response_json,
                &CancellationToken::new(),
                None,
                &_writer_lock,
            )?;
        }
        Ok(true)
    }

    /// F14 prepare 前只读探测；真正提交证明由 knowledge receipt 提供。
    pub fn has_confirmation(&self, confirmation_id: &str) -> CoreResult<bool> {
        Ok(self
            .load_knowledge()?
            .confirmations(None)?
            .iter()
            .any(|item| item.confirmation_id == confirmation_id))
    }

    pub(crate) fn acquire_writer_lock(&self) -> CoreResult<KnowledgeWriterGuard<'_>> {
        let process_guard = self.writer_lock.lock().map_err(lock_error)?;
        let Some(db_path) = self.db_path.as_ref() else {
            return Ok(KnowledgeWriterGuard {
                db_path: None,
                _process_guard: process_guard,
                _file: None,
            });
        };
        let lock_path = knowledge_writer_lock_path(db_path)?;
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        for attempt in 0..6_000 {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => {
                    return Ok(KnowledgeWriterGuard {
                        db_path: Some(db_path.clone()),
                        _process_guard: process_guard,
                        _file: Some(file),
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if attempt == 5_999 {
                        return Err(CoreError::validation(
                            "timed out waiting for knowledge writer",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(CoreError::validation("knowledge writer acquisition failed"))
    }

    fn validate_writer_lock(&self, writer_lock: &KnowledgeWriterGuard<'_>) -> CoreResult<()> {
        if writer_lock.db_path == self.db_path {
            Ok(())
        } else {
            Err(CoreError::validation(
                "knowledge writer guard belongs to a different store",
            ))
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_chapter_knowledge_with_operation_locked(
        &self,
        kb: &MemoryWritingKnowledgeBase,
        chapter_id: &str,
        operation_id: &str,
        request_hash: &str,
        response_json: &Value,
        cancellation: &CancellationToken,
        writer_lock: &KnowledgeWriterGuard<'_>,
    ) -> CoreResult<()> {
        self.save_knowledge_with_operation_for_chapter_locked(
            kb,
            Some(chapter_id),
            operation_id,
            request_hash,
            response_json,
            cancellation,
            None,
            writer_lock,
        )
    }

    pub fn prepare_summarizer_stage_operation(
        &self,
        scope_id: &str,
        parent_operation_id: &str,
        parent_operation_attempt: u32,
        parent_request_hash: &str,
        step: &str,
        request_hash: &str,
    ) -> CoreResult<SummarizerStagePreparation> {
        validate_summarizer_stage_identity(
            scope_id,
            parent_operation_id,
            parent_request_hash,
            step,
            request_hash,
        )?;
        let now_ms = unix_timestamp_ms_i64()?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let latest = transaction
            .query_row(
                "SELECT operation_id, scope_id, parent_operation_id, parent_operation_attempt,
                        parent_request_hash, step, stage_attempt, request_hash, status,
                        response_json
                 FROM summarizer_stage_operations
                 WHERE scope_id = ?1 AND step = ?2
                 ORDER BY stage_attempt DESC LIMIT 1",
                params![scope_id, step],
                read_summarizer_stage_operation,
            )
            .optional()
            .map_err(sqlite_error)?
            .map(parse_summarizer_stage_operation)
            .transpose()?;

        if let Some(operation) = latest.as_ref() {
            if operation.parent_operation_attempt > parent_operation_attempt {
                return Err(CoreError::validation(format!(
                    "stale summarizer parent operation attempt for step {step}"
                )));
            }
            if operation.parent_operation_attempt == parent_operation_attempt
                && (operation.parent_operation_id != parent_operation_id
                    || operation.parent_request_hash != parent_request_hash)
            {
                return Err(CoreError::validation(format!(
                    "summarizer parent operation identity mismatch for step {step}"
                )));
            }
            if operation.request_hash == request_hash {
                match operation.status {
                    SummarizerStageOperationStatus::Completed => {
                        let response_json = operation.response_json.clone().ok_or_else(|| {
                            CoreError::validation(format!(
                                "completed summarizer stage response is missing: {}",
                                operation.operation_id
                            ))
                        })?;
                        transaction.commit().map_err(sqlite_error)?;
                        return Ok(SummarizerStagePreparation::Replay {
                            operation_id: operation.operation_id.clone(),
                            response_json,
                        });
                    }
                    SummarizerStageOperationStatus::Prepared
                        if operation.parent_operation_attempt == parent_operation_attempt =>
                    {
                        transaction.commit().map_err(sqlite_error)?;
                        return Ok(SummarizerStagePreparation::Execute {
                            operation_id: operation.operation_id.clone(),
                        });
                    }
                    SummarizerStageOperationStatus::Dispatched
                    | SummarizerStageOperationStatus::InDoubt
                        if operation.parent_operation_attempt == parent_operation_attempt =>
                    {
                        if operation.status == SummarizerStageOperationStatus::Dispatched {
                            transaction
                                .execute(
                                    "UPDATE summarizer_stage_operations
                                     SET status = 'in_doubt', updated_at_ms = ?1
                                     WHERE operation_id = ?2 AND status = 'dispatched'",
                                    params![now_ms, operation.operation_id],
                                )
                                .map_err(sqlite_error)?;
                        }
                        transaction.commit().map_err(sqlite_error)?;
                        return Ok(SummarizerStagePreparation::InDoubt {
                            operation_id: operation.operation_id.clone(),
                        });
                    }
                    _ => {}
                }
            } else if operation.parent_operation_attempt == parent_operation_attempt {
                return Err(CoreError::validation(format!(
                    "summarizer stage request changed within the same parent operation: {step}"
                )));
            }

            if matches!(
                operation.status,
                SummarizerStageOperationStatus::Prepared
                    | SummarizerStageOperationStatus::Dispatched
                    | SummarizerStageOperationStatus::InDoubt
            ) {
                transaction
                    .execute(
                        "UPDATE summarizer_stage_operations
                         SET status = 'aborted', updated_at_ms = ?1
                         WHERE operation_id = ?2 AND status IN ('prepared', 'dispatched', 'in_doubt')",
                        params![now_ms, operation.operation_id],
                    )
                    .map_err(sqlite_error)?;
            }
        }

        let stage_attempt = latest
            .as_ref()
            .map(|operation| operation.stage_attempt.saturating_add(1))
            .unwrap_or(1);
        let operation_id = crate::skills::stable_text_hash(&format!(
            "summarizer-stage-operation-v1\0{scope_id}\0{step}\0{stage_attempt}"
        ));
        transaction
            .execute(
                "INSERT INTO summarizer_stage_operations(
                     operation_id, scope_id, parent_operation_id, parent_operation_attempt,
                     parent_request_hash, step, stage_attempt, request_hash, status,
                     response_json, created_at_ms, updated_at_ms
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'prepared',NULL,?9,?9)",
                params![
                    operation_id,
                    scope_id,
                    parent_operation_id,
                    i64::from(parent_operation_attempt),
                    parent_request_hash,
                    step,
                    i64::from(stage_attempt),
                    request_hash,
                    now_ms,
                ],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(SummarizerStagePreparation::Execute { operation_id })
    }

    pub fn mark_summarizer_stage_dispatched(&self, operation_id: &str) -> CoreResult<()> {
        self.transition_summarizer_stage_operation(
            operation_id,
            SummarizerStageOperationStatus::Prepared,
            SummarizerStageOperationStatus::Dispatched,
            None,
        )
    }

    pub fn complete_summarizer_stage_operation(
        &self,
        operation_id: &str,
        response_json: &Value,
    ) -> CoreResult<()> {
        self.transition_summarizer_stage_operation(
            operation_id,
            SummarizerStageOperationStatus::Dispatched,
            SummarizerStageOperationStatus::Completed,
            Some(response_json),
        )
    }

    pub fn abort_summarizer_stage_operation(&self, operation_id: &str) -> CoreResult<()> {
        let now_ms = unix_timestamp_ms_i64()?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let changed = connection
            .execute(
                "UPDATE summarizer_stage_operations
                 SET status = 'aborted', updated_at_ms = ?1
                 WHERE operation_id = ?2 AND status IN ('prepared', 'dispatched')",
                params![now_ms, operation_id],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(format!(
                "summarizer stage operation changed before abort: {operation_id}"
            )));
        }
        Ok(())
    }

    pub fn mark_summarizer_stage_in_doubt(&self, operation_id: &str) -> CoreResult<()> {
        self.transition_summarizer_stage_operation(
            operation_id,
            SummarizerStageOperationStatus::Dispatched,
            SummarizerStageOperationStatus::InDoubt,
            None,
        )
    }

    pub fn list_summarizer_stage_operations(
        &self,
        scope_id: &str,
    ) -> CoreResult<Vec<SummarizerStageOperation>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "SELECT operation_id, scope_id, parent_operation_id, parent_operation_attempt,
                        parent_request_hash, step, stage_attempt, request_hash, status,
                        response_json
                 FROM summarizer_stage_operations
                 WHERE scope_id = ?1
                 ORDER BY step, stage_attempt",
            )
            .map_err(sqlite_error)?;
        let rows: CoreResult<Vec<SummarizerStageOperation>> = statement
            .query_map(params![scope_id], read_summarizer_stage_operation)
            .map_err(sqlite_error)?
            .map(|row| {
                row.map_err(sqlite_error)
                    .and_then(parse_summarizer_stage_operation)
            })
            .collect();
        rows
    }

    fn transition_summarizer_stage_operation(
        &self,
        operation_id: &str,
        expected: SummarizerStageOperationStatus,
        next: SummarizerStageOperationStatus,
        response_json: Option<&Value>,
    ) -> CoreResult<()> {
        if operation_id.trim().is_empty() {
            return Err(CoreError::validation(
                "summarizer stage operation id cannot be empty",
            ));
        }
        if next == SummarizerStageOperationStatus::Completed && response_json.is_none() {
            return Err(CoreError::validation(
                "completed summarizer stage requires response",
            ));
        }
        let response_json = response_json.map(serde_json::to_string).transpose()?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let changed = connection
            .execute(
                "UPDATE summarizer_stage_operations
                 SET status = ?1, response_json = COALESCE(?2, response_json),
                     updated_at_ms = ?3
                 WHERE operation_id = ?4 AND status = ?5",
                params![
                    summarizer_stage_status_name(next),
                    response_json,
                    unix_timestamp_ms_i64()?,
                    operation_id,
                    summarizer_stage_status_name(expected),
                ],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(CoreError::validation(format!(
                "summarizer stage operation changed before {}: {operation_id}",
                summarizer_stage_status_name(next)
            )));
        }
        Ok(())
    }

    /// C2：单章 delta 落盘。删除并重建该 chapter 的 segments/events 链接，其它章不动。
    fn save_chapter_knowledge_transaction(
        &self,
        kb: &MemoryWritingKnowledgeBase,
        chapter_id: &str,
        operation: Option<(&str, &str, &Value)>,
        cancellation: &CancellationToken,
        fail_after: Option<usize>,
    ) -> CoreResult<()> {
        if chapter_id.trim().is_empty() {
            return Err(CoreError::validation("chapter_id cannot be empty"));
        }
        let mut phases = 0usize;
        let mut tick = |cancellation: &CancellationToken| -> CoreResult<()> {
            cancellation.check()?;
            phases += 1;
            if fail_after == Some(phases) {
                return Err(CoreError::validation(format!(
                    "injected knowledge chapter save failure after phase {phases}"
                )));
            }
            Ok(())
        };
        tick(cancellation)?;
        let conn = self.connection.lock().map_err(lock_error)?;
        let tx = conn.unchecked_transaction().map_err(sqlite_error)?;

        // 收集该章旧 segment，清理其事件链接后删除 segment。
        let mut old_segments = Vec::new();
        {
            let mut stmt = tx
                .prepare("SELECT segment_id FROM story_segments WHERE chapter_id = ?1")
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map(params![chapter_id], |row| row.get::<_, String>(0))
                .map_err(sqlite_error)?;
            for row in rows {
                old_segments.push(row.map_err(sqlite_error)?);
            }
        }
        for seg_id in &old_segments {
            tx.execute(
                "DELETE FROM event_segment_links WHERE segment_id = ?1",
                params![seg_id],
            )
            .map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM change_segment_links WHERE segment_id = ?1",
                params![seg_id],
            )
            .map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM foreshadowing_planted WHERE segment_id = ?1",
                params![seg_id],
            )
            .map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM foreshadowing_recovered WHERE segment_id = ?1",
                params![seg_id],
            )
            .map_err(sqlite_error)?;
        }
        tx.execute(
            "DELETE FROM event_chapter_links WHERE chapter_id = ?1",
            params![chapter_id],
        )
        .map_err(sqlite_error)?;
        // 不再属于任何章的事件删除
        tx.execute_batch(
            "DELETE FROM story_events
             WHERE event_id NOT IN (SELECT event_id FROM event_chapter_links);
             DELETE FROM event_segment_links
             WHERE event_id NOT IN (SELECT event_id FROM story_events);",
        )
        .map_err(sqlite_error)?;
        tx.execute(
            "DELETE FROM story_segments WHERE chapter_id = ?1",
            params![chapter_id],
        )
        .map_err(sqlite_error)?;
        tick(cancellation)?;

        // 写入该章故事段
        for seg in kb.all_segments()? {
            if seg.chapter_id != chapter_id {
                continue;
            }
            tx.execute(
                "INSERT INTO story_segments(segment_id, number, chapter_id, summary, source_json, metadata_json)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(segment_id) DO UPDATE SET
                     number=excluded.number, chapter_id=excluded.chapter_id,
                     summary=excluded.summary, source_json=excluded.source_json,
                     metadata_json=excluded.metadata_json",
                params![
                    seg.segment_id,
                    seg.number,
                    seg.chapter_id,
                    seg.summary,
                    serde_json::to_string(&seg.source)?,
                    serde_json::to_string(&seg.metadata)?,
                ],
            )
            .map_err(sqlite_error)?;
        }
        tick(cancellation)?;

        // 写入涉及该章的事件（内存侧已含跨章合并结果）
        for event in kb.all_events()? {
            if !event.chapter_ids.iter().any(|id| id == chapter_id) {
                continue;
            }
            tx.execute(
                "INSERT INTO story_events(event_id, summary, status, metadata_json)
                 VALUES(?1,?2,?3,?4)
                 ON CONFLICT(event_id) DO UPDATE SET
                     summary=excluded.summary, status=excluded.status,
                     metadata_json=excluded.metadata_json",
                params![
                    event.event_id,
                    event.summary,
                    event_status_str(event.status),
                    serde_json::to_string(&event.metadata)?,
                ],
            )
            .map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM event_segment_links WHERE event_id=?1",
                params![event.event_id],
            )
            .map_err(sqlite_error)?;
            for seg_id in &event.segment_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO event_segment_links(event_id, segment_id) VALUES(?1,?2)",
                    params![event.event_id, seg_id],
                )
                .map_err(sqlite_error)?;
            }
            tx.execute(
                "DELETE FROM event_chapter_links WHERE event_id=?1",
                params![event.event_id],
            )
            .map_err(sqlite_error)?;
            for ch_id in &event.chapter_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO event_chapter_links(event_id, chapter_id) VALUES(?1,?2)",
                    params![event.event_id, ch_id],
                )
                .map_err(sqlite_error)?;
            }
        }
        tick(cancellation)?;

        // C2：工作集内的 Planner 变化与伏笔会被流水线更新；与章节实体、
        // confirmation 和 operation receipt 在同一事务写回，不能只更新内存。
        for change in kb.registered_changes()? {
            tx.execute(
                "INSERT INTO registered_changes(change_id, function, status, content_json, metadata_json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(change_id) DO UPDATE SET
                     function=excluded.function, status=excluded.status,
                     content_json=excluded.content_json, metadata_json=excluded.metadata_json",
                params![
                    change.change_id,
                    register_function_str(change.function),
                    register_status_str(change.status),
                    serde_json::to_string(&change.content)?,
                    serde_json::to_string(&change.metadata)?,
                ],
            )
            .map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM change_segment_links WHERE change_id = ?1",
                params![change.change_id],
            )
            .map_err(sqlite_error)?;
            for segment_id in &change.linked_segment_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO change_segment_links(change_id, segment_id) VALUES(?1,?2)",
                    params![change.change_id, segment_id],
                )
                .map_err(sqlite_error)?;
            }
        }
        for record in kb.all_foreshadowing()? {
            tx.execute(
                "INSERT INTO foreshadowing(foreshadowing_id, title, description, status, metadata_json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(foreshadowing_id) DO UPDATE SET
                     title=excluded.title, description=excluded.description,
                     status=excluded.status, metadata_json=excluded.metadata_json",
                params![
                    record.foreshadowing_id,
                    record.title,
                    record.description,
                    foreshadowing_status_str(record.status),
                    serde_json::to_string(&record.metadata)?,
                ],
            )
            .map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM foreshadowing_planted WHERE foreshadowing_id = ?1",
                params![record.foreshadowing_id],
            )
            .map_err(sqlite_error)?;
            for segment_id in &record.planted_segment_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO foreshadowing_planted(foreshadowing_id, segment_id) VALUES(?1,?2)",
                    params![record.foreshadowing_id, segment_id],
                )
                .map_err(sqlite_error)?;
            }
            tx.execute(
                "DELETE FROM foreshadowing_recovered WHERE foreshadowing_id = ?1",
                params![record.foreshadowing_id],
            )
            .map_err(sqlite_error)?;
            for segment_id in &record.recovered_segment_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO foreshadowing_recovered(foreshadowing_id, segment_id) VALUES(?1,?2)",
                    params![record.foreshadowing_id, segment_id],
                )
                .map_err(sqlite_error)?;
            }
        }
        tick(cancellation)?;

        // 章/阶段总结与问题、确认项（确认项按 revision 追加，ON CONFLICT 安全）
        if let Some(summary) = kb.chapter_summary(chapter_id)? {
            tx.execute(
                "INSERT INTO chapter_summaries(chapter_id, summary) VALUES(?1,?2)
                 ON CONFLICT(chapter_id) DO UPDATE SET summary=excluded.summary",
                params![chapter_id, summary],
            )
            .map_err(sqlite_error)?;
        }
        let idx = kb.index_snapshot()?;
        if let Some(stage_id) = idx.chapter_stage.get(chapter_id) {
            if let Some(stage_summary) = kb.stage_summary(stage_id)? {
                tx.execute(
                    "INSERT INTO stage_summaries(stage_id, summary) VALUES(?1,?2)
                     ON CONFLICT(stage_id) DO UPDATE SET summary=excluded.summary",
                    params![stage_id, stage_summary],
                )
                .map_err(sqlite_error)?;
            }
            tx.execute(
                "INSERT INTO chapter_stage(chapter_id, stage_id) VALUES(?1,?2)
                 ON CONFLICT(chapter_id) DO UPDATE SET stage_id=excluded.stage_id",
                params![chapter_id, stage_id],
            )
            .map_err(sqlite_error)?;
        }
        for issue in kb.planner_issues(chapter_id)? {
            tx.execute(
                "INSERT INTO planner_issues(issue_id, change_id, chapter_id, reason,
                     related_sources_json, planner_explanation, correction_patch_json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(issue_id) DO UPDATE SET
                     change_id=excluded.change_id, chapter_id=excluded.chapter_id,
                     reason=excluded.reason, related_sources_json=excluded.related_sources_json,
                     planner_explanation=excluded.planner_explanation,
                     correction_patch_json=excluded.correction_patch_json",
                params![
                    issue.issue_id,
                    issue.change_id,
                    issue.chapter_id,
                    issue.reason,
                    serde_json::to_string(&issue.related_sources)?,
                    issue.planner_explanation.as_deref(),
                    issue
                        .correction_patch
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()?,
                ],
            )
            .map_err(sqlite_error)?;
        }
        for item in kb.confirmations(None)? {
            let meta_chapter = item
                .metadata
                .get("chapter_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !meta_chapter.is_empty() && meta_chapter != chapter_id {
                continue;
            }
            tx.execute(
                "INSERT INTO writing_confirmations(confirmation_id, kind, state, prompt_key, metadata_json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(confirmation_id) DO UPDATE SET
                     kind=excluded.kind, state=excluded.state,
                     prompt_key=excluded.prompt_key, metadata_json=excluded.metadata_json",
                params![
                    item.confirmation_id,
                    confirmation_kind_str(item.kind),
                    confirmation_state_str(item.state),
                    item.prompt_key,
                    serde_json::to_string(&item.metadata)?,
                ],
            )
            .map_err(sqlite_error)?;
        }
        tick(cancellation)?;

        if let Some((operation_id, request_hash, response_json)) = operation {
            let response_json = serde_json::to_string(response_json)?;
            let changed = tx
                .execute(
                    "INSERT INTO knowledge_operations(operation_id, request_hash, response_json, committed_at_ms)
                     VALUES(?1,?2,?3,?4)
                     ON CONFLICT(operation_id) DO NOTHING",
                    params![
                        operation_id,
                        request_hash,
                        response_json,
                        unix_timestamp_ms_i64()?,
                    ],
                )
                .map_err(sqlite_error)?;
            if changed != 1 {
                return Err(CoreError::validation(format!(
                    "knowledge operation already exists: {operation_id}"
                )));
            }
        }

        tick(cancellation)?;
        tx.commit().map_err(sqlite_error)
    }

    fn save_knowledge_transaction(
        &self,
        kb: &MemoryWritingKnowledgeBase,
        operation: Option<(&str, &str, &Value)>,
        cancellation: &CancellationToken,
    ) -> CoreResult<()> {
        cancellation.check()?;
        let conn = self.connection.lock().map_err(lock_error)?;
        let tx = conn.unchecked_transaction().map_err(sqlite_error)?;

        // 故事段与事件代表当前 active revision；先清除旧快照，再在同一事务重建。
        // 审核确认历史使用独立不可变 id，不在这里清除。
        // 全量路径保留给非章节场景；Summarizer 使用 save_chapter_knowledge_*。
        tx.execute_batch(
            "DELETE FROM event_segment_links;
             DELETE FROM event_chapter_links;
             DELETE FROM story_events;
             DELETE FROM story_segments;",
        )
        .map_err(sqlite_error)?;

        // -- 故事段 --
        for seg in kb.all_segments()? {
            tx.execute(
                "INSERT INTO story_segments(segment_id, number, chapter_id, summary, source_json, metadata_json)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(segment_id) DO UPDATE SET
                     number=excluded.number, chapter_id=excluded.chapter_id,
                     summary=excluded.summary, source_json=excluded.source_json,
                     metadata_json=excluded.metadata_json",
                params![
                    seg.segment_id, seg.number, seg.chapter_id, seg.summary,
                    serde_json::to_string(&seg.source)?,
                    serde_json::to_string(&seg.metadata)?,
                ],
            ).map_err(sqlite_error)?;
        }
        cancellation.check()?;

        // -- 事件 + 关联表 --
        for event in kb.all_events()? {
            tx.execute(
                "INSERT INTO story_events(event_id, summary, status, metadata_json)
                 VALUES(?1,?2,?3,?4)
                 ON CONFLICT(event_id) DO UPDATE SET
                     summary=excluded.summary, status=excluded.status,
                     metadata_json=excluded.metadata_json",
                params![
                    event.event_id,
                    event.summary,
                    event_status_str(event.status),
                    serde_json::to_string(&event.metadata)?,
                ],
            )
            .map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM event_segment_links WHERE event_id=?1",
                params![event.event_id],
            )
            .map_err(sqlite_error)?;
            for seg_id in &event.segment_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO event_segment_links(event_id, segment_id) VALUES(?1,?2)",
                    params![event.event_id, seg_id],
                )
                .map_err(sqlite_error)?;
            }
            tx.execute(
                "DELETE FROM event_chapter_links WHERE event_id=?1",
                params![event.event_id],
            )
            .map_err(sqlite_error)?;
            for ch_id in &event.chapter_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO event_chapter_links(event_id, chapter_id) VALUES(?1,?2)",
                    params![event.event_id, ch_id],
                )
                .map_err(sqlite_error)?;
            }
        }
        cancellation.check()?;

        // -- 注册项 + 关联表 --
        for change in kb.registered_changes()? {
            tx.execute(
                "INSERT INTO registered_changes(change_id, function, status, content_json, metadata_json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(change_id) DO UPDATE SET
                     function=excluded.function, status=excluded.status,
                     content_json=excluded.content_json, metadata_json=excluded.metadata_json",
                params![
                    change.change_id,
                    register_function_str(change.function),
                    register_status_str(change.status),
                    serde_json::to_string(&change.content)?,
                    serde_json::to_string(&change.metadata)?,
                ],
            ).map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM change_segment_links WHERE change_id=?1",
                params![change.change_id],
            )
            .map_err(sqlite_error)?;
            for seg_id in &change.linked_segment_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO change_segment_links(change_id, segment_id) VALUES(?1,?2)",
                    params![change.change_id, seg_id],
                ).map_err(sqlite_error)?;
            }
        }
        cancellation.check()?;

        // -- 伏笔 + 种植/回收段 --
        for record in kb.all_foreshadowing()? {
            tx.execute(
                "INSERT INTO foreshadowing(foreshadowing_id, title, description, status, metadata_json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(foreshadowing_id) DO UPDATE SET
                     title=excluded.title, description=excluded.description,
                     status=excluded.status, metadata_json=excluded.metadata_json",
                params![
                    record.foreshadowing_id, record.title, record.description,
                    foreshadowing_status_str(record.status),
                    serde_json::to_string(&record.metadata)?,
                ],
            ).map_err(sqlite_error)?;
            tx.execute(
                "DELETE FROM foreshadowing_planted WHERE foreshadowing_id=?1",
                params![record.foreshadowing_id],
            )
            .map_err(sqlite_error)?;
            for seg_id in &record.planted_segment_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO foreshadowing_planted(foreshadowing_id, segment_id) VALUES(?1,?2)",
                    params![record.foreshadowing_id, seg_id],
                ).map_err(sqlite_error)?;
            }
            tx.execute(
                "DELETE FROM foreshadowing_recovered WHERE foreshadowing_id=?1",
                params![record.foreshadowing_id],
            )
            .map_err(sqlite_error)?;
            for seg_id in &record.recovered_segment_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO foreshadowing_recovered(foreshadowing_id, segment_id) VALUES(?1,?2)",
                    params![record.foreshadowing_id, seg_id],
                ).map_err(sqlite_error)?;
            }
        }
        cancellation.check()?;

        // -- 总结文本 --
        for (ch_id, summary) in kb.chapter_summaries()? {
            tx.execute(
                "INSERT INTO chapter_summaries(chapter_id, summary) VALUES(?1,?2)
                 ON CONFLICT(chapter_id) DO UPDATE SET summary=excluded.summary",
                params![ch_id, summary],
            )
            .map_err(sqlite_error)?;
        }
        for (stage_id, summary) in kb.stage_summaries()? {
            tx.execute(
                "INSERT INTO stage_summaries(stage_id, summary) VALUES(?1,?2)
                 ON CONFLICT(stage_id) DO UPDATE SET summary=excluded.summary",
                params![stage_id, summary],
            )
            .map_err(sqlite_error)?;
        }
        cancellation.check()?;

        // -- 章节-阶段归属（从索引快照取）--
        let idx = kb.index_snapshot()?;
        for (ch_id, stage_id) in &idx.chapter_stage {
            tx.execute(
                "INSERT INTO chapter_stage(chapter_id, stage_id) VALUES(?1,?2)
                 ON CONFLICT(chapter_id) DO UPDATE SET stage_id=excluded.stage_id",
                params![ch_id, stage_id],
            )
            .map_err(sqlite_error)?;
        }

        // -- Planner 问题 --
        for issue in kb.planner_issues("")? {
            tx.execute(
                "INSERT INTO planner_issues(issue_id, change_id, chapter_id, reason,
                     related_sources_json, planner_explanation, correction_patch_json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(issue_id) DO UPDATE SET
                     change_id=excluded.change_id, chapter_id=excluded.chapter_id,
                     reason=excluded.reason, related_sources_json=excluded.related_sources_json,
                     planner_explanation=excluded.planner_explanation,
                     correction_patch_json=excluded.correction_patch_json",
                params![
                    issue.issue_id,
                    issue.change_id,
                    issue.chapter_id,
                    issue.reason,
                    serde_json::to_string(&issue.related_sources)?,
                    issue.planner_explanation.as_deref(),
                    issue
                        .correction_patch
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()?,
                ],
            )
            .map_err(sqlite_error)?;
        }
        cancellation.check()?;

        // -- 确认项 --
        for item in kb.confirmations(None)? {
            tx.execute(
                "INSERT INTO writing_confirmations(confirmation_id, kind, state, prompt_key, metadata_json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(confirmation_id) DO UPDATE SET
                     kind=excluded.kind, state=excluded.state,
                     prompt_key=excluded.prompt_key, metadata_json=excluded.metadata_json",
                params![
                    item.confirmation_id,
                    confirmation_kind_str(item.kind),
                    confirmation_state_str(item.state),
                    item.prompt_key,
                    serde_json::to_string(&item.metadata)?,
                ],
            ).map_err(sqlite_error)?;
        }

        cancellation.check()?;
        if let Some((operation_id, request_hash, response_json)) = operation {
            let response_json = serde_json::to_string(response_json)?;
            let changed = tx
                .execute(
                    "INSERT INTO knowledge_operations(operation_id, request_hash, response_json, committed_at_ms)
                     VALUES(?1,?2,?3,?4)
                     ON CONFLICT(operation_id) DO NOTHING",
                    params![
                        operation_id,
                        request_hash,
                        response_json,
                        unix_timestamp_ms_i64()?,
                    ],
                )
                .map_err(sqlite_error)?;
            if changed != 1 {
                return Err(CoreError::validation(format!(
                    "knowledge operation already exists: {operation_id}"
                )));
            }
        }

        tx.commit().map_err(sqlite_error)
    }

    /// F15/F18：批量加载 Summarizer 生成阶段所需的历史上下文。
    ///
    /// 事件、关系、注册变化、伏笔和阶段历史均按表批量读取；不会在实体循环中发起
    /// SQL。任何 JSON、枚举或关系完整性错误都会在外部 LLM dispatch 前返回。
    pub fn load_summary_generation_context(
        &self,
        chapter_id: &str,
    ) -> CoreResult<SummaryGenerationContext> {
        if chapter_id.trim().is_empty() {
            return Err(CoreError::validation("chapter_id cannot be empty"));
        }

        let conn = self.connection.lock().map_err(lock_error)?;
        let mut all_segment_ids = BTreeSet::new();
        {
            let mut statement = conn
                .prepare("SELECT segment_id FROM story_segments ORDER BY segment_id")
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(sqlite_error)?;
            for row in rows {
                all_segment_ids.insert(row.map_err(sqlite_error)?);
            }
        }

        let mut event_segments = load_string_multimap(
            &conn,
            "SELECT event_id, segment_id FROM event_segment_links ORDER BY event_id, segment_id",
        )?;
        let mut event_chapters = load_string_multimap(
            &conn,
            "SELECT event_id, chapter_id FROM event_chapter_links ORDER BY event_id, chapter_id",
        )?;
        let mut existing_events = Vec::new();
        {
            let mut statement = conn
                .prepare(
                    "SELECT event_id, summary, status, metadata_json
                     FROM story_events ORDER BY event_id",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (event_id, summary, status, metadata_json) = row.map_err(sqlite_error)?;
                let event = StoryEvent {
                    segment_ids: event_segments.remove(&event_id).unwrap_or_default(),
                    chapter_ids: event_chapters.remove(&event_id).unwrap_or_default(),
                    event_id,
                    summary,
                    status: parse_event_status(&status)?,
                    metadata: serde_json::from_str(&metadata_json)?,
                };
                event.validate()?;
                if let Some(missing) = event
                    .segment_ids
                    .iter()
                    .find(|segment_id| !all_segment_ids.contains(*segment_id))
                {
                    return Err(CoreError::validation(format!(
                        "story event references missing segment: {missing}"
                    )));
                }
                existing_events.push(event);
            }
        }
        if let Some(orphan) = event_segments.keys().chain(event_chapters.keys()).next() {
            return Err(CoreError::validation(format!(
                "event relation references missing event: {orphan}"
            )));
        }

        let no_explicit_changes = BTreeSet::new();
        let planned_changes = load_summary_changes(&conn, chapter_id, &no_explicit_changes)?;
        for change in &planned_changes {
            change.validate()?;
            if let Some(missing) = change
                .linked_segment_ids
                .iter()
                .find(|segment_id| !all_segment_ids.contains(*segment_id))
            {
                return Err(CoreError::validation(format!(
                    "registered change references missing segment: {missing}"
                )));
            }
        }

        let mut planted_segments = load_string_multimap(
            &conn,
            "SELECT foreshadowing_id, segment_id FROM foreshadowing_planted
             ORDER BY foreshadowing_id, segment_id",
        )?;
        let mut recovered_segments = load_string_multimap(
            &conn,
            "SELECT foreshadowing_id, segment_id FROM foreshadowing_recovered
             ORDER BY foreshadowing_id, segment_id",
        )?;
        let mut foreshadowing = Vec::new();
        {
            let mut statement = conn
                .prepare(
                    "SELECT foreshadowing_id, title, description, status, metadata_json
                     FROM foreshadowing ORDER BY foreshadowing_id",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (foreshadowing_id, title, description, status, metadata_json) =
                    row.map_err(sqlite_error)?;
                if foreshadowing_id.trim().is_empty()
                    || title.trim().is_empty()
                    || description.trim().is_empty()
                {
                    return Err(CoreError::validation(
                        "foreshadowing record contains an empty required field",
                    ));
                }
                let record = ForeshadowingRecord {
                    planted_segment_ids: planted_segments
                        .remove(&foreshadowing_id)
                        .unwrap_or_default(),
                    recovered_segment_ids: recovered_segments
                        .remove(&foreshadowing_id)
                        .unwrap_or_default(),
                    foreshadowing_id,
                    title,
                    description,
                    status: parse_foreshadowing_status(&status)?,
                    metadata: serde_json::from_str(&metadata_json)?,
                };
                if let Some(missing) = record
                    .planted_segment_ids
                    .iter()
                    .chain(record.recovered_segment_ids.iter())
                    .find(|segment_id| !all_segment_ids.contains(*segment_id))
                {
                    return Err(CoreError::validation(format!(
                        "foreshadowing references missing segment: {missing}"
                    )));
                }
                foreshadowing.push(record);
            }
        }
        if let Some(orphan) = planted_segments
            .keys()
            .chain(recovered_segments.keys())
            .next()
        {
            return Err(CoreError::validation(format!(
                "foreshadowing relation references missing record: {orphan}"
            )));
        }

        let mut stages = BTreeMap::<String, SummaryStageContext>::new();
        {
            let mut statement = conn
                .prepare("SELECT stage_id, summary FROM stage_summaries ORDER BY stage_id")
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (stage_id, summary) = row.map_err(sqlite_error)?;
                if stage_id.trim().is_empty() || summary.trim().is_empty() {
                    return Err(CoreError::validation(
                        "stage summary contains an empty required field",
                    ));
                }
                stages.insert(
                    stage_id.clone(),
                    SummaryStageContext {
                        stage_id,
                        stage_summary: Some(summary),
                        chapter_summaries: BTreeMap::new(),
                    },
                );
            }
        }
        let mut current_stage_id = None;
        {
            let mut statement = conn
                .prepare(
                    "SELECT relation.stage_id, relation.chapter_id, summaries.summary
                     FROM chapter_stage relation
                     LEFT JOIN chapter_summaries summaries
                       ON summaries.chapter_id = relation.chapter_id
                     ORDER BY relation.stage_id, relation.chapter_id",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (stage_id, related_chapter_id, chapter_summary) = row.map_err(sqlite_error)?;
                if stage_id.trim().is_empty() || related_chapter_id.trim().is_empty() {
                    return Err(CoreError::validation(
                        "chapter-stage relation contains an empty id",
                    ));
                }
                if related_chapter_id == chapter_id {
                    current_stage_id = Some(stage_id.clone());
                }
                let stage = stages
                    .entry(stage_id.clone())
                    .or_insert_with(|| SummaryStageContext {
                        stage_id,
                        stage_summary: None,
                        chapter_summaries: BTreeMap::new(),
                    });
                if let Some(summary) = chapter_summary {
                    if summary.trim().is_empty() {
                        return Err(CoreError::validation(
                            "chapter summary contains an empty summary",
                        ));
                    }
                    stage.chapter_summaries.insert(related_chapter_id, summary);
                }
            }
        }

        Ok(SummaryGenerationContext {
            existing_events,
            planned_changes,
            foreshadowing,
            stages: stages.into_values().collect(),
            current_stage_id,
        })
    }

    /// F20：读取作品树唯一使用的正式章节-阶段关系。
    pub fn load_chapter_stage_map(&self) -> CoreResult<BTreeMap<String, String>> {
        let conn = self.connection.lock().map_err(lock_error)?;
        load_chapter_stage_map_from_connection(&conn)
    }

    /// F2-c：在同一 SQLite 读快照中装配四层已确认知识，供正式检索索引同步。
    pub fn load_retrieval_snapshot(&self) -> CoreResult<KnowledgeRetrievalSnapshot> {
        let conn = self.connection.lock().map_err(lock_error)?;
        let chapter_stage = load_chapter_stage_map_from_connection(&conn)?;
        let mut entries = BTreeMap::<String, KnowledgeRetrievalEntry>::new();
        let mut segment_sources = BTreeMap::<String, (String, SourceSpan)>::new();
        let mut chapter_sources = BTreeMap::<String, Vec<SourceSpan>>::new();

        {
            let mut statement = conn
                .prepare(
                    "SELECT segment_id, chapter_id, summary, source_json, metadata_json
                     FROM story_segments ORDER BY segment_id",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (segment_id, chapter_id, summary, source_json, metadata_json) =
                    row.map_err(sqlite_error)?;
                validate_non_empty_store("segment_id", &segment_id)?;
                validate_non_empty_store("chapter_id", &chapter_id)?;
                validate_non_empty_store("segment summary", &summary)?;
                let source = serde_json::from_str::<SourceSpan>(&source_json)?;
                let entity_metadata = serde_json::from_str::<Value>(&metadata_json)?;
                segment_sources.insert(segment_id.clone(), (chapter_id.clone(), source.clone()));
                chapter_sources
                    .entry(chapter_id.clone())
                    .or_default()
                    .push(source.clone());
                entries.insert(
                    format!("story_segment:{segment_id}"),
                    KnowledgeRetrievalEntry {
                        entity_id: segment_id,
                        layer: "story_segment".to_owned(),
                        text: summary,
                        sources: vec![source],
                        metadata: json!({
                            "chapter_id": chapter_id,
                            "stage_id": chapter_stage.get(&chapter_id),
                            "entity_metadata": entity_metadata,
                            "confirmed": true,
                        }),
                    },
                );
            }
        }

        let mut event_segments = load_string_multimap(
            &conn,
            "SELECT event_id, segment_id FROM event_segment_links ORDER BY event_id, segment_id",
        )?;
        let mut event_chapters = load_string_multimap(
            &conn,
            "SELECT event_id, chapter_id FROM event_chapter_links ORDER BY event_id, chapter_id",
        )?;
        {
            let mut statement = conn
                .prepare(
                    "SELECT event_id, summary, status, metadata_json
                     FROM story_events ORDER BY event_id",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (event_id, summary, status, metadata_json) = row.map_err(sqlite_error)?;
                validate_non_empty_store("event_id", &event_id)?;
                validate_non_empty_store("event summary", &summary)?;
                parse_event_status(&status)?;
                let segment_ids = event_segments.remove(&event_id).unwrap_or_default();
                let chapter_ids = event_chapters.remove(&event_id).unwrap_or_default();
                if segment_ids.is_empty() || chapter_ids.is_empty() {
                    return Err(CoreError::validation(format!(
                        "story event {event_id} has incomplete retrieval relations"
                    )));
                }
                let mut sources = Vec::new();
                for segment_id in &segment_ids {
                    let (_, source) = segment_sources.get(segment_id).ok_or_else(|| {
                        CoreError::validation(format!(
                            "story event {event_id} references missing segment {segment_id}"
                        ))
                    })?;
                    sources.push(source.clone());
                }
                let entity_metadata = serde_json::from_str::<Value>(&metadata_json)?;
                entries.insert(
                    format!("story_event:{event_id}"),
                    KnowledgeRetrievalEntry {
                        entity_id: event_id,
                        layer: "story_event".to_owned(),
                        text: summary,
                        sources: dedupe_sources(sources),
                        metadata: json!({
                            "status": status,
                            "segment_ids": segment_ids,
                            "chapter_ids": chapter_ids,
                            "entity_metadata": entity_metadata,
                            "confirmed": true,
                        }),
                    },
                );
            }
        }
        if let Some((event_id, _)) = event_segments.into_iter().next() {
            return Err(CoreError::validation(format!(
                "event-segment relation references missing event {event_id}"
            )));
        }
        if let Some((event_id, _)) = event_chapters.into_iter().next() {
            return Err(CoreError::validation(format!(
                "event-chapter relation references missing event {event_id}"
            )));
        }

        {
            let mut statement = conn
                .prepare("SELECT chapter_id, summary FROM chapter_summaries ORDER BY chapter_id")
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (chapter_id, summary) = row.map_err(sqlite_error)?;
                validate_non_empty_store("chapter_id", &chapter_id)?;
                validate_non_empty_store("chapter summary", &summary)?;
                entries.insert(
                    format!("chapter_summary:{chapter_id}"),
                    KnowledgeRetrievalEntry {
                        entity_id: chapter_id.clone(),
                        layer: "chapter_summary".to_owned(),
                        text: summary,
                        sources: dedupe_sources(
                            chapter_sources.get(&chapter_id).cloned().unwrap_or_default(),
                        ),
                        metadata: json!({
                            "chapter_id": chapter_id,
                            "stage_id": chapter_stage.get(&chapter_id),
                            "confirmed": true,
                        }),
                    },
                );
            }
        }

        let mut stage_sources = BTreeMap::<String, Vec<SourceSpan>>::new();
        for (chapter_id, stage_id) in &chapter_stage {
            if let Some(sources) = chapter_sources.get(chapter_id) {
                stage_sources
                    .entry(stage_id.clone())
                    .or_default()
                    .extend(sources.iter().cloned());
            }
        }
        {
            let mut statement = conn
                .prepare("SELECT stage_id, summary FROM stage_summaries ORDER BY stage_id")
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (stage_id, summary) = row.map_err(sqlite_error)?;
                validate_non_empty_store("stage_id", &stage_id)?;
                validate_non_empty_store("stage summary", &summary)?;
                let chapter_ids = chapter_stage
                    .iter()
                    .filter_map(|(chapter_id, related_stage_id)| {
                        (related_stage_id == &stage_id).then_some(chapter_id.clone())
                    })
                    .collect::<Vec<_>>();
                entries.insert(
                    format!("stage_summary:{stage_id}"),
                    KnowledgeRetrievalEntry {
                        entity_id: stage_id.clone(),
                        layer: "stage_summary".to_owned(),
                        text: summary,
                        sources: dedupe_sources(
                            stage_sources.get(&stage_id).cloned().unwrap_or_default(),
                        ),
                        metadata: json!({
                            "stage_id": stage_id,
                            "chapter_ids": chapter_ids,
                            "confirmed": true,
                        }),
                    },
                );
            }
        }

        let entries = entries.into_values().collect::<Vec<_>>();
        let revision = format!("{:x}", Sha256::digest(serde_json::to_vec(&entries)?));
        Ok(KnowledgeRetrievalSnapshot { revision, entries })
    }

    /// F19/F20：批量读取作品页章节总结投影。
    ///
    /// 正式知识与确认历史分开装配；Pending payload 不进入 active 实体。任何损坏的
    /// JSON、枚举或悬空关系都会直接失败，避免作品页把损坏数据显示成空态。
    pub fn load_chapter_summary_view(&self, chapter_id: &str) -> CoreResult<ChapterSummaryView> {
        if chapter_id.trim().is_empty() {
            return Err(CoreError::validation("chapter_id cannot be empty"));
        }

        let conn = self.connection.lock().map_err(lock_error)?;
        let chapter_summary = conn
            .query_row(
                "SELECT summary FROM chapter_summaries WHERE chapter_id = ?1",
                params![chapter_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        if chapter_summary
            .as_deref()
            .is_some_and(|summary| summary.trim().is_empty())
        {
            return Err(CoreError::validation(
                "chapter summary contains an empty summary",
            ));
        }

        let chapter_stage = load_chapter_stage_map_from_connection(&conn)?;
        let stage = if let Some(stage_id) = chapter_stage.get(chapter_id) {
            let summary = conn
                .query_row(
                    "SELECT summary FROM stage_summaries WHERE stage_id = ?1",
                    params![stage_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(sqlite_error)?;
            if summary
                .as_deref()
                .is_some_and(|summary| summary.trim().is_empty())
            {
                return Err(CoreError::validation(
                    "stage summary contains an empty summary",
                ));
            }
            let chapter_ids = chapter_stage
                .iter()
                .filter(|(_, related_stage_id)| *related_stage_id == stage_id)
                .map(|(related_chapter_id, _)| related_chapter_id.clone())
                .collect();
            Some(ChapterStageSummaryView {
                stage_id: stage_id.clone(),
                summary,
                chapter_ids,
            })
        } else {
            None
        };

        let mut numbered_segments = Vec::new();
        {
            let mut statement = conn
                .prepare(
                    "SELECT segment_id, number, chapter_id, summary, source_json, metadata_json
                     FROM story_segments WHERE chapter_id = ?1",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(params![chapter_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (segment_id, number, persisted_chapter, summary, source_json, metadata_json) =
                    row.map_err(sqlite_error)?;
                let source: crate::contracts::SourceSpan = serde_json::from_str(&source_json)?;
                if source.document_id.trim().is_empty()
                    || source.range.is_empty()
                    || source.range.start > source.range.end
                    || source
                        .version
                        .as_deref()
                        .is_some_and(|version| version.trim().is_empty())
                {
                    return Err(CoreError::validation(format!(
                        "story segment contains an invalid source span: {segment_id}"
                    )));
                }
                let segment = StorySegment {
                    segment_id,
                    number: number.clone(),
                    chapter_id: persisted_chapter,
                    summary,
                    source,
                    metadata: serde_json::from_str(&metadata_json)?,
                };
                segment.validate()?;
                numbered_segments.push((parse_segment_number(&number)?, segment));
            }
        }
        numbered_segments.sort_by(|(left, _), (right, _)| left.cmp(right));
        let segments = numbered_segments
            .into_iter()
            .map(|(_, segment)| segment)
            .collect::<Vec<_>>();
        let segment_ids = segments
            .iter()
            .map(|segment| segment.segment_id.clone())
            .collect::<BTreeSet<_>>();

        let event_ids = load_distinct_ids_for_values(
            &conn,
            "SELECT DISTINCT event_id FROM event_chapter_links WHERE chapter_id IN",
            &BTreeSet::from([chapter_id.to_owned()]),
        )?;
        let mut event_segments = load_string_multimap_for_keys(
            &conn,
            "SELECT event_id, segment_id FROM event_segment_links WHERE event_id IN",
            &event_ids,
        )?;
        let mut event_chapters = load_string_multimap_for_keys(
            &conn,
            "SELECT event_id, chapter_id FROM event_chapter_links WHERE event_id IN",
            &event_ids,
        )?;
        let mut referenced_segment_ids = event_segments
            .values()
            .flat_map(|ids| ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        let mut events = Vec::new();
        for (event_id, summary, status, metadata_json) in
            load_event_rows_for_ids(&conn, &event_ids)?
        {
            let event = StoryEvent {
                segment_ids: event_segments.remove(&event_id).unwrap_or_default(),
                chapter_ids: event_chapters.remove(&event_id).unwrap_or_default(),
                event_id,
                summary,
                status: parse_event_status(&status)?,
                metadata: serde_json::from_str(&metadata_json)?,
            };
            event.validate()?;
            if !event.chapter_ids.iter().any(|id| id == chapter_id) {
                return Err(CoreError::validation(format!(
                    "story event is missing requested chapter relation: {}",
                    event.event_id
                )));
            }
            events.push(event);
        }
        reject_missing_entities(
            "event",
            &event_ids,
            events.iter().map(|event| &event.event_id),
        )?;
        if let Some(orphan) = event_segments.keys().chain(event_chapters.keys()).next() {
            return Err(CoreError::validation(format!(
                "event relation references missing event: {orphan}"
            )));
        }

        let change_ids = load_distinct_ids_for_values(
            &conn,
            "SELECT DISTINCT change_id FROM change_segment_links WHERE segment_id IN",
            &segment_ids,
        )?;
        let realized_changes = load_registered_changes_for_ids(&conn, &change_ids)?;
        for change in &realized_changes {
            if change.status != RegisteredChangeStatus::Realized {
                return Err(CoreError::validation(format!(
                    "change linked to chapter segment is not realized: {}",
                    change.change_id
                )));
            }
            referenced_segment_ids.extend(change.linked_segment_ids.iter().cloned());
        }

        let planted_ids = load_distinct_ids_for_values(
            &conn,
            "SELECT DISTINCT foreshadowing_id FROM foreshadowing_planted WHERE segment_id IN",
            &segment_ids,
        )?;
        let recovered_ids = load_distinct_ids_for_values(
            &conn,
            "SELECT DISTINCT foreshadowing_id FROM foreshadowing_recovered WHERE segment_id IN",
            &segment_ids,
        )?;
        let foreshadowing_ids = planted_ids
            .union(&recovered_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut planted_segments = load_string_multimap_for_keys(
            &conn,
            "SELECT foreshadowing_id, segment_id FROM foreshadowing_planted WHERE foreshadowing_id IN",
            &foreshadowing_ids,
        )?;
        let mut recovered_segments = load_string_multimap_for_keys(
            &conn,
            "SELECT foreshadowing_id, segment_id FROM foreshadowing_recovered WHERE foreshadowing_id IN",
            &foreshadowing_ids,
        )?;
        let mut foreshadowing = Vec::new();
        for (foreshadowing_id, title, description, status, metadata_json) in
            load_foreshadowing_rows_for_ids(&conn, &foreshadowing_ids)?
        {
            if foreshadowing_id.trim().is_empty()
                || title.trim().is_empty()
                || description.trim().is_empty()
            {
                return Err(CoreError::validation(
                    "foreshadowing record contains an empty required field",
                ));
            }
            let record = ForeshadowingRecord {
                planted_segment_ids: planted_segments
                    .remove(&foreshadowing_id)
                    .unwrap_or_default(),
                recovered_segment_ids: recovered_segments
                    .remove(&foreshadowing_id)
                    .unwrap_or_default(),
                foreshadowing_id,
                title,
                description,
                status: parse_foreshadowing_status(&status)?,
                metadata: serde_json::from_str(&metadata_json)?,
            };
            referenced_segment_ids.extend(record.planted_segment_ids.iter().cloned());
            referenced_segment_ids.extend(record.recovered_segment_ids.iter().cloned());
            foreshadowing.push(record);
        }
        reject_missing_entities(
            "foreshadowing",
            &foreshadowing_ids,
            foreshadowing.iter().map(|record| &record.foreshadowing_id),
        )?;
        if let Some(orphan) = planted_segments
            .keys()
            .chain(recovered_segments.keys())
            .next()
        {
            return Err(CoreError::validation(format!(
                "foreshadowing relation references missing record: {orphan}"
            )));
        }

        if !referenced_segment_ids.is_empty() {
            let existing_segment_ids = load_segment_rows_for_ids(&conn, &referenced_segment_ids)?
                .into_iter()
                .map(|row| row.0)
                .collect::<BTreeSet<_>>();
            if let Some(missing) = referenced_segment_ids
                .difference(&existing_segment_ids)
                .next()
            {
                return Err(CoreError::validation(format!(
                    "summary relation references missing segment: {missing}"
                )));
            }
        }

        let mut confirmations = Vec::new();
        {
            let mut statement = conn
                .prepare(
                    "SELECT confirmation_id, kind, state, metadata_json
                     FROM writing_confirmations
                     WHERE kind IN ('segment_summary','event_summary','chapter_summary','stage_summary')
                     ORDER BY confirmation_id",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (confirmation_id, kind, state, metadata_json) = row.map_err(sqlite_error)?;
                let metadata: Value = serde_json::from_str(&metadata_json)?;
                let related_chapter_id = metadata
                    .get("chapter_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CoreError::validation(format!(
                            "summary confirmation is missing chapter_id: {confirmation_id}"
                        ))
                    })?;
                if related_chapter_id != chapter_id {
                    continue;
                }
                let revision_id = metadata
                    .get("revision_id")
                    .map(|value| {
                        value.as_str().map(str::to_owned).ok_or_else(|| {
                            CoreError::validation(format!(
                                "summary confirmation has invalid revision_id: {confirmation_id}"
                            ))
                        })
                    })
                    .transpose()?;
                confirmations.push(ChapterSummaryConfirmationView {
                    confirmation_id,
                    kind: parse_confirmation_kind(&kind)?,
                    state: parse_confirmation_state(&state)?,
                    revision_id,
                });
            }
        }

        Ok(ChapterSummaryView {
            chapter_id: chapter_id.to_owned(),
            chapter_summary,
            stage,
            segments,
            events,
            realized_changes,
            foreshadowing,
            confirmations,
        })
    }

    /// C2：仅加载 Summarizer 提交所需的关系闭包。
    ///
    /// 工作集包含当前章、与当前章/草稿事件相连的跨章实体、仍待核对的 Planner
    /// 变化、草稿引用的伏笔、当前/目标阶段和全局 Pending 确认；无关章节正文实体
    /// 不进入内存，也不随每章总结线性增长。
    pub fn load_summary_working_set(
        &self,
        chapter_id: &str,
        draft: Option<&SummaryPipelineDraft>,
    ) -> CoreResult<MemoryWritingKnowledgeBase> {
        if chapter_id.trim().is_empty() {
            return Err(CoreError::validation("chapter_id cannot be empty"));
        }
        if draft.is_some_and(|draft| draft.chapter_id != chapter_id) {
            return Err(CoreError::validation(
                "summary working set chapter does not match draft",
            ));
        }

        let conn = self.connection.lock().map_err(lock_error)?;
        let kb = MemoryWritingKnowledgeBase::new();
        let mut segment_ids = BTreeSet::new();
        let mut event_ids = BTreeSet::new();

        {
            let mut statement = conn
                .prepare("SELECT segment_id FROM story_segments WHERE chapter_id = ?1")
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(params![chapter_id], |row| row.get::<_, String>(0))
                .map_err(sqlite_error)?;
            for row in rows {
                segment_ids.insert(row.map_err(sqlite_error)?);
            }
        }
        {
            let mut statement = conn
                .prepare("SELECT event_id FROM event_chapter_links WHERE chapter_id = ?1")
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(params![chapter_id], |row| row.get::<_, String>(0))
                .map_err(sqlite_error)?;
            for row in rows {
                event_ids.insert(row.map_err(sqlite_error)?);
            }
        }
        if let Some(draft) = draft {
            segment_ids.extend(
                draft
                    .segments
                    .iter()
                    .map(|segment| segment.segment_id.clone()),
            );
            event_ids.extend(draft.events.iter().map(|event| event.event_id.clone()));
        }

        let mut event_segments = load_string_multimap_for_keys(
            &conn,
            "SELECT event_id, segment_id FROM event_segment_links WHERE event_id IN",
            &event_ids,
        )?;
        let mut event_chapters = load_string_multimap_for_keys(
            &conn,
            "SELECT event_id, chapter_id FROM event_chapter_links WHERE event_id IN",
            &event_ids,
        )?;
        for ids in event_segments.values() {
            segment_ids.extend(ids.iter().cloned());
        }

        for (segment_id, number, persisted_chapter, summary, source_json, metadata_json) in
            load_segment_rows_for_ids(&conn, &segment_ids)?
        {
            if draft.is_some_and(|draft| {
                draft
                    .segments
                    .iter()
                    .any(|segment| segment.segment_id == segment_id)
            }) && persisted_chapter != chapter_id
            {
                return Err(CoreError::validation(format!(
                    "summary segment id belongs to another chapter: {segment_id}"
                )));
            }
            kb.upsert_segment(StorySegment {
                segment_id,
                number,
                chapter_id: persisted_chapter,
                summary,
                source: serde_json::from_str(&source_json)?,
                metadata: serde_json::from_str(&metadata_json)?,
            })?;
        }

        for (event_id, summary, status, metadata_json) in
            load_event_rows_for_ids(&conn, &event_ids)?
        {
            kb.upsert_event(StoryEvent {
                segment_ids: event_segments.remove(&event_id).unwrap_or_default(),
                chapter_ids: event_chapters.remove(&event_id).unwrap_or_default(),
                event_id,
                summary,
                status: parse_event_status(&status)?,
                metadata: serde_json::from_str(&metadata_json)?,
            })?;
        }

        let draft_change_ids = draft
            .into_iter()
            .flat_map(|draft| draft.realized_changes.iter())
            .map(|change| change.change_id.clone())
            .collect::<BTreeSet<_>>();
        for change in load_summary_changes(&conn, chapter_id, &draft_change_ids)? {
            kb.upsert_registered_change(change)?;
        }

        let foreshadowing_ids = draft
            .into_iter()
            .flat_map(|draft| draft.foreshadowing_updates.iter())
            .map(|update| update.foreshadowing_id.clone())
            .collect::<BTreeSet<_>>();
        let mut planted_segments = load_string_multimap_for_keys(
            &conn,
            "SELECT foreshadowing_id, segment_id FROM foreshadowing_planted WHERE foreshadowing_id IN",
            &foreshadowing_ids,
        )?;
        let mut recovered_segments = load_string_multimap_for_keys(
            &conn,
            "SELECT foreshadowing_id, segment_id FROM foreshadowing_recovered WHERE foreshadowing_id IN",
            &foreshadowing_ids,
        )?;
        for (foreshadowing_id, title, description, status, metadata_json) in
            load_foreshadowing_rows_for_ids(&conn, &foreshadowing_ids)?
        {
            kb.upsert_foreshadowing(ForeshadowingRecord {
                planted_segment_ids: planted_segments
                    .remove(&foreshadowing_id)
                    .unwrap_or_default(),
                recovered_segment_ids: recovered_segments
                    .remove(&foreshadowing_id)
                    .unwrap_or_default(),
                foreshadowing_id,
                title,
                description,
                status: parse_foreshadowing_status(&status)?,
                metadata: serde_json::from_str(&metadata_json)?,
            })?;
        }

        if let Some(summary) = conn
            .query_row(
                "SELECT summary FROM chapter_summaries WHERE chapter_id = ?1",
                params![chapter_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?
        {
            kb.upsert_chapter_summary(chapter_id, summary)?;
        }
        let current_stage = conn
            .query_row(
                "SELECT stage_id FROM chapter_stage WHERE chapter_id = ?1",
                params![chapter_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        let mut stage_ids = current_stage.iter().cloned().collect::<BTreeSet<_>>();
        if let Some(stage_id) = draft.and_then(|draft| draft.stage_id.as_ref()) {
            stage_ids.insert(stage_id.clone());
        }
        for stage_id in &stage_ids {
            if let Some(summary) = conn
                .query_row(
                    "SELECT summary FROM stage_summaries WHERE stage_id = ?1",
                    params![stage_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(sqlite_error)?
            {
                kb.upsert_stage_summary(stage_id, summary)?;
            }
        }
        if let Some(stage_id) = current_stage {
            kb.link_chapter_stage(chapter_id, &stage_id)?;
        }

        {
            let mut statement = conn
                .prepare(
                    "SELECT issue_id, change_id, chapter_id, reason,
                            related_sources_json, planner_explanation, correction_patch_json
                     FROM planner_issues WHERE chapter_id = ?1",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(params![chapter_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (issue_id, change_id, chapter_id, reason, sources, explanation, patch) =
                    row.map_err(sqlite_error)?;
                kb.add_planner_issue(PlannerIssue {
                    issue_id,
                    change_id,
                    chapter_id,
                    reason,
                    related_sources: serde_json::from_str(&sources)?,
                    planner_explanation: explanation,
                    correction_patch: patch.as_deref().map(serde_json::from_str).transpose()?,
                })?;
            }
        }
        {
            let mut statement = conn
                .prepare(
                    "SELECT confirmation_id, kind, state, prompt_key, metadata_json
                     FROM writing_confirmations WHERE state = 'pending'",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (confirmation_id, kind, state, prompt_key, metadata_json) =
                    row.map_err(sqlite_error)?;
                kb.upsert_confirmation(ConfirmationItem {
                    confirmation_id,
                    kind: parse_confirmation_kind(&kind)?,
                    state: parse_confirmation_state(&state)?,
                    prompt_key,
                    metadata: serde_json::from_str(&metadata_json)?,
                })?;
            }
        }
        Ok(kb)
    }

    /// 从数据库加载所有源实体，重放 upsert 重建内存知识库（含所有双向索引）。
    pub fn load_knowledge(&self) -> CoreResult<MemoryWritingKnowledgeBase> {
        let conn = self.connection.lock().map_err(lock_error)?;
        let kb = MemoryWritingKnowledgeBase::new();

        // -- 故事段 --
        {
            let mut stmt = conn
                .prepare(
                    "SELECT segment_id, number, chapter_id, summary, source_json, metadata_json
                 FROM story_segments ORDER BY chapter_id, number",
                )
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (segment_id, number, chapter_id, summary, source_json, metadata_json) =
                    row.map_err(sqlite_error)?;
                kb.upsert_segment(StorySegment {
                    segment_id,
                    number,
                    chapter_id,
                    summary,
                    source: serde_json::from_str(&source_json)?,
                    metadata: serde_json::from_str(&metadata_json)
                        .unwrap_or(serde_json::Value::Null),
                })?;
            }
        }

        // -- 事件 + 关联段/章节 --
        {
            let mut event_segments = load_string_multimap(
                &conn,
                "SELECT event_id, segment_id FROM event_segment_links ORDER BY event_id, segment_id",
            )?;
            let mut event_chapters = load_string_multimap(
                &conn,
                "SELECT event_id, chapter_id FROM event_chapter_links ORDER BY event_id, chapter_id",
            )?;
            let mut stmt = conn
                .prepare("SELECT event_id, summary, status, metadata_json FROM story_events")
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (event_id, summary, status_str, metadata_json) = row.map_err(sqlite_error)?;
                let segment_ids = event_segments.remove(&event_id).unwrap_or_default();
                let chapter_ids = event_chapters.remove(&event_id).unwrap_or_default();
                kb.upsert_event(StoryEvent {
                    event_id,
                    summary,
                    status: parse_event_status(&status_str)?,
                    segment_ids,
                    chapter_ids,
                    metadata: serde_json::from_str(&metadata_json)
                        .unwrap_or(serde_json::Value::Null),
                })?;
            }
        }

        // -- 注册项 + 关联段 --
        {
            let mut change_segments = load_string_multimap(
                &conn,
                "SELECT change_id, segment_id FROM change_segment_links ORDER BY change_id, segment_id",
            )?;
            let mut stmt = conn
                .prepare(
                    "SELECT change_id, function, status, content_json, metadata_json
                 FROM registered_changes",
                )
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (change_id, function_str, status_str, content_json, metadata_json) =
                    row.map_err(sqlite_error)?;
                let linked_segment_ids = change_segments.remove(&change_id).unwrap_or_default();
                let content: RegisterContent = serde_json::from_str(&content_json)?;
                kb.upsert_registered_change(crate::rag::models::RegisteredChange {
                    change_id,
                    function: parse_register_function(&function_str)?,
                    status: parse_register_status(&status_str)?,
                    content,
                    linked_segment_ids,
                    metadata: serde_json::from_str(&metadata_json)
                        .unwrap_or(serde_json::Value::Null),
                })?;
            }
        }

        // -- 伏笔 + 种植/回收段 --
        {
            let mut planted_segments = load_string_multimap(
                &conn,
                "SELECT foreshadowing_id, segment_id FROM foreshadowing_planted ORDER BY foreshadowing_id, segment_id",
            )?;
            let mut recovered_segments = load_string_multimap(
                &conn,
                "SELECT foreshadowing_id, segment_id FROM foreshadowing_recovered ORDER BY foreshadowing_id, segment_id",
            )?;
            let mut stmt = conn
                .prepare(
                    "SELECT foreshadowing_id, title, description, status, metadata_json
                 FROM foreshadowing",
                )
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (fid, title, description, status_str, metadata_json) =
                    row.map_err(sqlite_error)?;
                let planted = planted_segments.remove(&fid).unwrap_or_default();
                let recovered = recovered_segments.remove(&fid).unwrap_or_default();
                kb.upsert_foreshadowing(ForeshadowingRecord {
                    foreshadowing_id: fid,
                    title,
                    description,
                    status: parse_foreshadowing_status(&status_str)?,
                    planted_segment_ids: planted,
                    recovered_segment_ids: recovered,
                    metadata: serde_json::from_str(&metadata_json)
                        .unwrap_or(serde_json::Value::Null),
                })?;
            }
        }

        // -- 总结文本 --
        {
            let mut stmt = conn
                .prepare("SELECT chapter_id, summary FROM chapter_summaries")
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (ch_id, summary) = row.map_err(sqlite_error)?;
                kb.upsert_chapter_summary(ch_id, summary)?;
            }
        }
        {
            let mut stmt = conn
                .prepare("SELECT stage_id, summary FROM stage_summaries")
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (stage_id, summary) = row.map_err(sqlite_error)?;
                kb.upsert_stage_summary(stage_id, summary)?;
            }
        }

        // -- 章节-阶段归属 --
        {
            let mut stmt = conn
                .prepare("SELECT chapter_id, stage_id FROM chapter_stage")
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (ch_id, stage_id) = row.map_err(sqlite_error)?;
                kb.link_chapter_stage(&ch_id, &stage_id)?;
            }
        }

        // -- Planner 问题 --
        {
            let mut stmt = conn
                .prepare(
                    "SELECT issue_id, change_id, chapter_id, reason,
                        related_sources_json, planner_explanation, correction_patch_json
                 FROM planner_issues",
                )
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (
                    issue_id,
                    change_id,
                    chapter_id,
                    reason,
                    sources_json,
                    explanation,
                    patch_json,
                ) = row.map_err(sqlite_error)?;
                kb.add_planner_issue(PlannerIssue {
                    issue_id,
                    change_id,
                    chapter_id,
                    reason,
                    related_sources: serde_json::from_str(&sources_json).unwrap_or_default(),
                    planner_explanation: explanation,
                    correction_patch: patch_json
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()?,
                })?;
            }
        }

        // -- 确认项 --
        {
            let mut stmt = conn
                .prepare(
                    "SELECT confirmation_id, kind, state, prompt_key, metadata_json
                 FROM writing_confirmations",
                )
                .map_err(sqlite_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(sqlite_error)?;
            for row in rows {
                let (conf_id, kind_str, state_str, prompt_key, metadata_json) =
                    row.map_err(sqlite_error)?;
                kb.upsert_confirmation(ConfirmationItem {
                    confirmation_id: conf_id,
                    kind: parse_confirmation_kind(&kind_str)?,
                    state: parse_confirmation_state(&state_str)?,
                    prompt_key,
                    metadata: serde_json::from_str(&metadata_json)
                        .unwrap_or(serde_json::Value::Null),
                })?;
            }
        }

        Ok(kb)
    }
}

fn knowledge_writer_lock_path(db_path: &Path) -> CoreResult<PathBuf> {
    let canonical = if let Ok(canonical) = db_path.canonicalize() {
        canonical
    } else if let (Some(parent), Some(file_name)) = (db_path.parent(), db_path.file_name()) {
        parent
            .canonicalize()
            .map(|parent| parent.join(file_name))
            .unwrap_or_else(|_| db_path.to_path_buf())
    } else {
        db_path.to_path_buf()
    };
    let mut digest = Sha256::new();
    digest.update(b"ariadne-knowledge-writer-v1\0");
    digest.update(knowledge_path_identity_bytes(&canonical));
    Ok(std::env::temp_dir()
        .join("ariadne-knowledge-write-locks")
        .join(format!("{:x}.lock", digest.finalize())))
}

#[cfg(unix)]
fn knowledge_path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = b"unix\0".to_vec();
    bytes.extend_from_slice(path.as_os_str().as_bytes());
    bytes
}

#[cfg(windows)]
fn knowledge_path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut bytes = b"windows\0".to_vec();
    for unit in path.as_os_str().encode_wide() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

// ── 枚举字符串编解码 ────────────────────────────────────────────────────────

fn event_status_str(s: StoryEventStatus) -> &'static str {
    match s {
        StoryEventStatus::Ongoing => "ongoing",
        StoryEventStatus::Paused => "paused",
        StoryEventStatus::Completed => "completed",
    }
}
fn parse_event_status(s: &str) -> CoreResult<StoryEventStatus> {
    match s {
        "ongoing" => Ok(StoryEventStatus::Ongoing),
        "paused" => Ok(StoryEventStatus::Paused),
        "completed" => Ok(StoryEventStatus::Completed),
        _ => Err(CoreError::validation(format!(
            "unknown story event status in knowledge store: {s}"
        ))),
    }
}

fn register_function_str(f: RegisterFunction) -> &'static str {
    match f {
        RegisterFunction::CharacterProfile => "character_profile",
        RegisterFunction::CharacterPlan => "character_plan",
        RegisterFunction::CharacterTrait => "character_trait",
        RegisterFunction::Relationship => "relationship",
        RegisterFunction::Foreshadowing => "foreshadowing",
        RegisterFunction::ThemeAnchor => "theme_anchor",
    }
}
fn parse_register_function(s: &str) -> CoreResult<RegisterFunction> {
    match s {
        "character_profile" => Ok(RegisterFunction::CharacterProfile),
        "character_plan" => Ok(RegisterFunction::CharacterPlan),
        "character_trait" => Ok(RegisterFunction::CharacterTrait),
        "relationship" => Ok(RegisterFunction::Relationship),
        "foreshadowing" => Ok(RegisterFunction::Foreshadowing),
        "theme_anchor" => Ok(RegisterFunction::ThemeAnchor),
        _ => Err(CoreError::validation(format!(
            "unknown register function in knowledge store: {s}"
        ))),
    }
}

fn register_status_str(s: RegisteredChangeStatus) -> &'static str {
    match s {
        RegisteredChangeStatus::Planned => "planned",
        RegisteredChangeStatus::Realized => "realized",
        RegisteredChangeStatus::Deleted => "deleted",
    }
}
fn parse_register_status(s: &str) -> CoreResult<RegisteredChangeStatus> {
    match s {
        "planned" => Ok(RegisteredChangeStatus::Planned),
        "realized" => Ok(RegisteredChangeStatus::Realized),
        "deleted" => Ok(RegisteredChangeStatus::Deleted),
        _ => Err(CoreError::validation(format!(
            "unknown register status in knowledge store: {s}"
        ))),
    }
}

fn foreshadowing_status_str(s: ForeshadowingStatus) -> &'static str {
    match s {
        ForeshadowingStatus::Planned => "planned",
        ForeshadowingStatus::Planted => "planted",
        ForeshadowingStatus::Recovered => "recovered",
        ForeshadowingStatus::Abandoned => "abandoned",
    }
}
fn parse_foreshadowing_status(s: &str) -> CoreResult<ForeshadowingStatus> {
    match s {
        "planned" => Ok(ForeshadowingStatus::Planned),
        "planted" => Ok(ForeshadowingStatus::Planted),
        "recovered" => Ok(ForeshadowingStatus::Recovered),
        "abandoned" => Ok(ForeshadowingStatus::Abandoned),
        _ => Err(CoreError::validation(format!(
            "unknown foreshadowing status in knowledge store: {s}"
        ))),
    }
}

fn load_string_multimap(
    connection: &Connection,
    sql: &str,
) -> CoreResult<BTreeMap<String, Vec<String>>> {
    let mut statement = connection.prepare(sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(sqlite_error)?;
    let mut values = BTreeMap::<String, Vec<String>>::new();
    for row in rows {
        let (owner_id, value_id) = row.map_err(sqlite_error)?;
        values.entry(owner_id).or_default().push(value_id);
    }
    Ok(values)
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

fn load_string_multimap_for_keys(
    connection: &Connection,
    sql_prefix: &str,
    keys: &BTreeSet<String>,
) -> CoreResult<BTreeMap<String, Vec<String>>> {
    if keys.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "{sql_prefix} ({}) ORDER BY 1, 2",
        sql_placeholders(keys.len())
    );
    let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(keys.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(sqlite_error)?;
    let mut values = BTreeMap::<String, Vec<String>>::new();
    for row in rows {
        let (owner_id, value_id) = row.map_err(sqlite_error)?;
        values.entry(owner_id).or_default().push(value_id);
    }
    Ok(values)
}

type SegmentRow = (String, String, String, String, String, String);
type EventRow = (String, String, String, String);
type ForeshadowingRow = (String, String, String, String, String);

fn load_segment_rows_for_ids(
    connection: &Connection,
    ids: &BTreeSet<String>,
) -> CoreResult<Vec<SegmentRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT segment_id, number, chapter_id, summary, source_json, metadata_json
         FROM story_segments WHERE segment_id IN ({}) ORDER BY chapter_id, number",
        sql_placeholders(ids.len())
    );
    let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(sqlite_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
}

fn load_event_rows_for_ids(
    connection: &Connection,
    ids: &BTreeSet<String>,
) -> CoreResult<Vec<EventRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT event_id, summary, status, metadata_json FROM story_events
         WHERE event_id IN ({}) ORDER BY event_id",
        sql_placeholders(ids.len())
    );
    let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(sqlite_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
}

fn load_foreshadowing_rows_for_ids(
    connection: &Connection,
    ids: &BTreeSet<String>,
) -> CoreResult<Vec<ForeshadowingRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT foreshadowing_id, title, description, status, metadata_json
         FROM foreshadowing WHERE foreshadowing_id IN ({}) ORDER BY foreshadowing_id",
        sql_placeholders(ids.len())
    );
    let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(sqlite_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
}

fn load_chapter_stage_map_from_connection(
    connection: &Connection,
) -> CoreResult<BTreeMap<String, String>> {
    let mut statement = connection
        .prepare("SELECT chapter_id, stage_id FROM chapter_stage ORDER BY chapter_id")
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(sqlite_error)?;
    let mut chapter_stage = BTreeMap::new();
    for row in rows {
        let (chapter_id, stage_id) = row.map_err(sqlite_error)?;
        if chapter_id.trim().is_empty() || stage_id.trim().is_empty() {
            return Err(CoreError::validation(
                "chapter-stage relation contains an empty id",
            ));
        }
        if chapter_stage.insert(chapter_id.clone(), stage_id).is_some() {
            return Err(CoreError::validation(format!(
                "chapter has duplicate stage relations: {chapter_id}"
            )));
        }
    }
    Ok(chapter_stage)
}

fn load_distinct_ids_for_values(
    connection: &Connection,
    sql_prefix: &str,
    values: &BTreeSet<String>,
) -> CoreResult<BTreeSet<String>> {
    if values.is_empty() {
        return Ok(BTreeSet::new());
    }
    let sql = format!(
        "{sql_prefix} ({}) ORDER BY 1",
        sql_placeholders(values.len())
    );
    let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(values.iter()), |row| {
            row.get::<_, String>(0)
        })
        .map_err(sqlite_error)?;
    let mut ids = BTreeSet::new();
    for row in rows {
        let id = row.map_err(sqlite_error)?;
        if id.trim().is_empty() {
            return Err(CoreError::validation(
                "summary relation contains an empty id",
            ));
        }
        ids.insert(id);
    }
    Ok(ids)
}

fn load_registered_changes_for_ids(
    connection: &Connection,
    ids: &BTreeSet<String>,
) -> CoreResult<Vec<RegisteredChange>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT change_id, function, status, content_json, metadata_json
         FROM registered_changes WHERE change_id IN ({}) ORDER BY change_id",
        sql_placeholders(ids.len())
    );
    let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(sqlite_error)?;
    let mut linked_segments = load_string_multimap_for_keys(
        connection,
        "SELECT change_id, segment_id FROM change_segment_links WHERE change_id IN",
        ids,
    )?;
    let mut changes = Vec::new();
    for row in rows {
        let (change_id, function, status, content_json, metadata_json) =
            row.map_err(sqlite_error)?;
        let change = RegisteredChange {
            linked_segment_ids: linked_segments.remove(&change_id).unwrap_or_default(),
            change_id,
            function: parse_register_function(&function)?,
            status: parse_register_status(&status)?,
            content: serde_json::from_str(&content_json)?,
            metadata: serde_json::from_str(&metadata_json)?,
        };
        change.validate()?;
        changes.push(change);
    }
    reject_missing_entities(
        "registered change",
        ids,
        changes.iter().map(|item| &item.change_id),
    )?;
    if let Some(orphan) = linked_segments.keys().next() {
        return Err(CoreError::validation(format!(
            "change relation references missing change: {orphan}"
        )));
    }
    Ok(changes)
}

fn reject_missing_entities<'a>(
    label: &str,
    expected: &BTreeSet<String>,
    actual: impl Iterator<Item = &'a String>,
) -> CoreResult<()> {
    let actual = actual.cloned().collect::<BTreeSet<_>>();
    if let Some(missing) = expected.difference(&actual).next() {
        return Err(CoreError::validation(format!(
            "{label} relation references missing entity: {missing}"
        )));
    }
    Ok(())
}

fn load_summary_changes(
    connection: &Connection,
    chapter_id: &str,
    explicitly_referenced: &BTreeSet<String>,
) -> CoreResult<Vec<crate::rag::models::RegisteredChange>> {
    let mut statement = connection
        .prepare(
            "SELECT change_id, function, status, content_json, metadata_json
             FROM registered_changes WHERE status = 'planned'",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(sqlite_error)?;
    let mut changes = Vec::new();
    let mut loaded_ids = BTreeSet::new();
    for row in rows {
        let (change_id, function, status, content_json, metadata_json) =
            row.map_err(sqlite_error)?;
        let change = crate::rag::models::RegisteredChange {
            linked_segment_ids: Vec::new(),
            change_id: change_id.clone(),
            function: parse_register_function(&function)?,
            status: parse_register_status(&status)?,
            content: serde_json::from_str(&content_json)?,
            metadata: serde_json::from_str(&metadata_json)?,
        };
        if change.applies_to_chapter(chapter_id) || explicitly_referenced.contains(&change_id) {
            loaded_ids.insert(change_id);
            changes.push(change);
        }
    }
    let missing = explicitly_referenced
        .difference(&loaded_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        let sql = format!(
            "SELECT change_id, function, status, content_json, metadata_json
             FROM registered_changes WHERE change_id IN ({})",
            sql_placeholders(missing.len())
        );
        let mut statement = connection.prepare(&sql).map_err(sqlite_error)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(missing.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(sqlite_error)?;
        for row in rows {
            let (change_id, function, status, content_json, metadata_json) =
                row.map_err(sqlite_error)?;
            changes.push(crate::rag::models::RegisteredChange {
                linked_segment_ids: Vec::new(),
                change_id,
                function: parse_register_function(&function)?,
                status: parse_register_status(&status)?,
                content: serde_json::from_str(&content_json)?,
                metadata: serde_json::from_str(&metadata_json)?,
            });
        }
    }
    let selected_ids = changes
        .iter()
        .map(|change| change.change_id.clone())
        .collect::<BTreeSet<_>>();
    let mut change_segments = load_string_multimap_for_keys(
        connection,
        "SELECT change_id, segment_id FROM change_segment_links WHERE change_id IN",
        &selected_ids,
    )?;
    for change in &mut changes {
        change.linked_segment_ids = change_segments
            .remove(&change.change_id)
            .unwrap_or_default();
    }
    Ok(changes)
}

fn confirmation_kind_str(k: ConfirmationKind) -> &'static str {
    match k {
        ConfirmationKind::OutlinerOutput => "outliner_output",
        ConfirmationKind::DesignerOutput => "designer_output",
        ConfirmationKind::PlannerOutput => "planner_output",
        ConfirmationKind::PlannerRegister => "planner_register",
        ConfirmationKind::CriticReview => "critic_review",
        ConfirmationKind::PrudentReview => "prudent_review",
        ConfirmationKind::SegmentSummary => "segment_summary",
        ConfirmationKind::EventSummary => "event_summary",
        ConfirmationKind::ChapterSummary => "chapter_summary",
        ConfirmationKind::StageSummary => "stage_summary",
        ConfirmationKind::WriterCorrectionPatch => "writer_correction_patch",
        ConfirmationKind::PolisherCorrectionPatch => "polisher_correction_patch",
    }
}
fn parse_confirmation_kind(s: &str) -> CoreResult<ConfirmationKind> {
    match s {
        "outliner_output" => Ok(ConfirmationKind::OutlinerOutput),
        "designer_output" => Ok(ConfirmationKind::DesignerOutput),
        "planner_output" => Ok(ConfirmationKind::PlannerOutput),
        "planner_register" => Ok(ConfirmationKind::PlannerRegister),
        "critic_review" => Ok(ConfirmationKind::CriticReview),
        "prudent_review" => Ok(ConfirmationKind::PrudentReview),
        "segment_summary" => Ok(ConfirmationKind::SegmentSummary),
        "event_summary" => Ok(ConfirmationKind::EventSummary),
        "chapter_summary" => Ok(ConfirmationKind::ChapterSummary),
        "stage_summary" => Ok(ConfirmationKind::StageSummary),
        "writer_correction_patch" => Ok(ConfirmationKind::WriterCorrectionPatch),
        "polisher_correction_patch" => Ok(ConfirmationKind::PolisherCorrectionPatch),
        _ => Err(CoreError::validation(format!(
            "unknown confirmation kind in knowledge store: {s}"
        ))),
    }
}

fn confirmation_state_str(s: ConfirmationState) -> &'static str {
    match s {
        ConfirmationState::Pending => "pending",
        ConfirmationState::Skipped => "skipped",
        ConfirmationState::AutoAudited => "auto_audited",
        ConfirmationState::Approved => "approved",
        ConfirmationState::Rejected => "rejected",
    }
}
fn parse_confirmation_state(s: &str) -> CoreResult<ConfirmationState> {
    match s {
        "pending" => Ok(ConfirmationState::Pending),
        "skipped" => Ok(ConfirmationState::Skipped),
        "auto_audited" => Ok(ConfirmationState::AutoAudited),
        "approved" => Ok(ConfirmationState::Approved),
        "rejected" => Ok(ConfirmationState::Rejected),
        _ => Err(CoreError::validation(format!(
            "unknown confirmation state in knowledge store: {s}"
        ))),
    }
}

type RawSummarizerStageOperation = (
    String,
    String,
    String,
    i64,
    String,
    String,
    i64,
    String,
    String,
    Option<String>,
);

fn read_summarizer_stage_operation(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RawSummarizerStageOperation> {
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

fn parse_summarizer_stage_operation(
    raw: RawSummarizerStageOperation,
) -> CoreResult<SummarizerStageOperation> {
    let (
        operation_id,
        scope_id,
        parent_operation_id,
        parent_operation_attempt,
        parent_request_hash,
        step,
        stage_attempt,
        request_hash,
        status,
        response_json,
    ) = raw;
    Ok(SummarizerStageOperation {
        operation_id,
        scope_id,
        parent_operation_id,
        parent_operation_attempt: u32::try_from(parent_operation_attempt).map_err(|_| {
            CoreError::validation("summarizer parent operation attempt out of range")
        })?,
        parent_request_hash,
        step,
        stage_attempt: u32::try_from(stage_attempt)
            .map_err(|_| CoreError::validation("summarizer stage attempt out of range"))?,
        request_hash,
        status: parse_summarizer_stage_status(&status)?,
        response_json: response_json
            .map(|response| serde_json::from_str(&response))
            .transpose()?,
    })
}

fn summarizer_stage_status_name(status: SummarizerStageOperationStatus) -> &'static str {
    match status {
        SummarizerStageOperationStatus::Prepared => "prepared",
        SummarizerStageOperationStatus::Dispatched => "dispatched",
        SummarizerStageOperationStatus::Completed => "completed",
        SummarizerStageOperationStatus::InDoubt => "in_doubt",
        SummarizerStageOperationStatus::Aborted => "aborted",
    }
}

fn parse_summarizer_stage_status(value: &str) -> CoreResult<SummarizerStageOperationStatus> {
    match value {
        "prepared" => Ok(SummarizerStageOperationStatus::Prepared),
        "dispatched" => Ok(SummarizerStageOperationStatus::Dispatched),
        "completed" => Ok(SummarizerStageOperationStatus::Completed),
        "in_doubt" => Ok(SummarizerStageOperationStatus::InDoubt),
        "aborted" => Ok(SummarizerStageOperationStatus::Aborted),
        _ => Err(CoreError::validation(format!(
            "unknown summarizer stage operation status: {value}"
        ))),
    }
}

fn validate_summarizer_stage_identity(
    scope_id: &str,
    parent_operation_id: &str,
    parent_request_hash: &str,
    step: &str,
    request_hash: &str,
) -> CoreResult<()> {
    for (field, value) in [
        ("scope_id", scope_id),
        ("parent_operation_id", parent_operation_id),
        ("parent_request_hash", parent_request_hash),
        ("step", step),
        ("request_hash", request_hash),
    ] {
        if value.trim().is_empty() {
            return Err(CoreError::validation(format!(
                "summarizer stage {field} cannot be empty"
            )));
        }
    }
    Ok(())
}

// ── SQLite 工具函数 ──────────────────────────────────────────────────────────

fn validate_non_empty_store(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }
    Ok(())
}

fn dedupe_sources(sources: Vec<SourceSpan>) -> Vec<SourceSpan> {
    let mut unique = BTreeMap::new();
    for source in sources {
        unique.insert(
            (
                source.document_id.clone(),
                source.range.start,
                source.range.end,
                source.version.clone(),
            ),
            source,
        );
    }
    unique.into_values().collect()
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

fn sqlite_error(error: rusqlite::Error) -> CoreError {
    CoreError::External {
        service: "writing_knowledge_store".to_owned(),
        message: error.to_string(),
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation("writing knowledge store lock poisoned")
}

fn unix_timestamp_ms_i64() -> CoreResult<i64> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| CoreError::validation(format!("system time before epoch: {e}")))?
        .as_millis();
    i64::try_from(ms).map_err(|_| CoreError::validation("timestamp exceeds i64"))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn knowledge_writer_lock_serializes_distinct_store_instances() {
        let project = tempfile::tempdir().unwrap();
        let first = SqliteWritingKnowledgeStore::open(project.path()).unwrap();
        let guard = first.acquire_writer_lock().unwrap();
        let project_root = project.path().to_path_buf();
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let second = SqliteWritingKnowledgeStore::open(project_root).unwrap();
            started_tx.send(()).unwrap();
            second
                .save_knowledge(&MemoryWritingKnowledgeBase::new())
                .unwrap();
            finished_tx.send(()).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(finished_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(guard);
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }
}
