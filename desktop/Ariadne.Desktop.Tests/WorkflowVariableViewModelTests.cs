using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 工作流变量的前端契约：摘要句式的读写往返、脏标记、以及与 core 同口径的
/// 空值判定与渲染。
///
/// 单独成文件：变量是一条独立能力线，混进 NodePinAndImportTests 会让那个
/// 文件同时承担引脚、导入、变量三件事。
/// </summary>
public sealed class WorkflowVariableViewModelTests
{
    private static WorkflowNodeViewModel NewStartNode(Action? markDirty = null)
    {
        return new WorkflowNodeViewModel(
            id: "start-1",
            nodeType: "start",
            label: "起始",
            defaultWorkDir: string.Empty,
            x: 0,
            y: 0,
            runRequested: _ => { },
            clearSelection: () => { },
            markDirty: markDirty ?? (() => { }));
    }

    private static WorkflowVariableGroupViewModel NewGroup(Action? markDirty = null) =>
        new(key => key, (key, _) => key, markDirty);

    /// <summary>
    /// 句式改完必须能写回 start 节点 config。
    ///
    /// 这条钉住的是一个真实缺陷：曾经只做了读路径（CaptureExtra 把未知键当
    /// opaque 保留，所以磁盘上原有句式不丢），但 ToData 不写出该键——
    /// 作者在框里的编辑只活在内存里，切走再回来就没了，而他以为存过了。
    /// </summary>
    [Fact]
    public void EditedSummaryTemplate_RoundTripsThroughNodeData()
    {
        var node = NewStartNode();
        var group = NewGroup();
        group.Load(
            new[] { new WorkflowVariableDeclaration("chapter", "number", 1d, false, false) },
            summaryTemplate: null);
        node.Variables = group;

        group.SummaryTemplate = "写第{{var.chapter}}章";
        var data = node.ToData();

        Assert.True(data.TryGetValue("summary_template", out var stored));
        Assert.Equal("写第{{var.chapter}}章", stored);
    }

    /// <summary>
    /// 清空句式要让该键从 config 里消失，而不是存一个空串。
    ///
    /// 空串会被后端当成「句式就是空的」而渲染出一行空白；缺省语义是
    /// 「没有句式，按声明顺序拼」，只有键不存在才表达得出来。
    /// </summary>
    [Fact]
    public void ClearedSummaryTemplate_RemovesKeyInsteadOfStoringBlank()
    {
        var node = NewStartNode();
        var group = NewGroup();
        group.Load(Array.Empty<WorkflowVariableDeclaration>(), "写第{{var.chapter}}章");
        node.Variables = group;

        group.SummaryTemplate = "   ";
        var data = node.ToData();

        Assert.False(data.ContainsKey("summary_template"));
    }

    /// <summary>编辑句式要标脏画布，否则「改完切走再回来没了」。</summary>
    [Fact]
    public void EditingSummaryTemplate_MarksCanvasDirty()
    {
        var dirtyCount = 0;
        var group = NewGroup(() => dirtyCount++);
        group.Load(Array.Empty<WorkflowVariableDeclaration>(), summaryTemplate: null);

        group.SummaryTemplate = "第{{var.chapter}}章";

        Assert.Equal(1, dirtyCount);
    }

    /// <summary>
    /// Load 期间不得标脏：那是把磁盘值灌进来，不是用户编辑。
    /// 否则一打开画布就显示「有未保存改动」。
    /// </summary>
    [Fact]
    public void LoadingSummaryTemplate_DoesNotMarkDirty()
    {
        var dirtyCount = 0;
        var group = NewGroup(() => dirtyCount++);

        group.Load(
            new[] { new WorkflowVariableDeclaration("chapter", "number", 1d, false, false) },
            "写第{{var.chapter}}章");

        Assert.Equal(0, dirtyCount);
    }

