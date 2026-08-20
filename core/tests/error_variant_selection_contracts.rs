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
