[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$launcher = Join-Path $PSScriptRoot 'start-chatgpt-mcp-tunnel.ps1'
$task038Verifier = Join-Path $PSScriptRoot 'run-task038-task-submit.ps1'
if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
    throw 'TASK038_TUNNEL_LAUNCHER_MISSING'
}
if (-not (Test-Path -LiteralPath $task038Verifier -PathType Leaf)) {
    throw 'TASK038_LOCAL_VERIFIER_MISSING'
}

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )
    if ($Actual -ne $Expected) {
        throw $FailureCode
    }
}

function Assert-FailsWith {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )
    try {
        & $Action
    }
    catch {
        if ([string]$_.Exception.Message -eq $FailureCode) {
            return
        }
        throw
    }
    throw ('EXPECTED_FAILURE_MISSING_' + $FailureCode)
}

function Get-StringSha256 {
    param([Parameter(Mandatory = $true)][string]$Value)

    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function New-Task038TunnelProfileText {
    param(
        [Parameter(Mandatory = $true)][string]$TunnelId,
        [Parameter(Mandatory = $true)][string]$YamlLatticed
    )

    return ((@(
        'config_version: 1',
        'control_plane:',
        '  base_url: "https://api.openai.com"',
        '',
        ('  tunnel_id: "' + $TunnelId + '"'),
        '  api_key: "env:CONTROL_PLANE_API_KEY"',
        'health:',
        '  # Keep a fixed port when you want a stable local admin URL.',
        '  # For concurrent or clean-room runs, switch listen_addr to "127.0.0.1:0" and',
        '  # set url_file so another process can discover the resolved /healthz, /readyz,',
        '  # /metrics, and /ui base URL.',
        '  listen_addr: "127.0.0.1:8080"',
        '  # url_file: "/tmp/tunnel-client-health.url"',
        'admin_ui:',
        '  open_browser: false',
        'log:',
        '  level: info',
        '  format: json',
        'mcp:',
        '  commands:',
        '    - channel: main',
        ('      command: "''' + $YamlLatticed + '''"')
    ) -join "`n") + "`n")
}

$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testRoot = Join-Path $tempBase ('lattice-task038-tunnel-test-' + [Guid]::NewGuid().ToString('N'))
$testRoot = [IO.Path]::GetFullPath($testRoot)
if (-not $testRoot.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'TASK038_TEST_ROOT_REJECTED'
}
[IO.Directory]::CreateDirectory($testRoot) | Out-Null

$capturePath = Join-Path $testRoot 'capture.txt'
$ingressKindPath = Join-Path $testRoot 'ingress-kind.txt'
$ingressProfilePath = Join-Path $testRoot 'ingress-profile.txt'
$descendantPidPath = Join-Path $testRoot 'descendant-pid.txt'
$fakeClient = Join-Path $testRoot 'tunnel-client.exe'
$fakeRunClient = $fakeClient
$failingClient = Join-Path $testRoot 'failing-tunnel-client.exe'
$latticed = Join-Path $testRoot 'latticed.exe'
$deliveryLauncher = Join-Path $testRoot 'delivery-launcher.exe'
$gitExecutable = Join-Path $testRoot 'git.exe'
$unsafeLatticed = Join-Path $testRoot "bad'latticed.exe"
$profileDirectory = Join-Path $testRoot 'profiles'
$codexHome = Join-Path $testRoot 'codex-home'
$tunnelId = 'tunnel_0123456789abcdef0123456789abcdef'
$hostileVariables = @(
    'ALLOW_REMOTE_UI',
    'CA_BUNDLE',
    'CLOUDFLARED_MANAGED',
    'CLOUDFLARED_PATH',
    'CLOUDFLARED_TUNNEL_TOKEN',
    'CONTROL_PLANE_BASE_URL',
    'CONTROL_PLANE_CLIENT_CERT',
    'CONTROL_PLANE_CLIENT_KEY',
    'CONTROL_PLANE_EXTRA_HEADERS',
    'CONTROL_PLANE_HTTP_PROXY',
    'CONTROL_PLANE_MAX_INFLIGHT_REQUESTS',
    'CONTROL_PLANE_POLL_CHANNELS',
    'CONTROL_PLANE_POLL_DEADLINE_GUARDRAIL',
    'CONTROL_PLANE_POLL_TIMEOUT',
    'CONTROL_PLANE_TUNNEL_ID',
    'CONTROL_PLANE_URL_PATH',
    'GODEBUG',
    'HARPOON_ADDITIONAL_TRANSPORTS',
    'HARPOON_ALLOW_PLAINTEXT_HTTP',
    'HARPOON_CAPTURE_PAYLOADS',
    'HARPOON_HOSTS_INCLUDE_LOOPBACK',
    'HARPOON_HOSTS_INCLUDE_PRIVATE',
    'HARPOON_HOSTS_INCLUDE_REGEX',
    'HARPOON_HOSTS_INCLUDE_SUFFIX',
    'HARPOON_HTTP_PROXY',
    'HARPOON_MAX_REDIRECTS',
    'HARPOON_MAX_RESPONSE_BYTES',
    'HARPOON_TARGETS',
    'HEALTH_LISTEN_ADDR',
    'HEALTH_URL_FILE',
    'HTTP_PROXY',
    'HTTPS_PROXY',
    'LOG_FILE',
    'LOG_FORMAT',
    'LOG_HTTP_RAW_UNSAFE',
    'LOG_LEVEL',
    'MCP_CLIENT_CERT',
    'MCP_CLIENT_KEY',
    'MCP_COMMAND',
    'MCP_CONNECTION_MAX_TTL',
    'MCP_DISCOVERY_EXTRA_HEADERS',
    'MCP_EXTRA_HEADERS',
    'MCP_HTTP_PROXY',
    'MCP_MAX_CONCURRENT_REQUESTS',
    'MCP_SERVER_URL',
    'NO_PROXY',
    'OPENAI_ADMIN_KEY',
    'OPENAI_API_KEY',
    'OPEN_WEB_UI',
    'PID_FILE',
    'PROXY_CHECK_INTERVAL',
    'SSL_CERT_DIR',
    'SSL_CERT_FILE',
    'TUNNEL_CLIENT_CONFIG',
    'TUNNEL_CLIENT_HTTP_PROXY',
    'TUNNEL_CLIENT_PROFILE',
    'TUNNEL_CLIENT_PROFILE_DIR',
    'TUNNEL_CLIENT_PROFILE_FILE',
    'TUNNEL_CLIENT_LIFECYCLE_EVENT_PATH',
    'TUNNEL_CLIENT_LIFECYCLE_SESSION_ID',
    'TUNNEL_CLIENT_LIFECYCLE_CONFIG_GENERATION',
    'TUNNEL_CLIENT_LIFECYCLE_SAFE_CONFIG_SHA256'
)
$originalProcessEnvironment = [Environment]::GetEnvironmentVariables('Process')

function Set-Task038ValidRuntimeEnvironment {
    param([int]$Port = 56981)

    $digestA = ('a' * 64) -join ''
    $digestB = ('b' * 64) -join ''
    $values = [ordered]@{
        CONTROL_PLANE_API_KEY = 'test-runtime-key-not-a-secret'
        LATTICE_FULL_CHAIN_RUN_MODE = 'FRESH'
        LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
        LATTICE_DELIVERY_TIMEOUT_SECONDS = '300'
        LATTICE_TASK019_HOST = '127.0.0.1'
        LATTICE_TASK019_PORT = [string]$Port
        LATTICE_TASK019_RUN_ID = '0123456789abcdef0123456789abcdef'
        LATTICE_TASK019_PASSWORD = 'fixture-password-private'
        LATTICE_STORE_DAEMON_INSTANCE_ID = 'task038-tunnel-0123456789abcdef0123456789abcdef'
        LATTICE_STORE_DAEMON_EPOCH = '1'
        LATTICE_STORE_AUTHORITY_REVISION = '1'
        LATTICE_STORE_OBSERVATION_DIGEST = $digestA
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = $digestB
        LATTICE_DELIVERY_LAUNCHER = $deliveryLauncher
        LATTICE_DELIVERY_LAUNCHER_VERSION = 'fixture-1'
        LATTICE_DELIVERY_LAUNCHER_SHA256 = (Get-FileHash -LiteralPath $deliveryLauncher -Algorithm SHA256).Hash.ToLowerInvariant()
        LATTICE_DELIVERY_SCHEMA_DIR = (Join-Path $testRoot 'schemas')
        LATTICE_DELIVERY_CODEX_HOME = $codexHome
        LATTICE_DELIVERY_ROOT = (Join-Path $testRoot 'delivery-root')
        LATTICE_DELIVERY_GIT_EXE = $gitExecutable
    }
    foreach ($entry in $values.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
    }
}

function Get-ExpectedTask038SafeConfigSha256 {
    param([Parameter(Mandatory = $true)][string]$IngressProfileSha256)

    $lines = [Collections.Generic.List[string]]::new()
    [void]$lines.Add('lattice.task038.tunnel-safe-config.v1')
    foreach ($name in @(
        'LATTICE_FULL_CHAIN_RUN_MODE', 'LATTICE_DELIVERY_CODEX_MODE',
        'LATTICE_DELIVERY_TIMEOUT_SECONDS', 'LATTICE_TASK019_HOST', 'LATTICE_TASK019_PORT',
        'LATTICE_TASK019_RUN_ID', 'LATTICE_STORE_DAEMON_INSTANCE_ID',
        'LATTICE_STORE_DAEMON_EPOCH', 'LATTICE_STORE_AUTHORITY_REVISION',
        'LATTICE_STORE_OBSERVATION_DIGEST', 'LATTICE_STORE_AUTHORITY_HEAD_DIGEST',
        'LATTICE_DELIVERY_LAUNCHER', 'LATTICE_DELIVERY_LAUNCHER_VERSION',
        'LATTICE_DELIVERY_LAUNCHER_SHA256', 'LATTICE_DELIVERY_SCHEMA_DIR',
        'LATTICE_DELIVERY_CODEX_HOME', 'LATTICE_DELIVERY_ROOT', 'LATTICE_DELIVERY_GIT_EXE'
    )) {
        [void]$lines.Add($name + '=' + [Environment]::GetEnvironmentVariable($name, 'Process'))
    }
    [void]$lines.Add('LATTICE_TASK_INGRESS_KIND=CHATGPT_SECURE_MCP_TUNNEL')
    [void]$lines.Add('LATTICE_TASK_INGRESS_PROFILE_SHA256=' + $IngressProfileSha256)
    return Get-StringSha256 -Value ($lines -join "`n")
}

try {
    $fakeRunClientSource = @'
using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;

public static class Task038FakeTunnelClient
{
    private static string Hex(byte[] bytes)
    {
        StringBuilder value = new StringBuilder(bytes.Length * 2);
        foreach (byte item in bytes) value.Append(item.ToString("x2"));
        return value.ToString();
    }

    private static string HashDelimited(params string[] parts)
    {
        using (SHA256 algorithm = SHA256.Create())
            return Hex(algorithm.ComputeHash(new UTF8Encoding(false).GetBytes(String.Join("\n", parts))));
    }

    private static string HashFile(string path)
    {
        using (SHA256 algorithm = SHA256.Create())
        using (FileStream stream = File.OpenRead(path))
            return Hex(algorithm.ComputeHash(stream));
    }

    private static string HashCommand(params string[] args)
    {
        using (SHA256 algorithm = SHA256.Create())
        using (MemoryStream input = new MemoryStream())
        {
            byte[] domain = new UTF8Encoding(false).GetBytes("lattice.tunnel-client.session-command.v1");
            input.Write(domain, 0, domain.Length);
            foreach (string arg in args)
            {
                byte[] value = new UTF8Encoding(false).GetBytes(arg);
                byte[] length = BitConverter.GetBytes((ulong)value.Length);
                if (BitConverter.IsLittleEndian) Array.Reverse(length);
                input.Write(length, 0, length.Length);
                input.Write(value, 0, value.Length);
            }
            return Hex(algorithm.ComputeHash(input.ToArray()));
        }
    }

    private static string ProcessIdentityJson(int pid, string creation, string source, string executableSha256)
    {
        return "{\"pid\":" + pid + ",\"creation_time\":\"" + creation +
            "\",\"creation_time_source\":\"" + source + "\",\"exe_sha256\":\"" +
            executableSha256 + "\"}";
    }

    private static void AppendRecord(string path, string json)
    {
        File.AppendAllText(path, json + "\n", new UTF8Encoding(false));
    }

    private static string ConfiguredExecutable(string profileRoot)
    {
        string profile = File.ReadAllText(Path.Combine(profileRoot, "lattice-local.yaml"), new UTF8Encoding(false));
        string marker = "      command: \"'";
        foreach (string line in profile.Split(new char[] { '\n' }, StringSplitOptions.RemoveEmptyEntries))
        {
            if (line.StartsWith(marker, StringComparison.Ordinal) && line.EndsWith("'\"", StringComparison.Ordinal))
            {
                string value = line.Substring(marker.Length, line.Length - marker.Length - 2);
                return value.Replace("\\\\", "\\");
            }
        }
        throw new InvalidOperationException("fixture profile command missing");
    }

    private static void EmitLifecycle(string executablePath, string innerExecutableSha256)
    {
        string path = Environment.GetEnvironmentVariable("TUNNEL_CLIENT_LIFECYCLE_EVENT_PATH");
        string session = Environment.GetEnvironmentVariable("TUNNEL_CLIENT_LIFECYCLE_SESSION_ID");
        string generation = Environment.GetEnvironmentVariable("TUNNEL_CLIENT_LIFECYCLE_CONFIG_GENERATION");
        string safeConfig = Environment.GetEnvironmentVariable("TUNNEL_CLIENT_LIFECYCLE_SAFE_CONFIG_SHA256");
        int pid = Process.GetCurrentProcess().Id;
        string creation = DateTime.UtcNow.ToString("yyyy-MM-ddTHH:mm:ss.fffffffZ");
        string source = "WINDOWS_PROCESS_TIMES";
        string executableSha256 = innerExecutableSha256;
        string identity = ProcessIdentityJson(pid, creation, source, executableSha256);
        string commandSha256 = HashCommand("latticed-fixture");
        string endpointRef = "hmac-sha256:" + new string('d', 64);
        string previous = new string('0', 64);
        string[] kinds = new string[] { "SPAWN", "OPEN", "CLOSE_REQUESTED", "PIPE_CLOSED", "EXITED", "REAPED" };
        string scenario = Environment.GetEnvironmentVariable("LATTICE_STORE_DAEMON_EPOCH");
        DateTime timeline = DateTime.UtcNow;
        int maximum = scenario == "4" ? 2 : 6;
        for (int index = 0; index < maximum; index++)
        {
            string exit = index < 4 ? "null" : "0";
            string observed = timeline.AddMilliseconds(index * 10).ToString("yyyy-MM-ddTHH:mm:ss.fffffffZ");
            string idempotency = HashDelimited(
                "lattice.tunnel-client.lifecycle-idempotency.v1", session, generation, safeConfig,
                commandSha256, endpointRef, kinds[index], pid.ToString(), creation, source,
                executableSha256, exit
            );
            string eventHash = HashDelimited(
                "lattice.tunnel-client.lifecycle-event-hash.v1", previous, idempotency,
                (index + 1).ToString(), observed
            );
            string record = "{\"schema\":\"lattice.tunnel-client.lifecycle-event.v1\"," +
                "\"record_type\":\"LIFECYCLE\",\"component\":\"mcpclient\",\"event_type\":\"" + kinds[index] +
                "\",\"session_id\":\"" + session + "\",\"process_identity\":" + identity +
                ",\"config_generation\":" + generation + ",\"safe_config_sha256\":\"" + safeConfig +
                "\",\"session_command_sha256\":\"" + commandSha256 + "\",\"endpoint_ref\":\"" + endpointRef +
                "\",\"lifecycle_strategy\":{\"transport\":\"STDIO\",\"endpoint_kind\":\"ANONYMOUS_PIPE\"," +
                "\"spawn_mode\":\"DIRECT\",\"create_suspended_owned\":false,\"job_assignment_ownership\":\"EXTERNAL_OWNER\"}," +
                "\"ordinal\":" + (index + 1) + ",\"observed_at_utc\":\"" + observed + "\",\"exit_code\":" + exit +
                ",\"previous_event_sha256\":\"" + previous + "\",\"idempotency_key\":\"" + idempotency +
                "\",\"event_sha256\":\"" + eventHash + "\",\"lifecycle_classification\":\"UNKNOWN\"," +
                "\"threshold_profile_version\":null,\"thresholds\":{\"pipe_milliseconds\":null," +
                "\"exit_milliseconds\":null,\"reap_milliseconds\":null,\"confirm_milliseconds\":null}}";
            AppendRecord(path, record);
            previous = eventHash;

            if (scenario == "3" && index == 4)
            {
                string anomalyObserved = timeline.AddMilliseconds(45).ToString("yyyy-MM-ddTHH:mm:ss.fffffffZ");
                string observedCreation = "pid-reused-creation";
                string observedExecutable = new string('e', 64);
                string observedIdentity = ProcessIdentityJson(pid, observedCreation, source, observedExecutable);
                string anomalyIdempotency = HashDelimited(
                    "lattice.tunnel-client.lifecycle-anomaly-idempotency.v1", session, generation, safeConfig,
                    commandSha256, endpointRef, "PROCESS_IDENTITY_CONFLICT", pid.ToString(), creation,
                    source, executableSha256, pid.ToString(), observedCreation, source, observedExecutable
                );
                string anomalyHash = HashDelimited(
                    "lattice.tunnel-client.lifecycle-anomaly-hash.v1", previous, anomalyIdempotency,
                    "1", anomalyObserved
                );
                string anomaly = "{\"schema\":\"lattice.tunnel-client.lifecycle-anomaly.v1\"," +
                    "\"record_type\":\"ANOMALY\",\"component\":\"mcpclient\",\"anomaly_code\":\"PROCESS_IDENTITY_CONFLICT\"," +
                    "\"session_id\":\"" + session + "\",\"expected_process_identity\":" + identity +
                    ",\"observed_process_identity\":" + observedIdentity + ",\"config_generation\":" + generation +
                    ",\"safe_config_sha256\":\"" + safeConfig + "\",\"session_command_sha256\":\"" + commandSha256 +
                    "\",\"endpoint_ref\":\"" + endpointRef + "\",\"anomaly_ordinal\":1,\"observed_at_utc\":\"" +
                    anomalyObserved + "\",\"related_event_sha256\":\"" + previous + "\",\"idempotency_key\":\"" +
                    anomalyIdempotency + "\",\"anomaly_sha256\":\"" + anomalyHash +
                    "\",\"lifecycle_classification\":\"UNKNOWN\",\"threshold_profile_version\":null," +
                    "\"thresholds\":{\"pipe_milliseconds\":null,\"exit_milliseconds\":null," +
                    "\"reap_milliseconds\":null,\"confirm_milliseconds\":null}}";
                AppendRecord(path, anomaly);
            }
        }
        if (scenario == "4")
        {
            string observed = timeline.AddMilliseconds(25).ToString("yyyy-MM-ddTHH:mm:ss.fffffffZ");
            string anomalyIdempotency = HashDelimited(
                "lattice.tunnel-client.lifecycle-anomaly-idempotency.v1", session, generation, safeConfig,
                commandSha256, endpointRef, "UNEXPECTED_EXIT_BEFORE_CLOSE", pid.ToString(), creation,
                source, executableSha256, "null", "null", "null", "null"
            );
            string anomalyHash = HashDelimited(
                "lattice.tunnel-client.lifecycle-anomaly-hash.v1", previous, anomalyIdempotency, "1", observed
            );
            string anomaly = "{\"schema\":\"lattice.tunnel-client.lifecycle-anomaly.v1\"," +
                "\"record_type\":\"ANOMALY\",\"component\":\"mcpclient\",\"anomaly_code\":\"UNEXPECTED_EXIT_BEFORE_CLOSE\"," +
                "\"session_id\":\"" + session + "\",\"expected_process_identity\":" + identity +
                ",\"observed_process_identity\":null,\"config_generation\":" + generation +
                ",\"safe_config_sha256\":\"" + safeConfig + "\",\"session_command_sha256\":\"" + commandSha256 +
                "\",\"endpoint_ref\":\"" + endpointRef + "\",\"anomaly_ordinal\":1,\"observed_at_utc\":\"" +
                observed + "\",\"related_event_sha256\":\"" + previous + "\",\"idempotency_key\":\"" +
                anomalyIdempotency + "\",\"anomaly_sha256\":\"" + anomalyHash +
                "\",\"lifecycle_classification\":\"UNKNOWN\",\"threshold_profile_version\":null," +
                "\"thresholds\":{\"pipe_milliseconds\":null,\"exit_milliseconds\":null," +
                "\"reap_milliseconds\":null,\"confirm_milliseconds\":null}}";
            AppendRecord(path, anomaly);
        }
        if (scenario == "5")
        {
            string content = File.ReadAllText(path, new UTF8Encoding(false));
            string[] lines = content.Split(new char[] { '\n' }, StringSplitOptions.RemoveEmptyEntries);
            File.WriteAllText(path, lines[1] + "\n" + lines[0] + "\n", new UTF8Encoding(false));
        }
        if (scenario == "6")
        {
            string content = File.ReadAllText(path, new UTF8Encoding(false));
            File.WriteAllText(
                path,
                content.Replace("\"config_generation\":6", "\"config_generation\":\"6\""),
                new UTF8Encoding(false)
            );
        }
    }

    public static int Main(string[] args)
    {
        string root = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location);
        if (args.Length > 0 && args[0] != "run")
        {
            foreach (string name in new string[] {
                "OPENAI_API_KEY", "OPENAI_ADMIN_KEY", "HTTP_PROXY", "HTTPS_PROXY",
                "MCP_COMMAND", "TUNNEL_CLIENT_CONFIG", "LATTICE_TEST_PRESERVED",
                "TUNNEL_CLIENT_LIFECYCLE_EVENT_PATH", "TUNNEL_CLIENT_LIFECYCLE_SESSION_ID",
                "TUNNEL_CLIENT_LIFECYCLE_CONFIG_GENERATION", "TUNNEL_CLIENT_LIFECYCLE_SAFE_CONFIG_SHA256"
            })
                if (Environment.GetEnvironmentVariable(name) != null) return 41;
            File.WriteAllText(Path.Combine(root, "capture.txt"), String.Join(" ", args));
            return 0;
        }
        string[] required = new string[] {
            "CONTROL_PLANE_API_KEY", "LATTICE_FULL_CHAIN_RUN_MODE",
            "LATTICE_DELIVERY_CODEX_MODE", "LATTICE_DELIVERY_TIMEOUT_SECONDS",
            "LATTICE_TASK019_HOST", "LATTICE_TASK019_PORT", "LATTICE_TASK019_RUN_ID",
            "LATTICE_TASK019_PASSWORD", "LATTICE_STORE_DAEMON_INSTANCE_ID",
            "LATTICE_STORE_DAEMON_EPOCH", "LATTICE_STORE_AUTHORITY_REVISION",
            "LATTICE_STORE_OBSERVATION_DIGEST", "LATTICE_STORE_AUTHORITY_HEAD_DIGEST",
            "LATTICE_DELIVERY_LAUNCHER", "LATTICE_DELIVERY_LAUNCHER_VERSION",
            "LATTICE_DELIVERY_LAUNCHER_SHA256", "LATTICE_DELIVERY_SCHEMA_DIR",
            "LATTICE_DELIVERY_CODEX_HOME", "LATTICE_DELIVERY_ROOT",
            "LATTICE_DELIVERY_GIT_EXE", "LATTICE_TASK_INGRESS_KIND",
            "LATTICE_TASK_INGRESS_PROFILE_SHA256", "TUNNEL_CLIENT_LIFECYCLE_EVENT_PATH",
            "TUNNEL_CLIENT_LIFECYCLE_SESSION_ID", "TUNNEL_CLIENT_LIFECYCLE_CONFIG_GENERATION",
            "TUNNEL_CLIENT_LIFECYCLE_SAFE_CONFIG_SHA256"
        };
        foreach (string name in required)
            if (String.IsNullOrEmpty(Environment.GetEnvironmentVariable(name))) return 51;

        if (Environment.GetEnvironmentVariable("CONTROL_PLANE_API_KEY") != "test-runtime-key-not-a-secret")
            return 52;
        foreach (string name in new string[] {
            "OPENAI_API_KEY", "OPENAI_ADMIN_KEY", "HTTP_PROXY", "HTTPS_PROXY",
            "MCP_COMMAND", "TUNNEL_CLIENT_CONFIG", "LATTICE_TEST_PRESERVED"
        })
            if (Environment.GetEnvironmentVariable(name) != null) return 53;

        File.WriteAllText(Path.Combine(root, "capture.txt"), String.Join(" ", args));
        File.WriteAllText(Path.Combine(root, "ingress-kind.txt"), Environment.GetEnvironmentVariable("LATTICE_TASK_INGRESS_KIND"));
        File.WriteAllText(Path.Combine(root, "ingress-profile.txt"), Environment.GetEnvironmentVariable("LATTICE_TASK_INGRESS_PROFILE_SHA256"));
        EmitLifecycle(
            Assembly.GetExecutingAssembly().Location,
            HashFile(ConfiguredExecutable(args[4]))
        );

        ProcessStartInfo childInfo = new ProcessStartInfo();
        childInfo.FileName = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "ping.exe");
        childInfo.Arguments = "127.0.0.1 -n 30";
        childInfo.UseShellExecute = false;
        childInfo.CreateNoWindow = true;
        Process child = Process.Start(childInfo);
        File.WriteAllText(Path.Combine(root, "descendant-pid.txt"), child.Id.ToString());
        child.Dispose();
        return 0;
    }
}
'@
    Add-Type `
        -TypeDefinition $fakeRunClientSource `
        -Language CSharp `
        -OutputAssembly $fakeRunClient `
        -OutputType ConsoleApplication `
        -ErrorAction Stop
    Add-Type `
        -TypeDefinition 'public static class Task038FailingTunnelClient { public static int Main(string[] args) { return 9; } }' `
        -Language CSharp `
        -OutputAssembly $failingClient `
        -OutputType ConsoleApplication `
        -ErrorAction Stop
    & $failingClient
    if ($LASTEXITCODE -ne 9) {
        throw 'TASK038_FAILING_TUNNEL_CLIENT_FIXTURE_REJECTED'
    }
    [IO.File]::WriteAllBytes($latticed, [byte[]]@(0))
    [IO.File]::WriteAllBytes($deliveryLauncher, [byte[]]@(1))
    [IO.File]::WriteAllBytes($gitExecutable, [byte[]]@(2))
    [IO.File]::WriteAllBytes($unsafeLatticed, [byte[]]@(0))
    [IO.Directory]::CreateDirectory($codexHome) | Out-Null
    [Environment]::SetEnvironmentVariable('CONTROL_PLANE_API_KEY', $null, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TEST_PRESERVED', 'fixture-lattice-value', 'Process')
    foreach ($name in $hostileVariables) {
        [Environment]::SetEnvironmentVariable($name, 'fixture-hostile-value', 'Process')
    }

    & $launcher -Mode Init -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory -TunnelId $tunnelId -LatticedExecutable $latticed
    $profileRoot = [IO.Path]::GetFullPath($profileDirectory)
    $expectedInit = "init --sample sample_mcp_stdio_local --profile lattice-local --profile-dir $profileRoot --tunnel-id $tunnelId --mcp-command '$latticed'"
    Assert-Equal -Expected $expectedInit -Actual ([IO.File]::ReadAllText($capturePath).Trim()) -FailureCode 'TASK038_INIT_ARGUMENTS_REJECTED'

    $profilePath = Join-Path $profileDirectory 'lattice-local.yaml'
    $yamlLatticed = $latticed.Replace('\', '\\')
    $profileText = New-Task038TunnelProfileText -TunnelId $tunnelId -YamlLatticed $yamlLatticed
    [IO.File]::WriteAllText($profilePath, $profileText, [Text.UTF8Encoding]::new($false))

    & $launcher -Mode Doctor -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    $expectedDoctor = "doctor --profile lattice-local --profile-dir $profileRoot --explain"
    Assert-Equal -Expected $expectedDoctor -Actual ([IO.File]::ReadAllText($capturePath).Trim()) -FailureCode 'TASK038_DOCTOR_ARGUMENTS_REJECTED'

    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_RUNTIME_KEY_REQUIRED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    }
    Set-Task038ValidRuntimeEnvironment
    [Environment]::SetEnvironmentVariable('LATTICE_TASK_INGRESS_KIND', 'fixture-hostile-kind', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK_INGRESS_PROFILE_SHA256', 'fixture-hostile-profile', 'Process')

    foreach ($reservedPort in @(5432, 64272, 55432)) {
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PORT', [string]$reservedPort, 'Process')
        Assert-FailsWith -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED' -Action {
            & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
        }
    }
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PORT', '56981', 'Process')

    foreach ($numericMutation in @(
        @{ Name = 'LATTICE_DELIVERY_TIMEOUT_SECONDS'; Value = '0300' },
        @{ Name = 'LATTICE_TASK019_PORT'; Value = '056981' },
        @{ Name = 'LATTICE_STORE_DAEMON_EPOCH'; Value = '01' },
        @{ Name = 'LATTICE_STORE_AUTHORITY_REVISION'; Value = '01' }
    )) {
        [Environment]::SetEnvironmentVariable($numericMutation.Name, $numericMutation.Value, 'Process')
        Assert-FailsWith -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED' -Action {
            & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
        }
        Set-Task038ValidRuntimeEnvironment
    }

    foreach ($invalidRunId in @(
        '0123456789ABCDEF0123456789ABCDEF',
        '0123456789abcdef0123456789abcde',
        'g123456789abcdef0123456789abcdef',
        ' 0123456789abcdef0123456789abcdef',
        "0123456789abcdef0123456789abcdef`n"
    )) {
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_RUN_ID', $invalidRunId, 'Process')
        Assert-FailsWith -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED' -Action {
            & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
        }
    }
    [Environment]::SetEnvironmentVariable(
        'LATTICE_TASK019_RUN_ID',
        '0123456789abcdef0123456789abcdef',
        'Process'
    )

    $validLauncherSha256 = [Environment]::GetEnvironmentVariable('LATTICE_DELIVERY_LAUNCHER_SHA256', 'Process')
    [Environment]::SetEnvironmentVariable(
        'LATTICE_DELIVERY_LAUNCHER_SHA256',
        (('0' * 64) -join ''),
        'Process'
    )
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
    }
    [Environment]::SetEnvironmentVariable(
        'LATTICE_DELIVERY_LAUNCHER_SHA256',
        $validLauncherSha256,
        'Process'
    )

    $commandLine = '      command: "''' + $yamlLatticed + '''"'
    $profileMutations = @(
        @{
            Name = 'base-url'
            Text = $profileText.Replace(
                '  base_url: "https://api.openai.com"',
                '  base_url: "https://example.invalid"'
            )
        },
        @{
            Name = 'config-version'
            Text = $profileText.Replace('config_version: 1', 'config_version: 2')
        },
        @{
            Name = 'api-key-placeholder'
            Text = $profileText.Replace(
                '  api_key: "env:CONTROL_PLANE_API_KEY"',
                '  api_key: "${CONTROL_PLANE_API_KEY}"'
            )
        },
        @{
            Name = 'extra-key'
            Text = $profileText.Replace('control_plane:', "control_plane:`n  extra: false")
        },
        @{
            Name = 'extra-channel'
            Text = $profileText.Replace(
                '    - channel: main',
                "    - channel: main`n    - channel: secondary"
            )
        },
        @{
            Name = 'extra-command'
            Text = $profileText.Replace($commandLine, ($commandLine + "`n" + $commandLine))
        }
    )
    foreach ($mutation in $profileMutations) {
        if ([String]::Equals($profileText, $mutation.Text, [StringComparison]::Ordinal)) {
            throw ('TASK038_PROFILE_MUTATION_FIXTURE_REJECTED_' + $mutation.Name)
        }
        [IO.File]::WriteAllText($profilePath, $mutation.Text, [Text.UTF8Encoding]::new($false))
        Assert-FailsWith -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED' -Action {
            & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
        }
    }
    [IO.File]::WriteAllBytes($profilePath, [byte[]]@(0xff, 0xfe, 0xfd))
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
    }
    $profileUtf8 = [Text.UTF8Encoding]::new($false).GetBytes($profileText)
    $profileBomBytes = [byte[]]::new($profileUtf8.Length + 3)
    [Array]::Copy([byte[]]@(0xef, 0xbb, 0xbf), 0, $profileBomBytes, 0, 3)
    [Array]::Copy($profileUtf8, 0, $profileBomBytes, 3, $profileUtf8.Length)
    [IO.File]::WriteAllBytes($profilePath, $profileBomBytes)
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
    }
    [IO.File]::WriteAllText($profilePath, $profileText, [Text.UTF8Encoding]::new($false))

    $runOutput = @(& $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory)
    $expectedRun = "run --profile lattice-local --profile-dir $profileRoot"
    Assert-Equal -Expected $expectedRun -Actual ([IO.File]::ReadAllText($capturePath).Trim()) -FailureCode 'TASK038_RUN_ARGUMENTS_REJECTED'
    Assert-Equal -Expected 'CHATGPT_SECURE_MCP_TUNNEL' -Actual ([IO.File]::ReadAllText($ingressKindPath).Trim()) -FailureCode 'TASK038_RUN_INGRESS_KIND_REJECTED'
    $profileSha256 = (Get-FileHash -LiteralPath $profilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $expectedProfileCommitment = @(
        'lattice.task-ingress-profile.v2',
        'profile_name=lattice-local',
        ('profile_sha256=' + $profileSha256),
        ('tunnel_id=' + $tunnelId),
        'channel=main',
        ('tunnel_client_sha256=' + (Get-FileHash -LiteralPath $fakeRunClient -Algorithm SHA256).Hash.ToLowerInvariant()),
        ('latticed_sha256=' + (Get-FileHash -LiteralPath $latticed -Algorithm SHA256).Hash.ToLowerInvariant())
    ) -join "`n"
    $expectedProfileDigest = Get-StringSha256 -Value $expectedProfileCommitment
    $expectedSafeConfigSha256 = Get-ExpectedTask038SafeConfigSha256 -IngressProfileSha256 $expectedProfileDigest
    Assert-Equal -Expected $expectedProfileDigest -Actual ([IO.File]::ReadAllText($ingressProfilePath).Trim()) -FailureCode 'TASK038_RUN_INGRESS_PROFILE_REJECTED'
    $outerReceipt = $runOutput[-1] | ConvertFrom-Json -ErrorAction Stop
    Assert-Equal -Expected 'lattice.task038.tunnel-outer-lifecycle.v1' -Actual ([string]$outerReceipt.schema) -FailureCode 'TASK038_OUTER_RECEIPT_SCHEMA_REJECTED'
    Assert-Equal -Expected 'C_CALIBRATION_FIRST' -Actual ([string]$outerReceipt.lifecycle_threshold_decision) -FailureCode 'TASK038_LIFECYCLE_DECISION_REJECTED'
    Assert-Equal -Expected 'UNKNOWN' -Actual ([string]$outerReceipt.lifecycle_classification) -FailureCode 'TASK038_LIFECYCLE_CLASSIFICATION_REJECTED'
    if (
        $null -ne $outerReceipt.lifecycle_threshold_profile -or
        $null -ne $outerReceipt.lifecycle_thresholds.pipe_milliseconds -or
        $null -ne $outerReceipt.lifecycle_thresholds.exit_milliseconds -or
        $null -ne $outerReceipt.lifecycle_thresholds.reap_milliseconds -or
        $null -ne $outerReceipt.lifecycle_thresholds.confirm_milliseconds -or
        [int]$outerReceipt.tunnel_client_exit_code -ne 0 -or
        -not [bool]$outerReceipt.create_suspended -or
        -not [bool]$outerReceipt.job_assigned_before_resume -or
        [long]$outerReceipt.descendant_processes_after_cleanup -ne 0 -or
        [bool]$outerReceipt.leak_claimed -or
        -not [bool]$outerReceipt.profile_strict_utf8 -or
        [long]$outerReceipt.profile_byte_count -ne $profileUtf8.Length -or
        -not [bool]$outerReceipt.lifecycle_event_strict_utf8 -or
        [string]$outerReceipt.lifecycle_session_id -cnotmatch '\A[0-9a-f]{32}\z' -or
        [long]$outerReceipt.lifecycle_config_generation -ne 1 -or
        [string]$outerReceipt.lifecycle_safe_config_schema -cne 'lattice.task038.tunnel-safe-config.v1' -or
        [string]$outerReceipt.lifecycle_safe_config_sha256 -cne $expectedSafeConfigSha256 -or
        [string]$outerReceipt.lifecycle_safe_config_sha256 -ceq $expectedProfileDigest -or
        [long]$outerReceipt.lifecycle_safe_config_byte_count -lt 1 -or
        [long]$outerReceipt.lifecycle_event_count -ne 6 -or
        [long]$outerReceipt.lifecycle_anomaly_count -ne 0 -or
        -not [bool]$outerReceipt.lifecycle_chain_complete -or
        -not [bool]$outerReceipt.lifecycle_normal_close_complete -or
        [string]$outerReceipt.lifecycle_final_event_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        [long]$outerReceipt.lifecycle_inner_process_id -lt 1 -or
        [string]$outerReceipt.lifecycle_inner_process_creation_time_source -cne 'WINDOWS_PROCESS_TIMES' -or
        [string]$outerReceipt.lifecycle_inner_process_exe_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        [int]$outerReceipt.lifecycle_inner_exit_code -ne 0
    ) {
        throw 'TASK038_OUTER_RECEIPT_CONTENT_REJECTED'
    }
    $lifecycleEventPath = [string]$outerReceipt.lifecycle_event_path
    if (-not (Test-Path -LiteralPath $lifecycleEventPath -PathType Leaf)) {
        throw 'TASK038_LIFECYCLE_EVENT_FILE_MISSING'
    }
    Assert-Equal `
        -Expected ([string]$outerReceipt.lifecycle_event_raw_sha256) `
        -Actual ((Get-FileHash -LiteralPath $lifecycleEventPath -Algorithm SHA256).Hash.ToLowerInvariant()) `
        -FailureCode 'TASK038_LIFECYCLE_EVENT_RAW_SHA_REJECTED'
    if ([long]$outerReceipt.lifecycle_event_byte_count -ne (Get-Item -LiteralPath $lifecycleEventPath).Length) {
        throw 'TASK038_LIFECYCLE_EVENT_BYTE_COUNT_REJECTED'
    }
    $lifecycleRawText = [Text.UTF8Encoding]::new($false, $true).GetString(
        [IO.File]::ReadAllBytes($lifecycleEventPath)
    )
    if (
        $lifecycleRawText.Contains('test-runtime-key-not-a-secret') -or
        $lifecycleRawText.Contains('fixture-password-private')
    ) {
        throw 'TASK038_LIFECYCLE_CREDENTIAL_DISCLOSURE_REJECTED'
    }
    $descendantPid = [int]([IO.File]::ReadAllText($descendantPidPath).Trim())
    Start-Sleep -Milliseconds 200
    if ($null -ne (Get-Process -Id $descendantPid -ErrorAction SilentlyContinue)) {
        throw 'TASK038_TUNNEL_DESCENDANT_NOT_REAPED'
    }

    [Environment]::SetEnvironmentVariable('LATTICE_STORE_DAEMON_EPOCH', '3', 'Process')
    $pidReuseOutput = @(& $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory)
    $pidReuseReceipt = $pidReuseOutput[-1] | ConvertFrom-Json -ErrorAction Stop
    if (
        [long]$pidReuseReceipt.lifecycle_event_count -ne 6 -or
        [long]$pidReuseReceipt.lifecycle_anomaly_count -ne 1 -or
        [string]$pidReuseReceipt.lifecycle_anomaly_codes[0] -cne 'PROCESS_IDENTITY_CONFLICT' -or
        -not [bool]$pidReuseReceipt.lifecycle_chain_complete -or
        [bool]$pidReuseReceipt.lifecycle_normal_close_complete -or
        [string]$pidReuseReceipt.lifecycle_classification -cne 'UNKNOWN' -or
        [bool]$pidReuseReceipt.leak_claimed
    ) {
        throw 'TASK038_PID_REUSE_EVIDENCE_REJECTED'
    }

    [Environment]::SetEnvironmentVariable('LATTICE_STORE_DAEMON_EPOCH', '4', 'Process')
    $abnormalOutput = @(& $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory)
    $abnormalReceipt = $abnormalOutput[-1] | ConvertFrom-Json -ErrorAction Stop
    if (
        [long]$abnormalReceipt.lifecycle_event_count -ne 2 -or
        [long]$abnormalReceipt.lifecycle_anomaly_count -ne 1 -or
        [string]$abnormalReceipt.lifecycle_anomaly_codes[0] -cne 'UNEXPECTED_EXIT_BEFORE_CLOSE' -or
        [bool]$abnormalReceipt.lifecycle_chain_complete -or
        [bool]$abnormalReceipt.lifecycle_normal_close_complete -or
        $null -ne $abnormalReceipt.lifecycle_inner_exit_code -or
        [string]$abnormalReceipt.lifecycle_classification -cne 'UNKNOWN' -or
        [bool]$abnormalReceipt.leak_claimed
    ) {
        throw 'TASK038_ABNORMAL_PREFIX_EVIDENCE_REJECTED'
    }

    [Environment]::SetEnvironmentVariable('LATTICE_STORE_DAEMON_EPOCH', '5', 'Process')
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_LIFECYCLE_EVIDENCE_REJECTED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
    }
    [Environment]::SetEnvironmentVariable('LATTICE_STORE_DAEMON_EPOCH', '6', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_STORE_AUTHORITY_REVISION', '6', 'Process')
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_LIFECYCLE_EVIDENCE_REJECTED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory
    }
    [Environment]::SetEnvironmentVariable('LATTICE_STORE_DAEMON_EPOCH', '1', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_STORE_AUTHORITY_REVISION', '1', 'Process')

    [Environment]::SetEnvironmentVariable('LATTICE_STORE_AUTHORITY_REVISION', '2', 'Process')
    $generationOutput = @(& $launcher -Mode Run -TunnelClientExecutable $fakeRunClient -ProfileDirectory $profileDirectory)
    $generationReceipt = $generationOutput[-1] | ConvertFrom-Json -ErrorAction Stop
    if (
        [long]$generationReceipt.lifecycle_config_generation -ne 2 -or
        [string]$generationReceipt.lifecycle_session_id -ceq [string]$outerReceipt.lifecycle_session_id -or
        [string]$generationReceipt.lifecycle_event_path -ceq [string]$outerReceipt.lifecycle_event_path -or
        [string]$generationReceipt.lifecycle_safe_config_sha256 -ceq [string]$outerReceipt.lifecycle_safe_config_sha256
    ) {
        throw 'TASK038_LIFECYCLE_GENERATION_SWITCH_REJECTED'
    }
    [Environment]::SetEnvironmentVariable('LATTICE_STORE_AUTHORITY_REVISION', '1', 'Process')

    $overlapEnvironment = @{}
    foreach ($name in @(
        'CONTROL_PLANE_API_KEY', 'LATTICE_FULL_CHAIN_RUN_MODE', 'LATTICE_DELIVERY_CODEX_MODE',
        'LATTICE_DELIVERY_TIMEOUT_SECONDS', 'LATTICE_TASK019_HOST', 'LATTICE_TASK019_PORT',
        'LATTICE_TASK019_RUN_ID', 'LATTICE_TASK019_PASSWORD', 'LATTICE_STORE_DAEMON_INSTANCE_ID',
        'LATTICE_STORE_DAEMON_EPOCH', 'LATTICE_STORE_AUTHORITY_REVISION',
        'LATTICE_STORE_OBSERVATION_DIGEST', 'LATTICE_STORE_AUTHORITY_HEAD_DIGEST',
        'LATTICE_DELIVERY_LAUNCHER', 'LATTICE_DELIVERY_LAUNCHER_VERSION',
        'LATTICE_DELIVERY_LAUNCHER_SHA256', 'LATTICE_DELIVERY_SCHEMA_DIR',
        'LATTICE_DELIVERY_CODEX_HOME', 'LATTICE_DELIVERY_ROOT', 'LATTICE_DELIVERY_GIT_EXE'
    )) {
        $overlapEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    }
    $overlapAction = {
        param($LauncherPath, $ClientPath, $ProfilesPath, $EnvironmentValues, $Generation)
        $ErrorActionPreference = 'Stop'
        foreach ($entry in $EnvironmentValues.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        [Environment]::SetEnvironmentVariable('LATTICE_STORE_DAEMON_EPOCH', [string]$Generation, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_STORE_AUTHORITY_REVISION', [string]$Generation, 'Process')
        & $LauncherPath -Mode Run -TunnelClientExecutable $ClientPath -ProfileDirectory $ProfilesPath
    }
    $overlapJobs = @(
        Start-Job -ScriptBlock $overlapAction -ArgumentList $launcher, $fakeRunClient, $profileDirectory, $overlapEnvironment, 21
        Start-Job -ScriptBlock $overlapAction -ArgumentList $launcher, $fakeRunClient, $profileDirectory, $overlapEnvironment, 22
    )
    try {
        $null = Wait-Job -Job $overlapJobs -Timeout 60
        if (@($overlapJobs | Where-Object State -ne 'Completed').Count -ne 0) {
            throw 'TASK038_LIFECYCLE_OVERLAP_JOB_REJECTED'
        }
        $overlapReceipts = @($overlapJobs | ForEach-Object {
            $jobOutput = @(Receive-Job -Job $_ -ErrorAction Stop)
            $jobOutput[-1] | ConvertFrom-Json -ErrorAction Stop
        })
        if (
            $overlapReceipts.Count -ne 2 -or
            [long]$overlapReceipts[0].lifecycle_config_generation -eq [long]$overlapReceipts[1].lifecycle_config_generation -or
            [string]$overlapReceipts[0].lifecycle_session_id -ceq [string]$overlapReceipts[1].lifecycle_session_id -or
            [string]$overlapReceipts[0].lifecycle_event_path -ceq [string]$overlapReceipts[1].lifecycle_event_path -or
            -not [bool]$overlapReceipts[0].lifecycle_chain_complete -or
            -not [bool]$overlapReceipts[1].lifecycle_chain_complete
        ) {
            throw 'TASK038_LIFECYCLE_OVERLAP_EVIDENCE_REJECTED'
        }
    }
    finally {
        $overlapJobs | Remove-Job -Force -ErrorAction SilentlyContinue
    }
    Assert-Equal -Expected 'fixture-hostile-kind' -Actual ([Environment]::GetEnvironmentVariable('LATTICE_TASK_INGRESS_KIND', 'Process')) -FailureCode 'TASK038_PARENT_INGRESS_KIND_RESTORE_REJECTED'
    Assert-Equal -Expected 'fixture-hostile-profile' -Actual ([Environment]::GetEnvironmentVariable('LATTICE_TASK_INGRESS_PROFILE_SHA256', 'Process')) -FailureCode 'TASK038_PARENT_INGRESS_PROFILE_RESTORE_REJECTED'

    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_ID_REJECTED' -Action {
        & $launcher -Mode Init -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory -TunnelId 'tunnel_bad' -LatticedExecutable $latticed
    }
    Assert-FailsWith -FailureCode 'TASK038_LATTICED_COMMAND_REJECTED' -Action {
        & $launcher -Mode Init -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory -TunnelId $tunnelId -LatticedExecutable $unsafeLatticed
    }

    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_CLIENT_FAILED_DOCTOR' -Action {
        & $launcher -Mode Doctor -TunnelClientExecutable $failingClient -ProfileDirectory $profileDirectory
    }

    foreach ($name in $hostileVariables) {
        if ([Environment]::GetEnvironmentVariable($name, 'Process') -ne 'fixture-hostile-value') {
            throw 'TASK038_PARENT_ENVIRONMENT_RESTORE_REJECTED'
        }
    }

    $launcherText = [IO.File]::ReadAllText($launcher)
    if ($launcherText -match '(?im)^\s*\[string\]\s*\$.*(?:ApiKey|Credential|Secret)') {
        throw 'TASK038_CREDENTIAL_SURFACE_REJECTED'
    }
    foreach ($requiredClosure in @(
        'profile_sha256=',
        'lattice.task-ingress-profile.v2',
        'api_key: "env:CONTROL_PLANE_API_KEY"',
        'ReparsePoint',
        'CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT',
        'AssignProcessToJobObject',
        'ResumeThread',
        'TerminateJobObject',
        'Get-LatticeWindowsNativePathIdentityToken',
        'Test-LatticeWindowsNativePathIdentity',
        "port -in @(5432, 64272, 55432)",
        "lifecycle_threshold_decision = 'C_CALIBRATION_FIRST'",
        'lifecycle_threshold_profile = $null',
        "lifecycle_classification = 'UNKNOWN'",
        'leak_claimed = $false',
        'TUNNEL_CLIENT_LIFECYCLE_EVENT_PATH',
        'TUNNEL_CLIENT_LIFECYCLE_SESSION_ID',
        'TUNNEL_CLIENT_LIFECYCLE_CONFIG_GENERATION',
        'TUNNEL_CLIENT_LIFECYCLE_SAFE_CONFIG_SHA256',
        'lattice.tunnel-client.lifecycle-event.v1',
        'lattice.tunnel-client.lifecycle-anomaly.v1',
        'lattice.tunnel-client.lifecycle-idempotency.v1',
        'lattice.tunnel-client.lifecycle-event-hash.v1',
        'lattice.tunnel-client.lifecycle-anomaly-idempotency.v1',
        'lattice.tunnel-client.lifecycle-anomaly-hash.v1',
        'lattice.task038.tunnel-safe-config.v1',
        '1048576'
    )) {
        if ($launcherText.IndexOf($requiredClosure, [StringComparison]::Ordinal) -lt 0) {
            throw 'TASK038_TUNNEL_PROFILE_STATIC_CLOSURE_REJECTED'
        }
    }
    if ($launcherText.IndexOf('SKIP', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw 'TASK038_TUNNEL_SKIP_PATH_REJECTED'
    }
    $task038VerifierText = [IO.File]::ReadAllText($task038Verifier)
    if ($task038VerifierText -notmatch "'--bin', 'latticed'" -or
        $task038VerifierText -notmatch 'client_request_id\s*=\s*\$sameClientRequestId' -or
        $task038VerifierText -notmatch "intent\s*=\s*'CONTROLLED_CODEX_CANARY'" -or
        $task038VerifierText -notmatch "'lattice_task_submit'" -or
        $task038VerifierText -notmatch "'lattice_task_status'" -or
        $task038VerifierText -match '(?i)run-task037|lattice-full-chain|LATTICE_HERMES_|LATTICE_OPENCLAW') {
        throw 'TASK038_LOCAL_VERIFIER_ARGUMENTS_REJECTED'
    }
    Write-Output 'TASK038_TUNNEL_ENTRYPOINT=PASS'
}
finally {
    foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, $null, 'Process')
    }
    foreach ($entry in $originalProcessEnvironment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable(
            [string]$entry.Key,
            [string]$entry.Value,
            'Process'
        )
    }
    if (Test-Path -LiteralPath $testRoot) {
        $resolvedRoot = [IO.Path]::GetFullPath($testRoot)
        if (-not $resolvedRoot.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'TASK038_TEST_CLEANUP_REJECTED'
        }
        Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
    }
}