    /// <summary>非起始节点没有变量组，ToData 不应写出该键。</summary>
    [Fact]
    public void NonStartNode_DoesNotEmitSummaryTemplate()
    {
        var node = new WorkflowNodeViewModel(
            id: "llm-1",
            nodeType: "llm",
            label: "写作",
            defaultWorkDir: string.Empty,
            x: 0,
            y: 0,
            runRequested: _ => { },
            clearSelection: () => { },
            markDirty: () => { });

        Assert.False(node.ToData().ContainsKey("summary_template"));
    }

    /// <summary>折叠行渲染：占位符替换成当前取值，字符串不带 JSON 引号。</summary>
    [Fact]
    public void SummaryText_RendersCurrentValues()
    {
        var group = NewGroup();
        group.Load(
            new[]
            {
                new WorkflowVariableDeclaration("chapter", "number", 3d, false, false),
                new WorkflowVariableDeclaration("title", "string", "雪落时", false, false),
            },
            "写第{{var.chapter}}章《{{var.title}}》");

        Assert.Equal("写第3章《雪落时》", group.SummaryText);
    }

    /// <summary>缺省句式退回按声明顺序拼接，且跳过 hidden 变量。</summary>
    [Fact]
    public void SummaryText_FallsBackToDeclarationOrderWithoutHidden()
    {
        var group = NewGroup();
        group.Load(
            new[]
            {
                new WorkflowVariableDeclaration("chapter", "number", 3d, false, false),
                new WorkflowVariableDeclaration("pass", "number", 7d, false, true),
            },
            summaryTemplate: null);

        Assert.Equal("chapter 3", group.SummaryText);
    }

    /// <summary>hidden 变量不进执行页表单，连计数都不给。</summary>
    [Fact]
    public void HiddenVariables_DoNotAppearInForm()
    {
        var group = NewGroup();
        group.Load(
            new[]
            {
                new WorkflowVariableDeclaration("title", "string", "雪落时", false, false),
                new WorkflowVariableDeclaration("pass", "number", 7d, false, true),
            },
            summaryTemplate: null);

        Assert.Single(group.Variables);
        Assert.Equal("title", group.Variables[0].Name);
    }

    /// <summary>
    /// required 变量留空时拦住启动，并指名是哪个变量。
    /// 0 与 false 不算空——第 0 章合法。
    /// </summary>
    [Fact]
    public void BlockingReason_TripsOnBlankRequiredButNotOnZero()
    {
        var group = NewGroup();
        group.Load(
            new[] { new WorkflowVariableDeclaration("title", "string", null, true, false) },
            summaryTemplate: null);

        Assert.NotNull(group.BlockingReason());

        group.Variables[0].Text = "雪落时";
        Assert.Null(group.BlockingReason());

        var numeric = NewGroup();
        numeric.Load(
            new[] { new WorkflowVariableDeclaration("chapter", "number", null, true, false) },
            summaryTemplate: null);
        numeric.Variables[0].Text = "0";

        Assert.Null(numeric.BlockingReason());
    }

    /// <summary>启动载荷按声明类型转换，不是一律当字符串塞过去。</summary>
    [Fact]
    public void BuildStartVariables_ConvertsByDeclaredKind()
    {
        var group = NewGroup();
        group.Load(
            new[]
            {
                new WorkflowVariableDeclaration("chapter", "number", 2d, false, false),
                new WorkflowVariableDeclaration("title", "string", "雪落时", false, false),
                new WorkflowVariableDeclaration("polish", "boolean", true, false, false),
            },
            summaryTemplate: null);

        var payload = group.BuildStartVariables();

        Assert.Equal(2d, Assert.IsType<double>(payload["chapter"]));
        Assert.Equal("雪落时", Assert.IsType<string>(payload["title"]));
        Assert.True(Assert.IsType<bool>(payload["polish"]));
    }

    // ── 句式的 AI 生成 ──────────────────────────────────────────────

