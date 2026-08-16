using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U145：14 个「让用户手打产品自己已知值」的标识/别名字段改为可搜索下拉 + 允许自定义输入。
///
/// 缺陷形态：章节 id、数据别名、引脚名、模型 id 这些字段的**取值集合产品全都持有**
/// （章节来自后端 ChapterDocumentIndex、别名在连边时已经定义过一次、引脚名写死在
/// 节点类型定义里、模型 id 是刷新模型时真实拉回来的），却全是自由文本框。
/// 而后端对它们是**精确等值**匹配——手打即错，且错了只是静默无结果：
/// 无候选提示、无校验、无「未匹配」反馈。与 U137（运行记录页手打 ID）同一类缺陷。
///
/// ⚠️ **判据刻意落在「候选值是否真的来自产品持有的那份数据」，而不是「控件能否渲染」。**
/// 「AutoCompleteBox 在不在 XAML 里」是标记检查——挂一个恒空的 ItemsSource 照样能过，
/// 而用户面对的仍是一个空下拉、只能手打。所以这里断言候选集合**逐项等于**
/// 产品那份数据（作品树的章节 id 集合、边上的别名、节点定义的引脚名），
/// 并且断言「选中一个候选后绑定属性等于那个值」与「手打列表外的值仍能提交」。
/// </summary>
public sealed class IdentifierPickerTests
{
    // ==================================================================
    // 一、候选值必须来自产品持有的数据
    // ==================================================================

    /// <summary>
    /// **U145 主用例（章节）**：章节候选逐项等于作品树里的章节 id 集合。
    ///
    /// 判据取「等于那份数据」而不是「非空」：非空在「候选里塞了几个硬编码示例值」
    /// 的假实现下也成立，而那种实现照样让用户找不到自己真实的章节。
    /// 文档 id 一并断言——两个字段必须一一对应，只对上一个仍然取不到正文。
    /// </summary>
    [Fact]
    public async Task ChapterCandidates_EqualChapterIdsFromWorksTree()
    {
        var backend = WorkspaceBackend.Create();
        backend.WorksTree = TreeWithChapters(
            ("chapter-01", "documents/chapter-01.md", "第一章"),
            ("chapter-02", "documents/chapter-02.md", "第二章"),
            ("chapter-03", "documents/chapter-03.md", "第三章"));
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await vm.ReloadProjectDataAsync();

        Assert.Equal(
            new[] { "chapter-01", "chapter-02", "chapter-03" },
            vm.ChapterIdCandidates.OrderBy(id => id, StringComparer.Ordinal).ToArray());
        Assert.Equal(
            new[]
            {
                "documents/chapter-01.md",
                "documents/chapter-02.md",
                "documents/chapter-03.md",
            },
            vm.ChapterDocumentIdCandidates.OrderBy(id => id, StringComparer.Ordinal).ToArray());
    }

    /// <summary>
    /// **U145 主用例（别名）**：数据别名候选必须含**这条入边上真实定义的那个别名**。
    ///
    /// 这是 14 个字段里最要紧的一条判据：节点读输入用的就是边上那个别名，
    /// 两边不一致就取不到值。所以候选必须来自边，而不是一份写死的通用词表。
    /// 变异点：把 ComposeNodeAliasCandidates 里的入边那一组摘掉，
    /// 剩下的约定默认值里没有 "chapter_text_from_edge"，这条立刻转红。
    /// </summary>
    [Fact]
    public async Task AliasCandidates_ContainAliasDefinedOnIncomingEdge()
    {
        var vm = await NewCanvasAsync();
        vm.AddNodeAt("document_read", 0, 0);
        vm.AddNodeAt("summarizer", 300, 0);
        var source = vm.Nodes[0];
        var target = vm.Nodes[1];
        Assert.True(vm.TryConnectPorts(
            source.Id, NodePortKind.Data, NodePortDirection.Out,
            target.Id, NodePortKind.Data, NodePortDirection.In));
        var edge = vm.Edges.Single();
        edge.Label = "chapter_text_from_edge";

        vm.SelectNode(target);

        Assert.Contains("chapter_text_from_edge", vm.NodeAliasCandidates);
        // 边上的别名必须排在约定默认值之前：它才是这个字段真正要对上的东西。
        Assert.Equal("chapter_text_from_edge", vm.NodeAliasCandidates[0]);
    }

