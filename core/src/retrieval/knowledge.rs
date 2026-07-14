use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fs4::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult, ExecutionCancellation};
use crate::providers::ProviderCallContext;
use crate::rag::{KnowledgeRetrievalSnapshot, SqliteWritingKnowledgeStore};
use crate::retrieval::{
    ChunkDocument, FullTextRecord, FullTextStore, StoreHealth, TextEmbedder, VectorRecord,
    VectorStore,
};

const KNOWLEDGE_INDEX_SCHEMA_VERSION: u32 = 1;
const KNOWLEDGE_EMBEDDING_BATCH_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KnowledgeIndexManifest {
    schema_version: u32,
    revision: String,
    #[serde(default)]
    vector_signature: Option<String>,
    #[serde(default)]
    document_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KnowledgeIndexMarker {
    schema_version: u32,
    target_revision: String,
    #[serde(default)]
    vector_signature: Option<String>,
}

/// 四层知识索引同步结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeIndexSyncReport {
    pub revision: String,
    pub indexed_records: usize,
    pub changed: bool,
}

/// 将 metadata.db 的已确认四层知识同步到项目正式全文/向量索引。
pub struct KnowledgeIndexSynchronizer {
    project_root: PathBuf,
    manifest_path: PathBuf,
    marker_path: PathBuf,
    mutation_lock_path: PathBuf,
}

impl KnowledgeIndexSynchronizer {
    pub fn new(project_root: impl AsRef<Path>) -> CoreResult<Self> {
        let project_root = project_root.as_ref().canonicalize()?;
        let index_root = project_root.join(".indexes");
        std::fs::create_dir_all(&index_root)?;
        Ok(Self {
            project_root,
            manifest_path: index_root.join("knowledge-index-manifest.json"),
            marker_path: index_root.join("knowledge-index-rebuild-required.json"),
            mutation_lock_path: index_root.join("retrieval-index.lock"),
        })
    }

