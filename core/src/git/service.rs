use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::contracts::{CoreError, CoreResult, ExecutionCancellation, ExternalDispatchOutcome};
use crate::git::models::{
    ArchivePoint, BranchGraphNode, Checkpoint, CheckpointKind, GitCommitSummary, GitHealthReport,
    GitHealthStatus, RestoreReport,
};

const DEFAULT_GIT_USER_NAME: &str = "Ariadne";
const DEFAULT_GIT_USER_EMAIL: &str = "ariadne@local.invalid";
const DEFAULT_GIT_TIMEOUT: Duration = Duration::from_secs(120);
const GIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_GIT_STDOUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 256 * 1024;

/// 有界 Git diff 预览；完整输出只流式计数，不在内存中整体物化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDiffPreview {
    pub line_count: usize,
    pub preview: String,
}

#[derive(Debug)]
struct GitCommandOutput {
    status: ExitStatus,
    stdout: String,
    stdout_bytes: u64,
    stderr: String,
}

/// Git 服务，所有 Git 写操作通过同一把锁串行化。
#[derive(Debug)]
pub struct GitService {
    repo_root: PathBuf,
    lock: Mutex<()>,
    cancellation: ExecutionCancellation,
    timeout: Duration,
}

/// Git 暂存策略。所有路径都按仓库根目录解析，排除项使用 literal pathspec。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStagePolicy {
    pub ignored_paths: Vec<String>,
}

impl Default for GitStagePolicy {
    fn default() -> Self {
        Self {
            ignored_paths: default_ignored_paths(),
        }
    }
}

