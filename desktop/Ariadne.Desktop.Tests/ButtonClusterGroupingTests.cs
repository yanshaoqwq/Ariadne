using System.Text.RegularExpressions;
using System.Xml;
using System.Xml.Linq;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U205：横向按钮簇的**组边界**覆盖率。
///
/// # 这个文件守的到底是什么
///
/// U205 的结论是「间距不是根因，分组才是」：做得最好的那处（画布工具条）用的是
/// **2px 极小间距**，照样读得清，因为它有分组线；而绝大多数按钮簇没有任何分组手段，
/// 只能靠加大间距硬撑 —— 而加间距在已经溢出的顶栏上根本没有余量（U194-G）。
///
/// **度量口径取「连续无分隔目标数」，不是「间距」**。这一条是报告自己纠正过的
/// 度量错误，值得钉住：用间距做分母会把「分了组的紧凑排列」（画布工具条 2px）
/// 排成全项目最差，把「没分组但间距大的一长排」排成健康 —— 后者才是真正难扫视的。
///
/// # 为什么判据是「逐一对应」而不是「存在性」
///
/// 「某处有分隔线」这种断言在只装 2 处时照样绿（缺陷版本正是只装了 3 处）。
/// 所以本文件：
///   ① 机械发现全部**最外层**横向簇（**28 个实例 / 27 个签名** —— 见下），
///   ② 要求每一个簇都在 <see cref="ClusterRegistry"/> 里有判定
///      ⇒ 新写一个按钮簇而不分类时**必红**，
///   ③ 对判定为「需要组边界」的 9 个实例（8 个签名），逐个断言它真的有
///      <c>Border.group-divider</c> 且最长连排 ≤ 3。
///
/// **28 实例 / 27 签名**：`SaveKeyText | RevokeKeyText` 在 `SettingsPageView.axaml`
/// 的 494 与 524 行各有一份（两个服务商配置区各一套输入行），签名相同。
/// 登记表按签名索引 ⇒ 一项判定管两处，这是刻意的：同型两处应当同判同修，
/// 分开登记只会让其中一处被漏改。但**计数用实例数**，否则「有人复制出第三份」
/// 会静默通过。
///
/// # 修复前后的实测数字（口径见上）
///
/// | 指标 | 修复前 | 修复后 |
/// |---|---|---|
/// | 最外层横向簇（实例） | 28 | 28 |
/// | 需要组边界的簇里已有边界的（实例） | **3 / 9** | **9 / 9** |
/// | 全仓分组线实例 | 5（3 个地点、**3 套各不相同的手写数字**） | 12（全部走同一个 class + token） |
/// | 手写内联发丝线 | 5 | **0** |
/// | 最长连排 ≥ 4 的簇 | 3（作品页顶栏 5、画布视图组 5、运行控制 4） | 1（画布视图组 5，报告定夺「模板级别动」） |
///
/// ⚠️ **报告里 M=36 / N=5 这两个数字我重新数过**，见本文件 <c>DiscoverClusters</c>
/// 的口径说明与 U205 报告末尾的更正小节：36 是「直接子元素含 ≥2 个可点控件的容器」，
/// 里面有 6 个是**纵向**排布（空态按钮竖排、导航导轨），谈不上「横向堆在一起」；
/// 且它把画布工具条按 3 个内层分组各算一簇，而视觉上那是**一条**工具条。
/// 按「最外层横向连续目标行」重数是 28 个实例（27 个不同签名）。N=5 那一项报告写的是
/// 「3 个分组标题 TextBlock + 2 个发丝线地点」，实测发丝线是 **3 个地点共 5 个实例**。
/// </summary>
public sealed class ButtonClusterGroupingTests
{
    /// <summary>可点目标：Button 族。ComboBox/TextBox 不算 —— 它们不是"按钮簇"的成员。</summary>
    private static readonly HashSet<string> ClickableTags = new(StringComparer.Ordinal)
    {
        "Button", "ToggleButton", "RepeatButton", "SplitButton", "HyperlinkButton", "DropDownButton",
    };

    /// <summary>
    /// 菜单容器。**必须排除**：报告里推翻过一条 agent 结论 ——
    /// <c>Separator</c> 并非「全仓 0 命中」，实为 10 处，但**全部在菜单里**。
    /// ⇒ 「团队不会用分隔线」是错的前提，真相是「菜单分了组、按钮簇没分」。
    /// 把菜单卷进来统计会让覆盖率虚高，掩盖掉这个区分。
    /// </summary>
    private static readonly HashSet<string> MenuTags = new(StringComparer.Ordinal)
    {
        "MenuItem", "ContextMenu", "MenuFlyout", "Menu", "NativeMenu",
    };

    private const string DividerClass = "group-divider";

    /// <summary>簇的判定。</summary>
    private enum Verdict
    {
        /// <summary>混合了不同类别的动作（尤其是不可逆动作紧邻常规动作）⇒ 必须有组边界。</summary>
        NeedsBoundary,

        /// <summary>整簇是**一个**语义组（分段控件、tab 条、主/次动作对、窗口 chrome…）⇒ 加线反而是噪点。</summary>
        SingleGroup,
    }

