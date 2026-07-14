using System.Text.Json;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// Multi-page leave save: prepare all → journal plan → commit each (U65 / 00A).
/// No durable write until every page prepares successfully. Mid-commit failure
/// leaves a recoverable journal of which pages already committed.
/// </summary>
public static class BatchLeaveSaveCoordinator
{
    public sealed record PageResult(string PageTitle, bool Success, string? Error);

    public sealed class JournalState
    {
        public string Phase { get; set; } = "pending";
        public List<string> PlannedPages { get; set; } = new();
        public List<string> CommittedPages { get; set; } = new();
        public string? FailedPage { get; set; }
        public string? FailedError { get; set; }
    }

    public sealed class Result
    {
        public bool AllSucceeded { get; init; }
        public IReadOnlyList<string> CommittedPages { get; init; } = Array.Empty<string>();
        public string? FailedPage { get; init; }
        public string? FailedError { get; init; }
        public JournalState? Journal { get; init; }
    }

    /// <summary>
    /// Pure orchestration over already-selected dirty guards. Uses injected prepare/commit
    /// delegates so unit tests can drive the real control flow without Avalonia.
    /// </summary>
    public static async Task<Result> ExecuteAsync(
        IReadOnlyList<(string Title, Func<Task<bool>> Prepare, Func<Task<bool>> Commit)> pages,
        string? journalPath,
        CancellationToken cancellationToken = default)
    {
        if (pages.Count == 0)
        {
            return new Result { AllSucceeded = true };
        }

        var planned = pages.Select(p => p.Title).ToList();
        foreach (var page in pages)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (!await page.Prepare().ConfigureAwait(false))
            {
                return new Result
                {
                    AllSucceeded = false,
                    FailedPage = page.Title,
                    FailedError = "prepare_failed",
                    CommittedPages = Array.Empty<string>(),
                };
            }
        }

        var journal = new JournalState
        {
            Phase = "committing",
            PlannedPages = planned,
            CommittedPages = new List<string>(),
        };
        WriteJournal(journalPath, journal);

        foreach (var page in pages)
        {
            cancellationToken.ThrowIfCancellationRequested();
            try
            {
                if (!await page.Commit().ConfigureAwait(false))
                {
                    journal.Phase = "partial";
                    journal.FailedPage = page.Title;
                    journal.FailedError = "commit_returned_false";
                    WriteJournal(journalPath, journal);
                    return new Result
                    {
                        AllSucceeded = false,
                        CommittedPages = journal.CommittedPages.ToArray(),
                        FailedPage = page.Title,
                        FailedError = journal.FailedError,
                        Journal = journal,
                    };
                }
            }
            catch (Exception ex)
            {
                journal.Phase = "partial";
                journal.FailedPage = page.Title;
                journal.FailedError = ex.Message;
                WriteJournal(journalPath, journal);
                return new Result
                {
                    AllSucceeded = false,
                    CommittedPages = journal.CommittedPages.ToArray(),
                    FailedPage = page.Title,
                    FailedError = ex.Message,
                    Journal = journal,
                };
            }

            journal.CommittedPages.Add(page.Title);
            WriteJournal(journalPath, journal);
        }

        journal.Phase = "done";
        WriteJournal(journalPath, journal);
        ClearJournal(journalPath);
        return new Result
        {
            AllSucceeded = true,
            CommittedPages = planned,
            Journal = null,
        };
    }

    public static JournalState? ReadJournal(string? journalPath)
    {
        if (string.IsNullOrWhiteSpace(journalPath) || !File.Exists(journalPath))
        {
            return null;
        }

        try
        {
            return JsonSerializer.Deserialize<JournalState>(File.ReadAllText(journalPath));
        }
        catch
        {
            return null;
        }
    }

    private static void WriteJournal(string? journalPath, JournalState state)
    {
        if (string.IsNullOrWhiteSpace(journalPath))
        {
            return;
        }

        var dir = Path.GetDirectoryName(journalPath);
        if (!string.IsNullOrWhiteSpace(dir))
        {
            Directory.CreateDirectory(dir);
        }

        var tmp = journalPath + ".tmp";
        File.WriteAllText(tmp, JsonSerializer.Serialize(state, new JsonSerializerOptions { WriteIndented = true }));
        File.Move(tmp, journalPath, overwrite: true);
    }

    private static void ClearJournal(string? journalPath)
    {
        if (string.IsNullOrWhiteSpace(journalPath))
        {
            return;
        }

        try
        {
            if (File.Exists(journalPath))
            {
                File.Delete(journalPath);
            }
        }
        catch
        {
            // best-effort
        }
    }
}
