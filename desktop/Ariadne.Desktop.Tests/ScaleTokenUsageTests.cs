using System.Globalization;
using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U204：**尺度维度的第一道守卫**。
///
/// <para>
/// 项目已有 <see cref="ThemeStyleUsageTests"/>，但它的 6 条断言全在查
/// 「资源键引用是否有定义」与「颜色属性只绑颜色 token」——
/// <b>没有任何一条断言字号 / 圆角 / 描边必须走 token</b>。
/// 后果是页面文件里 100 多处尺寸字面量**零拦截**：
/// 修复前 <c>FontSize</c> 在 Views/Controls/App 下的 token 引用率是 1/46、
/// <c>BorderThickness</c> 是 0/72 —— token 看起来是个体系，实际是个摆设
/// （改 <c>Ariadne.Size.Body</c> 只影响主题里那几处，页面上纹丝不动）。
/// </para>
///
/// <para>
/// 这与 U200 同型：**守住了颜色，看起来像守住了整个视觉体系。**
/// </para>
///
/// <para>
/// ⚠️ 本文件的判据一律取「<b>计数上限 + 反向断言</b>」而不是「存在性」：
/// 存在性断言在「新写一处字面量」时照样全绿，而这条债的形态恰恰是
/// 「每次改动都会新增一处不一致」。反向断言（数量下降时强制下调基线）
/// 抄 <see cref="DisplayNameJsonTests"/> 那套 —— 那条用例已经证明这个模式
/// 管得住漂移；只有上限没有下限时，「清掉几处忘了调基线」会让基线虚高，
/// 从此遮住后续新增的字面量。
/// </para>
/// </summary>
public sealed class ScaleTokenUsageTests
{
    /// <summary>
    /// 基线 = 2026-08-21 U204-A 修复后的实测值（口径：Views + Controls + App.axaml，
    /// 排除 bin/obj，属性写法与 <c>&lt;Setter Property=… Value=…/&gt;</c> 写法都算）。
    ///
    /// **剩下这些为什么不收**（写清理由，否则下一个人会为了把数字压到 0 而乱建 token）：
    /// - <c>FontSize</c> 18 处：13(×9) / 15(×4) / 13.5 / 12.5 / 38 / 27 / 10。
    ///   这些值**没有对应 token**，收它们等于同批新建 7 个 token —— 而其中
    ///   `27`（作品页刊头衬线标题）与 `13`（顶栏标题刻意降级，见 U204-E）
    ///   是**刻意的一次性值**，给一次性值建 token 只会让 token 表变成字面量清单。
    ///   真正该做的是先判定它们该并入哪一档（U204-E 的工作），再收。
    /// - <c>CornerRadius</c> 44 处：只有 4/6/8 三档有 token，2/5/7/10/12/14/16/24
    ///   与四个方向不等的那些都没有。同上，判档在先。
    /// - <c>BorderThickness</c> 39 处：`0`(×15) 是「显式取消描边」不是尺度；
    ///   `1.5` 与 `3,0,0,1` 是 U204-C 点名要先**判定哪个是对的**再收的两组分叉
    ///   （`3,0,0,1` 比 `3,0,0,0` 多一条底边，可能是刻意给列表项做行分隔，
    ///   抹平会让列表项失去分隔）。**在判定之前收口就是把有意的差异删掉。**
    ///
    /// ⚠️ 三个数字都**排除注释里的字面量**（本仓有 3 处注释在描述历史取值，
    /// 例如 `SettingsPageView.axaml:117` 写「原先的 BorderThickness="1"」）。
    /// 把注释算进去会让基线随「改注释」漂动，而注释里的数字改不动任何渲染。
    /// </summary>
    private static readonly IReadOnlyDictionary<string, int> LiteralBaselines =
        new Dictionary<string, int>(StringComparer.Ordinal)
        {
            ["FontSize"] = 18,
            ["CornerRadius"] = 44,
            ["BorderThickness"] = 39,
        };

