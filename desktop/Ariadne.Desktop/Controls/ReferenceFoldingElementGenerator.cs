using System.Collections.Generic;
using Avalonia.Media;
using Avalonia.Media.TextFormatting;
using AvaloniaEdit.Document;
using AvaloniaEdit.Rendering;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// U150：把折叠态的 `{{ref:...}}` **在呈现层**换成一行短摘要。
///
/// # 不改文档文本，这是地基
///
/// `TextDocument.Text` 始终是用户写的原始模板（含 `{{ref:...}}` 字面量）。
/// 折叠只是 AvaloniaEdit 的 `VisualLineElementGenerator` 替换了这段的**显示**——
/// 一旦折叠改写了文本，用户保存的就不是他写的东西了（而他看不出差别，
/// 因为屏幕上显示的正是被改写后的样子）。
///
/// # 为什么用 FormattedTextElement 而不是 InlineObjectElement
///
/// `InlineObjectElement` 能塞任意控件（能做圆角胶囊、能挂点击），但每条引用都要
/// 实体化一个控件、参与 measure/arrange，并由 TextView 维护 inline object 生命周期。
/// U159 的教训正是「每节点 20 个绑定把重挂载压垮」——提示词里可以有 20 条引用，
/// 同一个陷阱。`FormattedTextElement` 是纯文本 run，没有控件开销，
/// 而「可点击」不靠控件实现：点击由编辑器的 `PointerPressed` 拿偏移做命中测试
/// （见 `ReferenceFoldingState.HitTest`），那条路对折叠/展开两态都通用。
/// </summary>
internal sealed class ReferenceFoldingElementGenerator : VisualLineElementGenerator
{
    private readonly Func<IReadOnlyList<ReferenceFoldingState.Segment>> _segments;
    private readonly Func<IBrush?> _labelBrush;

    /// <param name="segments">
    /// 取当前投影。传委托而不是快照：投影每次文本变化都会重算，
    /// 传值会让生成器抱着一份过期的偏移量去切文本（切在半个占位符上）。
    /// </param>
    /// <param name="labelBrush">折叠摘要的前景色；从主题现取，不在这里写颜色。</param>
    internal ReferenceFoldingElementGenerator(
        Func<IReadOnlyList<ReferenceFoldingState.Segment>> segments,
        Func<IBrush?> labelBrush)
    {
        _segments = segments;
        _labelBrush = labelBrush;
    }

    /// <summary>
    /// 报告「从 startOffset 起，我最早对哪个偏移感兴趣」。
    ///
    /// 只对**折叠态**的引用感兴趣：展开态要显示原始 `{{ref:...}}` 文本，
    /// 那正是 AvaloniaEdit 默认行为，不需要生成器插手。
    /// 返回 -1 表示「后面没有我的活了」——返回 0 会让 TextView 反复回调，
    /// 这是这个 API 最容易写错的一处。
    /// </summary>
    public override int GetFirstInterestedOffset(int startOffset)
    {
        var best = -1;
        foreach (var segment in _segments())
        {
            if (segment.IsExpanded || segment.Start < startOffset)
            {
                continue;
            }
            if (best < 0 || segment.Start < best)
            {
                best = segment.Start;
            }
        }
        return best;
    }

    /// <summary>
    /// 造出折叠摘要那个 run。
    ///
    /// `documentLength` 必须是**占位符在文档里的真实长度**：TextView 靠它把
    /// 视觉列映射回文档偏移。给错的话光标定位、选区、点击命中全部错位，
    /// 而症状是「点这里却选中了别处」，极难联想到这个参数。
    /// </summary>
    public override VisualLineElement? ConstructElement(int offset)
    {
        foreach (var segment in _segments())
        {
            if (segment.IsExpanded || segment.Start != offset)
            {
                continue;
            }

            // ⚠️ **不能在这里改 TextRunProperties**：`VisualLineElement.TextRunProperties`
            // 此刻是 null——框架在 `VisualLine.ConstructVisualElements` 之后才调
            // `SetTextRunProperties` 把全局属性拷进来。在这里碰它是 NullReferenceException，
            // 而异常发生在 VisualLine 构建里 ⇒ 症状是**整个编辑器不显示任何内容**，
            // 极难联想到是着色那一行。颜色改到 `ColorizedFoldElement.CreateTextRun` 里做，
            // 那时属性已经就位。
            return new ColorizedFoldElement(
                CollapsedText(segment),
                segment.End - segment.Start,
                _labelBrush);
        }

        return null;
    }

    /// <summary>
    /// 带前景色的折叠摘要 run。
    ///
    /// 单独一个类只为**推迟着色时机**：`TextRunProperties` 在
    /// <see cref="ConstructElement"/> 里还是 null，只有到 `CreateTextRun`
    /// 才被框架填好。基类的 `CreateTextRun` 会用这份属性造 run，
    /// 所以在调它之前设色即可。
    /// </summary>
    private sealed class ColorizedFoldElement : FormattedTextElement
    {
        private readonly Func<IBrush?> _brush;

        internal ColorizedFoldElement(string text, int documentLength, Func<IBrush?> brush)
            : base(text, documentLength)
        {
            _brush = brush;
        }

        public override TextRun CreateTextRun(
            int startVisualColumn,
            ITextRunConstructionContext context)
        {
            var brush = _brush();
            // 只在**两者都就位**时设：主题查不到画刷、或属性仍未注入时保持原样
            // ⇒ 继承编辑器前景色，而不是退成透明（那样折叠摘要会整条消失，
            // 看起来像引用被吃了）。
            if (brush is not null && TextRunProperties is not null)
            {
                TextRunProperties.SetForegroundBrush(brush);
            }
            return base.CreateTextRun(startVisualColumn, context);
        }
    }

    /// <summary>
    /// 折叠态显示的整串文字。
    ///
    /// 用 `‹ … ›` 而不是方括号：`[提供的正文参考：…]` 是**给 AI 看的**展开标记
    /// （`core/src/rag/reference.rs` 的 `EXPANSION_OPEN_PREFIX`），
    /// 两者形状必须能一眼区分，否则用户会以为编辑器里这行就是发出去的内容。
    /// 摘要正文由 `ReferenceFoldingState.CollapsedLabelFor` 决定（只取文件名 + 行段），
    /// 这里只加边框记号。
    ///
    /// ⚠️ `internal` 是为了让用例**钉住这个格式**：折叠标记与 AI 展开标记撞形状
    /// 是本条最容易犯的错（U150 文档特意点过），而它只有靠断言字符串才拦得住。
    /// </summary>
    internal static string CollapsedText(ReferenceFoldingState.Segment segment) =>
        $"‹{segment.CollapsedLabel}›";
}
