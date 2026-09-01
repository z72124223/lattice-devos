using Microsoft.Web.WebView2.Core;
using System.ComponentModel;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;

namespace Lattice.Control.Desktop;

public partial class MainWindow : Window
{
    private static readonly Uri DefaultControlUri = new("http://127.0.0.1:4317/");
    private readonly Uri _controlUri;

    public MainWindow()
    {
        InitializeComponent();
        _controlUri = ResolveControlUri(Environment.GetCommandLineArgs(), Environment.GetEnvironmentVariable("LATTICE_CONTROL_URL"));
        Loaded += MainWindow_Loaded;
    }

    private async void MainWindow_Loaded(object sender, RoutedEventArgs e)
    {
        await ConnectAsync();
    }

    private async Task ConnectAsync()
    {
        try
        {
            ControlView.Visibility = Visibility.Visible;
            ConnectionOverlay.Visibility = Visibility.Collapsed;
            ConnectionLabel.Text = "正在連線本機 LATTICE";
            ConnectionDot.Fill = new SolidColorBrush(Color.FromRgb(228, 167, 42));

            await ControlView.EnsureCoreWebView2Async();
            CoreWebView2 core = ControlView.CoreWebView2;
            core.Settings.AreDevToolsEnabled = false;
            core.Settings.AreDefaultContextMenusEnabled = false;
            core.Settings.AreBrowserAcceleratorKeysEnabled = false;
            core.Settings.IsStatusBarEnabled = false;
            core.Settings.IsZoomControlEnabled = false;
            core.NewWindowRequested -= Core_NewWindowRequested;
            core.NewWindowRequested += Core_NewWindowRequested;
            core.NavigationStarting -= Core_NavigationStarting;
            core.NavigationStarting += Core_NavigationStarting;
            core.NavigationCompleted -= Core_NavigationCompleted;
            core.NavigationCompleted += Core_NavigationCompleted;
            core.ProcessFailed -= Core_ProcessFailed;
            core.ProcessFailed += Core_ProcessFailed;
            core.Navigate(_controlUri.AbsoluteUri);
        }
        catch (Exception error)
        {
            ShowConnectionFailure($"無法啟動桌面顯示核心：{error.Message}");
        }
    }

    private void Core_NewWindowRequested(object? sender, CoreWebView2NewWindowRequestedEventArgs e)
    {
        e.Handled = true;
    }

    private void Core_NavigationStarting(object? sender, CoreWebView2NavigationStartingEventArgs e)
    {
        if (!Uri.TryCreate(e.Uri, UriKind.Absolute, out Uri? target) || !IsApprovedControlNavigation(target))
        {
            e.Cancel = true;
            ShowConnectionFailure("LATTICE 桌面程式只允許連線到這台電腦的控制核心。");
        }
    }

    private void Core_NavigationCompleted(object? sender, CoreWebView2NavigationCompletedEventArgs e)
    {
        if (!e.IsSuccess)
        {
            ShowConnectionFailure($"Control 尚未就緒（{e.WebErrorStatus}）。");
            return;
        }

        ConnectionOverlay.Visibility = Visibility.Collapsed;
        ControlView.Visibility = Visibility.Visible;
        ConnectionLabel.Text = "本機 LATTICE 已連線";
        ConnectionDot.Fill = new SolidColorBrush(Color.FromRgb(64, 224, 164));
    }

    private void Core_ProcessFailed(object? sender, CoreWebView2ProcessFailedEventArgs e)
    {
        ShowConnectionFailure("桌面顯示程序中斷，請重新連線。");
    }

    private void ShowConnectionFailure(string detail)
    {
        ControlView.Visibility = Visibility.Collapsed;
        ConnectionDetail.Text = detail;
        ConnectionOverlay.Visibility = Visibility.Visible;
        ConnectionLabel.Text = "LATTICE 未連線";
        ConnectionDot.Fill = new SolidColorBrush(Color.FromRgb(239, 95, 95));
    }

    private static Uri ResolveControlUri(IReadOnlyList<string> arguments, string? environmentValue)
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

    private static bool IsApprovedLoopback(Uri uri)
    {
        return string.Equals(uri.Scheme, Uri.UriSchemeHttp, StringComparison.OrdinalIgnoreCase)
            && string.Equals(uri.Host, "127.0.0.1", StringComparison.OrdinalIgnoreCase)
            && string.IsNullOrEmpty(uri.UserInfo)
            && !uri.IsDefaultPort;
    }

    private bool IsApprovedControlNavigation(Uri uri)
    {
        return IsApprovedLoopback(uri) && uri.Port == _controlUri.Port;
    }

    private void TitleBar_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        if (e.ClickCount == 2)
        {
            ToggleMaximize();
            return;
        }

        if (e.ButtonState == MouseButtonState.Pressed)
        {
            DragMove();
        }
    }

    private void Minimize_Click(object sender, RoutedEventArgs e) => WindowState = WindowState.Minimized;

    private void MaximizeRestore_Click(object sender, RoutedEventArgs e) => ToggleMaximize();

    private void Close_Click(object sender, RoutedEventArgs e) => Close();

    private async void Reconnect_Click(object sender, RoutedEventArgs e) => await ConnectAsync();

    private void ToggleMaximize()
    {
        WindowState = WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
    }

    protected override void OnClosing(CancelEventArgs e)
    {
        ControlView.Dispose();
        base.OnClosing(e);
    }
}
