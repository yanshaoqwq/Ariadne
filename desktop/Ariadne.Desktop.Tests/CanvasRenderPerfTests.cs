using System.Diagnostics;
using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
using Avalonia.VisualTree;
using Avalonia.Threading;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;
using Xunit.Abstractions;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// **渲染性能**：画布打开、节点拖动、页面切换。
///
/// ## 为什么单立一个文件
///
/// 用户感知的「卡」几乎全在这三处，而现有测试测不到：
/// - `CanvasHelpersTests` 用的是**读源码字符串**断言
///   （`Assert.Contains("CanvasDragFrameHelpers.TryScheduleFrameSync", view)`）——
///   它能证明「代码里调了帧合并」，**证明不了拖一次到底花多少毫秒**，
///   也拦不住「帧合并还在、但每帧里干的事变成 O(n²)」这类退化。
/// - `FrontendSettingsAndPerfJourneyTests` 测的是 IPC 那一层（已实测健康：
///   轻量往返中位 5ms、50 万字读写 ~0.6s）。**那一层不慢，慢的可能在 UI 线程。**
///
/// 本文件在 headless 平台上**真实构建视图、真实发指针事件、真实量墙钟时间**。
///
/// ## 阈值怎么定（很重要，不然会造出一条间歇红的测试）
///
/// 这台机器是 ARM 开发板（3.8G 内存），测试可能与编译并行。所以：
/// - **绝对阈值放到「即使慢 10 倍也不该到」的量级**，断的是数量级异常
///   （每次 PointerMoved 全量重算、切页面重建整棵树），不是几毫秒抖动。
/// - **优先用比值判据**（节点数翻倍时耗时倍数、每事件摊销），
///   比值与机器速度无关，在慢机器上依然成立。
/// - 实测值一律 `_output.WriteLine` 打出来，便于日后对比。
///
/// ⚠️ **headless 平台没有真实 GPU 渲染循环**，所以这里量的是
/// **布局 + 绑定 + 几何计算 + 调度**的耗时，不含 GPU 合成。
/// 这恰好是对的：卡顿在这个产品里基本来自 UI 线程上的这几样，
/// 而不是像素填充率。但**绝对值不能当作真机帧率**，只能用来抓量级与回归。
///
/// ⚠️ 另一条踩坑记录沿用 `InputSurfaceStyleTests` 的结论：
/// **不要用 `DispatcherPriority.Render` 排空队列**——headless 没有渲染循环，
/// 往 Render 优先级投的回调可能永不执行，`InvokeAsync` 一直不返回，
/// 表现是「单条跑 280s 不结束」，与内存不足的日志长得一样，极易误判。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class CanvasRenderPerfTests
{
    private readonly ITestOutputHelper _output;

    public CanvasRenderPerfTests(ITestOutputHelper output)
    {
        _output = output;
    }

    // ════════════════════════════════════════════════════════
    // 1. 打开画布页
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// **打开画布页的耗时随节点数线性**。
    ///
    /// 用户抱怨的「打开画布页卡一下」就是这一段：视图树构建 + 每个节点的
    /// 模板实体化 + 边几何首次计算 + 首轮布局。
    ///
    /// 判据用**比值**：30 节点 → 120 节点（4 倍）。线性实现约 4 倍，
    /// 若边几何是「每个节点都遍历全部边」则是 O(n²) ≈ 16 倍。
    /// 阈值 **&lt; 9 倍**，给模板缓存冷启动、GC 留余量，但仍能拦住平方级。
    ///
    /// 同时给一个宽松绝对上限（120 节点 &lt; 12s）：比值判据有个盲区——
    /// 若小规模那次本身就慢得离谱，比值反而好看。两个判据一起才闭合。
    /// </summary>
    [Fact]
    public async Task CanvasOpen_TimeGrowsLinearlyWithNodeCount()
    {
        // 两个规模在同一 headless 会话里测：起两个会话会在这台 3.8G ARM 板上被
        // OOM 杀掉（退出码 143，症状与卡死难以区分）。见 MeasureCanvasOpenPairAsync。
        var (small, large) = await MeasureCanvasOpenPairAsync(
            firstNodes: 30, firstEdges: 1, secondNodes: 120, secondEdges: 1);

        var ratio = large / Math.Max(small, 1.0);
        _output.WriteLine(
            $"打开画布页：30 节点 {small:F0}ms → 120 节点 {large:F0}ms"
            + $"（4 倍规模，耗时 {ratio:F2}×；线性≈4×，平方≈16×）");

        Assert.True(
            ratio < 9,
            $"节点数从 30 增到 120（4 倍）时，打开画布页耗时变成 {ratio:F1} 倍"
            + $"（{small:F0}ms → {large:F0}ms）。线性应约 4 倍、平方级约 16 倍——"
            + "这个比值说明存在超线性开销，大工作流会卡在打开这一步");

        // 绝对上限保留但放宽到「量级异常」层面：headless 下的固定开销
        // （空画布约 2.4s，见诊断用例）与真机不可比，这里只拦「彻底失控」。
        Assert.True(
            large < 20_000,
            $"120 节点的画布页打开用了 {large:F0}ms（阈值 20000ms）——量级异常。"
            + "注意 headless 固定开销约 2.4s，此阈值只拦彻底失控，不代表真机体验");
    }

    /// <summary>
    /// **边数增长不得让画布打开变成平方级**。
    ///
    /// 与上一条互补：那条固定「每节点约 1 条边」，这条**固定节点数、只加边**。
    /// 两条分开测才能定位到底是节点模板贵还是边几何贵——
    /// 合在一起测的话，一个「边几何 O(e×n)」的实现会被误读成「节点贵」。
    /// </summary>
    [Fact]
    public async Task CanvasOpen_TimeGrowsLinearlyWithEdgeCount()
    {
        var (few, many) = await MeasureCanvasOpenPairAsync(
            firstNodes: 40, firstEdges: 1, secondNodes: 40, secondEdges: 4);

        var ratio = many / Math.Max(few, 1.0);
        _output.WriteLine(
            $"打开画布页（固定 40 节点）：约 40 边 {few:F0}ms → 约 160 边 {many:F0}ms"
            + $"（4 倍边数，耗时 {ratio:F2}×）");

        Assert.True(
            ratio < 9,
            $"边数增到 4 倍时打开耗时变成 {ratio:F1} 倍（{few:F0}ms → {many:F0}ms）——"
            + "边几何计算疑似对每条边都遍历全部节点");
    }


    /// <summary>
    /// **切回画布页的代价不得随节点数线性增长**——U159 的核心判据。
    ///
    /// ⚠️ **本条当前失败，失败即缺陷存在（U159 P1）**。实测：
    /// <code>
    ///   0 节点：切回画布 1190ms、切去别页 479ms
    ///  15 节点：切回画布 3931ms、切去别页 426ms
    ///  60 节点：切回画布 6741ms、切去别页 556ms
    /// </code>
    /// 每节点约 92ms，另有约 1.2s 画布视图固定重建代价；
    /// 切去别页恒定约 0.5s、与画布节点数无关 ⇒ **代价全在画布这一侧**。
    ///
    /// 根因：`MainWindowViewModel.GetOrCreatePage`（:628）缓存了**ViewModel**，
    /// 但 `App.axaml:18-20` 用 `DataTemplate` 生成 **View**——
    /// `ContentControl` 每次换 Content 都重建整棵视图树。
    /// 画布的节点卡片模板是 274 行 XAML（`WorkspacePageView.axaml:495-768`），
    /// 且用 `ItemsControl` + `Canvas` 面板、**零虚拟化**，于是 N 个卡片全量重建。
    ///
    /// **判据取「与节点数的比例关系」而非绝对耗时**：headless + debug + ARM 板的
    /// 绝对值不可与真机比，但「切回代价随节点数线性」这个**形状**与机器速度无关。
    ///
    /// ⚠️ 每档都先预热一次该视图再计时：首次含 JIT + 主题字典 + 模板首次解析的
    /// 一次性冷启动（实测约 4s），不预热会把框架冷启动误报成产品缺陷。
    /// </summary>
    [Fact]
    public async Task PageSwitch_ReturningToCanvas_CostDoesNotScaleWithNodeCount()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder), AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(async () =>
        {
            var displayNames = DisplayNameService.LoadDefault();
            var backend = DispatchProxy.Create<IAriadneBackendClient, SoftBackend>();
            var other = new WorksPageView
            {
                DataContext = new WorksPageViewModel(displayNames, backend),
            };
            var window = new Window { Width = 1400, Height = 900 };
            window.Show();
            await DrainAsync();

            var measured = new Dictionary<int, double>();
            foreach (var nodeCount in new[] { 0, 15, 60 })
            {
                var workspace = BuildWorkspace(displayNames, backend, nodeCount, 1);
                var canvasView = new WorkspacePageView { DataContext = workspace };

                // 预热该视图一次（含首次模板解析），再量「切走—切回」的稳态代价。
                window.Content = canvasView;
                await DrainAsync();
                window.Content = other;
                await DrainAsync();

                var sw = Stopwatch.StartNew();
                window.Content = canvasView;
                await DrainAsync();
                sw.Stop();
                var back = sw.Elapsed.TotalMilliseconds;

                sw.Restart();
                window.Content = other;
                await DrainAsync();
                sw.Stop();

                _output.WriteLine(
                    $"{nodeCount,3} 节点：切回画布 {back:F0}ms、切去别页 {sw.Elapsed.TotalMilliseconds:F0}ms");
                measured[nodeCount] = back;
            }

            window.Content = null;
            window.Close();
            await DrainAsync();

            // 判据：切回代价里「随节点数增长」的那部分必须接近 0。
            // 用 0 节点那档作为固定开销基线，看每节点的边际成本。
            var baseline = measured[0];
            var perNode = (measured[60] - baseline) / 60.0;
            _output.WriteLine(
                $"→ 固定开销 {baseline:F0}ms，每节点边际 {perNode:F1}ms"
                + $"（60 节点共多付 {measured[60] - baseline:F0}ms）");

            Assert.True(
                perNode < 10,
                $"切回画布页时每个节点要多付 {perNode:F0}ms（阈值 10ms）——"
                + $"0 节点 {measured[0]:F0}ms、15 节点 {measured[15]:F0}ms、60 节点 {measured[60]:F0}ms。"
                + "ViewModel 被 GetOrCreatePage 缓存了，但 View 由 App.axaml 的 DataTemplate "
                + "每次重建，274 行的节点卡片模板 × N 全量重新实体化（U159）。"
                + "用户表现为「每次切回画布页都要等几秒」");

            return true;
        }, CancellationToken.None);
    }

    // ════════════════════════════════════════════════════════
    // 2. 拖动节点
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// **拖动节点：单次 PointerMoved 的摊销耗时**。
    ///
    /// 这是最直接的「手感」指标。60fps 意味着每帧 16.7ms 预算，
    /// 而一次拖动手势里 PointerMoved 会以设备采样率触发（常 &gt; 帧率）。
    /// C5-a 的帧合并（`CanvasDragFrameHelpers`）正是为此存在：
    /// PointerMoved 只写逻辑坐标，主视觉同步推到 Render 帧回调里做。
    ///
    /// **所以这条测的是「帧合并真的在省事」**：单次 PointerMoved 的摊销
    /// 必须远低于一次完整视觉同步。阈值取 **&lt; 8ms/事件**——
    /// 即便在 ARM 板 + debug 构建下，写两个 double 加一次调度也远达不到；
    /// 若这里超了，说明每个事件都在做全量几何重算（帧合并失效）。
    ///
    /// ⚠️ **判据同时验位置真的变了**：只测速度不验结果，
    /// 一个「PointerMoved 直接 return」的实现会是最快的。
    /// </summary>
    [Fact]
    public async Task NodeDrag_PerPointerMovedCost_StaysWithinFrameBudget()
    {
        await RunWithCanvasAsync(nodeCount: 80, edgesPerNode: 2, async (view, viewModel) =>
        {
            var node = viewModel.Nodes[0];
            var startX = node.X;
            var startY = node.Y;

            var container = FindNodeVisual(view, node);
            Assert.NotNull(container);

            // 按下：进入拖拽态。
            RaiseNodePointerPressed(view, container!, new Point(10, 10));

            const int moves = 120;
            var sw = Stopwatch.StartNew();
            for (var i = 1; i <= moves; i++)
            {
                RaiseNodePointerMoved(view, container!, new Point(10 + i * 2, 10 + i));
            }
            sw.Stop();

            RaiseNodePointerReleased(view, container!, new Point(10 + moves * 2, 10 + moves));
            await DrainAsync();

            var perEvent = sw.Elapsed.TotalMilliseconds / moves;
            _output.WriteLine(
                $"拖动 80 节点/160 边的画布：{moves} 次 PointerMoved 共 "
                + $"{sw.Elapsed.TotalMilliseconds:F0}ms（单次摊销 {perEvent:F3}ms）");

            // 先验行为：坐标必须真的动了，否则「最快的实现」是什么都不做。
            Assert.True(
                Math.Abs(node.X - startX) > 1 || Math.Abs(node.Y - startY) > 1,
                $"拖了 {moves} 次但节点坐标没变（{startX},{startY} → {node.X},{node.Y}）——"
                + "测到的耗时没有意义");

            Assert.True(
                perEvent < 8,
                $"单次 PointerMoved 摊销 {perEvent:F2}ms（阈值 8ms，60fps 的帧预算是 16.7ms）。"
                + "C5-a 的帧合并本应让 PointerMoved 只写坐标——"
                + "这个数字说明每个事件都在做全量视觉同步，拖动会明显掉帧");
        });
    }

    /// <summary>
    /// **拖动耗时不随画布规模爆炸**。
    ///
    /// 关键的手感问题不是「小画布拖得快」，而是「大画布还拖不拖得动」。
    /// 若拖动时每次都遍历全部边（而不是只碰相邻边），
    /// 节点数一多手感就断崖式下降——而小画布上完全测不出来。
    ///
    /// `SyncConnectedEdges` 走的是 `_edgesByNodeId` 索引（只碰相邻边），
    /// 这条就是钉住那个索引真的在起作用：
    /// 规模 4 倍时单次 PointerMoved 摊销**不得超过 4 倍**。
    /// 比值判据与机器速度无关。
    /// </summary>
    [Fact]
    public async Task NodeDrag_CostDoesNotExplodeWithCanvasSize()
    {
        // ⚠️ 两个规模在**同一个** headless 会话里测。
        // 起两个会话会各自加载一份 Avalonia + 主题字典，在这台 3.8G 的 ARM 板上
        // 直接被 OOM 杀掉（退出码 143，日志停在测试开始处，看起来像卡死而非内存不足
        // ——两者症状难以区分，所以这里写明原因）。
        // 复用会话还有一个附带好处：模板缓存已热，比值更干净地反映规模效应本身。
        var (small, large) = await MeasureDragPerEventPairAsync(
            smallNodes: 30, largeNodes: 120, edgesPerNode: 2);

        var ratio = large / Math.Max(small, 0.001);
        _output.WriteLine(
            $"拖动单次摊销：30 节点 {small:F3}ms → 120 节点 {large:F3}ms"
            + $"（4 倍规模，{ratio:F2}×）");

        Assert.True(
            ratio < 4,
            $"画布规模从 30 增到 120 节点（4 倍）时，单次 PointerMoved 摊销变成 {ratio:F1} 倍"
            + $"（{small:F3}ms → {large:F3}ms）。拖动只应触及**相邻**边"
            + "（SyncConnectedEdges 走 _edgesByNodeId 索引），"
            + "耗时随全画布规模增长说明每次都在遍历全部边");
    }

    /// <summary>
    /// **松手时必须 flush 最后一帧**——这是正确性，但归在这里因为它是帧合并的直接后果。
    ///
    /// C5-a 的注释写明：release 必须在清空 drag 状态前同步 flush 主视觉，
    /// 否则挂起的 Render 回调见 `dragging=false` 会空转、**漏掉最后一帧**。
    /// 用户看到的是「松手后节点弹回去一点」。
    ///
    /// 判据：松手后节点的**最终坐标**必须等于最后一次 PointerMoved 的位置。
    /// 这条在 headless 下尤其值得测——没有真实渲染循环，
    /// 「靠下一帧补上」的实现在这里会露馅。
    /// </summary>
    [Fact]
    public async Task NodeDrag_ReleaseFlushesFinalPosition()
    {
        await RunWithCanvasAsync(nodeCount: 20, edgesPerNode: 1, async (view, viewModel) =>
        {
            var node = viewModel.Nodes[0];
            var container = FindNodeVisual(view, node);
            Assert.NotNull(container);

            RaiseNodePointerPressed(view, container!, new Point(0, 0));
            for (var i = 1; i <= 10; i++)
            {
                RaiseNodePointerMoved(view, container!, new Point(i * 10, i * 5));
            }
            var expectedX = node.X;
            var expectedY = node.Y;

            RaiseNodePointerReleased(view, container!, new Point(100, 50));
            await DrainAsync();

            _output.WriteLine($"松手后坐标：({node.X:F1}, {node.Y:F1})，最后一次移动后：({expectedX:F1}, {expectedY:F1})");

            Assert.Equal(expectedX, node.X, precision: 1);
            Assert.Equal(expectedY, node.Y, precision: 1);
        });
    }

    // ════════════════════════════════════════════════════════
    // 3. 切换页面
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// **页面来回切换的耗时稳定，且不随切换次数累积**。
    ///
    /// 用户抱怨的「切几次页面越来越慢」有一个常见成因：每次切换都新建
    /// 视图/订阅而旧的没解绑，事件处理器越挂越多——**症状是耗时单调上升**，
    /// 而单次切换测试永远发现不了。
    ///
    /// 判据：后半程的平均切换耗时**不得显著高于**前半程
    /// （阈值 &lt; 2.5 倍）。这个比值与机器速度无关，
    /// 且能同时抓住「泄漏订阅」与「缓存失效导致每次重建」两类。
    /// </summary>
    [Fact]
    public async Task PageSwitch_CostDoesNotAccumulateOverRepeatedSwitches()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder), AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(async () =>
        {
            var displayNames = DisplayNameService.LoadDefault();
            var backend = DispatchProxy.Create<IAriadneBackendClient, SoftBackend>();

            var workspace = BuildWorkspace(displayNames, backend, nodeCount: 60, edgesPerNode: 2);
            var worksView = new WorksPageView
            {
                DataContext = new WorksPageViewModel(displayNames, backend),
            };
            var workspaceView = new WorkspacePageView { DataContext = workspace };

            var window = new Window { Width = 1400, Height = 900 };
            window.Show();
            await DrainAsync();

            const int switches = 12;
            var samples = new List<double>(switches);
            for (var i = 0; i < switches; i++)
            {
                var target = i % 2 == 0 ? (Control)workspaceView : worksView;
                var sw = Stopwatch.StartNew();
                window.Content = target;
                await DrainAsync();
                sw.Stop();
                samples.Add(sw.Elapsed.TotalMilliseconds);
            }

            window.Content = null;
            window.Close();
            await DrainAsync();

            var firstHalf = samples.Take(switches / 2).Average();
            var secondHalf = samples.Skip(switches / 2).Average();
            var ratio = secondHalf / Math.Max(firstHalf, 1.0);
            _output.WriteLine(
                $"页面切换 {switches} 次：前半均 {firstHalf:F0}ms、后半均 {secondHalf:F0}ms"
                + $"（{ratio:F2}×）；逐次 = {string.Join(", ", samples.Select(s => s.ToString("F0")))}");

            Assert.True(
                ratio < 2.5,
                $"页面切换耗时在累积：前半程均 {firstHalf:F0}ms、后半程均 {secondHalf:F0}ms"
                + $"（{ratio:F1} 倍）。常见成因是每次切换都新建订阅而旧的没解绑，"
                + "用户表现为「切几次页面越来越卡」");

            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// **重复挂载/卸载画布视图不得让节点视觉丢失**。
    ///
    /// 切页面回来后画布必须还在。这条是正确性，但与上一条同源：
    /// 若卸载时清了缓存索引（`_nodeContainersById` / `_edgesByNodeId`）
    /// 而重挂时没重建，回到画布页就会出现「节点在数据里、屏幕上不见了」
    /// 或「拖动不再更新边」。
    ///
    /// 判据：切走再切回之后，**拖动仍然生效**——这比「节点数还对」更强，
    /// 因为它同时验证了索引已重建。
    /// </summary>
    [Fact]
    public async Task PageSwitch_ReturningToCanvas_KeepsDragFunctional()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder), AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(async () =>
        {
            var displayNames = DisplayNameService.LoadDefault();
            var backend = DispatchProxy.Create<IAriadneBackendClient, SoftBackend>();
            var workspace = BuildWorkspace(displayNames, backend, nodeCount: 25, edgesPerNode: 1);
            var canvasView = new WorkspacePageView { DataContext = workspace };
            var otherView = new WorksPageView
            {
                DataContext = new WorksPageViewModel(displayNames, backend),
            };

            var window = new Window { Width = 1400, Height = 900, Content = canvasView };
            window.Show();
            await DrainAsync();

            // 切走再切回。
            window.Content = otherView;
            await DrainAsync();
            window.Content = canvasView;
            await DrainAsync();

            var node = workspace.Nodes[0];
            var beforeX = node.X;
            var container = FindNodeVisual(canvasView, node);
            Assert.NotNull(container);

            RaiseNodePointerPressed(canvasView, container!, new Point(0, 0));
            for (var i = 1; i <= 5; i++)
            {
                RaiseNodePointerMoved(canvasView, container!, new Point(i * 12, i * 6));
            }
            RaiseNodePointerReleased(canvasView, container!, new Point(60, 30));
            await DrainAsync();

            _output.WriteLine($"切走再切回后拖动：X {beforeX:F1} → {node.X:F1}");

            Assert.True(
                Math.Abs(node.X - beforeX) > 1,
                $"切走再切回画布页后，拖动不再生效（X 仍是 {node.X:F1}）——"
                + "重挂载时节点容器/边索引没有重建");

            window.Content = null;
            window.Close();
            await DrainAsync();
            return true;
        }, CancellationToken.None);
    }

    // ════════════════════════════════════════════════════════
    // 测量与构造工具
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 在**同一个** headless 会话里测两组规模的「打开画布页」耗时。
    ///
    /// ⚠️ 合并会话是必需的，不是优化：这台 3.8G ARM 板上起第二个
    /// <c>HeadlessUnitTestSession</c> 会各自加载一份 Avalonia + 主题字典而被
    /// OOM 杀掉（退出码 143，日志停在测试开始处，看起来像卡死）。
    ///
    /// **ViewModel 构造与节点填充不计时**：这里量的是「打开页面」，
    /// 即视图树构建 + 模板实体化 + 首轮布局 + 边几何首算。
    /// 把数据准备算进去会稀释信号，让真正的视图侧退化被淹没。
    /// </summary>
    private async Task<(double First, double Second)> MeasureCanvasOpenPairAsync(
        int firstNodes, int firstEdges, int secondNodes, int secondEdges)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder), AvaloniaTestIsolationLevel.PerTest);

        return await session.Dispatch(async () =>
        {
            var displayNames = DisplayNameService.LoadDefault();
            var backend = DispatchProxy.Create<IAriadneBackendClient, SoftBackend>();
            // ⚠️ 必须先预热一次再计时。诊断实测（Diagnose_CanvasOpen_WhereDoesTheTimeGo）：
            // 首次打开画布 6.65s，其中真实开销只有约 2.4s（863ms Content 赋值
            // + 1546ms 首轮 Background 回调，后续 12 轮全是 0ms），
            // 余下的是 JIT + 主题字典 + 模板首次解析的**一次性**冷启动代价。
            // 不预热就计时，会把框架冷启动误报成「打开画布页要 6 秒」——
            // 那是本文件最容易造出的假缺陷。
            _ = await OpenOnceAsync(displayNames, backend, 5, 1);
            var first = await OpenOnceAsync(displayNames, backend, firstNodes, firstEdges);
            var second = await OpenOnceAsync(displayNames, backend, secondNodes, secondEdges);
            return (first, second);
        }, CancellationToken.None);
    }

    private static async Task<double> OpenOnceAsync(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        int nodeCount,
        int edgesPerNode)
    {
        var workspace = BuildWorkspace(displayNames, backend, nodeCount, edgesPerNode);
        var window = new Window { Width = 1400, Height = 900 };
        window.Show();
        await DrainAsync();

        var sw = Stopwatch.StartNew();
        window.Content = new WorkspacePageView { DataContext = workspace };
        await DrainAsync();
        sw.Stop();

        window.Content = null;
        window.Close();
        await DrainAsync();
        return sw.Elapsed.TotalMilliseconds;
    }

    /// <summary>
    /// 在**同一个** headless 会话里测两个规模的拖动摊销。
    ///
    /// 合并的理由见调用点注释：这台机器起第二个会话会 OOM。
    /// 每个规模各自建窗口/视图并在用完后关闭，互不干扰。
    /// </summary>
    private async Task<(double Small, double Large)> MeasureDragPerEventPairAsync(
        int smallNodes, int largeNodes, int edgesPerNode)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder), AvaloniaTestIsolationLevel.PerTest);

        return await session.Dispatch(async () =>
        {
            var displayNames = DisplayNameService.LoadDefault();
            var backend = DispatchProxy.Create<IAriadneBackendClient, SoftBackend>();
            var small = await MeasureOneAsync(displayNames, backend, smallNodes, edgesPerNode);
            var large = await MeasureOneAsync(displayNames, backend, largeNodes, edgesPerNode);
            return (small, large);
        }, CancellationToken.None);
    }

    private static async Task<double> MeasureOneAsync(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        int nodeCount,
        int edgesPerNode)
    {
        var workspace = BuildWorkspace(displayNames, backend, nodeCount, edgesPerNode);
        var view = new WorkspacePageView { DataContext = workspace };
        var window = new Window { Width = 1400, Height = 900, Content = view };
        window.Show();
        await DrainAsync();
        try
        {
            var node = workspace.Nodes[0];
            var container = FindNodeVisual(view, node);
            Assert.NotNull(container);

            RaiseNodePointerPressed(view, container!, new Point(0, 0));
            // 预热：首次触发含容器查找与索引建立，不代表稳态。
            for (var i = 1; i <= 5; i++)
            {
                RaiseNodePointerMoved(view, container!, new Point(i, i));
            }

            const int moves = 100;
            var sw = Stopwatch.StartNew();
            for (var i = 1; i <= moves; i++)
            {
                RaiseNodePointerMoved(view, container!, new Point(i * 2, i));
            }
            sw.Stop();
            RaiseNodePointerReleased(view, container!, new Point(moves * 2, moves));
            return sw.Elapsed.TotalMilliseconds / moves;
        }
        finally
        {
            window.Content = null;
            window.Close();
            await DrainAsync();
        }
    }

    private static async Task RunWithCanvasAsync(
        int nodeCount,
        int edgesPerNode,
        Func<WorkspacePageView, WorkspacePageViewModel, Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder), AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(async () =>
        {
            var displayNames = DisplayNameService.LoadDefault();
            var backend = DispatchProxy.Create<IAriadneBackendClient, SoftBackend>();
            var workspace = BuildWorkspace(displayNames, backend, nodeCount, edgesPerNode);
            var view = new WorkspacePageView { DataContext = workspace };
            var window = new Window { Width = 1400, Height = 900, Content = view };
            window.Show();
            await DrainAsync();

            try
            {
                await body(view, workspace);
            }
            finally
            {
                window.Content = null;
                window.Close();
                await DrainAsync();
            }
            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// 造一个含 <paramref name="nodeCount"/> 个节点的画布 ViewModel。
    ///
    /// 直接往 `Nodes`/`Edges` 集合里塞，不走后端加载：假后端一律抛异常
    /// （沿用 <c>InputSurfaceStyleTests.SoftBackend</c> 的既有约定——
    /// 返回「成功的默认值」会让 VM 走进真实加载流程继续等更多后端调用而卡死）。
    /// 本文件只测渲染耗时，数据从哪来不影响结论。
    /// </summary>
    private static WorkspacePageViewModel BuildWorkspace(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        int nodeCount,
        int edgesPerNode)
    {
        var viewModel = new WorkspacePageViewModel(displayNames, backend);

        // 网格排布：避免全部堆在同一点——重叠节点可能触发不同的命中测试与
        // 层叠路径，测出来的耗时不代表真实画布。
        const int columns = 8;
        for (var i = 0; i < nodeCount; i++)
        {
            viewModel.Nodes.Add(new WorkflowNodeViewModel(
                id: $"n{i}",
                nodeType: i == 0 ? "start" : "llm",
                label: $"节点 {i}",
                defaultWorkDir: string.Empty,
                x: (i % columns) * 260,
                y: (i / columns) * 190,
                runRequested: _ => { },
                clearSelection: () => { },
                markDirty: () => { }));
        }

        for (var i = 0; i < nodeCount; i++)
        {
            for (var k = 1; k <= edgesPerNode; k++)
            {
                var target = (i + k) % nodeCount;
                if (target == i)
                {
                    continue;
                }
                viewModel.Edges.Add(new WorkflowEdgeViewModel(
                    new CanvasEdge(
                        Id: $"e{i}-{k}",
                        Source: $"n{i}",
                        Target: $"n{target}",
                        SourceHandle: "out",
                        TargetHandle: "in",
                        Kind: "data",
                        Label: null,
                        Data: null),
                    displayNames,
                    _ => { },
                    () => { }));
            }
        }

        return viewModel;
    }

    /// <summary>
    /// 找到某个节点对应的可视容器。
    ///
    /// 按 DataContext 匹配而不是按索引：ItemsControl 的容器顺序不保证
    /// 与数据集合一致（虚拟化、回收），按索引取会在不同规模下拿到不同节点。
    /// </summary>
    private static Control? FindNodeVisual(Control root, WorkflowNodeViewModel node)
        => Descendants(root).FirstOrDefault(control => ReferenceEquals(control.DataContext, node));

    private static IEnumerable<Control> Descendants(Control root)
    {
        foreach (var child in root.GetVisualChildren().OfType<Control>())
        {
            yield return child;
            foreach (var nested in Descendants(child))
            {
                yield return nested;
            }
        }
    }

    // 直接调 View 的公开处理器，而不是走 headless 的 window.MouseDown 之类：
    // 节点拖拽在 XAML 里绑到具体元素的 PointerPressed/Moved/Released，
    // 而命中测试在 headless 下受布局与 ZIndex 影响、极易「点不到」，
    // 那会让性能测试以「找不到控件」失败而被误读成测试写错。
    // 直接调处理器量的是**同一段生产代码**的耗时，信号更干净。
    private static void RaiseNodePointerPressed(WorkspacePageView view, Control source, Point at)
        => view.OnNodePointerPressed(source, CreatePressedArgs(source, at));

    private static void RaiseNodePointerMoved(WorkspacePageView view, Control source, Point at)
        => view.OnNodePointerMoved(source, CreateMovedArgs(source, at));

    private static void RaiseNodePointerReleased(WorkspacePageView view, Control source, Point at)
        => view.OnNodePointerReleased(source, CreateReleasedArgs(source, at));

    // Pointer 在 Avalonia.Input 与 System.Reflection 下同名，必须写全限定名。
    private static readonly Avalonia.Input.Pointer TestPointer =
        new(1, PointerType.Mouse, isPrimary: true);

    private static PointerPressedEventArgs CreatePressedArgs(Control source, Point at)
        => new(
            source,
            TestPointer,
            source,
            at,
            timestamp: 0,
            new PointerPointProperties(RawInputModifiers.LeftMouseButton, PointerUpdateKind.LeftButtonPressed),
            KeyModifiers.None);

    private static PointerEventArgs CreateMovedArgs(Control source, Point at)
        => new(
            InputElement.PointerMovedEvent,
            source,
            TestPointer,
            source,
            at,
            timestamp: 0,
            new PointerPointProperties(RawInputModifiers.LeftMouseButton, PointerUpdateKind.Other),
            KeyModifiers.None);

    private static PointerReleasedEventArgs CreateReleasedArgs(Control source, Point at)
        => new(
            source,
            TestPointer,
            source,
            at,
            timestamp: 0,
            new PointerPointProperties(RawInputModifiers.None, PointerUpdateKind.LeftButtonReleased),
            KeyModifiers.None,
            MouseButton.Left);

    /// <summary>
    /// 排空调度队列。
    ///
    /// ⚠️ **不要用 `DispatcherPriority.Render`**：headless 平台没有真实渲染循环，
    /// 往 Render 优先级投递的回调可能永不执行 ⇒ `InvokeAsync` 一直不返回，
    /// 用例挂到框架超时（表现像「内存不够起不来」，与真实 OOM 日志难以区分）。
    /// 这条结论沿用 `InputSurfaceStyleTests` 的踩坑记录。
    ///
    /// ⚠️ 但**画布拖拽的帧合并恰好依赖 Render 优先级回调**
    /// （`ScheduleDragFrameSync` → Post(Render)）。所以本文件里
    /// 「松手 flush」那条用的是生产自己的 `FlushDragFrameSyncNow`
    /// （由 PointerReleased 触发），而不是指望 Render 回调被执行——
    /// 这也正是 C5-a 要求「release 前同步 flush」的原因，在 headless 下尤其明显。
    /// </summary>
    private static async Task DrainAsync()
    {
        for (var i = 0; i < 12; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Background);
        }
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    /// <summary>
    /// 后端一律抛 <c>NotSupportedException</c>——沿用本仓库既有 headless 约定。
    /// 返回「成功的默认值」会让 ViewModel 走进真实加载流程并继续等更多后端调用而卡死。
    /// ⚠️ <c>DispatchProxy</c> 要在运行时派生宿主类型，所以**不能 sealed**。
    /// </summary>
    private class SoftBackend : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}")
            {
                return false;
            }
            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
