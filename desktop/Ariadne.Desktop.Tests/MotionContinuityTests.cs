using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U178-D/E/F + U164-B：三处「硬切」的结构护栏 + loading 基元。
///
/// **判据一律是结构断言（属性上挂了 Transition、且 Duration 落在 120–180ms），
/// 绝不测真实帧率或耗时。** 这不是偷懒，是这台机器上唯一诚实的做法：
/// 本机常有 4–6 个 agent 并发跑 dotnet，实测负载在 1.0 到 6.5 之间摆动，
/// 同一条用例的耗时能差一个数量级 ⇒ 基于时间的判据会随机红/绿，
/// 那种用例的信息量是零，还会训练人「红了就重跑」。
///
/// 120–180ms 这个区间不是我定的，是**既有代码的既成事实**：
/// 全仓 74 处过渡里 67 处（91%）落在其中，无一超 300ms。
/// 新增过渡必须落在同一区间，否则同一个产品里会出现两种节奏。
/// 关闭动作取入场的 0.8 倍（桌面惯例：决定已做完，多余等待读作卡顿）。
///
/// ⚠️ **本文件刻意不断言「动效好看」**——那不可判定。
/// 它断言的是「承载动效的那个属性上确实挂了过渡」，
/// 因为这类缺陷的真实形态就是**属性上什么都没挂**（U178 原报告实扫的结论：
/// SidebarWidth 绑到 Border.Width 而 Width 上没有 DoubleTransition）。
/// </summary>
public sealed class MotionContinuityTests
{
    /// 既有过渡尺度的下界/上界（毫秒）。取自全仓实扫，不是拍脑袋。
    private const double MinMs = 120;
    private const double MaxMs = 180;

