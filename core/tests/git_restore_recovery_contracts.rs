//! U196-B / U196-C：回档失败之后，作者还能不能继续写。
//!
//! # 这两条为什么必须放在同一个文件里
//!
//! 它们不是两个独立缺陷，而是**同一条死锁环路的两半**：
//!
//! 1. 工作区脏（极常见：刚写完一章、还没建存档）；
//! 2. 点「回档到副本」→ 后端 `ensure_clean_worktree` 拒绝；
//! 3. 但拒绝发生在 `begin_maintenance` **之后** ⇒ 项目落在 `failed` 维护态；
//! 4. `ensure_available` 对 `failed` 与 `active` 同等拦截 ⇒ 所有写操作被拒；
//! 5. 唯一能让工作区变干净的动作「创建存档」走 `acquire_project_mutation`，
//!    而那个函数**第一行就是 `ensure_available()`** ⇒ 一并被拒；
//! 6. 回到第 2 步：再点一次回档仍然脏、仍然失败。
//!
//! **环路闭合的根本原因是 `failed` 是吸收态**：`update_maintenance` 的 SQL 带
//! `WHERE id = 1 AND status = 'active'`，所以 `complete_maintenance` /
//! `update_maintenance_phase` 对 `failed` 全部无效，状态机自己出不来。
//!
//! ⇒ 报告 U196-B 里「真实出路是在 Git 页再点一次回档」这个前提**不成立**
//! （对最容易触发的成因不成立）。留档见报告的证伪小节。
//!
//! # 判据形态
//!
//! 全部落在**作者实际能做/看到的东西**上：一次真实的 `save_document_content_impl`
//! 是否成功、一次真实的 `create_checkpoint_impl` 是否成功、
//! 前端会收到的 `error.code` 与 `message_key` 是什么。
//!
//! ⚠️ 刻意**不**断言「某个函数返回了 Err」：缺陷版本里它们本来就返回 Err，
//! 只是变体不对、时机不对。那种断言在缺陷下照样全绿。

use std::sync::Arc;

use ariadne::command_error::CommandErrorCode;
use ariadne::commands::{
    create_checkpoint_impl, recover_project_maintenance, restore_to_new_branch,
    save_document_content_impl, AriadneAppState, RESTORE_DIRTY_WORKTREE_MESSAGE_KEY,
};
use ariadne::config::MemorySecretStore;
use ariadne::documents::IndexInvalidationOutbox;

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

/// 建一个有一次存档、Git 身份配好的项目，返回 (项目目录, app_state 目录, 首个 commit)。
fn project_with_one_checkpoint() -> (tempfile::TempDir, tempfile::TempDir, String) {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    run_git(temp.path(), ["config", "user.name", "Ariadne Test"]);
    run_git(
        temp.path(),
        ["config", "user.email", "ariadne@example.test"],
    );
    std::fs::write(temp.path().join("documents").join("chapter.md"), "base").unwrap();
    let checkpoint = create_checkpoint_impl(temp.path(), "base".to_owned()).unwrap();
    (temp, app_state, checkpoint.commit_id)
}

fn app_state(root: &std::path::Path, app_state_root: &std::path::Path) -> AriadneAppState {
    AriadneAppState::new(
        root.to_path_buf(),
        app_state_root,
        Arc::new(MemorySecretStore::default()),
    )
}

fn outbox(root: &std::path::Path) -> IndexInvalidationOutbox {
    IndexInvalidationOutbox::new(root.join(".runtime").join("index_invalidation.db"))
}

