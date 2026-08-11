[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$launcher = Join-Path $PSScriptRoot 'start-chatgpt-mcp-tunnel.ps1'
$task038Verifier = Join-Path $PSScriptRoot 'run-task038-task-submit.ps1'
$task019Harness = Join-Path $PSScriptRoot 'run-task019-postgres.ps1'
$nativeIdentityHelper = Join-Path $PSScriptRoot 'windows-native-path-identity.ps1'
if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
    throw 'TASK038_TUNNEL_LAUNCHER_MISSING'
}
if (-not (Test-Path -LiteralPath $task038Verifier -PathType Leaf)) {
    throw 'TASK038_LOCAL_VERIFIER_MISSING'
}
if (-not (Test-Path -LiteralPath $task019Harness -PathType Leaf)) {
    throw 'TASK038_POSTGRES_HARNESS_MISSING'
}
if (-not (Test-Path -LiteralPath $nativeIdentityHelper -PathType Leaf)) {
    throw 'TASK038_WINDOWS_NATIVE_IDENTITY_HELPER_MISSING'
}
$nativeIdentityHelperItem = Get-Item -LiteralPath $nativeIdentityHelper -Force
if ($nativeIdentityHelperItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
    throw 'TASK038_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}
$nativeIdentityHelperSource = [Text.UTF8Encoding]::new($false, $true).GetString(
    [IO.File]::ReadAllBytes($nativeIdentityHelperItem.FullName)
)
. ([scriptblock]::Create($nativeIdentityHelperSource))
Initialize-LatticeWindowsNativePathIdentity

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
        ('      command: "' + $YamlLatticed + '"')
    ) -join "`n") + "`n")
}

function Invoke-Task038OwnedProcessFaultTest {
    param([Parameter(Mandatory = $true)][string]$OwnedTypeSource)

    if ($null -eq ('Lattice.Task038.OwnedProcessTree' -as [type])) {
        Add-Type -TypeDefinition $OwnedTypeSource -Language CSharp -ErrorAction Stop
    }
    $command = Join-Path $env:SystemRoot 'System32\cmd.exe'
    $originalControlPlaneKey = [Environment]::GetEnvironmentVariable('CONTROL_PLANE_API_KEY', 'Process')
    $credentialOwned = $null
    try {
        [Environment]::SetEnvironmentVariable(
            'CONTROL_PLANE_API_KEY',
            'task038-direct-latticed-inheritance-sentinel',
            'Process'
        )
        $credentialPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
        $credentialOwned = [Lattice.Task038.OwnedProcessTree]::Start(
            $credentialPowerShell,
            '-NoProfile -NonInteractive -Command "Start-Sleep -Milliseconds 750; if ($env:CONTROL_PLANE_API_KEY -ceq ''task038-direct-latticed-inheritance-sentinel'') { exit 0 } else { exit 77 }"',
            (Get-Location).Path
        )
        $credentialProcess = $credentialOwned.RootProcess
        if (-not $credentialProcess.WaitForExit(10000) -or [int]$credentialProcess.ExitCode -ne 0) {
            throw 'TASK038_TUNNEL_DIRECT_LATTICED_KEY_INHERITANCE_REJECTED'
        }
    }
    finally {
        if ($null -ne $credentialOwned) {
            try { $null = $credentialOwned.TerminateAndWait(5000) } catch {}
            $credentialOwned.Dispose()
        }
        [Environment]::SetEnvironmentVariable(
            'CONTROL_PLANE_API_KEY',
            $originalControlPlaneKey,
            'Process'
        )
    }
    $owned = [Lattice.Task038.OwnedProcessTree]::Start(
        $command,
        '/d /c ping.exe -n 30 127.0.0.1 >nul',
        (Get-Location).Path
    )
    $root = $owned.RootProcess
    Start-Sleep -Milliseconds 800
    $children = @(Get-CimInstance Win32_Process | Where-Object { [int]$_.ParentProcessId -eq $root.Id })
    if ($root.HasExited -or $children.Count -lt 1 -or -not $owned.TerminateAndWait(15000)) {
        $owned.Dispose()
        throw 'TASK038_TUNNEL_JOB_NORMAL_REAP_REJECTED'
    }
    $rootId = $root.Id
    $childIds = @($children.ProcessId)
    $owned.Dispose()
    if (
        $null -ne (Get-Process -Id $rootId -ErrorAction SilentlyContinue) -or
        @($childIds | Where-Object { $null -ne (Get-Process -Id $_ -ErrorAction SilentlyContinue) }).Count -ne 0
    ) {
        throw 'TASK038_TUNNEL_JOB_NORMAL_REAP_REJECTED'
    }

    $sourceBase64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($OwnedTypeSource))
    $helperSource = @"
