using Avalonia.Media;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 颜色值编辑：hex / RGB 通道（供色板点选与次要 hex 共用）。
/// 界面以「色图」点选为主，不暴露 RGB 滑条。
/// </summary>
public sealed class ColorChannelEditor : ViewModelBase
{
    private readonly Action? _changed;
    private bool _suppress;
    private byte _r = 138;
    private byte _g = 143;
    private byte _b = 152;
    private string _hex = "#8A8F98";

    public ColorChannelEditor(Action? changed = null)
    {
        _changed = changed;
    }

    public double R
    {
        get => _r;
        set
        {
            var v = (byte)Math.Clamp((int)Math.Round(value), 0, 255);
            if (_r == v)
            {
                return;
            }
            _r = v;
            OnPropertyChanged();
            SyncHexFromChannels();
        }
    }

    public double G
    {
        get => _g;
        set
        {
            var v = (byte)Math.Clamp((int)Math.Round(value), 0, 255);
            if (_g == v)
            {
                return;
            }
            _g = v;
            OnPropertyChanged();
            SyncHexFromChannels();
        }
    }

    public double B
    {
        get => _b;
        set
        {
            var v = (byte)Math.Clamp((int)Math.Round(value), 0, 255);
            if (_b == v)
            {
                return;
            }
            _b = v;
            OnPropertyChanged();
            SyncHexFromChannels();
        }
    }

    public string Hex
    {
        get => _hex;
        set
        {
            if (_suppress)
            {
                if (SetProperty(ref _hex, value ?? string.Empty))
                {
                    // ignore
                }
                return;
            }

            if (!TryParseHex(value, out var r, out var g, out var b))
            {
                // 保留输入以便用户继续编辑，但不推进通道
                SetProperty(ref _hex, value ?? string.Empty);
                return;
            }

            _suppress = true;
            try
            {
                _r = r;
                _g = g;
                _b = b;
                _hex = ToHex(r, g, b);
                OnPropertyChanged(nameof(R));
                OnPropertyChanged(nameof(G));
                OnPropertyChanged(nameof(B));
                OnPropertyChanged(nameof(Hex));
                OnPropertyChanged(nameof(PreviewBrush));
                OnPropertyChanged(nameof(RgbSummary));
            }
            finally
            {
                _suppress = false;
            }
            _changed?.Invoke();
        }
    }

    public IBrush PreviewBrush => new SolidColorBrush(Color.FromRgb(_r, _g, _b));

    public string RgbSummary => $"R {_r}  G {_g}  B {_b}";

    public void SetFromHex(string? hex)
    {
        if (TryParseHex(hex, out var r, out var g, out var b))
        {
            _suppress = true;
            try
            {
                _r = r;
                _g = g;
                _b = b;
                _hex = ToHex(r, g, b);
                OnPropertyChanged(nameof(R));
                OnPropertyChanged(nameof(G));
                OnPropertyChanged(nameof(B));
                OnPropertyChanged(nameof(Hex));
                OnPropertyChanged(nameof(PreviewBrush));
                OnPropertyChanged(nameof(RgbSummary));
            }
            finally
            {
                _suppress = false;
            }
        }
        else if (!string.IsNullOrWhiteSpace(hex))
        {
            SetProperty(ref _hex, hex.Trim());
        }
    }

    public string ToHexValue() => ToHex(_r, _g, _b);

    private void SyncHexFromChannels()
    {
        if (_suppress)
        {
            return;
        }

        _hex = ToHex(_r, _g, _b);
        OnPropertyChanged(nameof(Hex));
        OnPropertyChanged(nameof(PreviewBrush));
        OnPropertyChanged(nameof(RgbSummary));
        _changed?.Invoke();
    }

