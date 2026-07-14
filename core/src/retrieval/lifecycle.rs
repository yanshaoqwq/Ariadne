//! F2-a / F2-b：项目索引生命周期共享边界。
//!
//! - **F2-a**：打开项目时，若已有可索引源但全文索引为空/缺失且 outbox 无在途 full rebuild，
//!   则入队与 Git restore 相同的 `full_rebuild_required` 事件。
//! - **F2-b**：搜索结果不得把磁盘上已过期的 chunk 当作当前事实；按 `source_version` 与正文比对过滤。

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::contracts::{CoreError, CoreResult};
use crate::documents::IndexInvalidationOutbox;
use crate::retrieval::models::RetrievalResult;
use crate::retrieval::runtime::content_version_for_bytes;

/// 项目是否在 `documents/` 或 `planning/` 下有可索引文本源。
pub fn has_indexable_project_sources(project_root: &Path) -> CoreResult<bool> {
    for directory in ["documents", "planning"] {
        if directory_has_indexable_text(&project_root.join(directory))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn directory_has_indexable_text(directory: &Path) -> CoreResult<bool> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            if directory_has_indexable_text(&path)? {
                return Ok(true);
            }
        } else if is_indexable_extension(&path) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_indexable_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("md" | "markdown" | "txt" | "text" | "json")
    )
}

/// SQLite 全文索引是否为空或缺失（F2-a 判定）。
pub fn full_text_index_is_empty(sqlite_path: &Path) -> CoreResult<bool> {
    if !sqlite_path.exists() {
        return Ok(true);
    }
    let connection = Connection::open(sqlite_path).map_err(|error| {
        CoreError::validation(format!(
            "failed to open full-text index for bootstrap check: {error}"
        ))
    })?;
    let table_exists: bool = connection
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='full_text_chunks'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !table_exists {
        return Ok(true);
    }
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM full_text_chunks", [], |row| {
            row.get(0)
        })
        .map_err(|error| {
            CoreError::validation(format!("failed to count full-text chunks: {error}"))
        })?;
    Ok(count <= 0)
}

/// F2-a：是否应在打开时调度全量重建。
pub fn project_needs_index_bootstrap(
    project_root: &Path,
    sqlite_index_path: &Path,
    outbox: &IndexInvalidationOutbox,
) -> CoreResult<bool> {
    if !has_indexable_project_sources(project_root)? {
        return Ok(false);
    }
    if outbox.has_incomplete_full_rebuild()? {
        return Ok(false);
    }
    full_text_index_is_empty(sqlite_index_path)
}

/// F2-a：幂等入队 open 引导用 full rebuild（与 Git restore 同事件形状）。
/// 返回 `Some(event_id)` 表示新入队；`None` 表示无需或已有在途 rebuild。
pub fn enqueue_open_bootstrap_full_rebuild(
    project_root: &Path,
    sqlite_index_path: &Path,
    outbox: &IndexInvalidationOutbox,
) -> CoreResult<Option<String>> {
    if !project_needs_index_bootstrap(project_root, sqlite_index_path, outbox)? {
        return Ok(None);
    }
    let root_id = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let event_id = outbox.prepare(
        &root_id,
        "open_project_bootstrap_full_rebuild",
        "bootstrap",
        true,
    )?;
    outbox.activate(&event_id)?;
    Ok(Some(event_id))
}

/// F2-b：丢弃与磁盘正文 `source_version` 不一致的检索结果，禁止旧 chunk 冒充当前事实。
pub fn filter_fresh_retrieval_results(
    results: Vec<RetrievalResult>,
) -> CoreResult<Vec<RetrievalResult>> {
    filter_fresh_retrieval_results_with_knowledge_revision(results, None)
}

/// 产品组合根在同步 metadata.db 后携带知识 revision，正文与知识分别执行新鲜度校验。
pub fn filter_fresh_retrieval_results_with_knowledge_revision(
    results: Vec<RetrievalResult>,
    knowledge_revision: Option<&str>,
) -> CoreResult<Vec<RetrievalResult>> {
    let mut fresh = Vec::with_capacity(results.len());
    for result in results {
        match result_is_current(&result, knowledge_revision) {
            Ok(true) => fresh.push(result),
            Ok(false) => {
                // 过期：静默丢弃，不得当作当前事实返回。
            }
            Err(error) => {
                // 无法读取磁盘时 fail-loud，避免在不确定状态下返回可能过期的片段。
                return Err(error);
            }
        }
    }
    Ok(fresh)
}

fn result_is_current(
    result: &RetrievalResult,
    knowledge_revision: Option<&str>,
) -> CoreResult<bool> {
    if result
        .metadata
        .get("ariadne_retrieval")
        .and_then(|value| value.get("source_kind"))
        .and_then(|value| value.as_str())
        == Some("knowledge")
    {
        let indexed_revision = result
            .metadata
            .get("knowledge_revision")
            .and_then(|value| value.as_str());
        return Ok(indexed_revision.is_some() && indexed_revision == knowledge_revision);
    }
    result_is_current_on_disk(result)
}

fn result_is_current_on_disk(result: &RetrievalResult) -> CoreResult<bool> {
    let path = PathBuf::from(&result.document_id);
    if !path.is_file() {
        // 无真实路径（内存夹具 / 已删除源）时不放行，避免伪造“当前事实”。
        return Ok(false);
    }
    let bytes = fs::read(&path)?;
    let current = content_version_for_bytes(&bytes);
    let indexed = result
        .metadata
        .get("source_version")
        .and_then(|value| value.as_str())
        .or_else(|| result.spans.iter().find_map(|span| span.version.as_deref()));
    Ok(indexed == Some(current.as_str()))
}

/// F2-b：outbox 仍有未完成失效时，搜索不得假装索引已与最新保存一致。
pub fn ensure_search_not_blocked_by_pending_index(
    outbox: &IndexInvalidationOutbox,
) -> CoreResult<()> {
    if outbox.has_incomplete_invalidation()? {
        return Err(CoreError::validation(
            "indexing_not_ready: project search blocked while index invalidation is pending or processing",
        ));
    }
    Ok(())
}
