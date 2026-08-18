using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using Avalonia.Diagnostics;
using Avalonia.Headless;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U144 / U148 / U154：输入面从「品牌色凹槽 + 四边框 + 圆角」改为**一条输入线**。
///
/// 三个编号是同一片语义的三种失效，合在一个文件里因为它们**互为对照**：
/// U144 是基础样式本身的产品问题（97 个 TextBox 一律读成表单格子），
/// U148/U154 是两种「写了覆盖但覆盖没生效」——
/// 前者作用对象错位（内联属性打不到 <c>/template/</c> 层），
/// 后者声明顺序错（Avalonia 同优先级**按文档顺序、后者胜**，
/// 没有 CSS 那种选择器特异性权重）。
///
/// ⚠️ **判据一律落在运行时实体化的模板层 <c>Border#PART_BorderElement</c> 上，
/// 不查主题标记文本。** 这是本文件唯一重要的设计决定：
/// U154 的缺陷形态恰恰是「样式写对了、位置错了」——
/// 标记断言（<c>theme.Contains("TextBox.search-input:focus …")</c>）在缺陷版本下**照样全绿**，
/// 因为那行字确实在文件里，只是被 300 行后的通用样式整个盖掉。
/// 只有去问「这个控件此刻实际拿到的 BorderThickness 是多少」才能分辨两者。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class InputSurfaceStyleTests
{
    /// <summary>
    /// U144 主用例：普通可编辑 <c>TextBox</c> 静息态**只有底线**，且无槽、无圆角。
    ///
    /// 四条断言各自不可省：
    /// - 底线存在（Bottom > 0）——线是「可编辑」的语义载体，没线等于没做
    /// - 三边归零——「一条线」的全部含义；缺陷版本是 1,1,1,1
    /// - 圆角归零——底线两端带圆角会向上翘成一对小括号，不再是一条基线
    /// - 底不铺色——槽是被拆掉的那个东西，铺回来就等于没改
    ///
    /// ⚠️ Thickness 是 left,top,right,bottom：写成 <c>0,0,1,0</c> 会得到一条**右侧竖线**，
    /// 一个字符之差、视觉上完全是另一回事，所以逐边分别断言而不是只比字符串。
    /// </summary>
    [Fact]
    public async Task EditableTextBox_AtRest_HasBottomLineOnly()
    {
        await RunWithSettingsAsync(async view =>
        {
            var border = await ResolveBorderElementAsync(view, box => !box.IsReadOnly);

            Assert.True(
                border.BorderThickness.Bottom > 0,
                "可编辑输入必须有底线：线本身承载「这里能改」的语义（U144）");
            Assert.Equal(0d, border.BorderThickness.Left);
            Assert.Equal(0d, border.BorderThickness.Top);
            Assert.Equal(0d, border.BorderThickness.Right);

            Assert.Equal(new CornerRadius(0), border.CornerRadius);

            Assert.True(
                IsVisuallyTransparent(border.Background),
                "静息态不得铺填充槽——槽正是 U144 要拆掉的那个东西，"
                + $"当前 Background = {Describe(border.Background)}");
        });
    }

    /// <summary>
    /// U144 聚焦态：线**加粗**并点亮成强调色，且仍然只有一条线（不回到四边框）。
    ///
    /// 判据取「线宽真的变了」+「渐变里出现强调色」两条：
    /// 只断言颜色会漏掉「粗细没变化 ⇒ 焦点几乎看不出来」，
    /// 只断言粗细会漏掉「加粗了但还是灰的 ⇒ 不像焦点」。
    ///
    /// ⚠️ 强调色**期望值从主题字典现取**，测试里零颜色字面量——
    /// 写死十六进制会让测试在换主题时反过来拦住正确的改动。
    /// ⚠️ 线宽刻意**没有** ThicknessTransition（见主题注释）：有补间的话
    /// 这里读到的可能是中间值，用例就退化成「等多久算够」的猜谜。
    /// </summary>
    [Fact]
    public async Task FocusedTextBox_ThickensLineAndPaintsAccent()
    {
        await RunWithSettingsAsync(async view =>
        {
            var box = await ResolveEditableTextBoxAsync(view);
            var border = ResolveBorderElement(box);
            var restThickness = border.BorderThickness.Bottom;

            Assert.True(box.Focus(), "可编辑输入必须能聚焦，否则这条用例测不到聚焦态");
            await DrainAsync();

            Assert.True(
                border.BorderThickness.Bottom > restThickness,
                $"聚焦必须把线加粗（静息 {restThickness} → 聚焦 {border.BorderThickness.Bottom}）："
                + "只变色不变粗时焦点在浅色主题下几乎看不出来");
            // 仍然只有一条线：加粗 + 变色已是足够反馈，不该退回四边框。
            Assert.Equal(0d, border.BorderThickness.Left);
            Assert.Equal(0d, border.BorderThickness.Top);
            Assert.Equal(0d, border.BorderThickness.Right);

            var accent = ResolveTokenColor(view, "Ariadne.Color.AccentPrimary");
            Assert.Contains(accent, CollectColors(border.BorderBrush));

            Assert.True(
                IsVisuallyTransparent(border.Background),
                "聚焦态同样不得铺槽（旧实现聚焦时会把 InputFill 铺回来）");
        });
    }

    /// <summary>
    /// U144 衍生语法规则：**可编辑→有线，只读→无线**。
    ///
    /// 这条是产品语义而非装饰：只读框可被 Tab 选中以复制文本，
    /// 一旦给它输入线就等于谎报「这里能改」——正是 U135 那类假控件的成因。
    ///
    /// ⚠️ Avalonia 12 **没有 `:readonly` 伪类**，主题用的是属性选择器
    /// <c>[IsReadOnly=True]</c>。照抄 CSS/WPF 的 `:readonly` 会静默匹配不上、
    /// 样式加了却毫无效果——这条用例同时钉住「选择器真的匹配上了」。
    /// </summary>
    [Fact]
    public async Task ReadOnlyTextBox_HasNoInputLine()
    {
        await RunWithSettingsAsync(async view =>
        {
            var box = new TextBox { IsReadOnly = true, Text = "backend-assigned-id" };
            var border = await AttachAndResolveAsync(view, box);

            Assert.Equal(new Thickness(0), border.BorderThickness);
            Assert.True(
                IsVisuallyTransparent(border.Background),
                "只读展示不该有任何输入位的痕迹（U144 衍生规则 / U135）");
        });
    }

    /// <summary>
    /// 只读 **且聚焦**仍然无线——顺序陷阱的直接检验。
    ///
    /// 主题里 <c>[IsReadOnly=True]:focus</c> 必须声明在通用 <c>:focus</c> **之后**才压得住。
    /// 把它挪到前面（U154 那种写法）本条立刻红，而标记断言察觉不到任何变化。
    /// </summary>
    [Fact]
    public async Task ReadOnlyTextBox_StaysLinelessWhenFocused()
    {
        await RunWithSettingsAsync(async view =>
        {
            // 只读框仍是 Tab 停靠位（可复制文本），所以它确实会进入 :focus 态——
            // 这不是构造出来的边角情形。
            var box = new TextBox { IsReadOnly = true, Text = "backend-assigned-id" };
            var border = await AttachAndResolveAsync(view, box);

            box.Focus();
            await DrainAsync();

            Assert.Equal(
                0d,
                border.BorderThickness.Bottom);
        });
    }

    /// <summary>
    /// U154：搜索框内层**全态无痕**，chrome 由外层 <c>Border.search-shell</c> 一体承担。
    ///
    /// 缺陷版本下压平写在文件约 :1510，而通用 <c>TextBox:focus /template/</c> 在约 :2026，
    /// 后者胜 ⇒ 聚焦时内层照样长出线，与外壳边框叠成双层。
    ///
    /// 静息与聚焦两态都查：漏任一态就会在那一态下重新长出线，
    /// 而「只在聚焦时才双层」恰恰是最容易漏测的形态。
    /// </summary>
    [Fact]
    public async Task SearchInput_IsLinelessInBothRestAndFocus()
    {
        await RunWithWorksAsync(projectAiTab: false, async view =>
        {
            var box = await ResolveByClassAsync(view, "search-input");
            var border = ResolveBorderElement(box);

            Assert.Equal(new Thickness(0), border.BorderThickness);

            box.Focus();
            await DrainAsync();

            Assert.Equal(
                new Thickness(0),
                border.BorderThickness);
            Assert.True(
                IsVisuallyTransparent(border.Background),
                "搜索框内层聚焦时不得铺底：外壳已经整体响应焦点（Border.search-shell:focus-within）");
        });
    }

    /// <summary>
    /// U154 配套：搜索**外壳**必须响应焦点。
    ///
    /// 上一条把内层压平，若外壳也不亮，结果是「聚焦后毫无反馈」——
    /// 那是比双层边框更严重的倒退（用户不知道焦点在哪）。
    /// 原实现只有 <c>:pointerover</c>，聚焦时外壳灰边纹丝不动。
    /// </summary>
    [Fact]
    public async Task SearchShell_TakesAccentBorderOnFocusWithin()
    {
        await RunWithWorksAsync(projectAiTab: false, async view =>
        {
            var box = await ResolveByClassAsync(view, "search-input");
            var shell = box.GetVisualAncestors()
                .OfType<Border>()
                .FirstOrDefault(candidate => candidate.Classes.Contains("search-shell"));
            Assert.NotNull(shell);

            var restBorder = shell!.BorderBrush;
            box.Focus();
            await DrainAsync();

            var accent = ResolveTokenColor(view, "Ariadne.Color.AccentPrimary");
            Assert.Contains(accent, CollectColors(shell.BorderBrush));
            // 前置：静息态本来不是强调色，否则这条用例不构成证据。
            Assert.DoesNotContain(accent, CollectColors(restBorder));
        });
    }

    /// <summary>
    /// U148：项目 AI 输入区内层无痕，焦点由外层 <c>Border.ai-composer:focus-within</c> 整框承担。
    ///
    /// 缺陷形态是**作用对象错位**：<c>ProjectAiComposer.axaml</c> 里
    /// <c>BorderThickness="0"</c> 设在 <c>TextBox</c> 自身，而主题焦点样式设在
    /// 模板内的 <c>Border#PART_BorderElement</c>——两者作用在**不同对象**上，
    /// 压根不冲突，所以谁也没盖掉谁，一次聚焦画出两条主题色线。
    ///
    /// ⚠️ 这也是为什么本文件全程读模板层：内联那个 0 在 <c>box.BorderThickness</c> 上
    /// 一直是 0，读控件自身的属性**在缺陷版本下也全绿**。
    /// </summary>
    [Fact]
    public async Task AiComposerInput_IsLinelessWhileShellCarriesFocus()
    {
        await RunWithWorksAsync(projectAiTab: true, async view =>
        {
            var shell = view.GetVisualDescendants()
                .OfType<Border>()
                .FirstOrDefault(candidate => candidate.Classes.Contains("ai-composer"));
            Assert.NotNull(shell);

            var box = shell!.GetVisualDescendants().OfType<TextBox>().FirstOrDefault();
            Assert.NotNull(box);
            var border = ResolveBorderElement(box!);

            Assert.Equal(new Thickness(0), border.BorderThickness);

            box!.Focus();
            await DrainAsync();

            Assert.Equal(
                new Thickness(0),
                border.BorderThickness);

            var accent = ResolveTokenColor(view, "Ariadne.Color.AccentPrimary");
            Assert.Contains(accent, CollectColors(shell.BorderBrush));
        });
    }

    /// <summary>
    /// U148 几何：外框 <c>MinHeight</c> 必须够装内容，否则它「从来没生效过」。
    ///
    /// 原值 128 比实需 173 小 45px——外框高度不是设计出来的、是被内容顶出来的，
    /// 这正是「外框比例巨差」的来源。
    ///
    /// ⚠️ **判据在 U164-E 之后改过一次，理由必须留档**：
    /// 原判据是「实测高度是否等于声明的 `MinHeight`」，它比「MinHeight 等于 173」好
    /// （后者是把同一个数抄进测试，改 padding 时不会红），但它有个隐含前提——
    /// **必须存在一个声明的 `MinHeight`**。
    ///
    /// U164-E 把这个前提推翻了：布局模型从「三行竖排」改成「单格叠放」，
    /// 高度只由 TextBox 的 `MinHeight="72"` 决定，外框自然包住它
    /// ⇒ **外框不该再有独立的 MinHeight**。那个数失效过两次
    /// （128 → 173 → 实需 178），根因是「把算出来的数抄进 XAML」这个修法本身：
    /// 它必须与 padding / spacing / 字号逐项同步，其中任一项变动差值就重现。
    ///
    /// 所以现在守的是**真正的性质**：框包住了输入框、且没有塌。
    /// 「外框声明了某个数」不是性质，是实现手段——而那个手段已被判定为错的。
    /// </summary>
    [Fact]
    public async Task AiComposerShell_WrapsTheInputWithoutADeclaredHeightNumber()
    {
        await RunWithWorksAsync(projectAiTab: true, async view =>
        {
            var shell = view.GetVisualDescendants()
                .OfType<Border>()
                .FirstOrDefault(candidate => candidate.Classes.Contains("ai-composer"));
            Assert.NotNull(shell);

            var input = shell!.GetVisualDescendants().OfType<TextBox>().FirstOrDefault();
            Assert.NotNull(input);

            // 性质一：外框**没有**独立声明的高度数字。
            // 这是 U164-E 的核心——不是「换了个更准的数」，而是让高度不再由
            // 一个需要人工同步的数声明。加回 MinHeight 会让这条红，那是对的。
            Assert.True(
                shell.MinHeight is 0 or double.NaN,
                $"外框又声明了 MinHeight={shell.MinHeight}。U164-E 刻意删掉了它："
                + "那个数与 padding/spacing/字号耦合，已经失效过两次（128 → 173 → 实需 178）。"
                + "要限制「输入面不要无限长高」请挂 MaxHeight，不要恢复 MinHeight。");

            // 性质二：框真的包住了输入框，没有塌成一条。
            // 这是删掉 MinHeight 之后唯一需要担心的失效形态。
            Assert.True(
                shell.Bounds.Height >= input!.Bounds.Height,
                $"外框高度 {shell.Bounds.Height} 小于输入框 {input.Bounds.Height}——框塌了。");
            Assert.True(
                input.Bounds.Height >= 72,
                $"输入框实测高度 {input.Bounds.Height} 小于它自己声明的 MinHeight=72，"
                + "说明父容器在压它——单格 Panel 不该发生这种事。");
        });
    }

    /// <summary>
    /// ComboBox 与 TextBox 用**同一套输入线**。
    ///
    /// 它的槽视觉独立定义在模板层 <c>Border#Background</c>（不是 <c>PART_BorderElement</c>），
    /// 所以改 TextBox 那一处**不会**带上它。若只改一半，设置页会出现
    /// 「输入线 + 品牌色槽 + 输入线」交替，比原来全是槽更难看——
    /// 这条钉住「可编辑控件长什么样，产品里只有一个答案」。
    ///
    /// ⚠️ 模板里还有**第二个** Border：<c>#HighlightBackground</c>，默认是
    /// **Windows 蓝 `#FF0078D7`**——一个从依赖的 Fluent 模板带进来、从未被本主题覆盖的
    /// 平台色。它在源码里 grep 不到（`0078D7` 全仓 0 命中），只有把控件实体化后
    /// **逐个数模板件**才会发现。所以这条用例遍历模板内**每一个** Border，
    /// 而不是只查已知那一个——「只查我知道的那个」正是这个漏洞活到现在的原因。
    /// </summary>
    [Fact]
    public async Task ComboBox_SharesTheSameInputLine()
    {
        await RunWithSettingsAsync(async view =>
        {
            var combo = view.GetVisualDescendants().OfType<ComboBox>().FirstOrDefault();
            Assert.NotNull(combo);
            await DrainAsync();

            var background = combo!.GetVisualDescendants()
                .OfType<Border>()
                .FirstOrDefault(border => border.Name == "Background");
            Assert.NotNull(background);

            Assert.Equal(0d, background!.BorderThickness.Left);
            Assert.Equal(0d, background.BorderThickness.Top);
            Assert.Equal(0d, background.BorderThickness.Right);
            Assert.True(background.BorderThickness.Bottom > 0, "ComboBox 也要有输入线");
            Assert.Equal(new CornerRadius(0), background.CornerRadius);
            Assert.True(
                IsVisuallyTransparent(background.Background),
                $"ComboBox 不得保留填充槽，当前 Background = {Describe(background.Background)}");

            // 模板内**任何**可见 Border 都不许铺不透明底：那必然是没被主题接管的
            // 平台默认色（`#HighlightBackground` 就是这么带进一个 Windows 蓝的）。
            var opaque = combo.GetVisualDescendants()
                .OfType<Border>()
                .Where(border => border.IsEffectivelyVisible)
                .Where(border => !IsVisuallyTransparent(border.Background))
                .Select(border => $"{border.Name ?? "(匿名)"} = {Describe(border.Background)}")
                .ToList();

            Assert.True(
                opaque.Count == 0,
                "ComboBox 模板内出现未被主题接管的不透明底——"
                + "极可能是 Fluent 默认平台色（硬约束：颜色只能来自主题）：\n  "
                + string.Join("\n  ", opaque));
        });
    }

    /// <summary>
    /// 全仓护栏：拆掉的填充槽令牌不得被任何样式重新引用。
    ///
    /// 上面的行为用例只覆盖到实际渲染出来的那几个控件；这条防的是
    /// 「在某个没被测到的页面/控件上把 InputFill 铺回去」——
    /// 槽一旦局部复活，产品里就同时存在两种输入语言，比统一用槽更糟。
    ///
    /// 令牌本身**刻意保留在字典里**（它是一枚合法的半透明墨色，
    /// 将来做代码块底纹之类仍可能用到），所以只禁 <c>Setter</c> 引用、不禁定义。
    /// </summary>
    [Fact]
    public void NoStyleReintroducesTheInputFillSlot()
    {
        var theme = File.ReadAllText(ResolveThemePath());
        var offenders = theme
            .Split('\n')
            .Select((line, index) => (line, number: index + 1))
            .Where(entry => entry.line.Contains("Ariadne.InputFill", StringComparison.Ordinal))
            // 定义处（SolidColorBrush x:Key=…）允许保留，只拦引用处。
            .Where(entry => !entry.line.Contains("x:Key", StringComparison.Ordinal))
            .Where(entry => !entry.line.Contains("Color x:Key", StringComparison.Ordinal))
            .ToList();

        Assert.True(
            offenders.Count == 0,
            "输入填充槽已在 U144 拆除，不得由样式重新引用：\n"
            + string.Join('\n', offenders.Select(entry => $"  :{entry.number} {entry.line.Trim()}")));
    }

    /// <summary>
    /// 全仓护栏：输入类控件的模板层压平样式必须声明在**它所收窄的那条通用样式**之后。
    ///
    /// 这是 U154 的**结构性**防线。上面 <c>SearchInput_*</c> 那条只覆盖搜索框一处，
    /// 而顺序陷阱会在任何新增专用覆盖上重演，且症状是「样式毫无效果」——
    /// 没有报错、没有警告，极难察觉。
    ///
    /// ⚠️ 判定的关键是**只比较真正会竞争的两条**，这里收窄了两次才得到正确判据：
    /// 1. 首版把全文件同层样式混作一批，于是
    ///    <c>NumericUpDown ButtonSpinner /template/ Border#PART_BorderElement</c>
    ///    （打同名模板件、但宿主是另一种控件）也参与比较；
    /// 2. 二版按宿主类型分了组，但仍把**伪类不同**的两条当成竞争——
    ///    <c>TextBox:disabled /template/</c> 与 <c>TextBox[IsReadOnly=True] /template/</c>
    ///    命中的是**互不相交**的元素集（禁用的框 vs 只读的框），谁先谁后毫无影响。
    ///
    /// 真正的竞争关系是「**同一伪类**下，带 class/属性限定的那条 vs 裸的那条」：
    /// 只有它们会命中同一个元素的同一属性。所以按伪类分桶比较。
    /// **分组错会让护栏反过来拦住正确的代码**——那比漏报危险得多。
    /// </summary>
    [Fact]
    public void ClassScopedTemplateOverridesComeAfterGenericOnes()
    {
        var lines = File.ReadAllText(ResolveThemePath()).Split('\n');
        // 按伪类分桶：key 是伪类串（无伪类 = 空串），value 是该桶里裸 TextBox 样式的最大行号。
        var genericByPseudo = new Dictionary<string, int>(StringComparer.Ordinal);
        var scoped = new List<(int Number, string Pseudo, string Text)>();

        for (var index = 0; index < lines.Length; index++)
        {
            var line = lines[index];
            var trimmed = line.TrimStart();
            // 只看样式声明起始行；注释里刻意写了选择器名以解释陷阱，不能算。
            if (!trimmed.StartsWith("<Style", StringComparison.Ordinal)
                && !trimmed.StartsWith("TextBox", StringComparison.Ordinal))
            {
                continue;
            }
            if (!line.Contains("/template/ Border#PART_BorderElement", StringComparison.Ordinal))
            {
                continue;
            }

            // 抽出这一行里的选择器片段（多选择器样式跨行书写，逐行判定即可：
            // 每个片段自己就是一条独立选择器）。
            var selectorStart = line.IndexOf("Selector=\"", StringComparison.Ordinal);
            var fragment = (selectorStart >= 0
                ? line[(selectorStart + "Selector=\"".Length)..]
                : trimmed).TrimStart();

            // 本条只管 TextBox 这一族：其他控件（NumericUpDown 等）各有自己的先后关系。
            if (!fragment.StartsWith("TextBox", StringComparison.Ordinal))
            {
                continue;
            }

            var head = fragment.Split(' ')[0];
            // 伪类 = head 里所有 `:xxx` 段；属性 / class 限定不算伪类。
            var pseudo = string.Concat(head.Split(':').Skip(1).Select(part => ":" + part));
            var isScoped = head.Contains('.', StringComparison.Ordinal)
                || head.Contains('[', StringComparison.Ordinal);

            if (isScoped)
            {
                scoped.Add((index + 1, pseudo, trimmed));
            }
            else if (!genericByPseudo.TryGetValue(pseudo, out var existing) || index + 1 > existing)
            {
                genericByPseudo[pseudo] = index + 1;
            }
        }

        Assert.NotEmpty(genericByPseudo);
        Assert.NotEmpty(scoped);

        // 只跟**同伪类**的裸样式比：同伪类才可能命中同一元素。
        var tooEarly = scoped
            .Where(entry => genericByPseudo.TryGetValue(entry.Pseudo, out var generic)
                && entry.Number < generic)
            .Select(entry => $"  :{entry.Number} {entry.Text}"
                + $"（同伪类「{(entry.Pseudo.Length == 0 ? "无" : entry.Pseudo)}」的裸样式在 :{genericByPseudo[entry.Pseudo]}）")
            .ToList();

        Assert.True(
            tooEarly.Count == 0,
            "Avalonia 同优先级按文档顺序、后者胜（没有 CSS 特异性权重）。"
            + "下列专用覆盖声明在同伪类的裸 TextBox 样式之前，会被完全盖掉——"
            + "样式写了等于没写，且无任何报错（U154）：\n"
            + string.Join('\n', tooEarly));
    }

    /// <summary>
    /// U160：<c>AutoCompleteBox</c>（U145 那 12 个标识输入框）的内层 <c>PART_TextBox</c>
    /// 必须与纯 <c>TextBox</c> 长得一样——静息与聚焦两态都一样。
    ///
    /// 用户原话：「有些输入框会有不同行为，例如切到其它输入框，原输入框闪白或亮起边框」。
    /// 三处实测差异逐条对应用户看到的东西：
    /// - <c>Background = #66ffffff</c>（半透明白底）→ 聚焦时被 BrushTransition 淡出 = **闪白**
    /// - <c>BorderThickness = 1,1,1,1</c>（四边框）→ 整圈亮 = **亮起边框**（纯 TextBox 只亮底线）
    /// - <c>CornerRadius = 3</c> → 圆角也不一致
    ///
    /// ⚠️ **判据取「与纯 TextBox 的实测值相等」而不是写死期望常量**：
    /// 这条用例要守的性质是「两种输入面视觉一致」，而不是「底线正好 1px」。
    /// 写死常量的话，将来有人调整基础输入线（U144 那套）就得同步改两处，
    /// 漏改一处就退回不一致——而那正是本缺陷的形态。
    /// 拿同一棵视觉树里的真 TextBox 当基准，不一致必红。
    /// </summary>
    [Fact]
    public async Task AutoCompleteBox_InnerTextBox_MatchesPlainTextBoxInBothStates()
    {
        await RunWithSettingsAsync(async view =>
        {
            var host = view.GetVisualDescendants().OfType<Panel>().FirstOrDefault();
            Assert.NotNull(host);

            var picker = new AutoCompleteBox { Text = "probe", Width = 200 };
            var plain = new TextBox { Text = "probe", Width = 200 };
            host!.Children.Add(picker);
            host.Children.Add(plain);
            await DrainAsync();

            var inner = picker.GetVisualDescendants()
                .OfType<TextBox>()
                .FirstOrDefault(candidate => candidate.Name == "PART_TextBox");
            Assert.NotNull(inner);

            // ── 静息态：四个视觉属性逐条与纯 TextBox 比 ──
            AssertInputSurfaceMatches(inner!, plain, "静息");

            // ── 聚焦态：焦点落在内层 TextBox 上（宿主只拿到 :focus-within）──
            Assert.True(inner!.Focus(), "内层 PART_TextBox 应当可获得焦点");
            await DrainAsync();
            // 底色有 0.14s 的 BrushTransition，读早了会拿到淡出中间值（正是「闪白」那一帧）。
            await Task.Delay(350);
            await DrainAsync();

            var pickerBorder = ResolveBorderElement(inner);
            Assert.Equal(new CornerRadius(0), pickerBorder.CornerRadius);
            Assert.Equal(0d, pickerBorder.BorderThickness.Left);
            Assert.Equal(0d, pickerBorder.BorderThickness.Top);
            Assert.Equal(0d, pickerBorder.BorderThickness.Right);
            Assert.True(
                pickerBorder.BorderThickness.Bottom > 0,
                "聚焦态仍要有输入线，否则用户不知道焦点在哪");
            Assert.True(
                IsVisuallyTransparent(pickerBorder.Background),
                "聚焦态铺底就是用户看到的「闪白」——"
                + $"当前 Background = {Describe(pickerBorder.Background)}");
        });
    }

    /// <summary>
    /// U160 的**机制**护栏：内层 <c>PART_TextBox</c> 的三个属性不得停留在
    /// <c>BindingPriority.Template</c> 且取 Fluent 默认值。
    ///
    /// 为什么单独立这一条、而不是靠上面那条行为用例就够：
    /// 上面那条只说「值不对」，这条说清「**为什么压不动**」，
    /// 从而拦住下一个人用错误手法去「修」它。
    ///
    /// <c>BindingPriority</c> 的数值是
    /// <c>LocalValue=0 &lt; StyleTrigger=1 &lt; Template=2 &lt; Style=3</c>，**小的胜**。
    /// AutoCompleteBox 的模板把宿主属性 <c>TemplateBinding</c> 到内层，内层因此拿 <c>Template</c>；
    /// 而全部 <c>TextBox … /template/ Border#PART_BorderElement</c> 样式是 <c>Style</c>——
    /// **输给 Template**。所以这里**不是** U154 那种「顺序错」，调位置无用；
    /// 实测证否过 <c>AutoCompleteBox /template/ TextBox#PART_TextBox</c> 与
    /// <c>AutoCompleteBox TextBox /template/ Border#PART_BorderElement</c> 两种写法。
    /// 唯一有效的解法是**把值设在宿主上**，让它顺模板自己的 TemplateBinding 流下去。
    ///
    /// 判据落在「宿主上这四个属性有 Style 来源的赋值」——那正是修复的实现方式，
    /// 摘掉主题里那条 <c>Style Selector="AutoCompleteBox"</c> 即红。
    /// </summary>
    [Fact]
    public async Task AutoCompleteBox_HostCarriesInputSurfaceTokens()
    {
        await RunWithSettingsAsync(async view =>
        {
            var host = view.GetVisualDescendants().OfType<Panel>().FirstOrDefault();
            Assert.NotNull(host);

            var picker = new AutoCompleteBox { Text = "probe", Width = 200 };
            host!.Children.Add(picker);
            await DrainAsync();

            // 宿主必须被主题接管：否则值到不了内层（内层的 Template 优先级压不动）。
            foreach (var (property, label) in new (AvaloniaProperty, string)[]
            {
                (TemplatedControl.BackgroundProperty, "Background"),
                (TemplatedControl.BorderThicknessProperty, "BorderThickness"),
                (TemplatedControl.CornerRadiusProperty, "CornerRadius"),
                (TemplatedControl.PaddingProperty, "Padding"),
                (TemplatedControl.ForegroundProperty, "Foreground"),
            })
            {
                var diagnostic = picker.GetDiagnostic(property);
                Assert.True(
                    diagnostic.Priority == Avalonia.Data.BindingPriority.Style
                    || diagnostic.Priority == Avalonia.Data.BindingPriority.StyleTrigger,
                    $"AutoCompleteBox 宿主的 {label} 必须由主题样式赋值（当前来源 "
                    + $"{diagnostic.Priority}）。宿主没被接管，值就到不了内层 PART_TextBox——"
                    + "而内层是 Template 优先级，任何 /template/ 选择器（Style 优先级）都压不动它。");
            }

            // Fluent 的默认值不得残留在内层：这几个就是用户看到的「闪白 + 整圈边框」。
            var inner = picker.GetVisualDescendants()
                .OfType<TextBox>()
                .FirstOrDefault(candidate => candidate.Name == "PART_TextBox");
            Assert.NotNull(inner);
            Assert.True(
                IsVisuallyTransparent(inner!.Background),
                $"内层残留 Fluent 半透明底 {Describe(inner.Background)}（= 用户看到的「闪白」）");
            Assert.Equal(new CornerRadius(0), inner.CornerRadius);
            Assert.Equal(0d, inner.BorderThickness.Top);
            Assert.Equal(0d, inner.BorderThickness.Left);
            Assert.Equal(0d, inner.BorderThickness.Right);

            // Foreground：Fluent 在 Template 层写死 Black/White 两个**平台色**，
            // 亮色下纯黑、暗色下纯白，都不是本主题 token（硬约束：颜色只能来自主题）。
            var expected = ResolveTokenColor(view, "Ariadne.TextPrimary");
            Assert.Equal(expected, Assert.IsAssignableFrom<ISolidColorBrush>(inner.Foreground).Color);
        });
    }

    /// <summary>
    /// U160 顺带核查：<c>ComboBox</c>（可编辑态）与 <c>NumericUpDown</c> 的内层输入框
    /// 有没有同一类残缺——即 Fluent 默认值卡在 <c>Template</c> 优先级上。
    ///
    /// 实测结论（本用例把它钉住）：**两者都没有 AutoCompleteBox 那种残缺**。
    /// 它们的内层 Background 也来自 <c>Template</c>，但值已是 <c>Transparent</c>
    /// （ComboBox 的槽由 <c>Border#Background</c> 单独承担、已被主题接管；
    /// NumericUpDown 的外层 ButtonSpinner 盒子已在上面压平），所以看不出差别。
    ///
    /// 为什么仍要立这条用例而不是只在报告里写一句「查过了，没问题」：
    /// 「当前没残缺」是个**会被改坏的结论**——任何人调 ComboBox/NumericUpDown 那几条样式
    /// 都可能把 Fluent 默认底放回来，而这种回归在视觉上就是用户报的同一个现象。
    /// </summary>
    [Fact]
    public async Task EditableComboBoxAndNumericUpDown_KeepNoFluentDefaultFill()
    {
        await RunWithSettingsAsync(async view =>
        {
            var host = view.GetVisualDescendants().OfType<Panel>().FirstOrDefault();
            Assert.NotNull(host);

            var combo = new ComboBox { Width = 200, IsEditable = true };
            var numeric = new NumericUpDown { Width = 200 };
            host!.Children.Add(combo);
            host.Children.Add(numeric);
            await DrainAsync();

            var comboInner = combo.GetVisualDescendants()
                .OfType<TextBox>()
                .FirstOrDefault(candidate => candidate.Name == "PART_EditableTextBox");
            Assert.NotNull(comboInner);
            Assert.True(
                IsVisuallyTransparent(comboInner!.Background),
                "可编辑 ComboBox 内层出现填充底（槽应由 Border#Background 单独承担）："
                + Describe(comboInner.Background));

            var numericInner = numeric.GetVisualDescendants()
                .OfType<TextBox>()
                .FirstOrDefault(candidate => candidate.Name == "PART_TextBox");
            Assert.NotNull(numericInner);
            Assert.True(
                IsVisuallyTransparent(numericInner!.Background),
                "NumericUpDown 内层出现填充底：" + Describe(numericInner.Background));
            Assert.Equal(new CornerRadius(0), numericInner.CornerRadius);
        });
    }

    /// <summary>
    /// 把「内层输入面与纯 TextBox 一致」这组比较收成一处。
    ///
    /// 基准是同一棵视觉树里的真 <c>TextBox</c>（而非写死常量）：
    /// 要守的性质是**一致性**，基础输入线将来怎么调都不该让这条用例需要同步改。
    /// </summary>
    private static void AssertInputSurfaceMatches(TextBox actual, TextBox baseline, string state)
    {
        Assert.Equal(baseline.BorderThickness, actual.BorderThickness);
        Assert.Equal(baseline.CornerRadius, actual.CornerRadius);
        Assert.Equal(baseline.Padding, actual.Padding);
        Assert.True(
            IsVisuallyTransparent(actual.Background) == IsVisuallyTransparent(baseline.Background),
            $"[{state}] 底色铺法与纯 TextBox 不一致："
            + $"AutoCompleteBox={Describe(actual.Background)} / TextBox={Describe(baseline.Background)}");
    }

    /// <summary>
    /// 拿到某个 <c>TextBox</c> 模板内的 <c>PART_BorderElement</c>。
    ///
    /// 走视觉树而不是 <c>GetTemplateChildren</c>：后者要求模板已 apply，
    /// 而视觉树查找在已 Show 的窗口里天然满足，且与用户实际看到的层级一致。
    /// </summary>
    private static Border ResolveBorderElement(TextBox box)
    {
        var border = box.GetVisualDescendants()
            .OfType<Border>()
            .FirstOrDefault(candidate => candidate.Name == "PART_BorderElement");
        Assert.NotNull(border);
        return border!;
    }

    private static async Task<TextBox> ResolveEditableTextBoxAsync(Control view)
    {
        await DrainAsync();
        var box = view.GetVisualDescendants()
            .OfType<TextBox>()
            .FirstOrDefault(candidate => !candidate.IsReadOnly
                && !candidate.Classes.Contains("search-input")
                && candidate.IsEffectivelyVisible);
        Assert.NotNull(box);
        return box!;
    }

    private static async Task<Border> ResolveBorderElementAsync(Control view, Func<TextBox, bool> filter)
    {
        await DrainAsync();
        var box = view.GetVisualDescendants()
            .OfType<TextBox>()
            .FirstOrDefault(candidate => filter(candidate)
                && !candidate.Classes.Contains("search-input")
                && candidate.IsEffectivelyVisible);
        Assert.NotNull(box);
        return ResolveBorderElement(box!);
    }

    private static async Task<TextBox> ResolveByClassAsync(Control view, string className)
    {
        await DrainAsync();
        var box = view.GetVisualDescendants()
            .OfType<TextBox>()
            .FirstOrDefault(candidate => candidate.Classes.Contains(className));
        Assert.NotNull(box);
        return box!;
    }

    /// <summary>
    /// 把临时控件挂进已渲染的视图树再取模板件。
    ///
    /// 只读 TextBox 在产品页面里不一定有实例（U135 已把它们换成 SelectableTextBlock），
    /// 但主题规则仍必须成立——**规则的正确性不该依赖恰好有人挂了它**，
    /// 否则将来有人新加一个只读框就会静默失去这条保护。
    /// 挂进真实视图（而非裸 Window）是为了拿到同一套 Styles 与主题变体。
    /// </summary>
    private static async Task<Border> AttachAndResolveAsync(Control view, TextBox box)
    {
        var host = view.GetVisualDescendants().OfType<Panel>().FirstOrDefault();
        Assert.NotNull(host);
        host!.Children.Add(box);
        await DrainAsync();
        return ResolveBorderElement(box);
    }

    /// <summary>
    /// 「视觉上没有铺底」：null / Transparent / Alpha 为 0 都算没铺。
    ///
    /// 不写成 <c>Assert.Null(Background)</c>：主题显式设了 <c>Transparent</c>
    /// （刻意如此——静息态若靠 TemplateBinding 传递，会与 :pointerover 那条走的路分叉，
    /// U148/U154 都是这种分叉的产物），两种写法视觉等价，断言不该只认一种。
    /// </summary>
    private static bool IsVisuallyTransparent(IBrush? brush) => brush switch
    {
        null => true,
        ISolidColorBrush solid => solid.Color.A == 0,
        _ => false,
    };

    private static string Describe(IBrush? brush) => brush switch
    {
        null => "null",
        ISolidColorBrush solid => solid.Color.ToString(),
        // 输入线是渐变（静息实色 + 右端收笔淡出），失败信息里必须看得见每个 stop，
        // 否则只报一句 "LinearGradientBrush" 等于没给线索。
        IGradientBrush gradient =>
            "grad(" + string.Join("|", gradient.GradientStops.Select(stop => $"{stop.Offset:0.##}:{stop.Color}")) + ")",
        _ => brush.GetType().Name,
    };

    /// <summary>
    /// 摊平画刷里的所有颜色：输入线用 <c>LinearGradientBrush</c> 承载
    /// （静息实色主体 + 右端收笔淡出），强调色是其中一个 GradientStop，
    /// 不是 <c>SolidColorBrush.Color</c>。
    /// </summary>
    private static IReadOnlyList<Color> CollectColors(IBrush? brush) => brush switch
    {
        ISolidColorBrush solid => new[] { solid.Color },
        IGradientBrush gradient => gradient.GradientStops.Select(stop => stop.Color).ToList(),
        _ => Array.Empty<Color>(),
    };

    /// <summary>期望色从主题字典现取，测试里零颜色字面量。</summary>
    private static Color ResolveTokenColor(Control view, string colorKey)
    {
        Assert.True(
            view.TryFindResource(colorKey, Application.Current?.ActualThemeVariant, out var resource),
            $"主题里找不到 {colorKey}，期望值无从取得");

        return resource switch
        {
            Color color => color,
            ISolidColorBrush brush => brush.Color,
            _ => throw new Xunit.Sdk.XunitException($"{colorKey} 不是颜色资源：{resource?.GetType().Name}"),
        };
    }

    /// <summary>
    /// 设置页宿主：它是全仓输入控件最密集的页面（40 多个自由文本/数值站点），
    /// 也是「改一处基础样式即覆盖全部」的验证场。
    ///
    /// ⚠️ 设置页按 <c>IsVisible</c> 分页，未选中的分页**不会实体化**——
    /// 必须显式选到含输入控件的那一页，否则拿到 0 个 TextBox 而用例「找不到控件」失败。
    /// </summary>
    private static async Task RunWithSettingsAsync(Func<SettingsPageView, Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var viewModel = new SettingsPageViewModel(
                DisplayNameService.LoadDefault(),
                DispatchProxy.Create<IAriadneBackendClient, SoftBackend>());
            viewModel.SelectTabForTests("models");

            var view = new SettingsPageView { DataContext = viewModel };
            var window = new Window { Width = 1280, Height = 900, Content = view };
            window.Show();
            await DrainAsync();

            try
            {
                await body(view);
            }
            finally
            {
                window.Content = null;
                window.Close();
                await DrainAsync();
            }

            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// 作品页宿主。<paramref name="projectAiTab"/> 决定右栏落在哪个标签页。
    ///
    /// ⚠️ 搜索框与 AI 输入区**互斥**：右栏是标签切换（`IsNavTreeTab` / `IsProjectAiTab`），
    /// 两者不可能同时实体化，所以不能用一个宿主同时测——首版试图这么做，
    /// 结果 composer 那两条以「找不到控件」失败，而那是最容易被误读成
    /// 「测试写错了」进而被删掉的失败形态。
    ///
    /// 两处都藏在条件渲染后面，默认状态下压根不实体化，所以先把 ViewModel 置到位：
    /// - 搜索框：`IsWorksTreeContent`（树加载完成）+ 右栏展开 + 导航树页
    /// - composer：右栏展开 + 项目 AI 页
    /// </summary>
    private static async Task RunWithWorksAsync(bool projectAiTab, Func<WorksPageView, Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var viewModel = new WorksPageViewModel(
                DisplayNameService.LoadDefault(),
                DispatchProxy.Create<IAriadneBackendClient, SoftBackend>());
            SetWorksTreeStateToContent(viewModel);
            viewModel.IsRightPanelOpen = true;
            viewModel.IsNavTreeTab = !projectAiTab;

            var view = new WorksPageView { DataContext = viewModel };
            var window = new Window { Width = 1400, Height = 900, Content = view };
            window.Show();
            await DrainAsync();
            // 判据里有实测高度（MinHeight 那条），高度只有 arrange 之后才有意义。
            window.UpdateLayout();
            await DrainAsync();

            try
            {
                await body(view);
            }
            finally
            {
                window.Content = null;
                window.Close();
                await DrainAsync();
            }

            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// 把作品页的树状态直接置成 Content，让 `IsWorksTreeContent` 那一支参与布局。
    ///
    /// 走反射而不是「喂一棵假树让它自己加载完」：后者要 mock 五六个后端方法，
    /// 而本文件测的是视觉样式，与树的内容无关——依赖越少，用例越不会因为
    /// 别处改了加载流程而无关地红掉。惯用法与 `ReadingEditingParityTests` 一致。
    /// </summary>
    private static void SetWorksTreeStateToContent(WorksPageViewModel viewModel)
    {
        var type = typeof(WorksPageViewModel);
        var stateType = type.GetNestedType("WorksTreeLoadState", BindingFlags.NonPublic)
                        ?? type.Assembly.GetTypes().First(candidate => candidate.Name == "WorksTreeLoadState");
        type.GetField("_worksTreeState", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(viewModel, Enum.Parse(stateType, "Content"));
    }

    private static string ResolveThemePath()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return Path.Combine(dir!, "Ariadne.Desktop", "Resources", "Styles", "AriadneTheme.axaml");
    }

    /// <summary>
    /// 排空调度队列。
    ///
    /// ⚠️ **不要用 `DispatcherPriority.Render`**：headless 平台没有真实渲染循环，
    /// 往 Render 优先级投递的回调可能永不执行 ⇒ `InvokeAsync` 一直不返回，
    /// 用例挂到测试框架超时（首版就是这样，单条跑 280s 都不结束，
    /// 表现像「内存不够起不来」而实际是卡死——两者的日志长得一样，极易误判）。
    ///
    /// `Background` + 一轮 `Loaded` 是本仓库其他 headless 用例的既有惯用法
    /// （见 `ReadOnlySurfaceTests`），足以让样式应用与布局完成。
    /// </summary>
    private static async Task DrainAsync()
    {
        for (var i = 0; i < 12; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Background);
        }

        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    /// <summary>
    /// 后端一律抛 <c>NotSupportedException</c>。
    ///
    /// ⚠️ **不要返回「成功的默认值」**（首版这么做，用例直接卡死、单条跑 280s 不结束）：
    /// 返回成功值会让 ViewModel 走进真实加载流程、继续等更多后端调用，
    /// 而假后端给不出它期待的形状（尤其 `IReadOnlyList&lt;T&gt;` 返回 null 时），
    /// 加载状态机就再也走不出去。
    ///
    /// 抛异常反而让 VM 立刻进错误分支、停止等待——本文件只测**视觉样式**，
    /// 页面处于错误态照样把控件实体化，样式该长什么样不受数据影响。
    /// 这是本仓库其他 headless 用例的既有惯用法（`ReadOnlySurfaceTests` 等）。
    ///
    /// ⚠️ <c>DispatchProxy</c> 要在运行时派生宿主类型，所以**不能 sealed**
    /// （否则 <c>ArgumentException: The base type cannot be sealed</c>）。
    /// </summary>
    private class SoftBackend : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}")
            {
                return false;
            }

            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
