use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::contracts::{CoreError, CoreResult};
use crate::retrieval::memory::sort_and_limit;
use crate::retrieval::query::sqlite_fts_literal_query;
use crate::retrieval::models::{
    ChunkDocument, FullTextRecord, FullTextSearchRequest, RebuildReport, RebuildStatus,
    RetrievalResult, RetrievalSource, StoreHealth,
};
use crate::retrieval::traits::FullTextStore;

const SCHEMA_VERSION: i64 = 1;

/// SQLite 持久化全文检索后端，作为真实 metadata/全文存储的基础实现。
#[derive(Debug)]
pub struct SqliteFullTextStore {
    connection: Mutex<Connection>,
}

impl SqliteFullTextStore {
    /// 打开磁盘 SQLite 全文索引。
    pub fn open(path: impl AsRef<Path>) -> CoreResult<Self> {
        let connection = Connection::open(path).map_err(sqlite_error)?;
        configure_connection(&connection, true)?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.migrate()?;
        Ok(store)
    }

    /// 打开内存 SQLite 全文索引，主要用于契约测试。
    pub fn open_in_memory() -> CoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(sqlite_error)?;
        configure_connection(&connection, false)?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> CoreResult<()> {
        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    name TEXT PRIMARY KEY,
                    version INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS full_text_chunks (
                    chunk_id TEXT PRIMARY KEY,
                    document_id TEXT NOT NULL,
                    text TEXT NOT NULL,
                    sources_json TEXT NOT NULL,
                    metadata_json TEXT NOT NULL
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS full_text_chunks_fts
                    USING fts5(chunk_id UNINDEXED, text);

                CREATE TABLE IF NOT EXISTS retrieval_store_state (
                    component TEXT PRIMARY KEY,
                    rebuild_reason TEXT
                );
                ",
            )
            .map_err(sqlite_error)?;
        connection
            .execute(
                "INSERT INTO schema_migrations(name, version)
                 VALUES('sqlite_full_text_store', ?1)
                 ON CONFLICT(name) DO UPDATE SET version = excluded.version",
                params![SCHEMA_VERSION],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }
}

impl FullTextStore for SqliteFullTextStore {
    /// 写入或覆盖全文记录，并同步 FTS 表。
    fn upsert(&self, records: Vec<FullTextRecord>) -> CoreResult<()> {
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        for record in records {
            validate_record(&record)?;
            let sources_json = serde_json::to_string(&record.chunk.sources)?;
            let metadata_json = serde_json::to_string(&record.chunk.metadata)?;
            transaction
                .execute(
                    "
                    INSERT INTO full_text_chunks(
                        chunk_id, document_id, text, sources_json, metadata_json
                    )
                    VALUES(?1, ?2, ?3, ?4, ?5)
                    ON CONFLICT(chunk_id) DO UPDATE SET
                        document_id = excluded.document_id,
                        text = excluded.text,
                        sources_json = excluded.sources_json,
                        metadata_json = excluded.metadata_json
                    ",
                    params![
                        record.chunk.chunk_id,
                        record.chunk.document_id,
                        record.chunk.text,
                        sources_json,
                        metadata_json,
                    ],
                )
                .map_err(sqlite_error)?;
            transaction
                .execute(
                    "DELETE FROM full_text_chunks_fts WHERE chunk_id = ?1",
                    params![record.chunk.chunk_id],
                )
                .map_err(sqlite_error)?;
            transaction
                .execute(
                    "INSERT INTO full_text_chunks_fts(chunk_id, text) VALUES(?1, ?2)",
                    params![record.chunk.chunk_id, record.chunk.text],
                )
                .map_err(sqlite_error)?;
        }
        clear_rebuild_reason(&transaction)?;
        transaction.commit().map_err(sqlite_error)
    }

