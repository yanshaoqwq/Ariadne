//! 13-B：Planner 在章节大纲里写下的正文引用 `{{ref:...}}`，及其展开器。
//!
//! # 这个模块解决什么
//!
//! Writer 默认只拿到各级总结与本章大纲，而**总结是有损压缩**：接续文风、还原
//! 伏笔细节、保持人物口吻，这些都需要看到前文**原文**。由 Planner 判断「哪些
//! 原文值得给 Writer 看」，用引用语法写进大纲；大纲文件因此保持轻量（存坐标不
//! 存拷贝，与项目的引用式数据流原则一致），只在装配 Writer 上下文时才展开。
//!
//! # 三条不可违背的设计约束
//!
//! **① 引用是内联锚定的。** 引用为大纲中**某一条具体指示**服务：
//!
//! ```text
//! 3. 回收第三章埋下的玉佩伏笔。原文在此：
//!    {{ref:chapter-03.md#L120-L145}}
//!    要呼应当时「她没有回头」的处理，这次让她主动提起，但语气要克制。
//! ```
//!
//! 所以展开一律**就地替换**（`expand_content_references` 逐段拷贝原文、只把占位
//! 符那一小段换掉），绝不把原文抽成末尾的「附录参考资料」——抽离之后 Writer
//! 无法知道这段原文是为哪条要求服务的，设计意图当场失效。
//!
//! **② 展开策略的判据是「消费方是不是 AI」，不是「在哪个界面」。** 任何送进 LLM
//! 请求体的路径都必须展开；只有渲染给人眼的路径才折叠。这是**安全边界**而非体验
//! 偏好：`ApprovalPolicy::should_auto_approve` 决定是否走自动审批，一旦走自动路
//! 径，审计 LLM 拿到的 payload 必须是展开后的实体内容，否则 Auto Mode 会在「没
//! 真正读到内容」的前提下批准变更。
//!
//! **③ 上游只产出展开态，折叠是纯前端的显示变换。** 展开发生在构造确认项 payload
//! 的**上游**（`rag/context.rs` 的上下文装配器），而不是渲染层各自处理。
//! `ExpandedOutline` 同时给出 `text`（已就地展开，供全部 AI 路径）与
//! `expanded`（结构化列表，供人类 UI 折叠渲染），因此不存在「某条 AI 路径忘记
//! 展开」的可能。
//!
//! **反模式（必须避免）**：在人类 UI 层做折叠、在 AI 层做展开——两套逻辑分散，
//! 早晚有一条 AI 路径漏掉展开。
//!
//! # 为什么展开器不自己碰文件系统
//!
//! `document_id → 正文` 的解析要项目根、路径沙箱与章节索引，这些都在 `documents`
//! 与 `commands` 层。展开器只声明它需要的能力（`ReferenceDocumentSource`），由
//! 调用方注入。好处有两个：
//! 1. 沙箱责任留在**已经**持有 `ensure_path_under_root` 与项目根的那一层，不在
//!    `rag` 里复制一份半成品路径校验；
//! 2. 展开逻辑（含 UTF-8 边界、护栏、失效处理）可以在不建临时项目的前提下被完整
//!    测试——不必为了测「行号越界怎么办」去起一个文件系统。
//!
//! 但**词法层的越权拦截留在这里**（`validate_document_id_scope`）：`..` 逃逸与绝
//! 对路径在进入 source 之前就被拒，不依赖调用方记得校验。这是纵深防御，不是
//! 重复劳动。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::contracts::{content_version_for_bytes, CoreError, CoreResult, SourceSpan, TextRange};
use crate::rag::line_patch::line_range_to_text_range;

/// 引用占位符的起始记号。
///
/// 刻意复用提示词模板的 `{{ }}` 外形（13B 方案 A）：编辑器的占位符高亮 / 悬停 /
/// Ctrl+左键展开（U115/U150）一套 UI 能力就能同时服务提示词变量与正文引用。
pub const CONTENT_REFERENCE_OPEN: &str = "{{ref:";

/// 引用占位符的结束记号。
pub const CONTENT_REFERENCE_CLOSE: &str = "}}";

/// 展开后包裹原文的起始标记前缀。
///
/// 用方括号而非【】、用「提供的正文参考」而非「引用正文 · 文件路径 行号」：
/// 对 LLM 来说**章节号与标题**比文件路径行号更有语义，也更省 token。
const EXPANSION_OPEN_PREFIX: &str = "[提供的正文参考：";

/// 展开后包裹原文的结束标记。
const EXPANSION_CLOSE: &str = "[正文参考结束]";

/// 引用的定位方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReferenceLocator {
    /// `#L120-L145`：1-based 闭区间行号。推荐 LLM 使用——行号直观、不易编错，
    /// 且 `find` 结果里同时给了行号与 byte 坐标。
    Lines { start: u64, end: u64 },
    /// `@1024-2048`：UTF-8 byte 半开区间。留给工具自动生成（例如将来做
    /// 「编辑器里选中一段 → 生成引用」），不指望模型手写。
    Bytes(TextRange),
    /// 无定位段：整篇。必须受长度护栏约束，否则百万字正文会灌爆上下文。
    Whole,
}

