using System.Reflection;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U206-B 回归：跨章知识查询的**前端入口**。
///
/// # 缺陷形态：实现完整 + 有测试 + 生产零调用者
///
/// 后端 `resolve_project_reference` 的 `知识` 前缀早就会扫全部 10 个 `FindScope`，
/// IPC 通、前端边界 `ResolveProjectReferenceAsync` 也实现了 —— 但 ViewModels/Views 里
/// **一个调用点都没有**。全前端唯一发出去的引用是审阅面板硬编码的
/// `@确认项:<id>`（U139④），6 种前缀里 5 种从未被用过。
/// 结果：作者写到第 40 章想问「阿青的性格是在哪一章定下的」，全应用没有任何入口。
///
/// # 判据为什么必须落在出站调用上
///
/// 这条缺陷本身就是「能构造出该调用，但没人调它」。因此
/// 「`ResolveProjectReferenceAsync` 存在且可用」这种断言在**缺陷仍在时照样全绿**
/// —— 它一直都是可用的，这正是缺陷的一半。所以下面每条都断言
/// **伪后端真的收到了那次 resolve_project_reference / project_ai_chat 出站请求**，
/// 并且引用串是后端 `parse_project_reference` 解析得动的形态。
/// </summary>
public sealed class KnowledgeReferenceLookupTests
{
    private const string Term = "阿青";

    [Fact]
    public async Task LookingUpKnowledgeSendsAResolveReferenceCall()
    {
        var (viewModel, backend) = await CreateAsync();
        viewModel.KnowledgeLookup.Term = Term;

        Assert.True(viewModel.KnowledgeLookup.LookupCommand.TryExecute());
        await WaitUntilAsync(() => backend.ResolveReferenceCalls.Count >= 1);

        // 判据落在**真实出站请求**上：缺陷版本里这个调用数永远是 0。
        var reference = Assert.Single(backend.ResolveReferenceCalls);
        Assert.Equal($"@知识:{Term}", reference);
    }

    [Fact]
    public async Task KnowledgeReferenceCarriesThePrefixBackendCanParse()
    {
        var (viewModel, backend) = await CreateAsync();
        viewModel.KnowledgeLookup.Term = $"  {Term}  ";

        Assert.True(viewModel.KnowledgeLookup.LookupCommand.TryExecute());
        await WaitUntilAsync(() => backend.ResolveReferenceCalls.Count >= 1);

        // 前缀不是装饰：顶层 `parse_project_reference` 要求引用里含 ':' 或 '/'，
        // 裸关键词会被判「project reference must contain ':' or '/'」——
        // 用户看到的是一条与「查不到」完全不相干的错误。
        var reference = Assert.Single(backend.ResolveReferenceCalls);
        Assert.StartsWith("@知识:", reference, StringComparison.Ordinal);
        // 输入框里的空白必须去掉：后端 `resolve_knowledge` 对快照源做的是
        // `entry.entity_id == normalized` 精确比较，带空格会静默落到模糊分支或不命中。
        Assert.EndsWith(Term, reference, StringComparison.Ordinal);
        Assert.DoesNotContain(' ', reference);
    }

    [Fact]
    public async Task LookupSurfacesWhereTheSettingWasEstablished()
    {
        var (viewModel, backend) = await CreateAsync();
        var panel = viewModel.KnowledgeLookup;
        panel.Term = Term;

        Assert.True(panel.LookupCommand.TryExecute());
        await WaitUntilAsync(() => backend.ResolveReferenceCalls.Count >= 1);
        await WaitUntilAsync(() => panel.HasResult);

        // 作者问的是「在哪一章定下的」——答案在 payload.source 里，
        // 顶层 summary 只有 snippet。只断言 HasResult 是不够的：
        // 出处没投影出来的话，界面上那句话就永远不出现，而 HasResult 照样为真。
        Assert.Equal("chapters/003.md", panel.ResultSource);
        Assert.Equal("阿青 · 性格", panel.ResultTitle);
        Assert.Contains("外冷内热", panel.ResultText);
    }

    [Fact]
    public async Task AskingAiAboutTheFoundSettingSendsItAsAReference()
    {
        var (viewModel, backend) = await CreateAsync();
        var panel = viewModel.KnowledgeLookup;
        panel.Term = Term;
        Assert.True(panel.LookupCommand.TryExecute());
        await WaitUntilAsync(() => panel.HasResult);

        Assert.True(panel.AskAiCommand.TryExecute());
        await WaitUntilAsync(() => backend.ProjectAiReferences.Count >= 1);

        // 走 references 而不是把 text 拼进 message：知识条目可能带整段分段正文，
        // 内联传大段文本违反引用式数据流，后端展开引用时会自己截断并记账。
        var references = Assert.Single(backend.ProjectAiReferences);
        Assert.NotNull(references);
        Assert.Contains($"@知识:{Term}", references!);
    }