    [Theory]
    [InlineData("FontSize")]
    [InlineData("CornerRadius")]
    [InlineData("BorderThickness")]
    public void ScaleLiteralsInPageFiles_StayWithinRecordedBaseline(string property)
    {
        var baseline = LiteralBaselines[property];
        var literals = CollectLiterals(property);

        Assert.True(
            literals.Count <= baseline,
            $"页面文件里 {property} 的字面量涨到 {literals.Count} 处，超过基线 {baseline}。"
            + "尺度必须走 Ariadne.* token —— 字面量会让 token 改了没反应。"
            + "新增的这些请先看主题里有没有同值 token；"
            + $"没有就先判定它属于哪一档，别直接写数字：\n{string.Join("\n", literals)}");

        // 反向：收干净了就要把基线调下来。少了这条，「清掉几处忘了调基线」
        // 会让基线虚高，从此遮住后续新增的字面量 —— DisplayNameJsonTests
        // 实测过这个形态（删掉 6 个死键后基线不调，照样全绿）。
        Assert.True(
            literals.Count >= baseline,
            $"页面文件里 {property} 只剩 {literals.Count} 处字面量，低于基线 {baseline}。"
            + $"这是好事，但请把 LiteralBaselines[\"{property}\"] 改成 {literals.Count}"
            + "——基线虚高会让它挡不住下一次漂移。");
    }

    /// <summary>
    /// U204-B：**版心推导链必须算得通。**
    ///
    /// 主题注释宣称「改字号时把这里一起改，两个数字的关系就写在这段注释里，
    /// 不会再分头漂移」——修复前那句话**没有任何执行形态**：
    /// `MeasureCharsPerLine` / `MeasureMaxWidth` / `SurfaceMaxWidth` 三个 token
    /// 全仓零引用，改它们不影响任何渲染，而真正决定稿纸宽度的是
    /// `WorksPageViewModel` 里的字面量 `720d`。
    ///
    /// # 判据为什么落在算术上而不是「token 存在」
    ///
    /// 「存在」在修复前就已经满足（三个 token 一直都在）。会变红的必须是
    /// **改了字号却没改版心**这个具体动作 —— 那正是注释承诺要防住的东西。
    /// XAML 静态资源算不了乘法，所以这条乘法只能由用例来算；
    /// 这不是权宜，而是「推导关系」在 XAML 里唯一可执行的落点。
    /// </summary>
    [Fact]
    public void ReadingMeasureDerivation_IsArithmeticallyConsistent()
    {
        var reading = ScaleTokenPaths.ThemeDouble("Ariadne.Size.Reading");
        var charsPerLine = ScaleTokenPaths.ThemeDouble("Ariadne.Reading.MeasureCharsPerLine");
        var measure = ScaleTokenPaths.ThemeDouble("Ariadne.Reading.MeasureMaxWidth");
        var surface = ScaleTokenPaths.ThemeDouble("Ariadne.Reading.SurfaceMaxWidth");
        var padding = ScaleTokenPaths.ThemeThickness("Ariadne.Reading.SurfacePadding");

        Assert.Equal(reading * charsPerLine, measure);
        Assert.Equal(measure + padding[0] + padding[2], surface);

        // 自检：CJK 单栏排版学建议 25–45 字/行。少了这条，把三个数字
        // 一起改成 1 × 1 = 1 也能让上面两条恒真 —— 那是一致的废值。
        Assert.InRange(charsPerLine, 25d, 45d);
    }

    /// <summary>
    /// U204-B：<c>WorksPageViewModel</c> 的兜底常量必须与主题 token 同值。
    ///
    /// ViewModel 会在没有 <c>Application</c> 的纯单元测试里被直接 new
    /// （`ReadingMeasureTests` 就是），所以它需要一个兜底值。
    /// **兜底值是这次修复最容易退化回字面量的地方** ——
    /// 若它可以与主题不同，那「稿纸宽度从主题读」在测试里就是句空话。
    /// 真实运行态读不读得到 token，由 headless 那条用例证（见
    /// <see cref="ReadingSurfaceRenderTests"/>）；本条只钉住两个数字同源。
    /// </summary>
    [Fact]
    public void ReadingSurfaceFallback_MatchesTheThemeToken()
    {
        var themeValue = ScaleTokenPaths.ThemeDouble("Ariadne.Reading.SurfaceMaxWidth");

        Assert.Equal(
            Ariadne.Desktop.ViewModels.WorksPageViewModel.SurfaceBaseWidthFallback,
            themeValue);
    }