    /// <summary>
    /// 全部 27 个最外层横向簇的判定表。**键是内容签名而不是行号**：
    /// 行号会随上下任何一处改动漂移，用行号做键的登记表在第二次改动后就全是假失败。
    ///
    /// ⚠️ 每一项都必须写理由。判为 <see cref="Verdict.SingleGroup"/> 时理由要回答
    /// 「为什么这些目标属于同一组」——写不出来的就该是 NeedsBoundary。
    /// </summary>
    private static readonly IReadOnlyDictionary<string, (Verdict Verdict, string Reason)> ClusterRegistry =
        new Dictionary<string, (Verdict, string)>(StringComparer.Ordinal)
        {
            // ---------- 需要组边界的 8 个 ----------
            ["MainWindow.axaml :: subtle:DiagnosticToggleText | subtle:DiagnosticClearText"] =
                (Verdict.NeedsBoundary,
                 "U205-C：「清除」会丢掉唯一的故障线索（诊断横幅是失败信息的兜底显示位），"
                 + "与「展开详情」原为两个同款 subtle 键、等距 8px、零视觉区分"),
            // ⚠️ 签名里**没有** chip-filter 那颗键：它在 `ItemsControl` 的 ItemTemplate 里
            // （`ActiveIdFilters` 每项一颗），静态 XAML 扫描看不到模板实例化出来的东西。
            // 施工者原来把它写进了签名，实测对不上 ⇒ 改签名而不是改发现逻辑：
            // 让扫描器去展开数据模板等于要它推断运行时数据，那条路没有尽头。
            // 判定不受影响 —— 缺陷本体是「清除筛选」与「标记已读」两类动作零区分。
            ["RunLogPageView.axaml :: subtle:ClearFiltersText | secondary:MarkReadText"] =
                (Verdict.NeedsBoundary,
                 "U205-F：「清除筛选」作用于筛选条件，「标记已读」改的是记录状态且不可撤销 —— 两类"),
            ["SettingsPageView.axaml :: secondary:SaveKeyText | danger:RevokeKeyText"] =
                (Verdict.NeedsBoundary,
                 "U195-I / U205-E：作者填完 key 想点保存，右边紧挨着就是不可逆的吊销，ColumnSpacing 一视同仁"),
            ["SettingsPageView.axaml :: secondary:RestoreOfficialTemplateRepositoryText | primary:SaveTemplateRepositoryText"] =
                (Verdict.NeedsBoundary,
                 "U205-E：与 U195-I 同型第二例 ——「恢复官方模板库」丢弃当前地址且不可撤销，却与保存等距 10px"),
            ["WelcomeView.axaml :: subtle:TutorialText | subtle:FeedbackText"] =
                (Verdict.NeedsBoundary,
                 "产品内引导 vs 外部反馈通道，两条不同去向的路；此处修复前就已有分组线（全项目 3 个地点之一）"),
            ["WorksPageView.axaml :: segment-button:ReadModeText | segment-button:EditModeText | icon-btn:QuickEditTitle | icon-btn:CreateChapterText | icon-btn:ImportText | icon-btn:ExportText | icon-btn save:SaveText"] =
                (Verdict.NeedsBoundary,
                 "U195-G-①（当年记为「未做」的那一半）：7 个目标 4 类，修复前只有 1 条线且 5 个动作内部零分组"),
            ["WorkspacePageView.axaml :: icon-btn:UndoText | icon-btn:RedoText | icon-btn:SubworkflowText | icon-btn:ReloadProjectCanvasText | icon-btn:ExportText | icon-btn save:SaveToolTipText | icon-btn:ZoomOutText | subtle zoom-readout:ResetZoomText | icon-btn:CtxFitViewText | icon-btn:CanvasFocusText | icon-btn:ZoomInText | icon-btn:OpenConfirmationHistoryText"] =
                (Verdict.NeedsBoundary,
                 "U195-L 判定的全项目模板：12 个目标 4 组（编辑/文件/视图/审计），分组线是它可读的唯一原因"),
            ["WorkspacePageView.axaml :: primary btn-compact:RunText | icon-btn:PauseText | icon-btn:ResumeText | icon-btn:StopText"] =
                (Verdict.NeedsBoundary,
                 "U195-H：暂停/继续是可逆流控，停止是不可逆终止，三键原为等距 4px ——"
                 + "运行中最容易误触的那一对距离最短"),

            // ---------- 单一语义组的 19 个 ----------
            ["MainWindow.axaml :: window-control:MinimizeWindowText | window-control:MaximizeWindowText | window-control close:CloseWindowText"] =
                (Verdict.SingleGroup,
                 "窗口 chrome 三键是一个整体（操作系统级约定，全平台都不在其间分组）；"
                 + "关闭键另有 StatusError 描边 + 加粗 hover + MainWindowCloseGuard 兜底。U205 附一判为健康"),
            ["RunLogPageView.axaml :: secondary btn-compact:CopySelectedText | secondary btn-compact:RefreshText"] =
                (Verdict.SingleGroup, "同为无损读取动作（导出到剪贴板 / 重新拉取），都不改数据"),
            ["SettingsPageView.axaml :: secondary:AddText | secondary:BrowseText"] =
                (Verdict.SingleGroup,
                 "同一个动作的两条来源：BrowseAsync 内部直接调 TryAdd（SettingsPageViewModel.cs:8944），"
                 + "「浏览」不是填输入框而是**也把路径加进列表**，两键结果相同"),
            ["SettingsPageView.axaml :: secondary:RestoreCurrentTabText | secondary:RestoreRecommendedDefaultsText"] =
                (Verdict.SingleGroup,
                 "两者同类（都是恢复），组边界分不开同类动作；它们真正的问题是**作用范围不可区分**，"
                 + "那是 U195-K（P2）的诉求 —— 要改文案/加范围说明，不是加线"),
            ["SettingsPageView.axaml :: secondary:TestProviderDraftText | secondary:RefreshText"] =
                (Verdict.SingleGroup, "同为对服务商的探测动作（测试连通 / 拉取模型目录），都不改本地配置"),
            ["SettingsPageView.axaml :: tab:ThemeEditDayText | tab:ThemeEditNightText"] =
                (Verdict.SingleGroup, "日/夜是同一个 tab 组的两个互斥项，组内不应再分组"),
            ["SettingsPageView.axaml :: theme-color-slot:SelectThemeMainChannelCommand | theme-color-slot:SelectThemeSurfaceChannelCommand | theme-color-slot:SelectThemeBrandChannelCommand"] =
                (Verdict.SingleGroup, "三个色槽是一组同级选择项，UniformGrid 等分本身就是组内排布"),
            ["SettingsPageView.axaml :: secondary:RefreshDiagnosticsText | secondary:CopyDiagnosticsText"] =
                (Verdict.SingleGroup, "同为诊断读取动作，都不改任何状态"),
            ["TemplateMarketPageView.axaml :: secondary btn-compact:#Root.DataContext.DetailText | primary btn-compact:#Root.DataContext.ImportText"] =
                (Verdict.SingleGroup, "卡片上的标准「次要 + 主要」动作对，主次已由 secondary/primary 的色彩层级表达"),
            ["WelcomeView.axaml :: welcome-recent-item:OpenCommand | subtle:⋯"] =
                (Verdict.SingleGroup,
                 "行本体是整行点击区（不是并排小按钮），⋯ 是它的溢出菜单；"
                 + "U205 附三已用真实渲染验掉「0px 间隙」这条 —— 那个键真正的缺陷是字形缺失（U206-A / U10000），"
                 + "在这里调间距/加线会改错东西"),
            ["WorksPageView.axaml :: secondary:QuickEditUndoText | primary:QuickEditApplyText"] =
                (Verdict.SingleGroup, "撤销/应用是同一次快速改写的一对决议键，属一组"),
            ["WorksPageView.axaml :: inspector-tab:NavTreeText | inspector-tab:ProjectAiText"] =
                (Verdict.SingleGroup, "右栏 tab 组的两个互斥项"),
            ["WorkspacePageView.axaml :: primary:CtxAddStartText | secondary:NodeLibraryText"] =
                (Verdict.SingleGroup, "空画布空态的「主要 + 次要」引导动作对"),
            ["WorkspacePageView.axaml :: secondary:VariableFill.UndoText | primary:VariableFill.ApplyText"] =
                (Verdict.SingleGroup, "变量填值面板的一对决议键，属一组"),
            ["WorkspacePageView.axaml :: tab:NodeLibraryText | tab:ExecutionText"] =
                (Verdict.SingleGroup, "画布下栏 tab 组的两个互斥项"),
            ["WorkspacePageView.axaml :: secondary:ExpandConfirmationsText | secondary:RefreshConfirmationsText"] =
                (Verdict.SingleGroup, "同为无损动作（展开视图 / 重新拉取），都不改确认项状态"),
            ["WorkspacePageView.axaml :: secondary:CollapseConfirmationsText | secondary:RefreshConfirmationsText"] =
                (Verdict.SingleGroup, "同上（收起态的镜像）"),
            ["WorkspacePageView.axaml :: secondary:RejectButtonText | primary:ApproveButtonText"] =
                (Verdict.SingleGroup,
                 "拒绝/通过是同一条确认项的一对决议键；源码注释说明了武装态下「通过」让位为「取消」，"
                 + "U205 附一已判为刻意设计、不算缺陷"),
            ["WorkspacePageView.axaml :: inspector-tab:ProjectAiText | inspector-tab:NodeDetailsText | inspector-tab:EdgeDetailsText"] =
                (Verdict.SingleGroup, "检查器 tab 组的三个互斥项"),
        };

