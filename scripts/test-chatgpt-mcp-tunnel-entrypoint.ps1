[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$launcher = Join-Path $PSScriptRoot 'start-chatgpt-mcp-tunnel.ps1'
$task037Verifier = Join-Path $PSScriptRoot 'run-task037-full-chain-verification.ps1'
if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
    throw 'TASK038_TUNNEL_LAUNCHER_MISSING'
}
if (-not (Test-Path -LiteralPath $task037Verifier -PathType Leaf)) {
    throw 'TASK038_TASK037_VERIFIER_MISSING'
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

$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testRoot = Join-Path $tempBase ('lattice-task038-tunnel-test-' + [Guid]::NewGuid().ToString('N'))
$testRoot = [IO.Path]::GetFullPath($testRoot)
if (-not $testRoot.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'TASK038_TEST_ROOT_REJECTED'
}
[IO.Directory]::CreateDirectory($testRoot) | Out-Null

$capturePath = Join-Path $testRoot 'capture.txt'
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
    $hostileBatchList = $hostileVariables -join ' '
    [IO.File]::WriteAllText(
        $fakeClient,
        "@echo off`r`nfor %%V in ($hostileBatchList) do if defined %%V exit /b 41`r`nif not `"%LATTICE_TEST_PRESERVED%`"==`"fixture-lattice-value`" exit /b 42`r`n> `"%~dp0capture.txt`" echo %*`r`nexit /b 0`r`n",
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
    foreach ($name in $hostileVariables) {
        [Environment]::SetEnvironmentVariable($name, 'fixture-hostile-value', 'Process')
    }

    & $launcher -Mode Init -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory -TunnelId $tunnelId -LatticedExecutable $latticed
    $profileRoot = [IO.Path]::GetFullPath($profileDirectory)
    $expectedInit = "init --sample sample_mcp_stdio_local --profile lattice-local --profile-dir $profileRoot --tunnel-id $tunnelId --mcp-command '$latticed'"
    Assert-Equal -Expected $expectedInit -Actual ([IO.File]::ReadAllText($capturePath).Trim()) -FailureCode 'TASK038_INIT_ARGUMENTS_REJECTED'

    & $launcher -Mode Doctor -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    $expectedDoctor = "doctor --profile lattice-local --profile-dir $profileRoot --explain"
    Assert-Equal -Expected $expectedDoctor -Actual ([IO.File]::ReadAllText($capturePath).Trim()) -FailureCode 'TASK038_DOCTOR_ARGUMENTS_REJECTED'

    Assert-FailsWith -FailureCode 'TASK038_TUNNEL_RUNTIME_KEY_REQUIRED' -Action {
        & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    }
    [Environment]::SetEnvironmentVariable('CONTROL_PLANE_API_KEY', 'test-runtime-key-not-a-secret', 'Process')
    & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
    $expectedRun = "run --profile lattice-local --profile-dir $profileRoot"
    Assert-Equal -Expected $expectedRun -Actual ([IO.File]::ReadAllText($capturePath).Trim()) -FailureCode 'TASK038_RUN_ARGUMENTS_REJECTED'

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
    $task037VerifierText = [IO.File]::ReadAllText($task037Verifier)
    if ($task037VerifierText.IndexOf('$taskBinding', [StringComparison]::Ordinal) -ge 0 -or
        $task037VerifierText -notmatch '(?m)^\s*arguments\s*=\s*\[ordered\]@\{\}\s*$') {
        throw 'TASK038_TASK037_VERIFIER_ARGUMENTS_REJECTED'
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
