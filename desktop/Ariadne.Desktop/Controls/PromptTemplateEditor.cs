using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using AvaloniaEdit;
using AvaloniaEdit.Document;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// U150 / U115：提示词模板编辑器。
///
/// # 为什么是 TextEditor 的子类而不是在页面 code-behind 里接线
///
/// 提示词编辑器出现在两个地方（画布节点检查器、设置页审批提示词）。接线写在
/// 页面里就要写两遍，而这类「注册生成器 + 挂点击 + 重投影」的时序很容易只在
/// 一处写对。做成控件后 XAML 只需换一个标签名。
///
/// # 它替代的是 `TextBox`，代价要说清
///
/// 原本是纯 `TextBox`。`TextBox` **办不到**给子串挂颜色与命中区（没有富文本呈现层），
/// 所以 `{{}}` 在里面就是普通文本：写错了要跑一次工作流、花一次 LLM 钱才知道。
/// 换 AvaloniaEdit 拿到了呈现能力，代价是它自己排版——
/// ⚠️ **不吃 `TextBlock.LineHeight`**（内联写了静默无效，U151 就是这么漏的），
/// 也不继承 `TextBox` 的主题样式，字体/字号/行高必须在这里显式设。
/// </summary>
public class PromptTemplateEditor : TextEditor
{
    /// <summary>
    /// ⚠️ **没有这一行，控件会渲染成一个完全空白的框**。
    ///
    /// Avalonia 按控件的 **StyleKey** 去找 `ControlTheme`，默认就是它的实际类型。
    /// AvaloniaEdit 的 dll 里那份 `ControlTheme` 是**键在 `TextEditor` 上**的，
    /// 所以子类查不到 ⇒ 没有模板 ⇒ 连 `TextArea` 都不会被实体化，
    /// 屏幕上是一片空白（而**不报任何错**）。
    ///
    /// 这是本条唯一靠单测抓不到的缺陷：headless 下 `TextArea` 本来就不被 arrange，
    /// 「没有模板」和「有模板但没布局」在测试里长得一模一样。
    /// **是真机开窗截图看出来的**——第一张截图里编辑器整块不见了，只剩标题。
    /// 换任何控件基类时都要先确认这一条。
    /// </summary>
    protected override Type StyleKeyOverride => typeof(TextEditor);

    /// <summary>折叠状态。属于控件而非 VM：它是**呈现状态**，不该进保存的数据。</summary>
    private readonly ReferenceFoldingState _folding = new();

    /// <summary>当前投影。每次文本变化重算，生成器通过委托读它。</summary>
    private IReadOnlyList<ReferenceFoldingState.Segment> _segments =
        Array.Empty<ReferenceFoldingState.Segment>();

    public PromptTemplateEditor()
    {
        // 提示词是模板不是散文：等宽字体让 `{{input.outline}}` 的括号对齐可读。
        // ⚠️ 字体/字号/槽色**在主题里设**（`ctl|PromptTemplateEditor` 那条），
        // 不在这里写死——颜色与尺度必须是主题 token，写在 C# 里就绕过了主题切换。
        // 这里只设与视觉无关的行为开关。
        WordWrap = true;
        ShowLineNumbers = false;

        // 折叠：注册生成器。它读 `_segments` 委托而非快照——投影每次文本变化重算，
        // 传快照会让生成器抱着过期偏移去切文本（切在半个占位符上）。
        TextArea.TextView.ElementGenerators.Add(
            new ReferenceFoldingElementGenerator(() => _segments, () => FoldedLabelBrush()));

        // 高亮：注册着色器。三档颜色的语义见 PromptPlaceholderColorizer。
        TextArea.TextView.LineTransformers.Add(
            new PromptPlaceholderColorizer(PlaceholderBrush));

        // 文本变化 → 重投影。挂 Document.Changed 而不是自己的 BoundText setter：
        // 用户敲键改的是 Document，绕过 BoundText。
        DocumentChanged += OnDocumentInstanceChanged;
        AttachDocument(Document);

        // Ctrl+左键：挂 Tunnel + handledEventsToo。理由见 OnEditorPointerPressed。
        AddHandler(
            PointerPressedEvent,
            OnEditorPointerPressed,
            Avalonia.Interactivity.RoutingStrategies.Tunnel,
            handledEventsToo: true);
    }

    /// <summary>Document 实例被换掉时重挂订阅——旧实例的事件不会再来，新实例的还没订上。</summary>
    private void OnDocumentInstanceChanged(object? sender, EventArgs e) => AttachDocument(Document);

