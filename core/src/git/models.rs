use serde::{Deserialize, Serialize};

/// Git 仓库健康状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHealthStatus {
    Healthy,
    NotRepository,
    Degraded,
    Unavailable,
}

/// Git 仓库健康检查报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHealthReport {
    pub status: GitHealthStatus,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub dirty: bool,
    pub reason: Option<String>,
}

/// Git commit 摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommitSummary {
    pub commit_id: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_kind: Option<CheckpointKind>,
}

/// 用户命名存档点创建结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivePoint {
    pub name: String,
    pub commit_id: String,
    pub message: String,
    pub checkpoint_kind: CheckpointKind,
}

/// 节点级 checkpoint 创建结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub checkpoint_id: String,
    pub node_id: String,
    pub commit_id: String,
    pub message: String,
    pub checkpoint_kind: CheckpointKind,
}

/// Git 存档点类型，供前端区分自动节点存档和用户手动存档。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    Auto,
    Manual,
}

/// 回档到新分支后的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreReport {
    pub new_branch: String,
    pub base_commit: String,
    pub index_rebuild_required: bool,
    pub runtime_rebind_required: bool,
}

/// 分支图中的单个节点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchGraphNode {
    pub commit_id: String,
    pub parents: Vec<String>,
    pub refs: Vec<String>,
    pub summary: String,
}
