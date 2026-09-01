using Lattice.Control.Desktop;
using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text;

static void Require(bool condition, string message)
{
    if (!condition) throw new InvalidOperationException(message);
}

static int ReservePort()
{
    TcpListener listener = new(IPAddress.Loopback, 0);
    listener.Start();
    int port = ((IPEndPoint)listener.LocalEndpoint).Port;
    listener.Stop();
    return port;
}

static string FindNode()
{
    foreach (string segment in (Environment.GetEnvironmentVariable("PATH") ?? "")
        .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
    {
        string candidate = Path.Combine(segment, OperatingSystem.IsWindows() ? "node.exe" : "node");
        if (File.Exists(candidate)) return Path.GetFullPath(candidate);
    }
    throw new InvalidOperationException("node executable was not found");
}

static ControlRuntimeLaunchSpec LaunchSpec(
    string repositoryRoot,
    string nodePath,
    int port,
    string localData,
    string? serverPath = null)
{
    string databasePath = Path.GetFullPath(Path.Combine(
        localData,
        "LATTICE",
        "control",
        "lattice-control.db"));
    return new(
        nodePath,
        serverPath ?? Path.Combine(repositoryRoot, "apps", "lattice-control", "src", "server.mjs"),
        repositoryRoot,
        new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
        {
            ["LATTICE_CONTROL_PORT"] = port.ToString(System.Globalization.CultureInfo.InvariantCulture),
            ["LOCALAPPDATA"] = localData,
            ["LATTICE_CONTROL_DATABASE_PATH"] = databasePath,
            ["LATTICE_CONTROL_DESKTOP_OWNED"] = "1",
        },
        databasePath);
}

static Process StartExternal(ControlRuntimeLaunchSpec spec)
{
    ProcessStartInfo start = new()
    {
        FileName = spec.ExecutablePath,
        WorkingDirectory = spec.WorkingDirectory,
        UseShellExecute = false,
        CreateNoWindow = true,
    };
    start.ArgumentList.Add(spec.ServerPath);
    foreach ((string key, string value) in spec.Environment)
    {
        start.Environment[key] = value;
    }
    return Process.Start(start) ?? throw new InvalidOperationException("external Control did not start");
}

static async Task WaitForRuntimeAsync(Uri origin, Process process)
{
    using HttpClient client = new(new SocketsHttpHandler { UseProxy = false });
    DateTimeOffset deadline = DateTimeOffset.UtcNow.AddSeconds(10);
    while (DateTimeOffset.UtcNow < deadline)
    {
        if (process.HasExited) throw new InvalidOperationException("Control exited before its runtime probe was ready");
        try
        {
            using HttpResponseMessage response = await client.GetAsync(new Uri(origin, "api/runtime"));
            if (response.StatusCode == HttpStatusCode.OK) return;
        }
        catch (HttpRequestException)
        {
        }
        await Task.Delay(100);
    }
    throw new TimeoutException("Control runtime probe did not become ready");
}

static async Task<int> WaitForPidFileAsync(string path)
{
    DateTimeOffset deadline = DateTimeOffset.UtcNow.AddSeconds(5);
    while (DateTimeOffset.UtcNow < deadline)
    {
        if (File.Exists(path))
        {
            string text = await File.ReadAllTextAsync(path);
            if (int.TryParse(text, out int pid) && pid > 0) return pid;
        }
        await Task.Delay(50);
    }
    throw new TimeoutException("owned Control fixture did not publish its child PID");
}

static void StopTestProcess(Process? process)
{
    if (process is null) return;
    try
    {
        if (!process.HasExited)
        {
            process.Kill(entireProcessTree: true);
            process.WaitForExit(5_000);
        }
    }
    finally
    {
        process.Dispose();
    }
}

string repositoryRoot = Path.GetFullPath(Environment.CurrentDirectory);
string nodePath = FindNode();
string temporaryRoot = Path.Combine(Path.GetTempPath(), $"lattice-control-lifecycle-{Guid.NewGuid():N}");
Directory.CreateDirectory(temporaryRoot);

try
{
    using (MemoryStream oversizedProbe = new(new byte[65_537], writable: false))
    {
        var boundedBody = await ControlRuntimeManager.ReadProbeBodyAsync(
            oversizedProbe,
            CancellationToken.None);
        Require(boundedBody.TooLarge, "oversized probe body was not rejected at the byte boundary");
        Require(boundedBody.Body.Length == 0, "oversized probe body retained content");
    }

    int disposeRacePort = ReservePort();
    string disposeRaceData = Path.Combine(temporaryRoot, "dispose-race-data");
    ControlRuntimeLaunchSpec disposeRaceSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        disposeRacePort,
        disposeRaceData);
    using (BlockingProbeHandler blockingProbe = new())
    {
        ControlRuntimeManager manager = new(
            new Uri($"http://127.0.0.1:{disposeRacePort}/"),
            disposeRaceSpec,
            probeHandler: blockingProbe);
        Task<ControlRuntimeEvaluation> pendingEnsure = manager.EnsureReadyAsync();
        await blockingProbe.Started.WaitAsync(TimeSpan.FromSeconds(5));
        manager.Dispose();
        blockingProbe.Release();
        bool disposedFailure = false;
        try
        {
            await pendingEnsure;
        }
        catch (ObjectDisposedException)
        {
            disposedFailure = true;
        }
        Require(disposedFailure, "concurrent dispose did not stop EnsureReady before launch");
        Require(
            !File.Exists(Path.Combine(disposeRaceData, "LATTICE", "control", "lattice-control.db")),
            "concurrent dispose allowed a post-dispose Control launch");
    }

    int missingFilesPort = ReservePort();
    string missingFilesData = Path.Combine(temporaryRoot, "missing-files-data");
    ControlRuntimeLaunchSpec missingFilesSpec = LaunchSpec(
        repositoryRoot,
        Path.Combine(temporaryRoot, "missing-node.exe"),
        missingFilesPort,
        missingFilesData);
    using (ControlRuntimeManager manager = new(
        new Uri($"http://127.0.0.1:{missingFilesPort}/"),
        missingFilesSpec,
        probeTimeout: TimeSpan.FromMilliseconds(250)))
    {
        ControlRuntimeEvaluation result = await manager.EnsureReadyAsync();
        Require(result.Health == ControlRuntimeHealth.STOPPED,
            "missing runtime files did not produce STOPPED");
        Require(result.Action == ControlRuntimeAction.FailClosed,
            "missing runtime files did not fail closed");
        Require(result.Detail == "CONTROL_RUNTIME_FILES_MISSING",
            "missing runtime files did not preserve the exact detail");
        Require(!manager.OwnsControl, "missing runtime files created an owned process");
    }

    int startFailurePort = ReservePort();
    string startFailureData = Path.Combine(temporaryRoot, "start-failure-data");
    string invalidExecutable = Path.Combine(temporaryRoot, "not-an-executable.txt");
    await File.WriteAllTextAsync(invalidExecutable, "not an executable");
    ControlRuntimeLaunchSpec startFailureSpec = LaunchSpec(
        repositoryRoot,
        invalidExecutable,
        startFailurePort,
        startFailureData);
    using (ControlRuntimeManager manager = new(
        new Uri($"http://127.0.0.1:{startFailurePort}/"),
        startFailureSpec,
        probeTimeout: TimeSpan.FromMilliseconds(250)))
    {
        ControlRuntimeEvaluation result = await manager.EnsureReadyAsync();
        Require(result.Health == ControlRuntimeHealth.STOPPED,
            "process start failure did not produce STOPPED");
        Require(result.Action == ControlRuntimeAction.FailClosed,
            "process start failure did not fail closed");
        Require(result.Detail == "CONTROL_PROCESS_START_FAILED",
            "process start failure did not preserve the exact detail");
        Require(!manager.OwnsControl, "process start failure retained an owned process");
    }

    int reusedPort = ReservePort();
    Uri reusedOrigin = new($"http://127.0.0.1:{reusedPort}/");
    ControlRuntimeLaunchSpec reusedSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        reusedPort,
        Path.Combine(temporaryRoot, "reuse-data"));
    Process? external = null;
    try
    {
        external = StartExternal(reusedSpec);
        await WaitForRuntimeAsync(reusedOrigin, external);
        using (ControlRuntimeManager manager = new(reusedOrigin, reusedSpec))
        {
            ControlRuntimeEvaluation result = await manager.EnsureReadyAsync();
            Require(result.Health == ControlRuntimeHealth.HEALTHY, "compatible external Control was not healthy");
            Require(!manager.OwnsControl, "compatible external Control was incorrectly owned");
        }
        Require(!external.HasExited, "disposing the desktop manager stopped a reused Control");
    }
    finally
    {
        StopTestProcess(external);
    }

    int crossScopePort = ReservePort();
    Uri crossScopeOrigin = new($"http://127.0.0.1:{crossScopePort}/");
    ControlRuntimeLaunchSpec externalScopeSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        crossScopePort,
        Path.Combine(temporaryRoot, "cross-scope-external-data"));
    ControlRuntimeLaunchSpec desktopScopeSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        crossScopePort,
        Path.Combine(temporaryRoot, "cross-scope-desktop-data"));
    Process? crossScopeExternal = null;
    try
    {
        crossScopeExternal = StartExternal(externalScopeSpec);
        await WaitForRuntimeAsync(crossScopeOrigin, crossScopeExternal);
        using (ControlRuntimeManager manager = new(crossScopeOrigin, desktopScopeSpec))
        {
            ControlRuntimeEvaluation result = await manager.EnsureReadyAsync();
            Require(result.Health == ControlRuntimeHealth.INCOMPATIBLE,
                "same-version different-scope Control was not incompatible");
            Require(result.Detail == "CONTROL_DATA_SCOPE_INCOMPATIBLE",
                "different data scope did not produce the exact fail-closed detail");
            Require(!manager.OwnsControl,
                "same-version different-scope Control was incorrectly owned");
        }
        Require(!crossScopeExternal.HasExited,
            "disposing the desktop manager stopped a different-scope Control");
    }
    finally
    {
        StopTestProcess(crossScopeExternal);
    }

    int foreignPort = ReservePort();
    using (ForeignRuntimeServer foreign = new(foreignPort))
    {
        await foreign.StartAsync();
        Uri foreignOrigin = new($"http://127.0.0.1:{foreignPort}/");
        ControlRuntimeLaunchSpec foreignSpec = LaunchSpec(
            repositoryRoot,
            nodePath,
            foreignPort,
            Path.Combine(temporaryRoot, "foreign-data"));
        using (ControlRuntimeManager manager = new(foreignOrigin, foreignSpec))
        {
            ControlRuntimeEvaluation result = await manager.EnsureReadyAsync();
            Require(result.Health == ControlRuntimeHealth.INCOMPATIBLE, "foreign Control was not incompatible");
            Require(result.Action == ControlRuntimeAction.FailClosed, "foreign Control did not fail closed");
            Require(!manager.OwnsControl, "foreign Control was incorrectly owned");
        }
        Require(foreign.IsRunning, "disposing the desktop manager stopped a foreign listener");
    }

    int oversizedForeignPort = ReservePort();
    using (ForeignRuntimeServer oversizedForeign = new(oversizedForeignPort, chunkedOversized: true))
    {
        await oversizedForeign.StartAsync();
        Uri oversizedForeignOrigin = new($"http://127.0.0.1:{oversizedForeignPort}/");
        ControlRuntimeLaunchSpec oversizedForeignSpec = LaunchSpec(
            repositoryRoot,
            nodePath,
            oversizedForeignPort,
            Path.Combine(temporaryRoot, "oversized-foreign-data"));
        using ControlRuntimeManager manager = new(oversizedForeignOrigin, oversizedForeignSpec);
        ControlRuntimeEvaluation result = await manager.EnsureReadyAsync();
        Require(result.Health == ControlRuntimeHealth.INCOMPATIBLE, "chunked oversized listener was not incompatible");
        Require(!manager.OwnsControl, "chunked oversized listener was incorrectly owned");
        Require(oversizedForeign.IsRunning, "chunked oversized listener was stopped by the manager");
    }

    int ownedPort = ReservePort();
    Uri ownedOrigin = new($"http://127.0.0.1:{ownedPort}/");
    ControlRuntimeLaunchSpec ownedSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        ownedPort,
        Path.Combine(temporaryRoot, "owned-data"));
    int firstOwnedPid;
    int replacementOwnedPid;
    ControlRuntimeManager ownedManager = new(ownedOrigin, ownedSpec);
    using (ownedManager)
    {
        ControlRuntimeEvaluation started = await ownedManager.EnsureReadyAsync();
        Require(started.Health == ControlRuntimeHealth.HEALTHY, "absent Control did not auto-start");
        Require(ownedManager.OwnsControl, "auto-started Control was not owned");
        firstOwnedPid = ownedManager.OwnedProcessId ?? throw new InvalidOperationException("owned PID missing");

        using (Process interrupted = Process.GetProcessById(firstOwnedPid))
        {
            interrupted.Kill(entireProcessTree: true);
            Require(interrupted.WaitForExit(5_000), "owned Control interruption timed out");
        }
        ControlRuntimeEvaluation stopped = await ownedManager.ProbeAsync();
        Require(stopped.Health == ControlRuntimeHealth.STOPPED, "owned Control interruption was not diagnosed");

        ControlRuntimeEvaluation reconnected = await ownedManager.EnsureReadyAsync();
        Require(reconnected.Health == ControlRuntimeHealth.HEALTHY, "owned Control did not reconnect");
        Require(ownedManager.OwnsControl, "replacement Control was not owned");
        replacementOwnedPid = ownedManager.OwnedProcessId ?? throw new InvalidOperationException("replacement PID missing");
        Require(replacementOwnedPid != firstOwnedPid, "owned Control reconnect reused the interrupted PID");
    }
    Require(!ownedManager.LastStopUsedHardKill,
        "clean owned Control close unexpectedly used hard kill");

    try
    {
        using Process replacement = Process.GetProcessById(replacementOwnedPid);
        Require(replacement.HasExited, "disposing the manager did not stop its owned Control");
    }
    catch (ArgumentException)
    {
    }

    int alreadyExitedPort = ReservePort();
    ControlRuntimeLaunchSpec alreadyExitedSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        alreadyExitedPort,
        Path.Combine(temporaryRoot, "already-exited-data"));
    ControlRuntimeManager alreadyExitedManager = new(
        new Uri($"http://127.0.0.1:{alreadyExitedPort}/"),
        alreadyExitedSpec);
    try
    {
        ControlRuntimeEvaluation started = await alreadyExitedManager.EnsureReadyAsync();
        Require(started.Health == ControlRuntimeHealth.HEALTHY,
            "already-exited fixture did not start as compatible owned Control");
        int processId = alreadyExitedManager.OwnedProcessId
            ?? throw new InvalidOperationException("already-exited owned PID missing");
        using (Process owned = Process.GetProcessById(processId))
        {
            owned.Kill(entireProcessTree: true);
            await owned.WaitForExitAsync();
        }
        Task? stopped = alreadyExitedManager.StopOwnedCoreAsync(CancellationToken.None);
        Require(stopped is not null,
            "already-exited owned stop returned a null shared task");
        await (stopped ?? throw new InvalidOperationException(
            "already-exited owned stop returned a null shared task"));
    }
    finally
    {
        await alreadyExitedManager.ShutdownAsync(CancellationToken.None);
    }

    int rejectedShutdownPort = ReservePort();
    ControlRuntimeLaunchSpec rejectedShutdownSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        rejectedShutdownPort,
        Path.Combine(temporaryRoot, "rejected-shutdown-data"),
        Path.Combine(
            repositoryRoot,
            "apps",
            "lattice-control",
            "test",
            "fixtures",
            "desktop-owned-shutdown-reject.mjs"));
    ControlRuntimeManager rejectedShutdownManager = new(
        new Uri($"http://127.0.0.1:{rejectedShutdownPort}/"),
        rejectedShutdownSpec);
    try
    {
        ControlRuntimeEvaluation started = await rejectedShutdownManager.EnsureReadyAsync();
        Require(started.Health == ControlRuntimeHealth.HEALTHY,
            "shutdown-rejection fixture did not become a compatible owned Control");
        bool rejected = false;
        try
        {
            await rejectedShutdownManager.StopOwnedCoreAsync(CancellationToken.None);
        }
        catch (InvalidOperationException error)
            when (error.Message == "CONTROL_OWNED_PROCESS_SHUTDOWN_REJECTED")
        {
            rejected = true;
        }
        Require(rejected, "owned Control exit 1 was incorrectly accepted as graceful shutdown");
        Require(!rejectedShutdownManager.LastStopUsedHardKill,
            "an explicit shutdown NACK was misclassified as the timeout hard-kill fallback");
    }
    finally
    {
        await rejectedShutdownManager.ShutdownAsync(CancellationToken.None);
    }

    int hungShutdownPort = ReservePort();
    Uri hungShutdownOrigin = new($"http://127.0.0.1:{hungShutdownPort}/");
    ControlRuntimeLaunchSpec hungShutdownSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        hungShutdownPort,
        Path.Combine(temporaryRoot, "hung-shutdown-data"),
        Path.Combine(
            repositoryRoot,
            "apps",
            "lattice-control",
            "test",
            "fixtures",
            "desktop-owned-shutdown-hang.mjs"));
    bool allowHungShutdownKill = true;
    ControlRuntimeManager hungShutdownManager = new(
        hungShutdownOrigin,
        hungShutdownSpec,
        shutdownTimeout: TimeSpan.FromMilliseconds(250),
        killOwnedProcess: process =>
        {
            if (!allowHungShutdownKill)
            {
                throw new System.ComponentModel.Win32Exception(
                    "simulated process-tree kill failure");
            }
            process.Kill(entireProcessTree: true);
        });
    int hungShutdownPid;
    using (hungShutdownManager)
    {
        ControlRuntimeEvaluation started = await hungShutdownManager.EnsureReadyAsync();
        Require(started.Health == ControlRuntimeHealth.HEALTHY,
            "hung-shutdown fixture did not become a compatible owned Control");
        hungShutdownPid = hungShutdownManager.OwnedProcessId
            ?? throw new InvalidOperationException("hung-shutdown owned PID missing");
        allowHungShutdownKill = false;
        Task stopping = hungShutdownManager.StopOwnedCoreAsync(CancellationToken.None);
        await Task.Delay(50);
        Task concurrentShutdown = hungShutdownManager.ShutdownAsync(CancellationToken.None);
        Require(!concurrentShutdown.IsCompleted,
            "concurrent window shutdown did not join the in-flight owned stop");
        bool firstStopFailed = false;
        try
        {
            await Task.WhenAll(stopping, concurrentShutdown);
        }
        catch (InvalidOperationException error)
            when (error.Message == "CONTROL_OWNED_PROCESS_STOP_FAILED")
        {
            firstStopFailed = true;
        }
        Require(firstStopFailed, "injected live-process kill failure was not surfaced");
        Require(hungShutdownManager.OwnsControl,
            "failed stop discarded the only live owned-process handle");
        Require(hungShutdownManager.OwnedProcessId == hungShutdownPid,
            "failed stop replaced or lost the exact owned PID");

        allowHungShutdownKill = true;
        await hungShutdownManager.ShutdownAsync(CancellationToken.None);
        Require(!hungShutdownManager.OwnsControl,
            "retrying shutdown did not stop the retained owned process");
    }
    Require(hungShutdownManager.LastStopUsedHardKill,
        "owned shutdown timeout did not use the bounded hard-kill fallback");
    try
    {
        using Process hungShutdown = Process.GetProcessById(hungShutdownPid);
        Require(hungShutdown.HasExited,
            "hard-kill fallback left the owned hung Control running");
    }
    catch (ArgumentException)
    {
    }

    int treeShutdownPort = ReservePort();
    Uri treeShutdownOrigin = new($"http://127.0.0.1:{treeShutdownPort}/");
    ControlRuntimeLaunchSpec treeShutdownSpec = LaunchSpec(
        repositoryRoot,
        nodePath,
        treeShutdownPort,
        Path.Combine(temporaryRoot, "tree-shutdown-data"),
        Path.Combine(
            repositoryRoot,
            "apps",
            "lattice-control",
            "test",
            "fixtures",
            "desktop-owned-shutdown-hang.mjs"));
    string childPidPath = $"{treeShutdownSpec.DatabasePath}.child.pid";
    ControlRuntimeManager treeShutdownManager = new(
        treeShutdownOrigin,
        treeShutdownSpec,
        shutdownTimeout: TimeSpan.FromMilliseconds(250));
    using (treeShutdownManager)
    {
        ControlRuntimeEvaluation started = await treeShutdownManager.EnsureReadyAsync();
        Require(started.Health == ControlRuntimeHealth.HEALTHY,
            "process-tree shutdown fixture did not become a compatible owned Control");
        int childPid = await WaitForPidFileAsync(childPidPath);
        using Process childProcess = Process.GetProcessById(childPid);
        await treeShutdownManager.ShutdownAsync(CancellationToken.None);
        Require(treeShutdownManager.LastStopUsedHardKill,
            "default owned shutdown did not use its bounded hard-kill fallback");
        Require(childProcess.WaitForExit(5_000),
            "default entire-process-tree fallback left an owned child running");
    }

    Console.WriteLine("LATTICE_DESKTOP_LIFECYCLE_TEST_PASS");
}
finally
{
    if (Directory.Exists(temporaryRoot)) Directory.Delete(temporaryRoot, recursive: true);
}

