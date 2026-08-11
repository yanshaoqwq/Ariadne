use std::fs;

use ariadne::contracts::CoreError;
use ariadne::git::{CheckpointKind, GitHealthStatus, GitService, GitStagePolicy};

/// 初始化带本地提交身份的临时 Git 仓库。
fn init_test_repo() -> (tempfile::TempDir, GitService) {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = GitService::new(temp_dir.path());
    service.init_repository().unwrap();
    run_git(temp_dir.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp_dir.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    (temp_dir, service)
}

/// 在临时仓库里执行 git 命令并断言成功。
fn run_git<const N: usize>(repo: &std::path::Path, args: [&str; N]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout<const N: usize>(repo: &std::path::Path, args: [&str; N]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn git_service_initializes_and_reports_health() {
    let (_temp_dir, service) = init_test_repo();

    let health = service.health_check().expect("health");

    assert_eq!(health.status, GitHealthStatus::Degraded);
    assert!(health.reason.unwrap().contains("no commits"));
}

#[test]
fn git_service_sets_project_local_commit_identity() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = GitService::new(temp_dir.path());
    service.init_repository().unwrap();
    fs::write(temp_dir.path().join("chapter.md"), "first").unwrap();

    let archive = service.create_archive_point("draft-1", None).unwrap();
    let commits = service.recent_commits(5).unwrap();
    let user_name = git_stdout(temp_dir.path(), ["config", "--local", "--get", "user.name"]);
    let user_email = git_stdout(
        temp_dir.path(),
        ["config", "--local", "--get", "user.email"],
    );

    assert!(!archive.commit_id.is_empty());
    assert_eq!(commits.len(), 1);
    assert_eq!(user_name.trim(), "Ariadne");
    assert_eq!(user_email.trim(), "ariadne@local.invalid");
}

#[test]
fn git_service_returns_empty_history_for_unborn_repository() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = GitService::new(temp_dir.path());
    service.init_repository().unwrap();

    assert!(service.recent_commits(5).unwrap().is_empty());
    assert!(service.branch_graph(5).unwrap().is_empty());
}

#[test]
fn git_service_creates_archive_and_checkpoint_commits() {
    let (temp_dir, service) = init_test_repo();
    fs::write(temp_dir.path().join("chapter.md"), "first").unwrap();

    let archive = service.create_archive_point("draft-1", None).unwrap();
    fs::write(temp_dir.path().join("chapter.md"), "second").unwrap();
    let checkpoint = service.create_checkpoint("node-1", None).unwrap();
    let commits = service.recent_commits(5).unwrap();

    assert_ne!(archive.commit_id, checkpoint.commit_id);
    assert_eq!(archive.checkpoint_kind, CheckpointKind::Manual);
    assert_eq!(checkpoint.checkpoint_kind, CheckpointKind::Auto);
    assert_eq!(checkpoint.node_id, "node-1");
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].checkpoint_kind, Some(CheckpointKind::Auto));
    assert_eq!(commits[1].checkpoint_kind, Some(CheckpointKind::Manual));
}

/// U116：运行态引用校验靠这个判定 checkpoint / patch commit 是否还在。
///
/// 不存在的 id 必须是 false（否则悬空引用永远查不出来），存在的必须是 true
/// （否则诊断天天误报），空串/非法 revision 既不能报存在也不能变成错误——
/// 诊断路径上抛错会把整份报告顶掉。
///
/// **最关键的是那条 40 位假 hex**：`rev-parse --verify` 不带 `^{commit}` 时
/// 会把任何合法 hex **原样回显**、根本不查对象存在性，于是每个悬空 id 都被判"健在"，
/// 校验形同虚设。这条断言是 `^{commit}` 后缀的直接回归（已变异验证：摘掉后缀它立刻变红）。
/// blob 标签那条则覆盖"对象存在但类型不是 commit"。
#[test]
fn commit_existence_check_accepts_real_commits_and_rejects_non_commit_objects() {
    let (temp_dir, service) = init_test_repo();
    fs::write(temp_dir.path().join("chapter.md"), "first").unwrap();
    let archive = service.create_archive_point("draft-1", None).unwrap();

    assert!(service.commit_exists(&archive.commit_id).unwrap());
    // 合法 hex 但仓库里没有这个对象。
    assert!(!service
        .commit_exists("0123456789abcdef0123456789abcdef01234567")
        .unwrap());
    // 空串与非法 revision 都不能报存在，也不能变成错误。
    assert!(!service.commit_exists("").unwrap());
    assert!(!service.commit_exists("   ").unwrap());
    assert!(!service.commit_exists("不是一个-revision").unwrap());

    // 指向 blob 的标签：rev-parse --verify 能解析，但它不是 commit，
    // 拿去恢复必然失败，所以必须报"不存在"。
    let blob_id = {
        let output = std::process::Command::new("git")
            .args(["hash-object", "-w", "chapter.md"])
            .current_dir(temp_dir.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    run_git(temp_dir.path(), ["tag", "blob-tag", &blob_id]);
    assert!(
        !service.commit_exists("blob-tag").unwrap(),
        "指向 blob 的标签不是 commit，必须报不存在"
    );
}

#[test]
fn git_history_exposes_time_author_head_and_checkpoint_semantics() {
    let (temp_dir, service) = init_test_repo();
    fs::write(temp_dir.path().join("chapter.md"), "first").unwrap();
    let archive = service.create_archive_point("draft-1", None).unwrap();

    let commits = service.recent_commits(5).unwrap();
    let graph = service.branch_graph(5).unwrap();

    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].commit_id, archive.commit_id);
    assert!(commits[0].timestamp_ms > 0);
    assert_eq!(commits[0].author.as_deref(), Some("Ariadne Test"));
    assert_eq!(commits[0].checkpoint_kind, Some(CheckpointKind::Manual));

    assert_eq!(graph.len(), 1);
    assert_eq!(graph[0].commit_id, archive.commit_id);
    assert!(graph[0].timestamp_ms > 0);
    assert_eq!(graph[0].author.as_deref(), Some("Ariadne Test"));
    assert_eq!(graph[0].checkpoint_kind, Some(CheckpointKind::Manual));
    assert!(graph[0].is_head);
    assert!(graph[0]
        .refs
        .iter()
        .any(|reference| reference.starts_with("HEAD -> ")));
}

