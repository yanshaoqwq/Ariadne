using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class GitTimelineStateTests
{
    [Fact]
    public async Task Timeline_PreservesGraphTimeAuthorHeadMergeAndLaneSemantics()
    {
        var now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        var backend = GitBackend.Create();
        backend.Graph = new[]
        {
            new BranchGraphNode(
                "merge-commit",
                new[] { "main-parent", "side-parent" },
                new[] { "HEAD -> main" },
                "Merge side branch",
                now,
                "Ariadne Test",
                "manual",
                true),
            new BranchGraphNode(
                "main-parent",
                new[] { "root" },
                new[] { "main" },
                "Main work",
                now - 60_000,
                "Main Author"),
            new BranchGraphNode(
                "side-parent",
                new[] { "root" },
                new[] { "feature" },
                "Side work",
                now - 120_000,
                "Side Author"),
        };
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        Assert.Equal(3, viewModel.Commits.Count);
        var merge = viewModel.Commits[0];
        Assert.True(merge.IsHead);
        Assert.True(merge.IsMerge);
        Assert.Equal("Ariadne Test", merge.AuthorText);
        Assert.NotEmpty(merge.TimestampText);
        Assert.DoesNotContain("[ui.git.time", merge.RelativeTimeText, StringComparison.Ordinal);
        Assert.Equal("手动", merge.KindText);
        Assert.Equal(0, merge.LaneIndex);
        Assert.Equal(1, viewModel.Commits[2].LaneIndex);
        Assert.True(viewModel.Commits[2].LaneOffset > 0);
    }

    [Fact]
    public async Task Selection_IsOneTwoWaySourceForListAndDetails()
    {
        var backend = GitBackend.Create();
        backend.Graph = new[]
        {
            Node("first", "First"),
            Node("second", "Second"),
        };
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();

        viewModel.SelectedCommit = viewModel.Commits[1];

        Assert.Equal("second", viewModel.SelectedCommitId);
        Assert.Equal("Second", viewModel.SelectedSummary);

        var xaml = File.ReadAllText(ResolveDesktopSource("Views", "GitPageView.axaml"));
        var codeBehind = File.ReadAllText(ResolveDesktopSource("Views", "GitPageView.axaml.cs"));
        var viewModelSource = File.ReadAllText(ResolveDesktopSource("ViewModels", "GitPageViewModel.cs"));
        Assert.Contains("<ListBox", xaml, StringComparison.Ordinal);
        Assert.Contains("SelectedItem=\"{Binding SelectedCommit, Mode=TwoWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("<VirtualizingStackPanel", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("<Canvas", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("Width=\"780\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("OnCommitPointerPressed", codeBehind, StringComparison.Ordinal);
        Assert.DoesNotContain("IsSelected", viewModelSource, StringComparison.Ordinal);
    }

    [Fact]
    public async Task CheckpointBusyState_RejectsRefreshRestoreAndDuplicateCheckpoint()
    {
        var backend = GitBackend.Create();
        backend.Graph = new[] { Node("first", "First") };
        var checkpointStarted = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var checkpointRelease = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        backend.CreateCheckpointHandler = async (_, cancellationToken) =>
        {
            checkpointStarted.TrySetResult();
            await checkpointRelease.Task.WaitAsync(cancellationToken);
            return new ArchivePoint("checkpoint", "new", "Saved", "manual");
        };
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.CreateCheckpointCommand.TryExecute());
        await checkpointStarted.Task;

        Assert.True(viewModel.IsBusy);
        Assert.True(viewModel.IsCheckpointing);
        Assert.False(viewModel.RefreshCommand.TryExecute());
        Assert.False(viewModel.CreateCheckpointCommand.TryExecute());
        Assert.False(viewModel.RestoreCommand.TryExecute());
        Assert.Equal(1, backend.CreateCheckpointCalls);

        checkpointRelease.TrySetResult();
        await WaitUntilAsync(() => viewModel.OperationState == GitOperationState.Idle);

        Assert.False(viewModel.IsBusy);
        Assert.Equal(1, backend.CreateCheckpointCalls);
        Assert.True(viewModel.RefreshCommand.CanExecute(null));
        Assert.True(viewModel.RestoreCommand.CanExecute(null));
    }

    [Theory]
    [InlineData("degraded", "ui.git.status.degraded", "ui.git.reason.no_commits")]
    [InlineData("not_repository", "ui.git.status.not_repository", "ui.git.reason.not_repository")]
    [InlineData("unexpected_backend_token", "ui.git.status.unavailable", "")]
    public async Task RepositoryHealth_MapsStatusAndReasonThroughLocalization(
        string status,
        string expectedStatusKey,
        string expectedReasonKey)
    {
        var backend = GitBackend.Create();
        backend.RepositoryStatus = new GitRepositoryStatus(
            status,
            null,
            null,
            false,
            "raw backend diagnostic must not be displayed",
            0,
            string.Empty);
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        var names = DisplayNameService.LoadDefault();
        Assert.Equal(names.Text(expectedStatusKey), viewModel.RepositoryStatusText);
        Assert.Equal(
            string.IsNullOrEmpty(expectedReasonKey) ? string.Empty : names.Text(expectedReasonKey),
            viewModel.RepositoryReasonText);
        Assert.DoesNotContain("raw backend diagnostic", viewModel.RepositoryReasonText, StringComparison.Ordinal);
    }

    [Fact]
    public async Task CopyCommitId_WhenClipboardFails_ReportsUserFacingError()
    {
        var backend = GitBackend.Create();
        backend.Graph = new[] { Node("first", "First") };
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();
        viewModel.SelectedCommit = viewModel.Commits[0];
        var error = new IOException("clipboard failed");
        viewModel.RequestCopyText = _ => Task.FromException(error);

        Assert.True(viewModel.CopyIdCommand.TryExecute());
        await WaitUntilAsync(() =>
            viewModel.StatusText == UserFacingError.Format(error, DisplayNameService.LoadDefault()));

        Assert.Equal(
            UserFacingError.Format(error, DisplayNameService.LoadDefault()),
            viewModel.StatusText);
    }

    private static GitPageViewModel NewViewModel(GitBackend backend) =>
        new(DisplayNameService.LoadDefault(), backend.Client);

    private static BranchGraphNode Node(string id, string summary) => new(
        id,
        Array.Empty<string>(),
        Array.Empty<string>(),
        summary,
        DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        "Ariadne Test");

    private static async Task WaitUntilAsync(Func<bool> predicate)
    {
        for (var attempt = 0; attempt < 200 && !predicate(); attempt++)
        {
            await Task.Delay(10);
        }
        Assert.True(predicate());
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var attempt = 0; attempt < 12 && walk is not null; attempt++)
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

    private class GitBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public IReadOnlyList<BranchGraphNode> Graph { get; set; } = Array.Empty<BranchGraphNode>();
        public GitRepositoryStatus RepositoryStatus { get; set; } = new(
            "healthy",
            "main",
            "merge-commit",
            false,
            null,
            0,
            string.Empty);
        public int CreateCheckpointCalls { get; private set; }
        public Func<string, CancellationToken, Task<ArchivePoint>> CreateCheckpointHandler { get; set; } =
            (message, _) => Task.FromResult(new ArchivePoint("checkpoint", "new", message, "manual"));

        public static GitBackend Create()
        {
            var client = Create<IAriadneBackendClient, GitBackend>();
            var backend = (GitBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitRepositoryStatusAsync))
            {
                return Task.FromResult(RepositoryStatus);
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitBranchGraphAsync))
            {
                return Task.FromResult(Graph);
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.CreateCheckpointAsync))
            {
                CreateCheckpointCalls++;
                return CreateCheckpointHandler((string)args![0]!, (CancellationToken)args[1]!);
            }

            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
