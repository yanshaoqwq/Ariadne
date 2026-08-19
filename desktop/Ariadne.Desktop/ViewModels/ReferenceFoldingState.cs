using System.Collections.Generic;
using System.Linq;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// U150 / U201：提示词编辑器里 `{{ref:...}}` 的**预览状态**。
///
/// ⚠️ 类名里的 "Folding" 是**历史遗留**，已不再有折叠这回事（U201-A）。
/// 保留名字只因改名要连带重命名文件，而本轮可改文件是逐个议定的。
///
/// # U201-A：编辑器里**始终**是 `{{ref:...}}` 字面量，预览另开一层
///
/// 上一轮做成了「默认折叠成 `‹chapter-01.md L2-3›`，Ctrl+左键换回字面量」。
/// **两态做反了**，而且反得有两处独立的错：
/// 1. **默认态吃掉了可编辑性。** 屏幕上看不到 `{{ref:` 这几个字，作者就无从照抄
///    语法写第二条同类引用，想改行段还得先「展开」成源码 ⇒ 默认只读、编辑要先解锁。
///    而这是个**编辑器**。
/// 2. **「展开」名不副实。** 原始诉求是「在编辑器里预览它会展开成什么」，
///    而那一版 Ctrl+左键给的是占位符原文，不是**被引的正文** ⇒ 预览从未实现。
///
/// 现在：文本流永远是作者写的字面量（AvaloniaEdit 原生显示 + 三档着色），
/// Ctrl+左键在**另一层**（浮层）里显示被引正文。
///
/// # 两种展开语义不是同一个开关
///
/// 用户明确要求过：**「对 AI 展开，对人类还是做成可 Ctrl 左键点开收起」**。
/// - **对 AI**：`{{ref:...}}` 在送进请求体**之前**必须变成真正的原文
///   （`core/src/workflow/integration.rs` 的 `expand_prompt_content_references`）。
///   那是**功能正确性**要求——占位符字面量进请求体是安全缺口。
/// - **对人**：预览「它会展开成什么」，看一眼即可。
///
/// **不要把两者做成同一个开关**，也**不要让预览走后端那条展开器**：
/// 后端那条带 `[提供的正文参考：…]` 标记、有条数与总长上限、要做越权校验。
/// 编辑期预览受运行期上限与越权规则影响，会让作者看到「预览一片空白」
/// 却完全不知道是因为撞了条数上限。预览只需要「取到那段正文、显示出来」。
///
/// # 为什么状态存在这里而不是控件里
///
/// 预览状态要在「重新解析」之后存活：用户在别处敲一个字，整段文本重新扫描，
/// 但打开着的预览不该被打回关闭——否则每敲一个字浮层就闪一下。
/// 所以状态按**引用的身份**（document_id + 定位串）而不是按偏移量记：
/// 偏移量在每次编辑后都变，拿它做 key 等于每次编辑都丢状态。
///
/// # 为什么预览是「最多一条」而不是一个集合
///
/// 上一版用 `HashSet` 记「哪些展开了」，因为行内替换天然可以同时展开多条。
/// 浮层不是：屏幕上同时开两个预览浮层会互相遮挡，而作者一次只看一条。
/// 单值也让「Esc 关掉当前预览」有确定语义（集合语义下「当前」是哪条并不明确）。
/// </summary>
public sealed class ReferenceFoldingState
{
    /// <summary>当前打开预览的引用身份；没有打开时为 null。</summary>
    private string? _openIdentity;

    /// <summary>当前预览显示的正文。与 <see cref="_openIdentity"/> 同生同灭。</summary>
    private string? _openBody;

    /// <summary>
    /// 一条引用在编辑器里的呈现。
    /// </summary>
    /// <param name="Start">占位符在**当前文本**里的起始偏移（UTF-16）。</param>
    /// <param name="End">占位符结束偏移（半开）。</param>
    /// <param name="Identity">跨编辑稳定的身份，用作预览状态的 key。</param>
    /// <param name="IsPreviewOpen">true = 这条引用的预览浮层正开着。</param>
    /// <param name="IsValid">语法是否合法；语法非法的引用不可预览（没有正文可取）。</param>
    /// <param name="PreviewLabel">预览浮层标题显示的文字（文件名 + 行段）。</param>
    public sealed record Segment(
        int Start,
        int End,
        string Identity,
        bool IsPreviewOpen,
        bool IsValid,
        string PreviewLabel);

    /// <summary>当前预览的正文；没有打开预览时为 null。</summary>
    public string? OpenPreviewBody => _openBody;

    /// <summary>当前是否有预览打开。</summary>
    public bool IsAnyPreviewOpen => _openIdentity is not null;

