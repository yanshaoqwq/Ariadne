use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::contracts::{CoreError, CoreResult};
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    ConfirmationItem, ConfirmationKind, ConfirmationState, ForeshadowingRecord,
    ForeshadowingStatus, PlannerIssue, RegisterContent, RegisterFunction, RegisteredChangeStatus,
    StoryEvent, StoryEventStatus, StorySegment,
};

pub const METADATA_DB_FILE: &str = "metadata.db";
const SCHEMA_VERSION: i64 = 1;

/// 写作知识库 SQLite 持久化后端，使用完整关系表存储源实体。
/// 22 张双向索引表全部可从源实体派生，加载时重放 upsert 路径重建，零重复代码。
#[derive(Debug)]
pub struct SqliteWritingKnowledgeStore {
    db_path: Option<PathBuf>,
    connection: Mutex<Connection>,
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
        let conn = self.connection.lock().map_err(lock_error)?;
        let tx = conn.unchecked_transaction().map_err(sqlite_error)?;

        // 故事段与事件代表当前 active revision；先清除旧快照，再在同一事务重建。
        // 审核确认历史使用独立不可变 id，不在这里清除。
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
                        .map(|p| serde_json::to_string(p))
                        .transpose()?,
                ],
            )
            .map_err(sqlite_error)?;
        }

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

        tx.commit().map_err(sqlite_error)
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
                    function: parse_register_function(&function_str),
                    status: parse_register_status(&status_str),
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
                    status: parse_foreshadowing_status(&status_str),
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
                    kind: parse_confirmation_kind(&kind_str),
                    state: parse_confirmation_state(&state_str),
                    prompt_key,
                    metadata: serde_json::from_str(&metadata_json)
                        .unwrap_or(serde_json::Value::Null),
                })?;
            }
        }

        Ok(kb)
    }
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
fn parse_register_function(s: &str) -> RegisterFunction {
    match s {
        "character_plan" => RegisterFunction::CharacterPlan,
        "character_trait" => RegisterFunction::CharacterTrait,
        "relationship" => RegisterFunction::Relationship,
        "foreshadowing" => RegisterFunction::Foreshadowing,
        "theme_anchor" => RegisterFunction::ThemeAnchor,
        _ => RegisterFunction::CharacterProfile,
    }
}

fn register_status_str(s: RegisteredChangeStatus) -> &'static str {
    match s {
        RegisteredChangeStatus::Planned => "planned",
        RegisteredChangeStatus::Realized => "realized",
        RegisteredChangeStatus::Deleted => "deleted",
    }
}
fn parse_register_status(s: &str) -> RegisteredChangeStatus {
    match s {
        "realized" => RegisteredChangeStatus::Realized,
        "deleted" => RegisteredChangeStatus::Deleted,
        _ => RegisteredChangeStatus::Planned,
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
fn parse_foreshadowing_status(s: &str) -> ForeshadowingStatus {
    match s {
        "planted" => ForeshadowingStatus::Planted,
        "recovered" => ForeshadowingStatus::Recovered,
        "abandoned" => ForeshadowingStatus::Abandoned,
        _ => ForeshadowingStatus::Planned,
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
fn parse_confirmation_kind(s: &str) -> ConfirmationKind {
    match s {
        "designer_output" => ConfirmationKind::DesignerOutput,
        "planner_output" => ConfirmationKind::PlannerOutput,
        "planner_register" => ConfirmationKind::PlannerRegister,
        "critic_review" => ConfirmationKind::CriticReview,
        "prudent_review" => ConfirmationKind::PrudentReview,
        "segment_summary" => ConfirmationKind::SegmentSummary,
        "event_summary" => ConfirmationKind::EventSummary,
        "chapter_summary" => ConfirmationKind::ChapterSummary,
        "stage_summary" => ConfirmationKind::StageSummary,
        "writer_correction_patch" => ConfirmationKind::WriterCorrectionPatch,
        "polisher_correction_patch" => ConfirmationKind::PolisherCorrectionPatch,
        _ => ConfirmationKind::OutlinerOutput,
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
fn parse_confirmation_state(s: &str) -> ConfirmationState {
    match s {
        "skipped" => ConfirmationState::Skipped,
        "auto_audited" => ConfirmationState::AutoAudited,
        "approved" => ConfirmationState::Approved,
        "rejected" => ConfirmationState::Rejected,
        _ => ConfirmationState::Pending,
    }
}

// ── SQLite 工具函数 ──────────────────────────────────────────────────────────

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
