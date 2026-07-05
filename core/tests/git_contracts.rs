use std::fs;

use ariadne::git::{CheckpointKind, GitHealthStatus, GitService};

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

#[test]
fn git_service_initializes_and_reports_health() {
    let (_temp_dir, service) = init_test_repo();

    let health = service.health_check();

    assert_eq!(health.status, GitHealthStatus::Degraded);
    assert!(health.reason.unwrap().contains("no commits"));
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
