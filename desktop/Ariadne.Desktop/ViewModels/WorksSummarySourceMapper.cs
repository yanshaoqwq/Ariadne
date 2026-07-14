using System.Text;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 将后端 SourceSpan 的 UTF-8 byte 半开区间映射为 Avalonia TextBox 使用的
/// UTF-16 索引。边界落在多字节字符内部时必须拒绝，避免定位到错误正文。
/// </summary>
public static class WorksSummarySourceMapper
{
    private static readonly UTF8Encoding StrictUtf8 = new(false, true);

    public static bool TryMapUtf8Range(
        string text,
        long byteStart,
        long byteEnd,
        out int utf16Start,
        out int utf16End)
    {
        utf16Start = 0;
        utf16End = 0;
        if (byteStart < 0 || byteEnd <= byteStart || byteEnd > int.MaxValue)
        {
            return false;
        }

        try
        {
            var bytes = StrictUtf8.GetBytes(text ?? string.Empty);
            if (byteEnd > bytes.Length
                || !IsUtf8Boundary(bytes, (int)byteStart)
                || !IsUtf8Boundary(bytes, (int)byteEnd))
            {
                return false;
            }

            utf16Start = StrictUtf8.GetCharCount(bytes, 0, (int)byteStart);
            utf16End = utf16Start + StrictUtf8.GetCharCount(
                bytes,
                (int)byteStart,
                (int)(byteEnd - byteStart));
            return utf16End > utf16Start;
        }
        catch (EncoderFallbackException)
        {
            return false;
        }
    }

    private static bool IsUtf8Boundary(byte[] bytes, int offset)
    {
        return offset == 0
               || offset == bytes.Length
               || (offset > 0
                   && offset < bytes.Length
                   && (bytes[offset] & 0b1100_0000) != 0b1000_0000);
    }
}
