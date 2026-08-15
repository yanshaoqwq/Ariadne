using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U136 + U140 回归：稿纸不像 Word 的「页」。
///
/// 原缺陷：正文测量宽 = Grid MaxWidth 1180 − 稿纸左右 padding 144 = **1036px**，
/// fontSize 16 ⇒ **每行约 65 个汉字**。对照 Word A4 默认版式（小四 ≈ 37.6 字/行）
/// 与排版学对 CJK 单栏的建议（25–45 字），65 字意味着回行时眼睛要横扫近一米、
/// 极易串行——**那是网页正文容器的宽度，不是书页的宽度**。
///
/// 判据落在**推导出的字数**上而不是「常量等于多少」：后者改个数字就能骗过，
/// 前者会在「调了字号却没调版心」时同样变红——那正是 U140 要防的漂移。
/// </summary>
public sealed class ReadingMeasureTests
{
    /// 正文字号，与 AriadneTheme.axaml 的 `*.reading` 样式一致。
    private const double ReadingFontSize = 16d;
    /// 稿纸左右内边距合计，与 `Ariadne.Reading.SurfacePadding` 的左右值一致。
    private const double SurfaceHorizontalPadding = 144d;

    [Fact]
    public void ClosedOutlinePanel_KeepsMeasureWithinCjkReadableRange()
    {
        var vm = CreateViewModel();
        Assert.False(vm.IsOutlinePanelOpen);

        // 对照栏关闭时，纸宽 = 测量宽 + 左右内边距，没有其他占位。
        var measure = vm.DocumentSurfaceMaxWidth - SurfaceHorizontalPadding;
        var charsPerLine = measure / ReadingFontSize;

        // CJK 全角字宽 ≈ 字号。25–45 是排版学对单栏中文的建议区间；
        // 缺陷版本这里是 (1180-144)/16 ≈ 64.75 字，远在区间之外。
        Assert.InRange(charsPerLine, 25d, 45d);
    }

    [Fact]
    public void OpeningOutlinePanel_WidensThePaperInsteadOfSqueezingTheBody()
    {
        var vm = CreateViewModel();
        var closedWidth = vm.DocumentSurfaceMaxWidth;

        vm.IsOutlinePanelOpen = true;

        // 对照栏必须把纸**加宽**，不能让正文让位：固定纸宽下展开 320px 的对照栏，
        // 版心会被压到 372px、每行只剩 23 个字——比 65 字/行还难读，
        // 等于把这次修复朝反方向推过了头。
        Assert.True(vm.DocumentSurfaceMaxWidth > closedWidth);
        Assert.Equal(closedWidth + vm.OutlinePanelWidth + 48d, vm.DocumentSurfaceMaxWidth);
    }

    [Fact]
    public void OpenOutlinePanel_StillKeepsBodyMeasureReadable()
    {
        var vm = CreateViewModel();
        vm.IsOutlinePanelOpen = true;

        // 加宽的量要正好抵消对照栏占位，正文测量宽应与关闭时一致。
        var bodyMeasure = vm.DocumentSurfaceMaxWidth
                          - SurfaceHorizontalPadding
                          - vm.OutlinePanelWidth
                          - 48d;
        Assert.InRange(bodyMeasure / ReadingFontSize, 25d, 45d);
    }

    [Fact]
    public void MastheadNoLongerExposesFileSystemMetadata()
    {
        // U136 ②：刊头曾打印「路径：{path} 版本：{version} {state}」——
        // 即在书名下方印文件路径、内容哈希、保存状态。那些属于状态栏/属性面板。
        // DocumentInfoText 已删除；这条用反射钉住它不会被"顺手加回来"。
        var property = typeof(WorksPageViewModel).GetProperty("DocumentInfoText");
        Assert.Null(property);

        // 保存状态本身没有丢——它挪到了顶栏状态区。
        var saveState = typeof(WorksPageViewModel).GetProperty(nameof(WorksPageViewModel.DocumentSaveStateText));
        Assert.NotNull(saveState);
    }

    private static WorksPageViewModel CreateViewModel()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, ReadingMeasureBackendProxy>();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.SeedOpenDocumentForTests("documents/chapter-1.md", "v1", "正文。");
        return vm;
    }

    private class ReadingMeasureBackendProxy : DispatchProxy
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
