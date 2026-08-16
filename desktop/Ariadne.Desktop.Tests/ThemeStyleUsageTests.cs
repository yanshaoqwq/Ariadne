using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U152：主题里定义了样式、但全仓没有任何控件挂那个 class —— 死样式。
///
/// **危害不在渲染，在判断**：死样式会**冒充「已完成的工作」**，让下一个改样式的人误判。
/// U151 就是实证——死样式 <c>TextBox.document-editor</c> 里设了 <c>LineHeight=30</c>，
/// 让人以为正文编辑器行高已统一，而真正在用的 <c>ae:TextEditor</c> 根本没设，实差 33%。
///
/// ⚠️ **扫描 class 使用有四种写法，漏一种就会把在用的类报成死类**（本文件踩了三次）：
/// <c>Classes="a b"</c>（XAML）、<c>Classes.xxx="{Binding}"</c>（XAML 条件类）、
/// <c>Classes.Add("x")</c> 与 <c>Classes.Set("x", cond)</c>（C#）。
/// 最初只扫前三种，于是 <c>compact</c> / <c>medium</c>（`MainWindow.axaml.cs` 用
/// <c>Classes.Set</c> 按窗宽挂）与 <c>on-accent</c>（`BrandLogo.axaml.cs` 同）
/// 三个**在用**的类被报成死类——12 项里有 3 项是误报。
///
/// **误报比漏报危险得多**（AGENTS.md 的死代码判定）：漏报只是少清一点，
/// 误报会让人删掉在用的东西。所以这条测试**只报告、白名单逐项写明理由**，
/// 而不是让人为了让它变绿去乱挂类名。
/// </summary>
public sealed class ThemeStyleUsageTests
{
    /// <summary>
    /// 已知死类白名单。**每一项必须写明为什么留着**——没有理由的项就该删样式，
    /// 而不是加进这张表。表越长越说明主题在腐坏。
    /// </summary>
    private static readonly IReadOnlyDictionary<string, string> KnownDeadClasses =
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            // U60 声称「窄屏收紧表单」，但这三个类全仓零挂载，所以那 4 条
            // `Window.compact` / `Window.medium` 级联规则**永不触发**——
            // 窄窗下真实发生的只有「设置页索引栏消失」（`settings-index-rail` 那条）。
            //
            // ⚠️ **保留规则、不删，是因为这是产品决策而非代码问题**：
            //   (a) 补挂：给设置页那批 `settings-section` 加挂 `panel-section`
            //       （760px 是 MinWidth，窄屏可达，22px 分区 padding 确实浪费），
            //       表单网格与竖排字段各自加挂 `form-grid` / `form-stack`。
            //       ⚠️ 会一次改掉窄/中屏下所有设置分区的 padding，须实际开窗看观感。
            //   (b) 删规则并改注释，承认响应式只做了索引栏那一条。
            // 现状（规则在、目标类空）**不要当成「响应式已经做了」**——它没做。
            // 主题里那段注释已改成如实描述，见 `AriadneTheme.axaml` 的 U60 断点级联一节。
            ["form-grid"] = "U60 响应式规则的目标类，全仓零挂载；补挂 or 删规则是待定产品决策",
            ["form-stack"] = "同 form-grid",
            ["panel-section"] = "同 form-grid；另在 Border 基础样式区也有一条定义",
        };

    /// <summary>
    /// 主题里定义了 class 选择器、却没有任何控件挂它 —— 除白名单外必须为空。
    ///
    /// 判据取「**全仓四种挂载写法的并集**」而不是只查 XAML：
    /// 三分之一的误报都来自漏扫 C# 侧的 `Classes.Set` / `Classes.Add`。
    /// </summary>
    [Fact]
    public void ThemeClassSelectorsAllHaveMountingSites()
    {
        var mounted = CollectMountedClasses();
        var defined = CollectThemeClassSelectors();

        var dead = defined
            .Where(entry => !mounted.Contains(entry.Key))
            .Where(entry => !KnownDeadClasses.ContainsKey(entry.Key))
            .Select(entry => $"  .{entry.Key} —— 定义在 AriadneTheme.axaml 第 {string.Join('/', entry.Value)} 行")
            .ToList();

        Assert.True(
            dead.Count == 0,
            "主题里这些 class 全仓无人挂载（U152）。死样式会冒充「已完成的工作」，"
            + "让下一个改样式的人误判（U151 就是这么来的）。\n"
            + "请二选一：删掉样式，或把它挂到真实控件上。\n"
            + "若确有理由保留，加进 KnownDeadClasses 并**写明理由**——"
            + "不要为了让本用例变绿而随便挂个类名。\n"
            + string.Join('\n', dead));
    }

    /// <summary>
    /// 反向对照：控件挂了 class、主题里却没有对应样式。
    ///
    /// 这条不是为了报缺陷（挂一个纯语义 class 供测试定位是合理的），
    /// 而是**证明扫描器两边都读对了**：若这一侧数量异常膨胀，
    /// 说明「定义」那侧的正则漏了写法，上一条的死类结论也就不可信。
    /// 阈值取宽松值——它是扫描器的自检，不是产品约束。
    /// </summary>
    [Fact]
    public void MountedClassesMostlyHaveThemeDefinitions()
    {
        var mounted = CollectMountedClasses();
        var defined = CollectThemeClassSelectors();
        var viewLocal = CollectViewLocalClassSelectors();

        var orphans = mounted
            .Where(name => !defined.ContainsKey(name))
            .Where(name => !viewLocal.Contains(name))
            .ToList();

        // 阈值 40：当前实测约 20 个（多为 ItemTemplate 内的语义标记与测试定位用）。
        // 这个数字翻倍就意味着扫描器某一侧读漏了，而不是产品突然多了 20 个孤儿类。
        Assert.True(
            orphans.Count < 40,
            $"挂了但无样式定义的 class 有 {orphans.Count} 个，异常偏多——"
            + "先怀疑扫描器漏了某种定义写法（视图内联 Styles / Design.PreviewWith），"
            + "而不是直接认定它们是产品缺陷：\n  "
            + string.Join("\n  ", orphans));
    }

    /// <summary>
    /// 收集全仓挂载的 class 名，**四种写法都要扫**。
    ///
    /// 漏任一种都会造出误报，而误报会让人删掉在用的东西。
    /// </summary>
    private static HashSet<string> CollectMountedClasses()
    {
        var mounted = new HashSet<string>(StringComparer.Ordinal);
        foreach (var file in EnumerateDesktopSources())
        {
            var text = File.ReadAllText(file);

            // ① XAML：Classes="a b c"
            foreach (Match match in Regex.Matches(text, @"Classes=""([^""]*)"""))
            {
                foreach (var name in match.Groups[1].Value.Split(
                    ' ', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
                {
                    mounted.Add(name);
                }
            }

            // ② XAML 条件类：Classes.selected="{Binding IsSelected}"
            foreach (Match match in Regex.Matches(text, @"Classes\.([A-Za-z][\w-]*)\s*="))
            {
                mounted.Add(match.Groups[1].Value);
            }

            // ③④ C#：Classes.Add("x") / Classes.Set("x", cond)
            // `Set` 是最容易漏的那种——compact / medium / on-accent 三个在用的类
            // 都只通过它挂载，漏掉就会被报成死类（U152 首版的 3 项误报）。
            foreach (Match match in Regex.Matches(text, @"Classes\.(?:Add|Set|Remove)\(\s*""([^""]+)"""))
            {
                mounted.Add(match.Groups[1].Value);
            }
        }

        return mounted;
    }

    /// <summary>主题文件里出现在 <c>Selector</c> 中的 class 名 → 行号列表。</summary>
    private static Dictionary<string, List<int>> CollectThemeClassSelectors()
    {
        var defined = new Dictionary<string, List<int>>(StringComparer.Ordinal);
        var lines = File.ReadAllLines(ResolveThemePath());
        for (var index = 0; index < lines.Length; index++)
        {
            foreach (Match selector in Regex.Matches(lines[index], @"Selector\s*=\s*""([^""]*)"""))
            {
                foreach (Match name in Regex.Matches(selector.Groups[1].Value, @"\.([A-Za-z][\w-]*)"))
                {
                    if (!defined.TryGetValue(name.Groups[1].Value, out var rows))
                    {
                        rows = new List<int>();
                        defined[name.Groups[1].Value] = rows;
                    }
                    rows.Add(index + 1);
                }
            }
        }

        return defined;
    }

    /// <summary>视图内联 <c>&lt;Styles&gt;</c> 里定义的 class（不在主题文件里，但同样是有定义的）。</summary>
    private static HashSet<string> CollectViewLocalClassSelectors()
    {
        var local = new HashSet<string>(StringComparer.Ordinal);
        var themePath = ResolveThemePath();
        foreach (var file in EnumerateDesktopSources())
        {
            if (string.Equals(file, themePath, StringComparison.Ordinal))
            {
                continue;
            }
            var text = File.ReadAllText(file);
            foreach (Match selector in Regex.Matches(text, @"Selector\s*=\s*""([^""]*)"""))
            {
                foreach (Match name in Regex.Matches(selector.Groups[1].Value, @"\.([A-Za-z][\w-]*)"))
                {
                    local.Add(name.Groups[1].Value);
                }
            }
        }

        return local;
    }

    private static IEnumerable<string> EnumerateDesktopSources()
    {
        var root = ResolveDesktopRoot();
        foreach (var file in Directory.EnumerateFiles(root, "*.*", SearchOption.AllDirectories))
        {
            var extension = Path.GetExtension(file);
            if (extension is not (".axaml" or ".cs"))
            {
                continue;
            }
            // 生成物里有 XAML 编译产生的副本，会把同一处使用重复计数。
            if (file.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                || file.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            {
                continue;
            }
            yield return file;
        }
    }

    private static string ResolveDesktopRoot() =>
        Path.Combine(ResolveSolutionDir(), "Ariadne.Desktop");

    private static string ResolveThemePath() =>
        Path.Combine(ResolveDesktopRoot(), "Resources", "Styles", "AriadneTheme.axaml");

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
