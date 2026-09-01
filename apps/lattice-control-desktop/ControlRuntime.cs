using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Net.Sockets;
using System.Reflection;
using System.Text;
using System.Text.Json;

namespace Lattice.Control.Desktop;

internal enum ControlRuntimeHealth
{
    HEALTHY,
    NOT_IMPLEMENTED,
    STOPPED,
    UNREACHABLE,
    INCOMPATIBLE,
    NO_DATA,
}

internal enum ControlRuntimeAction
{
    Reuse,
    StartOwned,
    FailClosed,
}

internal sealed record ControlRuntimeIdentity(string SchemaVersion, string Product, string Version);

internal sealed record ControlRuntimeEvaluation(
    ControlRuntimeHealth Health,
    ControlRuntimeAction Action,
    string Detail);

internal sealed record ControlRuntimeLaunchSpec(
    string ExecutablePath,
    string ServerPath,
    string WorkingDirectory,
    IReadOnlyDictionary<string, string> Environment);

internal static class ControlRuntimeContract
{
    internal const string SurfaceSchemaVersion = "lattice.control.runtime-surface.v1";
    internal const string IdentitySchemaVersion = "lattice.control.runtime-identity.v1";
    internal const string IdentityResourceName = "Lattice.Control.RuntimeIdentity.json";

    private static readonly string[] ExpectedCapabilityIds =
    [
        "control_sqlite",
        "codex_app_server",
        "work_mcp",
        "decision_mcp",
        "postgresql",
    ];

    internal static ControlRuntimeIdentity ExpectedIdentity { get; } = LoadExpectedIdentity();

    internal static ControlRuntimeEvaluation EvaluateProbe(
        bool tcpReachable,
        int? statusCode,
        string? responseBody)
    {
        if (statusCode == 200 && TryValidateSurface(responseBody))
        {
            return new(ControlRuntimeHealth.HEALTHY, ControlRuntimeAction.Reuse, "CONTROL_COMPATIBLE");
        }

        if (statusCode.HasValue || responseBody is not null || tcpReachable)
        {
            return new(
                ControlRuntimeHealth.INCOMPATIBLE,
                ControlRuntimeAction.FailClosed,
                "CONTROL_LISTENER_INCOMPATIBLE");
        }

        return new(
            ControlRuntimeHealth.UNREACHABLE,
            ControlRuntimeAction.StartOwned,
            "CONTROL_LISTENER_ABSENT");
    }

    private static ControlRuntimeIdentity LoadExpectedIdentity()
    {
        Assembly assembly = typeof(ControlRuntimeContract).Assembly;
        using Stream stream = assembly.GetManifestResourceStream(IdentityResourceName)
            ?? throw new InvalidOperationException("CONTROL_RUNTIME_IDENTITY_RESOURCE_MISSING");
        using JsonDocument document = JsonDocument.Parse(stream, new JsonDocumentOptions
        {
            AllowTrailingCommas = false,
            CommentHandling = JsonCommentHandling.Disallow,
            MaxDepth = 8,
        });
        JsonElement root = document.RootElement;
        if (!HasExactProperties(root, "schema_version", "product", "version"))
        {
            throw new InvalidOperationException("CONTROL_RUNTIME_IDENTITY_INVALID");
        }

        string? schemaVersion = StringProperty(root, "schema_version");
        string? product = StringProperty(root, "product");
        string? version = StringProperty(root, "version");
        if (
            schemaVersion != IdentitySchemaVersion
            || product != "LATTICE_CONTROL"
            || version is null
            || version.Length is < 1 or > 64
        )
        {
            throw new InvalidOperationException("CONTROL_RUNTIME_IDENTITY_INVALID");
        }
        return new(schemaVersion, product, version);
    }