/// 一条正文引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentReference {
    pub document_id: String,
    pub locator: ReferenceLocator,
    /// `@v=<content_hash>` 锚定的内容版本。
    ///
    /// 正文被改动后行号会失效。带上版本，展开时就能判断「这条引用是照着哪一版
    /// 正文数出来的」，从而**发出可诊断警告而非静默展开错误内容**。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// 扫描到的一个引用占位符。
///
/// 比提案给的 `(TextRange, ContentReference)` 多两个字段，理由是**语法非法的引用
/// 不能被静默丢掉**：丢掉之后 `{{ref:...}}` 字面量会留在大纲里一路进到 LLM 请求
/// 体，正是本模块要防的事。所以坏语法也要占一个位置，并带上错误说明供警告使用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceOccurrence {
    /// 占位符在**被扫描文本**中的 byte 半开区间（含 `{{ref:` 与 `}}`）。
    pub placeholder: TextRange,
    /// 占位符原文，用于诊断与「原样回填」。
    pub raw: String,
    /// 解析结果；`None` 表示语法非法，原因见 `parse_error`。
    pub reference: Option<ContentReference>,
    /// 语法非法时的可读原因。
    pub parse_error: Option<String>,
}

/// 展开器的长度与条数护栏。
///
/// 默认值与既有口径对齐：单次总量取 `frontend/service.rs` 的
/// `MAX_PROJECT_REFERENCE_TEXT_CHARS`（32K 字符），项目问答的正文引用一直用这个
/// 上限，两处用同一个数字才不会出现「同一本小说在两个入口能塞进去的量不一样」。
///
/// 单位是**字符**而非 byte：正文是中文，按 byte 限长等于对中文项目砍到三分之一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceExpansionLimits {
    /// 单条引用最大字符数，防止模型引整章。
    pub max_chars_per_reference: usize,
    /// 单次展开总字符数。
    pub max_total_chars: usize,
    /// 引用条数上限，防止碎片化滥用。
    pub max_references: usize,
}

impl Default for ReferenceExpansionLimits {
    /// 默认护栏：单条 8K 字符 / 总量 32K 字符 / 至多 20 条。
    fn default() -> Self {
        Self {
            max_chars_per_reference: 8 * 1024,
            max_total_chars: 32 * 1024,
            max_references: 20,
        }
    }
}

/// 展开器能取到的一篇文档。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceDocument {
    /// 文档全文。
    pub text: String,
    /// 当前内容版本；与引用里的 `@v=` 比对以判断引用是否过时。
    ///
    /// 调用方不提供时（`None`）视为「无法判断」，此时**不**发过时警告——
    /// 无从判断却报警只会训练用户忽略警告。
    pub version: Option<String>,
    /// 人类可读的章节标签，例如「第三章 雨夜」。
    ///
    /// 权威来源是章节索引 `ChapterDocumentEntry.title`（`documents/models.rs`），
    /// 那是作品页与导出都以之为准的标题。取不到时回落为 `document_id`。
    pub label: Option<String>,
}

impl ReferenceDocument {
    /// 用文档正文构造引用文档，版本按项目统一口径自行计算。
    ///
    /// 版本用 `content_version_for_bytes`（FNV-1a，`contracts/content.rs`）而不是
    /// `commands.rs` 的 `content_revision_hash`（SHA-256）：`SourceSpan.version`
    /// 与 `FileDocumentService` 的文档版本用的都是前者，引用锚定必须跟它们同口径，
    /// 否则同一份正文会算出两个互不可比的版本，过时判断永远为真。
    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let version = content_version_for_bytes(text.as_bytes());
        Self {
            text,
            version: Some(version),
            label: None,
        }
    }

    /// 挂上人类可读章节标签。
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.label = (!label.trim().is_empty()).then_some(label);
        self
    }
}

/// 展开器所需的文档访问能力。
///
/// 之所以是「按 id 惰性取」而不是「调用方先把要用的文档准备好」：在解析之前谁都
/// 不知道大纲引用了哪些文档，逼调用方两段式（先 parse 再 load 再 expand）等于把
/// 「记得展开」的责任摊给每个调用点，那正是约束 ③ 要消灭的风险。
pub trait ReferenceDocumentSource {
    /// 按 `document_id` 取文档；不存在返回 `Ok(None)`。
    ///
    /// **返回 `Err` 与返回 `Ok(None)` 语义不同**：前者是「取的过程出错了」
    /// （权限、IO），会让整次展开 fail-loud；后者是「这篇文档没有」，展开器把它
    /// 记成引用失效并继续处理其余引用。把权限失败降级成 `None` 会让越权引用看
    /// 起来只是「文档不存在」，掩盖真实的安全事件。
    fn load_reference_document(&self, document_id: &str) -> CoreResult<Option<ReferenceDocument>>;
}

