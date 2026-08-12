using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U129 回归：「查看/修改切换后页面位置不变」此前完全不成立——
/// 阅读模式压根没有滚动位置这个概念（U128 修好前连滚动都没有），
/// <c>IsEditMode</c> setter 对位置零处理，切回来永远从头开始。
///
/// 判据分两层：
/// (a) <see cref="ReadingPositionMapper"/> 的偏移 ↔ 块位置换算必须是可逆的；
/// (b) <c>IsEditMode</c> 切换时必须**先捕获后恢复**，且换文档要作废锚点。
/// 只测「有没有这两个钩子」是不够的——顺序反了照样丢位置，而现象一模一样。
/// </summary>
public sealed class WorksReadingPositionTests
{
    [Fact]
    public void OffsetFromBlockProgress_RoundTripsThroughLocate()
    {
        var blocks = CreateBlocks(400, 300, 500);

        // 第 2 块（起始 400）滚过一半 → 偏移 550。
        var offset = ReadingPositionMapper.OffsetFromBlockProgress(blocks, 1, 0.5d);
        Assert.Equal(550, offset);

        Assert.True(ReadingPositionMapper.TryLocateOffset(blocks, offset, out var index, out var ratio));
        Assert.Equal(1, index);
        Assert.Equal(0.5d, ratio, 3);
    }

    [Fact]
    public void TryLocateOffset_AssignsBlockBoundaryToFollowingBlock()
    {
        var blocks = CreateBlocks(400, 300, 500);

        // 偏移 400 正好是第 2 块的首字符。归给第 1 块会得到 ratio = 1
        // （滚到第 1 块尾部），视觉上差**一整块**——而块是 4000–6000 字符、约 8–12 屏。
        Assert.True(ReadingPositionMapper.TryLocateOffset(blocks, 400, out var index, out var ratio));
        Assert.Equal(1, index);
        Assert.Equal(0d, ratio, 6);
    }

    [Fact]
    public void TryLocateOffset_ClampsPastEndToLastBlock()
    {
        var blocks = CreateBlocks(400, 300, 500);

        // 正文末尾偏移等于末块 EndOffset，没有「后一块」可归；
        // 超出总长的偏移（正文被裁短后残留的旧锚点）也必须落在末块，不能返回 false。
        Assert.True(ReadingPositionMapper.TryLocateOffset(blocks, 1_200, out var index, out var ratio));
        Assert.Equal(2, index);
        Assert.Equal(1d, ratio, 6);

        Assert.True(ReadingPositionMapper.TryLocateOffset(blocks, 99_999, out var farIndex, out var farRatio));
        Assert.Equal(2, farIndex);
        Assert.Equal(1d, farRatio, 6);
    }

    [Fact]
    public void OffsetFromBlockProgress_RejectsNaNRatio()
    {
        var blocks = CreateBlocks(400, 300);

        // 高度为 0 的块做除法会产出 NaN；NaN 一路乘下去再 (int) 转换得到 0，
        // 那等于静默退回「从头开始」——正是本条缺陷的现象，必须显式挡掉。
        var offset = ReadingPositionMapper.OffsetFromBlockProgress(blocks, 1, double.NaN);
        Assert.Equal(400, offset);
    }

    [Fact]
    public void TryLocateOffset_ReturnsFalseWithoutBlocks()
    {
        Assert.False(ReadingPositionMapper.TryLocateOffset(
            Array.Empty<DocumentBlockViewModel>(),
            42,
            out _,
            out _));
    }

    [Fact]
    public void SwitchingViewMode_CapturesBeforeRestoring()
    {
        var vm = CreateViewModel();
        var trace = new List<string>();
        vm.CaptureReadingOffset = () =>
        {
            // 关键断言在这里而不在事后：捕获执行的**那一刻** IsEditMode 必须仍是旧值。
            // 只断言「capture 在 restore 之前」是不够的——把捕获挪到 SetProperty
            // 之后，那个顺序照样成立，但此刻 IsVisible 已经翻转、旧视图的
            // ScrollViewer/VisualLine 已问不出有效位置（未测量的控件返回 0，
            // 等于静默丢失位置，而现象与「压根没实现」一模一样）。
            trace.Add($"capture@IsEditMode={vm.IsEditMode}");
            return 137;
        };
        vm.RestoreReadingOffset = offset =>
        {
            trace.Add($"restore:{offset}@IsEditMode={vm.IsEditMode}");
        };

        vm.IsEditMode = true;

        Assert.Equal(
            new[] { "capture@IsEditMode=False", "restore:137@IsEditMode=True" },
            trace);
    }

    [Fact]
    public void SwitchingViewMode_KeepsLastAnchorWhenCaptureUnavailable()
    {
        var vm = CreateViewModel();
        var restored = new List<int>();
        var captured = new Queue<int?>(new int?[] { 500, null });
        vm.CaptureReadingOffset = () => captured.Count > 0 ? captured.Dequeue() : null;
        vm.RestoreReadingOffset = restored.Add;

        vm.IsEditMode = true;   // 捕获到 500
        vm.IsEditMode = false;  // 捕获失败（返回 null）

        // 捕获失败时**保留**上一个锚点，而不是覆盖成 0。
        // 覆盖成 0 会让「正文短暂未测量」这种时序抖动直接表现为跳回开头。
        Assert.Equal(new[] { 500, 500 }, restored);
    }

    [Fact]
    public void SwitchingViewMode_DoesNotRestoreWithoutAnyAnchor()
    {
        var vm = CreateViewModel();
        var restoreCount = 0;
        vm.CaptureReadingOffset = () => null;
        vm.RestoreReadingOffset = _ => restoreCount++;

        vm.IsEditMode = true;

        // 从未捕获到任何位置时不该发起恢复——传 0 进去等于主动滚到开头，
        // 而用户此刻可能正停在文档中段（例如刚由「定位到出处」跳过来）。
        Assert.Equal(0, restoreCount);
    }

    [Fact]
    public void OpeningAnotherDocument_InvalidatesAnchor()
    {
        var vm = CreateViewModel();
        vm.CaptureReadingOffset = () => 900;
        vm.RestoreReadingOffset = _ => { };
        vm.IsEditMode = true;

        var restored = new List<int>();
        vm.RestoreReadingOffset = restored.Add;
        vm.CaptureReadingOffset = () => null;
        vm.SeedOpenDocumentForTests("documents/chapter-9.md", "v1", "新的一章正文。");

        vm.IsEditMode = false;

        // 换文档后旧偏移必须作废：900 套到新正文（7 字符）上会滚到毫无关系的位置，
        // 而这种错位比「从头开始」更难被用户理解成 bug。
        Assert.Empty(restored);
    }

    private static IReadOnlyList<DocumentBlockViewModel> CreateBlocks(params int[] lengths)
    {
        var blocks = new List<DocumentBlockViewModel>();
        var offset = 0;
        for (var index = 0; index < lengths.Length; index++)
        {
            blocks.Add(new DocumentBlockViewModel(
                $"read-block-{index}",
                index,
                new string('文', lengths[index]),
                offset));
            offset += lengths[index];
        }
        return blocks;
    }

    private static WorksPageViewModel CreateViewModel()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, ReadingPositionBackendProxy>();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.SeedOpenDocumentForTests("documents/chapter-5.md", "v1", "第一段。\n第二段。\n第三段。");
        vm.IsEditMode = false;
        return vm;
    }

    private class ReadingPositionBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }

            if (targetMethod?.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (targetMethod?.ReturnType.IsGenericType == true
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object?[] { null });
            }

            return null;
        }
    }
}
