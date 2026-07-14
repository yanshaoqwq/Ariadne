use std::fs;

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