/// U196-C：工作区脏时回档被拒，而**项目一个字节都没动**。
///
/// # 判据为什么是「之后还能写」而不是「返回了 Err」
///
/// 缺陷版本同样返回 Err。真正的差别在于代价：缺陷版本把整个项目推进维护失败态
/// （下面那两条 `save` / `create_checkpoint` 在缺陷版本下**都会失败**），
/// 而正确版本只是拒绝了一次操作。
#[test]
fn dirty_worktree_restore_is_rejected_without_bricking_the_project() {
    let (temp, app_state_dir, commit_id) = project_with_one_checkpoint();
    let document = temp.path().join("documents").join("chapter.md");
    // 作者刚写了一段还没建存档——这就是「脏」的正常来源，不是异常状态。
    std::fs::write(&document, "作者刚写的一段，还没建存档").unwrap();
    let state = app_state(temp.path(), app_state_dir.path());

    let error = restore_to_new_branch(&state, commit_id, "restore/blocked".to_owned())
        .expect_err("工作区脏时回档必须被拒");

    // 1) 前端收到的 code：`Validation` 会让界面说「输入内容不符合要求，请检查后重试」，
    //    而作者的输入完全合法、真实原因是他有未提交的改动。
    assert_eq!(
        error.code,
        CommandErrorCode::Conflict,
        "脏工作区被分派成 {:?}；诊断：{error}",
        error.code
    );
    // 2) 作者读到的那句话：必须是回档专属文案，不是通用错误句。
    assert_eq!(error.message_key, RESTORE_DIRTY_WORKTREE_MESSAGE_KEY);

    // 3) **本条的核心**：维护态从未被置位 ⇒ 项目没有变成只读。
    assert!(
        outbox(temp.path()).maintenance_state().unwrap().is_none(),
        "拒绝发生在 begin_maintenance 之后 ⇒ 项目落进 failed 维护态，\
         一次极常见的误点代价是整个项目不可写"
    );

    // 4) 一次**真实的**保存必须成功。缺陷版本在这里会撞上
    //    「project maintenance blocks writes」。
    save_document_content_impl(
        temp.path(),
        document.to_string_lossy().into_owned(),
        "拒绝之后作者应当还能继续写".to_owned(),
    )
    .expect("回档被拒不该让保存正文一起失败");

    // 5) 一次**真实的**创建存档必须成功——这是死锁环路的关键一跳：
    //    「创建存档」正是让工作区变干净、从而能回档的那个动作。
    //    它在维护态下走 `acquire_project_mutation` 会被拒（见文件头注释第 5 步），
    //    于是作者失去唯一的自救手段。
    create_checkpoint_impl(temp.path(), "拒绝之后仍能建存档".to_owned())
        .expect("回档被拒不该让创建存档一起失败——那是作者唯一的自救动作");
}

