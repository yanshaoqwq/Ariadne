use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::artifacts::{ArtifactDescriptor, DocumentPatch};
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
}

/// 文档写入后的索引失效通知。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexInvalidation {
    pub document_id: DocumentId,
    pub reason: String,
    pub full_rebuild_required: bool,
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
    pub kind: crate::core::artifacts::ArtifactKind,
    pub media_type: String,
    pub bytes: Vec<u8>,
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
    ) -> crate::core::CoreResult<DocumentContent>;

    /// 写入完整文档内容。
    fn save_document(
        &self,
        request: DocumentWriteRequest,
    ) -> crate::core::CoreResult<DocumentWriteReport>;

    /// 预览 patch 结果。
    fn preview_patch(&self, patch: &DocumentPatch) -> crate::core::CoreResult<PatchPreview>;
}
