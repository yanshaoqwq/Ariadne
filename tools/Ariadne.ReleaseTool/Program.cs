using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;

return await ReleaseTool.RunAsync(args);

internal static class ReleaseTool
{
    public static async Task<int> RunAsync(string[] args)
    {
        try
        {
            if (args.Length == 0 || args[0] is "-h" or "--help")
            {
                Console.WriteLine("Usage:");
                Console.WriteLine("  Ariadne.ReleaseTool licenses [--root <repo>] [--output <file>]");
                Console.WriteLine("  Ariadne.ReleaseTool assemble --rid <rid> --desktop-publish <dir> --rust-bin-dir <dir> --output <dir> [--root <repo>]");
                Console.WriteLine("  Ariadne.ReleaseTool verify-package --package <dir> [--root <repo>] [--allow-platform-sealed-mutation]");
                return args.Length == 0 ? 2 : 0;
            }

            var root = ResolveRoot(ReadOption(args, "--root"));
            switch (args[0])
            {
                case "licenses":
                {
                    var output = ReadOption(args, "--output") is { } explicitOutput
                        ? Path.GetFullPath(explicitOutput, root)
                        : Path.Combine(root, "THIRD_PARTY_NOTICES.md");

                    var rust = await LoadRustPackagesAsync(root);
                    var dotnet = LoadDotNetPackages(root);
                    WriteNotices(output, rust, dotnet);
                    Console.WriteLine($"Generated {Path.GetRelativePath(root, output)} ({rust.Count} Rust, {dotnet.Count} .NET packages).");
                    return 0;
                }
                case "assemble":
                    AssemblePackage(root, args);
                    return 0;
                case "verify-package":
                    VerifyPackage(
                        root,
                        RequiredOption(args, "--package"),
                        HasFlag(args, "--allow-platform-sealed-mutation"));
                    return 0;
                default:
                    throw new ArgumentException($"Unknown command: {args[0]}");
            }
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"release-tool: {ex.Message}");
            return 1;
        }
    }

    private static string? ReadOption(string[] args, string name)
    {
        for (var i = 1; i < args.Length; i++)
        {
            if (string.Equals(args[i], name, StringComparison.Ordinal))
            {
                if (i + 1 >= args.Length || args[i + 1].StartsWith("--", StringComparison.Ordinal))
                {
                    throw new ArgumentException($"{name} requires a value");
                }

                return args[i + 1];
            }
        }

        return null;
    }

    private static string RequiredOption(string[] args, string name) =>
        ReadOption(args, name) ?? throw new ArgumentException($"{name} is required");

    private static bool HasFlag(string[] args, string name)
    {
        var count = args.Skip(1).Count(argument => string.Equals(argument, name, StringComparison.Ordinal));
        if (count > 1)
        {
            throw new ArgumentException($"{name} may only be specified once");
        }

        return count == 1;
    }

    private static string ResolveRoot(string? explicitRoot)
    {
        var directory = new DirectoryInfo(Path.GetFullPath(explicitRoot ?? Environment.CurrentDirectory));
        while (directory is not null)
        {
            if (File.Exists(Path.Combine(directory.FullName, "Cargo.toml"))
                && File.Exists(Path.Combine(directory.FullName, "desktop", "Ariadne.Desktop", "Ariadne.Desktop.csproj")))
            {
                return directory.FullName;
            }

            directory = directory.Parent;
        }

        throw new DirectoryNotFoundException("Could not locate the Ariadne repository root.");
    }

    private static void AssemblePackage(string root, string[] args)
    {
        var rid = RequiredOption(args, "--rid");
        ValidateReleaseRid(root, rid);
        var desktopPublish = Path.GetFullPath(RequiredOption(args, "--desktop-publish"), root);
        var rustBinDirectory = Path.GetFullPath(RequiredOption(args, "--rust-bin-dir"), root);
        var output = Path.GetFullPath(RequiredOption(args, "--output"), root);
        if (!Directory.Exists(desktopPublish))
        {
            throw new DirectoryNotFoundException($"Desktop publish directory does not exist: {desktopPublish}");
        }
        if (!Directory.Exists(rustBinDirectory))
        {
            throw new DirectoryNotFoundException($"Rust binary directory does not exist: {rustBinDirectory}");
        }
        if (string.Equals(output.TrimEnd(Path.DirectorySeparatorChar), root.TrimEnd(Path.DirectorySeparatorChar), StringComparison.Ordinal))
        {
            throw new InvalidOperationException("Package output cannot be the repository root.");
        }

        if (Directory.Exists(output))
        {
            Directory.Delete(output, recursive: true);
        }
        Directory.CreateDirectory(output);
        CopyDirectory(desktopPublish, output, path => !string.Equals(Path.GetExtension(path), ".pdb", StringComparison.OrdinalIgnoreCase));

        var executableSuffix = rid.StartsWith("win-", StringComparison.Ordinal) ? ".exe" : string.Empty;
        CopyExecutable(rustBinDirectory, Path.Combine(output, "Backend"), $"ariadne-ipc{executableSuffix}");
        CopyExecutable(rustBinDirectory, Path.Combine(output, "Tools"), $"ariadne{executableSuffix}");

        foreach (var file in new[] { "LICENSE", "NOTICE", "COMMERCIAL_LICENSE.md", "THIRD_PARTY_NOTICES.md" })
        {
            File.Copy(Path.Combine(root, file), Path.Combine(output, file), overwrite: true);
        }

        CopyReleaseIcons(root, output);
        if (rid.StartsWith("linux-", StringComparison.Ordinal))
        {
            WriteLinuxDesktopEntry(root, output);
        }

        var desktopAssembly = Path.Combine(output, "Ariadne.Desktop.dll");
        if (!File.Exists(desktopAssembly))
        {
            throw new FileNotFoundException("Desktop publish output does not contain Ariadne.Desktop.dll.", desktopAssembly);
        }
        var version = AssemblyName.GetAssemblyName(desktopAssembly).Version?.ToString(3)
            ?? throw new InvalidDataException("Desktop assembly has no product version.");
        var workspaceVersion = ReadWorkspaceVersion(root);
        if (!string.Equals(version, workspaceVersion, StringComparison.Ordinal))
        {
            throw new InvalidDataException($"Desktop version {version} does not match Cargo workspace version {workspaceVersion}.");
        }
        var manifest = new PackageManifest(
            2,
            version,
            rid,
            EnumerateManifestFiles(output, rid));
        WriteJson(Path.Combine(output, "release-manifest.json"), manifest);
        Console.WriteLine($"Assembled {output} ({manifest.Files.Count} files, version {version}, {rid}).");
    }

    private static void VerifyPackage(string root, string packageArgument, bool allowPlatformSealedMutation)
    {
        var package = Path.GetFullPath(packageArgument, root);
        var manifestPath = Path.Combine(package, "release-manifest.json");
        var manifest = JsonSerializer.Deserialize<PackageManifest>(
            File.ReadAllText(manifestPath),
            JsonOptions()) ?? throw new InvalidDataException("Release manifest is invalid.");
        if (manifest.SchemaVersion != 2)
        {
            throw new InvalidDataException("Release manifest schema is unsupported.");
        }
        ValidateReleaseRid(root, manifest.Rid);
        if (allowPlatformSealedMutation && !manifest.Rid.StartsWith("osx-", StringComparison.Ordinal))
        {
            throw new InvalidDataException("Platform-sealed mutation is only valid for a macOS package after app bundle sealing.");
        }

        var actualFiles = Directory.EnumerateFiles(package, "*", SearchOption.AllDirectories)
            .Where(path => !string.Equals(path, manifestPath, StringComparison.Ordinal))
            .Select(path => NormalizeRelativePath(package, path))
            .OrderBy(path => path, StringComparer.Ordinal)
            .ToArray();
        var expectedFiles = manifest.Files.Select(file => file.Path).OrderBy(path => path, StringComparer.Ordinal).ToArray();
        if (!actualFiles.SequenceEqual(expectedFiles, StringComparer.Ordinal))
        {
            throw new InvalidDataException("Package file set does not match release-manifest.json.");
        }
        var expectedPlatformSealed = manifest.Rid.StartsWith("osx-", StringComparison.Ordinal)
            ? new[] { "Ariadne.Desktop" }
            : Array.Empty<string>();
        var actualPlatformSealed = manifest.Files
            .Where(file => file.PlatformSealed)
            .Select(file => file.Path)
            .OrderBy(path => path, StringComparer.Ordinal)
            .ToArray();
        if (!actualPlatformSealed.SequenceEqual(expectedPlatformSealed, StringComparer.Ordinal))
        {
            throw new InvalidDataException("Release manifest platform-sealed paths do not match the RID contract.");
        }

        var forbiddenNames = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            ".git", "target", "obj", "bin", "secrets.json", "runtime.db", "metadata.db", ".env",
            "ariadne-server", "ariadne-server.exe",
        };
        var repositoryBytes = Encoding.UTF8.GetBytes(root);
        foreach (var entry in manifest.Files)
        {
            var components = entry.Path.Split('/');
            if (components.Any(forbiddenNames.Contains) || string.Equals(Path.GetExtension(entry.Path), ".pdb", StringComparison.OrdinalIgnoreCase))
            {
                throw new InvalidDataException($"Package contains a forbidden development or credential path: {entry.Path}");
            }

            var fullPath = Path.Combine(package, entry.Path.Replace('/', Path.DirectorySeparatorChar));
            var verifyManifestBytes = !allowPlatformSealedMutation || !entry.PlatformSealed;
            if (verifyManifestBytes && new FileInfo(fullPath).Length != entry.Size)
            {
                throw new InvalidDataException($"Package size mismatch: {entry.Path}");
            }
            if (verifyManifestBytes
                && !string.Equals(ComputeSha256(fullPath), entry.Sha256, StringComparison.OrdinalIgnoreCase))
            {
                throw new InvalidDataException($"Package hash mismatch: {entry.Path}");
            }
            if (FileContains(fullPath, repositoryBytes))
            {
                throw new InvalidDataException($"Package embeds the build repository absolute path: {entry.Path}");
            }
        }

        var executableSuffix = manifest.Rid.StartsWith("win-", StringComparison.Ordinal) ? ".exe" : string.Empty;
        foreach (var required in new[]
                 {
                     "LICENSE", "NOTICE", "COMMERCIAL_LICENSE.md", "THIRD_PARTY_NOTICES.md",
                     "Resources/display_name.json", "Resources/prompt_list.json",
                     $"Backend/ariadne-ipc{executableSuffix}", $"Tools/ariadne{executableSuffix}",
                 })
        {
            if (!expectedFiles.Contains(required, StringComparer.Ordinal))
            {
                throw new InvalidDataException($"Package is missing required release content: {required}");
            }
        }

        RunInstalledSmoke(package, manifest.Rid);
        Console.WriteLine($"Verified release package {package} ({manifest.Files.Count} files, {manifest.Rid}).");
    }

    private static void ValidateReleaseRid(string root, string rid)
    {
        using var document = JsonDocument.Parse(File.ReadAllText(Path.Combine(root, "packaging", "release-matrix.json")));
        var supported = document.RootElement.GetProperty("targets")
            .EnumerateArray()
            .Select(target => target.GetProperty("rid").GetString())
            .Where(value => value is not null)
            .ToHashSet(StringComparer.Ordinal);
        if (!supported.Contains(rid))
        {
            throw new ArgumentException($"Unsupported release RID: {rid}");
        }
    }

    private static string ReadWorkspaceVersion(string root)
    {
        var content = File.ReadAllText(Path.Combine(root, "Cargo.toml"));
        var match = System.Text.RegularExpressions.Regex.Match(
            content,
            @"(?ms)^\[workspace\.package\]\s*.*?^version\s*=\s*""([^""]+)""");
        return match.Success
            ? match.Groups[1].Value
            : throw new InvalidDataException("Cargo.toml has no workspace package version.");
    }

    private static void CopyDirectory(string source, string destination, Func<string, bool> include)
    {
        foreach (var directory in Directory.EnumerateDirectories(source, "*", SearchOption.AllDirectories))
        {
            Directory.CreateDirectory(Path.Combine(destination, Path.GetRelativePath(source, directory)));
        }
        foreach (var file in Directory.EnumerateFiles(source, "*", SearchOption.AllDirectories).Where(include))
        {
            var target = Path.Combine(destination, Path.GetRelativePath(source, file));
            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            File.Copy(file, target, overwrite: true);
        }
    }

    private static void CopyExecutable(string sourceDirectory, string destinationDirectory, string fileName)
    {
        var source = Path.Combine(sourceDirectory, fileName);
        if (!File.Exists(source))
        {
            throw new FileNotFoundException($"Required Rust release binary is missing: {fileName}", source);
        }
        Directory.CreateDirectory(destinationDirectory);
        var destination = Path.Combine(destinationDirectory, fileName);
        File.Copy(source, destination, overwrite: true);
        if (!OperatingSystem.IsWindows())
        {
            File.SetUnixFileMode(destination, UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute
                | UnixFileMode.GroupRead | UnixFileMode.GroupExecute
                | UnixFileMode.OtherRead | UnixFileMode.OtherExecute);
        }
    }

    private static void CopyReleaseIcons(string root, string output)
    {
        var source = Path.Combine(root, "desktop", "Ariadne.Desktop", "Assets");
        var destination = Path.Combine(output, "Integration", "icons");
        Directory.CreateDirectory(destination);
        foreach (var size in new[] { 16, 24, 32, 48, 64, 128, 256, 512 })
        {
            File.Copy(
                Path.Combine(source, $"app-icon-{size}.png"),
                Path.Combine(destination, $"ariadne-{size}.png"),
                overwrite: true);
        }
        File.Copy(Path.Combine(source, "app-icon.ico"), Path.Combine(destination, "ariadne.ico"), overwrite: true);
    }

    private static void WriteLinuxDesktopEntry(string root, string output)
    {
        var names = JsonSerializer.Deserialize<Dictionary<string, string>>(
            File.ReadAllText(Path.Combine(root, "core", "resources", "display_name.json")))
            ?? throw new InvalidDataException("display_name.json is invalid.");
        string Resource(string key) => names.TryGetValue(key, out var value) && !string.IsNullOrWhiteSpace(value)
            ? value.Replace("\r", " ", StringComparison.Ordinal).Replace("\n", " ", StringComparison.Ordinal)
            : throw new InvalidDataException($"Missing release metadata resource: {key}");

        var content = $"""
            [Desktop Entry]
            Type=Application
            Version=1.0
            Name={Resource("ui.brand.name")}
            GenericName={Resource("ui.release.desktop_generic_name")}
            Comment={Resource("ui.release.desktop_comment")}
            Icon=ariadne
            TryExec=/opt/ariadne/Ariadne.Desktop
            Exec=/opt/ariadne/Ariadne.Desktop
            Terminal=false
            Categories=Office;Publishing;
            StartupWMClass=Ariadne.Desktop
            """;
        var path = Path.Combine(output, "Integration", "linux", "ariadne.desktop");
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, content + "\n", new UTF8Encoding(false));
    }

    private static IReadOnlyList<ManifestFile> EnumerateManifestFiles(string package, string rid)
    {
        return Directory.EnumerateFiles(package, "*", SearchOption.AllDirectories)
            .Where(path => !string.Equals(Path.GetFileName(path), "release-manifest.json", StringComparison.Ordinal))
            .Select(path =>
            {
                var relativePath = NormalizeRelativePath(package, path);
                var platformSealed = rid.StartsWith("osx-", StringComparison.Ordinal)
                                     && string.Equals(relativePath, "Ariadne.Desktop", StringComparison.Ordinal);
                return new ManifestFile(
                    relativePath,
                    new FileInfo(path).Length,
                    ComputeSha256(path),
                    platformSealed);
            })
            .OrderBy(file => file.Path, StringComparer.Ordinal)
            .ToArray();
    }

    private static string NormalizeRelativePath(string root, string path) =>
        Path.GetRelativePath(root, path).Replace(Path.DirectorySeparatorChar, '/');

    private static string ComputeSha256(string path)
    {
        using var stream = File.OpenRead(path);
        return Convert.ToHexString(SHA256.HashData(stream)).ToLowerInvariant();
    }

    private static bool FileContains(string path, byte[] needle)
    {
        if (needle.Length == 0)
        {
            return false;
        }
        var haystack = File.ReadAllBytes(path);
        return haystack.AsSpan().IndexOf(needle) >= 0;
    }

    private static void RunInstalledSmoke(string package, string rid)
    {
        var expectedRid = CurrentRid();
        if (!string.Equals(rid, expectedRid, StringComparison.Ordinal))
        {
            throw new InvalidOperationException($"Package smoke test requires its native runner ({rid}); current runner is {expectedRid}.");
        }

        var executable = Path.Combine(package, OperatingSystem.IsWindows() ? "Ariadne.Desktop.exe" : "Ariadne.Desktop");
        using var process = Process.Start(new ProcessStartInfo
        {
            FileName = executable,
            Arguments = "--verify-installation",
            WorkingDirectory = package,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        }) ?? throw new InvalidOperationException("Could not start packaged desktop smoke test.");
        if (!process.WaitForExit(TimeSpan.FromSeconds(20)))
        {
            process.Kill(entireProcessTree: true);
            throw new TimeoutException("Packaged desktop smoke test timed out.");
        }
        var stdout = process.StandardOutput.ReadToEnd();
        var stderr = process.StandardError.ReadToEnd();
        if (process.ExitCode != 0 || !stdout.Contains("release layout is valid", StringComparison.Ordinal))
        {
            throw new InvalidOperationException($"Packaged desktop smoke test failed ({process.ExitCode}): {stderr.Trim()}");
        }
    }

    private static string CurrentRid()
    {
        var os = OperatingSystem.IsWindows() ? "win" : OperatingSystem.IsMacOS() ? "osx" : OperatingSystem.IsLinux() ? "linux" : "unknown";
        var architecture = RuntimeInformation.OSArchitecture switch
        {
            Architecture.X64 => "x64",
            Architecture.Arm64 => "arm64",
            var other => other.ToString().ToLowerInvariant(),
        };
        return $"{os}-{architecture}";
    }

    private static void WriteJson<T>(string path, T value)
    {
        File.WriteAllText(path, JsonSerializer.Serialize(value, JsonOptions()) + "\n", new UTF8Encoding(false));
    }

    private static JsonSerializerOptions JsonOptions() => new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        WriteIndented = true,
    };

    private static async Task<IReadOnlyList<PackageLicense>> LoadRustPackagesAsync(string root)
    {
        var cargo = ResolveCargo(root);
        var start = new ProcessStartInfo
        {
            FileName = cargo,
            WorkingDirectory = root,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        start.ArgumentList.Add("metadata");
        start.ArgumentList.Add("--format-version");
        start.ArgumentList.Add("1");
        start.ArgumentList.Add("--locked");
        start.ArgumentList.Add("--all-features");

        var localRustc = Path.Combine(root, ".rustup", "toolchains", "stable-aarch64-unknown-linux-gnu", "bin", "rustc");
        if (File.Exists(localRustc) && string.IsNullOrWhiteSpace(Environment.GetEnvironmentVariable("RUSTC")))
        {
            start.Environment["RUSTC"] = localRustc;
        }

        using var process = Process.Start(start) ?? throw new InvalidOperationException("Could not start cargo metadata.");
        var outputTask = process.StandardOutput.ReadToEndAsync();
        var errorTask = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();
        var output = await outputTask;
        var error = await errorTask;
        if (process.ExitCode != 0)
        {
            throw new InvalidOperationException($"cargo metadata failed ({process.ExitCode}): {error.Trim()}");
        }

        using var document = JsonDocument.Parse(output);
        var rootElement = document.RootElement;
        var workspace = rootElement.GetProperty("workspace_members")
            .EnumerateArray()
            .Select(value => value.GetString()!)
            .ToHashSet(StringComparer.Ordinal);
        var packages = rootElement.GetProperty("packages")
            .EnumerateArray()
            .ToDictionary(package => package.GetProperty("id").GetString()!, package => package.Clone(), StringComparer.Ordinal);

        var reachable = new HashSet<string>(workspace, StringComparer.Ordinal);
        var pending = new Queue<string>(workspace);
        var nodes = rootElement.GetProperty("resolve").GetProperty("nodes")
            .EnumerateArray()
            .ToDictionary(node => node.GetProperty("id").GetString()!, node => node.Clone(), StringComparer.Ordinal);
        while (pending.TryDequeue(out var packageId))
        {
            if (!nodes.TryGetValue(packageId, out var node))
            {
                continue;
            }

            foreach (var dependency in node.GetProperty("deps").EnumerateArray())
            {
                var isRuntimeOrBuildDependency = dependency.GetProperty("dep_kinds")
                    .EnumerateArray()
                    .Any(kind => !string.Equals(
                        kind.GetProperty("kind").ValueKind == JsonValueKind.Null
                            ? null
                            : kind.GetProperty("kind").GetString(),
                        "dev",
                        StringComparison.Ordinal));
                var dependencyId = dependency.GetProperty("pkg").GetString()!;
                if (isRuntimeOrBuildDependency && reachable.Add(dependencyId))
                {
                    pending.Enqueue(dependencyId);
                }
            }
        }

        return reachable
            .Where(id => !workspace.Contains(id))
            .Select(id => ToRustPackage(packages[id]))
            .OrderBy(package => package.Name, StringComparer.OrdinalIgnoreCase)
            .ThenBy(package => package.Version, StringComparer.Ordinal)
            .ToArray();
    }

    private static PackageLicense ToRustPackage(JsonElement package)
    {
        var name = package.GetProperty("name").GetString()!;
        var version = package.GetProperty("version").GetString()!;
        var license = OptionalString(package, "license");
        var licenseFile = OptionalString(package, "license_file");
        if (string.IsNullOrWhiteSpace(license) && string.IsNullOrWhiteSpace(licenseFile))
        {
            throw new InvalidDataException($"Rust package {name} {version} has no license metadata.");
        }

        return new PackageLicense(
            name,
            version,
            string.IsNullOrWhiteSpace(license) ? $"file:{licenseFile}" : license,
            OptionalString(package, "repository") ?? OptionalString(package, "homepage") ?? OptionalString(package, "source") ?? string.Empty);
    }

    private static IReadOnlyList<PackageLicense> LoadDotNetPackages(string root)
    {
        var assetsPath = Path.Combine(root, "desktop", "Ariadne.Desktop", "obj", "project.assets.json");
        if (!File.Exists(assetsPath))
        {
            throw new FileNotFoundException("Run dotnet restore before generating notices.", assetsPath);
        }

        using var document = JsonDocument.Parse(File.ReadAllText(assetsPath));
        var rootElement = document.RootElement;
        var packageFolders = rootElement.GetProperty("packageFolders")
            .EnumerateObject()
            .Select(folder => folder.Name)
            .ToArray();

        var result = new List<PackageLicense>();
        foreach (var library in rootElement.GetProperty("libraries").EnumerateObject())
        {
            if (!string.Equals(OptionalString(library.Value, "type"), "package", StringComparison.Ordinal))
            {
                continue;
            }

            var separator = library.Name.LastIndexOf('/');
            if (separator <= 0 || separator == library.Name.Length - 1)
            {
                throw new InvalidDataException($"Invalid NuGet library identity: {library.Name}");
            }

            var id = library.Name[..separator];
            var version = library.Name[(separator + 1)..];
            var relativePath = OptionalString(library.Value, "path") ?? $"{id.ToLowerInvariant()}/{version}";
            var nuspec = packageFolders
                .Select(folder => Path.Combine(folder, relativePath))
                .Where(Directory.Exists)
                .SelectMany(folder => Directory.EnumerateFiles(folder, "*.nuspec", SearchOption.TopDirectoryOnly))
                .FirstOrDefault();
            if (nuspec is null)
            {
                throw new FileNotFoundException($"Could not locate nuspec for {id} {version}.");
            }

            result.Add(ReadNuGetLicense(nuspec, id, version));
        }

        return result
            .OrderBy(package => package.Name, StringComparer.OrdinalIgnoreCase)
            .ThenBy(package => package.Version, StringComparer.Ordinal)
            .ToArray();
    }

    private static PackageLicense ReadNuGetLicense(string nuspecPath, string fallbackId, string fallbackVersion)
    {
        var document = XDocument.Load(nuspecPath, LoadOptions.None);
        var metadata = document.Descendants().FirstOrDefault(element => element.Name.LocalName == "metadata")
            ?? throw new InvalidDataException($"NuGet manifest has no metadata: {nuspecPath}");
        var id = ElementValue(metadata, "id") ?? fallbackId;
        var version = ElementValue(metadata, "version") ?? fallbackVersion;
        var licenseElement = metadata.Elements().FirstOrDefault(element => element.Name.LocalName == "license");
        var license = licenseElement?.Value.Trim();
        if (!string.IsNullOrWhiteSpace(license)
            && string.Equals(licenseElement?.Attribute("type")?.Value, "file", StringComparison.OrdinalIgnoreCase))
        {
            license = $"file:{license}";
        }

        license ??= ElementValue(metadata, "licenseUrl");
        if (string.IsNullOrWhiteSpace(license))
        {
            throw new InvalidDataException($"NuGet package {id} {version} has no license metadata.");
        }

        var repository = metadata.Elements().FirstOrDefault(element => element.Name.LocalName == "repository")?.Attribute("url")?.Value;
        var source = repository ?? ElementValue(metadata, "projectUrl") ?? string.Empty;
        return new PackageLicense(id, version, license, source);
    }

    private static string? ElementValue(XElement parent, string localName) =>
        parent.Elements().FirstOrDefault(element => element.Name.LocalName == localName)?.Value.Trim();

    private static string? OptionalString(JsonElement element, string propertyName)
    {
        if (!element.TryGetProperty(propertyName, out var property) || property.ValueKind is JsonValueKind.Null or JsonValueKind.Undefined)
        {
            return null;
        }

        return property.GetString();
    }

    private static string ResolveCargo(string root)
    {
        var configured = Environment.GetEnvironmentVariable("CARGO");
        if (!string.IsNullOrWhiteSpace(configured))
        {
            return configured;
        }

        var local = Path.Combine(root, ".rustup", "toolchains", "stable-aarch64-unknown-linux-gnu", "bin", "cargo");
        return File.Exists(local) ? local : "cargo";
    }

    private static void WriteNotices(
        string outputPath,
        IReadOnlyList<PackageLicense> rust,
        IReadOnlyList<PackageLicense> dotnet)
    {
        var content = new StringBuilder();
        content.AppendLine("# Third-Party Notices");
        content.AppendLine();
        content.AppendLine("This file is generated by `tools/Ariadne.ReleaseTool` from Cargo metadata and NuGet package manifests.");
        content.AppendLine("Ariadne's own license is in `LICENSE`; each component below remains governed by its listed license.");
        content.AppendLine();
        AppendTable(content, "Rust runtime and build dependencies", rust);
        AppendTable(content, ".NET desktop runtime and build dependencies", dotnet);

        Directory.CreateDirectory(Path.GetDirectoryName(outputPath)!);
        var normalized = content
            .ToString()
            .Replace("\r\n", "\n", StringComparison.Ordinal)
            .TrimEnd('\n');
        File.WriteAllText(outputPath, normalized + "\n", new UTF8Encoding(false));
    }

    private static void AppendTable(StringBuilder content, string title, IReadOnlyList<PackageLicense> packages)
    {
        content.AppendLine($"## {title}");
        content.AppendLine();
        content.AppendLine("| Package | Version | License | Source |");
        content.AppendLine("|---|---:|---|---|");
        foreach (var package in packages)
        {
            content.Append("| `").Append(Escape(package.Name)).Append("` | `")
                .Append(Escape(package.Version)).Append("` | `")
                .Append(Escape(package.License)).Append("` | ")
                .Append(string.IsNullOrWhiteSpace(package.Source) ? "-" : $"<{Escape(package.Source)}>")
                .AppendLine(" |");
        }

        content.AppendLine();
    }

    private static string Escape(string value) => value.Replace("|", "\\|", StringComparison.Ordinal);

    private sealed record PackageManifest(int SchemaVersion, string Version, string Rid, IReadOnlyList<ManifestFile> Files);

    private sealed record ManifestFile(string Path, long Size, string Sha256, bool PlatformSealed = false);

    private sealed record PackageLicense(string Name, string Version, string License, string Source);
}