/// U196-B：**真的走一遍死锁环路，再解开它**。
///
/// 这条用例本身就是死锁存在的可执行证明：
/// 中间那一段「创建存档被拒」不是顺带断言，它是环路闭合的那一跳。
///
/// # 为什么要手动置位 failed 而不是靠一次真实的失败回档
///
/// 因为 U196-C 修完之后，「脏工作区」这条路**再也进不了维护态**（那正是修复本体）。
/// 而 `failed` 态本身仍然可能由别的成因产生：磁盘满、断电、手动关窗口、
/// cancellation、checkout 冲突、索引重建失败……都会走到
/// `fail_maintenance("restore_incomplete", &error)`。
/// 所以这里用 outbox 的公开 API 复现**后果**（`failed` 态 + 脏工作区），
/// 再验证出路 —— 判据落在「项目回不回得到可写」，与成因无关。
#[test]
fn failed_maintenance_deadlock_is_broken_by_recovery_command() {
    let (temp, app_state_dir, _commit_id) = project_with_one_checkpoint();
    let document = temp.path().join("documents").join("chapter.md");
    std::fs::write(&document, "回档中断时作者手上还没提交的正文").unwrap();
    let state = app_state(temp.path(), app_state_dir.path());
    let gate = outbox(temp.path());

    // ——— 环路第 3 跳：回档中断，项目落在 failed ———
    gate.begin_maintenance("git_restore", "checking_out_branch")
        .unwrap();
    gate.fail_maintenance("restore_incomplete", "disk full while checking out")
        .unwrap();
    assert_eq!(
        gate.maintenance_state().unwrap().unwrap().status,
        "failed",
        "前提没造出来，下面的断言都失去意义"
    );

    // ——— 环路第 4 跳：所有写操作被拒 ———
    let write_error = save_document_content_impl(
        temp.path(),
        document.to_string_lossy().into_owned(),
        "作者想继续写".to_owned(),
    )
    .expect_err("failed 维护态必须拦住写入——门禁不生效的话本条无从谈起");
    assert!(
        write_error.contains("project maintenance blocks writes"),
        "拦住写入的不是维护门禁而是别的东西，用例测错了目标：{write_error}"
    );

    // ——— 环路第 5 跳（**死锁在这里闭合**）———
    // 「创建存档」是唯一能让工作区变干净、从而让回档成功的动作，
    // 而它走 `acquire_project_mutation`（第一行 `ensure_available()`）⇒ 同样被拒。
    // 于是「再点一次回档」这条报告认定的出路对脏工作区不成立。
    let checkpoint_error = create_checkpoint_impl(temp.path(), "想先清理工作区".to_owned())
        .expect_err("维护态下创建存档竟然成功了？那死锁不成立，本条前提要重写");
    assert!(
        checkpoint_error.contains("project maintenance blocks writes"),
        "创建存档失败的原因不是维护门禁：{checkpoint_error}"
    );

    // ——— 出路：解除维护态 ———
    let report = recover_project_maintenance(&state).expect("恢复命令必须能解除 failed 态");
    assert_eq!(report.cleared_kind, "git_restore");
    assert_eq!(report.cleared_phase, "restore_incomplete");
    assert_eq!(
        report.cleared_error.as_deref(),
        Some("disk full while checking out"),
        "中断诊断必须带回给作者——那是他判断「回档做了多少」的唯一线索"
    );

    // 状态机真的出来了（不能只看命令返回 ok）。
    let after = gate.maintenance_state().unwrap().unwrap();
    assert_eq!(
        after.status, "completed",
        "命令报成功但状态仍是 {}，写操作照旧被拒",
        after.status
    );

    // ——— 判据：作者实际能做的两件事都回来了 ———
    save_document_content_impl(
        temp.path(),
        document.to_string_lossy().into_owned(),
        "解除之后作者继续写".to_owned(),
    )
    .expect("解除维护态之后一次真实的保存必须成功");
    create_checkpoint_impl(temp.path(), "解除之后建一个存档".to_owned())
        .expect("解除维护态之后创建存档必须成功——这是回到干净工作区的那一步");

    // 索引重建必须入队：中断的回档之后索引与正文的对应关系不可信，
    // 直接放行写入会留下「搜到的是回档前的段落」且无法归因。
    //
    // ⚠️ 判据取「事件存在过」而不是 `has_incomplete_full_rebuild()`（**变异 M3 时发现**）：
    // 恢复命令末尾会**故意**起一个后台索引 worker，而那个 worker 的工作就是
    // 把这条事件消费掉。于是 `has_incomplete_full_rebuild()` 的结果取决于
    // 「worker 有没有抢在断言之前跑完」——一个真正的竞态判据，时红时绿。
    // 飘忽的断言比没有断言更糟：它会被后人当成 flaky 而直接删掉。
    //
    // `reason` 是我们自己写进去的字符串，无论事件此刻是 pending 还是已 completed，
    // 它都留在库里 ⇒ 判据确定，且仍然验证的是「重建真的被安排了」。
    let rebuild_events = full_rebuild_reasons(temp.path());
    assert!(
        rebuild_events
            .iter()
            .any(|reason| reason == "maintenance_recovery_full_rebuild"),
        "解除维护态却没有安排全量重建 ⇒ 作者从此搜到的可能是回档前的内容。\
         库里的 full_rebuild 事件：{rebuild_events:?}"
    );
}