`$source = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$sourceBase64'))
Add-Type -TypeDefinition `$source -Language CSharp -ErrorAction Stop
`$command = Join-Path `$env:SystemRoot 'System32\cmd.exe'
`$owned = [Lattice.Task038.OwnedProcessTree]::Start(`$command, '/d /c ping.exe -n 30 127.0.0.1 >nul', (Get-Location).Path)
Start-Sleep -Milliseconds 800
`$root = `$owned.RootProcess
`$child = @(Get-CimInstance Win32_Process | Where-Object { [int]`$_.ParentProcessId -eq `$root.Id } | Select-Object -First 1)
[Console]::Out.WriteLine(([string]`$root.Id + '|' + [string]`$child.ProcessId))
[Console]::Out.Flush()
Start-Sleep -Seconds 30
"@
    $encodedHelper = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($helperSource))
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = (Get-Command powershell.exe -ErrorAction Stop).Source
    $startInfo.Arguments = '-NoProfile -NonInteractive -EncodedCommand ' + $encodedHelper
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $owner = [Diagnostics.Process]::Start($startInfo)
    $parentDeathRoot = 0
    $parentDeathChild = 0
    try {
        $lineTask = $owner.StandardOutput.ReadLineAsync()
        if (-not $lineTask.Wait(15000)) {
            throw 'TASK038_TUNNEL_JOB_PARENT_DEATH_SETUP_REJECTED'
        }
        $line = [string]$lineTask.Result
        if ($line -notmatch '^(\d+)\|(\d+)$') {
            throw 'TASK038_TUNNEL_JOB_PARENT_DEATH_SETUP_REJECTED'
        }
        $parentDeathRoot = [int]$Matches[1]
        $parentDeathChild = [int]$Matches[2]
        $owner.Kill()
        if (-not $owner.WaitForExit(15000)) {
            throw 'TASK038_TUNNEL_JOB_PARENT_DEATH_OWNER_REJECTED'
        }
        $null = $owner.StandardError.ReadToEnd()
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        do {
            $rootAlive = $null -ne (Get-Process -Id $parentDeathRoot -ErrorAction SilentlyContinue)
            $childAlive = $null -ne (Get-Process -Id $parentDeathChild -ErrorAction SilentlyContinue)
            if (-not $rootAlive -and -not $childAlive) { break }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $deadline)
        if ($rootAlive -or $childAlive) {
            throw 'TASK038_TUNNEL_JOB_PARENT_DEATH_REAP_REJECTED'
        }
    }
    finally {
        try { if (-not $owner.HasExited) { $owner.Kill() } } catch {}
        $owner.Dispose()
        foreach ($processId in @($parentDeathRoot, $parentDeathChild)) {
            if ($processId -gt 0) { Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue }
        }
    }
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
$fakeClient = Join-Path $testRoot 'tunnel-client.cmd'
$failingClient = Join-Path $testRoot 'failing-tunnel-client.cmd'
$latticed = Join-Path $testRoot 'latticed.exe'
$unsafeLatticed = Join-Path $testRoot "bad'latticed.exe"
$profileDirectory = Join-Path $testRoot 'profiles'
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
    'TUNNEL_CLIENT_PROFILE_FILE'
)
$originalProcessEnvironment = [Environment]::GetEnvironmentVariables('Process')

try {
    $nativeContainmentRoot = Join-Path $testRoot 'native-containment-root'
    [IO.Directory]::CreateDirectory($nativeContainmentRoot) | Out-Null
    $nativeContainmentMarker = Join-Path $nativeContainmentRoot '.lattice-native-marker-v1'
    $nativeMarkerBytes = [Text.UTF8Encoding]::new($false).GetBytes("lattice.native-marker.v1`n")
    [IO.File]::WriteAllBytes($nativeContainmentMarker, $nativeMarkerBytes)
    $nativeContainment = New-LatticeWindowsNativeContainmentSnapshot `
        -ParentPath $testRoot `
        -RootPath $nativeContainmentRoot `
        -MarkerPath $nativeContainmentMarker
    if (
        -not (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $nativeContainment) -or
        [string]$nativeContainment.parent_identity -cnotmatch '^lattice\.win-file-id\.v1:[0-9a-f]{16}:[0-9a-f]{32}:d$' -or
        [string]$nativeContainment.root_identity -cnotmatch '^lattice\.win-file-id\.v1:[0-9a-f]{16}:[0-9a-f]{32}:d$' -or
        [string]$nativeContainment.marker_identity -cnotmatch '^lattice\.win-file-id\.v1:[0-9a-f]{16}:[0-9a-f]{32}:f$'
    ) {
        throw 'TASK038_WINDOWS_NATIVE_CONTAINMENT_REJECTED'
    }
    [IO.File]::Delete($nativeContainmentMarker)
    [IO.File]::WriteAllBytes($nativeContainmentMarker, $nativeMarkerBytes)
    if (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $nativeContainment) {
        throw 'TASK038_WINDOWS_NATIVE_REPLACEMENT_ACCEPTED'
    }
    $nativeContainment = New-LatticeWindowsNativeContainmentSnapshot `
        -ParentPath $testRoot `
        -RootPath $nativeContainmentRoot `
        -MarkerPath $nativeContainmentMarker
    Assert-LatticeWindowsNativeContainmentSnapshot `
        -Snapshot $nativeContainment `
        -FailureCode 'TASK038_WINDOWS_NATIVE_CONTAINMENT_REJECTED'

    $hostileBatchList = $hostileVariables -join ' '
    [IO.File]::WriteAllText(
        $fakeClient,
        "@echo off`r`nfor %%V in ($hostileBatchList) do if defined %%V exit /b 41`r`nif defined LATTICE_TEST_PRESERVED exit /b 42`r`nif defined LATTICE_WRITER_LEASE_ADMIN_URL exit /b 46`r`nif defined LATTICE_WRITER_LEASE_MIGRATOR_URL exit /b 47`r`nif defined LATTICE_WRITER_LEASE_RUNTIME_URL exit /b 48`r`nif `"%~1`"==`"run`" if not `"%LATTICE_TASK_INGRESS_KIND%`"==`"CHATGPT_SECURE_MCP_TUNNEL`" exit /b 43`r`nif `"%~1`"==`"run`" if `"%LATTICE_TASK_INGRESS_PROFILE_SHA256%`"==`"fixture-hostile-profile`" exit /b 44`r`nif `"%~1`"==`"run`" if `"%LATTICE_TASK_INGRESS_PROFILE_SHA256%`"==`"`" exit /b 45`r`nif `"%~1`"==`"run`" > `"%~dp0ingress-kind.txt`" echo %LATTICE_TASK_INGRESS_KIND%`r`nif `"%~1`"==`"run`" > `"%~dp0ingress-profile.txt`" echo %LATTICE_TASK_INGRESS_PROFILE_SHA256%`r`n> `"%~dp0capture.txt`" echo %*`r`nexit /b 0`r`n",
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllText(
        $failingClient,
        "@echo off`r`nexit /b 9`r`n",
        [Text.UTF8Encoding]::new($false)
    )
    [IO.File]::WriteAllBytes($latticed, [byte[]]@(0))
    [IO.File]::WriteAllBytes($unsafeLatticed, [byte[]]@(0))
    [Environment]::SetEnvironmentVariable('CONTROL_PLANE_API_KEY', $null, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TEST_PRESERVED', 'fixture-lattice-value', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_WRITER_LEASE_ADMIN_URL', 'fixture-admin-dsn', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_WRITER_LEASE_MIGRATOR_URL', 'fixture-migrator-dsn', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_WRITER_LEASE_RUNTIME_URL', 'fixture-runtime-dsn', 'Process')
    foreach ($name in $hostileVariables) {
        [Environment]::SetEnvironmentVariable($name, 'fixture-hostile-value', 'Process')
    }

    & $launcher -Mode Init -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory -TunnelId $tunnelId -LatticedExecutable $latticed
    $profileRoot = [IO.Path]::GetFullPath($profileDirectory)
    $expectedInit = "init --sample sample_mcp_stdio_local --profile lattice-local --profile-dir $profileRoot --tunnel-id $tunnelId --mcp-command '$latticed'"
    Assert-Equal -Expected $expectedInit -Actual ([IO.File]::ReadAllText($capturePath).Trim()) -FailureCode 'TASK038_INIT_ARGUMENTS_REJECTED'

    $profilePath = Join-Path $profileDirectory 'lattice-local.yaml'
    $yamlLatticed = ("'" + $latticed + "'").Replace('\', '\\')
    $profileText = New-Task038TunnelProfileText -TunnelId $tunnelId -YamlLatticed $yamlLatticed
    [IO.File]::WriteAllText($profilePath, $profileText, [Text.UTF8Encoding]::new($false))

    & $launcher -Mode Doctor -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    $expectedDoctor = "doctor --profile lattice-local --profile-dir $profileRoot --explain"
    Assert-Equal -Expected $expectedDoctor -Actual ([IO.File]::ReadAllText($capturePath).Trim()) -FailureCode 'TASK038_DOCTOR_ARGUMENTS_REJECTED'

    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_RUNTIME_KEY_REQUIRED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    }
    [Environment]::SetEnvironmentVariable('CONTROL_PLANE_API_KEY', 'test-runtime-key-not-a-secret', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK_INGRESS_KIND', 'fixture-hostile-kind', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK_INGRESS_PROFILE_SHA256', 'fixture-hostile-profile', 'Process')

    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    }
    $runtimeEnvironment = [ordered]@{
        LATTICE_FULL_CHAIN_RUN_MODE = 'FRESH'
        LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
        LATTICE_DELIVERY_TIMEOUT_SECONDS = '300'
        LATTICE_TASK019_HOST = '127.0.0.1'
        LATTICE_TASK019_PORT = '50468'
        LATTICE_TASK019_RUN_ID = '0123456789abcdef0123456789abcdef'
        LATTICE_TASK019_PASSWORD = 'fixture-database-password'
        LATTICE_STORE_DAEMON_INSTANCE_ID = 'task038-tunnel-0123456789abcdef0123456789abcdef'
        LATTICE_STORE_DAEMON_EPOCH = '1786394996'
        LATTICE_STORE_AUTHORITY_REVISION = '1786394996'
        LATTICE_STORE_OBSERVATION_DIGEST = ('a' * 64)
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = ('b' * 64)
        LATTICE_DELIVERY_LAUNCHER = $latticed
        LATTICE_DELIVERY_LAUNCHER_VERSION = 'codex-cli-test'
        LATTICE_DELIVERY_LAUNCHER_SHA256 = ('c' * 64)
        LATTICE_DELIVERY_SCHEMA_DIR = (Join-Path $testRoot 'schema')
        LATTICE_DELIVERY_CODEX_HOME = $testRoot
        LATTICE_DELIVERY_ROOT = (Join-Path $testRoot 'delivery')
        LATTICE_DELIVERY_GIT_EXE = $latticed
    }
    foreach ($entry in $runtimeEnvironment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
    }

    $commandLine = '      command: "' + $yamlLatticed + '"'
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
            & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
        }
    }
    [IO.File]::WriteAllText($profilePath, $profileText, [Text.UTF8Encoding]::new($false))

    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_CLIENT_IDENTITY_REJECTED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    }
    $profileSha256 = (Get-FileHash -LiteralPath $profilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $expectedProfileCommitment = @(
        'lattice.task-ingress-profile.v2',
        'profile_name=lattice-local',
        ('profile_sha256=' + $profileSha256),
        ('tunnel_id=' + $tunnelId),
        'channel=main',
        ('tunnel_client_sha256=' + (Get-FileHash -LiteralPath $fakeClient -Algorithm SHA256).Hash.ToLowerInvariant()),
        ('latticed_sha256=' + (Get-FileHash -LiteralPath $latticed -Algorithm SHA256).Hash.ToLowerInvariant())
    ) -join "`n"
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
    $launcherTokens = $null
    $launcherParseErrors = $null
    $launcherAst = [Management.Automation.Language.Parser]::ParseFile(
        $launcher,
        [ref]$launcherTokens,
        [ref]$launcherParseErrors
    )
    if ($launcherParseErrors.Count -ne 0) {
        throw 'TASK038_TUNNEL_LAUNCHER_AST_REJECTED'
    }
    $launcherFunctions = @{}
    foreach ($functionAst in @($launcherAst.FindAll({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst]
    }, $true))) {
        $launcherFunctions[[string]$functionAst.Name] = $functionAst
    }
    foreach ($requiredFunction in @(
        'Assert-ManagedSecretFreeText',
        'Get-ByteArraySha256',
        'Initialize-Task038Utf8DiagnosticType',
        'Get-ManagedTunnelLogDiagnostic',
        'ConvertFrom-ManagedTunnelJsonLines',
        'Write-ManagedTunnelLogDiagnosticEvidence',
        'Get-ManagedTunnelLifecycleEvidence',
        'Test-SameProcessIdSet',
        'Write-ManagedSessionObservation',
        'Write-ManagedJsonEvidence',
        'Remove-ManagedCodexAuthCredential',
        'Resolve-RequiredLeafPath',
        'Get-FileSha256',
        'Assert-ManagedCanonicalRestartReceipt'
    )) {
        if (-not $launcherFunctions.ContainsKey($requiredFunction)) {
            throw 'TASK038_TUNNEL_DURABLE_EVIDENCE_SOURCE_REJECTED'
        }
    }
    . ([scriptblock]::Create($launcherFunctions['Assert-ManagedSecretFreeText'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Get-ByteArraySha256'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Initialize-Task038Utf8DiagnosticType'].Extent.Text))
    Initialize-Task038Utf8DiagnosticType
    . ([scriptblock]::Create($launcherFunctions['Get-ManagedTunnelLogDiagnostic'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['ConvertFrom-ManagedTunnelJsonLines'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Write-ManagedTunnelLogDiagnosticEvidence'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Get-ManagedTunnelLifecycleEvidence'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Test-SameProcessIdSet'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Assert-NoReparsePath'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Get-CanonicalPath'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Test-ExactPath'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Write-ManagedJsonEvidence'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Remove-ManagedCodexAuthCredential'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Resolve-RequiredLeafPath'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Get-FileSha256'].Extent.Text))
    . ([scriptblock]::Create($launcherFunctions['Assert-ManagedCanonicalRestartReceipt'].Extent.Text))
    $script:ManagedSecretValues = [Collections.Generic.List[string]]::new()
    $receiptBinarySha = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    $canonicalAcceptanceReceipt = Join-Path $testRoot 'four-tool-acceptance-final.json'
    $canonicalAcceptance = [ordered]@{
        schema_version = 'lattice.task038.four-tool-acceptance.v1'
        status = 'PASS'
        mode = 'FULL'
        failure_code = $null
        completed_stages = @(
            'binary_identity',
            'postgres_before',
            'four_tool_discovery',
            'read_only_status',
            'bounded_submit',
            'postgres_after_submit',
            'postgres_restart_controller',
            'cross_session_durable_status',
            'postgres_durable_replay',
            'process_cleanup'
        )
        binary_sha256 = $receiptBinarySha
        source_commit = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
        discovered_tools = @(
            'lattice_delivery_run',
            'lattice_delivery_status',
            'lattice_task_submit',
            'lattice_task_status'
        )
        delivery_run_invoked = $false
        hermes_run_invoked = $false
        tool_level_is_error_checked = $true
        cross_session_status_checked = $true
        postgres_evidence_checked = $true
        postgres_restart_required = $true
        postgres_restart_observed = $true
        process_cleanup_checked = $true
        process_cleanup_status = 'PASS'
        process_cleanup_basis = 'WINDOWS_JOB_OBJECT_NATIVE_HANDLE'
        exact_pass_marker = 'TASK038_FOUR_TOOL_ACCEPTANCE=PASS'
        exact_pass_marker_count = 1
        skip_marker_count = 0
        not_run = $false
    }
    [IO.File]::WriteAllText(
        $canonicalAcceptanceReceipt,
        ($canonicalAcceptance | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    $validatedAcceptance = Assert-ManagedCanonicalRestartReceipt `
        -ReceiptPath $canonicalAcceptanceReceipt `
        -ExpectedReceiptSha256 ((Get-FileHash -LiteralPath $canonicalAcceptanceReceipt -Algorithm SHA256).Hash.ToLowerInvariant()) `
        -ExpectedLatticedSha256 $receiptBinarySha
    Assert-Equal `
        -Expected ((Get-FileHash -LiteralPath $canonicalAcceptanceReceipt -Algorithm SHA256).Hash.ToLowerInvariant()) `
        -Actual ([string]$validatedAcceptance.Sha256) `
        -FailureCode 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED'
    $canonicalAcceptance.skip_marker_count = 1
    [IO.File]::WriteAllText(
        $canonicalAcceptanceReceipt,
        ($canonicalAcceptance | ConvertTo-Json -Depth 8),
        [Text.UTF8Encoding]::new($false)
    )
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED' -Action {
        Assert-ManagedCanonicalRestartReceipt `
            -ReceiptPath $canonicalAcceptanceReceipt `
            -ExpectedReceiptSha256 ((Get-FileHash -LiteralPath $canonicalAcceptanceReceipt -Algorithm SHA256).Hash.ToLowerInvariant()) `
            -ExpectedLatticedSha256 $receiptBinarySha
    }
    if (
        -not (Test-SameProcessIdSet -Before @() -After @()) -or
        -not (Test-SameProcessIdSet -Before @(17, 3) -After @(3, 17)) -or
        (Test-SameProcessIdSet -Before @() -After @(17)) -or
        (Test-SameProcessIdSet -Before @(17) -After @())
    ) {
        throw 'TASK038_TUNNEL_PROCESS_SET_COMPARISON_REJECTED'
    }
    $lifecycleLog = Join-Path $testRoot 'managed-lifecycle.jsonl'
    $lifecycleRows = @(
        [ordered]@{
            msg = 'stdio MCP command started'
            component = 'mcpclient'
            transport = 'stdio'
            command = "'C:\controlled\latticed.exe'"
            client_instance_id = '0123456789abcdef0123456789abcdef'
            tunnel_id = 'tunnel_0123456789abcdef0123456789abcdef'
        },
        [ordered]@{
            msg = 'command response deadline reached; dropping without posting a response'
            component = 'dispatcher'
            request_id = 'request-0123456789abcdef'
            client_instance_id = '0123456789abcdef0123456789abcdef'
            tunnel_id = 'tunnel_0123456789abcdef0123456789abcdef'
        },
        [ordered]@{
            msg = 'MCP connection TTL reached; stopping response forwarding'
            component = 'dispatcher'
            request_id = 'request-0123456789abcdef'
            client_instance_id = '0123456789abcdef0123456789abcdef'
            tunnel_id = 'tunnel_0123456789abcdef0123456789abcdef'
        }
    )
    $lifecycleText = (@($lifecycleRows | ForEach-Object {
        $_ | ConvertTo-Json -Depth 5 -Compress
    }) -join "`n") + "`n"
    [IO.File]::WriteAllText($lifecycleLog, $lifecycleText, [Text.UTF8Encoding]::new($false))
    $lifecycleCanonicalLog = Join-Path $testRoot 'managed-lifecycle.canonical.jsonl'
    $liveLogWriter = [IO.FileStream]::new(
        $lifecycleLog,
        [IO.FileMode]::Open,
        [IO.FileAccess]::ReadWrite,
        [IO.FileShare]::ReadWrite
    )
    try {
        $lifecycleDiagnostic = Get-ManagedTunnelLogDiagnostic `
            -LogPath $lifecycleLog `
            -DiagnosticPath $lifecycleCanonicalLog `
            -DiagnosticRoot $testRoot
    }
    finally {
        $liveLogWriter.Dispose()
    }
    $lifecycleEvidence = Get-ManagedTunnelLifecycleEvidence -LogDiagnostic $lifecycleDiagnostic
    if (
        [string]$lifecycleEvidence.schema -cne 'lattice.task038.tunnel-lifecycle-evidence.v2' -or
        [string]$lifecycleEvidence.raw_log_sha256 -cne (Get-FileHash -LiteralPath $lifecycleLog -Algorithm SHA256).Hash.ToLowerInvariant() -or
        [long]$lifecycleEvidence.raw_log_byte_count -ne ([IO.FileInfo]$lifecycleLog).Length -or
        -not [bool]$lifecycleEvidence.raw_log_strict_utf8 -or
        [int]$lifecycleEvidence.utf8_replacement_count -ne 0 -or
        [string]$lifecycleEvidence.canonical_log_sha256 -cne (Get-FileHash -LiteralPath $lifecycleCanonicalLog -Algorithm SHA256).Hash.ToLowerInvariant() -or
        [int]$lifecycleEvidence.record_count -ne 3 -or
        @($lifecycleEvidence.client_instance_ids).Count -ne 1 -or
        @($lifecycleEvidence.tunnel_ids).Count -ne 1 -or
        @($lifecycleEvidence.request_ids).Count -ne 1 -or
        [int]$lifecycleEvidence.message_counts.'stdio MCP command started' -ne 1 -or
        [int]$lifecycleEvidence.message_counts.'command response deadline reached; dropping without posting a response' -ne 1 -or
        [int]$lifecycleEvidence.message_counts.'MCP connection TTL reached; stopping response forwarding' -ne 1
    ) {
        throw 'TASK038_TUNNEL_DURABLE_LIFECYCLE_EVIDENCE_REJECTED'
    }

    $invalidUtf8Log = Join-Path $testRoot 'managed-invalid-utf8.jsonl'
    $invalidPrefix = [Text.UTF8Encoding]::new($false).GetBytes('{"msg":"health server listening","component":"health","detail":"')
    $invalidSuffix = [Text.UTF8Encoding]::new($false).GetBytes('("}' + "`n")
    $invalidBytes = [byte[]]::new($invalidPrefix.Length + 1 + $invalidSuffix.Length)
    [Array]::Copy($invalidPrefix, 0, $invalidBytes, 0, $invalidPrefix.Length)
    $invalidBytes[$invalidPrefix.Length] = 0xc3
    [Array]::Copy($invalidSuffix, 0, $invalidBytes, $invalidPrefix.Length + 1, $invalidSuffix.Length)
    [IO.File]::WriteAllBytes($invalidUtf8Log, $invalidBytes)
    $invalidCanonicalLog = Join-Path $testRoot 'managed-invalid-utf8.canonical.jsonl'
    $invalidDiagnostic = Get-ManagedTunnelLogDiagnostic `
        -LogPath $invalidUtf8Log `
        -DiagnosticPath $invalidCanonicalLog `
        -DiagnosticRoot $testRoot
    $invalidLifecycle = Get-ManagedTunnelLifecycleEvidence -LogDiagnostic $invalidDiagnostic
    if (
        [bool]$invalidDiagnostic.strict_utf8 -or
        [int]$invalidDiagnostic.replacement_count -ne 1 -or
        [int]$invalidDiagnostic.first_invalid_byte_offset -ne $invalidPrefix.Length -or
        [string]$invalidDiagnostic.raw_sha256 -cne (Get-ByteArraySha256 -Bytes $invalidBytes) -or
        [long]$invalidDiagnostic.raw_byte_count -ne $invalidBytes.LongLength -or
        [int]$invalidLifecycle.record_count -ne 1 -or
        [int]$invalidLifecycle.utf8_replacement_count -ne 1
    ) {
        throw 'TASK038_TUNNEL_LOG_CANONICALIZATION_REJECTED'
    }
    try { $null = [Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($invalidCanonicalLog)) }
    catch { throw 'TASK038_TUNNEL_LOG_CANONICALIZATION_REJECTED' }

    $validReplacementLog = Join-Path $testRoot 'managed-valid-replacement-character.jsonl'
    [IO.File]::WriteAllText(
        $validReplacementLog,
        ('{"msg":"health server listening","component":"health","detail":"' + [char]0xfffd + '"}' + "`n"),
        [Text.UTF8Encoding]::new($false)
    )
    $validReplacementDiagnostic = Get-ManagedTunnelLogDiagnostic -LogPath $validReplacementLog
    if (
        -not [bool]$validReplacementDiagnostic.strict_utf8 -or
        [int]$validReplacementDiagnostic.replacement_count -ne 0 -or
        [int]$validReplacementDiagnostic.first_invalid_byte_offset -ne -1
    ) {
        throw 'TASK038_TUNNEL_LOG_CANONICALIZATION_REJECTED'
    }

    $invalidJsonLog = Join-Path $testRoot 'managed-invalid-json.jsonl'
    $validJsonBytes = [Text.UTF8Encoding]::new($false).GetBytes('{"msg":"health server listening","component":"health"}' + "`n")
    $invalidJsonBytes = [byte[]]::new($validJsonBytes.Length + 1)
    $invalidJsonBytes[0] = 0xff
    [Array]::Copy($validJsonBytes, 0, $invalidJsonBytes, 1, $validJsonBytes.Length)
    [IO.File]::WriteAllBytes($invalidJsonLog, $invalidJsonBytes)
    $invalidJsonDiagnostic = Get-ManagedTunnelLogDiagnostic -LogPath $invalidJsonLog
    $invalidJsonSidecar = Join-Path $testRoot 'session-1-tunnel-log-diagnostic.json'
    Write-ManagedTunnelLogDiagnosticEvidence `
        -Session 1 `
        -EvidenceRoot $testRoot `
        -LogDiagnostic $invalidJsonDiagnostic
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_DURABLE_LIFECYCLE_EVIDENCE_REJECTED' -Action {
        Get-ManagedTunnelLifecycleEvidence -LogDiagnostic $invalidJsonDiagnostic
    }
    $invalidJsonSidecarValue = Get-Content -LiteralPath $invalidJsonSidecar -Raw -Encoding UTF8 | ConvertFrom-Json
    if (
        [string]$invalidJsonSidecarValue.raw_log_sha256 -cne [string]$invalidJsonDiagnostic.raw_sha256 -or
        [long]$invalidJsonSidecarValue.raw_log_byte_count -ne [long]$invalidJsonDiagnostic.raw_byte_count -or
        [int]$invalidJsonSidecarValue.utf8_replacement_count -ne [int]$invalidJsonDiagnostic.replacement_count -or
        [int]$invalidJsonSidecarValue.first_invalid_byte_offset -ne [int]$invalidJsonDiagnostic.first_invalid_byte_offset
    ) {
        throw 'TASK038_TUNNEL_DURABLE_LOG_EVIDENCE_REJECTED'
    }

    foreach ($invalidJsonText in @(
        "42`n",
        "[]`n",
        "null`n",
        '{"msg":"health server listening"}',
        "{}`n`n"
    )) {
        $invalidShapePath = Join-Path $testRoot ('managed-invalid-shape-' + [Guid]::NewGuid().ToString('N') + '.jsonl')
        [IO.File]::WriteAllText($invalidShapePath, $invalidJsonText, [Text.UTF8Encoding]::new($false))
        $invalidShapeDiagnostic = Get-ManagedTunnelLogDiagnostic -LogPath $invalidShapePath
        Assert-FailsWith -FailureCode 'TASK038_TUNNEL_DURABLE_LIFECYCLE_EVIDENCE_REJECTED' -Action {
            Get-ManagedTunnelLifecycleEvidence -LogDiagnostic $invalidShapeDiagnostic
        }
    }

    $durableRawRoot = Join-Path $testRoot 'disposable-log-root'
    $durableEvidenceRoot = Join-Path $testRoot 'durable-log-evidence'
    [IO.Directory]::CreateDirectory($durableRawRoot) | Out-Null
    [IO.Directory]::CreateDirectory($durableEvidenceRoot) | Out-Null
    $durableRawLog = Join-Path $durableRawRoot 'tunnel-client.jsonl'
    $durableCanonicalLog = Join-Path $durableEvidenceRoot 'session-1-tunnel-client.canonical.jsonl'
    [IO.File]::WriteAllText($durableRawLog, $lifecycleText, [Text.UTF8Encoding]::new($false))
    $durableDiagnostic = Get-ManagedTunnelLogDiagnostic `
        -LogPath $durableRawLog `
        -DiagnosticPath $durableCanonicalLog `
        -DiagnosticRoot $durableEvidenceRoot
    Remove-Item -LiteralPath $durableRawRoot -Recurse -Force
    if (
        -not (Test-Path -LiteralPath $durableCanonicalLog -PathType Leaf) -or
        (Get-FileHash -LiteralPath $durableCanonicalLog -Algorithm SHA256).Hash.ToLowerInvariant() -cne
            [string]$durableDiagnostic.canonical_sha256
    ) {
        throw 'TASK038_TUNNEL_DURABLE_LOG_EVIDENCE_REJECTED'
    }

    $script:ManagedSecretValues.Add('fixture-secret-value')
    [IO.File]::AppendAllText(
        $lifecycleLog,
        '{"msg":"health server listening","token":"fixture-secret-value"}' + "`n",
        [Text.UTF8Encoding]::new($false)
    )
    $secretCanonicalLog = Join-Path $testRoot 'managed-secret.canonical.jsonl'
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_MANAGED_OUTPUT_SECRET_REJECTED' -Action {
        Get-ManagedTunnelLogDiagnostic `
            -LogPath $lifecycleLog `
            -DiagnosticPath $secretCanonicalLog `
            -DiagnosticRoot $testRoot
    }
    if (Test-Path -LiteralPath $secretCanonicalLog) {
        throw 'TASK038_TUNNEL_MANAGED_OUTPUT_SECRET_REJECTED'
    }
    $script:ManagedSecretValues.Clear()

    $exclusiveEvidence = Join-Path $testRoot 'exclusive-evidence.json'
    Write-ManagedJsonEvidence -Path $exclusiveEvidence -Value ([ordered]@{ kind = 'observation' })
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED' -Action {
        Write-ManagedJsonEvidence -Path $exclusiveEvidence -Value ([ordered]@{ kind = 'replacement' })
    }
    if ((Get-Content -LiteralPath $exclusiveEvidence -Raw -Encoding UTF8) -cne "{`"kind`":`"observation`"}`n") {
        throw 'TASK038_TUNNEL_OBSERVATION_EXCLUSIVE_CREATE_REJECTED'
    }

    $cleanupRuntimeId = '0123456789abcdef0123456789abcdef'
    $cleanupCluster = Join-Path $testRoot 'cleanup-cluster'
    $cleanupHome = Join-Path $cleanupCluster 'task038-tunnel-codex-home'
    [IO.Directory]::CreateDirectory($cleanupHome) | Out-Null
    [IO.File]::WriteAllText((Join-Path $cleanupHome '.lattice-codex-home-v1'), "lattice.codex-home.v1`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $cleanupHome '.lattice-task038-tunnel-owner-v1'), ('lattice.task038-tunnel-home.v1:' + $cleanupRuntimeId + "`n"), [Text.UTF8Encoding]::new($false))
    $cleanupAuth = Join-Path $cleanupHome 'auth.json'
    [IO.File]::WriteAllText($cleanupAuth, "fixture-auth`n", [Text.UTF8Encoding]::new($false))
    Remove-ManagedCodexAuthCredential -ClusterRoot $cleanupCluster -CodexHome $cleanupHome -RuntimeId $cleanupRuntimeId
    if (Test-Path -LiteralPath $cleanupAuth) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }

    $outsideHome = Join-Path $testRoot 'outside-credential-home'
    [IO.Directory]::CreateDirectory($outsideHome) | Out-Null
    [IO.File]::WriteAllText((Join-Path $outsideHome '.lattice-codex-home-v1'), "lattice.codex-home.v1`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $outsideHome '.lattice-task038-tunnel-owner-v1'), ('lattice.task038-tunnel-home.v1:' + $cleanupRuntimeId + "`n"), [Text.UTF8Encoding]::new($false))
    $outsideAuth = Join-Path $outsideHome 'auth.json'
    [IO.File]::WriteAllText($outsideAuth, "outside-sentinel`n", [Text.UTF8Encoding]::new($false))
    if (Test-Path -LiteralPath $cleanupHome) { Remove-Item -LiteralPath $cleanupHome -Recurse -Force }
    New-Item -ItemType Junction -Path $cleanupHome -Target $outsideHome | Out-Null
    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED' -Action {
        Remove-ManagedCodexAuthCredential -ClusterRoot $cleanupCluster -CodexHome $cleanupHome -RuntimeId $cleanupRuntimeId
    }
    if ((Get-Content -LiteralPath $outsideAuth -Raw -Encoding UTF8) -cne "outside-sentinel`n") {
        throw 'TASK038_TUNNEL_CODEX_AUTH_SOURCE_CHANGED'
    }
    if ($launcherText -match '(?im)^\s*\[string\]\s*\$.*(?:ApiKey|Credential|Secret)') {
        throw 'TASK038_CREDENTIAL_SURFACE_REJECTED'
    }
    foreach ($requiredClosure in @(
        'profile_sha256=',
        'lattice.task-ingress-profile.v2',
        'api_key: "env:CONTROL_PLANE_API_KEY"',
        'latticed_sha256=',
        '7d3c7d492ce84b52835e11865a835a8a5bcd4a669dee84e169aa11b314dc952a',
        'TASK038_TUNNEL_CLIENT_IDENTITY_REJECTED',
        'session-$session-observation.json',
        'session-$session-tunnel-lifecycle.json',
        'TASK038_TUNNEL_SESSION_TUNNEL_PROCESS_SET_CHANGED',
        'TASK038_TUNNEL_SESSION_LATTICED_PROCESS_SET_CHANGED',
        'TASK038_TUNNEL_SESSION_PID_FILE_RETAINED',
        'TASK038_TUNNEL_SESSION_HEALTH_FILE_RETAINED',
        'TASK038_TUNNEL_SESSION_OWNER_PID_RETAINED',
        'TASK038_TUNNEL_SESSION_STOP_SIGNAL_RETAINED',
        'TASK038_TUNNEL_SESSION_WORKING_DIRECTORY_NOT_EMPTY',
        'TASK038_TUNNEL_SESSION_LOG_MISSING',
        'TASK038_TUNNEL_SESSION_LOG_PATH_REJECTED',
        'TASK038_TUNNEL_SESSION_LOG_EMPTY',
        'TASK038_TUNNEL_SESSION_LOG_TOO_LARGE',
        'TASK038_TUNNEL_SESSION_LOG_READ_REJECTED',
        'TASK038_TUNNEL_SESSION_LOG_SIZE_CHANGED',
        'TASK038_TUNNEL_SESSION_LOG_BOM_REJECTED',
        'TASK038_TUNNEL_SESSION_LOG_CANONICALIZATION_REJECTED',
        'TASK038_TUNNEL_DIAGNOSTIC_COPY_REJECTED',
        'TASK038_TUNNEL_SESSION_LOG_JSON_REJECTED',
        'raw_log_sha256',
        'utf8_replacement_count',
        'canonical_log_sha256',
        'TASK038_TUNNEL_SESSION_CLEANUP_CONTAINMENT_REJECTED',
        'lattice.task038.four-tool-acceptance.v1',
        'TASK038_FOUR_TOOL_ACCEPTANCE=PASS',
        'skip_marker_count',
        'New-LatticeWindowsNativeContainmentSnapshot',
        '[IO.File]::Delete($ownedSessionFile.Path)',
        'ReparsePoint'
    )) {
        if ($launcherText.IndexOf($requiredClosure, [StringComparison]::Ordinal) -lt 0) {
            throw 'TASK038_TUNNEL_PROFILE_STATIC_CLOSURE_REJECTED'
        }
    }
    $ownedTypeMatches = @([regex]::Matches(
        $launcherText,
        "Add-Type -TypeDefinition @'\r?\n(?<source>.*?)\r?\n'@ -Language CSharp",
        [Text.RegularExpressions.RegexOptions]::Singleline
    ) | Where-Object { $_.Groups['source'].Value.Contains('OwnedProcessTree') })
    if ($ownedTypeMatches.Count -ne 1) {
        throw 'TASK038_TUNNEL_JOB_SOURCE_REJECTED'
    }
    Invoke-Task038OwnedProcessFaultTest -OwnedTypeSource $ownedTypeMatches[0].Groups['source'].Value

    $task019HarnessText = [IO.File]::ReadAllText($task019Harness)
    foreach ($cleanupClosure in @(
        'Task038CanonicalAcceptanceReceipt',
        'Task038CanonicalAcceptanceReceiptSha256',
        'TASK019_CLEANUP_CONTAINMENT_REJECTED',
        'Remove-HarnessOutputFiles -Root $clusterRoot -ContainmentSnapshot $cleanupContainment',
        '-CanonicalAcceptanceReceipt $Task038CanonicalAcceptanceReceipt',
        '-CanonicalAcceptanceReceiptSha256 $Task038CanonicalAcceptanceReceiptSha256'
    )) {
        if ($task019HarnessText.IndexOf($cleanupClosure, [StringComparison]::Ordinal) -lt 0) {
            throw 'TASK038_TUNNEL_POSTGRES_CLEANUP_STATIC_CLOSURE_REJECTED'
        }
    }
    $watchdogMatches = @([regex]::Matches(
        $task019HarnessText,
        '\$source = \$nativeIdentityHelperSource \+ \[Environment\]::NewLine \+ @''\r?\n(?<source>.*?)\r?\n''@',
        [Text.RegularExpressions.RegexOptions]::Singleline
    ) | Where-Object { $_.Groups['source'].Value.Contains('TASK038_WATCHDOG_PARENT_PID') })
    if ($watchdogMatches.Count -ne 1) {
        throw 'TASK038_TUNNEL_WATCHDOG_SOURCE_REJECTED'
    }
    $watchdogTokens = $null
    $watchdogErrors = $null
    [void][Management.Automation.Language.Parser]::ParseInput(
        $watchdogMatches[0].Groups['source'].Value,
        [ref]$watchdogTokens,
        [ref]$watchdogErrors
    )
    if ($watchdogErrors.Count -ne 0) {
        throw 'TASK038_TUNNEL_WATCHDOG_AST_REJECTED'
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