    [Fact]
    public async Task AskAiStaysDisabledUntilSomethingWasActuallyFound()
    {
        var (viewModel, _) = await CreateAsync();
        var panel = viewModel.KnowledgeLookup;

        // 没查过就不能「就这条设定问 AI」：那时没有任何引用串可传，
        // 点了只能发一个注定被后端拒的引用，等于摆一个死按钮。
        Assert.False(panel.AskAiCommand.CanExecute(null));
        panel.Term = Term;
        Assert.False(panel.AskAiCommand.CanExecute(null));
    }

    [Fact]
    public async Task LookupWithoutATermIsNotDispatched()
    {
        var (viewModel, backend) = await CreateAsync();

        // 空关键词发出去只会拿回「project reference id must not be empty」，
        // 那条错误对作者毫无意义。按钮此时应当不可用，而不是发一次注定失败的请求。
        Assert.False(viewModel.KnowledgeLookup.LookupCommand.CanExecute(null));
        Assert.False(viewModel.KnowledgeLookup.LookupCommand.TryExecute());
        Assert.Empty(backend.ResolveReferenceCalls);
    }

    [Fact]
    public async Task AFailedSecondLookupDoesNotLeaveTheFirstResultOnScreen()
    {
        var (viewModel, backend) = await CreateAsync();
        var panel = viewModel.KnowledgeLookup;
        panel.Term = Term;
        Assert.True(panel.LookupCommand.TryExecute());
        await WaitUntilAsync(() => panel.HasResult);

        // 第二次查一个知识库里没有的词。
        backend.FailResolve = true;
        panel.Term = "不存在的人物";
        Assert.True(panel.LookupCommand.TryExecute());
        await WaitUntilAsync(() => backend.ResolveReferenceCalls.Count >= 2);
        await WaitUntilAsync(() => !panel.HasResult);

        // 留着上一次结果最危险：作者会把第 3 章那条出处当成新查这个词的答案。
        Assert.False(panel.HasResult);
        Assert.Equal(string.Empty, panel.ResultSource);
        // 而且此时「问 AI」也必须跟着失效，否则它会拿旧引用去问。
        Assert.False(panel.AskAiCommand.CanExecute(null));
    }

    /// <summary>
    /// 入口必须真的挂在**界面**上。
    ///
    /// 上面几条断言的是「命令一执行就发出正确的 IPC」，但命令挂不挂在 View 上它们
    /// 一概不知 —— VM 接线全对、XAML 里一个绑定都没有的话，作者面前照样没有入口，
    /// 而那几条依然全绿。这正是 U206-B 的形态（能力齐备、入口不存在），
    /// 所以必须补一条「呈现层真的引用了这两个命令」的判据。
    ///
    /// 判据取**源 XAML 文本**而不是实体化后的控件树：这块面板挂在右栏项目 AI 页里，
    /// 要实体化就得起 headless 会话 + 展开右栏 + 切到那一页；本机 3.8G 内存下
    /// 多起会话会被静默 OOM。读源文件在这里是**更强**的判据 —— 它同时证明了
    /// 「绑定写在 XAML 里」，而实体化只能证明「此刻这一条路径下它在」。
    /// </summary>
    [Fact]
    public void TheProjectAiPanelActuallyMountsTheLookupEntry()
    {
        var xaml = File.ReadAllText(Path.Combine(
            ResolveSolutionDir(), "Ariadne.Desktop", "Views", "WorkspacePageView.axaml"));

        // ⚠️ 断言必须带上绑定的**闭合花括号**：写成裸的
        // `Assert.Contains("KnowledgeLookup.LookupCommand", xaml)` 会被前缀吞并 ——
        // 把绑定改成 `LookupCommandXX`（一个不存在的属性，界面上按钮彻底失效）
        // 照样能匹配上，用例全绿。这是变异测试当场抓出来的：
        // U206 报告本身也记过同一个坑（`ui.works.characters` 被
        // `ui.works.characters_count` 假命中）。
        Assert.Contains("KnowledgeLookup.LookupCommand}", xaml);
        Assert.Contains("KnowledgeLookup.Term,", xaml);
        // 出处那一行是作者要的答案本身，不显示它等于查了个空。
        Assert.Contains("KnowledgeLookup.ResultSource}", xaml);
        Assert.Contains("KnowledgeLookup.AskAiCommand}", xaml);
    }

