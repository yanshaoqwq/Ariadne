use std::sync::Arc;

use ariadne::command_error::CommandErrorCode;
use ariadne::commands::{
    get_document_content_impl, save_document_content, save_document_content_with_version,
    AriadneAppState,
};
use ariadne::config::MemorySecretStore;

/// U196-A 回归：错误变体选对了，四件事一起修好。
///
/// # 缺陷本体
///
/// `CoreError::validation(msg)` 是个万能构造函数，被当成**错误垃圾桶**用了：
/// 正文保存版本冲突（CAS）、日预算打满这类完全无关的失败都落在同一句
/// `ui.error.validation`「输入内容不符合要求，请检查后重试」上 ——
/// 而这句话对它们**没有一次是对的**（作者的输入完全合法）。
///
/// # 判据为什么必须落在 `error.code` 上
///
/// 报告特意警告过：**不能**断言 `validate_write_base_version` 返回 `Err`。
/// 缺陷版本里它本来就返回 `Err`，只是变体选错了 ⇒ 那种断言在缺陷下照样全绿。
/// `error.code` 是 `command_error.rs` 的 `from_core` 按变体分派出来的结果，
/// 也就是**前端真正收到的东西**，它决定界面显示哪句文案。
fn new_state(root: &std::path::Path, app_state: &std::path::Path) -> AriadneAppState {
    AriadneAppState::new(
        root.to_path_buf(),
        app_state,
        Arc::new(MemorySecretStore::default()),
    )
}

/// CAS 冲突必须是 `Conflict`，不是 `Validation`。
///
/// 目标文案 `ui.error.conflict`「内容已被其它操作更新，请刷新后重试。」
/// （`display_name.json:1353`）**早就存在**，此前只是到不了这条路上。
#[test]
fn stale_base_version_reports_conflict_not_validation() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = new_state(temp.path(), app_state.path());
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let document_id = temp
        .path()
        .join("documents")
        .join("chapter-001.md")
        .to_string_lossy()
        .into_owned();

    // 先写一版拿到 version，再用它写第二版——这一步必须成功，
    // 否则下面的冲突断言测的是「随便一个失败」而不是「版本对不上」。
    let first = save_document_content(&state, document_id.clone(), "第一版".to_owned()).unwrap();
    let stale_version = first.metadata.version.clone();
    let second = save_document_content_with_version(
        &state,
        document_id.clone(),
        "第二版".to_owned(),
        Some(stale_version.clone()),
    )
    .unwrap();
    assert_ne!(
        second.metadata.version, stale_version,
        "两次写入的 version 相同 ⇒ 下面拿旧 version 去写根本不构成冲突，用例失效"
    );

    // 拿已经过期的 version 再写：这就是作者在另一个页面改过正文之后的处境。
    let error = save_document_content_with_version(
        &state,
        document_id.clone(),
        "第三版".to_owned(),
        Some(stale_version),
    )
    .unwrap_err();

    assert_eq!(
        error.code,
        CommandErrorCode::Conflict,
        "CAS 冲突被分派成 {:?} ⇒ 界面会说「输入内容不符合要求」，\
         而作者的输入完全合法、真实原因是正文在别处被改过。\
         那句话把他引向「检查自己刚打的字」，正确动作是「刷新后重做」",
        error.code
    );

    // 文案键必须是存在的那一个：code 对了但键不存在的话界面显示 [ui.error.conflict]。
    assert_eq!(error.code.message_key(), "ui.error.conflict");

    // 冲突不能顺带把正文写坏——拒绝就该是彻底的拒绝。
    assert_eq!(
        get_document_content_impl(temp.path(), Some(document_id), None).unwrap(),
        "第二版",
        "CAS 拒绝之后磁盘上应保持第二版；变成第三版说明检查发生在写入之后"
    );
}

/// 诊断串要带上两个版本值。
///
/// 作者看不懂哈希，但**它们不同**这一点本身是证据；
/// 而排查「为什么明明只有我一个人在编辑」时需要这两个值。
#[test]
fn conflict_diagnostic_carries_both_versions() {
    let temp = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    let state = new_state(temp.path(), app_state.path());
    ariadne::frontend::initialize_project(temp.path()).unwrap();

    let document_id = temp
        .path()
        .join("documents")
        .join("chapter-001.md")
        .to_string_lossy()
        .into_owned();

    let first = save_document_content(&state, document_id.clone(), "第一版".to_owned()).unwrap();
    let stale = first.metadata.version.clone();
    let second =
        save_document_content_with_version(&state, document_id.clone(), "第二版".to_owned(), Some(stale.clone()))
            .unwrap();

    let error =
        save_document_content_with_version(&state, document_id, "第三版".to_owned(), Some(stale.clone()))
            .unwrap_err();

    assert!(
        error.contains(&stale),
        "诊断里没有作者手上那个过期版本：{error:?}"
    );
    assert!(
        error.contains(&second.metadata.version),
        "诊断里没有磁盘上的当前版本，排查时无从判断被谁改过：{error:?}"
    );
}

