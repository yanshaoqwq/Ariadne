using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U163-B：导入源在项目外是正常用例，不得被前端判为错误。
///
/// 后端两个校验函数刻意不同（已读代码复核）：
/// `import_source_path_buf`（commands.rs:14409）只禁 `..`，绝对路径原样放行；
/// `project_path_buf`（:14418）才额外要求 ensure_path_under_root。
/// `import_chapter`（:2276）分别调用二者，并在源在项目外时把源目录临时加进只读沙箱。
/// 前端此前对源和落点用同一套「必须在项目内」，把后端明确支持的能力挡死了——
/// 而文件选择器不受此限，于是「浏览让你选、选完告诉你不行」。
///
/// 判据取 IsValid 与 NormalizedPath 两者：只测 IsValid 会漏掉
/// 「放行了但把绝对路径当相对路径拼到项目根后面」这种更难查的形态。
/// </summary>
public sealed class WorksImportOutsideProjectSourceTests
{
    private const string ProjectRoot = "/home/author/novels/my-book";

    [Theory]
    [InlineData("/home/author/Downloads/第一章.md")]
    [InlineData("/media/usb/稿子/ch01.txt")]
    [InlineData("/tmp/exported/scrivener/chapter-03.markdown")]
    public void ImportSource_OutsideProject_IsAccepted(string source)
    {
        var result = WorksImportHelper.ValidateProjectPath(
            source,
            ProjectRoot,
            requireDocumentsDirectory: false,
            requireInsideProject: false);

        Assert.True(
            result.IsValid,
            $"项目外的源稿被判为 {result.Error}，导入按钮会灰掉——"
            + "作者从下载目录/U盘挑稿子是最常见的用法，后端本来支持。");

        // 关键：必须保持绝对路径原样，不能被当成项目相对路径拼到项目根后面
        Assert.Equal(source, result.NormalizedPath);
        Assert.DoesNotContain(ProjectRoot, result.NormalizedPath, StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("../../etc/passwd", ImportPathError.ParentTraversal)]
    [InlineData("/home/author/../../etc/passwd", ImportPathError.ParentTraversal)]
    [InlineData("C:\\x\\y.md", ImportPathError.UnsupportedPathForm)]
    [InlineData("\\\\server\\share\\ch01.md", ImportPathError.UnsupportedPathForm)]
    public void ImportSource_StillRejectsTraversalAndBadForm(string source, ImportPathError expected)
    {
        var result = WorksImportHelper.ValidateProjectPath(
            source,
            ProjectRoot,
            requireDocumentsDirectory: false,
            requireInsideProject: false);

        Assert.False(result.IsValid, $"放宽「项目内」后不该连带放行 {source}");
        Assert.Equal(expected, result.Error);
        Assert.Equal(string.Empty, result.NormalizedPath);
    }

    [Fact]
    public void ImportSource_InsideProject_StaysProjectRelative()
    {
        // 项目内的源仍按项目相对路径规范化（后端会拼到项目根下），语义不变
        var result = WorksImportHelper.ValidateProjectPath(
            ProjectRoot + "/planning/imports/chapter.md",
            ProjectRoot,
            requireDocumentsDirectory: false,
            requireInsideProject: false);

        Assert.True(result.IsValid);
        Assert.Equal("planning/imports/chapter.md", result.NormalizedPath);
    }

    [Fact]
    public void ImportTarget_OutsideProject_IsStillRejected()
    {
        // 落点是我们要写入的地方，必须留在项目内——放宽只针对源
        var result = WorksImportHelper.ValidateProjectPath(
            "/home/author/Downloads/第一章.md",
            ProjectRoot,
            requireDocumentsDirectory: true);

        Assert.False(result.IsValid, "落点在项目外必须仍被拒（后端 project_path_buf 也会拒）");
        Assert.Equal(ImportPathError.OutsideProject, result.Error);
    }

    [Fact]
    public void ImportSource_HomePrefixed_IsExpandedToAbsolutePath()
    {
        // 后端不认 `~`：import_source_path_buf 会把它当相对路径拼到项目根下，
        // 得到一个不存在的路径。所以必须在前端就展开。
        var result = WorksImportHelper.ValidateProjectPath(
            "~/Downloads/稿子.md",
            ProjectRoot,
            requireDocumentsDirectory: false,
            requireInsideProject: false);

        Assert.True(result.IsValid, $"~/ 前缀的源稿被判为 {result.Error}");
        Assert.False(
            result.NormalizedPath.StartsWith('~'),
            "`~` 必须在前端展开成绝对路径，否则后端会把它拼到项目根下变成不存在的路径：显示为 "
            + result.NormalizedPath);
        Assert.True(Path.IsPathRooted(result.NormalizedPath));
        Assert.EndsWith("Downloads/稿子.md", result.NormalizedPath, StringComparison.Ordinal);
    }

    /// <summary>
    /// 上面几条都只测纯函数，**挡不住「函数放宽了但调用点没传 false」**——
    /// 把 WorksPageViewModel 的 requireInsideProject 改回 true，那些用例照样全绿。
    /// 所以这一条必须落在用户真实动作上：在作品页填一个项目外的源稿，
    /// 判据取「导入按钮能不能按」+「后端实际收到的 source_path 是什么」。
    /// </summary>
    [Fact]
    public async Task WorksPage_ImportButtonEnabled_AndSendsAbsoluteSourceOutsideProject()
    {
        var root = Path.Combine(Path.GetTempPath(), $"ariadne-u163b-{Guid.NewGuid():N}");
        Directory.CreateDirectory(Path.Combine(root, "documents"));
        var outsideSource = Path.Combine(Path.GetTempPath(), $"ariadne-u163b-src-{Guid.NewGuid():N}.md");
        await File.WriteAllTextAsync(outsideSource, "# 第一章\n正文\n");
        try
        {
            var backend = ImportCaptureBackend.Create(root);
            var vm = new WorksPageViewModel(
                Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(), backend.Client);

            // 打开导入面板会带出项目根（真实路径，异步）
            vm.ToggleImportPanelCommand.Execute(null);
            await backend.ProjectRootRequested.Task;
            await Task.Yield();

            vm.ImportChapterId = "chapter-one";
            vm.ImportChapterTitle = "第一章";
            vm.ImportOrder = 1m;
            vm.ImportSourcePath = outsideSource;      // 作者从「浏览」里选的项目外文件
            vm.ImportTargetPath = "documents/chapter-01.md";

            Assert.False(
                vm.HasImportSourceError,
                "项目外的源稿被标成错误：" + vm.ImportSourceErrorText
                + "（文件选择器让作者选了它，校验却说不行）");
            Assert.True(
                vm.ImportCommand.CanExecute(null),
                "导入按钮被禁用——作者选完项目外的稿子就无路可走了");

            vm.ImportCommand.Execute(null);
            await backend.ImportRequested.Task;

            Assert.NotNull(backend.LastRequest);
            Assert.Equal(outsideSource, backend.LastRequest!.SourcePath);
            Assert.Equal("documents/chapter-01.md", backend.LastRequest.TargetPath);
        }
        finally
        {
            File.Delete(outsideSource);
            Directory.Delete(root, recursive: true);
        }
    }

    private class ImportCaptureBackend : System.Reflection.DispatchProxy
    {
        private string _root = string.Empty;

        public IAriadneBackendClient Client { get; private set; } = null!;
        public ChapterImportRequest? LastRequest { get; private set; }
        public TaskCompletionSource<bool> ProjectRootRequested { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public TaskCompletionSource<bool> ImportRequested { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public static ImportCaptureBackend Create(string root)
        {
            var client = Create<IAriadneBackendClient, ImportCaptureBackend>();
            var backend = (ImportCaptureBackend)(object)client;
            backend.Client = client;
            backend._root = root;
            return backend;
        }

        protected override object? Invoke(System.Reflection.MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.GetCurrentProjectAsync))
            {
                ProjectRootRequested.TrySetResult(true);
                return Task.FromResult<CurrentProjectStatus?>(new CurrentProjectStatus(_root, "U163B"));
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.ImportChapterAsync)
                && args is { Length: > 0 } && args[0] is ChapterImportRequest request)
            {
                LastRequest = request;
                ImportRequested.TrySetResult(true);
                return Task.FromResult(new ChapterImportReport(null, null));
            }

            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object?[]
                    {
                        resultType.IsValueType ? Activator.CreateInstance(resultType) : null,
                    });
            }

            return targetMethod.ReturnType.IsValueType
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
