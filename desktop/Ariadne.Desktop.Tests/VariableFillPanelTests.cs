using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 13C 第 5 项：执行页 Ctrl+K「让 AI 填变量值」。
///
/// 这条能力的危险面**不在能不能填上**，而在**填错了看不出来**：
/// AI 给一个未声明的变量名、给 number 一句中文、或作者在等回话期间自己改了取值，
/// 三种情形若被静默吞掉，作者会以为表单里那些值是他要的，然后按运行——
/// 而那次运行会烧真钱写出错误的一章。
///
/// 所以判据一律落在**作者可见的结果**上：改动清单里到底有什么、被拒的名字有没有说出来、
/// 应用/撤销按钮此刻是否真的能按。不断言「Parse 有没有被调用」那类过程事实。
///
/// 纯逻辑 + 纯 ViewModel，不实体化视觉树：面板的守卫全在 VM 里，
/// 挂进 Window 只会引入 headless 的实体化顺序风险（见 ReadingEditingParityTests 注释），
/// 换不来任何额外覆盖。
/// </summary>
public sealed class VariableFillPanelTests
{
    // ── VariableFillProtocol.Parse ────────────────────────────────────

    /// <summary>
    /// 未声明的名字进 RejectedNames，而不是被静默丢掉。
    ///
    /// 静默丢是最糟的形态：作者看到 diff 里只有一条改动，会以为 AI 只想改那一个，
    /// 而实际上 AI 还想改另一个——它写错了名字，而这件事没人告诉他。
    /// </summary>
    [Fact]
    public void Parse_ReportsUndeclaredNamesInsteadOfDroppingThem()
    {
        var variables = Variables(("chapter", "number", "1"));

        var parsed = VariableFillProtocol.Parse("chapter=4\nchapters=9", variables);

        Assert.Equal(new[] { "chapter" }, parsed.Changes.Select(change => change.Name));
        Assert.Equal(new[] { "chapters" }, parsed.RejectedNames);
    }

    /// <summary>
    /// 类型不合法的取值进 RejectedNames，**绝不进改动清单**。
    ///
    /// 「number 收到「第三章」」是这条最真实的形态：模型很自然地把章节写成中文序数。
    /// 一旦它落进表单，作者看到的是一个红框（HasParseError），而运行会被
    /// BlockingReason 拦住——他会以为是自己填错了，而其实是 AI 给的。
    /// 更糟的是若他没注意红框，只会看到「运行按钮灰着，不知道为什么」。
    /// </summary>
    [Fact]
    public void Parse_RejectsValuesThatDoNotMatchTheDeclaredKind()
    {
        var variables = Variables(("chapter", "number", "1"));

        var parsed = VariableFillProtocol.Parse("chapter=第三章", variables);

        Assert.Empty(parsed.Changes);
        Assert.Equal(new[] { "chapter" }, parsed.RejectedNames);
    }

    /// <summary>布尔同理：只认 true/false，不认「是」。</summary>
    [Fact]
    public void Parse_RejectsNonBooleanTextForBooleanVariables()
    {
        var variables = Variables(("polish", "boolean", "false"));

        var parsed = VariableFillProtocol.Parse("polish=是", variables);

        Assert.Empty(parsed.Changes);
        Assert.Equal(new[] { "polish" }, parsed.RejectedNames);
    }

    /// <summary>
    /// 与当前值相同的条目既不算改动、也不算被拒。
    ///
    /// 算改动会让 diff 里出现「- chapter：3」「+ chapter：3」这种没变的对照行，
    /// 而 AI 常把整张清单原样复述一遍——那会淹掉真正改了的那一行。
    /// 算被拒则会冒出一句「以下条目未被采用」，说的却是一件完全正常的事。
    /// </summary>
    [Fact]
    public void Parse_TreatsUnchangedValuesAsNeitherChangeNorRejection()
    {
        var variables = Variables(("chapter", "number", "3"), ("title", "string", "雪落时"));

        var parsed = VariableFillProtocol.Parse("chapter=3\ntitle=惊蛰", variables);

        Assert.Equal(new[] { "title" }, parsed.Changes.Select(change => change.Name));
        Assert.Empty(parsed.RejectedNames);
    }

