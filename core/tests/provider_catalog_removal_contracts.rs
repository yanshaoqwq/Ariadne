use std::sync::Arc;

use ariadne::commands::{
    get_provider_config, preview_provider_removal, remove_provider, save_provider_key,
    save_provider_settings, AriadneAppState, ProviderSettingsUpdate,
};
use ariadne::config::{MemorySecretStore, ModelConfig, ProviderCatalogStore, SecretStore};
use ariadne::contracts::{ProviderCapability, ProviderType};

/// U157（P1）：删除 Provider 后**应用级目录里的条目也必须消失**。
///
/// 缺陷形态：`save_provider_settings` 把 Provider **同时**写进项目配置和
/// 应用级目录（`app-state/provider_catalog.json`，跨项目共享），
/// 但 `remove_provider` **只清项目配置那一份**——全函数不出现 `ProviderCatalogStore`；
/// 而 `provider_config_status_from_config_with_app_state` 又会把目录条目**合并回列表**。
///
/// **更根本一层**：`ProviderCatalog` 上有 `upsert` / `merge_authorized` /
/// `project_projection`，但 `remove` **从来没被建出来过**——写侧建了、删侧没建。
///
/// 用户看到的是「删除按钮没用」，并陷入一个「灰着、不能用、又删不掉」的僵尸条目：
/// 项目授权已移除、密钥已删、默认路由已清、拿它跑工作流已被拒，
/// 但它在设置页列表里仍显示为一个完整条目，磁盘上 base_url 与模型清单原样留着——
/// 用户删的是「不用这家服务商了」，留下的却是这家的完整接入配置。
///
/// **产品决策已定夺：从应用级彻底删除**（不是「只移除本项目授权」）。
fn provider_update(provider_id: &str) -> ProviderSettingsUpdate {
    ProviderSettingsUpdate {
        provider_id: provider_id.to_owned(),
        provider_type: ProviderType::OpenAi,
        display_name: "Shared".to_owned(),
        enabled: true,
        base_url: None,
        models: vec![ModelConfig {
            model_id: "m1".to_owned(),
            capability: ProviderCapability::Llm,
            max_context_tokens: Some(32_000),
            input_cost_per_million_tokens: None,
            output_cost_per_million_tokens: None,
        }],
        make_default_llm: true,
        make_default_embedding: false,
        make_default_reranker: false,
        make_default_search: false,
    }
}

/// **U157 主用例**：判据是**磁盘上 `provider_catalog.json` 不含该条目**。
///
/// ⚠️ 报告特别指定了这个判据，因为只断言 API 返回值挡不住一种改法：
/// 「返回时过滤掉、磁盘照留」——那样用例全绿而 base_url 仍在盘上，
/// 换个项目打开又会看到它。**这就是本条的变异测试点**。
///
/// 三个层次一起断言，各自不可省：
/// 1. 磁盘 JSON 里不含 provider_id（根本判据）
/// 2. `remove_provider` **自己返回的**那份状态里不含它
///    （原缺陷下这一条就已经失败——不是前端缓存也不是没刷新）
/// 3. 重新读一次配置仍不含它（排除「返回值对但没落盘」）
#[test]
fn removing_a_provider_erases_the_app_level_catalog_entry() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    let state = AriadneAppState::new(project.path(), app_state.path(), Arc::clone(&secrets));

    save_provider_settings(&state, provider_update("shared-openai")).unwrap();
    save_provider_key(&state, "shared_openai".to_owned(), "key".to_owned()).unwrap();

    // 前置：保存确实把它写进了应用级目录，否则本用例测不到「删除清不清目录」。
    let catalog_store = ProviderCatalogStore::default_for_app(app_state.path());
    let before = catalog_store.read().unwrap();
    assert!(
        before
            .providers
            .iter()
            .any(|provider| provider.provider_id == "shared_openai"),
        "前置不成立：保存没有把 Provider 写进应用级目录，用例失去意义"
    );

    let preview = preview_provider_removal(&state, "shared_openai".to_owned()).unwrap();
    let status = remove_provider(&state, "shared_openai".to_owned(), preview.revision).unwrap();

    // ① 根本判据：磁盘上不含它。
    let after = catalog_store.read().unwrap();
    assert!(
        !after
            .providers
            .iter()
            .any(|provider| provider.provider_id == "shared_openai"),
        "应用级目录里仍留着 shared_openai——用户删的是「不用这家服务商了」，\
         留下的却是这家的完整接入配置（U157）。磁盘内容：{:?}",
        after.providers
    );

    // ② remove_provider 自己返回的状态里就不该有它。
    assert!(
        !status
            .providers
            .iter()
            .any(|entry| entry.provider == "shared_openai"),
        "remove_provider 返回的状态里仍含该 Provider——设置页会直接显示成「删除按钮没用」"
    );

    // ③ 重读一次：排除「返回值算对了但没落盘」。
    let reread = get_provider_config(&state).unwrap();
    assert!(
        !reread
            .providers
            .iter()
            .any(|entry| entry.provider == "shared_openai"),
        "重新读取配置后 shared_openai 又出现了——说明删除没有真正落盘"
    );
}

