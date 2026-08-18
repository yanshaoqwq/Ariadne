using Avalonia.Controls;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// U164-E：AutoMode + 项目 AI 输入框的组合面，Works / Workspace 共用。
///
/// <para>存在的理由是**位置**而非行为：AutoMode 按用户要求移到了对话框外
/// （「离开对话框，浮在对话框上，并没有任何边框」），而
/// <see cref="ProjectAiComposer"/> 是两页共用的。若让两个宿主各写一份 AutoMode，
/// 两页的它会各自漂移——而 `ProjectAiComposer` 本身就是为消除这种两页重复
/// 才抽出来的，同样的错不该重犯。</para>
/// </summary>
public partial class ProjectAiPanel : UserControl
{
    public ProjectAiPanel()
    {
        InitializeComponent();
    }
}