    /// <summary>
    /// 同名重复只认第一条。
    ///
    /// 后一条无从判断是「修正」还是幻觉（模型没有「我刚说错了」这种标记），
    /// 取先出现的至少可预测；取后出现的会让同一次回复在不同长度下给出不同结果。
    /// </summary>
    [Fact]
    public void Parse_KeepsOnlyTheFirstEntryForARepeatedName()
    {
        var variables = Variables(("chapter", "number", "1"));

        var parsed = VariableFillProtocol.Parse("chapter=4\nchapter=9", variables);

        var change = Assert.Single(parsed.Changes);
        Assert.Equal("4", change.NewText);
    }

    /// <summary>
    /// 代码块围栏、列表符号、引号都要在入口处剥掉。
    ///
    /// 提示词里已经写了「不要解释、不要引号、不要代码块」，但模型仍常裹上——
    /// 与其指望它每次守规矩，不如收口（同 CleanGeneratedSummary 的取舍）。
    /// 不剥的后果不是「少了点美观」：`- chapter` 会被当成变量名而进 RejectedNames，
    /// 作者收到一句「chapter 未被采用（变量不存在）」，而 chapter 明明就在表单里。
    /// </summary>
    [Theory]
    [InlineData("```\nchapter=4\n```")]
    [InlineData("- chapter=4")]
    [InlineData("* chapter = 4")]
    [InlineData("chapter：\"4\"")]
    [InlineData("chapter：「4」")]
    public void Parse_StripsFencesBulletsAndQuotes(string answer)
    {
        var variables = Variables(("chapter", "number", "1"));

        var parsed = VariableFillProtocol.Parse(answer, variables);

        var change = Assert.Single(parsed.Changes);
        Assert.Equal("chapter", change.Name);
        Assert.Equal("4", change.NewText);
        Assert.Empty(parsed.RejectedNames);
    }

    /// <summary>
    /// 空回复给出空结果，而不是抛异常。
    ///
    /// 空回复是真实可达的（模型返回空串、被安全策略截断），
    /// 抛出去会在面板里显示成一句技术错误，而实际情况只是「它没给建议」。
    /// </summary>
    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   \n  ")]
    public void Parse_OnBlankAnswerYieldsNothingAtAll(string? answer)
    {
        var parsed = VariableFillProtocol.Parse(answer, Variables(("chapter", "number", "1")));

        Assert.Empty(parsed.Changes);
        Assert.Empty(parsed.RejectedNames);
    }

    /// <summary>
    /// 旧值要跟着改动一起带出来——diff 的「- 行」全靠它。
    ///
    /// 只带新值的话作者看到的是「chapter 改成 4」，无从判断这是递进还是回退。
    /// </summary>
    [Fact]
    public void Parse_CarriesTheOldValueForEveryChange()
    {
        var variables = Variables(("chapter", "number", "3"));

        var change = Assert.Single(VariableFillProtocol.Parse("chapter=4", variables).Changes);

        Assert.Equal("3", change.OldText);
        Assert.Equal("4", change.NewText);
    }

    // ── VariableFillSession ───────────────────────────────────────────

    /// <summary>
    /// 快照比的是**整张表**，不只是将被改动的那几个。
    ///
    /// 句式渲染出来的那句话由所有变量共同决定：作者在等 AI 回话期间改了 title，
    /// 他核对的那条 diff（只讲 chapter）就不再是他将得到的结果。
    /// </summary>
    [Fact]
    public void MatchesCurrent_FailsWhenAnyUntouchedVariableChanged()
    {
        var snapshot = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["chapter"] = "3",
            ["title"] = "雪落时",
        };
        var session = new VariableFillSession(
            snapshot,
            new[] { new VariableFillChange("chapter", "3", "4") });