    /// <summary>
    /// Ctrl+左键 = 展开/收起光标处的引用。
    ///
    /// # 为什么挂 Tunnel
    ///
    /// `TextArea` 自己处理 `PointerPressed` 来移动光标并起选区，Bubble 阶段事件
    /// 可能已被它标记 Handled ⇒ 时灵时不灵。这条结论抄自 U132
    /// （`WorksPageView` 的阅读面点击也踩过同一个坑）。
    ///
    /// # 为什么**不需要**防冒泡到画布多选
    ///
    /// 报告称本手势与「Ctrl+点选节点=加入多选」冲突，实测**不成立**：多选处理器挂在
    /// 节点卡标题栏（`node-card-header`）的 Bubble 阶段，而本编辑器在右栏检查器——
    /// 两者是视觉树上的**平行分支**，Bubble 只往自己的祖先走。守卫见
    /// `ReferenceGestureRoutingTests`（前提变了会红）。
    ///
    /// 但仍然置 `Handled = true`：不是为了防冒泡，而是**为了不移动光标**。
    /// 让 TextArea 继续处理会把光标跳到点击处并清掉选区，那是「我只想看一眼引用」
    /// 时不该发生的副作用。
    /// </summary>
    /// ⚠️ `internal` 而非 `private`：测试要**直接调它并传真实的
    /// `PointerPressedEventArgs`**（含 Ctrl 修饰键），这样走的是同一段生产代码。
    /// headless 下往控件发合成指针事件受布局与命中测试影响、极易「点不到」，
    /// 那会让用例以「找不到控件」失败而被误读成测试写错（沿用 `CanvasRenderPerfTests` 的结论）。
    internal void OnEditorPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.KeyModifiers.HasFlag(KeyModifiers.Control)
            || !e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }

        // ⚠️ **不用 `TextEditor.GetPositionFromPoint`**：它内部对
        // `this.TranslatePoint(point, TextArea.TextView)` 直接取 `.Value`，
        // 而控件未参与渲染时那个换算返回 null ⇒ `InvalidOperationException`。
        // 走 `TextView.GetPosition` 少一次坐标换算，也就没有这个雷。
        //
        // 传进来的点必须已经是 **TextView 坐标系**的（调用方按事件源换算）。
        // 传控件坐标会偏掉左边距与滚动量 ⇒ 命中到别的引用或什么都不命中，
        // 症状是「点了没反应」，最容易被误判成手势根本没接上。
        var textView = TextArea.TextView;
        var position = textView.GetPosition(e.GetPosition(textView) + textView.ScrollOffset);
        if (position is null || Document is null)
        {
            return;
        }

        if (ToggleReferenceAtOffset(Document.GetOffset(position.Value.Location)))
        {
            e.Handled = true;
        }
    }

    /// <summary>
    /// 展开/收起某个**文档偏移**处的引用；返回是否真的命中并切换了。
    ///
    /// # 为什么把它从点击处理器里切出来
    ///
    /// 这一段（命中测试 → Toggle → 重投影）才是本条的**功能**，坐标换算只是把
    /// 鼠标位置翻译成偏移。切开之后用例可以直接喂偏移，跳过一段**在 headless 下
    /// 根本不成立**的坐标数学：AvaloniaEdit 的 `TextArea` 在 headless 里
    /// **从不被 arrange**（实测 `TextArea.Bounds` 恒为 0×0，而外层 editor 是 520×320），
    /// 于是 `TextView.VisualLines` 为空、`GetPosition` 一律返回 null。
    ///
    /// ⚠️ 代价必须说清：这样切出来的用例**证明不了「鼠标点在那个字上真能算出这个偏移」**。
    /// 那半段由真机验证（本条已开窗看过）与 `GetPosition` 这个框架 API 自身负责。
    /// 用例守住的是「给定命中偏移，展开/收起是否真的发生并反映到呈现投影上」——
    /// 那正是缺陷所在的一半（此前这一半**完全不存在**）。
    /// </summary>
    internal bool ToggleReferenceAtOffset(int offset)
    {
        if (Document is null)
        {
            return false;
        }

        var occurrences = ContentReferenceSyntax.Parse(Document.Text);
        var hit = ReferenceFoldingState.HitTest(occurrences, offset);
        if (hit is null)
        {
            // 没点在引用上：放手让 TextArea 正常定位光标。
            return false;
        }

        _folding.Toggle(hit);
        // **必须重投影**：状态改了而投影没重算的话，屏幕上还是旧的折叠态——
        // 那正是「点了 Ctrl+左键，什么都没发生」这类缺陷的成因。
        Reproject();
        return true;
    }

    private TextDocument? _attachedDocument;

    private void AttachDocument(TextDocument? document)
    {
        if (ReferenceEquals(_attachedDocument, document))
        {
            return;
        }
        if (_attachedDocument is not null)
        {
            _attachedDocument.TextChanged -= OnDocumentTextChanged;
        }
        _attachedDocument = document;
        if (_attachedDocument is not null)
        {
            _attachedDocument.TextChanged += OnDocumentTextChanged;
        }
        Reproject();
    }

    private void OnDocumentTextChanged(object? sender, EventArgs e)
    {
        Reproject();
        // 文本变了就把 BoundText 同步回去。用 `!=` 短路是必需的：
        // 无条件赋值会与下面的 BoundText→Document 方向形成回环。
        var text = Document?.Text ?? string.Empty;
        if (!string.Equals(BoundText, text, StringComparison.Ordinal))
        {
            SetCurrentValue(BoundTextProperty, text);
        }
    }

    /// <summary>
    /// 重算折叠投影并让 TextView 重画。
    ///
    /// **必须 Redraw**：`ElementGenerators` 只在构建 VisualLine 时被问一次，
    /// 投影变了但视图不重建的话屏幕上还是旧的折叠态——
    /// 这正是「点了 Ctrl+左键，什么都没发生」那类缺陷的成因。
    /// </summary>
    private void Reproject()
    {
        _segments = _folding.Project(Document?.Text);
        TextArea.TextView.Redraw();
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property != BoundTextProperty)
        {
            return;
        }

        var incoming = change.GetNewValue<string?>() ?? string.Empty;
        if (Document is null || string.Equals(Document.Text, incoming, StringComparison.Ordinal))
        {
            return;
        }

        // 外部（切换选中节点）换了模板：**全部收起**。
        // 上一个节点的展开态带到下一个节点会让人困惑——
        // 而身份是 documentId+行段，两个节点引同一份文档时确实会撞上。
        _folding.CollapseAll();
        Document.Text = incoming;
    }

    /// <summary>
    /// 双向绑定用的文本属性。
    ///
    /// ⚠️ `TextEditor` 在 12.0.0 里**没有** `TextProperty`（只有 CLR 属性 `Text`），
    /// 所以 `Text="{Binding ...}"` 绑不上。这里补一个可绑定属性，
    /// 与 `Document.Text` 双向同步。
    /// </summary>
    public static readonly StyledProperty<string?> BoundTextProperty =
        AvaloniaProperty.Register<PromptTemplateEditor, string?>(
            nameof(BoundText),
            defaultBindingMode: Avalonia.Data.BindingMode.TwoWay);

    public string? BoundText
    {
        get => GetValue(BoundTextProperty);
        set => SetValue(BoundTextProperty, value);
    }

    /// <summary>
    /// 当前折叠投影。
    ///
    /// ⚠️ `internal` 供用例断言「默认折叠 / Ctrl 点击后展开」这条**用户可见结果**。
    /// 判据取这里而不是「`_folding` 里有没有那个 identity」：后者是对内部状态的断言，
    /// 而缺陷完全可以是「状态改了但没 Reproject」——那时屏幕上什么都没变，
    /// 只断言状态的用例照样全绿。
    /// </summary>
    internal IReadOnlyList<ReferenceFoldingState.Segment> CurrentSegments => _segments;

    /// <summary>
    /// 折叠摘要的前景色。
    ///
    /// 用 `TextSubtle`：折叠摘要是**元信息**（「这里引了 chapter-42.md 的 L120-145」），
    /// 不是作者写的话。跟正文同色会让人误读成模板内容。
    /// </summary>
    private IBrush? FoldedLabelBrush() => FindBrush("Ariadne.TextSubtle");

    /// <summary>按占位符种类取画刷。三档语义见 <see cref="PromptPlaceholderColorizer"/>。</summary>
    private IBrush? PlaceholderBrush(PromptPlaceholderSyntax.PlaceholderKind kind) => kind switch
    {
        // 合法：强调色。它在四套主题下都是「可用/已连通」的语汇。
        PromptPlaceholderSyntax.PlaceholderKind.Reference => FindBrush("Ariadne.AccentPrimary"),
        PromptPlaceholderSyntax.PlaceholderKind.KnownVariable => FindBrush("Ariadne.AccentPrimary"),
        // 待确认：次级文字色，**刻意不用警告色**。编辑期无从判断真伪，
        // 标黄同样是误报——只是比标红温和一点，代价一样（训练用户忽略颜色）。
        PromptPlaceholderSyntax.PlaceholderKind.UnverifiableVariable => FindBrush("Ariadne.TextSecondary"),
        // 确定会失败：错误色。⚠️ 没有 `Ariadne.StatusDanger` 这个 key（曾拼错过），
        // 正确名是 StatusError。
        PromptPlaceholderSyntax.PlaceholderKind.MalformedReference => FindBrush("Ariadne.StatusError"),
        PromptPlaceholderSyntax.PlaceholderKind.RejectedVariable => FindBrush("Ariadne.StatusError"),
        _ => null,
    };

    /// <summary>
    /// 从主题现取画刷。
    ///
    /// 用 `TryFindResource` 而非 `DynamicResource` 绑定：这些颜色要给
    /// AvaloniaEdit 的 `TextRunProperties` 用，那不是 AvaloniaProperty，绑不上去。
    /// **取不到时返回 null**，调用方一律「不改颜色」而不是设透明——
    /// DynamicResource 缺键时属性停在未赋值状态，强设 null 会把文字画成透明。
    /// </summary>
    private IBrush? FindBrush(string key) =>
        // `TryFindResource` 是 ResourceNodeExtensions 上的扩展方法，必须写 `this.`。
        this.TryFindResource(key, ActualThemeVariant, out var value) ? value as IBrush : null;
}
