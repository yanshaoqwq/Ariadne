using System.Diagnostics;
using System.Text;
using System.Text.Json;

namespace Ariadne.Desktop.Backend;

public sealed class JsonLineBackendClient : IAriadneBackendClient
{
    private readonly string? _backendCommand;
    private readonly JsonSerializerOptions _jsonOptions = new(JsonSerializerDefaults.Web);

    private JsonLineBackendClient(string? backendCommand)
    {
        _backendCommand = backendCommand;
    }

    public static JsonLineBackendClient CreateDefault()
    {
        return new JsonLineBackendClient(Environment.GetEnvironmentVariable("ARIADNE_BACKEND_IPC") ?? DiscoverBackendCommand());
    }

    public Task<IReadOnlyList<RecentProjectEntry>> ListRecentProjectsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeOrEmptyListAsync<RecentProjectEntry>("list_recent_projects", null, cancellationToken);
    }

    public Task<AppStatus?> GetAppStatusAsync(CancellationToken cancellationToken = default)
    {
        return InvokeAsync<AppStatus>("get_app_status", null, cancellationToken);
    }

    public Task<CurrentProjectStatus?> GetCurrentProjectAsync(CancellationToken cancellationToken = default)
    {
        return InvokeAsync<CurrentProjectStatus>("get_current_project", null, cancellationToken);
    }

    public async Task<T?> InvokeAsync<T>(
        string method,
        object? parameters = null,
        CancellationToken cancellationToken = default)
    {
        return await InvokeOrDefaultAsync<T>(method, parameters, cancellationToken).ConfigureAwait(false);
    }

    private async Task<IReadOnlyList<T>> InvokeOrEmptyListAsync<T>(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        var result = await InvokeOrDefaultAsync<List<T>>(method, parameters, cancellationToken).ConfigureAwait(false);
        return result is null ? Array.Empty<T>() : result;
    }

    private async Task<T?> InvokeOrDefaultAsync<T>(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(_backendCommand))
        {
            return default;
        }

        var startInfo = new ProcessStartInfo
        {
            FileName = ResolveCommandFileName(_backendCommand),
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            StandardInputEncoding = Encoding.UTF8,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };
        foreach (var argument in ResolveCommandArguments(_backendCommand))
        {
            startInfo.ArgumentList.Add(argument);
        }

        using var process = Process.Start(startInfo);
        if (process is null)
        {
            return default;
        }

        var request = JsonSerializer.Serialize(new { method, @params = parameters ?? new { } }, _jsonOptions);
        await process.StandardInput.WriteLineAsync(request.AsMemory(), cancellationToken).ConfigureAwait(false);
        process.StandardInput.Close();

        var output = await process.StandardOutput.ReadToEndAsync(cancellationToken).ConfigureAwait(false);
        await process.WaitForExitAsync(cancellationToken).ConfigureAwait(false);

        if (process.ExitCode != 0 || string.IsNullOrWhiteSpace(output))
        {
            return default;
        }

        var result = JsonSerializer.Deserialize<BackendResult<T>>(output, _jsonOptions);
        if (result?.Ok != true)
        {
            return default;
        }

        return result.Data;
    }

    private static string? DiscoverBackendCommand()
    {
        var candidates = new[]
        {
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "core", "target", "debug", "ariadne-ipc")),
            Path.GetFullPath(Path.Combine(Environment.CurrentDirectory, "core", "target", "debug", "ariadne-ipc")),
            Path.GetFullPath(Path.Combine(Environment.CurrentDirectory, "target", "debug", "ariadne-ipc")),
        };

        return candidates.FirstOrDefault(File.Exists);
    }

    private static string ResolveCommandFileName(string command)
    {
        return command.Split(' ', StringSplitOptions.RemoveEmptyEntries).FirstOrDefault() ?? command;
    }

    private static IEnumerable<string> ResolveCommandArguments(string command)
    {
        return command.Split(' ', StringSplitOptions.RemoveEmptyEntries).Skip(1);
    }
}
