using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U150：提示词编辑框里的 Ctrl+左键**不会**误触发画布多选——守住这条结论的**前提**。
///
/// # 报告说有手势冲突，实测没有
///
/// 报告称「编辑框就在画布右栏，Ctrl+左键展开引用会连带把节点加进多选」，
/// 并列为「必须先解决的真实冲突」。逐条实测后结论是**不冲突**：
///
/// 1. 多选处理器挂在**节点卡标题栏**上（`node-card-header` 的
///    `PointerPressed="OnNodePointerPressed"`），XAML 属性绑定 = **Bubble 阶段**
/// 2. 编辑框在右栏检查器，与节点卡是视觉树上的**平行分支**；
///    Bubble 只往自己的祖先走 ⇒ 编辑框的点击永远到不了节点卡的处理器
/// 3. 唯一能先拿到事件的是 Tunnel 处理器（节点库拖拽），但它有正确的前置过滤
///
/// # 为什么还要这个测试
///
/// **结论对，但前提会变。** 把多选处理器上移到画布容器、或改挂 Tunnel，
/// 冲突就真的出现了——而那时症状是「点一下引用，节点莫名进了多选」，
/// 极难联想到是事件路由变了。
///
/// 所以这里钉的不是「不冲突」（那是推论），而是**推论依赖的三个前提**。
/// 前提一变就红，并在失败信息里指出「要重新评估 U150 的手势冲突」。
///
/// ⚠️ 这是**源码断言**，它不过 XAML 编译也不实体化视觉树——
/// 本机 Avalonia headless 起不来，只能退到这一层。
/// 代价要说清：它证明不了「运行时真的不冒泡」，只证明「代码仍是我实测时那个形状」。
/// 真正的运行时验证要等能跑 headless 的环境。
/// </summary>
public sealed class ReferenceGestureRoutingTests
{
    /// <summary>
    /// 前提 1+2：多选挂在节点卡标题栏的 Bubble 阶段。
    ///
    /// 断言 XAML 里那个属性绑定仍在 `node-card-header` 上。
    /// 属性绑定形式本身就意味着 Bubble——Avalonia 没有「在 XAML 里指定 Tunnel」的语法，
    /// 要 Tunnel 必须写 `AddHandler(..., RoutingStrategies.Tunnel)`。
    /// 所以「在 XAML 里」= 「Bubble」，这一条同时钉住了阶段。
    /// </summary>
    [Fact]
    public void MultiSelectStaysOnTheNodeCardBubblePhase()
    {
        var markup = File.ReadAllText(ResolveDesktopFile("Views", "WorkspacePageView.axaml"));

        var handlerAt = markup.IndexOf(
            "PointerPressed=\"OnNodePointerPressed\"",
            StringComparison.Ordinal);
        Assert.True(
            handlerAt > 0,
            "找不到 OnNodePointerPressed 的 XAML 绑定。若它改成了 AddHandler，"
            + "务必确认没有挂 Tunnel——那会让它先于提示词编辑框拿到事件，"
            + "U150 的 Ctrl+左键就会连带触发多选。");

        // 往前找最近的元素起始标签，确认宿主是节点卡标题栏。
        var tagStart = markup.LastIndexOf('<', handlerAt);
        Assert.True(tagStart >= 0);
        var hostTag = markup[tagStart..handlerAt];
        Assert.Contains("node-card-header", hostTag, StringComparison.Ordinal);
    }

    /// <summary>
    /// 前提 3：Tunnel 阶段的指针处理器必须有「不是我的目标就放手」的前置过滤。
    ///
    /// Tunnel + `handledEventsToo: true` 意味着它**先于所有人**拿到每一次指针按下，
    /// 包括提示词编辑框里的。没有前置过滤它就会吞掉一切。
    ///
    /// 判据取「处理器体内先做目标判定再动状态」：
    /// 用 `FindNodeLibraryItem(...) is not { }` 这个具体形状断言，
    /// 而不是断言「有 if」——后者任何实现都满足，等于没测。
    /// </summary>
    [Fact]
    public void TunnelPointerHandlerFiltersToItsOwnTargets()
    {
        var source = File.ReadAllText(
            ResolveDesktopFile("Views", "WorkspacePageView.axaml.cs"));

        // 确认它真的挂在 Tunnel（这是本条存在的理由）。
        var tunnelAt = source.IndexOf(
            "OnNodeLibraryItemPointerPressed,\n            Avalonia.Interactivity.RoutingStrategies.Tunnel",
            StringComparison.Ordinal);
        if (tunnelAt < 0)
        {
            // 换行/缩进可能被格式化工具改动，退一步只查两个标记同时出现。
            Assert.Contains("RoutingStrategies.Tunnel", source, StringComparison.Ordinal);
            Assert.Contains("OnNodeLibraryItemPointerPressed", source, StringComparison.Ordinal);
        }

        var bodyAt = source.IndexOf(
            "public void OnNodeLibraryItemPointerPressed(",
            StringComparison.Ordinal);
        Assert.True(bodyAt > 0, "找不到 Tunnel 指针处理器的实现");
        var body = source[bodyAt..Math.Min(bodyAt + 700, source.Length)];

        Assert.Contains("FindNodeLibraryItem", body, StringComparison.Ordinal);
        // 前置过滤必须在动任何状态**之前**：先 return 才不会污染拖拽状态机。
        var guardAt = body.IndexOf("FindNodeLibraryItem", StringComparison.Ordinal);
        var mutateAt = body.IndexOf("_libraryPointerDown", StringComparison.Ordinal);
        Assert.True(
            guardAt > 0 && mutateAt > guardAt,
            "Tunnel 处理器在做目标判定之前就改了拖拽状态。它先于提示词编辑框拿到"
            + "每一次指针按下，一旦提前置位，编辑框里的点击会把节点库拖拽状态机激活。");
    }

    private static string ResolveDesktopFile(params string[] parts)
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }
        Assert.NotNull(dir);
        var path = Path.Combine(
            new[] { dir!, "Ariadne.Desktop" }.Concat(parts).ToArray());
        Assert.True(File.Exists(path), $"找不到 {path}");
        return path;
    }
}
