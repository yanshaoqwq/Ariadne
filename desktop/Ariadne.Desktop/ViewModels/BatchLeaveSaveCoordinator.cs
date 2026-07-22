using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// Multi-page leave save: prepare all, persist an operation journal, then commit in order.
/// The journal supports crash reconciliation without persisting mutable or secret payloads.
/// </summary>
public static class BatchLeaveSaveCoordinator
{
    public sealed record PageRequest(
        string PageId,
        string Title,
        Func<Task<bool>> Prepare,
        Func<Task<bool>> Commit,
        Func<Task> Abort,
        Func<string?> ReadPayloadIdentity);

    public sealed class JournalState
    {
        public int Version { get; set; } = 2;
        public string OperationId { get; set; } = Guid.NewGuid().ToString("N");
        public string ProjectIdentity { get; set; } = string.Empty;
        public string Phase { get; set; } = "pending";
        public List<string> PlannedPageIds { get; set; } = new();
        public List<string> PlannedPages { get; set; } = new();
        public Dictionary<string, string> PayloadIdentities { get; set; } = new(StringComparer.Ordinal);
        public List<string> CommittedPageIds { get; set; } = new();
        public List<string> CommittedPages { get; set; } = new();
        public string? FailedPageId { get; set; }
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

    public static string DefaultJournalPath => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "Ariadne",
        "leave-save.journal.json");

    public static string CreatePayloadIdentity(string payload) =>
        Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(payload))).ToLowerInvariant();

    public static Task<Result> ExecuteAsync(
        IReadOnlyList<(string Title, Func<Task<bool>> Prepare, Func<Task<bool>> Commit)> pages,
        string? journalPath,
        CancellationToken cancellationToken = default) =>
        ExecuteAsync(
            pages.Select(page => new PageRequest(
                page.Title,
                page.Title,
                page.Prepare,
                page.Commit,
                () => Task.CompletedTask,
                () => null)).ToList(),
            journalPath,
            projectIdentity: string.Empty,
            cancellationToken);

    public static async Task<Result> ExecuteAsync(
        IReadOnlyList<PageRequest> pages,
        string? journalPath,
        string projectIdentity,
        CancellationToken cancellationToken = default)
    {
        if (pages.Count == 0)
        {
            return new Result { AllSucceeded = true };
        }

        var prepared = new List<PageRequest>(pages.Count);
        PageRequest? preparing = null;
        try
        {
            foreach (var page in pages)
            {
                cancellationToken.ThrowIfCancellationRequested();
                preparing = page;
                if (!await page.Prepare().ConfigureAwait(false))
                {
                    await AbortPagesAsync(prepared.Append(page).Reverse()).ConfigureAwait(false);
                    return new Result
                    {
                        AllSucceeded = false,
                        FailedPage = page.Title,
                        FailedError = "prepare_failed",
                    };
                }
                prepared.Add(page);
                preparing = null;
            }
        }
        catch
        {
            await AbortPagesAsync(
                preparing is null
                    ? prepared.AsEnumerable().Reverse()
                    : prepared.Append(preparing).Reverse()).ConfigureAwait(false);
            throw;
        }

        var journal = new JournalState
        {
            ProjectIdentity = projectIdentity,
            Phase = "committing",
            PlannedPageIds = pages.Select(page => page.PageId).ToList(),
            PlannedPages = pages.Select(page => page.Title).ToList(),
            PayloadIdentities = pages
                .Select(page => (page.PageId, Identity: page.ReadPayloadIdentity()))
                .Where(pair => !string.IsNullOrWhiteSpace(pair.Identity))
                .ToDictionary(
                    pair => pair.PageId,
                    pair => pair.Identity!,
                    StringComparer.Ordinal),
        };

        try
        {
            WriteJournal(journalPath, journal);
        }
        catch
        {
            await AbortPagesAsync(prepared.AsEnumerable().Reverse()).ConfigureAwait(false);
            throw;
        }

        for (var index = 0; index < pages.Count; index++)
        {
            var page = pages[index];
            if (cancellationToken.IsCancellationRequested)
            {
                await AbortPagesAsync(pages.Skip(index).Reverse()).ConfigureAwait(false);
                cancellationToken.ThrowIfCancellationRequested();
            }
            try
            {
                if (!await page.Commit().ConfigureAwait(false))
                {
                    return await FailCommitAsync(
                        pages,
                        index,
                        page,
                        "commit_returned_false",
                        journalPath,
                        journal).ConfigureAwait(false);
                }
            }
            catch (Exception ex)
            {
                return await FailCommitAsync(
                    pages,
                    index,
                    page,
                    ex.Message,
                    journalPath,
                    journal).ConfigureAwait(false);
            }

            journal.CommittedPageIds.Add(page.PageId);
            journal.CommittedPages.Add(page.Title);
            WriteJournal(journalPath, journal);
        }

        journal.Phase = "done";
        WriteJournal(journalPath, journal);
        ClearJournal(journalPath);
        return new Result
        {
            AllSucceeded = true,
            CommittedPages = journal.PlannedPages,
        };
    }

    private static async Task<Result> FailCommitAsync(
        IReadOnlyList<PageRequest> pages,
        int failedIndex,
        PageRequest failed,
        string error,
        string? journalPath,
        JournalState journal)
    {
        journal.Phase = "partial";
        journal.FailedPageId = failed.PageId;
        journal.FailedPage = failed.Title;
        journal.FailedError = error;
        WriteJournal(journalPath, journal);
        await AbortPagesAsync(pages.Skip(failedIndex).Reverse()).ConfigureAwait(false);
        return new Result
        {
            AllSucceeded = false,
            CommittedPages = journal.CommittedPages.ToArray(),
            FailedPage = failed.Title,
            FailedError = error,
            Journal = journal,
        };
    }

    private static async Task AbortPagesAsync(IEnumerable<PageRequest> pages)
    {
        foreach (var page in pages)
        {
            try
            {
                await page.Abort().ConfigureAwait(false);
            }
            catch
            {
                // Abort only clears in-memory prepared state; continue clearing the remaining pages.
            }
        }
    }

    public static JournalState? ReadJournal(string? journalPath)
    {
        if (string.IsNullOrWhiteSpace(journalPath) || !File.Exists(journalPath))
        {
            return null;
        }

        try
        {
            var state = JsonSerializer.Deserialize<JournalState>(File.ReadAllText(journalPath));
            return state is { Version: 2 }
                   && !string.IsNullOrWhiteSpace(state.OperationId)
                   && state.Phase is "committing" or "partial" or "done"
                ? state
                : null;
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

    public static void ClearJournal(string? journalPath)
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
