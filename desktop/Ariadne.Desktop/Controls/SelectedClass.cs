using Avalonia;
using Avalonia.Controls;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 将布尔选中态绑定到标准 selected 样式类。
/// </summary>
public sealed class SelectedClass : AvaloniaObject
{
    public static readonly AttachedProperty<bool> IsSelectedProperty =
        AvaloniaProperty.RegisterAttached<SelectedClass, Control, bool>("IsSelected");

    static SelectedClass()
    {
        IsSelectedProperty.Changed.AddClassHandler<Control>((control, _) =>
        {
            control.Classes.Set("selected", control.GetValue(IsSelectedProperty));
        });
    }

    public static bool GetIsSelected(Control control)
    {
        return control.GetValue(IsSelectedProperty);
    }

    public static void SetIsSelected(Control control, bool value)
    {
        control.SetValue(IsSelectedProperty, value);
    }
}
