using System.Globalization;
using System.Text.Json;

namespace Ariadne.Desktop.Localization;

/// 支持多语言的显示名服务。
/// - 中文 display_name.json 为基底（永远加载）。
/// - display_name.{lang}.json 为覆盖层：匹配 key 覆盖中文，缺失 key 回退中文。
/// - 首次加载自动按 CultureInfo.CurrentUICulture 选语言；可在运行时调用 SwitchLanguage 切换。
/// - 语言入口按 Resources/display_name.*.json 自动发现；新增覆盖文件即可出现在设置页。
public sealed class DisplayNameService
{
    private readonly IReadOnlyDictionary<string, string> _base;
    private IReadOnlyDictionary<string, string> _overlay;
    private readonly string _resourceDir;
    private readonly HashSet<string> _availableLanguageSet;

    private DisplayNameService(
        IReadOnlyDictionary<string, string> baseNames,
        IReadOnlyDictionary<string, string> overlay,
        string resourceDir,
        IReadOnlyList<string> availableLanguages)
    {
        _base = baseNames;
        _overlay = overlay;
        _resourceDir = resourceDir;
        AvailableLanguages = availableLanguages;
        _availableLanguageSet = new HashSet<string>(availableLanguages, StringComparer.OrdinalIgnoreCase);
    }

    public static DisplayNameService Current { get; private set; } = new(
        new Dictionary<string, string>(),
        new Dictionary<string, string>(),
        string.Empty,
        new[] { "zh" });

    public event EventHandler? LanguageChanged;

    public IReadOnlyList<string> AvailableLanguages { get; }

    /// 当前语言代码。
    public string CurrentLanguage { get; private set; } = "zh";

    public static void Initialize(DisplayNameService service)
    {
        Current = service;
    }

    /// 加载默认服务：从候选路径找到资源目录，按系统语言自动选叠加层。
    public static DisplayNameService LoadDefault()
    {
        var resourceDir = FindResourceDir();
        var baseNames = LoadJson(Path.Combine(resourceDir, "display_name.json"));
        var availableLanguages = DiscoverLanguages(resourceDir);
        var bootstrap = new DisplayNameService(
            baseNames,
            new Dictionary<string, string>(),
            resourceDir,
            availableLanguages);

        var systemLang = bootstrap.NormalizeAvailableLanguage(DetectSystemLanguage());
        var overlay = LoadOverlay(resourceDir, systemLang);

        var service = new DisplayNameService(baseNames, overlay, resourceDir, availableLanguages)
        {
            CurrentLanguage = systemLang,
        };
        return service;
    }

    /// 运行时切换语言（保存后调用此方法）。
    public void SwitchLanguage(string langCode)
    {
        var lang = NormalizeAvailableLanguage(langCode);
        _overlay = LoadOverlay(_resourceDir, lang);
        CurrentLanguage = lang;
        LanguageChanged?.Invoke(this, EventArgs.Empty);
    }

    public string NormalizeAvailableLanguage(string? langCode)
    {
        var lang = NormalizeLanguageCode(langCode);
        if (_availableLanguageSet.Contains(lang))
        {
            return lang;
        }

        var primary = lang.Split('-', StringSplitOptions.RemoveEmptyEntries).FirstOrDefault() ?? string.Empty;
        if (string.Equals(primary, "jp", StringComparison.OrdinalIgnoreCase))
        {
            primary = "ja";
        }
        if (_availableLanguageSet.Contains(primary))
        {
            return primary;
        }

        return "zh";
    }

    public static string NormalizeLanguageCode(string? langCode)
    {
        var lang = (langCode ?? string.Empty).Trim().Replace('_', '-').ToLowerInvariant();
        if (string.IsNullOrEmpty(lang))
        {
            return "zh";
        }
        if (lang == "jp")
        {
            return "ja";
        }
        if (lang.StartsWith("jp-", StringComparison.Ordinal))
        {
            return "ja" + lang[2..];
        }
        return lang;
    }

    /// 查找 key 对应的文案：优先叠加层，缺则回退中文基底，再缺则返回 [key] 以便自查。
    public string Text(string key)
    {
        if (_overlay.TryGetValue(key, out var overlayValue) && !string.IsNullOrEmpty(overlayValue))
        {
            return overlayValue;
        }
        return _base.TryGetValue(key, out var baseValue) ? baseValue : $"[{key}]";
    }

    public string LanguageLabel(string langCode)
    {
        var lang = NormalizeAvailableLanguage(langCode);
        if (TryGetText($"ui.settings.misc.language.{lang}", out var configuredLabel))
        {
            return configuredLabel;
        }

        try
        {
            return CultureInfo.GetCultureInfo(lang).NativeName;
        }
        catch (CultureNotFoundException)
        {
            return lang.ToUpperInvariant();
        }
    }

