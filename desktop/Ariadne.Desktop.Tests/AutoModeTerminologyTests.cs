using System.Text.Json;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U208-D：产品术语在中文界面里**只能有一种写法**。
///
/// # 缺陷本体不是「界面上有英文」，是「同一个术语两种译法并存」
///
/// 报告只点名了一处显示位（`ProjectAiPanel.axaml:37` 那一行）。实际扫下来，
/// 中文包里 **8 个键**用英文原词、**6 个键**已经用中文译法 ⇒ 作者在同一个
/// 设置分区里会看到两个名字指同一个东西。两处最刺眼的对照：
///
/// 1. **标题与它自己的说明矛盾**：`ui.settings.index.budget`（分区标题）用英文原词，
///    紧跟的 `ui.settings.index.budget.desc`（同一分区的说明）已经是中文译法。
/// 2. **同一句话内部并列**：`ui.settings.automation.confirmation.help` 把中文的
///    「普通模式」与英文原词并列作对照 —— **同句并列比同屏并列更刺眼**。
///
/// # 判据为什么不取「那一个键的值不等于英文原词」
///
/// 那种判据只覆盖被点名的一处，改完剩下 7 处照样不一致，而用例是绿的 ——
/// 这正是本轮 A/B/D/E 四条共有的元模式（正确规则没推广到同类兄弟）。
/// 所以主判据取**全表扫描**：中文包里不允许任何值含该英文原词。
///
/// # 反向判据同样重要
///
/// 英文包**必须保留**英文原词（那是产品的英文名），日文包保留它自己的译名。
/// 少了这一条，"统一"很容易被做成"把三份包都改成中文"，
/// 那是另一个方向的缺陷。术语的**标识符**（后端 `auto_mode` 字段、
/// 权限模型里的预授权）一律不在本条范围内 —— 本条只管 `display_name.json` 的**值**。
/// </summary>
public sealed class AutoModeTerminologyTests
{
    /// <summary>界面显示用的英文原词（ASCII 两词形态），只在判据里出现，不写进产品文案。</summary>
    private const string AsciiTerm = "auto mode";

    /// <summary>已有的中文译法，沿用不造新词。</summary>
    private const string ChineseTerm = "自动模式";

    /// <summary>
    /// 主判据：中文包里**没有任何值**再用英文原词。
    /// </summary>
    [Fact]
    public void ChinesePack_UsesOneSpelling_ForTheAutomationTerm()
    {
        var zh = LoadPack("display_name.json");

        // 自检下限：解析失败/读错文件时字典会很小，若不设下限，下面的"零命中"是假绿。
        Assert.True(zh.Count > 1000, $"中文包只解析出 {zh.Count} 个键，读错文件了。");

        var stillEnglish = zh
            .Where(pair => pair.Value.Contains(AsciiTerm, StringComparison.OrdinalIgnoreCase))
            .Select(pair => pair.Key)
            .OrderBy(key => key, StringComparer.Ordinal)
            .ToList();

        Assert.True(
            stillEnglish.Count == 0,
            "中文界面里这些键还在用英文原词，与另外那些已用中文译法的键并存 —— "
            + "作者会以为是两个不同的东西（U208-D）：\n  "
            + string.Join("\n  ", stillEnglish));

        // 正向：译法必须真的在用。只有"零英文"这一条时，把值删空也能通过。
        var translated = zh.Count(pair => pair.Value.Contains(ChineseTerm, StringComparison.Ordinal));
        Assert.True(
            translated >= 8,
            $"中文译法只出现在 {translated} 个键里 —— 术语像是被删掉而不是被统一（U208-D）。");
    }

    /// <summary>
    /// 报告点名的那处显示位：`ProjectAutomationState.Label` 查的就是这个键。
    ///
    /// 单独立一条是因为它和上面那条**可以只坏一个**：有人可能把全表扫干净了，
    /// 却顺手把这个键删掉/改名，那时界面显示的是 `[key]` 而全表扫描仍然绿。
    /// </summary>
    [Fact]
    public void AutoModeLabelKey_StillExists_AndReadsInChinese()
    {
        var zh = LoadPack("display_name.json");

        // 键名（标识符）不许动，只动值。
        Assert.True(
            zh.ContainsKey("ui.settings.automation.auto_mode"),
            "ui.settings.automation.auto_mode 不见了 —— ProjectAutomationState.Label "
            + "会显示成 [key]（DisplayNameService 缺键静默回落，不报错）。");
        Assert.Contains(ChineseTerm, zh["ui.settings.automation.auto_mode"], StringComparison.Ordinal);
    }

    /// <summary>
    /// 反向基准：英文包保留英文原词，日文包保留自己的译名。
    ///
    /// 这条不是重复。它钉住"往哪个方向统一"：若有人为了"一致"把三份包
    /// 都改成中文，上面那条仍绿而**这条会红**。
    /// </summary>
    [Fact]
    public void EnglishAndJapanesePacks_KeepTheirOwnSpelling()
    {
        var en = LoadPack("display_name.en.json");
        var ja = LoadPack("display_name.ja.json");

        Assert.Contains(
            AsciiTerm,
            en["ui.settings.automation.auto_mode"],
            StringComparison.OrdinalIgnoreCase);

        // 日文值不做字面断言（不替用户做翻译决定），只钉住两件事：
        // 键在、且不是被中文顶替掉的。
        var japanese = ja["ui.settings.automation.auto_mode"];
        Assert.False(string.IsNullOrWhiteSpace(japanese));
        Assert.DoesNotContain(ChineseTerm, japanese, StringComparison.Ordinal);
    }

    /// <summary>
    /// 直接读语言包文件，不经 <c>DisplayNameService</c>：
    /// 服务在缺键时返回 <c>[key]</c>，那会把「键没了」表现成「值不含某字符串」而误判为通过。
    /// 抄 <see cref="ImportSourceCopyMatchesCapabilityTests"/> 的做法。
    /// </summary>
    private static IReadOnlyDictionary<string, string> LoadPack(string fileName)
    {
        var path = Path.Combine(ResolveRepoRoot(), "core", "resources", fileName);
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        var map = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var property in document.RootElement.EnumerateObject())
        {
            if (property.Value.ValueKind == JsonValueKind.String)
            {
                map[property.Name] = property.Value.GetString() ?? string.Empty;
            }
        }
        return map;
    }

    private static string ResolveRepoRoot()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null
            && !File.Exists(Path.Combine(dir, "core", "resources", "display_name.json")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return dir!;
    }
}
