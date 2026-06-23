use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::core::{CoreError, CoreResult, NodeId, RunId, WorkflowId};
use crate::costs::models::{CostCategory, CostQuery, CostRecord, NewCostRecord};

pub const COSTS_DB_FILE: &str = "costs.db";
const SCHEMA_VERSION: i64 = 1;

/// 成本账本抽象，供内存/SQLite/后续其他存储复用。
pub trait CostLedger: Send + Sync {
    /// 记录一次成本事件。
    fn record_cost(&self, record: NewCostRecord) -> CoreResult<CostRecord>;
    /// 计算符合条件的总成本。
    fn total_cost(&self, query: &CostQuery) -> CoreResult<f64>;
    /// 列出符合条件的成本事件。
    fn list_costs(&self, query: &CostQuery) -> CoreResult<Vec<CostRecord>>;
}

/// SQLite 成本账本实现。
#[derive(Debug)]
pub struct SqliteCostLedger {
    db_path: Option<PathBuf>,
    connection: Mutex<Connection>,
}

impl SqliteCostLedger {
    /// 在项目根目录打开 `costs.db`。
    pub fn open(project_root: impl AsRef<Path>) -> CoreResult<Self> {
        let db_path = project_root.as_ref().join(COSTS_DB_FILE);
        let connection = Connection::open(&db_path).map_err(sqlite_error)?;
        let ledger = Self {
            db_path: Some(db_path),
            connection: Mutex::new(connection),
        };
        ledger.migrate()?;
        Ok(ledger)
    }

    /// 打开内存数据库，主要用于测试。
    pub fn open_in_memory() -> CoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(sqlite_error)?;
        let ledger = Self {
            db_path: None,
            connection: Mutex::new(connection),
        };
        ledger.migrate()?;
        Ok(ledger)
    }

    /// 返回数据库路径；内存模式下为 None。
    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    /// 执行成本数据库幂等迁移。
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

                CREATE TABLE IF NOT EXISTS cost_events (
                    cost_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    occurred_at_ms INTEGER NOT NULL,
                    category TEXT NOT NULL,
                    provider_id TEXT,
                    model_id TEXT,
                    workflow_id TEXT,
                    run_id TEXT,
                    node_id TEXT,
                    tool_call_id TEXT,
                    input_tokens INTEGER,
                    output_tokens INTEGER,
                    amount_usd REAL NOT NULL CHECK(amount_usd >= 0),
                    metadata_json TEXT NOT NULL DEFAULT '{}'
                );

                CREATE INDEX IF NOT EXISTS idx_cost_events_time
                    ON cost_events(occurred_at_ms);
                CREATE INDEX IF NOT EXISTS idx_cost_events_workflow
                    ON cost_events(workflow_id, run_id, node_id);
                CREATE INDEX IF NOT EXISTS idx_cost_events_category
                    ON cost_events(category);
                ",
            )
            .map_err(sqlite_error)?;

        connection
            .execute(
                "
                INSERT INTO schema_migrations(name, version, applied_at_ms)
                VALUES('costs', ?1, ?2)
                ON CONFLICT(name) DO UPDATE SET
                    version = excluded.version,
                    applied_at_ms = excluded.applied_at_ms
                ",
                params![SCHEMA_VERSION, unix_timestamp_ms_i64()?],
            )
            .map_err(sqlite_error)?;

        Ok(())
    }
}

