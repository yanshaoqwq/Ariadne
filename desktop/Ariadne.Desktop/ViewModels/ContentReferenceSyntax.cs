using System.Collections.Generic;
using System.Globalization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// U150：`{{ref:...}}` 正文引用的**前端**词法层。不依赖 Avalonia，便于纯逻辑单测。
///
/// # 为什么在 C# 再实现一遍，而不是新开一条 IPC
///
/// 这个解析器要在**每次按键之后**跑一遍（占位符高亮随输入实时更新）。走 IPC 意味着
/// 每敲一个字符就跨一次进程边界：JSON 序列化 + 管道往返 + 反序列化，还得处理
/// 「上一次请求未回、用户已经又敲了三下」的乱序。为了给一个纯词法判断（找 `{{ref:`
/// 与 `}}`、切几段字符串）付这个代价是不成比例的——而且解析失败时前端仍然只能显示
/// 「有误」，跨进程一趟并没有换来任何后端独有的信息。
///
/// 权威语法定义仍在 `core/src/rag/reference.rs`（**它**决定送进 LLM 的是什么，
/// 这里只决定给人看的是什么）。两份实现的漂移风险由
/// `core/tests/fixtures/content_reference_cases.json` 这一份**双方共读**的语料收口：
/// Rust 侧 `content_reference_expansion_contracts.rs` 与 C# 侧
/// `ContentReferenceSyntaxTests` 读同一个文件、断言同一批期望值。任一侧改了语法而
/// 没同步，红的是那一侧的用例——漂移无法静默发生。
///
/// # 偏移量口径：这里是 UTF-16，Rust 那边是 byte
///
/// **刻意不一致，也不可能一致。** Rust 的 `TextRange` 是 UTF-8 byte 半开区间；C# 的
/// string 索引是 UTF-16 code unit。正文是中文，同一个占位符在两种口径下的数值必然
/// 不同（一个汉字 3 byte vs 1 code unit）。AvaloniaEdit 的
/// <c>TextDocument</c> 用的是 UTF-16 偏移，所以这里必须给 UTF-16——换算成 byte 再
/// 换回来只会多两次出错机会。
///
/// 因此共读语料**不比较偏移数值**，比较的是「按各自偏移切出来的子串等于 `raw`」这条
/// 不变式——那才是偏移量真正要保证的性质，且在两种口径下都成立。
/// </summary>
public static class ContentReferenceSyntax
{
    /// <summary>与 Rust 的 <c>CONTENT_REFERENCE_OPEN</c> 同值。</summary>
    public const string Open = "{{ref:";

    /// <summary>与 Rust 的 <c>CONTENT_REFERENCE_CLOSE</c> 同值。</summary>
    public const string Close = "}}";

    /// <summary>定位方式；与 Rust 的 <c>ReferenceLocator</c> 三个变体一一对应。</summary>
    public enum LocatorKind
    {
        /// <summary><c>#L120-L145</c>：1-based 闭区间行号。</summary>
        Lines,

        /// <summary><c>@1024-2048</c>：UTF-8 byte 半开区间（由工具生成，人不手写）。</summary>
        Bytes,

        /// <summary>无定位段：整篇。</summary>
        Whole,
    }

    /// <summary>
    /// 扫描到的一个占位符。
    ///
    /// 语法非法的引用**同样占一个位置**（<see cref="ParseError"/> 非空、
    /// <see cref="DocumentId"/> 为空）——与 Rust 侧同一个态度：坏语法不能被静默丢掉，
    /// 否则用户在编辑器里看不出自己写错了，只能等运行时 fail-loud。
    /// </summary>
    public sealed record Occurrence
    {
        /// <summary>占位符在被扫描文本中的起点（UTF-16，含 <c>{{ref:</c>）。</summary>
        public required int Start { get; init; }

        /// <summary>占位符在被扫描文本中的终点（UTF-16 半开，含 <c>}}</c>）。</summary>
        public required int End { get; init; }

        /// <summary>占位符原文，用于诊断与原样回填。</summary>
        public required string Raw { get; init; }