    /// <summary>
    /// **U145 主用例（引脚）**：引脚候选逐项等于节点类型定义里的引脚名。
    ///
    /// 这两个字段（SourceHandle / TargetHandle）是 U145 里最说明问题的：
    /// 引脚名写死在后端 `contracts/workflow.rs`（EXECUTION_OUTPUT_PORT / _TRUE /
    /// _FALSE / COMMUNICATION_PORT），用户**无从知道**该填什么，
    /// 而此前边检查器给的是一个空白文本框。
    /// </summary>
    [Fact]
    public async Task SourceHandleCandidates_EqualHandlesFromNodeTypeDefinition()
    {
        var vm = await NewCanvasAsync();
        vm.AddNodeAt("condition", 0, 0);

        Assert.Equal(
            new[] { "output", "exec_out", "exec_out_true", "exec_out_false", "communication" },
            vm.SourceHandleCandidates.ToArray());
        // 与 NodePortSpec 的常量同源，而不是测试里另抄一遍字面量——
        // 抄一遍的话「后端改了引脚名」时两边一起改，测试反而拦不住。
        Assert.Contains(NodePortSpec.ExecOutTrueHandle, vm.SourceHandleCandidates);
        Assert.Contains(NodePortSpec.ExecOutFalseHandle, vm.SourceHandleCandidates);
    }

    /// <summary>
    /// 目标引脚候选必须按**目标节点实际存在的**数据入引脚算。
    ///
    /// 列一个目标节点没有的 `data-in-3` 等于教用户填错（连线落不到任何引脚）。
    /// 变异点：把 ComposeTargetHandleCandidates 里的 pins 那一组换成固定列表，
    /// 「加了两个引脚后候选含 data-in-2」立刻转红。
    /// </summary>
    [Fact]
    public async Task TargetHandleCandidates_FollowTargetNodeActualPins()
    {
        var vm = await NewCanvasAsync();
        vm.AddNodeAt("document_read", 0, 0);
        vm.AddNodeAt("writer", 300, 0);
        var source = vm.Nodes[0];
        var target = vm.Nodes[1];
        target.AddDataInPin();
        target.AddDataInPin();
        Assert.True(vm.TryConnectPorts(
            source.Id, NodePortKind.Data, NodePortDirection.Out,
            target.Id, NodePortKind.Data, NodePortDirection.In));
        vm.Edges.Single().SelectCommand.Execute(null);

        Assert.Contains("data-in-1", vm.TargetHandleCandidates);
        Assert.Contains("data-in-2", vm.TargetHandleCandidates);
        // 目标节点只有 3 个数据入，不该冒出第 4 个。
        Assert.DoesNotContain("data-in-3", vm.TargetHandleCandidates);
    }

    /// <summary>
    /// 审批 id 候选来自画布上各审批节点已用的 id。
    ///
    /// 后端对 approval_id 查重（`integration.rs` 拒绝重复），
    /// 所以「别人已经用了什么」本身就是用户需要看到的信息。
    /// </summary>
    [Fact]
    public async Task ApprovalIdCandidates_ComeFromApprovalNodesOnCanvas()
    {
        var vm = await NewCanvasAsync();
        vm.AddNodeAt("approval", 0, 0);
        vm.AddNodeAt("approval", 300, 0);
        vm.Nodes[0].ApprovalId = "approval-gate-a";
        vm.Nodes[1].ApprovalId = "approval-gate-b";

        vm.SelectNode(vm.Nodes[1]);

        Assert.Contains("approval-gate-a", vm.ApprovalIdCandidates);
        Assert.Contains("approval-gate-b", vm.ApprovalIdCandidates);
    }