    /// <summary>
    /// 「最长连排 ≤ 3」的豁免。**每项必须写明为什么豁免**。
    /// 这张表越长越说明约定在腐坏；现在只有一项，且是报告明令「别动」的那处。
    /// </summary>
    private static readonly IReadOnlyDictionary<string, (int MaxRun, string Reason)> LongRunWaivers =
        new Dictionary<string, (int, string)>(StringComparer.Ordinal)
        {
            ["WorkspacePageView.axaml :: icon-btn:UndoText | icon-btn:RedoText | icon-btn:SubworkflowText | icon-btn:ReloadProjectCanvasText | icon-btn:ExportText | icon-btn save:SaveToolTipText | icon-btn:ZoomOutText | subtle zoom-readout:ResetZoomText | icon-btn:CtxFitViewText | icon-btn:CanvasFocusText | icon-btn:ZoomInText | icon-btn:OpenConfirmationHistoryText"] =
                (5,
                 "画布工具条视图组 5 键（− 倍率 Fit 焦点 +）。U195-L / U205 附一两次判定这条工具条"
                 + "「模板级，别动」；U195-L 已把「视图组 5 键偏多」记为小瑕疵，"
                 + "拆它要先解决「− 与 + 被三个键隔开」（U195-N）那条排序问题，属另一条线的活。"
                 + "⚠️ 在这里顺手插一条线会把 − 和 + 分到两组，那是把小瑕疵换成真缺陷。"),
        };

    // ==================== 判据 ====================

