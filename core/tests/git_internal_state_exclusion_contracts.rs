//! U207-A：`metadata.db`（写作知识库，18 张关系表）绝不能进版本控制。
//!
//! **判据刻意落在真实 git 落盘产物上，而不是 `default_ignored_paths()` 这张常量表。**
//! 原因是排除清单和「文件是否被跟踪」之间隔着一个真实的语义断层：
//! pathspec 排除只影响本次 `add`/`status`，**不会**让历史上已被跟踪的文件变成未跟踪。
//! 只断言常量表里有 `"metadata.db"` 的用例，在存量项目上会一路全绿，
//! 而那些项目的每个存档仍然继续携带 160KB+ 的二进制 blob。
//!
//! 所以每条用例都走真实 `git`：`git ls-files` 看索引、`git ls-tree` 看提交出的树、
//! `git status` 看工作区，并且**同时**断言磁盘上的知识库文件还在、还能被打开读出内容
//! —— 后一条是为了钉住「我们摘的是索引，不是作者的数据」。

use std::path::Path;

use ariadne::commands::create_checkpoint_impl;
use ariadne::contracts::{SourceSpan, TextRange};
use ariadne::git::GitService;
use ariadne::rag::{MemoryWritingKnowledgeBase, SqliteWritingKnowledgeStore, StorySegment};
use serde_json::Value;

const SENTINEL_SEGMENT_ID: &str = "u207a-segment";

/// 初始化一个真实项目（含 git 仓库、`.config/app.yaml`、documents 目录）。
fn init_project() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), &["config", "user.name", "Ariadne Test"]);
    run_git(temp.path(), &["config", "user.email", "ariadne@example.test"]);
    std::fs::write(temp.path().join("documents").join("chapter.md"), "第一章正文").unwrap();
    temp
}

/// 用真实的 `SqliteWritingKnowledgeStore` 造出 `metadata.db`，并写入一条可回读的哨兵记录。
///
/// 不用 `std::fs::write` 伪造二进制：伪造出来的文件无法回读，
/// 就没法在存量迁移那条用例里证明「知识库内容一个字节都没丢」。
fn seed_knowledge_base(project: &Path, summary: &str) {
    let knowledge = MemoryWritingKnowledgeBase::new();
    knowledge
        .upsert_segment(StorySegment {
            segment_id: SENTINEL_SEGMENT_ID.to_owned(),
            number: "1".to_owned(),
            chapter_id: "chapter-1".to_owned(),
            summary: summary.to_owned(),
            source: SourceSpan {
                document_id: "documents/chapter.md".to_owned(),
                range: TextRange { start: 0, end: 6 },
                version: Some("u207a-v1".to_owned()),
            },
            metadata: Value::Null,
        })
        .unwrap();
    SqliteWritingKnowledgeStore::open(project)
        .unwrap()
        .save_knowledge(&knowledge)
        .unwrap();
    assert!(
        project.join("metadata.db").exists(),
        "前置条件不成立：真实 store 没有生成 metadata.db"
    );
}