    /// <summary>解析 #RGB / #RRGGBB（可无 #）。</summary>
    public static bool TryParseHex(string? input, out byte r, out byte g, out byte b)
    {
        r = g = b = 0;
        if (string.IsNullOrWhiteSpace(input))
        {
            return false;
        }

        var s = input.Trim();
        if (s.StartsWith('#') || s.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
        {
            s = s.StartsWith('#') ? s[1..] : s[2..];
        }

        if (s.Length == 3
            && IsHex(s[0]) && IsHex(s[1]) && IsHex(s[2]))
        {
            r = ExpandNibble(s[0]);
            g = ExpandNibble(s[1]);
            b = ExpandNibble(s[2]);
            return true;
        }

        if (s.Length == 6
            && byte.TryParse(s[..2], System.Globalization.NumberStyles.HexNumber, null, out r)
            && byte.TryParse(s[2..4], System.Globalization.NumberStyles.HexNumber, null, out g)
            && byte.TryParse(s[4..6], System.Globalization.NumberStyles.HexNumber, null, out b))
        {
            return true;
        }

        return false;
    }

    public static string ToHex(byte r, byte g, byte b) =>
        $"#{r:X2}{g:X2}{b:X2}";

    private static bool IsHex(char c) =>
        (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');

    private static byte ExpandNibble(char c)
    {
        var n = c switch
        {
            >= '0' and <= '9' => c - '0',
            >= 'a' and <= 'f' => c - 'a' + 10,
            >= 'A' and <= 'F' => c - 'A' + 10,
            _ => 0,
        };
        return (byte)((n << 4) | n);
    }
}

/// <summary>色图中的一格色块。</summary>
public sealed class ColorSwatchItemViewModel : ViewModelBase
{
    private bool _isSelected;

    public ColorSwatchItemViewModel(string hex, Action<string> select)
    {
        Hex = NormalizeHex(hex);
        Brush = new SolidColorBrush(ParseOrDefault(Hex));
        SelectCommand = new RelayCommand(() => select(Hex));
    }

    public string Hex { get; }
    public IBrush Brush { get; }
    public RelayCommand SelectCommand { get; }

    public bool IsSelected
    {
        get => _isSelected;
        set => SetProperty(ref _isSelected, value);
    }

    public static string NormalizeHex(string hex)
    {
        if (ColorChannelEditor.TryParseHex(hex, out var r, out var g, out var b))
        {
            return ColorChannelEditor.ToHex(r, g, b);
        }

        return "#808080";
    }

    private static Color ParseOrDefault(string hex)
    {
        if (ColorChannelEditor.TryParseHex(hex, out var r, out var g, out var b))
        {
            return Color.FromRgb(r, g, b);
        }

        return Color.FromRgb(0x80, 0x80, 0x80);
    }
}

/// <summary>主题自定义三色通道。</summary>
public enum ThemeColorChannel
{
    Main,
    Surface,
    Brand,
}

/// <summary>
/// 个性化色图：色相 × 深浅的平面色板（点选，非 RGB 滑条）。
/// </summary>
public static class ColorPaletteMap
{
    /// <summary>
    /// 生成色图：列=色相，行=深浅（含中性灰列）。
    /// 默认 12 色相 × 5 阶 + 1 列灰 = 65 格，视觉成一张色图。
    /// </summary>
    public static IReadOnlyList<string> BuildHexMap(int hueSteps = 12, int lightSteps = 5)
    {
        hueSteps = Math.Clamp(hueSteps, 6, 24);
        lightSteps = Math.Clamp(lightSteps, 3, 8);
        var list = new List<string>(hueSteps * lightSteps + lightSteps);

        // 行：浅 → 深；列：色相环
        for (var li = 0; li < lightSteps; li++)
        {
            // 亮度从 0.88 → 0.28
            var t = lightSteps == 1 ? 0.5 : (double)li / (lightSteps - 1);
            var lightness = 0.88 - t * 0.60;
            var saturation = 0.42 + (1.0 - Math.Abs(t - 0.45) * 1.2) * 0.28;
            saturation = Math.Clamp(saturation, 0.35, 0.78);

            for (var hi = 0; hi < hueSteps; hi++)
            {
                var hue = hi * (360.0 / hueSteps);
                list.Add(HslToHex(hue, saturation, lightness));
            }

            // 每行末尾一格中性灰（同亮度）
            var g = (byte)Math.Clamp((int)Math.Round(lightness * 255), 0, 255);
            list.Add(ColorChannelEditor.ToHex(g, g, g));
        }

        // 追加 Git 常用默认（保证可点回默认）
        EnsurePresent(list, "#8A8F98");
        EnsurePresent(list, "#F59E0B");
        EnsurePresent(list, "#2E726B");
        EnsurePresent(list, "#2563EB");
        EnsurePresent(list, "#DC2626");
        EnsurePresent(list, "#7C3AED");
        return list;
    }

    public static int Columns(int hueSteps = 12) => Math.Clamp(hueSteps, 6, 24) + 1;

    private static void EnsurePresent(List<string> list, string hex)
    {
        var n = ColorSwatchItemViewModel.NormalizeHex(hex);
        if (!list.Any(h => string.Equals(h, n, StringComparison.OrdinalIgnoreCase)))
        {
            list.Add(n);
        }
    }

    /// <summary>HSL → #RRGGBB（H 0–360, S/L 0–1）。</summary>
    public static string HslToHex(double h, double s, double l)
    {
        h = ((h % 360) + 360) % 360;
        s = Math.Clamp(s, 0, 1);
        l = Math.Clamp(l, 0, 1);
        var c = (1 - Math.Abs(2 * l - 1)) * s;
        var x = c * (1 - Math.Abs(h / 60 % 2 - 1));
        var m = l - c / 2;
        double r1, g1, b1;
        if (h < 60) { r1 = c; g1 = x; b1 = 0; }
        else if (h < 120) { r1 = x; g1 = c; b1 = 0; }
        else if (h < 180) { r1 = 0; g1 = c; b1 = x; }
        else if (h < 240) { r1 = 0; g1 = x; b1 = c; }
        else if (h < 300) { r1 = x; g1 = 0; b1 = c; }
        else { r1 = c; g1 = 0; b1 = x; }

        var r = (byte)Math.Clamp((int)Math.Round((r1 + m) * 255), 0, 255);
        var g = (byte)Math.Clamp((int)Math.Round((g1 + m) * 255), 0, 255);
        var b = (byte)Math.Clamp((int)Math.Round((b1 + m) * 255), 0, 255);
        return ColorChannelEditor.ToHex(r, g, b);
    }
}

/// <summary>再进入供应商编辑时表单数据源选择。</summary>
public static class ProviderFormResolver
{
    /// <summary>
    /// 有 leave-save / 草稿快照时优先快照，避免用过期 _providerConfig 覆盖刚保存的表单。
    /// </summary>
    public static bool PreferFormSnapshotOverConfig(bool hasFormSnapshot) => hasFormSnapshot;
}

/// <summary>为新供应商分配不与现有 id 冲突的标识。</summary>
public static class ProviderIdAllocator
{
    public static string Allocate(IEnumerable<string> existingIds, string preferredBase = "provider")
    {
        var used = new HashSet<string>(
            existingIds.Where(id => !string.IsNullOrWhiteSpace(id)),
            StringComparer.OrdinalIgnoreCase);
        var baseId = string.IsNullOrWhiteSpace(preferredBase) ? "provider" : preferredBase.Trim();
        baseId = new string(baseId.Select(ch =>
            char.IsLetterOrDigit(ch) || ch is '_' or '-' ? char.ToLowerInvariant(ch) : '_').ToArray());
        if (string.IsNullOrWhiteSpace(baseId))
        {
            baseId = "provider";
        }

        if (!used.Contains(baseId))
        {
            return baseId;
        }

        for (var i = 2; i < 10000; i++)
        {
            var candidate = $"{baseId}_{i}";
            if (!used.Contains(candidate))
            {
                return candidate;
            }
        }

        return $"{baseId}_{Guid.NewGuid():N}"[..20];
    }
}
