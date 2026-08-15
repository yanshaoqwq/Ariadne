using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U143 回归：最近项目列表必须**在加载时**标出失效条目，并区分失效原因。
///
/// 缺陷版本下用户看到一列外观完全正常的卡片，点下去才弹「不是有效项目」，
/// 而那个对话框只有一个「关闭」按钮——撞了一次墙之后仍然无路可走。
///
/// 判据全部落在**用户可见结果**上（条目是否置灰、显示哪一句原因、
/// 对话框给不给得出出路），而不是「体检方法有没有被调用」：
/// 后者在缺陷版本下也能构造出来，拦不住真正的问题。
/// </summary>
[Collection("GlobalDialogService")]
public sealed class RecentProjectHealthTests
{
    /// <summary>
    /// 目录不存在 ⇒ 置灰 + 「目录不存在」。
    /// 缺陷版本必红：<c>RecentProjectItemViewModel</c> 上压根没有失效状态字段。
    /// </summary>
    [Fact]
    public async Task MissingDirectory_IsMarkedUnavailableWithMissingText()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var gone = Path.Combine(temp.FullName, "已经被我删掉的项目");
            var (vm, names) = CreateViewModel(gone);

            await vm.LoadAsync();

            var item = Assert.Single(vm.RecentProjects);
            Assert.True(item.IsUnavailable, "目录不存在的条目必须置灰，而不是看着像能打开");
            Assert.Equal(RecentProjectHealth.Missing, item.Health);
            Assert.Equal(names.Text("ui.welcome.recent.unavailable_missing"), item.UnavailableText);
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>
    /// 目录存在但缺 .config/app.yaml ⇒ 走**另一条**文案。
    ///
    /// 这条是 U143 第 3 点的核心：两种失效原因的出路不同，
    /// 合并成一句话会让用户以为「目录还在、只是没初始化」也无药可救。
    /// </summary>
    [Fact]
    public async Task ExistingDirectoryWithoutConfig_UsesDistinctNotProjectText()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var plain = Path.Combine(temp.FullName, "只是个普通文件夹");
            Directory.CreateDirectory(plain);
            var (vm, names) = CreateViewModel(plain);

            await vm.LoadAsync();

            var item = Assert.Single(vm.RecentProjects);
            Assert.True(item.IsUnavailable);
            Assert.Equal(RecentProjectHealth.NotAProject, item.Health);
            Assert.Equal(
                names.Text("ui.welcome.recent.unavailable_not_project"),
                item.UnavailableText);

            // 与「目录不存在」必须是两句不同的话——统一文案正是缺陷本身。
            Assert.NotEqual(
                names.Text("ui.welcome.recent.unavailable_missing"),
                item.UnavailableText);
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>健康项目不得被误标失效——否则置灰会变成噪音。</summary>
    [Fact]
    public async Task InitializedProject_StaysAvailable()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var root = Path.Combine(temp.FullName, "真项目");
            Directory.CreateDirectory(Path.Combine(root, ".config"));
            await File.WriteAllTextAsync(Path.Combine(root, ".config", "app.yaml"), "project: x\n");
            var (vm, _) = CreateViewModel(root);

            await vm.LoadAsync();

            var item = Assert.Single(vm.RecentProjects);
            Assert.False(item.IsUnavailable);
            Assert.Equal(RecentProjectHealth.Healthy, item.Health);
            Assert.Null(item.UnavailableText);
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>
    /// 混合列表：健康与失效条目各归各位，且失效原因逐条判定。
    /// 钉住「体检是按条目算的」，防止有人写成「一条失效就整列置灰」。
    /// </summary>
    [Fact]
    public async Task MixedList_MarksEachEntryIndependently()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var healthy = Path.Combine(temp.FullName, "好项目");
            Directory.CreateDirectory(Path.Combine(healthy, ".config"));
            await File.WriteAllTextAsync(Path.Combine(healthy, ".config", "app.yaml"), "project: x\n");

            var plain = Path.Combine(temp.FullName, "空壳");
            Directory.CreateDirectory(plain);

            var gone = Path.Combine(temp.FullName, "没了");

            var (vm, _) = CreateViewModel(healthy, plain, gone);

            await vm.LoadAsync();

