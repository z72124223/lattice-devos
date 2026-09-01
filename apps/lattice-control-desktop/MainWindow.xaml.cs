using Microsoft.Web.WebView2.Core;
using System.ComponentModel;
using System.IO;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;

namespace Lattice.Control.Desktop;

public partial class MainWindow : Window
{
    private readonly Uri _controlUri;
    private readonly ControlRuntimeManager? _controlRuntime;
    private readonly CancellationTokenSource _lifetimeCancellation = new();
    private readonly DispatcherTimer _reconnectTimer;
    private readonly DispatcherTimer _healthTimer;
    private readonly HashSet<ulong> _blockedNavigationIds = new();
    private CoreWebView2Environment? _webViewEnvironment;
    private ulong? _currentNavigationId;
    private bool _eventsConfigured;
    private bool _isConnecting;
    private bool _isClosing;

    public MainWindow()
    {
        InitializeComponent();
        _controlUri = DesktopPolicy.ResolveControlUri(
            Environment.GetCommandLineArgs(),
            Environment.GetEnvironmentVariable("LATTICE_CONTROL_URL"));
        if (DesktopPolicy.ShouldManageControl(_controlUri))
        {
            _controlRuntime = ControlRuntimeManager.CreatePackaged(_controlUri);
        }
        _reconnectTimer = new DispatcherTimer { Interval = DesktopPolicy.ReconnectInterval };
        _reconnectTimer.Tick += ReconnectTimer_Tick;
        _healthTimer = new DispatcherTimer { Interval = DesktopPolicy.RuntimeHealthInterval };
        _healthTimer.Tick += HealthTimer_Tick;
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

            if (_controlRuntime is not null)
            {
                ControlRuntimeEvaluation runtime = await _controlRuntime.EnsureReadyAsync(
                    _lifetimeCancellation.Token);
                if (runtime.Health != ControlRuntimeHealth.HEALTHY)
                {
                    ShowRuntimeFailure(runtime);
                    ScheduleReconnect();
                    return;
                }
                SetRuntimeStatus(ControlRuntimeHealth.HEALTHY);
            }

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
                core.AddWebResourceRequestedFilter("*", CoreWebView2WebResourceContext.Document);
                core.WebResourceRequested += Core_WebResourceRequested;
                core.ProcessFailed += Core_ProcessFailed;
                _eventsConfigured = true;
            }
            _currentNavigationId = null;
            core.Navigate(_controlUri.AbsoluteUri);
        }
        catch (OperationCanceledException) when (_isClosing)
        {
        }
        catch (ObjectDisposedException) when (_isClosing)
        {
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

    private async void HealthTimer_Tick(object? sender, EventArgs e)
    {
        _healthTimer.Stop();
        if (_isClosing || _controlRuntime is null) return;
        try
        {
            ControlRuntimeEvaluation runtime = await _controlRuntime.ProbeAsync(
                _lifetimeCancellation.Token);
            if (runtime.Health != ControlRuntimeHealth.HEALTHY)
            {
                ShowRuntimeFailure(runtime);
                ScheduleReconnect();
                return;
            }
            _healthTimer.Start();
        }
        catch (OperationCanceledException) when (_isClosing)
        {
        }
        catch (ObjectDisposedException) when (_isClosing)
        {
        }
        catch (Exception error)
        {
            SetRuntimeStatus(ControlRuntimeHealth.UNREACHABLE);
            ShowConnectionFailure($"Control 健康檢查失敗：{error.Message}");
            ScheduleReconnect();
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
            BlockNavigation(e.NavigationId);
            return;
        }

        _blockedNavigationIds.Clear();
        _currentNavigationId = e.NavigationId;
    }

    private void Core_WebResourceRequested(object? sender, CoreWebView2WebResourceRequestedEventArgs e)
    {
        if (Uri.TryCreate(e.Request.Uri, UriKind.Absolute, out Uri? target) &&
            IsApprovedControlNavigation(target))
        {
            return;
        }

        if (sender is CoreWebView2 core)
        {
            e.Response = core.Environment.CreateWebResourceResponse(
                Stream.Null,
                403,
                "Blocked",
                "Content-Type: text/plain");
        }
        BlockNavigation(_currentNavigationId);
    }

    private async void Core_NavigationCompleted(object? sender, CoreWebView2NavigationCompletedEventArgs e)
    {
        bool wasBlocked = _blockedNavigationIds.Remove(e.NavigationId);
        if (!DesktopPolicy.CanApplyNavigationResult(
            _currentNavigationId,
            e.NavigationId,
            _isClosing))
        {
            return;
        }
        if (wasBlocked)
        {
            ShowBlockedNavigation();
            return;
        }

        if (!e.IsSuccess)
        {
            bool applied = await ShowDiagnosedConnectionFailureAsync(
                e.NavigationId,
                $"Control 尚未就緒（{e.WebErrorStatus}）。");
            if (applied) ScheduleReconnect();
            return;
        }

        if (ControlView.Source is not Uri source || !IsApprovedControlNavigation(source))
        {
            BlockNavigation(e.NavigationId);
            return;
        }

        _reconnectTimer.Stop();
        ConnectionOverlay.Visibility = Visibility.Collapsed;
        ControlView.Visibility = Visibility.Visible;
        ConnectionLabel.Text = "本機 LATTICE 已連線";
        AutomationProperties.SetItemStatus(ConnectionLabel, "connected");
        ConnectionDot.Fill = new SolidColorBrush(Color.FromRgb(64, 224, 164));
        SetRuntimeStatus(ControlRuntimeHealth.HEALTHY);
        if (_controlRuntime is not null)
        {
            _healthTimer.Start();
        }
    }

    private void Core_ProcessFailed(object? sender, CoreWebView2ProcessFailedEventArgs e)
    {
        ShowConnectionFailure("桌面顯示程序中斷，請重新連線。");
        ScheduleReconnect();
    }

    private async Task<bool> ShowDiagnosedConnectionFailureAsync(
        ulong navigationId,
        string fallbackDetail)
    {
        if (!DesktopPolicy.CanApplyNavigationResult(
            _currentNavigationId,
            navigationId,
            _isClosing)) return false;
        if (_controlRuntime is not null)
        {
            try
            {
                ControlRuntimeEvaluation runtime = await _controlRuntime.ProbeAsync(
                    _lifetimeCancellation.Token);
                if (!DesktopPolicy.CanApplyNavigationResult(
                    _currentNavigationId,
                    navigationId,
                    _isClosing)) return false;
                if (runtime.Health != ControlRuntimeHealth.HEALTHY)
                {
                    ShowRuntimeFailure(runtime);
                    return true;
                }
            }
            catch (OperationCanceledException) when (_isClosing)
            {
                return false;
            }
            catch (ObjectDisposedException) when (_isClosing)
            {
                return false;
            }
            catch (Exception error)
            {
                if (!DesktopPolicy.CanApplyNavigationResult(
                    _currentNavigationId,
                    navigationId,
                    _isClosing)) return false;
                ShowConnectionFailure($"Control 診斷失敗：{error.Message}");
                return true;
            }
        }
        if (!DesktopPolicy.CanApplyNavigationResult(
            _currentNavigationId,
            navigationId,
            _isClosing)) return false;
        ShowConnectionFailure(fallbackDetail);
        return true;
    }

    private void ShowRuntimeFailure(ControlRuntimeEvaluation runtime)
    {
        string detail = runtime.Health switch
        {
            ControlRuntimeHealth.STOPPED =>
                "Control 程序已停止；LATTICE 會在下一次重連啟動新的自有程序。",
            ControlRuntimeHealth.INCOMPATIBLE =>
                "127.0.0.1:4317 已有陌生或不相容的服務；LATTICE 已停止接管，也不會關閉它。",
            ControlRuntimeHealth.UNREACHABLE =>
                "Control 無法連線；只有確認 4317 沒有 listener 時，LATTICE 才會啟動封裝版本。",
            _ => $"Control 目前狀態：{runtime.Health}。",
        };
        SetRuntimeStatus(runtime.Health);
        ShowConnectionFailure(detail, runtime.Health.ToString().ToLowerInvariant());
    }

    private void ShowConnectionFailure(
        string detail,
        string itemStatus = "offline")
    {
        _healthTimer.Stop();
        ControlView.Visibility = Visibility.Collapsed;
        ConnectionDetail.Text = detail;
        ConnectionOverlay.Visibility = Visibility.Visible;
        ConnectionLabel.Text = "LATTICE 未連線";
        AutomationProperties.SetItemStatus(ConnectionLabel, itemStatus);
        ConnectionDot.Fill = new SolidColorBrush(Color.FromRgb(239, 95, 95));
    }

    private void BlockNavigation(ulong? navigationId)
    {
        if (navigationId.HasValue)
        {
            _blockedNavigationIds.Add(navigationId.Value);
        }
        ShowBlockedNavigation();
    }

    private void ShowBlockedNavigation()
    {
        ShowConnectionFailure(
            "LATTICE 桌面程式只允許連線到這台電腦的控制核心。",
            "external_navigation_blocked");
        ScheduleReconnect();
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

    private void SetRuntimeStatus(ControlRuntimeHealth health)
    {
        RuntimeStatusLabel.Text = health.ToString();
        AutomationProperties.SetItemStatus(RuntimeStatusLabel, health.ToString());
        RuntimeStatusLabel.Foreground = new SolidColorBrush(health switch
        {
            ControlRuntimeHealth.HEALTHY => Color.FromRgb(64, 224, 164),
            ControlRuntimeHealth.STOPPED => Color.FromRgb(228, 167, 42),
            ControlRuntimeHealth.INCOMPATIBLE => Color.FromRgb(173, 120, 255),
            ControlRuntimeHealth.NO_DATA => Color.FromRgb(69, 167, 202),
            ControlRuntimeHealth.NOT_IMPLEMENTED => Color.FromRgb(124, 135, 144),
            _ => Color.FromRgb(239, 95, 95),
        });
    }

    private bool IsApprovedControlNavigation(Uri uri)
    {
        return DesktopPolicy.IsApprovedControlNavigation(uri, _controlUri);
    }

    private void ScheduleReconnect()
    {
        _healthTimer.Stop();
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
        _lifetimeCancellation.Cancel();
        _reconnectTimer.Stop();
        _healthTimer.Stop();
        _reconnectTimer.Tick -= ReconnectTimer_Tick;
        _healthTimer.Tick -= HealthTimer_Tick;
        _controlRuntime?.Dispose();
        _lifetimeCancellation.Dispose();
        ControlView.Dispose();
        base.OnClosing(e);
    }
}
