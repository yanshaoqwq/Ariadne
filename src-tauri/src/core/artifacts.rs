use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::ports::{SourceSpan, TextRange};
use crate::core::workflow::{NodeId, RunId};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchHunk {
    pub range: TextRange,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentPatch {
    pub document_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<String>,
    #[serde(default)]
    pub hunks: Vec<PatchHunk>,
}

impl DocumentPatch {
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}
