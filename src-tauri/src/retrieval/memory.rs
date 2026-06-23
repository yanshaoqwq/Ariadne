use std::collections::BTreeMap;
use std::sync::RwLock;

use crate::core::{CoreError, CoreResult};
use crate::retrieval::models::{
    FullTextRecord, FullTextSearchRequest, RebuildReport, RebuildStatus, RetrievalResult,
    RetrievalSource, StoreHealth, VectorRecord, VectorSearchRequest,
};
use crate::retrieval::traits::{FullTextStore, VectorStore};

#[derive(Debug, Default)]
pub struct MemoryVectorStore {
    records: RwLock<BTreeMap<String, VectorRecord>>,
    rebuild_reason: RwLock<Option<String>>,
}

impl MemoryVectorStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl VectorStore for MemoryVectorStore {
    fn upsert(&self, records: Vec<VectorRecord>) -> CoreResult<()> {
        let mut stored = self.records.write().map_err(lock_error)?;
        for record in records {
            validate_vector_record(&record)?;
            stored.insert(record.chunk.chunk_id.clone(), record);
        }
        *self.rebuild_reason.write().map_err(lock_error)? = None;
        Ok(())
    }

    fn delete_document(&self, document_id: &str) -> CoreResult<usize> {
        let mut stored = self.records.write().map_err(lock_error)?;
        let before = stored.len();
        stored.retain(|_, record| record.chunk.document_id != document_id);
        Ok(before.saturating_sub(stored.len()))
    }

    fn search(&self, request: VectorSearchRequest) -> CoreResult<Vec<RetrievalResult>> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }

        if request.query_embedding.is_empty() {
            return Err(CoreError::validation("query_embedding cannot be empty"));
        }

        let stored = self.records.read().map_err(lock_error)?;
        let mut results = Vec::new();
        for record in stored.values() {
            if !metadata_matches(&record.chunk.metadata, &request.filters) {
                continue;
            }

            let score = cosine_similarity(&request.query_embedding, &record.embedding)?;
            results.push(RetrievalResult::from_chunk(
                &record.chunk,
                score,
                RetrievalSource::Vector,
            ));
        }

        sort_and_limit(&mut results, request.limit);
        Ok(results)
    }

    fn health_check(&self) -> CoreResult<StoreHealth> {
        if let Some(reason) = self.rebuild_reason.read().map_err(lock_error)?.clone() {
            return Ok(StoreHealth::rebuild_required("memory_vector_store", reason));
        }

        Ok(StoreHealth::healthy("memory_vector_store"))
    }

    fn mark_rebuild_required(&self, reason: &str) -> CoreResult<()> {
        *self.rebuild_reason.write().map_err(lock_error)? = Some(reason.to_owned());
        Ok(())
    }

    fn rebuild_from_records(&self, records: Vec<VectorRecord>) -> CoreResult<RebuildReport> {
        let processed_items = records.len() as u64;
        let mut next = BTreeMap::new();
        for record in records {
            validate_vector_record(&record)?;
            next.insert(record.chunk.chunk_id.clone(), record);
        }

        *self.records.write().map_err(lock_error)? = next;
        *self.rebuild_reason.write().map_err(lock_error)? = None;

        Ok(RebuildReport {
            component: "memory_vector_store".to_owned(),
            status: RebuildStatus::Completed,
            processed_items,
            error: None,
        })
    }
}

#[derive(Debug, Default)]
pub struct MemoryFullTextStore {
    records: RwLock<BTreeMap<String, FullTextRecord>>,
    rebuild_reason: RwLock<Option<String>>,
}

impl MemoryFullTextStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl FullTextStore for MemoryFullTextStore {
    fn upsert(&self, records: Vec<FullTextRecord>) -> CoreResult<()> {
        let mut stored = self.records.write().map_err(lock_error)?;
        for record in records {
            validate_full_text_record(&record)?;
            stored.insert(record.chunk.chunk_id.clone(), record);
        }
        *self.rebuild_reason.write().map_err(lock_error)? = None;
        Ok(())
    }

    fn delete_document(&self, document_id: &str) -> CoreResult<usize> {
        let mut stored = self.records.write().map_err(lock_error)?;
        let before = stored.len();
        stored.retain(|_, record| record.chunk.document_id != document_id);
        Ok(before.saturating_sub(stored.len()))
    }

    fn search(&self, request: FullTextSearchRequest) -> CoreResult<Vec<RetrievalResult>> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }

        let terms = tokenize(&request.query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        let stored = self.records.read().map_err(lock_error)?;
        let mut results = Vec::new();
        for record in stored.values() {
            if !metadata_matches(&record.chunk.metadata, &request.filters) {
                continue;
            }

            let score = full_text_score(&record.chunk.text, &terms);
            if score <= 0.0 {
                continue;
            }

            results.push(RetrievalResult::from_chunk(
                &record.chunk,
                score,
                RetrievalSource::FullText,
            ));
        }

        sort_and_limit(&mut results, request.limit);
        Ok(results)
    }

    fn health_check(&self) -> CoreResult<StoreHealth> {
        if let Some(reason) = self.rebuild_reason.read().map_err(lock_error)?.clone() {
            return Ok(StoreHealth::rebuild_required(
                "memory_full_text_store",
                reason,
            ));
        }

        Ok(StoreHealth::healthy("memory_full_text_store"))
    }

    fn mark_rebuild_required(&self, reason: &str) -> CoreResult<()> {
        *self.rebuild_reason.write().map_err(lock_error)? = Some(reason.to_owned());
        Ok(())
    }

    fn rebuild_from_records(&self, records: Vec<FullTextRecord>) -> CoreResult<RebuildReport> {
        let processed_items = records.len() as u64;
        let mut next = BTreeMap::new();
        for record in records {
            validate_full_text_record(&record)?;
            next.insert(record.chunk.chunk_id.clone(), record);
        }

        *self.records.write().map_err(lock_error)? = next;
        *self.rebuild_reason.write().map_err(lock_error)? = None;

        Ok(RebuildReport {
            component: "memory_full_text_store".to_owned(),
            status: RebuildStatus::Completed,
            processed_items,
            error: None,
        })
    }
}

