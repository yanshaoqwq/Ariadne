use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult};
use crate::retrieval::models::{
    ChunkDocument, RebuildReport, RebuildStatus, RetrievalResult, RetrievalSource, StoreHealth,
    VectorRecord, VectorSearchRequest,
};
use crate::retrieval::traits::VectorStore;

/// 通过 Qdrant REST API 接入的真实向量索引后端。
#[derive(Debug, Clone)]
pub struct QdrantVectorStore {
    endpoint: String,
    collection: String,
    vector_size: usize,
    client: Client,
}

impl QdrantVectorStore {
    /// 创建 Qdrant REST 后端。
    pub fn new(
        endpoint: impl Into<String>,
        collection: impl Into<String>,
        vector_size: usize,
    ) -> CoreResult<Self> {
        if vector_size == 0 {
            return Err(CoreError::validation("qdrant vector_size cannot be zero"));
        }
        let endpoint = endpoint.into().trim_end_matches('/').to_owned();
        let collection = collection.into();
        validate_non_empty("qdrant endpoint", &endpoint)?;
        validate_non_empty("qdrant collection", &collection)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(qdrant_error)?;
        Ok(Self {
            endpoint,
            collection,
            vector_size,
            client,
        })
    }

    fn collection_url(&self, suffix: &str) -> String {
        format!(
            "{}/collections/{}{}",
            self.endpoint, self.collection, suffix
        )
    }

    fn ensure_collection(&self) -> CoreResult<()> {
        // 已存在的 collection 直接复用；否则 Qdrant 会对重复创建返回 4xx，
        // 导致第一次之后的所有写入都失败。
        let existing = self
            .client
            .get(self.collection_url(""))
            .send()
            .map_err(qdrant_error)?;
        if existing.status().is_success() {
            return Ok(());
        }

        let body = json!({
            "vectors": {
                "size": self.vector_size,
                "distance": "Cosine"
            }
        });
        let response = self
            .client
            .put(self.collection_url(""))
            .json(&body)
            .send()
            .map_err(qdrant_error)?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(qdrant_error(format!(
            "create collection returned {}",
            response.status()
        )))
    }
}

impl VectorStore for QdrantVectorStore {
    /// 写入或覆盖 Qdrant points。
    fn upsert(&self, records: Vec<VectorRecord>) -> CoreResult<()> {
        self.ensure_collection()?;
        let mut points = Vec::new();
        for record in records {
            validate_vector_record(&record, self.vector_size)?;
            points.push(json!({
                "id": stable_point_id(&record.chunk.chunk_id),
                // Qdrant point id 只接受无符号整数或 UUID；FNV 哈希本身就是 u64，
                // 以整数形式提交，不能转成十六进制字符串（会被拒为非法 id）。
                "vector": record.embedding,
                "payload": {
                    "chunk_id": record.chunk.chunk_id,
                    "document_id": record.chunk.document_id,
                    "text": record.chunk.text,
                    "sources": record.chunk.sources,
                    "metadata": record.chunk.metadata,
                }
            }));
        }
        let response = self
            .client
            .put(self.collection_url("/points?wait=true"))
            .json(&json!({ "points": points }))
            .send()
            .map_err(qdrant_error)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(qdrant_error(format!(
                "upsert returned {}",
                response.status()
            )))
        }
    }

    /// 删除指定文档下的向量点。
    fn delete_document(&self, document_id: &str) -> CoreResult<usize> {
        validate_non_empty("document_id", document_id)?;
        let response = self
            .client
            .post(self.collection_url("/points/delete?wait=true"))
            .json(&json!({
                "filter": {
                    "must": [{
                        "key": "document_id",
                        "match": { "value": document_id }
                    }]
                }
            }))
            .send()
            .map_err(qdrant_error)?;
        if response.status().is_success() {
            Ok(0)
        } else {
            Err(qdrant_error(format!(
                "delete document returned {}",
                response.status()
            )))
        }
    }

    /// 通过 Qdrant search API 执行向量检索。
    fn search(&self, request: VectorSearchRequest) -> CoreResult<Vec<RetrievalResult>> {
        if request.limit == 0 {
            return Ok(Vec::new());
        }
        if request.query_embedding.len() != self.vector_size {
            return Err(CoreError::validation(format!(
                "query embedding dimension {} does not match qdrant vector_size {}",
                request.query_embedding.len(),
                self.vector_size
            )));
        }
        let response = self
            .client
            .post(self.collection_url("/points/search"))
            .json(&json!({
                "vector": request.query_embedding,
                "limit": request.limit,
                "with_payload": true,
                "filter": qdrant_filter(&request.filters),
            }))
            .send()
            .map_err(qdrant_error)?;
        if !response.status().is_success() {
            return Err(qdrant_error(format!(
                "search returned {}",
                response.status()
            )));
        }
        let value = response.json::<Value>().map_err(qdrant_error)?;
        let hits = value
            .get("result")
            .and_then(Value::as_array)
            .ok_or_else(|| qdrant_error("search response missing result array"))?;
        hits.iter().map(qdrant_hit_to_result).collect()
    }

    /// 返回 Qdrant collection 健康状态。
    fn health_check(&self) -> CoreResult<StoreHealth> {
        let response = self.client.get(self.collection_url("")).send();
        match response {
            Ok(response) if response.status().is_success() => {
                Ok(StoreHealth::healthy("qdrant_vector_store"))
            }
            Ok(response) => Ok(StoreHealth::unavailable(
                "qdrant_vector_store",
                format!("collection status {}", response.status()),
            )),
            Err(error) => Ok(StoreHealth::unavailable(
                "qdrant_vector_store",
                error.to_string(),
            )),
        }
    }

    /// Qdrant 索引可从源记录重建；这里仅保留健康标记接口。
    fn mark_rebuild_required(&self, _reason: &str) -> CoreResult<()> {
        Ok(())
    }

    /// 删除后重新写入源记录。
    fn rebuild_from_records(&self, records: Vec<VectorRecord>) -> CoreResult<RebuildReport> {
        let processed_items = records.len() as u64;
        self.ensure_collection()?;
        self.upsert(records)?;
        Ok(RebuildReport {
            component: "qdrant_vector_store".to_owned(),
            status: RebuildStatus::Completed,
            processed_items,
            error: None,
        })
    }
}

