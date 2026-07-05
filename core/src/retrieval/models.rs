use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::SourceSpan;

/// 可同时进入向量索引和全文索引的文本块。
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
    /// 创建没有来源片段和 metadata 的文本块。
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

/// 向量索引记录，包含 chunk 和对应 embedding。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorRecord {
    pub chunk: ChunkDocument,
    pub embedding: Vec<f32>,
}

/// 全文索引记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullTextRecord {
    pub chunk: ChunkDocument,
}

/// 检索结果来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSource {
    Vector,
    FullText,
    Hybrid,
}

/// 标准化检索结果，供 RAG、SearchNode 和 UI 使用。
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
    /// 从 chunk 构造检索结果，并保留来源引用。
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

/// 向量检索请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorSearchRequest {
    pub query_embedding: Vec<f32>,
    pub limit: usize,
    #[serde(default)]
    pub filters: BTreeMap<String, String>,
}

impl VectorSearchRequest {
    /// 创建不带 metadata filter 的向量检索请求。
    pub fn new(query_embedding: Vec<f32>, limit: usize) -> Self {
        Self {
            query_embedding,
            limit,
            filters: BTreeMap::new(),
        }
    }
}

/// 全文检索请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextSearchRequest {
    pub query: String,
    pub limit: usize,
    #[serde(default)]
    pub filters: BTreeMap<String, String>,
}

impl FullTextSearchRequest {
    /// 创建不带 metadata filter 的全文检索请求。
    pub fn new(query: impl Into<String>, limit: usize) -> Self {
        Self {
            query: query.into(),
            limit,
            filters: BTreeMap::new(),
        }
    }
}

/// 混合检索请求，可同时带文本查询和向量查询。
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
    /// 创建使用默认权重的混合检索请求。
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

/// 传给 reranker 的候选结果集合。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankInput {
    pub query: String,
    pub results: Vec<RetrievalResult>,
    pub limit: usize,
}

/// 可重建检索组件的健康状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreStatus {
    Healthy,
    Degraded,
    Unavailable,
    RebuildRequired,
}

/// 单个检索组件的健康报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreHealth {
    pub component: String,
    pub status: StoreStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl StoreHealth {
    /// 构造健康状态。
    pub fn healthy(component: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: StoreStatus::Healthy,
            reason: None,
        }
    }

    /// 构造降级状态。
    pub fn degraded(component: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: StoreStatus::Degraded,
            reason: Some(reason.into()),
        }
    }

    /// 构造不可用状态。
    pub fn unavailable(component: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: StoreStatus::Unavailable,
            reason: Some(reason.into()),
        }
    }

    /// 构造需要重建状态。
    pub fn rebuild_required(component: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: StoreStatus::RebuildRequired,
            reason: Some(reason.into()),
        }
    }
}

/// 索引重建生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebuildStatus {
    NotNeeded,
    Required,
    Running,
    Completed,
    Failed,
}

/// 重建索引后的报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildReport {
    pub component: String,
    pub status: RebuildStatus,
    pub processed_items: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 默认向量检索权重。
fn default_vector_weight() -> f32 {
    0.55
}

/// 默认全文检索权重。
fn default_full_text_weight() -> f32 {
    0.45
}
