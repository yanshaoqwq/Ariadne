using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U146：5 处把「列表」塞进多行文本框，单条无法校验。
///
/// 三条后果：(1) 单条路径无法校验——整个文本框只能整体接受或拒绝，用户不知道哪一行错；
/// (2) 看不出哪条已失效（目录被删/移动后无任何提示）；
/// (3) 首尾空格静默出错——<c>" /home/x"</c> 与 <c>"/home/x"</c> 在文本里看不出区别，
/// 但作为路径是两个值。这些是**权限边界配置**，静默出错等于权限判定与用户意图不符。
///
/// **判据落在真实保存内容与真实脏状态上**，不是「chip 列表非空」：
/// 后者在投影写回没接线时照样能过（chip 加进集合了，宿主字符串却没变），
/// 那正是「改了但保存按钮不亮」「保存丢内容」这类缺陷的藏身处。
/// </summary>
public sealed class PathListChipTests
{
    /// <summary>
    /// **首尾空格必须在加入时就被吃掉，且保存出去的值不含空格。**
    ///
    /// 判据取 <c>ToPolicy()</c> 的产物（真正发给后端的东西），
    /// 而不是 chip 自己的 Path——后者只证明 UI 显示对了。
    /// </summary>
    [Fact]
    public void AddingPathWithSurroundingWhitespace_SerializesTrimmedValue()
    {
        var profile = NewScopeProfile();

        Assert.True(profile.ReadableRootChips.TryAdd("  /home/author/novel  ", out var failure), failure);

        var chip = Assert.Single(profile.ReadableRootChips.Chips);
        Assert.Equal("/home/author/novel", chip.PathText);

        // 真实保存内容：这是权限判定实际会用到的值。
        var policy = profile.ToPolicy();
        Assert.Equal(new[] { "/home/author/novel" }, policy.ReadableFileRoots);
        Assert.DoesNotContain(policy.ReadableFileRoots, path => path != path.Trim());

        // 宿主字符串（序列化形态 + 脏状态快照的载体）也必须已同步。
        Assert.Equal("/home/author/novel", profile.ReadableRootsText);
    }

    /// <summary>删掉一个 chip 后，序列化结果里不能再有它。</summary>
    [Fact]
    public void RemovingChip_DropsPathFromSerializedOutput()
    {
        var profile = NewScopeProfile();
        Assert.True(profile.ReadableRootChips.TryAdd("/home/author/a", out _));
        Assert.True(profile.ReadableRootChips.TryAdd("/home/author/b", out _));
        Assert.Equal(2, profile.ReadableRootChips.Chips.Count);

        var doomed = Assert.Single(
            profile.ReadableRootChips.Chips,
            chip => chip.PathText == "/home/author/a");

        // 走 chip 自己的移除命令：这是用户点 × 时真正执行的路径。
        Assert.True(doomed.RemoveCommand.TryExecute());

        var policy = profile.ToPolicy();
        Assert.Equal(new[] { "/home/author/b" }, policy.ReadableFileRoots);
        Assert.DoesNotContain("/home/author/a", profile.ReadableRootsText, StringComparison.Ordinal);
    }

    /// <summary>
    /// **通过 chip 改动后页面必须进入脏状态。**
    ///
    /// 这是最容易漏的一环——chip 集合是新加的对象，如果它绕开宿主的字符串 setter
    /// 直接维护自己的列表，界面上看着改了，`HasUnsavedChanges` 却仍是 false，
    /// 保存按钮不亮，用户的改动在离开页面时被静默丢弃。
    ///
    /// 判据取 <c>SettingsPageViewModel.HasUnsavedChanges</c>（驱动保存按钮那个属性），
    /// 而不是「chip 集合变了没有」。
    /// </summary>
    [Fact]
    public async Task AddingChip_MarksPageDirty()
    {
        var backend = ChipBackend.Create();
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        Assert.False(vm.HasUnsavedChanges);

        Assert.True(vm.ReadableRootChips.TryAdd("/home/author/added", out var failure), failure);

        Assert.True(
            vm.HasUnsavedChanges,
            "通过 chip 加了一条路径后页面必须为脏，否则保存按钮不亮、改动会被静默丢弃");
        Assert.Contains("/home/author/added", vm.ReadableRootsText, StringComparison.Ordinal);
    }

    /// <summary>删 chip 同样要标脏——加和删是两条独立的写回路径。</summary>
    [Fact]
    public async Task RemovingChip_MarksPageDirty()
    {
        var backend = ChipBackend.Create();
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        Assert.False(vm.HasUnsavedChanges);

        var chip = Assert.Single(
            vm.ReadableRootChips.Chips,
            item => item.PathText == ChipBackend.SeededRoot);
        Assert.True(chip.RemoveCommand.TryExecute());

        Assert.True(vm.HasUnsavedChanges, "删掉一条路径后页面必须为脏");
        Assert.DoesNotContain(ChipBackend.SeededRoot, vm.ReadableRootsText, StringComparison.Ordinal);
    }