/// 内存文档表形式的引用来源。
///
/// 生产用途：调用方（工作流装配层）已经从章节索引拿到候选章节的正文与标题时，
/// 直接建这张表注入，不必再往下传一个文件系统句柄。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InMemoryReferenceDocuments {
    documents: BTreeMap<String, ReferenceDocument>,
}

impl InMemoryReferenceDocuments {
    /// 建空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一篇文档。
    pub fn insert(
        &mut self,
        document_id: impl Into<String>,
        document: ReferenceDocument,
    ) -> &mut Self {
        self.documents.insert(document_id.into(), document);
        self
    }

    /// 链式登记一篇文档。
    pub fn with_document(
        mut self,
        document_id: impl Into<String>,
        document: ReferenceDocument,
    ) -> Self {
        self.insert(document_id, document);
        self
    }
}

impl ReferenceDocumentSource for InMemoryReferenceDocuments {
    /// 从内存表按 id 取文档。
    fn load_reference_document(&self, document_id: &str) -> CoreResult<Option<ReferenceDocument>> {
        Ok(self.documents.get(document_id).cloned())
    }
}

/// 展开过程中产生的警告类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceWarningKind {
    /// 占位符语法非法，无法解析。
    MalformedSyntax,
    /// `document_id` 试图越出项目范围（`..` 逃逸或绝对路径）。
    DocumentOutOfScope,
    /// 文档不存在。
    DocumentMissing,
    /// 行号或 byte 区间越界，已截断到文档末尾。
    LocatorOutOfRange,
    /// 引用锚定的版本与当前文档不符，引用可能已过时。
    VersionMismatch,
    /// 命中长度护栏，正文被截断。
    Truncated,
    /// 命中条数护栏，该引用未展开。
    ReferenceLimitExceeded,
    /// 被引用正文里还含引用，按「禁止递归」不再展开。
    NestedReferenceNotExpanded,
}

/// 一条展开警告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceWarning {
    pub kind: ReferenceWarningKind,
    /// 相关文档 id；语法非法时为占位符原文。
    pub document_id: String,
    /// 占位符在原文中的位置，供编辑器标红定位。
    pub placeholder: TextRange,
    /// 可读原因。**必须**足以让人判断该改哪里，不能只写「引用失效」。
    pub detail: String,
}

/// 一条已展开的引用，供人类 UI 折叠渲染与审计溯源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedReference {
    /// 占位符在**原始**大纲中的位置；大纲编辑器按它定位折叠锚点。
    pub placeholder_range: TextRange,
    /// 展开块在**展开后文本**中的位置。
    ///
    /// 与 `placeholder_range` 都要给，因为两个消费方坐标系不同：编辑器持有大纲
    /// 原文（用前者），而确认项 payload 里存的是展开态（用后者）。只给前者的话，
    /// 审阅界面拿到 payload 后没法把那段原文折起来。
    pub expanded_range: TextRange,
    /// 引用坐标；`sources`/审计据此溯源「Writer 看到的这段来自哪里」。
    pub span: SourceSpan,
    /// 展开标记里用的人类可读标签，例如「第三章 雨夜」。
    pub chapter_label: String,
    /// 实际展开出的原文（已过护栏截断）。
    pub text: String,
}

/// 展开结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedOutline {
    /// 就地展开后的完整文本。**全部 AI 路径都用这一份。**
    pub text: String,
    /// 结构化展开结果，供人类 UI 折叠与审计。
    #[serde(default)]
    pub expanded: Vec<ExpandedReference>,
    /// 失效 / 截断 / 越权警告。绝不静默置空——与 `render_prompt_template`
    /// 「未知变量报错而非替换成空串」保持同一态度。
    #[serde(default)]
    pub warnings: Vec<ReferenceWarning>,
}

impl ExpandedOutline {
    /// 展开出的全部引用坐标，直接灌进 `WritingContextSection.sources`。
    pub fn source_spans(&self) -> Vec<SourceSpan> {
        self.expanded
            .iter()
            .map(|reference| reference.span.clone())
            .collect()
    }
}

/// 文本里是否含正文引用占位符。
///
/// AI 路径的兜底哨兵：装配层在产出上下文前用它确认「没有任何 `{{ref:` 残留」。
/// 之所以需要单独一个便宜的检查而不是「相信展开器」——提示词模板渲染器
/// (`render_prompt_template`) 只扫描**模板**里的 `{{}}`，代入的**值**不会被再
/// 扫一遍。也就是说未展开的引用藏在大纲值里时，渲染器一声不响就把它送进请求体。
pub fn contains_content_reference(text: &str) -> bool {
    text.contains(CONTENT_REFERENCE_OPEN)
}