    /// <summary>
    /// 每一个最外层横向簇都必须在登记表里有判定。
    ///
    /// 这一条是整个文件的**闸门**：它让「新写一个按钮簇」这件事必须同时回答
    /// 「它是一组还是多组」。缺陷版本的成因不是有人反对分组，而是
    /// **分组从来不是一个必须回答的问题** —— 于是默认答案永远是"不分"。
    /// </summary>
    [Fact]
    public void EveryHorizontalButtonClusterIsClassified()
    {
        var clusters = DiscoverClusters();

        var unknown = clusters
            .Where(cluster => !ClusterRegistry.ContainsKey(cluster.Signature))
            .Select(cluster => $"{cluster.File}:{cluster.Line} <{cluster.Tag}> 目标={cluster.TargetCount}\n    \"{cluster.Signature}\"")
            .ToList();

        Assert.True(
            unknown.Count == 0,
            $"有 {unknown.Count} 个横向按钮簇没在 ClusterRegistry 里分类（U205-A）。\n"
            + "请判定它是「单一语义组」还是「需要组边界」，并把**理由**写进表里 ——\n"
            + "写不出「为什么这些目标属于同一组」的，就该判 NeedsBoundary 并加上 "
            + $"<Border Classes=\"{DividerClass}\" />。\n\n"
            + string.Join("\n", unknown));

        var stale = ClusterRegistry.Keys
            .Where(key => clusters.All(cluster => cluster.Signature != key))
            .ToList();
        Assert.True(
            stale.Count == 0,
            $"登记表里有 {stale.Count} 项在源码里已不存在（改过文案或删过按钮？）——\n"
            + "过期登记项会让覆盖率数字虚高：它们照样被计入分母。\n  "
            + string.Join("\n  ", stale)
            + $"\n\n实际发现的 {clusters.Count} 个簇（用于比对签名差异）：\n  "
            + string.Join("\n  ", clusters.Select(cluster =>
                $"{cluster.File}:{cluster.Line} 目标={cluster.TargetCount} \"{cluster.Signature}\"")));

        // 总数钉死：簇数变了必须回来看一眼是不是又堆出一排。
        //
        // ⚠️ **28 个实例 / 27 个签名**：`SaveKeyText | RevokeKeyText` 在
        // `SettingsPageView.axaml` 的 494 与 524 行**各有一份**（两个服务商配置区
        // 各一套输入行），签名完全相同。登记表按签名索引 ⇒ 一项判定覆盖两个实例，
        // 这是对的：同型的两处应当同判、同修，分开登记只会让其中一处被漏改。
        // 但计数必须用实例数，否则「有人复制出第三份」这件事会静默通过。
        Assert.Equal(28, clusters.Count);
        Assert.Equal(
            27,
            clusters.Select(cluster => cluster.Signature).Distinct(StringComparer.Ordinal).Count());
    }

    /// <summary>
    /// **本文件的主判据**：判为「需要组边界」的簇，逐个必须真的有组边界。
    ///
    /// 判据取「逐一对应」而非「存在性」：断言「某处有分隔线」在只装 3 处时照样绿，
    /// 而缺陷版本正是只装了 3 处（8 个需要边界的簇里 3 个有 = 3/8）。
    /// </summary>
    [Fact]
    public void EveryClusterThatNeedsABoundaryHasOne()
    {
        var clusters = DiscoverClusters();
        var needBoundary = clusters
            .Where(cluster => ClusterRegistry.TryGetValue(cluster.Signature, out var entry)
                && entry.Verdict == Verdict.NeedsBoundary)
            .ToList();

        // 自检：分母本身必须是真实发现出来的，不能因为发现逻辑坏掉而变成 0/0 全绿。
        // 9 个**实例** = 8 个签名，`SaveKeyText | RevokeKeyText` 那一项覆盖两处
        //（`SettingsPageView.axaml:494` 与 `:524`，两个服务商配置区各一套）。
        Assert.Equal(9, needBoundary.Count);
        Assert.Equal(
            8,
            needBoundary.Select(cluster => cluster.Signature).Distinct(StringComparer.Ordinal).Count());

        var naked = needBoundary.Where(cluster => cluster.DividerCount == 0).ToList();
        Assert.True(
            naked.Count == 0,
            $"组边界覆盖率 {needBoundary.Count - naked.Count}/{needBoundary.Count}"
            + $"（修复前 3/9，修复后应为 9/9；实例计数，SaveKey/RevokeKey 那一签名占两处）。"
            + $"下列簇被判定为多组却没有任何 Border.{DividerClass}：\n  "
            + string.Join("\n  ", naked.Select(cluster =>
                $"{cluster.File}:{cluster.Line} 目标={cluster.TargetCount} 理由={ClusterRegistry[cluster.Signature].Reason}")));

        // 最长连排：报告自己推导出的度量 —— 「挤」是「连续无分隔目标数」的函数。
        var tooLong = needBoundary
            .Where(cluster => cluster.MaxRun > 3
                && !(LongRunWaivers.TryGetValue(cluster.Signature, out var waiver) && cluster.MaxRun <= waiver.MaxRun))
            .ToList();
        Assert.True(
            tooLong.Count == 0,
            "下列簇里有一段 ≥4 个连续无分隔的可点目标（修复前有 3 处：作品页顶栏 5、"
            + "画布视图组 5、运行控制 4）。要么补一条组边界，要么把它连同理由写进 LongRunWaivers：\n  "
            + string.Join("\n  ", tooLong.Select(cluster =>
                $"{cluster.File}:{cluster.Line} 最长连排={cluster.MaxRun} 序列={cluster.Sequence}")));
    }

    /// <summary>
    /// 判为「单一语义组」的簇**不该**有组边界。
    ///
    /// 这一条是上一条的反向闸门。没有它，「让所有簇都变绿」的最省事做法就是
    /// 给每个 2 键对都插一条线 —— 那会把 tab 条、分段控件、主/次动作对全部切开，
    /// 分组线因为处处都在而不再传达任何信息（分组的信息量来自它的**稀缺**）。
    /// </summary>
    [Fact]
    public void SingleGroupClustersStayUndivided()
    {
        var offenders = DiscoverClusters()
            .Where(cluster => ClusterRegistry.TryGetValue(cluster.Signature, out var entry)
                && entry.Verdict == Verdict.SingleGroup
                && cluster.DividerCount > 0)
            .ToList();

        Assert.True(
            offenders.Count == 0,
            $"下列簇登记为单一语义组，却插了 {DividerClass}：\n  "
            + string.Join("\n  ", offenders.Select(cluster =>
                $"{cluster.File}:{cluster.Line} 线={cluster.DividerCount} 理由={ClusterRegistry[cluster.Signature].Reason}"))
            + "\n把 tab 条 / 分段控件 / 主次动作对切开，会让分组线失去信息量 ——"
            + "\n若确实认为它是多组，改登记表的判定并写清理由，而不是留着矛盾。");
    }

    // ==================== 机械发现 ====================