#[test]
fn git_service_streams_bounded_diff_preview_and_reuses_porcelain_status() {
    let (temp_dir, service) = init_test_repo();
    let document = temp_dir.path().join("chapter.md");
    fs::write(&document, "original\n").unwrap();
    service.create_checkpoint("initial", None).unwrap();
    let changed = (0..2_000)
        .map(|index| format!("changed line {index}\n"))
        .collect::<String>();
    fs::write(&document, changed).unwrap();

    let (health, porcelain) = service
        .health_check_with_policy(&GitStagePolicy::default())
        .unwrap();
    let diff = service
        .diff_preview_with_policy(&GitStagePolicy::default(), 128)
        .unwrap();

    assert!(health.dirty);
    assert!(!porcelain.trim().is_empty());
    assert!(diff.line_count > 2_000);
    assert_eq!(diff.preview.chars().count(), 128);
    assert!(diff.preview.contains("diff --git"));
}

#[test]
fn git_service_bounds_preview_for_a_single_very_long_line() {
    let (temp_dir, service) = init_test_repo();
    let document = temp_dir.path().join("chapter.md");
    fs::write(&document, "original\n").unwrap();
    service.create_checkpoint("initial", None).unwrap();
    fs::write(&document, format!("{}\n", "长".repeat(700_000))).unwrap();

    let diff = service
        .diff_preview_with_policy(&GitStagePolicy::default(), 96)
        .unwrap();

    assert!(diff.line_count > 0);
    assert_eq!(diff.preview.chars().count(), 96);
    assert!(diff.preview.contains("diff --git"));
}

#[test]
fn git_health_distinguishes_absent_repository_from_corrupt_metadata() {
    let absent = tempfile::tempdir().unwrap();
    let absent_health = GitService::new(absent.path()).health_check().unwrap();
    assert_eq!(absent_health.status, GitHealthStatus::NotRepository);

    let corrupt = tempfile::tempdir().unwrap();
    fs::write(corrupt.path().join(".git"), "broken git metadata\n").unwrap();
    let error = GitService::new(corrupt.path()).health_check().unwrap_err();

    assert!(matches!(error, CoreError::External { ref service, .. } if service == "git"));
}

#[test]
fn git_service_excludes_default_runtime_paths_from_checkpoints() {
    let (temp_dir, service) = init_test_repo();
    fs::create_dir_all(temp_dir.path().join("documents")).unwrap();
    fs::create_dir_all(temp_dir.path().join(".runtime")).unwrap();
    fs::write(temp_dir.path().join("documents").join("chapter.md"), "正文").unwrap();
    fs::write(
        temp_dir.path().join(".runtime").join("runtime.db"),
        "runtime",
    )
    .unwrap();

    service.create_checkpoint("node-1", None).unwrap();

    let tree = git_stdout(temp_dir.path(), ["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(tree.contains("documents/chapter.md"));
    assert!(!tree.contains(".runtime/runtime.db"));
}

#[test]
fn git_service_restores_to_new_branch_and_marks_rebuild() {
    let (temp_dir, service) = init_test_repo();
    fs::write(temp_dir.path().join("chapter.md"), "first").unwrap();
    let archive = service.create_archive_point("draft-1", None).unwrap();
    fs::write(temp_dir.path().join("chapter.md"), "second").unwrap();
    service.create_checkpoint("node-1", None).unwrap();

    let report = service
        .restore_to_new_branch(&archive.commit_id, "restore/draft-1")
        .unwrap();

    assert_eq!(report.new_branch, "restore/draft-1");
    assert!(report.index_rebuild_required);
    assert!(report.runtime_rebind_required);
}

#[test]
fn git_service_rejects_restore_with_dirty_worktree() {
    let (temp_dir, service) = init_test_repo();
    fs::write(temp_dir.path().join("chapter.md"), "first").unwrap();
    let archive = service.create_archive_point("draft-1", None).unwrap();
    fs::write(temp_dir.path().join("chapter.md"), "dirty").unwrap();

    let error = service
        .restore_to_new_branch(&archive.commit_id, "restore/dirty")
        .unwrap_err();

    assert!(error.to_string().contains("worktree must be clean"));
}
