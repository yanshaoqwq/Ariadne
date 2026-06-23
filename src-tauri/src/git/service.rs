use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use crate::core::{CoreError, CoreResult};
use crate::git::models::{
    ArchivePoint, BranchGraphNode, Checkpoint, GitCommitSummary, GitHealthReport, GitHealthStatus,
    RestoreReport,
};

/// Git 服务，所有 Git 写操作通过同一把锁串行化。
#[derive(Debug)]
pub struct GitService {
    repo_root: PathBuf,
    lock: Mutex<()>,
}

impl GitService {
    /// 创建 Git 服务。
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            lock: Mutex::new(()),
        }
    }

    /// 返回仓库根目录。
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// 初始化 Git 仓库；已存在仓库时保持幂等。
    pub fn init_repository(&self) -> CoreResult<()> {
        let _guard = self.git_guard()?;
        self.run_git(["init"])?;
        Ok(())
    }

    /// 执行 Git 健康检查。
    pub fn health_check(&self) -> GitHealthReport {
        let inside = self.run_git(["rev-parse", "--is-inside-work-tree"]);
        if inside.is_err() {
            return GitHealthReport {
                status: GitHealthStatus::NotRepository,
                branch: None,
                head: None,
                dirty: false,
                reason: Some("not a git repository".to_owned()),
            };
        }

        let head = self.run_git(["rev-parse", "--verify", "HEAD"]).ok();
        let branch = self
            .run_git(["branch", "--show-current"])
            .ok()
            .filter(|value| !value.trim().is_empty());
        let dirty = self
            .run_git(["status", "--porcelain"])
            .map(|output| !output.trim().is_empty())
            .unwrap_or(true);

        let status = if head.is_some() {
            GitHealthStatus::Healthy
        } else {
            GitHealthStatus::Degraded
        };

        GitHealthReport {
            status,
            branch,
            head,
            dirty,
            reason: (status == GitHealthStatus::Degraded)
                .then_some("repository has no commits yet".to_owned()),
        }
    }

    /// 创建用户命名存档点 commit。
    pub fn create_archive_point(
        &self,
        name: &str,
        message: Option<&str>,
    ) -> CoreResult<ArchivePoint> {
        validate_non_empty("archive point name", name)?;
        let _guard = self.git_guard()?;
        self.stage_all()?;
        let commit_message = message
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Archive: {name}"));
        let commit_id = self.commit_allow_empty(&commit_message)?;
        Ok(ArchivePoint {
            name: name.to_owned(),
            commit_id,
            message: commit_message,
        })
    }

    /// 创建节点级 checkpoint commit。
    pub fn create_checkpoint(
        &self,
        node_id: &str,
        message: Option<&str>,
    ) -> CoreResult<Checkpoint> {
        validate_non_empty("node_id", node_id)?;
        let _guard = self.git_guard()?;
        self.stage_all()?;
        let commit_message = message
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Checkpoint: node {node_id}"));
        let commit_id = self.commit_allow_empty(&commit_message)?;
        Ok(Checkpoint {
            checkpoint_id: commit_id.clone(),
            node_id: node_id.to_owned(),
            commit_id,
            message: commit_message,
        })
    }

    /// 返回工作区 diff。
    pub fn diff(&self) -> CoreResult<String> {
        self.run_git(["diff", "--"])
    }

    /// 返回最近 commit 摘要。
    pub fn recent_commits(&self, limit: usize) -> CoreResult<Vec<GitCommitSummary>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let output = self.run_git(["log", "--format=%H%x1f%s", &format!("-n{limit}")])?;
        Ok(output
            .lines()
            .filter_map(|line| {
                let (commit_id, summary) = line.split_once('\x1f')?;
                Some(GitCommitSummary {
                    commit_id: commit_id.to_owned(),
                    summary: summary.to_owned(),
                })
            })
            .collect())
    }

    /// 读取简化分支图。
    pub fn branch_graph(&self, limit: usize) -> CoreResult<Vec<BranchGraphNode>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let output = self.run_git([
            "log",
            "--all",
            "--decorate=short",
            "--format=%H%x1f%P%x1f%D%x1f%s",
            &format!("-n{limit}"),
        ])?;
        Ok(output.lines().filter_map(parse_branch_graph_node).collect())
    }

    /// 回档到指定 commit，但必须创建新分支保护当前工作。
    pub fn restore_to_new_branch(
        &self,
        commit_id: &str,
        new_branch: &str,
    ) -> CoreResult<RestoreReport> {
        validate_non_empty("commit_id", commit_id)?;
        validate_branch_name(new_branch)?;
        let _guard = self.git_guard()?;
        self.ensure_clean_worktree()?;
        self.run_git(["rev-parse", "--verify", commit_id])?;
        self.run_git(["checkout", "-b", new_branch, commit_id])?;
        Ok(RestoreReport {
            new_branch: new_branch.to_owned(),
            base_commit: commit_id.to_owned(),
            index_rebuild_required: true,
            runtime_rebind_required: true,
        })
    }

    /// 检测 Git 仓库损坏时创建备份目录名，实际复制由上层确认后执行。
    pub fn backup_dir_name(&self) -> String {
        "git-backup-before-reinit".to_owned()
    }

    /// 重新初始化仓库，保留工作区文件。
    pub fn reinitialize_repository(&self) -> CoreResult<()> {
        let _guard = self.git_guard()?;
        self.run_git(["init"])?;
        Ok(())
    }

    /// 获取 Git 操作互斥锁。
    fn git_guard(&self) -> CoreResult<MutexGuard<'_, ()>> {
        self.lock
            .lock()
            .map_err(|_| CoreError::validation("git service lock poisoned"))
    }

    /// 暂存所有当前变更。
    fn stage_all(&self) -> CoreResult<()> {
        self.run_git(["add", "--all"])?;
        Ok(())
    }

    /// 创建 commit；即使没有文件变更，也允许创建 checkpoint。
    fn commit_allow_empty(&self, message: &str) -> CoreResult<String> {
        self.run_git(["commit", "--allow-empty", "-m", message])?;
        self.run_git(["rev-parse", "HEAD"])
    }

    /// 回档前要求工作区干净，避免覆盖用户未保存改动。
    fn ensure_clean_worktree(&self) -> CoreResult<()> {
        let status = self.run_git(["status", "--porcelain"])?;
        if status.trim().is_empty() {
            Ok(())
        } else {
            Err(CoreError::validation(
                "worktree must be clean before restore_to_new_branch",
            ))
        }
    }

    /// 执行 Git 命令并返回 stdout。
    fn run_git<I, S>(&self, args: I) -> CoreResult<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_root)
            .output()?;

        if output.status.success() {
            return String::from_utf8(output.stdout)
                .map(|value| value.trim_end().to_owned())
                .map_err(|error| {
                    CoreError::validation(format!("git output is not utf-8: {error}"))
                });
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(CoreError::External {
            service: "git".to_owned(),
            message: if stderr.is_empty() {
                format!("git exited with status {}", output.status)
            } else {
                stderr
            },
        })
    }
}