impl CostLedger for SqliteCostLedger {
    /// 写入成本事件并返回带数据库 id 的记录。
    fn record_cost(&self, record: NewCostRecord) -> CoreResult<CostRecord> {
        record.validate()?;
        let occurred_at_ms = u64_to_i64(record.occurred_at_ms, "occurred_at_ms")?;
        let input_tokens = optional_u64_to_i64(record.input_tokens, "input_tokens")?;
        let output_tokens = optional_u64_to_i64(record.output_tokens, "output_tokens")?;
        let metadata_json = serde_json::to_string(&record.metadata)?;

        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .execute(
                "
                INSERT INTO cost_events (
                    occurred_at_ms, category, provider_id, model_id,
                    workflow_id, run_id, node_id, tool_call_id,
                    input_tokens, output_tokens, amount_usd, metadata_json
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ",
                params![
                    occurred_at_ms,
                    record.category.as_str(),
                    record.provider_id.as_deref(),
                    record.model_id.as_deref(),
                    record.workflow_id.as_ref().map(WorkflowId::as_str),
                    record.run_id.as_ref().map(RunId::as_str),
                    record.node_id.as_ref().map(NodeId::as_str),
                    record.tool_call_id.as_deref(),
                    input_tokens,
                    output_tokens,
                    record.amount_usd,
                    metadata_json,
                ],
            )
            .map_err(sqlite_error)?;

        let cost_id = connection.last_insert_rowid();
        Ok(CostRecord { cost_id, record })
    }

    /// 通过 list_costs 复用查询逻辑并累加金额。
    fn total_cost(&self, query: &CostQuery) -> CoreResult<f64> {
        let records = self.list_costs(query)?;
        Ok(records
            .iter()
            .map(|record| record.record.amount_usd)
            .sum::<f64>())
    }

    /// 查询全部记录后在 Rust 层过滤。
    fn list_costs(&self, query: &CostQuery) -> CoreResult<Vec<CostRecord>> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "
                SELECT
                    cost_id, occurred_at_ms, category, provider_id, model_id,
                    workflow_id, run_id, node_id, tool_call_id,
                    input_tokens, output_tokens, amount_usd, metadata_json
                FROM cost_events
                ORDER BY occurred_at_ms ASC, cost_id ASC
                ",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], |row| row_to_cost_record(row))
            .map_err(sqlite_error)?;

        let mut records = Vec::new();
        for row in rows {
            let record = row.map_err(sqlite_error)?;
            if matches_query(&record, query) {
                records.push(record);
            }
        }

        Ok(records)
    }
}

/// 读取成本数据库当前 schema version。
pub fn schema_version(ledger: &SqliteCostLedger) -> CoreResult<Option<i64>> {
    let connection = ledger.connection.lock().map_err(lock_error)?;
    connection
        .query_row(
            "SELECT version FROM schema_migrations WHERE name = 'costs'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_error)
}

/// 将 SQLite 行转换为成本记录。
fn row_to_cost_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<CostRecord> {
    let category_raw: String = row.get(2)?;
    let metadata_json: String = row.get(12)?;
    // metadata 不能阻断账本读取；损坏时降级为 Null，保留其他字段。
    let metadata = serde_json::from_str::<Value>(&metadata_json).unwrap_or(Value::Null);

    Ok(CostRecord {
        cost_id: row.get(0)?,
        record: NewCostRecord {
            occurred_at_ms: i64_to_u64(row.get(1)?, "occurred_at_ms"),
            category: CostCategory::parse(&category_raw).unwrap_or(CostCategory::Other),
            provider_id: row.get(3)?,
            model_id: row.get(4)?,
            workflow_id: row.get::<_, Option<String>>(5)?.map(WorkflowId::new),
            run_id: row.get::<_, Option<String>>(6)?.map(RunId::new),
            node_id: row.get::<_, Option<String>>(7)?.map(NodeId::new),
            tool_call_id: row.get(8)?,
            input_tokens: row.get::<_, Option<i64>>(9)?.and_then(i64_to_u64_checked),
            output_tokens: row.get::<_, Option<i64>>(10)?.and_then(i64_to_u64_checked),
            amount_usd: row.get(11)?,
            metadata,
        },
    })
}

/// 判断记录是否满足查询条件。
fn matches_query(record: &CostRecord, query: &CostQuery) -> bool {
    if query
        .start_ms
        .is_some_and(|start| record.record.occurred_at_ms < start)
    {
        return false;
    }

    if query
        .end_ms
        .is_some_and(|end| record.record.occurred_at_ms >= end)
    {
        return false;
    }

    if query
        .workflow_id
        .as_ref()
        .is_some_and(|id| record.record.workflow_id.as_ref() != Some(id))
    {
        return false;
    }

    if query
        .run_id
        .as_ref()
        .is_some_and(|id| record.record.run_id.as_ref() != Some(id))
    {
        return false;
    }

    if query
        .node_id
        .as_ref()
        .is_some_and(|id| record.record.node_id.as_ref() != Some(id))
    {
        return false;
    }

    if query
        .category
        .is_some_and(|category| record.record.category != category)
    {
        return false;
    }

    true
}

/// 返回当前 Unix 毫秒时间戳，并转成 SQLite 友好的 i64。
fn unix_timestamp_ms_i64() -> CoreResult<i64> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            CoreError::validation(format!("system time before unix epoch: {error}"))
        })?;
    u64_to_i64(duration.as_millis() as u64, "timestamp_ms")
}

