use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::artifacts::{ArtifactDescriptor, DocumentPatch};
use crate::contracts::{CoreError, SourceSpan};
use crate::git::Checkpoint;

/// 文档唯一标识，当前用规范化后的绝对路径生成。
pub type DocumentId = String;

/// 文档服务支持的文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    Markdown,
    Text,
    Json,
}

impl DocumentFormat {
    /// 根据文件扩展名识别文档格式。
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        match path.extension().and_then(|value| value.to_str()) {
            Some("md" | "markdown") => Some(Self::Markdown),
            Some("txt" | "text") => Some(Self::Text),
            Some("json") => Some(Self::Json),
            _ => None,
        }
    }

    /// 返回文档格式对应的媒体类型。
    pub fn media_type(&self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Text => "text/plain; charset=utf-8",
            Self::Json => "application/json; charset=utf-8",
        }
    }
}

/// 文档元数据，不包含正文，避免运行状态重复保存大文本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub document_id: DocumentId,
    pub path: PathBuf,
    pub format: DocumentFormat,
    pub media_type: String,
    pub size_bytes: u64,
    pub version: String,
}

/// 文档读取结果，正文只在显式打开文档时返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentContent {
    pub metadata: DocumentMetadata,
    pub content: String,
}

/// 打开文档请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentReadRequest {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<DocumentFormat>,
}

/// 写入文档请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentWriteRequest {
    pub path: PathBuf,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<DocumentFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<String>,
}

/// 文档写入后的索引失效通知。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexInvalidation {
    pub document_id: DocumentId,
    pub reason: String,
    pub full_rebuild_required: bool,
}

/// 章节索引中的文档类型，用于区分正文、章节大纲和辅助材料。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterDocumentKind {
    ChapterBody,
    Outline,
    Notes,
}

/// 单个章节文档索引条目；作品页和导出都以此为准，不靠文件名猜测章节正文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterDocumentEntry {
    pub chapter_id: String,
    pub document_id: DocumentId,
    pub path: PathBuf,
    pub title: String,
    pub order: u64,
    pub kind: ChapterDocumentKind,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_ref: Option<SourceSpan>,
}

impl ChapterDocumentEntry {
    /// 校验章节索引条目必要字段，避免 UI 或 Export 使用不可定位记录。
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        validate_non_empty("chapter_id", &self.chapter_id)?;
        validate_non_empty("document_id", &self.document_id)?;
        validate_non_empty("title", &self.title)?;
        validate_non_empty("version", &self.version)?;
        if self.path.as_os_str().is_empty() {
            return Err(CoreError::validation(
                "chapter document path cannot be empty",
            ));
        }
        Ok(())
    }
}

/// 章节文档索引，保存章节正文和相关规划文档的稳定排序。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterDocumentIndex {
    pub index_version: String,
    #[serde(default)]
    pub entries: Vec<ChapterDocumentEntry>,
}

impl ChapterDocumentIndex {
    /// 创建章节索引并校验条目。
    pub fn new(
        index_version: impl Into<String>,
        entries: Vec<ChapterDocumentEntry>,
    ) -> crate::contracts::CoreResult<Self> {
        let index = Self {
            index_version: index_version.into(),
            entries,
        };
        index.validate()?;
        Ok(index)
    }

    /// 校验索引版本、条目字段和章节正文 document_id 唯一性。
    pub fn validate(&self) -> crate::contracts::CoreResult<()> {
        validate_non_empty("index_version", &self.index_version)?;
        let mut body_document_ids = std::collections::BTreeSet::new();
        for entry in &self.entries {
            entry.validate()?;
            if entry.kind == ChapterDocumentKind::ChapterBody
                && !body_document_ids.insert(entry.document_id.clone())
            {
                return Err(CoreError::validation(
                    "duplicate chapter body document_id in chapter index",
                ));
            }
        }
        Ok(())
    }

    /// 返回可在作品页默认展示和合并导出的章节正文条目，按 order 稳定排序。
    pub fn chapter_bodies(&self) -> Vec<&ChapterDocumentEntry> {
        let mut entries = self
            .entries
            .iter()
            .filter(|entry| entry.kind == ChapterDocumentKind::ChapterBody)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.chapter_id.cmp(&right.chapter_id))
                .then_with(|| left.document_id.cmp(&right.document_id))
        });
        entries
    }

    /// 根据用户选择解析本次导出的正文 document_id；未选择时导出全部正文。
    pub fn export_document_ids(
        &self,
        selected_chapter_ids: &[String],
    ) -> crate::contracts::CoreResult<Vec<DocumentId>> {
        self.validate()?;
        let bodies = self.chapter_bodies();
        if selected_chapter_ids.is_empty() {
            return Ok(bodies
                .into_iter()
                .map(|entry| entry.document_id.clone())
                .collect());
        }

        let selected = selected_chapter_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let document_ids = bodies
            .into_iter()
            .filter(|entry| selected.contains(&entry.chapter_id))
            .map(|entry| entry.document_id.clone())
            .collect::<Vec<_>>();
        if document_ids.len() != selected.len() {
            return Err(CoreError::validation(
                "selected chapter is missing chapter_body entry",
            ));
        }
        Ok(document_ids)
    }
}

/// 文档写入报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentWriteReport {
    pub metadata: DocumentMetadata,
    pub index_invalidation: IndexInvalidation,
}

/// patch 预览报告，用于普通模式下给 UI 确认。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchPreview {
    pub document_id: DocumentId,
    pub base_version: String,
    pub result_version: String,
    pub hunk_count: usize,
    pub changed: bool,
    pub diff: String,
}

/// patch 应用报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchApplyReport {
    pub preview: PatchPreview,
    pub metadata: DocumentMetadata,
    pub index_invalidation: IndexInvalidation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,
}

/// 自动或手动应用 patch 时的 checkpoint 配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchCheckpointRequest {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Artifact 写入请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactWriteRequest {
    pub artifact_id: String,
    pub kind: crate::contracts::artifacts::ArtifactKind,
    pub media_type: String,
    pub bytes: Vec<u8>,
    /// 工作流副作用的稳定 operation id；相同 id 只能提交相同 artifact 载荷。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// Artifact 写入报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactWriteReport {
    pub descriptor: ArtifactDescriptor,
}

/// 文档模块统一 trait，便于后续替换成异步或分片实现。
pub trait DocumentRepository {
    /// 读取文档元数据和正文。
    fn open_document(
        &self,
        request: DocumentReadRequest,
    ) -> crate::contracts::CoreResult<DocumentContent>;

    /// 写入完整文档内容。
    fn save_document(
        &self,
        request: DocumentWriteRequest,
    ) -> crate::contracts::CoreResult<DocumentWriteReport>;

    /// 预览 patch 结果。
    fn preview_patch(&self, patch: &DocumentPatch) -> crate::contracts::CoreResult<PatchPreview>;
}

/// 校验非空字符串字段。
fn validate_non_empty(field: &str, value: &str) -> crate::contracts::CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }
    Ok(())
}
