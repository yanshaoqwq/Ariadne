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
    /// U162-B：齿轮，不是太阳。
    /// 旧数据是「中心圆 + 8 根脱开的放射直线」——那是亮度/太阳的画法，
    /// 齿轮的辨识特征是**齿与轮缘相连的梯形凸起**，齿间有可见的齿根谷。
    /// 结构：6 颗齿的闭合外轮廓（齿顶 R=7.2 平台 + 齿根圆弧 A7.2 相连）+ 中心 r=3 圆孔。
    /// 6 齿是 16–20px 下的上限：本项目图标一律 Path+Stroke(1.6)，齿数再多齿根谷
    /// （现约 2.7px）就窄于笔宽，缩到 13px 会糊成一个圆环。
    /// 必须画成**闭合轮廓 + 中心孔**而非填充：Path.icon 基础样式是 Fill=Transparent，
    /// 依赖 Fill 的几何在这套样式下什么都看不见。
    /// ⚠️ 与 AriadneTheme.axaml 的 Ariadne.Icon.Settings 是同一份数据，
    /// 两处必须同步（有一致性用例钉住）——只改一处会让侧栏与 XAML 两个入口图标不一样。
    private const string SettingsData =
        "M9.89,5.11 L10,1.69 L14,1.69 L14.11,5.11 A7.2,7.2 0 0 1 16.91,6.73 L19.92,5.11 L21.93,8.58 L19.02,10.38 A7.2,7.2 0 0 1 19.02,13.62 L21.93,15.42 L19.92,18.89 L16.91,17.27 A7.2,7.2 0 0 1 14.11,18.89 L14,22.31 L10,22.31 L9.89,18.89 A7.2,7.2 0 0 1 7.09,17.27 L4.08,18.89 L2.07,15.42 L4.98,13.62 A7.2,7.2 0 0 1 4.98,10.38 L2.07,8.58 L4.08,5.11 L7.09,6.73 A7.2,7.2 0 0 1 9.89,5.11 Z M12,12 m-3,0 a3,3 0 1,0 6,0 a3,3 0 1,0 -6,0";
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
