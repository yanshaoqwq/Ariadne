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

    /// <summary>
    /// U201-C：新建节点时写进 <c>PromptTemplate</c> 的**默认提示词占位符**（一行）。
    ///
    /// 过去这里返回的是 `ResolveNodePrompt` 的 300~470 字全文，两个后果：右栏编辑框
    /// 一进节点就被占满（而作者绝大多数时候不改它）；工作流文件里存着一份全文副本，
    /// 官方将来调整默认提示词，已建的节点不会跟着更新。
    ///
    /// 生成侧**只给当前界面语言那一种**写法（「解析宽容、生成唯一」里的「唯一」）；
    /// 后端解析接受三种语言写法的并集，见 `core/src/rag/default_prompt.rs`。
    ///
    /// 缺 key 时返回空串**而不是回落成全文**：`DisplayNameService.Text` 缺 key 会返回
    /// `[key]` 这种自查标记，把它存进工作流文件会让后端解析不出来、节点 fail-loud。
    /// 返回空串则退化成「新节点提示词为空」——作者一眼看得见，也能自己填。
    /// </summary>
    public static string ResolveNodePromptPlaceholder(string? nodeType, Func<string, string> text)
    {
        var type = (nodeType ?? string.Empty).Trim().ToLowerInvariant();
        if (string.IsNullOrEmpty(type) || type is "start" or "llm")
        {
            return string.Empty;
        }

        var literal = text($"ui.prompt.default_placeholder.{type}");
        // `[key]` 是 DisplayNameService 的缺键标记；不能把它当文案用。
        if (string.IsNullOrWhiteSpace(literal)
            || (literal.StartsWith('[') && literal.EndsWith(']')))
        {
            return string.Empty;
        }

        return "{{" + literal + "}}";
    }

    /// <summary>
    /// 某段提示词是否仍是「作者没改过的默认占位符」。
    ///
    /// ⚠️ 判据必须容纳**任意一种语言**的写法，而不只是当前界面语言那一种：
    /// 节点可能是在中文界面建的、现在切到了英文界面（占位符存在工作流文件里，
    /// 不随界面语言改写）。只比当前语言会把「没改过」误判成「改成了别的」，
    /// 于是每次切语言都多出一份需要保存的假改动。
    ///
    /// 传入 <paramref name="allLanguageLiterals"/> 是全部语言包里该 agent 的写法。
    /// </summary>
    public static bool IsUnmodifiedDefaultPlaceholder(
        string? promptTemplate,
        IEnumerable<string> allLanguageLiterals)
    {
        var value = NormalizePlaceholder(promptTemplate);
        if (value.Length == 0)
        {
            return false;
        }

        foreach (var literal in allLanguageLiterals)
        {
            if (string.Equals(value, NormalizePlaceholder("{{" + literal + "}}"), StringComparison.Ordinal))
            {
                return true;
            }
        }

        return false;
    }

    /// <summary>
    /// 归一化占位符写法：去掉全部空白并小写折叠。
    ///
    /// 与后端 `normalize_placeholder_literal` 同一套规则——两边不一致会让
    /// 「前端认为作者改过、后端认为没改过」这类分歧无声出现。
    /// </summary>
    private static string NormalizePlaceholder(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return string.Empty;
        }

        var builder = new System.Text.StringBuilder(value.Length);
        foreach (var ch in value)
        {
            if (!char.IsWhiteSpace(ch))
            {
                builder.Append(char.ToLowerInvariant(ch));
            }
        }

        return builder.ToString();
    }

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

    /// <summary>
    /// 执行页变量摘要句式的生成提示词（<c>workflow.variable_summary</c>）。
    ///
    /// 缺键时返回空串而不是塞一份兜底文案：静默兜底会让「提示词丢了」变成
    /// 「AI 表现变差」这种查不出来的症状，调用方据此禁用入口更诚实。
    /// </summary>
    public static string ResolveWorkflowVariableSummaryPrompt() =>
        Load().TryGetValue("workflow.variable_summary", out var entry)
            ? entry.Prompt ?? string.Empty
            : string.Empty;

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