impl GitService {
    /// 创建 Git 服务。
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            lock: Mutex::new(()),
            cancellation: ExecutionCancellation::new(),
            timeout: DEFAULT_GIT_TIMEOUT,
        }
    }

    /// 为本次命令链绑定统一取消令牌与墙钟上限。
    pub fn with_execution_policy(
        mut self,
        cancellation: ExecutionCancellation,
        timeout: Duration,
    ) -> Self {
        self.cancellation = cancellation;
        self.timeout = timeout.max(GIT_POLL_INTERVAL);
        self
    }

    /// 返回仓库根目录。
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// 初始化 Git 仓库；已存在仓库时保持幂等。
    pub fn init_repository(&self) -> CoreResult<()> {
        let _guard = self.git_guard()?;
        self.run_git(["init"])?;
        self.ensure_local_commit_identity()?;
        Ok(())
    }

    /// 执行 Git 健康检查。Strict: errors surface as `Unavailable` with reason, not as NotRepository.
    pub fn health_check(&self) -> CoreResult<GitHealthReport> {
        self.health_check_with_policy(&GitStagePolicy::default())
            .map(|(health, _)| health)
    }

    /// 一次读取 porcelain 状态并同时生成健康报告，供状态页避免重复执行 git status。
    pub fn health_check_with_policy(
        &self,
        policy: &GitStagePolicy,
    ) -> CoreResult<(GitHealthReport, String)> {
        let repository_probe = self.run_git_output(["rev-parse", "--is-inside-work-tree"])?;
        if !repository_probe.status.success() {
            // 没有本地 Git 元数据时才是普通“未初始化”；已有 .git 却探测失败
            // 表示损坏、权限或其它真实故障，必须保留严格错误而不是伪装成未初始化。
            if !self.local_git_metadata_exists()? {
                return Ok((
                    GitHealthReport {
                        status: GitHealthStatus::NotRepository,
                        branch: None,
                        head: None,
                        dirty: false,
                        reason: Some("not a git repository".to_owned()),
                    },
                    String::new(),
                ));
            }
            return Err(git_command_error(&repository_probe));
        }
        ensure_git_stdout_within_limit(&repository_probe)?;

        let head = self.optional_git_value(["rev-parse", "--verify", "--quiet", "HEAD"])?;
        let branch = self
            .optional_git_value(["branch", "--show-current"])?
            .filter(|value| !value.trim().is_empty());
        let porcelain = self.status_with_policy(policy)?;
        let dirty = !porcelain.trim().is_empty();

        let status = if head.is_some() {
            GitHealthStatus::Healthy
        } else {
            GitHealthStatus::Degraded
        };

        Ok((
            GitHealthReport {
                status,
                branch,
                head,
                dirty,
                reason: (status == GitHealthStatus::Degraded)
                    .then_some("repository has no commits yet".to_owned()),
            },
            porcelain,
        ))
    }

    /// 创建用户命名存档点 commit。
    pub fn create_archive_point(
        &self,
        name: &str,
        message: Option<&str>,
    ) -> CoreResult<ArchivePoint> {
        self.create_archive_point_with_policy(name, message, &GitStagePolicy::default())
    }

    /// 使用指定暂存策略创建用户命名存档点 commit。
    pub fn create_archive_point_with_policy(
        &self,
        name: &str,
        message: Option<&str>,
        policy: &GitStagePolicy,
    ) -> CoreResult<ArchivePoint> {
        validate_non_empty("archive point name", name)?;
        let _guard = self.git_guard()?;
        self.stage_all(policy)?;
        let commit_message = message
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Archive: {name}"));
        let commit_id = self.commit_allow_empty(&commit_message)?;
        Ok(ArchivePoint {
            name: name.to_owned(),
            commit_id,
            message: commit_message,
            checkpoint_kind: CheckpointKind::Manual,
        })
    }

    /// 创建节点级 checkpoint commit。
    pub fn create_checkpoint(
        &self,
        node_id: &str,
        message: Option<&str>,
    ) -> CoreResult<Checkpoint> {
        self.create_checkpoint_with_policy(node_id, message, &GitStagePolicy::default())
    }

    /// 使用指定暂存策略创建节点级 checkpoint commit。
    pub fn create_checkpoint_with_policy(
        &self,
        node_id: &str,
        message: Option<&str>,
        policy: &GitStagePolicy,
    ) -> CoreResult<Checkpoint> {
        validate_non_empty("node_id", node_id)?;
        let _guard = self.git_guard()?;
        self.stage_all(policy)?;
        let commit_message = message
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Checkpoint: node {node_id}"));
        let commit_id = self.commit_allow_empty(&commit_message)?;
        Ok(Checkpoint {
            checkpoint_id: commit_id.clone(),
            node_id: node_id.to_owned(),
            commit_id,
            message: commit_message,
            checkpoint_kind: CheckpointKind::Auto,
        })
    }

    /// 返回工作区 diff。
    pub fn diff(&self) -> CoreResult<String> {
        self.run_git(["diff", "--"])
    }

    /// 按暂存策略返回工作区 diff，和存档/状态展示排除规则保持一致。
    pub fn diff_with_policy(&self, policy: &GitStagePolicy) -> CoreResult<String> {
        let mut args = vec!["diff".to_owned(), "--".to_owned(), ".".to_owned()];
        args.extend(policy.exclude_pathspecs()?);
        self.run_git(args)
    }

    /// 流式统计完整 diff 行数，但只保留指定字符数的预览，避免大型 diff 整体驻留内存。
    /// Concurrently drains stderr (C8) so a full stderr pipe cannot deadlock stdout read.
    pub fn diff_preview_with_policy(
        &self,
        policy: &GitStagePolicy,
        preview_char_limit: usize,
    ) -> CoreResult<GitDiffPreview> {
        let mut args = vec!["diff".to_owned(), "--".to_owned(), ".".to_owned()];
        args.extend(policy.exclude_pathspecs()?);
        let mut child = self.spawn_git(args)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CoreError::validation("git diff stdout pipe is unavailable"))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| CoreError::validation("git diff stderr pipe is unavailable"))?;
        let stdout_handle =
            std::thread::spawn(move || read_diff_preview(stdout, preview_char_limit));
        let stderr_handle = std::thread::spawn(move || {
            drain_bounded(stderr_pipe, MAX_GIT_STDERR_BYTES).map_err(CoreError::from)
        });
        let (status, preview_result, (stderr, _)) = self.finish_git_child(
            &mut child,
            "diff",
            stdout_handle,
            stderr_handle,
            "git diff stdout",
            "git diff stderr",
        )?;
        if !status.success() {
            return Err(git_command_error_from_parts(status, &stderr));
        }
        Ok(preview_result)
    }

    /// 按暂存策略返回 porcelain 状态。
    pub fn status_with_policy(&self, policy: &GitStagePolicy) -> CoreResult<String> {
        let mut args = vec![
            "status".to_owned(),
            "--porcelain".to_owned(),
            "--untracked-files=all".to_owned(),
            "--".to_owned(),
            ".".to_owned(),
        ];
        args.extend(policy.exclude_pathspecs()?);
        self.run_git(args)
    }

    /// 返回最近 commit 摘要。
    pub fn recent_commits(&self, limit: usize) -> CoreResult<Vec<GitCommitSummary>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        if !self.has_head_commit()? {
            return Ok(Vec::new());
        }

        let output = self.run_git([
            "log",
            "--format=%H%x1f%ct%x1f%an%x1f%s",
            &format!("-n{limit}"),
        ])?;
        Ok(output
            .lines()
            .filter_map(parse_git_commit_summary)
            .collect())
    }

    /// 读取简化分支图。
    pub fn branch_graph(&self, limit: usize) -> CoreResult<Vec<BranchGraphNode>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        if !self.has_head_commit()? {
            return Ok(Vec::new());
        }

        let output = self.run_git([
            "log",
            "--all",
            "--decorate=short",
            "--format=%H%x1f%P%x1f%D%x1f%ct%x1f%an%x1f%s",
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
        self.restore_to_new_branch_with_policy(commit_id, new_branch, &GitStagePolicy::default())
    }

    /// 使用项目 Git 排除策略回档，运行时数据库和索引等内部文件不应阻止安全回档。
    pub fn restore_to_new_branch_with_policy(
        &self,
        commit_id: &str,
        new_branch: &str,
        policy: &GitStagePolicy,
    ) -> CoreResult<RestoreReport> {
        validate_non_empty("commit_id", commit_id)?;
        validate_branch_name(new_branch)?;
        let _guard = self.git_guard()?;
        self.ensure_clean_worktree(policy)?;
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
        self.ensure_local_commit_identity()?;
        Ok(())
    }

    /// 获取 Git 操作互斥锁。
    fn git_guard(&self) -> CoreResult<MutexGuard<'_, ()>> {
        self.lock
            .lock()
            .map_err(|_| CoreError::validation("git service lock poisoned"))
    }

    /// 暂存所有当前变更。
    fn stage_all(&self, policy: &GitStagePolicy) -> CoreResult<()> {
        let mut args = vec![
            "add".to_owned(),
            "--all".to_owned(),
            "--".to_owned(),
            ".".to_owned(),
        ];
        args.extend(policy.exclude_pathspecs()?);
        self.run_git(args)?;
        Ok(())
    }

    /// 创建 commit；即使没有文件变更，也允许创建 checkpoint。
    fn commit_allow_empty(&self, message: &str) -> CoreResult<String> {
        self.ensure_local_commit_identity()?;
        self.run_git(["commit", "--allow-empty", "-m", message])?;
        self.run_git(["rev-parse", "HEAD"])
    }

    /// Ariadne 管理项目内存档提交；仓库本地缺身份时写入默认身份，避免依赖用户全局 Git 配置。
    fn ensure_local_commit_identity(&self) -> CoreResult<()> {
        if !self.has_local_config("user.name")? {
            self.run_git(["config", "--local", "user.name", DEFAULT_GIT_USER_NAME])?;
        }
        if !self.has_local_config("user.email")? {
            self.run_git(["config", "--local", "user.email", DEFAULT_GIT_USER_EMAIL])?;
        }
        Ok(())
    }

    fn has_local_config(&self, key: &str) -> CoreResult<bool> {
        self.optional_git_value(["config", "--local", "--get", key])
            .map(|value| value.is_some_and(|value| !value.trim().is_empty()))
    }

    fn has_head_commit(&self) -> CoreResult<bool> {
        self.optional_git_value(["rev-parse", "--verify", "--quiet", "HEAD"])
            .map(|value| value.is_some())
    }

    fn local_git_metadata_exists(&self) -> CoreResult<bool> {
        self.repo_root
            .join(".git")
            .try_exists()
            .map_err(CoreError::from)
    }

    /// 回档前要求工作区干净，避免覆盖用户未保存改动。
    fn ensure_clean_worktree(&self, policy: &GitStagePolicy) -> CoreResult<()> {
        let status = self.status_with_policy(policy)?;
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
        let output = self.run_git_output(args)?;
        ensure_git_stdout_within_limit(&output)?;
        if output.status.success() {
            return Ok(output.stdout.trim_end().to_owned());
        }

        Err(git_command_error(&output))
    }

    fn run_git_output<I, S>(&self, args: I) -> CoreResult<GitCommandOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .collect::<Vec<_>>();
        let operation = args
            .first()
            .map(|arg| arg.to_string_lossy().into_owned())
            .unwrap_or_else(|| "command".to_owned());
        let mut child = self.spawn_git(args)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CoreError::validation("git stdout pipe is unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CoreError::validation("git stderr pipe is unavailable"))?;
        let stdout_handle = std::thread::spawn(move || {
            drain_bounded(stdout, MAX_GIT_STDOUT_BYTES).map_err(CoreError::from)
        });
        let stderr_handle = std::thread::spawn(move || {
            drain_bounded(stderr, MAX_GIT_STDERR_BYTES).map_err(CoreError::from)
        });
        let (status, (stdout, stdout_bytes), (stderr, _)) = self.finish_git_child(
            &mut child,
            &operation,
            stdout_handle,
            stderr_handle,
            "git stdout",
            "git stderr",
        )?;
        Ok(GitCommandOutput {
            status,
            stdout,
            stdout_bytes,
            stderr,
        })
    }

    fn spawn_git(&self, args: Vec<impl Into<OsString>>) -> CoreResult<Child> {
        self.cancellation.check()?;
        let mut command = Command::new("git");
        command
            .args(args.into_iter().map(Into::into))
            .current_dir(&self.repo_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        Ok(command.spawn()?)
    }

    fn optional_git_value<I, S>(&self, args: I) -> CoreResult<Option<String>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = self.run_git_output(args)?;
        ensure_git_stdout_within_limit(&output)?;
        if output.status.success() {
            return Ok(Some(output.stdout.trim_end().to_owned()));
        }
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        Err(git_command_error(&output))
    }

    fn finish_git_child<T>(
        &self,
        child: &mut Child,
        operation: &str,
        stdout_handle: std::thread::JoinHandle<CoreResult<T>>,
        stderr_handle: std::thread::JoinHandle<CoreResult<(String, u64)>>,
        stdout_stream: &str,
        stderr_stream: &str,
    ) -> CoreResult<(ExitStatus, T, (String, u64))> {
        let wait_result = self.wait_for_git_child(child, operation);
        // 即使取消、超时或 wait 失败，也必须回收两条 pipe reader，避免快速取消
        // 把已失去所有权的后台读取线程留在进程内。
        let stdout_result = join_git_reader(stdout_handle, stdout_stream);
        let stderr_result = join_git_reader(stderr_handle, stderr_stream);
        let status = wait_result?;
        Ok((status, stdout_result?, stderr_result?))
    }

    fn wait_for_git_child(&self, child: &mut Child, operation: &str) -> CoreResult<ExitStatus> {
        let started = Instant::now();
        loop {
            if self.cancellation.is_cancelled() {
                terminate_git_process_tree(child);
                return Err(CoreError::external_cancelled(
                    "git",
                    ExternalDispatchOutcome::DispatchedUnknown,
                ));
            }
            if started.elapsed() >= self.timeout {
                terminate_git_process_tree(child);
                return Err(CoreError::ExternalOperation {
                    service: "git".to_owned(),
                    outcome: ExternalDispatchOutcome::DispatchedUnknown,
                    message: format!(
                        "git {operation} timed out after {} ms",
                        self.timeout.as_millis()
                    ),
                });
            }
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {}
                Err(error) => {
                    terminate_git_process_tree(child);
                    return Err(CoreError::from(error));
                }
            }
            std::thread::sleep(GIT_POLL_INTERVAL);
        }
    }
}