    private static string ResolveSolutionDir()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return dir!;
    }

    private static string DesktopRoot() =>
        Path.Combine(ResolveSolutionDir(), "Ariadne.Desktop");

    private static string ReadTheme() =>
        File.ReadAllText(Path.Combine(
            DesktopRoot(), "Resources", "Styles", "AriadneTheme.axaml"));

    private static string ReadView(string name) =>
        File.ReadAllText(Path.Combine(DesktopRoot(), "Views", name));

    /// <summary>
    /// 把 <c>Duration="0:0:0.14"</c> 解析成毫秒。
    /// 单独抽出来是因为三条用例都要用，且**解析失败必须当场失败**——
    /// 若正则跑空而返回 null，调用方的 Assert 会变成「没有任何过渡也算通过」。
    /// </summary>
    private static double ParseDurationMs(string durationAttr)
    {
        var m = Regex.Match(durationAttr, @"Duration=""(\d+):(\d+):([\d.]+)""");
        Assert.True(m.Success, $"无法解析 Duration：{durationAttr}");
        var h = double.Parse(m.Groups[1].Value);
        var min = double.Parse(m.Groups[2].Value);
        var sec = double.Parse(m.Groups[3].Value);
        return ((h * 60 + min) * 60 + sec) * 1000;
    }

    /// <summary>
    /// 取出某个 Style 选择器块的正文（到下一个同级 <c>&lt;Style</c> 或文件尾）。
    /// 判据落在**块内**而不是整份文件：全文搜 "DoubleTransition Property=\"Width\""
    /// 会被任何别处的同名过渡满足，那样的用例摘掉目标修复后照样绿。
    ///
    /// 选择器按**空白折叠后**匹配：多类型选择器在 XAML 里是换行 + 缩进写的
    /// （`Selector="A.x,\n   B.x"`），按原文精确匹配会找不到。
    /// </summary>
    private static string StyleBlock(string xaml, string selector)
    {
        var flat = Regex.Replace(xaml, @"\s+", " ");
        var needle = Regex.Replace($"Selector=\"{selector}", @"\s+", " ");
        var start = flat.IndexOf(needle, StringComparison.Ordinal);
        Assert.True(start >= 0, $"主题里找不到选择器 {selector}");
        var end = flat.IndexOf("<Style ", start + 1, StringComparison.Ordinal);
        return end < 0 ? flat[start..] : flat[start..end];
    }

    /// <summary>
    /// **U178-D 主用例**：折叠侧栏的 160px 宽度变化必须是过渡，不是硬跳。
    ///
    /// 缺陷形态（原报告实扫）：`SidebarWidth` 返回 `224 : 64` 直接绑到
    /// `Border.Width`，而 `Width` 上没有 `DoubleTransition` ⇒ 一帧跳完 160px。
    ///
    /// ⚠️ **判据取「绑 SidebarWidth 的 Border 全部挂 app-rail」而不是「至少一个」。**
    /// 这不是防御性写法，是实测踩出来的：MainWindow 里有**两个** Border 绑
    /// 同一个 SidebarWidth（顶部品牌轨 + 下方导轨）。只给一个加过渡的症状**更差**——
    /// 两条本该同宽的竖轨在 180ms 里宽度不一致，接缝处出现明显错位台阶。
    /// 只断言「至少一个」的用例会给这个半成品放绿灯。
    /// </summary>
    [Fact]
    public void SidebarWidth_AnimatesInsteadOfJumping()
    {
        var view = ReadView("MainWindow.axaml");

        // 靠「绑了 SidebarWidth」定位，不靠行号——行号会随别人的改动漂移，
        // 而这个绑定是这些控件的身份。
        var bound = Regex.Matches(view, @"<Border\b[^>]*?Width=""\{Binding SidebarWidth\}""");
        Assert.True(bound.Count >= 2, $"预期至少 2 个 Border 绑 SidebarWidth，实测 {bound.Count}");
        foreach (Match m in bound)
        {
            Assert.Contains("app-rail", m.Value);
        }

        var block = StyleBlock(ReadTheme(), "Border.app-rail\"");
        var widthTransition = Regex.Match(
            block, @"<DoubleTransition\s+Property=""Width""[^>]*>");
        Assert.True(
            widthTransition.Success,
            "Border.app-rail 必须给 Width 挂 DoubleTransition —— "
                + "缺陷形态就是「Width 上什么都没挂」，160px 一帧跳完、图标瞬移。"
                + $"当前样式块：\n{block}");

        var ms = ParseDurationMs(widthTransition.Value);
        Assert.InRange(ms, MinMs, MaxMs);
    }

    /// <summary>
    /// U178-D 配套：展开/折叠两套内容原先 `IsVisible` 互斥硬切——
    /// 宽度即便平滑了，文字仍会在某一帧凭空出现/消失。
    ///
    /// 判据取「互斥面**全部**挂了 rail-face」。实测共 8 个，分三种元素类型
    /// （`Grid` 6 个 = MainWindow 4 + App 2、`StackPanel` 1 个、`ctl:BrandLogo` 1 个），
    /// 分布在 MainWindow.axaml 与 App.axaml 两个文件。
    /// 类型这件事必须查：Avalonia 的选择器要带类型前缀（裸 `.rail-face` 报 AVLN2200），
    /// 漏掉一个类型 ⇒ 那个面的样式静默不匹配、瞬现如故。
    /// 我第一版把总数写成 4（只数了 Grid、还漏了 App.axaml），基线就红——
    /// **这正是这条断言存在的意义**：它逼人去数清楚，而不是凭印象。
    /// </summary>
    [Fact]
    public void SidebarFaces_AllCarryFadeClass()
    {
        var theme = ReadTheme();
        var block = StyleBlock(theme, @"Border.app-rail Grid.rail-face,");
        var opacity = Regex.Match(block, @"<DoubleTransition\s+Property=""Opacity""[^>]*>");
        Assert.True(opacity.Success, $"rail-face 必须挂 Opacity 过渡。当前：\n{block}");
        Assert.InRange(ParseDurationMs(opacity.Value), MinMs, MaxMs);

        // 每个「靠 SidebarExpanded / SidebarCollapsed 控制显隐」的元素都要挂 rail-face，
        // 且它的元素类型必须在选择器里被列出——否则样式静默不生效。
        var seen = 0;
        foreach (var path in new[]
        {
            Path.Combine(DesktopRoot(), "Views", "MainWindow.axaml"),
            Path.Combine(DesktopRoot(), "App.axaml"),
        })
        {
            var text = File.ReadAllText(path);
            foreach (Match m in Regex.Matches(
                text,
                @"<(?<tag>[\w:]+)\b[^>]*?IsVisible=""\{Binding Sidebar(Expanded|Collapsed)\}""[^>]*>"))
            {
                seen++;
                var tag = m.Groups["tag"].Value;
                Assert.Contains("rail-face", m.Value);

                // 选择器里必须出现这个元素类型（XAML 的 `ctl:X` 在选择器里写作 `ctl|X`）。
                var selectorType = tag.Replace(':', '|');
                Assert.Contains($"{selectorType}.rail-face", block);
            }
        }

        // 前置：真的扫到了那 8 个面。扫到 0 个时上面的 foreach 一次都不执行，
        // 用例会「因为什么都没检查」而通过——这正是空测的典型形态。
        Assert.Equal(8, seen);
    }

    /// <summary>
    /// **U178-E 主用例**：全局弹窗必须有进场动效，且关闭比打开快。
    ///
    /// 缺陷形态：遮罩 `IsVisible="{Binding Dialog.IsOpen}"` 直接翻，
    /// `Border.glass-dialog` 样式只有 Background/Border/Shadow、**无 Transitions**
    /// ⇒ 一个 `ExperimentalAcrylicBorder` 从无到有硬闪。
    ///
    /// ⚠️ **第一条断言是「遮罩不再用 IsVisible 承载开合」，这条最容易被改回去。**
    /// `IsVisible=false` 的控件在 Avalonia 12 里根本不参与渲染
    /// （MeasureCore/ArrangeCore 整个包在 `if (IsVisible)` 里），
    /// 所以只要 IsVisible 还绑在 Dialog.IsOpen 上，**退场过渡在物理上播不出来**——
    /// 那时即便主题里挂满了 Transition，用户看到的仍是硬切。
    /// 只断言「主题里有 Transition」的用例会给那个状态放绿灯。
    /// </summary>
    [Fact]
    public void GlobalDialog_FadesInAndClosesFaster()
    {
        var view = ReadView("MainWindow.axaml");
        var scrimStart = view.IndexOf("x:Name=\"DialogScrim\"", StringComparison.Ordinal);
        Assert.True(scrimStart >= 0, "MainWindow 里找不到 DialogScrim");
        var scrimTag = view[scrimStart..view.IndexOf('>', scrimStart)];

        Assert.DoesNotContain("IsVisible=\"{Binding Dialog.IsOpen}\"", scrimTag);
        Assert.Contains("dialog-scrim", scrimTag);
        Assert.Contains("Classes.dialog-open=\"{Binding Dialog.IsOpen}\"", scrimTag);

        var theme = ReadTheme();

        // 入场（.dialog-open 那一侧）与退场（基础态那一侧）各自的时长。
        var openScrim = DurationOf(theme, "Border.dialog-scrim.dialog-open", "Opacity");
        var closeScrim = DurationOf(theme, "Border.dialog-scrim\"", "Opacity");
        var openPanel = DurationOf(theme, "ContentControl.dialog-panel.dialog-open", "Opacity");
        var closePanel = DurationOf(theme, "ContentControl.dialog-panel\"", "Opacity");

        foreach (var (name, ms) in new[]
        {
            ("遮罩入场", openScrim), ("面板入场", openPanel),
        })
        {
            Assert.InRange(ms, MinMs, MaxMs);
        }

        // **关闭取入场的 0.8 倍**：桌面惯例——决定已经做完，多余的等待读作卡顿。
        // 用 0.8 的相对关系断言而不是写死 128/112：改入场时长时这条自动跟随，
        // 写死数字的用例会在下次调节奏时变成「拦住正确实现」的绊脚石。
        Assert.Equal(openScrim * 0.8, closeScrim, precision: 6);
        Assert.Equal(openPanel * 0.8, closePanel, precision: 6);

        // 面板还要有 0.96→1.0 的缩放（「从中间长出来」的那一半观感）。
        var panelOpen = StyleBlock(theme, "ContentControl.dialog-panel.dialog-open");
        Assert.Contains("TransformOperationsTransition Property=\"RenderTransform\"", panelOpen);
        Assert.Contains("scale(1)", panelOpen);
        var panelRest = StyleBlock(theme, "ContentControl.dialog-panel\"");
        Assert.Contains("scale(0.96)", panelRest);

        // 缓动必须是既有语汇。过冲曲线（BackEaseOut/ElasticEaseOut）刻意不用在弹窗上：
        // 确认弹窗常承载破坏性操作，过冲会让它显得轻浮。
        Assert.Contains("CubicEaseOut", panelOpen);
    }

    /// <summary>
    /// 从某个样式块里取出指定属性的过渡时长（毫秒）。
    /// 找不到就当场失败——返回 0 会让 InRange 断言变成「没挂过渡也算通过」。
    /// </summary>
    private static double DurationOf(string theme, string selector, string property)
    {
        var block = StyleBlock(theme, selector);
        var m = Regex.Match(
            block, $@"<\w+Transition\s+Property=""{property}""[^>]*>");
        Assert.True(m.Success, $"{selector} 上没有 {property} 的过渡。块内容：\n{block}");
        return ParseDurationMs(m.Value);
    }

    /// <summary>
    /// **U178-F / U164-B 主用例**：`Expander` 必须有项目侧样式。
    ///
    /// 缺陷形态（两份报告都实扫过）：全项目 14 个 Expander
    /// （SettingsPageView 8 / WorksPageView 5 / GitPageView 1），
    /// 而 `AriadneTheme.axaml` 里 grep `Expander` 命中 **0** ⇒ 全走 FluentTheme 默认，
    /// 与项目其余控件（节点卡 / settings-section / chip / detail-row）语汇完全不同。
    ///
    /// ⚠️ **判据落在「资源键被覆盖」而不是「有 /template/ 选择器」**，
    /// 这是探针实测决定的：Fluent 的 Expander 内部部件
    /// （`Border#ToggleButtonBackground`、`Border#ExpanderContent`）
    /// 的 Background/BorderBrush/Padding 全部处在 `Template` 优先级（=2），
    /// 而 Style 是 3，**数值小者胜 ⇒ `/template/` 选择器改不动它们**。
    /// 唯一有效的覆盖路径是改 TemplateBinding 的**来源**，即那些资源键。
    /// 若有人把这套改回 `/template/` 写法，它会编译通过、运行时静默失效，
    /// 而这条用例会红。
    /// </summary>
    [Fact]
    public void Expander_HasProjectSideStyling()
    {
        var theme = ReadTheme();

        // 覆盖 Fluent 资源键。**用 XDocument 解析，不用正则**：
        // 正则解析 XAML 是本项目反复出事的地方（死代码扫描器已因正则错误自伤两次，
        // 把有真实调用者的函数报成死代码）。这里第一版正则 `[^/]*?/>` 就立刻出错——
        // 它跨过了行尾去匹配下一个元素，把相邻键的值算到了当前键上。
        var doc = System.Xml.Linq.XDocument.Load(Path.Combine(
            DesktopRoot(), "Resources", "Styles", "AriadneTheme.axaml"));
        var keyName = System.Xml.Linq.XName.Get(
            "Key", "http://schemas.microsoft.com/winfx/2006/xaml");
        // ⚠️ 只收 `Expander*` 键，不能对全表 ToDictionary：
        // 亮/暗两套 ThemeDictionaries 会给同一个 `Ariadne.Color.*` 各定义一次，
        // 全表建字典必抛「重复键」（实测 Ariadne.Color.WindowBase 撞上）。
        // Expander 覆盖是主题无关的单份定义，所以按前缀筛完再建字典是安全的——
        // 顺带用 Single 钉住「只有一份」：出现第二份就是有人在别处又覆盖了一遍，
        // 那种分叉正是 U152 反复出现的形态。
        var byKey = doc.Descendants()
            .Select(e => (Element: e, Key: e.Attribute(keyName)?.Value))
            .Where(x => x.Key is not null
                && x.Key.StartsWith("Expander", StringComparison.Ordinal))
            .GroupBy(x => x.Key!, StringComparer.Ordinal)
            .ToDictionary(g => g.Key, g => g.Single().Element, StringComparer.Ordinal);

        // 会随主题变的颜色**必须**走 Ariadne.* token（AGENTS.md 硬约束）：
        // Fluent 的默认值实测是硬编码（#fff2f2f2 / Black），暗色主题下必然错。
        foreach (var key in new[]
        {
            "ExpanderHeaderBackground",
            "ExpanderHeaderBackgroundPointerOver",
            "ExpanderHeaderForeground",
            "ExpanderHeaderForegroundPointerOver",
            "ExpanderChevronForeground",
            "ExpanderChevronForegroundPointerOver",
        })
        {
            Assert.True(byKey.ContainsKey(key), $"主题必须覆盖 Fluent 资源键 {key}");
            var color = byKey[key].Attribute("Color")?.Value;
            Assert.NotNull(color);
            Assert.Contains("DynamicResource Ariadne.", color!);
        }

        // 这几个**刻意是 Transparent**，不是漏了 token：
        // 去掉内容区底色与描边正是「不画盒子」那条既有语汇（Expander 常成组堆叠，
        // 每个都带底色+边会立刻堆成田字格）。断言写成「必须是 Transparent」
        // 而不是「必须是 token」，否则这条用例会反过来逼人把盒子加回来。
        foreach (var key in new[]
        {
            "ExpanderHeaderBorderBrush",
            "ExpanderContentBackground",
            "ExpanderContentBorderBrush",
            "ExpanderChevronBackground",
            "ExpanderChevronBorderBrush",
        })
        {
            Assert.True(byKey.ContainsKey(key), $"主题必须覆盖 Fluent 资源键 {key}");
            Assert.Equal("Transparent", byKey[key].Attribute("Color")?.Value);
        }

        // 头部 hover 反馈：过渡挂在**宿主** ToggleButton 上（实测它自己的
        // Background/Foreground 是 Style 优先级，打得到）。
        var header = StyleBlock(theme, "Expander /template/ ToggleButton#ExpanderHeader\"");
        var bg = Regex.Match(header, @"<BrushTransition\s+Property=""Background""[^>]*>");
        Assert.True(bg.Success, $"Expander 头部必须有 Background 过渡。块：\n{header}");
        Assert.InRange(ParseDurationMs(bg.Value), MinMs, MaxMs);

        // ⚠️ chevron 的选择器**必须穿两层 /template/**：它不在 Expander 的模板里，
        // 而在 ToggleButton#ExpanderHeader 自己的模板里（Fluent 给那个 ToggleButton
        // 挂了独立 ControlTheme）。单层选择器编译通过但运行时静默不匹配——
        // 实测症状是 Width 停在 NaN、Transitions 为空。
        Assert.Contains(
            "Expander /template/ ToggleButton#ExpanderHeader /template/ Path#ExpandCollapseChevron",
            Regex.Replace(theme, @"\s+", " "));
    }

    /// <summary>
    /// U164-B 顺带清理：`Expander.inspector-expander` 是死样式，必须已删。
    ///
    /// 判据取「这个类名在全仓一次都不出现」——包括样式定义侧。
    /// 只查「有没有控件挂它」是不够的：死样式的危害在于**冒充已完成的工作**
    /// （U151 实证：死样式里设了 LineHeight=30，让人以为行高已统一），
    /// 所以样式本身也必须消失，而不是留着等下一个人误读。
    /// </summary>
    [Fact]
    public void DeadInspectorExpanderStyle_IsGone()
    {
        foreach (var file in Directory.EnumerateFiles(
            DesktopRoot(), "*.axaml", SearchOption.AllDirectories))
        {
            if (file.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}")
                || file.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}"))
            {
                continue;
            }

            var text = File.ReadAllText(file);
            // 剔注释：注释里写「这里曾有一条 inspector-expander，已删」是历史记录，
            // 不是死样式本身。不剔会让「如实记录为什么删」反过来触发失败。
            var body = Regex.Replace(text, "<!--.*?-->", " ", RegexOptions.Singleline);
            Assert.DoesNotContain("inspector-expander", body);
        }
    }

    /// <summary>
    /// **U178-F loading 基元**：`BusyDots` 必须真的被挂在某个加载态上。
    ///
    /// 立项背景是一次全仓实测：`ProgressRing` / `IsIndeterminate` / `Skeleton` /
    /// `Shimmer` 命中数**全为 0**，唯一的 `ProgressBar` 是顶栏预算条（那是量表，
    /// 不是加载指示）⇒ 耗时期间用户能看到的唯一变化是一行文字换了内容。
    ///
    /// ⚠️ **判据取「有真实挂载点」而非「控件存在」**：一个没人用的基元
    /// 与不存在的基元对用户等价，而且更糟——它会冒充「已完成的工作」
    /// （U151/U152 的死样式就是这个形态）。所以这条断言查的是视图里的引用。
    /// </summary>
    [Fact]
    public void BusyDots_IsMountedOnARealLoadingState()
    {
        var mounts = new List<string>();
        foreach (var file in Directory.EnumerateFiles(
            DesktopRoot(), "*.axaml", SearchOption.AllDirectories))
        {
            if (file.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}")
                || file.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}"))
            {
                continue;
            }

            var body = Regex.Replace(
                File.ReadAllText(file), "<!--.*?-->", " ", RegexOptions.Singleline);
            if (body.Contains("BusyDots", StringComparison.Ordinal))
            {
                mounts.Add(Path.GetFileName(file));
            }
        }

        Assert.NotEmpty(mounts);

        // 挂载处必须把 IsActive 绑到某个 loading 状态上，而不是写死 True——
        // 写死 True 的指示器会**永远在转**，那比没有指示器更糟：
        // 它把「正在进行」这个信号的信息量降到零。
        var worksView = ReadView("WorksPageView.axaml");
        var tag = Regex.Match(worksView, @"<ctl:BusyDots\b[^>]*>");
        Assert.True(tag.Success, "WorksPageView 应挂 BusyDots（总结加载态）");
        Assert.Matches(@"IsActive=""\{Binding \w*(Loading|Busy)\w*\}""", tag.Value);
    }

    /// <summary>
    /// U178-D/E/F 的「减少动态效果」门控：勾上该偏好后，本轮新加的过渡必须**真的**消失。
    ///
    /// ⚠️ **这条是唯一的运行时用例，其余全是结构断言**，因为它问的问题
    /// 结构断言答不了：覆盖样式究竟有没有赢。文本层面看不出胜负——
    /// 缺陷形态是**样式写对了但没生效**（U154 同型），那时标记断言照样全绿。
    ///
    /// 三段式断言（关 → 开 → 再关）而不是只查开启态：
    /// 若覆盖样式写成了无条件生效，「开启态为空」照样成立，
    /// 只有「摘掉偏好后过渡回来了」才能区分「门控生效」与「过渡被永久删掉」。
    ///
    /// ⚠️⚠️ **`session.Dispatch` 必须用有返回值的重载（`Func&lt;Task&lt;T&gt;&gt;`）。**
    /// 这是变异测试当场抓出来的：第一版用 `Func&lt;Task&gt;` 重载，
    /// 断言失败**被静默吞掉**——我把选择器类名改成压根不存在的
    /// `reduce-motion-NOPE`，用例照样绿；再往里插一句
    /// `Assert.Fail("PROBE")`，**仍然绿**。那一版是彻底的空测。
    /// 既有的 `ReadingEditingParityTests.RunHeadlessAsync` 用的就是
    /// `async () => { await body(); return true; }`，照它写才能把异常带出来。
    /// </summary>
    [Fact]
    public async Task ReduceMotion_ActuallyStripsTheNewTransitions()
    {
        using var session = Avalonia.Headless.HeadlessUnitTestSession.StartNew(typeof(App));
        await session.Dispatch(
            async () =>
            {
                var rail = new Avalonia.Controls.Border();
                rail.Classes.Add("app-rail");
                var window = new Avalonia.Controls.Window
                {
                    Width = 600,
                    Height = 400,
                    Content = rail,
                };
                window.Show();
                await Settle();

                // 基线：默认（未开偏好）时过渡在。
                Assert.NotNull(rail.Transitions);
                Assert.NotEmpty(rail.Transitions!);

                // 开启偏好 ⇒ 过渡被置空。
                window.Classes.Add("reduce-motion");
                await Settle();
                Assert.NotNull(rail.Transitions);
                Assert.Empty(rail.Transitions!);

                // 摘掉偏好 ⇒ 过渡回来。这一步区分「门控生效」与「过渡被永久删掉」。
                window.Classes.Remove("reduce-motion");
                await Settle();
                Assert.NotEmpty(rail.Transitions!);

                window.Close();
                return true;
            },
            CancellationToken.None);
    }

    private static Task Settle() =>
        Avalonia.Threading.Dispatcher.UIThread
            .InvokeAsync(() => { }, Avalonia.Threading.DispatcherPriority.Loaded)
            .GetTask();

    /// <summary>
    /// 门控的**接线**用例：Window 上必须真的有人挂 <c>reduce-motion</c> 这个类。
    ///
    /// 与上面那条运行时用例互补、都不可省：上面那条自己往 Window 上加类，
    /// 所以**即便生产代码从没挂过这个类，它也全绿**——这正是 U108/U114/U117
    /// 那类「实现完整 + 有测试覆盖 + 生产零调用者」缺陷的形态。
    /// 这条查的是生产侧的发射点。
    /// </summary>
    [Fact]
    public void ReduceMotionClass_IsWiredFromTheMotionPreference()
    {
        var code = File.ReadAllText(Path.Combine(
            DesktopRoot(), "Views", "MainWindow.axaml.cs"));
        Assert.Contains("Classes.Set(\"reduce-motion\", MotionPreferences.ReduceMotion)", code);

        // 还必须订阅变更：只在构造时挂一次的话，用户在设置页改完偏好要重启才生效。
        Assert.Contains("MotionPreferences.Changed +=", code);
    }
}