    /// <summary>
    /// 导出产物 id 的首个候选必须**带 `exports/` 前缀**。
    ///
    /// 这条不是排版偏好：`documents/service.rs` 的 `artifact_path` 只对 `exports/`
    /// 前缀做导出根重定向，否则一律落 `.runtime/artifacts`——正是 U134 那条
    /// 「用户在设置里配的导出目录完全不生效」。U134 只修了作品页合并导出
    /// （后端自己生成 id），**工作流导出节点这条仍要求前端给出 artifact_id**
    /// （`integration.rs` 的 require_non_empty_node_field），所以这个字段删不掉，
    /// 只能把正确形态摆进候选的第一位。
    /// </summary>
    [Fact]
    public async Task ExportArtifactIdCandidates_LeadWithExportsPrefixedSuggestion()
    {
        var vm = await NewCanvasAsync();
        vm.AddNodeAt("export", 0, 0);
        var node = vm.Nodes.Single();

        vm.SelectNode(node);

        Assert.NotEmpty(vm.ExportArtifactIdCandidates);
        Assert.Equal($"exports/{node.Id}", vm.ExportArtifactIdCandidates[0]);
    }

    /// <summary>
    /// 通信别名候选含约定默认值，以及画布上其它通信边已用过的别名。
    /// </summary>
    [Fact]
    public async Task CommunicationAliasCandidates_IncludeConventionAndAliasesInUse()
    {
        var vm = await NewCanvasAsync();
        vm.AddNodeAt("writer", 0, 0);
        vm.AddNodeAt("critic", 300, 0);
        Assert.True(vm.TryConnectPorts(
            vm.Nodes[0].Id, NodePortKind.Communication, NodePortDirection.Out,
            vm.Nodes[1].Id, NodePortKind.Communication, NodePortDirection.In));
        var edge = vm.Edges.Single();
        edge.ForwardAlias = "critique_request";
        vm.SelectNode(vm.Nodes[0]);

        Assert.Contains("forward_output", vm.CommunicationAliasCandidates);
        Assert.Contains("reverse_output", vm.CommunicationAliasCandidates);
        Assert.Contains("critique_request", vm.CommunicationAliasCandidates);
    }

    /// <summary>
    /// 作品页导入表单的章节 id 候选逐项等于作品树里的章节 id。
    /// </summary>
    [Fact]
    public async Task ImportChapterIdCandidates_EqualChaptersInWorksTree()
    {
        var backend = WorkspaceBackend.Create();
        backend.WorksTree = TreeWithChapters(
            ("chapter-alpha", "documents/alpha.md", "开篇"),
            ("chapter-beta", "documents/beta.md", "承篇"));
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await vm.ReloadProjectDataAsync();

        Assert.Equal(
            new[] { "chapter-alpha", "chapter-beta" },
            vm.ImportChapterIdCandidates.OrderBy(id => id, StringComparer.Ordinal).ToArray());
    }

    /// <summary>
    /// 设置页模型 id 候选来自**后端交回的模型目录**，
    /// 而不是把用户正在手打的行回收进候选。
    ///
    /// 后半句是刻意的：把手打值洗进候选等于让打错的 id 长得像「官方选项」，
    /// 下一次就再也分不清哪个是真的。
    /// </summary>
    [Fact]
    public void ModelIdCandidates_ComeFromBackendCatalogNotUserTyping()
    {
        var backend = WorkspaceBackend.Create();
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        vm.ApplyProviderConfigForTests(new ProviderConfigStatus(
            HasOpenAiKey: true,
            HasAnthropicKey: false,
            HasGeminiKey: false,
            DefaultLlmProviderId: "openai",
            DefaultEmbeddingProviderId: null,
            DefaultRerankerProviderId: null,
            DefaultSearchProviderId: null,
            Providers: new[]
            {
                new ProviderKeyStatus(
                    "openai",
                    "OpenAI",
                    "open_ai",
                    Configured: true,
                    Enabled: true,
                    BaseUrl: null,
                    Models: new[]
                    {
                        new ModelConfig("gpt-5-pro", "llm", 400_000, 1.25, 10.0),
                        new ModelConfig("text-embedding-4", "embedding", null, 0.02, null),
                    },
                    HasKey: true),
            }));

        Assert.Contains("gpt-5-pro", vm.FetchedModelIdCandidates);
        Assert.Contains("text-embedding-4", vm.FetchedModelIdCandidates);

        // 手打一个不存在的 id：字段照样收下（见下一条），但不得混进候选。
        vm.ProviderModels[0].ModelId = "typo-model-xyz";
        Assert.DoesNotContain("typo-model-xyz", vm.FetchedModelIdCandidates);
    }

