use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::contracts::{CoreError, CoreResult, ExecutionCancellation, SourceSpan, TextRange};
use crate::documents::{IndexInvalidationEvent, IndexInvalidationOutbox};
use crate::providers::ProviderCallContext;
use crate::retrieval::models::VectorRecord;
use crate::retrieval::traits::{TextEmbedder, VectorStore};
use crate::retrieval::{ChunkDocument, FullTextRecord, FullTextStore};

const EMBEDDING_BATCH_SIZE: usize = 64;

/// 单次索引 worker 执行结果，供诊断和后台调度记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexingWorkerReport {
    pub event_id: String,
    pub document_id: String,
    pub source_version: String,
    pub indexed_chunks: usize,
    pub superseded: bool,
    /// 是否对本事件执行了向量 upsert（未配置向量后端时为 false）。
    #[serde(default)]
    pub vector_indexed: bool,
}

/// 消费文档 outbox，写入 Tantivy + SQLite FTS，可选同步 VectorStore。
pub struct IndexingWorker {
    outbox: IndexInvalidationOutbox,
    tantivy: Arc<dyn FullTextStore>,
    sqlite: Arc<dyn FullTextStore>,
    /// F1：配置可达时注入；None 表示仅全文（明确不写向量）。
    vector: Option<Arc<dyn VectorStore>>,
    /// 与 vector 成对存在；生产向量只能来自真实 EmbeddingProvider。
    embedder: Option<Arc<dyn TextEmbedder>>,
    mutation_lock_path: PathBuf,
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
        Self::build(
            outbox,
            tantivy,
            sqlite,
            None,
            None,
            chunk_size_chars,
            chunk_overlap_chars,
        )
    }

    /// F1：真实向量构造入口；VectorStore 与 TextEmbedder 必须成对注入。
    pub fn with_vector_store(
        outbox: IndexInvalidationOutbox,
        tantivy: Arc<dyn FullTextStore>,
        sqlite: Arc<dyn FullTextStore>,
        vector: Arc<dyn VectorStore>,
        embedder: Arc<dyn TextEmbedder>,
        chunk_size_chars: usize,
        chunk_overlap_chars: usize,
    ) -> CoreResult<Self> {
        Self::build(
            outbox,
            tantivy,
            sqlite,
            Some(vector),
            Some(embedder),
            chunk_size_chars,
            chunk_overlap_chars,
        )
    }

    fn build(
        outbox: IndexInvalidationOutbox,
        tantivy: Arc<dyn FullTextStore>,
        sqlite: Arc<dyn FullTextStore>,
        vector: Option<Arc<dyn VectorStore>>,
        embedder: Option<Arc<dyn TextEmbedder>>,
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
        if vector.is_some() != embedder.is_some() {
            return Err(CoreError::validation(
                "vector store and text embedder must be configured together",
            ));
        }
        if embedder
            .as_ref()
            .is_some_and(|value| value.dimensions() == 0)
        {
            return Err(CoreError::validation(
                "embedding dimensions must be positive",
            ));
        }
        let mutation_lock_path = retrieval_index_lock_path(&outbox);
        Ok(Self {
            outbox,
            tantivy,
            sqlite,
            vector,
            embedder,
            mutation_lock_path,
            chunk_size_chars,
            chunk_overlap_chars,
        })
    }

    /// 处理一个待索引事件；没有事件时返回 None。
    pub fn process_next(&self) -> CoreResult<Option<IndexingWorkerReport>> {
        self.process_next_with_cancellation(&ExecutionCancellation::new())
    }

    pub fn process_next_with_cancellation(
        &self,
        cancellation: &ExecutionCancellation,
    ) -> CoreResult<Option<IndexingWorkerReport>> {
        cancellation.check()?;
        let Some(event) = self.outbox.claim_next()? else {
            return Ok(None);
        };
        match self.process_event(&event, cancellation) {
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

    fn process_event(
        &self,
        event: &IndexInvalidationEvent,
        cancellation: &ExecutionCancellation,
    ) -> CoreResult<IndexingWorkerReport> {
        cancellation.check()?;
        if event.full_rebuild_required {
            let records = collect_project_full_text_records(
                Path::new(&event.document_id),
                self.chunk_size_chars,
                self.chunk_overlap_chars,
                cancellation,
            )?;
            let indexed_chunks = records.len();
            let vectors = self.embed_records(event, &records, cancellation)?;
            cancellation.check()?;
            let _index_lock = crate::retrieval::knowledge::acquire_retrieval_index_lock(
                &self.mutation_lock_path,
            )?;
            cancellation.check()?;
            self.tantivy.rebuild_from_records(records.clone())?;
            self.sqlite.rebuild_from_records(records)?;
            let vector_indexed = if let (Some(vector), Some(vectors)) = (&self.vector, vectors) {
                vector.rebuild_from_records(vectors)?;
                true
            } else {
                false
            };
            return Ok(IndexingWorkerReport {
                event_id: event.event_id.clone(),
                document_id: event.document_id.clone(),
                source_version: event.source_version.clone(),
                indexed_chunks,
                superseded: false,
                vector_indexed,
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
                vector_indexed: false,
            });
        }
        let chunks = chunk_document(
            &event.document_id,
            &event.source_version,
            &content,
            self.chunk_size_chars,
            self.chunk_overlap_chars,
            cancellation,
        )?;
        let records = chunks
            .iter()
            .cloned()
            .map(|chunk| FullTextRecord { chunk })
            .collect::<Vec<_>>();
        // 远端 embedding 先完成，再修改任一索引；失败时旧索引保持原状。
        let vectors = self.embed_records(event, &records, cancellation)?;
        cancellation.check()?;
        let _index_lock =
            crate::retrieval::knowledge::acquire_retrieval_index_lock(&self.mutation_lock_path)?;
        cancellation.check()?;
        self.tantivy.delete_document(&event.document_id)?;
        self.sqlite.delete_document(&event.document_id)?;
        if let Some(vector) = &self.vector {
            vector.delete_document(&event.document_id)?;
        }
        self.tantivy.upsert(records.clone())?;
        self.sqlite.upsert(records.clone())?;
        let vector_indexed = if let (Some(vector), Some(vectors)) = (&self.vector, vectors) {
            vector.upsert(vectors).map_err(|error| {
                CoreError::validation(format!(
                    "vector index upsert failed after full-text write for {}: {error}",
                    event.document_id
                ))
            })?;
            true
        } else {
            false
        };
        Ok(IndexingWorkerReport {
            event_id: event.event_id.clone(),
            document_id: event.document_id.clone(),
            source_version: event.source_version.clone(),
            indexed_chunks: chunks.len(),
            superseded: false,
            vector_indexed,
        })
    }

    fn embed_records(
        &self,
        event: &IndexInvalidationEvent,
        records: &[FullTextRecord],
        cancellation: &ExecutionCancellation,
    ) -> CoreResult<Option<Vec<VectorRecord>>> {
        let Some(embedder) = &self.embedder else {
            return Ok(None);
        };
        let mut vector_records = Vec::with_capacity(records.len());
        for (batch_index, batch) in records.chunks(EMBEDDING_BATCH_SIZE).enumerate() {
            cancellation.check()?;
            let mut context = ProviderCallContext::new(embedder.provider_id());
            context.cancellation = cancellation.clone();
            context.operation_id = Some(format!(
                "retrieval-index-embedding:{}:{}:{batch_index}",
                event.event_id, event.source_version
            ));
            let embeddings = embedder.embed(
                context,
                batch
                    .iter()
                    .map(|record| record.chunk.text.clone())
                    .collect(),
            )?;
            if embeddings.len() != batch.len() {
                return Err(CoreError::validation(format!(
                    "embedding provider returned {} vectors for {} chunks",
                    embeddings.len(),
                    batch.len()
                )));
            }
            vector_records.extend(batch.iter().zip(embeddings).map(|(record, embedding)| {
                VectorRecord {
                    chunk: record.chunk.clone(),
                    embedding,
                }
            }));
        }
        Ok(Some(vector_records))
    }
}

fn retrieval_index_lock_path(outbox: &IndexInvalidationOutbox) -> PathBuf {
    let database_parent = outbox.path().parent().unwrap_or_else(|| Path::new("."));
    let project_root =
        if database_parent.file_name().and_then(|value| value.to_str()) == Some(".runtime") {
            database_parent.parent().unwrap_or(database_parent)
        } else {
            database_parent
        };
    project_root.join(".indexes").join("retrieval-index.lock")
}

fn collect_project_full_text_records(
    project_root: &Path,
    chunk_size_chars: usize,
    overlap_chars: usize,
    cancellation: &ExecutionCancellation,
) -> CoreResult<Vec<FullTextRecord>> {
    let mut paths = Vec::new();
    let config = crate::config::ConfigStore::new(project_root).load_or_create()?;
    let layout = crate::config::ProjectLayout::from_app(project_root, &config.app)?;
    for directory in [layout.documents, project_root.join("planning")] {
        collect_text_paths(&directory, &mut paths)?;
    }
    paths.sort();
    let mut records = Vec::new();
    for path in paths {
        cancellation.check()?;
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
                cancellation,
            )?
            .into_iter()
            .map(|chunk| FullTextRecord { chunk }),
        );
    }
    // F2-c：full rebuild 必须与增量知识同步包含同一四层已确认知识，
    // 否则 Git restore / 配置重建会把知识候选从正式索引中抹掉。
    let (_, knowledge_records) = crate::retrieval::knowledge::records_from_project(project_root)?;
    records.extend(knowledge_records);
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
    cancellation: &ExecutionCancellation,
) -> CoreResult<Vec<ChunkDocument>> {
    cancellation.check()?;
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
        if chunks.len() % 256 == 0 {
            cancellation.check()?;
        }
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
    content_version_for_bytes(bytes)
}

/// 与索引 chunk / outbox `source_version` 同一算法（F2-b 新鲜度比对）。
pub fn content_version_for_bytes(bytes: &[u8]) -> String {
    crate::contracts::content_version_for_bytes(bytes)
}