    /// <summary>不存在的目录被标为失效，且失效说明有文字（不能只置灰）。</summary>
    [Fact]
    public async Task MissingDirectory_IsMarkedUnavailableWithText()
    {
        var profile = NewScopeProfile();
        var missing = Path.Combine(Path.GetTempPath(), "ariadne-u146-missing-" + Guid.NewGuid().ToString("N"));
        Assert.False(Directory.Exists(missing));

        Assert.True(profile.ReadableRootChips.TryAdd(missing, out _));
        var chip = Assert.Single(profile.ReadableRootChips.Chips);

        // 体检是异步（磁盘 IO 不能在 UI 线程同步跑），等结论落地。
        await WaitUntilAsync(() => chip.Health != PathChipHealth.Unknown);

        Assert.Equal(PathChipHealth.Missing, chip.Health);
        Assert.True(chip.IsUnavailable);
        Assert.True(
            chip.HasUnavailableText,
            "失效 chip 必须配文字说明，只置灰的话用户无从知道是被删了还是指错了");
        Assert.False(chip.UnavailableText.StartsWith('['), "失效说明的 display_name key 缺失");
    }

    /// <summary>
    /// 存在的目录判为健康；指到**文件**上单独成一类——
    /// 「目录不存在」与「这是文件」的出路不同，混成一类等于让用户猜。
    /// </summary>
    [Fact]
    public async Task ExistingDirectoryIsHealthy_AndFileIsNotADirectory()
    {
        var root = Directory.CreateTempSubdirectory("ariadne-u146-");
        try
        {
            var filePath = Path.Combine(root.FullName, "not-a-dir.txt");
            await File.WriteAllTextAsync(filePath, "x");

            var profile = NewScopeProfile();
            Assert.True(profile.ReadableRootChips.TryAdd(root.FullName, out _));
            Assert.True(profile.ReadableRootChips.TryAdd(filePath, out _));

            var dirChip = Assert.Single(profile.ReadableRootChips.Chips, c => c.PathText == root.FullName);
            var fileChip = Assert.Single(profile.ReadableRootChips.Chips, c => c.PathText == filePath);
            await WaitUntilAsync(() =>
                dirChip.Health != PathChipHealth.Unknown && fileChip.Health != PathChipHealth.Unknown);

            Assert.Equal(PathChipHealth.Healthy, dirChip.Health);
            Assert.False(dirChip.IsUnavailable);
            Assert.Equal(PathChipHealth.NotADirectory, fileChip.Health);
            Assert.True(fileChip.IsUnavailable);
        }
        finally
        {
            root.Delete(recursive: true);
        }
    }

    /// <summary>
    /// 空行、重复项、非法路径都要**当场拒绝并给出原因**——
    /// 这是「单条无法校验」那条后果的正解：错的那条根本进不了列表。
    /// </summary>
    [Fact]
    public void InvalidEntries_AreRejectedWithReasonAndNeverEnterList()
    {
        var profile = NewScopeProfile();
        Assert.True(profile.ReadableRootChips.TryAdd("/home/author/ok", out _));

        // 空行
        Assert.False(profile.ReadableRootChips.TryAdd("   ", out var emptyFailure));
        Assert.False(string.IsNullOrWhiteSpace(emptyFailure));

        // 相对路径进不了「必须绝对」的字段
        Assert.False(profile.ReadableRootChips.TryAdd("relative/path", out var relativeFailure));
        Assert.False(string.IsNullOrWhiteSpace(relativeFailure));

        // 含 .. 的绝对路径（越权风险）
        Assert.False(profile.ReadableRootChips.TryAdd("/home/author/../etc", out var parentFailure));
        Assert.False(string.IsNullOrWhiteSpace(parentFailure));

        // 重复项——含首尾空格的同一条也算重复，因为 trim 后等值
        Assert.False(profile.ReadableRootChips.TryAdd("  /home/author/ok ", out var dupFailure));
        Assert.False(string.IsNullOrWhiteSpace(dupFailure));

        Assert.Single(profile.ReadableRootChips.Chips);
        Assert.Equal(new[] { "/home/author/ok" }, profile.ToPolicy().ReadableFileRoots);
    }

    /// <summary>
    /// 输入框提交：成功清空输入框，失败保留原文并显示错误。
    /// 失败还留着原文是刻意的——清掉会让用户重新敲一遍才能改那一个笔误。
    /// </summary>
    [Fact]
    public void DraftCommit_ClearsOnSuccessAndKeepsTextWithErrorOnFailure()
    {
        var profile = NewScopeProfile();
        var list = profile.ReadableRootChips;

        list.DraftPath = "not-absolute";
        Assert.False(list.TryCommitDraft());
        Assert.Equal("not-absolute", list.DraftPath);
        Assert.True(list.HasDraftError);
        Assert.Empty(list.Chips);

        list.DraftPath = "/home/author/good";
        Assert.True(list.TryCommitDraft());
        Assert.Equal(string.Empty, list.DraftPath);
        Assert.False(list.HasDraftError);
        Assert.Single(list.Chips);
    }

