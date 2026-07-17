using System.Reflection;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 驱动 Works 真实 SendProjectAi 路径（选区注入 + 假 quick_edit），证明「选中 → 发送 → 只改选区」可用。
/// </summary>
public sealed class WorksProjectAiSelectionUsabilityTests
{
    [Fact]
    public async Task SendProjectAi_WithSelection_CallsQuickEdit_AndOnlyReplacesRange()
    {
        var backend = RecordingBackend.Create();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), (IAriadneBackendClient)(object)backend);
        backend.Host = vm;
        const string original = "开头旧句结尾";
        vm.SeedOpenDocumentForTests("documents/ch1.md", "v1", original);

        vm.RequestEditorSelection = () => new EditorTextSelection(2, 4, "旧句");
        backend.NextQuickEdit = new QuickEditResult("旧句", "新句", "-旧句\n+新句");

        vm.ProjectAiMessage = "改紧凑一点";
        await vm.SendProjectAiAsync().ConfigureAwait(true);

        Assert.True(backend.QuickEditCalled, "selection path must call QuickEdit");
        Assert.False(backend.ProjectAiChatCalled, "with selection must not fall through to free-form chat");
        Assert.NotNull(backend.LastQuickEditRequest);
        Assert.Equal("旧句", backend.LastQuickEditRequest!.SelectedText);
        Assert.Equal("改紧凑一点", backend.LastQuickEditRequest.Instruction);
        Assert.Equal("开头新句结尾", vm.DocumentContent);
        Assert.StartsWith("开头", vm.DocumentContent, StringComparison.Ordinal);
        Assert.EndsWith("结尾", vm.DocumentContent, StringComparison.Ordinal);
        Assert.DoesNotContain("旧句", vm.DocumentContent, StringComparison.Ordinal);
        Assert.True(vm.HasUnsavedChanges, "applied AI edit must mark dirty");
        Assert.True(vm.HasProjectAiBubbles);
        Assert.False(string.IsNullOrWhiteSpace(vm.StatusText));
    }

    [Fact]
    public async Task SendProjectAi_WithoutSelection_UsesProjectAiChat_DoesNotCallQuickEdit()
    {
        var backend = RecordingBackend.Create();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), (IAriadneBackendClient)(object)backend);
        backend.Host = vm;
        vm.SeedOpenDocumentForTests("documents/ch1.md", "v1", "全文内容");
        vm.RequestEditorSelection = () => new EditorTextSelection(0, 0, string.Empty);
        backend.NextChat = new ProjectAiResponse(
            "这是回答",
            new[] { new ProjectAiChatMessage("assistant", "这是回答") },
            null,
            string.Empty);

        vm.ProjectAiMessage = "这篇在写什么？";
        await vm.SendProjectAiAsync().ConfigureAwait(true);

        Assert.True(backend.ProjectAiChatCalled);
        Assert.False(backend.QuickEditCalled);
        Assert.Equal("全文内容", vm.DocumentContent);
    }

    [Fact]
    public async Task SendProjectAi_StaleBufferAfterQuickEdit_DoesNotOverwrite()
    {
        var backend = RecordingBackend.Create();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), (IAriadneBackendClient)(object)backend);
        backend.Host = vm;
        backend.NextQuickEdit = new QuickEditResult("旧句", "新句", "d");
        backend.MutateDocumentBeforeReturn = host => host.DocumentContent = "开头改句结尾";

        vm.SeedOpenDocumentForTests("documents/ch1.md", "v1", "开头旧句结尾");
        vm.RequestEditorSelection = () => new EditorTextSelection(2, 4, "旧句");
        vm.ProjectAiMessage = "改写";
        await vm.SendProjectAiAsync().ConfigureAwait(true);

        Assert.Equal("开头改句结尾", vm.DocumentContent);
        Assert.DoesNotContain("新句", vm.DocumentContent, StringComparison.Ordinal);
    }

    [Fact]
    public void WorksPageView_WiresStickySelectionHandlers()
    {
        var viewCs = File.ReadAllText(ResolveSource("Views", "WorksPageView.axaml.cs"));
        var viewAxaml = File.ReadAllText(ResolveSource("Views", "WorksPageView.axaml"));
        Assert.Contains("RequestEditorSelection = CurrentEditorSelection", viewCs, StringComparison.Ordinal);
        Assert.Contains("_stickySelection", viewCs, StringComparison.Ordinal);
        Assert.Contains("LostFocus=\"OnDocumentEditorLostFocus\"", viewAxaml, StringComparison.Ordinal);
        Assert.Contains("PointerReleased=\"OnDocumentEditorPointerReleased\"", viewAxaml, StringComparison.Ordinal);
        Assert.Contains("KeyUp=\"OnDocumentEditorKeyUp\"", viewAxaml, StringComparison.Ordinal);
        Assert.Contains("CaptureStickySelection", viewCs, StringComparison.Ordinal);
        Assert.DoesNotContain("OnDocumentBlockEditor", viewAxaml, StringComparison.Ordinal);
        Assert.Contains("UpdateSummarySelectionFromEditor", viewCs, StringComparison.Ordinal);
        Assert.Contains("ClearStickyEditorSelection", viewCs, StringComparison.Ordinal);
        Assert.Contains("clearWhenEmpty: true", viewCs, StringComparison.Ordinal);
    }

    [Fact]
    public void StickyPolicy_EmptyWhileFocused_Clears_LostFocusEmpty_Keeps()
    {
        var sticky = new EditorTextSelection(2, 4, "旧句");
        // Intentional deselect (focused PointerReleased with empty caret).
        Assert.Null(EditorStickySelectionPolicy.Update(sticky, 3, 3, "", clearWhenEmpty: true));
        // Focus moved to Project AI: empty sample must not wipe sticky.
        var kept = EditorStickySelectionPolicy.Update(sticky, 3, 3, "", clearWhenEmpty: false);
        Assert.NotNull(kept);
        Assert.Equal(2, kept!.Start);
        Assert.Equal(4, kept.End);
        Assert.Null(EditorStickySelectionPolicy.ClearOnDocumentChange());
    }

    [Fact]
    public async Task SendProjectAi_WithoutSelection_FinalStatusIsNoSelectionHint()
    {
        var backend = RecordingBackend.Create();
        var names = DisplayNameService.LoadDefault();
        var vm = new WorksPageViewModel(names, (IAriadneBackendClient)(object)backend);
        backend.Host = vm;
        vm.SeedOpenDocumentForTests("documents/ch1.md", "v1", "全文内容");
        vm.RequestEditorSelection = () => new EditorTextSelection(0, 0, string.Empty);
        backend.NextChat = new ProjectAiResponse(
            "这是回答",
            new[] { new ProjectAiChatMessage("assistant", "这是回答") },
            null,
            string.Empty);

        vm.ProjectAiMessage = "这篇在写什么？";
        await vm.SendProjectAiAsync().ConfigureAwait(true);

        var expected = names.Text("ui.works.project_ai.no_selection_hint");
        Assert.Equal(expected, vm.StatusText);
        Assert.DoesNotContain(names.Text("ui.common.configured"), vm.StatusText, StringComparison.Ordinal);
        Assert.Equal("全文内容", vm.DocumentContent);
    }

    [Fact]
    public async Task SeedOpenDocument_InvokesClearStickyCallback()
    {
        var backend = RecordingBackend.Create();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), (IAriadneBackendClient)(object)backend);
        var cleared = 0;
        vm.ClearStickyEditorSelection = () => cleared++;
        vm.SeedOpenDocumentForTests("documents/a.md", "v1", "aaa");
        Assert.True(cleared >= 1, "document open must clear sticky selection");
        vm.SeedOpenDocumentForTests("documents/b.md", "v1", "bbb");
        Assert.True(cleared >= 2, "document switch must clear sticky again");
    }

    [Fact]
    public async Task ChapterSummary_LoadsFormalProjection_AndRevealRejectsDirtySource()
    {
        var backend = RecordingBackend.Create();
        var names = DisplayNameService.LoadDefault();
        var vm = new WorksPageViewModel(names, (IAriadneBackendClient)(object)backend);
        backend.Host = vm;
        const string document = "甲😀乙";
        backend.NextSummary = new ChapterSummaryView(
            "chapter-1",
            "章节正式总结",
            new ChapterStageSummaryView("stage-main", "阶段正式总结", new[] { "chapter-1" }),
            new[]
            {
                new StorySegmentView(
                    "chapter-1::seg-1",
                    "1",
                    "chapter-1",
                    "表情所在故事段",
                    new WritingSourceSpan(
                        "documents/chapter-1.md",
                        new TextRange(3, 7),
                        "v1")),
            },
            new[]
            {
                new StoryEventView(
                    "event-1",
                    "角色在表情处作出决定",
                    "ongoing",
                    new[] { "chapter-1::seg-1" },
                    new[] { "chapter-1" }),
            },
            new[]
            {
                new RegisteredChangeView(
                    "change-1",
                    "character_trait",
                    "realized",
                    JsonDocument.Parse("""{"kind":"character_trait","content":{"character":"阿青","to_value":"坚定"}}""").RootElement.Clone(),
                    new[] { "chapter-1::seg-1" }),
            },
            Array.Empty<ForeshadowingView>(),
            new[]
            {
                new ChapterSummaryConfirmationView("confirm-1", "chapter_summary", "approved", "rev-1"),
            });

        vm.SeedOpenDocumentForTests("documents/chapter-1.md", "v1", document);
        var revealed = new List<(int Start, int End)>();
        vm.RequestRevealEditorRange = (start, end) => revealed.Add((start, end));

        await vm.LoadChapterSummaryForTests("chapter-1").ConfigureAwait(true);

        Assert.True(vm.ShowSummaryContent);
        Assert.Equal("章节正式总结", vm.ChapterSummaryText);
        Assert.Equal("stage-main", vm.SummaryStageId);
        Assert.Single(vm.SummarySegments);
        Assert.Single(vm.SummaryEvents);
        Assert.Single(vm.SummaryChanges);
        Assert.Single(vm.SummaryConfirmations);
        Assert.True(vm.SummarySegments[0].IsSourceFresh);

        vm.UpdateSummarySelectionFromEditor(new EditorTextSelection(1, 1, string.Empty));
        Assert.True(vm.HasActiveSummarySegment);
        Assert.True(vm.SummarySegments[0].IsSelected);
        Assert.Contains("chapter-1::seg-1", vm.ActiveSummarySegmentText, StringComparison.Ordinal);
        Assert.Contains("event-1", vm.ActiveSummarySegmentText, StringComparison.Ordinal);

        vm.SummarySegments[0].RevealCommand.Execute(null);
        Assert.Equal((1, 3), Assert.Single(revealed));

        vm.DocumentContent = document + "改";
        Assert.False(vm.SummarySegments[0].IsSourceFresh);
        Assert.False(vm.SummarySegments[0].IsSelected);
        Assert.False(vm.HasActiveSummarySegment);
        vm.SummarySegments[0].RevealCommand.Execute(null);
        Assert.Single(revealed);
        Assert.Equal(names.Text("ui.works.summary.source_unsaved"), vm.StatusText);
    }

    private static string ResolveSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && walk is not null; i++)
        {
            var candidate = Path.Combine(new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }

            walk = walk.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }

    /// <summary>
    /// DispatchProxy 实例本身是 RecordingBackend，可同时当作 IAriadneBackendClient 使用。
    /// </summary>
    private class RecordingBackend : DispatchProxy
    {
        public bool QuickEditCalled { get; private set; }
        public bool ProjectAiChatCalled { get; private set; }
        public QuickEditRequest? LastQuickEditRequest { get; private set; }
        public QuickEditResult NextQuickEdit { get; set; } = new("x", "y", "d");
        public ProjectAiResponse NextChat { get; set; } =
            new("ok", Array.Empty<ProjectAiChatMessage>(), null, "");
        public ChapterSummaryView NextSummary { get; set; } = new(
            "chapter-1",
            null,
            null,
            Array.Empty<StorySegmentView>(),
            Array.Empty<StoryEventView>(),
            Array.Empty<RegisteredChangeView>(),
            Array.Empty<ForeshadowingView>(),
            Array.Empty<ChapterSummaryConfirmationView>());
        public Action<WorksPageViewModel>? MutateDocumentBeforeReturn { get; set; }
        public WorksPageViewModel? Host { get; set; }

        public static RecordingBackend Create()
        {
            var proxy = Create<IAriadneBackendClient, RecordingBackend>();
            return (RecordingBackend)proxy;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            var name = targetMethod.Name;
            if (name == "get_HasProjectRoot")
            {
                return true;
            }

            if (name == nameof(IAriadneBackendClient.QuickEditAsync))
            {
                QuickEditCalled = true;
                LastQuickEditRequest = args is { Length: > 0 } ? args[0] as QuickEditRequest : null;
                if (Host is not null)
                {
                    MutateDocumentBeforeReturn?.Invoke(Host);
                }

                return Task.FromResult(NextQuickEdit);
            }

            if (name == nameof(IAriadneBackendClient.ProjectAiChatAsync))
            {
                ProjectAiChatCalled = true;
                return Task.FromResult(NextChat);
            }

            if (name == nameof(IAriadneBackendClient.GetChapterSummaryViewAsync))
            {
                return Task.FromResult(NextSummary);
            }

            if (targetMethod.ReturnType == typeof(void) || targetMethod.ReturnType == typeof(Task))
            {
                return targetMethod.ReturnType == typeof(Task) ? Task.CompletedTask : null;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var t = targetMethod.ReturnType.GetGenericArguments()[0];
                object? value = null;
                if (t == typeof(IReadOnlyList<RecentProjectEntry>))
                {
                    value = Array.Empty<RecentProjectEntry>();
                }
                else if (t == typeof(WorksTreeNode))
                {
                    value = new WorksTreeNode("root", "folder", "root", "", Array.Empty<WorksTreeNode>());
                }
                else if (t.IsValueType)
                {
                    value = Activator.CreateInstance(t);
                }

                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(t)
                    .Invoke(null, new[] { value });
            }

            return targetMethod.ReturnType.IsValueType
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
