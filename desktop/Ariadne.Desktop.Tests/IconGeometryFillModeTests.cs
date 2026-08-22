using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U213-C 顺带发现：**`PathIcon` 配「只有描边、没有面积」的几何 = 什么都不画。**
///
/// # 缺陷形态（三个关闭按钮在屏上是空白的）
///
/// `Ariadne.Icon.Close` 的几何是两条线段（`M… L… M… L…`），没有闭合子路径、
/// 没有弧也没有曲线 ⇒ **围不出任何面积**。
/// 而 `PathIcon` 的渲染语义是**用 `Foreground` 填充路径**、不描边。
/// 填充一个零面积图形，结果就是一个像素都不画。
///
/// 三处实例（`WorkspacePageView` 的变量填值浮层关闭键、`WorksPageView` 的
/// 大纲对照关闭键与快速编辑关闭键）因此**长期不可见**：按钮在、能点、有 tooltip，
/// 只是没有图案。⚠️ Avalonia 不为此报任何错 —— 与本仓已记的
/// 「缺资源键静默失效」同族：**渲染层的错配一律静默**。
///
/// 正确写法是 `Path Classes="icon"`（主题里那条样式设了 `Stroke` +
/// `StrokeThickness` + `Fill=Transparent`，即描边语义）。
///
/// # 这条守卫为什么必要：11/30 个几何都是纯折线
///
/// 主题里 30 个 `StreamGeometry` 有 **11 个**是零面积折线。也就是说
/// 「随手写个 `PathIcon` 配一个图标键」有超过三分之一的概率画出空白，
/// 而且**编译通过、测试全绿、只有肉眼能发现**。
///
/// 本仓已记「图标改动必须渲染出来看」——但那要求人**记得去看**。
/// 这条守卫把它变成机器判定：把两类东西各自算出来，再断言交集为空。
///
/// ⚠️ 判据刻意**不是**「某处别用 PathIcon」，而是「**PathIcon 与折线几何不许配对**」：
/// 前者会误伤 `PathIcon` 配实心几何那些正确用法（主题里 19 个几何是有面积的）。
/// </summary>
public sealed class IconGeometryFillModeTests
{
    /// <summary>
    /// 主判据：任何 `PathIcon` 都不许引用零面积（纯折线）几何。
    /// </summary>
    [Fact]
    public void NoPathIcon_FillsAStrokeOnlyGeometry()
    {
        var strokeOnly = StrokeOnlyGeometryKeys();
        Assert.NotEmpty(strokeOnly); // 前提自检：主题里确实有折线几何，否则本条恒真

        var offenders = new List<string>();
        foreach (var file in Directory.EnumerateFiles(
                     DesktopRoot(), "*.axaml", SearchOption.AllDirectories))
        {
            if (file.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}")
                || file.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}"))
            {
                continue;
            }

            var markup = File.ReadAllText(file);
            foreach (Match element in Regex.Matches(markup, @"<PathIcon\b(.*?)/>", RegexOptions.Singleline))
            {
                var key = Regex.Match(
                    element.Groups[1].Value,
                    @"(?:Dynamic|Static)Resource\s+([\w.]+)\s*\}");
                if (!key.Success || !strokeOnly.Contains(key.Groups[1].Value))
                {
                    continue;
                }

                var line = markup[..element.Index].Count(c => c == '\n') + 1;
                offenders.Add(
                    $"{Path.GetFileName(file)}:{line} 用 PathIcon 填充 {key.Groups[1].Value}");
            }
        }

        Assert.True(
            offenders.Count == 0,
            "PathIcon 用 Foreground **填充**路径，而这些几何只有描边、没有面积 ⇒ "
            + "按钮在屏上是空白的（能点、有 tooltip、没有图案），且 Avalonia 不报错。\n"
            + "改用 `Path Classes=\"icon\"`（描边语义）。\n"
            + string.Join("\n", offenders));
    }

    /// <summary>
    /// 前提哨兵：`Path.icon` 样式仍然是**描边**语义。
    ///
    /// 上面那条判据的整个前提是「`Path Classes="icon"` 会描边、所以是正确替代」。
    /// 哪天有人把那条样式改成填充（或把 `Fill` 从 `Transparent` 改掉），
    /// 上面全绿而屏幕上的图标**全部消失** —— 这条负责在那一刻变红。
    /// </summary>
    [Fact]
    public void PathIconClass_IsStillAStrokeStyle()
    {
        var theme = File.ReadAllText(
            Path.Combine(DesktopRoot(), "Resources", "Styles", "AriadneTheme.axaml"));
        var start = theme.IndexOf("<Style Selector=\"Path.icon\">", StringComparison.Ordinal);
        Assert.True(start > 0, "Path.icon 样式不见了 ⇒ 本族判据的前提没了，请重新定");
        var body = theme[start..theme.IndexOf("</Style>", start, StringComparison.Ordinal)];

        Assert.Contains("Property=\"Stroke\"", body, StringComparison.Ordinal);
        Assert.Contains("Property=\"StrokeThickness\"", body, StringComparison.Ordinal);
        // Fill 必须是透明：折线几何被填充时会画出诡异的三角形色块而不是线条。
        Assert.Contains("Property=\"Fill\" Value=\"Transparent\"", body, StringComparison.Ordinal);
    }

    /// <summary>
    /// 主题里所有「只有描边、没有面积」的几何键。
    ///
    /// 判据：路径数据里没有 `Z`（闭合）、也没有弧/曲线命令（`A`/`C`/`Q`/`S`/`T`）
    /// ⇒ 它只是折线，填充结果为空。
    /// </summary>
    private static HashSet<string> StrokeOnlyGeometryKeys()
    {
        var theme = File.ReadAllText(
            Path.Combine(DesktopRoot(), "Resources", "Styles", "AriadneTheme.axaml"));
        var keys = new HashSet<string>(StringComparer.Ordinal);
        foreach (Match geometry in Regex.Matches(
                     theme,
                     @"<StreamGeometry x:Key=""([^""]+)"">(.*?)</StreamGeometry>",
                     RegexOptions.Singleline))
        {
            var data = geometry.Groups[2].Value;
            if (Regex.IsMatch(data, "[ZzAaCcQqSsTt]"))
            {
                continue;
            }
            if (Regex.IsMatch(data, "[MmLlHhVv]"))
            {
                keys.Add(geometry.Groups[1].Value);
            }
        }
        return keys;
    }

    private static string DesktopRoot()
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var attempt = 0; attempt < 12 && walk is not null; attempt++)
        {
            var candidate = Path.Combine(walk.FullName, "desktop", "Ariadne.Desktop");
            if (Directory.Exists(candidate))
            {
                return candidate;
            }
            walk = walk.Parent;
        }
        throw new DirectoryNotFoundException("desktop/Ariadne.Desktop");
    }
}
