using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U153：两处「展开详情」把内容裁掉——<c>MaxHeight</c> 挂在没有滚动能力的 <c>TextBlock</c> 上。
///
/// 根因是控件能力误用：<c>MaxHeight</c> 对 <c>ScrollViewer</c> 是「限高可滚」，
/// 对 <c>TextBlock</c> 是**「裁掉多余」**——写法一样、行为相反。且裁切发生在**测量阶段**，
/// <c>TextBlock</c> 向父级上报的 <c>DesiredSize</c> 已被钉死，**外层再包滚动也救不回来**。
///
/// **判据取「内容是否可达」而不是「MaxHeight 等于多少」**：
/// 后者是标记问题（把 96 改成 320 照样能过，可内容仍被裁），
/// 只有「承载文本的容器 Extent 超过 Viewport 且能滚」才证明用户真的够得着下面的内容。
/// 缺陷版本下 <c>TextBlock</c> 外面根本没有 <c>ScrollViewer</c>，
/// <c>FindScrollHost</c> 返回 null，第一条断言即失败。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class ScrollableDetailTests
{
    /// <summary>典型后端诊断：40 行调用栈。报告实测这样一段需要 524px，而原限高只给 92px。</summary>
    private static string LongDiagnostic() => string.Join(
        Environment.NewLine,
        Enumerable.Range(0, 40).Select(i =>
            $"at Ariadne.Core.Module{i}.Handler.Invoke(request, cancellationToken) line {i * 17 + 3}"));

    /// <summary>
    /// U153 复核结论：**顶栏诊断详情不会被 MaxHeight 裁掉，报告的前提不成立。**
    ///
    /// 报告称「典型 40 行调用栈需 524px，实际显示 92px，可见 17.6%」。
    /// 实测否证：`MainWindowViewModel.Observe` 取的是 `UserFailure.RedactedDiagnostic`，
    /// 它走 `UserFacingError.Sanitize`——该函数**把换行压成空格并截断到 96 字符**
    /// （`UserFacingError.cs` 的 `s.Replace('\n', ' ')` + `if (s.Length > 96) s = s[..93] + "…"`）。
    /// 所以诊断详情**永远只有一行、不超过 96 字符**，96px 的限高压根碰不到它。
    ///
    /// 生产代码仍已改对（`MaxHeight` 从 `TextBlock` 移到 `ScrollViewer`，限高放宽到 320）——
    /// 那个改动本身是对的（写法在两类控件上语义相反，见 Git diff 那条用例），
    /// 只是它防的不是「40 行调用栈被裁」，而是「将来若放宽脱敏就不会突然开始裁」。
    ///
    /// 于是这条用例改为钉住**真实契约**：脱敏保证单行且 ≤96 字符。
    /// 这比断言「内容超出视口」有价值——后者在当前实现下永远为假，写了就是空测。
    /// </summary>
    [Fact]
    public void TopBarDiagnosticIsSanitizedToASingleShortLine()
    {
        var viewModel = new MainWindowViewModel(
            DisplayNameService.LoadDefault(),
            NoopBackend.Create());

        // 走生产入口 Observe(UserFailure)：诊断文本的 setter 是 private，
        // 绕过它去反射赋值就测不到「真实失败如何呈现」这条链路。
        viewModel.Observe(new UserFailure("internal", LongDiagnostic()));

        Assert.True(viewModel.HasDiagnostic);
        var detail = viewModel.DiagnosticDetailText;

        // 单行：换行已被压成空格，顶栏不会因为一条诊断突然长高。
        Assert.DoesNotContain('\n', detail);
        Assert.DoesNotContain('\r', detail);

        // ≤96 字符：这是 Sanitize 的硬上限，也是「MaxHeight 碰不到它」的原因。
        Assert.True(
            detail.Length <= 96,
            $"脱敏后诊断长度 {detail.Length} 超过 96——" +
            "若这条红了说明 Sanitize 的截断被放宽，" +
            "此时必须复核顶栏那个 MaxHeight=320 是否还够（U153）");

        // 40 行样本确实被压缩了，不是样本本来就短。
        Assert.True(LongDiagnostic().Split('\n').Length >= 40);
        Assert.EndsWith("…", detail, StringComparison.Ordinal);
    }

    /// <summary>
    /// Git 右栏 diff 预览同理：后端专门截到 4000 字符送来，前端原先只显示 7.4%。
    ///
    /// 这条同时守住「嵌套滚动」的取舍：右栏本身在一个 ScrollViewer 里，
    /// 内层 diff 的滚动宿主**必须是它自己那个**，而不是外层右栏——
    /// 若断言拿到的是外层，说明 MaxHeight 还挂错了地方、内层压根没有滚动能力。
    /// </summary>
    [Fact]
    public async Task GitDiffPreview_IsScrollableInsteadOfClipped()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            // 4000 字符是后端 diff_preview_with_policy(&policy, 4000) 的实际上限，
            // 用比它小的样本会让这条用例在缺陷版本下也可能侥幸通过。
            var diff = string.Join(
                Environment.NewLine,
                Enumerable.Range(0, 160).Select(i =>
                    i % 3 == 0
                        ? $"+ 第 {i} 行新增的正文，用来把预览撑过 220px 的限高。"
                        : $"- 第 {i} 行被删掉的正文。"));
            Assert.True(diff.Length > 3000, "样本要接近后端 4000 字符上限才有代表性");

            var backend = GitDiffBackend.Create(diff);
            var viewModel = new GitPageViewModel(DisplayNameService.LoadDefault(), backend);
            await viewModel.ReloadProjectDataAsync();
            Assert.True(viewModel.HasDiffPreview);

            // diff 预览所在的 Expander 绑了 IsVisible={Binding HasSelection}，
            // 不选中存档点整块不渲染——那样测到的是「没渲染」而不是「渲染了但被裁」。
            Assert.NotEmpty(viewModel.Commits);
            viewModel.SelectedCommit = viewModel.Commits[0];
            Assert.True(viewModel.HasSelection);

            var view = new GitPageView { DataContext = viewModel };
            var window = new Window
            {
                Width = 1100,
                Height = 820,
                Content = view,
            };
            window.Show();
            await DrainAsync();

            try
            {
                // diff 预览在 Expander 里，展开才会实体化。
                var expander = window.GetVisualDescendants()
                    .OfType<Expander>()
                    .FirstOrDefault();
                Assert.NotNull(expander);
                expander!.IsExpanded = true;
                await DrainAsync();

                var preview = window.GetVisualDescendants()
                    .OfType<TextBlock>()
                    .FirstOrDefault(block => ReferenceEquals(block.Text, viewModel.DiffPreviewText)
                        || string.Equals(block.Text, viewModel.DiffPreviewText, StringComparison.Ordinal));
                Assert.NotNull(preview);

                var host = FindScrollHost(preview!);
                Assert.NotNull(host);

                Assert.True(
                    host!.Extent.Height > host.Viewport.Height,
                    $"diff 预览内容高度 {host.Extent.Height} 必须超过视口 {host.Viewport.Height}");

                host.ScrollToEnd();
                await DrainAsync();
                Assert.True(host.Offset.Y > 0, "diff 预览必须真的能滚动");
            }
            finally
            {
                window.Close();
                await DrainAsync();
            }
            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// 全仓护栏：<c>TextBlock</c> 起始标签上不得再出现 <c>MaxHeight</c>。
    ///
    /// 上面两条只钉住这两个站点，但这类缺陷「写法一样、行为相反」，极易在别处重犯
    /// （报告里全仓分类核实的结论就是只有这 2 处，说明作者在其他地方都挂对了）。
    /// 这条把结论钉成断言，新增第三处时立刻红。
    /// </summary>
    [Fact]
    public void NoTextBlockCarriesMaxHeight_AcrossAllViews()
    {
        var viewsDir = ResolveDesktopDir("Views");
        var offenders = new List<string>();
        foreach (var file in Directory.EnumerateFiles(viewsDir, "*.axaml", SearchOption.AllDirectories))
        {
            var markup = File.ReadAllText(file);
            // 显式声明 Match 而不是 var：MatchCollection 的枚举项被推断为 object?，
            // 会让下面的 ToString() 触发 CS8602（解引用可能为空）。
            foreach (System.Text.RegularExpressions.Match match in System.Text.RegularExpressions.Regex.Matches(
                         markup,
                         @"<TextBlock\s[\s\S]*?/?>",
                         System.Text.RegularExpressions.RegexOptions.CultureInvariant))
            {
                var tag = match.Value;
                if (tag.Contains("MaxHeight", StringComparison.Ordinal))
                {
                    offenders.Add($"{Path.GetFileName(file) ?? file}: {tag.ReplaceLineEndings(" ")}");
                }
            }
        }

        Assert.True(
            offenders.Count == 0,
            "TextBlock 上的 MaxHeight 是裁切而非限高可滚（U153），请改挂到外包 ScrollViewer 上：\n"
            + string.Join('\n', offenders));
    }

    /// <summary>
    /// 找到承载指定控件滚动的最近祖先 ScrollViewer。
    ///
    /// 用「最近祖先」而非「页面里任意一个」：Git 右栏本身就在一个 ScrollViewer 里，
    /// 取错层会让缺陷版本也显示为「有滚动」——那正是报告里
    /// 「外层 ScrollViewer 救不了它」要防的误判。
    /// </summary>
    private static ScrollViewer? FindScrollHost(Visual target) =>
        target.GetVisualAncestors().OfType<ScrollViewer>().FirstOrDefault();

    private static TextBlock? FindDiagnosticDetailBlock(Visual root, MainWindowViewModel viewModel) =>
        root.GetVisualDescendants()
            .OfType<TextBlock>()
            .FirstOrDefault(block => string.Equals(
                block.Text,
                viewModel.DiagnosticDetailText,
                StringComparison.Ordinal));

    private static string ResolveDesktopDir(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var attempt = 0; attempt < 12 && walk is not null; attempt++)
        {
            var candidate = Path.Combine(
                new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (Directory.Exists(candidate))
            {
                return candidate;
            }
            walk = walk.Parent;
        }
        throw new DirectoryNotFoundException(string.Join('/', parts));
    }

    private static async Task DrainAsync()
    {
        for (var i = 0; i < 16; i++)
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

    /// <summary>Git 页只需要仓库状态与分支图；diff 预览由 status.DiffPreview 承载。</summary>
    // DispatchProxy 不能派生 sealed 类型。
    private class GitDiffBackend : DispatchProxy
    {
        private string _diff = string.Empty;

        public static IAriadneBackendClient Create(string diff)
        {
            var client = Create<IAriadneBackendClient, GitDiffBackend>();
            ((GitDiffBackend)(object)client)._diff = diff;
            return client;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            switch (targetMethod?.Name)
            {
                case "get_HasProjectRoot":
                    return true;
                case nameof(IAriadneBackendClient.GetGitRepositoryStatusAsync):
                    return Task.FromResult(new GitRepositoryStatus(
                        "healthy",
                        "main",
                        "abcdef1234567890",
                        true,
                        null,
                        120,
                        _diff));
                case nameof(IAriadneBackendClient.GetGitBranchGraphAsync):
                    return Task.FromResult<IReadOnlyList<BranchGraphNode>>(new[]
                    {
                        new BranchGraphNode(
                            "abcdef1234567890",
                            Array.Empty<string>(),
                            new[] { "main" },
                            "测试存档点",
                            DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                            "Ariadne Test"),
                    });
                default:
                    return UnsupportedTask(targetMethod);
            }
        }

        private static object? UnsupportedTask(MethodInfo? method)
        {
            if (method is null || method.ReturnType == typeof(void))
            {
                return null;
            }
            if (method.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (method.ReturnType.IsGenericType
                && method.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = method.ReturnType.GetGenericArguments()[0];
                return typeof(Task)
                    .GetMethods(BindingFlags.Public | BindingFlags.Static)
                    .Single(candidate => candidate.Name == nameof(Task.FromException)
                        && candidate.IsGenericMethodDefinition)
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object[] { new NotSupportedException(method.Name) });
            }
            return method.ReturnType.IsValueType ? Activator.CreateInstance(method.ReturnType) : null;
        }
    }

    // DispatchProxy 要在运行时派生宿主类型，所以**不能 sealed**
    // （否则 ArgumentException: The base type cannot be sealed）。
    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() => Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return false;
            }
            var returnType = targetMethod?.ReturnType;
            if (returnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (returnType is not null
                && returnType.IsGenericType
                && returnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = returnType.GetGenericArguments()[0];
                return typeof(Task)
                    .GetMethods(BindingFlags.Public | BindingFlags.Static)
                    .Single(candidate => candidate.Name == nameof(Task.FromException)
                        && candidate.IsGenericMethodDefinition)
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object[] { new NotSupportedException(targetMethod?.Name ?? "?") });
            }
            return returnType is not null && returnType.IsValueType
                ? Activator.CreateInstance(returnType)
                : null;
        }
    }
}