/// 校验非空字段。
fn validate_non_empty(field: &str, value: &str) -> CoreResult<()> {
    if value.trim().is_empty() {
        return Err(CoreError::validation(format!("{field} cannot be empty")));
    }

    Ok(())
}

/// 只允许简单安全的分支名，避免把用户输入直接变成危险 refspec。
fn validate_branch_name(branch: &str) -> CoreResult<()> {
    validate_non_empty("branch name", branch)?;
    if branch.starts_with('-')
        || branch.contains("..")
        || branch.contains(' ')
        || branch.contains('~')
        || branch.contains('^')
        || branch.contains(':')
        || branch.contains('\\')
        || branch.ends_with('/')
        || branch.ends_with(".lock")
    {
        return Err(CoreError::validation("invalid branch name"));
    }

    Ok(())
}

/// 解析 `git log` 输出为分支图节点。
fn parse_branch_graph_node(line: &str) -> Option<BranchGraphNode> {
    let mut parts = line.split('\x1f');
    let commit_id = parts.next()?.to_owned();
    let parents = parts
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let refs = parts
        .next()
        .unwrap_or_default()
        .split(", ")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    let summary = parts.next().unwrap_or_default().to_owned();

    Some(BranchGraphNode {
        commit_id,
        parents,
        refs,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_name_rejects_unsafe_values() {
        assert!(validate_branch_name("restore/checkpoint-1").is_ok());
        assert!(validate_branch_name("../main").is_err());
        assert!(validate_branch_name("-bad").is_err());
    }

    #[test]
    fn branch_graph_parser_handles_refs_and_parents() {
        let node =
            parse_branch_graph_node("abc\x1fparent1 parent2\x1fHEAD -> main, tag: v1\x1fmsg")
                .unwrap();

        assert_eq!(node.commit_id, "abc");
        assert_eq!(node.parents, vec!["parent1", "parent2"]);
        assert_eq!(node.refs, vec!["HEAD -> main", "tag: v1"]);
        assert_eq!(node.summary, "msg");
    }
}
