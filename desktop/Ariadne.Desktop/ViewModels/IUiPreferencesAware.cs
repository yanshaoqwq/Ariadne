using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop.ViewModels;

/// <summary>接收应用级个性化偏好；项目切换不得改变这些值。</summary>
public interface IUiPreferencesAware
{
    void ApplyUiPreferences(UiPreferences preferences);
}
