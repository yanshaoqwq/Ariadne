using System.Diagnostics;

namespace Ariadne.Desktop.ViewModels;

/// 在用户默认浏览器中打开受控的外部链接。
internal static class ExternalLinkOpener
{
    public static bool TryOpen(string url)
    {
        if (!Uri.TryCreate(url, UriKind.Absolute, out var uri)
            || (uri.Scheme != Uri.UriSchemeHttp && uri.Scheme != Uri.UriSchemeHttps))
        {
            return false;
        }

        try
        {
            using var process = Process.Start(new ProcessStartInfo
            {
                FileName = uri.AbsoluteUri,
                UseShellExecute = true,
            });
            return process is not null;
        }
        catch (Exception)
        {
            return false;
        }
    }
}