/// 扫描文本里的全部引用占位符，按出现顺序返回。
///
/// 位置信息是 byte 半开区间，且一定落在 UTF-8 字符边界上：`str::find` 只会返回
/// 字符边界处的偏移，而记号 `{{ref:` / `}}` 全是 ASCII。
pub fn parse_content_references(text: &str) -> Vec<ReferenceOccurrence> {
    let mut occurrences = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative_open) = text[cursor..].find(CONTENT_REFERENCE_OPEN) {
        let open = cursor + relative_open;
        let body_start = open + CONTENT_REFERENCE_OPEN.len();
        let Some(relative_close) = text[body_start..].find(CONTENT_REFERENCE_CLOSE) else {
            // 没有闭合记号：后面不可能再有完整占位符，收尾。
            // 不把「未闭合」记成 occurrence——它没有确定的结束位置，无法就地替换；
            // 留在文本里由装配层的哨兵拦住，报错信息比这里能给的更完整。
            break;
        };
        let close = body_start + relative_close;
        let placeholder_end = close + CONTENT_REFERENCE_CLOSE.len();
        let raw = text[open..placeholder_end].to_owned();
        let placeholder = TextRange {
            start: open as u64,
            end: placeholder_end as u64,
        };
        let occurrence = match parse_reference_body(&text[body_start..close]) {
            Ok(reference) => ReferenceOccurrence {
                placeholder,
                raw,
                reference: Some(reference),
                parse_error: None,
            },
            Err(message) => ReferenceOccurrence {
                placeholder,
                raw,
                reference: None,
                parse_error: Some(message),
            },
        };
        occurrences.push(occurrence);
        cursor = placeholder_end;
    }
    occurrences
}

/// 解析占位符内部（`{{ref:` 与 `}}` 之间）的定位串。
///
/// 返回 `Result<_, String>` 而非 `CoreResult`：这里的失败是**单条引用**的语法
/// 问题，会变成一条警告继续往下走，不该让整次展开中止。用 `CoreError` 会诱使
/// 调用方 `?` 掉，从而把「一条引用写坏」升级成「整章大纲装配失败」。
fn parse_reference_body(body: &str) -> Result<ContentReference, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("引用为空：应写作 {{ref:文档ID#L起始-L结束}}".to_owned());
    }

    // 先摘版本锚定 `@v=...`；它一定在最后，且与 byte 定位的 `@` 靠 `v=` 前缀区分。
    let (locator_part, version) = match body.rfind("@v=") {
        Some(index) => {
            let version = body[index + 3..].trim();
            if version.is_empty() {
                return Err("版本锚定 @v= 后面没有内容".to_owned());
            }
            (body[..index].trim(), Some(version.to_owned()))
        }
        None => (body, None),
    };

    if locator_part.is_empty() {
        return Err("引用缺少文档 ID".to_owned());
    }

    // 行号定位 `#L120-L145`
    if let Some((document_id, lines)) = locator_part.split_once('#') {
        let locator = parse_line_locator(lines)?;
        return finish_reference(document_id, locator, version);
    }
    // byte 定位 `@1024-2048`
    if let Some((document_id, bytes)) = locator_part.split_once('@') {
        let locator = parse_byte_locator(bytes)?;
        return finish_reference(document_id, locator, version);
    }
    finish_reference(locator_part, ReferenceLocator::Whole, version)
}

/// 组装引用并校验 document_id 非空。
fn finish_reference(
    document_id: &str,
    locator: ReferenceLocator,
    version: Option<String>,
) -> Result<ContentReference, String> {
    let document_id = document_id.trim();
    if document_id.is_empty() {
        return Err("引用缺少文档 ID".to_owned());
    }
    Ok(ContentReference {
        document_id: document_id.to_owned(),
        locator,
        version,
    })
}

/// 解析 `L120-L145` 形式的行号区间；单行写作 `L120` 也接受。
fn parse_line_locator(raw: &str) -> Result<ReferenceLocator, String> {
    let raw = raw.trim();
    let parse_line = |value: &str| -> Result<u64, String> {
        let digits = value
            .trim()
            .strip_prefix('L')
            .or_else(|| value.trim().strip_prefix('l'))
            .unwrap_or(value.trim());
        digits
            .parse::<u64>()
            .map_err(|_| format!("行号无法解析：{value}"))
    };
    let (start, end) = match raw.split_once('-') {
        Some((start, end)) => (parse_line(start)?, parse_line(end)?),
        // 单行引用：`#L120` 等价于 `#L120-L120`。允许它是因为模型经常这么写，
        // 拒绝只会换来一条无谓的失效警告。
        None => {
            let line = parse_line(raw)?;
            (line, line)
        }
    };
    if start == 0 || end == 0 {
        return Err("行号是 1-based，不能为 0".to_owned());
    }
    if start > end {
        return Err(format!("行号区间起点 {start} 大于终点 {end}"));
    }
    Ok(ReferenceLocator::Lines { start, end })
}

/// 解析 `1024-2048` 形式的 byte 半开区间。
fn parse_byte_locator(raw: &str) -> Result<ReferenceLocator, String> {
    let raw = raw.trim();
    let Some((start, end)) = raw.split_once('-') else {
        return Err(format!("byte 区间应写作 起始-结束，实际是：{raw}"));
    };
    let start = start
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("byte 起点无法解析：{start}"))?;
    let end = end
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("byte 终点无法解析：{end}"))?;
    let range = TextRange::new(start, end).map_err(|_| format!("byte 区间非法：{start}-{end}"))?;
    Ok(ReferenceLocator::Bytes(range))
}

