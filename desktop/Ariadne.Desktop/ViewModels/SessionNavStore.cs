namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 会话级导航暂存：从开始页点侧栏进入后记住页面，下次无项目启动时恢复。
/// </summary>
public static class SessionNavStore
{
    private static string StorePath =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Ariadne",
            "session-nav.txt");

    public static string? LoadLastNavId()
    {
        try
        {
            var path = StorePath;
            if (!File.Exists(path))
            {
                return null;
            }

            var id = File.ReadAllText(path).Trim();
            return string.IsNullOrWhiteSpace(id) ? null : id;
        }
        catch
        {
            return null;
        }
    }

    public static void SaveLastNavId(string? navId)
    {
        try
        {
            if (string.IsNullOrWhiteSpace(navId))
            {
                return;
            }

            var path = StorePath;
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, navId.Trim());
        }
        catch
        {
            // 暂存失败不阻断导航
        }
    }
}