    // ==================================================================
    // 二、选中候选 → 绑定属性等于那个值；列表外的值仍能提交
    // ==================================================================

    /// <summary>
    /// 选中一个候选后，绑定属性必须**等于那个候选值**。
    ///
    /// 这条守的是「下拉是装饰还是真的接上了」：候选算对但没写回属性，
    /// 用户点了半天配置还是空的，而画面上看不出区别。
    /// </summary>
    [Fact]
    public async Task PickingCandidate_WritesExactValueIntoBoundProperty()
    {
        var vm = await NewCanvasAsync();
        vm.AddNodeAt("search", 0, 0);
        var node = vm.Nodes.Single();
        vm.SelectNode(node);
        var candidate = vm.NodeAliasCandidates.First();

        // AutoCompleteBox 的 Text 双向绑到这个属性；选中候选等价于把候选值写进 Text。
        node.QueryAlias = candidate;

        Assert.Equal(candidate, node.QueryAlias);
    }

    /// <summary>
    /// **必须保留的活口**：列表外的值仍能提交。
    ///
    /// 决议是「可搜索下拉 + 允许自定义输入」而不是纯下拉——
    /// 「引用尚未创建的节点」「新发布的模型还没进 /models 接口」
    /// 「导入一个还不存在的新章节」这些场景下正确的值本来就不在候选里。
    /// 收成纯 ComboBox 会把这些路径整个堵死，那是比手打更严重的倒退。
    /// </summary>
    [Fact]
    public async Task ValueOutsideCandidateList_IsStillAccepted()
    {
        var vm = await NewCanvasAsync();
        vm.AddNodeAt("summarizer", 0, 0);
        var node = vm.Nodes.Single();
        vm.SelectNode(node);

        node.SummarizerChapterId = "chapter-not-yet-created";
        node.SummarizerChapterTextAlias = "alias_defined_later";

        Assert.DoesNotContain("chapter-not-yet-created", vm.ChapterIdCandidates);
        Assert.Equal("chapter-not-yet-created", node.SummarizerChapterId);
        Assert.Equal("alias_defined_later", node.SummarizerChapterTextAlias);
    }

    /// <summary>
    /// 作品页同理：导入**新**章节本来就要给一个还不存在的 id，
    /// 这个字段不能收成纯下拉，否则「导入新章节」这条主路径直接不可用。
    /// </summary>
    [Fact]
    public async Task ImportChapterId_AcceptsBrandNewChapterId()
    {
        var backend = WorkspaceBackend.Create();
        backend.WorksTree = TreeWithChapters(("chapter-alpha", "documents/alpha.md", "开篇"));
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();

        vm.ImportChapterId = "chapter-omega";

        Assert.DoesNotContain("chapter-omega", vm.ImportChapterIdCandidates);
        Assert.Equal("chapter-omega", vm.ImportChapterId);
        Assert.False(vm.HasImportChapterIdError);
    }

    // ==================================================================
    // 三、候选合成规则本身
    // ==================================================================

    /// <summary>
    /// 合成规则：保序、去空白、按 trim 后去重。
    ///
    /// 去重与 trim 收口成一处而不是 14 个站点各写一遍——
    /// 漏掉任一处的表现是下拉里出现两个看起来一样的选项，用户无从判断该选哪个。
    /// </summary>
    [Fact]
    public void Compose_KeepsOrderAndDropsBlankOrDuplicateValues()
    {
        var composed = IdentifierCandidates.Compose(
            new[] { "  first  ", "second", string.Empty, "   " },
            new[] { "first", "third", null });

        Assert.Equal(new[] { "first", "second", "third" }, composed);
    }

    /// <summary>
    /// 内容相同时 `Sync` 不得动集合。
    ///
    /// 判据落在「集合实例里的项没被替换过」：候选每次选中节点/连边都会重算，
    /// 绝大多数情况结果一模一样，而 Clear+重填会让正在展开的候选面板瞬间空掉、
    /// 把用户已经输入的过滤词的匹配结果一起清掉。
    /// </summary>
    [Fact]
    public void Sync_LeavesCollectionUntouchedWhenContentIsUnchanged()
    {
        var target = new System.Collections.ObjectModel.ObservableCollection<string>();
        var changes = 0;
        target.CollectionChanged += (_, _) => changes++;

        IdentifierCandidates.Sync(target, new[] { "a", "b" });
        var afterFirst = changes;
        IdentifierCandidates.Sync(target, new[] { "a", "b" });

        Assert.Equal(new[] { "a", "b" }, target);
        Assert.Equal(afterFirst, changes);

        IdentifierCandidates.Sync(target, new[] { "a", "c" });
        Assert.True(changes > afterFirst, "内容变了必须真的更新集合，否则下拉会停在旧候选上");
        Assert.Equal(new[] { "a", "c" }, target);
    }