fn validate_vector_record(record: &VectorRecord) -> CoreResult<()> {
    validate_chunk(
        &record.chunk.chunk_id,
        &record.chunk.document_id,
        &record.chunk.text,
    )?;
    if record.embedding.is_empty() {
        return Err(CoreError::validation("vector embedding cannot be empty"));
    }

    if record.embedding.iter().any(|value| !value.is_finite()) {
        return Err(CoreError::validation(
            "vector embedding contains non-finite value",
        ));
    }

    Ok(())
}

fn validate_full_text_record(record: &FullTextRecord) -> CoreResult<()> {
    validate_chunk(
        &record.chunk.chunk_id,
        &record.chunk.document_id,
        &record.chunk.text,
    )
}

fn validate_chunk(chunk_id: &str, document_id: &str, text: &str) -> CoreResult<()> {
    if chunk_id.trim().is_empty() {
        return Err(CoreError::validation("chunk_id cannot be empty"));
    }

    if document_id.trim().is_empty() {
        return Err(CoreError::validation("document_id cannot be empty"));
    }

    if text.trim().is_empty() {
        return Err(CoreError::validation("chunk text cannot be empty"));
    }

    Ok(())
}

fn cosine_similarity(query: &[f32], embedding: &[f32]) -> CoreResult<f32> {
    if query.len() != embedding.len() {
        return Err(CoreError::validation(format!(
            "query embedding dimension {} does not match record dimension {}",
            query.len(),
            embedding.len()
        )));
    }

    let mut dot = 0.0_f32;
    let mut query_norm = 0.0_f32;
    let mut embedding_norm = 0.0_f32;
    for (left, right) in query.iter().zip(embedding) {
        dot += left * right;
        query_norm += left * left;
        embedding_norm += right * right;
    }

    if query_norm == 0.0 || embedding_norm == 0.0 {
        return Ok(0.0);
    }

    Ok(dot / (query_norm.sqrt() * embedding_norm.sqrt()))
}

fn full_text_score(text: &str, terms: &[String]) -> f32 {
    let normalized_text = text.to_lowercase();
    let mut score = 0.0_f32;
    for term in terms {
        let occurrences = normalized_text.matches(term).count();
        score += occurrences as f32;
    }
    score / terms.len() as f32
}

fn tokenize(query: &str) -> Vec<String> {
    query
        .split(|character: char| character.is_whitespace() || character.is_ascii_punctuation())
        .filter_map(|term| {
            let normalized = term.trim().to_lowercase();
            (!normalized.is_empty()).then_some(normalized)
        })
        .collect()
}

fn metadata_matches(metadata: &serde_json::Value, filters: &BTreeMap<String, String>) -> bool {
    filters.iter().all(|(key, expected)| {
        metadata
            .get(key)
            .and_then(|value| value.as_str())
            .is_some_and(|actual| actual == expected)
    })
}

pub(crate) fn sort_and_limit(results: &mut Vec<RetrievalResult>, limit: usize) {
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    results.truncate(limit);
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation(format!("retrieval store lock poisoned: {error}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::retrieval::models::ChunkDocument;

    #[test]
    fn vector_search_orders_by_similarity() {
        let store = MemoryVectorStore::new();
        store
            .upsert(vec![
                VectorRecord {
                    chunk: ChunkDocument::new("chunk-a", "doc", "alpha"),
                    embedding: vec![1.0, 0.0],
                },
                VectorRecord {
                    chunk: ChunkDocument::new("chunk-b", "doc", "beta"),
                    embedding: vec![0.0, 1.0],
                },
            ])
            .unwrap();

        let results = store
            .search(VectorSearchRequest::new(vec![1.0, 0.0], 2))
            .unwrap();

        assert_eq!(results[0].chunk_id, "chunk-a");
    }

    #[test]
    fn full_text_search_filters_metadata() {
        let store = MemoryFullTextStore::new();
        let mut keep = ChunkDocument::new("chunk-a", "doc", "rust rust search");
        keep.metadata = json!({ "layer": "hot" });
        let mut skip = ChunkDocument::new("chunk-b", "doc", "rust search");
        skip.metadata = json!({ "layer": "cold" });
        store
            .upsert(vec![
                FullTextRecord { chunk: keep },
                FullTextRecord { chunk: skip },
            ])
            .unwrap();

        let mut request = FullTextSearchRequest::new("rust", 10);
        request.filters.insert("layer".to_owned(), "hot".to_owned());
        let results = store.search(request).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "chunk-a");
    }
}