    /// <summary>一个横向按钮簇的实测形态。</summary>
    private sealed record Cluster(
        string File,
        int Line,
        string Tag,
        string Signature,
        int TargetCount,
        int DividerCount,
        int MaxRun,
        string Sequence);

    /// <summary>
    /// 扫全部页面 XAML，找出**最外层**横向按钮簇。
    ///
    /// # 口径（这是报告 M=36 与本文件 27 的差异来源，务必读完再改）
    ///
    /// 1. **只认横向容器**：<c>StackPanel Orientation="Horizontal"</c>（Avalonia 的
    ///    StackPanel 默认是 Vertical，所以必须显式写了 Horizontal）、
    ///    以及带 <c>ColumnDefinitions</c> 的 <c>Grid</c>、<c>UniformGrid</c>、<c>WrapPanel</c>。
    ///    报告的 36 里有 6 个是**纵向**排布（空态按钮竖排、导航导轨）——
    ///    竖排谈不上「横向堆在一起」，那是另一个问题。
    /// 2. **只认最外层**：一个簇内部再嵌的横向容器不单独计数。报告把画布工具条的
    ///    3 个内层分组各算一簇，而视觉上那是**一条**工具条 —— 按内层数会让
    ///    「分了组的」在分母里占三份，分组做得越好数字越差。
    /// 3. **排除菜单**：见 <see cref="MenuTags"/> 的注释（10 处 Separator 全在菜单里）。
    /// 4. **≥2 个可点目标**才算簇。
    ///
    /// # 签名怎么构造
    ///
    /// <c>文件 :: class:内容 | class:内容 …</c>。用 class + 绑定路径而不是行号：
    /// **行号会随上下任何一处改动漂移**，用行号做键的登记表在第二次改动后全是假失败。
    /// 取 <c>Classes</c> 是因为它同时携带了视觉层级（primary/danger/subtle），
    /// 而「不可逆动作紧邻常规动作」这个判定正是靠它读出来的。
    /// </summary>
    private static List<Cluster> DiscoverClusters()
    {
        var result = new List<Cluster>();
        foreach (var path in EnumeratePageMarkup())
        {
            var document = XDocument.Parse(
                File.ReadAllText(path), LoadOptions.SetLineInfo);
            var root = document.Root;
            if (root is null)
            {
                continue;
            }

            // 自顶向下遍历：命中一个簇之后**不再进入它的子树**，实现「最外层」。
            CollectClusters(root, Path.GetFileName(path), result, insideCluster: false);
        }

        return result
            .OrderBy(cluster => cluster.File, StringComparer.Ordinal)
            .ThenBy(cluster => cluster.Line)
            .ToList();
    }

    private static void CollectClusters(
        XElement element, string file, List<Cluster> sink, bool insideCluster)
    {
        var name = element.Name.LocalName;
        if (MenuTags.Contains(name))
        {
            return;
        }

        var claimed = false;
        if (!insideCluster && IsHorizontalContainer(element) && !IsLayoutScaffold(element))
        {
            var cluster = Describe(element, file);
            if (cluster is not null)
            {
                sink.Add(cluster);
                claimed = true;
            }
        }
        else if (!insideCluster && IsMultiRowGrid(element))
        {
            // 多行 Grid：按 Grid.Row 分组，**每一行**各自作为候选簇量一次。
            // 见 IsSingleRowGrid 的注释：「多行」不等于「按钮跨行分散」，
            // 页头那种「三行布局、两颗键都在第 0 行紧邻」的形态必须找得到。
            foreach (var row in element.Elements()
                         .Where(child => !child.Name.LocalName.Contains('.', StringComparison.Ordinal))
                         .GroupBy(ReadRow)
                         .OrderBy(group => group.Key))
            {
                var cluster = DescribeRow(element, row.ToList(), file);
                if (cluster is not null)
                {
                    sink.Add(cluster);
                }
            }

            // 🔴 **刻意不设 claimed**。这里踩过一次：原先认领任意一行就封锁整棵子树，
            // 结果 `WorksPageView.axaml:119` 那个多行 Grid 的某一行成簇之后，
            // 深埋在同一个 Grid 别处的 QuickEdit 决议键（`:486`）整簇消失，
            // 表现为「登记表里有 1 项在源码里已不存在」——而它明明就在源码里。
            // 多行 Grid 是**版式容器**：它某一行有一排键，不代表它别的行、
            // 别的子树里就没有独立的按钮簇。封锁子树只对「这一排就是全部」的
            // 单行容器成立。
            // ⇒ 我为找这一个假 stale 花了四轮（先怀疑签名有不可见字符、
            // 再怀疑祖先封锁、再去追共同祖先），根因却在自己上一步刚写的那行赋值上。
            // **改动引入的缺陷，第一嫌疑人是改动本身，不是被改的对象。**
        }

        foreach (var child in element.Elements())
        {
            CollectClusters(child, file, sink, insideCluster || claimed);
        }
    }

