using System.IO;

namespace Lattice.Control.Desktop;

internal static class DesktopPolicy
{
    internal static Uri DefaultControlUri { get; } = new("http://127.0.0.1:4317/");

    internal static TimeSpan ReconnectInterval { get; } = TimeSpan.FromSeconds(2);

    internal static TimeSpan RuntimeHealthInterval { get; } = TimeSpan.FromSeconds(1);

    internal static string WebViewUserDataFolder => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "LATTICE",
        "ControlDesktop",
        "WebView2");

    internal static Uri ResolveControlUri(IReadOnlyList<string> arguments, string? environmentValue)
    {
        string? candidate = null;
        for (int index = 0; index + 1 < arguments.Count; index += 1)
        {
            if (string.Equals(arguments[index], "--url", StringComparison.OrdinalIgnoreCase))
            {
                candidate = arguments[index + 1];
                break;
            }
        }

        candidate ??= environmentValue;
        if (Uri.TryCreate(candidate, UriKind.Absolute, out Uri? parsed) && IsApprovedLoopback(parsed))
        {
            return parsed;
        }

        return DefaultControlUri;
    }

    internal static bool IsApprovedLoopback(Uri uri)
    {
        return string.Equals(uri.Scheme, Uri.UriSchemeHttp, StringComparison.OrdinalIgnoreCase)
            && string.Equals(uri.Host, "127.0.0.1", StringComparison.OrdinalIgnoreCase)
            && string.IsNullOrEmpty(uri.UserInfo)
            && !uri.IsDefaultPort;
    }

    internal static bool IsApprovedControlNavigation(Uri target, Uri controlUri)
    {
        return IsApprovedLoopback(target)
            && IsApprovedLoopback(controlUri)
            && target.Port == controlUri.Port;
    }

    internal static bool ShouldManageControl(Uri controlUri)
    {
        return controlUri == DefaultControlUri;
    }

    internal static bool CanApplyNavigationResult(
        ulong? currentNavigationId,
        ulong completedNavigationId,
        bool isClosing)
    {
        return !isClosing && currentNavigationId == completedNavigationId;
    }
}
