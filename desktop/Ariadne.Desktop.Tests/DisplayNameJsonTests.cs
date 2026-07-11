using System.Text.Json;
using Ariadne.Desktop.Localization;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// Ensures the shipped display_name pack parses and loads through the real desktop path.
/// </summary>
public sealed class DisplayNameJsonTests
{
    private static string ResolveDisplayNamePath()
    {
        // Prefer the source-of-truth pack linked into the desktop project.
        var candidates = new[]
        {
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "Resources", "display_name.json")),
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "Ariadne.Desktop", "bin", "Debug", "net10.0", "Resources", "display_name.json")),
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "..", "core", "resources", "display_name.json")),
        };

        foreach (var path in candidates)
        {
            if (File.Exists(path))
            {
                return path;
            }
        }

        // Walk up from test bin to repo core/resources.
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && dir is not null; i++)
        {
            var candidate = Path.Combine(dir.FullName, "core", "resources", "display_name.json");
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }

        throw new FileNotFoundException("Could not locate display_name.json from test base " + AppContext.BaseDirectory);
    }

    [Fact]
    public void DisplayNameJson_ParsesAsStringDictionary()
    {
        var path = ResolveDisplayNamePath();
        using var stream = File.OpenRead(path);
        var map = JsonSerializer.Deserialize<Dictionary<string, string>>(stream);

        Assert.NotNull(map);
        Assert.True(map!.Count > 100, $"expected rich pack, got {map.Count} keys from {path}");
        Assert.True(map.ContainsKey("ui.workspace.node_timeout_seconds"));
        Assert.Equal("超时（秒）", map["ui.workspace.node_timeout_seconds"]);
        Assert.True(map.ContainsKey("ui.workspace.node_timeout"));
        Assert.Equal("超时（ms）", map["ui.workspace.node_timeout"]);
        Assert.True(map.ContainsKey("ui.common.optional"));
        Assert.Equal("可选", map["ui.common.optional"]);
        Assert.True(map.ContainsKey("ui.common.unit.seconds"));
        Assert.Equal("秒", map["ui.common.unit.seconds"]);
        Assert.True(map.ContainsKey("ui.works.export_done_title"));
        Assert.True(map.ContainsKey("ui.works.export_done_message"));
        Assert.True(map.ContainsKey("ui.works.export_open_folder"));
        Assert.True(map.ContainsKey("ui.workspace.start_node.browse_work_dir"));
        Assert.True(map.ContainsKey("ui.workspace.export_autosaved"));
        // No bare string tokens that would break object parse (regression for line 616).
        Assert.DoesNotContain(map.Values, v => v is null);
    }

    [Fact]
    public void DisplayNameService_LoadDefault_ResolvesWorkspaceKeysNotBrackets()
    {
        // Real service path used by WorkspacePageViewModel labels (LoadDefault + Text).
        var service = DisplayNameService.LoadDefault();
        var timeout = service.Text("ui.workspace.node_timeout_seconds");
        var minimap = service.Text("ui.workspace.minimap");

        Assert.False(string.IsNullOrWhiteSpace(timeout), "timeout label empty — pack failed to load?");
        Assert.DoesNotContain("[", timeout);
        Assert.DoesNotContain("]", timeout);
        Assert.Equal("超时（秒）", timeout);

        Assert.False(string.IsNullOrWhiteSpace(minimap));
        Assert.DoesNotContain("ui.workspace.minimap", minimap);
    }
}
