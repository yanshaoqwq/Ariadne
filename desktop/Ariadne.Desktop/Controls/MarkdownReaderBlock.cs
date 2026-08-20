using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Documents;
using Avalonia.Layout;
using Avalonia.Media;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// U203：阅读态的 Markdown 渲染宿主。一个实例 = 一个虚拟化正文块。
///
/// # 为什么是控件而不是「一堆嵌套 ItemsControl + DataTemplate」
///
/// 阅读态的既有能力全都挂在「容器里的 <see cref="SelectableTextBlock"/>」上：
/// U132 的 Ctrl+A/Ctrl+C 跨块归并按 `DataContext is DocumentBlockViewModel` 找块，
/// U151/U200 的两态排版一致性用例也按同一条件取正文块。用内层 ItemsControl 的话
/// 每个片段的 DataContext 会变成片段自己的 VM，**那些用例会一条都找不到正文块**
/// （而且是「找不到」而非「值不对」，失败信息完全不指向真实原因）。
/// 这里直接建子控件、不给它们设 DataContext ⇒ 沿逻辑树继承到
/// <c>DocumentBlockViewModel</c>，既有寻址方式全部继续成立。
///
/// # 与修改态刻意不一致
///
/// 修改态（`ae:TextEditor`）里作者看到的是**源码**，`#`、`**` 必须原样在。
/// 只有阅读态渲染。⚠️ 这不是缺陷，别「统一」——两态服务于两件不同的事：
/// 一边是改稿，一边是读成书。
///
/// # 视觉全部走主题样式，C# 里只挂 class
///
/// 字号/字重/颜色/间距/边框一律由 `AriadneTheme.axaml` 的 `.md-*` 样式给，
/// 代码里不出现任何数字或色值：颜色不许魔法数字（项目硬约束），
/// 而且 class 名用字面量挂（`ThemeStyleUsageTests` 的死样式扫描只认字面量，
/// 用 `$"md-h{level}"` 插值会让 6 个在用的类被报成死样式）。
/// </summary>
public sealed class MarkdownReaderBlock : StackPanel
{
    /// <summary>本块的原始正文（含 Markdown 标记）。</summary>
    public static readonly StyledProperty<string?> SourceProperty =
        AvaloniaProperty.Register<MarkdownReaderBlock, string?>(nameof(Source));

    /// <summary>首行是否是上一块某行的后半截（虚拟化硬切在行中间时为 true）。</summary>
    public static readonly StyledProperty<bool> ContinuesPreviousLineProperty =
        AvaloniaProperty.Register<MarkdownReaderBlock, bool>(nameof(ContinuesPreviousLine));

    private readonly List<SelectableTextBlock> _selectableSegments = new();

    public string? Source
    {
        get => GetValue(SourceProperty);
        set => SetValue(SourceProperty, value);
    }

    public bool ContinuesPreviousLine
    {
        get => GetValue(ContinuesPreviousLineProperty);
        set => SetValue(ContinuesPreviousLineProperty, value);
    }