/// 词法层的作用域校验：拒绝 `..` 逃逸与绝对路径。
///
/// 真正的文件系统沙箱是 `ReferenceDocumentSource` 实现者的责任
/// （那一层才持有项目根，才能调 `ensure_path_under_root`）。这里做的是**纵深
/// 防御**：让 `{{ref:../../etc/passwd}}` 在触及 source 之前就被拒掉，不依赖每个
/// source 实现都记得校验。
fn validate_document_id_scope(document_id: &str) -> Result<(), String> {
    if document_id.starts_with('/') || document_id.starts_with('\\') {
        return Err(format!("引用不能使用绝对路径：{document_id}"));
    }
    // Windows 盘符形式 `C:\...` 同样是绝对路径。
    if document_id.len() >= 2 && document_id.as_bytes()[1] == b':' {
        return Err(format!("引用不能使用绝对路径：{document_id}"));
    }
    for part in document_id.split(['/', '\\']) {
        if part == ".." {
            return Err(format!("引用不能跳出项目目录：{document_id}"));
        }
    }
    Ok(())
}

/// 把引用的定位段换算成 `SourceSpan`（UTF-8 byte 半开区间）。
///
/// 行号 → byte 一律走 `line_patch::line_range_to_text_range`（内部
/// `split_inclusive('\n')`）。**不要**在这里新写字节遍历找 `\n`：正文是中文，
/// 按 byte 找换行会切在多字节字符中间，`TextRange` 落到非字符边界，后续 patch
/// 直接 panic 或写出乱码。
///
/// 越界不报错而是**截断到文档末尾**并由调用方记警告：正文改短之后旧引用越界是
/// 常态，为此让整章装配失败太脆；但截断必须留痕，否则「引到的内容悄悄少了一
/// 半」无从察觉。返回值第二项即「是否发生了截断」。
pub fn to_source_span(
    reference: &ContentReference,
    document_text: &str,
) -> CoreResult<(SourceSpan, bool)> {
    let (range, clamped) = match &reference.locator {
        ReferenceLocator::Lines { start, end } => {
            let line_count = line_count_of(document_text);
            if line_count == 0 {
                // 空文档：任何 1-based 行号都无从对应，落到空区间并记截断。
                (TextRange { start: 0, end: 0 }, true)
            } else {
                let clamped_start = (*start).min(line_count);
                let clamped_end = (*end).min(line_count);
                let clamped = clamped_start != *start || clamped_end != *end;
                let range = line_range_to_text_range(document_text, clamped_start, clamped_end)?;
                (range, clamped)
            }
        }
        ReferenceLocator::Bytes(range) => {
            let document_len = document_text.len() as u64;
            let start = range.start.min(document_len);
            let end = range.end.min(document_len);
            let clamped = start != range.start || end != range.end;
            // byte 定位是外部输入，必须显式确认落在字符边界上——UTF-8 安全不能
            // 指望「工具生成的坐标一定对」。
            let start_usize = usize::try_from(start)
                .map_err(|_| CoreError::validation("reference byte start exceeds usize range"))?;
            let end_usize = usize::try_from(end)
                .map_err(|_| CoreError::validation("reference byte end exceeds usize range"))?;
            if !document_text.is_char_boundary(start_usize)
                || !document_text.is_char_boundary(end_usize)
            {
                return Err(CoreError::validation(format!(
                    "reference byte range {}-{} is not aligned to UTF-8 character boundaries in {}",
                    range.start, range.end, reference.document_id
                )));
            }
            (TextRange::new(start, end)?, clamped)
        }
        ReferenceLocator::Whole => (TextRange::new(0, document_text.len() as u64)?, false),
    };

    Ok((
        SourceSpan {
            document_id: reference.document_id.clone(),
            range,
            version: reference.version.clone(),
        },
        clamped,
    ))
}

/// 把 `SourceSpan` 渲染成可直接抄进大纲的 `{{ref:...}}` 字面量。
///
/// 用 byte 形态而非行号形态：`SourceSpan` 里存的就是 byte 区间，换算成行号需要
/// 文档全文，而 `find` 的调用现场未必拿着那篇文档（引用天生跨文档）。byte 形态
/// 正是提案为「工具自动生成」保留的那一种。
///
/// 这个函数存在的意义是让 Planner 提示词里「结果里的出处坐标可以直接照抄」成为
/// **事实**：模型不必自己拼 document_id、不必数行号，抄一个字符串即可。凭空构造
/// 坐标是 LLM 在这个语法上最容易出错的地方。
pub fn reference_syntax_for_span(span: &SourceSpan) -> String {
    let version = span
        .version
        .as_deref()
        .map(|value| format!("@v={value}"))
        .unwrap_or_default();
    format!(
        "{CONTENT_REFERENCE_OPEN}{}@{}-{}{version}{CONTENT_REFERENCE_CLOSE}",
        span.document_id, span.range.start, span.range.end
    )
}