/// 读出 outbox 里所有 `full_rebuild_required = 1` 事件的 reason（不论当前状态）。
///
/// 直接开 SQLite 而不用 `IndexInvalidationOutbox` 的公开 API：后者只提供
/// 「还没做完的」（`has_incomplete_full_rebuild` / `pending`），而本用例要断言的是
/// **重建被安排过**这件事——它对「已经被 worker 做完了」同样成立。
fn full_rebuild_reasons(project_root: &std::path::Path) -> Vec<String> {
    let connection = rusqlite::Connection::open(
        project_root.join(".runtime").join("index_invalidation.db"),
    )
    .unwrap();
    let mut statement = connection
        .prepare("SELECT reason FROM index_invalidation_events WHERE full_rebuild_required = 1")
        .unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

/// U196-B 的安全边界：**正在进行**的维护不能被解除。
///
/// `active` 表示确实有一场 checkout 在跑，清掉它等于绕过那段临界区的保护
/// （回档正在换分支时放行写入 = 写到一半的工作区上）。
///
/// 这条性质有两道独立保障，本用例钉住的是**外层**那道：
/// `recover_project_maintenance` 自己的 `filter(status == "failed")`。
/// 内层是 `begin_maintenance` 见到 `active` 直接返回 Err（`outbox.rs:71-79`）。
/// 摘掉任一道，另一道仍拦得住——但两道都要在，因为它们的失败文案不同：
/// 外层能给作者一句「没有需要解除的失败状态」，内层给的是英文诊断。
#[test]
fn recovery_refuses_to_clear_an_active_maintenance() {
    let (temp, app_state_dir, _commit_id) = project_with_one_checkpoint();
    let state = app_state(temp.path(), app_state_dir.path());
    let gate = outbox(temp.path());

    gate.begin_maintenance("git_restore", "checking_out_branch")
        .unwrap();

    let error = recover_project_maintenance(&state)
        .expect_err("active 维护态必须拒绝解除——那是一场正在跑的回档");
    assert_eq!(error.code, CommandErrorCode::Conflict);
    assert_eq!(
        error.message_key,
        ariadne::commands::MAINTENANCE_NOT_FAILED_MESSAGE_KEY
    );

    // 关键：状态一个字都没被改动。若这里变成 `completed`，
    // 意味着一场进行中的回档被顶掉了，比原缺陷严重得多。
    assert_eq!(
        gate.maintenance_state().unwrap().unwrap().status,
        "active",
        "解除被拒，但维护状态却被改了"
    );
}

/// U196-B：没有可解除的东西时**明确拒绝**，不报成功。
///
/// 把「什么都没做」说成「已恢复」会让作者以为写操作解禁了，
/// 而拦他的其实是别的东西——那属于「把失败报成成功」，
/// 本仓在 U160（工作流导出是报告成功的空动作）上已经踩过一次。
#[test]
fn recovery_refuses_when_there_is_nothing_to_recover() {
    let (temp, app_state_dir, _commit_id) = project_with_one_checkpoint();
    let state = app_state(temp.path(), app_state_dir.path());

    let error = recover_project_maintenance(&state)
        .expect_err("健康项目上点解除必须被拒，不能报「已恢复」");
    assert_eq!(error.code, CommandErrorCode::Conflict);
    assert_eq!(
        error.message_key,
        ariadne::commands::MAINTENANCE_NOT_FAILED_MESSAGE_KEY
    );

    // 没有维护记录的项目不该被这次调用凭空写出一条记录。
    assert!(outbox(temp.path()).maintenance_state().unwrap().is_none());
}

/// 两个自定义 message_key 必须真的存在于语言包里。
///
/// `CommandError::with_key` 挂的键不受编译器保护：拼错或忘记建键时，
/// `UserFailure.PrimaryText` 会走「键解析不出来就回落到 code 表」那条路
/// ⇒ 作者又读回那句「输入内容不符合要求」，而 code 与 key 的断言**全都是绿的**。
/// 这正是 U196-A 记下的「code 对了但键不存在」那种形态。
#[test]
fn custom_message_keys_exist_in_every_language_pack() {
    for pack in [
        "../core/resources/display_name.json",
        "../core/resources/display_name.en.json",
        "../core/resources/display_name.ja.json",
    ] {
        let raw = std::fs::read_to_string(pack).unwrap_or_else(|_| panic!("读不到语言包 {pack}"));
        let map: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).unwrap_or_else(|error| panic!("{pack} 不是平坦映射：{error}"));
        for key in [
            RESTORE_DIRTY_WORKTREE_MESSAGE_KEY,
            ariadne::commands::MAINTENANCE_NOT_FAILED_MESSAGE_KEY,
        ] {
            let value = map
                .get(key)
                .unwrap_or_else(|| panic!("{pack} 缺 {key} ⇒ 界面会显示 [{key}] 或回落到通用错误句"));
            assert!(!value.trim().is_empty(), "{pack} 的 {key} 是空串");
        }
    }
}
