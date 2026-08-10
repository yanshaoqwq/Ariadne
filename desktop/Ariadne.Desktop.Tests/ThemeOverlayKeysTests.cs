using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 守护「个性化强调色 → 实际渲染」这条链路。
///
/// 回归背景：ThemeApplication 过去只覆盖 Brush 令牌（Ariadne.AccentPrimary 等），
/// 但渐变刷的 GradientStop 只能吃 Color，所以主色按钮 / 欢迎页主操作卡 / 空态插画
/// 绑的是 Ariadne.Color.*。那批键从未被覆盖，于是用户换了强调色后
/// 「保存节点配置」「添加起始节点」这类主按钮纹丝不动，仍是字典里写死的预设青绿。
///
/// 这里用源码级断言（不需要 Avalonia 运行时，因此不受 headless 会话限制）：
/// Apply 写入的键集合必须与 Reset 删除的键集合完全一致。
/// </summary>
public sealed class ThemeOverlayKeysTests
{
    private static string ResolveThemeApplicationSource()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return Path.Combine(dir!, "Ariadne.Desktop", "ThemeApplication.cs");
    }

    /// <summary>
    /// Apply 里 SetBrush/SetColor 写入的键，必须与 OverlayBrushKeys（Reset 依据）逐一对应。
    /// 漏删 → 切回预设主题后仍残留上一套自定义色；误删 → 清掉字典预设值，控件失色。
    /// </summary>
    [Fact]
    public void AppliedResourceKeys_MatchResetKeyList_Exactly()
    {
        var source = File.ReadAllText(ResolveThemeApplicationSource());

        var applied = Regex.Matches(source, @"Set(?:Brush|Color)\(resources,\s*""([^""]+)""")
            .Select(m => m.Groups[1].Value)
            .ToHashSet(StringComparer.Ordinal);

        Assert.NotEmpty(applied);

        var declared = ThemeApplication.OverlayBrushKeys.ToHashSet(StringComparer.Ordinal);

        // 双向差集分别报错，失败信息直接指出是漏删还是误删。
        var leakedOnReset = applied.Except(declared).OrderBy(k => k, StringComparer.Ordinal).ToArray();
        var removedButNeverSet = declared.Except(applied).OrderBy(k => k, StringComparer.Ordinal).ToArray();

        Assert.Empty(leakedOnReset);
        Assert.Empty(removedButNeverSet);
    }

    /// <summary>
    /// 渐变面（主操作按钮 / 欢迎页卡 / 空态插画）依赖的 Color 令牌必须在覆盖清单里。
    /// 这是上述回归的直接触点，单独钉死，避免有人只加 Brush 键就以为接好了。
    /// </summary>
    [Fact]
    public void GradientColorTokens_AreThemeOverridable()
    {
        var declared = ThemeApplication.OverlayBrushKeys;

        Assert.Contains("Ariadne.Color.AccentPrimary", declared);
        Assert.Contains("Ariadne.Color.AccentHover", declared);
        Assert.Contains("Ariadne.Color.AccentPressed", declared);
    }
}