/// 按 `SourceSpan` 从文档正文取出被引用的片段。
///
/// UTF-8 边界检查的**唯一实现**：`tools.rs` 的 `attach_document_text`（find 的
/// `include_text` 回填）也调这里。两处各写一遍的话，早晚有一处忘了检查边界，
/// 而那一处就会把 range 落在多字节字符中间的片段当正文交出去。
pub fn text_for_source_span<'a>(document_text: &'a str, span: &SourceSpan) -> CoreResult<&'a str> {
    let start = usize::try_from(span.range.start)
        .map_err(|_| CoreError::validation("source span start exceeds usize range"))?;
    let end = usize::try_from(span.range.end)
        .map_err(|_| CoreError::validation("source span end exceeds usize range"))?;
    document_text.get(start..end).ok_or_else(|| {
        CoreError::validation(format!(
            "source span {start}..{end} is not aligned to UTF-8 character boundaries in {}",
            span.document_id
        ))
    })
}

/// 就地展开文本里的全部 `{{ref:...}}`。
///
/// 「就地」是本函数的核心性质，也是最容易被后人「优化」掉的性质：输出按
/// `原文[游标..占位符起点] + 展开块 + …` 逐段拼接，因此引用**前后**的大纲指示
/// 文字在展开后仍与原文相邻。若改成「把原文收集起来附在末尾」，Writer 就无法
/// 知道某段原文是为哪条要求服务的——见模块头约束 ①。
pub fn expand_content_references(
    text: &str,
    documents: &dyn ReferenceDocumentSource,
    limits: &ReferenceExpansionLimits,
) -> CoreResult<ExpandedOutline> {
    let occurrences = parse_content_references(text);
    if occurrences.is_empty() {
        return Ok(ExpandedOutline {
            text: text.to_owned(),
            expanded: Vec::new(),
            warnings: Vec::new(),
        });
    }

    let mut output = String::with_capacity(text.len());
    let mut expanded = Vec::new();
    let mut warnings = Vec::new();
    let mut cursor = 0usize;
    let mut total_chars_used = 0usize;
    let mut expanded_count = 0usize;

    for occurrence in &occurrences {
        let start = usize::try_from(occurrence.placeholder.start)
            .map_err(|_| CoreError::validation("reference placeholder start exceeds usize"))?;
        let end = usize::try_from(occurrence.placeholder.end)
            .map_err(|_| CoreError::validation("reference placeholder end exceeds usize"))?;
        // 占位符之前的大纲原文照抄——这一句就是「就地」。
        output.push_str(
            text.get(cursor..start)
                .ok_or_else(|| CoreError::validation("reference placeholder range is invalid"))?,
        );
        cursor = end;

        let replacement = match expand_one(
            occurrence,
            documents,
            limits,
            &mut total_chars_used,
            &mut expanded_count,
            &mut warnings,
        )? {
            ExpansionOutcome::Block { label, body, span } => {
                let block = render_expansion_block(&label, &body);
                let block_start = output.len() as u64;
                let block_end = block_start + block.len() as u64;
                expanded.push(ExpandedReference {
                    placeholder_range: occurrence.placeholder,
                    expanded_range: TextRange::new(block_start, block_end)?,
                    span,
                    chapter_label: label,
                    text: body,
                });
                block
            }
            ExpansionOutcome::Marker(marker) => marker,
        };
        output.push_str(&replacement);
    }

    output.push_str(
        text.get(cursor..)
            .ok_or_else(|| CoreError::validation("reference placeholder range is invalid"))?,
    );

    Ok(ExpandedOutline {
        text: output,
        expanded,
        warnings,
    })
}

/// 单个占位符的替换结果。
enum ExpansionOutcome {
    /// 成功取到正文，替换为完整参考块。
    Block {
        label: String,
        body: String,
        span: SourceSpan,
    },
    /// 无法展开，替换为可诊断的失效标记。
    Marker(String),
}