fn qdrant_filter(filters: &std::collections::BTreeMap<String, String>) -> Value {
    if filters.is_empty() {
        return Value::Null;
    }
    Value::Object(serde_json::Map::from_iter([(
        "must".to_owned(),
        Value::Array(
            filters
                .iter()
                .map(|(key, value)| {
                    json!({
                        "key": format!("metadata.{key}"),
                        "match": { "value": value }
                    })
                })
                .collect(),
        ),
    )]))
}

fn qdrant_hit_to_result(hit: &Value) -> CoreResult<RetrievalResult> {
    let payload = hit
        .get("payload")
        .ok_or_else(|| qdrant_error("qdrant hit missing payload"))?;
    let chunk = ChunkDocument {
        chunk_id: string_field(payload, "chunk_id")?,
        document_id: string_field(payload, "document_id")?,
        text: string_field(payload, "text")?,
        sources: serde_json::from_value(payload.get("sources").cloned().unwrap_or(Value::Null))
            .unwrap_or_default(),
        metadata: payload.get("metadata").cloned().unwrap_or(Value::Null),
    };
    let score = hit.get("score").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    Ok(RetrievalResult::from_chunk(
        &chunk,
        score,
        RetrievalSource::Vector,
    ))
}

fn string_field(value: &Value, key: &str) -> CoreResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| qdrant_error(format!("qdrant payload missing {key}")))
}

fn validate_vector_record(record: &VectorRecord, vector_size: usize) -> CoreResult<()> {
    validate_non_empty("chunk_id", &record.chunk.chunk_id)?;
    validate_non_empty("document_id", &record.chunk.document_id)?;
    validate_non_empty("text", &record.chunk.text)?;
    if record.embedding.len() != vector_size {
        return Err(CoreError::validation(format!(
            "vector dimension {} does not match qdrant vector_size {vector_size}",
            record.embedding.len()
        )));
    }
    if record.embedding.iter().any(|value| !value.is_finite()) {
        return Err(CoreError::validation(
            "vector embedding contains non-finite value",
        ));
    }
    Ok(())
}

/// 生成稳定的 Qdrant point id。FNV-1a 哈希本身就是 u64，直接作为无符号整数
/// point id 返回；不能格式化成十六进制字符串，否则 Qdrant 会拒绝该 id。
fn stable_point_id(chunk_id: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in chunk_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn validate_non_empty(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }
    Ok(())
}

fn qdrant_error(message: impl std::fmt::Display) -> CoreError {
    CoreError::External {
        service: "qdrant".to_owned(),
        message: message.to_string(),
    }
}