/// U208-A 回归：「没配模型」不能报成「内容可能已被移动或删除」。
///
/// # 为什么判据必须是 `code` 而不是「返回了 Err」
///
/// 缺陷版本里它**本来就返回 Err**（`CommandError::not_found(...)`），
/// 只是变体选错。断言 `is_err()` 在缺陷下照样全绿——这与本文件另两条同理。
///
/// # 为什么这条是 P0 而不是文案瑕疵
///
/// `ui.error.not_found`「找不到所需内容，可能已被移动或删除」会让作者去
/// **翻版本页找回并没有丢的内容**，而正确动作是「去配置页配一个模型」。
/// 第一次用的人必然撞上（没配过模型就提问）⇒ 首次使用路径上的错误归因，
/// 代价是作者认为软件坏了。
#[test]
fn missing_provider_reports_not_configured_not_not_found() {
    let temp = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(temp.path()).unwrap();
    let secrets = MemorySecretStore::default();

    // 全新项目：没有任何 provider 配置，正是新用户第一次提问时的处境。
    let error = ariadne::commands::project_ai_chat_impl(
        temp.path(),
        &secrets,
        ariadne::commands::ProjectAiRequest {
            message: "hello".to_owned(),
            ..Default::default()
        },
    )
    .expect_err("没有启用的 LLM provider 时必须失败");

    assert_eq!(
        error.code,
        CommandErrorCode::NotConfigured,
        "没配模型被分派成 {:?}。若是 NotFound，界面显示「找不到所需内容，\
         可能已被移动或删除」——作者会去翻版本页找回没丢的东西，\
         而他要做的是去配置页配模型。这是把人指向相反方向。诊断：{error}",
        error.code
    );

    // 文案键必须存在：code 对了但键缺失的话界面显示 [ui.error.not_configured]。
    assert_eq!(error.code.message_key(), "ui.error.not_configured");
}

/// U208-A 的「同类兄弟」守卫：不让同一模式在别处再来一遍。
///
/// # 为什么单靠上面那条行为用例不够
///
/// U196-A 修完 `Validation` 当垃圾桶那两个实例之后，**同一模式在别处照旧**——
/// 本轮就在 `commands.rs` 里找出 3 处漏网的 `not_found("... is not configured")`
/// （`:3642` provider id 查不到、`:9392` 同上另一路、`:10756` Web Search 未配）。
/// 行为用例只钉住它走到的那一条路；模式守卫钉住的是**类**。
///
/// # 判据形态刻意是源码扫描
///
/// 这类缺陷的共同特征是「构造点选错变体」，而构造点分散在几十处、
/// 大多没有便于构造的入口。逐个写行为用例的成本远高于收益，
/// 而扫描能一次覆盖全部现存与将来新增的构造点。
#[test]
fn no_provider_configuration_failure_uses_not_found() {
    let source = include_str!("../src/commands.rs");
    let mut offenders = Vec::new();

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        // 注释行不算：注释里成段引用示例写法是正常的，
        // 而那恰好会被当成真构造点（同类坑见 half-wired 那条记忆）。
        if trimmed.starts_with("//") || trimmed.starts_with("///") {
            continue;
        }
        if !line.contains("not_found") {
            continue;
        }
        // 「未配置」这件事的两种说法都盖上。
        if line.contains("is not configured") || line.contains("is configured") {
            offenders.push(format!("  commands.rs:{}: {}", index + 1, line.trim()));
        }
    }

    assert!(
        offenders.is_empty(),
        "这些构造点用 `not_found` 报「未配置」，界面会显示「找不到所需内容，\
         可能已被移动或删除」，把作者指向找回没丢的内容。应改用 `not_configured`：\n{}",
        offenders.join("\n")
    );

    // 自检下限：正则/关键词失配时循环空跑也会绿。
    // 确认扫描确实看到了这一类构造点（改用正确变体之后它们仍在源码里）。
    let configured_sites = source
        .lines()
        .filter(|line| line.contains("not_configured"))
        .count();
    assert!(
        configured_sites >= 4,
        "只在 commands.rs 找到 {configured_sites} 处 not_configured 构造点，\
         少于修复时确认的 4 处 ⇒ 扫描口径失配，上面的检查可能是空跑"
    );
}