    /// <summary>
    /// 宿主字符串被旧路径改写后（继承投影 / 推荐默认值 / 后端载入），chip 必须跟着变。
    /// 漏掉这条同步的表现是「界面显示旧路径，保存下去的是新路径」——最难查的一类不一致。
    /// </summary>
    [Fact]
    public void HostStringRewrite_ResyncsChipProjection()
    {
        var profile = NewScopeProfile();
        Assert.True(profile.ReadableRootChips.TryAdd("/home/author/first", out _));

        profile.ReadableRootsText = string.Join(
            Environment.NewLine,
            "/home/author/second",
            "/home/author/third");

        Assert.Equal(
            new[] { "/home/author/second", "/home/author/third" },
            profile.ReadableRootChips.Chips.Select(chip => chip.PathText));
    }

    /// <summary>
    /// Git 忽略路径走相对路径规则：绝对路径要被拒，合法相对路径要能进。
    /// 这条守住「同一份 chip 组件在两种取值域下的判定不能串味」。
    /// </summary>
    [Fact]
    public async Task IgnoredPaths_UseRelativeRulesAndRejectAbsolute()
    {
        var backend = ChipBackend.Create();
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());

        Assert.False(vm.IgnoredPathChips.TryAdd("/absolute/path", out var absFailure));
        Assert.False(string.IsNullOrWhiteSpace(absFailure));

        Assert.True(vm.IgnoredPathChips.TryAdd("  .cache/  ", out var failure), failure);
        var chip = Assert.Single(vm.IgnoredPathChips.Chips);
        Assert.Equal(".cache", chip.PathText);
        Assert.Equal(".cache", vm.IgnoredPathsText);
    }

    /// <summary>
    /// 历史配置里的非法值必须**显示出来**而不是被静默过滤掉。
    /// 过滤掉会让它在界面上消失、却仍在保存时报错——用户永远找不到那一条。
    /// </summary>
    [Fact]
    public void PreexistingInvalidValue_StaysVisibleAsChip()
    {
        var profile = NewScopeProfile();
        profile.ReadableRootsText = string.Join(
            Environment.NewLine,
            "/home/author/valid",
            "legacy-relative-value");

        Assert.Equal(
            new[] { "/home/author/valid", "legacy-relative-value" },
            profile.ReadableRootChips.Chips.Select(chip => chip.PathText));
    }

    private static PermissionScopeProfileViewModel NewScopeProfile()
    {
        var empty = new PermissionPolicy(
            false, false, false, false, false,
            Array.Empty<string>(),
            Array.Empty<string>());
        return new PermissionScopeProfileViewModel(
            "workflow_nodes",
            "工作流节点",
            empty,
            empty,
            () => { },
            browse: null,
            displayNames: DisplayNameService.LoadDefault());
    }

    private static async Task WaitUntilAsync(Func<bool> predicate)
    {
        for (var attempt = 0; attempt < 300 && !predicate(); attempt++)
        {
            await Task.Delay(10);
        }
        Assert.True(predicate(), "等待条件超时");
    }

    /// <summary>只回答 chip 测试需要的两个 section，其余一律 NotSupported。</summary>
    // DispatchProxy 要在运行时派生宿主类型，所以**不能 sealed**。
    private class ChipBackend : DispatchProxy
    {
        internal const string SeededRoot = "/seeded/root";

        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, ChipBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            switch (targetMethod?.Name)
            {
                case "get_HasProjectRoot":
                    return true;
                case nameof(IAriadneBackendClient.GetNodePresetSettingsAsync):
                    return Task.FromResult(new NodePresetSettings(
                        Array.Empty<NodeTypePreset>(),
                        string.Empty,
                        30_000,
                        1));
                case nameof(IAriadneBackendClient.GetPermissionsSettingsAsync):
                    var policy = new PermissionPolicy(
                        false, false, false, false, false,
                        new[] { SeededRoot },
                        new[] { SeededRoot });
                    return Task.FromResult(new PermissionsSettings(
                        policy,
                        new Dictionary<string, PermissionPolicy?>
                        {
                            ["workflow_nodes"] = null,
                            ["project_ai"] = null,
                        },
                        new Dictionary<string, IReadOnlyDictionary<string, bool?>>()));
                default:
                    return UnsupportedTask(targetMethod);
            }
        }

        private static object? UnsupportedTask(MethodInfo? method)
        {
            if (method is null || method.ReturnType == typeof(void))
            {
                return null;
            }
            if (method.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (method.ReturnType.IsGenericType
                && method.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = method.ReturnType.GetGenericArguments()[0];
                return typeof(Task)
                    .GetMethods(BindingFlags.Public | BindingFlags.Static)
                    .Single(candidate => candidate.Name == nameof(Task.FromException)
                        && candidate.IsGenericMethodDefinition)
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object[] { new NotSupportedException(method.Name) });
            }
            return method.ReturnType.IsValueType ? Activator.CreateInstance(method.ReturnType) : null;
        }
    }
}