    pub fn sync(
        &self,
        tantivy: &Arc<dyn FullTextStore>,
        sqlite: &Arc<dyn FullTextStore>,
        vector: Option<&Arc<dyn VectorStore>>,
        embedder: Option<&Arc<dyn TextEmbedder>>,
        vector_signature: Option<&str>,
        cancellation: Option<&ExecutionCancellation>,
    ) -> CoreResult<KnowledgeIndexSyncReport> {
        if vector.is_some() != embedder.is_some() || vector.is_some() != vector_signature.is_some()
        {
            return Err(CoreError::validation(
                "knowledge index vector store, embedder and signature must be configured together",
            ));
        }

        let snapshot =
            SqliteWritingKnowledgeStore::open(&self.project_root)?.load_retrieval_snapshot()?;
        let records = records_from_snapshot(&snapshot)?;
        let document_ids = records
            .iter()
            .map(|record| record.chunk.document_id.clone())
            .collect::<Vec<_>>();
        let manifest = read_manifest(&self.manifest_path)?;
        let marker = read_marker(&self.marker_path)?;
        let requested_vector_signature = vector_signature.map(str::to_owned);
        if marker.is_none()
            && manifest.as_ref().is_some_and(|manifest| {
                manifest.schema_version == KNOWLEDGE_INDEX_SCHEMA_VERSION
                    && manifest.revision == snapshot.revision
                    && manifest.vector_signature == requested_vector_signature
                    && manifest.document_ids == document_ids
            })
        {
            return Ok(KnowledgeIndexSyncReport {
                revision: snapshot.revision,
                indexed_records: records.len(),
                changed: false,
            });
        }

        // 远端 embedding 在任何索引破坏前完成；失败时旧 generation 仍完整可用。
        let vectors =
            embed_knowledge_records(&snapshot.revision, &records, embedder, cancellation)?;
        let _mutation_lock = acquire_retrieval_index_lock(&self.mutation_lock_path)?;
        crate::config::store::atomic_write(
            &self.marker_path,
            &serde_json::to_vec_pretty(&KnowledgeIndexMarker {
                schema_version: KNOWLEDGE_INDEX_SCHEMA_VERSION,
                target_revision: snapshot.revision.clone(),
                vector_signature: requested_vector_signature.clone(),
            })?,
        )?;

        let mut stale_documents = BTreeSet::new();
        if let Some(manifest) = &manifest {
            stale_documents.extend(manifest.document_ids.iter().cloned());
        }
        stale_documents.extend(document_ids.iter().cloned());
        for document_id in stale_documents {
            tantivy.delete_document(&document_id)?;
            sqlite.delete_document(&document_id)?;
            if let Some(vector) = vector {
                vector.delete_document(&document_id)?;
            }
        }
        if !records.is_empty() {
            tantivy.upsert(records.clone())?;
            sqlite.upsert(records.clone())?;
        }
        if let (Some(vector), Some(vectors)) = (vector, vectors) {
            if !vectors.is_empty() {
                vector.upsert(vectors)?;
            }
        }

        crate::config::store::atomic_write(
            &self.manifest_path,
            &serde_json::to_vec_pretty(&KnowledgeIndexManifest {
                schema_version: KNOWLEDGE_INDEX_SCHEMA_VERSION,
                revision: snapshot.revision.clone(),
                vector_signature: requested_vector_signature,
                document_ids,
            })?,
        )?;
        match std::fs::remove_file(&self.marker_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        Ok(KnowledgeIndexSyncReport {
            revision: snapshot.revision,
            indexed_records: records.len(),
            changed: true,
        })
    }

    pub fn health_check(&self, vector_signature: Option<&str>) -> CoreResult<StoreHealth> {
        if read_marker(&self.marker_path)?.is_some() {
            return Ok(StoreHealth::rebuild_required(
                "knowledge_retrieval_index",
                "knowledge index rebuild marker is present",
            ));
        }
        let snapshot =
            SqliteWritingKnowledgeStore::open(&self.project_root)?.load_retrieval_snapshot()?;
        let expected_documents = records_from_snapshot(&snapshot)?
            .into_iter()
            .map(|record| record.chunk.document_id)
            .collect::<Vec<_>>();
        let Some(manifest) = read_manifest(&self.manifest_path)? else {
            return Ok(StoreHealth::rebuild_required(
                "knowledge_retrieval_index",
                "knowledge index manifest is missing",
            ));
        };
        if manifest.schema_version != KNOWLEDGE_INDEX_SCHEMA_VERSION
            || manifest.revision != snapshot.revision
            || manifest.vector_signature != vector_signature.map(str::to_owned)
            || manifest.document_ids != expected_documents
        {
            return Ok(StoreHealth::rebuild_required(
                "knowledge_retrieval_index",
                "knowledge index manifest does not match metadata.db",
            ));
        }
        Ok(StoreHealth::healthy("knowledge_retrieval_index"))
    }
}

pub(crate) fn records_from_project(
    project_root: &Path,
) -> CoreResult<(String, Vec<FullTextRecord>)> {
    let snapshot = SqliteWritingKnowledgeStore::open(project_root)?.load_retrieval_snapshot()?;
    let records = records_from_snapshot(&snapshot)?;
    Ok((snapshot.revision, records))
}

fn records_from_snapshot(snapshot: &KnowledgeRetrievalSnapshot) -> CoreResult<Vec<FullTextRecord>> {
    snapshot
        .entries
        .iter()
        .map(|entry| {
            let layer_weight = match entry.layer.as_str() {
                "story_segment" => 1.05,
                "story_event" => 1.15,
                "chapter_summary" => 1.25,
                "stage_summary" => 1.35,
                other => {
                    return Err(CoreError::validation(format!(
                        "unknown knowledge retrieval layer: {other}"
                    )))
                }
            };
            let mut metadata = entry.metadata.as_object().cloned().ok_or_else(|| {
                CoreError::validation("knowledge retrieval metadata must be an object")
            })?;
            metadata.insert("knowledge_layer".to_owned(), json!(entry.layer));
            metadata.insert("knowledge_entity_id".to_owned(), json!(entry.entity_id));
            metadata.insert("knowledge_revision".to_owned(), json!(snapshot.revision));
            metadata.insert(
                "ariadne_retrieval".to_owned(),
                json!({
                    "source_kind": "knowledge",
                    "layer": entry.layer,
                    "layer_weight": layer_weight,
                    "knowledge_revision": snapshot.revision,
                }),
            );
            let document_id = format!("ariadne-knowledge://{}/{}", entry.layer, entry.entity_id);
            Ok(FullTextRecord {
                chunk: ChunkDocument {
                    chunk_id: format!("knowledge:{}:{}", entry.layer, entry.entity_id),
                    document_id,
                    text: entry.text.clone(),
                    sources: entry.sources.clone(),
                    metadata: Value::Object(metadata),
                },
            })
        })
        .collect()
}

fn embed_knowledge_records(
    revision: &str,
    records: &[FullTextRecord],
    embedder: Option<&Arc<dyn TextEmbedder>>,
    cancellation: Option<&ExecutionCancellation>,
) -> CoreResult<Option<Vec<VectorRecord>>> {
    let Some(embedder) = embedder else {
        return Ok(None);
    };
    let mut vectors = Vec::with_capacity(records.len());
    for (batch_index, batch) in records.chunks(KNOWLEDGE_EMBEDDING_BATCH_SIZE).enumerate() {
        let mut context = ProviderCallContext::new(embedder.provider_id());
        if let Some(cancellation) = cancellation {
            context.cancellation = cancellation.clone();
        }
        context.operation_id = Some(format!(
            "knowledge-index-embedding:{revision}:{batch_index}"
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
                "embedding provider returned {} vectors for {} knowledge records",
                embeddings.len(),
                batch.len()
            )));
        }
        vectors.extend(
            batch
                .iter()
                .zip(embeddings)
                .map(|(record, embedding)| VectorRecord {
                    chunk: record.chunk.clone(),
                    embedding,
                }),
        );
    }
    Ok(Some(vectors))
}

fn read_manifest(path: &Path) -> CoreResult<Option<KnowledgeIndexManifest>> {
    read_optional_json(path)
}

fn read_marker(path: &Path) -> CoreResult<Option<KnowledgeIndexMarker>> {
    read_optional_json(path)
}

fn read_optional_json<T: for<'de> Deserialize<'de>>(path: &Path) -> CoreResult<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn acquire_retrieval_index_lock(path: &Path) -> CoreResult<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock_exclusive()?;
    Ok(file)
}