    /// <summary>
    /// 横向容器判定。
    ///
    /// ⚠️ <c>StackPanel</c> 必须显式 <c>Orientation="Horizontal"</c> ——
    /// Avalonia 的默认值是 Vertical，把无 Orientation 的 StackPanel 当横向
    /// 会把所有竖排都卷进来（那正是报告 36 这个数字偏大的一半原因）。
    ///
    /// ⚠️ <c>Grid</c> 只排除**多行表单**这一类，不按弹性列排除。
    /// `SettingsPageView.axaml:395` 是 `RowDefinitions="Auto,Auto,Auto,Auto"`
    /// 的四行表单，第 3 列各有一个「浏览」键 —— 4 个键**纵向**排在一列里，
    /// 签名却长得像「4 个同款键并排」。这类误判最危险：它会让人去给一个
    /// 根本不存在的横排加分组线。
    ///
    /// 🔴 **我一度按「含弹性列 `*` 就不算相邻」排除，那是错的**，
    /// 并且错得很有代表性：它一次杀掉 12 个真簇，其中包括
    /// `SaveKeyText | RevokeKeyText`（保存紧邻不可逆吊销）——**U205 的核心缺陷之一**。
    /// 原因是那个 Grid 第 0 列是输入框占 `*`，两颗键在**右侧紧邻**：
    /// 弹性列的存在只说明「这一行里有个会伸缩的东西」，
    /// 完全不能推出「这一行的按钮彼此被推开」。
    /// ⇒ 相邻性由 <see cref="Flatten"/> 按**列索引连续性**判定，不由列定义猜。
    /// </summary>
    private static bool IsHorizontalContainer(XElement element)
    {
        var name = element.Name.LocalName;
        var orientation = (string?)element.Attribute("Orientation");

        return name switch
        {
            "StackPanel" or "VirtualizingStackPanel" =>
                string.Equals(orientation, "Horizontal", StringComparison.Ordinal),
            "DockPanel" or "WrapPanel" => true,
            "UniformGrid" => true,
            "Grid" => IsSingleRowGrid(element),
            _ => false,
        };
    }

    /// <summary>
    /// **版式骨架**：能横向排布，但它的直接子元素是「区域」而不是「键」
    /// ⇒ 不认领簇，只允许继续下探，让真正的一排键在子容器里被找到。
    ///
    /// 两类：
    /// 1. <c>DockPanel</c> —— 按边停靠，`WorkspacePageView.axaml:1261` 用它把
    ///    「下栏标题栏」与「内容区」上下分开，直接子元素根本不是并排关系。
    /// 2. **含弹性列 `*` 的 Grid** —— `MainWindow.axaml:45`（`Auto,*,Auto,Auto`）
    ///    把项目菜单顶到最左、窗口控制键推到最右；`RunLogPageView.axaml:106` 同型。
    ///
    /// ⚠️ 与「整个排除」的区别很关键：骨架仍然**下探**。
    /// 我第一版把含 `*` 的 Grid 直接判成非容器（既不认领也不下探），
    /// 结果 12 个真簇消失，含 `SaveKeyText | RevokeKeyText` 这个 U205 核心缺陷 ——
    /// 因为那个 Grid 自己就是真簇的宿主（第 0 列输入框占 `*`，两颗键在右侧紧邻）。
    /// 所以「有 `*`」只在**它的按钮分散在被空列隔开的多个区域**时才说明不相邻，
    /// 那种情况由 <see cref="Flatten"/> 的列索引跳空处理；
    /// 而这里判骨架的依据是**直接子元素里没有可点目标**（全是容器）。
    /// </summary>
    private static bool IsLayoutScaffold(XElement element)
    {
        var name = element.Name.LocalName;
        if (name is not ("DockPanel" or "Grid"))
        {
            return false;
        }

        // 直接子元素里有可点目标 ⇒ 它自己就是一排键的宿主，不是骨架。
        //
        // 🔴 这里**刻意不穿透** `UnwrapSingleChild`。我原先穿透了，结果
        // `WorksPageView.axaml:36` 那个整页 DockPanel 被判成「有直接目标」：
        // 它的直接子元素是 `Border`（页头容器），穿透后拿到里面的 `Grid`，
        // 于是骨架判定失效 ⇒ 它认领了整页、把 7 个跨区域的目标展平成一个签名
        //（含 `icon-btn:QuickEditTitle` 这种根本不是按钮的东西），
        // 还顺手封锁了深处的 QuickEdit 决议键那一簇。
        // ⇒ 穿透的用途是「一颗被 Border 包起来的按钮仍算一个目标」，
        // 用在「这一层有没有按钮」的判定上就变成了「往下挖到有按钮为止」，
        // 那是两件不同的事。
        var hasDirectTarget = element.Elements().Any(child =>
            ClickableTags.Contains(child.Name.LocalName));
        if (hasDirectTarget)
        {
            return false;
        }

        // 没有直接目标，但**子孙**里有横向簇 ⇒ 骨架（真簇在下面）。
        //
        // ⚠️ 这里要看子孙而不是直接子元素：`WorksPageView.axaml:36` 的 DockPanel
        // 直接子元素只有一个 `Border`（页头容器），横向的 Grid 在 Border 里面。
        // 只看直接子元素的话它两个条件都不满足 ⇒ 判成「自己就是一排键」，
        // 于是把整页 7 个跨区域目标展平成一个签名，还封锁了深处的真簇。
        // 与上面 `hasDirectTarget` 的「刻意不穿透」不矛盾：
        // 「这一层有没有按钮」要精确到本层，「下面还有没有真簇」则要看整棵子树。
        return element.Descendants().Any(descendant =>
            descendant != element && IsHorizontalContainer(descendant));
    }

    /// <summary>
    /// 单行 Grid 才当成「一排键」直接量；**多行 Grid 按行拆开**再量。
    ///
    /// 🔴 我一度写成「多行 Grid 整个排除」，那杀错了两个真簇。典型是
    /// `SettingsPageView.axaml:276`：`RowDefinitions="Auto,Auto,Auto"` 的三行页头，
    /// 但「恢复本页」「恢复推荐默认」两颗键都在**第 0 行**的第 2/3 列紧邻。
    /// ⇒ 「多行」说明这个 Grid 承载了多行内容，**不说明**它的按钮跨行分散。
    /// 真正该排除的是 `SettingsPageView.axaml:395` 那种「同一列上每行一颗键」
    /// 的表单 —— 那些键纵向排列，按行拆开后每行只剩 1 颗，自然不足以成簇。
    ///
    /// ⇒ 判定改成：多行 Grid 不自己成簇，而是<b>每一行各自</b>作为候选簇量一次
    /// （见 <see cref="CollectClusters"/> 里的 <c>ExpandGridRows</c> 分支）。
    /// 这样两种情形都对：同行紧邻的被找到，跨行分散的自动落空。
    /// </summary>
    private static bool IsSingleRowGrid(XElement element)
    {
        if (string.IsNullOrWhiteSpace((string?)element.Attribute("ColumnDefinitions")))
        {
            return false;
        }

        var rows = (string?)element.Attribute("RowDefinitions");
        return string.IsNullOrWhiteSpace(rows) || rows!.Split(',').Length <= 1;
    }