    /// <summary>
    /// U204-D：<c>TextBlock.subtitle</c> 在全项目**只能有一个字号**。
    ///
    /// 修复前它被两处上下文覆盖，同一个 class 名渲染成 16 / 20 / 11 三种字号
    /// （42 / 12 / 9 处实例）—— 读代码时看见 <c>Classes="subtitle"</c>
    /// 完全无法判断它会有多大，而 20 撞上 <c>Size.Title</c>(20)、
    /// 11 又低于 <c>Size.Caption</c>(12)：两个覆盖值都跑到别的层级去了。
    ///
    /// # 判据取「声明数 == 1」而不是「某个覆盖不存在」
    ///
    /// 「那两条覆盖不存在」只挡得住那两条回来，挡不住**第三条新的**。
    /// 而这条债的形态恰恰是「下一个人再加一处上下文覆盖」——
    /// U204 全文的主题就是「每次改动都会新增一处不一致」。
    /// </summary>
    [Fact]
    public void SubtitleClass_HasExactlyOneFontSizeDeclaration()
    {
        var declarations = CollectFontSizeDeclarationsFor("subtitle");

        Assert.Single(declarations);
        Assert.Contains("Ariadne.Size.Subtitle", declarations[0].Value);
    }

    /// <summary>
    /// U204-D 的另一半：拆出来的两个类各自也必须只有一个字号，
    /// 且都**挂 token 而不是字面量**。
    ///
    /// ⚠️ 少了「挂 token」这一条，把 <c>.empty-title</c> 写成
    /// <c>FontSize="20"</c> 照样单一 —— 那只是把覆盖搬了个家，
    /// 主题改 <c>Size.Title</c> 时空态标题依旧不跟随。
    /// </summary>
    [Theory]
    [InlineData("empty-title", "Ariadne.Size.Title")]
    [InlineData("inspector-label", "Ariadne.Size.Micro")]
    public void SplitOutTextClasses_EachDeclareOneTokenBackedFontSize(string className, string token)
    {
        var declarations = CollectFontSizeDeclarationsFor(className);

        Assert.Single(declarations);
        Assert.Contains(token, declarations[0].Value);
    }

    /// <summary>
    /// 扫全仓 axaml，找出所有「选择器落在 <c>TextBlock.{className}</c> 上、
    /// 且设了 <c>FontSize</c>」的样式块。
    ///
    /// 刻意扫**整个仓**（主题 + 页面）：D 条那两处覆盖一处在主题
    /// （`Border.empty-state TextBlock.subtitle`）、一处在页面
    /// （`WorkspacePageView` 的 `Border.inspector-group TextBlock.subtitle`）——
    /// 只扫一边会漏掉另一边，而那正好各占一半。
    /// </summary>
    private static List<(string Selector, string Value)> CollectFontSizeDeclarationsFor(string className)
    {
        var styleBlock = new Regex(
            @"<Style\s+Selector=""(?<sel>[^""]*)"">(?<body>.*?)</Style>",
            RegexOptions.Compiled | RegexOptions.Singleline);
        var fontSizeSetter = new Regex(
            @"<Setter\s+Property=""FontSize""\s+Value=""(?<v>[^""]*)""\s*/>",
            RegexOptions.Compiled);

        var files = EnumeratePageFiles().Append(ScaleTokenPaths.ThemePath);
        var found = new List<(string, string)>();
        foreach (var file in files)
        {
            var text = File.ReadAllText(file);
            foreach (Match block in styleBlock.Matches(text))
            {
                var selector = block.Groups["sel"].Value;
                // 判据取「选择器的最后一段是 TextBlock.{className}」：
                // 后代选择器（`Border.x TextBlock.subtitle`）与裸选择器
                // （`TextBlock.subtitle`）都要算 —— 前者正是 D 条的病灶。
                var last = selector.Split(' ', StringSplitOptions.RemoveEmptyEntries)[^1];
                if (last != $"TextBlock.{className}")
                {
                    continue;
                }

                var setter = fontSizeSetter.Match(block.Groups["body"].Value);
                if (setter.Success)
                {
                    found.Add((selector, setter.Groups["v"].Value));
                }
            }
        }

        return found;
    }

    /// <summary>
    /// U204-A/C：发丝线的两个 token 必须同值。
    ///
    /// <c>Ariadne.Stroke.Hairline</c> 是 <c>x:Double</c>（给 Width/Height 用），
    /// <c>Ariadne.Stroke.HairlineAll</c> 是 <c>Thickness</c>（给 BorderThickness 用）。
    /// 必须分成两个 key 是因为 <c>DynamicResource</c> **不跑类型转换器** ——
    /// 把 double 绑到 <c>BorderThickness</c> 会**静默失效**（描边整个消失，
    /// 不报错也不回落）。代价是同一个设计值有两处定义 ⇒ 会漂。这条钉住它。
    /// </summary>
    [Fact]
    public void HairlineTokens_KeepTheSameValueAcrossBothTargetTypes()
    {
        var asDouble = ScaleTokenPaths.ThemeDouble("Ariadne.Stroke.Hairline");
        var asThickness = ScaleTokenPaths.ThemeThickness("Ariadne.Stroke.HairlineAll");

        Assert.All(asThickness, side => Assert.Equal(asDouble, side));
    }