/// 回读知识库里的哨兵概括；返回 None 表示记录丢了。
fn read_sentinel_summary(project: &Path) -> Option<String> {
    SqliteWritingKnowledgeStore::open(project)
        .unwrap()
        .load_knowledge()
        .unwrap()
        .segment(SENTINEL_SEGMENT_ID)
        .unwrap()
        .map(|segment| segment.summary)
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// 索引里被跟踪的文件（这是「是否会被提交」的唯一权威口径）。
fn tracked_files(repo: &Path) -> Vec<String> {
    git_stdout(repo, &["ls-files"])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// HEAD 提交出的树（证明存档产物本身干净）。
fn committed_files(repo: &Path) -> Vec<String> {
    git_stdout(repo, &["ls-tree", "-r", "--name-only", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// 模拟**修复前**的旧版本存档：用那一版（排除清单里没有 metadata.db）的 `git add` 提交一次。
///
/// 这是存量场景的关键前置：真实老项目的 `git ls-files | grep '\.db$'` 只有 metadata.db
/// 一项（另两个库当年就排全了），所以这里照抄旧清单而不是简单 `git add --all`，
/// 否则会把 `.runtime`/`.indexes` 也跟踪进去，造出一个现实中不存在的仓库状态。
fn legacy_stage_and_commit(repo: &Path, message: &str) {
    let mut args: Vec<String> = vec![
        "add".to_owned(),
        "--all".to_owned(),
        "--".to_owned(),
        ".".to_owned(),
    ];
    for path in [
        ".cache",
        ".runtime",
        ".indexes",
        ".knowledge",
        "costs.db",
        "costs.db-wal",
        "costs.db-shm",
        "runtime.db",
        "runtime.db-wal",
        "runtime.db-shm",
    ] {
        args.push(format!(":(exclude,top,literal){path}"));
    }
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run_git(repo, &borrowed);
    run_git(repo, &["commit", "-m", message]);
}

#[test]
fn manual_checkpoint_never_tracks_metadata_db() {
    let temp = init_project();
    seed_knowledge_base(temp.path(), "存档不该带上我");

    create_checkpoint_impl(temp.path(), "首个存档".to_owned()).unwrap();

    let tracked = tracked_files(temp.path());
    let committed = committed_files(temp.path());
    // 正文必须在里面：否则本条用例可能只是因为存档整体没生效而"全绿"。
    assert!(
        committed.iter().any(|path| path == "documents/chapter.md"),
        "存档没有提交正文，用例失去意义：{committed:?}"
    );
    assert!(
        !tracked.iter().any(|path| path.starts_with("metadata.db")),
        "metadata.db 仍在索引中：{tracked:?}"
    );
    assert!(
        !committed.iter().any(|path| path.starts_with("metadata.db")),
        "metadata.db 进了存档提交：{committed:?}"
    );
    assert_eq!(
        read_sentinel_summary(temp.path()).as_deref(),
        Some("存档不该带上我"),
        "知识库内容必须完好"
    );
}

/// 存量场景（本条最重要）：老项目里 metadata.db 已被跟踪，
/// 走一次正常存档后它必须不再被跟踪，**而磁盘上的知识库必须一个字节都没少**。
#[test]
fn legacy_tracked_metadata_db_is_untracked_without_deleting_the_knowledge_base() {
    let temp = init_project();
    seed_knowledge_base(temp.path(), "第三章定下的设定");
    legacy_stage_and_commit(temp.path(), "旧版本存档");

    // 前置条件：必须真的处于"已被跟踪"的状态，否则本条用例什么都没验证。
    assert!(
        tracked_files(temp.path())
            .iter()
            .any(|path| path == "metadata.db"),
        "前置条件不成立：模拟的旧项目里 metadata.db 并未被跟踪"
    );

    create_checkpoint_impl(temp.path(), "修复后的存档".to_owned()).unwrap();

    let tracked = tracked_files(temp.path());
    let committed = committed_files(temp.path());
    assert!(
        !tracked.iter().any(|path| path == "metadata.db"),
        "存量迁移没生效，metadata.db 仍在索引里：{tracked:?}"
    );
    assert!(
        !committed.iter().any(|path| path == "metadata.db"),
        "新存档仍然携带 metadata.db：{committed:?}"
    );

    // 这三条一起证明"我们摘的是索引，不是作者的知识库"。
    assert!(
        temp.path().join("metadata.db").exists(),
        "迁移把作者的 metadata.db 从磁盘上删掉了"
    );
    assert_eq!(
        read_sentinel_summary(temp.path()).as_deref(),
        Some("第三章定下的设定"),
        "metadata.db 还在，但里面的记录读不出来了"
    );
    // 无 pathspec 的 status：文件应表现为"未跟踪但存在"。
    assert!(
        git_stdout(temp.path(), &["status", "--porcelain", "--untracked-files=all"])
            .lines()
            .any(|line| line.starts_with("??") && line.contains("metadata.db")),
        "metadata.db 应显示为未跟踪文件"
    );
}

/// `-wal`/`-shm` 两个附属文件同样要迁移：知识库开着 `journal_mode = WAL`，
/// 旧版本清单里连它们也没排，老仓库里可能三个文件都被跟踪。
#[test]
fn legacy_tracked_wal_and_shm_variants_are_untracked_too() {
    let temp = init_project();
    seed_knowledge_base(temp.path(), "WAL 附属文件也要摘");
    std::fs::write(temp.path().join("metadata.db-wal"), b"fake-wal").unwrap();
    std::fs::write(temp.path().join("metadata.db-shm"), b"fake-shm").unwrap();
    legacy_stage_and_commit(temp.path(), "旧版本存档带 WAL");

    let before = tracked_files(temp.path());
    for name in ["metadata.db", "metadata.db-wal", "metadata.db-shm"] {
        assert!(
            before.iter().any(|path| path == name),
            "前置条件不成立：{name} 未被跟踪"
        );
    }

    create_checkpoint_impl(temp.path(), "修复后的存档".to_owned()).unwrap();

    let tracked = tracked_files(temp.path());
    for name in ["metadata.db", "metadata.db-wal", "metadata.db-shm"] {
        assert!(
            !tracked.iter().any(|path| path == name),
            "{name} 仍在索引里：{tracked:?}"
        );
        assert!(
            temp.path().join(name).exists(),
            "{name} 被从磁盘删除了"
        );
    }
}

/// 幂等：连续存档不出错，也不会把已摘掉的文件重新拉回索引。
#[test]
fn repeated_checkpoints_stay_clean_and_do_not_error() {
    let temp = init_project();
    seed_knowledge_base(temp.path(), "幂等验证");
    legacy_stage_and_commit(temp.path(), "旧版本存档");

    for round in 0..3 {
        // 每轮都改一次知识库，模拟"总结节点又跑了一次"。
        seed_knowledge_base(temp.path(), &format!("幂等验证-{round}"));
        std::fs::write(
            temp.path().join("documents").join("chapter.md"),
            format!("第一章正文-{round}"),
        )
        .unwrap();
        create_checkpoint_impl(temp.path(), format!("存档-{round}")).unwrap();
        let tracked = tracked_files(temp.path());
        assert!(
            !tracked.iter().any(|path| path.starts_with("metadata.db")),
            "第 {round} 轮存档后 metadata.db 又被跟踪了：{tracked:?}"
        );
    }

    assert_eq!(
        read_sentinel_summary(temp.path()).as_deref(),
        Some("幂等验证-2"),
        "三轮存档后知识库应保留最后一次写入"
    );
}

/// 节点级 checkpoint 走的是 `GitService::create_checkpoint`（默认策略，
/// 不经过 `git_stage_policy_from_config`）——`documents/service.rs` 的写回链路用它。
/// 两条提交路径都必须干净，否则等于只修了一半。
#[test]
fn node_checkpoint_path_also_untracks_metadata_db() {
    let temp = init_project();
    seed_knowledge_base(temp.path(), "节点存档路径");
    legacy_stage_and_commit(temp.path(), "旧版本存档");
    assert!(
        tracked_files(temp.path())
            .iter()
            .any(|path| path == "metadata.db"),
        "前置条件不成立"
    );

    GitService::new(temp.path())
        .create_checkpoint("summarizer", None)
        .unwrap();

    let tracked = tracked_files(temp.path());
    assert!(
        !tracked.iter().any(|path| path.starts_with("metadata.db")),
        "节点 checkpoint 路径仍跟踪 metadata.db：{tracked:?}"
    );
    assert!(temp.path().join("metadata.db").exists());
}
