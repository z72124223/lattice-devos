using System.Buffers.Binary;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Net.Sockets;
using System.Reflection;
using System.Security.Cryptography;
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

internal sealed record ControlRuntimeDataScope(
    string SchemaVersion,
    string Store,
    int StoreSchemaVersion,
    string AuthorityClass,
    string RegistryAuthority,
    string Digest);

internal sealed record ControlDataScopeContract(
    string SchemaVersion,
    string Store,
    int StoreSchemaVersion,
    string AuthorityClass,
    string RegistryAuthority);

internal sealed record ControlRuntimeEvaluation(
    ControlRuntimeHealth Health,
    ControlRuntimeAction Action,
    string Detail);

internal sealed record ControlRuntimeLaunchSpec(
    string ExecutablePath,
    string ServerPath,
    string WorkingDirectory,
    IReadOnlyDictionary<string, string> Environment,
    string DatabasePath);

internal static class ControlRuntimeContract
{
    internal const string SurfaceSchemaVersion = "lattice.control.runtime-surface.v2";
    internal const string IdentitySchemaVersion = "lattice.control.runtime-identity.v1";
    internal const string IdentityResourceName = "Lattice.Control.RuntimeIdentity.json";
    internal const string DataScopeContractResourceName = "Lattice.Control.DataScopeContract.json";
    private static readonly ControlDataScopeContract DataScopeContract = LoadDataScopeContract();
    internal static readonly string DataScopeSchemaVersion = DataScopeContract.SchemaVersion;
    internal static readonly string DataScopeStore = DataScopeContract.Store;
    internal static readonly int DataScopeStoreSchemaVersion = DataScopeContract.StoreSchemaVersion;
    internal static readonly string DataScopeAuthorityClass = DataScopeContract.AuthorityClass;
    internal static readonly string DataScopeRegistryAuthority = DataScopeContract.RegistryAuthority;

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
        string? responseBody,
        ControlRuntimeDataScope expectedScope)
    {
        string detail = "CONTROL_LISTENER_INCOMPATIBLE";
        if (statusCode == 200 && TryValidateSurface(responseBody, expectedScope, out detail))
        {
            return new(ControlRuntimeHealth.HEALTHY, ControlRuntimeAction.Reuse, detail);
        }

        if (statusCode.HasValue || responseBody is not null || tcpReachable)
        {
            return new(
                ControlRuntimeHealth.INCOMPATIBLE,
                ControlRuntimeAction.FailClosed,
                statusCode == 200 ? detail : "CONTROL_LISTENER_INCOMPATIBLE");
        }

        return new(
            ControlRuntimeHealth.UNREACHABLE,
            ControlRuntimeAction.StartOwned,
            "CONTROL_LISTENER_ABSENT");
    }

    internal static ControlRuntimeDataScope DataScopeForDatabasePath(string databasePath)
    {
        if (string.IsNullOrEmpty(databasePath) || databasePath.IndexOf('\0') >= 0)
        {
            throw new ArgumentException("CONTROL_DATABASE_PATH_INVALID", nameof(databasePath));
        }
        string normalized = Path.GetFullPath(databasePath).Replace('\\', '/');
        if (OperatingSystem.IsWindows())
        {
            char[] characters = normalized.ToCharArray();
            for (int index = 0; index < characters.Length; index += 1)
            {
                if (characters[index] is >= 'A' and <= 'Z')
                {
                    characters[index] = (char)(characters[index] + ('a' - 'A'));
                }
            }
            normalized = new string(characters);
        }
        string[] identity =
        [
            DataScopeSchemaVersion,
            DataScopeStore,
            DataScopeStoreSchemaVersion.ToString(CultureInfo.InvariantCulture),
            DataScopeAuthorityClass,
            DataScopeRegistryAuthority,
            normalized,
        ];
        using MemoryStream preimage = new();
        byte[] encodedLength = new byte[4];
        foreach (string value in identity)
        {
            byte[] encoded = Encoding.UTF8.GetBytes(value);
            BinaryPrimitives.WriteUInt32BigEndian(encodedLength, checked((uint)encoded.Length));
            preimage.Write(encodedLength);
            preimage.Write(encoded);
        }
        byte[] digest = SHA256.HashData(preimage.ToArray());
        return new(
            DataScopeSchemaVersion,
            DataScopeStore,
            DataScopeStoreSchemaVersion,
            DataScopeAuthorityClass,
            DataScopeRegistryAuthority,
            Convert.ToHexString(digest).ToLowerInvariant());
    }

    private static ControlDataScopeContract LoadDataScopeContract()
    {
        Assembly assembly = typeof(ControlRuntimeContract).Assembly;
        using Stream stream = assembly.GetManifestResourceStream(DataScopeContractResourceName)
            ?? throw new InvalidOperationException("CONTROL_DATA_SCOPE_CONTRACT_MISSING");
        using JsonDocument document = JsonDocument.Parse(stream, new JsonDocumentOptions
        {
            AllowTrailingCommas = false,
            CommentHandling = JsonCommentHandling.Disallow,
            MaxDepth = 8,
        });
        JsonElement root = document.RootElement;
        if (!HasExactProperties(
            root,
            "schema_version",
            "store",
            "store_schema_version",
            "authority_class",
            "registry_authority"))
        {
            throw new InvalidOperationException("CONTROL_DATA_SCOPE_CONTRACT_INVALID");
        }
        string? schemaVersion = StringProperty(root, "schema_version");
        string? store = StringProperty(root, "store");
        string? authorityClass = StringProperty(root, "authority_class");
        string? registryAuthority = StringProperty(root, "registry_authority");
        if (
            schemaVersion != "lattice.control.data-scope.v1"
            || store != "CONTROL_SQLITE"
            || !root.TryGetProperty("store_schema_version", out JsonElement storeSchemaVersion)
            || storeSchemaVersion.ValueKind != JsonValueKind.Number
            || !storeSchemaVersion.TryGetInt32(out int parsedStoreSchemaVersion)
            || parsedStoreSchemaVersion < 1
            || authorityClass != "CONTROL_LOCAL_PRODUCT_STATE"
            || registryAuthority != "NONE"
        )
        {
            throw new InvalidOperationException("CONTROL_DATA_SCOPE_CONTRACT_INVALID");
        }
        return new(
            schemaVersion,
            store,
            parsedStoreSchemaVersion,
            authorityClass,
            registryAuthority);
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

    private static bool TryValidateSurface(
        string? responseBody,
        ControlRuntimeDataScope expectedScope,
        out string detail)
    {
        detail = "CONTROL_LISTENER_INCOMPATIBLE";
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
                !HasExactProperties(
                    root,
                    "schema_version",
                    "identity",
                    "data_scope",
                    "reconciliation_required",
                    "health",
                    "capabilities")
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

            if (
                !root.TryGetProperty("data_scope", out JsonElement dataScope)
                || !HasExactProperties(
                    dataScope,
                    "schema_version",
                    "store",
                    "store_schema_version",
                    "authority_class",
                    "registry_authority",
                    "digest")
                || StringProperty(dataScope, "schema_version") != expectedScope.SchemaVersion
                || StringProperty(dataScope, "store") != expectedScope.Store
                || !dataScope.TryGetProperty("store_schema_version", out JsonElement storeSchemaVersion)
                || storeSchemaVersion.ValueKind != JsonValueKind.Number
                || !storeSchemaVersion.TryGetInt32(out int observedStoreSchemaVersion)
                || observedStoreSchemaVersion != expectedScope.StoreSchemaVersion
                || StringProperty(dataScope, "authority_class") != expectedScope.AuthorityClass
                || StringProperty(dataScope, "registry_authority") != expectedScope.RegistryAuthority
                || StringProperty(dataScope, "digest") != expectedScope.Digest
            )
            {
                detail = "CONTROL_DATA_SCOPE_INCOMPATIBLE";
                return false;
            }
            if (
                !root.TryGetProperty("reconciliation_required", out JsonElement reconciliation)
                || reconciliation.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
            )
            {
                return false;
            }
            bool reconciliationRequired = reconciliation.GetBoolean();

            int index = 0;
            foreach (JsonElement capability in capabilities.EnumerateArray())
            {
                if (
                    !HasExactProperties(capability, "id", "label", "status", "has_data")
                    || StringProperty(capability, "id") != ExpectedCapabilityIds[index]
                    || !IsBoundedDisplayText(StringProperty(capability, "label"))
                    || !ValidCapabilityStatus(ExpectedCapabilityIds[index], StringProperty(capability, "status"))
                    || !ValidCapabilityData(ExpectedCapabilityIds[index], capability)
                )
                {
                    return false;
                }
                index += 1;
            }
            detail = reconciliationRequired
                ? "CONTROL_RECONCILIATION_REQUIRED"
                : "CONTROL_COMPATIBLE";
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
                or nameof(ControlRuntimeHealth.UNREACHABLE)
                or nameof(ControlRuntimeHealth.INCOMPATIBLE),
            "postgresql" => status == nameof(ControlRuntimeHealth.NOT_IMPLEMENTED),
            _ => false,
        };
    }

    private static bool ValidCapabilityData(string id, JsonElement capability)
    {
        if (!capability.TryGetProperty("has_data", out JsonElement hasData)) return false;
        return id is "work_mcp" or "decision_mcp"
            ? hasData.ValueKind is JsonValueKind.True or JsonValueKind.False
            : hasData.ValueKind == JsonValueKind.Null;
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
    private readonly TimeSpan shutdownTimeout;
    private readonly Action<Process> killOwnedProcess;
    private readonly ControlRuntimeDataScope expectedDataScope;
    private readonly SemaphoreSlim lifecycleGate = new(1, 1);
    private readonly object ownershipSync = new();
    private Process? ownedProcess;
    private Task? ownedStopTask;
    private Task? shutdownTask;
    private volatile bool disposed;

    internal ControlRuntimeManager(
        Uri controlOrigin,
        ControlRuntimeLaunchSpec launchSpec,
        TimeSpan? probeTimeout = null,
        TimeSpan? startupTimeout = null,
        TimeSpan? shutdownTimeout = null,
        HttpMessageHandler? probeHandler = null,
        Action<Process>? killOwnedProcess = null)
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
        expectedDataScope = ControlRuntimeContract.DataScopeForDatabasePath(launchSpec.DatabasePath);
        this.probeTimeout = probeTimeout ?? TimeSpan.FromSeconds(2);
        this.startupTimeout = startupTimeout ?? TimeSpan.FromSeconds(10);
        this.shutdownTimeout = shutdownTimeout ?? TimeSpan.FromSeconds(8);
        this.killOwnedProcess = killOwnedProcess
            ?? (process => process.Kill(entireProcessTree: true));
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
        string localApplicationData = Environment.GetEnvironmentVariable("LOCALAPPDATA")
            ?? Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        string databasePath = Path.GetFullPath(Path.Combine(
            localApplicationData,
            "LATTICE",
            "control",
            "lattice-control.db"));
        return new ControlRuntimeManager(
            controlOrigin,
            new ControlRuntimeLaunchSpec(
                nodePath,
                serverPath,
                runtimeRoot,
                new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
                {
                    ["LATTICE_CONTROL_PORT"] = controlOrigin.Port.ToString(CultureInfo.InvariantCulture),
                    ["LATTICE_CONTROL_DATABASE_PATH"] = databasePath,
                    ["LATTICE_CONTROL_DESKTOP_OWNED"] = "1",
                },
                databasePath));
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

    internal bool LastStopUsedHardKill { get; private set; }

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
                    await StopOwnedCoreAsync(CancellationToken.None);
                    return probe;
                }
                await Task.Delay(TimeSpan.FromMilliseconds(100), cancellationToken);
            }

            await StopOwnedCoreAsync(CancellationToken.None);
            return new(
                ControlRuntimeHealth.UNREACHABLE,
                ControlRuntimeAction.FailClosed,
                "CONTROL_STARTUP_TIMEOUT");
        }
        catch
        {
            await StopOwnedCoreAsync(CancellationToken.None);
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
            return ControlRuntimeContract.EvaluateProbe(true, statusCode, body, expectedDataScope);
        }
        bool tcpReachable = await ProbeTcpAsync(cancellationToken);
        return ControlRuntimeContract.EvaluateProbe(tcpReachable, null, null, expectedDataScope);
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
            && Path.IsPathFullyQualified(launchSpec.DatabasePath)
            && File.Exists(launchSpec.ExecutablePath)
            && File.Exists(launchSpec.ServerPath)
            && Directory.Exists(launchSpec.WorkingDirectory)
            && launchSpec.Environment.TryGetValue(
                "LATTICE_CONTROL_DATABASE_PATH",
                out string? configuredDatabasePath)
            && Path.GetFullPath(configuredDatabasePath)
                .Equals(Path.GetFullPath(launchSpec.DatabasePath), StringComparison.OrdinalIgnoreCase)
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
                RedirectStandardInput = true,
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

    internal Task StopOwnedCoreAsync(CancellationToken cancellationToken)
    {
        Task stopTask;
        lock (ownershipSync)
        {
            stopTask = GetOrStartOwnedStopLocked();
        }
        return cancellationToken.CanBeCanceled
            ? stopTask.WaitAsync(cancellationToken)
            : stopTask;
    }

    private Task GetOrStartOwnedStopLocked()
    {
        if (ownedStopTask is not null) return ownedStopTask;
        Process? process = ownedProcess;
        if (process is null) return Task.CompletedTask;
        TaskCompletionSource completion = new(TaskCreationOptions.RunContinuationsAsynchronously);
        Task stopTask = completion.Task;
        ownedStopTask = stopTask;
        _ = StopOwnedAndReleaseAsync(process, completion);
        return stopTask;
    }

    private async Task StopOwnedAndReleaseAsync(
        Process process,
        TaskCompletionSource completion)
    {
        Exception? failure = null;
        try
        {
            await StopProcessAndVerifyAsync(process, CancellationToken.None);
        }
        catch (Exception error)
        {
            failure = error;
        }
        finally
        {
            bool exited = HasExited(process);
            lock (ownershipSync)
            {
                if (exited && ReferenceEquals(ownedProcess, process)) ownedProcess = null;
                if (ReferenceEquals(ownedStopTask, completion.Task)) ownedStopTask = null;
            }
            if (exited) process.Dispose();
        }
        if (failure is null) completion.TrySetResult();
        else completion.TrySetException(failure);
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

    private async Task StopProcessAndVerifyAsync(
        Process process,
        CancellationToken cancellationToken)
    {
        LastStopUsedHardKill = false;
        bool shutdownAttempted = false;
        try
        {
            if (!HasExited(process))
            {
                using CancellationTokenSource timeout = CancellationTokenSource.CreateLinkedTokenSource(
                    cancellationToken);
                timeout.CancelAfter(shutdownTimeout);
                try
                {
                    string frame = JsonSerializer.Serialize(new
                    {
                        schema_version = "lattice.control.desktop-shutdown.v1",
                        operation = "shutdown",
                        data_scope_digest = expectedDataScope.Digest,
                    });
                    shutdownAttempted = true;
                    await process.StandardInput.WriteLineAsync(frame.AsMemory(), timeout.Token);
                    await process.StandardInput.FlushAsync(timeout.Token);
                    await process.WaitForExitAsync(timeout.Token);
                }
                catch (Exception error) when (
                    error is OperationCanceledException
                        or InvalidOperationException
                        or IOException
                        or ObjectDisposedException)
                {
                    if (!HasExited(process))
                    {
                        LastStopUsedHardKill = true;
                        killOwnedProcess(process);
                    }
                }
                if (!HasExited(process) && !process.WaitForExit(10_000))
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
        if (
            shutdownAttempted
            && !LastStopUsedHardKill
            && HasExited(process)
            && process.ExitCode != 0)
        {
            throw new InvalidOperationException("CONTROL_OWNED_PROCESS_SHUTDOWN_REJECTED");
        }
    }

    private void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(disposed, this);
    }

    public void Dispose()
    {
        ShutdownAsync(CancellationToken.None).GetAwaiter().GetResult();
    }

    internal Task ShutdownAsync(CancellationToken cancellationToken = default)
    {
        lock (ownershipSync)
        {
            if (shutdownTask is not null) return shutdownTask;
            disposed = true;
            Task stopTask = GetOrStartOwnedStopLocked();
            shutdownTask = ShutdownAfterOwnedStopAsync(stopTask);
            return shutdownTask;
        }
    }

    private async Task ShutdownAfterOwnedStopAsync(Task stopTask)
    {
        try
        {
            await stopTask;
        }
        catch
        {
            lock (ownershipSync)
            {
                if (ownedProcess is not null && !HasExited(ownedProcess))
                {
                    shutdownTask = null;
                }
            }
            throw;
        }
        finally
        {
            httpClient.Dispose();
        }
    }
}
