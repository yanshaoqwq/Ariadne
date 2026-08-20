//! U207-F：版本页右栏「未提交变更」与「变更摘要」必须说同一件事。
//!
//! # 判据落在哪里
//!
//! 缺陷的用户可见形态是**两行数据自相矛盾**：
//! 「未提交变更 = 存在未提交变更」与「变更摘要 = 0 行 diff」同屏出现。
//! 所以这里每条用例都断言 `health.dirty` 与 `diff.line_count > 0` **一致**，
//! 而不是断言「diff 命令被调用了」或「行数大于某个数」——
//! 后两者在原缺陷下照样能全绿（原实现确实调用了 diff，也确实返回了 0）。
//!
//! # 报告前提的证伪
//!
//! `U207-实机交互取证第二轮.md` 的 F 段推断根因是
//! 「diff 命令的 pathspec 与 status 的口径不一致」。**这一条不成立**：
//! `diff_preview_with_policy` 与 `status_with_policy` 从一开始就都调
//! `policy.exclude_pathspecs()`，排除清单是同一份。
//! 真正的错位在**比较对象**（工作区↔索引 vs 工作区↔HEAD+未跟踪）。
//! `u207f_exclusion_policy_still_holds_after_alignment` 就是钉住这个区别的：
//! 排除机制必须**照旧生效**（它是 U207-A 的修复），只有比较对象变了。

use std::fs;

use ariadne::git::{GitService, GitStagePolicy};

/// 初始化带本地提交身份的临时 Git 仓库。
fn init_test_repo() -> (tempfile::TempDir, GitService) {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = GitService::new(temp_dir.path());
    service.init_repository().unwrap();
    run_git(temp_dir.path(), &["config", "user.name", "Ariadne Test"]);
    run_git(
        temp_dir.path(),
        &["config", "user.email", "ariadne@example.test"],
    );
    (temp_dir, service)
}