    /// <summary>
    /// **唯一豁免的两个 token：它们在 XAML 里没有可引用的形态。**
    ///
    /// U204-B 原本要求「要么接线要么删」，但这两个是**推导链的输入与中间量**，
    /// 不是任何控件属性能吃的值：
    /// - `MeasureCharsPerLine`(36) 的语义是「每行几个汉字」——
    ///   Avalonia 里没有这个属性，它只能参与计算。
    /// - `MeasureMaxWidth`(576) 是正文测量宽，而正文宽从来不直接设：
    ///   实际设的是稿纸外框宽（`SurfaceMaxWidth`），测量宽 = 外框 − 内边距。
    ///
    /// ⇒ 硬凑一个引用点毫无意义（那个引用不会改变任何渲染，只是把「没人读」
    /// 藏起来）；删掉则丢掉「36 字/行」这个**设计决策的唯一记录**，
    /// 版心宽度从此变回一个无出处的 576。
    /// **第三条路：让它们被推导守卫消费。**
    /// <see cref="ReadingMeasureDerivation_IsArithmeticallyConsistent"/> 直接读这两个
    /// 数并验算 —— 改字号忘改版心会当场变红。它们从此不是「没人读的数字」，
    /// 而是一条**会失败**的约束的两个操作数。
    /// </summary>
    private static readonly IReadOnlyDictionary<string, string> DerivationOnlyTokens =
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["Ariadne.Reading.MeasureCharsPerLine"] =
                "推导链输入（每行字数）；XAML 无对应属性，由 ReadingMeasureDerivation 用例消费",
            ["Ariadne.Reading.MeasureMaxWidth"] =
                "推导链中间量（正文测量宽）；实际设的是 SurfaceMaxWidth，同上由推导用例消费",
        };

    /// <summary>
    /// 豁免表里的 token 必须**真的**被推导用例读到，否则这张表就是死代码的藏身处。
    /// 判据取「本文件里出现该 key 的字符串字面量」——它只会在推导用例里出现。
    /// </summary>
    [Fact]
    public void DerivationOnlyTokens_AreActuallyReadByTheDerivationTest()
    {
        var self = File.ReadAllText(
            Path.Combine(ScaleTokenPaths.SolutionDir, "Ariadne.Desktop.Tests", "ScaleTokenUsageTests.cs"));

        foreach (var (key, reason) in DerivationOnlyTokens)
        {
            Assert.False(string.IsNullOrWhiteSpace(reason), $"{key} 的豁免理由不能为空");
            Assert.True(
                self.Contains($"ThemeDouble(\"{key}\")", StringComparison.Ordinal),
                $"{key} 被豁免出「必须有生产消费者」，理由是「由推导用例消费」，"
                + "但本文件里没有任何一处真的读它 ⇒ 豁免表变成了死 token 的藏身处。"
                + "要么让推导用例读它，要么把它从主题里删掉。");
        }
    }

    /// <summary>
    /// U204-B：尺度 token **每一个都必须有生产消费者**。
    ///
    /// 修复前 6 个 token 全仓零引用（报告写 7 个，其中
    /// <c>Ariadne.Radius.Small</c> 已在 `059ec1e`/U207-C 里被接上，见报告更正）。
    /// 按本仓的判定标准（AGENTS.md 死代码一节）：**完全体之后仍没有消费者的
    /// 契约，就是没用的契约** —— 11 个模块全落地、12 个页面全成型之后
    /// 仍零调用者，那是废弃设计而不是「待实现」。
    ///
    /// # 判据取「逐一对应」而不是「存在性」
    ///
    /// 断言的是**每个** token 各自都有引用，而不是「有 N 个 token 被引用」。
    /// 后者会被「多引用一次已经在用的 token」满足 —— 那种守卫在
    /// 「新加一个没人读的 token」时照样全绿，而这正是 B 条的形态。
    /// </summary>
    [Fact]
    public void EveryScaleToken_HasAtLeastOneProductionConsumer()
    {
        var theme = File.ReadAllText(ScaleTokenPaths.ThemePath);
        var definition = new Regex(
            @"x:Key=""(?<k>Ariadne\.(?:Size|Radius|Page|Stroke|Group|Control|Reading)\.[A-Za-z0-9.]+)""",
            RegexOptions.Compiled);
        var keys = definition.Matches(theme)
            .Select(m => m.Groups["k"].Value)
            .Where(k => !DerivationOnlyTokens.ContainsKey(k))
            .Distinct(StringComparer.Ordinal)
            .ToList();

        Assert.True(keys.Count >= 15, $"只解析出 {keys.Count} 个尺度 token 定义，正则大概失效了");

        // 生产侧文本分两组，**判据不能混用**：
        // - XAML（主题 + 页面）里的引用形态只有 `{DynamicResource K}` / `{StaticResource K}`；
        // - C# 里的引用形态是字符串字面量 `"K"`（`TryGetResource("K", …)`）。
        //
        // ⚠️ **这里踩过一次**：起初把两种形态对同一个大 haystack 一起查，
        // 于是主题里的 `x:Key="K"` 自己命中了 `"K"` 这条规则 ⇒
        // **每个 token 都「有消费者」，本用例恒真**。变异测试（往主题里塞一个
        // 谁都不读的 `Ariadne.Size.Ghost`）当场抓到它照样全绿。
        // 教训与 U204 本身同型：定义处冒充引用处，而那正是本条要查的东西。
        var xamlTexts = new List<string> { theme };
        xamlTexts.AddRange(EnumeratePageFiles().Select(File.ReadAllText));

        // 刻意**不**把测试工程算进消费者：被测试引用不等于被产品读到，
        // 那正是 U108/U114/U117「实现完整 + 有测试 + 生产零调用者」的形态。
        var csharpTexts = new List<string>();
        foreach (var cs in Directory.EnumerateFiles(ScaleTokenPaths.DesktopRoot, "*.cs", SearchOption.AllDirectories))
        {
            if (cs.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                || cs.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            {
                continue;
            }

            csharpTexts.Add(File.ReadAllText(cs));
        }

        var orphans = new List<string>();
        foreach (var key in keys)
        {
            var referenced =
                xamlTexts.Any(text =>
                    text.Contains($"DynamicResource {key}", StringComparison.Ordinal)
                    || text.Contains($"StaticResource {key}", StringComparison.Ordinal))
                || csharpTexts.Any(text => text.Contains($"\"{key}\"", StringComparison.Ordinal));
            if (!referenced)
            {
                orphans.Add(key);
            }
        }

        Assert.True(
            orphans.Count == 0,
            $"{orphans.Count} 个尺度 token 生产侧零引用：\n  {string.Join("\n  ", orphans)}\n"
            + "按本仓判定标准，完全体之后仍没有消费者的契约就是废弃设计 ——"
            + "要么接线，要么删掉。**不要为了让本用例变绿而硬凑一个引用点**："
            + "凑出来的引用不改变任何渲染，只是把「没人读」藏起来。"
            + "若某个 token 确有理由留着不接线，请在本用例加白名单并写明理由。");
    }

    /// <summary>
    /// 收集页面文件里某个属性的字面量位置（属性写法 + Setter 写法）。
    /// ⚠️ **必须同时扫 Setter 写法**：本项目页面文件里就有
    /// <c>&lt;Setter Property="BorderThickness" Value="1" /&gt;</c> 这种局部样式，
    /// 只扫属性写法会漏掉一整类，且漏得毫无提示 —— 那种「守卫全绿但债还在」
    /// 正是 U204 附二说的形态。
    /// </summary>
    private static List<string> CollectLiterals(string property)
    {
        // 值以 `{` 打头的是绑定/资源引用，不算字面量。
        var attribute = new Regex(property + @"=""(?<v>[^""{][^""]*)""", RegexOptions.Compiled);
        var setter = new Regex(
            @"<Setter\s+Property=""" + property + @"""\s+Value=""(?<v>[^""{][^""]*)""\s*/>",
            RegexOptions.Compiled);

        var found = new List<string>();
        foreach (var file in EnumeratePageFiles())
        {
            var text = File.ReadAllText(file);
            var relative = Path.GetRelativePath(ScaleTokenPaths.DesktopRoot, file);
            foreach (var regex in new[] { attribute, setter })
            {
                foreach (Match match in regex.Matches(text))
                {
                    // 注释里描述历史值的那种（SettingsPageView 有一处）不该算 ——
                    // 但也不能简单跳过整行注释：注释与代码可能同行。
                    if (IsInsideComment(text, match.Index))
                    {
                        continue;
                    }

                    var line = text[..match.Index].Count(c => c == '\n') + 1;
                    found.Add($"  {relative}:{line}  {property}=\"{match.Groups["v"].Value}\"");
                }
            }
        }

        return found;
    }

    private static bool IsInsideComment(string text, int index)
    {
        var open = text.LastIndexOf("<!--", index, StringComparison.Ordinal);
        if (open < 0)
        {
            return false;
        }

        var close = text.IndexOf("-->", open, StringComparison.Ordinal);
        return close < 0 || close > index;
    }

    private static IEnumerable<string> EnumeratePageFiles()
    {
        foreach (var dir in new[] { "Views", "Controls" })
        {
            var full = Path.Combine(ScaleTokenPaths.DesktopRoot, dir);
            if (!Directory.Exists(full))
            {
                continue;
            }

            foreach (var file in Directory.EnumerateFiles(full, "*.axaml", SearchOption.AllDirectories))
            {
                if (file.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                    || file.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
                {
                    continue;
                }

                yield return file;
            }
        }

        yield return Path.Combine(ScaleTokenPaths.DesktopRoot, "App.axaml");
    }
}

/// 路径解析。单独一个类，避免与别的用例抢同名 helper。
internal static class ScaleTokenPaths
{
    public static string DesktopRoot => Path.Combine(SolutionDir, "Ariadne.Desktop");

    public static string ThemePath =>
        Path.Combine(DesktopRoot, "Resources", "Styles", "AriadneTheme.axaml");

    public static string SolutionDir
    {
        get
        {
            var dir = AppContext.BaseDirectory;
            while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
            {
                dir = Path.GetDirectoryName(dir);
            }

            Assert.NotNull(dir);
            return dir!;
        }
    }

    /// <summary>
    /// 从主题文件解析出某个 <c>x:Double</c> token 的值。
    /// 找不到时直接 <c>Assert.Fail</c> —— 返回 0 会让调用方的算术断言
    /// 变成「0 == 0」那种恒真式，那是空测。
    /// </summary>
    public static double ThemeDouble(string key)
    {
        var theme = File.ReadAllText(ThemePath);
        var match = Regex.Match(
            theme,
            @"<x:Double\s+x:Key=""" + Regex.Escape(key) + @""">(?<v>[0-9.]+)</x:Double>");
        Assert.True(match.Success, $"主题里找不到 x:Double token「{key}」——键名改了就该同批改本用例。");
        return double.Parse(match.Groups["v"].Value, CultureInfo.InvariantCulture);
    }

    /// <summary>
    /// 从主题文件解析出某个 <c>CornerRadius</c> token 的第一个分量。
    /// ⚠️ 圆角 token 是 <c>&lt;CornerRadius&gt;</c> 而不是 <c>&lt;x:Double&gt;</c> ——
    /// 拿 <see cref="ThemeDouble"/> 去查会「找不到键」，那是解析器的问题
    /// 而不是主题的问题，很容易被误读成缺 token。
    /// </summary>
    public static double ThemeCornerRadius(string key)
    {
        var theme = File.ReadAllText(ThemePath);
        var match = Regex.Match(
            theme,
            @"<CornerRadius\s+x:Key=""" + Regex.Escape(key) + @""">(?<v>[0-9.,]+)</CornerRadius>");
        Assert.True(match.Success, $"主题里找不到 CornerRadius token「{key}」。");
        return double.Parse(match.Groups["v"].Value.Split(',')[0], CultureInfo.InvariantCulture);
    }

    /// 从主题文件解析出某个 <c>Thickness</c> token 的四个方向值。
    public static double[] ThemeThickness(string key)
    {
        var theme = File.ReadAllText(ThemePath);
        var match = Regex.Match(
            theme,
            @"<Thickness\s+x:Key=""" + Regex.Escape(key) + @""">(?<v>[0-9.,]+)</Thickness>");
        Assert.True(match.Success, $"主题里找不到 Thickness token「{key}」。");
        var parts = match.Groups["v"].Value
            .Split(',')
            .Select(p => double.Parse(p, CultureInfo.InvariantCulture))
            .ToArray();
        return parts.Length switch
        {
            1 => [parts[0], parts[0], parts[0], parts[0]],
            2 => [parts[0], parts[1], parts[0], parts[1]],
            4 => parts,
            _ => throw new InvalidOperationException($"{key} 的 Thickness 分量数 {parts.Length} 不合法"),
        };
    }
}
