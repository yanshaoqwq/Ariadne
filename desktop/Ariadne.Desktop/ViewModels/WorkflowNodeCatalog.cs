using System.Text.Json;
using System.Text.Json.Serialization;

namespace Ariadne.Desktop.ViewModels;

public sealed record WorkflowNodeCatalogEntry(
    [property: JsonPropertyName("node_type")] string NodeType,
    [property: JsonPropertyName("aliases")] IReadOnlyList<string> Aliases,
    [property: JsonPropertyName("preset_type")] string PresetType,
    [property: JsonPropertyName("display_name_key")] string DisplayNameKey,
    [property: JsonPropertyName("library_group")] string LibraryGroup,
    [property: JsonPropertyName("config_kind")] string ConfigKind,
    [property: JsonPropertyName("execution_kind")] string ExecutionKind,
    [property: JsonPropertyName("default_budget_usd")] double DefaultBudgetUsd,
    [property: JsonPropertyName("project_search_tool")] string? ProjectSearchTool,
    [property: JsonPropertyName("web_search_tool")] string? WebSearchTool)
{
    public bool HasModelExecution => ExecutionKind is "model" or "summarizer";
}

/// <summary>
/// 从后端同源资源加载产品节点目录。节点库、别名和配置面板不得再各自维护类型列表。
/// </summary>
public static class WorkflowNodeCatalog
{
    private static readonly Lazy<IReadOnlyList<WorkflowNodeCatalogEntry>> Entries = new(Load);
    private static readonly Lazy<IReadOnlyDictionary<string, WorkflowNodeCatalogEntry>> Index =
        new(BuildIndex);

    public static IReadOnlyList<WorkflowNodeCatalogEntry> All => Entries.Value;

    public static IEnumerable<WorkflowNodeCatalogEntry> ForGroup(string group) =>
        All.Where(entry => string.Equals(entry.LibraryGroup, group, StringComparison.Ordinal));

    public static WorkflowNodeCatalogEntry? FindKnown(string? nodeType)
    {
        var type = (nodeType ?? string.Empty).Trim();
        return Index.Value.TryGetValue(type, out var entry) ? entry : null;
    }

    public static WorkflowNodeCatalogEntry Resolve(string? nodeType)
    {
        var type = (nodeType ?? string.Empty).Trim();
        return FindKnown(type) ?? new WorkflowNodeCatalogEntry(
            type,
            Array.Empty<string>(),
            type,
            string.Empty,
            "extension",
            "extension",
            "external",
            0,
            null,
            null);
    }

    private static IReadOnlyDictionary<string, WorkflowNodeCatalogEntry> BuildIndex()
    {
        var index = new Dictionary<string, WorkflowNodeCatalogEntry>(StringComparer.Ordinal);
        foreach (var entry in All)
        {
            AddUnique(index, entry.NodeType, entry);
            foreach (var alias in entry.Aliases)
            {
                AddUnique(index, alias, entry);
            }
        }
        return index;
    }

    private static void AddUnique(
        IDictionary<string, WorkflowNodeCatalogEntry> index,
        string key,
        WorkflowNodeCatalogEntry entry)
    {
        if (string.IsNullOrWhiteSpace(key) || index.ContainsKey(key))
        {
            throw new InvalidDataException($"Duplicate or empty workflow node catalog key: {key}");
        }
        index.Add(key, entry);
    }

    private static IReadOnlyList<WorkflowNodeCatalogEntry> Load()
    {
        var path = CandidatePaths().FirstOrDefault(File.Exists)
                   ?? throw new FileNotFoundException("workflow_node_catalog.json was not shipped");
        using var stream = File.OpenRead(path);
        var entries = JsonSerializer.Deserialize<List<WorkflowNodeCatalogEntry>>(stream)
                      ?? throw new InvalidDataException("workflow node catalog is empty");
        if (entries.Count == 0)
        {
            throw new InvalidDataException("workflow node catalog is empty");
        }
        return entries;
    }

    private static IEnumerable<string> CandidatePaths()
    {
        foreach (var start in new[] { AppContext.BaseDirectory, Environment.CurrentDirectory })
        {
            var directory = new DirectoryInfo(Path.GetFullPath(start));
            for (var depth = 0; directory is not null && depth < 8; depth++, directory = directory.Parent)
            {
                yield return Path.Combine(directory.FullName, "Resources", "workflow_node_catalog.json");
                yield return Path.Combine(directory.FullName, "core", "resources", "workflow_node_catalog.json");
            }
        }
    }
}