/// 将可选 u64 安全转换成 i64。
fn optional_u64_to_i64(value: Option<u64>, field: &str) -> CoreResult<Option<i64>> {
    value.map(|value| u64_to_i64(value, field)).transpose()
}

/// 将 u64 安全转换成 i64，避免写入 SQLite 时溢出。
fn u64_to_i64(value: u64, field: &str) -> CoreResult<i64> {
    i64::try_from(value).map_err(|_| CoreError::validation(format!("{field} exceeds i64 range")))
}

/// 将 SQLite i64 转回 u64；异常负数在 debug 下暴露。
fn i64_to_u64(value: i64, field: &str) -> u64 {
    i64_to_u64_checked(value).unwrap_or_else(|| {
        debug_assert!(false, "{field} cannot be negative");
        0
    })
}

/// 将非负 i64 转成 u64。
fn i64_to_u64_checked(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

/// 将锁中毒转换成统一错误。
fn lock_error<T>(_: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation("cost ledger lock poisoned")
}

/// 将 rusqlite 错误转换成统一外部服务错误。
fn sqlite_error(error: rusqlite::Error) -> CoreError {
    CoreError::External {
        service: "sqlite".to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn migration_is_idempotent() {
        let ledger = SqliteCostLedger::open_in_memory().unwrap();
        ledger.migrate().unwrap();

        assert_eq!(schema_version(&ledger).unwrap(), Some(SCHEMA_VERSION));
    }

    #[test]
    fn ledger_records_and_sums_costs() {
        let ledger = SqliteCostLedger::open_in_memory().unwrap();
        ledger
            .record_cost(NewCostRecord {
                occurred_at_ms: 10,
                category: CostCategory::Llm,
                provider_id: Some("openai".to_owned()),
                model_id: Some("gpt".to_owned()),
                workflow_id: Some(WorkflowId::new("wf-1")),
                run_id: Some(RunId::new("run-1")),
                node_id: Some(NodeId::new("node-1")),
                tool_call_id: Some("tool-1".to_owned()),
                input_tokens: Some(100),
                output_tokens: Some(50),
                amount_usd: 0.25,
                metadata: json!({ "round": 1 }),
            })
            .unwrap();
        ledger
            .record_cost(NewCostRecord {
                occurred_at_ms: 20,
                category: CostCategory::SearchApi,
                provider_id: Some("search".to_owned()),
                model_id: None,
                workflow_id: Some(WorkflowId::new("wf-1")),
                run_id: Some(RunId::new("run-1")),
                node_id: Some(NodeId::new("node-2")),
                tool_call_id: None,
                input_tokens: None,
                output_tokens: None,
                amount_usd: 0.10,
                metadata: Value::Null,
            })
            .unwrap();

        let total = ledger
            .total_cost(&CostQuery {
                workflow_id: Some(WorkflowId::new("wf-1")),
                ..CostQuery::default()
            })
            .unwrap();

        assert_eq!(total, 0.35);
    }
}
