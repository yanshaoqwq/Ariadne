using System.Text.Json;

namespace Ariadne.Desktop.Localization;

/// <summary>
/// 从 shipped <c>prompt_list.json</c> 解析节点默认提示词（agent_prompt.*）。
/// </summary>
public static class PromptCatalog
{
    private static IReadOnlyDictionary<string, PromptEntry>? _cache;
    private static readonly object Gate = new();

    public sealed record PromptEntry(string Prompt, string? Describe);

    /// <summary>解析节点类型对应的默认提示词正文；无匹配返回空串。</summary>
    public static string ResolveNodePrompt(string? nodeType)
    {
        var type = (nodeType ?? string.Empty).Trim().ToLowerInvariant();
        if (string.IsNullOrEmpty(type) || type is "start" or "llm")
        {
            return string.Empty;
        }

        var map = Load();
        // 优先 agent_prompt.{type}，其次 node_template.{type}.default 的整段（通常含占位符）
        if (map.TryGetValue($"agent_prompt.{type}", out var agent)
            && !string.IsNullOrWhiteSpace(agent.Prompt))
        {
            return agent.Prompt;
        }

        if (map.TryGetValue($"node_template.{type}.default", out var tmpl)
            && !string.IsNullOrWhiteSpace(tmpl.Prompt))
        {
            return tmpl.Prompt;
        }

        return string.Empty;
    }

    /// <summary>纯函数：在已加载 map 上解析（供单测注入）。</summary>
    public static string ResolveNodePromptFromMap(
        string? nodeType,
        IReadOnlyDictionary<string, PromptEntry> map)
    {
        var type = (nodeType ?? string.Empty).Trim().ToLowerInvariant();
        if (string.IsNullOrEmpty(type) || map.Count == 0)
        {
            return string.Empty;
        }

        if (map.TryGetValue($"agent_prompt.{type}", out var agent)
            && !string.IsNullOrWhiteSpace(agent.Prompt))
        {
            return agent.Prompt;
        }

        if (map.TryGetValue($"node_template.{type}.default", out var tmpl)
            && !string.IsNullOrWhiteSpace(tmpl.Prompt))
        {
            return tmpl.Prompt;
        }

        return string.Empty;
    }

    public static IReadOnlyDictionary<string, PromptEntry> Load()
    {
        lock (Gate)
        {
            if (_cache is not null)
            {
                return _cache;
            }

            _cache = LoadFromDisk() ?? new Dictionary<string, PromptEntry>(StringComparer.Ordinal);
            return _cache;
        }
    }

    /// <summary>测试可重置缓存。</summary>
    public static void ResetCacheForTests()
    {
        lock (Gate)
        {
            _cache = null;
        }
    }

    private static IReadOnlyDictionary<string, PromptEntry>? LoadFromDisk()
    {
        foreach (var path in CandidatePaths())
        {
            if (!File.Exists(path))
            {
                continue;
            }

            try
            {
                using var stream = File.OpenRead(path);
                using var doc = JsonDocument.Parse(stream);
                var map = new Dictionary<string, PromptEntry>(StringComparer.Ordinal);
                foreach (var prop in doc.RootElement.EnumerateObject())
                {
                    if (prop.Value.ValueKind != JsonValueKind.Object)
                    {
                        continue;
                    }

                    var prompt = prop.Value.TryGetProperty("prompt", out var p)
                        ? p.GetString() ?? string.Empty
                        : string.Empty;
                    var describe = prop.Value.TryGetProperty("describe", out var d)
                        ? d.GetString()
                        : null;
                    map[prop.Name] = new PromptEntry(prompt, describe);
                }

                return map;
            }
            catch
            {
                // try next path
            }
        }

        return null;
    }

    private static IEnumerable<string> CandidatePaths()
    {
        var baseDir = AppContext.BaseDirectory;
        yield return Path.Combine(baseDir, "Resources", "prompt_list.json");
        yield return Path.Combine(baseDir, "prompt_list.json");

        // 开发时相对仓库
        var dir = new DirectoryInfo(baseDir);
        for (var i = 0; i < 8 && dir is not null; i++, dir = dir.Parent)
        {
            yield return Path.Combine(dir.FullName, "core", "resources", "prompt_list.json");
            yield return Path.Combine(dir.FullName, "resources", "prompt_list.json");
        }
    }
}