    /// <summary>
    /// 生成提示词必须来自 prompt_list.json，不能硬编码在 C# 里。
    ///
    /// 这条同时钉住资源键存在：键改名或漏发布时，生成入口会静默失效，
    /// 而那种失效在 UI 上只表现为「AI 效果变差」，极难回溯。
    /// </summary>
    [Fact]
    public void GenerateSummaryPrompt_ComesFromPromptList()
    {
        var template = Ariadne.Desktop.Localization.PromptCatalog
            .ResolveWorkflowVariableSummaryPrompt();

        Assert.False(string.IsNullOrWhiteSpace(template));
        Assert.Contains("{{variables}}", template, StringComparison.Ordinal);
        Assert.Contains("{{current}}", template, StringComparison.Ordinal);
    }

    /// <summary>提示词的两个槽要被真实内容填上，且带上当前取值供 AI 参考。</summary>
    [Fact]
    public void GenerateSummaryPrompt_FillsVariablesAndCurrentSlots()
    {
        var group = NewGroup();
        group.Load(
            new[] { new WorkflowVariableDeclaration("chapter", "number", 3d, false, false) },
            "写第{{var.chapter}}章");

        var prompt = group.BuildGenerateSummaryPrompt();

        Assert.DoesNotContain("{{variables}}", prompt, StringComparison.Ordinal);
        Assert.DoesNotContain("{{current}}", prompt, StringComparison.Ordinal);
        Assert.Contains("chapter", prompt, StringComparison.Ordinal);
        Assert.Contains("写第{{var.chapter}}章", prompt, StringComparison.Ordinal);
    }

    /// <summary>
    /// 未注入生成通道时命令不可用 —— 点了没反应比禁用更糟。
    /// </summary>
    [Fact]
    public void GenerateSummaryCommand_DisabledWithoutChannel()
    {
        var group = NewGroup();

        Assert.False(group.GenerateSummaryCommand.CanExecute(null));

        group.RequestGenerateSummary = () => Task.CompletedTask;
        Assert.True(group.GenerateSummaryCommand.CanExecute(null));
    }

    /// <summary>生成中禁用，避免连点发出多次请求；结束后恢复。</summary>
    [Fact]
    public async Task GenerateSummaryCommand_DisabledWhileRunning()
    {
        var gate = new TaskCompletionSource();
        var group = NewGroup();
        group.RequestGenerateSummary = () => gate.Task;

        group.GenerateSummaryCommand.Execute(null);

        Assert.True(group.IsGeneratingSummary);
        Assert.False(group.GenerateSummaryCommand.CanExecute(null));

        gate.SetResult();
        await gate.Task;
        // 让 finally 里的复位跑完
        await Task.Yield();

        Assert.False(group.IsGeneratingSummary);
    }

    /// <summary>
    /// 生成失败也要解除进行态，否则一次失败把按钮永久禁用。
    /// </summary>
    [Fact]
    public async Task GenerateSummary_ResetsRunningStateOnFailure()
    {
        var group = NewGroup();
        group.RequestGenerateSummary = () => Task.FromException(new InvalidOperationException("boom"));

        group.GenerateSummaryCommand.Execute(null);
        await Task.Yield();
        await Task.Yield();

        Assert.False(group.IsGeneratingSummary);
    }

    /// <summary>
    /// AI 常无视「只回一句话」而裹上代码块或引号；入口处必须收口。
    /// </summary>
    [Theory]
    [InlineData("写第{{var.chapter}}章", "写第{{var.chapter}}章")]
    [InlineData("```\n写第{{var.chapter}}章\n```", "写第{{var.chapter}}章")]
    [InlineData("\"写第{{var.chapter}}章\"", "写第{{var.chapter}}章")]
    [InlineData("「写第{{var.chapter}}章」", "写第{{var.chapter}}章")]
    [InlineData("\n\n写第{{var.chapter}}章\n这句话概括了…", "写第{{var.chapter}}章")]
    [InlineData("", "")]
    [InlineData("   ", "")]
    public void CleanGeneratedSummary_StripsWrappers(string answer, string expected)
    {
        Assert.Equal(expected, WorkflowVariableRules.CleanGeneratedSummary(answer));
    }
}