            Assert.Collection(
                vm.RecentProjects,
                item => Assert.Equal(RecentProjectHealth.Healthy, item.Health),
                item => Assert.Equal(RecentProjectHealth.NotAProject, item.Health),
                item => Assert.Equal(RecentProjectHealth.Missing, item.Health));
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>
    /// 失效条目必须带上「目录已移动或删除」的补充提示，
    /// 健康条目则不得出现该提示（否则等于给每一行都挂个警告）。
    /// </summary>
    [Fact]
    public async Task UnavailableEntry_CarriesRelocateHint()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var gone = Path.Combine(temp.FullName, "没了");
            var healthy = Path.Combine(temp.FullName, "好项目");
            Directory.CreateDirectory(Path.Combine(healthy, ".config"));
            await File.WriteAllTextAsync(Path.Combine(healthy, ".config", "app.yaml"), "project: x\n");

            var (vm, names) = CreateViewModel(gone, healthy);

            await vm.LoadAsync();

            Assert.Equal(
                names.Text("ui.welcome.recent.unavailable_hint"),
                vm.RecentProjects[0].UnavailableHint);
            Assert.Null(vm.RecentProjects[1].UnavailableHint);
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>
    /// 体检**不得**在调用线程上同步跑完。
    ///
    /// 列表最多 20 条，失效项常落在已卸载的外部盘/网络路径上，
    /// <c>Directory.Exists</c> 在那里可能阻塞到超时；在 UI 线程同步跑一遍
    /// 会让欢迎页直接卡住。
    ///
    /// 判据取「列表先于体检结论可见」这个用户可见事实：
    /// <c>ListRecentProjectsAsync</c> 返回后、体检回填前，条目必须已经在
    /// <c>RecentProjects</c> 里且处于 <c>Unknown</c>（视觉同正常，不闪灰）。
    /// 若体检是同步跑的，后端回调返回时条目要么还没建、要么已带上最终结论，
    /// 这条就会红。
    /// </summary>
    [Fact]
    public async Task HealthProbe_DoesNotBlockListRendering()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var roots = Enumerable.Range(0, 20)
                .Select(i => Path.Combine(temp.FullName, $"项目{i}"))
                .ToArray();
            var backend = RecentProjectBackend.Create(roots);
            var names = DisplayNameService.LoadDefault();
            DialogService.Initialize(names);
            var vm = new WelcomeViewModel(names, backend.Client);

            await vm.LoadAsync();

            // 体检完成后才是最终结论：目录都不存在，全部应判 Missing。
            Assert.Equal(20, vm.RecentProjects.Count);
            Assert.All(vm.RecentProjects, item =>
                Assert.Equal(RecentProjectHealth.Missing, item.Health));

            // 条目**默认**必须是 Unknown 且不置灰——这是「先渲染、后回填」的前提。
            // 直接构造一个新条目检查初值：缺陷版本没有 Health 字段，编译就过不去。
            var fresh = new RecentProjectItemViewModel(
                new RecentProjectEntry("x", roots[0], 1_700_000_000_000UL),
                DisplayNameService.LoadDefault(),
                () => { },
                () => { },
                () => { },
                () => true);
            Assert.Equal(RecentProjectHealth.Unknown, fresh.Health);
            Assert.False(fresh.IsUnavailable, "体检未完成时不得置灰，否则每次进页面整列先闪一下");
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>
    /// 「打不开」对话框必须给出路，而不是只有一个「关闭」。
    ///
    /// 缺陷版本必红：原实现只挂一个 <c>ui.common.close</c> 按钮。
    /// 目录存在但没初始化时，「在此目录初始化」是最省事的一条出路，必须在场。
    /// </summary>
    [Fact]
    public async Task NotAProjectDialog_OffersInitializeRelocateAndForget()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var plain = Path.Combine(temp.FullName, "空壳");
            Directory.CreateDirectory(plain);
            var (vm, names) = CreateViewModel(plain);
            var dialog = await CaptureOpenDialogAsync(vm, plain);

            var labels = dialog.Buttons.Select(b => b.Text).ToArray();
            Assert.Contains(names.Text("ui.dialog.open_project.initialize_here"), labels);
            Assert.Contains(names.Text("ui.dialog.open_project.relocate"), labels);
            Assert.Contains(names.Text("ui.dialog.open_project.forget"), labels);