    /// <summary>多行 Grid ⇒ 可按行拆开找簇。</summary>
    private static bool IsMultiRowGrid(XElement element)
        => element.Name.LocalName == "Grid"
           && !string.IsNullOrWhiteSpace((string?)element.Attribute("ColumnDefinitions"))
           && ((string?)element.Attribute("RowDefinitions"))?.Split(',').Length > 1;

    /// <summary>
    /// 量一个候选容器：可点目标、组边界、最长连排。
    ///
    /// ⚠️ **必须穿透纯排布用的嵌套横向容器**，这是本方法最容易写错的地方。
    /// 画布工具条的真实结构是「外层 StackPanel 包 3 个内层 StackPanel，
    /// 每个内层装 3~5 颗键」——外层的**直接子元素里一颗按钮都没有**。
    /// 只看直接子元素的话：外层因 targets&lt;2 被跳过，然后递归把 3 个内层
    /// 各算一簇 ⇒ 分组做得越好、簇数越多，而「一条工具条」被拆成三份，
    /// **这正是登记表注释里说的那个让分子分母都失真的口径**（我第一版就是这么错的）。
    ///
    /// 所以这里按「视觉上的一行」展平：遇到子容器仍是横向排布容器时，
    /// 递进去继续收它的目标与分隔线，保持左→右的顺序（最长连排靠这个顺序算）。
    /// 展平**不跨越**非排布控件（Button 的 Content 里的图标不会被算成目标），
    /// 因为 <see cref="ClickableTags"/> 命中后就不再往里走。
    /// </summary>
    private static Cluster? Describe(XElement container, string file)
    {
        var parts = new List<string>();
        var sequence = new List<char>();
        var counters = new Counters();

        Flatten(container, parts, sequence, counters);

        if (counters.Targets < 2)
        {
            return null;
        }

        return new Cluster(
            File: file,
            Line: ((IXmlLineInfo)container).LineNumber,
            Tag: container.Name.LocalName,
            Signature: $"{file} :: {string.Join(" | ", parts)}",
            TargetCount: counters.Targets,
            DividerCount: counters.Dividers,
            MaxRun: LongestRun(sequence),
            Sequence: new string(sequence.ToArray()));
    }

    private sealed class Counters
    {
        public int Targets;
        public int Dividers;
    }

    /// <summary>
    /// 按「视觉上的一行」把一个横向容器展平成 目标/分隔线 序列。见
    /// <see cref="Describe"/> 的注释说明为什么必须展平而不能只看直接子元素。
    ///
    /// ⚠️ 在 Grid 里还要处理**列索引跳空**：`MainWindow.axaml:45` 的
    /// `Auto,*,Auto,Auto` 把项目菜单放在第 0 列、窗口控制键放在第 2/3 列，
    /// 中间第 1 列是整条标题栏宽的弹性空白 ⇒ 它们在屏幕上根本不相邻，
    /// 而「堆在一起」这个缺陷的前提就是相邻。跳空处插一个 `|` 断开连排，
    /// 使最长连排反映真实的视觉紧邻，同时**不**把它们排除出簇
    ///（那样会连带杀掉右端真正紧邻的那几颗键）。
    ///
    /// 🔴 这一段是我第二次尝试才对的。第一次写成「含 `*` 的 Grid 整个不算簇」，
    /// 一次杀掉 12 个真簇（含 `SaveKeyText | RevokeKeyText` 这个 U205 核心缺陷）。
    /// **教训**：容器的列定义描述的是布局能力，不是元素间的实际距离；
    /// 要判「相邻」就得看元素**各自落在哪一列**，不能看容器声明了什么列。
    /// </summary>
    private static void Flatten(
        XElement container, List<string> parts, List<char> sequence, Counters counters)
    {
        var isGrid = container.Name.LocalName == "Grid";
        var previousColumn = int.MinValue;

        foreach (var child in container.Elements())
        {
            var effective = UnwrapSingleChild(child);
            var name = effective.Name.LocalName;

            // Grid.ColumnDefinitions 之类的属性元素不是内容。
            if (name.Contains('.', StringComparison.Ordinal) || MenuTags.Contains(name))
            {
                continue;
            }

            var classes = (string?)effective.Attribute("Classes") ?? string.Empty;
            var isDivider = name == "Border" && classes.Split(' ').Contains(DividerClass);
            var isTarget = ClickableTags.Contains(name);
            var isNested = !isDivider && !isTarget && IsHorizontalContainer(effective);
            if (!isDivider && !isTarget && !isNested)
            {
                continue;
            }

            // Grid 里：与上一个内容元素之间隔了 ≥1 个空列 ⇒ 视觉上不相邻，断开连排。
            if (isGrid)
            {
                var column = ReadColumn(child);
                if (previousColumn != int.MinValue && column - previousColumn > 1)
                {
                    sequence.Add('|');
                }

                previousColumn = column;
            }

            if (isDivider)
            {
                counters.Dividers++;
                sequence.Add('|');
                continue;
            }

            if (isTarget)
            {
                counters.Targets++;
                sequence.Add('*');
                parts.Add($"{classes}:{DescribeTarget(effective)}");
                continue;
            }

            // 纯排布用的嵌套横向容器 ⇒ 递进去，序列保持左→右。
            Flatten(effective, parts, sequence, counters);
        }
    }

