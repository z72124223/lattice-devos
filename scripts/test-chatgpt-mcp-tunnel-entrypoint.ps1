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
        "@echo off`r`nfor %%V in ($hostileBatchList) do if defined %%V exit /b 41`r`nif not `"%LATTICE_TEST_PRESERVED%`"==`"fixture-lattice-value`" exit /b 42`r`nif `"%~1`"==`"run`" if not `"%LATTICE_TASK_INGRESS_KIND%`"==`"CHATGPT_SECURE_MCP_TUNNEL`" exit /b 43`r`nif `"%~1`"==`"run`" if `"%LATTICE_TASK_INGRESS_PROFILE_SHA256%`"==`"fixture-hostile-profile`" exit /b 44`r`nif `"%~1`"==`"run`" if `"%LATTICE_TASK_INGRESS_PROFILE_SHA256%`"==`"`" exit /b 45`r`nif `"%~1`"==`"run`" > `"%~dp0ingress-kind.txt`" echo %LATTICE_TASK_INGRESS_KIND%`r`nif `"%~1`"==`"run`" > `"%~dp0ingress-profile.txt`" echo %LATTICE_TASK_INGRESS_PROFILE_SHA256%`r`n> `"%~dp0capture.txt`" echo %*`r`nexit /b 0`r`n",
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
    [Environment]::SetEnvironmentVariable('CONTROL_PLANE_API_KEY', 'test-runtime-key-not-a-secret', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK_INGRESS_KIND', 'fixture-hostile-kind', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK_INGRESS_PROFILE_SHA256', 'fixture-hostile-profile', 'Process')

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
            & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
        }
    }
    [IO.File]::WriteAllText($profilePath, $profileText, [Text.UTF8Encoding]::new($false))

    & $launcher -Mode Run -TunnelClientExecutable $fakeClient -ProfileDirectory $profileDirectory
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
        ('tunnel_client_sha256=' + (Get-FileHash -LiteralPath $fakeClient -Algorithm SHA256).Hash.ToLowerInvariant()),
        ('latticed_sha256=' + (Get-FileHash -LiteralPath $latticed -Algorithm SHA256).Hash.ToLowerInvariant())
    ) -join "`n"
    Assert-Equal -Expected (Get-StringSha256 -Value $expectedProfileCommitment) -Actual ([IO.File]::ReadAllText($ingressProfilePath).Trim()) -FailureCode 'TASK038_RUN_INGRESS_PROFILE_REJECTED'
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
        'ReparsePoint'
    )) {
        if ($launcherText.IndexOf($requiredClosure, [StringComparison]::Ordinal) -lt 0) {
            throw 'TASK038_TUNNEL_PROFILE_STATIC_CLOSURE_REJECTED'
        }
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
