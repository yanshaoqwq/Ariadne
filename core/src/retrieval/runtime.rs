use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::contracts::{CoreError, CoreResult, SourceSpan, TextRange};
use crate::documents::{IndexInvalidationEvent, IndexInvalidationOutbox};
use crate::retrieval::{ChunkDocument, FullTextRecord, FullTextStore};

/// 单次索引 worker 执行结果，供诊断和后台调度记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexingWorkerReport {
    pub event_id: String,
    pub document_id: String,
    pub source_version: String,
    pub indexed_chunks: usize,
    pub superseded: bool,
}

/// 消费文档 outbox，并同步写入 Tantivy 与 SQLite FTS。
pub struct IndexingWorker {
    outbox: IndexInvalidationOutbox,
    tantivy: Arc<dyn FullTextStore>,
    sqlite: Arc<dyn FullTextStore>,
    chunk_size_chars: usize,
    chunk_overlap_chars: usize,
}

impl IndexingWorker {
    pub fn new(
        outbox: IndexInvalidationOutbox,
        tantivy: Arc<dyn FullTextStore>,
        sqlite: Arc<dyn FullTextStore>,
        chunk_size_chars: usize,
        chunk_overlap_chars: usize,
    ) -> CoreResult<Self> {
        if chunk_size_chars == 0 {
            return Err(CoreError::validation("chunk_size_chars must be positive"));
        }
        if chunk_overlap_chars >= chunk_size_chars {
            return Err(CoreError::validation(
                "chunk_overlap_chars must be smaller than chunk_size_chars",
            ));
        }
        Ok(Self {
            outbox,
            tantivy,
            sqlite,
            chunk_size_chars,
            chunk_overlap_chars,
        })
    }

    /// 处理一个待索引事件；没有事件时返回 None。
    pub fn process_next(&self) -> CoreResult<Option<IndexingWorkerReport>> {
        let Some(event) = self.outbox.claim_next()? else {
            return Ok(None);
        };
        match self.process_event(&event) {
            Ok(report) => {
                if !report.superseded {
                    self.outbox.complete(&event.event_id)?;
                }
                Ok(Some(report))
            }
            Err(error) => {
                self.outbox.retry(&event.event_id)?;
                Err(error)
            }
        }
    }

    fn process_event(&self, event: &IndexInvalidationEvent) -> CoreResult<IndexingWorkerReport> {
        if event.full_rebuild_required {
            let records = collect_project_full_text_records(
                Path::new(&event.document_id),
                self.chunk_size_chars,
                self.chunk_overlap_chars,
            )?;
            let indexed_chunks = records.len();
            self.tantivy.rebuild_from_records(records.clone())?;
            self.sqlite.rebuild_from_records(records)?;
            return Ok(IndexingWorkerReport {
                event_id: event.event_id.clone(),
                document_id: event.document_id.clone(),
                source_version: event.source_version.clone(),
                indexed_chunks,
                superseded: false,
            });
        }
        let content = fs::read_to_string(&event.document_id)?;
        let actual_version = content_version(content.as_bytes());
        if actual_version != event.source_version {
            self.outbox.supersede(&event.event_id)?;
            return Ok(IndexingWorkerReport {
                event_id: event.event_id.clone(),
                document_id: event.document_id.clone(),
                source_version: event.source_version.clone(),
                indexed_chunks: 0,
                superseded: true,
            });
        }
        let chunks = chunk_document(
            &event.document_id,
            &event.source_version,
            &content,
            self.chunk_size_chars,
            self.chunk_overlap_chars,
        )?;
        self.tantivy.delete_document(&event.document_id)?;
        self.sqlite.delete_document(&event.document_id)?;
        let records = chunks
            .iter()
            .cloned()
            .map(|chunk| FullTextRecord { chunk })
            .collect::<Vec<_>>();
        self.tantivy.upsert(records.clone())?;
        self.sqlite.upsert(records)?;
        Ok(IndexingWorkerReport {
            event_id: event.event_id.clone(),
            document_id: event.document_id.clone(),
            source_version: event.source_version.clone(),
            indexed_chunks: chunks.len(),
            superseded: false,
        })
    }
}

fn collect_project_full_text_records(
    project_root: &Path,
    chunk_size_chars: usize,
    overlap_chars: usize,
) -> CoreResult<Vec<FullTextRecord>> {
    let mut paths = Vec::new();
    for directory in ["documents", "planning"] {
        collect_text_paths(&project_root.join(directory), &mut paths)?;
    }
    paths.sort();
    let mut records = Vec::new();
    for path in paths {
        let content = fs::read_to_string(&path)?;
        let version = content_version(content.as_bytes());
        let document_id = path
            .canonicalize()
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        records.extend(
            chunk_document(
                &document_id,
                &version,
                &content,
                chunk_size_chars,
                overlap_chars,
            )?
            .into_iter()
            .map(|chunk| FullTextRecord { chunk }),
        );
    }
    Ok(records)
}

fn collect_text_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> CoreResult<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            collect_text_paths(&path, paths)?;
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("md" | "markdown" | "txt" | "text" | "json")
        ) {
            paths.push(path);
        }
    }
    Ok(())
}

fn chunk_document(
    document_id: &str,
    version: &str,
    content: &str,
    chunk_size_chars: usize,
    overlap_chars: usize,
) -> CoreResult<Vec<ChunkDocument>> {
    if content.is_empty() {
        return Ok(Vec::new());
    }
    let boundaries = content
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(content.len()))
        .collect::<Vec<_>>();
    let char_count = boundaries.len().saturating_sub(1);
    let step = chunk_size_chars - overlap_chars;
    let mut chunks = Vec::new();
    let mut start_char = 0;
    while start_char < char_count {
        let end_char = start_char.saturating_add(chunk_size_chars).min(char_count);
        let start = boundaries[start_char];
        let end = boundaries[end_char];
        let chunk_id = format!("{}:{}-{}:{}", document_id, start, end, version);
        chunks.push(ChunkDocument {
            chunk_id,
            document_id: document_id.to_owned(),
            text: content[start..end].to_owned(),
            sources: vec![SourceSpan {
                document_id: document_id.to_owned(),
                range: TextRange::new(start as u64, end as u64)?,
                version: Some(version.to_owned()),
            }],
            metadata: json!({
                "source_version": version,
                "start_offset": start,
                "end_offset": end,
            }),
        });
        if end_char == char_count {
            break;
        }
        start_char = start_char.saturating_add(step);
    }
    Ok(chunks)
}

fn content_version(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
