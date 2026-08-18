using System.IO.Compression;
using Ariadne.Desktop;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U163-D：线描图案不得比实心剪影占得更少。
///
/// 这条不变量的**方向就是根因**，不只是「数值偏大」：线描母版走的是应用内 Logo
/// 与桌面/开始菜单图标（大尺寸、清晰优先），实心剪影走的是小尺寸任务栏窗口图标——
/// 只有后者需要贴边余量（某些平台的圆角遮罩会切掉贴边像素）。
/// 原先线描 0.04 &gt; 实心 0.015，方向恰好反了，于是欢迎页 62px 圆牌里的 Logo
/// 只占 47%、桌面图标在图标网格里比邻居小一圈。
///
/// ⚠️ 这里**不**量渲染出来的位图。本机 Avalonia headless 平台不做真实光栅化：
/// 两种 inset 渲染出的 alpha 包围盒完全相同（实测均为 1.0000w/1.0000h，
/// 像素还是噪声：distinct=5610、(128,128)=A0），拿它做判据是恒真的空测——
/// 我先写了那一版，变异后仍全绿，才换成现在的判据。
/// 现在取两个生产常量的**关系**（不是各自的值，那才恒真）+ 母版填充率实测。
/// </summary>
public sealed class AppIconInsetTests
{
    [Fact]
    public void LineArtInset_IsNotLargerThanSolidSilhouetteInset()
    {
        Assert.True(
            AppIconPainter.LineInsetFraction <= AppIconPainter.SolidInsetFraction,
            $"线描内缩 {AppIconPainter.LineInsetFraction} 大于实心剪影的 "
            + $"{AppIconPainter.SolidInsetFraction}，方向反了："
            + "线描用在应用内 Logo 与桌面图标（大尺寸、要占满容器），"
            + "实心剪影才需要贴边余量。这样改回去，欢迎页圆牌里的图案会再次显得偏小，"
            + "桌面图标也会比图标网格里的邻居小一圈。");
    }

    [Fact]
    public void LineArtMaster_AlreadyCarriesItsOwnPadding_SoNoExtraInsetIsNeeded()
    {
        // 线描母版自身横向留白仅 14/15px（≈2.8%），本来就不需要再内缩。
        // 这条同时钉住「换母版时别换成留白很大的版本」。
        var box = MeasureOpaqueBoundingBox("app-icon-master.png");

        Assert.True(
            box.WidthFraction > 0.90,
            $"线描母版横向只填充 {box.WidthFraction:P1}，母版自身留白过大；"
            + "此时把 inset 降为 0 也救不回图案偏小，应当重新导出母版。");
    }

    [Fact]
    public void LineArtMaster_VerticalPaddingRemainsTheDominantShrink()
    {
        // 留档「层次 2」（重新导出母版）的依据：线描母版上下留白 52px 是左右 14px 的
        // 3.5 倍，纵向只占 79.7%。inset→0 救不了这一项。
        // 这条不强制现在修，只在有人重导出母版时给出可对照的基线。
        var line = MeasureOpaqueBoundingBox("app-icon-master.png");
        var solid = MeasureOpaqueBoundingBox("app-icon-taskbar-master.png");

        Assert.True(
            line.HeightFraction <= solid.HeightFraction + 0.001,
            "线描母版纵向填充率已追上实心母版——若已重新导出母版，请更新本用例基线"
            + $"（line={line.HeightFraction:P1}, solid={solid.HeightFraction:P1}）。");
    }

    private readonly record struct OpaqueBox(double WidthFraction, double HeightFraction);

