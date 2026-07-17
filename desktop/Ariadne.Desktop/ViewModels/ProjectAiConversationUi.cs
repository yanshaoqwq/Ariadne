using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop.ViewModels;

/// <summary>Project AI revision/delta 在两个正式页面上的唯一视觉应用协议。</summary>
internal static class ProjectAiConversationUi
{
    private const int MaxClientHistoryMessages = 64;

    public static long Apply(
        ProjectAiResponse response,
        List<ProjectAiChatMessage> history,
        ObservableCollection<ChatBubbleViewModel> bubbles,
        long? currentRevision)
    {
        var knownRevision = currentRevision ?? 0;
        var revisionAdvanced = response.ConversationRevision > knownRevision;
        var legacyResponse = response.NewMessages is null && response.ConversationRevision == 0;
        IReadOnlyList<ProjectAiChatMessage> delta;
        if (!currentRevision.HasValue && response.ConversationSnapshot is { Count: > 0 } snapshot)
        {
            delta = snapshot;
            history.Clear();
            history.AddRange(snapshot);
        }
        else if (legacyResponse)
        {
            delta = LegacyDelta(history, response.ChatHistory);
            history.Clear();
            history.AddRange(response.ChatHistory);
        }
        else
        {
            delta = revisionAdvanced
                ? response.NewMessages ?? Array.Empty<ProjectAiChatMessage>()
                : Array.Empty<ProjectAiChatMessage>();
            history.AddRange(delta);
        }

        if (history.Count > MaxClientHistoryMessages)
        {
            history.RemoveRange(0, history.Count - MaxClientHistoryMessages);
        }
        foreach (var message in delta)
        {
            if (!string.IsNullOrWhiteSpace(message.Content))
            {
                bubbles.Add(new ChatBubbleViewModel(message.Role, message.Content));
            }
        }
        if (bubbles.Count == 0 && !string.IsNullOrWhiteSpace(response.Answer))
        {
            bubbles.Add(new ChatBubbleViewModel("assistant", response.Answer));
        }
        return Math.Max(knownRevision, response.ConversationRevision);
    }

    public static bool ContextWasCompacted(ProjectAiResponse response)
    {
        return response.HistoryTruncated
               || response.MemoryTruncated
               || response.ReferencesTruncated
               || response.SummaryTruncated;
    }

    private static IReadOnlyList<ProjectAiChatMessage> LegacyDelta(
        IReadOnlyList<ProjectAiChatMessage> current,
        IReadOnlyList<ProjectAiChatMessage> response)
    {
        if (response.Count >= current.Count
            && current.Select((message, index) => message == response[index]).All(matches => matches))
        {
            return response.Skip(current.Count).ToArray();
        }
        return response.TakeLast(Math.Min(2, response.Count)).ToArray();
    }
}
