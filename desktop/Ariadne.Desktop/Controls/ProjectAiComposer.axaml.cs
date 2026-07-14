using Avalonia.Controls;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 项目 AI 一体输入区：Works / Workspace 共用，避免两页复制 markup。
/// 焦点描边由主题 <c>Border.ai-composer:focus-within</c> 统一。
/// </summary>
public partial class ProjectAiComposer : UserControl
{
    public ProjectAiComposer()
    {
        InitializeComponent();
    }
}