    /// <summary>
    /// 纯 CPU 解 PNG 量不透明像素包围盒——不经 Avalonia 渲染，
    /// 因此不受 headless 平台不光栅化的影响。
    /// </summary>
    private static OpaqueBox MeasureOpaqueBoundingBox(string assetFileName)
    {
        var path = Path.Combine(ResolveRepoRoot(), "desktop", "Ariadne.Desktop", "Assets", assetFileName);
        var (width, height, pixels) = DecodePng(path);

        int minX = width, minY = height, maxX = -1, maxY = -1;
        for (var y = 0; y < height; y++)
        {
            for (var x = 0; x < width; x++)
            {
                // RGBA，alpha 阈值 8 与手工量测口径一致
                if (pixels[(y * width + x) * 4 + 3] <= 8)
                {
                    continue;
                }

                if (x < minX) minX = x;
                if (x > maxX) maxX = x;
                if (y < minY) minY = y;
                if (y > maxY) maxY = y;
            }
        }

        Assert.True(maxX >= 0, $"{assetFileName} 整幅透明，母版读错了");
        return new OpaqueBox(
            (maxX - minX + 1) / (double)width,
            (maxY - minY + 1) / (double)height);
    }

    /// <summary>最小 PNG 解码：只支持 8bit RGBA（两个母版都是），够用即止。</summary>
    private static (int Width, int Height, byte[] Pixels) DecodePng(string path)
    {
        var data = File.ReadAllBytes(path);
        using var idat = new MemoryStream();
        int width = 0, height = 0, bitDepth = 0, colorType = 0;
        var pos = 8;
        while (pos + 8 <= data.Length)
        {
            var length = (data[pos] << 24) | (data[pos + 1] << 16) | (data[pos + 2] << 8) | data[pos + 3];
            var type = System.Text.Encoding.ASCII.GetString(data, pos + 4, 4);
            if (type == "IHDR")
            {
                width = (data[pos + 8] << 24) | (data[pos + 9] << 16) | (data[pos + 10] << 8) | data[pos + 11];
                height = (data[pos + 12] << 24) | (data[pos + 13] << 16) | (data[pos + 14] << 8) | data[pos + 15];
                bitDepth = data[pos + 16];
                colorType = data[pos + 17];
            }
            else if (type == "IDAT")
            {
                idat.Write(data, pos + 8, length);
            }

            pos += 12 + length;
        }

        Assert.Equal(8, bitDepth);
        Assert.Equal(6, colorType);   // RGBA

        idat.Position = 0;
        using var zlib = new ZLibStream(idat, CompressionMode.Decompress);
        using var raw = new MemoryStream();
        zlib.CopyTo(raw);
        var bytes = raw.ToArray();

        const int channels = 4;
        var stride = width * channels;
        var output = new byte[stride * height];
        var previous = new byte[stride];
        var offset = 0;
        for (var y = 0; y < height; y++)
        {
            var filter = bytes[offset++];
            var line = new byte[stride];
            Array.Copy(bytes, offset, line, 0, stride);
            offset += stride;

            for (var x = 0; x < stride; x++)
            {
                int a = x >= channels ? line[x - channels] : 0;
                int b = previous[x];
                int c = x >= channels ? previous[x - channels] : 0;
                int value = filter switch
                {
                    0 => line[x],
                    1 => line[x] + a,
                    2 => line[x] + b,
                    3 => line[x] + ((a + b) >> 1),
                    4 => line[x] + Paeth(a, b, c),
                    _ => throw new InvalidDataException($"unsupported PNG filter {filter}"),
                };
                line[x] = (byte)(value & 0xFF);
            }

            Array.Copy(line, 0, output, y * stride, stride);
            previous = line;
        }

        return (width, height, output);
    }

    private static int Paeth(int a, int b, int c)
    {
        var p = a + b - c;
        var pa = Math.Abs(p - a);
        var pb = Math.Abs(p - b);
        var pc = Math.Abs(p - c);
        return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
    }

    private static string ResolveRepoRoot()
    {
        var path = Path.GetDirectoryName(typeof(AppIconInsetTests).Assembly.Location)!;
        while (!string.IsNullOrEmpty(path) && !File.Exists(Path.Combine(path, "desktop", "Ariadne.slnx")))
        {
            path = Directory.GetParent(path)?.FullName ?? string.Empty;
        }

        return path;
    }
}
