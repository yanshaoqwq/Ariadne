using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class ProjectAutomationStateTests
{
    [Fact]
    public async Task Toggle_CommitsBackendReadbackAndIsSharedAcrossPages()
    {
        var backend = AutomationBackend.Create();
        var state = new ProjectAutomationState(DisplayNameService.LoadDefault(), backend.Client);
        var workspace = new WorkspacePageViewModel(
            DisplayNameService.LoadDefault(), backend.Client, projectAutomation: state);
        var settings = new SettingsPageViewModel(
            DisplayNameService.LoadDefault(), backend.Client, projectAutomation: state);

        state.ApplyBackendValue(false);
        await state.SetEnabledAsync(true);

        Assert.True(state.IsEnabled);
        Assert.Same(state, workspace.ProjectAutomation);
        Assert.Equal(1, backend.SetCalls);
        Assert.Equal(1, backend.GetCalls);
        Assert.DoesNotContain(
            settings.GetType().GetProperties(),
            property => property.Name == "AutoModeEnabled");
    }

    [Fact]
    public async Task BeginProjectSession_InvalidatesLoadedValueAndReloadsNewProjectState()
    {
        var backend = AutomationBackend.Create();
        var state = new ProjectAutomationState(DisplayNameService.LoadDefault(), backend.Client);
        state.ApplyBackendValue(true);

        await state.EnsureLoadedAsync();
        Assert.Equal(0, backend.GetCalls);

        backend.Enabled = false;
        state.BeginProjectSession();
        await state.EnsureLoadedAsync();

        Assert.False(state.IsEnabled);
        Assert.Equal(1, backend.GetCalls);
    }

    /// <summary>
    /// AutoMode 在整个产品里**只有一份**，并且它是**开关**而不是可点整块。
    ///
    /// # 这条用例原来是红的，正解是改它而不是删断言
    ///
    /// 旧断言是「<c>Controls/ProjectAiComposer.axaml</c> 里含
    /// <c>ProjectAutomation.ToggleCommand</c>」。U164-E 把 AutoMode 从 composer
    /// 搬到了 <c>ProjectAiPanel</c>（对话框**外**），那个字符串在 composer 里
    /// 已经不存在 ⇒ 用例红了，而产品是对的。这是本仓反复出现的
    /// 「改了产品没同批改用例」。
    ///
    /// 删掉断言是错的：那样一来「哪天有人把 AutoMode 搬回 composer，
    /// 或者让作品页与工作区各写一份」不会有任何东西变红，而
    /// <c>ProjectAiPanel</c> 存在的**唯一**理由就是「AutoMode 与对话框的相对关系
    /// 只定义一次」。所以判据改成**反向、全局**的：扫全部 <c>.axaml</c>，
    /// 绑 <c>ProjectAutomation.*</c> 的文件必须**恰好一个**，且是那个共用控件。
    /// 位置换了它照样成立，重复了它一定红 —— 比钉死某个文件名更耐改。
    ///
    /// # 第二半：形态必须是开关
    ///
    /// U213-E 把满宽 <c>Button.subtle</c> +「选中琥珀底」换成
    /// 「标签 + <c>ToggleSwitch</c>」（用户原话：「做成悬浮文字和开关」）。
    /// 这里同时钉住：panel 里有 <c>ToggleSwitch</c>、且**不再**有把整块按钮
    /// 涂成选中态的 <c>SelectedClass.IsSelected</c> 绑定 —— 后者正是那块
    /// 满宽琥珀「卡片」的来源。
    ///
    /// ⚠️ 源码文本判据前必须先剥掉 XAML 注释：本仓刚踩过「变异标记/注释里
    /// 复述了被断言的字符串，于是 <c>Assert.Contains</c> 命中注释本身、变异全绿」。
    /// 做法抄 <c>GitSuccessMessageSurvivesRefreshTests.StripXamlComments</c>。
    /// </summary>
    [Fact]
    public void AutoModeToggle_IsASingleSwitchInTheSharedPanel_NotAClickableBlock()
    {
        var panel = StripXamlComments(
            File.ReadAllText(ResolveDesktopSource("Controls", "ProjectAiPanel.axaml")));

        // 形态：开关在，且绑的是带守卫的可写属性。
        Assert.Contains("<ToggleSwitch", panel, StringComparison.Ordinal);
        Assert.Contains(
            "IsChecked=\"{Binding ProjectAutomation.IsEnabledRequest, Mode=TwoWay}\"",
            panel,
            StringComparison.Ordinal);
        // 反向：不再有「整块可点 + 选中涂底」那套 —— 那是满宽琥珀卡片的来源。
        Assert.DoesNotContain(
            "SelectedClass.IsSelected=\"{Binding ProjectAutomation",
            panel,
            StringComparison.Ordinal);
        Assert.DoesNotContain(
            "Command=\"{Binding ProjectAutomation.ToggleCommand}\"",
            panel,
            StringComparison.Ordinal);

        // 唯一性：全仓只有这一个视图碰 ProjectAutomation。
        var owners = Directory
            .EnumerateFiles(ResolveDesktopRoot(), "*.axaml", SearchOption.AllDirectories)
            .Where(path => StripXamlComments(File.ReadAllText(path))
                .Contains("ProjectAutomation.", StringComparison.Ordinal))
            .Select(path => Path.GetFileName(path))
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToList();
        Assert.Equal(new[] { "ProjectAiPanel.axaml" }, owners);

        // 设置页仍然没有自己那份草稿状态（U164-E 之前的旧形态）。
        var settings = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));
        Assert.DoesNotContain("AutoModeEnabled", settings, StringComparison.Ordinal);
    }

    /// <summary>
    /// **无项目时拨开关必须被挡下，并且开关要弹回真实值。**
    ///
    /// # 这是本轮最容易做错的一处
    ///
    /// 换成 <c>ToggleSwitch</c> 后最自然的写法是把 <c>IsChecked</c> 双向绑到
    /// <see cref="ProjectAutomationState.IsEnabled"/>。那会**整条绕过守卫**：
    /// 能力判据在 <see cref="ProjectAutomationState.CanToggle"/> 上
    /// （<c>HasProjectRoot &amp;&amp; !IsBusy</c>），而 <c>SetEnabledAsync</c>
    /// 里那句 <c>!HasProjectRoot</c> 只是 <c>return</c> —— 不发通知、不报错。
    /// 于是开关**停在用户拨到的位置**，与真实状态相反：一个静默的谎。
    ///
    /// # 判据为什么是「PropertyChanged 被发出」而不是「后端没被调用」
    ///
    /// 「后端没被调用」在**缺陷版本里也成立**（`SetEnabledAsync` 自己就 return 了）
    /// ⇒ 那是一条空测。真正决定用户可见结果的一环是：VM 有没有告诉绑定
    /// 「重新读一次 getter」。发了，开关弹回；不发，开关留在错的位置上。
    /// 所以这里断言通知真的发生，且读回来是 false。
    /// </summary>
    [Fact]
    public void IsEnabledRequest_WithoutProject_IsRejectedAndSnapsBack()
    {
        var backend = AutomationBackend.Create();
        backend.HasProject = false;
        var state = new ProjectAutomationState(DisplayNameService.LoadDefault(), backend.Client);

        Assert.False(state.CanToggle);
        Assert.True(state.IsBlockedWithoutProject);

        var notified = new List<string?>();
        state.PropertyChanged += (_, args) => notified.Add(args.PropertyName);

        state.IsEnabledRequest = true;

        // 开关弹回：绑定必须被叫去重读 getter，否则界面停在「开」。
        Assert.Contains(nameof(ProjectAutomationState.IsEnabledRequest), notified);
        Assert.False(state.IsEnabledRequest);
        Assert.False(state.IsEnabled);
        Assert.Equal(0, backend.SetCalls);
    }

    /// <summary>
    /// 有项目时同一个属性必须真的落盘 —— 否则上面那条可以用「永远拒绝」通过。
    ///
    /// 判据取「后端 set 收到了那一次调用」+「权威值以后端回读为准」，
    /// 不取「属性等于我刚写的值」：后者在乐观写入的实现下也成立，
    /// 而本产品刻意不乐观写入（值只能来自后端回读）。
    /// </summary>
    [Fact]
    public async Task IsEnabledRequest_WithProject_WritesThroughAndCommitsReadback()
    {
        var backend = AutomationBackend.Create();
        var state = new ProjectAutomationState(DisplayNameService.LoadDefault(), backend.Client);
        state.ApplyBackendValue(false);

        state.IsEnabledRequest = true;
        // 等到 IsBusy 落回（CanToggle 恢复为 true）＝ SetEnabledAsync 的 finally 已跑完。
        // 不造测试专用 API：CanToggle 是产品自己就有的公开判据。
        await WaitUntilAsync(() => backend.SetCalls >= 1 && state.CanToggle);

        Assert.Equal(1, backend.SetCalls);
        Assert.Equal(1, backend.GetCalls);
        Assert.True(state.IsEnabledRequest);
        Assert.True(state.IsEnabled);
    }

    /// <summary>
    /// 禁用理由必须有真文案 —— AGENTS.md「错误必须配文字」。
    ///
    /// 三份语言包都要有这个键：<c>DisplayNameService</c> 缺键时返回
    /// <c>[key]</c>（zh）或静默回落到中文（en/ja），两种都不报错，
    /// **唯一的发现途径就是这条断言**。
    /// </summary>
    [Fact]
    public void BlockedReasonText_HasRealCopy_NotAKeyPlaceholder()
    {
        var backend = AutomationBackend.Create();
        backend.HasProject = false;
        var state = new ProjectAutomationState(DisplayNameService.LoadDefault(), backend.Client);

        Assert.False(string.IsNullOrWhiteSpace(state.BlockedReasonText));
        Assert.DoesNotContain("[ui.", state.BlockedReasonText, StringComparison.Ordinal);
        // 界面上这句话必须真的有渲染位，否则又是「有属性、界面零消费」。
        var panel = StripXamlComments(
            File.ReadAllText(ResolveDesktopSource("Controls", "ProjectAiPanel.axaml")));
        Assert.Contains(
            "Text=\"{Binding ProjectAutomation.BlockedReasonText}\"",
            panel,
            StringComparison.Ordinal);
        Assert.Contains(
            "IsVisible=\"{Binding ProjectAutomation.IsBlockedWithoutProject}\"",
            panel,
            StringComparison.Ordinal);
    }

    /// <summary>剥掉 <c>&lt;!-- --&gt;</c> 注释，只留真实标记。</summary>
    private static string StripXamlComments(string markup)
        => System.Text.RegularExpressions.Regex.Replace(
            markup, "<!--.*?-->", string.Empty,
            System.Text.RegularExpressions.RegexOptions.Singleline);

    private static async Task WaitUntilAsync(Func<bool> condition)
    {
        for (var attempt = 0; attempt < 200 && !condition(); attempt++)
        {
            await Task.Delay(10);
        }
        Assert.True(condition(), "等待超时：异步落盘没有在预期时间内完成。");
    }

    private static string ResolveDesktopRoot()
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(directory.FullName, "desktop", "Ariadne.Desktop");
            if (Directory.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }
        throw new DirectoryNotFoundException("desktop/Ariadne.Desktop");
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }
        throw new FileNotFoundException(string.Join('/', parts));
    }

    private class AutomationBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool Enabled { get; set; }

        /// <summary>
        /// 默认 true（既有用例都假设有项目）；无项目那条守卫用例把它设 false。
        /// 这不是「测试专用 API」——它是 mock 自己的状态，生产代码看不到。
        /// </summary>
        public bool HasProject { get; set; } = true;

        public int GetCalls { get; private set; }
        public int SetCalls { get; private set; }

        public static AutomationBackend Create()
        {
            var client = Create<IAriadneBackendClient, AutomationBackend>();
            var backend = (AutomationBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }
            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return HasProject;
            }

            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.GetAutomationSettingsAsync) => ReadSettings(),
                nameof(IAriadneBackendClient.SetAutoModeAsync) => SetEnabled((bool)args![0]!),
                _ => targetMethod.ReturnType.IsValueType
                    ? Activator.CreateInstance(targetMethod.ReturnType)
                    : null,
            };
            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }
            return value;
        }

        private AutomationSettings ReadSettings()
        {
            GetCalls++;
            return new AutomationSettings(
                new BudgetStatus(0, 0, 0, Enabled),
                Array.Empty<ConfirmationPolicySetting>());
        }

        private object? SetEnabled(bool enabled)
        {
            SetCalls++;
            Enabled = enabled;
            return null;
        }
    }
}