    /// <summary>
    /// 跨编辑稳定的引用身份。
    ///
    /// **刻意不含偏移量**：在文本开头敲一个字，后面每条引用的偏移都变，
    /// 但它们还是「同一条引用」，预览状态必须跟着。
    ///
    /// 同一份文档的**不同行段**算不同引用（`#L2-L3` 与 `#L9-L9` 各自独立），
    /// 所以身份要含定位串。**版本锚定不含**：`@v=abc` 换成 `@v=def` 指的还是
    /// 同一段正文，只是作者更新了锚点，没有理由把预览关掉。
    /// </summary>
    public static string IdentityOf(ContentReferenceSyntax.Occurrence occurrence) =>
        occurrence.Locator switch
        {
            ContentReferenceSyntax.LocatorKind.Lines =>
                $"{occurrence.DocumentId}#L{occurrence.RangeStart}-L{occurrence.RangeEnd}",
            ContentReferenceSyntax.LocatorKind.Bytes =>
                $"{occurrence.DocumentId}@{occurrence.RangeStart}-{occurrence.RangeEnd}",
            _ => occurrence.DocumentId,
        };

    /// <summary>
    /// 按当前文本算出每条引用的呈现状态。
    ///
    /// 每次文本变化后重算。**不修改文本**——返回的是呈现指令，
    /// 文档内容始终是用户写的那份原始模板。
    ///
    /// ⚠️ U201-A 之后这个投影**不再驱动文本替换**（没有 ElementGenerator 了）。
    /// 它现在只回答两件事：「这条引用在哪」（供命中测试与用例断言）与
    /// 「它的预览是否正开着」（供浮层开合）。
    /// </summary>
    public IReadOnlyList<Segment> Project(string? text)
    {
        // 绝大多数提示词没有引用，先挡一次省掉整趟扫描。
        if (!ContentReferenceSyntax.ContainsReference(text))
        {
            return Array.Empty<Segment>();
        }

        return ContentReferenceSyntax.Parse(text)
            .Select(occurrence =>
            {
                var identity = IdentityOf(occurrence);
                return new Segment(
                    occurrence.Start,
                    occurrence.End,
                    identity,
                    // 语法非法的引用一律不开预览：它连 document_id 都没解析出来，
                    // 没有可取的正文。开一个空浮层只会让用户以为是自己点坏了。
                    IsPreviewOpen: occurrence.IsValid
                        && string.Equals(_openIdentity, identity, StringComparison.Ordinal),
                    occurrence.IsValid,
                    PreviewLabelFor(occurrence));
            })
            .ToList();
    }

    /// <summary>
    /// 记下「这条引用的预览已打开，正文是这段」。
    ///
    /// # U201-B：为什么正文由调用方传进来
    ///
    /// 取正文是**异步**的（要走一次 IPC），而 <see cref="Project"/> 是同步纯函数。
    /// 若让本类自己去取，`Project` 就得变成 async 或者藏一个「取到再通知」的回调，
    /// 两条都会把「呈现投影」这个纯函数搞成有生命周期的东西。
    ///
    /// 于是分工是：控件负责**先取到正文**，取到了才调这个方法。
    /// ⇒ **「有匹配才能预览」由这个签名本身保证**：没有正文根本没法开预览，
    /// 而不是靠调用方记得先判断一下（那种规则一定会漂移）。
    /// </summary>
    /// <returns>false = 拒绝开（语法非法或正文为 null）。</returns>
    public bool OpenPreview(ContentReferenceSyntax.Occurrence occurrence, string? body)
    {
        // 语法非法 ⇒ 无 document_id 可取；正文 null ⇒ 取不到（文档不存在/后端拒绝）。
        // 两种都不开。**空串是允许的**：被引的那几行确实可以是空行，
        // 那时显示一个空预览是**如实**的，与「取不到」不是同一件事。
        if (!occurrence.IsValid || body is null)
        {
            return false;
        }

        _openIdentity = IdentityOf(occurrence);
        _openBody = body;
        return true;
    }

    /// <summary>关掉当前预览。Esc、点别处、再次 Ctrl+左键同一条都走这里。</summary>
    public void ClosePreview()
    {
        _openIdentity = null;
        _openBody = null;
    }

    /// <summary>
    /// 这条引用的预览此刻是否正开着。
    ///
    /// 控件用它判断 Ctrl+左键该「开」还是「关」——同一条再点一次是收起。
    /// 收起走这条同步路径而不是先去取一次正文：那会为一个即将关掉的浮层
    /// 白跑一次 IPC，而且网络慢时点「关」要等一会儿才关。
    /// </summary>
    public bool IsPreviewOpenFor(ContentReferenceSyntax.Occurrence occurrence) =>
        occurrence.IsValid
        && string.Equals(_openIdentity, IdentityOf(occurrence), StringComparison.Ordinal);

    /// <summary>切到别的节点时关掉预览——上一个节点的预览留着会让人困惑。</summary>
    public void CollapseAll() => ClosePreview();

    /// <summary>命中测试：给定光标偏移，落在哪条引用上（没有则 null）。</summary>
    ///
    /// <remarks>
    /// 用**半开区间**判定：`[Start, End)`。占位符紧邻时（`}}{{ref:`）
    /// 闭区间会让边界那一个偏移同时命中两条，点击行为随实现顺序而变。
    /// </remarks>
    public static ContentReferenceSyntax.Occurrence? HitTest(
        IReadOnlyList<ContentReferenceSyntax.Occurrence> occurrences,
        int offset) =>
        occurrences.FirstOrDefault(item => offset >= item.Start && offset < item.End);

