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

    // U116：曾有 `diff()` 与 `diff_with_policy(policy)` 一次性把整个 diff 读进 String，
    // 已删且不要重建。取代它们的是下面的 `diff_preview_with_policy`：同样的 pathspec，
    // 但流式统计行数、只保留限定字符数的预览，并并发排空 stderr 防死锁（C8）。
    // 百万字项目的一次 diff 足以撑爆内存，"先拿到全文再截断" 是错的顺序。

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
        // 先处理存量：排除 pathspec 只影响本次 add/status，**不会**让历史上已被
        // 跟踪的文件变成未跟踪（U207-A）。不先摘掉它，每个新 commit 都会继续
        // 携带索引里那份旧 blob，排除清单形同虚设。
        self.untrack_internal_state_files()?;
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

    /// 把历史上已被跟踪的 Ariadne 内部状态文件从**索引**中摘掉（U207-A 存量迁移）。
    ///
    /// ⚠️ **`git rm --cached` 只动索引，磁盘上的文件原样保留**。
    /// 也就是说这里不会删掉作者的写作知识库（`metadata.db`，18 张关系表），
    /// 只是让它今后不再被提交。谁把 `--cached` 去掉，就会在下一次存档时
    /// 真的删掉作者的全部写作知识——这不是笔误能挽回的那类改动。
    ///
    /// **幂等**：先 `git ls-files` 探一次，只有真的还在索引里才执行 `git rm`。
    /// 稳定态（绝大多数存档）下的代价是一次索引读取，不会反复改索引。
    /// 探测**不是**可省的优化：`git rm` 遇到匹配不到任何文件的 pathspec 会以
    /// `fatal: pathspec ... did not match any files` 退出 128（实测确认），
    /// 无条件执行会让第二次以后的每一次存档直接失败。
    ///
    /// **刻意不用一次性哨兵**（例如在 `.runtime` 下写个「已迁移」标记）：
    /// `restore_to_new_branch` 会 checkout 到修复之前的历史 commit，那棵树里
    /// `metadata.db` 仍是被跟踪的，checkout 之后它又回到索引里。哨兵在那之后
    /// 永不再触发，回档一次就把缺陷带回来了——探测式判断才是回档后依然正确的做法。
    ///
    /// **作用域刻意是固定清单 `INTERNAL_STATE_FILES`，不是 `policy.ignored_paths`**：
    /// 后者会随配置带上作者内容（`track_documents = false` 时整个 documents 目录都在里面），
    /// 照它摘索引会让下一个 commit 看起来像「作者的正文被全部删除」。
    fn untrack_internal_state_files(&self) -> CoreResult<()> {
        let mut list_args = vec!["ls-files".to_owned(), "-z".to_owned(), "--".to_owned()];
        list_args.extend(
            INTERNAL_STATE_FILES
                .iter()
                .map(|name| format!(":(top,literal){name}")),
        );
        let listed = self.run_git(list_args)?;
        // `-z` 输出以 NUL 分隔（且末尾带一个 NUL），空段直接丢掉。
        // 用 `-z` 而非按行读：避免 git 对非 ASCII 路径加引号转义那套规则。
        let tracked: Vec<String> = listed
            .split('\0')
            .filter(|entry| !entry.is_empty())
            .map(str::to_owned)
            .collect();
        if tracked.is_empty() {
            return Ok(());
        }

        // `--force` 只是免掉「索引内容与 HEAD 和工作区都不同」那道拦阻。
        // 对这些文件我们本来就想丢掉那份暂存内容；而 `--cached` 保证工作区文件不受影响，
        // 所以这里的 force 不可能毁掉任何用户数据。不加它的话，一个处于奇怪暂存态的
        // metadata.db 会让 `git rm` 报错，进而让用户点「创建存档」直接失败——
        // 清理动作绝不该把主功能拖下水。
        let mut remove_args = vec![
            "rm".to_owned(),
            "--cached".to_owned(),
            "--force".to_owned(),
            "--quiet".to_owned(),
            "--".to_owned(),
        ];
        remove_args.extend(
            tracked
                .into_iter()
                .map(|path| format!(":(top,literal){path}")),
        );
        self.run_git(remove_args)?;
        Ok(())
    }

    /// 判断某个 commit 是否仍存在于本仓库。
    ///
    /// 供运行态引用校验用：checkpoint 与 patch session 都以 commit id 记在运行快照里，
    /// 用户手动 `git gc`/回滚分支后这些 id 会悬空，必须能查出来告诉用户。
    ///
    /// 用 `--verify --quiet` 而非 `cat-file`：commit 不存在时前者以退出码 1 静默返回，
    /// 由 `optional_git_value` 映射成 `None`，不会把"正常的不存在"变成错误。
    ///
    /// **`^{commit}` 后缀不可省**（实测确认，摘掉它回归测试立刻变红）：
    /// 不带它时 `rev-parse --verify` 对任何合法的 40 位 hex **原样回显**、
    /// 根本不查对象是否存在，于是每个悬空 commit id 都会被判定为"健在"，
    /// 引用校验彻底失效。加上后缀才强制 Git 解析并确认目标真是一个 commit 对象，
    /// 顺带也挡住了指向 blob/tree 的标签。
    pub fn commit_exists(&self, commit_id: &str) -> CoreResult<bool> {
        if commit_id.trim().is_empty() {
            return Ok(false);
        }
        let spec = format!("{commit_id}^{{commit}}");
        Ok(self
            .optional_git_value(["rev-parse", "--verify", "--quiet", spec.as_str()])?
            .is_some())
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

/// 项目根下由 Ariadne 自己生成、**永远不该进版本控制**的 SQLite 状态文件。
///
/// 三个库都开了 `PRAGMA journal_mode = WAL`（见 `rag/store.rs`、`costs`、`retrieval/runtime`），
/// 所以每个库都要连 `-wal`/`-shm` 两个附属文件一起排：只排主库会让存档在 WAL
/// 未 checkpoint 的时刻照样带上二进制增量。
///
/// U207-A：`metadata.db` 此前**一个变体都没排**（另两个库连附属文件都排全了），
/// 于是写作知识库每次总结都给存档塞一个 160KB+ 的二进制 diff，
/// 且 SQLite 页面重排让「改一条记录」产生大范围字节变化。
///
/// ⚠️ 这个清单同时是**存量迁移**（`untrack_internal_state_files`）的作用域，
/// 因此只允许放「机器生成的项目内部状态」——绝不能放任何作者产出的路径。
const INTERNAL_STATE_FILES: &[&str] = &[
    "metadata.db",
    "metadata.db-wal",
    "metadata.db-shm",
    "costs.db",
    "costs.db-wal",
    "costs.db-shm",
    "runtime.db",
    "runtime.db-wal",
    "runtime.db-shm",
];

fn default_ignored_paths() -> Vec<String> {
    let mut paths = vec![
        ".cache".to_owned(),
        ".runtime".to_owned(),
        ".indexes".to_owned(),
        ".knowledge".to_owned(),
    ];
    paths.extend(INTERNAL_STATE_FILES.iter().map(|name| (*name).to_owned()));
    paths
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