    public string Format(string key, IReadOnlyDictionary<string, string> variables)
    {
        var value = Text(key);
        foreach (var (name, replacement) in variables)
        {
            value = value.Replace("{" + name + "}", replacement, StringComparison.Ordinal);
        }
        return value;
    }

    private bool TryGetText(string key, out string value)
    {
        if (_overlay.TryGetValue(key, out var overlayValue) && !string.IsNullOrEmpty(overlayValue))
        {
            value = overlayValue;
            return true;
        }
        if (_base.TryGetValue(key, out var baseValue) && !string.IsNullOrEmpty(baseValue))
        {
            value = baseValue;
            return true;
        }

        value = string.Empty;
        return false;
    }

    // ————————————————————————————————————————————————
    // 私有辅助
    // ————————————————————————————————————————————————

    private static string FindResourceDir()
    {
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var root in CandidateResourceRoots())
        {
            foreach (var relativePath in new[] { "Resources", Path.Combine("core", "resources") })
            {
                var dir = Path.GetFullPath(Path.Combine(root, relativePath));
                if (seen.Add(dir) && File.Exists(Path.Combine(dir, "display_name.json")))
                {
                    return dir;
                }
            }
        }

        return string.Empty;
    }

    private static IEnumerable<string> CandidateResourceRoots()
    {
        foreach (var start in new[] { AppContext.BaseDirectory, Environment.CurrentDirectory })
        {
            var directory = new DirectoryInfo(Path.GetFullPath(start));
            for (var depth = 0; directory is not null && depth < 8; depth++)
            {
                yield return directory.FullName;
                directory = directory.Parent;
            }
        }
    }

    private static IReadOnlyDictionary<string, string> LoadJson(string path)
    {
        if (!File.Exists(path))
        {
            return new Dictionary<string, string>();
        }

        try
        {
            using var stream = File.OpenRead(path);
            return JsonSerializer.Deserialize<Dictionary<string, string>>(stream)
                   ?? new Dictionary<string, string>();
        }
        catch
        {
            return new Dictionary<string, string>();
        }
    }

    private static IReadOnlyDictionary<string, string> LoadOverlay(string dir, string lang)
    {
        if (lang == "zh" || string.IsNullOrEmpty(dir))
        {
            return new Dictionary<string, string>();
        }

        var path = Path.Combine(dir, $"display_name.{lang}.json");
        return LoadJson(path);
    }

    private static IReadOnlyList<string> DiscoverLanguages(string dir)
    {
        var codes = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "zh" };
        if (!Directory.Exists(dir))
        {
            return new[] { "zh" };
        }

        foreach (var path in Directory.EnumerateFiles(dir, "display_name.*.json"))
        {
            var fileName = Path.GetFileName(path);
            const string prefix = "display_name.";
            const string suffix = ".json";
            if (!fileName.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)
                || !fileName.EndsWith(suffix, StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            var rawCode = fileName[prefix.Length..^suffix.Length];
            var code = NormalizeLanguageCode(rawCode);
            // v1: only product languages with real entries; EN/JA stubs marked out_of_scope are never product languages (U40).
            if (!string.IsNullOrWhiteSpace(code) && IsProductLanguagePack(path))
            {
                codes.Add(code);
            }
        }

        return new[] { "zh" }
            .Concat(codes
                .Where(code => !string.Equals(code, "zh", StringComparison.OrdinalIgnoreCase))
                .OrderBy(code => code, StringComparer.OrdinalIgnoreCase))
            .ToArray();
    }

    /// <summary>
    /// Product language packs must have real UI keys and must not declare out_of_scope_for_v1 / zh-only stubs.
    /// </summary>
    public static bool IsProductLanguagePack(string path)
    {
        var map = LoadJson(path);
        if (map.TryGetValue("_status", out var status)
            && status.Contains("out_of_scope", StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        if (map.TryGetValue("_release", out var release)
            && release.Contains("zh-only", StringComparison.OrdinalIgnoreCase)
            && !path.EndsWith("display_name.json", StringComparison.OrdinalIgnoreCase)
            && !path.EndsWith("display_name.zh.json", StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        return map.Any(entry =>
            !entry.Key.StartsWith('_')
            && !string.IsNullOrWhiteSpace(entry.Value));
    }

    private static bool HasDisplayNameEntries(string path) => IsProductLanguagePack(path);

    /// 按系统语言返回语言代码，最终是否可用由 NormalizeAvailableLanguage 决定。
    private static string DetectSystemLanguage()
    {
        var culture = CultureInfo.CurrentUICulture;
        var iso = culture.TwoLetterISOLanguageName.ToLowerInvariant();
        return NormalizeLanguageCode(iso);
    }
}
