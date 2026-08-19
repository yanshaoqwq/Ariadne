//! U201-C：节点「默认提示词占位符」的接受集合与展开器。
//!
//! # 这个模块解决什么
//!
//! 新建写作节点时，前端过去把 agent 的**默认提示词全文**（300~470 字）写进
//! `node.PromptTemplate` 并随工作流存盘。两个后果：右栏编辑框一进节点就被占满，
//! 而作者绝大多数时候不改它；工作流文件里存着一份全文副本，官方将来调整默认
//! 提示词，已建的节点不会跟着更新。
//!
//! 改法与 `{{ref:...}}` 同形：节点里**只存一行占位符字面量**
//! （`{{outliner 默认提示词}}`），运行时在这里展开成 `prompt_list.json` 里的全文。
//!
//! # 为什么接受集合是「三种语言写法的并集」
//!
//! 占位符是**给人看、也让人手打**的——这条功能的全部意义就是让作者看见并能照抄
//! 语法。作者照抄的是**屏幕上那一行**：中文界面抄中文写法、英文界面抄英文写法。
//! 于是有三条硬约束：
//!
//! 1. **手打是一等公民。** 若只认一种「内部标识符」，就得显示时翻译、保存时反向
//!    翻译；而手打的那份**没经过翻译器**，反向认不出就静默丢失——等于一边让人
//!    照抄，一边不认他抄的东西。
//! 2. **切界面语言不能让已存文件失效。** 占位符存在工作流文件里：中文界面建好、
//!    切英文界面，文件里还是中文写法。只认当前语言会立刻失效。
//! 3. **工作流要跨语言流转。** 模板市场（`install_template` /
//!    `export_workflow_selection`）让中文用户导出的模板进到英文用户的工程里，
//!    英文界面再编辑保存 ⇒ **同一文件里两种写法并存**。只认一种，另一种会落进
//!    fail-loud 的「未知占位符」分支，或更糟：原样进 LLM 请求体。
//!
//! ⇒ **解析宽容、生成唯一**：解析接受三种语言写法的并集，全部归一到同一个内部
//! 标识（`WritingAgentKind`）；生成/显示时按当前界面语言给一种。这与后端 provider
//! id 归一化是同一个模式。
//!
//! # 为什么并集必须从语言包动态构建
//!
//! en/ja 的待补译清单还在补。硬编码一张字面量表在补译之后会漏，而漏的形态是
//! 「英文界面手打的写法认不出」——用户看不到任何报错，只会发现节点行为不对，
//! 几乎无法归因。所以表由 `display_name*.json` 三份**编译期内联**的资源现算
//! （`resources.rs` 的 `DISPLAY_NAME_JSON` / `DISPLAY_NAME_EN_JSON` /
//! `DISPLAY_NAME_JA_JSON`），补译即自动生效。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::contracts::{CoreError, CoreResult};
use crate::rag::models::WritingAgentKind;
use crate::rag::resources::PromptResources;

/// 占位符文案的资源 key 前缀；后接 `WritingAgentKind::node_type()`。
pub const DEFAULT_PROMPT_PLACEHOLDER_KEY_PREFIX: &str = "ui.prompt.default_placeholder.";

/// 返回某 agent 的占位符文案资源 key。
pub fn default_prompt_placeholder_key(agent: WritingAgentKind) -> String {
    format!(
        "{DEFAULT_PROMPT_PLACEHOLDER_KEY_PREFIX}{}",
        agent.node_type()
    )
}

/// 占位符字面量的接受集合：**归一化后的**写法 → agent。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultPromptPlaceholderTable {
    accepted: BTreeMap<String, WritingAgentKind>,
    /// 同一写法被两个 agent 共用时记在这里；解析到它就 fail-loud。
    ///
    /// 不在建表时报错是刻意的：语言包里一处翻译撞车不该让**整个应用**起不来，
    /// 但用到那条歧义写法的节点必须停下——否则会静默跑错 agent 的提示词。
    ambiguous: BTreeSet<String>,
}

impl DefaultPromptPlaceholderTable {
    /// 把一份「key → 文案」资源表并进接受集合。
    ///
    /// 三份语言包依次并入即得并集；重复写法（多语言译文相同，例如 en 与 ja 的
    /// 待补译都回落成同一串）指向同一 agent 时不算歧义。
    pub fn merge_pack(&mut self, pack: &BTreeMap<String, String>) {
        for agent in WritingAgentKind::ALL {
            let Some(literal) = pack.get(&default_prompt_placeholder_key(agent)) else {
                continue;
            };
            let normalized = normalize_placeholder_literal(literal);
            if normalized.is_empty() {
                continue;
            }
            match self.accepted.get(&normalized) {
                Some(existing) if *existing == agent => {}
                Some(_) => {
                    self.ambiguous.insert(normalized);
                }
                None => {
                    self.accepted.insert(normalized, agent);
                }
            }
        }
    }