internal sealed class ForeignRuntimeServer : IDisposable
{
    private readonly TcpListener listener;
    private readonly bool chunkedOversized;
    private readonly CancellationTokenSource cancellation = new();
    private Task? acceptLoop;

    internal ForeignRuntimeServer(int port, bool chunkedOversized = false)
    {
        listener = new TcpListener(IPAddress.Loopback, port);
        this.chunkedOversized = chunkedOversized;
    }

    internal bool IsRunning => acceptLoop is { IsCompleted: false };

    internal Task StartAsync()
    {
        listener.Start();
        acceptLoop = AcceptAsync(cancellation.Token);
        return Task.CompletedTask;
    }

    private async Task AcceptAsync(CancellationToken token)
    {
        try
        {
            while (!token.IsCancellationRequested)
            {
                TcpClient client = await listener.AcceptTcpClientAsync(token);
                _ = HandleAsync(client, token, chunkedOversized);
            }
        }
        catch (OperationCanceledException)
        {
        }
    }

    private static async Task HandleAsync(
        TcpClient client,
        CancellationToken token,
        bool chunkedOversized)
    {
        using (client)
        using (NetworkStream stream = client.GetStream())
        using (StreamReader reader = new(stream, Encoding.ASCII, false, 1_024, leaveOpen: true))
        {
            while (!string.IsNullOrEmpty(await reader.ReadLineAsync(token)))
            {
            }
            if (chunkedOversized)
            {
                byte[] oversizedPayload = new byte[131_072];
                Array.Fill(oversizedPayload, (byte)'x');
                byte[] chunkedHeaders = Encoding.ASCII.GetBytes(
                    $"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{oversizedPayload.Length:x}\r\n");
                await stream.WriteAsync(chunkedHeaders, token);
                await stream.WriteAsync(oversizedPayload, token);
                await stream.WriteAsync(Encoding.ASCII.GetBytes("\r\n0\r\n\r\n"), token);
                return;
            }
            const string body = "{\"schema_version\":\"lattice.control.runtime-surface.v1\",\"identity\":{\"schema_version\":\"lattice.control.runtime-identity.v1\",\"product\":\"LATTICE_CONTROL\",\"version\":\"0.9.0\"},\"health\":\"HEALTHY\",\"capabilities\":[]}";
            byte[] payload = Encoding.UTF8.GetBytes(body);
            byte[] headers = Encoding.ASCII.GetBytes(
                $"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {payload.Length}\r\nConnection: close\r\n\r\n");
            await stream.WriteAsync(headers, token);
            await stream.WriteAsync(payload, token);
        }
    }

    public void Dispose()
    {
        cancellation.Cancel();
        listener.Stop();
        try { acceptLoop?.Wait(2_000); } catch (AggregateException) { }
        cancellation.Dispose();
    }
}

internal sealed class BlockingProbeHandler : HttpMessageHandler
{
    private readonly TaskCompletionSource started = new(
        TaskCreationOptions.RunContinuationsAsynchronously);
    private readonly TaskCompletionSource released = new(
        TaskCreationOptions.RunContinuationsAsynchronously);

    internal Task Started => started.Task;

    internal void Release() => released.TrySetResult();

    protected override async Task<HttpResponseMessage> SendAsync(
        HttpRequestMessage request,
        CancellationToken cancellationToken)
    {
        started.TrySetResult();
        await released.Task.WaitAsync(cancellationToken);
        throw new HttpRequestException("deterministic no-listener probe");
    }
}
