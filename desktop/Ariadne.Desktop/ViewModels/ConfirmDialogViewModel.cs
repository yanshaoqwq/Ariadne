using System.Collections.ObjectModel;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 未保存离开的用户选择。
public enum UnsavedLeaveChoice
{
    /// 先保存再离开。
    Save,

    /// 丢弃更改直接离开。
    Discard,

    /// 取消，留在当前页。
    Cancel,
}

/// 通用确认弹窗 ViewModel：标题 + 正文 + 一组按钮。
/// - 做成通用组件以便复用（未保存离开、删除确认、覆盖确认等共用同一套视图/样式）。
/// - 结果通过 <see cref="Completion"/> 以 int（按钮索引）异步返回；便捷工厂再把索引映射为语义枚举。
/// - 按钮回调只负责置结果；实际业务（保存等）由调用方在 await 之后处理。
public sealed class ConfirmDialogViewModel : ViewModelBase
{
    private readonly TaskCompletionSource<int> _completion =
        new(TaskCreationOptions.RunContinuationsAsynchronously);
    private string _inputText = string.Empty;

    public ConfirmDialogViewModel(string title, string message, IReadOnlyList<DialogButton> buttons)
    {
        Title = title;
        Message = message;
        Buttons = new ObservableCollection<DialogButton>(buttons);

        // 每个按钮点击后关闭弹窗并回填其结果索引；仅首个未完成的置值生效。
        for (var i = 0; i < Buttons.Count; i++)
        {
            var result = Buttons[i].ResultIndex;
            Buttons[i].Command = new RelayCommand(() => Complete(result));
        }
    }

    public string Title { get; }

    public string Message { get; }

    public ObservableCollection<DialogButton> Buttons { get; }

    /// <summary>可选输入框标签；非空时显示输入区（如新建项目名称）。</summary>
    public string? InputLabel { get; init; }

    public string? InputPlaceholder { get; init; }

    public bool HasInput => !string.IsNullOrWhiteSpace(InputLabel);

    public string InputText
    {
        get => _inputText;
        set => SetProperty(ref _inputText, value ?? string.Empty);
    }

    /// 取消/默认按钮索引：Esc 或点击遮罩时采用；-1 表示无。
    public int CancelResultIndex { get; init; } = -1;

    /// 主按钮（确认）结果索引；有输入时用于「非空才可确认」校验。
    public int ConfirmResultIndex { get; init; } = 0;

    /// 有输入时是否要求非空才能点确认。
    public bool RequireNonEmptyInput { get; init; }

    /// 弹窗结果任务：值为被点击按钮的 ResultIndex。
    public Task<int> Completion => _completion.Task;

    /// 由服务在 Esc / 点击遮罩时调用。
    public void Cancel()
    {
        if (CancelResultIndex >= 0)
        {
            Complete(CancelResultIndex);
        }
    }

    private void Complete(int result)
    {
        if (RequireNonEmptyInput
            && result == ConfirmResultIndex
            && HasInput
            && string.IsNullOrWhiteSpace(InputText))
        {
            return;
        }

        _completion.TrySetResult(result);
    }

    /// 「未保存离开」三按钮工厂：保存 / 不保存 / 取消。
    public static ConfirmDialogViewModel UnsavedLeave(DisplayNameService names)
    {
        var buttons = new[]
        {
            new DialogButton(names.Text("ui.dialog.unsaved.save"), DialogButtonVariant.Primary, (int)UnsavedLeaveChoice.Save),
            new DialogButton(names.Text("ui.dialog.unsaved.discard"), DialogButtonVariant.Danger, (int)UnsavedLeaveChoice.Discard),
            new DialogButton(names.Text("ui.dialog.unsaved.cancel"), DialogButtonVariant.Subtle, (int)UnsavedLeaveChoice.Cancel),
        };

        return new ConfirmDialogViewModel(
            names.Text("ui.dialog.unsaved.title"),
            names.Text("ui.dialog.unsaved.message"),
            buttons)
        {
            CancelResultIndex = (int)UnsavedLeaveChoice.Cancel,
        };
    }

    /// 新建项目：输入项目名称后确认。
    public static ConfirmDialogViewModel CreateProjectName(DisplayNameService names, string? defaultName = null)
    {
        var buttons = new[]
        {
            new DialogButton(names.Text("ui.dialog.create_project.confirm"), DialogButtonVariant.Primary, 0),
            new DialogButton(names.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
        };

        return new ConfirmDialogViewModel(
            names.Text("ui.dialog.create_project.title"),
            names.Text("ui.dialog.create_project.message"),
            buttons)
        {
            InputLabel = names.Text("ui.dialog.create_project.name_label"),
            InputPlaceholder = names.Text("ui.dialog.create_project.name_placeholder"),
            InputText = defaultName ?? names.Text("ui.dialog.create_project.default_name"),
            RequireNonEmptyInput = true,
            ConfirmResultIndex = 0,
            CancelResultIndex = 1,
        };
    }
}

/// 弹窗按钮样式变体，映射到主题里的 Button Classes。
public enum DialogButtonVariant
{
    Primary,
    Danger,
    Subtle,
}

/// 单个弹窗按钮：文案 + 样式变体 + 结果索引 + 点击命令（由 VM 注入）。
public sealed class DialogButton : ViewModelBase
{
    public DialogButton(string text, DialogButtonVariant variant, int resultIndex)
    {
        Text = text;
        Variant = variant;
        ResultIndex = resultIndex;
    }

    public string Text { get; }

    public DialogButtonVariant Variant { get; }

    public int ResultIndex { get; }

    /// 供视图绑定的样式类名（primary/danger/subtle）。
    public string VariantClass => Variant switch
    {
        DialogButtonVariant.Primary => "primary",
        DialogButtonVariant.Danger => "danger",
        _ => "subtle",
    };

    public bool IsPrimary => Variant == DialogButtonVariant.Primary;

    public bool IsDanger => Variant == DialogButtonVariant.Danger;

    public bool IsSubtle => Variant == DialogButtonVariant.Subtle;

    public RelayCommand? Command { get; set; }
}
