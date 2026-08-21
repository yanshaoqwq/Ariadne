using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Media;
using Avalonia.Styling;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U184：通用控件必须体现本项目的设计语言（品牌青绿），而不是 FluentTheme 默认蓝。
///
/// **判据刻意取「真实运行态的画刷值」而不是源码文本**。
/// 断言 <c>AriadneTheme.axaml</c> 里出现 <c>AccentPrimary</c> 这类源码判据在本仓库
/// 是无效的：把资源键改名、改到另一个 token、甚至整条键写成 Fluent 不认识的名字，
/// 源码里那个字符串照样在，用例照样绿。**只有把控件实体化、读它模板部件上
/// 真正生效的那个 Brush，才能区分「写了」与「生效了」。**
///
/// 这类缺陷在本仓库有明确前科（AGENTS.md「测试全绿 ≠ 功能可用」）：
/// 主题里写了样式而实际不生效的形态至少有三种，全都能骗过源码断言 ——
/// <list type="bullet">
///   <item>选择器优先级不够（<c>/template/</c> 是 Style=3，压不住
///     StyleTrigger=1 / Template=2，本文件测的两个控件正是这种）；</item>
///   <item>资源键名拼错 ⇒ 死键，静默无效果；</item>
///   <item>类名全仓无人挂载 ⇒ 死样式（U152）。</item>
/// </list>
///
/// **期望值从主题令牌本身取**（<c>TryFindResource("Ariadne.Color.*")</c>），
/// 不写死色值：既满足「颜色只能来自 token」，也让本用例在设计师调整青绿色号后
/// 依然成立 —— 它钉的是「控件跟随主题令牌」这个性质，不是某个具体色号。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class ControlSurfaceThemingTests
{
    /// <summary>
    /// FluentTheme 未被覆盖时那块默认蓝（实测 <c>#ff0078d7</c>，即 SystemAccentColor）。
    ///
    /// **这里写死一个色值是刻意的、且不违反「颜色只能来自 token」**：
    /// 它不是本产品要用的颜色，而是要**排除**的外来值。
    /// 把它写成常量的价值是：即便将来有人把主题令牌本身错改成近似的蓝，
    /// 上面那条「等于令牌」的断言会被骗过，而这条「不等于 Fluent 蓝」不会。
    /// 两条一起才覆盖住「配色跑偏」的两个方向。
    /// </summary>
    private static readonly Color FluentDefaultBlue = Color.Parse("#ff0078d7");

    /// <summary>
    /// 选中态的勾选框底色必须是品牌青绿。
    ///
    /// **这是 U184 的核心断言**：全仓 39 处 CheckBox 在修复前勾上就是一块 Fluent 蓝
    /// （主题里关于 CheckBox 的唯一一条样式只设了字体族，配色从未被碰过）。
    /// 判据落在 <c>Border#NormalRectangle</c> 的 Background 上 —— 那是用户
    /// 真正看见的那 20×20 方块，而不是 CheckBox 宿主的 Background
    /// （宿主那个是 Transparent，永远「正确」，拿它当判据等于什么都没测）。
    /// </summary>
    [Fact]
    public async Task CheckedCheckBoxFillFollowsBrandAccentNotFluentBlue()
    {
        await RunHeadlessAsync(async () =>
        {
            var box = new CheckBox { Content = "U184", IsChecked = true };
            var window = new Window { Width = 320, Height = 200, Content = box };
            window.Show();
            await Drain();

            var expected = ResolveThemeColor(box, "Ariadne.Color.AccentPrimary");
            var actual = ResolvePartColor(box, "NormalRectangle");

            Assert.NotEqual(FluentDefaultBlue, actual);
            Assert.Equal(expected, actual);

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// 勾号必须走 <c>TextOnAccent</c>，不是硬编码 <c>White</c>。
    ///
    /// <c>UI组件状态表.md:46</c> 明确要求「现有主按钮多用 text-white；
    /// 应改读 text-on-accent（深色/纸墨主题下 on-accent 已按对比调过）」——
    /// 勾号是同一类「压在强调色上的前景」，同一条规定。
    /// Fluent 默认这九个键全是字面量 <c>White</c>。
    /// </summary>
    [Fact]
    public async Task CheckedGlyphFollowsTextOnAccentToken()
    {
        await RunHeadlessAsync(async () =>
        {
            var box = new CheckBox { Content = "U184", IsChecked = true };
            var window = new Window { Width = 320, Height = 200, Content = box };
            window.Show();
            await Drain();

            var expected = ResolveThemeColor(box, "Ariadne.Color.TextOnAccent");
            var glyph = box.GetVisualDescendants().OfType<Avalonia.Controls.Shapes.Path>()
                .FirstOrDefault(p => p.Name == "CheckGlyph");
            Assert.NotNull(glyph);
            var actual = (glyph!.Fill as ISolidColorBrush)?.Color;
            Assert.NotNull(actual);
            Assert.Equal(expected, actual!.Value);

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// 未选中态的描边必须来自主题边线令牌。
    ///
    /// 这是 39 处里**最常见的形态**（权限清单、追踪开关多数默认关），
    /// 也是最容易被漏掉的一态 —— 修「选中态是蓝的」时很自然只改 Checked 那几个键。
    /// Fluent 默认 <c>#99000000</c> 是硬编码半透明黑：暗色主题下等于黑框贴黑底，
    /// 框在哪都看不出来。
    /// </summary>
    [Fact]
    public async Task UncheckedCheckBoxStrokeFollowsThemeBorderToken()
    {
        await RunHeadlessAsync(async () =>
        {
            var box = new CheckBox { Content = "U184", IsChecked = false };
            var window = new Window { Width = 320, Height = 200, Content = box };
            window.Show();
            await Drain();

            var expected = ResolveThemeColor(box, "Ariadne.Color.BorderDefault");
            var rect = box.GetVisualDescendants().OfType<Border>()
                .FirstOrDefault(b => b.Name == "NormalRectangle");
            Assert.NotNull(rect);
            var actual = (rect!.BorderBrush as ISolidColorBrush)?.Color;
            Assert.NotNull(actual);
            Assert.Equal(expected, actual!.Value);

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// 标签文字色必须来自主题令牌，**不能是 Fluent 那个硬编码 <c>Black</c>**。
    ///
    /// 这与「选中态是蓝的」是**两个独立缺陷**，修一个不会连带修另一个 ——
    /// Fluent 那九个 <c>CheckBoxForeground*</c> 键的默认值实测全是字面量 <c>Black</c>
    /// （不是「跟随继承的 Foreground」）。后果是**暗色主题下 39 处复选框的文案
    /// 全是纯黑贴深底**，其中约 30 处在配置页，等于暗色主题下配置页几乎所有
    /// 开关的说明文字都读不出来。
    ///
    /// 判据取 CheckBox 宿主的 Foreground：这九个键正是喂给宿主 Foreground 的，
    /// 文字由 <c>PART_ContentPresenter</c> 继承下去。
    /// </summary>
    [Fact]
    public async Task CheckBoxLabelForegroundFollowsThemeTextToken()
    {
        await RunHeadlessAsync(async () =>
        {
            var box = new CheckBox { Content = "U184", IsChecked = false };
            var window = new Window { Width = 320, Height = 200, Content = box };
            window.Show();
            await Drain();

            var expected = ResolveThemeColor(box, "Ariadne.Color.TextPrimary");
            var actual = (box.Foreground as ISolidColorBrush)?.Color;
            Assert.NotNull(actual);
            Assert.NotEqual(Colors.Black, actual!.Value);
            Assert.Equal(expected, actual!.Value);

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// 主题里 <c>CheckBoxCheckBackgroundStrokeChecked</c> 这个键**必须不存在**。
    ///
    /// 探针实测 Fluent 12.0.5 **没有**这个键（<c>TryFindResource</c>=False）——
    /// 选中态方框靠 Fill 铺满，无需描边。写上去就是**死键**：不报错、不生效，
    /// 纯粹制造「看着配齐了」的错觉，正是 U152 那类「死样式冒充已完成工作」。
    ///
    /// 这条守的是**下一个人**：改这段时最自然的动作就是「Unchecked 有 Stroke，
    /// Checked 也补一个吧」。它会在那一刻当场红，并把理由指出来。
    /// </summary>
    [Fact]
    public void NoDeadCheckedStrokeKeyInTheme()
    {
        var theme = File.ReadAllText(ResolveThemePath());
        Assert.DoesNotContain("x:Key=\"CheckBoxCheckBackgroundStrokeChecked\"", theme);
        // 基线：确认本用例读到的确实是那份写了 CheckBox 键的主题文件，
        // 而不是路径解析错了拿到一份不含任何 CheckBox 键的文本（那会让上面
        // 那条 DoesNotContain 无条件通过 —— 又一个空测）。
        Assert.Contains("x:Key=\"CheckBoxCheckBackgroundStrokeCheckedPointerOver\"", theme);
    }

    /// <summary>
    /// Slider 的 track 与 thumb 必须**分别**配色，且都来自主题令牌。
    ///
    /// <c>UI组件状态表.md:56</c>：「slider → track `bg-hover`，thumb `brand-primary`」。
    /// 修复前主题里关于 Slider 的唯一一条样式是
    /// <c>&lt;Setter Property="Minimum" Value="0" /&gt;</c> —— 一个取值下限，不是视觉。
    /// 实测三个部件全是 Fluent 蓝 / 硬编码黑。
    ///
    /// ⚠️ 这 3 处 Slider 正是 <c>SpectrumColorPicker</c> 的 RGB 三条 ——
    /// **用户在这里挑主题强调色，而滑条自己是微软蓝、改完主题也不跟着变**。
    ///
    /// 一条用例里同时断言 track 与 thumb 是刻意的：规范要求的是「两者分别配色」
    /// 这个**关系**，只测一个的话，把两者都设成同色也能过，而那正是规范要避免的
    /// （「蓝轨 + 蓝球」分不出已走过多少）。
    /// </summary>
    [Fact]
    public async Task SliderTrackAndThumbUseSeparateThemeTokens()
    {
        await RunHeadlessAsync(async () =>
        {
            var slider = new Slider { Minimum = 0, Maximum = 100, Value = 40, Width = 200 };
            var window = new Window { Width = 320, Height = 200, Content = slider };
            window.Show();
            await Drain();

            // 未走过的轨 → 规范的 bg-hover。
            //
            // ⚠️ **判据取 `PART_IncreaseButton` 而不是 `Border#TrackBackground`** ——
            // 这是被本用例第一次运行当场证伪的写法。Fluent 的 Slider 模板里
            // **有两个都叫 `TrackBackground` 的 Border**（`PART_DecreaseButton` 与
            // `PART_IncreaseButton` 各自的模板内一个），`FirstOrDefault(名字匹配)`
            // 拿到的是 decrease 那一侧 —— 也就是**已走过的值段**，
            // 于是「未走过的轨」这条断言实际在测值段，得到 AccentPrimary 而非 bg-hover。
            // 两个 RepeatButton 的名字是唯一的，用它们才不会认错部件。
            // （教训：`FirstOrDefault(Name == …)` 在复合控件里可能有多个同名部件。）
            var trackExpected = ResolveThemeColor(slider, "Ariadne.Color.BackgroundHover");
            var track = slider.GetVisualDescendants()
                .OfType<RepeatButton>()
                .FirstOrDefault(b => b.Name == "PART_IncreaseButton");
            Assert.NotNull(track);
            var trackActual = (track!.Background as ISolidColorBrush)?.Color;
            Assert.NotNull(trackActual);
            Assert.NotEqual(FluentDefaultBlue, trackActual!.Value);
            Assert.Equal(trackExpected, trackActual!.Value);

            // 滑块（thumb）→ 规范的 brand-primary
            var thumbExpected = ResolveThemeColor(slider, "Ariadne.Color.AccentPrimary");
            // thumb 在模板里只有一个，名字唯一，可直接按名取。
            var thumb = slider.GetVisualDescendants()
                .OfType<Avalonia.Controls.Primitives.Thumb>()
                .FirstOrDefault(t => t.Name == "thumb");
            Assert.NotNull(thumb);
            var thumbActual = (thumb!.Background as ISolidColorBrush)?.Color;
            Assert.NotNull(thumbActual);
            Assert.NotEqual(FluentDefaultBlue, thumbActual!.Value);
            Assert.Equal(thumbExpected, thumbActual!.Value);

            // 两者必须不同色：这才是「track/thumb 各自配色」的实质。
            Assert.NotEqual(trackActual!.Value, thumbActual!.Value);

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// **暗色主题下复选框的标签文字必须真的是暗色主题的文字色。**
    ///
    /// 这条钉的是一个我自己在本轮制造、并由截图当场抓出来的缺陷 ——
    /// 上面几条用例**全都测不到它**，因为它们跑在默认（亮色）变体下。
    ///
    /// **成因**：Fluent 覆盖键最初写在外层 <c>Styles.Resources</c> 里，
    /// 那份 <c>SolidColorBrush</c> 的 <c>Color="{DynamicResource Ariadne.Color.TextPrimary}"</c>
    /// **只解析一次、且不带主题变体上下文** ⇒ 暗色主题下它拿到的是**亮色的值**。
    /// 实测：<c>ThemeVariant.Dark</c> 时 <c>Ariadne.Color.TextPrimary</c> = <c>#ffeceef0</c>，
    /// 而 <c>CheckBoxForegroundUnchecked</c> 解析成 <c>#ff1b1f22</c>（亮色那个深色字）。
    /// 后果是**暗色主题下已启用复选框的标签深色贴深底、整行看不见**
    /// （反而禁用那两行看得见，因为走的是另一个键）——
    /// 恰好是我在注释里写着「要避免」的那个形态，却以另一条路径重新造了出来。
    ///
    /// **修法**：把这些键放进 <c>ThemeDictionaries</c> 的 Light / Dark 两份里各一套，
    /// 用 <c>StaticResource</c> 引用同字典内的颜色 ⇒ 随字典整体切换。
    ///
    /// **判据必须显式指定 Dark 变体**：这是唯一能区分「亮色恰好对」与
    /// 「两个主题都对」的做法。截图之外没有别的发现途径 ——
    /// 缺键会被守卫抓到，而**取到错误的值**不会。
    /// </summary>
    [Fact]
    public async Task DarkVariantCheckBoxLabelUsesDarkThemeTextColor()
    {
        await RunHeadlessAsync(async () =>
        {
            var box = new CheckBox { Content = "U184", IsChecked = false };
            var window = new Window
            {
                Width = 320,
                Height = 200,
                RequestedThemeVariant = ThemeVariant.Dark,
                Content = box,
            };
            window.Show();
            await Drain();

            // 前提：确认这个窗口真的在暗色变体下，否则本用例什么都没测。
            Assert.Equal(ThemeVariant.Dark, box.ActualThemeVariant);

            var darkText = ResolveThemeColor(box, "Ariadne.Color.TextPrimary");
            var actual = (box.Foreground as ISolidColorBrush)?.Color;
            Assert.NotNull(actual);
            Assert.Equal(darkText, actual!.Value);

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// **个性化换强调色后，复选框与滑块必须跟着变。**
    ///
    /// 这条钉的是本轮第二个自造缺陷，同样由探针实测抓出、上面各条都测不到。
    ///
    /// **成因是一对互相拉扯的约束**：
    /// <list type="number">
    ///   <item>覆盖键放进 <c>ThemeDictionaries</c> 才能明暗各取对值，
    ///     而字典内部只能用 <c>StaticResource</c>（<c>DynamicResource</c>
    ///     在字典里不带变体上下文，暗色会取到亮色值）；</item>
    ///   <item><c>StaticResource</c> 在加载后就锁定 ⇒ <c>ThemeApplication.Apply</c>
    ///     运行时改写 <c>Ariadne.Color.AccentPrimary</c> 时它**不跟随**。</item>
    /// </list>
    /// 实测：把强调色换成橙 <c>#ffb45309</c> 后该令牌确实变了，
    /// 而勾选框与滑块仍是预设青绿 <c>#ff2e726b</c> —— 与 <c>ThemeApplication</c>
    /// 注释里记的「个性化换色后渐变面纹丝不动」是同一缺陷的另一处犯案。
    /// 尤其讽刺：那 3 个滑块**就是取色器本身**。
    ///
    /// **解法是分工**：明暗由字典的 StaticResource 管，个性化换色由
    /// <c>Apply</c> 的运行时覆盖管（并同步登记进 <c>OverlayBrushKeys</c>，
    /// 否则 <c>ThemeOverlayKeysTests</c> 的键对等守卫会红）。
    ///
    /// 判据必须是「<c>Apply</c> 之后真实控件的画刷变了」——
    /// 「键在不在 OverlayBrushKeys 里」是弱判据，那只证明登记了、不证明生效。
    /// 末尾恢复预设并断言回到青绿，顺带覆盖 Reset 路径不留僵尸色。
    /// </summary>
    [Fact]
    public async Task PersonalizedAccentReachesCheckBoxAndSlider()
    {
        await RunHeadlessAsync(async () =>
        {
            var box = new CheckBox { Content = "U184", IsChecked = true };
            var slider = new Slider { Minimum = 0, Maximum = 100, Value = 50, Width = 180 };
            var window = new Window
            {
                Width = 360,
                Height = 220,
                Content = new StackPanel { Children = { box, slider } },
            };
            window.Show();
            await Drain();

            Color? CheckFill() => (box.GetVisualDescendants().OfType<Border>()
                .FirstOrDefault(b => b.Name == "NormalRectangle")?.Background as ISolidColorBrush)?.Color;
            Color? ThumbFill() => (slider.GetVisualDescendants()
                .OfType<Avalonia.Controls.Primitives.Thumb>()
                .FirstOrDefault(t => t.Name == "thumb")?.Background as ISolidColorBrush)?.Color;

            var preset = ResolveThemeColor(box, "Ariadne.Color.AccentPrimary");
            Assert.Equal(preset, CheckFill());
            Assert.Equal(preset, ThumbFill());

            // 换一个与青绿明显不同的强调色（橙），模拟用户在个性化页改色。
            const string BrandHex = "#B45309";
            var brand = Color.Parse(BrandHex);
            Assert.NotEqual(preset, brand);
            ThemeApplication.Apply("light", "#EDF0EE", "#FAFBFA", BrandHex);
            await Drain();

            Assert.Equal(brand, CheckFill());
            Assert.Equal(brand, ThumbFill());

            // 恢复预设：不能留下上一套主题色（Reset 漏删会变成僵尸键）。
            ThemeApplication.Apply("light");
            await Drain();
            Assert.Equal(preset, CheckFill());
            Assert.Equal(preset, ThumbFill());

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// U213-E：**开态开关的轨道底色必须是品牌强调色，不是 Fluent 默认蓝。**
    ///
    /// 本轮把 AutoMode 从「满宽 <c>Button.subtle</c> + 选中琥珀底」换成
    /// 「标签 + <c>ToggleSwitch</c>」，而在此之前**全仓 ToggleSwitch 使用数 = 0**
    /// ⇒ 这是第一次把这个控件引进产品，它的默认外观就是 FluentTheme 的。
    /// 与 U164-B 的 Expander 是同一个问题、只是换了控件：不配色就是把一个丑
    /// 换成另一个丑。这条用例是「新控件必须同批配色」这个约束的**唯一**执行者。
    ///
    /// 判据落在 <c>Border#SwitchKnobBounds</c> 的 Background 上 ——
    /// 那是 Fluent 模板里画**开态轨道**的那个部件（关态轨是另一个
    /// <c>Border#OuterBorder</c>，两者靠 Opacity 互换）。拿 ToggleSwitch 宿主的
    /// Background 当判据等于什么都没测：宿主那层我们刻意设成透明。
    ///
    /// ⚠️ 部件名与资源键名都是从 <c>Avalonia.Themes.Fluent.dll</c> 的字符串表
    /// 抽出来的，不是照抄上游教程 —— Avalonia 缺资源键/错部件名都是**静默失效**。
    /// 这条用例同时充当那批键名的存在性证明：名字写错时它会红。
    /// </summary>
    [Fact]
    public async Task CheckedToggleSwitchTrackFollowsBrandAccentNotFluentBlue()
    {
        await RunHeadlessAsync(async () =>
        {
            var toggle = new ToggleSwitch { IsChecked = true, OnContent = null, OffContent = null };
            var window = new Window { Width = 320, Height = 200, Content = toggle };
            window.Show();
            await Drain();

            var expected = ResolveThemeColor(toggle, "Ariadne.Color.AccentPrimary");
            var actual = ResolvePartColor(toggle, "SwitchKnobBounds");

            Assert.NotEqual(FluentDefaultBlue, actual);
            Assert.Equal(expected, actual);

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// U213-E：关态轨道**不铺底**，只有一圈边线色描边。
    ///
    /// 这一条是「不要卡片」这个产品约束在控件层的落点，也是与 CheckBox 未选中态
    /// 同一手势的守卫。缺了它，「关态轨道铺一块灰」这种改法能让上面那条照样绿，
    /// 而 AutoMode 悬在对话框外、周围没有容器 —— 一铺底就又冒出一块方形色片，
    /// 也就是用户这轮抱怨的「难看的卡片」换个尺寸重演。
    ///
    /// 判据取两条：关态轨的 Background **透明**（alpha=0），描边等于
    /// <c>BorderDefault</c> 令牌。只断透明不够：描边也没了的话开关就整个消失。
    /// </summary>
    [Fact]
    public async Task UncheckedToggleSwitchTrackIsOutlineOnlyWithNoFill()
    {
        await RunHeadlessAsync(async () =>
        {
            var toggle = new ToggleSwitch { IsChecked = false, OnContent = null, OffContent = null };
            var window = new Window { Width = 320, Height = 200, Content = toggle };
            window.Show();
            await Drain();

            var track = toggle.GetVisualDescendants().OfType<Border>()
                .FirstOrDefault(border => border.Name == "OuterBorder");
            Assert.NotNull(track);

            var fill = track!.Background as ISolidColorBrush;
            Assert.NotNull(fill);
            Assert.Equal(0, fill!.Color.A);

            var stroke = track.BorderBrush as ISolidColorBrush;
            Assert.NotNull(stroke);
            Assert.Equal(ResolveThemeColor(toggle, "Ariadne.Color.BorderDefault"), stroke!.Color);

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// U213-E：**个性化换强调色后，开关必须跟着变。**
    ///
    /// # 这一条是渲染取证抓出来的，不是照抄先例
    ///
    /// 我先只加了主题字典里的资源键，然后按纪律真的把界面渲染出来看
    /// （玫瑰主题、开态、实机 Xvfb 截图）—— 旁边的主按钮是玫瑰色
    /// <c>#DA706A</c>，而开关轨道仍是**预设青绿** <c>#6FB9AD</c>：
    /// 同一屏上两个强调色。成因与 U184 记的那对拉扯完全一致
    /// （见 <see cref="PersonalizedAccentReachesCheckBoxAndSlider"/> 的注释）：
    /// 字典内部只能用 <c>StaticResource</c>，而它加载后就锁定、不跟随
    /// <c>ThemeApplication.Apply</c> 的运行时改写。
    ///
    /// ⇒ 结论推广成一条规则：**引入任何吃 Fluent 资源键的新控件，
    /// 都必须同批把它的强调色键登记进 <c>OverlayBrushKeys</c> 并在 Apply 里写入**。
    /// 光配字典是做一半，而做一半这件事**不报错**。
    ///
    /// 判据同 U184 那条：Apply 之后真实部件的画刷变了、恢复预设后回到青绿
    /// （后半段覆盖 Reset 不留僵尸色）。「键在不在清单里」是弱判据。
    /// </summary>
    [Fact]
    public async Task PersonalizedAccentReachesToggleSwitch()
    {
        await RunHeadlessAsync(async () =>
        {
            var toggle = new ToggleSwitch { IsChecked = true, OnContent = null, OffContent = null };
            var window = new Window { Width = 320, Height = 200, Content = toggle };
            window.Show();
            await Drain();

            Color? TrackFill() => (toggle.GetVisualDescendants().OfType<Border>()
                .FirstOrDefault(border => border.Name == "SwitchKnobBounds")
                ?.Background as ISolidColorBrush)?.Color;

            var preset = ResolveThemeColor(toggle, "Ariadne.Color.AccentPrimary");
            Assert.Equal(preset, TrackFill());

            const string BrandHex = "#B45309";
            var brand = Color.Parse(BrandHex);
            Assert.NotEqual(preset, brand);
            ThemeApplication.Apply("light", "#EDF0EE", "#FAFBFA", BrandHex);
            await Drain();

            Assert.Equal(brand, TrackFill());

            ThemeApplication.Apply("light");
            await Drain();
            Assert.Equal(preset, TrackFill());

            window.Close();
            await Drain();
        });
    }

    /// <summary>
    /// 主题令牌的真实色值。缺键直接失败：那意味着判据本身失效。</summary>
    ///
    /// ⚠️ **必须传 <c>ActualThemeVariant</c>**，这是变异测试第二次抓出来的坑
    /// （第一次是被吞掉的断言，见 <c>RunHeadlessAsync</c> 的注释）。
    /// <c>Ariadne.Color.*</c> 全部定义在 <c>ResourceDictionary.ThemeDictionaries</c>
    /// 的 <c>Light</c> / <c>Dark</c> 两份字典里（亮暗各 122 个令牌一一对应），
    /// 而**不带变体的 <c>TryFindResource(key, out …)</c> 查不到它们** ——
    /// 实测五条用例全部红在「主题令牌 X 不存在」这句上，也就是红在**基线**断言、
    /// 而不是我要测的那条。这正是本仓库记过的「测试基建缺陷伪装成产品缺陷」：
    /// 若不追到这一层，很容易误判成「令牌真的没定义」而去改主题文件。
    /// </summary>
    private static Color ResolveThemeColor(Control host, string key)
    {
        Assert.True(
            host.TryFindResource(key, host.ActualThemeVariant, out var value),
            $"主题令牌 {key} 不存在（注意 Ariadne.Color.* 在 ThemeDictionaries 里，"
            + "查询必须带 ThemeVariant，否则这里会误报「令牌不存在」）");
        return Assert.IsType<Color>(value);
    }

    /// <summary>模板部件的 Background 色。找不到部件直接失败（模板结构变了）。</summary>
    private static Color ResolvePartColor(Control host, string partName)
    {
        var part = host.GetVisualDescendants().OfType<Border>()
            .FirstOrDefault(b => b.Name == partName);
        Assert.NotNull(part);
        var brush = part!.Background as ISolidColorBrush;
        Assert.NotNull(brush);
        return brush!.Color;
    }

    private static string ResolveThemePath()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return Path.Combine(dir!, "Ariadne.Desktop", "Resources", "Styles", "AriadneTheme.axaml");
    }

    private static Task Drain() =>
        Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded).GetTask();

    /// <summary>
    /// ⚠️ **必须用返回值的那个 <c>Dispatch</c> 重载**，且必须显式建
    /// <c>HeadlessAppBuilder</c> —— 这是变异测试当场抓出来的坑。
    ///
    /// 本文件首版写的是 <c>session.Dispatch(body, ct)</c>（<c>Func&lt;Task&gt;</c> 重载）。
    /// 症状：**用例体里的断言失败被静默吞掉，测试照样报绿**。
    /// 我把核心资源键改名（等于摘掉修复）后 6 条全绿；进一步插一句
    /// <c>Assert.True(false, "PROBE")</c> —— **它也是绿的**，而编译器确实报了
    /// xUnit2020 警告、dll 时间戳也比源码新，即程序集是新的、代码真的跑了。
    /// 也就是说那个重载把异常连同断言一起丢了 ⇒ **整个文件本来是一组空测**。
    ///
    /// 换成 <c>Dispatch(async () =&gt; { await body(); return true; }, ct)</c> 后，
    /// 同一个变异立刻红在目标断言上（见提交说明）。
    /// <c>AvaloniaTestIsolationLevel.PerTest</c> 与 <c>ReadingEditingParityTests</c>
    /// 保持一致：本机 headless 的存活取决于实体化顺序，照抄那份已验证可跑的配置。
    /// </summary>
    private static async Task RunHeadlessAsync(Func<Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            await body();
            return true;
        }, CancellationToken.None);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}