    /// <summary>读 <c>Grid.Column</c>（缺省为 0）。</summary>
    private static int ReadColumn(XElement element)
        => int.TryParse((string?)element.Attribute("Grid.Column"), out var column) ? column : 0;

    /// <summary>读 <c>Grid.Row</c>（缺省为 0）。</summary>
    private static int ReadRow(XElement element)
        => int.TryParse((string?)element.Attribute("Grid.Row"), out var row) ? row : 0;

    /// <summary>
    /// 量多行 Grid 的**某一行**。除了只看这一行的元素，其余口径与
    /// <see cref="Describe"/> 完全一致（含列索引跳空断开连排）。
    /// </summary>
    private static Cluster? DescribeRow(XElement grid, List<XElement> rowChildren, string file)
    {
        var parts = new List<string>();
        var sequence = new List<char>();
        var counters = new Counters();
        var previousColumn = int.MinValue;

        foreach (var child in rowChildren)
        {
            var effective = UnwrapSingleChild(child);
            var name = effective.Name.LocalName;
            if (MenuTags.Contains(name))
            {
                continue;
            }

            var classes = (string?)effective.Attribute("Classes") ?? string.Empty;
            var isDivider = name == "Border" && classes.Split(' ').Contains(DividerClass);
            var isTarget = ClickableTags.Contains(name);
            var isNested = !isDivider && !isTarget && IsHorizontalContainer(effective);
            if (!isDivider && !isTarget && !isNested)
            {
                continue;
            }

            var column = ReadColumn(child);
            if (previousColumn != int.MinValue && column - previousColumn > 1)
            {
                sequence.Add('|');
            }

            previousColumn = column;

            if (isDivider)
            {
                counters.Dividers++;
                sequence.Add('|');
                continue;
            }

            if (isTarget)
            {
                counters.Targets++;
                sequence.Add('*');
                parts.Add($"{classes}:{DescribeTarget(effective)}");
                continue;
            }

            Flatten(effective, parts, sequence, counters);
        }

        if (counters.Targets < 2)
        {
            return null;
        }

        return new Cluster(
            File: file,
            Line: ((IXmlLineInfo)grid).LineNumber,
            Tag: grid.Name.LocalName,
            Signature: $"{file} :: {string.Join(" | ", parts)}",
            TargetCount: counters.Targets,
            DividerCount: counters.Dividers,
            MaxRun: LongestRun(sequence),
            Sequence: new string(sequence.ToArray()));
    }

    /// <summary>
    /// 单子元素的纯包装容器（Border/ContentControl 包一个按钮）穿透过去。
    /// 见 <see cref="Describe"/> 里的说明：不穿透会低估最长连排。
    /// </summary>
    private static XElement UnwrapSingleChild(XElement element)
    {
        var current = element;
        for (var depth = 0; depth < 3; depth++)
        {
            if (current.Name.LocalName is not ("Border" or "ContentControl" or "Panel"))
            {
                return current;
            }

            var children = current.Elements()
                .Where(child => !child.Name.LocalName.Contains('.', StringComparison.Ordinal))
                .ToList();
            if (children.Count != 1)
            {
                return current;
            }

            current = children[0];
        }

        return current;
    }

    /// <summary>
    /// 目标的身份串：优先取绑定的文案属性，其次 Command，最后字面 Content。
    ///
    /// 取「绑的是什么」而不是「显示什么字」：所有可见文案都走
    /// <c>display_name.json</c>，翻译一改字面量就变，而绑定路径稳定。
    /// </summary>
    private static string DescribeTarget(XElement target)
    {
        foreach (var attribute in new[] { "Content", "ToolTip.Tip", "Command", "AutomationProperties.Name" })
        {
            var raw = (string?)target.Attribute(attribute);
            if (string.IsNullOrWhiteSpace(raw))
            {
                continue;
            }

            var match = Regex.Match(raw, @"\{\s*Binding\s+([^,}]+)");
            if (match.Success)
            {
                return match.Groups[1].Value.Trim();
            }

            if (!raw.StartsWith("{", StringComparison.Ordinal))
            {
                return raw.Trim();
            }
        }

        // 走到这里说明它既没绑文案也没字面内容（例如只有一个图标 Path 子元素）。
        var child = target.Elements()
            .Select(node => (string?)node.Attribute("Command"))
            .FirstOrDefault(command => !string.IsNullOrWhiteSpace(command));
        return child is null ? target.Name.LocalName : child.Trim();
    }

    /// <summary>最长的连续可点目标段（`*` 连排，被 `|` 打断）。</summary>
    private static int LongestRun(IReadOnlyList<char> sequence)
    {
        var best = 0;
        var run = 0;
        foreach (var slot in sequence)
        {
            if (slot == '*')
            {
                run++;
                best = Math.Max(best, run);
            }
            else
            {
                run = 0;
            }
        }

        return best;
    }

    private static IEnumerable<string> EnumeratePageMarkup()
    {
        var viewsRoot = Path.Combine(ResolveRepoRoot(), "desktop", "Ariadne.Desktop", "Views");
        Assert.True(Directory.Exists(viewsRoot), $"找不到 Views 目录：{viewsRoot}");
        return Directory
            .EnumerateFiles(viewsRoot, "*.axaml", SearchOption.AllDirectories)
            .OrderBy(path => path, StringComparer.Ordinal);
    }

    private static string ResolveRepoRoot()
    {
        var dir = AppContext.BaseDirectory;
        for (var i = 0; i < 8 && dir is not null; i++)
        {
            if (Directory.Exists(Path.Combine(dir, "desktop", "Ariadne.Desktop", "Views")))
            {
                return dir;
            }

            dir = Path.GetDirectoryName(dir);
        }

        throw new DirectoryNotFoundException($"从 {AppContext.BaseDirectory} 向上找不到仓库根");
    }
}
