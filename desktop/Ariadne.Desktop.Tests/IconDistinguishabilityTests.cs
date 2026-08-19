using System;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U195-G：作品页顶栏并列的 Import / Export 两枚图标不许同形。
///
/// **原缺陷**：两枚 `StreamGeometry` 共用完全相同的托盘底
/// （`M4,16 L4,20 L20,20 L20,16`）和同一根竖线，**唯一差别是箭头朝上还是朝下**。
/// 16px 尺寸下相邻并列几乎不可分辨 —— 而这两个动作后果相反（读入 vs 写出），
/// 在作品页顶栏（`WorksPageView.axaml`）正好紧挨着。
///
/// **为什么判据取「不共用轮廓子串」而不是「两者不相等」**：
/// 缺陷版本里两者本来就不相等（箭头方向不同），断言"不相等"会全绿。
/// 真正要守的是**轮廓骨架不能相同** —— 那才是 16px 下的可辨识性来源。
///
/// ⚠️ 图标是矢量 `Geometry`，headless 测不出「画出来像不像」。这条守的是
/// 「两枚不许退回同形」这个可静态检查的性质，**不能替代真机看一眼**。
/// （项目已有教训：headless 下"没模板"与"有模板没布局"同形。）
/// </summary>
public sealed class IconDistinguishabilityTests
{
    private static string ReadThemeXaml()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return File.ReadAllText(Path.Combine(
            dir!, "Ariadne.Desktop", "Resources", "Styles", "AriadneTheme.axaml"));
    }

    private static string GeometryFor(string key)
    {
        var xaml = ReadThemeXaml();
        var match = Regex.Match(
            xaml,
            $@"<StreamGeometry\s+x:Key=""{Regex.Escape(key)}"">(?<d>[^<]*)</StreamGeometry>");
        Assert.True(match.Success, $"主题里找不到 {key} 的 StreamGeometry 定义");
        return match.Groups["d"].Value.Trim();
    }

    /// <summary>
    /// 把路径拆成一组「命令 + 坐标」段，用于比较轮廓骨架而非整串文本。
    /// </summary>
    private static string[] SubPaths(string geometry) =>
        geometry.Split('M', StringSplitOptions.RemoveEmptyEntries);

    [Fact]
    public void ImportAndExportShareNoSubPath()
    {
        var import = GeometryFor("Ariadne.Icon.Import");
        var export = GeometryFor("Ariadne.Icon.Export");

        // 自检：两枚都必须解析出多段，否则下面的交集判断在解析失败时会假绿。
        var importParts = SubPaths(import);
        var exportParts = SubPaths(export);
        Assert.True(importParts.Length >= 2, $"Import 只解析出 {importParts.Length} 段，正则可能失效");
        Assert.True(exportParts.Length >= 2, $"Export 只解析出 {exportParts.Length} 段，正则可能失效");

        // 核心判据：一个子路径都不许相同。
        // 原缺陷正是共用 `4,16 L4,20 L20,20 L20,16` 这一整段托盘底。
        foreach (var part in importParts)
        {
            var normalized = part.Trim();
            Assert.DoesNotContain(
                normalized,
                exportParts,
                StringComparer.Ordinal);
        }
    }

    [Fact]
    public void ImportAndExportDifferByMoreThanArrowDirection()
    {
        var import = GeometryFor("Ariadne.Icon.Import");
        var export = GeometryFor("Ariadne.Icon.Export");

        // 「只翻箭头」的形态特征：把两串里的坐标顺序抹平后会高度重合。
        // 这里用一个便宜但有效的代理判据 —— 两串的**坐标点集合**不许几乎相同。
        var importPoints = Regex.Matches(import, @"-?\d+(?:\.\d+)?,-?\d+(?:\.\d+)?")
            .Select(m => m.Value).ToHashSet(StringComparer.Ordinal);
        var exportPoints = Regex.Matches(export, @"-?\d+(?:\.\d+)?,-?\d+(?:\.\d+)?")
            .Select(m => m.Value).ToHashSet(StringComparer.Ordinal);

        Assert.NotEmpty(importPoints);
        Assert.NotEmpty(exportPoints);

        var shared = importPoints.Intersect(exportPoints, StringComparer.Ordinal).Count();
        var smaller = Math.Min(importPoints.Count, exportPoints.Count);
        var overlapRatio = (double)shared / smaller;

        // 阈值取 0.5：缺陷版本的重合率是 5/6 ≈ 0.83（托盘底 4 点 + 竖线端点）。
        // 完全无重合不现实（都在 24×24 网格里，边缘点难免撞），所以不要求 0。
        Assert.True(
            overlapRatio < 0.5,
            $"Import/Export 的坐标点重合率 {overlapRatio:P0}（{shared}/{smaller}）过高，"
            + "说明两枚仍共用大部分轮廓，16px 下不可分辨");
    }

    [Fact]
    public void BothIconsStayReferencedSoThisGuardStaysMeaningful()
    {
        // 这条与上两条互补：上面保证「不同形」，这条保证**两枚都还在用**。
        // 若有人把 Export 整个删掉，上面两条会因为找不到定义而红，
        // 但红的原因说不清；这条说得清。
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }
        Assert.NotNull(dir);

        var viewsDir = Path.Combine(dir!, "Ariadne.Desktop", "Views");
        var allViews = string.Concat(
            Directory.EnumerateFiles(viewsDir, "*.axaml").Select(File.ReadAllText));

        Assert.Contains("Ariadne.Icon.Import", allViews);
        Assert.Contains("Ariadne.Icon.Export", allViews);
    }
}
