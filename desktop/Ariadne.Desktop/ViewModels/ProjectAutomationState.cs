using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 当前项目 Auto Mode 的唯一桌面状态源。工作区、作品页与设置页共享实例，
/// 写入后必须以后端回读值提交 UI，项目切换时以代次隔离迟到响应。
/// </summary>
public sealed class ProjectAutomationState : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly SemaphoreSlim _gate = new(1, 1);
    private long _projectGeneration;
    private bool _isLoaded;
    private bool _isEnabled;
    private bool _isBusy;
    private string _statusText = string.Empty;

    public ProjectAutomationState(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        ToggleCommand = new RelayCommand(() => _ = SetEnabledAsync(!IsEnabled), CanToggle);
        _displayNames.LanguageChanged += (_, _) =>
        {
            OnPropertyChanged(nameof(Label));
            OnPropertyChanged(nameof(StateText));
        };
    }

    public string Label => _displayNames.Text("ui.settings.automation.auto_mode");
    public string StateText => _displayNames.Text(IsEnabled ? "ui.common.enabled" : "ui.common.disabled");

    public bool IsEnabled
    {
        get => _isEnabled;
        private set
        {
            if (SetProperty(ref _isEnabled, value))
            {
                OnPropertyChanged(nameof(StateText));
            }
        }
    }

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (SetProperty(ref _isBusy, value))
            {
                ToggleCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string StatusText
    {
        get => _statusText;
        private set
        {
            if (SetProperty(ref _statusText, value))
            {
                OnPropertyChanged(nameof(HasStatus));
            }
        }
    }

    public bool HasStatus => !string.IsNullOrWhiteSpace(StatusText);

    public RelayCommand ToggleCommand { get; }

    public void BeginProjectSession()
    {
        Interlocked.Increment(ref _projectGeneration);
        _isLoaded = false;
        IsBusy = false;
        IsEnabled = false;
        StatusText = string.Empty;
        ToggleCommand.NotifyCanExecuteChanged();
    }

    public async Task EnsureLoadedAsync(CancellationToken cancellationToken = default)
    {
        if (_isLoaded || !_backend.HasProjectRoot)
        {
            return;
        }

        var generation = Volatile.Read(ref _projectGeneration);
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(true);
        try
        {
            if (_isLoaded || generation != Volatile.Read(ref _projectGeneration))
            {
                return;
            }
            IsBusy = true;
            var settings = await _backend.GetAutomationSettingsAsync(cancellationToken).ConfigureAwait(true);
            if (generation == Volatile.Read(ref _projectGeneration))
            {
                ApplyBackendValue(settings.Budget.AutoModeEnabled);
                _isLoaded = true;
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (generation == Volatile.Read(ref _projectGeneration))
            {
                StatusText = UserFacingError.Format(ex, _displayNames);
            }
        }
        finally
        {
            if (generation == Volatile.Read(ref _projectGeneration))
            {
                IsBusy = false;
            }
            _gate.Release();
        }
    }

    public async Task SetEnabledAsync(bool enabled, CancellationToken cancellationToken = default)
    {
        if (!_backend.HasProjectRoot)
        {
            return;
        }

        var generation = Volatile.Read(ref _projectGeneration);
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(true);
        try
        {
            if (generation != Volatile.Read(ref _projectGeneration))
            {
                return;
            }
            IsBusy = true;
            StatusText = string.Empty;
            await _backend.SetAutoModeAsync(enabled, cancellationToken).ConfigureAwait(true);
            var authoritative = await _backend.GetAutomationSettingsAsync(cancellationToken).ConfigureAwait(true);
            if (generation == Volatile.Read(ref _projectGeneration))
            {
                ApplyBackendValue(authoritative.Budget.AutoModeEnabled);
                _isLoaded = true;
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (generation == Volatile.Read(ref _projectGeneration))
            {
                StatusText = UserFacingError.Format(ex, _displayNames);
                _isLoaded = false;
            }
        }
        finally
        {
            if (generation == Volatile.Read(ref _projectGeneration))
            {
                IsBusy = false;
            }
            _gate.Release();
        }
    }

    public void ApplyBackendValue(bool enabled)
    {
        IsEnabled = enabled;
        _isLoaded = true;
        StatusText = string.Empty;
    }

    private bool CanToggle() => _backend.HasProjectRoot && !IsBusy;
}
