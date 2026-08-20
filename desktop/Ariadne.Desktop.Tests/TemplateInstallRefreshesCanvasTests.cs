using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U207-D / U198-A：装完模板，画布上什么都没有。
///
/// 后端 `install_template` 早已把模板图并进项目画布并落盘
/// （`merge_workflow_into_project_canvas` + `save_workflow_graph_locked`），
/// 前端却没人通知画布页重载 ⇒ 作者切回画布看到的是空画布 + 空态引导，
/// 会判定「导入失败」再点一次。
///
/// ⚠️ **判据必须是「节点可见」，不能是「不再撞 expected_revision 冲突」**——
/// 后者在「画布永远空着」时也能满足（U207-D 明确要求）。所以本文件第一条用例断言的是
/// `WorkspacePageViewModel.HasNodes` / `Nodes.Count`，也就是空态遮罩
/// （`WorkspacePageView.axaml` 里 `IsVisible="{Binding !HasNodes}"`）会不会消失。
/// </summary>
public sealed class TemplateInstallRefreshesCanvasTests
{
    /// <summary>
    /// 主判据：装完模板后**不重新访问画布页**，画布页手里的节点就应该已经是新的。
    /// 全链路走真实 `MainWindowViewModel` 页面工厂，不注入替身页面——
    /// 被修的那处接线正在页面工厂里，注入替身等于把待验的东西绕开。
    /// </summary>
    [Fact]
    public async Task TemplateInstall_MakesCanvasNodesVisibleWithoutRevisitingCanvasPage()
    {
        var backend = CanvasRefreshBackend.Create();
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await window.PreloadProjectPagesForTestsAsync();
        var canvas = Assert.IsType<WorkspacePageViewModel>(window.GetPageForTests("workspace"));
        var templates = Assert.IsType<TemplateMarketPageViewModel>(window.GetPageForTests("templates"));

        // 装模板之前：画布空 ⇒ 空态引导可见。
        Assert.False(canvas.HasNodes);
        Assert.Empty(canvas.Nodes);

        await templates.EnsureInitialCatalogLoadedAsync();
        var card = Assert.Single(templates.Templates);
        await templates.InstallForTestsAsync(card);

        // 装完之后：作者没有再切页，画布页自己就拿到了并入模板后的图。
        Assert.True(backend.InstallCalled);
        Assert.True(canvas.HasNodes);
        Assert.Equal(
            new[] { "official-novel-starter__outliner", "official-novel-starter__writer" },
            canvas.Nodes.Select(node => node.Id).ToArray());
        Assert.Equal(2, backend.LoadProjectCanvasCalls);
    }

    /// <summary>
    /// 复用 `ReloadCachedProjectPagesAsync` 的前提条件：**发起页不能把自己也重载一遍**。
    ///
    /// 模板页自己的 `ReloadProjectDataAsync` 是「换项目了」语义（清空目录 + 作废在途请求），
    /// 在「刚装完模板」这个时刻触发会一次造成三处倒退：目录列表被清空、
    /// 「已导入模板：X」被抹掉、以及 `_requestGeneration` 被自增导致本次安装的
    /// `FinishRequest` 当成过期请求跳过 ⇒ 整页永久忙碌、所有按钮禁用。
    /// 这三条都是用户看得见的，所以逐条钉住。
    /// </summary>
    [Fact]
    public async Task TemplateInstall_KeepsOwnPageUsable_CatalogStatusAndBusyStateIntact()
    {
        var backend = CanvasRefreshBackend.Create();
        var names = DisplayNameService.LoadDefault();
        var window = new MainWindowViewModel(names, backend.Client);
        await window.PreloadProjectPagesForTestsAsync();
        var templates = Assert.IsType<TemplateMarketPageViewModel>(window.GetPageForTests("templates"));

        await templates.EnsureInitialCatalogLoadedAsync();
        var card = Assert.Single(templates.Templates);
        await templates.InstallForTestsAsync(card);

        Assert.False(templates.IsBusy);
        Assert.True(templates.CanInteract);
        Assert.True(card.InstallCommand.CanExecute(null));
        Assert.Single(templates.Templates);
        Assert.True(templates.HasResults);
        Assert.Equal(
            names.Format(
                "ui.template.imported",
                new Dictionary<string, string> { ["name"] = names.Text("ui.template.builtin.novel_starter.name") }),
            templates.StatusText);
    }

    /// <summary>
    /// 安装会重写磁盘上的项目画布，随后画布页要被重载覆盖 ⇒ 未保存的画布改动会被静默丢弃。
    /// 所以安装前要过未保存改动闸门，闸门说「取消」时**一个后端写请求都不能发出**。
    /// </summary>
    [Fact]
    public async Task TemplateInstall_AbortsBeforeWriting_WhenUnsavedChangesGuardCancels()
    {
        var backend = CanvasRefreshBackend.Create();
        var names = DisplayNameService.LoadDefault();
        var reloadCalls = 0;
        var templates = new TemplateMarketPageViewModel(
            names,
            backend.Client,
            () =>
            {
                reloadCalls++;
                return Task.CompletedTask;
            },
            () => Task.FromResult(false));

        await templates.EnsureInitialCatalogLoadedAsync();
        var card = Assert.Single(templates.Templates);
        await templates.InstallForTestsAsync(card);

        Assert.False(backend.InstallCalled);
        Assert.Equal(0, reloadCalls);
        Assert.Equal(names.Text("ui.common.cancel"), templates.StatusText);
        Assert.False(templates.IsBusy);
    }

