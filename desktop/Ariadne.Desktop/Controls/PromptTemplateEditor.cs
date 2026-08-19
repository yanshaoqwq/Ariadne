using System.Collections.Generic;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using Avalonia.Input;
using Avalonia.Media;
using AvaloniaEdit;
using AvaloniaEdit.Document;
using Ariadne.Desktop.Localization;
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

    /// <summary>
    /// 引用预览状态。属于控件而非 VM：它是**呈现状态**，不该进保存的数据。
    /// </summary>
    private readonly ReferenceFoldingState _folding = new();

    /// <summary>当前投影。每次文本变化重算。</summary>
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

        // ⚠️ **这里刻意没有 ElementGenerator**（U201-A）。曾注册过一个把
        // `{{ref:...}}` 替换成折叠摘要的生成器，两态做反了——默认屏幕上看不到
        // `{{ref:` 字面量，作者就无从照抄语法写第二条引用、想改行段得先「展开」。
        // 编辑器的文本流现在**永远**显示作者写的字面量，预览另开一层。
        // 完整理由（含 FormattedTextElement 静默截断的实测）见下方预览一节的注释。

        // 高亮：注册着色器。三档颜色的语义见 PromptPlaceholderColorizer。
        // 这一层**保留**：它只改颜色、不改文本，字面量照样可读可编辑。
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
    /// 切换某个**文档偏移**处引用的预览；返回是否真的命中。
    ///
    /// # 为什么把它从点击处理器里切出来
    ///
    /// 这一段（命中测试 → 取正文 → 开浮层）才是本条的**功能**，坐标换算只是把
    /// 鼠标位置翻译成偏移。切开之后用例可以直接喂偏移，跳过一段**在 headless 下
    /// 根本不成立**的坐标数学：AvaloniaEdit 的 `TextArea` 在 headless 里
    /// **从不被 arrange**（实测 `TextArea.Bounds` 恒为 0×0，而外层 editor 是 520×320），
    /// 于是 `TextView.VisualLines` 为空、`GetPosition` 一律返回 null。
    ///
    /// ⚠️ 代价必须说清：这样切出来的用例**证明不了「鼠标点在那个字上真能算出这个偏移」**。
    /// 那半段由真机验证与 `GetPosition` 这个框架 API 自身负责。
    ///
    /// # 返回值只表示「命中」，不表示「预览开了」
    ///
    /// 这两件事必须分开：命中了但取不到正文时，浮层不开（U201-B），
    /// 但事件仍要算 Handled——否则 TextArea 会顺手把光标跳到点击处并清掉选区，
    /// 而用户只是想看一眼引用。所以「是否开成」由 <see cref="IsPreviewOpen"/> 报告。
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

        // 同一条再点一次 = 收起。走同步路径而不是先取一次正文：
        // 那会为一个即将关掉的浮层白跑一次 IPC，网络慢时点「关」还要等一会儿才关。
        if (_folding.IsPreviewOpenFor(hit))
        {
            ClosePreview();
            return true;
        }

        // 语法非法：连 document_id 都没解析出来，没有可取的正文。
        // 不开浮层，但给出**可读的原因**——它要修的是语法，静默无反应会让他以为
        // 手势没接上，而真正的问题是自己少打了一个字符。
        if (!hit.IsValid)
        {
            ShowPreviewNotice(
                hit,
                DisplayNameService.Current.Text("ui.node.prompt.preview_malformed"));
            return true;
        }

        // 取正文是异步的（一次 IPC）。这里**不 await**：点击处理器必须立刻返回，
        // 否则指针事件的 Handled 传播会等在 IPC 上（几十到几百毫秒的输入卡顿）。
        //
        // ⚠️ 但要**把这个 Task 存下来**（`PendingPreview`），不能真的丢掉。
        // 用例若只靠「把 dispatcher 队列跑几轮」来等它，就变成了拿时序赌运气：
        // 本轮实测到两次偶发失败，症状是**失败信息为空**的红——
        // 续体在会话拆掉之后才跑，断言异常没有归属的用例可挂。
        // 而偶发红比没有用例更糟：它会被当成噪音关掉，连同它守的性质一起。
        _pendingPreview = OpenPreviewAsync(hit);
        return true;
    }

    /// <summary>
    /// 上一次 Ctrl+左键触发的取正文任务；没有触发过时为已完成任务。
    ///
    /// ⚠️ `internal` 只为让用例**确定地**等到预览落定（`await PendingPreview`），
    /// 而不是靠 drain dispatcher 猜时序。生产代码不读它——
    /// 生产侧的「取完了要干什么」全写在 `OpenPreviewAsync` 里面。
    /// </summary>
    internal Task PendingPreview => _pendingPreview;

    private Task _pendingPreview = Task.CompletedTask;

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
    /// 重算引用投影。
    ///
    /// ⚠️ U201-A 之后**不再需要 `TextView.Redraw()`**：投影已不驱动任何文本替换
    /// （ElementGenerator 整个删了），文本流由 AvaloniaEdit 原生渲染、
    /// 着色由 `LineTransformers` 在每次文本变化时自行重跑。
    /// 留一个 Redraw 只会在每次按键时多刷一遍整个视图。
    /// </summary>
    private void Reproject()
    {
        _segments = _folding.Project(Document?.Text);
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

        // 外部（切换选中节点）换了模板：**关掉预览**。
        // 上一个节点的预览带到下一个节点会让人困惑——
        // 而身份是 documentId+行段，两个节点引同一份文档时确实会撞上。
        _folding.CollapseAll();
        _previewFlyout?.Hide();
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
    /// 当前呈现投影。
    ///
    /// ⚠️ `internal` 供用例断言「预览开在哪条引用上」这条**用户可见结果**。
    /// 判据取这里而不是「`_folding` 里那个字段是什么」：后者是对内部状态的断言，
    /// 而缺陷完全可以是「状态改了但浮层没开」——那时屏幕上什么都没变，
    /// 只断言状态的用例照样全绿。
    /// </summary>
    internal IReadOnlyList<ReferenceFoldingState.Segment> CurrentSegments => _segments;

    // ========================================================================
    // U201：引用预览（Ctrl+左键 → 浮层显示被引正文）
    //
    // # 为什么不做行内替换（三条独立理由，任一条都足以否掉它）
    //
    // 这里曾注册过一个 `VisualLineElementGenerator`，把 `{{ref:...}}` 在呈现层
    // 替换成一行摘要（`ReferenceFoldingElementGenerator`，U201-A 整个删掉了）。
    // 留下这份说明，因为下一个人看到「预览需求」时第一反应仍然会是
    // 「写个 ElementGenerator」——那正是被踩过两轮的那条路。
    //
    // 1. **它把编辑器变成了只读面板。** 屏幕上看不到 `{{ref:` 这几个字，作者就无从
    //    照抄语法写第二条同类引用，想把 `#L2-L3` 改成 `#L2-L5` 还得先「展开」回
    //    源码。默认只读、编辑要先解锁——而这是个**编辑器**。这是 U201-A 的本体。
    //
    // 2. **多行内容压不进一个视觉行的位置。** 被引正文动辄几行几十行，而
    //    `FormattedTextElement` 在换行处**静默截断**（实测 AvaloniaEdit 12.0.0：
    //    传 `"first\nsecond"` 得到的 `TextLine.Length` 只有 6、内容是 `"first\n"`，
    //    `second` 整段消失且不报任何错）。换 `InlineObjectElement` 能显示多行，
    //    但 `documentLength` 是占位符长度而视觉高度是 N 行 ⇒ 光标定位、选区、
    //    行号映射全部错位，症状是「点这里却选中了别处」。
    //    截断不是要绕开的障碍，它是「把 N 行压进 1 行」这个矛盾的表现。
    //
    // 3. **每条引用实体化一个控件会压垮重挂载路径。** 提示词里可以有 20 条引用，
    //    与 U159 那 5~7 秒完全同形（每节点 20 个祖先绑定，成本落在 attach/detach）。
    //
    // ⇒ 预览是「看一眼」不是「编辑」，它没有理由待在文本流里。
    //
    // # 为什么用 `Flyout` 而不新造一套浮层
    //
    // 三个现成机制都查过：
    // - `Border.glass-dialog`（画布页变量填值那种）要长在**页面**的 XAML 里，
    //   而本控件出现在两个地方（画布节点检查器、设置页审批提示词）⇒ 要接两遍，
    //   且都得改页面 VM 才能驱动开合。
    // - `ToolTip` 能程序化开（实测 headless 下 `SetIsOpen` 有效），但它是
    //   **悬停语汇**：鼠标一动就消失，而作者要在预览里读几行字。
    // - `Flyout` 由控件自己持有、`ShowAt(this)` 即开、自带 light dismiss
    //   （点别处/Esc 自动关），且主题里**已有** `FlyoutPresenter` 样式
    //   （AriadneTheme.axaml:2937，毛玻璃 + GlassBorder + 圆角 12）⇒ 观感自动一致。
    //   实测 headless 下 `ShowAt` 后内容真的被 measure（`Bounds` 非零），可断言。
    //
    // ⚠️ 单例复用同一个 `Flyout` 与同一个内容控件，**不是**每次 new：
    // 每次新建会在 light dismiss 之后留下悬挂的 popup root（U159 的同类账）。
    // ========================================================================

    /// <summary>
    /// 取被引文档正文的委托；为 null 时预览不可用。
    ///
    /// # 为什么是委托而不是直接注入 `IAriadneBackendClient`
    ///
    /// 本控件是**呈现层**，它需要的能力只有「给我这个 document_id 的正文」一条。
    /// 注入整个后端客户端会让控件能调六十个命令，而其中任何一个被顺手用上都会让
    /// 「呈现层不发起业务操作」这条边界失效。委托把可及范围收窄到恰好一条。
    ///
    /// # 为什么允许为 null
    ///
    /// 控件在**没有后端**的场合也要能用：设计器预览、单测、以及页面还没接线时。
    /// 为 null 时 Ctrl+左键给一条「预览不可用」的可读提示而不是静默无反应——
    /// 静默无反应会让用户以为手势坏了，而真正的原因是这个口子没接。
    ///
    /// ⚠️ **必须是 `StyledProperty` 而不是普通 CLR 属性**：这个委托的唯一来源是
    /// 页面 VM（后端客户端只在 VM 手里），而页面只能通过 XAML 绑定把它交进来。
    /// 普通属性在 XAML 里绑不上 —— 它曾是 `internal Func<...>` 普通属性，
    /// 于是「预览」在生产里恒显示「暂不可用」，而所有单测（直接赋值）全绿。
    /// 这是本项目反复出现的形态：能力做好了、没接到用户看得见处。
    /// </summary>
    public static readonly StyledProperty<Func<string, Task<string?>>?> DocumentTextProviderProperty =
        AvaloniaProperty.Register<PromptTemplateEditor, Func<string, Task<string?>>?>(
            nameof(DocumentTextProvider));

    public Func<string, Task<string?>>? DocumentTextProvider
    {
        get => GetValue(DocumentTextProviderProperty);
        set => SetValue(DocumentTextProviderProperty, value);
    }

    /// <summary>预览浮层。复用单例，见上方注释。</summary>
    private Flyout? _previewFlyout;

    /// <summary>浮层里显示的正文/提示文本。</summary>
    private SelectableTextBlock? _previewBody;

    /// <summary>浮层标题（引用出处：文件名 + 行段）。</summary>
    private TextBlock? _previewTitle;

    /// <summary>
    /// 预览此刻是否开着。
    ///
    /// ⚠️ 判据落在 **`Flyout.IsOpen`（真实浮层）+ 状态对象** 两者都成立上：
    /// 只看状态对象的话，「状态改了但浮层没开」这个缺陷照样全绿，
    /// 而那时用户点了 Ctrl+左键屏幕上什么都没发生。
    /// </summary>
    internal bool IsPreviewOpen =>
        _previewFlyout?.IsOpen == true && _folding.IsAnyPreviewOpen;

    /// <summary>
    /// 浮层里此刻显示的文字。用例用它区分「显示的是被引正文」
    /// 与「显示的是占位符字面量或摘要」——那正是 U201-A 做反了的地方。
    /// </summary>
    internal string? PreviewBodyText => _previewBody?.Text;

    /// <summary>浮层标题栏文字（出处）。</summary>
    internal string? PreviewTitleText => _previewTitle?.Text;

    /// <summary>
    /// 取正文 → 开浮层。**U201-B 的「有匹配才能预览」就落在这里。**
    ///
    /// 三种取不到的形态一律不开浮层、只给提示：
    /// 1. 委托为 null（没接后端）；
    /// 2. 委托返回 null 或抛异常（文档不存在、后端拒绝、IPC 断了）；
    /// 3. 语法非法（在调用方就挡掉了，走不到这里）。
    ///
    /// ⚠️ **「取到空串」不算取不到**：被引的那几行确实可以是空行，
    /// 那时显示一个空预览是如实的。把空串也当失败会让作者以为文档丢了。
    ///
    /// ⚠️ **必须捕获异常**：这是个 `async void` 语义的 fire-and-forget
    /// （调用方 `_ = OpenPreviewAsync(...)`），漏出去的异常会走
    /// `TaskScheduler.UnobservedTaskException` ⇒ 进程级崩溃或静默吞掉，
    /// 两者都比「显示一条取不到的提示」坏。
    /// </summary>
    private async Task OpenPreviewAsync(ContentReferenceSyntax.Occurrence occurrence)
    {
        var provider = DocumentTextProvider;
        if (provider is null)
        {
            ShowPreviewNotice(
                occurrence,
                DisplayNameService.Current.Text("ui.node.prompt.preview_unavailable"));
            return;
        }

        string? documentText;
        try
        {
            documentText = await provider(occurrence.DocumentId).ConfigureAwait(true);
        }
        catch (Exception)
        {
            // 具体异常不外显：文档取不到对作者是同一件事，而 IPC 的内部错误文本
            // （序列化细节、路径）对他没有可操作性，只会显得像崩溃。
            documentText = null;
        }

        if (documentText is null)
        {
            ShowPreviewNotice(
                occurrence,
                DisplayNameService.Current.Format(
                    "ui.node.prompt.preview_missing",
                    new Dictionary<string, string> { ["document"] = occurrence.DocumentId }));
            return;
        }

        // 切出这条引用指的那一段。整篇塞进浮层等于没回答「引的是哪几行」。
        var body = ReferenceFoldingState.SliceForPreview(documentText, occurrence);
        if (!_folding.OpenPreview(occurrence, body))
        {
            return;
        }

        Reproject();
        ShowPreviewFlyout(_folding.OpenPreviewBody ?? string.Empty, PreviewTitleFor(occurrence));
    }

    /// <summary>
    /// 显示一条「预览开不出来」的提示。
    ///
    /// **不是静默无反应**：作者按了 Ctrl+左键，屏幕上必须有反应，否则他会以为
    /// 手势坏了并反复点击，而真正的原因（文档不存在 / 语法写错 / 没接后端）
    /// 只有这条提示说得出来。
    ///
    /// ⚠️ 提示走同一个浮层但**不进入展开态**（不调 `OpenPreview`）：
    /// U201-B 要求「取不到正文就不可展开」，而「浮层里显示一行提示」与
    /// 「预览已打开」是两回事——`IsPreviewOpen` 因此仍为 false，
    /// 再点一次同一条会重新去取而不是被当成「收起」。
    /// </summary>
    private void ShowPreviewNotice(
        ContentReferenceSyntax.Occurrence occurrence,
        string notice)
    {
        _folding.ClosePreview();
        Reproject();
        ShowPreviewFlyout(notice, PreviewTitleFor(occurrence));
    }

    /// <summary>浮层标题：合法引用给出处，非法引用给原因。</summary>
    private static string PreviewTitleFor(ContentReferenceSyntax.Occurrence occurrence) =>
        occurrence.IsValid
            ? ReferenceFoldingState.IdentityOf(occurrence)
            : occurrence.ParseError ?? occurrence.Raw;

    /// <summary>
    /// 开浮层（懒建一次，之后复用）。
    ///
    /// ⚠️ 内容控件用 `SelectableTextBlock` 而不是 `TextBox`：预览是**只读**的，
    /// 而只读内容不得由 `TextBox` 承载（U135：主题里没有 `:readonly` 样式，
    /// 它会像输入框一样亮起、抢焦点，但一个字也打不进去）。
    /// `SelectableTextBlock` 保留了「能选中复制那几行」这个真实需求。
    ///
    /// ⚠️ `ShowAt(this)` 锚在编辑器整体而不是点击处：headless 下拿不到有效坐标
    /// （`TextArea` 从不被 arrange），而真机上锚在控件边缘也够用——
    /// 浮层不该盖住作者正在读的那一行，锚在点击处恰好会。
    /// </summary>
    private void ShowPreviewFlyout(string body, string title)
    {
        if (_previewFlyout is null)
        {
            _previewTitle = new TextBlock
            {
                // 出处是元信息不是正文，用次级色区分；主题查不到就不改色（见 FindBrush）。
                Foreground = FindBrush("Ariadne.TextSubtle"),
                TextWrapping = TextWrapping.Wrap,
            };
            _previewBody = new SelectableTextBlock
            {
                TextWrapping = TextWrapping.Wrap,
                // 被引的是**正文**，用阅读字体（衬线），与作品页正文同一语汇。
                FontFamily = FindFont("Ariadne.Font.Reading") ?? FontFamily,
            };
            _previewFlyout = new Flyout
            {
                Placement = PlacementMode.BottomEdgeAlignedLeft,
                Content = new ScrollViewer
                {
                    // 被引正文可以有几十行。给上限 + 滚动，而不是让浮层长过屏幕。
                    MaxHeight = 260,
                    MaxWidth = 420,
                    Content = new StackPanel
                    {
                        Spacing = 6,
                        Children = { _previewTitle, _previewBody },
                    },
                },
            };
        }

        _previewTitle!.Text = title;
        _previewBody!.Text = body;
        _previewFlyout.ShowAt(this);
    }

    /// <summary>关掉预览：状态与浮层**一起**关。</summary>
    ///
    /// <remarks>
    /// 两者必须同批：只关状态会留一个开着的浮层（内容还是上一条引用的正文，
    /// 而作者以为它反映当前状态）；只关浮层会让状态以为还开着，
    /// 于是下次 Ctrl+左键同一条被当成「收起」而什么都不显示。
    /// </remarks>
    private void ClosePreview()
    {
        _folding.ClosePreview();
        _previewFlyout?.Hide();
        Reproject();
    }

    /// <summary>
    /// 从主题取字体族。与 <see cref="FindBrush"/> 同一理由：
    /// 这些值要给非 AvaloniaProperty 的地方用，绑不上去；取不到就返回 null 让调用方保持原样。
    /// </summary>
    private FontFamily? FindFont(string key) =>
        this.TryFindResource(key, ActualThemeVariant, out var value) ? value as FontFamily : null;

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

