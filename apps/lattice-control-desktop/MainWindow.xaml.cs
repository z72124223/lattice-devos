using Microsoft.Web.WebView2.Core;
using System.ComponentModel;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;

namespace Lattice.Control.Desktop;

public partial class MainWindow : Window
{
    private readonly Uri _controlUri;
    private readonly DispatcherTimer _reconnectTimer;
    private CoreWebView2Environment? _webViewEnvironment;
    private bool _eventsConfigured;
    private bool _isConnecting;
    private bool _isClosing;

    public MainWindow()
    {
        InitializeComponent();
        _controlUri = DesktopPolicy.ResolveControlUri(
            Environment.GetCommandLineArgs(),
            Environment.GetEnvironmentVariable("LATTICE_CONTROL_URL"));
        _reconnectTimer = new DispatcherTimer { Interval = DesktopPolicy.ReconnectInterval };
        _reconnectTimer.Tick += ReconnectTimer_Tick;
        Loaded += MainWindow_Loaded;
    }

    private async void MainWindow_Loaded(object sender, RoutedEventArgs e)
    {
        await ConnectAsync();
    }

    private async Task ConnectAsync()
    {
        if (_isClosing || _isConnecting) return;
        _isConnecting = true;
        _reconnectTimer.Stop();
        try
        {
            ShowConnectingState();

            _webViewEnvironment ??= await CoreWebView2Environment.CreateAsync(
                browserExecutableFolder: null,
                userDataFolder: DesktopPolicy.WebViewUserDataFolder);
            await ControlView.EnsureCoreWebView2Async(_webViewEnvironment);
            CoreWebView2 core = ControlView.CoreWebView2;
            core.Settings.AreDevToolsEnabled = false;
            core.Settings.AreDefaultContextMenusEnabled = false;
            core.Settings.AreBrowserAcceleratorKeysEnabled = false;
            core.Settings.IsStatusBarEnabled = false;
            core.Settings.IsZoomControlEnabled = false;
            if (!_eventsConfigured)
            {
                core.NewWindowRequested += Core_NewWindowRequested;
                core.NavigationStarting += Core_NavigationStarting;
                core.NavigationCompleted += Core_NavigationCompleted;
                core.ProcessFailed += Core_ProcessFailed;
                _eventsConfigured = true;
            }
            core.Navigate(_controlUri.AbsoluteUri);
        }
        catch (Exception error)
        {
            ShowConnectionFailure($"無法啟動桌面顯示核心：{error.Message}");
            ScheduleReconnect();
        }
        finally
        {
            _isConnecting = false;
        }
    }

    private async void ReconnectTimer_Tick(object? sender, EventArgs e)
    {
        await ConnectAsync();
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
            ShowConnectionFailure(
                "LATTICE 桌面程式只允許連線到這台電腦的控制核心。",
                "external_navigation_blocked");
            ScheduleReconnect();
        }
    }

    private void Core_NavigationCompleted(object? sender, CoreWebView2NavigationCompletedEventArgs e)
    {
        if (!e.IsSuccess)
        {
            ShowConnectionFailure($"Control 尚未就緒（{e.WebErrorStatus}）。");
            ScheduleReconnect();
            return;
        }

        _reconnectTimer.Stop();
        ConnectionOverlay.Visibility = Visibility.Collapsed;
        ControlView.Visibility = Visibility.Visible;
        ConnectionLabel.Text = "本機 LATTICE 已連線";
        AutomationProperties.SetItemStatus(ConnectionLabel, "connected");
        ConnectionDot.Fill = new SolidColorBrush(Color.FromRgb(64, 224, 164));
    }

    private void Core_ProcessFailed(object? sender, CoreWebView2ProcessFailedEventArgs e)
    {
        ShowConnectionFailure("桌面顯示程序中斷，請重新連線。");
        ScheduleReconnect();
    }

    private void ShowConnectionFailure(string detail, string itemStatus = "offline")
    {
        ControlView.Visibility = Visibility.Collapsed;
        ConnectionDetail.Text = detail;
        ConnectionOverlay.Visibility = Visibility.Visible;
        ConnectionLabel.Text = "LATTICE 未連線";
        AutomationProperties.SetItemStatus(ConnectionLabel, itemStatus);
        ConnectionDot.Fill = new SolidColorBrush(Color.FromRgb(239, 95, 95));
    }

    private void ShowConnectingState()
    {
        ControlView.Visibility = Visibility.Collapsed;
        ConnectionOverlay.Visibility = Visibility.Visible;
        ConnectionDetail.Text = "正在嘗試連線到本機 Control；視窗會保持開啟並自動重試。";
        ConnectionLabel.Text = "正在連線本機 LATTICE";
        AutomationProperties.SetItemStatus(ConnectionLabel, "connecting");
        ConnectionDot.Fill = new SolidColorBrush(Color.FromRgb(228, 167, 42));
    }

    private bool IsApprovedControlNavigation(Uri uri)
    {
        return DesktopPolicy.IsApprovedControlNavigation(uri, _controlUri);
    }

    private void ScheduleReconnect()
    {
        if (_isClosing || _reconnectTimer.IsEnabled) return;
        _reconnectTimer.Start();
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

    private async void Reconnect_Click(object sender, RoutedEventArgs e)
    {
        _reconnectTimer.Stop();
        await ConnectAsync();
    }

    private void ToggleMaximize()
    {
        WindowState = WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
    }

    protected override void OnClosing(CancelEventArgs e)
    {
        _isClosing = true;
        _reconnectTimer.Stop();
        _reconnectTimer.Tick -= ReconnectTimer_Tick;
        ControlView.Dispose();
        base.OnClosing(e);
    }
}
