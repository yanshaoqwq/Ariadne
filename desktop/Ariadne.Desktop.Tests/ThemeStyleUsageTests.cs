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

    /// <summary>
    /// 每个 <c>{DynamicResource Ariadne.*}</c> 引用的 key 都必须在主题里真的有定义。
    ///
    /// **为什么需要这条守卫**：Avalonia 的 <c>DynamicResource</c> 在 key 缺失时
    /// **既不报错、也不回落**——属性留在未赋值状态。后果按属性而异：
    /// <c>Foreground</c> 退回继承值（看起来"只是颜色不对"），
    /// 而 <c>Ellipse.Fill</c> 拿到 null 意味着**那个形状压根没画出来**。
    ///
    /// 实际发生过两处（2026-08-18 修）：
    /// - <c>Ariadne.TextMuted</c>（正确名 <c>TextSubtle</c>）—— 作品页索引圆点
    ///   从未画出，「未选中项更淡」这个设计意图也从未生效；
    /// - <c>Ariadne.StatusDanger</c>（正确名 <c>StatusError</c>）——
    ///   配置页 **7 处校验错误文案**字色未赋值。用户填错模型 id / 成本 /
    ///   上下文长度时，错误提示不显危险色。
    ///
    /// 两处都是**拼错了一个近义词**，且都活了很久：没有任何编译期或运行期报错
    /// 途径，只能靠键集合比对发现。（`display_name.json` 缺 key 至少会返回
    /// `[key]` 让人看见，主题这边连这个都没有。）
    ///
    /// 判据取「引用集合 ⊆ 定义集合」而非逐个白名单：白名单要人工维护，
    /// 新加一个 token 就多一处忘记更新的机会。
    /// </summary>
    [Fact]
    public void EveryDynamicResourceTokenReferenceHasADefinition()
    {
        var defined = CollectDefinedThemeTokens();

        var missing = new SortedDictionary<string, List<string>>(StringComparer.Ordinal);
        // 用现成的 EnumerateDesktopSources：它已排除 obj/bin——生成物里有 XAML
        // 编译产生的副本，会把同一处使用重复计数。
        foreach (var file in EnumerateDesktopSources())
        {
            if (!file.EndsWith(".axaml", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }
            var relative = Path.GetRelativePath(ResolveDesktopRoot(), file);
            // 同样剔注释：注释里提到旧键名（例如"为什么从 TextMuted 改成 TextSubtle"）
            // 是历史记录，不是引用。U177 复核时就被这类文字污染过计数。
            foreach (Match match in Regex.Matches(
                StripXamlComments(File.ReadAllText(file)), @"\{DynamicResource (Ariadne\.[A-Za-z0-9.]+)\}"))
            {
                var key = match.Groups[1].Value;
                if (defined.Contains(key))
                {
                    continue;
                }
                if (!missing.TryGetValue(key, out var sites))
                {
                    sites = new List<string>();
                    missing[key] = sites;
                }
                if (!sites.Contains(relative))
                {
                    sites.Add(relative);
                }
            }
        }

        Assert.True(
            missing.Count == 0,
            "以下 token 被引用但主题里没有定义。DynamicResource 缺 key 时**不报错也不回落**，"
            + "属性会留在未赋值状态（Foreground 退回继承值、Fill 直接不画）：\n"
            + string.Join("\n", missing.Select(entry =>
                $"  {entry.Key} ← {string.Join('、', entry.Value)}")));
    }

    /// <summary>
    /// 主题里 <c>x:Key="Ariadne.*"</c> 的全部定义。三条守卫共用同一份定义集合——
    /// 各自重算一遍正则的话，改定义侧写法时只会修好其中一条，另两条静默失去意义。
    /// </summary>
    private static HashSet<string> CollectDefinedThemeTokens()
    {
        var theme = File.ReadAllText(ResolveThemePath());
        var defined = Regex
            .Matches(theme, @"x:Key=""(Ariadne\.[A-Za-z0-9.]+)""")
            .Select(match => match.Groups[1].Value)
            .ToHashSet(StringComparer.Ordinal);
        Assert.True(defined.Count > 50, $"主题里只解析出 {defined.Count} 个 token 定义，正则大概失效了");
        return defined;
    }

    /// <summary>
    /// 把 XAML 注释体替换成等量换行，**保留行号**。
    ///
    /// 为什么必须做：主题文件第 434 行的注释在教人怎么用图标——
    /// <c>用法：&lt;Path Data="{StaticResource Ariadne.Icon.X}" .../&gt;</c>。
    /// 那个 <c>X</c> 是占位符而非真键，不剔注释就会被报成缺失键。
    /// U177 报告里踩过同型：用 <c>grep -c</c> 数原始文本、把说明文字算成缺陷，
    /// 于是"还剩 9 处"其实只有 8 处。**误报会让人去改本来正确的东西。**
    ///
    /// 换成等量 <c>\n</c> 而不是直接删：失败信息里的行号是人唯一的定位依据，
    /// 删掉注释会让后面每一行的行号整体前移，报出的位置指向别处。
    /// </summary>
    private static string StripXamlComments(string text) =>
        Regex.Replace(
            text,
            "<!--.*?-->",
            match => new string('\n', match.Value.Count(ch => ch == '\n')),
            RegexOptions.Singleline);

    /// <summary>
    /// U180 第 1 类：<c>{StaticResource Ariadne.*}</c> 缺键**同样静默**，所以同样要守。
    ///
    /// **纳入的依据是实测，不是类比**（2026-08-18，Avalonia 12.0.5 / net10.0）。
    /// 直觉上 StaticResource 是加载期解析、"缺键该抛异常"，实测三步全部否定了这个直觉：
    /// <list type="number">
    ///   <item>把 <c>WelcomeView.axaml</c> 里一处 <c>Data="{StaticResource Ariadne.Icon.Add}"</c>
    ///     改成不存在的 <c>Ariadne.Icon.AddZZZNOPE</c>，<c>dotnet build</c>
    ///     ⇒ <b>0 警告 0 错误</b>。XAML 编译器不校验资源键存在性。</item>
    ///   <item>headless 里 <c>new WelcomeView()</c> ⇒ <b>不抛异常</b>，构造正常返回。</item>
    ///   <item>遍历该视图逻辑树的 13 个 <c>Path</c>，<c>Data == null</c> 的恰好 <b>1 个</b>
    ///     ⇒ 就是那个缺键的图标，**它压根没画出来**。</item>
    /// </list>
    /// 也即：StaticResource 缺键与 DynamicResource 缺键**失效形态完全一致**——
    /// 无编译期报错、无运行期异常、属性留 null。<b>它不属于"已有报错途径"，
    /// 所以纳入守卫的价值与 DynamicResource 那条等同。</b>
    ///
    /// 危害在本仓库尤其实在：全仓 53 处非主题文件的 StaticResource **几乎全是
    /// <c>Path.Data="{StaticResource Ariadne.Icon.*}"</c>**（图标 geometry）。
    /// <c>Data</c> 为 null 的 <c>Path</c> 不绘制任何东西 ⇒ 拼错一个图标名的后果是
    /// **按钮上的图标整个消失**，而按钮本身、tooltip、点击行为都正常。
    /// 这比 U177 的"错误文案不显红"更难从截图上察觉，因为空白处看不出少了什么。
    ///
    /// 判据与上一条同构（引用集合 ⊆ 定义集合），刻意不做白名单。
    /// </summary>
    [Fact]
    public void EveryStaticResourceTokenReferenceHasADefinition()
    {
        var defined = CollectDefinedThemeTokens();

        var missing = new SortedDictionary<string, List<string>>(StringComparer.Ordinal);
        foreach (var file in EnumerateDesktopSources())
        {
            if (!file.EndsWith(".axaml", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }
            var relative = Path.GetRelativePath(ResolveDesktopRoot(), file);
            // 剔注释：主题里有一处"用法：{StaticResource Ariadne.Icon.X}"的说明文字，
            // 不剔就会把占位符 X 报成缺失键（U177 同型误报）。
            var text = StripXamlComments(File.ReadAllText(file));
            foreach (Match match in Regex.Matches(text, @"\{StaticResource (Ariadne\.[A-Za-z0-9.]+)\}"))
            {
                var key = match.Groups[1].Value;
                if (defined.Contains(key))
                {
                    continue;
                }
                var line = text.Take(match.Index).Count(ch => ch == '\n') + 1;
                if (!missing.TryGetValue(key, out var sites))
                {
                    sites = new List<string>();
                    missing[key] = sites;
                }
                sites.Add($"{relative}:{line}");
            }
        }

        Assert.True(
            missing.Count == 0,
            "以下 token 以 StaticResource 被引用但主题里没有定义。实测（Avalonia 12.0.5）"
            + "StaticResource 缺键与 DynamicResource 一样**既不编译期报错、也不运行期抛异常**，"
            + "属性留在 null——本仓库这类引用几乎全是 Path.Data 图标 geometry，"
            + "null 意味着**那个图标整个不画**：\n"
            + string.Join("\n", missing.Select(entry =>
                $"  {entry.Key} ← {string.Join('、', entry.Value)}")));
    }

    /// <summary>
    /// U180 第 2 类：C# 侧的资源键字面量也必须有定义。
    ///
    /// **为什么要守**：XAML 不是唯一的键来源。这些地方的键是 C# 字符串，
    /// 走 <c>TryFindResource</c> / <c>TryGetResource</c>，
    /// 拼错时**连"属性留 null"都不会发生**——三处调用都写成
    /// <c>if (TryFind…) { 赋值 }</c>，查不到就整个跳过赋值分支，
    /// 静默程度比 XAML 那两类更高（没有任何一处会走进异常或日志）：
    /// <list type="bullet">
    ///   <item><c>ConfirmDialogViewModel.IconBrushKey</c> —— 5 个 severity 分支，
    ///     拼错则对话框图标停在继承色，"危险"与"提示"看起来一样；</item>
    ///   <item><c>WorkspacePageView.PortKindBrushKey</c> —— 3 种引脚色，
    ///     拼错则拖拽橡皮筋线不换色（该函数注释明确说"查不到时不动 Stroke"）；</item>
    ///   <item><c>MainWindow</c> 的 Restore/Maximize 图标、
    ///     <c>WorksPageView</c> 的 <c>Ariadne.Reading.LineHeight</c>（拼错则
    ///     编辑器行高不与阅读态对齐，正是 U151 那个缺陷的形态）；</item>
    ///   <item><c>AppIconPainter.ResolveColor</c> 的 13 个调用点。</item>
    /// </list>
    /// 这些当前**恰好全部命中定义**（前一轮已逐个追查）。本条的价值不是现在报缺陷，
    /// 而是把"现在恰好是对的"变成"以后改坏会当场红"——
    /// 与 U177 那两处一样，这类拼写错误没有任何别的发现途径。
    ///
    /// **怎么在不维护映射表的前提下认出"哪些字面量是资源键"**：
    /// 用命名约定——主题里 190 个键**每一段都以大写字母开头**
    /// （实测：190/190 匹配 <c>Ariadne(\.[A-Z]\w*)+</c>，零例外）。
    /// 非键的 <c>Ariadne.*</c> 字面量都是文件名/包名，段首小写
    /// （<c>Ariadne.app</c>、<c>Ariadne.iconset</c>）⇒ 天然被约定排除，
    /// **不需要为它们写豁免项**。这比"列出所有调用点"的白名单稳：
    /// 新增一个 <c>TryFindResource</c> 调用点会自动被覆盖，而不是被漏掉。
    /// </summary>
    [Fact]
    public void EveryCSharpResourceKeyLiteralHasADefinition()
    {
        var defined = CollectDefinedThemeTokens();

        // 主题令牌命名约定：`Ariadne` 之后每一段都以大写字母开头。
        // 用它把资源键从"恰好也叫 Ariadne.xxx"的文件名字面量里区分出来。
        var keyShaped = new Regex(@"^Ariadne(\.[A-Z][A-Za-z0-9]*)+$");
        Assert.True(
            defined.All(key => keyShaped.IsMatch(key)),
            "主题里存在不符合「每段首字母大写」约定的 token，本用例赖以区分"
            + "「资源键」与「同前缀的文件名字面量」的判据已失效，需改判据而不是加豁免：\n  "
            + string.Join("\n  ", defined.Where(key => !keyShaped.IsMatch(key))));

        var missing = new SortedDictionary<string, List<string>>(StringComparer.Ordinal);
        foreach (var file in EnumerateDesktopSources())
        {
            if (!file.EndsWith(".cs", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }
            var relative = Path.GetRelativePath(ResolveDesktopRoot(), file);
            var text = File.ReadAllText(file);
            foreach (Match match in Regex.Matches(text, @"""(Ariadne\.[A-Za-z0-9.]+)"""))
            {
                var key = match.Groups[1].Value;
                if (!keyShaped.IsMatch(key) || defined.Contains(key) || IsRuntimeWrittenKey(key))
                {
                    continue;
                }
                var line = text.Take(match.Index).Count(ch => ch == '\n') + 1;
                if (!missing.TryGetValue(key, out var sites))
                {
                    sites = new List<string>();
                    missing[key] = sites;
                }
                sites.Add($"{relative}:{line}");
            }
        }

        Assert.True(
            missing.Count == 0,
            "以下资源键在 C# 里被查询但主题里没有定义。这条路径比 XAML 更静默——"
            + "`TryFindResource` 查不到时调用方直接跳过赋值分支，既无异常也无日志，"
            + "表现为「那个颜色/图标/行高就是没跟着变」：\n"
            + string.Join("\n", missing.Select(entry =>
                $"  {entry.Key} ← {string.Join('、', entry.Value)}")));
    }

    /// <summary>
    /// U180 第 3 类（**按实测改了方向**）：吃 <c>Color</c> 的属性只能绑 Color 令牌。
    ///
    /// ⚠️ **任务书假设的那个方向经实测不成立，所以没做**：
    /// 「把 <c>Color</c> 赋给 <c>Background</c>/<c>Fill</c>（需要 <c>IBrush</c>）会静默失效」
    /// 是错的。Avalonia 12.0.5 有 <c>Color → IBrush</c> 的隐式转换，实测两次都正常上色：
    /// <list type="bullet">
    ///   <item>headless 里给 <c>Border.Background</c> / <c>Ellipse.Fill</c> 绑
    ///     <c>Ariadne.Color.AccentPrimary</c>（主题里确是 <c>&lt;Color&gt;</c>）
    ///     ⇒ 取到 <c>#ff2e726b</c>，不是 null；</item>
    ///   <item>把同样的改动写进 <c>WelcomeView.axaml</c> 走**编译** XAML
    ///     ⇒ 同样是 <c>#ff2e726b</c>。</item>
    /// </list>
    /// 也即这个方向**根本不是缺陷**，为它写守卫会拦住正确的代码。
    ///
    /// **但反方向是真的**：吃 <c>Color</c> 的属性绑到 Brush 令牌上会静默拿到
    /// <c>#00000000</c>（全透明）—— 实测给 <c>GradientStop.Color</c> 绑
    /// <c>Ariadne.AccentPrimary</c>（Brush）即得此结果。Brush→Color 没有反向转换，
    /// 于是**渐变整段变透明**，而控件、布局、其它渐变端点全都正常。
    /// 这正是 <c>ThemeOverlayKeysTests</c> 注释里记的那个真实回归的形态
    /// （渐变面用户换强调色后纹丝不动），所以这个方向值得守。
    ///
    /// **判据不需要人工维护映射表**（这是决定做不做的关键）：
    /// 主题自己就声明了类型 —— <c>&lt;Color x:Key="Ariadne.Color.*"&gt;</c> 118 处、
    /// 其余 <c>Ariadne.*</c> 是 Brush/Geometry/Thickness。而 XAML 侧
    /// 「哪些属性要 Color」也有零维护的判据：**属性名以 <c>Color</c> 结尾**
    /// （<c>Color=</c> / <c>TintColor=</c>，实测全仓 166 处引用全部指向
    /// <c>Ariadne.Color.*</c>，零例外）。两侧都从源码推导，
    /// 新加一个 Color 令牌或一个吃 Color 的属性都会自动纳入。
    /// </summary>
    [Fact]
    public void ColorTypedPropertiesOnlyBindColorTokens()
    {
        var colorTokens = Regex
            .Matches(
                File.ReadAllText(ResolveThemePath()),
                @"<Color\s+x:Key=""(Ariadne\.[A-Za-z0-9.]+)""")
            .Select(match => match.Groups[1].Value)
            .ToHashSet(StringComparer.Ordinal);
        Assert.True(
            colorTokens.Count > 50,
            $"只解析出 {colorTokens.Count} 个 <Color> 令牌，正则大概失效了");

        var offenders = new List<string>();
        foreach (var file in EnumerateDesktopSources())
        {
            if (!file.EndsWith(".axaml", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }
            var relative = Path.GetRelativePath(ResolveDesktopRoot(), file);
            var text = StripXamlComments(File.ReadAllText(file));
            // 属性名以 Color 结尾（Color= / TintColor= / …）＝ 该属性的类型是 Color。
            foreach (Match match in Regex.Matches(
                text,
                @"([A-Za-z]*Color)=""\{(?:Static|Dynamic)Resource (Ariadne\.[A-Za-z0-9.]+)\}"""))
            {
                var key = match.Groups[2].Value;
                if (colorTokens.Contains(key))
                {
                    continue;
                }
                var line = text.Take(match.Index).Count(ch => ch == '\n') + 1;
                offenders.Add($"  {relative}:{line} —— {match.Groups[1].Value}=\"{key}\"");
            }
        }

        Assert.True(
            offenders.Count == 0,
            "以下属性的类型是 Color，却绑了非 Color 令牌（多半是 Brush）。"
            + "实测 Avalonia 12.0.5 **没有 Brush→Color 的反向转换**，"
            + "属性会静默取到 #00000000 全透明——渐变整段消失，"
            + "而控件与其它端点全都正常，极难从截图察觉。"
            + "请改绑对应的 Ariadne.Color.* 令牌：\n"
            + string.Join("\n", offenders));
    }

    /// <summary>
    /// 运行时写入应用级资源字典、因此主题文件里查不到的键 —— **不算缺失**。
    ///
    /// 只有一项：<c>Ariadne.ThemeOverlay.Active</c>（<c>ThemeApplication.cs:35</c>）。
    /// 它是个哨兵值而非主题令牌：<c>Apply</c> 在 <c>:271</c> 写入当前覆盖层 id、
    /// <c>Reset</c> 在 <c>:357</c> 移除，用来判断"当前是否挂着个性化覆盖层"。
    /// 没有控件绑它，主题字典里也**不该**有它的定义。
    ///
    /// 判据刻意取"以 <c>Ariadne.ThemeOverlay.</c> 打头"而不是精确等值：
    /// 这类运行时哨兵将来若增加（例如再加个 <c>.Variant</c>），
    /// 按前缀走就不用回来改这里；而它与真令牌的命名空间是分开的，不会误放真缺陷。
    ///
    /// ⚠️ <c>ThemeApplication.OverlayBrushKeys</c> 里那 40 多个键**不在豁免范围内**：
    /// 它们是 <c>Apply</c> 覆盖的**既有**令牌，主题字典里必须有预设值
    /// （否则用不上个性化主题时控件失色），所以它们该被本用例检查、也确实全部命中。
    /// </summary>
    private static bool IsRuntimeWrittenKey(string key) =>
        key.StartsWith("Ariadne.ThemeOverlay.", StringComparison.Ordinal);

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
