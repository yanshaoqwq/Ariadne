using System;
using System.IO;
using SkiaSharp;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U10000 守卫：欢迎页品牌 Logo「只剩底了」。
///
/// <para>
/// **用户原报**：「欢迎页品牌图标没了，只剩底了」。
/// 强调色渐变圆牌画出来了，里面的 Logo 图案几乎不可见。
/// </para>
///
/// <para>
/// **根因不是代码错**（这一点花了两轮才定死，两条错误假设都被证伪：
/// `ResolveColor` 拿画刷键完全正常、`Image.Source` 也确实被赋了值）。
/// 根因是**母版笔画太细**：7px/512 = 1.4% 宽，缩到 44px 只剩 0.6px 亚像素宽。
/// 亚像素宽的线在面积平均下永远拿不到满不透明度 ⇒ 墨迹平均 alpha 只有 75
/// （母版原值 192，仅 39%），被冲淡成认不出的划痕。
/// 同一份母版渲到 88px 就很清楚 ⇒ **母版没问题，是「用在 44px」这件事不成立。**
/// </para>
///
/// <para>
/// **处置（用户定夺）**：加粗母版笔画约 2.5 倍（7px → 17px/512），
/// 圆牌尺寸与欢迎页版式都不动。
/// </para>
///
/// <para>
/// ⚠️ **判据刻意落在「44px 上的可见度」，不是文件哈希**：
/// 换一版同样细的母版哈希也会变，但缺陷照旧。要守的性质是
/// 「缩到实际呈现尺寸后，墨迹仍然足够实」。
/// </para>
/// </summary>
public sealed class BrandLogoLegibilityTests
{
    /// <summary>欢迎页圆牌里 Logo 的实际尺寸（`WelcomeView.axaml`：Width/Height="44"）。</summary>
    private const int WelcomeLogoSize = 44;

    /// <summary>
    /// 44px 上墨迹平均 alpha 必须够高。
    ///
    /// 缺陷版本是 75（母版 192 的 39%）；加粗后 164（85%）。
    /// 阈值取 130：留出插值与未来微调的余量，同时把「又换回细笔画母版」挡在门外。
    /// </summary>
    [Fact]
    public void MasterArtwork_StaysLegibleAtTheSizeTheWelcomePageActuallyUses()
    {
        var alpha = LoadMasterAlphaAt(WelcomeLogoSize);

        var inkValues = 0;
        long inkSum = 0;
        foreach (var value in alpha)
        {
            if (value > 8)
            {
                inkValues++;
                inkSum += value;
            }
        }

        Assert.True(inkValues > 0, "母版在 44px 上没有任何墨迹像素——图案整个没了");
        var average = inkSum / (double)inkValues;

        Assert.True(
            average >= 130,
            $"44px 上墨迹平均 alpha 只有 {average:F0}（阈值 130）。"
            + "笔画细到亚像素宽时会被面积平均冲淡成划痕，作者看到的就是「只剩底了」。"
            + "缺陷版本此处是 75。");
    }

    /// <summary>
    /// 可见像素占比也要够 —— 平均 alpha 高但只有几个像素同样认不出。
    ///
    /// 两条判据缺一不可：只测平均值时，一个「只剩三个实心点」的母版也能过。
    /// 缺陷版本 6.6%，加粗后 13.7%。
    /// </summary>
    [Fact]
    public void MasterArtwork_CoversEnoughPixelsToBeRecognisable()
    {
        var alpha = LoadMasterAlphaAt(WelcomeLogoSize);

        var visible = 0;
        foreach (var value in alpha)
        {
            if (value > 65)
            {
                visible++;
            }
        }

        var ratio = visible / (double)alpha.Length;
        Assert.True(
            ratio >= 0.09,
            $"44px 上只有 {ratio:P1} 的像素够亮（阈值 9%）。缺陷版本是 6.6%。");
    }

    /// <summary>
    /// 反向约束：加粗不能过头把图案糊成一团。
    ///
    /// ⚠️ 这一条与上面两条是**反方向的压力** —— 只测「够不够实」的话，
    /// 有人把母版改成一个实心方块也能全绿。本项目已有先例
    /// （U207-F 的「让 diff 看见更多」与「让 diff 看不见内部状态」）。
    /// </summary>
    [Fact]
    public void MasterArtwork_IsNotSoBoldThatItBecomesABlob()
    {
        foreach (var size in new[] { 16, 32, 44, 88, 128, 256 })
        {
            var alpha = LoadMasterAlphaAt(size);
            var solid = 0;
            foreach (var value in alpha)
            {
                if (value > 128)
                {
                    solid++;
                }
            }

            var fill = solid / (double)alpha.Length;
            Assert.True(
                fill <= 0.42,
                $"{size}px 上实心像素占了 {fill:P1}（上限 42%）——笔画过粗，图案糊成一团");
        }
    }

    /// <summary>
    /// 读母版 alpha 并**面积平均**降采样到指定尺寸。
    ///
    /// ⚠️ 刻意**不走** `AssetLoader`：headless 测试里没有 Avalonia 运行时，
    /// `IAssetLoader` 未注册（实测 `InvalidOperationException`）。
    /// 同目录 `AppIconInsetTests` 也是直接读文件 + 手写 PNG 解码，沿用那条路。
    ///
    /// ⚠️ 面积平均正是**缺陷的成因本身**：亚像素宽的笔画在面积平均下
    /// 拿不到满不透明度。所以这里必须自己做平均，而不是取最近邻 ——
    /// 取最近邻会让细笔画母版也「看起来很实」，守卫就失效了。
    /// </summary>
    private static byte[] LoadMasterAlphaAt(int size)
    {
        var (width, height, alpha) = ReadMasterAlpha();
        var result = new byte[size * size];

        for (var y = 0; y < size; y++)
        {
            var srcTop = y * height / size;
            var srcBottom = Math.Max(srcTop + 1, (y + 1) * height / size);
            for (var x = 0; x < size; x++)
            {
                var srcLeft = x * width / size;
                var srcRight = Math.Max(srcLeft + 1, (x + 1) * width / size);

                long sum = 0;
                var count = 0;
                for (var sy = srcTop; sy < srcBottom; sy++)
                {
                    for (var sx = srcLeft; sx < srcRight; sx++)
                    {
                        sum += alpha[(sy * width) + sx];
                        count++;
                    }
                }

                result[(y * size) + x] = (byte)(sum / Math.Max(1, count));
            }
        }

        return result;
    }

    private static (int Width, int Height, byte[] Alpha) ReadMasterAlpha()
    {
        var path = Path.Combine(
            ResolveRepoRoot(), "desktop", "Ariadne.Desktop", "Assets", "app-icon-master.png");
        using var image = SKBitmap.Decode(path);
        Assert.NotNull(image);

        var alpha = new byte[image.Width * image.Height];
        for (var y = 0; y < image.Height; y++)
        {
            for (var x = 0; x < image.Width; x++)
            {
                alpha[(y * image.Width) + x] = image.GetPixel(x, y).Alpha;
            }
        }

        return (image.Width, image.Height, alpha);
    }

    private static string ResolveRepoRoot()
    {
        var dir = AppContext.BaseDirectory;
        for (var i = 0; i < 8 && dir is not null; i++)
        {
            if (Directory.Exists(Path.Combine(dir, "desktop", "Ariadne.Desktop", "Assets")))
            {
                return dir;
            }

            dir = Path.GetDirectoryName(dir);
        }

        throw new DirectoryNotFoundException($"从 {AppContext.BaseDirectory} 向上找不到仓库根");
    }
}