    /// 解析一个 `{{}}` 内部的写法；不是默认提示词占位符时返回 `Ok(None)`。
    pub fn resolve(&self, body: &str) -> CoreResult<Option<WritingAgentKind>> {
        let normalized = normalize_placeholder_literal(body);
        if normalized.is_empty() {
            return Ok(None);
        }
        if self.ambiguous.contains(&normalized) {
            return Err(CoreError::validation(format!(
                "default prompt placeholder `{body}` is ambiguous: two agents share this wording \
                 in the display name packs; fix `{DEFAULT_PROMPT_PLACEHOLDER_KEY_PREFIX}*` so each \
                 agent has a distinct literal"
            )));
        }
        Ok(self.accepted.get(&normalized).copied())
    }

}

/// 归一化一条写法：去掉**全部**空白并按 ASCII 小写折叠。
///
/// 去空白而不是只 trim：屏幕上那行中文写成 `{{outliner 默认提示词}}`，
/// 作者手打时极可能少打那个空格（中文里本来不需要空格）。两种都得认——
/// 这条功能就是为「照抄」服务的，为一个空格判他写错等于白做。
/// 英文写法两侧都归一，`Outliner Default Prompt` 与 `outliner default prompt`
/// 因此等价。
pub fn normalize_placeholder_literal(literal: &str) -> String {
    literal
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 全进程共享的接受集合；三份内联语言包只解析一次。
///
/// 存 `Result` 而不是 `Table`：建表失败必须能**传播出去**（理由见
/// `placeholder_table`），而 `OnceLock` 的初始化闭包不能返回错误。
/// 把 `Result` 整个缓存起来即可——三份都是编译期常量，成功/失败对每次运行相同，
/// 不存在「重试一次就好了」的情形。
type PlaceholderTableResult = Result<DefaultPromptPlaceholderTable, String>;
static PLACEHOLDER_TABLE: OnceLock<PlaceholderTableResult> = OnceLock::new();

/// 取（并首次构建）接受集合。
///
/// # 为什么建表失败要 fail-loud
///
/// 首版在这里把失败降级成一张空表，理由是「不该让整个应用起不来」。那是错的：
/// 空表 ⇒ 每一个占位符都认不出 ⇒ **每个写作节点都跑不起来**，而报错指向
/// 「未展开的占位符」，把人引向「是不是我写错了这一行」，而真正的原因是
/// 语言包坏了。降级并没有换来可用性，只换来了错误的诊断方向。
///
/// 三份语言包是 `include_str!` 的编译期常量 ⇒ 解析失败意味着我们发了个坏二进制，
/// 测试套件在发版前就会红，用户侧不存在这个状态。
pub fn placeholder_table() -> CoreResult<&'static DefaultPromptPlaceholderTable> {
    PLACEHOLDER_TABLE
        .get_or_init(|| {
            let packs = crate::rag::resources::all_display_name_packs()
                .map_err(|error| format!("{error:?}"))?;
            let mut table = DefaultPromptPlaceholderTable::default();
            for pack in &packs {
                table.merge_pack(pack);
            }
            Ok(table)
        })
        .as_ref()
        .map_err(|error| {
            CoreError::validation(format!(
                "cannot build the default prompt placeholder table from the bundled display name \
                 packs: {error}"
            ))
        })
}

/// 文本里是否可能含默认提示词占位符（便宜的预筛）。
///
/// 只查 `{{`：这里不能像 `contains_content_reference` 那样查一个专属前缀——
/// 占位符**没有前缀**，它整体就是一句本地化文案。预筛的唯一职责是让
/// 「完全不含占位符的模板」零成本跳过。
fn may_contain_placeholder(text: &str) -> bool {
    text.contains("{{")
}

