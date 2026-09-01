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

static ControlRuntimeLaunchSpec LaunchSpec(string repositoryRoot, string nodePath, int port, string localData)
{
    return new(
        nodePath,
        Path.Combine(repositoryRoot, "apps", "lattice-control", "src", "server.mjs"),
        repositoryRoot,
        new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
        {
            ["LATTICE_CONTROL_PORT"] = port.ToString(System.Globalization.CultureInfo.InvariantCulture),
            ["LOCALAPPDATA"] = localData,
        });
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
    using (ControlRuntimeManager manager = new(ownedOrigin, ownedSpec))
    {
        ControlRuntimeEvaluation started = await manager.EnsureReadyAsync();
        Require(started.Health == ControlRuntimeHealth.HEALTHY, "absent Control did not auto-start");
        Require(manager.OwnsControl, "auto-started Control was not owned");
        firstOwnedPid = manager.OwnedProcessId ?? throw new InvalidOperationException("owned PID missing");

        using (Process interrupted = Process.GetProcessById(firstOwnedPid))
        {
            interrupted.Kill(entireProcessTree: true);
            Require(interrupted.WaitForExit(5_000), "owned Control interruption timed out");
        }
        ControlRuntimeEvaluation stopped = await manager.ProbeAsync();
        Require(stopped.Health == ControlRuntimeHealth.STOPPED, "owned Control interruption was not diagnosed");

        ControlRuntimeEvaluation reconnected = await manager.EnsureReadyAsync();
        Require(reconnected.Health == ControlRuntimeHealth.HEALTHY, "owned Control did not reconnect");
        Require(manager.OwnsControl, "replacement Control was not owned");
        replacementOwnedPid = manager.OwnedProcessId ?? throw new InvalidOperationException("replacement PID missing");
        Require(replacementOwnedPid != firstOwnedPid, "owned Control reconnect reused the interrupted PID");
    }

    try
    {
        using Process replacement = Process.GetProcessById(replacementOwnedPid);
        Require(replacement.HasExited, "disposing the manager did not stop its owned Control");
    }
    catch (ArgumentException)
    {
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
