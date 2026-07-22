using System.Reflection;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class CachedPageLocalizationTests
{
    [Fact]
    public void LanguageSwitchRefreshesCachedPageBindingsAndMaterializedOptions()
    {
        using var resources = new TemporaryLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = NoopBackend.Create();
        var window = new MainWindowViewModel(names, backend, id => id switch
        {
            "run_logs" => new RunLogPageViewModel(names, backend),
            "works" => new WorksPageViewModel(names, backend),
            "templates" => new TemplateMarketPageViewModel(names, backend),
            _ => null,
        }, _ => { });
        var runLogs = Assert.IsType<RunLogPageViewModel>(window.GetPageForTests("run_logs"));
        var works = Assert.IsType<WorksPageViewModel>(window.GetPageForTests("works"));
        var templates = Assert.IsType<TemplateMarketPageViewModel>(window.GetPageForTests("templates"));

        names.SwitchLanguage("fr");

        Assert.Equal("Journal FR", runLogs.Title);
        Assert.Equal("Tous les niveaux", runLogs.LevelOptions[0].Label);
        Assert.Equal("Markdown FR", works.ExportFormats[0].Label);
        Assert.Equal("Roman FR", templates.Tags[0].Title);
    }

    private sealed class TemporaryLanguageResources : IDisposable
    {
        public TemporaryLanguageResources()
        {
            Path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), $"ariadne-language-{Guid.NewGuid():N}");
            Directory.CreateDirectory(Path);
            Write("display_name.json", new Dictionary<string, string>
            {
                ["ui.run_log.title"] = "日志",
                ["ui.run_log.all_levels"] = "全部级别",
                ["ui.works.export_format.markdown"] = "Markdown",
                ["ui.template.tag.novel"] = "小说",
            });
            Write("display_name.fr.json", new Dictionary<string, string>
            {
                ["ui.run_log.title"] = "Journal FR",
                ["ui.run_log.all_levels"] = "Tous les niveaux",
                ["ui.works.export_format.markdown"] = "Markdown FR",
                ["ui.template.tag.novel"] = "Roman FR",
            });
        }

        public string Path { get; }

        private void Write(string fileName, IReadOnlyDictionary<string, string> values)
        {
            File.WriteAllText(
                System.IO.Path.Combine(Path, fileName),
                JsonSerializer.Serialize(values));
        }

        public void Dispose() => Directory.Delete(Path, recursive: true);
    }

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}")
            {
                return false;
            }
            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
