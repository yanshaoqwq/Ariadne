using System.Reflection;
using System.Xml;
using System.Xml.Linq;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U213-A：**配置页读取失败时，凭据保护的补救入口跟着一起灰掉** —— 闭环死锁。
///
/// # 缺陷形态（用户亲报）
///
/// 配置页顶部报「六个分区未能加载」，同屏还有一条「凭据存储已锁定，保存密钥会失败，
/// 请配置本地主密码」的提示；而主密码输入框和「设置本地主密码」按钮**是灰的、点不动**。
/// ⇒ 读取失败 → 提示要设主密码 → 设主密码的入口被读取失败禁用。用户无路可走。
///
/// # 根因：`IsEnabled` 沿视觉树继承，子级压不回来
///
/// 权限页根 StackPanel 原先写着 `IsEnabled="{Binding IsPermissionsEditable}"`，
/// 而 `IsPermissionsEditable = CanSave("permissions")`，第一个条件就是
/// `_draftState.IsLoaded(section)`。凭据保护那一节落在这个 StackPanel 的子树里，
/// 于是整节随之禁用。
///
/// U176 当年在那个 Border 上留了一段注释，明确写着「IsEnabled 刻意**不绑**
/// IsPermissionsEditable……恰恰会在后端有问题的时候锁死用户唯一的补救出路」——
/// **判断完全正确，实现无效**：它只做到了「自己身上不绑」，而祖先绑了。
/// Avalonia 的 `IsEnabled` 是继承性属性（实际可交互性看 `IsEffectivelyEnabled`），
/// 子级再写 `IsEnabled="True"` 也压不回来。
///
/// # 判据为什么必须落在 `IsEffectivelyEnabled`
///
/// 缺陷版本里主密码 TextBox **自己的** `IsEnabled` 就是 `true`（它谁都没绑），
/// 断言 `IsEnabled` 会恒绿。这正是 U176 那套 9 条全绿的
/// <c>SecretProtectionRecoveryReachableTests</c> 漏掉这条的原因：
/// 它断言的是「客户端有方法」「XAML 里有按钮」「按钮接了命令」——
/// 三条在死锁状态下**全部成立**。
///
/// 本文件用两层判据，各自不可省：
/// - **结构层**（<see cref="RecoveryEntry_HasNoDisablingGateAnywhereOnItsAncestorChain"/>）：
///   解析 `SettingsPageView.axaml`，钉住补救入口的**祖先链上没有任何 `IsEnabled`**。
///   这一条钉的正是修复本体，且不依赖 headless 能否起窗口。
/// - **运行层**（<see cref="PermissionsSectionFailedToLoad_LeavesTheRecoveryEntryInteractive"/>）：
///   在 headless 里实体化整页、走真实的权限读取失败路径，断言
///   `IsEffectivelyEnabled`。这一条钉的是「用户手指落下去到底点不点得动」，
///   能拦住结构层看不见的失效方式（例如把门搬进 Style / 模板层）。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class SecretRecoveryEscapesLoadFailureTests
{
    /// <summary>凭据保护那一节里真正要能点的三个控件。</summary>
    private static readonly string[] RecoveryControlNames =
    {
        "SecretMasterPasswordInput",
        "SetSecretMasterPasswordButton",
        "AllowUnprotectedSecretsButton",
    };

    /// <summary>
    /// 结构判据：补救入口的**整条祖先链**上不许出现 `IsEnabled`。
    ///
    /// 为什么不是「祖先链上不许绑 IsPermissionsEditable」这种更窄的写法：
    /// 换成别的任何 `IsEnabled` 绑定（`IsPresetsEditable`、`HasProjectRoot`、
    /// 某个 busy 标志）都会重造同一个死锁，缺陷的形状是「补救入口被挡在别人的
    /// 前置条件后面」，与挡它的是哪个属性无关。
    ///
    /// 同时反向钉住一条：**禁用门必须仍然存在**，只是不在这条链上。
    /// 少了这半句，「把整页所有 IsEnabled 删干净」也能让上半句全绿——
    /// 那会放开一片没有读取基线的表单，等于把死锁换成脏写。
    /// </summary>
    [Fact]
    public void RecoveryEntry_HasNoDisablingGateAnywhereOnItsAncestorChain()
    {
        var path = ResolveDesktopSource("Views", "SettingsPageView.axaml");
        var document = XDocument.Load(path, LoadOptions.SetLineInfo);
        var xamlNamespace = XNamespace.Get("http://schemas.microsoft.com/winfx/2006/xaml");

        // 锚点先立：名字改了要当场红，而不是让下面的循环空转成绿。
        var anchored = RecoveryControlNames
            .Append("SecretProtectionSectionAnchor")
            .ToArray();

        foreach (var name in anchored)
        {
            var element = document
                .Descendants()
                .SingleOrDefault(candidate =>
                    (string?)candidate.Attribute(xamlNamespace + "Name") == name);

            Assert.True(
                element is not null,
                $"配置页里找不到 x:Name=\"{name}\"——凭据保护补救入口的守卫失去了着力点，"
                + "改名时请同步本用例而不是让它空转（U213-A）。");

            var gated = element!
                .Ancestors()
                .Where(ancestor => ancestor.Attribute("IsEnabled") is not null)
                .Select(ancestor =>
                    $"<{ancestor.Name.LocalName} IsEnabled=\"{ancestor.Attribute("IsEnabled")!.Value}\">"
                    + $"（第 {((IXmlLineInfo)ancestor).LineNumber} 行）")
                .ToList();

            Assert.True(
                gated.Count == 0,
                $"`{name}` 的祖先链上有禁用门：{string.Join('、', gated)}。"
                + "Avalonia 的 IsEnabled 沿视觉树继承，子级写 IsEnabled=\"True\" 压不回来，"
                + "⇒ 后端读取失败时用户唯一的补救出路会跟着故障一起灰掉（U213-A）。");
        }

        // 反向：权限页里那些「有读取基线才能编辑」的小节仍要各自带门。
        var avaloniaNamespace = XNamespace.Get("https://github.com/avaloniaui");
        var permissionsRoot = document
            .Descendants(avaloniaNamespace + "StackPanel")
            .Single(element =>
                (string?)element.Attribute("IsVisible") == "{Binding IsPermissionsSelected}");

        Assert.Null(permissionsRoot.Attribute("IsEnabled"));

        var gates = permissionsRoot
            .Descendants()
            .Count(element =>
                (string?)element.Attribute("IsEnabled") == "{Binding IsPermissionsEditable}");

        // 5 = 权限档案 / 能力 / 作用域覆盖 / 工具控制 / 路径根。
        // 数字写死是刻意的：加了小节忘了配门时这条会红，逼人当场决定
        // 「这一节该不该受读取基线约束」——那正是本缺陷缺失的那次决定。
        Assert.Equal(5, gates);
    }

    /// <summary>
    /// 运行判据：权限分区**真的读取失败**之后，主密码输入框与两个补救按钮的
    /// `IsEffectivelyEnabled` 必须仍为 `true`。
    ///
    /// 三组断言各自不可省：
    /// - 补救入口 `IsEffectivelyEnabled == true` ⇒ 死锁解除（本条是主判据）；
    /// - 权限档案下拉 `IsEffectivelyEnabled == false` ⇒ 证明测量的确实是「继承下来的
    ///   可交互性」，而且禁用门**没有被一并删掉**。少了这条，把整页 IsEnabled 删干净
    ///   也能让主判据全绿；
    /// - 节点预设那一节 `IsEffectivelyEnabled == false` ⇒ 门是**逐节**生效的，
    ///   而不是只剩权限档案那一处。
    ///
    /// ⚠️ 必须先把「高级权限控制」Expander 展开：折叠时那棵子树不挂进视觉树，
    /// `IsEffectivelyEnabled` 不会重算，会得到一个与产品行为无关的值。
    /// ⚠️ `session.Dispatch` 用 `Func&lt;Task&lt;T&gt;&gt;` 重载——`Func&lt;Task&gt;` 那个重载
    /// 会静默不执行 body 并报绿。
    /// </summary>
    [Fact]
    public async Task PermissionsSectionFailedToLoad_LeavesTheRecoveryEntryInteractive()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(
            async () =>
            {
                var names = DisplayNameService.LoadDefault();
                var vm = new SettingsPageViewModel(names, PermissionsLoadFailureBackend.Create());

                // 前提核实：读取真的失败了，而且失败范围正是 permissions。
                Assert.False(await vm.ReloadPermissionPresetProjectionForTestsAsync());
                Assert.False(vm.IsPermissionsEditable);
                // 预设读取本身成功，但 `IsPresetsEditable` 照样为假——这是刻意的：
                // 节点预设的权限投影建立在全局权限之上，半截读取不许暴露成
                // 「可保存」（`SettingsPermissionPresetCompositionTests` 的
                // PartialPermissionPresetLoad_DoesNotExposeMixedSavableProjection 钉着它）。
                // 记在这里是因为我一开始误以为它该是真，差点把这条设计写成缺陷。
                Assert.False(vm.IsPresetsEditable);

                vm.SelectTabForTests("permissions");
                vm.AreAdvancedPermissionsExpanded = true;

                var view = new SettingsPageView { DataContext = vm };
                var window = new Window { Width = 1280, Height = 900, Content = view };
                window.Show();
                await DrainDispatcherAsync();

                foreach (var name in RecoveryControlNames)
                {
                    var control = view.FindControl<Control>(name);
                    Assert.NotNull(control);
                    Assert.True(
                        control!.IsEffectivelyEnabled,
                        $"权限分区读取失败时 `{name}` 不可交互"
                        + $"（自身 IsEnabled={control.IsEnabled}，"
                        + $"实际可交互 IsEffectivelyEnabled={control.IsEffectivelyEnabled}）。"
                        + "用户会同时看到「分区未能加载」与「请配置本地主密码」，"
                        + "而唯一的补救入口点不动（U213-A）。"
                        + "注意：断言 IsEnabled 在缺陷版本里也为 true，恒绿。");
                }

                // 对照 1：受读取基线约束的表单必须仍然是灰的。
                var profile = view.FindControl<ComboBox>("PermissionProfileSelector");
                Assert.NotNull(profile);
                Assert.False(
                    profile!.IsEffectivelyEnabled,
                    "权限档案下拉在分区读取失败时仍可编辑——没有基线的表单被放开了，"
                    + "这会把死锁换成脏写。");

                // 对照 2：节点预设那一节绑的是自己的 IsPresetsEditable（U164-A 的刻意选择），
                // 此刻同样为假 ⇒ 证明「下沉到小节」的门是逐节生效的，不是被一并删光。
                var presets = view.FindControl<Control>("NodePresetsSectionAnchor");
                Assert.NotNull(presets);
                Assert.False(
                    presets!.IsEffectivelyEnabled,
                    "节点预设那一节在半截读取下仍可编辑——`IsPresetsEditable` 为假时"
                    + "它必须是灰的，否则会保存出一份基于残缺权限投影的预设。");

                window.Content = null;
                window.Close();
                await DrainDispatcherAsync();
                return true;
            },
            CancellationToken.None);
    }

    private static async Task DrainDispatcherAsync()
    {
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.SystemIdle);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var index = 0; index < 12 && walk is not null; index++)
        {
            var candidate = Path.Combine(
                new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }

            walk = walk.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }

    /// <summary>
    /// U213-A 的最小后端桩：**权限读取失败、预设读取成功**。
    ///
    /// 走的是 `LoadPermissionPresetSectionsAsync` 这条真实读取路径
    /// （`ReloadPermissionPresetProjectionForTestsAsync`），而不是直接去戳
    /// `_draftState`：门是由「这一分区有没有落基线」判的，只有真实读取失败
    /// 才会让 `IsLoaded("permissions")` 保持 false。
    ///
    /// 预设刻意成功：这样 `IsPresetsEditable` 为真，可以顺带钉住
    /// 「节点预设那一节不该被权限读取失败连坐」——它的源码注释一直这么写，
    /// 而祖先门让那句注释长期落空。
    ///
    /// `DispatchProxy` 宿主不能 `sealed`（运行时要派生它）。
    /// </summary>
    internal class PermissionsLoadFailureBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create()
        {
            return Create<IAriadneBackendClient, PermissionsLoadFailureBackend>()!;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            return targetMethod?.Name switch
            {
                "get_HasProjectRoot" => true,
                nameof(IAriadneBackendClient.GetNodePresetSettingsAsync) =>
                    Task.FromResult(new NodePresetSettings(
                        Array.Empty<NodeTypePreset>(), "gpt-x", 60_000, 0.5)),
                nameof(IAriadneBackendClient.GetPermissionsSettingsAsync) =>
                    Task.FromException<PermissionsSettings>(
                        new InvalidOperationException("permissions unavailable")),
                // 其余方法本用例不该碰：返回 null 会让生产代码吃 NRE（mock 违约），
                // 所以直接炸掉，谁多调了一条立刻看得见。
                _ => throw new NotSupportedException(targetMethod?.Name),
            };
        }
    }
}