    private static bool TryValidateSurface(string? responseBody)
    {
        if (string.IsNullOrEmpty(responseBody) || responseBody.Length > 65_536)
        {
            return false;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(responseBody, new JsonDocumentOptions
            {
                AllowTrailingCommas = false,
                CommentHandling = JsonCommentHandling.Disallow,
                MaxDepth = 8,
            });
            JsonElement root = document.RootElement;
            if (
                !HasExactProperties(root, "schema_version", "identity", "health", "capabilities")
                || StringProperty(root, "schema_version") != SurfaceSchemaVersion
                || StringProperty(root, "health") != nameof(ControlRuntimeHealth.HEALTHY)
                || !root.TryGetProperty("identity", out JsonElement identity)
                || !HasExactProperties(identity, "schema_version", "product", "version")
                || StringProperty(identity, "schema_version") != ExpectedIdentity.SchemaVersion
                || StringProperty(identity, "product") != ExpectedIdentity.Product
                || StringProperty(identity, "version") != ExpectedIdentity.Version
                || !root.TryGetProperty("capabilities", out JsonElement capabilities)
                || capabilities.ValueKind != JsonValueKind.Array
                || capabilities.GetArrayLength() != ExpectedCapabilityIds.Length
            )
            {
                return false;
            }

            int index = 0;
            foreach (JsonElement capability in capabilities.EnumerateArray())
            {
                if (
                    !HasExactProperties(capability, "id", "label", "status")
                    || StringProperty(capability, "id") != ExpectedCapabilityIds[index]
                    || !IsBoundedDisplayText(StringProperty(capability, "label"))
                    || !ValidCapabilityStatus(ExpectedCapabilityIds[index], StringProperty(capability, "status"))
                )
                {
                    return false;
                }
                index += 1;
            }
            return true;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private static bool ValidCapabilityStatus(string id, string? status)
    {
        return id switch
        {
            "control_sqlite" => status == nameof(ControlRuntimeHealth.HEALTHY),
            "codex_app_server" => status is nameof(ControlRuntimeHealth.HEALTHY)
                or nameof(ControlRuntimeHealth.STOPPED),
            "work_mcp" or "decision_mcp" => status is nameof(ControlRuntimeHealth.HEALTHY)
                or nameof(ControlRuntimeHealth.NO_DATA),
            "postgresql" => status == nameof(ControlRuntimeHealth.NOT_IMPLEMENTED),
            _ => false,
        };
    }

    private static bool IsBoundedDisplayText(string? value)
    {
        return value is { Length: >= 1 and <= 64 }
            && !value.Any(character => char.IsControl(character));
    }

    private static string? StringProperty(JsonElement element, string name)
    {
        return element.TryGetProperty(name, out JsonElement value)
            && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
    }

    private static bool HasExactProperties(JsonElement element, params string[] names)
    {
        if (element.ValueKind != JsonValueKind.Object)
        {
            return false;
        }
        HashSet<string> expected = new(names, StringComparer.Ordinal);
        int count = 0;
        foreach (JsonProperty property in element.EnumerateObject())
        {
            count += 1;
            if (!expected.Remove(property.Name))
            {
                return false;
            }
        }
        return count == names.Length && expected.Count == 0;
    }
}

internal sealed class ControlRuntimeManager : IDisposable
{
    private const int MaximumProbeBytes = 65_536;
    private readonly Uri controlOrigin;
    private readonly Uri runtimeProbeUri;
    private readonly ControlRuntimeLaunchSpec launchSpec;
    private readonly HttpClient httpClient;
    private readonly TimeSpan probeTimeout;
    private readonly TimeSpan startupTimeout;
    private readonly SemaphoreSlim lifecycleGate = new(1, 1);
    private readonly object ownershipSync = new();
    private Process? ownedProcess;
    private volatile bool disposed;

    internal ControlRuntimeManager(
        Uri controlOrigin,
        ControlRuntimeLaunchSpec launchSpec,
        TimeSpan? probeTimeout = null,
        TimeSpan? startupTimeout = null,
        HttpMessageHandler? probeHandler = null)
    {
        if (
            controlOrigin.Scheme != Uri.UriSchemeHttp
            || controlOrigin.Host != "127.0.0.1"
            || controlOrigin.IsDefaultPort
            || !string.IsNullOrEmpty(controlOrigin.UserInfo)
        )
        {
            throw new ArgumentException("Control origin must be an explicit 127.0.0.1 HTTP port", nameof(controlOrigin));
        }
        this.controlOrigin = new Uri(controlOrigin.GetLeftPart(UriPartial.Authority) + "/");
        runtimeProbeUri = new Uri(this.controlOrigin, "api/runtime");
        this.launchSpec = launchSpec;
        this.probeTimeout = probeTimeout ?? TimeSpan.FromSeconds(2);
        this.startupTimeout = startupTimeout ?? TimeSpan.FromSeconds(10);
        HttpMessageHandler handler = probeHandler ?? new SocketsHttpHandler
        {
            AllowAutoRedirect = false,
            ConnectTimeout = this.probeTimeout,
            UseCookies = false,
            UseProxy = false,
        };
        httpClient = new HttpClient(handler)
        {
            Timeout = Timeout.InfiniteTimeSpan,
            MaxResponseContentBufferSize = MaximumProbeBytes,
        };
    }

    internal static ControlRuntimeManager CreatePackaged(Uri controlOrigin)
    {
        string applicationRoot = Path.GetFullPath(AppContext.BaseDirectory);
        string runtimeRoot = Path.Combine(applicationRoot, "control-runtime");
        string nodePath = Path.Combine(runtimeRoot, "node.exe");
        string serverPath = Path.Combine(
            runtimeRoot,
            "apps",
            "lattice-control",
            "src",
            "server.mjs");
        return new ControlRuntimeManager(
            controlOrigin,
            new ControlRuntimeLaunchSpec(
                nodePath,
                serverPath,
                runtimeRoot,
                new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
                {
                    ["LATTICE_CONTROL_PORT"] = controlOrigin.Port.ToString(CultureInfo.InvariantCulture),
                }));
    }

    internal bool OwnsControl
    {
        get
        {
            lock (ownershipSync)
            {
                Process? process = ownedProcess;
                return process is not null && !HasExited(process);
            }
        }
    }

    internal int? OwnedProcessId
    {
        get
        {
            lock (ownershipSync)
            {
                Process? process = ownedProcess;
                return process is not null && !HasExited(process) ? process.Id : null;
            }
        }
    }

    internal async Task<ControlRuntimeEvaluation> ProbeAsync(CancellationToken cancellationToken = default)
    {
        ThrowIfDisposed();
        await lifecycleGate.WaitAsync(cancellationToken);
        try
        {
            ThrowIfDisposed();
            return await ProbeCoreAsync(cancellationToken);
        }
        finally
        {
            lifecycleGate.Release();
        }
    }

    internal async Task<ControlRuntimeEvaluation> EnsureReadyAsync(
        CancellationToken cancellationToken = default)
    {
        ThrowIfDisposed();
        await lifecycleGate.WaitAsync(cancellationToken);
        try
        {
            ThrowIfDisposed();
            ClearExitedOwnedProcess();

            ControlRuntimeEvaluation initial = await ProbeCoreAsync(cancellationToken);
            cancellationToken.ThrowIfCancellationRequested();
            ThrowIfDisposed();
            if (initial.Health == ControlRuntimeHealth.HEALTHY
                || initial.Action != ControlRuntimeAction.StartOwned)
            {
                return initial;
            }
            if (HasOwnedProcess())
            {
                return new(
                    ControlRuntimeHealth.UNREACHABLE,
                    ControlRuntimeAction.FailClosed,
                    "CONTROL_OWNED_PROCESS_UNREACHABLE");
            }

            if (!ValidLaunchSpec())
            {
                return new(
                    ControlRuntimeHealth.STOPPED,
                    ControlRuntimeAction.FailClosed,
                    "CONTROL_RUNTIME_FILES_MISSING");
            }

            try
            {
                StartOwnedProcess(cancellationToken);
            }
            catch (Exception error) when (error is InvalidOperationException or System.ComponentModel.Win32Exception)
            {
                return new(
                    ControlRuntimeHealth.STOPPED,
                    ControlRuntimeAction.FailClosed,
                    "CONTROL_PROCESS_START_FAILED");
            }

            DateTimeOffset deadline = DateTimeOffset.UtcNow.Add(startupTimeout);
            while (DateTimeOffset.UtcNow < deadline)
            {
                cancellationToken.ThrowIfCancellationRequested();
                ThrowIfDisposed();
                Process process = OwnedProcessOrThrow();
                if (HasExited(process))
                {
                    ClearOwnedProcess();
                    ControlRuntimeEvaluation raced = await ProbeNetworkAsync(cancellationToken);
                    if (raced.Health == ControlRuntimeHealth.HEALTHY)
                    {
                        return raced;
                    }
                    return raced.Health == ControlRuntimeHealth.INCOMPATIBLE
                        ? raced
                        : new(
                            ControlRuntimeHealth.STOPPED,
                            ControlRuntimeAction.FailClosed,
                            "CONTROL_PROCESS_EXITED");
                }

                ControlRuntimeEvaluation probe = await ProbeNetworkAsync(cancellationToken);
                cancellationToken.ThrowIfCancellationRequested();
                ThrowIfDisposed();
                if (probe.Health == ControlRuntimeHealth.HEALTHY)
                {
                    return new(
                        ControlRuntimeHealth.HEALTHY,
                        ControlRuntimeAction.StartOwned,
                        "CONTROL_OWNED_READY");
                }
                if (probe.Health == ControlRuntimeHealth.INCOMPATIBLE)
                {
                    StopOwnedCore();
                    return probe;
                }
                await Task.Delay(TimeSpan.FromMilliseconds(100), cancellationToken);
            }

            StopOwnedCore();
            return new(
                ControlRuntimeHealth.UNREACHABLE,
                ControlRuntimeAction.FailClosed,
                "CONTROL_STARTUP_TIMEOUT");
        }
        catch
        {
            StopOwnedCore();
            throw;
        }
        finally
        {
            lifecycleGate.Release();
        }
    }

    private async Task<ControlRuntimeEvaluation> ProbeCoreAsync(CancellationToken cancellationToken)
    {
        Process? process = OwnedProcessSnapshot();
        if (process is not null && HasExited(process))
        {
            return new(
                ControlRuntimeHealth.STOPPED,
                ControlRuntimeAction.FailClosed,
                "CONTROL_OWNED_PROCESS_EXITED");
        }
        return await ProbeNetworkAsync(cancellationToken);
    }

    private async Task<ControlRuntimeEvaluation> ProbeNetworkAsync(CancellationToken cancellationToken)
    {
        (int? statusCode, string? body) = await ProbeHttpAsync(cancellationToken);
        if (statusCode.HasValue || body is not null)
        {
            return ControlRuntimeContract.EvaluateProbe(true, statusCode, body);
        }
        bool tcpReachable = await ProbeTcpAsync(cancellationToken);
        return ControlRuntimeContract.EvaluateProbe(tcpReachable, null, null);
    }

    private async Task<(int? StatusCode, string? Body)> ProbeHttpAsync(
        CancellationToken cancellationToken)
    {
        using CancellationTokenSource timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(probeTimeout);
        try
        {
            using HttpRequestMessage request = new(HttpMethod.Get, runtimeProbeUri);
            using HttpResponseMessage response = await httpClient.SendAsync(
                request,
                HttpCompletionOption.ResponseHeadersRead,
                timeout.Token);
            if (response.Content.Headers.ContentLength is > MaximumProbeBytes)
            {
                return ((int)response.StatusCode, "CONTROL_RUNTIME_RESPONSE_TOO_LARGE");
            }
            await using Stream stream = await response.Content.ReadAsStreamAsync(timeout.Token);
            (bool tooLarge, string body) = await ReadProbeBodyAsync(stream, timeout.Token);
            if (tooLarge)
            {
                return ((int)response.StatusCode, "CONTROL_RUNTIME_RESPONSE_TOO_LARGE");
            }
            return ((int)response.StatusCode, body);
        }
        catch (Exception error) when (
            error is HttpRequestException
                or IOException
                or OperationCanceledException
                or ObjectDisposedException
                or DecoderFallbackException)
        {
            cancellationToken.ThrowIfCancellationRequested();
            return (null, null);
        }
    }

    internal static async Task<(bool TooLarge, string Body)> ReadProbeBodyAsync(
        Stream stream,
        CancellationToken cancellationToken)
    {
        byte[] buffer = new byte[8_192];
        using MemoryStream body = new(MaximumProbeBytes);
        while (true)
        {
            int remainingWithSentinel = MaximumProbeBytes - checked((int)body.Length) + 1;
            int read = await stream.ReadAsync(
                buffer.AsMemory(0, Math.Min(buffer.Length, remainingWithSentinel)),
                cancellationToken);
            if (read == 0) break;
            if (body.Length + read > MaximumProbeBytes)
            {
                return (true, string.Empty);
            }
            body.Write(buffer, 0, read);
        }
        return (
            false,
            new UTF8Encoding(false, true).GetString(body.GetBuffer(), 0, checked((int)body.Length)));
    }

    private async Task<bool> ProbeTcpAsync(CancellationToken cancellationToken)
    {
        using CancellationTokenSource timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(probeTimeout);
        using TcpClient client = new(AddressFamily.InterNetwork);
        try
        {
            await client.ConnectAsync(IPAddress.Loopback, controlOrigin.Port, timeout.Token);
            return true;
        }
        catch (Exception error) when (error is SocketException or OperationCanceledException)
        {
            cancellationToken.ThrowIfCancellationRequested();
            return false;
        }
    }

    private bool ValidLaunchSpec()
    {
        return Path.IsPathFullyQualified(launchSpec.ExecutablePath)
            && Path.IsPathFullyQualified(launchSpec.ServerPath)
            && Path.IsPathFullyQualified(launchSpec.WorkingDirectory)
            && File.Exists(launchSpec.ExecutablePath)
            && File.Exists(launchSpec.ServerPath)
            && Directory.Exists(launchSpec.WorkingDirectory)
            && launchSpec.Environment.All(pair =>
                pair.Key.Length is >= 1 and <= 128
                && pair.Value.Length <= 4_096
                && !pair.Key.Any(character => char.IsControl(character) || character == '='));
    }

    private void StartOwnedProcess(CancellationToken cancellationToken)
    {
        lock (ownershipSync)
        {
            ThrowIfDisposed();
            cancellationToken.ThrowIfCancellationRequested();
            if (ownedProcess is not null)
            {
                throw new InvalidOperationException("CONTROL_PROCESS_ALREADY_OWNED");
            }
            ProcessStartInfo start = new()
            {
                FileName = launchSpec.ExecutablePath,
                WorkingDirectory = launchSpec.WorkingDirectory,
                UseShellExecute = false,
                CreateNoWindow = true,
            };
            start.ArgumentList.Add(launchSpec.ServerPath);
            foreach ((string key, string value) in launchSpec.Environment)
            {
                start.Environment[key] = value;
            }
            ownedProcess = Process.Start(start)
                ?? throw new InvalidOperationException("CONTROL_PROCESS_START_FAILED");
        }
    }

    private static bool HasExited(Process process)
    {
        try
        {
            return process.HasExited;
        }
        catch (Exception error) when (error is InvalidOperationException or ObjectDisposedException)
        {
            return true;
        }
    }

    private void ClearOwnedProcess()
    {
        lock (ownershipSync)
        {
            Process? process = ownedProcess;
            ownedProcess = null;
            process?.Dispose();
        }
    }

    private void StopOwnedCore()
    {
        lock (ownershipSync)
        {
            StopOwnedUnderLock();
        }
    }

    private void StopOwnedUnderLock()
    {
        Process? process = ownedProcess;
        if (process is null) return;
        StopProcessAndVerify(process);
        ownedProcess = null;
        process.Dispose();
    }

    private Process OwnedProcessOrThrow()
    {
        lock (ownershipSync)
        {
            return ownedProcess ?? throw new InvalidOperationException("CONTROL_OWNED_PROCESS_MISSING");
        }
    }

    private Process? OwnedProcessSnapshot()
    {
        lock (ownershipSync)
        {
            return ownedProcess;
        }
    }

    private bool HasOwnedProcess()
    {
        lock (ownershipSync)
        {
            return ownedProcess is not null;
        }
    }

    private void ClearExitedOwnedProcess()
    {
        lock (ownershipSync)
        {
            if (ownedProcess is null || !HasExited(ownedProcess)) return;
            Process process = ownedProcess;
            ownedProcess = null;
            process.Dispose();
        }
    }

    private static void StopProcessAndVerify(Process process)
    {
        try
        {
            if (!HasExited(process))
            {
                process.Kill(entireProcessTree: true);
                if (!process.WaitForExit(10_000))
                {
                    throw new InvalidOperationException("CONTROL_OWNED_PROCESS_STOP_TIMEOUT");
                }
            }
        }
        catch (Exception error) when (error is InvalidOperationException or System.ComponentModel.Win32Exception)
        {
            if (!HasExited(process))
            {
                throw new InvalidOperationException("CONTROL_OWNED_PROCESS_STOP_FAILED", error);
            }
        }
    }

    private void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(disposed, this);
    }

    public void Dispose()
    {
        try
        {
            lock (ownershipSync)
            {
                if (disposed && ownedProcess is null) return;
                disposed = true;
                StopOwnedUnderLock();
            }
        }
        finally
        {
            httpClient.Dispose();
        }
        // A cancelled probe can still be unwinding through the finally block and
        // must be allowed to release this gate during desktop shutdown.
    }
}