    /// <summary>
    /// 无壳独立构造（既有单元测试的用法）时两个回调都可省略，行为保持原样：
    /// 不通知任何人、也不因缺回调而拒绝安装。
    /// </summary>
    [Fact]
    public async Task TemplateInstall_WithoutHostCallbacks_StillInstalls()
    {
        var backend = CanvasRefreshBackend.Create();
        var templates = new TemplateMarketPageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await templates.EnsureInitialCatalogLoadedAsync();
        await templates.InstallForTestsAsync(Assert.Single(templates.Templates));

        Assert.True(backend.InstallCalled);
        Assert.False(templates.IsBusy);
    }

    /// <summary>
    /// 后端替身：安装前项目画布是空的，安装后返回并入模板节点的画布。
    /// 这一点复刻真实后端行为——`install_template` 自己就把模板并进了 `default.json`。
    /// </summary>
    private class CanvasRefreshBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        public bool InstallCalled { get; private set; }

        public int LoadProjectCanvasCalls { get; private set; }

        public static CanvasRefreshBackend Create()
        {
            var client = Create<IAriadneBackendClient, CanvasRefreshBackend>();
            var backend = (CanvasRefreshBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.Name == $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}")
            {
                return true;
            }

            object? value;
            switch (targetMethod.Name)
            {
                case nameof(IAriadneBackendClient.LoadProjectCanvasAsync):
                    LoadProjectCanvasCalls++;
                    value = InstallCalled ? MergedCanvas() : EmptyCanvas();
                    break;
                case nameof(IAriadneBackendClient.InstallTemplateAsync):
                    InstallCalled = true;
                    value = new TemplateInstallReport(
                        "official-novel-starter",
                        "1.0.0",
                        "workflows/official-novel-starter/workflow.json",
                        false,
                        Array.Empty<string>());
                    break;
                case nameof(IAriadneBackendClient.GetTemplateRepositorySettingsAsync):
                    value = new TemplateRepositorySettings("ariadne://official-templates/v1");
                    break;
                case nameof(IAriadneBackendClient.SearchTemplatesAsync):
                    value = new[]
                    {
                        new TemplateSummary(
                            "official-novel-starter",
                            "ui.template.builtin.novel_starter.name",
                            new[] { "ui.template.tag.novel" },
                            false),
                    };
                    break;
                case nameof(IAriadneBackendClient.GetCurrentProjectAsync):
                    value = new CurrentProjectStatus("/projects/demo", "Demo");
                    break;
                case nameof(IAriadneBackendClient.ListConfirmationsAsync):
                    value = Array.Empty<ConfirmationLogEntry>();
                    break;
                case nameof(IAriadneBackendClient.GetProviderConfigAsync):
                    value = EmptyProviderConfig();
                    break;
                case nameof(IAriadneBackendClient.GetWorksTreeAsync):
                    value = EmptyWorksTree();
                    break;
                case nameof(IAriadneBackendClient.GetGitRepositoryStatusAsync):
                    value = EmptyGitStatus();
                    break;
                case nameof(IAriadneBackendClient.GetGitBranchGraphAsync):
                    value = Array.Empty<BranchGraphNode>();
                    break;
                case nameof(IAriadneBackendClient.GetSidebarBadgesAsync):
                    value = new SidebarBadgeCounts(0, 0, 0);
                    break;
                case nameof(IAriadneBackendClient.GetBudgetStatusAsync):
                    value = new BudgetStatus(0, 0, null, false);
                    break;
                case nameof(IAriadneBackendClient.GetAutomationSettingsAsync):
                    value = new AutomationSettings(
                        new BudgetStatus(0, 0, null, false),
                        Array.Empty<ConfirmationPolicySetting>());
                    break;
                default:
                    value = null;
                    break;
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
                    .Invoke(null, new object?[] { value });
            }

            return value;
        }

        private static WorkflowGraphData EmptyCanvas() => new(
            "default",
            "Project Canvas",
            Array.Empty<CanvasNode>(),
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: "rev-before-install");

        private static WorkflowGraphData MergedCanvas() => new(
            "default",
            "Project Canvas",
            new[]
            {
                new CanvasNode(
                    "official-novel-starter__outliner",
                    "llm",
                    "Outliner",
                    new Dictionary<string, object?>(),
                    new CanvasPosition(0, 0)),
                new CanvasNode(
                    "official-novel-starter__writer",
                    "llm",
                    "Writer",
                    new Dictionary<string, object?>(),
                    new CanvasPosition(280, 0)),
            },
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: "rev-after-install");

        private static ProviderConfigStatus EmptyProviderConfig() => new(
            false,
            false,
            false,
            null,
            null,
            null,
            null,
            Array.Empty<ProviderKeyStatus>());

        private static WorksTreeNode EmptyWorksTree() => new(
            "root",
            "root",
            "Root",
            string.Empty,
            Array.Empty<WorksTreeNode>());

        private static GitRepositoryStatus EmptyGitStatus() => new(
            "clean",
            null,
            null,
            false,
            null,
            0,
            string.Empty);
    }
}
