using System.Globalization;
using System.Text.Json;

namespace Ariadne.Desktop.Localization;

/// 支持多语言的显示名服务。
/// - 中文 display_name.json 为基底（永远加载）。
/// - display_name.{lang}.json 为覆盖层：匹配 key 覆盖中文，缺失 key 回退中文。
/// - 首次加载自动按 CultureInfo.CurrentUICulture 选语言；可在运行时调用 SwitchLanguage 切换。
/// - 当前版本预留接口：zh（中文基底）、en（英语覆盖）、ja（日语覆盖）；
///   翻译文件（display_name.en.json / display_name.ja.json）为空白占位，后期填入翻译。
public sealed class DisplayNameService
{
    // 已知支持的语言代码（和文件名后缀对应）
    public static readonly IReadOnlyList<string> SupportedLanguages = new[] { "zh", "en", "ja" };

    private readonly IReadOnlyDictionary<string, string> _base;
    private IReadOnlyDictionary<string, string> _overlay;
    private readonly string _resourceDir;

    private DisplayNameService(
        IReadOnlyDictionary<string, string> baseNames,
        IReadOnlyDictionary<string, string> overlay,
        string resourceDir)
    {
        _base = baseNames;
        _overlay = overlay;
        _resourceDir = resourceDir;
    }

    public static DisplayNameService Current { get; private set; } = new(
        new Dictionary<string, string>(),
        new Dictionary<string, string>(),
        string.Empty);

    /// 当前语言代码（zh / en / ja）。
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

        var systemLang = DetectSystemLanguage();
        var overlay = LoadOverlay(resourceDir, systemLang);

        var service = new DisplayNameService(baseNames, overlay, resourceDir)
        {
            CurrentLanguage = systemLang,
        };
        return service;
    }

    /// 运行时切换语言（保存后调用此方法）。
    public void SwitchLanguage(string langCode)
    {
        var lang = SupportedLanguages.Contains(langCode) ? langCode : "zh";
        _overlay = LoadOverlay(_resourceDir, lang);
        CurrentLanguage = lang;
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

    public string Format(string key, IReadOnlyDictionary<string, string> variables)
    {
        var value = Text(key);
        foreach (var (name, replacement) in variables)
        {
            value = value.Replace("{" + name + "}", replacement, StringComparison.Ordinal);
        }
        return value;
    }

    // ————————————————————————————————————————————————
    // 私有辅助
    // ————————————————————————————————————————————————

    private static string FindResourceDir()
    {
        var candidates = new[]
        {
            Path.Combine(AppContext.BaseDirectory, "Resources"),
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "core", "resources")),
            Path.GetFullPath(Path.Combine(Environment.CurrentDirectory, "core", "resources")),
        };

        foreach (var dir in candidates)
        {
            if (File.Exists(Path.Combine(dir, "display_name.json")))
            {
                return dir;
            }
        }

        return string.Empty;
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

    /// 按系统语言自动匹配支持的语言代码：zh/en/ja，其余回退 zh。
    private static string DetectSystemLanguage()
    {
        var culture = CultureInfo.CurrentUICulture;
        var iso = culture.TwoLetterISOLanguageName.ToLowerInvariant();
        return iso switch
        {
            "zh" => "zh",
            "en" => "en",
            "ja" => "ja",
            _ => "zh",
        };
    }
}
