using System.Xml.Linq;
using Avalonia.Media;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U162 的 A / B / E 三条回归用例。
///
/// 这三条是文档 `:327` 起那一节点名「该写」的，交接时**未落地**（`4733428`
/// 只提交了实现）。C/D/F 三条**刻意不写**，理由见文档：那几条判据本质是
/// 「好不好看」，写 `Assert.Equal(36, width)` 只会把一次主观取值固化成契约，
/// 下次微调必须同时改测试 ⇒ 测试变成阻力，而它证明的只是「常量等于常量」。
///
/// ⚠️ **源码文本断言完全不过 XAML 编译**（AGENTS.md 记着，本项目曾因此
/// 提交过编译不通过的主题文件而测试全绿）。所以改完 `.axaml` 必须
/// `dotnet build`，本文件的断言不能替代它。
/// </summary>
public sealed class BrandChromeContractTests
{
    /// <summary>
    /// U162-A：标题栏 Logo 的线描必须是**主题强调色**，不是 `TextOnAccent`。
    ///
    /// <para>判据取 <c>MapPixel</c> 的**输出颜色**，不取「<c>OnAccent</c> 属性等于 false」——
    /// 后者是实现细节，换个等价写法就会假红；前者是用户真正看到的东西。
    /// <c>MapPixel</c> 是纯函数（`AppIconRecolor.cs:32`），不必起 GUI。</para>
    ///
    /// <para>「线描像素」= 非纸色像素。给一个纯黑输入像素（母版里线描就是黑的），
    /// 断言它被映射成传入的 accent 色。</para>
    /// </summary>
    [Theory]
    [InlineData(0x2E, 0x72, 0x6B)] // 亮色 Ariadne.AccentPrimary
    [InlineData(0x6F, 0xB9, 0xAD)] // 暗色 Ariadne.AccentPrimary
    public void BrandLogo_InkPixel_BecomesAccentColor(byte accentR, byte accentG, byte accentB)
    {
        // 纸色传一个明显不同的值，这样「线描误用纸色」会立刻暴露。
        var mapped = AppIconRecolor.MapPixel(
            r: 0x00, g: 0x00, b: 0x00, a: 0xFF,
            accentR: accentR, accentG: accentG, accentB: accentB,
            paperR: 0xFF, paperG: 0xFF, paperB: 0xFF);

        Assert.Equal((accentR, accentG, accentB), (mapped.R, mapped.G, mapped.B));
        Assert.Equal(0xFF, mapped.A);
    }

    /// <summary>
    /// 配套：`MainWindow.axaml` 里的 BrandLogo 不得设 <c>OnAccent="True"</c>。
    ///
    /// <para>⚠️ **必须查属性值，不能查字符串出现**：
    /// <c>Assert.DoesNotContain("OnAccent", axaml)</c> 会被将来合法的
    /// <c>OnAccent="False"</c> 绊倒——而 `4733428` 之后那里**正是**显式写着 False，
    /// 所以字符串断言现在就是假红。用 XML 解析取属性值
    /// （同 `CanvasZoomTransformOriginTests` 的做法）。</para>
    /// </summary>
    [Fact]
    public void MainWindow_BrandLogo_IsNotMarkedOnAccent()
    {
        var document = XDocument.Load(ResolveDesktopFile("Views", "MainWindow.axaml"));
        var logos = document.Descendants()
            .Where(element => element.Name.LocalName == "BrandLogo")
            .ToList();

        // 自检：一个都找不到说明控件名改了或路径解析错了，而不是「没有违规」。
        Assert.True(
            logos.Count > 0,
            "MainWindow.axaml 里找不到任何 BrandLogo —— 控件改名或路径解析错了，本用例已失效。");

        foreach (var logo in logos)
        {
            var onAccent = logo.Attribute("OnAccent")?.Value;
            Assert.False(
                string.Equals(onAccent, "True", StringComparison.OrdinalIgnoreCase),
                "标题栏 Logo 被标成 OnAccent=True，线描会用 TextOnAccent 而不是主题强调色"
                + "（U162-A）。标题栏底色是纸色不是 Accent 底，用 TextOnAccent 会让 logo"
                + "在浅色主题下几乎看不见。");
        }
    }

    /// <summary>
    /// U162-B：Settings 齿轮的几何数据两处必须一致。
    ///
    /// <para>这条防的是「只改一处」——图标同时存在于 C# 常量
    /// （`IconGeometries.SettingsData`，供代码里动态构造）与主题资源
    /// （`Ariadne.Icon.Settings`，供 XAML 引用）。两处漂移的表现是
    /// 「有的地方齿轮变了、有的没变」，而这在截图里极难发现。</para>
    ///
    /// <para>⚠️ 「这个图形是不是齿轮」**无法自动判定**，只能人看。
    /// 本用例只钉一致性，不钉形状。</para>
    /// </summary>
    [Fact]
    public void SettingsIconGeometry_IsIdenticalInBothDefinitions()
    {
        var csharp = NormalizeGeometry(ReadSettingsDataFromCSharp());
        var theme = NormalizeGeometry(ReadSettingsGeometryFromTheme());

        Assert.False(string.IsNullOrWhiteSpace(csharp), "从 IconGeometries.cs 里没解析出 SettingsData");
        Assert.False(string.IsNullOrWhiteSpace(theme), "从 AriadneTheme.axaml 里没解析出 Ariadne.Icon.Settings");

        Assert.Equal(theme, csharp);
    }