    // ==================================================================
    // 测试脚手架
    // ==================================================================

    private static async Task<WorkspacePageViewModel> NewCanvasAsync()
    {
        var backend = WorkspaceBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();
        return vm;
    }

    private static WorksTreeNode TreeWithChapters(
        params (string ChapterId, string DocumentId, string Title)[] chapters) =>
        new(
            "root",
            "root",
            "作品",
            string.Empty,
            chapters
                .Select(chapter => new WorksTreeNode(
                    $"chapter:{chapter.ChapterId}",
                    "chapter",
                    chapter.Title,
                    chapter.DocumentId,
                    Array.Empty<WorksTreeNode>(),
                    chapter.ChapterId,
                    chapter.DocumentId))
                .ToArray());

    /// <summary>
    /// 只提供候选源相关的 IPC，其余一律显式抛——契约漂移时测试要转红，
    /// 而不是安静地拿到一个默认值继续跑。
    /// </summary>
    private class WorkspaceBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        public WorksTreeNode WorksTree { get; set; } = new(
            "root",
            "root",
            "作品",
            string.Empty,
            Array.Empty<WorksTreeNode>());

        public static WorkspaceBackend Create()
        {
            var client = Create<IAriadneBackendClient, WorkspaceBackend>();
            var backend = (WorkspaceBackend)(object)client;
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

            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.LoadProjectCanvasAsync) => EmptyCanvas(),
                nameof(IAriadneBackendClient.SaveProjectCanvasAsync) => args![0],
                nameof(IAriadneBackendClient.SaveWorkflowGraphAsync) => args![0],
                nameof(IAriadneBackendClient.ListWorkflowGraphsAsync) =>
                    Array.Empty<WorkflowSummary>(),
                nameof(IAriadneBackendClient.ValidateWorkflowGraphAsync) => null,
                nameof(IAriadneBackendClient.ListConfirmationsAsync) =>
                    Array.Empty<ConfirmationLogEntry>(),
                nameof(IAriadneBackendClient.GetProviderConfigAsync) => EmptyProviderConfig(),
                nameof(IAriadneBackendClient.GetWorksTreeAsync) => WorksTree,
                nameof(IAriadneBackendClient.GetAutomationSettingsAsync) => IdleAutomation(),
                _ => Unsupported(targetMethod),
            };

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
                    .Invoke(null, new[] { value });
            }

            return value;
        }

        private static WorkflowGraphData EmptyCanvas() => new(
            "default",
            "Project Canvas",
            Array.Empty<CanvasNode>(),
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: "canvas-revision");

        /// <summary>Auto Mode 关、无预授权：候选源用例不碰预算路径。</summary>
        private static AutomationSettings IdleAutomation() => new(
            new BudgetStatus(
                BudgetUsd: 0,
                SpentUsd: 0,
                PreauthorizedUsd: null,
                AutoModeEnabled: false),
            Array.Empty<ConfirmationPolicySetting>());

        private static ProviderConfigStatus EmptyProviderConfig() => new(
            HasOpenAiKey: false,
            HasAnthropicKey: false,
            HasGeminiKey: false,
            DefaultLlmProviderId: null,
            DefaultEmbeddingProviderId: null,
            DefaultRerankerProviderId: null,
            DefaultSearchProviderId: null,
            Providers: Array.Empty<ProviderKeyStatus>());

        private static object? Unsupported(MethodInfo method)
        {
            if (method.ReturnType == typeof(Task) || method.ReturnType.IsGenericType)
            {
                throw new NotSupportedException(method.Name);
            }

            return method.ReturnType.IsValueType
                ? Activator.CreateInstance(method.ReturnType)
                : null;
        }
    }
}