/// 展开单个占位符。
///
/// **绝不静默置空**：每条失败路径都同时产出「用户可见的失效标记」与「结构化
/// 警告」。静默置空会让 Writer 以为那条指示后面本来就没有原文，而人类审阅时
/// 也看不出少了什么。
fn expand_one(
    occurrence: &ReferenceOccurrence,
    documents: &dyn ReferenceDocumentSource,
    limits: &ReferenceExpansionLimits,
    total_chars_used: &mut usize,
    expanded_count: &mut usize,
    warnings: &mut Vec<ReferenceWarning>,
) -> CoreResult<ExpansionOutcome> {
    let Some(reference) = &occurrence.reference else {
        let detail = occurrence
            .parse_error
            .clone()
            .unwrap_or_else(|| "引用语法非法".to_owned());
        warnings.push(ReferenceWarning {
            kind: ReferenceWarningKind::MalformedSyntax,
            document_id: occurrence.raw.clone(),
            placeholder: occurrence.placeholder,
            detail: detail.clone(),
        });
        return Ok(ExpansionOutcome::Marker(format!("[引用失效：{detail}]")));
    };

    if let Err(detail) = validate_document_id_scope(&reference.document_id) {
        warnings.push(ReferenceWarning {
            kind: ReferenceWarningKind::DocumentOutOfScope,
            document_id: reference.document_id.clone(),
            placeholder: occurrence.placeholder,
            detail: detail.clone(),
        });
        return Ok(ExpansionOutcome::Marker(format!("[引用失效：{detail}]")));
    }

    if *expanded_count >= limits.max_references {
        let detail = format!("单次展开最多 {} 条引用，其余未展开", limits.max_references);
        warnings.push(ReferenceWarning {
            kind: ReferenceWarningKind::ReferenceLimitExceeded,
            document_id: reference.document_id.clone(),
            placeholder: occurrence.placeholder,
            detail: detail.clone(),
        });
        return Ok(ExpansionOutcome::Marker(format!("[引用未展开：{detail}]")));
    }

    let Some(document) = documents.load_reference_document(&reference.document_id)? else {
        let detail = format!("文档不存在 {}", reference.document_id);
        warnings.push(ReferenceWarning {
            kind: ReferenceWarningKind::DocumentMissing,
            document_id: reference.document_id.clone(),
            placeholder: occurrence.placeholder,
            detail: detail.clone(),
        });
        return Ok(ExpansionOutcome::Marker(format!("[引用失效：{detail}]")));
    };

    let label = document
        .label
        .clone()
        .unwrap_or_else(|| reference.document_id.clone());

    let (span, clamped) = to_source_span(reference, &document.text)?;
    if clamped {
        warnings.push(ReferenceWarning {
            kind: ReferenceWarningKind::LocatorOutOfRange,
            document_id: reference.document_id.clone(),
            placeholder: occurrence.placeholder,
            detail: format!(
                "引用位置越出 {} 的当前范围，已截断到文档末尾",
                reference.document_id
            ),
        });
    }

    // 版本不匹配仍按当前行号展开，但要标注可能过时——正文改动后行号漂移是常态，
    // 拒绝展开会让 Writer 一段原文都看不到，比「看到可能过时的原文」更糟。
    // 只有两边**都**有版本时才比较：无从判断却报警只会训练用户忽略警告。
    let mut stale = false;
    if let (Some(anchored), Some(current)) = (&reference.version, &document.version) {
        if anchored != current {
            stale = true;
            warnings.push(ReferenceWarning {
                kind: ReferenceWarningKind::VersionMismatch,
                document_id: reference.document_id.clone(),
                placeholder: occurrence.placeholder,
                detail: format!(
                    "引用锚定版本 {anchored} 与 {} 的当前版本 {current} 不一致，行号可能已漂移",
                    reference.document_id
                ),
            });
        }
    }

    let raw_body = text_for_source_span(&document.text, &span)?;

    // 长度护栏：单条上限与剩余总量取小者。按**字符**计数，byte 口径会把中文砍成
    // 三分之一。
    let remaining_total = limits.max_total_chars.saturating_sub(*total_chars_used);
    let budget = limits.max_chars_per_reference.min(remaining_total);
    let raw_char_count = raw_body.chars().count();
    let mut body = if raw_char_count > budget {
        let detail = format!("引用正文 {raw_char_count} 字符超出可用额度 {budget} 字符，已截断",);
        warnings.push(ReferenceWarning {
            kind: ReferenceWarningKind::Truncated,
            document_id: reference.document_id.clone(),
            placeholder: occurrence.placeholder,
            detail,
        });
        let mut truncated: String = raw_body.chars().take(budget).collect();
        truncated.push_str("\n[正文参考在此处被截断：超出长度上限]");
        truncated
    } else {
        raw_body.to_owned()
    };

    // 禁止递归展开：被引正文里若还有 `{{ref:}}`，替换成惰性标记而不是原样留下。
    // 原样留下会让占位符一路进到 LLM 请求体（约束 ②），也会让装配层的哨兵把
    // 「合法的嵌套」误判成「漏了展开」。
    if contains_content_reference(&body) {
        warnings.push(ReferenceWarning {
            kind: ReferenceWarningKind::NestedReferenceNotExpanded,
            document_id: reference.document_id.clone(),
            placeholder: occurrence.placeholder,
            detail: format!(
                "{} 的被引段落里含正文引用，按禁止递归不再展开",
                reference.document_id
            ),
        });
        body = neutralize_nested_references(&body);
    }

    *total_chars_used = total_chars_used.saturating_add(body.chars().count());
    *expanded_count = expanded_count.saturating_add(1);

    let label = if stale {
        format!("{label}（引用可能已过时）")
    } else {
        label
    };

    Ok(ExpansionOutcome::Block { label, body, span })
}

