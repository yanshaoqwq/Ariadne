use std::path::Path;
use std::sync::Mutex;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{
    Field, IndexRecordOption, OwnedValue, Schema, TantivyDocument, TextFieldIndexing, TextOptions,
    STORED, STRING,
};
use tantivy::tokenizer::NgramTokenizer;
use tantivy::{doc, Index, IndexReader, IndexWriter, Term};

use crate::contracts::{CoreError, CoreResult};
use crate::retrieval::memory::sort_and_limit;
use crate::retrieval::models::{
    ChunkDocument, FullTextRecord, FullTextSearchRequest, RebuildReport, RebuildStatus,
    RetrievalResult, RetrievalSource, StoreHealth,
};
use crate::retrieval::query::tantivy_literal_query;
use crate::retrieval::traits::FullTextStore;

const ARIADNE_TEXT_TOKENIZER: &str = "ariadne_cjk_ngram";

/// Tantivy 真实全文检索后端。
pub struct TantivyFullTextStore {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: TantivyFields,
    rebuild_reason: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Copy)]
struct TantivyFields {
    chunk_id: Field,
    document_id: Field,
    text: Field,
    sources_json: Field,
    metadata_json: Field,
}

impl TantivyFullTextStore {
    /// 打开或创建磁盘 Tantivy 索引。
    pub fn open(path: impl AsRef<Path>) -> CoreResult<Self> {
        std::fs::create_dir_all(path.as_ref())?;
        let (schema, fields) = build_schema();
        let directory = tantivy::directory::MmapDirectory::open(path).map_err(tantivy_error)?;
        let index = Index::open_or_create(directory, schema).map_err(tantivy_error)?;
        Self::from_index(index, fields)
    }

    /// 打开内存索引，主要用于契约测试。
    pub fn open_in_memory() -> CoreResult<Self> {
        let (schema, fields) = build_schema();
        let index = Index::create_in_ram(schema);
        Self::from_index(index, fields)
    }

    fn from_index(index: Index, fields: TantivyFields) -> CoreResult<Self> {
        index.tokenizers().register(
            ARIADNE_TEXT_TOKENIZER,
            NgramTokenizer::all_ngrams(1, 3).map_err(tantivy_error)?,
        );
        let writer = index.writer(50_000_000).map_err(tantivy_error)?;
        let reader = index.reader().map_err(tantivy_error)?;
        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            fields,
            rebuild_reason: Mutex::new(None),
        })
    }
}

impl FullTextStore for TantivyFullTextStore {
    /// 写入或覆盖全文记录。
    fn upsert(&self, records: Vec<FullTextRecord>) -> CoreResult<()> {
        let mut writer = self.writer.lock().map_err(lock_error)?;
        for record in records {
            validate_record(&record)?;
            let term = Term::from_field_text(self.fields.chunk_id, &record.chunk.chunk_id);
            writer.delete_term(term);
            writer
                .add_document(record_to_doc(&record, self.fields)?)
                .map_err(tantivy_error)?;
        }
        writer.commit().map_err(tantivy_error)?;
        self.reader.reload().map_err(tantivy_error)?;
        *self.rebuild_reason.lock().map_err(lock_error)? = None;
        Ok(())
    }

    /// 删除指定文档下的所有全文记录。
    fn delete_document(&self, document_id: &str) -> CoreResult<usize> {
        validate_non_empty("document_id", document_id)?;
        let mut writer = self.writer.lock().map_err(lock_error)?;
        writer.delete_term(Term::from_field_text(self.fields.document_id, document_id));
        writer.commit().map_err(tantivy_error)?;
        self.reader.reload().map_err(tantivy_error)?;
        Ok(0)
    }

    /// 使用 Tantivy QueryParser 检索文本，并在内存中应用 metadata filter。
    fn search(&self, request: FullTextSearchRequest) -> CoreResult<Vec<RetrievalResult>> {
        if request.limit == 0 || request.query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let searcher = self.reader.searcher();
        let mut parser = QueryParser::for_index(&self.index, vec![self.fields.text]);
        parser.set_conjunction_by_default();
        let literal_query = tantivy_literal_query(&request.query)?;
        let query = parser
            .parse_query(&literal_query)
            .map_err(|error| CoreError::validation(format!("tantivy query error: {error}")))?;
        let top_docs = searcher
            .search(
                &query,
                &TopDocs::with_limit(request.limit.saturating_mul(3).max(request.limit)),
            )
            .map_err(tantivy_error)?;
        let mut results = Vec::new();
        for (score, address) in top_docs {
            let doc = searcher
                .doc::<TantivyDocument>(address)
                .map_err(tantivy_error)?;
            let chunk = doc_to_chunk(&doc, self.fields)?;
            if !metadata_matches(&chunk.metadata, &request.filters) {
                continue;
            }
            results.push(RetrievalResult::from_chunk(
                &chunk,
                score,
                RetrievalSource::FullText,
            ));
        }
        sort_and_limit(&mut results, request.limit);
        Ok(results)
    }