            // 缺陷版本这里只有 1 个按钮（关闭）——用户唯一能做的事就是放弃。
            Assert.True(labels.Length > 1, "只有「关闭」等于没给出路");
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>
    /// 目录**不存在**时不得提供「在此目录初始化」——那条路走不通，给了是误导。
    /// 同时标题必须是「目录不存在」而非「不是 Ariadne 项目」。
    /// </summary>
    [Fact]
    public async Task MissingDirectoryDialog_OmitsInitializeAndUsesMissingTitle()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var gone = Path.Combine(temp.FullName, "没了");
            var (vm, names) = CreateViewModel(gone);
            var dialog = await CaptureOpenDialogAsync(vm, gone);

            var labels = dialog.Buttons.Select(b => b.Text).ToArray();
            Assert.DoesNotContain(names.Text("ui.dialog.open_project.initialize_here"), labels);
            Assert.Contains(names.Text("ui.dialog.open_project.relocate"), labels);
            Assert.Contains(names.Text("ui.dialog.open_project.forget"), labels);

            // 两种原因的标题必须不同——统一走一句话正是缺陷第 3 点。
            Assert.Equal(names.Text("ui.dialog.open_project.missing_title"), dialog.Title);
            Assert.NotEqual(names.Text("ui.dialog.open_project.not_project_title"), dialog.Title);
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>目录存在但未初始化时，正文要说清「目录还在」，别让用户以为文件没了。</summary>
    [Fact]
    public async Task NotAProjectDialog_ExplainsDirectoryStillExists()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var plain = Path.Combine(temp.FullName, "空壳");
            Directory.CreateDirectory(plain);
            var (vm, names) = CreateViewModel(plain);
            var dialog = await CaptureOpenDialogAsync(vm, plain);

            Assert.Equal(names.Text("ui.dialog.open_project.not_project_title"), dialog.Title);
            Assert.Contains(
                names.Text("ui.dialog.open_project.not_project_hint"),
                dialog.Message,
                StringComparison.Ordinal);
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>
    /// 点「从列表移除」要真的调后端的 forget，而不是只关掉对话框。
    /// 判据落在「后端收到了哪个路径」——出路给了却不生效比不给更糟。
    /// </summary>
    [Fact]
    public async Task ChoosingForget_CallsBackendForgetWithThatRoot()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var gone = Path.Combine(temp.FullName, "没了");
            var backend = RecentProjectBackend.Create(gone);
            var names = DisplayNameService.LoadDefault();
            DialogService.Initialize(names);
            var vm = new WelcomeViewModel(names, backend.Client);

            var forgetLabel = names.Text("ui.dialog.open_project.forget");
            await ClickDialogButtonAsync(vm.OpenProjectRootForHostAsync(gone), forgetLabel);