/// 把嵌套引用换成不可再解析的惰性标记。
fn neutralize_nested_references(body: &str) -> String {
    let occurrences = parse_content_references(body);
    let mut output = String::with_capacity(body.len());
    let mut cursor = 0usize;
    for occurrence in occurrences {
        let start = occurrence.placeholder.start as usize;
        let end = occurrence.placeholder.end as usize;
        if start < cursor || end > body.len() {
            continue;
        }
        output.push_str(&body[cursor..start]);
        output.push_str("[未展开的嵌套引用]");
        cursor = end;
    }
    output.push_str(&body[cursor..]);
    output
}

/// 渲染展开块。
///
/// 形态固定为三行结构，方括号 + 「提供的正文参考」：
/// ```text
/// [提供的正文参考：第三章 雨夜]
/// （原文）
/// [正文参考结束]
/// ```
fn render_expansion_block(label: &str, body: &str) -> String {
    format!("{EXPANSION_OPEN_PREFIX}{label}]\n{body}\n{EXPANSION_CLOSE}")
}

/// 文档行数。
///
/// 必须与 `line_patch::line_ranges` 完全同口径，否则「行号是否越界」的判断会和
/// 真正做换算的那个函数漂移：这里说没越界、那里 panic 于索引越界。那边用的是
/// `split_inclusive('\n')`（换行符归属该行，末尾无换行时最后一段仍算一行），
/// 所以这里也只能用它——不要换成 `lines().count()`，两者在「以 `\n` 结尾」时结果
/// 相同但在空文本上不同，而空文档正是分章写作最常见的起点。
fn line_count_of(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    text.split_inclusive('\n').count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三种定位形态都要能解析，且位置信息可用于就地替换。
    #[test]
    fn parses_line_byte_and_whole_locators() {
        let text = "甲 {{ref:chapter-03.md#L2-L3}} 乙 {{ref:a.md@0-6}} 丙 {{ref:b.md}}";
        let occurrences = parse_content_references(text);
        assert_eq!(occurrences.len(), 3);
        assert_eq!(
            occurrences[0].reference.as_ref().map(|r| r.locator.clone()),
            Some(ReferenceLocator::Lines { start: 2, end: 3 })
        );
        assert_eq!(
            occurrences[1].reference.as_ref().map(|r| r.locator.clone()),
            Some(ReferenceLocator::Bytes(TextRange { start: 0, end: 6 }))
        );
        assert_eq!(
            occurrences[2].reference.as_ref().map(|r| r.locator.clone()),
            Some(ReferenceLocator::Whole)
        );
        // 位置必须能切回原占位符，否则就地替换会错位。
        for occurrence in &occurrences {
            let start = occurrence.placeholder.start as usize;
            let end = occurrence.placeholder.end as usize;
            assert_eq!(&text[start..end], occurrence.raw);
        }
    }

    /// 版本锚定要与 byte 定位的 `@` 区分开。
    #[test]
    fn parses_version_anchor_without_confusing_byte_locator() {
        let occurrences = parse_content_references("{{ref:chapter-03.md#L1-L2@v=abc123}}");
        let reference = occurrences[0]
            .reference
            .as_ref()
            .expect("带版本锚定的行号引用必须能解析");
        assert_eq!(reference.document_id, "chapter-03.md");
        assert_eq!(reference.version.as_deref(), Some("abc123"));
        assert_eq!(
            reference.locator,
            ReferenceLocator::Lines { start: 1, end: 2 }
        );
    }

    /// 中文正文按行号引用，换算结果必须落在字符边界上。
    #[test]
    fn line_locator_to_span_stays_on_utf8_boundaries() {
        let document = "第一行甲\n第二行乙\n第三行丙";
        let reference = ContentReference {
            document_id: "chapter-03.md".to_owned(),
            locator: ReferenceLocator::Lines { start: 2, end: 2 },
            version: None,
        };
        let (span, clamped) = to_source_span(&reference, document).expect("行号换算必须成功");
        assert!(!clamped);
        let excerpt = text_for_source_span(document, &span).expect("片段必须可切出");
        assert_eq!(excerpt, "第二行乙\n");
    }

    /// 就地替换：引用前后的指示文字在展开后仍与原文相邻。
    #[test]
    fn expansion_keeps_surrounding_instructions_adjacent() {
        let documents = InMemoryReferenceDocuments::new().with_document(
            "chapter-03.md",
            ReferenceDocument::from_text("她没有回头。").with_label("第三章 雨夜"),
        );
        let outline = "回收玉佩伏笔。原文在此：\n{{ref:chapter-03.md}}\n语气要克制。";
        let result =
            expand_content_references(outline, &documents, &ReferenceExpansionLimits::default())
                .expect("展开必须成功");

        assert!(!contains_content_reference(&result.text));
        let anchor = result
            .text
            .find("[提供的正文参考：第三章 雨夜]")
            .expect("展开标记必须存在");
        let before = result.text.find("原文在此：").expect("前置指示必须保留");
        let after = result.text.find("语气要克制").expect("后置指示必须保留");
        assert!(
            before < anchor && anchor < after,
            "展开块必须夹在两条指示之间"
        );
    }
}
