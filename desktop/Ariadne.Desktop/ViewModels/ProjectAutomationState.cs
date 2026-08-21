using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 当前项目 Auto Mode 的唯一桌面状态源。工作区、作品页与设置页共享实例，
/// 写入后必须以后端回读值提交 UI，项目切换时以代次隔离迟到响应。
///
/// # U213-E：<c>StateText</c>（「开启 / 关闭」那行字）已删除，别加回来
///
/// 界面形态从「整块可点按钮 + 右侧状态字」改成「标签 + <c>ToggleSwitch</c>」之后，
/// **开关本体就是状态呈现**：拨到哪边一眼可见，屏幕阅读器也由 ToggleSwitch
/// 自己播报选中态。再留一个状态字属性等于给「两处状态文字」留后门——
/// 用户这轮的原话是「做成悬浮文字和开关」，两件东西，不是三件。
/// 按本仓死代码判定：被取代的设计就删，不是「留着以后可能用」。
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
        ToggleCommand = new RelayCommand(() => _ = SetEnabledAsync(!IsEnabled), () => CanToggle);
        _displayNames.LanguageChanged += (_, _) =>
        {
            OnPropertyChanged(nameof(Label));
            OnPropertyChanged(nameof(BlockedReasonText));
        };
    }

    public string Label => _displayNames.Text("ui.settings.automation.auto_mode");

    /// <summary>
    /// 开关不可拨动时的**原因文案**。AGENTS.md「错误必须配文字」：
    /// 不能只把开关灰掉——灰掉的开关只说明「现在不行」，不说明「为什么」，
    /// 作者会以为功能坏了而不是「还没打开项目」。
    /// </summary>
    public string BlockedReasonText => _displayNames.Text("ui.automation.auto_mode.needs_project");

    public bool IsEnabled
    {
        get => _isEnabled;
        private set
        {
            if (SetProperty(ref _isEnabled, value))
            {
                // 开关绑的是 IsEnabledRequest，不带这一句权威值回来时开关不会跟着动。
                OnPropertyChanged(nameof(IsEnabledRequest));
            }
        }
    }

    /// <summary>
    /// 开关（<c>ToggleSwitch.IsChecked</c>）的**唯一**双向绑定端点（U213-E / U164-D）。
    ///
    /// # 为什么不让 View 直接双向绑 <see cref="IsEnabled"/>
    ///
    /// <see cref="IsEnabled"/> 的语义是「后端回读后确认的权威值」，它的 setter 是私有的，
    /// 而开关能力的守卫在 <see cref="CanToggle"/> 上（无项目 / 正在落盘时不许拨）。
    /// 直接把 <c>IsChecked</c> 双向绑到 <see cref="IsEnabled"/> 会**整条绕过守卫**：
    /// 开关照样能拨到「开」，`SetEnabledAsync` 里那句 `!HasProjectRoot` 直接 return，
    /// 于是**开关停在「开」上、后端什么都没发生**——一个静默的谎。
    ///
    /// # 拒绝路径为什么必须 <c>OnPropertyChanged</c>
    ///
    /// 这是本属性的**本体**，不是防御性冗余。`ToggleSwitch` 拨动时先把
    /// `IsChecked` 写成新值（LocalValue），再经绑定写进这里。若这里静静地不接受，
    /// 视图上那个开关就**停在用户拨到的位置**——与真实状态相反。
    /// 显式通知会让绑定回读 getter、把开关弹回真实值，作者才看得出「没生效」。
    /// ⚠️ 通知必须无条件发（`value == IsEnabled` 时才可省），
    /// 不要用 `SetProperty` 那套「值没变就不通知」——这里要通知的正是「值没变」。
    /// </summary>
    public bool IsEnabledRequest
    {
        get => IsEnabled;
        set
        {
            if (value == IsEnabled)
            {
                return;
            }
            if (!CanToggle)
            {
                // 弹回：告诉绑定「重新读一次 getter」，视图上的开关随之回到真实值。
                OnPropertyChanged();
                return;
            }
            // 落盘 + 后端回读由 SetEnabledAsync 负责；IsEnabled 变更会带出本属性的通知
            // （见 OnPropertyChanged 覆写），所以这里不预先乐观写入。
            _ = SetEnabledAsync(value);
        }
    }

    /// <summary>
    /// 开关能不能拨。与 <see cref="ToggleCommand"/> 的 CanExecute **共用同一个判据**，
    /// 保证「命令不可执行」与「开关灰掉」不会各说一套。
    /// </summary>
    public bool CanToggle => _backend.HasProjectRoot && !IsBusy;

    /// <summary>
    /// 拦住开关的原因是「没项目」而不是「正在忙」——只有前者要给文字。
    /// 忙只有一次后端往返那么短，给它配文案只会闪一下，比不给更糟。
    /// </summary>
    public bool IsBlockedWithoutProject => !_backend.HasProjectRoot;

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (SetProperty(ref _isBusy, value))
            {
                ToggleCommand.NotifyCanExecuteChanged();
                // CanToggle 是 IsBusy 的派生量，开关的 IsEnabled 绑的是它。
                OnPropertyChanged(nameof(CanToggle));
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
        // `_backend.HasProjectRoot` 是后端属性、自己不发通知，项目换了只有这里知道。
        // 缺这两句会让开关的灰/亮与「还没打开项目」那句话卡在上一个项目的状态上。
        OnPropertyChanged(nameof(CanToggle));
        OnPropertyChanged(nameof(IsBlockedWithoutProject));
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
}