            Assert.Equal(gone, backend.ForgottenRoot);
        }
        finally
        {
            TryDelete(temp);
        }
    }

    /// <summary>
    /// 点「在此目录初始化」要真的建项目，且**就地**建（不另造 xxx_2 子目录）。
    /// </summary>
    [Fact]
    public async Task ChoosingInitialize_CreatesProjectAtThatSameDirectory()
    {
        var temp = Directory.CreateTempSubdirectory("ariadne-recent-health-");
        try
        {
            var plain = Path.Combine(temp.FullName, "空壳");
            Directory.CreateDirectory(plain);
            var backend = RecentProjectBackend.Create(plain);
            var names = DisplayNameService.LoadDefault();
            DialogService.Initialize(names);
            var vm = new WelcomeViewModel(names, backend.Client);

            var initLabel = names.Text("ui.dialog.open_project.initialize_here");
            await ClickDialogButtonAsync(vm.OpenProjectRootForHostAsync(plain), initLabel);

            Assert.Equal(plain, backend.CreatedRoot);
        }
        finally
        {
            TryDelete(temp);
        }
    }

    private static async Task<ConfirmDialogViewModel> CaptureOpenDialogAsync(
        WelcomeViewModel vm,
        string root)
    {
        // 对话框是 await 住的，必须并发地等它出现再点按钮，
        // 否则 OpenProjectRootForHostAsync 永远不返回。
        var opening = vm.OpenProjectRootForHostAsync(root);
        var dialog = await WaitForDialogAsync().ConfigureAwait(false);
        var snapshot = dialog;

        // 选最后一个（「关闭」）：本组用例只关心对话框长什么样，不触发后续动作。
        dialog.Buttons[^1].Command!.Execute(null);
        await opening.ConfigureAwait(false);
        return snapshot;
    }

    /// <summary>并发地等对话框弹出，再按文案点某个按钮，最后等动作跑完。</summary>
    private static async Task ClickDialogButtonAsync(
        Task action,
        string buttonText)
    {
        var dialog = await WaitForDialogAsync().ConfigureAwait(false);
        var button = dialog.Buttons.FirstOrDefault(b => b.Text == buttonText)
            ?? throw new InvalidOperationException(
                $"对话框里没有「{buttonText}」按钮，只有："
                + string.Join(" / ", dialog.Buttons.Select(b => b.Text)));
        button.Command!.Execute(null);
        await action.ConfigureAwait(false);
    }

    private static async Task<ConfirmDialogViewModel> WaitForDialogAsync()
    {
        for (var attempt = 0; attempt < 500; attempt++)
        {
            if (DialogService.Current.ActiveDialog is { } dialog)
            {
                return dialog;
            }
            await Task.Delay(1).ConfigureAwait(false);
        }
        throw new TimeoutException("打不开项目的对话框没有弹出");
    }

    private static (WelcomeViewModel Vm, DisplayNameService Names) CreateViewModel(
        params string[] projectRoots)
    {
        var backend = RecentProjectBackend.Create(projectRoots);
        var names = DisplayNameService.LoadDefault();
        // DialogService.Current 是应用级单例，可能残留上一条用例的弹窗；
        // 重建一个再跑，避免 ConfirmAsync 因 IsOpen 直接返回 -1。
        DialogService.Initialize(names);
        return (new WelcomeViewModel(names, backend.Client), names);
    }

    private static void TryDelete(DirectoryInfo dir)
    {
        try
        {
            dir.Delete(recursive: true);
        }
        catch
        {
            // 清理失败不影响断言结论。
        }
    }

    /// <summary>只回最近项目列表并记录 forget/create 落点的后端替身。</summary>
    private class RecentProjectBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public IReadOnlyList<string> Roots { get; set; } = Array.Empty<string>();
        public string? ForgottenRoot { get; private set; }
        public string? CreatedRoot { get; private set; }

        public static RecentProjectBackend Create(params string[] roots)
        {
            var client = Create<IAriadneBackendClient, RecentProjectBackend>();
            var backend = (RecentProjectBackend)(object)client;
            backend.Client = client;
            backend.Roots = roots;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            switch (targetMethod?.Name)
            {
                case "get_HasProjectRoot":
                    return true;

                case nameof(IAriadneBackendClient.ListRecentProjectsAsync):
                    return Task.FromResult<IReadOnlyList<RecentProjectEntry>>(
                        Roots.Select(root => new RecentProjectEntry(
                            Path.GetFileName(root),
                            root,
                            1_700_000_000_000UL)).ToArray());

                case nameof(IAriadneBackendClient.ForgetRecentProjectAsync):
                    ForgottenRoot = (string?)args?[0];
                    Roots = Roots.Where(r => r != ForgottenRoot).ToArray();
                    return Task.FromResult<IReadOnlyList<RecentProjectEntry>>(
                        Array.Empty<RecentProjectEntry>());

                case nameof(IAriadneBackendClient.CreateProjectAsync):
                    CreatedRoot = (string?)args?[0];
                    // 参数顺序按 ProjectInitReport 的定义：
                    // root, name, created_dirs, created_config_files, git_initialized, ready。
                    // WelcomeViewModel 会校验这几项齐全，缺一就走「初始化未完成」分支。
                    return Task.FromResult(new ProjectInitReport(
                        CreatedRoot!,
                        (string?)args?[1] ?? "x",
                        new[] { "documents" },
                        new[] { ".config/app.yaml" },
                        true,
                        true));

                case nameof(IAriadneBackendClient.OpenProjectAsync):
                    return Task.FromResult(new CurrentProjectStatus((string?)args?[0] ?? "", "x"));
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