    /// <summary>
    /// 几何字符串归一：折叠空白差异。
    ///
    /// 两处一个在 C# 字符串里、一个在 XAML 属性里，换行与缩进习惯不同，
    /// 逐字比较会因为排版差异假红。路径命令本身对空白不敏感，
    /// 所以折叠空白是安全的归一，而不是放宽判据。
    /// </summary>
    private static string NormalizeGeometry(string? value) =>
        string.Join(' ', (value ?? string.Empty)
            .Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries));

    private static string? ReadSettingsDataFromCSharp()
    {
        var text = File.ReadAllText(ResolveDesktopFile("ViewModels", "IconGeometries.cs"));
        var match = System.Text.RegularExpressions.Regex.Match(
            text, @"SettingsData\s*=\s*""([^""]*)""");
        return match.Success ? match.Groups[1].Value : null;
    }

    private static string? ReadSettingsGeometryFromTheme()
    {
        var document = XDocument.Load(ResolveDesktopFile(
            "Resources", "Styles", "AriadneTheme.axaml"));
        var key = XName.Get("Key", "http://schemas.microsoft.com/winfx/2006/xaml");
        var node = document.Descendants()
            .FirstOrDefault(element => element.Attribute(key)?.Value == "Ariadne.Icon.Settings");
        // Geometry 资源既可能写成元素内容，也可能写成属性值，两种都取。
        return node?.Value is { Length: > 0 } inner ? inner : node?.Attribute("Data")?.Value;
    }


    /// <summary>
    /// U162-E 的唯一不变量：设置页标签页**选中与未选中的 <c>BorderThickness</c> 之和相等**。
    ///
    /// <para>E 条把选中态从「底色块 + 四边描边」改成了下划线（底边 2px 强调色）。
    /// 下划线态有个必然的陷阱：**只给选中态加 2px 会让切换标签时整行跳动**
    /// ——未选中项没有那 2px，高度差一个像素带。</para>
    ///
    /// <para>现状已做对：未选中态也声明 <c>0,0,0,2</c>，只是
    /// <c>BorderBrush=Transparent</c>（`SettingsPageView.axaml:120-127`）。
    /// 本用例钉的就是这条性质，防的是将来有人「清理掉那个看不见的边框」。</para>
    ///
    /// <para>⚠️ 这是 C–F 四条里**唯一**值得写断言的：它是不变量（两者相等），
    /// 而不是取值（宽度等于 36）。取值型断言会把一次主观调参固化成契约，
    /// 下次微调就变成阻力——文档 `:361` 已点明这一点。</para>
    /// </summary>
    [Fact]
    public void SettingsTabs_SelectedAndUnselected_HaveEqualBorderFootprint()
    {
        var view = File.ReadAllText(ResolveDesktopFile("Views", "SettingsPageView.axaml"));

        var unselected = ReadSetter(view, "ListBox.settings-tabs ListBoxItem", "BorderThickness");
        var selected = ReadSetter(view, "ListBox.settings-tabs ListBoxItem:selected", "BorderThickness");

        // 自检：任一侧解析不出来说明选择器改名或结构变了，本用例已失效——
        // 必须失败而不是静默通过（否则守卫消失了但看板还是绿的）。
        Assert.False(
            string.IsNullOrWhiteSpace(unselected),
            "解析不到未选中态的 BorderThickness —— 选择器 `ListBox.settings-tabs ListBoxItem` 改了？本用例已失效。");
        Assert.False(
            string.IsNullOrWhiteSpace(selected),
            "解析不到选中态的 BorderThickness —— 选择器改了？本用例已失效。");

        Assert.Equal(SumThickness(unselected!), SumThickness(selected!));
    }

    /// <summary>
    /// 取某个 Style 选择器下某个 Setter 的值。
    ///
    /// ⚠️ 选择器必须**精确匹配到引号**（`Selector="X"`），不能用 Contains：
    /// `ListBox.settings-tabs ListBoxItem` 是
    /// `ListBox.settings-tabs ListBoxItem:selected` 的前缀，
    /// 不带结束引号会让未选中态的查询命中选中态那一条，两边取到同一个值
    /// ⇒ 断言恒真（典型的「唯一可选项」型空测）。
    /// </summary>
    private static string? ReadSetter(string xaml, string selector, string property)
    {
        var anchor = xaml.IndexOf($"Selector=\"{selector}\"", StringComparison.Ordinal);
        if (anchor < 0)
        {
            return null;
        }

        // 只在本 Style 块内找：下一个 </Style> 之前。
        var end = xaml.IndexOf("</Style>", anchor, StringComparison.Ordinal);
        var block = end < 0 ? xaml[anchor..] : xaml[anchor..end];
        var match = System.Text.RegularExpressions.Regex.Match(
            block, $@"Property=""{property}""\s+Value=""([^""]*)""");
        return match.Success ? match.Groups[1].Value : null;
    }

    /// <summary>把 `左,上,右,下` 或单值加总，用于比较「占位高度」。</summary>
    private static double SumThickness(string value) => value
        .Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
        .Select(part => double.Parse(part, System.Globalization.CultureInfo.InvariantCulture))
        .Sum();

    private static string ResolveDesktopFile(params string[] parts)
    {
        var root = Path.Combine(ResolveSolutionDir(), "Ariadne.Desktop");
        return Path.Combine(new[] { root }.Concat(parts).ToArray());
    }

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
}