        Assert.True(session.MatchesCurrent(new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["chapter"] = "3",
            ["title"] = "雪落时",
        }));
        Assert.False(session.MatchesCurrent(new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["chapter"] = "3",
            ["title"] = "惊蛰",
        }));
    }

    /// <summary>
    /// 变量数量变了（切了工作流 / 改了声明）也算作废。
    ///
    /// 逐键比对若只遍历快照的键，多出来的新变量会被漏掉——那个变量从没被 AI 看过，
    /// 而句式已经把它渲染进去了。
    /// </summary>
    [Fact]
    public void MatchesCurrent_FailsWhenTheVariableSetItselfChanged()
    {
        var session = new VariableFillSession(
            new Dictionary<string, string>(StringComparer.Ordinal) { ["chapter"] = "3" },
            new[] { new VariableFillChange("chapter", "3", "4") });

        Assert.False(session.MatchesCurrent(new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["chapter"] = "3",
            ["tone"] = "克制",
        }));
    }

    /// <summary>
    /// diff 文本必须与后端 quick_edit 同前缀，好让 QuickEditDiffLineViewModel 原样着色。
    ///
    /// 判据取「解析回来的行是不是真的被判成删除/新增」，而不是断言字符串长相：
    /// 前者才是这条设计要的结果（复用着色），后者改个分隔符就假红。
    /// </summary>
    [Fact]
    public void BuildDiffText_ProducesLinesTheSharedDiffViewClassifies()
    {
        var session = new VariableFillSession(
            new Dictionary<string, string>(StringComparer.Ordinal) { ["chapter"] = "3" },
            new[] { new VariableFillChange("chapter", "3", "4") });

        var lines = session.BuildDiffText(_ => "（空）")
            .Split('\n', StringSplitOptions.RemoveEmptyEntries)
            .Select(line => new QuickEditDiffLineViewModel(line))
            .ToList();

        Assert.Equal(2, lines.Count);
        Assert.Equal(QuickEditDiffLineKind.Removed, lines[0].Kind);
        Assert.Equal("chapter：3", lines[0].Text);
        Assert.Equal(QuickEditDiffLineKind.Added, lines[1].Kind);
        Assert.Equal("chapter：4", lines[1].Text);
    }

    /// <summary>
    /// 空值渲染成占位符：两行都是「chapter：」读起来像渲染坏了，
    /// 作者会怀疑面板有毛病而不是「这个变量本来是空的」。
    /// </summary>
    [Fact]
    public void BuildDiffText_RendersBlankSideAsPlaceholder()
    {
        var session = new VariableFillSession(
            new Dictionary<string, string>(StringComparer.Ordinal) { ["title"] = string.Empty },
            new[] { new VariableFillChange("title", string.Empty, "雪落时") });

        var removed = new QuickEditDiffLineViewModel(
            session.BuildDiffText(_ => "（空）").Split('\n')[0]);

        Assert.Equal("title：（空）", removed.Text);
    }

    // ── VariableFillUndoState ─────────────────────────────────────────

    /// <summary>
    /// 应用后又被手改就不许撤销——否则撤销会连作者刚敲的字一起抹掉。
    ///
    /// 这是「撤销」与「回滚到某个历史点」的区别：本面板只承诺撤掉自己那一次。
    /// </summary>
    [Fact]
    public void CanUndo_RefusesAfterTheAuthorEditedValuesAgain()
    {
        var undo = new VariableFillUndoState(
            new Dictionary<string, string>(StringComparer.Ordinal) { ["chapter"] = "4" },
            new Dictionary<string, string>(StringComparer.Ordinal) { ["chapter"] = "3" });

        Assert.True(undo.CanUndo(new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["chapter"] = "4",
        }));
        Assert.False(undo.CanUndo(new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["chapter"] = "5",
        }));
    }

    // ── VariableFillPanelViewModel ────────────────────────────────────

    /// <summary>
    /// 没有可填目标时 Open 拒绝开窗，并说明原因。
    ///
    /// 开一个空面板等于摆死按钮：作者写完说明按「生成取值」，什么都不会发生。
    /// </summary>
    [Fact]
    public void Open_WithoutTargetStaysClosedAndExplainsWhy()
    {
        var panel = NewPanel(out var reports);

        Assert.False(panel.Open(null));

        Assert.False(panel.IsOpen);
        Assert.Equal(Text("ui.workspace.variable_fill.no_target"), Assert.Single(reports));
    }

    /// <summary>说明框空着时不许生成：空指令发出去只会得到一次白花钱的调用。</summary>
    [Fact]
    public void GenerateCommand_RequiresBothAChannelAndAnInstruction()
    {
        var panel = NewPanel(out _);
        panel.Open(Group(("chapter", "number", "3")));

        Assert.False(panel.GenerateCommand.CanExecute(null));

        panel.RequestFill = _ => Task.FromResult("chapter=4");
        Assert.False(panel.GenerateCommand.CanExecute(null));

        panel.Instruction = "接着写下一章";
        Assert.True(panel.GenerateCommand.CanExecute(null));
    }

    /// <summary>
    /// 生成 → 应用：取值真的落进表单，且能撤回原值。
    ///
    /// 判据取「表单里的 Text 是多少」而非「Changes 里有几条」——
    /// 后者在「解析对了但没写回表单」时照样全绿，而那正是用户唯一在意的一环。
    /// </summary>
    [Fact]
    public async Task ApplyThenUndo_MovesTheFormValueAndPutsItBack()
    {
        var panel = NewPanel(out _);
        var group = Group(("chapter", "number", "3"));
        panel.RequestFill = _ => Task.FromResult("chapter=4");
        panel.Open(group);
        panel.Instruction = "接着写下一章";

        await GenerateAsync(panel);

        Assert.True(panel.ApplyCommand.TryExecute());
        Assert.Equal("4", group.Variables[0].Text);

        Assert.True(panel.UndoCommand.TryExecute());
        Assert.Equal("3", group.Variables[0].Text);
    }

    /// <summary>
    /// 等 AI 回话期间取值变了 ⇒ 整条建议作废，**不留一个能按的应用键**。
    ///
    /// 半套上去会得到「作者没写过、AI 也没提过」的第三种状态：
    /// AI 基于 chapter=3 提议 title 改名，而作者已经把 chapter 改成 7 了。
    /// </summary>
    [Fact]
    public async Task StaleSuggestion_IsDiscardedWhenValuesChangedWhileWaiting()
    {
        var panel = NewPanel(out var reports);
        var group = Group(("chapter", "number", "3"));
        // 在「请求已发出、回复未到」这个窗口里模拟作者手改取值。
        panel.RequestFill = _ =>
        {
            group.Variables[0].Text = "7";
            return Task.FromResult("chapter=4");
        };
        panel.Open(group);
        panel.Instruction = "接着写下一章";

        await GenerateAsync(panel);

        Assert.False(panel.ApplyCommand.CanExecute(null));
        Assert.False(panel.HasDiff);
        Assert.Equal("7", group.Variables[0].Text);
        Assert.Contains(Text("ui.workspace.variable_fill.outdated"), reports);
    }

    /// <summary>
    /// 被拒条目要说出来。静默丢弃会让作者以为 AI 也填了那几个变量。
    /// </summary>
    [Fact]
    public async Task RejectedEntries_AreNamedInTheStatusLine()
    {
        var panel = NewPanel(out var reports);
        panel.RequestFill = _ => Task.FromResult("chapter=第三章\ntitle=雪落时");
        panel.Open(Group(("chapter", "number", "3"), ("title", "string", "旧名")));
        panel.Instruction = "接着写下一章";

        await GenerateAsync(panel);

        // title 改动成立，chapter 因类型被拒——两件事同时发生时报的必须是被拒那句，
        // 否则「已给出取值，请核对」会盖掉唯一提示 chapter 出问题的信息。
        Assert.True(panel.HasDiff);
        Assert.Contains("chapter", reports[^1], StringComparison.Ordinal);
    }

    /// <summary>
    /// 改了说明就作废上一条 diff。
    ///
    /// 留着会让「应用」落到与眼前说明不符的改动上：作者把「下一章」改成
    /// 「回到第一章重写」，而待应用的仍是 chapter=4。
    /// </summary>
    [Fact]
    public async Task EditingTheInstruction_InvalidatesThePendingSuggestion()
    {
        var panel = NewPanel(out _);
        panel.RequestFill = _ => Task.FromResult("chapter=4");
        panel.Open(Group(("chapter", "number", "3")));
        panel.Instruction = "接着写下一章";
        await GenerateAsync(panel);
        Assert.True(panel.HasDiff);

        panel.Instruction = "回到第一章重写";

        Assert.False(panel.HasDiff);
        Assert.False(panel.ApplyCommand.CanExecute(null));
    }

    /// <summary>
    /// 关窗丢弃未应用的建议，但**保留撤销态**。
    ///
    /// 前者：留着会在下次开窗给出一条与当前取值早已不同步的旧 diff，
    /// 而应用又会被守卫拒绝——等于摆一个死按钮（U130 同类）。
    /// 后者：刚应用完随手关窗是常态，此时仍该能撤回那一次。
    ///
    /// 保留撤销态还有一条更硬的理由：**关窗并不回滚已应用的取值**，
    /// 那些值留在执行页的变量表单里（面板只是个浮层，表单不随它消失）。
    /// 关窗若连撤销一起清掉，作者就得到一个「这条路再也退不回去」的改动。
    /// 撤销态的作用域本来就锚在**节点的取值**上而不是面板的开合上——
    /// 这也正是 <c>Open</c> 只在**换了目标节点**时才 ClearUndo 的原因。
    /// </summary>
    [Fact]
    public async Task Close_DropsThePendingDiffButKeepsUndoAvailable()
    {
        var panel = NewPanel(out _);
        var group = Group(("chapter", "number", "3"));
        // 每次回话给一个**新**取值。
        //
        // ⚠️ 这里不能两次都返回同一个 "chapter=4"：第一次应用后表单里已经是 4，
        // 第二次回同样的 4 就与当前值相同，按本文件
        // Parse_TreatsUnchangedValuesAsNeitherChangeNorRejection 钉住的规则
        // 既不算改动也不算被拒 ⇒ 根本不会产生第二条建议，
        // 「有未应用的 diff」这个前提压根没搭起来，用例测不到关窗丢弃那一步。
        var answers = new Queue<string>(new[] { "chapter=4", "chapter=5" });
        panel.RequestFill = _ => Task.FromResult(answers.Dequeue());
        panel.Open(group);
        panel.Instruction = "接着写下一章";
        await GenerateAsync(panel);
        Assert.True(panel.ApplyCommand.TryExecute());
        Assert.Equal("4", group.Variables[0].Text);

        // 再生成一条、不应用，然后关窗。
        panel.Instruction = "再往后一章";
        await GenerateAsync(panel);
        Assert.True(panel.HasDiff);

        Assert.True(panel.CloseCommand.TryExecute());

        Assert.False(panel.IsOpen);
        Assert.False(panel.HasDiff);
        // 关窗没有回滚 4，所以撤销这条路必须还在。
        Assert.Equal("4", group.Variables[0].Text);
        Assert.True(panel.UndoCommand.CanExecute(null));
    }

    /// <summary>
    /// 换目标节点必须清空撤销态：那些取值属于上一个节点，套过来是张冠李戴。
    /// </summary>
    [Fact]
    public async Task SwitchingTarget_ClearsTheUndoStateOfThePreviousNode()
    {
        var panel = NewPanel(out _);
        var first = Group(("chapter", "number", "3"));
        panel.RequestFill = _ => Task.FromResult("chapter=4");
        panel.Open(first);
        panel.Instruction = "接着写下一章";
        await GenerateAsync(panel);
        Assert.True(panel.ApplyCommand.TryExecute());
        Assert.True(panel.UndoCommand.CanExecute(null));

        panel.Open(Group(("chapter", "number", "9")));

        Assert.False(panel.UndoCommand.CanExecute(null));
    }

    /// <summary>
    /// 生成中不许关窗：请求已经发出去了（钱已经花了），关窗只会让结果无处落地。
    /// </summary>
    [Fact]
    public async Task Close_IsBlockedWhileTheRequestIsInFlight()
    {
        var gate = new TaskCompletionSource<string>();
        var panel = NewPanel(out _);
        panel.RequestFill = _ => gate.Task;
        panel.Open(Group(("chapter", "number", "3")));
        panel.Instruction = "接着写下一章";

        panel.GenerateCommand.Execute(null);

        Assert.True(panel.IsGenerating);
        Assert.False(panel.IsCloseEnabled);
        Assert.False(panel.CloseCommand.TryExecute());
        Assert.True(panel.IsOpen);

        gate.SetResult("chapter=4");
        await Drain();
        Assert.False(panel.IsGenerating);
        Assert.True(panel.IsCloseEnabled);
    }

    /// <summary>
    /// 请求失败也要解除生成态，否则一次失败把面板永久卡在「生成中…」。
    /// </summary>
    [Fact]
    public async Task FailedRequest_ReleasesTheGeneratingStateAndReportsTheError()
    {
        var panel = NewPanel(out var reports);
        panel.RequestFill = _ => Task.FromException<string>(new InvalidOperationException("boom"));
        panel.Open(Group(("chapter", "number", "3")));
        panel.Instruction = "接着写下一章";

        await GenerateAsync(panel);

        Assert.False(panel.IsGenerating);
        Assert.True(panel.GenerateCommand.CanExecute(null));
        Assert.Contains("boom", reports[^1], StringComparison.Ordinal);
    }

    /// <summary>
    /// 组装的请求正文里必须带上变量清单与当前值，且回复格式约定要写明。
    ///
    /// 清单是 AI 唯一的信息来源（它看不见执行页）；格式约定与 Parse 咬合，
    /// 少了它模型会用散文回答，Parse 一条也解析不出来——表现为「AI 没提出任何改动」，
    /// 而看起来像模型不听话。
    /// </summary>
    [Fact]
    public async Task OutboundMessage_CarriesTheVariableRosterAndTheReplyContract()
    {
        var panel = NewPanel(out _);
        string? sent = null;
        panel.RequestFill = message =>
        {
            sent = message;
            return Task.FromResult(string.Empty);
        };
        panel.Open(Group(("chapter", "number", "3"), ("title", "string", "")));
        panel.Instruction = "接着写下一章";

        await GenerateAsync(panel);

        Assert.NotNull(sent);
        Assert.Contains("接着写下一章", sent!, StringComparison.Ordinal);
        Assert.Contains("chapter", sent!, StringComparison.Ordinal);
        Assert.Contains("number", sent!, StringComparison.Ordinal);
        // 空值写成占位符而不是留白：留白会让那一行读成「当前值：」，
        // 模型分不清是空还是被截断。
        Assert.Contains("title（string）当前值：（空）", sent!, StringComparison.Ordinal);
        Assert.Contains("变量名=取值", sent!, StringComparison.Ordinal);
    }

    /// <summary>
    /// 生成中改文案而不是只禁用按钮：按钮变灰但字不变，看起来像卡住了。
    /// </summary>
    [Fact]
    public async Task GenerateButtonText_SwitchesToTheInProgressWording()
    {
        var gate = new TaskCompletionSource<string>();
        var panel = NewPanel(out _);
        panel.RequestFill = _ => gate.Task;
        panel.Open(Group(("chapter", "number", "3")));
        panel.Instruction = "接着写下一章";

        Assert.Equal(Text("ui.workspace.variable_fill.generate"), panel.GenerateText);

        panel.GenerateCommand.Execute(null);
        Assert.Equal(Text("ui.workspace.variable_fill.generating"), panel.GenerateText);

        gate.SetResult("chapter=4");
        await Drain();
        Assert.Equal(Text("ui.workspace.variable_fill.generate"), panel.GenerateText);
    }

    /// <summary>
    /// AI 一条改动都没提时说明白，而不是留一个空的对照区。
    /// </summary>
    [Fact]
    public async Task NoChangeAnswer_SaysSoInsteadOfShowingAnEmptyDiff()
    {
        var panel = NewPanel(out var reports);
        panel.RequestFill = _ => Task.FromResult("chapter=3");
        panel.Open(Group(("chapter", "number", "3")));
        panel.Instruction = "保持原样";

        await GenerateAsync(panel);

        Assert.False(panel.HasDiff);
        Assert.Equal(Text("ui.workspace.variable_fill.no_change"), reports[^1]);
    }

    /// <summary>
    /// 面板文案全部来自 display_name.json，缺键会以 `[key]` 形态露出来。
    ///
    /// 这条同时钉住那批键存在：漏发布时界面只是显示成方括号键名，
    /// 不报错、不阻断，没有自然的发现途径。
    /// </summary>
    [Fact]
    public void PanelCopy_ResolvesEveryKeyFromTheDisplayNamePack()
    {
        var panel = NewPanel(out _);

        foreach (var copy in new[]
                 {
                     panel.TitleText, panel.PlaceholderText, panel.DiffText,
                     panel.ApplyText, panel.UndoText, panel.CloseText, panel.GenerateText,
                 })
        {
            Assert.False(string.IsNullOrWhiteSpace(copy));
            Assert.DoesNotContain("[ui.", copy, StringComparison.Ordinal);
        }
    }

    // ── 夹具 ──────────────────────────────────────────────────────────

    private static readonly DisplayNameService Names = DisplayNameService.LoadDefault();

    private static string Text(string key) => Names.Text(key);

    private static VariableFillPanelViewModel NewPanel(out List<string> reports)
    {
        var collected = new List<string>();
        reports = collected;
        return new VariableFillPanelViewModel(
            Names.Text,
            Names.Format,
            collected.Add,
            ex => ex.Message);
    }

    /// <summary>按 (名字, 类型, 初值) 建一组变量行。</summary>
    private static WorkflowVariableGroupViewModel Group(
        params (string Name, string Kind, string Value)[] declarations)
    {
        var group = new WorkflowVariableGroupViewModel(Names.Text, Names.Format);
        group.Load(
            declarations
                .Select(item => new WorkflowVariableDeclaration(
                    item.Name,
                    item.Kind,
                    item.Value,
                    Required: false,
                    Hidden: false))
                .ToArray(),
            summaryTemplate: null);
        return group;
    }

    private static IReadOnlyList<WorkflowVariableViewModel> Variables(
        params (string Name, string Kind, string Value)[] declarations) =>
        Group(declarations).Variables;

    /// <summary>
    /// 跑一次生成并等它落地。
    ///
    /// 命令是 fire-and-forget（`() => _ = GenerateAsync()`，与项目里其它异步命令同形），
    /// 所以只能靠让出调度来等——没有可 await 的句柄。
    /// </summary>
    private static async Task GenerateAsync(VariableFillPanelViewModel panel)
    {
        panel.GenerateCommand.Execute(null);
        await Drain();
    }

    private static async Task Drain()
    {
        for (var attempt = 0; attempt < 20; attempt++)
        {
            await Task.Yield();
        }
    }
}
