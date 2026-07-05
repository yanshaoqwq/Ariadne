using Avalonia.Media;

namespace Ariadne.Desktop.ViewModels;

/// 矢量图标几何：描边线条路径，供导航等处用 Geometry 渲染，
/// 彻底避开图标字体缺失（Segoe Fluent Icons 在 Linux 上不存在会渲染成豆腐块）。
/// 与 Resources/Styles/AriadneTheme.axaml 的 Ariadne.Icon.* 保持同形。
public static class IconGeometries
{
    // 侧栏导航（路径数据，与 AriadneTheme.axaml 的 Ariadne.Icon.* 同形）
    private const string WorkspaceData =
        "M6,6 m-3,0 a3,3 0 1,0 6,0 a3,3 0 1,0 -6,0 M18,6 m-3,0 a3,3 0 1,0 6,0 a3,3 0 1,0 -6,0 M12,18 m-3,0 a3,3 0 1,0 6,0 a3,3 0 1,0 -6,0 M7.5,8 L11,15 M16.5,8 L13,15";
    private const string WorksData =
        "M5,4 L14,4 L19,9 L19,20 L5,20 Z M14,4 L14,9 L19,9 M8,13 L16,13 M8,16 L16,16";
    private const string GitData =
        "M7,5 m-2.5,0 a2.5,2.5 0 1,0 5,0 a2.5,2.5 0 1,0 -5,0 M7,19 m-2.5,0 a2.5,2.5 0 1,0 5,0 a2.5,2.5 0 1,0 -5,0 M17,8 m-2.5,0 a2.5,2.5 0 1,0 5,0 a2.5,2.5 0 1,0 -5,0 M7,7.5 L7,16.5 M7,12 C7,9 17,12 17,10.5";
    private const string RunLogData =
        "M4,6 L7,6 M10,6 L20,6 M4,12 L7,12 M10,12 L20,12 M4,18 L7,18 M10,18 L20,18";
    private const string TemplatesData =
        "M4,4 L11,4 L11,11 L4,11 Z M13,4 L20,4 L20,11 L13,11 Z M4,13 L11,13 L11,20 L4,20 Z M13,13 L20,13 L20,20 L13,20 Z";
    private const string SettingsData =
        "M12,12 m-3,0 a3,3 0 1,0 6,0 a3,3 0 1,0 -6,0 M12,3 L12,6 M12,18 L12,21 M3,12 L6,12 M18,12 L21,12 M5.6,5.6 L7.7,7.7 M16.3,16.3 L18.4,18.4 M18.4,5.6 L16.3,7.7 M7.7,16.3 L5.6,18.4";
    private const string InfoData =
        "M12,12 m-9,0 a9,9 0 1,0 18,0 a9,9 0 1,0 -18,0 M12,11 L12,16 M12,8 L12,8.5";
    private const string FeedbackData =
        "M4,5 L20,5 L20,16 L13,16 L9,20 L9,16 L4,16 Z";

    // 解析好的 Geometry，供 VM 直接赋给导航项 Icon。
    public static Geometry? Workspace { get; } = Parse(WorkspaceData);
    public static Geometry? Works { get; } = Parse(WorksData);
    public static Geometry? Git { get; } = Parse(GitData);
    public static Geometry? RunLog { get; } = Parse(RunLogData);
    public static Geometry? Templates { get; } = Parse(TemplatesData);
    public static Geometry? Settings { get; } = Parse(SettingsData);
    public static Geometry? Info { get; } = Parse(InfoData);
    public static Geometry? Feedback { get; } = Parse(FeedbackData);

    /// 解析为 Geometry；解析失败返回 null（不致命，图标位留空而非崩溃）。
    public static Geometry? Parse(string data)
    {
        try
        {
            return Geometry.Parse(data);
        }
        catch
        {
            return null;
        }
    }
}