        /// <summary>被引文档 id；解析失败时为空串。</summary>
        public string DocumentId { get; init; } = string.Empty;

        /// <summary>定位方式。</summary>
        public LocatorKind Locator { get; init; } = LocatorKind.Whole;

        /// <summary>行号 / byte 区间起点；<see cref="LocatorKind.Whole"/> 时为 0。</summary>
        public long RangeStart { get; init; }

        /// <summary>行号 / byte 区间终点；<see cref="LocatorKind.Whole"/> 时为 0。</summary>
        public long RangeEnd { get; init; }

        /// <summary><c>@v=</c> 锚定的内容版本；未锚定时为 null。</summary>
        public string? Version { get; init; }

        /// <summary>语法非法时的可读原因；合法时为 null。</summary>
        public string? ParseError { get; init; }

        /// <summary>是否解析成功。</summary>
        public bool IsValid => ParseError is null;

        /// <summary>占位符长度（UTF-16）。</summary>
        public int Length => End - Start;
    }

    /// <summary>
    /// 便宜的前置筛，对应 Rust 的 <c>contains_content_reference</c>。
    ///
    /// 提示词编辑器每次按键都要判断「要不要重算折叠段」，绝大多数提示词里根本没有
    /// 引用；先用一次 <c>Contains</c> 挡掉，比无条件走完整扫描省得多。
    /// </summary>
    public static bool ContainsReference(string? text) =>
        !string.IsNullOrEmpty(text) && text.Contains(Open, StringComparison.Ordinal);

    /// <summary>
    /// 扫描全部占位符，按出现顺序返回。对应 Rust 的 <c>parse_content_references</c>。
    ///
    /// **未闭合的 <c>{{ref:</c> 不记成 occurrence**——与 Rust 侧同一取舍：它没有确定的
    /// 结束位置，无法就地替换，也无法给它划一段折叠区。留在文本里由运行期的哨兵拦住，
    /// 那时能给出比这里更完整的报错。
    /// </summary>
    public static IReadOnlyList<Occurrence> Parse(string? text)
    {
        var occurrences = new List<Occurrence>();
        if (string.IsNullOrEmpty(text))
        {
            return occurrences;
        }

        var cursor = 0;
        while (cursor < text.Length)
        {
            var open = text.IndexOf(Open, cursor, StringComparison.Ordinal);
            if (open < 0)
            {
                break;
            }

            var bodyStart = open + Open.Length;
            var close = text.IndexOf(Close, bodyStart, StringComparison.Ordinal);
            if (close < 0)
            {
                // 没有闭合记号：后面不可能再有完整占位符，收尾。
                break;
            }

            var end = close + Close.Length;
            occurrences.Add(ParseBody(text[bodyStart..close]) with
            {
                Start = open,
                End = end,
                Raw = text[open..end],
            });
            cursor = end;
        }

        return occurrences;
    }

    /// <summary>
    /// 解析占位符内部（<c>{{ref:</c> 与 <c>}}</c> 之间）的定位串。
    ///
    /// 返回一个「壳」记录，位置字段由调用方补齐。不抛异常：单条引用写坏是常态输入，
    /// 用异常做控制流会让「用户打字打到一半」这种中间态刷出一串异常。
    /// </summary>
    private static Occurrence ParseBody(string rawBody)
    {
        var body = rawBody.Trim();
        if (body.Length == 0)
        {
            return Malformed("引用为空：应写作 {{ref:文档ID#L起始-L结束}}");
        }

        // 先摘版本锚定 `@v=`；它一定在最后，且与 byte 定位的 `@` 靠 `v=` 前缀区分。
        string? version = null;
        var locatorPart = body;
        var versionAt = body.LastIndexOf("@v=", StringComparison.Ordinal);
        if (versionAt >= 0)
        {
            version = body[(versionAt + 3)..].Trim();
            if (version.Length == 0)
            {
                return Malformed("版本锚定 @v= 后面没有内容");
            }
            locatorPart = body[..versionAt].Trim();
        }

        if (locatorPart.Length == 0)
        {
            return Malformed("引用缺少文档 ID");
        }

        // 行号定位 `#L120-L145`
        var hash = locatorPart.IndexOf('#', StringComparison.Ordinal);
        if (hash >= 0)
        {
            return ParseLineLocator(locatorPart[..hash], locatorPart[(hash + 1)..], version);
        }

        // byte 定位 `@1024-2048`
        var at = locatorPart.IndexOf('@', StringComparison.Ordinal);
        if (at >= 0)
        {
            return ParseByteLocator(locatorPart[..at], locatorPart[(at + 1)..], version);
        }

        return Finish(locatorPart, LocatorKind.Whole, 0, 0, version);
    }

