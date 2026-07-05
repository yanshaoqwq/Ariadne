use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::ports::{SourceSpan, TextRange};
use crate::contracts::workflow::{NodeId, RunId};

/// Artifact 类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Document,
    Chunk,
    Patch,
    Diff,
    Export,
    ModelOutput,
    SkillOutput,
    SearchResult,
    CostReport,
    Other,
}

/// Artifact 描述信息，记录存储位置和来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactDescriptor {
    pub artifact_id: String,
    pub kind: ArtifactKind,
    pub media_type: String,
    pub storage_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_run: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_node: Option<NodeId>,
    #[serde(default)]
    pub sources: Vec<SourceSpan>,
    #[serde(default)]
    pub metadata: Value,
}

/// 文档 patch 的单个替换片段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchHunk {
    pub range: TextRange,
    pub replacement: String,
}

/// 针对单个文档的一组 patch。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentPatch {
    pub document_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<String>,
    #[serde(default)]
    pub hunks: Vec<PatchHunk>,
}

impl DocumentPatch {
    /// 判断 patch 是否没有任何 hunk。
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}