    /// <summary>
    /// 从**整篇文档正文**里切出这条引用指的那一段，供预览显示。
    ///
    /// # 为什么前端自己切，而不是让后端展开器给
    ///
    /// 后端 `rag/reference.rs` 的 `expand_content_references` 是**给 AI 展开**的那条链：
    /// 它加 `[提供的正文参考：…]` 标记、有条数与总长上限、要做越权校验。
    /// 复用它会让**编辑期预览受运行期规则影响**——作者看到「预览一片空白」，
    /// 而真实原因是撞了单次展开条数上限，界面上完全看不出来。
    /// 预览要的只是「那几行长什么样」，所以走 `GetDocumentContentAsync` 取全文、
    /// 在这里切。⇒ 行段口径与后端**允许**有一两行的差别，那不是缺陷。
    ///
    /// # 行口径
    ///
    /// 用 `1-based 闭区间`，与 Rust 侧 `line_count_of` 同取舍：
    /// 按 `\n` 切且换行符归属该行，末尾无换行时最后一段仍算一行。
    /// 越界**截断到文档末尾**而不是报错——正文改短之后旧引用越界是常态，
    /// 为此让预览失败太脆（后端同样是截断 + 记警告）。
    ///
    /// ⚠️ **byte 定位（`@1024-2048`）不做切片，返回整篇**：那是 UTF-8 byte 区间，
    /// 而这里手上是 UTF-16 字符串，换算要重做一遍 UTF-8 编码与边界校验
    /// （`ContentReferenceSyntax` 顶部注释解释了两侧口径为何刻意不一致）。
    /// byte 形态是工具生成的、人不手写，为它做一套易错的换算不划算；
    /// 显示整篇仍然如实回答了「引的是哪篇」。
    /// </summary>
    public static string SliceForPreview(
        string documentText,
        ContentReferenceSyntax.Occurrence occurrence)
    {
        if (occurrence.Locator != ContentReferenceSyntax.LocatorKind.Lines
            || documentText.Length == 0)
        {
            return documentText;
        }

        // 与 Rust 的 `split_inclusive('\n')` 同口径：换行符归属它结束的那一行。
        var lines = new List<string>();
        var lineStart = 0;
        for (var i = 0; i < documentText.Length; i++)
        {
            if (documentText[i] == '\n')
            {
                lines.Add(documentText[lineStart..(i + 1)]);
                lineStart = i + 1;
            }
        }
        if (lineStart < documentText.Length)
        {
            lines.Add(documentText[lineStart..]);
        }

        // 1-based 闭区间 → 0-based 半开，并截断到文档范围内。
        // `RangeStart` 是 long（语法层允许作者写很大的数），先夹到 int 再转。
        var start = (int)Math.Clamp(occurrence.RangeStart - 1, 0, lines.Count);
        var end = (int)Math.Clamp(occurrence.RangeEnd, 0, lines.Count);
        if (end <= start)
        {
            // 起点已越到末尾之后（或区间空）：给空串而不是给整篇。
            // 给整篇会让「行号写错了」看起来像「引用没生效」——两者提示完全不同。
            return string.Empty;
        }

        return string.Concat(lines.GetRange(start, end - start));
    }

    /// <summary>
    /// 预览浮层标题里显示的文字。
    ///
    /// ⚠️ **与「给 AI 看的展开标记」不是同一套字符串**（U150 文档特意点过这一条）。
    /// 给 AI 的是 `[提供的正文参考：xx章 章节标题名]…[正文参考结束]`——它要让模型
    /// 知道这段是引来的、边界在哪。给人的是浮层标题栏里一行**尽量短**的出处。
    ///
    /// 语法非法时给原因而不是路径：这时用户要修的是语法，给他看「文档ID」没用。
    /// （非法引用不会开浮层，但这个标签仍可能出现在诊断与用例里。）
    /// </summary>
    private static string PreviewLabelFor(ContentReferenceSyntax.Occurrence occurrence)
    {
        if (!occurrence.IsValid)
        {
            return occurrence.ParseError ?? "引用写法有误";
        }

        // 只取文件名：`chapters/第三卷/chapter-42.md` 在浮层标题里占掉整行，
        // 而作者认得出 `chapter-42.md`。
        var name = occurrence.DocumentId;
        var slash = name.LastIndexOfAny(new[] { '/', '\\' });
        if (slash >= 0 && slash + 1 < name.Length)
        {
            name = name[(slash + 1)..];
        }

        return occurrence.Locator switch
        {
            ContentReferenceSyntax.LocatorKind.Lines =>
                $"{name} L{occurrence.RangeStart}-{occurrence.RangeEnd}",
            ContentReferenceSyntax.LocatorKind.Bytes => $"{name} @{occurrence.RangeStart}",
            _ => name,
        };
    }
}