    /// <summary>
    /// 本块渲染出的可选中片段，按正文顺序。
    ///
    /// U132 的选区归并需要它：一个块现在可能渲染成多个 <see cref="SelectableTextBlock"/>
    /// （标题、正文、引用各自一个控件），只取第一个会让 Ctrl+A 只选中每块的开头一段。
    /// </summary>
    public IReadOnlyList<SelectableTextBlock> SelectableSegments => _selectableSegments;

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == SourceProperty || change.Property == ContinuesPreviousLineProperty)
        {
            Rebuild();
        }
    }

    /// <summary>
    /// 重建子控件。
    ///
    /// 虚拟化会回收容器并改绑新块，届时 <see cref="SourceProperty"/> 变化会再走一遍这里，
    /// 所以必须先彻底清空——残留旧片段就是「滚回去发现多出一段别的章节」。
    /// </summary>
    private void Rebuild()
    {
        Children.Clear();
        _selectableSegments.Clear();

        var segments = MarkdownReadingParser.Parse(Source, ContinuesPreviousLine);
        foreach (var segment in segments)
        {
            var child = BuildSegment(segment);
            if (child is not null)
            {
                Children.Add(child);
            }
        }
    }

    private Control? BuildSegment(MarkdownSegment segment) => segment.Kind switch
    {
        MarkdownSegmentKind.ThematicBreak => BuildRule(),
        MarkdownSegmentKind.Heading => BuildHeading(segment),
        MarkdownSegmentKind.Quote => BuildQuote(segment),
        MarkdownSegmentKind.ListItem => BuildListItem(segment),
        MarkdownSegmentKind.CodeBlock => BuildCodeBlock(segment),
        _ => BuildParagraph(segment),
    };

    /// <summary>分隔线：一条发丝线。高度/色/上下留白全在 `.md-rule` 样式里。</summary>
    private static Control BuildRule()
    {
        var rule = new Border();
        rule.Classes.Add("md-rule");
        return rule;
    }

    private Control BuildParagraph(MarkdownSegment segment)
    {
        var text = NewTextBlock(segment);
        text.Classes.Add("reading");
        text.Classes.Add("md-paragraph");
        return text;
    }

    private Control BuildHeading(MarkdownSegment segment)
    {
        var text = NewTextBlock(segment);
        text.Classes.Add("md-heading");
        // 字面量挂类：死样式扫描器只认字面量，插值会把 6 个在用的类报成死样式。
        text.Classes.Add(segment.HeadingLevel switch
        {
            1 => "md-h1",
            2 => "md-h2",
            3 => "md-h3",
            4 => "md-h4",
            5 => "md-h5",
            _ => "md-h6",
        });
        return text;
    }

    /// <summary>引用块：左侧竖线 + 次级文字色。竖线是 Border 的左边框，不是画出来的。</summary>
    private Control BuildQuote(MarkdownSegment segment)
    {
        var text = NewTextBlock(segment);
        text.Classes.Add("reading");
        text.Classes.Add("md-quote-text");

        var frame = new Border { Child = text };
        frame.Classes.Add("md-quote");
        return frame;
    }

    /// <summary>
    /// 列表项：标记与内容分两列。
    ///
    /// 标记不放进同一个 <see cref="SelectableTextBlock"/>：那样换行后第二行会顶到
    /// 圆点下面（没有悬挂缩进），且 Ctrl+C 会把「•」一起复制走。
    /// 标记用普通 TextBlock（不可选中）正好两件事一起解决。
    /// </summary>
    private Control BuildListItem(MarkdownSegment segment)
    {
        var marker = new TextBlock { Text = segment.ListMarker ?? string.Empty };
        marker.Classes.Add("reading");
        marker.Classes.Add("md-list-marker");

        var text = NewTextBlock(segment);
        text.Classes.Add("reading");
        text.Classes.Add("md-list-text");
        Grid.SetColumn(text, 1);

        var row = new Grid
        {
            ColumnDefinitions = new ColumnDefinitions("Auto,*"),
        };
        row.Classes.Add("md-list-row");
        row.Children.Add(marker);
        row.Children.Add(text);
        return row;
    }

    /// <summary>围栏代码块：等宽字 + 代码底色。块内不做行内解析（见解析器注释）。</summary>
    private Control BuildCodeBlock(MarkdownSegment segment)
    {
        var text = NewTextBlock(segment);
        text.Classes.Add("md-code-text");

        var frame = new Border { Child = text };
        frame.Classes.Add("md-code");
        return frame;
    }

    /// <summary>
    /// 造一个承载片段文本的 <see cref="SelectableTextBlock"/> 并登记进
    /// <see cref="SelectableSegments"/>。
    ///
    /// # 纯文本走 Text、带格式才走 Inlines（**刻意分两条路**）
    ///
    /// 探针实测（Avalonia 12.0.5）：一旦往 <c>Inlines</c> 里加东西，
    /// <c>TextBlock.Text</c> 就恒为 **null**，全文只能从 <c>Inlines.Text</c> 取；
    /// <c>SelectedText</c> 则按 <c>Inlines.Text</c> 的偏移取值（偏移不错位，
    /// <c>LineBreak</c> 在其中正好贡献一个 `\n`）。
    /// ⇒ 无格式段落走 <c>Text</c> 这条路的收益是：它与本条修复**之前**的渲染
    /// 完全是同一条代码路径，绝大多数正文（中文小说里带 `**` 的行是少数）
    /// 的排版风险为零，U151/U200 那批「两态排版必须一致」的用例也不受影响。
    ///
    /// # 段内换行必须用 LineBreak，不能把 `\n` 塞进 Run
    ///
    /// 项目已知踩坑：AvaloniaEdit 的 <c>FormattedTextElement</c> 遇 `\n` 会**静默截断**。
    /// 这里不是同一个类，但「多行塞进一个 inline 元素」是同一种赌注——
    /// <c>LineBreak</c> 是语义明确的那条路，且探针证实它对
    /// <c>Inlines.Text</c> / <c>SelectedText</c> 的偏移贡献恰好是一个 `\n`
    /// （所以复制出来的正文行数是对的）。
    /// </summary>
    private SelectableTextBlock NewTextBlock(MarkdownSegment segment)
    {
        var block = new SelectableTextBlock
        {
            TextWrapping = TextWrapping.Wrap,
            HorizontalAlignment = HorizontalAlignment.Stretch,
        };

        if (segment.IsPlainText)
        {
            block.Text = segment.VisibleText;
        }
        else
        {
            foreach (var inline in segment.Inlines)
            {
                block.Inlines!.Add(CreateInline(inline));
            }
        }

        _selectableSegments.Add(block);
        return block;
    }

    /// <summary>
    /// 行内片段 → Avalonia inline。
    ///
    /// 用 <see cref="Run"/> 的属性（FontWeight/FontStyle）而不是 <c>Bold</c>/<c>Italic</c>
    /// 这两个容器类：容器类要求内容是 inline 集合，而这里每个片段已经是叶子文本，
    /// 多套一层只会多一层布局。行内代码的底色与等宽字体走
    /// <c>Ariadne.BackgroundCode</c> / <c>Ariadne.Font.Mono</c> 两个 token，
    /// ⚠️ inline 挂不上 class（<see cref="Run"/> 不是 Control，样式选择器选不到它），
    /// 所以这里只能用 <c>TryFindResource</c> 现取 token —— 但**取的仍是 token**，
    /// 不是硬编码色值。
    /// </summary>
    private Avalonia.Controls.Documents.Inline CreateInline(MarkdownInline inline)
    {
        if (inline.Kind == MarkdownInlineKind.LineBreak)
        {
            return new LineBreak();
        }

        var run = new Run(inline.Text);
        switch (inline.Kind)
        {
            case MarkdownInlineKind.Bold:
                run.FontWeight = FontWeight.Bold;
                break;
            case MarkdownInlineKind.Italic:
                run.FontStyle = FontStyle.Italic;
                break;
            case MarkdownInlineKind.BoldItalic:
                run.FontWeight = FontWeight.Bold;
                run.FontStyle = FontStyle.Italic;
                break;
            case MarkdownInlineKind.Code:
                if (this.TryFindResource("Ariadne.Font.Mono", ActualThemeVariant, out var mono)
                    && mono is FontFamily monoFamily)
                {
                    run.FontFamily = monoFamily;
                }
                if (this.TryFindResource("Ariadne.BackgroundCode", ActualThemeVariant, out var background)
                    && background is IBrush codeBackground)
                {
                    run.Background = codeBackground;
                }
                break;
        }
        return run;
    }
}