fn run_git(repo: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// 版本页右栏那两行的**同屏快照**：dirty 文案与 diff 行数各自取自哪里，
/// 就在这里各取一次，然后比它们是否讲同一件事。
struct PanelRows {
    dirty: bool,
    diff_line_count: usize,
    diff_preview: String,
}

fn read_panel_rows(service: &GitService, policy: &GitStagePolicy) -> PanelRows {
    let (health, _porcelain) = service.health_check_with_policy(policy).unwrap();
    // 预览限额取大，好让用例能对着路径名断言；生产用的是 4000。
    let diff = service.diff_preview_with_policy(policy, 20_000).unwrap();
    PanelRows {
        dirty: health.dirty,
        diff_line_count: diff.line_count,
        diff_preview: diff.preview,
    }
}

impl PanelRows {
    /// 唯一的核心断言：两行不许互相打脸。
    fn assert_rows_agree(&self) {
        assert_eq!(
            self.dirty,
            self.diff_line_count > 0,
            "版本页两行自相矛盾：未提交变更={} 而变更摘要={} 行 diff\n预览：{}",
            self.dirty,
            self.diff_line_count,
            self.diff_preview
        );
    }
}

/// 写 12 章正文但一次都没 `git add` —— 这正是报告实机取证时的现场。
fn write_twelve_untracked_chapters(root: &std::path::Path) {
    let chapters = root.join("documents").join("chapters");
    fs::create_dir_all(&chapters).unwrap();
    for index in 1..=12 {
        fs::write(
            chapters.join(format!("ch{index:03}.md")),
            format!("# 第 {index} 章\n\n这一章有正文，共三行。\n落在磁盘上，未提交。\n"),
        )
        .unwrap();
    }
}

#[test]
fn u207f_untracked_chapters_do_not_report_zero_line_diff() {
    let (temp_dir, service) = init_test_repo();
    let policy = GitStagePolicy::default();
    fs::write(temp_dir.path().join("README.md"), "seed\n").unwrap();
    service.create_checkpoint("seed", None).unwrap();

    write_twelve_untracked_chapters(temp_dir.path());
    let rows = read_panel_rows(&service, &policy);

    // 现场事实：12 章确实在磁盘上、确实未提交。
    assert!(rows.dirty, "12 章未提交正文必须让 dirty 成立");
    rows.assert_rows_agree();
    // 每章 4 行正文 + 6 行 diff 头（diff/new file/index/---/+++/@@），12 章共 120 行。
    assert_eq!(rows.diff_line_count, 120, "预览：{}", rows.diff_preview);
    assert!(
        rows.diff_preview.contains("documents/chapters/ch001.md"),
        "预览里应能看到章节路径：{}",
        rows.diff_preview
    );
}

#[test]
fn u207f_clean_worktree_agrees_on_zero_diff() {
    let (temp_dir, service) = init_test_repo();
    let policy = GitStagePolicy::default();
    write_twelve_untracked_chapters(temp_dir.path());
    service.create_checkpoint("存了 12 章", None).unwrap();

    let rows = read_panel_rows(&service, &policy);

    // 反方向也要一致：都说「没东西要存」。
    assert!(!rows.dirty, "刚存完档应当干净");
    rows.assert_rows_agree();
    assert_eq!(rows.diff_line_count, 0);
}

#[test]
fn u207f_deletion_and_staged_change_stay_inside_the_diff_summary() {
    let (temp_dir, service) = init_test_repo();
    let policy = GitStagePolicy::default();
    fs::write(temp_dir.path().join("keep.md"), "a\nb\nc\n").unwrap();
    fs::write(temp_dir.path().join("gone.md"), "g1\ng2\n").unwrap();
    service.create_checkpoint("seed", None).unwrap();

    // 三种形态各来一个：删除 / 已暂存的新文件 / 未暂存的修改。
    fs::remove_file(temp_dir.path().join("gone.md")).unwrap();
    fs::write(temp_dir.path().join("staged.md"), "s1\n").unwrap();
    run_git(temp_dir.path(), &["add", "staged.md"]);
    fs::write(temp_dir.path().join("keep.md"), "a\nb\nc\nd\n").unwrap();

    let rows = read_panel_rows(&service, &policy);

    rows.assert_rows_agree();
    for path in ["gone.md", "staged.md", "keep.md"] {
        assert!(
            rows.diff_preview.contains(path),
            "{path} 必须出现在变更摘要里，否则又是一处与 status 打脸的口径：{}",
            rows.diff_preview
        );
    }
    // `--ignore-removal` 掉了就会丢这一行，是本条最容易回归的一处。
    assert!(
        rows.diff_preview.contains("-g1"),
        "删除必须体现为减行：{}",
        rows.diff_preview
    );
}

#[test]
fn u207f_first_files_before_any_commit_count_as_additions() {
    let (temp_dir, service) = init_test_repo();
    let policy = GitStagePolicy::default();
    // 空仓库（无 HEAD）：这条守的是 read-tree 的分支选择，
    // 误用 `read-tree HEAD` 会让 git 直接 `fatal: bad revision 'HEAD'`。
    write_twelve_untracked_chapters(temp_dir.path());

    let rows = read_panel_rows(&service, &policy);

    assert!(rows.dirty);
    rows.assert_rows_agree();
    assert_eq!(rows.diff_line_count, 120);
}

#[test]
fn u207f_exclusion_policy_still_holds_after_alignment() {
    // ⚠️ 这条守的是 U207-A：项目**刻意不用 `.gitignore`**，靠 pathspec 排除内部状态。
    // 对齐口径时若图省事把排除项摘掉（或漏传给临时索引那两条命令），
    // 内部状态文件就会重新出现在变更摘要里，等于把 U207-A 一起弄坏。
    let (temp_dir, service) = init_test_repo();
    let policy = GitStagePolicy::default();
    fs::write(temp_dir.path().join("README.md"), "seed\n").unwrap();
    service.create_checkpoint("seed", None).unwrap();

    fs::create_dir_all(temp_dir.path().join(".indexes")).unwrap();
    fs::write(temp_dir.path().join(".indexes").join("blob.bin"), "junk\n").unwrap();
    fs::write(temp_dir.path().join("metadata.db"), "sqlite-ish\n").unwrap();
    fs::write(temp_dir.path().join("costs.db-wal"), "wal\n").unwrap();

    let rows = read_panel_rows(&service, &policy);

    assert!(
        !rows.dirty,
        "只有内部状态文件变动时 status 不该报脏（排除策略）"
    );
    rows.assert_rows_agree();
    assert_eq!(
        rows.diff_line_count, 0,
        "内部状态文件不许进变更摘要：{}",
        rows.diff_preview
    );
}

#[test]
fn u207f_diff_preview_leaves_real_index_and_body_text_out_of_object_store() {
    let (temp_dir, service) = init_test_repo();
    let policy = GitStagePolicy::default();
    fs::write(temp_dir.path().join("README.md"), "seed\n").unwrap();
    service.create_checkpoint("seed", None).unwrap();
    write_twelve_untracked_chapters(temp_dir.path());

    let status_before = run_git(
        temp_dir.path(),
        &["status", "--porcelain", "--untracked-files=all"],
    );
    // 正文的 blob 哈希：算出来但**不写入**对象库（hash-object 不带 -w）。
    let chapter_blob = run_git(
        temp_dir.path(),
        &["hash-object", "documents/chapters/ch001.md"],
    )
    .trim()
    .to_owned();
    assert!(
        !object_exists(temp_dir.path(), &chapter_blob),
        "前置条件坏了：正文 blob 在动作之前就已在对象库里"
    );

    let diff = service.diff_preview_with_policy(&policy, 200).unwrap();
    assert!(diff.line_count > 0);

    // 真实索引绝不能被临时索引这套动作波及：一旦被暂存，
    // 下一次 checkpoint 会把作者没打算存的东西一起提交。
    assert_eq!(
        run_git(
            temp_dir.path(),
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        status_before,
        "看一眼变更摘要不该改变仓库状态"
    );
    // `-N`（intent-to-add）只记「将要加入」，**不写正文内容**。
    // 换成 `git add -A` 这条就会红：那会把每一章都写成 blob，
    // 于是每次刷新版本页都往对象库塞一批松散对象（百万字项目里是真实成本）。
    //
    // ⚠️ 这里刻意不断言「对象数一个都没多」：`-N` 会写一个**共享的空 blob**
    // （e69de29，0 字节），内容寻址所以整个仓库一辈子只有这一个。
    // 断言总数不变会让用例因为这一个 0 字节对象而红，
    // 那是把无害实现细节当缺陷 —— 判据该落在「正文有没有被写进去」。
    assert!(
        !object_exists(temp_dir.path(), &chapter_blob),
        "变更摘要把正文写进了对象库（blob {chapter_blob}）"
    );
    // 临时索引落在 .git/ 下，不能出现在工作区里（放工作区会把自己扫进自己的 diff）。
    assert!(
        !status_before.contains("ariadne-diff-index"),
        "临时索引泄漏进了工作区：{status_before}"
    );
    for entry in fs::read_dir(temp_dir.path()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with("ariadne-diff-index"),
            "临时索引残留在工作区根：{name}"
        );
    }
}

/// 对象是否已在对象库里。用 `cat-file -e`（存在返回 0，缺失返回非 0）。
fn object_exists(repo: &std::path::Path, object_id: &str) -> bool {
    std::process::Command::new("git")
        .args(["cat-file", "-e", object_id])
        .current_dir(repo)
        .output()
        .unwrap()
        .status
        .success()
}

/// 本文件最有力的一条：**同时**守住「口径对齐」与「排除机制没被删掉」。
///
/// 这两件事有天然的对立张力 —— 让 diff 看见更多东西（对齐 status）
/// 与让 diff 看不见内部状态文件（U207-A 的排除）是反方向的压力。
/// 只测其中一边，另一边被弄坏时用例照样全绿：
/// - 只测「对齐」：把排除项摘掉也全绿 ⇒ U207-A 被悄悄推翻；
/// - 只测「排除」：diff 退回裸 `git diff` 也全绿 ⇒ 本条缺陷原地复活。
///
/// 所以现场必须**两种变动同时存在**：12 章未提交正文 + 内部状态 db 也刚变过。
#[test]
fn u207f_twelve_chapters_count_while_excluded_databases_do_not() {
    let (temp_dir, service) = init_test_repo();
    let policy = GitStagePolicy::default();
    fs::write(temp_dir.path().join("README.md"), "seed\n").unwrap();
    service.create_checkpoint("seed", None).unwrap();

    // 作者产出：12 章未提交正文（报告实机取证时的现场）。
    write_twelve_untracked_chapters(temp_dir.path());
    // 机器产出：写作知识库/预算/运行时三个 db 刚写过一轮，连 WAL 附属文件一起。
    // 每个都刻意写**很多行**——一旦被算进行数，与 120 的差距会立刻暴露。
    let noise = (0..500)
        .map(|index| format!("binary-ish row {index}\n"))
        .collect::<String>();
    for name in [
        "metadata.db",
        "metadata.db-wal",
        "metadata.db-shm",
        "costs.db",
        "costs.db-wal",
        "runtime.db",
        "runtime.db-wal",
    ] {
        fs::write(temp_dir.path().join(name), &noise).unwrap();
    }
    fs::create_dir_all(temp_dir.path().join(".indexes")).unwrap();
    fs::write(temp_dir.path().join(".indexes").join("seg.bin"), &noise).unwrap();

    let rows = read_panel_rows(&service, &policy);

    // ① 两行一致：说脏就必须有行数（本条缺陷的用户可见形态）。
    assert!(rows.dirty, "12 章未提交正文必须让 dirty 成立");
    rows.assert_rows_agree();
    // ② 行数**精确等于** 12 章的量：多一行就说明 db 噪声漏了进来。
    //    用等号而不是 `> 0`：`> 0` 在排除项失效时照样绿。
    assert_eq!(
        rows.diff_line_count, 120,
        "行数不等于 12 章的 120 行，说明被排除的内部状态文件被算了进去：{}",
        rows.diff_preview
    );
    // ③ 预览里只能有正文，不能有任何一个内部状态文件名。
    assert!(
        rows.diff_preview.contains("documents/chapters/ch001.md"),
        "正文没进变更摘要：{}",
        rows.diff_preview
    );
    for name in ["metadata.db", "costs.db", "runtime.db", ".indexes"] {
        assert!(
            !rows.diff_preview.contains(name),
            "被排除的 {name} 出现在变更摘要里（U207-A 的排除机制被弄坏了）：{}",
            rows.diff_preview
        );
    }
}