    private static string ResolveSolutionDir()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return dir!;
    }

    private static async Task WaitUntilAsync(Func<bool> condition)
    {
        for (var attempt = 0; attempt < 200; attempt++)
        {
            if (condition())
            {
                return;
            }
            await Task.Delay(10);
        }
        Assert.Fail("等待条件超时");
    }

    private static async Task<(WorkspacePageViewModel ViewModel, LookupBackend Backend)> CreateAsync()
    {
        var backend = LookupBackend.Create();
        var viewModel = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();
        return (viewModel, backend);
    }

    /// <summary>
    /// 伪后端：只伪造 resolve_project_reference / project_ai_chat / 画布三条路径，
    /// 其余调用返回 default（会被 VM 的 catch 写进 StatusText，不影响判据）。
    /// </summary>
    private class LookupBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        /// <summary>每次 resolve_project_reference 的**出站引用串**，主判据落在这里。</summary>
        public List<string> ResolveReferenceCalls { get; } = new();

        /// <summary>每次 project_ai_chat 的出站 references 参数。</summary>
        public List<IReadOnlyList<string>?> ProjectAiReferences { get; } = new();

        /// <summary>置 true 时 resolve 抛后端那种 validation 错误（模拟「查不到」）。</summary>
        public bool FailResolve { get; set; }

        public static LookupBackend Create()
        {
            var client = Create<IAriadneBackendClient, LookupBackend>();
            var backend = (LookupBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }

            object? value;
            switch (targetMethod.Name)
            {
                case nameof(IAriadneBackendClient.ResolveProjectReferenceAsync):
                    ResolveReferenceCalls.Add((string?)args?[0] ?? string.Empty);
                    if (FailResolve)
                    {
                        // 后端查不到时抛的就是 validation 错误，不是返回空结果。
                        return FaultedTask(
                            targetMethod,
                            new InvalidOperationException("knowledge item not found: 阿青"));
                    }
                    value = KnowledgeReference((string?)args?[0] ?? string.Empty);
                    break;
                case nameof(IAriadneBackendClient.ProjectAiChatAsync):
                    ProjectAiReferences.Add(ByName(targetMethod, args, "references") as IReadOnlyList<string>);
                    value = new ProjectAiResponse(
                        "这条设定最早出现在第 3 章。",
                        Array.Empty<ProjectAiChatMessage>(),
                        null,
                        string.Empty);
                    break;
                case nameof(IAriadneBackendClient.LoadProjectCanvasAsync):
                    value = new WorkflowGraphData(
                        "default",
                        "Project Canvas",
                        Array.Empty<CanvasNode>(),
                        Array.Empty<CanvasEdge>(),
                        new Dictionary<string, object?>(),
                        ContentRevision: "canvas-revision");
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
                value ??= resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }

            return value;
        }

        /// <summary>按**形参名**取值：签名新增可选参数时不会静默取错位置。</summary>
        private static object? ByName(MethodInfo method, object?[]? args, string name)
        {
            var parameters = method.GetParameters();
            for (var index = 0; index < parameters.Length; index++)
            {
                if (parameters[index].Name == name)
                {
                    return args?[index];
                }
            }
            return null;
        }

        private static object FaultedTask(MethodInfo method, Exception error)
        {
            var resultType = method.ReturnType.GetGenericArguments()[0];
            return typeof(Task).GetMethod(nameof(Task.FromException), 1, new[] { typeof(Exception) })!
                .MakeGenericMethod(resultType)
                .Invoke(null, new object[] { error })!;
        }

        /// <summary>
        /// 后端 `resolve_knowledge` 的返回形状：出处 / 标题 / 正文都在 payload 里，
        /// 顶层 summary 只是 snippet。payload 用 JsonElement——生产路径反序列化出来的
        /// 就是它，用字典会让「取字段」这一环在测试里比生产里更容易成功。
        /// </summary>
        private static ProjectReference KnowledgeReference(string reference) => new(
            reference,
            "knowledge",
            "character-trait-1",
            "阿青：外冷内热，见血会怕",
            JsonSerializer.Deserialize<JsonElement>(
                """
                {
                  "title": "阿青 · 性格",
                  "source": "chapters/003.md",
                  "spans": [],
                  "text": "阿青外冷内热，第一次见血时手抖了很久。",
                  "metadata": {}
                }
                """));
    }
}
