using System.IO;

namespace Lattice.Control.Desktop;

internal sealed record ControlRuntimeFailurePresentation(string Detail, bool AutoReconnect);
internal sealed record ControlEndpointSelection(Uri Uri, bool ManageControl);

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

    internal static ControlEndpointSelection ResolveControlTarget(
        IReadOnlyList<string> arguments,
        string? environmentValue)
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
            return new(parsed, ManageControl: false);
        }

        return new(DefaultControlUri, ManageControl: true);
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

    internal static bool CanApplyNavigationResult(
        ulong? currentNavigationId,
        ulong completedNavigationId,
        long currentGeneration,
        long completedGeneration,
        bool isClosing)
    {
        return !isClosing
            && currentNavigationId == completedNavigationId
            && currentGeneration == completedGeneration;
    }

    internal static long NextNavigationGeneration(long currentGeneration)
    {
        return currentGeneration == long.MaxValue ? 1 : currentGeneration + 1;
    }

    internal static ControlRuntimeFailurePresentation DescribeRuntimeFailure(
        ControlRuntimeEvaluation runtime)
    {
        return runtime switch
        {
            { Detail: "CONTROL_RUNTIME_FILES_MISSING" } => new(
                "Control 執行檔不完整；此候選無法自動修復，請改用完整候選包。",
                false),
            { Detail: "CONTROL_PROCESS_START_FAILED" } => new(
                "Control 程序啟動失敗；LATTICE 已停止自動重試，請檢查候選檔案與本機執行條件。",
                false),
            { Detail: "CONTROL_DATA_SCOPE_INCOMPATIBLE" } => new(
                "127.0.0.1:4317 的 Control 使用不同資料範圍；LATTICE 已停止接管。跨資料範圍只能用明確 profile 或 --url 選擇。",
                false),
            { Detail: "CONTROL_RECONCILIATION_REQUIRED" } => new(
                "Control 有未完成工作需要先對帳；LATTICE 已停止接管，避免啟動新的 effect。",
                false),
            { Health: ControlRuntimeHealth.INCOMPATIBLE } => new(
                "127.0.0.1:4317 已有陌生或不相容的服務；LATTICE 已停止接管，也不會關閉它。",
                false),
            { Detail: "CONTROL_OWNED_PROCESS_EXITED" or "CONTROL_PROCESS_EXITED" } => new(
                "Control 程序意外停止；LATTICE 會在下一次重連啟動新的自有程序。",
                true),
            { Detail: "CONTROL_STARTUP_TIMEOUT" or "CONTROL_OWNED_PROCESS_UNREACHABLE" } => new(
                "Control 暫時無法就緒；LATTICE 會在下一次重連重新檢查。",
                true),
            { Health: ControlRuntimeHealth.UNREACHABLE, Action: ControlRuntimeAction.StartOwned } => new(
                "Control 無法連線；確認 4317 沒有 listener 後，LATTICE 會在下一次重連啟動封裝版本。",
                true),
            { Action: ControlRuntimeAction.FailClosed } => new(
                $"Control 已 fail closed（{runtime.Detail}）；LATTICE 不會自動重試。",
                false),
            _ => new($"Control 目前狀態：{runtime.Health}。", false),
        };
    }
}