/// U201-C：把 prompt 里的「默认提示词占位符」展开成 `prompt_list.json` 里的全文。
///
/// **没有占位符就原样返回**，所以不用这条语法的模板完全不受影响。
///
/// # 为什么这一步必须排在最前
///
/// 写作节点的 prompt 处理是三步流水线（见 `workflow/integration.rs`）：
///   1. 本函数：默认提示词占位符 → 全文
///   2. `expand_prompt_content_references`：`{{ref:...}}` → 正文
///   3. `render_prompt_template`：`{{input.x}}` / `{{本章大纲}}` 等变量 → 值
///
/// **本步的产物可能自带后两类占位符**——`agent_prompt.planner` 正文里就写着
/// `{{ref:文档ID#L起始-L结束}}`，而 `node_template.*` 系列写着 `{{本章大纲}}`。
/// 若本步排在第 2 步之后，它引入的引用就**没人展开了**，会一路撞到渲染器的
/// fail-loud 分支，症状是「节点报某个变量无法解析，而作者的模板里根本没写过
/// 那个变量名」——极难归因。第 2、3 步之间的既有顺序同理且不可动。
///
/// # 为什么不把它做成 `render_prompt_template` 里的一个变量分支
///
/// 那样它就落到第 3 步了，等于上面说的错序；而且渲染器**只扫模板、不扫代入值**
/// （`prompt_template.rs` 已就此留注），占位符展开出来的 `{{ref:...}}` 不会被
/// 再扫一遍，第 2 步已经走过去了。所以必须是独立的前置一步。
///
/// # 不递归
///
/// 展开产物里若又出现默认提示词占位符（只可能是有人把占位符文案写进了
/// `prompt_list.json`），本函数**不再展开它**，而是让下面的后置条件把它拦住。
/// 一次性展开的深度上限是 1，理由与 `MAX_TEMPLATE_RENDER_DEPTH` 同源：
/// 提示词资源之间互相引用没有正当用例，放开只会引入无限展开的可能。
pub fn expand_default_prompt_placeholders(
    prompt: &str,
    prompts: &PromptResources,
) -> CoreResult<String> {
    expand_default_prompt_placeholders_with_table(prompt, prompts, placeholder_table()?)
}

/// 可注入接受集合的实现，供测试构造受控语言包。
pub fn expand_default_prompt_placeholders_with_table(
    prompt: &str,
    prompts: &PromptResources,
    table: &DefaultPromptPlaceholderTable,
) -> CoreResult<String> {
    if !may_contain_placeholder(prompt) {
        return Ok(prompt.to_owned());
    }

    let mut output = String::with_capacity(prompt.len());
    let mut rest = prompt;
    let mut expanded_any = false;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            // 未闭合：不是本步的职责（渲染器会给出更完整的报错），原样收尾。
            output.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let body = &after_open[..end];
        match table.resolve(body)? {
            Some(agent) => {
                let key = agent.prompt_key();
                let resource = prompts.get(key).ok_or_else(|| {
                    CoreError::validation(format!(
                        "default prompt placeholder `{{{{{body}}}}}` resolves to agent {} but \
                         prompt resource `{key}` is missing",
                        agent.node_type()
                    ))
                })?;
                if resource.prompt.trim().is_empty() {
                    return Err(CoreError::validation(format!(
                        "default prompt placeholder `{{{{{body}}}}}` resolves to empty prompt \
                         resource `{key}`; refusing to hand an empty role definition to a model"
                    )));
                }
                output.push_str(&resource.prompt);
                expanded_any = true;
            }
            // 不是默认提示词占位符（`{{ref:...}}` / `{{本章大纲}}` / `{{var.x}}`）：
            // 原样留给后两步。这里**不能**报未知占位符——本步只认自己那一类。
            None => {
                output.push_str("{{");
                output.push_str(body);
                output.push_str("}}");
            }
        }
        rest = &after_open[end + 2..];
    }
    output.push_str(rest);

    // 后置条件：展开过之后不能再有本类占位符残留。
    //
    // 照 `expand_prompt_content_references` 的写法（U201-C 与 13-B 同一形态的
    // 风险）：上面有若干「不替换」的分支，哪天有人新加一条忘了替换，占位符就会
    // 一路进请求体——模型会收到「{{outliner 默认提示词}}」这行字面量当作角色设定，
    // 而节点照样报成功。在这里 fail-loud 比在生产里静默跑完好得多。
    //
    // 只在**展开过**的情况下检查：没展开过说明本模板与本步无关（例如纯
    // `{{ref:...}}` 模板），扫它没有意义，也会把第 2、3 步的正常占位符误伤。
    if expanded_any {
        if let Some(residual) = find_placeholder_literal(&output, table)? {
            return Err(CoreError::validation(format!(
                "prompt still contains default prompt placeholder `{{{{{residual}}}}}` after \
                 expansion; refusing to hand an unexpanded placeholder to a model"
            )));
        }
    }

    Ok(output)
}

/// 扫描文本里第一个仍可被识别为默认提示词占位符的写法。
fn find_placeholder_literal(
    text: &str,
    table: &DefaultPromptPlaceholderTable,
) -> CoreResult<Option<String>> {
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            break;
        };
        let body = &after_open[..end];
        if table.resolve(body)?.is_some() {
            return Ok(Some(body.to_owned()));
        }
        rest = &after_open[end + 2..];
    }
    Ok(None)
}
