using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.Wpf;
using System.IO;
using System.Net.Http;
using System.Text.Json;
using System.Windows;

namespace Lattice.Control.Desktop;

internal sealed class ResultWindow : Window
{
    private readonly Uri _resultUri;
    private readonly WebView2 _view = new();
    internal CoreWebView2 BrowserCore => _view.CoreWebView2;

    internal ResultWindow(Uri resultUri)
    {
        _resultUri = resultUri;
        Title = "LATTICE · 已驗收成果";
        Width = 1000; Height = 760; MinWidth = 600; MinHeight = 420;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        Content = _view;
        Closed += (_, _) => _view.Dispose();
    }

    internal static async Task<bool> IsRetainedPreviewAsync(Uri controlUri, Uri target, CancellationToken cancellation)
    {
        if (!DesktopPolicy.IsApprovedLoopback(target) || target.AbsolutePath != "/"
            || target.Query.Length != 0 || target.Fragment.Length != 0) return false;
        using var handler = new HttpClientHandler { AllowAutoRedirect = false };
        using var client = new HttpClient(handler) { Timeout = TimeSpan.FromSeconds(10), MaxResponseContentBufferSize = 4096 };
        var query = new Uri(controlUri, "/api/result-preview?url=" + Uri.EscapeDataString(target.AbsoluteUri));
        using HttpResponseMessage response = await client.GetAsync(query, cancellation);
        if (!response.IsSuccessStatusCode) return false;
        using JsonDocument packet = JsonDocument.Parse(await response.Content.ReadAsStringAsync(cancellation));
        return packet.RootElement.GetProperty("schema_version").GetString() == "lattice.control.result-preview.v1"
            && packet.RootElement.GetProperty("url").GetString() == target.AbsoluteUri;
    }

    internal async Task InitializeAsync(CoreWebView2Environment environment)
    {
        await _view.EnsureCoreWebView2Async(environment);
        CoreWebView2 core = BrowserCore;
        core.Settings.AreDevToolsEnabled = false;
        core.Settings.AreDefaultContextMenusEnabled = false;
        core.Settings.AreBrowserAcceleratorKeysEnabled = false;
        core.NewWindowRequested += (_, e) => e.Handled = true;
        core.PermissionRequested += (_, e) => e.State = CoreWebView2PermissionState.Deny;
        core.NavigationStarting += (_, e) => e.Cancel = !Uri.TryCreate(e.Uri, UriKind.Absolute, out Uri? uri)
            || !DesktopPolicy.IsApprovedControlNavigation(uri, _resultUri);
        core.AddWebResourceRequestedFilter("*", CoreWebView2WebResourceContext.Document);
        core.WebResourceRequested += (_, e) =>
        {
            if (!Uri.TryCreate(e.Request.Uri, UriKind.Absolute, out Uri? uri)
                || !DesktopPolicy.IsApprovedControlNavigation(uri, _resultUri))
                e.Response = core.Environment.CreateWebResourceResponse(Stream.Null, 403, "Blocked", "Content-Type: text/plain");
        };
    }
}