/// 删除后**不存在需要重删的僵尸状态**。
///
/// 报告里这条原是「未实测的推测」，实测**成立**：缺陷版本下第二次
/// `preview_provider_removal` 报 `NotFound: provider is not configured`
/// （因为它走项目配置，而该 Provider 已不在其中），
/// 于是用户看着列表里那条僵尸条目，**连重试的路都没有**。
///
/// 修好之后第二次删除**仍然**报 NotFound——但那已是正确行为：
/// 它真的不在了。所以判据不能是「第二次删除要成功」，
/// 而必须是「**列表里没有它，因此没人会去点第二次**」。
/// 判据选错这一层会逼出一个错误的实现（让删除对不存在的 Provider 也返回成功）。
#[test]
fn no_zombie_entry_remains_that_would_need_a_second_removal() {
    let project = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project.path()).unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
    let state = AriadneAppState::new(project.path(), app_state.path(), Arc::clone(&secrets));

    save_provider_settings(&state, provider_update("shared-openai")).unwrap();
    let preview = preview_provider_removal(&state, "shared_openai".to_owned()).unwrap();
    remove_provider(&state, "shared_openai".to_owned(), preview.revision).unwrap();

    let listed = get_provider_config(&state).unwrap();
    assert!(
        !listed
            .providers
            .iter()
            .any(|entry| entry.provider == "shared_openai"),
        "列表里仍有该条目 ⇒ 用户会去点第二次删除，而那必然报 NotFound（U157）"
    );
}

/// 跨项目语义：在项目 A 里删除，**项目 B 也不再看到它**。
///
/// 这是「从应用级彻底删除」这条产品决策的直接检验。
/// 目录是跨项目共享的，所以删除的影响面必须是可验证的、而不是靠注释声明。
/// 预览对话框据此必须明确告知「这会影响其它项目」——
/// 那条文案的存在理由就是本用例证明的这个事实。
#[test]
fn removal_is_visible_from_a_second_project_sharing_the_catalog() {
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    let app_state = tempfile::tempdir().unwrap();
    ariadne::frontend::initialize_project(project_a.path()).unwrap();
    ariadne::frontend::initialize_project(project_b.path()).unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());

    let state_a = AriadneAppState::new(project_a.path(), app_state.path(), Arc::clone(&secrets));
    save_provider_settings(&state_a, provider_update("shared-openai")).unwrap();

    // 前置：共享目录里确实有它（两个项目指向同一个 app_state）。
    let catalog_store = ProviderCatalogStore::default_for_app(app_state.path());
    assert!(catalog_store
        .read()
        .unwrap()
        .providers
        .iter()
        .any(|provider| provider.provider_id == "shared_openai"));

    let preview = preview_provider_removal(&state_a, "shared_openai".to_owned()).unwrap();
    remove_provider(&state_a, "shared_openai".to_owned(), preview.revision).unwrap();

    // 项目 B 用同一个 app_state 打开：目录条目已被移除，它也看不到了。
    let state_b = AriadneAppState::new(project_b.path(), app_state.path(), Arc::clone(&secrets));
    let listed_in_b = get_provider_config(&state_b).unwrap();
    assert!(
        !listed_in_b
            .providers
            .iter()
            .any(|entry| entry.provider == "shared_openai"),
        "项目 B 仍看到该 Provider——「从应用级彻底删除」这条决策没有真正生效"
    );
}