    /// <summary>解析 <c>L120-L145</c>；单行写作 <c>L120</c> 也接受。</summary>
    private static Occurrence ParseLineLocator(string documentId, string raw, string? version)
    {
        var lines = raw.Trim();
        long start;
        long end;
        var dash = lines.IndexOf('-', StringComparison.Ordinal);
        if (dash >= 0)
        {
            if (!TryParseLine(lines[..dash], out start))
            {
                return Malformed($"行号无法解析：{lines[..dash]}");
            }
            if (!TryParseLine(lines[(dash + 1)..], out end))
            {
                return Malformed($"行号无法解析：{lines[(dash + 1)..]}");
            }
        }
        else
        {
            // 单行引用：`#L120` 等价于 `#L120-L120`。允许它是因为模型经常这么写，
            // 拒绝只会换来一条无谓的失效警告。
            if (!TryParseLine(lines, out start))
            {
                return Malformed($"行号无法解析：{lines}");
            }
            end = start;
        }

        if (start == 0 || end == 0)
        {
            return Malformed("行号是 1-based，不能为 0");
        }
        if (start > end)
        {
            return Malformed($"行号区间起点 {start} 大于终点 {end}");
        }

        return Finish(documentId, LocatorKind.Lines, start, end, version);
    }

    /// <summary>解析 <c>1024-2048</c> 形式的 byte 半开区间。</summary>
    private static Occurrence ParseByteLocator(string documentId, string raw, string? version)
    {
        var bytes = raw.Trim();
        var dash = bytes.IndexOf('-', StringComparison.Ordinal);
        if (dash < 0)
        {
            return Malformed($"byte 区间应写作 起始-结束，实际是：{bytes}");
        }

        var startText = bytes[..dash].Trim();
        var endText = bytes[(dash + 1)..].Trim();
        if (!long.TryParse(startText, NumberStyles.None, CultureInfo.InvariantCulture, out var start))
        {
            return Malformed($"byte 起点无法解析：{startText}");
        }
        if (!long.TryParse(endText, NumberStyles.None, CultureInfo.InvariantCulture, out var end))
        {
            return Malformed($"byte 终点无法解析：{endText}");
        }
        if (start > end)
        {
            return Malformed($"byte 区间非法：{start}-{end}");
        }

        return Finish(documentId, LocatorKind.Bytes, start, end, version);
    }

    /// <summary>行号可带 <c>L</c>/<c>l</c> 前缀，也可裸写数字。</summary>
    private static bool TryParseLine(string value, out long line)
    {
        var digits = value.Trim();
        if (digits.StartsWith('L') || digits.StartsWith('l'))
        {
            digits = digits[1..];
        }
        return long.TryParse(digits, NumberStyles.None, CultureInfo.InvariantCulture, out line);
    }

    /// <summary>组装结果并校验 documentId 非空。</summary>
    private static Occurrence Finish(
        string documentId,
        LocatorKind locator,
        long start,
        long end,
        string? version)
    {
        var id = documentId.Trim();
        if (id.Length == 0)
        {
            return Malformed("引用缺少文档 ID");
        }

        return new Occurrence
        {
            Start = 0,
            End = 0,
            Raw = string.Empty,
            DocumentId = id,
            Locator = locator,
            RangeStart = start,
            RangeEnd = end,
            Version = version,
        };
    }

    /// <summary>坏语法的壳记录。</summary>
    private static Occurrence Malformed(string reason) => new()
    {
        Start = 0,
        End = 0,
        Raw = string.Empty,
        ParseError = reason,
    };
}