fn terminate_git_process_tree(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(any(unix, windows)))]
    let _ = child.kill();
    let _ = child.wait();
}

fn ensure_git_stdout_within_limit(output: &GitCommandOutput) -> CoreResult<()> {
    if output.stdout_bytes <= MAX_GIT_STDOUT_BYTES as u64 {
        return Ok(());
    }
    Err(CoreError::ResourceLimitExceeded {
        resource: "git_stdout".to_owned(),
        reason: format!("output exceeds {MAX_GIT_STDOUT_BYTES} bytes"),
    })
}

fn git_command_error(output: &GitCommandOutput) -> CoreError {
    git_command_error_from_parts(output.status, &output.stderr)
}

fn git_command_error_from_parts(status: ExitStatus, stderr: &str) -> CoreError {
    CoreError::External {
        service: "git".to_owned(),
        message: if stderr.trim().is_empty() {
            format!("git exited with status {status}")
        } else {
            stderr.trim().chars().take(2000).collect()
        },
    }
}

fn drain_bounded(mut reader: impl Read, limit: usize) -> std::io::Result<(String, u64)> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0u8; 8192];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        let remaining = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    Ok((String::from_utf8_lossy(&retained).into_owned(), total))
}

fn read_diff_preview(
    mut stdout: impl Read,
    preview_char_limit: usize,
) -> CoreResult<GitDiffPreview> {
    let mut buffer = [0u8; 8192];
    let mut line_count = 0usize;
    let mut preview_bytes = Vec::with_capacity(preview_char_limit.saturating_mul(4));
    let preview_byte_limit = preview_char_limit.saturating_mul(4).saturating_add(4);
    let mut saw_bytes = false;
    let mut ended_with_newline = true;
    loop {
        let read = stdout.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        saw_bytes = true;
        ended_with_newline = buffer[read - 1] == b'\n';
        line_count =
            line_count.saturating_add(buffer[..read].iter().filter(|byte| **byte == b'\n').count());
        if preview_bytes.len() < preview_byte_limit {
            let remaining = preview_byte_limit - preview_bytes.len();
            preview_bytes.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    if saw_bytes && !ended_with_newline {
        line_count = line_count.saturating_add(1);
    }
    Ok(GitDiffPreview {
        line_count,
        preview: String::from_utf8_lossy(&preview_bytes)
            .chars()
            .take(preview_char_limit)
            .collect(),
    })
}

fn join_git_reader<T>(
    handle: std::thread::JoinHandle<CoreResult<T>>,
    stream: &str,
) -> CoreResult<T> {
    handle.join().map_err(|_| CoreError::External {
        service: "git".to_owned(),
        message: format!("{stream} reader panicked"),
    })?
}

impl GitStagePolicy {
    /// 追加排除路径，自动去重和规范化。
    pub fn with_ignored_paths(mut self, paths: impl IntoIterator<Item = String>) -> Self {
        self.ignored_paths.extend(paths);
        self
    }

    fn exclude_pathspecs(&self) -> CoreResult<Vec<String>> {
        Ok(self
            .ignored_paths
            .iter()
            .map(|path| crate::config::normalize_git_ignored_path(path))
            .collect::<CoreResult<BTreeSet<_>>>()?
            .into_iter()
            .map(|path| format!(":(exclude,top,literal){path}"))
            .collect())
    }
}

fn default_ignored_paths() -> Vec<String> {
    vec![
        ".cache".to_owned(),
        ".runtime".to_owned(),
        ".indexes".to_owned(),
        ".knowledge".to_owned(),
        "costs.db".to_owned(),
        "costs.db-wal".to_owned(),
        "costs.db-shm".to_owned(),
        "runtime.db".to_owned(),
        "runtime.db-wal".to_owned(),
        "runtime.db-shm".to_owned(),
    ]
}

fn checkpoint_kind_from_summary(summary: &str) -> Option<CheckpointKind> {
    if summary.starts_with("Checkpoint:") {
        Some(CheckpointKind::Auto)
    } else if summary.starts_with("Archive:") {
        Some(CheckpointKind::Manual)
    } else {
        None
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
    let mut parts = line.splitn(6, '\x1f');
    let commit_id = parts.next()?.to_owned();
    let parents = parts
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let refs: Vec<String> = parts
        .next()
        .unwrap_or_default()
        .split(", ")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    let timestamp_ms = parse_git_timestamp_ms(parts.next()?)?;
    let author = non_empty(parts.next().unwrap_or_default());
    let summary = parts.next().unwrap_or_default().to_owned();
    let checkpoint_kind = checkpoint_kind_from_summary(&summary);
    let is_head = refs.iter().any(|value| {
        value == "HEAD" || value.starts_with("HEAD -> ") || value.ends_with(" -> HEAD")
    });

    Some(BranchGraphNode {
        commit_id,
        parents,
        refs,
        summary,
        timestamp_ms,
        author,
        checkpoint_kind,
        is_head,
    })
}

fn parse_git_commit_summary(line: &str) -> Option<GitCommitSummary> {
    let mut parts = line.splitn(4, '\x1f');
    let commit_id = parts.next()?.to_owned();
    let timestamp_ms = parse_git_timestamp_ms(parts.next()?)?;
    let author = non_empty(parts.next().unwrap_or_default());
    let summary = parts.next().unwrap_or_default().to_owned();
    Some(GitCommitSummary {
        checkpoint_kind: checkpoint_kind_from_summary(&summary),
        commit_id,
        summary,
        timestamp_ms,
        author,
    })
}

fn parse_git_timestamp_ms(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()?.checked_mul(1000)
}

fn non_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn branch_name_rejects_unsafe_values() {
        assert!(validate_branch_name("restore/checkpoint-1").is_ok());
        assert!(validate_branch_name("../main").is_err());
        assert!(validate_branch_name("-bad").is_err());
    }

    #[test]
    fn branch_graph_parser_handles_refs_and_parents() {
        let node = parse_branch_graph_node(
            "abc\x1fparent1 parent2\x1fHEAD -> main, tag: v1\x1f1721000000\x1fAriadne\x1fArchive: msg",
        )
        .unwrap();

        assert_eq!(node.commit_id, "abc");
        assert_eq!(node.parents, vec!["parent1", "parent2"]);
        assert_eq!(node.refs, vec!["HEAD -> main", "tag: v1"]);
        assert_eq!(node.summary, "Archive: msg");
        assert_eq!(node.timestamp_ms, 1_721_000_000_000);
        assert_eq!(node.author.as_deref(), Some("Ariadne"));
        assert_eq!(node.checkpoint_kind, Some(CheckpointKind::Manual));
        assert!(node.is_head);
    }

    #[test]
    fn recent_commit_parser_preserves_time_author_and_kind() {
        let commit =
            parse_git_commit_summary("abc\x1f1721000000\x1fAriadne\x1fCheckpoint: chapter")
                .unwrap();

        assert_eq!(commit.timestamp_ms, 1_721_000_000_000);
        assert_eq!(commit.author.as_deref(), Some("Ariadne"));
        assert_eq!(commit.checkpoint_kind, Some(CheckpointKind::Auto));
    }

    #[cfg(unix)]
    #[test]
    fn c9_git_runner_cancels_long_hook_and_process_tree() {
        let temp = tempfile::tempdir().unwrap();
        GitService::new(temp.path()).init_repository().unwrap();
        std::fs::write(temp.path().join("chapter.md"), "draft").unwrap();
        install_slow_pre_commit_hook(temp.path());

        let cancellation = ExecutionCancellation::new();
        let cancel_from_thread = cancellation.clone();
        let marker = temp.path().join("hook-started");
        let canceller = std::thread::spawn(move || {
            let started = Instant::now();
            while !marker.exists() && started.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(5));
            }
            cancel_from_thread.cancel();
        });
        let service = GitService::new(temp.path())
            .with_execution_policy(cancellation, Duration::from_secs(5));
        let started = Instant::now();
        let error = service.create_archive_point("cancelled", None).unwrap_err();
        canceller.join().unwrap();

        assert!(matches!(
            error,
            CoreError::ExternalCancellation { .. } | CoreError::Cancelled
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn c9_git_runner_times_out_long_hook() {
        let temp = tempfile::tempdir().unwrap();
        GitService::new(temp.path()).init_repository().unwrap();
        std::fs::write(temp.path().join("chapter.md"), "draft").unwrap();
        install_slow_pre_commit_hook(temp.path());

        let service = GitService::new(temp.path())
            .with_execution_policy(ExecutionCancellation::new(), Duration::from_millis(200));
        let started = Instant::now();
        let error = service.create_archive_point("timeout", None).unwrap_err();

        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    fn install_slow_pre_commit_hook(repo: &Path) {
        let hook = repo.join(".git").join("hooks").join("pre-commit");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\ntouch '{}'\nsleep 30\n",
                repo.join("hook-started").display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(hook, permissions).unwrap();
    }
}
