use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::SourceSpan;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkDocument {
    pub chunk_id: String,
    pub document_id: String,
    pub text: String,
    #[serde(default)]
    pub sources: Vec<SourceSpan>,
    #[serde(default)]
    pub metadata: Value,
}

impl ChunkDocument {
    pub fn new(
        chunk_id: impl Into<String>,
        document_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            chunk_id: chunk_id.into(),
            document_id: document_id.into(),
            text: text.into(),
            sources: Vec::new(),
            metadata: Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorRecord {
    pub chunk: ChunkDocument,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullTextRecord {
    pub chunk: ChunkDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSource {
    Vector,
    FullText,
    Hybrid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub chunk_id: String,
    pub document_id: String,
    pub snippet: String,
    pub score: f32,
    pub source: RetrievalSource,
    #[serde(default)]
    pub spans: Vec<SourceSpan>,
    #[serde(default)]
    pub metadata: Value,
}

impl RetrievalResult {
    pub fn from_chunk(chunk: &ChunkDocument, score: f32, source: RetrievalSource) -> Self {
        Self {
            chunk_id: chunk.chunk_id.clone(),
            document_id: chunk.document_id.clone(),
            snippet: chunk.text.clone(),
            score,
            source,
            spans: chunk.sources.clone(),
            metadata: chunk.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorSearchRequest {
    pub query_embedding: Vec<f32>,
    pub limit: usize,
    #[serde(default)]
    pub filters: BTreeMap<String, String>,
}

impl VectorSearchRequest {
    pub fn new(query_embedding: Vec<f32>, limit: usize) -> Self {
        Self {
            query_embedding,
            limit,
            filters: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextSearchRequest {
    pub query: String,
    pub limit: usize,
    #[serde(default)]
    pub filters: BTreeMap<String, String>,
}

impl FullTextSearchRequest {
    pub fn new(query: impl Into<String>, limit: usize) -> Self {
        Self {
            query: query.into(),
            limit,
            filters: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HybridSearchRequest {
    pub query: String,
    #[serde(default)]
    pub query_embedding: Option<Vec<f32>>,
    pub limit: usize,
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    #[serde(default = "default_full_text_weight")]
    pub full_text_weight: f32,
    #[serde(default)]
    pub filters: BTreeMap<String, String>,
}

impl HybridSearchRequest {
    pub fn new(query: impl Into<String>, query_embedding: Option<Vec<f32>>, limit: usize) -> Self {
        Self {
            query: query.into(),
            query_embedding,
            limit,
            vector_weight: default_vector_weight(),
            full_text_weight: default_full_text_weight(),
            filters: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankInput {
    pub query: String,
    pub results: Vec<RetrievalResult>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreStatus {
    Healthy,
    Degraded,
    Unavailable,
    RebuildRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreHealth {
    pub component: String,
    pub status: StoreStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl StoreHealth {
    pub fn healthy(component: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: StoreStatus::Healthy,
            reason: None,
        }
    }

    pub fn degraded(component: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: StoreStatus::Degraded,
            reason: Some(reason.into()),
        }
    }

    pub fn unavailable(component: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: StoreStatus::Unavailable,
            reason: Some(reason.into()),
        }
    }

    pub fn rebuild_required(component: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: StoreStatus::RebuildRequired,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebuildStatus {
    NotNeeded,
    Required,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildReport {
    pub component: String,
    pub status: RebuildStatus,
    pub processed_items: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn default_vector_weight() -> f32 {
    0.55
}

fn default_full_text_weight() -> f32 {
    0.45
}
