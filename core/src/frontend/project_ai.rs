use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::FileExt;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::contracts::{content_version_for_bytes, CoreError, CoreResult};

const ACTIVE_MESSAGE_LIMIT: usize = 48;
const ACTIVE_MESSAGE_TARGET: usize = 32;
const ACTIVE_CHARACTER_LIMIT: usize = 64 * 1024;
const ACTIVE_CHARACTER_TARGET: usize = 48 * 1024;
const SUMMARY_MESSAGE_CHARACTER_LIMIT: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAiStoredMessage {
    pub sequence: u64,
    pub revision: u64,
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAiSummaryChunk {
    pub summary_id: u64,
    pub from_sequence: u64,
    pub to_sequence: u64,
    pub from_revision: u64,
    pub to_revision: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAiMemoryEntry {
    pub memory_id: u64,
    pub entity_id: String,
    pub logical_key: String,
    pub value: String,
    pub source: String,
    pub source_version: String,
    pub source_line: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAiConversationSnapshot {
    pub conversation_id: String,
    pub revision: u64,
    pub summary_revision: u64,
    pub messages: Vec<ProjectAiStoredMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectAiAppendOutcome {
    Saved {
        snapshot: ProjectAiConversationSnapshot,
        appended: Vec<ProjectAiStoredMessage>,
    },
    RevisionConflict {
        actual_revision: u64,
    },
}

pub struct ProjectAiConversationGuard {
    _lock: File,
}

pub struct ProjectAiConversationStore {
    connection: Mutex<Connection>,
}

impl ProjectAiConversationStore {
    pub fn try_acquire_conversation(
        project_root: &Path,
        conversation_id: &str,
    ) -> CoreResult<Option<ProjectAiConversationGuard>> {
        validate_conversation_id(conversation_id)?;
        let lock_root = project_root.join(".runtime/project_ai_locks");
        std::fs::create_dir_all(&lock_root)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_root.join(format!("{conversation_id}.lock")))?;
        match FileExt::try_lock_exclusive(&lock) {
            Ok(()) => Ok(Some(ProjectAiConversationGuard { _lock: lock })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn open(project_root: &Path) -> CoreResult<Self> {
        let runtime_root = project_root.join(".runtime");
        std::fs::create_dir_all(&runtime_root)?;
        let connection =
            Connection::open(runtime_root.join("project_ai.db")).map_err(sqlite_error)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> CoreResult<Self> {
        Self::from_connection(Connection::open_in_memory().map_err(sqlite_error)?)
    }

    fn from_connection(connection: Connection) -> CoreResult<Self> {
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS project_ai_conversations (
                     conversation_id TEXT PRIMARY KEY,
                     revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
                     next_sequence INTEGER NOT NULL DEFAULT 1 CHECK(next_sequence >= 1),
                     summary_revision INTEGER NOT NULL DEFAULT 0 CHECK(summary_revision >= 0),
                     updated_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS project_ai_messages (
                     conversation_id TEXT NOT NULL,
                     sequence INTEGER NOT NULL CHECK(sequence >= 1),
                     revision INTEGER NOT NULL CHECK(revision >= 0),
                     role TEXT NOT NULL CHECK(role IN ('system', 'user', 'assistant')),
                     content TEXT NOT NULL,
                     created_at_ms INTEGER NOT NULL,
                     PRIMARY KEY (conversation_id, sequence),
                     FOREIGN KEY (conversation_id) REFERENCES project_ai_conversations(conversation_id) ON DELETE CASCADE
                 );
                 CREATE INDEX IF NOT EXISTS idx_project_ai_messages_revision
                     ON project_ai_messages(conversation_id, revision, sequence);
                 CREATE TABLE IF NOT EXISTS project_ai_summaries (
                     summary_id INTEGER PRIMARY KEY AUTOINCREMENT,
                     conversation_id TEXT NOT NULL,
                     from_sequence INTEGER NOT NULL,
                     to_sequence INTEGER NOT NULL,
                     from_revision INTEGER NOT NULL,
                     to_revision INTEGER NOT NULL,
                     summary_text TEXT NOT NULL,
                     created_at_ms INTEGER NOT NULL,
                     FOREIGN KEY (conversation_id) REFERENCES project_ai_conversations(conversation_id) ON DELETE CASCADE
                 );
                 CREATE INDEX IF NOT EXISTS idx_project_ai_summaries_range
                     ON project_ai_summaries(conversation_id, to_sequence DESC);
                 CREATE TABLE IF NOT EXISTS project_ai_memory (
                     memory_id INTEGER PRIMARY KEY AUTOINCREMENT,
                     entity_id TEXT NOT NULL,
                     logical_key TEXT NOT NULL,
                     value TEXT NOT NULL,
                     source TEXT NOT NULL,
                     source_version TEXT NOT NULL,
                     source_line INTEGER NOT NULL,
                     updated_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS project_ai_meta (
                     key TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );",
            )
            .map_err(sqlite_error)?;
        migrate_project_memory_projection(&connection)?;
        connection
            .execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_project_ai_memory_entity
                     ON project_ai_memory(source, entity_id);
                 CREATE INDEX IF NOT EXISTS idx_project_ai_memory_source
                     ON project_ai_memory(source, source_version, source_line);",
            )
            .map_err(sqlite_error)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn load_or_seed(
        &self,
        conversation_id: &str,
        seed: &[(String, String)],
    ) -> CoreResult<ProjectAiConversationSnapshot> {
        validate_conversation_id(conversation_id)?;
        let now_ms = unix_timestamp_ms()?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        ensure_conversation(&transaction, conversation_id, now_ms)?;
        let (revision, next_sequence, _) = conversation_state(&transaction, conversation_id)?;
        let message_count = transaction
            .query_row(
                "SELECT COUNT(*) FROM project_ai_messages WHERE conversation_id = ?1",
                params![conversation_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sqlite_error)?;
        if message_count == 0 && revision == 0 && !seed.is_empty() {
            let mut sequence = next_sequence;
            for (role, content) in seed {
                validate_role(role)?;
                if content.trim().is_empty() {
                    continue;
                }
                transaction
                    .execute(
                        "INSERT INTO project_ai_messages
                         (conversation_id, sequence, revision, role, content, created_at_ms)
                         VALUES (?1, ?2, 0, ?3, ?4, ?5)",
                        params![conversation_id, sequence, role, content, now_ms],
                    )
                    .map_err(sqlite_error)?;
                sequence += 1;
            }
            transaction
                .execute(
                    "UPDATE project_ai_conversations
                     SET next_sequence = ?1, updated_at_ms = ?2
                     WHERE conversation_id = ?3",
                    params![sequence, now_ms, conversation_id],
                )
                .map_err(sqlite_error)?;
            compact_conversation(&transaction, conversation_id, revision, now_ms)?;
        }
        transaction.commit().map_err(sqlite_error)?;
        drop(connection);
        self.load(conversation_id)
    }

    pub fn load(&self, conversation_id: &str) -> CoreResult<ProjectAiConversationSnapshot> {
        validate_conversation_id(conversation_id)?;
        let connection = self.connection.lock().map_err(lock_error)?;
        load_snapshot(&connection, conversation_id)
    }

    pub fn append_messages(
        &self,
        conversation_id: &str,
        expected_revision: u64,
        messages: &[(String, String)],
    ) -> CoreResult<ProjectAiAppendOutcome> {
        validate_conversation_id(conversation_id)?;
        if messages.is_empty() {
            return Ok(ProjectAiAppendOutcome::Saved {
                snapshot: self.load(conversation_id)?,
                appended: Vec::new(),
            });
        }
        let now_ms = unix_timestamp_ms()?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        ensure_conversation(&transaction, conversation_id, now_ms)?;
        let (actual_revision, mut next_sequence, _) =
            conversation_state(&transaction, conversation_id)?;
        let expected_revision = i64::try_from(expected_revision).map_err(|_| {
            CoreError::validation("project AI expected conversation revision exceeds SQLite range")
        })?;
        if actual_revision != expected_revision {
            transaction.commit().map_err(sqlite_error)?;
            return Ok(ProjectAiAppendOutcome::RevisionConflict {
                actual_revision: u64::try_from(actual_revision).map_err(|_| {
                    CoreError::validation("project AI conversation revision is invalid")
                })?,
            });
        }
        let next_revision = actual_revision
            .checked_add(1)
            .ok_or_else(|| CoreError::validation("project AI conversation revision overflow"))?;
        let mut appended = Vec::with_capacity(messages.len());
        for (role, content) in messages {
            validate_role(role)?;
            if content.trim().is_empty() {
                continue;
            }
            transaction
                .execute(
                    "INSERT INTO project_ai_messages
                     (conversation_id, sequence, revision, role, content, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        conversation_id,
                        next_sequence,
                        next_revision,
                        role,
                        content,
                        now_ms
                    ],
                )
                .map_err(sqlite_error)?;
            appended.push(ProjectAiStoredMessage {
                sequence: u64::try_from(next_sequence)
                    .map_err(|_| CoreError::validation("project AI message sequence overflow"))?,
                revision: u64::try_from(next_revision).map_err(|_| {
                    CoreError::validation("project AI conversation revision overflow")
                })?,
                role: role.clone(),
                content: content.clone(),
            });
            next_sequence += 1;
        }
        transaction
            .execute(
                "UPDATE project_ai_conversations
                 SET revision = ?1, next_sequence = ?2, updated_at_ms = ?3
                 WHERE conversation_id = ?4 AND revision = ?5",
                params![
                    next_revision,
                    next_sequence,
                    now_ms,
                    conversation_id,
                    actual_revision
                ],
            )
            .map_err(sqlite_error)?;
        compact_conversation(&transaction, conversation_id, next_revision, now_ms)?;
        transaction.commit().map_err(sqlite_error)?;
        drop(connection);
        Ok(ProjectAiAppendOutcome::Saved {
            snapshot: self.load(conversation_id)?,
            appended,
        })
    }

    pub fn select_summary_chunks(
        &self,
        conversation_id: &str,
        query: &str,
        limit: usize,
    ) -> CoreResult<Vec<ProjectAiSummaryChunk>> {
        validate_conversation_id(conversation_id)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut selected = Vec::new();
        let mut ids = BTreeSet::new();
        for term in query_terms(query).into_iter().take(4) {
            let pattern = format!("%{term}%");
            let mut statement = connection
                .prepare(
                    "SELECT summary_id, from_sequence, to_sequence, from_revision,
                            to_revision, summary_text
                     FROM project_ai_summaries
                     WHERE conversation_id = ?1 AND lower(summary_text) LIKE ?2
                     ORDER BY to_sequence DESC LIMIT ?3",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(
                    params![conversation_id, pattern, limit as i64],
                    read_summary,
                )
                .map_err(sqlite_error)?;
            for row in rows {
                let summary = row.map_err(sqlite_error)?;
                if ids.insert(summary.summary_id) {
                    selected.push(summary);
                }
                if selected.len() >= limit {
                    break;
                }
            }
            if selected.len() >= limit {
                break;
            }
        }
        if selected.is_empty() {
            let mut statement = connection
                .prepare(
                    "SELECT summary_id, from_sequence, to_sequence, from_revision,
                            to_revision, summary_text
                     FROM project_ai_summaries
                     WHERE conversation_id = ?1
                     ORDER BY to_sequence DESC LIMIT ?2",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(params![conversation_id, limit as i64], read_summary)
                .map_err(sqlite_error)?;
            for row in rows {
                let summary = row.map_err(sqlite_error)?;
                if ids.insert(summary.summary_id) {
                    selected.push(summary);
                }
                if selected.len() >= limit {
                    break;
                }
            }
        }
        selected.sort_by_key(|summary| summary.from_sequence);
        Ok(selected)
    }

    pub fn synchronize_project_memory(&self, content: &str) -> CoreResult<String> {
        let source_version = content_version_for_bytes(content.as_bytes());
        let now_ms = unix_timestamp_ms()?;
        let entries = parse_project_memory(content);
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let current = transaction
            .query_row(
                "SELECT value FROM project_ai_meta WHERE key = 'project_memory_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;
        if current.as_deref() != Some(source_version.as_str()) {
            transaction
                .execute("DELETE FROM project_ai_memory", [])
                .map_err(sqlite_error)?;
            for (entity_id, source_line, logical_key, value) in entries {
                transaction
                    .execute(
                        "INSERT INTO project_ai_memory
                         (entity_id, logical_key, value, source, source_version, source_line,
                          updated_at_ms)
                         VALUES (?1, ?2, ?3, 'project_memory.md', ?4, ?5, ?6)",
                        params![
                            entity_id,
                            logical_key,
                            value,
                            source_version,
                            source_line,
                            now_ms
                        ],
                    )
                    .map_err(sqlite_error)?;
            }
            transaction
                .execute(
                    "INSERT INTO project_ai_meta(key, value)
                     VALUES ('project_memory_version', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![source_version],
                )
                .map_err(sqlite_error)?;
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(source_version)
    }

    pub fn select_project_memory(
        &self,
        query: &str,
        limit: usize,
    ) -> CoreResult<Vec<ProjectAiMemoryEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut selected = Vec::new();
        let mut ids = BTreeSet::new();
        for term in query_terms(query).into_iter().take(4) {
            let pattern = format!("%{term}%");
            let mut statement = connection
                .prepare(
                    "SELECT memory_id, entity_id, logical_key, value, source, source_version,
                            source_line
                     FROM project_ai_memory
                     WHERE lower(logical_key) LIKE ?1 OR lower(value) LIKE ?1
                     ORDER BY source_line DESC LIMIT ?2",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(params![pattern, limit as i64], read_memory)
                .map_err(sqlite_error)?;
            for row in rows {
                let memory = row.map_err(sqlite_error)?;
                if ids.insert(memory.memory_id) {
                    selected.push(memory);
                }
                if selected.len() >= limit {
                    break;
                }
            }
            if selected.len() >= limit {
                break;
            }
        }
        if selected.is_empty() {
            let mut statement = connection
                .prepare(
                    "SELECT memory_id, entity_id, logical_key, value, source, source_version,
                            source_line
                     FROM project_ai_memory
                     ORDER BY source_line DESC LIMIT ?1",
                )
                .map_err(sqlite_error)?;
            let rows = statement
                .query_map(params![limit as i64], read_memory)
                .map_err(sqlite_error)?;
            for row in rows {
                let memory = row.map_err(sqlite_error)?;
                if ids.insert(memory.memory_id) {
                    selected.push(memory);
                }
                if selected.len() >= limit {
                    break;
                }
            }
        }
        selected.sort_by_key(|memory| memory.source_line);
        Ok(selected)
    }
}

fn ensure_conversation(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    now_ms: i64,
) -> CoreResult<()> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO project_ai_conversations
             (conversation_id, revision, next_sequence, summary_revision, updated_at_ms)
             VALUES (?1, 0, 1, 0, ?2)",
            params![conversation_id, now_ms],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

fn conversation_state(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> CoreResult<(i64, i64, i64)> {
    transaction
        .query_row(
            "SELECT revision, next_sequence, summary_revision
             FROM project_ai_conversations WHERE conversation_id = ?1",
            params![conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(sqlite_error)
}

fn compact_conversation(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    revision: i64,
    now_ms: i64,
) -> CoreResult<()> {
    let mut statement = transaction
        .prepare(
            "SELECT sequence, revision, role, content
             FROM project_ai_messages
             WHERE conversation_id = ?1 ORDER BY sequence",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map(params![conversation_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(sqlite_error)?;
    let messages = rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)?;
    let total_chars = messages
        .iter()
        .map(|message| message.3.chars().count())
        .sum::<usize>();
    if messages.len() <= ACTIVE_MESSAGE_LIMIT && total_chars <= ACTIVE_CHARACTER_LIMIT {
        return Ok(());
    }
    let mut cut = messages.len().saturating_sub(ACTIVE_MESSAGE_TARGET);
    let mut remaining_chars = messages[cut..]
        .iter()
        .map(|message| message.3.chars().count())
        .sum::<usize>();
    while remaining_chars > ACTIVE_CHARACTER_TARGET && cut < messages.len().saturating_sub(2) {
        remaining_chars = remaining_chars.saturating_sub(messages[cut].3.chars().count());
        cut += 1;
    }
    while cut < messages.len().saturating_sub(1) && messages[cut].2 == "assistant" {
        cut += 1;
    }
    if cut == 0 {
        return Ok(());
    }
    let compacted = &messages[..cut];
    let from_sequence = compacted.first().map(|message| message.0).unwrap_or(0);
    let to_sequence = compacted.last().map(|message| message.0).unwrap_or(0);
    let from_revision = compacted.first().map(|message| message.1).unwrap_or(0);
    let to_revision = compacted.last().map(|message| message.1).unwrap_or(0);
    let mut summary = String::new();
    for (sequence, message_revision, role, content) in compacted {
        let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
        let selected = normalized
            .chars()
            .take(SUMMARY_MESSAGE_CHARACTER_LIMIT)
            .collect::<String>();
        summary.push_str(&format!(
            "[revision={message_revision} sequence={sequence} role={role}] {selected}\n"
        ));
    }
    transaction
        .execute(
            "INSERT INTO project_ai_summaries
             (conversation_id, from_sequence, to_sequence, from_revision, to_revision,
              summary_text, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                conversation_id,
                from_sequence,
                to_sequence,
                from_revision,
                to_revision,
                summary,
                now_ms
            ],
        )
        .map_err(sqlite_error)?;
    transaction
        .execute(
            "DELETE FROM project_ai_messages
             WHERE conversation_id = ?1 AND sequence <= ?2",
            params![conversation_id, to_sequence],
        )
        .map_err(sqlite_error)?;
    transaction
        .execute(
            "UPDATE project_ai_conversations SET summary_revision = ?1
             WHERE conversation_id = ?2",
            params![revision, conversation_id],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

fn load_snapshot(
    connection: &Connection,
    conversation_id: &str,
) -> CoreResult<ProjectAiConversationSnapshot> {
    let state = connection
        .query_row(
            "SELECT revision, summary_revision FROM project_ai_conversations
             WHERE conversation_id = ?1",
            params![conversation_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some((revision, summary_revision)) = state else {
        return Ok(ProjectAiConversationSnapshot {
            conversation_id: conversation_id.to_owned(),
            revision: 0,
            summary_revision: 0,
            messages: Vec::new(),
        });
    };
    let mut statement = connection
        .prepare(
            "SELECT sequence, revision, role, content FROM project_ai_messages
             WHERE conversation_id = ?1 ORDER BY sequence",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map(params![conversation_id], |row| {
            Ok(ProjectAiStoredMessage {
                sequence: u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                revision: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                role: row.get(2)?,
                content: row.get(3)?,
            })
        })
        .map_err(sqlite_error)?;
    Ok(ProjectAiConversationSnapshot {
        conversation_id: conversation_id.to_owned(),
        revision: u64::try_from(revision)
            .map_err(|_| CoreError::validation("project AI conversation revision is invalid"))?,
        summary_revision: u64::try_from(summary_revision)
            .map_err(|_| CoreError::validation("project AI summary revision is invalid"))?,
        messages: rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)?,
    })
}

fn read_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectAiSummaryChunk> {
    Ok(ProjectAiSummaryChunk {
        summary_id: u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
        from_sequence: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
        to_sequence: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
        from_revision: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
        to_revision: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
        text: row.get(5)?,
    })
}

fn read_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectAiMemoryEntry> {
    Ok(ProjectAiMemoryEntry {
        memory_id: u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
        entity_id: row.get(1)?,
        logical_key: row.get(2)?,
        value: row.get(3)?,
        source: row.get(4)?,
        source_version: row.get(5)?,
        source_line: u64::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
    })
}

fn parse_project_memory(content: &str) -> Vec<(String, i64, String, String)> {
    let mut key_occurrences = BTreeMap::<String, usize>::new();
    let mut note_occurrences = BTreeMap::<String, usize>::new();
    content
        .lines()
        .enumerate()
        .filter_map(|(index, raw)| {
            let line = raw
                .trim()
                .trim_start_matches(['-', '*'])
                .trim()
                .trim_start_matches('#')
                .trim();
            if line.is_empty() {
                return None;
            }
            let split = line.split_once('：').or_else(|| line.split_once(':'));
            let (logical_key, value, identity) = match split {
                Some((key, value)) if !key.trim().is_empty() && !value.trim().is_empty() => {
                    let logical_key = key.trim().to_owned();
                    let occurrence = key_occurrences.entry(logical_key.clone()).or_default();
                    *occurrence += 1;
                    let identity = format!("key:{logical_key}:{}", *occurrence);
                    (logical_key, value.trim().to_owned(), identity)
                }
                _ => {
                    let occurrence = note_occurrences.entry(line.to_owned()).or_default();
                    *occurrence += 1;
                    (
                        format!("note-{}", index + 1),
                        line.to_owned(),
                        format!("note:{line}:{}", *occurrence),
                    )
                }
            };
            let entity_id = format!("memory-{}", content_version_for_bytes(identity.as_bytes()));
            Some((entity_id, (index + 1) as i64, logical_key, value))
        })
        .collect()
}

fn migrate_project_memory_projection(connection: &Connection) -> CoreResult<()> {
    let mut statement = connection
        .prepare("PRAGMA table_info(project_ai_memory)")
        .map_err(sqlite_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_error)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(sqlite_error)?;
    if ["entity_id", "source"]
        .iter()
        .all(|column| columns.contains(*column))
    {
        return Ok(());
    }
    drop(statement);
    connection
        .execute_batch(
            "DROP TABLE project_ai_memory;
             CREATE TABLE project_ai_memory (
                 memory_id INTEGER PRIMARY KEY AUTOINCREMENT,
                 entity_id TEXT NOT NULL,
                 logical_key TEXT NOT NULL,
                 value TEXT NOT NULL,
                 source TEXT NOT NULL,
                 source_version TEXT NOT NULL,
                 source_line INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX idx_project_ai_memory_entity
                 ON project_ai_memory(source, entity_id);
             CREATE INDEX idx_project_ai_memory_source
                 ON project_ai_memory(source, source_version, source_line);
             DELETE FROM project_ai_meta WHERE key = 'project_memory_version';",
        )
        .map_err(sqlite_error)
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms = query
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms
}

fn validate_conversation_id(conversation_id: &str) -> CoreResult<()> {
    if conversation_id.is_empty()
        || conversation_id.len() > 128
        || !conversation_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(CoreError::validation(
            "project AI conversation_id must be 1-128 ASCII letters, digits, '.', '-' or '_'",
        ));
    }
    Ok(())
}

fn validate_role(role: &str) -> CoreResult<()> {
    if matches!(role, "system" | "user" | "assistant") {
        Ok(())
    } else {
        Err(CoreError::validation(format!(
            "unsupported project AI message role: {role}"
        )))
    }
}

fn unix_timestamp_ms() -> CoreResult<i64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            CoreError::validation(format!("system clock is before unix epoch: {error}"))
        })?
        .as_millis();
    i64::try_from(millis)
        .map_err(|_| CoreError::validation("project AI timestamp exceeds SQLite range"))
}

fn sqlite_error(error: rusqlite::Error) -> CoreError {
    CoreError::External {
        service: "sqlite".to_owned(),
        message: error.to_string(),
    }
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation(format!(
        "project AI conversation store lock poisoned: {error}"
    ))
}
