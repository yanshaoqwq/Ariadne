using System.Text.Json;
using System.Text.RegularExpressions;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Avalonia.Automation;
using Avalonia.Automation.Peers;
using Avalonia.Automation.Provider;
using Avalonia.Controls;
using Avalonia.Input;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class SettingsAccessibilityTests
{
    [Fact]
    public void SettingsFormInputs_HaveProgrammaticNamesAndErrorsAreLive()
    {
        var view = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));
        var inputTags = Regex.Matches(
            view,
            @"<(?<type>TextBox|ComboBox|ToggleSwitch)\s[\s\S]*?>",
            RegexOptions.CultureInvariant);
        var unnamed = inputTags
            .Select(match => match.Value.ReplaceLineEndings(" ").Trim())
            .Where(tag => !tag.Contains("AutomationProperties.Name", StringComparison.Ordinal))
            .ToArray();

        Assert.NotEmpty(inputTags);
        Assert.True(unnamed.Length == 0, "settings inputs without accessible names:\n" + string.Join('\n', unnamed));
        Assert.Equal(3, Regex.Matches(view, @"<ctl:SpectrumColorPicker\s[\s\S]*?AccessibleName=").Count);
        Assert.Contains("AutomationProperties.LiveSetting=\"Assertive\"", view, StringComparison.Ordinal);
    }

    [Fact]
    public void SettingsNavigation_UsesStandardSingleSelectionWithoutBypassingLeaveGuard()
    {
        var view = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));
        var viewModel = File.ReadAllText(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));

        Assert.Contains("<ListBox ItemsSource=\"{Binding Tabs}\"", view, StringComparison.Ordinal);
        Assert.Contains("SelectionMode=\"Single\"", view, StringComparison.Ordinal);
        Assert.Contains("SelectedItem=\"{Binding NavigationSelection, Mode=TwoWay}\"", view, StringComparison.Ordinal);
        Assert.Contains("await SelectTabAsync(tab)", viewModel, StringComparison.Ordinal);
        Assert.Contains("ConfirmLeaveIfNeededAsync()", viewModel, StringComparison.Ordinal);
        Assert.Contains("OnPropertyChanged(nameof(NavigationSelection))", viewModel, StringComparison.Ordinal);
    }

    [Fact]
    public void SpectrumPicker_ExposesFocusOrderKeyboardEditingAndEscapeClose()
    {
        var xaml = File.ReadAllText(ResolveDesktopSource("Controls", "SpectrumColorPicker.axaml"));
        var code = File.ReadAllText(ResolveDesktopSource("Controls", "SpectrumColorPicker.axaml.cs"));

        Assert.Contains("Focusable=\"True\"", xaml, StringComparison.Ordinal);
        Assert.Contains("KeyDown=\"OnSvKeyDown\"", xaml, StringComparison.Ordinal);
        Assert.Contains("KeyDown=\"OnHueKeyDown\"", xaml, StringComparison.Ordinal);
        Assert.Contains("KeyDown=\"OnPopupKeyDown\"", xaml, StringComparison.Ordinal);
        Assert.Contains("TabIndex=\"5\"", xaml, StringComparison.Ordinal);
        Assert.Contains("e.Key != Key.Escape", code, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.SetName(CollapsedSwatch", code, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.SetName(SvField", code, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.SetName(HueBar", code, StringComparison.Ordinal);
    }

    [Fact]
    public void AvaloniaAutomationPeers_ExposeProgrammaticNamesAndExactRangeControls()
    {
        var swatch = new Button();
        var red = new Slider { Minimum = 0, Maximum = 255, Value = 46 };
        AutomationProperties.SetName(swatch, "测试颜色，当前值 #2E726B");
        AutomationProperties.SetName(red, "红色通道");
        var swatchPeer = ControlAutomationPeer.CreatePeerForElement(swatch);
        var redPeer = ControlAutomationPeer.CreatePeerForElement(red);
        Assert.Contains("测试颜色", swatchPeer.GetName(), StringComparison.Ordinal);
        Assert.Contains("#2E726B", swatchPeer.GetName(), StringComparison.OrdinalIgnoreCase);
        Assert.NotNull(redPeer.GetProvider<IRangeValueProvider>());
    }

    [Fact]
    public void SpectrumKeyboardEditing_AdjustsBothAxesAndClamps()
    {
        var saturation = 0.5;
        var value = 0.5;

        Assert.True(SpectrumColorPicker.TryAdjustSaturationValue(Key.Left, false, ref saturation, ref value));
        Assert.Equal(0.49, saturation, 6);
        Assert.True(SpectrumColorPicker.TryAdjustSaturationValue(Key.Up, true, ref saturation, ref value));
        Assert.Equal(0.6, value, 6);

        saturation = 0;
        value = 1;
        Assert.True(SpectrumColorPicker.TryAdjustSaturationValue(Key.Left, true, ref saturation, ref value));
        Assert.True(SpectrumColorPicker.TryAdjustSaturationValue(Key.PageUp, false, ref saturation, ref value));
        Assert.Equal(0, saturation);
        Assert.Equal(1, value);
        Assert.False(SpectrumColorPicker.TryAdjustSaturationValue(Key.Enter, false, ref saturation, ref value));
    }

    [Fact]
    public void SpectrumKeyboardEditing_AdjustsHueWithFineAndLargeSteps()
    {
        var hue = 180.0;

        Assert.True(SpectrumColorPicker.TryAdjustHue(Key.Up, false, ref hue));
        Assert.Equal(179, hue);
        Assert.True(SpectrumColorPicker.TryAdjustHue(Key.PageDown, false, ref hue));
        Assert.Equal(189, hue);
        Assert.True(SpectrumColorPicker.TryAdjustHue(Key.End, false, ref hue));
        Assert.Equal(360, hue);
        Assert.True(SpectrumColorPicker.TryAdjustHue(Key.Right, true, ref hue));
        Assert.Equal(360, hue);
        Assert.False(SpectrumColorPicker.TryAdjustHue(Key.Enter, false, ref hue));
    }

    [Fact]
    public void SpectrumAccessibilityText_ComesFromDisplayResources()
    {
        var resourcePath = ResolveDesktopSource("..", "..", "core", "resources", "display_name.json");
        using var document = JsonDocument.Parse(File.ReadAllText(resourcePath));
        var root = document.RootElement;
        var keys = new[]
        {
            "ui.color.channel_red",
            "ui.color.channel_green",
            "ui.color.channel_blue",
            "ui.color.rgb_value",
            "ui.color.picker.current_value",
            "ui.color.picker.help",
            "ui.color.saturation_value.current",
            "ui.color.saturation_value.help",
            "ui.color.hue.current",
            "ui.color.hue.help",
        };

        Assert.All(keys, key => Assert.True(root.TryGetProperty(key, out _), $"missing display resource: {key}"));
    }

    [Fact]
    public void SettingsTutorial_UsesImmediateSharedDialogWithoutPreferenceMutation()
    {
        var view = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));
        var viewModel = File.ReadAllText(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));
        var start = viewModel.IndexOf("internal async Task ShowTutorialAsync()", StringComparison.Ordinal);
        var end = viewModel.IndexOf("private Task<bool> SaveMiscAsync()", start, StringComparison.Ordinal);

        Assert.True(start >= 0 && end > start);
        var action = viewModel[start..end];
        var miscStart = view.IndexOf("<StackPanel IsVisible=\"{Binding IsMiscSelected}\" Spacing=\"18\">", StringComparison.Ordinal);
        var tutorialStart = view.IndexOf("Command=\"{Binding ShowTutorialCommand}\"", miscStart, StringComparison.Ordinal);
        var editableSettingsStart = view.IndexOf("<StackPanel IsEnabled=\"{Binding IsMiscEditable}\" Spacing=\"18\">", miscStart, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding ShowTutorialCommand}\"", view, StringComparison.Ordinal);
        Assert.Contains("Content=\"{Binding OpenTutorialText}\"", view, StringComparison.Ordinal);
        Assert.True(miscStart >= 0 && tutorialStart > miscStart && editableSettingsStart > tutorialStart);
        Assert.Contains("HelpDialogFactory.CreateTutorialDialog(_displayNames)", action, StringComparison.Ordinal);
        Assert.Contains("DialogService.Current", action, StringComparison.Ordinal);
        Assert.DoesNotContain("SaveUiPreferencesAsync", action, StringComparison.Ordinal);
        Assert.DoesNotContain("OnboardingSeen", action, StringComparison.Ordinal);
        Assert.DoesNotContain("ResetOnboarding", view + viewModel, StringComparison.Ordinal);
    }

    [Fact]
    public void SettingsPersonalization_HidesCompatibilityFlagAndExposesPositivePanelPreference()
    {
        var view = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));
        var viewModel = File.ReadAllText(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));

        Assert.Contains("IsChecked=\"{Binding ProjectPanelVisible, Mode=TwoWay}\"", view, StringComparison.Ordinal);
        Assert.DoesNotContain("OnboardingSeen", view, StringComparison.Ordinal);
        Assert.DoesNotContain("public bool OnboardingSeen", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("nameof(OnboardingSeen)", viewModel, StringComparison.Ordinal);
        Assert.Contains("_uiPreferences?.OnboardingSeen ?? false", viewModel, StringComparison.Ordinal);
    }

    [Fact]
    public void SharedTutorialDialog_UsesLocalizedTutorialContent()
    {
        var names = DisplayNameService.LoadDefault();
        var dialog = HelpDialogFactory.CreateTutorialDialog(names);

        Assert.Equal(names.Text("ui.tutorial.title"), dialog.Title);
        Assert.Contains(names.Text("ui.tutorial.step.workspace"), dialog.Message, StringComparison.Ordinal);
        Assert.Contains(names.Text("ui.tutorial.step.works"), dialog.Message, StringComparison.Ordinal);
        Assert.Contains(names.Text("ui.tutorial.step.confirmations"), dialog.Message, StringComparison.Ordinal);
        Assert.Contains(names.Text("ui.tutorial.step.settings"), dialog.Message, StringComparison.Ordinal);
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        for (var depth = 0; directory is not null && depth < 10; depth++)
        {
            var candidate = Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }
}
