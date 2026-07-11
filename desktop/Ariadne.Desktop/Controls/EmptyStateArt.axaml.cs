using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Threading;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 分母题空态插画。三主空态用模型线描 + Accent 重着色；其余轻量 Path。
/// Kind：Workspace | WorksSnap | GitRag | WelcomeQuill | RunLog | Templates | ProjectAi
/// </summary>
public partial class EmptyStateArt : UserControl
{
    public static readonly StyledProperty<string> KindProperty =
        AvaloniaProperty.Register<EmptyStateArt, string>(nameof(Kind), defaultValue: "Workspace");

    private Bitmap? _current;

    public EmptyStateArt()
    {
        InitializeComponent();
        PropertyChanged += (_, e) =>
        {
            if (e.Property == KindProperty)
            {
                ApplyKind();
            }
        };
        Loaded += (_, _) =>
        {
            AppIconPainter.IconColorsChanged += OnThemeColorsChanged;
            ApplyKind();
        };
        Unloaded += (_, _) => AppIconPainter.IconColorsChanged -= OnThemeColorsChanged;
    }

    public string Kind
    {
        get => GetValue(KindProperty);
        set => SetValue(KindProperty, value);
    }

    private void OnThemeColorsChanged()
    {
        Dispatcher.UIThread.Post(ApplyKind, DispatcherPriority.Background);
    }

    private void ApplyKind()
    {
        var kind = (Kind ?? "Workspace").Trim();
        var asset = kind.ToLowerInvariant() switch
        {
            "workspace" => "avares://Ariadne.Desktop/Assets/empty/workspace.png",
            "workssnap" or "works" => "avares://Ariadne.Desktop/Assets/empty/works-snap.png",
            "gitrag" or "git" => "avares://Ariadne.Desktop/Assets/empty/git-rag.png",
            _ => null,
        };

        var useImage = asset is not null;
        if (ArtImage is not null)
        {
            ArtImage.IsVisible = useImage;
        }

        SetVis(WelcomeQuillArt, kind, "WelcomeQuill");
        SetVis(RunLogArt, kind, "RunLog");
        SetVis(TemplatesArt, kind, "Templates");
        SetVis(ProjectAiArt, kind, "ProjectAi");

        if (!useImage || ArtImage is null)
        {
            ClearImage();
            return;
        }

        try
        {
            var accent = AppIconPainter.ResolveColor(
                "Ariadne.AccentPrimary",
                Color.FromRgb(0x35, 0x6F, 0x68));
            var next = AppIconPainter.RenderAssetBitmap(asset!, accent, 256, transparentPaper: true);
            var old = _current;
            _current = next;
            ArtImage.Source = next;
            old?.Dispose();
        }
        catch
        {
            ClearImage();
        }
    }

    private void ClearImage()
    {
        if (ArtImage is not null)
        {
            ArtImage.Source = null;
        }

        _current?.Dispose();
        _current = null;
    }

    private static void SetVis(Control? control, string kind, string expected)
    {
        if (control is not null)
        {
            control.IsVisible = string.Equals(kind, expected, StringComparison.OrdinalIgnoreCase);
        }
    }
}
