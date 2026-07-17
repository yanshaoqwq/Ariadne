using System.Windows.Input;

namespace Ariadne.Desktop.ViewModels;

public sealed class RelayCommand : ICommand
{
    private readonly Action _execute;
    private readonly Func<bool>? _canExecute;

    public RelayCommand(Action execute, Func<bool>? canExecute = null)
    {
        _execute = execute;
        _canExecute = canExecute;
    }

    public event EventHandler? CanExecuteChanged;

    public bool CanExecute(object? parameter)
    {
        return _canExecute?.Invoke() ?? true;
    }

    public void Execute(object? parameter)
    {
        _execute();
    }

    /// <summary>
    /// 供键盘、代码路径等非 Avalonia CommandSource 调用：先检查当前能力，再执行动作。
    /// </summary>
    public bool TryExecute(object? parameter = null)
    {
        if (!CanExecute(parameter))
        {
            return false;
        }

        Execute(parameter);
        return true;
    }

    public void NotifyCanExecuteChanged()
    {
        CanExecuteChanged?.Invoke(this, EventArgs.Empty);
    }
}