    /// 返回 Tantivy 索引健康状态。
    fn health_check(&self) -> CoreResult<StoreHealth> {
        if let Some(reason) = self.rebuild_reason.lock().map_err(lock_error)?.clone() {
            return Ok(StoreHealth::rebuild_required(
                "tantivy_full_text_store",
                reason,
            ));
        }
        Ok(StoreHealth::healthy("tantivy_full_text_store"))
    }

    /// 标记需要重建。
    fn mark_rebuild_required(&self, reason: &str) -> CoreResult<()> {
        *self.rebuild_reason.lock().map_err(lock_error)? = Some(reason.to_owned());
        Ok(())
    }

    /// 使用源记录重建索引。
    fn rebuild_from_records(&self, records: Vec<FullTextRecord>) -> CoreResult<RebuildReport> {
        let processed_items = records.len() as u64;
        let mut writer = self.writer.lock().map_err(lock_error)?;
        writer.delete_all_documents().map_err(tantivy_error)?;
        for record in records {
            validate_record(&record)?;
            writer
                .add_document(record_to_doc(&record, self.fields)?)
                .map_err(tantivy_error)?;
        }
        writer.commit().map_err(tantivy_error)?;
        self.reader.reload().map_err(tantivy_error)?;
        *self.rebuild_reason.lock().map_err(lock_error)? = None;
        Ok(RebuildReport {
            component: "tantivy_full_text_store".to_owned(),
            status: RebuildStatus::Completed,
            processed_items,
            error: None,
        })
    }
}

fn build_schema() -> (Schema, TantivyFields) {
    let mut builder = Schema::builder();
    let chunk_id = builder.add_text_field("chunk_id", STORED | STRING);
    let document_id = builder.add_text_field("document_id", STORED | STRING);
    let text_options = TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(ARIADNE_TEXT_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let text = builder.add_text_field("text", text_options);
    let sources_json = builder.add_text_field("sources_json", STORED);
    let metadata_json = builder.add_text_field("metadata_json", STORED);
    (
        builder.build(),
        TantivyFields {
            chunk_id,
            document_id,
            text,
            sources_json,
            metadata_json,
        },
    )
}

fn record_to_doc(record: &FullTextRecord, fields: TantivyFields) -> CoreResult<TantivyDocument> {
    Ok(doc!(
        fields.chunk_id => record.chunk.chunk_id.clone(),
        fields.document_id => record.chunk.document_id.clone(),
        fields.text => record.chunk.text.clone(),
        fields.sources_json => serde_json::to_string(&record.chunk.sources)?,
        fields.metadata_json => serde_json::to_string(&record.chunk.metadata)?,
    ))
}

fn doc_to_chunk(doc: &TantivyDocument, fields: TantivyFields) -> CoreResult<ChunkDocument> {
    Ok(ChunkDocument {
        chunk_id: text_value(doc, fields.chunk_id, "chunk_id")?,
        document_id: text_value(doc, fields.document_id, "document_id")?,
        text: text_value(doc, fields.text, "text")?,
        sources: serde_json::from_str(&text_value(doc, fields.sources_json, "sources_json")?)
            .unwrap_or_default(),
        metadata: serde_json::from_str(&text_value(doc, fields.metadata_json, "metadata_json")?)
            .unwrap_or(serde_json::Value::Null),
    })
}

fn text_value(doc: &TantivyDocument, field: Field, name: &str) -> CoreResult<String> {
    doc.get_first(field)
        .and_then(|value| match value {
            OwnedValue::Str(text) => Some(text.as_str()),
            _ => None,
        })
        .map(ToOwned::to_owned)
        .ok_or_else(|| CoreError::validation(format!("tantivy document missing {name}")))
}

fn validate_record(record: &FullTextRecord) -> CoreResult<()> {
    validate_non_empty("chunk_id", &record.chunk.chunk_id)?;
    validate_non_empty("document_id", &record.chunk.document_id)?;
    validate_non_empty("text", &record.chunk.text)
}

fn metadata_matches(
    metadata: &serde_json::Value,
    filters: &std::collections::BTreeMap<String, String>,
) -> bool {
    filters.iter().all(|(key, expected)| {
        metadata
            .get(key)
            .and_then(|value| value.as_str())
            .is_some_and(|actual| actual == expected)
    })
}

fn validate_non_empty(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }
    Ok(())
}

fn tantivy_error(message: impl std::fmt::Display) -> CoreError {
    CoreError::External {
        service: "tantivy".to_owned(),
        message: message.to_string(),
    }
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation(format!("tantivy store lock poisoned: {error}"))
}