    /// 删除指定文档下的所有全文记录。
    fn delete_document(&self, document_id: &str) -> CoreResult<usize> {
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let chunk_ids = {
            let mut statement = transaction
                .prepare("SELECT chunk_id FROM full_text_chunks WHERE document_id = ?1")
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(params![document_id], |row| row.get::<_, String>(0))
                .map_err(sqlite_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)?
        };
        for chunk_id in &chunk_ids {
            transaction
                .execute(
                    "DELETE FROM full_text_chunks_fts WHERE chunk_id = ?1",
                    params![chunk_id],
                )
                .map_err(sqlite_error)?;
        }
        let deleted = transaction
            .execute(
                "DELETE FROM full_text_chunks WHERE document_id = ?1",
                params![document_id],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(deleted)
    }

    /// 使用 SQLite FTS5 执行全文检索，并在内存中应用 metadata filter。
    fn search(&self, request: FullTextSearchRequest) -> CoreResult<Vec<RetrievalResult>> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }
        if request.query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let query = sqlite_fts_literal_query(&request.query)?;
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "
                SELECT c.chunk_id, c.document_id, c.text, c.sources_json, c.metadata_json,
                       bm25(full_text_chunks_fts) AS rank
                FROM full_text_chunks_fts
                JOIN full_text_chunks c ON c.chunk_id = full_text_chunks_fts.chunk_id
                WHERE full_text_chunks_fts MATCH ?1
                ORDER BY rank ASC
                LIMIT ?2
                ",
            )
            .map_err(sqlite_error)?;
        let candidate_limit = i64::try_from(request.limit.saturating_mul(3).max(request.limit))
            .map_err(|_| CoreError::validation("full text search limit exceeds i64"))?;
        let rows = statement
            .query_map(params![query, candidate_limit], |row| {
                let rank = row.get::<_, f64>(5)?;
                Ok((
                    ChunkDocument {
                        chunk_id: row.get(0)?,
                        document_id: row.get(1)?,
                        text: row.get(2)?,
                        sources: serde_json::from_str(&row.get::<_, String>(3)?).map_err(
                            |error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    3,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            },
                        )?,
                        metadata: serde_json::from_str(&row.get::<_, String>(4)?).map_err(
                            |error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    4,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            },
                        )?,
                    },
                    rank,
                ))
            })
            .map_err(sqlite_error)?;
        let mut results = Vec::new();
        for row in rows {
            let (chunk, rank) = row.map_err(sqlite_error)?;
            if !metadata_matches(&chunk.metadata, &request.filters) {
                continue;
            }
            // FTS5 bm25() 返回负值且越负越相关，直接 max(0.0) 会把所有结果压成
            // 同一分数。改用 logistic 变换，把 rank 映射到 (0,1) 且随相关度单调
            // 递增，既保留 bm25 排序，又能与向量分数做加权融合。
            let score = (1.0_f64 / (1.0_f64 + rank.exp())) as f32;
            results.push(RetrievalResult::from_chunk(
                &chunk,
                score,
                RetrievalSource::FullText,
            ));
        }
        sort_and_limit(&mut results, request.limit);
        Ok(results)
    }

    /// 返回 SQLite 全文索引的健康状态。
    fn health_check(&self) -> CoreResult<StoreHealth> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let reason = rebuild_reason(&connection)?;
        if let Some(reason) = reason {
            return Ok(StoreHealth::rebuild_required(
                "sqlite_full_text_store",
                reason,
            ));
        }
        Ok(StoreHealth::healthy("sqlite_full_text_store"))
    }

    /// 标记 SQLite 全文索引需要重建。
    fn mark_rebuild_required(&self, reason: &str) -> CoreResult<()> {
        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .execute(
                "INSERT INTO retrieval_store_state(component, rebuild_reason)
                 VALUES('sqlite_full_text_store', ?1)
                 ON CONFLICT(component) DO UPDATE SET rebuild_reason = excluded.rebuild_reason",
                params![reason],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    /// 用源记录重建整个 SQLite 全文索引。
    fn rebuild_from_records(&self, records: Vec<FullTextRecord>) -> CoreResult<RebuildReport> {
        let processed_items = records.len() as u64;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        transaction
            .execute("DELETE FROM full_text_chunks_fts", [])
            .map_err(sqlite_error)?;
        transaction
            .execute("DELETE FROM full_text_chunks", [])
            .map_err(sqlite_error)?;
        for record in records {
            validate_record(&record)?;
            transaction
                .execute(
                    "
                    INSERT INTO full_text_chunks(
                        chunk_id, document_id, text, sources_json, metadata_json
                    )
                    VALUES(?1, ?2, ?3, ?4, ?5)
                    ",
                    params![
                        record.chunk.chunk_id,
                        record.chunk.document_id,
                        record.chunk.text,
                        serde_json::to_string(&record.chunk.sources)?,
                        serde_json::to_string(&record.chunk.metadata)?,
                    ],
                )
                .map_err(sqlite_error)?;
            transaction
                .execute(
                    "INSERT INTO full_text_chunks_fts(chunk_id, text) VALUES(?1, ?2)",
                    params![record.chunk.chunk_id, record.chunk.text],
                )
                .map_err(sqlite_error)?;
        }
        clear_rebuild_reason(&transaction)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(RebuildReport {
            component: "sqlite_full_text_store".to_owned(),
            status: RebuildStatus::Completed,
            processed_items,
            error: None,
        })
    }
}

fn validate_record(record: &FullTextRecord) -> CoreResult<()> {
    if record.chunk.chunk_id.trim().is_empty() {
        return Err(CoreError::validation("chunk_id cannot be empty"));
    }
    if record.chunk.document_id.trim().is_empty() {
        return Err(CoreError::validation("document_id cannot be empty"));
    }
    if record.chunk.text.trim().is_empty() {
        return Err(CoreError::validation("chunk text cannot be empty"));
    }
    Ok(())
}

fn metadata_matches(
    metadata: &serde_json::Value,
    filters: &std::collections::BTreeMap<String, String>,
) -> bool {
    filters.iter().all(|(key, expected)| {
        metadata
            .get(key)
            .and_then(|value| value.as_str())
            .is_some_and(|actual| actual == expected)
    })
}

fn rebuild_reason(connection: &Connection) -> CoreResult<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT rebuild_reason FROM retrieval_store_state WHERE component = 'sqlite_full_text_store'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(sqlite_error)?
        .flatten())
}

fn clear_rebuild_reason(connection: &Connection) -> CoreResult<()> {
    connection
        .execute(
            "INSERT INTO retrieval_store_state(component, rebuild_reason)
             VALUES('sqlite_full_text_store', NULL)
             ON CONFLICT(component) DO UPDATE SET rebuild_reason = NULL",
            [],
        )
        .map_err(sqlite_error)?;
    Ok(())
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

fn lock_error<T>(_: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation("sqlite full text store lock poisoned")
}

fn sqlite_error(error: rusqlite::Error) -> CoreError {
    CoreError::External {
        service: "sqlite_full_text_store".to_owned(),
        message: error.to_string(),
    }
}
