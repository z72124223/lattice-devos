using Microsoft.Web.WebView2.Core;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Shell;
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
    private HwndSource? _windowSource;
    private ulong? _currentNavigationId;
    private long _navigationGeneration;
    private bool _eventsConfigured;
    private bool _isConnecting;
    private bool _isClosing;
    private bool _shutdownComplete;
    private bool _shutdownFailed;

    public MainWindow()
    {
        // Register first so this hit-test hook precedes WindowChrome's source hook.
        SourceInitialized += MainWindow_SourceInitialized;
        InitializeComponent();
        ControlEndpointSelection controlTarget = DesktopPolicy.ResolveControlTarget(
            Environment.GetCommandLineArgs(),
            Environment.GetEnvironmentVariable("LATTICE_CONTROL_URL"));
        _controlUri = controlTarget.Uri;
        if (controlTarget.ManageControl)
        {
            _controlRuntime = ControlRuntimeManager.CreatePackaged(_controlUri);
        }
        _reconnectTimer = new DispatcherTimer { Interval = DesktopPolicy.ReconnectInterval };
        _reconnectTimer.Tick += ReconnectTimer_Tick;
        _healthTimer = new DispatcherTimer { Interval = DesktopPolicy.RuntimeHealthInterval };
        _healthTimer.Tick += HealthTimer_Tick;
        Loaded += MainWindow_Loaded;
    }

    private void MainWindow_SourceInitialized(object? sender, EventArgs e)
    {
        nint handle = new WindowInteropHelper(this).Handle;
        _windowSource = HwndSource.FromHwnd(handle);
        _windowSource?.AddHook(WindowProc);
    }

    private nint WindowProc(
        nint windowHandle,
        int message,
        nint wordParameter,
        nint longParameter,
        ref bool handled)
    {
        if (
            message != WindowResizeHitTestPolicy.WmNcHitTest
            || ResizeMode is not (ResizeMode.CanResize or ResizeMode.CanResizeWithGrip)
            || !GetWindowRect(windowHandle, out NativeRect windowRect))
        {
            return nint.Zero;
        }

        long packedPoint = longParameter.ToInt64();
        int screenX = unchecked((short)(packedPoint & 0xffff));
        int screenY = unchecked((short)((packedPoint >> 16) & 0xffff));
        DpiScale dpi = VisualTreeHelper.GetDpi(this);
        Thickness border = WindowChrome.GetWindowChrome(this)?.ResizeBorderThickness
            ?? new Thickness(7);
        WindowResizeHit hit = WindowResizeHitTestPolicy.EvaluatePhysical(
            screenX,
            screenY,
            windowRect.Left,
            windowRect.Top,
            windowRect.Right,
            windowRect.Bottom,
            dpi.DpiScaleX,
            dpi.DpiScaleY,
            new WindowResizeInsets(border.Left, border.Top, border.Right, border.Bottom),
            WindowState == WindowState.Maximized);
        if (hit == WindowResizeHit.Client) return nint.Zero;

        handled = true;
        return (nint)(int)hit;
    }

    protected override void OnClosed(EventArgs e)
    {
        SourceInitialized -= MainWindow_SourceInitialized;
        _windowSource?.RemoveHook(WindowProc);
        _windowSource = null;
        base.OnClosed(e);
    }

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetWindowRect(nint windowHandle, out NativeRect windowRect);

    [StructLayout(LayoutKind.Sequential)]
    private struct NativeRect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    private async void MainWindow_Loaded(object sender, RoutedEventArgs e)
    {
        await ConnectAsync();
    }

    private async Task ConnectAsync()
    {
        if (_isClosing || _isConnecting) return;
        _isConnecting = true;
        _navigationGeneration = DesktopPolicy.NextNavigationGeneration(_navigationGeneration);
        _currentNavigationId = null;
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
                    if (ShowRuntimeFailure(runtime)) ScheduleReconnect();
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
                if (ShowRuntimeFailure(runtime)) ScheduleReconnect();
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
        _currentNavigationId = e.NavigationId;
        _navigationGeneration = DesktopPolicy.NextNavigationGeneration(_navigationGeneration);
        if (!Uri.TryCreate(e.Uri, UriKind.Absolute, out Uri? target) || !IsApprovedControlNavigation(target))
        {
            e.Cancel = true;
            BlockNavigation(e.NavigationId, advanceGeneration: false);
            return;
        }

        _blockedNavigationIds.Clear();
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
        long navigationGeneration = _navigationGeneration;
        bool wasBlocked = _blockedNavigationIds.Remove(e.NavigationId);
        if (!DesktopPolicy.CanApplyNavigationResult(
            _currentNavigationId,
            e.NavigationId,
            _navigationGeneration,
            navigationGeneration,
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
                navigationGeneration,
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
        long navigationGeneration,
        string fallbackDetail)
    {
        if (!DesktopPolicy.CanApplyNavigationResult(
            _currentNavigationId,
            navigationId,
            _navigationGeneration,
            navigationGeneration,
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
                    _navigationGeneration,
                    navigationGeneration,
                    _isClosing)) return false;
                if (runtime.Health != ControlRuntimeHealth.HEALTHY)
                {
                    return ShowRuntimeFailure(runtime);
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
                    _navigationGeneration,
                    navigationGeneration,
                    _isClosing)) return false;
                ShowConnectionFailure($"Control 診斷失敗：{error.Message}");
                return true;
            }
        }
        if (!DesktopPolicy.CanApplyNavigationResult(
            _currentNavigationId,
            navigationId,
            _navigationGeneration,
            navigationGeneration,
            _isClosing)) return false;
        ShowConnectionFailure(fallbackDetail);
        return true;
    }

    private bool ShowRuntimeFailure(ControlRuntimeEvaluation runtime)
    {
        ControlRuntimeFailurePresentation presentation = DesktopPolicy.DescribeRuntimeFailure(runtime);
        SetRuntimeStatus(runtime.Health);
        ShowConnectionFailure(presentation.Detail, runtime.Health.ToString().ToLowerInvariant());
        return presentation.AutoReconnect;
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

    private void BlockNavigation(ulong? navigationId, bool advanceGeneration = true)
    {
        if (advanceGeneration)
        {
            _navigationGeneration = DesktopPolicy.NextNavigationGeneration(_navigationGeneration);
        }
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
        if (_shutdownFailed)
        {
            _shutdownFailed = false;
            _isClosing = true;
            ReconnectButton.IsEnabled = false;
            await CompleteShutdownAsync();
            return;
        }
        _reconnectTimer.Stop();
        await ConnectAsync();
    }

    private void ToggleMaximize()
    {
        WindowState = WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
    }

    protected override void OnClosing(CancelEventArgs e)
    {
        if (_shutdownComplete)
        {
            base.OnClosing(e);
            return;
        }
        e.Cancel = true;
        if (_isClosing) return;
        _isClosing = true;
        _lifetimeCancellation.Cancel();
        _reconnectTimer.Stop();
        _healthTimer.Stop();
        _reconnectTimer.Tick -= ReconnectTimer_Tick;
        _healthTimer.Tick -= HealthTimer_Tick;
        ReconnectButton.IsEnabled = false;
        _ = CompleteShutdownAsync();
    }

    private async Task CompleteShutdownAsync()
    {
        try
        {
            if (_controlRuntime is not null)
            {
                await _controlRuntime.ShutdownAsync(CancellationToken.None);
            }
        }
        catch (Exception error)
        {
            if (_controlRuntime?.OwnsControl == true)
            {
                _shutdownFailed = true;
                _isClosing = false;
                SetRuntimeStatus(ControlRuntimeHealth.UNREACHABLE);
                ShowConnectionFailure(
                    $"無法安全停止這個桌面啟動的 Control：{error.Message}。請按下「重試安全關閉」。",
                    "shutdown_failed");
                ReconnectButton.Content = "重試安全關閉";
                ReconnectButton.IsEnabled = true;
                AutomationProperties.SetItemStatus(ReconnectButton, "retry_safe_shutdown");
                return;
            }
        }
        _lifetimeCancellation.Dispose();
        ControlView.Dispose();
        _shutdownComplete = true;
        _ = Dispatcher.BeginInvoke(Close);
    }
}
