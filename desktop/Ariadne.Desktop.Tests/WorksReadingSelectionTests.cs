using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U132 回归：阅读态既没有 Ctrl+A/Ctrl+C，又摆着两个会误导的可见入口。
///
/// 缺陷有三层，测试逐层钉住：
/// ① 右键菜单「复制」在非编辑态**直接回退整章正文**——实测把 51088 字符
///    塞进剪贴板、无视用户选中的 10 个字；
/// ② 「全选」带 IsEditMode 前置判断，阅读态点了毫无反应；
/// ③ 快捷键全链路无一处处理 Ctrl+A/Ctrl+C。
///
/// 剪贴板与键盘事件要真实 UI 才能测，所以这里钉住**可测的那一层**：
/// 跨块选区归并（复制内容的唯一来源）与右键菜单可见性。
/// 判据是「归并出的文本是不是选中那段」，而不是「函数能否调用」——
/// 后者在缺陷版本下同样为真。
/// </summary>
public sealed class WorksReadingSelectionTests
{
    [Fact]
    public void Aggregate_ReturnsOnlySelectedSpan_NotWholeChapter()
    {
        var blocks = CreateBlocks("第一块正文内容", "第二块正文内容");

        // 只选第 1 块的第 2-5 个字符。
        var text = ReadingSelectionAggregator.Aggregate(
            blocks,
            new[] { new ReadingSelectionAggregator.BlockSelection(0, 2, 5) });

        // 缺陷版本返回的是整章（两块拼起来 14 字符）。
        Assert.Equal("块正文", text);
    }

    [Fact]
    public void Aggregate_JoinsBlocksInDocumentOrderRegardlessOfSampleOrder()
    {
        var blocks = CreateBlocks("前块", "后块");

        // 视觉树的枚举顺序未必等于正文顺序，故采样顺序刻意倒置。
        var text = ReadingSelectionAggregator.Aggregate(
            blocks,
            new[]
            {
                new ReadingSelectionAggregator.BlockSelection(1, 0, 2),
                new ReadingSelectionAggregator.BlockSelection(0, 0, 2),
            });

        // 按采样顺序拼会得到「后块前块」，那是把小说读倒了。
        Assert.Equal("前块后块", text);
    }

    [Fact]
    public void Aggregate_SkipsEmptySelections()
    {
        var blocks = CreateBlocks("甲块内容", "乙块内容");

        // Avalonia 里未选中的块 Start/End 都是 0。不跳过这些，
        // 「有没有选区」的判断会永远为真，复制就退化成复制整章——正是本条缺陷。
        var selections = new[]
        {
            new ReadingSelectionAggregator.BlockSelection(0, 0, 0),
            new ReadingSelectionAggregator.BlockSelection(1, 1, 3),
        };

        Assert.Equal("块内", ReadingSelectionAggregator.Aggregate(blocks, selections));
        Assert.True(ReadingSelectionAggregator.HasSelection(selections));
    }

    [Fact]
    public void Aggregate_SkipsInvertedSelections()
    {
        var blocks = CreateBlocks("甲块内容", "乙块内容");

        // End < Start 的倒置采样必须被**过滤掉**，不能只靠后面的钳位兜着。
        // 钳位会把 (2,0) 变成 (2,2) 即空串——看起来结果一样，
        // 但那条采样已经进了 ordered 列表，于是 ordered.Count > 0、
        // 「有选区」判定成立。缺陷版本据此走进复制分支，而拼出来是空串：
        // 用户按 Ctrl+C 后剪贴板被**清空**，他刚才复制的别的东西也没了。
        var selections = new[]
        {
            new ReadingSelectionAggregator.BlockSelection(0, 3, 1),
            new ReadingSelectionAggregator.BlockSelection(1, 2, 2),
        };

        Assert.False(ReadingSelectionAggregator.HasSelection(selections));
        Assert.Equal(string.Empty, ReadingSelectionAggregator.Aggregate(blocks, selections));
    }

    [Fact]
    public void HasSelection_IsFalseWhenNothingSelected()
    {
        var selections = new[]
        {
            new ReadingSelectionAggregator.BlockSelection(0, 0, 0),
            new ReadingSelectionAggregator.BlockSelection(1, 4, 4),
        };

        // 一字未选时必须能判出来。判不出来 → 走「回退整章」分支，
        // 用户按 Ctrl+C 会以为复制成功了，直到粘贴出五万字才发现。
        Assert.False(ReadingSelectionAggregator.HasSelection(selections));
        Assert.Equal(string.Empty, ReadingSelectionAggregator.Aggregate(
            CreateBlocks("甲块内容", "乙块内容"),
            selections));
    }

    [Fact]
    public void Aggregate_ClampsOutOfRangeSamples()
    {
        var blocks = CreateBlocks("短块");

        // 文本刚变化、选区尚未同步的那一帧会给出越界值。
        // 直接切片就是 ArgumentOutOfRangeException——用户只会看到「复制没反应」。
        var text = ReadingSelectionAggregator.Aggregate(
            blocks,
            new[] { new ReadingSelectionAggregator.BlockSelection(0, 1, 999) });

        Assert.Equal("块", text);
    }

    [Fact]
    public void Aggregate_IgnoresSelectionsForBlocksThatNoLongerExist()
    {
        var blocks = CreateBlocks("仅一块");

        // 正文被裁短后残留的旧采样。索引越界不能抛，那会让整次复制失败。
        var text = ReadingSelectionAggregator.Aggregate(
            blocks,
            new[]
            {
                new ReadingSelectionAggregator.BlockSelection(7, 0, 2),
                new ReadingSelectionAggregator.BlockSelection(0, 0, 2),
            });

        Assert.Equal("仅一", text);
    }

    [Fact]
    public void SelectionContextItems_AreHiddenInReadMode()
    {
        var vm = CreateViewModel();

        vm.IsEditMode = false;
        // 产品决策：阅读态不显示复制/全选按钮，只保留快捷键。
        // 缺陷版本两项都可见，且都是错的——比没有更糟，
        // 因为用户会以为自己操作错了，而不是知道这里没这个功能。
        Assert.False(vm.ShowSelectionContextItems);

        vm.IsEditMode = true;
        Assert.True(vm.ShowSelectionContextItems);
    }

    private static IReadOnlyList<DocumentBlockViewModel> CreateBlocks(params string[] texts)
    {
        var blocks = new List<DocumentBlockViewModel>();
        var offset = 0;
        for (var index = 0; index < texts.Length; index++)
        {
            blocks.Add(new DocumentBlockViewModel($"read-block-{index}", index, texts[index], offset));
            offset += texts[index].Length;
        }
        return blocks;
    }

    private static WorksPageViewModel CreateViewModel()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, ReadingSelectionBackendProxy>();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.SeedOpenDocumentForTests("documents/chapter-2.md", "v1", "第一段。\n第二段。");
        return vm;
    }

    private class ReadingSelectionBackendProxy : DispatchProxy
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
