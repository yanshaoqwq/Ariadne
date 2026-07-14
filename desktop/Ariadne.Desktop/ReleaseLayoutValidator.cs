using System.Diagnostics;
using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop;

internal static class ReleaseLayoutValidator
{
    private static readonly string[] RequiredFiles =
    {
        "LICENSE",
        "NOTICE",
        "COMMERCIAL_LICENSE.md",
        "THIRD_PARTY_NOTICES.md",
        "Resources/display_name.json",
        "Resources/prompt_list.json",
    };

    public static bool TryValidate(string appDirectory, out string error)
    {
        if (Directory.EnumerateFiles(appDirectory, "*", SearchOption.AllDirectories)
            .Select(Path.GetFileName)
            .Any(name => string.Equals(name, "ariadne-server", StringComparison.OrdinalIgnoreCase)
                         || string.Equals(name, "ariadne-server.exe", StringComparison.OrdinalIgnoreCase)))
        {
            error = "release package must not contain the remote REST server";
            return false;
        }

        foreach (var relativePath in RequiredFiles)
        {
            var path = Path.Combine(appDirectory, relativePath.Replace('/', Path.DirectorySeparatorChar));
            if (!File.Exists(path))
            {
                error = $"required release file is missing: {relativePath}";
                return false;
            }
        }

        var backend = JsonLineBackendClient.FindPackagedBackendCommand(appDirectory);
        if (backend is null)
        {
            error = "packaged backend sidecar is missing from Backend/";
            return false;
        }

        if (!TryRunBackendHelp(backend, out error))
        {
            return false;
        }

        error = string.Empty;
        return true;
    }

    private static bool TryRunBackendHelp(string backend, out string error)
    {
        using var process = Process.Start(new ProcessStartInfo
        {
            FileName = backend,
            Arguments = "--help",
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            CreateNoWindow = true,
        });
        if (process is null)
        {
            error = "failed to start packaged backend sidecar";
            return false;
        }

        if (!process.WaitForExit(TimeSpan.FromSeconds(10)))
        {
            process.Kill(entireProcessTree: true);
            error = "packaged backend sidecar did not finish its smoke check";
            return false;
        }

        var stdout = process.StandardOutput.ReadToEnd();
        var stderr = process.StandardError.ReadToEnd();
        if (process.ExitCode != 0 || !stdout.Contains("usage: ariadne-ipc", StringComparison.Ordinal))
        {
            error = $"packaged backend sidecar smoke check failed: {stderr.Trim()}";
            return false;
        }

        error = string.Empty;
        return true;
    }
}
