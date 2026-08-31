#requires -Version 7.0

<#
.SYNOPSIS
Runs the bounded Phase 4 managed-foreman live acceptance.

.DESCRIPTION
The live mode creates a disposable Git repository, an isolated loopback Control
instance, and an owned PostgreSQL 17 cluster. It uses newline-delimited MCP over
one long-lived latticed process for each restart generation. The first process
records the formal foreman checkpoint, the second submits and observes one real
Codex task, and the third proves restart replay without another worker attempt.

The harness-created Git repository has no remote. Its Git control snapshots prove
only HEAD, status, remotes, local config, and refs; external effects are not
measured. Real Codex mode fails closed before dispatch until credential read
isolation has independent evidence. StaticSelfTestOnly performs no database,
Control, Git, latticed, or Codex runtime action.

ScriptedActiveRestart reuses the same disposable Git, Control, PostgreSQL, and
formal runtime setup, but runs a pinned scripted App Server that retains one
exact in-progress turn across a hard foreman process restart. It is not real
Codex evidence.

Wsl2LinuxLive keeps both the source and exact managed worktree under the Ubuntu
Linux home. It submits under a fresh DISABLED process, materializes and replays
the production WSL execution descriptor and worktree, then starts a fresh ACTIVE
process with only the final descriptor. Wsl2TechnicalPreflightOnly stops before
ACTIVE with durable provider effects equal to zero and never reads account state.
#>

[CmdletBinding()]
param(
    [string]$BinaryPath,

    [string]$CodexExecutablePath = (
        'C:\Users\f7212\Documents\Codex\2026-07-29\lattice-worktrees\' +
        'codex-app-server-repair\target\codex-official\0.146.0\' +
        'node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe'
    ),

    [ValidateRange(15, 300)]
    [int]$ProcessTimeoutSeconds = 120,

    # The product owns a 900-second task deadline shared by worker attempts
    # and the independent reviewer.  The harness adds only a 60-second
    # observation/cleanup grace and must never terminate a still-authorized
    # task inside that product window.
    [ValidateRange(900, 960)]
    [int]$AcceptanceTimeoutSeconds = 960,

    [switch]$SkipBuild,

    [switch]$KeepArtifacts,

    [switch]$StaticSelfTestOnly,

    [switch]$StaticReceiptPersistenceSelfTestOnly,

    [switch]$StaticMcpPollingSelfTestOnly,

    [switch]$ScriptedActiveRestart,

    [switch]$Wsl2LinuxLive,

    [switch]$Wsl2TechnicalPreflightOnly,

    [ValidatePattern('\A/home/[a-z_][a-z0-9_-]{0,31}/[A-Za-z0-9._/-]{1,900}\z')]
    [string]$Wsl2TaskRoot = '/home/zk/lattice-phase4-wsl2-acceptance-20260828'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:WindowsOemCodePage = (Get-Culture).TextInfo.OEMCodePage
if ($script:WindowsOemCodePage -lt 1 -or $script:WindowsOemCodePage -gt 65535) {
    throw 'PHASE4_WINDOWS_OEM_CODE_PAGE_REJECTED'
}
$script:RepositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$script:PostgresBin = if (
    $StaticSelfTestOnly -or
    $StaticReceiptPersistenceSelfTestOnly -or
    $StaticMcpPollingSelfTestOnly
) {
    Join-Path ([IO.Path]::GetTempPath()) 'lattice-phase4-postgresql-selftest'
}
elseif ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
    'C:\Program Files\PostgreSQL\17\bin'
}
else {
    throw 'PHASE4_WINDOWS_LIVE_REQUIRED'
}
$script:InitDb = Join-Path $script:PostgresBin 'initdb.exe'
$script:PgCtl = Join-Path $script:PostgresBin 'pg_ctl.exe'
$script:Postgres = Join-Path $script:PostgresBin 'postgres.exe'
$script:Psql = Join-Path $script:PostgresBin 'psql.exe'
$script:Netstat = Join-Path `
    ([Environment]::GetFolderPath([Environment+SpecialFolder]::System)) 'netstat.exe'
$script:ControlServer = Join-Path $script:RepositoryRoot 'apps\lattice-control\src\server.mjs'
$script:ManagedBridge = Join-Path `
    $script:RepositoryRoot 'apps\lattice-control\src\managed-codex-worker-bridge.mjs'
$script:ManagedWorktreeBridge = Join-Path `
    $script:RepositoryRoot 'apps\lattice-control\src\managed-worktree-bridge.mjs'
$script:Wsl2Materializer = Join-Path `
    $script:RepositoryRoot 'scripts\materialize-phase4-wsl2-live-environment.mjs'
$script:Wsl2PreflightBridge = Join-Path `
    $script:RepositoryRoot 'apps\lattice-control\src\wsl2-execution-preflight-bridge.mjs'
$script:Wsl2ProviderSubtreeReconciler = Join-Path `
    $script:RepositoryRoot 'apps\lattice-control\src\wsl2-provider-subtree-reconcile.mjs'
$script:Wsl = Join-Path `
    ([Environment]::GetFolderPath([Environment+SpecialFolder]::System)) 'wsl.exe'
$script:Wsl2Distribution = 'Ubuntu'
$script:Wsl2LinuxLiveEnabled = [bool]$Wsl2LinuxLive
$script:ExpectedCodexSha256 = 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb'
$script:ExpectedCodexLauncherVersion = 'codex-cli 0.146.0'
$script:ExpectedWsl2CodexConfigSha256 =
    '63dd522b51703a8545980eafadaac4ad864d6fe3bf303786f3704e115ca799cd'
$script:Wsl2SupervisorSource = Join-Path `
    $script:RepositoryRoot 'apps\lattice-control\src\wsl2-codex-supervisor.mjs'
$script:ExpectedRustupToolchain = '1.97.1-x86_64-pc-windows-msvc'
$script:ExpectedCargoIdentityLines = @(
    'cargo 1.97.1 (c980f4866 2026-06-30)',
    'release: 1.97.1',
    'commit-hash: c980f4866141969fab6254a680546a277789d6f0',
    'commit-date: 2026-06-30',
    'host: x86_64-pc-windows-msvc'
)
$script:OwnerKind = 'LATTICE_PHASE4_MANAGED_FOREMAN_ACCEPTANCE_V1'
$script:ForbiddenPorts = @(4317, 5432, 55432, 58743, 64272)
$script:MaximumMcpStatusPolls = 48
$script:MaximumMcpToolCalls = 56
$script:MinimumMcpStatusResponseBudgetMilliseconds = 5000
$script:OwnedProcessHelper = Join-Path $PSScriptRoot 'phase4-owned-process.ps1'
if (-not (Test-Path -LiteralPath $script:OwnedProcessHelper -PathType Leaf)) {
    throw 'PHASE4_OWNED_PROCESS_HELPER_MISSING'
}
. $script:OwnedProcessHelper

function Get-Phase4RandomHex {
    param([Parameter(Mandatory = $true)][ValidateRange(1, 1024)][int]$ByteCount)

    $bytes = [byte[]]::new($ByteCount)
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Get-Phase4StringSha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return (($sha.ComputeHash($script:Utf8.GetBytes($Value)) |
            ForEach-Object { $_.ToString('x2') }) -join '')
    }
    finally {
        $sha.Dispose()
    }
}

function Get-Phase4FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-Phase4JsonFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    [IO.File]::WriteAllText(
        [IO.Path]::GetFullPath($Path),
        (($Value | ConvertTo-Json -Depth 30) + "`n"),
        $script:Utf8
    )
}

function Write-Phase4AtomicCreateNewUtf8File {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Content
    )

    $finalPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($finalPath)
    if ([string]::IsNullOrWhiteSpace($parent) -or
        -not (Test-Path -LiteralPath $parent -PathType Container)) {
        throw 'PHASE4_FAILURE_RECEIPT_PARENT_REJECTED'
    }
    $temporaryPath = $finalPath + '.' + (Get-Phase4RandomHex -ByteCount 8) + '.tmp'
    $bytes = $script:Utf8.GetBytes($Content)
    $stream = $null
    try {
        $stream = [IO.FileStream]::new(
            $temporaryPath,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
        $stream.Dispose()
        $stream = $null
        [IO.File]::Move($temporaryPath, $finalPath, $false)
    }
    catch {
        if ($null -ne $stream) { $stream.Dispose() }
        if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
            [IO.File]::Delete($temporaryPath)
        }
        throw
    }
}

function Test-Phase4FailureReceiptPersistence {
    $tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
        [IO.Path]::DirectorySeparatorChar
    )
    $rootName = 'lattice-phase4-receipt-selftest-' + (Get-Phase4RandomHex -ByteCount 8)
    $testRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $rootName))
    if ([IO.Path]::GetDirectoryName($testRoot) -cne $tempParent) {
        throw 'PHASE4_FAILURE_RECEIPT_SELFTEST_ROOT_REJECTED'
    }
    $null = [IO.Directory]::CreateDirectory($testRoot)
    try {
        $receiptPath = Join-Path $testRoot 'final-failure-receipt.json'
        $digestPath = Join-Path $testRoot 'final-failure-receipt.sha256'
        $content = '{"schema":"lattice.phase4-managed-foreman.acceptance.v1","status":"FAIL"}' +
            "`n"
        Write-Phase4AtomicCreateNewUtf8File -Path $receiptPath -Content $content
        $digest = Get-Phase4FileSha256 -Path $receiptPath
        Write-Phase4AtomicCreateNewUtf8File -Path $digestPath `
            -Content ($digest + '  final-failure-receipt.json' + "`n")
        $overwriteRejected = $false
        try {
            Write-Phase4AtomicCreateNewUtf8File -Path $receiptPath -Content "substituted`n"
        }
        catch {
            $overwriteRejected = $true
        }
        if (-not $overwriteRejected -or
            [IO.File]::ReadAllText($receiptPath, $script:Utf8) -cne $content -or
            [IO.File]::ReadAllText($digestPath, $script:Utf8) -cne
                ($digest + '  final-failure-receipt.json' + "`n")) {
            throw 'PHASE4_FAILURE_RECEIPT_SELFTEST_REJECTED'
        }
        $partialReceiptPath = Join-Path $testRoot 'partial-final-failure-receipt.json'
        $blockedDigestPath = Join-Path $testRoot 'blocked-digest.sha256'
        $null = [IO.Directory]::CreateDirectory($blockedDigestPath)
        Write-Phase4AtomicCreateNewUtf8File -Path $partialReceiptPath -Content $content
        $digestFailureRejected = $false
        try {
            Write-Phase4AtomicCreateNewUtf8File -Path $blockedDigestPath `
                -Content ((Get-Phase4FileSha256 -Path $partialReceiptPath) + "`n")
        }
        catch {
            $digestFailureRejected = $true
        }
        if (-not $digestFailureRejected -or
            [IO.File]::ReadAllText($partialReceiptPath, $script:Utf8) -cne $content) {
            throw 'PHASE4_FAILURE_RECEIPT_PARTIAL_PUBLISH_SELFTEST_REJECTED'
        }
        return [ordered]@{
            schema = 'lattice.phase4-failure-receipt-persistence-selftest.v1'
            status = 'PASS'
            create_new = $true
            overwrite_rejected = $true
            digest_verified = $true
            digest_failure_preserved_receipt = $true
        }
    }
    finally {
        if (Test-Path -LiteralPath $testRoot -PathType Container) {
            [IO.Directory]::Delete($testRoot, $true)
        }
    }
}

function Test-Phase4McpStatusTimeoutBehavior {
    $completion = [Threading.Tasks.TaskCompletionSource[string]]::new(
        [Threading.Tasks.TaskCreationOptions]::RunContinuationsAsynchronously
    )
    $fakeProcess = [pscustomobject][ordered]@{
        pending = $completion.Task
        write_count = [long]0
    }
    $fakeProcess | Add-Member -MemberType ScriptMethod -Name WriteStandardInput -Value {
        param($Value, $AppendNewline, $TimeoutMilliseconds)
        $this.write_count = [long]$this.write_count + 1
        return $true
    }
    $fakeProcess | Add-Member -MemberType ScriptMethod -Name ReadStandardOutputLineBounded -Value {
        param($MaximumBytes)
        return $this.pending
    }
    $session = [pscustomobject][ordered]@{
        process = $fakeProcess
        next_id = [long]3
        tool_call_count = [long]0
        notification_count = [long]0
        response_contaminated = $false
    }
    $taskRef = 'a' * 64
    $lastStatus = [pscustomobject][ordered]@{
        schema_version = 'lattice.task.status.v4'
        task_state = 'AWAITING_EXECUTION_APPROVAL'
        status = 'BLOCKED'
        task_ref = $taskRef
        failure_code = 'LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED'
        attempt = $null
        worker_running = $false
        thread_id = $null
        turn_id = $null
    }
    $diagnostic = $null
    $typedTimeout = $false
    try {
        $null = Invoke-Phase4McpStatusForGate -Session $session -TaskRef $taskRef `
            -TimeoutSeconds 1 -TimeoutMilliseconds 25 `
            -TimeoutCode 'PHASE4_WSL2_ACTIVE_STATUS_RESPONSE_TIMEOUT' `
            -Stage 'WSL2_ACTIVE_ACCEPTED_START' -PollOrdinal 1 `
            -PollOrigin ([DateTimeOffset]::UtcNow) `
            -RemainingAtDispatchMilliseconds 480000 -LastCompletedStatus $lastStatus `
            -TimeoutDiagnostic ([ref]$diagnostic)
    }
    catch {
        $typedTimeout = [string]$_.Exception.Message -ceq
            'PHASE4_WSL2_ACTIVE_STATUS_RESPONSE_TIMEOUT'
    }
    if (-not $typedTimeout -or -not [bool]$session.response_contaminated -or
        [long]$session.tool_call_count -ne 1 -or [long]$fakeProcess.write_count -ne 1 -or
        [string]$diagnostic.stage -cne 'WSL2_ACTIVE_ACCEPTED_START' -or
        [long]$diagnostic.request_id -ne 3 -or [long]$diagnostic.poll_ordinal -ne 1 -or
        [long]$diagnostic.remaining_at_dispatch_milliseconds -ne 480000 -or
        [long]$diagnostic.configured_response_timeout_seconds -ne 1 -or
        [long]$diagnostic.effective_response_timeout_milliseconds -ne 25 -or
        [string]$diagnostic.last_completed_candidate.failure_code -cne
            'LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED') {
        throw 'PHASE4_MCP_STATUS_TIMEOUT_SELFTEST_REJECTED'
    }
    $completion.SetResult(
        '{"jsonrpc":"2.0","id":3,"result":{"isError":false,"structuredContent":{}}}'
    )
    $reuseRejected = $false
    try {
        $null = Invoke-Phase4McpStatusForGate -Session $session -TaskRef $taskRef `
            -TimeoutSeconds 1 -TimeoutMilliseconds 25 `
            -TimeoutCode 'PHASE4_WSL2_ACTIVE_STATUS_RESPONSE_TIMEOUT' `
            -Stage 'WSL2_ACTIVE_ACCEPTED_START' -PollOrdinal 2 `
            -PollOrigin ([DateTimeOffset]::UtcNow) `
            -RemainingAtDispatchMilliseconds 479000 -LastCompletedStatus $lastStatus `
            -TimeoutDiagnostic ([ref]$diagnostic)
    }
    catch {
        $reuseRejected = [string]$_.Exception.Message -ceq 'PHASE4_MCP_SESSION_CONTAMINATED'
    }
    if (-not $reuseRejected -or [long]$session.tool_call_count -ne 1 -or
        [long]$fakeProcess.write_count -ne 1) {
        throw 'PHASE4_MCP_STATUS_LATE_RESPONSE_SELFTEST_REJECTED'
    }
    return [ordered]@{
        schema = 'lattice.phase4-mcp-status-timeout-selftest.v1'
        status = 'PASS'
        typed_timeout = $true
        diagnostic_bound = $true
        late_response_rejected = $true
        session_reuse_rejected = $true
        extra_tool_calls = 0
    }
}

function ConvertTo-Phase4FailureCode {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Message)

    if ($Message -cmatch '\A[A-Z][A-Z0-9_]{0,127}\z') { return $Message }
    return 'PHASE4_HARNESS_RUNTIME_ERROR'
}

function Assert-Phase4RegularFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw $Failure }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw $Failure }
}

function Assert-Phase4Directory {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { throw $Failure }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw $Failure }
}

function Assert-Phase4ContainedPath {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    $canonicalRoot = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $canonicalPath = [IO.Path]::GetFullPath($Path)
    if (-not $canonicalPath.StartsWith(
        $canonicalRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw $Failure
    }
    return $canonicalPath
}

function Get-Phase4OwnedCodexConfig {
    return ((@(
        'cli_auth_credentials_store = "keyring"',
        'approval_policy = "never"',
        'sandbox_mode = "workspace-write"',
        'model = "gpt-5.6-sol"',
        'model_reasoning_effort = "low"',
        '',
        '[shell_environment_policy]',
        'inherit = "all"',
        'ignore_default_excludes = false',
        'include_only = ["SystemRoot", "WINDIR", "ComSpec", "PATH", "PATHEXT", "PROCESSOR_ARCHITECTURE", "NUMBER_OF_PROCESSORS", "TEMP", "TMP", "LANG", "LC_ALL"]',
        'experimental_use_profile = false',
        '',
        '[windows]',
        'sandbox = "unelevated"',
        '',
        '[features]',
        'plugins = false'
    ) -join "`n") + "`n")
}

function Assert-Phase4ManagedCodexHome {
    param([Parameter(Mandatory = $true)][string]$Path)

    $expected = [IO.Path]::GetFullPath(
        (Join-Path $env:LOCALAPPDATA 'LATTICE\runtime-codex-home-keyring-v1')
    )
    $actual = [IO.Path]::GetFullPath($Path)
    if (-not [string]::Equals($actual, $expected, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'CREDENTIAL_READ_ISOLATION_NOT_VERIFIED'
    }
    Assert-Phase4Directory -Path $actual -Failure 'CREDENTIAL_READ_ISOLATION_NOT_VERIFIED'
    $marker = Join-Path $actual '.lattice-codex-home-v1'
    $config = Join-Path $actual 'config.toml'
    $auth = Join-Path $actual 'auth.json'
    Assert-Phase4RegularFile -Path $marker -Failure 'CREDENTIAL_READ_ISOLATION_NOT_VERIFIED'
    Assert-Phase4RegularFile -Path $config -Failure 'CREDENTIAL_READ_ISOLATION_NOT_VERIFIED'
    if (Test-Path -LiteralPath $auth) { throw 'CREDENTIAL_READ_ISOLATION_NOT_VERIFIED' }
    $expectedMarker = [Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
    $expectedConfig = $script:Utf8.GetBytes((Get-Phase4OwnedCodexConfig))
    if (
        [Convert]::ToBase64String([IO.File]::ReadAllBytes($marker)) -cne
            [Convert]::ToBase64String($expectedMarker) -or
        [Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne
            [Convert]::ToBase64String($expectedConfig)
    ) {
        throw 'CREDENTIAL_READ_ISOLATION_NOT_VERIFIED'
    }
    return $actual
}

function New-Phase4ClosedEnvironment {
    param([Parameter(Mandatory = $true)][Collections.IDictionary]$Values)

    $environment = [ordered]@{}
    foreach ($name in @('SystemRoot', 'WINDIR', 'TEMP', 'TMP', 'PATH', 'ComSpec')) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (-not [string]::IsNullOrWhiteSpace($value)) { $environment[$name] = $value }
    }
    foreach ($entry in $Values.GetEnumerator()) {
        $environment[[string]$entry.Key] = [string]$entry.Value
    }
    $environment['NO_COLOR'] = '1'
    return $environment
}

function Invoke-Phase4Process {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Argument,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [AllowNull()][AllowEmptyString()][string]$StandardInput,
        [Parameter(Mandatory = $true)][ValidateRange(1, 930)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][string]$Failure,
        [switch]$AllowNonZeroExit,
        [switch]$DoNotWaitForRedirectEof,
        [ValidateRange(1, 65535)][int]$OutputEncodingCodePage = 65001
    )

    $script:LastPhase4ProcessTreeTerminationProven = $false
    $process = $null
    try {
        $process = Start-Phase4OwnedProcessJob -Executable $Executable -Argument $Argument `
            -Environment $Environment -WorkingDirectory $WorkingDirectory -Failure $Failure `
            -OutputEncodingCodePage $OutputEncodingCodePage
        $stdoutTask = $null
        $stderrTask = $null
        if (-not $DoNotWaitForRedirectEof) {
            $stdoutTask = $process.ReadStandardOutputToEndBounded(16777216)
            $stderrTask = $process.ReadStandardErrorToEndBounded(16777216)
        }
        if ($null -ne $StandardInput -and
            -not $process.WriteStandardInput($StandardInput, $false, 5000)) {
            Stop-Phase4OwnedProcessJob -OwnedProcess $process `
                -Failure ($Failure + '_PROCESS_TREE_CLEANUP_REJECTED')
            $script:LastPhase4ProcessTreeTerminationProven = $true
            throw ($Failure + '_STDIN_WRITE_REJECTED')
        }
        $process.StandardInput.Close()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            Stop-Phase4OwnedProcessJob -OwnedProcess $process `
                -Failure ($Failure + '_PROCESS_TREE_CLEANUP_REJECTED')
            $script:LastPhase4ProcessTreeTerminationProven = $true
            throw $Failure
        }
        Close-Phase4OwnedProcessJob -OwnedProcess $process `
            -Failure ($Failure + '_PROCESS_TREE_CLEANUP_REJECTED')
        $stdout = if ($DoNotWaitForRedirectEof) { '' } else { $stdoutTask.GetAwaiter().GetResult() }
        $stderr = if ($DoNotWaitForRedirectEof) { '' } else { $stderrTask.GetAwaiter().GetResult() }
        if ($stdout.Length -gt 16777216 -or $stderr.Length -gt 16777216) {
            throw ($Failure + '_OUTPUT_REJECTED')
        }
        if ($process.ExitCode -ne 0 -and -not $AllowNonZeroExit) { throw $Failure }
        return [pscustomobject][ordered]@{
            exit_code = [int]$process.ExitCode
            stdout = [string]$stdout
            stderr_byte_count = [long]$script:Utf8.GetByteCount([string]$stderr)
            stderr_sha256 = Get-Phase4StringSha256 -Value ([string]$stderr)
        }
    }
    finally {
        if ($null -ne $process) { $process.Dispose() }
    }
}

function ConvertTo-Phase4WslUncPath {
    param([Parameter(Mandatory = $true)][string]$LinuxPath)

    if ($LinuxPath -cnotmatch '\A/home/[a-z_][a-z0-9_-]{0,31}(?:/[A-Za-z0-9._-]+)+\z' -or
        $LinuxPath.Contains('/./', [StringComparison]::Ordinal) -or
        $LinuxPath.Contains('/../', [StringComparison]::Ordinal) -or
        $LinuxPath.EndsWith('/.', [StringComparison]::Ordinal) -or
        $LinuxPath.EndsWith('/..', [StringComparison]::Ordinal)) {
        throw 'PHASE4_WSL2_LINUX_PATH_REJECTED'
    }
    return ('\\wsl.localhost\' + $script:Wsl2Distribution +
        $LinuxPath.Replace('/', '\'))
}

function ConvertFrom-Phase4WslUncPath {
    param([Parameter(Mandatory = $true)][string]$WindowsPath)

    $prefix = '\\wsl.localhost\' + $script:Wsl2Distribution + '\'
    $canonical = [IO.Path]::GetFullPath($WindowsPath)
    if (-not $canonical.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'PHASE4_WSL2_UNC_PATH_REJECTED'
    }
    $linux = '/' + $canonical.Substring($prefix.Length).Replace('\', '/')
    if ($linux -cnotmatch '\A/home/[a-z_][a-z0-9_-]{0,31}(?:/[A-Za-z0-9._-]+)+\z' -or
        $linux.Contains('/./', [StringComparison]::Ordinal) -or
        $linux.Contains('/../', [StringComparison]::Ordinal)) {
        throw 'PHASE4_WSL2_UNC_PATH_REJECTED'
    }
    return $linux
}

function Invoke-Phase4WslRelay {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Argument,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [Parameter(Mandatory = $true)][string]$LinuxHome,
        [Parameter(Mandatory = $true)][string]$LinuxTemp,
        [AllowNull()][AllowEmptyString()][string]$StandardInput,
        [Parameter(Mandatory = $true)][ValidateRange(1, 930)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][string]$Failure,
        [switch]$AllowNonZeroExit
    )

    if ($Executable -cnotmatch '\A/(?:[A-Za-z0-9._-]+/)*[A-Za-z0-9._-]+\z' -or
        $LinuxHome -cnotmatch '\A/home/[A-Za-z0-9._/-]+\z' -or
        $LinuxTemp -cnotmatch '\A/home/[A-Za-z0-9._/-]+\z') {
        throw $Failure
    }
    $arguments = [Collections.Generic.List[string]]::new()
    foreach ($value in @(
        '-d', $script:Wsl2Distribution, '--exec', '/usr/bin/env', '-i',
        ('HOME=' + $LinuxHome), ('TMPDIR=' + $LinuxTemp),
        'XDG_RUNTIME_DIR=/run/user/1000', 'PATH=/usr/bin:/bin',
        'LANG=C.UTF-8', 'LC_ALL=C.UTF-8', 'NO_COLOR=1'
    )) {
        $arguments.Add([string]$value)
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        $name = [string]$entry.Key
        $value = [string]$entry.Value
        if ($name -cnotmatch '\A[A-Z][A-Z0-9_]{0,63}\z' -or
            $value.Contains("`0", [StringComparison]::Ordinal)) {
            throw $Failure
        }
        $arguments.Add($name + '=' + $value)
    }
    $arguments.Add($Executable)
    foreach ($value in $Argument) { $arguments.Add([string]$value) }
    return Invoke-Phase4Process -Executable $script:Wsl -Argument @($arguments) `
        -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
        -WorkingDirectory $script:RepositoryRoot -StandardInput $StandardInput `
        -TimeoutSeconds $TimeoutSeconds -Failure $Failure -AllowNonZeroExit:$AllowNonZeroExit
}

function Invoke-Phase4WslControlPlane {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Argument,
        [Parameter(Mandatory = $true)][string]$Failure,
        [switch]$AllowNonZeroExit
    )

    return Invoke-Phase4WslRelay -Executable '/usr/bin/timeout' -Argument (@(
        '--signal=TERM', '--kill-after=2s', '10s', $Executable
    ) + @($Argument)) -Environment ([ordered]@{
        DBUS_SESSION_BUS_ADDRESS = 'unix:path:/run/user/1000/bus'
    }) -LinuxHome $script:Wsl2HarnessHome -LinuxTemp $script:Wsl2HarnessTemp `
        -StandardInput $null -TimeoutSeconds 15 -Failure $Failure `
        -AllowNonZeroExit:$AllowNonZeroExit
}

function Get-Phase4WslCommandUnitState {
    param([Parameter(Mandatory = $true)][string]$Unit)

    if ($Unit -cnotmatch ('\A' + [regex]::Escape($script:WslCommandUnitPrefix) +
            '-[0-9a-f]{12}\.service\z')) {
        throw 'PHASE4_WSL2_COMMAND_UNIT_IDENTITY_REJECTED'
    }
    $result = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
        '--user', '--no-pager', 'show', $Unit,
        '--property=Id', '--property=LoadState', '--property=ActiveState',
        '--property=SubState', '--property=Result', '--property=ControlGroup',
        '--property=MainPID'
    ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_QUERY_FAILED' -AllowNonZeroExit
    if ([int]$result.exit_code -ne 0 -or [long]$result.stderr_byte_count -ne 0 -or
        $script:Utf8.GetByteCount([string]$result.stdout) -gt 8192) {
        throw 'PHASE4_WSL2_COMMAND_UNIT_QUERY_REJECTED'
    }
    $expected = @('Id', 'LoadState', 'ActiveState', 'SubState', 'Result', 'ControlGroup', 'MainPID')
    $values = [ordered]@{}
    foreach ($line in @([string]$result.stdout -split '\r?\n' | Where-Object { $_.Length -gt 0 })) {
        if ($line -cnotmatch '\A(?<name>[A-Za-z]+)=(?<value>.*)\z' -or
            [string]$Matches.name -cnotin $expected -or $values.Contains([string]$Matches.name)) {
            throw 'PHASE4_WSL2_COMMAND_UNIT_QUERY_REJECTED'
        }
        $values[[string]$Matches.name] = [string]$Matches.value
    }
    if ($values.Count -ne $expected.Count -or
        @($expected | Where-Object { -not $values.Contains($_) }).Count -ne 0 -or
        [string]$values.Id -cne $Unit -or [string]$values.MainPID -cnotmatch '\A[0-9]+\z') {
        throw 'PHASE4_WSL2_COMMAND_UNIT_QUERY_REJECTED'
    }
    return [pscustomobject][ordered]@{
        load_state = [string]$values.LoadState
        active_state = [string]$values.ActiveState
        sub_state = [string]$values.SubState
        result = [string]$values.Result
        cgroup_path = [string]$values.ControlGroup
        main_pid = [long]$values.MainPID
    }
}

function Close-Phase4WslCommandUnit {
    param([Parameter(Mandatory = $true)][string]$Unit)

    $canonical = '/user.slice/user-1000.slice/user@1000.service/app.slice/' + $Unit
    $state = Get-Phase4WslCommandUnitState -Unit $Unit
    if ((-not [string]::IsNullOrWhiteSpace([string]$state.cgroup_path)) -and
        [string]$state.cgroup_path -cne $canonical) {
        throw 'PHASE4_WSL2_COMMAND_CGROUP_IDENTITY_REJECTED'
    }
    if ([string]$state.active_state -cne 'inactive' -or [long]$state.main_pid -ne 0) {
        $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
            '--user', '--no-pager', 'kill', '--kill-who=all', '--signal=SIGTERM', $Unit
        ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_STOP_FAILED' -AllowNonZeroExit
        $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
            '--user', '--no-pager', '--no-block', 'stop', $Unit
        ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_STOP_FAILED' -AllowNonZeroExit
        $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
            '--user', '--no-pager', 'reset-failed', $Unit
        ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_RESET_FAILED' -AllowNonZeroExit
    }
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
    do {
        $state = Get-Phase4WslCommandUnitState -Unit $Unit
        $absent = Invoke-Phase4WslControlPlane -Executable '/usr/bin/test' -Argument @(
            '!', '-e', ('/sys/fs/cgroup' + $canonical)
        ) -Failure 'PHASE4_WSL2_COMMAND_CGROUP_QUERY_FAILED' -AllowNonZeroExit
        if ([string]$state.active_state -ceq 'inactive' -and
            [string]$state.sub_state -ceq 'dead' -and [long]$state.main_pid -eq 0 -and
            [int]$absent.exit_code -eq 0) {
            break
        }
        if ([string]$state.active_state -ceq 'failed' -and
            [long]$state.main_pid -eq 0 -and [int]$absent.exit_code -eq 0) {
            $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
                '--user', '--no-pager', 'reset-failed', $Unit
            ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_RESET_FAILED' -AllowNonZeroExit
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    if ([string]$state.active_state -cne 'inactive' -or
        [string]$state.sub_state -cne 'dead' -or [long]$state.main_pid -ne 0 -or
        [int]$absent.exit_code -ne 0) {
        $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
            '--user', '--no-pager', 'kill', '--kill-who=all', '--signal=SIGKILL', $Unit
        ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_FORCE_STOP_FAILED' -AllowNonZeroExit
        $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
            '--user', '--no-pager', '--no-block', 'stop', '--force', $Unit
        ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_FORCE_STOP_FAILED' -AllowNonZeroExit
        $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
            '--user', '--no-pager', 'reset-failed', $Unit
        ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_RESET_FAILED' -AllowNonZeroExit
        $deadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
        do {
            $state = Get-Phase4WslCommandUnitState -Unit $Unit
            $absent = Invoke-Phase4WslControlPlane -Executable '/usr/bin/test' -Argument @(
                '!', '-e', ('/sys/fs/cgroup' + $canonical)
            ) -Failure 'PHASE4_WSL2_COMMAND_CGROUP_QUERY_FAILED' -AllowNonZeroExit
            if ([string]$state.active_state -ceq 'inactive' -and
                [string]$state.sub_state -ceq 'dead' -and [long]$state.main_pid -eq 0 -and
                [int]$absent.exit_code -eq 0) {
                break
            }
            if ([string]$state.active_state -ceq 'failed' -and
                [long]$state.main_pid -eq 0 -and [int]$absent.exit_code -eq 0) {
                $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
                    '--user', '--no-pager', 'reset-failed', $Unit
                ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_RESET_FAILED' -AllowNonZeroExit
            }
            Start-Sleep -Milliseconds 100
        } while ([DateTimeOffset]::UtcNow -lt $deadline)
    }
    if ([string]$state.active_state -cne 'inactive' -or
        [string]$state.sub_state -cne 'dead' -or [long]$state.main_pid -ne 0 -or
        [int]$absent.exit_code -ne 0) {
        throw 'PHASE4_WSL2_COMMAND_UNIT_CLEANUP_REJECTED'
    }
    $null = Invoke-Phase4WslControlPlane -Executable '/usr/bin/systemctl' -Argument @(
        '--user', '--no-pager', 'reset-failed', $Unit
    ) -Failure 'PHASE4_WSL2_COMMAND_UNIT_RESET_FAILED' -AllowNonZeroExit
}

function Close-Phase4WslOpenCommandUnits {
    if ($null -eq $script:WslOpenCommandUnits) { return }
    for ($attempt = 1; $attempt -le 3 -and $script:WslOpenCommandUnits.Count -ne 0; $attempt++) {
        foreach ($unit in @($script:WslOpenCommandUnits | Sort-Object -CaseSensitive)) {
            try {
                Close-Phase4WslCommandUnit -Unit $unit
                $null = $script:WslOpenCommandUnits.Remove($unit)
            }
            catch {}
        }
        if ($script:WslOpenCommandUnits.Count -ne 0 -and $attempt -lt 3) {
            Start-Sleep -Seconds 1
        }
    }
    if ($script:WslOpenCommandUnits.Count -ne 0) {
        throw 'PHASE4_WSL2_COMMAND_UNIT_CLEANUP_REJECTED'
    }
}

function Invoke-Phase4WslProcess {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Argument,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [Parameter(Mandatory = $true)][string]$LinuxHome,
        [Parameter(Mandatory = $true)][string]$LinuxTemp,
        [AllowNull()][AllowEmptyString()][string]$StandardInput,
        [Parameter(Mandatory = $true)][ValidateRange(1, 900)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][string]$Failure,
        [switch]$AllowNonZeroExit
    )

    if ([string]::IsNullOrWhiteSpace($script:WslCommandUnitPrefix)) { throw $Failure }
    $script:WslCommandUnitCounter = [long]$script:WslCommandUnitCounter + 1
    $unit = $script:WslCommandUnitPrefix + '-' +
        ([long]$script:WslCommandUnitCounter).ToString('x12') + '.service'
    if ($null -eq $script:WslOpenCommandUnits -or
        -not $script:WslOpenCommandUnits.Add($unit)) {
        throw 'PHASE4_WSL2_COMMAND_UNIT_IDENTITY_REJECTED'
    }
    $serviceArguments = [Collections.Generic.List[string]]::new()
    foreach ($value in @(
        '--user', '--quiet', '--wait', '--pipe', ('--unit=' + $unit),
        '--property=Type=exec', '--property=KillMode=control-group',
        ('--property=RuntimeMaxSec=' + $TimeoutSeconds + 's'),
        '--property=TimeoutStopSec=5s', ('--property=WorkingDirectory=' + $LinuxHome),
        '--', '/usr/bin/env', '-i', ('HOME=' + $LinuxHome), ('TMPDIR=' + $LinuxTemp),
        'XDG_RUNTIME_DIR=/run/user/1000', 'PATH=/usr/bin:/bin',
        'LANG=C.UTF-8', 'LC_ALL=C.UTF-8', 'NO_COLOR=1'
    )) {
        $serviceArguments.Add([string]$value)
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        $name = [string]$entry.Key
        $value = [string]$entry.Value
        if ($name -cnotmatch '\A[A-Z][A-Z0-9_]{0,63}\z' -or
            $value.Contains("`0", [StringComparison]::Ordinal)) {
            throw $Failure
        }
        $serviceArguments.Add($name + '=' + $value)
    }
    $serviceArguments.Add($Executable)
    foreach ($value in $Argument) { $serviceArguments.Add([string]$value) }
    try {
        return Invoke-Phase4WslRelay -Executable '/usr/bin/systemd-run' `
            -Argument @($serviceArguments) -Environment ([ordered]@{}) `
            -LinuxHome $LinuxHome -LinuxTemp $LinuxTemp -StandardInput $StandardInput `
            -TimeoutSeconds ($TimeoutSeconds + 15) -Failure $Failure `
            -AllowNonZeroExit:$AllowNonZeroExit
    }
    finally {
        Close-Phase4WslCommandUnit -Unit $unit
        $null = $script:WslOpenCommandUnits.Remove($unit)
    }
}

function Invoke-Phase4RepositoryGit {
    param(
        [Parameter(Mandatory = $true)][string]$Git,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Argument,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string]$Failure,
        [switch]$AllowNonZeroExit
    )

    if (-not $script:Wsl2LinuxLiveEnabled) {
        return Invoke-Phase4Process -Executable $Git -Argument $Argument `
            -Environment $Environment -WorkingDirectory $WorkingDirectory -StandardInput $null `
            -TimeoutSeconds 30 -Failure $Failure -AllowNonZeroExit:$AllowNonZeroExit
    }
    $mapped = [Collections.Generic.List[string]]::new()
    foreach ($value in $Argument) {
        if ([string]$value -like '\\wsl.localhost\*') {
            $mapped.Add((ConvertFrom-Phase4WslUncPath -WindowsPath ([string]$value)))
        }
        else {
            $mapped.Add([string]$value)
        }
    }
    $gitArguments = @(
        '--no-pager', '--no-replace-objects', '--literal-pathspecs',
        '-c', ('core.hooksPath=' + $script:Wsl2HarnessGitHooks),
        '-c', 'core.fsmonitor=false', '-c', 'core.untrackedCache=false',
        '-c', 'protocol.allow=never', '-c', 'protocol.file.allow=never',
        '-c', 'protocol.ext.allow=never'
    ) + @($mapped)
    return Invoke-Phase4WslProcess -Executable '/usr/bin/git' -Argument $gitArguments `
        -Environment ([ordered]@{
            GIT_CONFIG_NOSYSTEM = '1'
            GIT_CONFIG_GLOBAL = ($script:Wsl2HarnessHome + '/.gitconfig')
            GIT_TERMINAL_PROMPT = '0'
            GIT_OPTIONAL_LOCKS = '0'
            GIT_ATTR_NOSYSTEM = '1'
        }) -LinuxHome $script:Wsl2HarnessHome -LinuxTemp $script:Wsl2HarnessTemp `
        -StandardInput $null -TimeoutSeconds 30 -Failure $Failure `
        -AllowNonZeroExit:$AllowNonZeroExit
}

function Assert-Phase4NoCredentialShapedJsonStrings {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    $pending = [Collections.Generic.Queue[object]]::new()
    $pending.Enqueue($Value)
    while ($pending.Count -gt 0) {
        $current = $pending.Dequeue()
        if ($null -eq $current) { continue }
        if ($current -is [string]) {
            $text = [string]$current
            $lower = $text.ToLowerInvariant()
            if ($text -match '(?:\bBearer\s+[A-Za-z0-9._~+/=-]{12,}|\b(?:password|passwd|api[_-]?key|access[_-]?token|secret)\s*[:=]\s*["'']?[A-Za-z0-9._~+/=-]{8,}|\bgh[pousr]_[A-Za-z0-9]{12,}|\bsk-[A-Za-z0-9_-]{16,}|[a-z][a-z0-9+.-]*://[^\s/:@]+:[^\s/@]+@)' -or
                $lower.Contains('github_pat_', [StringComparison]::Ordinal) -or
                $lower.Contains('glpat-', [StringComparison]::Ordinal) -or
                $lower.Contains('npm_', [StringComparison]::Ordinal) -or
                $lower.Contains('pypi-', [StringComparison]::Ordinal) -or
                $lower -cmatch 'xox[abprs]-' -or
                ($lower.Contains('-----begin ', [StringComparison]::Ordinal) -and
                    $lower.Contains('private key-----', [StringComparison]::Ordinal)) -or
                $text -cmatch '(^|[^A-Za-z0-9])(AKIA|ASIA)[A-Z0-9]{16}([^A-Za-z0-9]|$)') {
                throw $Failure
            }
            continue
        }
        if ($current -is [Collections.IDictionary]) {
            foreach ($entry in $current.GetEnumerator()) { $pending.Enqueue($entry.Value) }
            continue
        }
        if ($current -is [Collections.IEnumerable]) {
            foreach ($item in $current) { $pending.Enqueue($item) }
            continue
        }
        foreach ($property in $current.PSObject.Properties) {
            $pending.Enqueue($property.Value)
        }
    }
}

function Assert-Phase4ExactJsonProperties {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Expected,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    if ($null -eq $Value -or $Value -is [string] -or
        $Value -is [Collections.IDictionary] -or $Value -is [Collections.IEnumerable]) {
        throw $Failure
    }
    $actual = @($Value.PSObject.Properties | ForEach-Object { [string]$_.Name })
    if ($actual.Count -ne $Expected.Count) { throw $Failure }
    $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($name in $actual) {
        if (-not $names.Add($name)) { throw $Failure }
    }
    foreach ($name in $Expected) {
        if (-not $names.Contains($name)) { throw $Failure }
    }
}

function Get-Phase4CanonicalUserServiceCgroupPath {
    param(
        [Parameter(Mandatory = $true)][long]$OwnerUid,
        [Parameter(Mandatory = $true)][string]$Unit
    )

    if ($OwnerUid -le 0 -or $Unit -cnotmatch '\A[A-Za-z0-9_.@:-]+\.service\z') {
        throw 'PHASE4_WSL2_PROVIDER_CGROUP_IDENTITY_REJECTED'
    }
    return [string]::Format(
        [Globalization.CultureInfo]::InvariantCulture,
        '/user.slice/user-{0}.slice/user@{0}.service/app.slice/{1}',
        $OwnerUid,
        $Unit
    )
}

function Invoke-Phase4Wsl2Materializer {
    param(
        [Parameter(Mandatory = $true)][string]$NodeExecutable,
        [Parameter(Mandatory = $true)][string]$TaskRoot,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$ProcessFence,
        [Parameter(Mandatory = $true)][string]$WorktreeRef,
        [Parameter(Mandatory = $true)][string]$ExpectedRepositoryHead,
        [Parameter(Mandatory = $true)][string]$ExpectedSupervisorSha256
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ProcessFence -cnotmatch '\A[0-9a-f]{64}\z' -or
        $WorktreeRef -cnotmatch '\Aworktree:sha256:[0-9a-f]{64}\z' -or
        $ExpectedRepositoryHead -cnotmatch '\A[0-9a-f]{40}\z' -or
        $ExpectedSupervisorSha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        -not $Repository.StartsWith($TaskRoot + '/managed-worktrees/', [StringComparison]::Ordinal)) {
        throw 'PHASE4_WSL2_MATERIALIZER_INPUT_REJECTED'
    }
    $result = Invoke-Phase4Process -Executable $NodeExecutable -Argument @(
        $script:Wsl2Materializer,
        '--task-root', $TaskRoot,
        '--repository', $Repository,
        '--task-ref', $TaskRef,
        '--process-fence', $ProcessFence,
        '--worktree-ref', $WorktreeRef,
        '--expected-repository-head', $ExpectedRepositoryHead,
        '--expected-supervisor-sha256', $ExpectedSupervisorSha256
    ) -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
        -WorkingDirectory $script:RepositoryRoot -StandardInput $null -TimeoutSeconds 300 `
        -Failure 'PHASE4_WSL2_MATERIALIZER_FAILED' -AllowNonZeroExit
    $lines = @($result.stdout -split '\r?\n' | Where-Object { $_.Length -gt 0 })
    if ($lines.Count -ne 1 -or $script:Utf8.GetByteCount($lines[0]) -gt 16384) {
        throw 'PHASE4_WSL2_MATERIALIZER_OUTPUT_REJECTED'
    }
    try { $record = $lines[0] | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_MATERIALIZER_OUTPUT_REJECTED' }
    if ([int]$result.exit_code -ne 0 -or [string]$record.status -cne 'PASS') {
        $code = [string]$record.code
        if ($code -cmatch '\APHASE4_WSL2_[A-Z0-9_]{1,96}\z') { throw $code }
        throw 'PHASE4_WSL2_MATERIALIZER_FAILED'
    }
    if ([string]$record.schema -cne 'lattice.phase4-wsl2-live-materialization/1.0' -or
        [string]$record.task_ref -cne $TaskRef -or [int]$record.attempt -ne 1 -or
        [long]$record.provider_effect_count -ne 0 -or
        [string]$record.repository_head -cne $ExpectedRepositoryHead -or
        [string]$record.expected_repository_head -cne $ExpectedRepositoryHead -or
        [string]$record.execution_environment_ref -cnotmatch
            '\Aexecution-environment:sha256:[0-9a-f]{64}\z' -or
        [string]$record.credential_authority_kind -cne 'LINUX_KEYRING' -or
        [string]$record.credential_seal_digest -cnotmatch
            '\Acredential-seal:sha256:[0-9a-f]{64}\z' -or
        [string]$record.verification_toolchain_ref -cnotmatch
            '\Awsl2-verification-toolchain:sha256:[0-9a-f]{64}\z' -or
        [string]$record.process_fence_authority_ref -cnotmatch
            '\Awsl2-process-fence-authority:sha256:[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_MATERIALIZER_OUTPUT_REJECTED'
    }
    $evidenceDirectory = [IO.Path]::GetFullPath([string]$record.evidence_directory)
    $expectedEvidencePrefix = ConvertTo-Phase4WslUncPath -LinuxPath ($TaskRoot + '/verifier-state')
    if (-not $evidenceDirectory.StartsWith(
            $expectedEvidencePrefix + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'PHASE4_WSL2_MATERIALIZER_EVIDENCE_PATH_REJECTED'
    }
    $descriptorPath = Join-Path $evidenceDirectory 'execution-environment.json'
    Assert-Phase4RegularFile -Path $descriptorPath `
        -Failure 'PHASE4_WSL2_DESCRIPTOR_FILE_REJECTED'
    $descriptorBytes = [IO.File]::ReadAllBytes($descriptorPath)
    if ($descriptorBytes.Length -lt 2 -or $descriptorBytes.Length -gt 16385 -or
        $descriptorBytes[-1] -ne 10 -or $descriptorBytes -contains 13) {
        throw 'PHASE4_WSL2_DESCRIPTOR_FILE_REJECTED'
    }
    $descriptorJson = $script:Utf8.GetString($descriptorBytes, 0, $descriptorBytes.Length - 1)
    if ($descriptorJson -cmatch '(?i)"(?:password|token|secret|auth_json)"\s*:') {
        throw 'PHASE4_WSL2_DESCRIPTOR_SECRET_REJECTED'
    }
    try { $descriptor = $descriptorJson | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_DESCRIPTOR_FILE_REJECTED' }
    Assert-Phase4NoCredentialShapedJsonStrings -Value $descriptor `
        -Failure 'PHASE4_WSL2_DESCRIPTOR_SECRET_REJECTED'
    if ([string]$descriptor.schema -cne 'lattice.execution-environment.wsl2-linux/1.1' -or
        [string]$descriptor.kind -cne 'WSL2_LINUX' -or
        [string]$descriptor.distribution -cne $script:Wsl2Distribution -or
        [string]$descriptor.identity_digest -cne [string]$record.execution_environment_ref -or
        [string]$descriptor.linux.cwd -cne $Repository -or
        [string]$descriptor.linux.repository_head -cne $ExpectedRepositoryHead -or
        [string]$descriptor.credential_authority.kind -cne 'LINUX_KEYRING' -or
        [string]$descriptor.credential_authority.authority_digest -cnotmatch
            '\Awsl2-credential-authority:sha256:[0-9a-f]{64}\z' -or
        [string]$descriptor.verification_toolchain.identity_digest -cne
            [string]$record.verification_toolchain_ref -or
        [string]$descriptor.process_fence.identity_digest -cne
            [string]$record.process_fence_authority_ref -or
        [string]$descriptor.path_mapping.windows_path -cne
            (ConvertTo-Phase4WslUncPath -LinuxPath $Repository) -or
        [string]$descriptor.path_mapping.linux_path -cne $Repository) {
        throw 'PHASE4_WSL2_DESCRIPTOR_IDENTITY_REJECTED'
    }
    $preflightPath = Join-Path $evidenceDirectory 'zero-model-preflight.json'
    Assert-Phase4RegularFile -Path $preflightPath `
        -Failure 'PHASE4_WSL2_PREFLIGHT_FILE_REJECTED'
    $preflightBytes = [IO.File]::ReadAllBytes($preflightPath)
    if ($preflightBytes.Length -lt 2 -or $preflightBytes.Length -gt 65537 -or
        $preflightBytes[-1] -ne 10 -or $preflightBytes -contains 13) {
        throw 'PHASE4_WSL2_PREFLIGHT_FILE_REJECTED'
    }
    $preflightJson = $script:Utf8.GetString(
        $preflightBytes,
        0,
        $preflightBytes.Length - 1
    )
    try { $preflight = $preflightJson | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_PREFLIGHT_FILE_REJECTED' }
    Assert-Phase4NoCredentialShapedJsonStrings -Value $preflight `
        -Failure 'PHASE4_WSL2_PREFLIGHT_SECRET_REJECTED'
    if ([string]$preflight.schema -cne 'lattice.wsl2-zero-model-preflight/1.0' -or
        [string]$preflight.status -cne 'PASS' -or
        [string]$preflight.task_ref -cne $TaskRef -or [int]$preflight.attempt -ne 1 -or
        [string]$preflight.worktree_ref -cne $WorktreeRef -or
        [string]$preflight.execution_environment_ref -cne [string]$record.execution_environment_ref -or
        [string]$preflight.process_fence.fence -cne $ProcessFence -or
        [string]$preflight.process_fence.authority_ref -cne
            [string]$record.process_fence_authority_ref -or
        [long]$preflight.provider_effect_count -ne 0 -or
        [long]$preflight.effect_counters.provider_effect_count -ne 0 -or
        [long]$preflight.effect_counters.thread_start -ne 0 -or
        [long]$preflight.effect_counters.turn_start -ne 0 -or
        [bool]$preflight.connector_auth_ready) {
        throw 'PHASE4_WSL2_PREFLIGHT_FILE_REJECTED'
    }
    Assert-Phase4ExactJsonProperties -Value $preflight.process_fence -Expected @(
        'fence', 'authority_ref', 'service_unit', 'cgroup_path', 'cgroup_version',
        'delegated', 'boot_id_digest', 'supervisor_zero_descendants', 'outer_post_exit'
    ) -Failure 'PHASE4_WSL2_PREFLIGHT_FILE_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $preflight.process_fence.outer_post_exit `
        -Expected @(
            'unit', 'active_state', 'sub_state', 'result', 'cgroup_path', 'delegate',
            'cgroup_exists', 'populated'
        ) -Failure 'PHASE4_WSL2_PREFLIGHT_FILE_REJECTED'
    $expectedPreflightUnit = [string]$descriptor.process_fence.unit_prefix +
        '-preflight-' + $ProcessFence.Substring(0, 12) + '.service'
    $expectedPreflightCgroup = Get-Phase4CanonicalUserServiceCgroupPath `
        -OwnerUid ([long]$descriptor.verification_toolchain.owner_uid) `
        -Unit $expectedPreflightUnit
    $preflightCgroup = [string]$preflight.process_fence.cgroup_path
    $outer = $preflight.process_fence.outer_post_exit
    $cgroupClosed = (
        $outer.cgroup_exists -is [bool] -and
        ((-not [bool]$outer.cgroup_exists -and $null -eq $outer.populated) -or
         ([bool]$outer.cgroup_exists -and $null -ne $outer.populated -and
          [long]$outer.populated -eq 0))
    )
    if ([string]$preflight.process_fence.service_unit -cne $expectedPreflightUnit -or
        [string]$outer.unit -cne $expectedPreflightUnit -or
        $preflightCgroup -cne $expectedPreflightCgroup -or
        [string]$outer.cgroup_path -cne $preflightCgroup -or
        [long]$preflight.process_fence.cgroup_version -ne 2 -or
        [bool]$preflight.process_fence.delegated -or
        [string]$preflight.process_fence.boot_id_digest -cnotmatch
            '\Awsl-boot:sha256:[0-9a-f]{64}\z' -or
        -not [bool]$preflight.process_fence.supervisor_zero_descendants -or
        [string]$outer.active_state -cne 'inactive' -or
        [string]$outer.sub_state -cne 'dead' -or [string]$outer.result -cne 'success' -or
        [string]$outer.delegate -cne 'no' -or -not $cgroupClosed) {
        throw 'PHASE4_WSL2_PREFLIGHT_PROCESS_FENCE_REJECTED'
    }
    return [pscustomobject][ordered]@{
        record = $record
        descriptor = $descriptor
        descriptor_json = $descriptorJson
        descriptor_path = $descriptorPath
        preflight = $preflight
        preflight_path = $preflightPath
        evidence_directory = $evidenceDirectory
        process_fence = $ProcessFence
        worktree_ref = $WorktreeRef
        process_evidence = [pscustomobject][ordered]@{
            timeout_seconds = 300
            stdout_byte_count = [long]$script:Utf8.GetByteCount([string]$result.stdout)
            stdout_sha256 = Get-Phase4StringSha256 -Value ([string]$result.stdout)
            stderr_byte_count = [long]$result.stderr_byte_count
            stderr_sha256 = [string]$result.stderr_sha256
            exit_code = [int]$result.exit_code
        }
    }
}

function Invoke-Phase4ManagedWorktreeBridge {
    param(
        [Parameter(Mandatory = $true)][string]$NodeExecutable,
        [Parameter(Mandatory = $true)][ValidateSet('prepare', 'verify')][string]$Operation,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$WorktreeRoot,
        [Parameter(Mandatory = $true)][string]$GitExecutable,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$TaskId,
        [Parameter(Mandatory = $true)][string]$BaseCommit,
        [AllowNull()]$ExpectedBaselineSha256,
        [Parameter(Mandatory = $true)][AllowNull()]$ExpectedExecutionEnvironmentRef,
        [Parameter(Mandatory = $true)][string]$ExecutionEnvironmentJson
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $TaskId -cnotmatch '\ATASK-[A-Z0-9][A-Z0-9_-]{2,63}\z' -or
        $BaseCommit -cnotmatch '\A[0-9a-f]{40}\z' -or
        ($Operation -ceq 'prepare' -and $null -ne $ExpectedBaselineSha256) -or
        ($Operation -ceq 'verify' -and
            [string]$ExpectedBaselineSha256 -cnotmatch '\A[0-9a-f]{64}\z') -or
        [string]$ExpectedExecutionEnvironmentRef -cnotmatch
            '\Aexecution-environment:sha256:[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_WORKTREE_BRIDGE_INPUT_REJECTED'
    }
    $command = [ordered]@{
        schema = 'lattice.managed-worktree-command/1.1'
        operation = $Operation
        repository_root = $RepositoryRoot
        worktree_root = $WorktreeRoot
        git_executable = $GitExecutable
        task_ref = $TaskRef
        task_id = $TaskId
        base_commit = $BaseCommit
        expected_baseline_sha256 = $ExpectedBaselineSha256
        expected_execution_environment_ref = $ExpectedExecutionEnvironmentRef
    }
    $inputLine = ($command | ConvertTo-Json -Compress -Depth 10) + "`n"
    $result = Invoke-Phase4Process -Executable $NodeExecutable -Argument @(
        $script:ManagedWorktreeBridge
    ) -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{
        LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON = $ExecutionEnvironmentJson
    })) -WorkingDirectory $script:RepositoryRoot -StandardInput $inputLine `
        -TimeoutSeconds 120 -Failure 'PHASE4_WSL2_WORKTREE_BRIDGE_FAILED' -AllowNonZeroExit
    $lines = @($result.stdout -split '\r?\n' | Where-Object { $_.Length -gt 0 })
    if ($lines.Count -ne 1 -or $script:Utf8.GetByteCount($lines[0]) -gt 32768) {
        throw 'PHASE4_WSL2_WORKTREE_BRIDGE_OUTPUT_REJECTED'
    }
    try { $record = $lines[0] | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_WORKTREE_BRIDGE_OUTPUT_REJECTED' }
    if ([int]$result.exit_code -ne 0 -or [string]$record.kind -cne 'result') {
        $code = [string]$record.code
        if ($code -cmatch '\AMANAGED_WORKTREE_[A-Z0-9_]{1,80}\z') { throw $code }
        throw 'PHASE4_WSL2_WORKTREE_BRIDGE_FAILED'
    }
    if ([string]$record.schema -cne 'lattice.managed-worktree-bridge-result/1.0' -or
        [string]$record.operation -cne $Operation -or
        [string]$record.task_ref -cne $TaskRef -or
        [string]$record.task_id -cne $TaskId -or
        [string]$record.base_commit -cne $BaseCommit -or
        [string]$record.baseline_sha256 -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_WORKTREE_BRIDGE_OUTPUT_REJECTED'
    }
    $record | Add-Member -NotePropertyName harness_process_evidence `
        -NotePropertyValue ([pscustomobject][ordered]@{
            timeout_seconds = 120
            stdout_byte_count = [long]$script:Utf8.GetByteCount([string]$result.stdout)
            stdout_sha256 = Get-Phase4StringSha256 -Value ([string]$result.stdout)
            stderr_byte_count = [long]$result.stderr_byte_count
            stderr_sha256 = [string]$result.stderr_sha256
            exit_code = [int]$result.exit_code
        })
    return $record
}

function New-Phase4ResealedExecutionEnvironmentSubstitution {
    param(
        [Parameter(Mandatory = $true)][string]$NodeExecutable,
        [Parameter(Mandatory = $true)][string]$DescriptorJson
    )

    $domainModule = Join-Path `
        $script:RepositoryRoot 'apps\lattice-control\src\wsl2-execution-domain.mjs'
    $fixtureScript = @'
import { pathToFileURL } from "node:url";

const domain = await import(pathToFileURL(process.argv[1]).href);
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const descriptor = JSON.parse(Buffer.concat(chunks).toString("utf8"));
const replacement = "f".repeat(64);
descriptor.linux.git_sha256 = descriptor.linux.git_sha256 === replacement
  ? "e".repeat(64)
  : replacement;
descriptor.identity_digest = domain.executionEnvironmentIdentity(descriptor);
const validated = domain.validateWsl2ExecutionEnvironment(descriptor);
process.stdout.write(domain.canonicalJson(validated));
'@
    $result = Invoke-Phase4Process -Executable $NodeExecutable -Argument @(
        '--input-type=module', '--eval', $fixtureScript, $domainModule
    ) -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
        -WorkingDirectory $script:RepositoryRoot `
        -StandardInput ($DescriptorJson + "`n") -TimeoutSeconds 30 `
        -Failure 'PHASE4_WSL2_DESCRIPTOR_SUBSTITUTION_FIXTURE_REJECTED'
    $lines = @($result.stdout -split '\r?\n' | Where-Object { $_.Length -gt 0 })
    if ($lines.Count -ne 1 -or $script:Utf8.GetByteCount($lines[0]) -gt 262144) {
        throw 'PHASE4_WSL2_DESCRIPTOR_SUBSTITUTION_FIXTURE_REJECTED'
    }
    try { $descriptor = $lines[0] | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_DESCRIPTOR_SUBSTITUTION_FIXTURE_REJECTED' }
    if ([string]$descriptor.identity_digest -cnotmatch
            '\Aexecution-environment:sha256:[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_DESCRIPTOR_SUBSTITUTION_FIXTURE_REJECTED'
    }
    return [pscustomobject][ordered]@{
        descriptor_json = [string]$lines[0]
        execution_environment_ref = [string]$descriptor.identity_digest
    }
}

function Assert-Phase4CargoIdentity {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$VerboseVersion)

    $lines = @($VerboseVersion -split '\r?\n' | Where-Object { $_.Length -gt 0 })
    if ($lines.Count -lt $script:ExpectedCargoIdentityLines.Count) {
        throw 'PHASE4_CARGO_IDENTITY_REJECTED'
    }
    for ($index = 0; $index -lt $script:ExpectedCargoIdentityLines.Count; $index++) {
        if ([string]$lines[$index] -cne [string]$script:ExpectedCargoIdentityLines[$index]) {
            throw 'PHASE4_CARGO_IDENTITY_REJECTED'
        }
    }
}

function Get-Phase4ListenerPids {
    param([Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port)

    Assert-Phase4RegularFile -Path $script:Netstat -Failure 'PHASE4_NETSTAT_BINARY_MISSING'
    $result = Invoke-Phase4Process -Executable $script:Netstat -Argument @('-ano', '-p', 'tcp') `
        -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
        -WorkingDirectory $script:RepositoryRoot -StandardInput $null -TimeoutSeconds 5 `
        -Failure 'PHASE4_NETSTAT_FAILED' `
        -OutputEncodingCodePage (Get-Culture).TextInfo.OEMCodePage
    $listenerPids = [Collections.Generic.HashSet[int]]::new()
    foreach ($line in @($result.stdout -split '\r?\n')) {
        if ($line -cnotmatch (
            '\A\s*TCP\s+(?<local>\S+):(?<local_port>[0-9]{1,5})\s+' +
            '(?<remote>\S+):(?<remote_port>[0-9]{1,5})\s+\S+\s+' +
            '(?<process_id>[0-9]+)\s*\z'
        )) {
            continue
        }
        if ([int]$Matches.local_port -eq $Port -and [int]$Matches.remote_port -eq 0 -and
            [int]$Matches.process_id -gt 0) {
            $null = $listenerPids.Add([int]$Matches.process_id)
        }
    }
    return @($listenerPids | Sort-Object)
}

function New-Phase4AvailablePort {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [int[]]$AdditionalForbidden
    )

    for ($attempt = 0; $attempt -lt 64; $attempt++) {
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
        try {
            $listener.Start()
            $port = [int]([Net.IPEndPoint]$listener.LocalEndpoint).Port
        }
        finally {
            $listener.Stop()
        }
        if ($port -notin ($script:ForbiddenPorts + $AdditionalForbidden) -and
            @(Get-Phase4ListenerPids -Port $port).Count -eq 0) {
            return $port
        }
    }
    throw 'PHASE4_LOOPBACK_PORT_UNAVAILABLE'
}

function Get-Phase4OwnerMarker {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    Assert-Phase4Directory -Path $RunRoot -Failure 'PHASE4_OWNER_ROOT_REJECTED'
    Assert-Phase4RegularFile -Path $MarkerPath -Failure 'PHASE4_OWNER_MARKER_REJECTED'
    try { $marker = [IO.File]::ReadAllText($MarkerPath, $script:Utf8) | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_OWNER_MARKER_REJECTED' }
    if ([string]$marker.owner -cne $script:OwnerKind -or
        [string]$marker.run_id -cne $RunId -or
        [string]$marker.root -cne [IO.Path]::GetFullPath($RunRoot) -or
        [string]$marker.postgres_executable -cne [IO.Path]::GetFullPath($script:Postgres) -or
        [string]$marker.postgres_sha256 -cne (Get-Phase4FileSha256 -Path $script:Postgres)) {
        throw 'PHASE4_OWNER_MARKER_REJECTED'
    }
    return $marker
}

function Get-Phase4OwnedPostgresProcessRecord {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath,
        [switch]$AllowExited,
        [switch]$AllowListenerAbsent
    )

    $null = Get-Phase4OwnerMarker -RunRoot $RunRoot -RunId $RunId -MarkerPath $MarkerPath
    Assert-Phase4Directory -Path $DataRoot -Failure 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    $pidFile = Join-Path $DataRoot 'postmaster.pid'
    Assert-Phase4RegularFile -Path $pidFile -Failure 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    $lines = @(Get-Content -LiteralPath $pidFile -TotalCount 4)
    if ($lines.Count -ne 4 -or [string]$lines[0] -cnotmatch '\A[1-9][0-9]*\z' -or
        [string]$lines[2] -cnotmatch '\A[1-9][0-9]*\z' -or
        [string]$lines[3] -cnotmatch '\A[1-9][0-9]*\z' -or
        [IO.Path]::GetFullPath([string]$lines[1]) -cne [IO.Path]::GetFullPath($DataRoot) -or
        [int]$lines[3] -ne $Port) {
        throw 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    }
    $postgresPid = [int]$lines[0]
    $process = Get-Process -Id $postgresPid -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        if ($AllowExited -and @(Get-Phase4ListenerPids -Port $Port).Count -eq 0) {
            return [pscustomobject][ordered]@{
                process_id = $postgresPid
                process_start_utc_ticks = $null
                process = $null
            }
        }
        throw 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    }
    if ([IO.Path]::GetFullPath($process.Path) -cne [IO.Path]::GetFullPath($script:Postgres)) {
        throw 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    }
    if ($null -eq $script:postgresOwnedProcessJob -or
        -not $script:postgresOwnedProcessJob.ContainsProcessHandle($process.Handle)) {
        throw 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    }
    $processStart = [DateTimeOffset]::new($process.StartTime.ToUniversalTime())
    if ([Math]::Abs($processStart.ToUnixTimeSeconds() - [long]$lines[2]) -gt 5) {
        throw 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    }
    $listeners = @(Get-Phase4ListenerPids -Port $Port)
    if ((-not $AllowListenerAbsent -and
            ($listeners.Count -ne 1 -or [int]$listeners[0] -ne $postgresPid)) -or
        ($AllowListenerAbsent -and $listeners.Count -ne 0 -and
            ($listeners.Count -ne 1 -or [int]$listeners[0] -ne $postgresPid))) {
        throw 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    }
    return [pscustomobject][ordered]@{
        process_id = $postgresPid
        process_start_utc_ticks = [long]$process.StartTime.ToUniversalTime().Ticks
        process = $process
    }
}

function Close-Phase4PostgresOwnedJob {
    param([switch]$TerminateRemaining)

    if ($null -eq $script:postgresOwnedProcessJob) { return }
    $ownedJob = $script:postgresOwnedProcessJob
    try {
        if ([long]$ownedJob.ActiveProcessCount() -ne 0) {
            if (-not $TerminateRemaining) { throw 'PHASE4_POSTGRES_STOP_PROOF_REJECTED' }
            Stop-Phase4OwnedProcessJob -OwnedProcess $ownedJob `
                -Failure 'PHASE4_POSTGRES_STOP_PROOF_REJECTED'
        }
        Close-Phase4OwnedProcessJob -OwnedProcess $ownedJob `
            -Failure 'PHASE4_POSTGRES_STOP_PROOF_REJECTED'
    }
    finally {
        $ownedJob.Dispose()
        $script:postgresOwnedProcessJob = $null
    }
}

function Start-Phase4Postgres {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    $marker = Get-Phase4OwnerMarker -RunRoot $RunRoot -RunId $RunId -MarkerPath $MarkerPath
    Assert-Phase4Directory -Path $DataRoot -Failure 'PHASE4_POSTGRES_DATA_REJECTED'
    if (@(Get-Phase4ListenerPids -Port $Port).Count -ne 0) {
        throw 'PHASE4_POSTGRES_PORT_COLLISION'
    }
    $log = Assert-Phase4ContainedPath -Root $RunRoot -Path (Join-Path $RunRoot 'postgres.log') `
        -Failure 'PHASE4_POSTGRES_LOG_REJECTED'
    $options = "-p $Port -h 127.0.0.1 -c ssl=off -c fsync=on -c synchronous_commit=on " +
        '-c full_page_writes=on -c max_prepared_transactions=0'
    $script:postgresStartMayOwnProcess = $true
    $script:postgresLauncherTerminalProven = $false
    $launcher = $null
    try {
        $launcher = Start-Phase4OwnedProcessJob -Executable $script:PgCtl -Argument @(
            '-D', $DataRoot, '-l', $log, '-o', $options, '-W', 'start'
        ) -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
            -WorkingDirectory $RunRoot -Failure 'PHASE4_POSTGRES_START_FAILED' `
            -OutputEncodingCodePage $script:WindowsOemCodePage
        $script:postgresOwnedProcessJob = $launcher
        $launcher.StandardInput.Close()
        if (-not $launcher.WaitForExit(60000)) {
            Stop-Phase4OwnedProcessJob -OwnedProcess $launcher `
                -Failure 'PHASE4_POSTGRES_START_PROCESS_TREE_CLEANUP_REJECTED'
            $script:LastPhase4ProcessTreeTerminationProven = $true
            throw 'PHASE4_POSTGRES_START_FAILED'
        }
        $script:postgresLauncherTerminalProven = $true
        if ([int]$launcher.ExitCode -ne 0) { throw 'PHASE4_POSTGRES_START_FAILED' }
    }
    catch {
        $script:postgresLauncherTerminalProven =
            ($null -ne $launcher -and [bool]$launcher.HasExited) -or
            [bool]$script:LastPhase4ProcessTreeTerminationProven
        throw
    }

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(20)
    $postgresPid = $null
    $process = $null
    $listenerReady = $false
    do {
        $pidFile = Join-Path $DataRoot 'postmaster.pid'
        if (Test-Path -LiteralPath $pidFile -PathType Leaf) {
            $pidText = (Get-Content -LiteralPath $pidFile -TotalCount 1).Trim()
            if ($pidText -cmatch '\A[1-9][0-9]*\z') {
                $postgresPid = [int]$pidText
                $process = Get-Process -Id $postgresPid -ErrorAction SilentlyContinue
                $listeners = @(Get-Phase4ListenerPids -Port $Port)
                if ($null -ne $process -and $listeners.Count -eq 1 -and $listeners[0] -eq $postgresPid) {
                    $listenerReady = $true
                    break
                }
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)

    if (-not $listenerReady -or $null -eq $process) {
        throw 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    }
    $owned = Get-Phase4OwnedPostgresProcessRecord -RunRoot $RunRoot -RunId $RunId `
        -Port $Port -DataRoot $DataRoot -MarkerPath $MarkerPath
    $script:postgresProcessIdentity = [pscustomobject][ordered]@{
        process_id = [int]$owned.process_id
        process_start_utc_ticks = [long]$owned.process_start_utc_ticks
    }
    return $script:postgresProcessIdentity
}

function Assert-Phase4OwnedLivePostgres {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    return Get-Phase4OwnedPostgresProcessRecord -RunRoot $RunRoot -RunId $RunId `
        -Port $Port -DataRoot $DataRoot -MarkerPath $MarkerPath
}

function Stop-Phase4Postgres {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    $null = Get-Phase4OwnerMarker -RunRoot $RunRoot -RunId $RunId -MarkerPath $MarkerPath
    Assert-Phase4Directory -Path $DataRoot -Failure 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    $pidFile = Join-Path $DataRoot 'postmaster.pid'
    $absenceDeadline = [DateTimeOffset]::UtcNow.AddSeconds(5)
    $absenceObservations = 0
    while (-not (Test-Path -LiteralPath $pidFile -PathType Leaf) -and
        [DateTimeOffset]::UtcNow -lt $absenceDeadline) {
        $status = Invoke-Phase4Process -Executable $script:PgCtl -Argument @(
            '-D', $DataRoot, 'status'
        ) -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
            -WorkingDirectory $RunRoot -StandardInput $null -TimeoutSeconds 10 `
            -Failure 'PHASE4_POSTGRES_STOP_PROOF_REJECTED' -AllowNonZeroExit `
            -OutputEncodingCodePage $script:WindowsOemCodePage
        if (-not $script:postgresLauncherTerminalProven -or
            [int]$status.exit_code -ne 3 -or
            @(Get-Phase4ListenerPids -Port $Port).Count -ne 0) {
            throw 'PHASE4_POSTGRES_STOP_PROOF_REJECTED'
        }
        $absenceObservations += 1
        Start-Sleep -Milliseconds 250
    }
    if (-not (Test-Path -LiteralPath $pidFile -PathType Leaf)) {
        if ($absenceObservations -lt 10) { throw 'PHASE4_POSTGRES_STOP_PROOF_REJECTED' }
        if ($null -ne $script:postgresOwnedProcessJob -and
            [long]$script:postgresOwnedProcessJob.ActiveProcessCount() -ne 0) {
            Close-Phase4PostgresOwnedJob -TerminateRemaining
        }
        if ($null -ne $script:postgresProcessIdentity) {
            $null = Assert-Phase4ProcessIdentityAbsent `
                -ProcessId ([int]$script:postgresProcessIdentity.process_id) `
                -ProcessStartUtcTicks ([long]$script:postgresProcessIdentity.process_start_utc_ticks)
        }
        Close-Phase4PostgresOwnedJob
        $script:postgresStartMayOwnProcess = $false
        $script:postgresLauncherTerminalProven = $false
        $script:postgresProcessIdentity = $null
        return
    }
    $owned = Get-Phase4OwnedPostgresProcessRecord -RunRoot $RunRoot -RunId $RunId `
        -Port $Port -DataRoot $DataRoot -MarkerPath $MarkerPath -AllowExited `
        -AllowListenerAbsent
    if ($null -eq $owned.process) {
        if ($null -ne $script:postgresProcessIdentity) {
            $null = Assert-Phase4ProcessIdentityAbsent `
                -ProcessId ([int]$script:postgresProcessIdentity.process_id) `
                -ProcessStartUtcTicks ([long]$script:postgresProcessIdentity.process_start_utc_ticks)
        }
        if ($null -ne $script:postgresOwnedProcessJob -and
            [long]$script:postgresOwnedProcessJob.ActiveProcessCount() -ne 0) {
            Close-Phase4PostgresOwnedJob -TerminateRemaining
        }
        Close-Phase4PostgresOwnedJob
        $script:postgresStartMayOwnProcess = $false
        $script:postgresLauncherTerminalProven = $false
        $script:postgresProcessIdentity = $null
        return
    }
    if ($null -ne $script:postgresProcessIdentity -and
        ([int]$owned.process_id -ne [int]$script:postgresProcessIdentity.process_id -or
         [long]$owned.process_start_utc_ticks -ne
            [long]$script:postgresProcessIdentity.process_start_utc_ticks)) {
        throw 'PHASE4_POSTGRES_OWNERSHIP_REJECTED'
    }
    try {
        $null = Invoke-Phase4Process -Executable $script:PgCtl -Argument @(
            '-D', $DataRoot, '-m', 'fast', '-W', 'stop'
        ) -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
            -WorkingDirectory $RunRoot -StandardInput $null -TimeoutSeconds 60 `
            -Failure 'PHASE4_POSTGRES_STOP_FAILED' -DoNotWaitForRedirectEof `
            -OutputEncodingCodePage $script:WindowsOemCodePage
    }
    catch {
        Close-Phase4PostgresOwnedJob -TerminateRemaining
        throw
    }
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(15)
    while (@(Get-Phase4ListenerPids -Port $Port).Count -ne 0 -and
        [DateTimeOffset]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
    if (@(Get-Phase4ListenerPids -Port $Port).Count -ne 0) {
        throw 'PHASE4_POSTGRES_STOP_PROOF_REJECTED'
    }
    $null = Assert-Phase4ProcessIdentityAbsent -ProcessId ([int]$owned.process_id) `
        -ProcessStartUtcTicks ([long]$owned.process_start_utc_ticks)
    Close-Phase4PostgresOwnedJob
    $script:postgresStartMayOwnProcess = $false
    $script:postgresLauncherTerminalProven = $false
    $script:postgresProcessIdentity = $null
}

function Invoke-Phase4Psql {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$Sql,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    $environment = New-Phase4ClosedEnvironment -Values ([ordered]@{
        PGPASSWORD = $Password
        PGCONNECT_TIMEOUT = '5'
        PGCLIENTENCODING = 'UTF8'
        LANG = 'C'
        LC_ALL = 'C'
    })
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        $result = Invoke-Phase4Process -Executable $script:Psql -Argument @(
            '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-h', '127.0.0.1', '-p', [string]$Port,
            '-U', 'runtime_bootstrap', '-d', $Database, '-c', $Sql
        ) -Environment $environment -WorkingDirectory $WorkingDirectory -StandardInput $null `
            -TimeoutSeconds 30 -Failure $Failure -AllowNonZeroExit
        if ([int]$result.exit_code -eq 0) { return ([string]$result.stdout).Trim() }
        if ([int]$result.exit_code -ne 2 -or $attempt -eq 3) { throw $Failure }
        Start-Sleep -Milliseconds (100 * $attempt)
    }
    throw $Failure
}

function Get-Phase4WslDurableEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z') { throw 'PHASE4_TASK_REF_REJECTED' }
    $sql = @"
WITH target AS (SELECT decode('$TaskRef','hex') AS task_ref),
validated_environment AS (
    SELECT environment.*
      FROM target t
      CROSS JOIN LATERAL foreman_execution.read_execution_environment_rows_v1(t.task_ref)
        AS environment
),
raw_environment AS (
    SELECT environment.*
      FROM ONLY foreman_execution.execution_environments AS environment, target t
     WHERE environment.task_ref=t.task_ref
)
SELECT pg_catalog.jsonb_build_object(
    'promotion_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'attempt_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref),
    'attempt_number', (SELECT a.attempt_number FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'attempt_id', (SELECT a.attempt_id FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'attempt_execution_environment_ref', (SELECT a.execution_environment_ref FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'provider_effect_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.provider_dispatch_claims d, target t WHERE d.task_ref=t.task_ref),
    'worker_thread_dispatch_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.provider_dispatch_claims d, target t WHERE d.task_ref=t.task_ref AND d.operation_kind='WORKER_THREAD'),
    'worker_turn_dispatch_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.provider_dispatch_claims d, target t WHERE d.task_ref=t.task_ref AND d.operation_kind='WORKER_TURN'),
    'review_thread_dispatch_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.provider_dispatch_claims d, target t WHERE d.task_ref=t.task_ref AND d.operation_kind='REVIEW_THREAD'),
    'review_turn_dispatch_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.provider_dispatch_claims d, target t WHERE d.task_ref=t.task_ref AND d.operation_kind='REVIEW_TURN'),
    'observation_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref),
    'artifact_outbox_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.staged_artifact_references o, target t WHERE o.task_ref=t.task_ref),
    'pending_worker_claim_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.pending_worker_claims p, target t WHERE p.task_ref=t.task_ref),
    'thread_count', (SELECT pg_catalog.count(DISTINCT o.thread_id) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref),
    'turn_count', (SELECT pg_catalog.count(DISTINCT o.turn_id) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref AND o.turn_id IS NOT NULL),
    'reconciled_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref AND o.observation_kind='RECONCILED'),
    'terminal_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref AND o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')),
    'environment_count', (SELECT pg_catalog.count(*) FROM raw_environment),
    'validated_environment_count', (SELECT pg_catalog.count(*) FROM validated_environment),
    'environment_ref', (SELECT environment_ref FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'canonical_descriptor', (SELECT canonical_descriptor FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'descriptor_schema', (SELECT descriptor_schema FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'environment_kind', (SELECT environment_kind FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'distribution', (SELECT distribution FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'linux_repository_path', (SELECT linux_repository_path FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'repository_head', (SELECT repository_head FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'repository_identity_ref', (SELECT repository_identity_ref FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'launcher_path', (SELECT launcher_path FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'launcher_version', (SELECT launcher_version FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'launcher_sha256', (SELECT pg_catalog.encode(launcher_digest,'hex') FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'node_path', (SELECT node_path FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'node_version', (SELECT node_version FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'node_sha256', (SELECT pg_catalog.encode(node_digest,'hex') FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'git_path', (SELECT git_path FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'git_version', (SELECT git_version FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'git_sha256', (SELECT pg_catalog.encode(git_digest,'hex') FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'supervisor_path', (SELECT supervisor_path FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'supervisor_sha256', (SELECT pg_catalog.encode(supervisor_digest,'hex') FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'credential_authority_kind', (SELECT credential_authority_kind FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'credential_authority_ref', (SELECT credential_authority_ref FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'process_fence_ref', (SELECT process_fence_identity_ref FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'verification_toolchain_ref', (SELECT verification_toolchain_identity_ref FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'path_mapping_windows_path', (SELECT path_mapping_windows_path FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'path_mapping_linux_path', (SELECT path_mapping_linux_path FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'path_mapping_ref', (SELECT path_mapping_ref FROM raw_environment ORDER BY attempt_number DESC LIMIT 1),
    'execution_domain_digest', (SELECT pg_catalog.encode(execution_domain_digest,'hex') FROM raw_environment ORDER BY attempt_number DESC LIMIT 1)
);
"@
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database -Sql $sql `
        -WorkingDirectory $WorkingDirectory -Failure 'PHASE4_WSL2_DURABLE_EVIDENCE_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_DURABLE_EVIDENCE_REJECTED' }
    return [pscustomobject][ordered]@{
        raw = $raw
        digest = Get-Phase4StringSha256 -Value $raw
        value = $value
    }
}

function Assert-Phase4WslProviderEffectsUnchanged {
    param(
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After
    )

    if ([long]$After.attempt_count -ne [long]$Before.attempt_count -or
        [string]$After.attempt_id -cne [string]$Before.attempt_id -or
        [long]$After.provider_effect_count -ne [long]$Before.provider_effect_count -or
        [long]$After.worker_thread_dispatch_count -ne
            [long]$Before.worker_thread_dispatch_count -or
        [long]$After.worker_turn_dispatch_count -ne
            [long]$Before.worker_turn_dispatch_count -or
        [long]$After.review_thread_dispatch_count -ne
            [long]$Before.review_thread_dispatch_count -or
        [long]$After.review_turn_dispatch_count -ne
            [long]$Before.review_turn_dispatch_count -or
        [long]$After.environment_count -ne [long]$Before.environment_count -or
        [long]$After.validated_environment_count -ne
            [long]$Before.validated_environment_count -or
        [long]$After.artifact_outbox_count -ne [long]$Before.artifact_outbox_count -or
        [long]$After.pending_worker_claim_count -ne
            [long]$Before.pending_worker_claim_count -or
        [string]$After.environment_ref -cne [string]$Before.environment_ref -or
        [string]$After.canonical_descriptor -cne [string]$Before.canonical_descriptor) {
        throw 'PHASE4_WSL2_PROVIDER_EFFECTS_CHANGED_AFTER_HARD_STOP'
    }
}

function Get-Phase4WslProviderPreflightEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$ProjectId,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)][string]$ExpectedWorktreeRef
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ProjectId -cnotmatch '\A[a-z0-9][a-z0-9._-]{1,63}\z' -or
        $ExpectedWorktreeRef -cnotmatch '\Aworktree:sha256:[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_PROVIDER_PREFLIGHT_IDENTITY_REJECTED'
    }
    $sql = @"
WITH managed AS (
    SELECT evidence.*
      FROM foreman_execution.read_managed_evidence_v1(
          decode('$TaskRef','hex'),1::smallint
      )
        AS evidence
     WHERE evidence.payload_schema='lattice.wsl2-zero-model-preflight/1.0'
       AND evidence.producer_id='lattice-runtime-wsl2-preflight-bridge'
), replay AS (
    SELECT replay.record_digest, replay.ledger_event_sequence,
           pg_catalog.encode(replay.ledger_event_digest,'hex') AS ledger_event_digest,
           replay.recorded_at
      FROM foreman_execution.read_task_replay_v1(decode('$TaskRef','hex')) AS replay
     WHERE replay.record_kind='ARTIFACT_REFERENCE'
       AND replay.attempt_number=1
), matches AS (
    SELECT managed.project_id, managed.evidence_kind, managed.media_type,
           managed.payload_schema, managed.producer_id, managed.producer_version,
           pg_catalog.encode(managed.producer_digest,'hex') AS producer_digest,
           managed.created_at,
           pg_catalog.convert_from(managed.evidence_bytes,'UTF8') AS evidence_json,
           pg_catalog.encode(managed.content_digest,'hex') AS content_digest,
           pg_catalog.encode(managed.descriptor_digest,'hex') AS descriptor_digest,
           replay.ledger_event_sequence::text AS ledger_event_sequence,
           replay.ledger_event_digest, replay.recorded_at AS ledger_recorded_at
      FROM managed
      JOIN replay ON replay.record_digest=managed.descriptor_digest
)
SELECT pg_catalog.jsonb_build_object(
    'count', pg_catalog.count(*),
    'records', COALESCE(
        pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            'project_id',project_id,'evidence_kind',evidence_kind,
            'media_type',media_type,'payload_schema',payload_schema,
            'producer_id',producer_id,'producer_version',producer_version,
            'producer_digest',producer_digest,'created_at',created_at,
            'evidence_json',evidence_json,'content_digest',content_digest,
            'descriptor_digest',descriptor_digest,
            'ledger_event_sequence',ledger_event_sequence,
            'ledger_event_digest',ledger_event_digest,
            'ledger_recorded_at',ledger_recorded_at
        ) ORDER BY ledger_event_sequence::numeric, descriptor_digest), '[]'::jsonb
    )
) FROM matches;
"@
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database `
        -Sql $sql -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_WSL2_PROVIDER_PREFLIGHT_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED' }
    $records = @($value.records)
    if ([long]$value.count -ne 1 -or $records.Count -ne 1) {
        throw 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED'
    }
    $record = $records[0]
    Assert-Phase4RegularFile -Path $script:Wsl2PreflightBridge `
        -Failure 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED'
    if ([string]$record.project_id -cne $ProjectId -or
        [string]$record.evidence_kind -cne 'WORKER_LIFECYCLE' -or
        [string]$record.media_type -cne 'application/json' -or
        [string]$record.payload_schema -cne 'lattice.wsl2-zero-model-preflight/1.0' -or
        [string]$record.producer_id -cne 'lattice-runtime-wsl2-preflight-bridge' -or
        [string]$record.producer_version -cne '1.0' -or
        [string]$record.producer_digest -cne
            (Get-Phase4FileSha256 -Path $script:Wsl2PreflightBridge) -or
        [string]$record.content_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$record.descriptor_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$record.ledger_event_sequence -cnotmatch '\A[1-9][0-9]*\z' -or
        [string]$record.ledger_event_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$record.content_digest -cne
            (Get-Phase4StringSha256 -Value ([string]$record.evidence_json)) -or
        $script:Utf8.GetByteCount([string]$record.evidence_json) -gt 1048576) {
        throw 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED'
    }
    try { $receipt = [string]$record.evidence_json | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED' }
    Assert-Phase4NoCredentialShapedJsonStrings -Value $receipt `
        -Failure 'PHASE4_WSL2_PROVIDER_PREFLIGHT_SECRET_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $receipt -Expected @(
        'schema', 'status', 'task_ref', 'attempt', 'worktree_ref',
        'execution_environment_ref', 'descriptor_digest', 'distribution_identity_ref',
        'linux_cwd', 'repository_head', 'repository_identity', 'codex_home_digest',
        'credential_authority_ref', 'credential_seal_digest',
        'verification_toolchain_ref', 'immutable_snapshot_ref', 'sandbox_policy_ref',
        'privilege_boundary_ref', 'process_fence', 'isolation', 'probes',
        'effect_counters', 'provider_effect_count', 'bounds', 'timeout', 'continuation',
        'connector_auth_ready', 'receipt_digest'
    ) -Failure 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED'
    $descriptor = $Materialization.descriptor
    $environmentRef = [string]$Materialization.record.execution_environment_ref
    if ([string]$receipt.schema -cne 'lattice.wsl2-zero-model-preflight/1.0' -or
        [string]$receipt.status -cne 'PASS' -or [string]$receipt.task_ref -cne $TaskRef -or
        [int]$receipt.attempt -ne 1 -or
        [string]$receipt.worktree_ref -cne $ExpectedWorktreeRef -or
        [string]$receipt.execution_environment_ref -cne $environmentRef -or
        [string]$receipt.descriptor_digest -cne $environmentRef -or
        [string]$receipt.distribution_identity_ref -cne
            [string]$descriptor.distribution_identity.identity_digest -or
        [string]$receipt.linux_cwd -cne [string]$descriptor.linux.cwd -or
        [string]$receipt.repository_head -cne [string]$descriptor.linux.repository_head -or
        [string]$receipt.repository_identity -cne [string]$descriptor.linux.repository_identity -or
        [string]$receipt.credential_authority_ref -cne
            [string]$descriptor.credential_authority.authority_digest -or
        [string]$receipt.credential_seal_digest -cnotmatch
            '\Acredential-seal:sha256:[0-9a-f]{64}\z' -or
        [string]$receipt.verification_toolchain_ref -cne
            [string]$descriptor.verification_toolchain.identity_digest -or
        [string]$receipt.immutable_snapshot_ref -cne
            [string]$descriptor.immutable_snapshot.snapshot_digest -or
        [string]$receipt.sandbox_policy_ref -cne
            [string]$descriptor.sandbox_policy.policy_digest -or
        [string]$receipt.privilege_boundary_ref -cne
            [string]$descriptor.privilege_boundary.boundary_digest -or
        [long]$receipt.provider_effect_count -ne 0 -or
        [long]$receipt.effect_counters.account_read -ne 0 -or
        [long]$receipt.effect_counters.thread_start -ne 0 -or
        [long]$receipt.effect_counters.turn_start -ne 0 -or
        [long]$receipt.effect_counters.provider_effect_count -ne 0 -or
        [bool]$receipt.connector_auth_ready -or
        [int]$receipt.continuation.attempt -ne 1 -or
        $null -ne $receipt.continuation.retry_of -or
        $null -ne $receipt.continuation.reconnect_of -or
        [string]$receipt.receipt_digest -cnotmatch
            '\Awsl2-preflight:sha256:[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED'
    }
    Assert-Phase4ExactJsonProperties -Value $receipt.process_fence -Expected @(
        'fence', 'authority_ref', 'service_unit', 'cgroup_path', 'cgroup_version',
        'delegated', 'boot_id_digest', 'supervisor_zero_descendants', 'outer_post_exit'
    ) -Failure 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $receipt.process_fence.outer_post_exit `
        -Expected @(
            'unit', 'active_state', 'sub_state', 'result', 'cgroup_path', 'delegate',
            'cgroup_exists', 'populated'
    ) -Failure 'PHASE4_WSL2_PROVIDER_PREFLIGHT_EVIDENCE_REJECTED'
    $fence = [string]$receipt.process_fence.fence
    if ($fence -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_PROVIDER_PREFLIGHT_FENCE_REJECTED'
    }
    $expectedPreflightUnit = [string]$descriptor.process_fence.unit_prefix +
        '-preflight-' + $fence.Substring(0, 12) + '.service'
    $providerUnit = [string]$descriptor.process_fence.unit_prefix +
        '-provider-' + $fence.Substring(0, 12) + '.service'
    $expectedPreflightCgroup = Get-Phase4CanonicalUserServiceCgroupPath `
        -OwnerUid ([long]$descriptor.verification_toolchain.owner_uid) `
        -Unit $expectedPreflightUnit
    $preflightCgroup = [string]$receipt.process_fence.cgroup_path
    $outer = $receipt.process_fence.outer_post_exit
    $cgroupClosed = (
        $outer.cgroup_exists -is [bool] -and
        ((-not [bool]$outer.cgroup_exists -and $null -eq $outer.populated) -or
         ([bool]$outer.cgroup_exists -and $null -ne $outer.populated -and
          [long]$outer.populated -eq 0))
    )
    if ([string]$descriptor.process_fence.unit_prefix -cne
            ('lattice-wsl2-' + $TaskRef.Substring(0, 16)) -or
        [string]$receipt.process_fence.authority_ref -cne
            [string]$descriptor.process_fence.identity_digest -or
        [string]$receipt.process_fence.service_unit -cne $expectedPreflightUnit -or
        [string]$outer.unit -cne $expectedPreflightUnit -or
        $preflightCgroup -cne $expectedPreflightCgroup -or
        [string]$outer.cgroup_path -cne $preflightCgroup -or
        [long]$receipt.process_fence.cgroup_version -ne 2 -or
        [bool]$receipt.process_fence.delegated -or
        [string]$receipt.process_fence.boot_id_digest -cnotmatch
            '\Awsl-boot:sha256:[0-9a-f]{64}\z' -or
        -not [bool]$receipt.process_fence.supervisor_zero_descendants -or
        [string]$outer.active_state -cne 'inactive' -or
        [string]$outer.sub_state -cne 'dead' -or [string]$outer.result -cne 'success' -or
        [string]$outer.delegate -cne 'no' -or -not $cgroupClosed -or
        [string]$receipt.isolation.root -cne
            [string]$descriptor.verification_toolchain.isolation_root) {
        throw 'PHASE4_WSL2_PROVIDER_PREFLIGHT_FENCE_REJECTED'
    }
    return [pscustomobject][ordered]@{
        receipt = $receipt
        receipt_json = [string]$record.evidence_json
        receipt_digest = [string]$receipt.receipt_digest
        artifact_ref = 'evidence:sha256:' + [string]$record.descriptor_digest
        artifact_descriptor_digest = [string]$record.descriptor_digest
        artifact_content_digest = [string]$record.content_digest
        sql_evidence_digest = Get-Phase4StringSha256 -Value $raw
        process_fence = $fence
        boot_id_digest = [string]$receipt.process_fence.boot_id_digest
        provider_unit = $providerUnit
        ledger_event_sequence = [string]$record.ledger_event_sequence
        ledger_event_digest = [string]$record.ledger_event_digest
        ledger_recorded_at = [string]$record.ledger_recorded_at
    }
}

function Close-Phase4WslTaskOwnedUnits {
    param(
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)]$Materialization
    )

    $descriptor = $Materialization.descriptor
    $unitPrefix = [string]$descriptor.process_fence.unit_prefix
    $ownerUid = [long]$descriptor.verification_toolchain.owner_uid
    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $unitPrefix -cne ('lattice-wsl2-' + $TaskRef.Substring(0, 16)) -or
        $ownerUid -le 0 -or
        [string]$descriptor.process_fence.user_runtime_dir -cne ('/run/user/' + $ownerUid)) {
        throw 'PHASE4_WSL2_TASK_UNIT_CLEANUP_IDENTITY_REJECTED'
    }
    $inventory = Invoke-Phase4WslProcess `
        -Executable ([string]$descriptor.process_fence.systemctl_path) -Argument @(
            '--user', '--no-pager', '--plain', '--no-legend', '--all',
            'list-units', ($unitPrefix + '-preflight-*.service'),
            ($unitPrefix + '-provider-*.service'),
            ($unitPrefix + '-reviewer-*.service')
        ) -Environment ([ordered]@{
            DBUS_SESSION_BUS_ADDRESS = 'unix:path:' +
                [string]$descriptor.process_fence.user_runtime_dir + '/bus'
        }) -LinuxHome $script:Wsl2HarnessHome -LinuxTemp $script:Wsl2HarnessTemp `
        -StandardInput $null -TimeoutSeconds 15 `
        -Failure 'PHASE4_WSL2_TASK_UNIT_INVENTORY_FAILED'
    if ([long]$inventory.stderr_byte_count -ne 0 -or
        $script:Utf8.GetByteCount([string]$inventory.stdout) -gt 65536) {
        throw 'PHASE4_WSL2_TASK_UNIT_INVENTORY_REJECTED'
    }
    $unitRegex = '\A' + [regex]::Escape($unitPrefix) + '-' +
        '(?:preflight|provider|reviewer)' +
        '-[0-9a-f]{12}\.service\z'
    $units = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($line in @([string]$inventory.stdout -split '\r?\n' |
            Where-Object { $_.Length -gt 0 })) {
        $unit = @($line -split '\s+')[0]
        if ($unit -cnotmatch $unitRegex -or -not $units.Add($unit) -or $units.Count -gt 12) {
            throw 'PHASE4_WSL2_TASK_UNIT_INVENTORY_REJECTED'
        }
    }
    $closed = [Collections.Generic.List[object]]::new()
    foreach ($unit in @($units | Sort-Object -CaseSensitive)) {
        $before = Get-Phase4WslSystemdUnitState -Materialization $Materialization -Unit $unit
        $canonicalCgroupPath = Get-Phase4CanonicalUserServiceCgroupPath `
            -OwnerUid $ownerUid -Unit $unit
        if ((-not [string]::IsNullOrWhiteSpace([string]$before.cgroup_path)) -and
            [string]$before.cgroup_path -cne $canonicalCgroupPath) {
            throw 'PHASE4_WSL2_PROVIDER_CGROUP_IDENTITY_REJECTED'
        }
        $actions = [Collections.Generic.List[object]]::new()
        if ([string]$before.active_state -cne 'inactive' -or [long]$before.main_pid -ne 0) {
            foreach ($step in @(
                [pscustomobject]@{
                    name = 'TERM'
                    arguments = @('kill', '--kill-who=all', '--signal=SIGTERM')
                },
                [pscustomobject]@{ name = 'STOP'; arguments = @('--no-block', 'stop') }
            )) {
                $action = Invoke-Phase4WslProcess `
                    -Executable ([string]$descriptor.process_fence.systemctl_path) `
                    -Argument (@('--user', '--no-pager') + @($step.arguments) + @($unit)) `
                    -Environment ([ordered]@{
                        DBUS_SESSION_BUS_ADDRESS = 'unix:path:' +
                            [string]$descriptor.process_fence.user_runtime_dir + '/bus'
                    }) -LinuxHome $script:Wsl2HarnessHome -LinuxTemp $script:Wsl2HarnessTemp `
                    -StandardInput $null -TimeoutSeconds 15 `
                    -Failure 'PHASE4_WSL2_TASK_UNIT_STOP_FAILED'
                if ([long]$action.stderr_byte_count -ne 0) {
                    throw 'PHASE4_WSL2_TASK_UNIT_STOP_REJECTED'
                }
                $actions.Add([pscustomobject][ordered]@{
                    action = [string]$step.name
                    exit_code = [int]$action.exit_code
                    stdout_sha256 = [string]$action.stdout_sha256
                    stderr_sha256 = [string]$action.stderr_sha256
                })
            }
        }
        $settleDeadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
        do {
            $after = Get-Phase4WslSystemdUnitState `
                -Materialization $Materialization -Unit $unit
            if ([string]$after.active_state -ceq 'inactive' -and
                [string]$after.sub_state -ceq 'dead' -and [long]$after.main_pid -eq 0) {
                break
            }
            Start-Sleep -Milliseconds 100
        } while ($actions.Count -ne 0 -and [DateTimeOffset]::UtcNow -lt $settleDeadline)
        if (([string]$after.active_state -cne 'inactive' -or
            [string]$after.sub_state -cne 'dead' -or [long]$after.main_pid -ne 0) -and
            $actions.Count -ne 0) {
            foreach ($step in @(
                [pscustomobject]@{
                    name = 'KILL'
                    arguments = @('kill', '--kill-who=all', '--signal=SIGKILL')
                },
                [pscustomobject]@{
                    name = 'FORCE_STOP'
                    arguments = @('--no-block', 'stop', '--force')
                }
            )) {
                $action = Invoke-Phase4WslProcess `
                    -Executable ([string]$descriptor.process_fence.systemctl_path) `
                    -Argument (@('--user', '--no-pager') + @($step.arguments) + @($unit)) `
                    -Environment ([ordered]@{
                        DBUS_SESSION_BUS_ADDRESS = 'unix:path:' +
                            [string]$descriptor.process_fence.user_runtime_dir + '/bus'
                    }) -LinuxHome $script:Wsl2HarnessHome -LinuxTemp $script:Wsl2HarnessTemp `
                    -StandardInput $null -TimeoutSeconds 15 `
                    -Failure 'PHASE4_WSL2_TASK_UNIT_FORCE_STOP_FAILED'
                if ([long]$action.stderr_byte_count -ne 0) {
                    throw 'PHASE4_WSL2_TASK_UNIT_FORCE_STOP_REJECTED'
                }
                $actions.Add([pscustomobject][ordered]@{
                    action = [string]$step.name
                    exit_code = [int]$action.exit_code
                    stdout_sha256 = [string]$action.stdout_sha256
                    stderr_sha256 = [string]$action.stderr_sha256
                })
            }
            $settleDeadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
            do {
                $after = Get-Phase4WslSystemdUnitState `
                    -Materialization $Materialization -Unit $unit
                if ([string]$after.active_state -ceq 'inactive' -and
                    [string]$after.sub_state -ceq 'dead' -and [long]$after.main_pid -eq 0) {
                    break
                }
                Start-Sleep -Milliseconds 100
            } while ([DateTimeOffset]::UtcNow -lt $settleDeadline)
        }
        if ([string]$after.active_state -cne 'inactive' -or
            [string]$after.sub_state -cne 'dead' -or [long]$after.main_pid -ne 0 -or
            ((-not [string]::IsNullOrWhiteSpace([string]$after.cgroup_path)) -and
                [string]$after.cgroup_path -cne $canonicalCgroupPath)) {
            throw 'PHASE4_WSL2_FAILURE_SUBTREE_RECONCILIATION_REQUIRED'
        }
        $cgroupEvidence = Get-Phase4WslCgroupSnapshot `
            -Materialization $Materialization -Unit $unit `
            -CgroupPath $canonicalCgroupPath -OldMarkers $null
        $cgroupClosed = (
            ((-not [bool]$cgroupEvidence.value.exists -and
                $null -eq $cgroupEvidence.value.populated) -or
             ([bool]$cgroupEvidence.value.exists -and
                [long]$cgroupEvidence.value.populated -eq 0)) -and
            @($cgroupEvidence.value.process_markers).Count -eq 0
        )
        if (-not $cgroupClosed) {
            throw 'PHASE4_WSL2_FAILURE_SUBTREE_RECONCILIATION_REQUIRED'
        }
        $closed.Add([pscustomobject][ordered]@{
            unit = $unit
            role = $(
                if ($unit.Contains('-preflight-', [StringComparison]::Ordinal)) { 'PREFLIGHT' }
                elseif ($unit.Contains('-provider-', [StringComparison]::Ordinal)) { 'PROVIDER' }
                else { 'REVIEWER' }
            )
            stop_issued = ($actions.Count -gt 0)
            cleanup_actions = @($actions)
            before = $before
            after = $after
            cgroup_closed = $true
            cgroup_evidence = $cgroupEvidence
        })
    }
    return [pscustomobject][ordered]@{
        schema = 'lattice.phase4-wsl2-task-unit-cleanup/1.0'
        status = 'CLOSED'
        gate_passed = $true
        task_unit_count = $units.Count
        active_unit_count = 0
        units = @($closed)
        inventory_stdout_sha256 = Get-Phase4StringSha256 -Value ([string]$inventory.stdout)
    }
}

function Invoke-Phase4WslFailureSubtreeCleanup {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$ProjectId,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)][string]$NodeExecutable,
        [Parameter(Mandatory = $true)][ValidateRange(0, 16)][int]$ProviderEffectCount
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ProjectId -cnotmatch '\A[a-z0-9][a-z0-9._-]{1,63}\z' -or
        [string]$Materialization.record.execution_environment_ref -cnotmatch
            '\Aexecution-environment:sha256:[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_REJECTED'
    }
    Assert-Phase4RegularFile -Path $NodeExecutable `
        -Failure 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_REJECTED'
    Assert-Phase4RegularFile -Path $script:Wsl2ProviderSubtreeReconciler `
        -Failure 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_REJECTED'
    $sql = @"
WITH managed AS (
    SELECT evidence.project_id, evidence.payload_schema,
           pg_catalog.convert_from(evidence.evidence_bytes,'UTF8') AS evidence_json,
           pg_catalog.encode(evidence.content_digest,'hex') AS content_digest,
           pg_catalog.encode(evidence.descriptor_digest,'hex') AS descriptor_digest
      FROM foreman_execution.read_managed_evidence_v1(
          decode('$TaskRef','hex'),1::smallint
      )
        AS evidence
     WHERE evidence.payload_schema IN (
         'lattice.wsl2-zero-model-preflight/1.0',
         'lattice.wsl2-provider-subtree-marker/1.0',
         'lattice.wsl2-provider-subtree-receipt/1.0',
         'lattice.wsl2-provider-subtree-reconciliation/1.0'
     )
), decoded AS (
    SELECT managed.*, managed.evidence_json::jsonb AS payload FROM managed
), open_markers AS (
    SELECT marker.*
      FROM decoded marker
     WHERE marker.payload_schema='lattice.wsl2-provider-subtree-marker/1.0'
       AND marker.payload->>'status'='OPEN'
       AND NOT EXISTS (
           SELECT 1 FROM decoded closed
            WHERE (
                (closed.payload_schema='lattice.wsl2-provider-subtree-receipt/1.0'
                 AND closed.payload->>'status'='CLOSED')
                OR
                (closed.payload_schema='lattice.wsl2-provider-subtree-reconciliation/1.0'
                 AND closed.payload->>'status'='RECONCILED')
            )
              AND closed.payload->>'source_marker_digest'=marker.payload->>'marker_digest'
              AND closed.payload->>'task_ref'=marker.payload->>'task_ref'
              AND closed.payload->>'attempt'=marker.payload->>'attempt'
              AND closed.payload->>'role'=marker.payload->>'role'
       )
), linked AS (
    SELECT marker.project_id, marker.evidence_json AS marker_json,
           preflight.evidence_json AS preflight_json,
           preflight.descriptor_digest AS preflight_descriptor_digest,
           preflight.content_digest AS preflight_content_digest
      FROM open_markers marker
      JOIN decoded preflight
        ON preflight.payload_schema='lattice.wsl2-zero-model-preflight/1.0'
       AND preflight.descriptor_digest=
           marker.payload->>'source_preflight_descriptor_digest'
       AND preflight.content_digest=marker.payload->>'source_preflight_content_digest'
)
SELECT pg_catalog.jsonb_build_object(
    'open_marker_count', (SELECT pg_catalog.count(*) FROM open_markers),
    'count', pg_catalog.count(*),
    'records', COALESCE(pg_catalog.jsonb_agg(
        pg_catalog.jsonb_build_object(
            'project_id',project_id,'marker_json',marker_json,
            'preflight_json',preflight_json,
            'preflight_descriptor_digest',preflight_descriptor_digest,
            'preflight_content_digest',preflight_content_digest
        ) ORDER BY marker_json
    ), '[]'::jsonb)
) FROM linked;
"@
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database `
        -Sql $sql -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_QUERY_REJECTED' }
    $records = @($value.records)
    if ([long]$value.open_marker_count -ne [long]$value.count -or
        [long]$value.count -ne $records.Count -or $records.Count -gt 4) {
        throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_QUERY_REJECTED'
    }

    $results = [Collections.Generic.List[object]]::new()
    foreach ($record in $records) {
        if ([string]$record.project_id -cne $ProjectId -or
            [string]$record.preflight_descriptor_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.preflight_content_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.preflight_content_digest -cne
                (Get-Phase4StringSha256 -Value ([string]$record.preflight_json))) {
            throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_QUERY_REJECTED'
        }
        try {
            $marker = [string]$record.marker_json | ConvertFrom-Json -ErrorAction Stop
            $preflight = [string]$record.preflight_json | ConvertFrom-Json -ErrorAction Stop
        }
        catch { throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_QUERY_REJECTED' }
        Assert-Phase4NoCredentialShapedJsonStrings -Value $marker `
            -Failure 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_SECRET_REJECTED'
        Assert-Phase4NoCredentialShapedJsonStrings -Value $preflight `
            -Failure 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_SECRET_REJECTED'
        $descriptorDigest = Get-Phase4StringSha256 `
            -Value ([string]$Materialization.descriptor_json)
        if ([string]$marker.task_ref -cne $TaskRef -or [int]$marker.attempt -ne 1 -or
            [string]$marker.status -cne 'OPEN' -or
            [string]$marker.role -cnotin @('PROVIDER', 'REVIEWER') -or
            [string]$marker.execution_environment_ref -cne
                [string]$Materialization.record.execution_environment_ref -or
            [string]$marker.descriptor_digest -cne $descriptorDigest -or
            [string]$marker.packet_digest -cnotmatch
                '\Aattempt-packet:sha256:[0-9a-f]{64}\z' -or
            [string]$preflight.receipt_digest -cne
                [string]$marker.source_preflight_receipt_digest) {
            throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_IDENTITY_REJECTED'
        }
        $request = [ordered]@{
            schema = 'lattice.wsl2-provider-subtree-reconcile-request/1.0'
            descriptor_json = [string]$Materialization.descriptor_json
            descriptor_digest = $descriptorDigest
            source_preflight = [ordered]@{
                descriptor_digest = [string]$record.preflight_descriptor_digest
                content_digest = [string]$record.preflight_content_digest
                receipt_json = [string]$record.preflight_json
            }
            open_marker = $marker
            packet_digest = [string]$marker.packet_digest
            provider_effect_count_before = $ProviderEffectCount
            provider_effect_count_after = $ProviderEffectCount
        }
        if ([string]$marker.role -ceq 'REVIEWER') {
            $request.schema = 'lattice.wsl2-reviewer-subtree-reconcile-request/1.0'
            $request['reviewer_context'] = [ordered]@{
                task_ref = $TaskRef
                attempt = 1
                subject_digest = [string]$marker.subject_digest
                model_call_identity = [string]$marker.model_call_identity
                worktree_ref = [string]$marker.worktree_ref
                repository_head = [string]$marker.repository_head
                execution_environment_ref = [string]$marker.execution_environment_ref
                packet_digest = [string]$marker.packet_digest
            }
        }
        $requestJson = $request | ConvertTo-Json -Compress -Depth 30
        $reconciledProcess = Invoke-Phase4Process -Executable $NodeExecutable `
            -Argument @($script:Wsl2ProviderSubtreeReconciler) `
            -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
            -WorkingDirectory $script:RepositoryRoot -StandardInput ($requestJson + "`n") `
            -TimeoutSeconds 45 -Failure 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_FAILED' `
            -AllowNonZeroExit
        $lines = @([string]$reconciledProcess.stdout -split '\r?\n' |
            Where-Object { $_.Length -gt 0 })
        if ([int]$reconciledProcess.exit_code -ne 0 -or
            [long]$reconciledProcess.stderr_byte_count -ne 0 -or $lines.Count -ne 1 -or
            $script:Utf8.GetByteCount($lines[0]) -gt 65536) {
            throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_FAILED'
        }
        try { $reconciled = $lines[0] | ConvertFrom-Json -ErrorAction Stop }
        catch { throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_REJECTED' }
        Assert-Phase4NoCredentialShapedJsonStrings -Value $reconciled `
            -Failure 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_SECRET_REJECTED'
        $outer = $reconciled.outer_post_exit
        $cgroupClosed = ($outer.cgroup_exists -is [bool]) -and (
            (-not [bool]$outer.cgroup_exists -and $null -eq $outer.populated) -or
            ([bool]$outer.cgroup_exists -and $null -ne $outer.populated -and
                [long]$outer.populated -eq 0)
        )
        if ([string]$reconciled.status -cne 'RECONCILED' -or
            [string]$reconciled.task_ref -cne $TaskRef -or
            [string]$reconciled.role -cne [string]$marker.role -or
            [long]$reconciled.provider_effect_count_before -ne $ProviderEffectCount -or
            [long]$reconciled.provider_effect_count_after -ne $ProviderEffectCount -or
            [string]$outer.active_state -cne 'inactive' -or
            [string]$outer.sub_state -cne 'dead' -or -not $cgroupClosed) {
            throw 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_REJECTED'
        }
        $results.Add($reconciled)
    }

    $unitCleanup = Close-Phase4WslTaskOwnedUnits `
        -TaskRef $TaskRef -Materialization $Materialization
    $durableReconciliationRequired = $records.Count -ne 0
    return [pscustomobject][ordered]@{
        schema = 'lattice.phase4-wsl2-failure-subtree-cleanup/1.0'
        status = $(if ($durableReconciliationRequired) { 'PHYSICAL_ONLY' } else { 'CLOSED' })
        gate_passed = (-not $durableReconciliationRequired)
        open_marker_count = $records.Count
        reconciled_count = $results.Count
        durable_open_marker_count = $records.Count
        reconciliation_required = $durableReconciliationRequired
        reconciliations = @($results)
        task_unit_count = [long]$unitCleanup.task_unit_count
        active_unit_count = 0
        unit_cleanup = $unitCleanup
        inventory_stdout_sha256 = [string]$unitCleanup.inventory_stdout_sha256
        query_evidence_digest = Get-Phase4StringSha256 -Value $raw
    }
}

function Get-Phase4WslProviderSubtreeSegmentRef {
    param(
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][ValidateRange(1, 3)][int]$Attempt,
        [Parameter(Mandatory = $true)][string]$SourcePreflightDescriptorDigest,
        [Parameter(Mandatory = $true)][string]$SourcePreflightContentDigest,
        [Parameter(Mandatory = $true)][string]$SourcePreflightReceiptDigest,
        [Parameter(Mandatory = $true)][string]$Fence,
        [Parameter(Mandatory = $true)][ValidateSet('PROVIDER', 'REVIEWER')][string]$Role,
        [AllowNull()][string]$RetryOf,
        [AllowNull()][string]$ReconnectOf,
        [AllowNull()][string]$SubjectDigest,
        [AllowNull()][string]$ModelCallIdentity
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $SourcePreflightDescriptorDigest -cnotmatch '\A[0-9a-f]{64}\z' -or
        $SourcePreflightContentDigest -cnotmatch '\A[0-9a-f]{64}\z' -or
        $SourcePreflightReceiptDigest -cnotmatch
            '\Awsl2-preflight:sha256:[0-9a-f]{64}\z' -or
        $Fence -cnotmatch '\A[0-9a-f]{64}\z' -or
        ($null -ne $RetryOf -and $RetryOf -cnotmatch
            '\A(?:attempt|verifier)-receipt:sha256:[0-9a-f]{64}\z') -or
        ($null -ne $ReconnectOf -and $ReconnectOf -cnotmatch
            '\A(?:attempt|verifier)-receipt:sha256:[0-9a-f]{64}\z') -or
        ($null -ne $RetryOf -and $null -ne $ReconnectOf)) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_SEGMENT_IDENTITY_REJECTED'
    }
    $subject = [ordered]@{
        attempt = $Attempt
        continuation = [ordered]@{
            reconnect_of = $ReconnectOf
            retry_of = $RetryOf
        }
        fence = $Fence
        role = $Role
        source_preflight_content_digest = $SourcePreflightContentDigest
        source_preflight_descriptor_digest = $SourcePreflightDescriptorDigest
        source_preflight_receipt_digest = $SourcePreflightReceiptDigest
        task_ref = $TaskRef
    }
    if ($Role -ceq 'REVIEWER') {
        if ($SubjectDigest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]::IsNullOrWhiteSpace($ModelCallIdentity) -or
            $script:Utf8.GetByteCount($ModelCallIdentity) -gt 256) {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_SEGMENT_IDENTITY_REJECTED'
        }
        # canonicalJson sorts these reviewer-only fields between role and source_*.  Rebuild
        # the ordered map explicitly so this independent harness computes the same identity.
        $subject = [ordered]@{
            attempt = $Attempt
            continuation = [ordered]@{
                reconnect_of = $ReconnectOf
                retry_of = $RetryOf
            }
            fence = $Fence
            model_call_identity = $ModelCallIdentity
            role = $Role
            source_preflight_content_digest = $SourcePreflightContentDigest
            source_preflight_descriptor_digest = $SourcePreflightDescriptorDigest
            source_preflight_receipt_digest = $SourcePreflightReceiptDigest
            subject_digest = $SubjectDigest
            task_ref = $TaskRef
        }
    }
    $canonical = $subject | ConvertTo-Json -Compress -Depth 6
    return 'provider-subtree-segment:sha256:' + (Get-Phase4StringSha256 -Value $canonical)
}

function Get-Phase4WslProviderSubtreeOpenEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$ProjectId,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)]$PreflightEvidence,
        [Parameter(Mandatory = $true)][string]$ExpectedWorktreeRef,
        [Parameter(Mandatory = $true)][string]$ExpectedPacketDigest,
        [Parameter(Mandatory = $true)][string]$ExpectedProducerDigest
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ProjectId -cnotmatch '\A[a-z0-9][a-z0-9._-]{1,63}\z' -or
        $ExpectedWorktreeRef -cnotmatch '\Aworktree:sha256:[0-9a-f]{64}\z' -or
        $ExpectedPacketDigest -cnotmatch '\Aattempt-packet:sha256:[0-9a-f]{64}\z' -or
        $ExpectedProducerDigest -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_IDENTITY_REJECTED'
    }
    $sql = @"
WITH managed AS (
    SELECT evidence.*
      FROM foreman_execution.read_managed_evidence_v1(
          decode('$TaskRef','hex'),1::smallint
      )
        AS evidence
     WHERE evidence.payload_schema='lattice.wsl2-provider-subtree-marker/1.0'
       AND pg_catalog.convert_from(evidence.evidence_bytes,'UTF8')::jsonb->>'role'='PROVIDER'
), replay AS (
    SELECT replay.record_digest, replay.ledger_event_sequence,
           pg_catalog.encode(replay.ledger_event_digest,'hex') AS ledger_event_digest,
           replay.recorded_at
      FROM foreman_execution.read_task_replay_v1(decode('$TaskRef','hex')) AS replay
     WHERE replay.record_kind='ARTIFACT_REFERENCE'
       AND replay.attempt_number=1
), matches AS (
    SELECT managed.project_id, managed.evidence_kind, managed.media_type,
           managed.payload_schema, managed.producer_id, managed.producer_version,
           pg_catalog.encode(managed.producer_digest,'hex') AS producer_digest,
           managed.created_at,
           pg_catalog.convert_from(managed.evidence_bytes,'UTF8') AS evidence_json,
           pg_catalog.encode(managed.content_digest,'hex') AS content_digest,
           pg_catalog.encode(managed.descriptor_digest,'hex') AS descriptor_digest,
           replay.ledger_event_sequence::text AS ledger_event_sequence,
           replay.ledger_event_digest, replay.recorded_at AS ledger_recorded_at
      FROM managed
      JOIN replay ON replay.record_digest=managed.descriptor_digest
)
SELECT pg_catalog.jsonb_build_object(
    'count', pg_catalog.count(*),
    'records', COALESCE(
        pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            'project_id',project_id,'evidence_kind',evidence_kind,
            'media_type',media_type,'payload_schema',payload_schema,
            'producer_id',producer_id,'producer_version',producer_version,
            'producer_digest',producer_digest,'created_at',created_at,
            'evidence_json',evidence_json,'content_digest',content_digest,
            'descriptor_digest',descriptor_digest,
            'ledger_event_sequence',ledger_event_sequence,
            'ledger_event_digest',ledger_event_digest,
            'ledger_recorded_at',ledger_recorded_at
        ) ORDER BY ledger_event_sequence::numeric, descriptor_digest), '[]'::jsonb
    )
) FROM matches;
"@
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database `
        -Sql $sql -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_EVIDENCE_REJECTED' }
    $records = @($value.records)
    if ([long]$value.count -ne 1 -or $records.Count -ne 1) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_EVIDENCE_REJECTED'
    }
    $record = $records[0]
    if ([string]$record.project_id -cne $ProjectId -or
        [string]$record.evidence_kind -cne 'WORKER_LIFECYCLE' -or
        [string]$record.media_type -cne 'application/json' -or
        [string]$record.payload_schema -cne 'lattice.wsl2-provider-subtree-marker/1.0' -or
        [string]$record.producer_id -cne 'lattice-managed-codex-worker' -or
        [string]$record.producer_version -cne '0.1.0' -or
        [string]$record.producer_digest -cne $ExpectedProducerDigest -or
        [string]$record.content_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$record.descriptor_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$record.ledger_event_sequence -cnotmatch '\A[1-9][0-9]*\z' -or
        [string]$record.ledger_event_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [decimal]$record.ledger_event_sequence -le
            [decimal]$PreflightEvidence.ledger_event_sequence -or
        [string]$record.content_digest -cne
            (Get-Phase4StringSha256 -Value ([string]$record.evidence_json)) -or
        $script:Utf8.GetByteCount([string]$record.evidence_json) -gt 16384) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_EVIDENCE_REJECTED'
    }
    try { $marker = [string]$record.evidence_json | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_EVIDENCE_REJECTED' }
    Assert-Phase4NoCredentialShapedJsonStrings -Value $marker `
        -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_SECRET_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $marker -Expected @(
        'schema', 'status', 'task_ref', 'attempt', 'packet_digest', 'worktree_ref',
        'repository_head', 'execution_environment_ref', 'descriptor_digest',
        'source_preflight_descriptor_digest', 'source_preflight_content_digest',
        'source_preflight_receipt_digest', 'role', 'provider_subtree_segment_ref',
        'process_marker', 'boot_id_digest',
        'credential_seal_digest', 'continuation', 'provider_effect_count', 'marker_digest'
    ) -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_EVIDENCE_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $marker.process_marker -Expected @(
        'schema', 'fence', 'unit', 'execution_environment_ref', 'credential_seal_digest',
        'boot_id_digest', 'pid', 'process_start_ticks', 'process_group_id', 'cgroup_path',
        'cgroup_version', 'delegated', 'attempt', 'retry_of', 'reconnect_of'
    ) -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_EVIDENCE_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $marker.continuation `
        -Expected @('retry_of', 'reconnect_of') `
        -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_EVIDENCE_REJECTED'
    $descriptorDigest = Get-Phase4StringSha256 -Value (
        [string]$Materialization.descriptor_json
    )
    $processMarker = $marker.process_marker
    $cgroupPath = [string]$processMarker.cgroup_path
    $expectedProviderCgroup = Get-Phase4CanonicalUserServiceCgroupPath `
        -OwnerUid ([long]$Materialization.descriptor.verification_toolchain.owner_uid) `
        -Unit ([string]$processMarker.unit)
    if ([string]$marker.schema -cne 'lattice.wsl2-provider-subtree-marker/1.0' -or
        [string]$marker.status -cne 'OPEN' -or [string]$marker.task_ref -cne $TaskRef -or
        [int]$marker.attempt -ne 1 -or
        [string]$marker.packet_digest -cne $ExpectedPacketDigest -or
        [string]$marker.worktree_ref -cne $ExpectedWorktreeRef -or
        [string]$marker.repository_head -cne
            [string]$Materialization.descriptor.linux.repository_head -or
        [string]$marker.execution_environment_ref -cne
            [string]$Materialization.record.execution_environment_ref -or
        [string]$marker.descriptor_digest -cne $descriptorDigest -or
        [string]$marker.source_preflight_descriptor_digest -cne
            [string]$PreflightEvidence.artifact_descriptor_digest -or
        [string]$marker.source_preflight_content_digest -cne
            [string]$PreflightEvidence.artifact_content_digest -or
        [string]$marker.source_preflight_receipt_digest -cne
            [string]$PreflightEvidence.receipt_digest -or
        [string]$marker.role -cne 'PROVIDER' -or [long]$marker.provider_effect_count -ne 0 -or
        [string]$marker.boot_id_digest -cne [string]$PreflightEvidence.boot_id_digest -or
        [string]$marker.credential_seal_digest -cne
            [string]$PreflightEvidence.receipt.credential_seal_digest -or
        $null -ne $marker.continuation.retry_of -or
        $null -ne $marker.continuation.reconnect_of -or
        [string]$marker.provider_subtree_segment_ref -cne
            (Get-Phase4WslProviderSubtreeSegmentRef -TaskRef $TaskRef -Attempt 1 `
                -SourcePreflightDescriptorDigest (
                    [string]$PreflightEvidence.artifact_descriptor_digest
                ) -SourcePreflightContentDigest (
                    [string]$PreflightEvidence.artifact_content_digest
                ) -SourcePreflightReceiptDigest ([string]$PreflightEvidence.receipt_digest) `
                -Fence ([string]$processMarker.fence) -Role 'PROVIDER' `
                -RetryOf $null -ReconnectOf $null) -or
        [string]$marker.marker_digest -cnotmatch
            '\Aprovider-subtree-marker:sha256:[0-9a-f]{64}\z' -or
        [string]$processMarker.schema -cne 'lattice.wsl2-process-fence/1.1' -or
        [string]$processMarker.fence -cne [string]$PreflightEvidence.process_fence -or
        [string]$processMarker.unit -cne [string]$PreflightEvidence.provider_unit -or
        [string]$processMarker.execution_environment_ref -cne
            [string]$marker.execution_environment_ref -or
        [string]$processMarker.credential_seal_digest -cne
            [string]$marker.credential_seal_digest -or
        [string]$processMarker.boot_id_digest -cne [string]$marker.boot_id_digest -or
        [long]$processMarker.pid -le 0 -or
        [string]$processMarker.process_start_ticks -cnotmatch '\A[1-9][0-9]*\z' -or
        [long]$processMarker.process_group_id -le 0 -or
        $cgroupPath -cne $expectedProviderCgroup -or
        [long]$processMarker.cgroup_version -ne 2 -or [bool]$processMarker.delegated -or
        [int]$processMarker.attempt -ne 1 -or $null -ne $processMarker.retry_of -or
        $null -ne $processMarker.reconnect_of) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_EVIDENCE_REJECTED'
    }
    $processIdentity = [pscustomobject][ordered]@{
        schema = 'lattice.wsl2-process-identity/1.0'
        boot_id_digest = [string]$processMarker.boot_id_digest
        pid = [long]$processMarker.pid
        process_start_ticks = [string]$processMarker.process_start_ticks
        process_group_id = [string]$processMarker.process_group_id
        cgroup_path = $cgroupPath
    }
    return [pscustomobject][ordered]@{
        marker = $marker
        marker_json = [string]$record.evidence_json
        marker_digest = [string]$marker.marker_digest
        artifact_ref = 'evidence:sha256:' + [string]$record.descriptor_digest
        artifact_descriptor_digest = [string]$record.descriptor_digest
        artifact_content_digest = [string]$record.content_digest
        producer_digest = [string]$record.producer_digest
        provider_subtree_segment_ref = [string]$marker.provider_subtree_segment_ref
        process_identity = $processIdentity
        ledger_event_sequence = [string]$record.ledger_event_sequence
        ledger_event_digest = [string]$record.ledger_event_digest
        ledger_recorded_at = [string]$record.ledger_recorded_at
        sql_evidence_digest = Get-Phase4StringSha256 -Value $raw
    }
}

function Get-Phase4WslSystemdUnitState {
    param(
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)][string]$Unit
    )

    $descriptor = $Materialization.descriptor
    if ($Unit -cnotmatch
            '\Alattice-wsl2-[0-9a-f]{16}-(?:preflight|provider|reviewer)-[0-9a-f]{12}\.service\z' -or
        -not $Unit.StartsWith(
            [string]$descriptor.process_fence.unit_prefix + '-',
            [StringComparison]::Ordinal
        ) -or
        [string]$descriptor.process_fence.systemctl_path -cnotmatch
            '\A/(?:[A-Za-z0-9._-]+/)*[A-Za-z0-9._-]+\z' -or
        [string]$descriptor.process_fence.systemctl_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$descriptor.process_fence.user_runtime_dir -cne '/run/user/1000') {
        throw 'PHASE4_WSL2_PROVIDER_UNIT_IDENTITY_REJECTED'
    }
    $result = Invoke-Phase4WslProcess `
        -Executable ([string]$descriptor.process_fence.systemctl_path) -Argument @(
            '--user', '--no-pager', 'show', $Unit,
            '--property=Id', '--property=LoadState', '--property=ActiveState',
            '--property=SubState', '--property=Result', '--property=ControlGroup',
            '--property=MainPID'
        ) -Environment ([ordered]@{
            DBUS_SESSION_BUS_ADDRESS = 'unix:path:' +
                [string]$descriptor.process_fence.user_runtime_dir + '/bus'
        }) -LinuxHome $script:Wsl2HarnessHome -LinuxTemp $script:Wsl2HarnessTemp `
        -StandardInput $null -TimeoutSeconds 15 `
        -Failure 'PHASE4_WSL2_PROVIDER_UNIT_QUERY_FAILED'
    if ([int]$result.exit_code -ne 0 -or [long]$result.stderr_byte_count -ne 0 -or
        $script:Utf8.GetByteCount([string]$result.stdout) -gt 8192) {
        throw 'PHASE4_WSL2_PROVIDER_UNIT_QUERY_REJECTED'
    }
    $expected = @('Id', 'LoadState', 'ActiveState', 'SubState', 'Result', 'ControlGroup', 'MainPID')
    $values = [ordered]@{}
    foreach ($line in @([string]$result.stdout -split '\r?\n' | Where-Object { $_.Length -gt 0 })) {
        if ($line -cnotmatch '\A(?<name>[A-Za-z]+)=(?<value>.*)\z' -or
            [string]$Matches.name -cnotin $expected -or $values.Contains([string]$Matches.name)) {
            throw 'PHASE4_WSL2_PROVIDER_UNIT_QUERY_REJECTED'
        }
        $values[[string]$Matches.name] = [string]$Matches.value
    }
    if ($values.Count -ne $expected.Count -or
        @($expected | Where-Object { -not $values.Contains($_) }).Count -ne 0 -or
        [string]$values.Id -cne $Unit -or [string]$values.MainPID -cnotmatch '\A[0-9]+\z') {
        throw 'PHASE4_WSL2_PROVIDER_UNIT_QUERY_REJECTED'
    }
    return [pscustomobject][ordered]@{
        unit = $Unit
        load_state = [string]$values.LoadState
        active_state = [string]$values.ActiveState
        sub_state = [string]$values.SubState
        result = [string]$values.Result
        cgroup_path = [string]$values.ControlGroup
        main_pid = [long]$values.MainPID
        systemctl_identity = [pscustomobject][ordered]@{
            path = [string]$descriptor.process_fence.systemctl_path
            version = [string]$descriptor.process_fence.systemctl_version
            sha256 = [string]$descriptor.process_fence.systemctl_sha256
        }
        process_evidence = [pscustomobject][ordered]@{
            timeout_seconds = 15
            stdout_byte_count = [long]$script:Utf8.GetByteCount([string]$result.stdout)
            stdout_sha256 = Get-Phase4StringSha256 -Value ([string]$result.stdout)
            stderr_byte_count = [long]$result.stderr_byte_count
            stderr_sha256 = [string]$result.stderr_sha256
            exit_code = [int]$result.exit_code
        }
    }
}

function Get-Phase4WslProviderSubtreeReconciliationEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$ProjectId,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)]$PreflightEvidence,
        [Parameter(Mandatory = $true)]$OpenMarkerEvidence,
        [Parameter(Mandatory = $true)][string]$ExpectedWorktreeRef,
        [Parameter(Mandatory = $true)][string]$ExpectedPacketDigest,
        [Parameter(Mandatory = $true)][string]$ExpectedInitialProducerDigest,
        [Parameter(Mandatory = $true)][string]$ExpectedReconcilerProducerDigest,
        [Parameter(Mandatory = $true)][ValidateRange(0, 16)][int]$ExpectedProviderEffectCount,
        [switch]$RequireSuccessorOpen
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ProjectId -cnotmatch '\A[a-z0-9][a-z0-9._-]{1,63}\z' -or
        $ExpectedWorktreeRef -cnotmatch '\Aworktree:sha256:[0-9a-f]{64}\z' -or
        $ExpectedPacketDigest -cnotmatch '\Aattempt-packet:sha256:[0-9a-f]{64}\z' -or
        $ExpectedInitialProducerDigest -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ExpectedReconcilerProducerDigest -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $sql = @"
WITH managed AS (
    SELECT evidence.*,
           pg_catalog.convert_from(evidence.evidence_bytes,'UTF8') AS evidence_json
      FROM foreman_execution.read_managed_evidence_v1(
          decode('$TaskRef','hex'),1::smallint
      )
        AS evidence
     WHERE (
               evidence.payload_schema='lattice.wsl2-zero-model-preflight/1.0'
               AND evidence.producer_id='lattice-runtime-wsl2-preflight-bridge'
           ) OR (
               evidence.payload_schema IN (
                   'lattice.wsl2-provider-subtree-marker/1.0',
                   'lattice.wsl2-provider-subtree-receipt/1.0',
                   'lattice.wsl2-provider-subtree-reconciliation/1.0'
               )
               AND evidence.evidence_bytes <> ''::bytea
               AND pg_catalog.convert_from(evidence.evidence_bytes,'UTF8')::jsonb->>'role'='PROVIDER'
           )
), replay AS (
    SELECT replay.record_digest, replay.ledger_event_sequence,
           pg_catalog.encode(replay.ledger_event_digest,'hex') AS ledger_event_digest,
           replay.recorded_at
      FROM foreman_execution.read_task_replay_v1(decode('$TaskRef','hex')) AS replay
     WHERE replay.record_kind='ARTIFACT_REFERENCE'
       AND replay.attempt_number=1
), linked AS (
    SELECT managed.project_id, managed.evidence_kind, managed.media_type,
           managed.payload_schema, managed.producer_id, managed.producer_version,
           pg_catalog.encode(managed.producer_digest,'hex') AS producer_digest,
           managed.created_at, managed.evidence_json,
           pg_catalog.encode(managed.content_digest,'hex') AS content_digest,
           pg_catalog.encode(managed.descriptor_digest,'hex') AS descriptor_digest,
           replay.ledger_event_sequence::text AS ledger_event_sequence,
           replay.ledger_event_digest, replay.recorded_at AS ledger_recorded_at
      FROM managed
      JOIN replay ON replay.record_digest=managed.descriptor_digest
)
SELECT pg_catalog.jsonb_build_object(
    'count', pg_catalog.count(*),
    'records', COALESCE(
        pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            'project_id',project_id,'evidence_kind',evidence_kind,
            'media_type',media_type,'payload_schema',payload_schema,
            'producer_id',producer_id,'producer_version',producer_version,
            'producer_digest',producer_digest,'created_at',created_at,
            'evidence_json',evidence_json,'content_digest',content_digest,
            'descriptor_digest',descriptor_digest,
            'ledger_event_sequence',ledger_event_sequence,
            'ledger_event_digest',ledger_event_digest,
            'ledger_recorded_at',ledger_recorded_at
        ) ORDER BY ledger_event_sequence::numeric, descriptor_digest), '[]'::jsonb
    )
) FROM linked;
"@
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database `
        -Sql $sql -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED' }
    $records = @($value.records)
    if ([long]$value.count -ne $records.Count -or
        $records.Count -lt 5 -or $records.Count -gt 64) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $entries = [Collections.Generic.List[object]]::new()
    foreach ($record in $records) {
        if ([string]$record.project_id -cne $ProjectId -or
            [string]$record.evidence_kind -cne 'WORKER_LIFECYCLE' -or
            [string]$record.media_type -cne 'application/json' -or
            [string]$record.producer_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.content_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.descriptor_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.ledger_event_sequence -cnotmatch '\A[1-9][0-9]*\z' -or
            [string]$record.ledger_event_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.content_digest -cne
                (Get-Phase4StringSha256 -Value ([string]$record.evidence_json)) -or
            $script:Utf8.GetByteCount([string]$record.evidence_json) -gt 1048576) {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        }
        try { $payload = [string]$record.evidence_json | ConvertFrom-Json -ErrorAction Stop }
        catch { throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED' }
        Assert-Phase4NoCredentialShapedJsonStrings -Value $payload `
            -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_SECRET_REJECTED'
        $entries.Add([pscustomobject][ordered]@{ record = $record; payload = $payload })
    }
    $preflightEntries = @($entries | Where-Object {
        [string]$_.record.payload_schema -ceq 'lattice.wsl2-zero-model-preflight/1.0'
    })
    $markerEntries = @($entries | Where-Object {
        [string]$_.record.payload_schema -ceq 'lattice.wsl2-provider-subtree-marker/1.0'
    })
    $receiptEntries = @($entries | Where-Object {
        [string]$_.record.payload_schema -ceq 'lattice.wsl2-provider-subtree-receipt/1.0'
    })
    $reconciliationEntries = @($entries | Where-Object {
        [string]$_.record.payload_schema -ceq
            'lattice.wsl2-provider-subtree-reconciliation/1.0'
    })
    if ($preflightEntries.Count -lt 2 -or $preflightEntries.Count -gt 16 -or
        $markerEntries.Count -lt 2 -or $markerEntries.Count -gt 16 -or
        $receiptEntries.Count -gt 16 -or $reconciliationEntries.Count -lt 1 -or
        $reconciliationEntries.Count -gt 16) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $rootPreflightEntries = @($preflightEntries | Where-Object {
        [string]$_.payload.receipt_digest -ceq [string]$PreflightEvidence.receipt_digest
    })
    $rootMarkerEntries = @($markerEntries | Where-Object {
        [string]$_.payload.marker_digest -ceq [string]$OpenMarkerEvidence.marker_digest
    })
    if ($rootPreflightEntries.Count -ne 1 -or $rootMarkerEntries.Count -ne 1) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $rootPreflightEntry = $rootPreflightEntries[0]
    $rootMarkerEntry = $rootMarkerEntries[0]
    $rootReconciliationEntries = @($reconciliationEntries | Where-Object {
        [string]$_.payload.source_marker_digest -ceq
            [string]$OpenMarkerEvidence.marker_digest -and
        [string]$_.payload.provider_subtree_segment_ref -ceq
            [string]$OpenMarkerEvidence.provider_subtree_segment_ref
    })
    if ($rootReconciliationEntries.Count -ne 1) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $reconciliationEntry = $rootReconciliationEntries[0]
    $successorPreflightEntries = @($preflightEntries | Where-Object {
        [decimal]$_.record.ledger_event_sequence -gt
            [decimal]$reconciliationEntry.record.ledger_event_sequence
    } | Sort-Object { [decimal]$_.record.ledger_event_sequence })
    if ($successorPreflightEntries.Count -lt 1) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $successorPreflightEntry = $successorPreflightEntries[0]
    $successorMarkerEntries = @($markerEntries | Where-Object {
        [string]$_.payload.source_preflight_descriptor_digest -ceq
            [string]$successorPreflightEntry.record.descriptor_digest -and
        [string]$_.payload.source_preflight_content_digest -ceq
            [string]$successorPreflightEntry.record.content_digest -and
        [string]$_.payload.source_preflight_receipt_digest -ceq
            [string]$successorPreflightEntry.payload.receipt_digest
    })
    if ($successorMarkerEntries.Count -ne 1) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $successorMarkerEntry = $successorMarkerEntries[0]
    $rootPreflight = $rootPreflightEntry.payload
    $rootMarker = $rootMarkerEntry.payload
    $successorPreflight = $successorPreflightEntry.payload
    $successorMarker = $successorMarkerEntry.payload
    $reconciliation = $reconciliationEntry.payload
    if ([string]$rootPreflightEntry.record.evidence_json -cne
            [string]$PreflightEvidence.receipt_json -or
        [string]$rootPreflightEntry.record.descriptor_digest -cne
            [string]$PreflightEvidence.artifact_descriptor_digest -or
        [string]$rootPreflightEntry.record.content_digest -cne
            [string]$PreflightEvidence.artifact_content_digest -or
        [string]$rootPreflightEntry.record.ledger_event_sequence -cne
            [string]$PreflightEvidence.ledger_event_sequence -or
        [string]$rootMarkerEntry.record.evidence_json -cne
            [string]$OpenMarkerEvidence.marker_json -or
        [string]$rootMarkerEntry.record.descriptor_digest -cne
            [string]$OpenMarkerEvidence.artifact_descriptor_digest -or
        [string]$rootMarkerEntry.record.content_digest -cne
            [string]$OpenMarkerEvidence.artifact_content_digest -or
        [string]$rootMarkerEntry.record.producer_digest -cne $ExpectedInitialProducerDigest) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    foreach ($preflightEntry in @($rootPreflightEntry, $successorPreflightEntry)) {
        $preflight = $preflightEntry.payload
        Assert-Phase4ExactJsonProperties -Value $preflight -Expected @(
            'schema', 'status', 'task_ref', 'attempt', 'worktree_ref',
            'execution_environment_ref', 'descriptor_digest', 'distribution_identity_ref',
            'linux_cwd', 'repository_head', 'repository_identity', 'codex_home_digest',
            'credential_authority_ref', 'credential_seal_digest',
            'verification_toolchain_ref', 'immutable_snapshot_ref', 'sandbox_policy_ref',
            'privilege_boundary_ref', 'process_fence', 'isolation', 'probes',
            'effect_counters', 'provider_effect_count', 'bounds', 'timeout', 'continuation',
            'connector_auth_ready', 'receipt_digest'
        ) -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        Assert-Phase4ExactJsonProperties -Value $preflight.continuation `
            -Expected @('attempt', 'retry_of', 'reconnect_of') `
            -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        if ([string]$preflightEntry.record.producer_id -cne
                'lattice-runtime-wsl2-preflight-bridge' -or
            [string]$preflightEntry.record.producer_version -cne '1.0' -or
            [string]$preflightEntry.record.producer_digest -cne
                (Get-Phase4FileSha256 -Path $script:Wsl2PreflightBridge) -or
            [string]$preflight.schema -cne 'lattice.wsl2-zero-model-preflight/1.0' -or
            [string]$preflight.status -cne 'PASS' -or
            [string]$preflight.task_ref -cne $TaskRef -or [int]$preflight.attempt -ne 1 -or
            [string]$preflight.worktree_ref -cne $ExpectedWorktreeRef -or
            [string]$preflight.execution_environment_ref -cne
                [string]$Materialization.record.execution_environment_ref -or
            [string]$preflight.descriptor_digest -cne
                [string]$Materialization.record.execution_environment_ref -or
            [string]$preflight.repository_head -cne
                [string]$Materialization.descriptor.linux.repository_head -or
            [long]$preflight.provider_effect_count -ne 0 -or
            [long]$preflight.effect_counters.provider_effect_count -ne 0 -or
            [int]$preflight.continuation.attempt -ne 1 -or
            [string]$preflight.receipt_digest -cnotmatch
                '\Awsl2-preflight:sha256:[0-9a-f]{64}\z' -or
            [string]$preflight.process_fence.fence -cnotmatch '\A[0-9a-f]{64}\z') {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        }
    }
    $successorReconnectOf = [string]$successorPreflight.continuation.reconnect_of
    if ($null -ne $rootPreflight.continuation.retry_of -or
        $null -ne $rootPreflight.continuation.reconnect_of -or
        $null -ne $successorPreflight.continuation.retry_of -or
        $successorReconnectOf -cnotmatch '\Aattempt-receipt:sha256:[0-9a-f]{64}\z' -or
        [string]$successorPreflight.process_fence.fence -ceq
            [string]$rootPreflight.process_fence.fence) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    foreach ($markerEntry in @($rootMarkerEntry, $successorMarkerEntry)) {
        $marker = $markerEntry.payload
        Assert-Phase4ExactJsonProperties -Value $marker -Expected @(
            'schema', 'status', 'task_ref', 'attempt', 'packet_digest', 'worktree_ref',
            'repository_head', 'execution_environment_ref', 'descriptor_digest',
            'source_preflight_descriptor_digest', 'source_preflight_content_digest',
            'source_preflight_receipt_digest', 'role', 'provider_subtree_segment_ref',
            'process_marker', 'boot_id_digest', 'credential_seal_digest', 'continuation',
            'provider_effect_count', 'marker_digest'
        ) -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        Assert-Phase4ExactJsonProperties -Value $marker.continuation `
            -Expected @('retry_of', 'reconnect_of') `
            -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        if ([string]$markerEntry.record.producer_id -cne 'lattice-managed-codex-worker' -or
            [string]$markerEntry.record.producer_version -cne '0.1.0' -or
            [string]$marker.schema -cne 'lattice.wsl2-provider-subtree-marker/1.0' -or
            [string]$marker.status -cne 'OPEN' -or [string]$marker.role -cne 'PROVIDER' -or
            [string]$marker.task_ref -cne $TaskRef -or [int]$marker.attempt -ne 1 -or
            [string]$marker.packet_digest -cne $ExpectedPacketDigest -or
            [string]$marker.worktree_ref -cne $ExpectedWorktreeRef -or
            [string]$marker.repository_head -cne
                [string]$Materialization.descriptor.linux.repository_head -or
            [string]$marker.execution_environment_ref -cne
                [string]$Materialization.record.execution_environment_ref -or
            [string]$marker.descriptor_digest -cne
                (Get-Phase4StringSha256 -Value ([string]$Materialization.descriptor_json)) -or
            [long]$marker.provider_effect_count -ne 0 -or
            [string]$marker.marker_digest -cnotmatch
                '\Aprovider-subtree-marker:sha256:[0-9a-f]{64}\z' -or
            [string]$marker.process_marker.schema -cne 'lattice.wsl2-process-fence/1.1' -or
            [string]$marker.process_marker.fence -cnotmatch '\A[0-9a-f]{64}\z') {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        }
    }
    if ([string]$successorMarkerEntry.record.producer_digest -cne
            $ExpectedReconcilerProducerDigest -or
        [string]$successorMarker.source_preflight_descriptor_digest -cne
            [string]$successorPreflightEntry.record.descriptor_digest -or
        [string]$successorMarker.source_preflight_content_digest -cne
            [string]$successorPreflightEntry.record.content_digest -or
        [string]$successorMarker.source_preflight_receipt_digest -cne
            [string]$successorPreflight.receipt_digest -or
        [string]$successorMarker.process_marker.fence -cne
            [string]$successorPreflight.process_fence.fence -or
        [string]$successorMarker.continuation.reconnect_of -cne $successorReconnectOf -or
        $null -ne $successorMarker.continuation.retry_of -or
        [string]$successorMarker.provider_subtree_segment_ref -cne
            (Get-Phase4WslProviderSubtreeSegmentRef -TaskRef $TaskRef -Attempt 1 `
                -SourcePreflightDescriptorDigest (
                    [string]$successorPreflightEntry.record.descriptor_digest
                ) -SourcePreflightContentDigest (
                    [string]$successorPreflightEntry.record.content_digest
                ) -SourcePreflightReceiptDigest ([string]$successorPreflight.receipt_digest) `
                -Fence ([string]$successorMarker.process_marker.fence) -Role 'PROVIDER' `
                -RetryOf $null -ReconnectOf $successorReconnectOf)) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $successorReceiptEntries = @($receiptEntries | Where-Object {
        [string]$_.payload.source_marker_digest -ceq [string]$successorMarker.marker_digest
    })
    if ($successorReceiptEntries.Count -gt 1) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    if ($successorReceiptEntries.Count -eq 1) {
        $successorReceiptEntry = $successorReceiptEntries[0]
        $successorReceipt = $successorReceiptEntry.payload
        Assert-Phase4ExactJsonProperties -Value $successorReceipt -Expected @(
            'schema', 'status', 'task_ref', 'attempt', 'packet_digest', 'worktree_ref',
            'repository_head', 'execution_environment_ref', 'descriptor_digest',
            'source_preflight_descriptor_digest', 'source_preflight_content_digest',
            'source_preflight_receipt_digest', 'role', 'provider_subtree_segment_ref',
            'source_marker_digest', 'process_marker', 'subtree_exit', 'outer_post_exit',
            'boot_id_digest', 'credential_seal_digest', 'continuation',
            'provider_effect_count', 'receipt_digest'
        ) -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        if ([string]$successorReceiptEntry.record.producer_id -cne
                'lattice-managed-codex-worker' -or
            [string]$successorReceiptEntry.record.producer_version -cne '0.1.0' -or
            [string]$successorReceiptEntry.record.producer_digest -cne
                $ExpectedReconcilerProducerDigest -or
            [string]$successorReceipt.schema -cne
                'lattice.wsl2-provider-subtree-receipt/1.0' -or
            [string]$successorReceipt.status -cne 'CLOSED' -or
            [string]$successorReceipt.task_ref -cne $TaskRef -or
            [int]$successorReceipt.attempt -ne 1 -or
            [string]$successorReceipt.packet_digest -cne $ExpectedPacketDigest -or
            [string]$successorReceipt.provider_subtree_segment_ref -cne
                [string]$successorMarker.provider_subtree_segment_ref -or
            [string]$successorReceipt.source_marker_digest -cne
                [string]$successorMarker.marker_digest -or
            [string]$successorReceipt.source_preflight_descriptor_digest -cne
                [string]$successorMarker.source_preflight_descriptor_digest -or
            [string]$successorReceipt.source_preflight_content_digest -cne
                [string]$successorMarker.source_preflight_content_digest -or
            [string]$successorReceipt.source_preflight_receipt_digest -cne
                [string]$successorMarker.source_preflight_receipt_digest -or
            ($successorReceipt.process_marker | ConvertTo-Json -Compress -Depth 12) -cne
                ($successorMarker.process_marker | ConvertTo-Json -Compress -Depth 12) -or
            [long]$successorReceipt.provider_effect_count -ne $ExpectedProviderEffectCount -or
            [string]$successorReceipt.receipt_digest -cnotmatch
                '\Aprovider-subtree-receipt:sha256:[0-9a-f]{64}\z' -or
            [decimal]$successorReceiptEntry.record.ledger_event_sequence -le
                [decimal]$successorMarkerEntry.record.ledger_event_sequence) {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        }
    }
    Assert-Phase4ExactJsonProperties -Value $reconciliation -Expected @(
        'schema', 'status', 'task_ref', 'attempt', 'worktree_ref', 'repository_head',
        'execution_environment_ref', 'descriptor_digest',
        'source_preflight_descriptor_digest', 'source_preflight_content_digest',
        'source_preflight_receipt_digest', 'role', 'provider_subtree_segment_ref',
        'marker_observation', 'source_marker_digest', 'packet_digest', 'process_marker',
        'fence', 'unit', 'cgroup_path', 'boot_id_digest', 'credential_seal_digest',
        'continuation', 'cleanup', 'outer_post_exit', 'provider_effect_count_before',
        'provider_effect_count_after', 'reconciliation_digest'
    ) -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $reconciliation.continuation `
        -Expected @('retry_of', 'reconnect_of') `
        -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $reconciliation.cleanup `
        -Expected @('schema', 'actions') `
        -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $reconciliation.outer_post_exit -Expected @(
        'schema', 'unit', 'fence', 'cgroup_path', 'boot_id_digest', 'active_state',
        'sub_state', 'result', 'delegate', 'cgroup_exists', 'populated'
    ) -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    $outer = $reconciliation.outer_post_exit
    $actions = @($reconciliation.cleanup.actions)
    $expectedActions = @('TERM', 'STOP', 'KILL', 'FORCE_STOP')
    if ($actions.Count -notin @(0, 2, 4)) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    for ($index = 0; $index -lt $actions.Count; $index++) {
        $action = $actions[$index]
        Assert-Phase4ExactJsonProperties -Value $action -Expected @(
            'sequence', 'action', 'result', 'exit_code', 'signal', 'stdout_bytes',
            'stderr_bytes', 'stdout_sha256', 'stderr_sha256'
        ) -Failure 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        if ([long]$action.sequence -ne ($index + 1) -or
            [string]$action.action -cne $expectedActions[$index] -or
            [string]$action.result -cnotin @('SUCCESS', 'EXIT_NONZERO', 'TRANSPORT_ERROR') -or
            [long]$action.stdout_bytes -lt 0 -or [long]$action.stdout_bytes -gt 65536 -or
            [long]$action.stderr_bytes -lt 0 -or [long]$action.stderr_bytes -gt 65536 -or
            [string]$action.stdout_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$action.stderr_sha256 -cnotmatch '\A[0-9a-f]{64}\z') {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        }
    }
    $cgroupClosed = ($outer.cgroup_exists -is [bool]) -and (
        (-not [bool]$outer.cgroup_exists -and $null -eq $outer.populated) -or
        ([bool]$outer.cgroup_exists -and $null -ne $outer.populated -and
            [long]$outer.populated -eq 0)
    )
    if ([string]$reconciliationEntry.record.producer_id -cne
            'lattice-runtime-wsl2-provider-subtree-reconciler' -or
        [string]$reconciliationEntry.record.producer_version -cne '0.1.0' -or
        [string]$reconciliationEntry.record.producer_digest -cne
            $ExpectedReconcilerProducerDigest -or
        [string]$reconciliation.schema -cne
            'lattice.wsl2-provider-subtree-reconciliation/1.0' -or
        [string]$reconciliation.status -cne 'RECONCILED' -or
        [string]$reconciliation.task_ref -cne $TaskRef -or
        [int]$reconciliation.attempt -ne 1 -or
        [string]$reconciliation.worktree_ref -cne $ExpectedWorktreeRef -or
        [string]$reconciliation.repository_head -cne
            [string]$Materialization.descriptor.linux.repository_head -or
        [string]$reconciliation.execution_environment_ref -cne
            [string]$Materialization.record.execution_environment_ref -or
        [string]$reconciliation.descriptor_digest -cne
            (Get-Phase4StringSha256 -Value ([string]$Materialization.descriptor_json)) -or
        [string]$reconciliation.source_preflight_descriptor_digest -cne
            [string]$PreflightEvidence.artifact_descriptor_digest -or
        [string]$reconciliation.source_preflight_content_digest -cne
            [string]$PreflightEvidence.artifact_content_digest -or
        [string]$reconciliation.source_preflight_receipt_digest -cne
            [string]$PreflightEvidence.receipt_digest -or
        [string]$reconciliation.role -cne 'PROVIDER' -or
        [string]$reconciliation.provider_subtree_segment_ref -cne
            [string]$OpenMarkerEvidence.provider_subtree_segment_ref -or
        [string]$reconciliation.marker_observation -cne 'PRESENT' -or
        [string]$reconciliation.source_marker_digest -cne
            [string]$OpenMarkerEvidence.marker_digest -or
        [string]$reconciliation.packet_digest -cne $ExpectedPacketDigest -or
        ($reconciliation.process_marker | ConvertTo-Json -Compress -Depth 12) -cne
            ($rootMarker.process_marker | ConvertTo-Json -Compress -Depth 12) -or
        [string]$reconciliation.fence -cne [string]$rootMarker.process_marker.fence -or
        [string]$reconciliation.unit -cne [string]$rootMarker.process_marker.unit -or
        [string]$reconciliation.cgroup_path -cne [string]$rootMarker.process_marker.cgroup_path -or
        [string]$reconciliation.boot_id_digest -cne [string]$rootMarker.boot_id_digest -or
        [string]$reconciliation.credential_seal_digest -cne
            [string]$rootMarker.credential_seal_digest -or
        $null -ne $reconciliation.continuation.retry_of -or
        $null -ne $reconciliation.continuation.reconnect_of -or
        [string]$reconciliation.cleanup.schema -cne
            'lattice.wsl2-provider-subtree-cleanup/1.0' -or
        [string]$outer.schema -cne 'lattice.wsl2-provider-outer-post-exit/1.0' -or
        [string]$outer.unit -cne [string]$reconciliation.unit -or
        [string]$outer.fence -cne [string]$reconciliation.fence -or
        [string]$outer.cgroup_path -cne [string]$reconciliation.cgroup_path -or
        [string]$outer.boot_id_digest -cne [string]$reconciliation.boot_id_digest -or
        [string]$outer.active_state -cne 'inactive' -or [string]$outer.sub_state -cne 'dead' -or
        [string]$outer.delegate -cne 'no' -or -not $cgroupClosed -or
        [long]$reconciliation.provider_effect_count_before -ne
            $ExpectedProviderEffectCount -or
        [long]$reconciliation.provider_effect_count_after -ne
            $ExpectedProviderEffectCount -or
        [string]$reconciliation.reconciliation_digest -cnotmatch
            '\Aprovider-subtree-reconciliation:sha256:[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
    }
    $rootPreflightSequence = [decimal]$rootPreflightEntry.record.ledger_event_sequence
    $rootMarkerSequence = [decimal]$rootMarkerEntry.record.ledger_event_sequence
    $reconciliationSequence = [decimal]$reconciliationEntry.record.ledger_event_sequence
    $successorPreflightSequence = [decimal]$successorPreflightEntry.record.ledger_event_sequence
    $successorMarkerSequence = [decimal]$successorMarkerEntry.record.ledger_event_sequence
    if ($rootMarkerSequence -le $rootPreflightSequence -or
        $reconciliationSequence -le $rootMarkerSequence -or
        $successorPreflightSequence -le $reconciliationSequence -or
        $successorMarkerSequence -le $successorPreflightSequence -or
        ($RequireSuccessorOpen -and [string]$successorMarker.status -cne 'OPEN')) {
        throw 'PHASE4_WSL2_PROVIDER_SUBTREE_DURABLE_ORDER_REJECTED'
    }
    $chainBinding = [ordered]@{
        reconciliation_descriptor_digest = [string]$reconciliationEntry.record.descriptor_digest
        reconciliation_ledger_event_sequence =
            [string]$reconciliationEntry.record.ledger_event_sequence
        root_marker_descriptor_digest = [string]$rootMarkerEntry.record.descriptor_digest
        root_marker_ledger_event_sequence = [string]$rootMarkerEntry.record.ledger_event_sequence
        root_preflight_descriptor_digest = [string]$rootPreflightEntry.record.descriptor_digest
        root_preflight_ledger_event_sequence =
            [string]$rootPreflightEntry.record.ledger_event_sequence
        successor_marker_descriptor_digest = [string]$successorMarkerEntry.record.descriptor_digest
        successor_marker_ledger_event_sequence =
            [string]$successorMarkerEntry.record.ledger_event_sequence
        successor_preflight_descriptor_digest =
            [string]$successorPreflightEntry.record.descriptor_digest
        successor_preflight_ledger_event_sequence =
            [string]$successorPreflightEntry.record.ledger_event_sequence
    }
    return [pscustomobject][ordered]@{
        value = $reconciliation
        evidence_digest = Get-Phase4StringSha256 -Value (
            [string]$reconciliationEntry.record.evidence_json
        )
        artifact_ref = 'evidence:sha256:' +
            [string]$reconciliationEntry.record.descriptor_digest
        artifact_descriptor_digest = [string]$reconciliationEntry.record.descriptor_digest
        artifact_content_digest = [string]$reconciliationEntry.record.content_digest
        producer_digest = [string]$reconciliationEntry.record.producer_digest
        ledger_event_sequence = [string]$reconciliationEntry.record.ledger_event_sequence
        ledger_event_digest = [string]$reconciliationEntry.record.ledger_event_digest
        successor_preflight = $successorPreflight
        successor_preflight_artifact_ref = 'evidence:sha256:' +
            [string]$successorPreflightEntry.record.descriptor_digest
        successor_preflight_ledger_event_sequence =
            [string]$successorPreflightEntry.record.ledger_event_sequence
        successor_marker = $successorMarker
        successor_marker_artifact_ref = 'evidence:sha256:' +
            [string]$successorMarkerEntry.record.descriptor_digest
        successor_marker_ledger_event_sequence =
            [string]$successorMarkerEntry.record.ledger_event_sequence
        chain_sql_evidence_digest = Get-Phase4StringSha256 -Value $raw
        chain_binding_digest = Get-Phase4StringSha256 -Value (
            $chainBinding | ConvertTo-Json -Compress -Depth 5
        )
        chain_record_count = $records.Count
    }
}

function Get-Phase4WslReviewerSubtreeEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$ProjectId,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)][string]$ExpectedWorktreeRef,
        [Parameter(Mandatory = $true)][string]$ExpectedModelCallIdentity,
        [Parameter(Mandatory = $true)][string]$ExpectedProducerDigest,
        [Parameter(Mandatory = $true)][ValidateRange(0, 16)][int]$ExpectedProviderEffectCount
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ProjectId -cnotmatch '\A[a-z0-9][a-z0-9._-]{1,63}\z' -or
        $ExpectedWorktreeRef -cnotmatch '\Aworktree:sha256:[0-9a-f]{64}\z' -or
        [string]::IsNullOrWhiteSpace($ExpectedModelCallIdentity) -or
        $script:Utf8.GetByteCount($ExpectedModelCallIdentity) -gt 256 -or
        $ExpectedProducerDigest -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    }
    $sql = @"
WITH managed AS (
    SELECT evidence.*,
           pg_catalog.convert_from(evidence.evidence_bytes,'UTF8') AS evidence_json
      FROM foreman_execution.read_managed_evidence_v1(
          decode('$TaskRef','hex'),1::smallint
      )
        AS evidence
     WHERE (
               evidence.payload_schema='lattice.wsl2-zero-model-preflight/1.0'
               AND evidence.producer_id='lattice-managed-semantic-reviewer'
           ) OR (
               evidence.payload_schema IN (
                   'lattice.wsl2-provider-subtree-marker/1.0',
                   'lattice.wsl2-provider-subtree-receipt/1.0',
                   'lattice.wsl2-provider-subtree-reconciliation/1.0'
               )
               AND evidence.evidence_bytes <> ''::bytea
               AND pg_catalog.convert_from(evidence.evidence_bytes,'UTF8')::jsonb->>'role'='REVIEWER'
           )
), replay AS (
    SELECT replay.record_digest, replay.ledger_event_sequence,
           pg_catalog.encode(replay.ledger_event_digest,'hex') AS ledger_event_digest,
           replay.recorded_at
      FROM foreman_execution.read_task_replay_v1(decode('$TaskRef','hex')) AS replay
     WHERE replay.record_kind='ARTIFACT_REFERENCE'
       AND replay.attempt_number=1
), linked AS (
    SELECT managed.project_id, managed.evidence_kind, managed.media_type,
           managed.payload_schema, managed.producer_id, managed.producer_version,
           pg_catalog.encode(managed.producer_digest,'hex') AS producer_digest,
           managed.created_at, managed.evidence_json,
           pg_catalog.encode(managed.content_digest,'hex') AS content_digest,
           pg_catalog.encode(managed.descriptor_digest,'hex') AS descriptor_digest,
           replay.ledger_event_sequence::text AS ledger_event_sequence,
           replay.ledger_event_digest, replay.recorded_at AS ledger_recorded_at
      FROM managed
      JOIN replay ON replay.record_digest=managed.descriptor_digest
)
SELECT pg_catalog.jsonb_build_object(
    'count', pg_catalog.count(*),
    'records', COALESCE(
        pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            'project_id',project_id,'evidence_kind',evidence_kind,
            'media_type',media_type,'payload_schema',payload_schema,
            'producer_id',producer_id,'producer_version',producer_version,
            'producer_digest',producer_digest,'created_at',created_at,
            'evidence_json',evidence_json,'content_digest',content_digest,
            'descriptor_digest',descriptor_digest,
            'ledger_event_sequence',ledger_event_sequence,
            'ledger_event_digest',ledger_event_digest,
            'ledger_recorded_at',ledger_recorded_at
        ) ORDER BY ledger_event_sequence::numeric, descriptor_digest), '[]'::jsonb
    )
) FROM linked;
"@
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database `
        -Sql $sql -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_WSL2_REVIEWER_SUBTREE_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED' }
    $records = @($value.records)
    if ([long]$value.count -ne 3 -or $records.Count -ne 3) {
        throw 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    }
    $entries = [Collections.Generic.List[object]]::new()
    foreach ($record in $records) {
        if ([string]$record.project_id -cne $ProjectId -or
            [string]$record.evidence_kind -cne 'WORKER_LIFECYCLE' -or
            [string]$record.media_type -cne 'application/json' -or
            [string]$record.producer_version -cne '0.1.0' -or
            [string]$record.producer_digest -cne $ExpectedProducerDigest -or
            [string]$record.content_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.descriptor_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.ledger_event_sequence -cnotmatch '\A[1-9][0-9]*\z' -or
            [string]$record.ledger_event_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$record.content_digest -cne
                (Get-Phase4StringSha256 -Value ([string]$record.evidence_json)) -or
            $script:Utf8.GetByteCount([string]$record.evidence_json) -gt 1048576) {
            throw 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
        }
        try { $payload = [string]$record.evidence_json | ConvertFrom-Json -ErrorAction Stop }
        catch { throw 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED' }
        Assert-Phase4NoCredentialShapedJsonStrings -Value $payload `
            -Failure 'PHASE4_WSL2_REVIEWER_SUBTREE_SECRET_REJECTED'
        $entries.Add([pscustomobject][ordered]@{ record = $record; payload = $payload })
    }
    $preflights = @($entries | Where-Object {
        [string]$_.record.payload_schema -ceq 'lattice.wsl2-zero-model-preflight/1.0'
    })
    $markers = @($entries | Where-Object {
        [string]$_.record.payload_schema -ceq 'lattice.wsl2-provider-subtree-marker/1.0'
    })
    $receipts = @($entries | Where-Object {
        [string]$_.record.payload_schema -ceq 'lattice.wsl2-provider-subtree-receipt/1.0'
    })
    if ($preflights.Count -ne 1 -or $markers.Count -ne 1 -or $receipts.Count -ne 1) {
        throw 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    }
    $preflightEntry = $preflights[0]
    $markerEntry = $markers[0]
    $receiptEntry = $receipts[0]
    $preflight = $preflightEntry.payload
    $marker = $markerEntry.payload
    $receipt = $receiptEntry.payload
    Assert-Phase4ExactJsonProperties -Value $preflight -Expected @(
        'schema', 'status', 'task_ref', 'attempt', 'worktree_ref',
        'execution_environment_ref', 'descriptor_digest', 'distribution_identity_ref',
        'linux_cwd', 'repository_head', 'repository_identity', 'codex_home_digest',
        'credential_authority_ref', 'credential_seal_digest',
        'verification_toolchain_ref', 'immutable_snapshot_ref', 'sandbox_policy_ref',
        'privilege_boundary_ref', 'process_fence', 'isolation', 'probes',
        'effect_counters', 'provider_effect_count', 'bounds', 'timeout', 'continuation',
        'connector_auth_ready', 'receipt_digest'
    ) -Failure 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    foreach ($payload in @($marker, $receipt)) {
        Assert-Phase4ExactJsonProperties -Value $payload.continuation `
            -Expected @('retry_of', 'reconnect_of') `
            -Failure 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    }
    Assert-Phase4ExactJsonProperties -Value $marker -Expected @(
        'schema', 'status', 'task_ref', 'attempt', 'packet_digest', 'worktree_ref',
        'repository_head', 'execution_environment_ref', 'descriptor_digest',
        'source_preflight_descriptor_digest', 'source_preflight_content_digest',
        'source_preflight_receipt_digest', 'role', 'provider_subtree_segment_ref',
        'process_marker', 'boot_id_digest', 'credential_seal_digest', 'continuation',
        'provider_effect_count', 'marker_digest', 'subject_digest', 'model_call_identity'
    ) -Failure 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $receipt -Expected @(
        'schema', 'status', 'task_ref', 'attempt', 'packet_digest', 'worktree_ref',
        'repository_head', 'execution_environment_ref', 'descriptor_digest',
        'source_preflight_descriptor_digest', 'source_preflight_content_digest',
        'source_preflight_receipt_digest', 'role', 'provider_subtree_segment_ref',
        'source_marker_digest', 'process_marker', 'subtree_exit', 'outer_post_exit',
        'boot_id_digest', 'credential_seal_digest', 'continuation',
        'provider_effect_count', 'receipt_digest', 'subject_digest', 'model_call_identity'
    ) -Failure 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $receipt.subtree_exit -Expected @(
        'schema', 'fence', 'unit', 'execution_environment_ref', 'credential_seal_digest',
        'cgroup_path', 'zero_descendants', 'credential_seal_intact',
        'credential_watch_intact', 'keyring_daemon_sha256',
        'keyring_library_manifest_digest', 'tool_input_identities', 'stdout_bytes',
        'stderr_bytes', 'stdout_limit_bytes', 'stderr_limit_bytes',
        'output_bound_exceeded', 'timeout_ms', 'timed_out', 'interrupted', 'stdin_bytes',
        'stdin_sha256', 'stdin_complete', 'attempt', 'retry_of', 'reconnect_of',
        'exit_code', 'exit_signal'
    ) -Failure 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $receipt.outer_post_exit -Expected @(
        'schema', 'unit', 'fence', 'cgroup_path', 'boot_id_digest', 'active_state',
        'sub_state', 'result', 'delegate', 'cgroup_exists', 'populated'
    ) -Failure 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    $descriptorDigest = Get-Phase4StringSha256 -Value ([string]$Materialization.descriptor_json)
    $reconnectOf = [string]$preflight.continuation.reconnect_of
    $reviewerFence = [string]$marker.process_marker.fence
    if ($reviewerFence -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    }
    $expectedReviewerUnit = [string]$Materialization.descriptor.process_fence.unit_prefix +
        '-provider-' + $reviewerFence.Substring(0, 12) + '.service'
    $expectedReviewerCgroup = Get-Phase4CanonicalUserServiceCgroupPath `
        -OwnerUid ([long]$Materialization.descriptor.verification_toolchain.owner_uid) `
        -Unit $expectedReviewerUnit
    if ([string]$preflightEntry.record.producer_id -cne 'lattice-managed-semantic-reviewer' -or
        [string]$markerEntry.record.producer_id -cne 'lattice-managed-codex-worker' -or
        [string]$receiptEntry.record.producer_id -cne 'lattice-managed-codex-worker' -or
        [string]$preflight.schema -cne 'lattice.wsl2-zero-model-preflight/1.0' -or
        [string]$preflight.status -cne 'PASS' -or [string]$preflight.task_ref -cne $TaskRef -or
        [int]$preflight.attempt -ne 1 -or
        [string]$preflight.worktree_ref -cne $ExpectedWorktreeRef -or
        [string]$preflight.execution_environment_ref -cne
            [string]$Materialization.record.execution_environment_ref -or
        [string]$preflight.descriptor_digest -cne
            [string]$Materialization.record.execution_environment_ref -or
        [string]$preflight.repository_head -cne
            [string]$Materialization.descriptor.linux.repository_head -or
        [long]$preflight.provider_effect_count -ne 0 -or
        [long]$preflight.effect_counters.provider_effect_count -ne 0 -or
        $null -ne $preflight.continuation.retry_of -or
        $reconnectOf -cnotmatch '\Aattempt-receipt:sha256:[0-9a-f]{64}\z' -or
        [string]$preflight.receipt_digest -cnotmatch
            '\Awsl2-preflight:sha256:[0-9a-f]{64}\z' -or
        [string]$marker.schema -cne 'lattice.wsl2-provider-subtree-marker/1.0' -or
        [string]$marker.status -cne 'OPEN' -or [string]$marker.role -cne 'REVIEWER' -or
        [string]$marker.task_ref -cne $TaskRef -or [int]$marker.attempt -ne 1 -or
        [string]$marker.packet_digest -cnotmatch
            '\Aattempt-packet:sha256:[0-9a-f]{64}\z' -or
        [string]$marker.worktree_ref -cne $ExpectedWorktreeRef -or
        [string]$marker.repository_head -cne
            [string]$Materialization.descriptor.linux.repository_head -or
        [string]$marker.execution_environment_ref -cne
            [string]$Materialization.record.execution_environment_ref -or
        [string]$marker.descriptor_digest -cne $descriptorDigest -or
        [string]$marker.source_preflight_descriptor_digest -cne
            [string]$preflightEntry.record.descriptor_digest -or
        [string]$marker.source_preflight_content_digest -cne
            [string]$preflightEntry.record.content_digest -or
        [string]$marker.source_preflight_receipt_digest -cne
            [string]$preflight.receipt_digest -or
        [string]$marker.model_call_identity -cne $ExpectedModelCallIdentity -or
        [string]$marker.process_marker.unit -cne $expectedReviewerUnit -or
        [string]$marker.process_marker.cgroup_path -cne $expectedReviewerCgroup -or
        [string]$marker.boot_id_digest -cne [string]$marker.process_marker.boot_id_digest -or
        [string]$marker.boot_id_digest -cne [string]$preflight.process_fence.boot_id_digest -or
        [string]$marker.credential_seal_digest -cne
            [string]$marker.process_marker.credential_seal_digest -or
        [string]$marker.credential_seal_digest -cne [string]$preflight.credential_seal_digest -or
        [long]$marker.process_marker.cgroup_version -ne 2 -or
        [bool]$marker.process_marker.delegated -or
        [string]$marker.subject_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$marker.continuation.reconnect_of -cne $reconnectOf -or
        $null -ne $marker.continuation.retry_of -or
        [long]$marker.provider_effect_count -ne 0 -or
        [string]$marker.provider_subtree_segment_ref -cne
            (Get-Phase4WslProviderSubtreeSegmentRef -TaskRef $TaskRef -Attempt 1 `
                -SourcePreflightDescriptorDigest (
                    [string]$preflightEntry.record.descriptor_digest
                ) -SourcePreflightContentDigest ([string]$preflightEntry.record.content_digest) `
                -SourcePreflightReceiptDigest ([string]$preflight.receipt_digest) `
                -Fence ([string]$marker.process_marker.fence) -Role 'REVIEWER' `
                -RetryOf $null -ReconnectOf $reconnectOf `
                -SubjectDigest ([string]$marker.subject_digest) `
                -ModelCallIdentity $ExpectedModelCallIdentity) -or
        [string]$receipt.schema -cne 'lattice.wsl2-provider-subtree-receipt/1.0' -or
        [string]$receipt.status -cne 'CLOSED' -or [string]$receipt.role -cne 'REVIEWER' -or
        [string]$receipt.source_marker_digest -cne [string]$marker.marker_digest -or
        [string]$receipt.provider_subtree_segment_ref -cne
            [string]$marker.provider_subtree_segment_ref -or
        [string]$receipt.packet_digest -cne [string]$marker.packet_digest -or
        [string]$receipt.model_call_identity -cne $ExpectedModelCallIdentity -or
        [string]$receipt.subject_digest -cne [string]$marker.subject_digest -or
        [string]$receipt.source_preflight_descriptor_digest -cne
            [string]$marker.source_preflight_descriptor_digest -or
        [string]$receipt.source_preflight_content_digest -cne
            [string]$marker.source_preflight_content_digest -or
        [string]$receipt.source_preflight_receipt_digest -cne
            [string]$marker.source_preflight_receipt_digest -or
        ($receipt.process_marker | ConvertTo-Json -Compress -Depth 12) -cne
            ($marker.process_marker | ConvertTo-Json -Compress -Depth 12) -or
        [string]$receipt.boot_id_digest -cne [string]$marker.boot_id_digest -or
        [string]$receipt.credential_seal_digest -cne [string]$marker.credential_seal_digest -or
        [string]$receipt.subtree_exit.schema -cne 'lattice.wsl2-subtree-exit/1.2' -or
        [string]$receipt.subtree_exit.fence -cne [string]$marker.process_marker.fence -or
        [string]$receipt.subtree_exit.unit -cne $expectedReviewerUnit -or
        [string]$receipt.subtree_exit.execution_environment_ref -cne
            [string]$marker.process_marker.execution_environment_ref -or
        [string]$receipt.subtree_exit.credential_seal_digest -cne
            [string]$marker.process_marker.credential_seal_digest -or
        [string]$receipt.subtree_exit.cgroup_path -cne $expectedReviewerCgroup -or
        -not [bool]$receipt.subtree_exit.zero_descendants -or
        -not [bool]$receipt.subtree_exit.credential_seal_intact -or
        -not [bool]$receipt.subtree_exit.credential_watch_intact -or
        [int]$receipt.subtree_exit.attempt -ne 1 -or
        $null -ne $receipt.subtree_exit.retry_of -or
        [string]$receipt.subtree_exit.reconnect_of -cne $reconnectOf -or
        [long]$receipt.provider_effect_count -ne $ExpectedProviderEffectCount -or
        [string]$receipt.receipt_digest -cnotmatch
            '\Aprovider-subtree-receipt:sha256:[0-9a-f]{64}\z') {
        throw 'PHASE4_WSL2_REVIEWER_SUBTREE_EVIDENCE_REJECTED'
    }
    $outer = $receipt.outer_post_exit
    $cgroupClosed = ($outer.cgroup_exists -is [bool]) -and (
        (-not [bool]$outer.cgroup_exists -and $null -eq $outer.populated) -or
        ([bool]$outer.cgroup_exists -and $null -ne $outer.populated -and
            [long]$outer.populated -eq 0)
    )
    if ([string]$outer.schema -cne 'lattice.wsl2-provider-outer-post-exit/1.0' -or
        [string]$outer.active_state -cne 'inactive' -or
        [string]$outer.sub_state -cne 'dead' -or [string]$outer.delegate -cne 'no' -or
        [string]$outer.unit -cne $expectedReviewerUnit -or
        [string]$outer.fence -cne [string]$marker.process_marker.fence -or
        [string]$outer.cgroup_path -cne $expectedReviewerCgroup -or
        [string]$outer.boot_id_digest -cne [string]$marker.boot_id_digest -or
        [string]$outer.result -cnotmatch '\A[a-z0-9-]{1,32}\z' -or
        -not $cgroupClosed -or
        [decimal]$markerEntry.record.ledger_event_sequence -le
            [decimal]$preflightEntry.record.ledger_event_sequence -or
        [decimal]$receiptEntry.record.ledger_event_sequence -le
            [decimal]$markerEntry.record.ledger_event_sequence) {
        throw 'PHASE4_WSL2_REVIEWER_SUBTREE_DURABLE_ORDER_REJECTED'
    }
    return [pscustomobject][ordered]@{
        preflight_artifact_ref = 'evidence:sha256:' +
            [string]$preflightEntry.record.descriptor_digest
        marker_artifact_ref = 'evidence:sha256:' + [string]$markerEntry.record.descriptor_digest
        receipt_artifact_ref = 'evidence:sha256:' + [string]$receiptEntry.record.descriptor_digest
        provider_subtree_segment_ref = [string]$marker.provider_subtree_segment_ref
        marker_digest = [string]$marker.marker_digest
        receipt_digest = [string]$receipt.receipt_digest
        model_call_identity = [string]$marker.model_call_identity
        producer_digest = [string]$markerEntry.record.producer_digest
        preflight_ledger_event_sequence = [string]$preflightEntry.record.ledger_event_sequence
        marker_ledger_event_sequence = [string]$markerEntry.record.ledger_event_sequence
        receipt_ledger_event_sequence = [string]$receiptEntry.record.ledger_event_sequence
        sql_evidence_digest = Get-Phase4StringSha256 -Value $raw
    }
}

function Get-Phase4WslCgroupSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)][string]$Unit,
        [Parameter(Mandatory = $true)][string]$CgroupPath,
        [AllowNull()]$OldMarkers
    )

    $descriptor = $Materialization.descriptor
    $canonicalCgroupPath = Get-Phase4CanonicalUserServiceCgroupPath `
        -OwnerUid ([long]$descriptor.verification_toolchain.owner_uid) -Unit $Unit
    if ([string]$descriptor.process_fence.cgroup_mount -cne '/sys/fs/cgroup' -or
        [string]$descriptor.process_fence.supervisor_bootstrap_node.path -cnotmatch
            '\A/(?:[A-Za-z0-9._-]+/)*[A-Za-z0-9._-]+\z' -or
        [string]$descriptor.process_fence.supervisor_bootstrap_node.sha256 -cnotmatch
            '\A[0-9a-f]{64}\z' -or
        $CgroupPath -cne $canonicalCgroupPath) {
        throw 'PHASE4_WSL2_PROVIDER_CGROUP_IDENTITY_REJECTED'
    }
    $old = if ($null -eq $OldMarkers) { @() } else { @($OldMarkers) }
    if ($old.Count -gt 256) { throw 'PHASE4_WSL2_PROVIDER_CGROUP_IDENTITY_REJECTED' }
    $oldJson = $old | ConvertTo-Json -Compress -Depth 8 -AsArray
    $oldBase64 = [Convert]::ToBase64String($script:Utf8.GetBytes([string]$oldJson))
    if ($oldBase64.Length -gt 65536) {
        throw 'PHASE4_WSL2_PROVIDER_CGROUP_IDENTITY_REJECTED'
    }
    $probeSource = @'
"use strict";
const fs = require("node:fs");
const crypto = require("node:crypto");
try {
  const mount = process.env.LATTICE_PHASE4_CGROUP_MOUNT;
  const rootPath = process.env.LATTICE_PHASE4_CGROUP_PATH;
  const unit = process.env.LATTICE_PHASE4_PROVIDER_UNIT;
  const old = JSON.parse(Buffer.from(process.env.LATTICE_PHASE4_OLD_MARKERS_B64, "base64").toString("utf8"));
  const fail = () => { throw new Error("REJECTED"); };
  if (mount !== "/sys/fs/cgroup" || !/^\/(?:[A-Za-z0-9_.@:-]+\/)*[A-Za-z0-9_.@:-]+$/u.test(rootPath)
      || !rootPath.endsWith(`/${unit}`) || !Array.isArray(old) || old.length > 256) fail();
  const boot = fs.readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim();
  if (!/^[0-9a-f-]{36}$/u.test(boot)) fail();
  const bootDigest = `wsl-boot:sha256:${crypto.createHash("sha256").update(boot, "utf8").digest("hex")}`;
  const statFor = (pid) => {
    const text = fs.readFileSync(`/proc/${pid}/stat`, "utf8").trim();
    const boundary = text.lastIndexOf(") ");
    if (boundary < 3 || text.slice(0, text.indexOf(" ")) !== String(pid)) fail();
    const tail = text.slice(boundary + 2).split(" ");
    if (tail.length < 20 || !/^\d+$/u.test(tail[2]) || !/^\d+$/u.test(tail[19])) fail();
    return { process_group_id: tail[2], process_start_ticks: tail[19] };
  };
  const cgroupFor = (pid) => {
    const lines = fs.readFileSync(`/proc/${pid}/cgroup`, "utf8").trim().split("\n");
    const unified = lines.filter((line) => line.startsWith("0::"));
    if (unified.length !== 1 || !/^0::\/(?:[A-Za-z0-9_.@:-]+\/)*[A-Za-z0-9_.@:-]+$/u.test(unified[0])) fail();
    return unified[0].slice(3);
  };
  for (const marker of old) {
    if (!marker || Object.keys(marker).sort().join(",") !==
        "boot_id_digest,cgroup_path,pid,process_group_id,process_start_ticks,schema"
        || marker.schema !== "lattice.wsl2-process-identity/1.0"
        || !/^wsl-boot:sha256:[0-9a-f]{64}$/u.test(marker.boot_id_digest)
        || !Number.isSafeInteger(marker.pid) || marker.pid <= 0
        || !/^\d+$/u.test(marker.process_group_id) || !/^\d+$/u.test(marker.process_start_ticks)
        || typeof marker.cgroup_path !== "string") fail();
  }
  const oldSurvivors = [];
  for (const marker of old) {
    if (marker.boot_id_digest !== bootDigest) continue;
    try {
      const current = statFor(marker.pid);
      if (current.process_start_ticks === marker.process_start_ticks) oldSurvivors.push(marker);
    } catch (error) {
      if (error?.code !== "ENOENT" && error?.code !== "ESRCH") throw error;
    }
  }
  const root = `${mount}${rootPath}`;
  let exists = true;
  try {
    const rootStat = fs.lstatSync(root);
    if (!rootStat.isDirectory() || rootStat.isSymbolicLink()) fail();
  } catch (error) {
    if (error?.code === "ENOENT") exists = false; else throw error;
  }
  let populated = null;
  const processMarkers = [];
  if (exists) {
    const eventLines = fs.readFileSync(`${root}/cgroup.events`, "utf8").trim().split("\n");
    const values = eventLines.filter((line) => /^populated [01]$/u.test(line));
    if (values.length !== 1) fail();
    populated = Number(values[0].slice("populated ".length));
    const pending = [{ fsPath: root, cgroupPath: rootPath }];
    const pids = new Set();
    let visited = 0;
    while (pending.length > 0) {
      const current = pending.shift();
      visited += 1;
      if (visited > 256) fail();
      for (const line of fs.readFileSync(`${current.fsPath}/cgroup.procs`, "utf8").split("\n")) {
        if (line === "") continue;
        if (!/^[1-9]\d*$/u.test(line)) fail();
        pids.add(Number(line));
        if (pids.size > 256) fail();
      }
      for (const entry of fs.readdirSync(current.fsPath, { withFileTypes: true })) {
        if (entry.isSymbolicLink()) fail();
        if (entry.isDirectory()) pending.push({
          fsPath: `${current.fsPath}/${entry.name}`,
          cgroupPath: `${current.cgroupPath}/${entry.name}`,
        });
      }
    }
    for (const pid of [...pids].sort((left, right) => left - right)) {
      try {
        const stat = statFor(pid);
        const cgroupPath = cgroupFor(pid);
        if (cgroupPath !== rootPath && !cgroupPath.startsWith(`${rootPath}/`)) fail();
        processMarkers.push({
          schema: "lattice.wsl2-process-identity/1.0",
          boot_id_digest: bootDigest,
          pid,
          process_start_ticks: stat.process_start_ticks,
          process_group_id: stat.process_group_id,
          cgroup_path: cgroupPath,
        });
      } catch (error) {
        if (error?.code !== "ENOENT" && error?.code !== "ESRCH") throw error;
      }
    }
  }
  process.stdout.write(`${JSON.stringify({
    schema: "lattice.phase4-wsl2-cgroup-snapshot/1.0",
    unit, cgroup_path: rootPath, boot_id_digest: bootDigest, exists, populated,
    process_markers: processMarkers, old_survivors: oldSurvivors,
  })}\n`);
} catch {
  process.exitCode = 1;
}
'@
    $result = Invoke-Phase4WslProcess `
        -Executable ([string]$descriptor.process_fence.supervisor_bootstrap_node.path) `
        -Argument @('-') -Environment ([ordered]@{
            LATTICE_PHASE4_CGROUP_MOUNT = [string]$descriptor.process_fence.cgroup_mount
            LATTICE_PHASE4_CGROUP_PATH = $CgroupPath
            LATTICE_PHASE4_PROVIDER_UNIT = $Unit
            LATTICE_PHASE4_OLD_MARKERS_B64 = $oldBase64
        }) -LinuxHome $script:Wsl2HarnessHome -LinuxTemp $script:Wsl2HarnessTemp `
        -StandardInput $probeSource -TimeoutSeconds 15 `
        -Failure 'PHASE4_WSL2_PROVIDER_CGROUP_QUERY_FAILED' -AllowNonZeroExit
    $lines = @([string]$result.stdout -split '\r?\n' | Where-Object { $_.Length -gt 0 })
    if ([int]$result.exit_code -ne 0 -or [long]$result.stderr_byte_count -ne 0 -or
        $lines.Count -ne 1 -or $script:Utf8.GetByteCount($lines[0]) -gt 65536) {
        throw 'PHASE4_WSL2_PROVIDER_CGROUP_QUERY_REJECTED'
    }
    try { $snapshot = $lines[0] | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_WSL2_PROVIDER_CGROUP_QUERY_REJECTED' }
    Assert-Phase4NoCredentialShapedJsonStrings -Value $snapshot `
        -Failure 'PHASE4_WSL2_PROVIDER_CGROUP_SECRET_REJECTED'
    Assert-Phase4ExactJsonProperties -Value $snapshot -Expected @(
        'schema', 'unit', 'cgroup_path', 'boot_id_digest', 'exists', 'populated',
        'process_markers', 'old_survivors'
    ) -Failure 'PHASE4_WSL2_PROVIDER_CGROUP_QUERY_REJECTED'
    if ([string]$snapshot.schema -cne 'lattice.phase4-wsl2-cgroup-snapshot/1.0' -or
        [string]$snapshot.unit -cne $Unit -or [string]$snapshot.cgroup_path -cne $CgroupPath -or
        [string]$snapshot.boot_id_digest -cnotmatch '\Awsl-boot:sha256:[0-9a-f]{64}\z' -or
        @($snapshot.process_markers).Count -gt 256 -or @($snapshot.old_survivors).Count -gt 256) {
        throw 'PHASE4_WSL2_PROVIDER_CGROUP_QUERY_REJECTED'
    }
    foreach ($marker in @($snapshot.process_markers) + @($snapshot.old_survivors)) {
        Assert-Phase4ExactJsonProperties -Value $marker -Expected @(
            'schema', 'boot_id_digest', 'pid', 'process_start_ticks',
            'process_group_id', 'cgroup_path'
        ) -Failure 'PHASE4_WSL2_PROVIDER_CGROUP_QUERY_REJECTED'
        if ([string]$marker.schema -cne 'lattice.wsl2-process-identity/1.0' -or
            [string]$marker.boot_id_digest -cnotmatch '\Awsl-boot:sha256:[0-9a-f]{64}\z' -or
            [long]$marker.pid -le 0 -or
            [string]$marker.process_start_ticks -cnotmatch '\A[0-9]+\z' -or
            [string]$marker.process_group_id -cnotmatch '\A[0-9]+\z' -or
            [string]$marker.cgroup_path -cnotmatch
                '\A/(?:[A-Za-z0-9_.@:-]+/)*[A-Za-z0-9_.@:-]+\z') {
            throw 'PHASE4_WSL2_PROVIDER_CGROUP_QUERY_REJECTED'
        }
    }
    return [pscustomobject][ordered]@{
        value = $snapshot
        evidence_digest = Get-Phase4StringSha256 -Value $lines[0]
        node_identity = [pscustomobject][ordered]@{
            path = [string]$descriptor.process_fence.supervisor_bootstrap_node.path
            version = [string]$descriptor.process_fence.supervisor_bootstrap_node.version
            sha256 = [string]$descriptor.process_fence.supervisor_bootstrap_node.sha256
        }
        process_evidence = [pscustomobject][ordered]@{
            timeout_seconds = 15
            stdout_byte_count = [long]$script:Utf8.GetByteCount([string]$result.stdout)
            stdout_sha256 = Get-Phase4StringSha256 -Value ([string]$result.stdout)
            stderr_byte_count = [long]$result.stderr_byte_count
            stderr_sha256 = [string]$result.stderr_sha256
            exit_code = [int]$result.exit_code
        }
    }
}

function Get-Phase4WslProviderFenceEvidence {
    param(
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)]$PreflightEvidence,
        [Parameter(Mandatory = $true)][ValidateSet('ACTIVE', 'CLOSED')][string]$Phase,
        [AllowNull()][string]$ExpectedCgroupPath,
        [AllowNull()]$OldMarkers
    )

    $unit = [string]$PreflightEvidence.provider_unit
    $state = Get-Phase4WslSystemdUnitState -Materialization $Materialization -Unit $unit
    $cgroupPath = if ($Phase -ceq 'ACTIVE') {
        [string]$state.cgroup_path
    }
    else {
        [string]$ExpectedCgroupPath
    }
    $canonicalCgroupPath = Get-Phase4CanonicalUserServiceCgroupPath `
        -OwnerUid ([long]$Materialization.descriptor.verification_toolchain.owner_uid) `
        -Unit $unit
    if ($cgroupPath -cne $canonicalCgroupPath) {
        throw 'PHASE4_WSL2_STALE_PROVIDER_FENCE_REJECTED'
    }
    $snapshot = Get-Phase4WslCgroupSnapshot -Materialization $Materialization `
        -Unit $unit -CgroupPath $cgroupPath -OldMarkers $OldMarkers
    $markers = @($snapshot.value.process_markers)
    $survivors = @($snapshot.value.old_survivors)
    if ([string]$snapshot.value.boot_id_digest -cne
        [string]$PreflightEvidence.boot_id_digest) {
        throw 'PHASE4_WSL2_STALE_PROVIDER_FENCE_REJECTED'
    }
    $gatePassed = $true
    if ($Phase -ceq 'ACTIVE') {
        if ([string]$state.load_state -cne 'loaded' -or
            [string]$state.active_state -cne 'active' -or
            [string]$state.sub_state -cne 'running' -or [long]$state.main_pid -le 0 -or
            [string]$state.cgroup_path -cne $cgroupPath -or
            -not [bool]$snapshot.value.exists -or [long]$snapshot.value.populated -ne 1 -or
            $markers.Count -lt 1 -or $survivors.Count -ne 0 -or
            @($markers | Where-Object { [long]$_.pid -eq [long]$state.main_pid }).Count -ne 1) {
            throw 'PHASE4_WSL2_ACTIVE_PROVIDER_FENCE_REJECTED'
        }
    }
    else {
        $cgroupClosed = ((-not [bool]$snapshot.value.exists -and
                $null -eq $snapshot.value.populated) -or
            ([bool]$snapshot.value.exists -and $null -ne $snapshot.value.populated -and
                [long]$snapshot.value.populated -eq 0))
        $gatePassed = -not ([string]$state.load_state -cnotin @('loaded', 'not-found') -or
            [string]$state.active_state -cne 'inactive' -or
            [string]$state.sub_state -cne 'dead' -or [long]$state.main_pid -ne 0 -or
            ([string]$state.cgroup_path -cne '' -and
                [string]$state.cgroup_path -cne $cgroupPath) -or
            -not $cgroupClosed -or $markers.Count -ne 0 -or $survivors.Count -ne 0)
    }
    $subject = [ordered]@{
        schema = 'lattice.phase4-wsl2-provider-fence-evidence/1.0'
        phase = $Phase
        provider_unit = $unit
        process_fence = [string]$PreflightEvidence.process_fence
        cgroup_path = $cgroupPath
        boot_id_digest = [string]$snapshot.value.boot_id_digest
        load_state = [string]$state.load_state
        active_state = [string]$state.active_state
        sub_state = [string]$state.sub_state
        main_pid = [long]$state.main_pid
        cgroup_exists = [bool]$snapshot.value.exists
        cgroup_populated = $snapshot.value.populated
        process_markers = $markers
        exact_old_processes_absent = ($survivors.Count -eq 0)
        gate_passed = $gatePassed
        systemctl_evidence_digest = [string]$state.process_evidence.stdout_sha256
        cgroup_evidence_digest = [string]$snapshot.evidence_digest
    }
    return [pscustomobject][ordered]@{
        value = [pscustomobject]$subject
        evidence_digest = Get-Phase4StringSha256 -Value (
            $subject | ConvertTo-Json -Compress -Depth 12
        )
        systemctl = $state
        cgroup = $snapshot
    }
}

function Get-Phase4TaskSubmissionIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z') { throw 'PHASE4_TASK_REF_REJECTED' }
    $sql = "SELECT pg_catalog.jsonb_build_object(" +
        "'schema_version',s.schema_version,'client_request_id',s.client_request_id," +
        "'project_id',s.project_id,'task_id',s.task_id,'task_ref',s.task_ref," +
        "'admission_action',s.admission_action,'stream_id',pg_catalog.encode(s.stream_id,'hex')," +
        "'envelope_digest',pg_catalog.encode(s.envelope_digest,'hex')) " +
        "FROM ONLY control.task_submission_envelopes s WHERE s.task_ref='$TaskRef';"
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database -Sql $sql `
        -WorkingDirectory $WorkingDirectory -Failure 'PHASE4_TASK_SUBMISSION_IDENTITY_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_TASK_SUBMISSION_IDENTITY_REJECTED' }
    if ([string]$value.schema_version -cne 'lattice.task-ledger.task-submission/1.0' -or
        [string]$value.task_ref -cne $TaskRef -or
        [string]$value.task_id -cnotmatch '\ATASK-GENERAL-[0-9A-F]{40}\z' -or
        [string]$value.admission_action -cne 'GENERAL_TASK_INTAKE_V1' -or
        [string]$value.stream_id -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$value.envelope_digest -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_TASK_SUBMISSION_IDENTITY_REJECTED'
    }
    return $value
}

function Assert-Phase4WslZeroProviderEvidence {
    param([Parameter(Mandatory = $true)]$Evidence)

    $value = $Evidence.value
    if ([long]$value.attempt_count -ne 0 -or
        [long]$value.provider_effect_count -ne 0 -or
        [long]$value.observation_count -ne 0 -or
        [long]$value.artifact_outbox_count -ne 0 -or
        [long]$value.pending_worker_claim_count -ne 0 -or
        [long]$value.environment_count -ne 0 -or
        [long]$value.validated_environment_count -ne 0) {
        throw 'PHASE4_WSL2_PREFLIGHT_PROVIDER_EFFECT_REJECTED'
    }
}

function Assert-Phase4WslExecutionEnvironmentEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)]$Materialization,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string]$RepositoryHead,
        [switch]$RequireReconciled
    )

    $value = $Evidence.value
    $descriptor = $Materialization.descriptor
    $environmentRef = [string]$Materialization.record.execution_environment_ref
    if ([long]$value.attempt_count -ne 1 -or [long]$value.attempt_count -gt 3 -or
        [int]$value.attempt_number -ne 1 -or
        [long]$value.environment_count -ne 1 -or
        [long]$value.validated_environment_count -ne 1 -or
        [string]$value.attempt_execution_environment_ref -cne $environmentRef -or
        [string]$value.environment_ref -cne $environmentRef -or
        [string]$value.canonical_descriptor -cne [string]$Materialization.descriptor_json -or
        [string]$value.descriptor_schema -cne 'lattice.execution-environment.wsl2-linux/1.1' -or
        [string]$value.environment_kind -cne 'WSL2_LINUX' -or
        [string]$value.distribution -cne $script:Wsl2Distribution -or
        [string]$value.linux_repository_path -cne $Repository -or
        [string]$value.repository_head -cne $RepositoryHead -or
        [string]$value.repository_identity_ref -cne [string]$descriptor.linux.repository_identity -or
        [string]$value.credential_authority_kind -cne 'LINUX_KEYRING' -or
        [string]$value.credential_authority_ref -cne
            [string]$descriptor.credential_authority.authority_digest -or
        [string]$value.process_fence_ref -cne [string]$descriptor.process_fence.identity_digest -or
        [string]$value.verification_toolchain_ref -cne
            [string]$descriptor.verification_toolchain.identity_digest -or
        [string]$value.path_mapping_windows_path -cne
            (ConvertTo-Phase4WslUncPath -LinuxPath $Repository) -or
        [string]$value.path_mapping_linux_path -cne $Repository -or
        [string]$value.path_mapping_ref -cne [string]$descriptor.path_mapping.digest -or
        [string]$value.execution_domain_digest -cne $environmentRef.Substring(
            'execution-environment:sha256:'.Length
        ) -or
        [string]$value.launcher_path -cne [string]$descriptor.linux.launcher_path -or
        [string]$value.launcher_version -cne [string]$descriptor.linux.launcher_version -or
        [string]$value.launcher_sha256 -cne [string]$descriptor.linux.launcher_sha256 -or
        [string]$value.node_path -cne [string]$descriptor.linux.node_path -or
        [string]$value.node_version -cne [string]$descriptor.linux.node_version -or
        [string]$value.node_sha256 -cne [string]$descriptor.linux.node_sha256 -or
        [string]$value.git_path -cne [string]$descriptor.linux.git_path -or
        [string]$value.git_version -cne [string]$descriptor.linux.git_version -or
        [string]$value.git_sha256 -cne [string]$descriptor.linux.git_sha256 -or
        [string]$value.supervisor_path -cne [string]$descriptor.linux.supervisor_path -or
        [string]$value.supervisor_sha256 -cne [string]$descriptor.linux.supervisor_sha256 -or
        [long]$value.provider_effect_count -ne 4 -or
        [long]$value.worker_thread_dispatch_count -ne 1 -or
        [long]$value.worker_turn_dispatch_count -ne 1 -or
        [long]$value.review_thread_dispatch_count -ne 1 -or
        [long]$value.review_turn_dispatch_count -ne 1 -or
        [long]$value.artifact_outbox_count -ne 0 -or
        [long]$value.pending_worker_claim_count -ne 0 -or
        [long]$value.thread_count -ne 1 -or [long]$value.turn_count -ne 1 -or
        ($RequireReconciled -and [long]$value.reconciled_count -lt 1)) {
        throw 'PHASE4_WSL2_DURABLE_EXECUTION_ENVIRONMENT_REJECTED'
    }
}

function Invoke-Phase4ControlJson {
    param(
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][ValidateSet('GET', 'POST')][string]$Method,
        [Parameter(Mandatory = $true)][string]$Path,
        [AllowNull()]$Body,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.UseProxy = $false
    $client = [Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(10)
    $request = $null
    $response = $null
    try {
        $uri = [Uri]::new("http://127.0.0.1:$Port$Path")
        $request = [Net.Http.HttpRequestMessage]::new(
            [Net.Http.HttpMethod]::new($Method),
            $uri
        )
        if ($Method -ceq 'POST') {
            $json = $Body | ConvertTo-Json -Compress -Depth 20
            $request.Content = [Net.Http.StringContent]::new(
                $json, $script:Utf8, 'application/json'
            )
        }
        $response = $client.Send($request)
        $content = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        if (-not $response.IsSuccessStatusCode -or $content.Length -gt 1048576) { throw $Failure }
        try { return $content | ConvertFrom-Json -ErrorAction Stop }
        catch { throw $Failure }
    }
    finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $request) { $request.Dispose() }
        $client.Dispose()
        $handler.Dispose()
    }
}

function Get-Phase4ControlProjectDigest {
    param([Parameter(Mandatory = $true)]$Project)

    $projection = [ordered]@{}
    foreach ($name in @(
        $Project.PSObject.Properties.Name |
            Where-Object { [string]$_ -cne 'created' } |
            Sort-Object -CaseSensitive
    )) {
        $projection[[string]$name] = $Project.PSObject.Properties[[string]$name].Value
    }
    return Get-Phase4StringSha256 -Value (
        $projection | ConvertTo-Json -Compress -Depth 20
    )
}

function Start-Phase4Control {
    param(
        [Parameter(Mandatory = $true)][string]$NodeExecutable,
        [Parameter(Mandatory = $true)][string]$ControlHome,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port
    )

    Assert-Phase4Directory -Path $ControlHome -Failure 'PHASE4_CONTROL_HOME_REJECTED'
    if (@(Get-Phase4ListenerPids -Port $Port).Count -ne 0) {
        throw 'PHASE4_CONTROL_PORT_COLLISION'
    }
    $environment = New-Phase4ClosedEnvironment -Values ([ordered]@{
        LOCALAPPDATA = $ControlHome
        LATTICE_CONTROL_PORT = [string]$Port
    })
    $process = Start-Phase4OwnedProcessJob -Executable $NodeExecutable `
        -Argument @($script:ControlServer) -Environment $environment `
        -WorkingDirectory $script:RepositoryRoot -Failure 'PHASE4_CONTROL_START_FAILED'
    $process.StandardInput.Close()
    $stdoutTask = $process.ReadStandardOutputToEndBounded(1048576)
    $stderrTask = $process.ReadStandardErrorToEndBounded(1048576)
    $startedTicks = [long]$process.StartTime.ToUniversalTime().Ticks

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(20)
    $ready = $false
    do {
        if ($process.HasExited) { break }
        $listeners = @(Get-Phase4ListenerPids -Port $Port)
        if ($listeners.Count -eq 1 -and $listeners[0] -eq $process.Id) {
            try {
                $state = Invoke-Phase4ControlJson -Port $Port -Method GET -Path '/api/state' `
                    -Body $null -Failure 'PHASE4_CONTROL_NOT_READY'
                if ($null -ne $state.PSObject.Properties['projects']) {
                    $ready = $true
                    break
                }
            }
            catch {}
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)

    if (-not $ready) {
        try {
            if ([long]$process.ActiveProcessCount() -ne 0) {
                Stop-Phase4OwnedProcessJob -OwnedProcess $process `
                    -Failure 'PHASE4_CONTROL_START_CLEANUP_REJECTED'
            }
        } catch {}
        try { $null = $process.WaitForExit(5000) } catch {}
        $process.Dispose()
        throw 'PHASE4_CONTROL_START_FAILED'
    }
    return [pscustomobject][ordered]@{
        process = $process
        stdout_task = $stdoutTask
        stderr_task = $stderrTask
        process_id = [int]$process.Id
        process_start_utc_ticks = $startedTicks
        executable = [IO.Path]::GetFullPath($NodeExecutable)
        port = $Port
    }
}

function Stop-Phase4Control {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [switch]$SuppressFailure
    )

    $failure = $null
    $cleanupFailure = $null
    $cleanupProven = $false
    $stderr = ''
    try {
        $process = $Session.process
        if (-not $process.HasExited) {
            if ([IO.Path]::GetFullPath($process.Path) -cne [string]$Session.executable -or
                [long]$process.StartTime.ToUniversalTime().Ticks -ne
                [long]$Session.process_start_utc_ticks) {
                throw 'PHASE4_CONTROL_OWNERSHIP_REJECTED'
            }
            Stop-Phase4OwnedProcessJob -OwnedProcess $process `
                -Failure 'PHASE4_CONTROL_STOP_FAILED'
        }
        if (-not $process.WaitForExit(10000)) { throw 'PHASE4_CONTROL_STOP_FAILED' }
        Close-Phase4OwnedProcessJob -OwnedProcess $process `
            -Failure 'PHASE4_CONTROL_STOP_PROOF_REJECTED'
        $stdout = $Session.stdout_task.GetAwaiter().GetResult()
        $stderr = $Session.stderr_task.GetAwaiter().GetResult()
        if ($stdout.Length -gt 1048576 -or $stderr.Length -gt 1048576) {
            throw 'PHASE4_CONTROL_OUTPUT_REJECTED'
        }
        $deadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
        while (@(Get-Phase4ListenerPids -Port ([int]$Session.port)).Count -ne 0 -and
            [DateTimeOffset]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 100
        }
        if (@(Get-Phase4ListenerPids -Port ([int]$Session.port)).Count -ne 0) {
            throw 'PHASE4_CONTROL_STOP_PROOF_REJECTED'
        }
        $cleanupProven = $true
        return [pscustomobject][ordered]@{
            stderr_byte_count = [long]$script:Utf8.GetByteCount([string]$stderr)
            stderr_sha256 = Get-Phase4StringSha256 -Value ([string]$stderr)
            job_empty = $true
            listener_absent = $true
            suppressed_failure = $false
        }
    }
    catch {
        $failure = $_
        try {
            $process = $Session.process
            if ([long]$process.ActiveProcessCount() -ne 0) {
                Stop-Phase4OwnedProcessJob -OwnedProcess $process `
                    -Failure 'PHASE4_CONTROL_STOP_PROOF_REJECTED'
            }
            if (-not $process.WaitForExit(10000)) { throw 'PHASE4_CONTROL_STOP_PROOF_REJECTED' }
            Close-Phase4OwnedProcessJob -OwnedProcess $process `
                -Failure 'PHASE4_CONTROL_STOP_PROOF_REJECTED'
            if ([long]$process.ActiveProcessCount() -ne 0 -or
                @(Get-Phase4ListenerPids -Port ([int]$Session.port)).Count -ne 0) {
                throw 'PHASE4_CONTROL_STOP_PROOF_REJECTED'
            }
            $cleanupProven = $true
        }
        catch { $cleanupFailure = $_ }
    }
    finally {
        try { $Session.process.Dispose() } catch {}
    }
    if ($null -ne $cleanupFailure) { throw $cleanupFailure }
    if ($null -ne $failure -and -not $SuppressFailure) { throw $failure }
    if ($null -ne $failure -and $cleanupProven) {
        return [pscustomobject][ordered]@{
            stderr_byte_count = [long]0
            stderr_sha256 = Get-Phase4StringSha256 -Value ''
            job_empty = $true
            listener_absent = $true
            suppressed_failure = $true
        }
    }
    throw 'PHASE4_CONTROL_STOP_PROOF_REJECTED'
}

function Write-Phase4McpMessage {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)]$Message
    )

    $line = $Message | ConvertTo-Json -Compress -Depth 50
    if ($script:Utf8.GetByteCount($line) -gt 65536) { throw 'PHASE4_MCP_INPUT_REJECTED' }
    if (-not $Session.process.WriteStandardInput($line, $true, 5000)) {
        Stop-Phase4OwnedProcessJob -OwnedProcess $Session.process `
            -Failure 'PHASE4_MCP_INPUT_PROCESS_TREE_CLEANUP_REJECTED'
        throw 'PHASE4_MCP_INPUT_TIMEOUT'
    }
}

function Read-Phase4McpResponse {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)][long]$ExpectedId,
        [Parameter(Mandatory = $true)][ValidateRange(1, 900)][int]$TimeoutSeconds,
        [ValidateRange(0, 900000)][int]$TimeoutMilliseconds = 0
    )

    $effectiveTimeoutMilliseconds = [long]$TimeoutSeconds * 1000
    if ($TimeoutMilliseconds -gt 0) {
        $effectiveTimeoutMilliseconds = [long][Math]::Min(
            $effectiveTimeoutMilliseconds,
            [long]$TimeoutMilliseconds
        )
    }
    $deadline = [DateTimeOffset]::UtcNow.AddMilliseconds($effectiveTimeoutMilliseconds)
    do {
        $remaining = [int][Math]::Max(
            1,
            [Math]::Min(
                [int]::MaxValue,
                [Math]::Ceiling(($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds)
            )
        )
        $task = $Session.process.ReadStandardOutputLineBounded(1048576)
        if (-not $task.Wait($remaining)) { throw 'PHASE4_MCP_RESPONSE_TIMEOUT' }
        $line = $task.Result
        if ($null -eq $line) {
            if ($Session.process.WaitForExit(2000)) {
                $diagnostic = [string]$Session.stderr_task.GetAwaiter().GetResult()
                $diagnostic = [regex]::Replace(
                    $diagnostic,
                    '(?i)(postgres(?:ql)?://)[^@\s]+@',
                    '$1[redacted]@'
                )
                $diagnostic = [regex]::Replace(
                    $diagnostic,
                    '(?i)(password|token|secret)=([^\s;]+)',
                    '$1=[redacted]'
                )
                if ($diagnostic.Length -gt 8192) {
                    $diagnostic = $diagnostic.Substring($diagnostic.Length - 8192)
                }
                [Console]::Error.WriteLine(
                    'LATTICE_MCP_EOF_DIAGNOSTIC:' + [int]$Session.process.ExitCode + ':' + $diagnostic
                )
            }
            throw 'PHASE4_MCP_STDOUT_EOF'
        }
        if ($script:Utf8.GetByteCount($line) -gt 1048576) { throw 'PHASE4_MCP_RESPONSE_REJECTED' }
        try { $response = $line | ConvertFrom-Json -ErrorAction Stop }
        catch { throw 'PHASE4_MCP_RESPONSE_REJECTED' }
        if ([string]$response.jsonrpc -cne '2.0') { throw 'PHASE4_MCP_RESPONSE_REJECTED' }
        if ($null -eq $response.PSObject.Properties['id']) {
            $Session.notification_count = [long]$Session.notification_count + 1
            continue
        }
        if ([long]$response.id -ne $ExpectedId) { throw 'PHASE4_MCP_RESPONSE_ID_REJECTED' }
        if ($null -ne $response.PSObject.Properties['error'] -or
            $null -eq $response.PSObject.Properties['result']) {
            throw 'PHASE4_MCP_JSONRPC_ERROR'
        }
        return $response
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw 'PHASE4_MCP_RESPONSE_TIMEOUT'
}

function Start-Phase4McpSession {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Latticed,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [Parameter(Mandatory = $true)][ValidateRange(1, 300)][int]$TimeoutSeconds
    )

    $closed = New-Phase4ClosedEnvironment -Values $Environment
    $process = Start-Phase4OwnedProcessJob -Executable $Latticed -Argument @() `
        -Environment $closed -WorkingDirectory $script:RepositoryRoot `
        -Failure 'PHASE4_LATTICED_START_FAILED'
    $process.StandardInput.NewLine = "`n"
    $session = [pscustomobject][ordered]@{
        name = $Name
        process = $process
        stderr_task = $process.ReadStandardErrorToEndBounded(8388608)
        process_id = [int]$process.Id
        process_start_utc_ticks = [long]$process.StartTime.ToUniversalTime().Ticks
        executable = [IO.Path]::GetFullPath($Latticed)
        next_id = [long]3
        tool_call_count = [long]0
        notification_count = [long]0
        response_contaminated = $false
    }
    try {
        Write-Phase4McpMessage -Session $session -Message ([ordered]@{
            jsonrpc = '2.0'
            id = 1
            method = 'initialize'
            params = [ordered]@{
                protocolVersion = '2025-11-25'
                capabilities = [ordered]@{}
                clientInfo = [ordered]@{ name = 'lattice-phase4-acceptance'; version = '1.0.0' }
            }
        })
        $initialize = Read-Phase4McpResponse -Session $session -ExpectedId 1 `
            -TimeoutSeconds $TimeoutSeconds
        if ([string]$initialize.result.protocolVersion -cne '2025-11-25' -or
            [string]$initialize.result.serverInfo.name -cne 'latticed') {
            throw 'PHASE4_MCP_INITIALIZE_REJECTED'
        }
        Write-Phase4McpMessage -Session $session -Message ([ordered]@{
            jsonrpc = '2.0'
            method = 'notifications/initialized'
        })
        Write-Phase4McpMessage -Session $session -Message ([ordered]@{
            jsonrpc = '2.0'
            id = 2
            method = 'tools/list'
            params = [ordered]@{}
        })
        $discovery = Read-Phase4McpResponse -Session $session -ExpectedId 2 `
            -TimeoutSeconds $TimeoutSeconds
        $toolNames = @($discovery.result.tools | ForEach-Object { [string]$_.name })
        foreach ($required in @(
            'lattice_foreman_checkpoint', 'lattice_task_submit', 'lattice_task_status'
        )) {
            if ($required -notin $toolNames) { throw 'PHASE4_MCP_TOOL_SURFACE_REJECTED' }
        }
        return $session
    }
    catch {
        try {
            if ([long]$process.ActiveProcessCount() -ne 0) {
                Stop-Phase4OwnedProcessJob -OwnedProcess $process `
                    -Failure 'PHASE4_LATTICED_START_CLEANUP_REJECTED'
            }
        } catch {}
        try { $null = $process.WaitForExit(5000) } catch {}
        $process.Dispose()
        throw
    }
}

function Invoke-Phase4McpTool {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)][string]$ToolName,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Arguments,
        [Parameter(Mandatory = $true)][ValidateRange(1, 900)][int]$TimeoutSeconds,
        [ValidateRange(0, 900000)][int]$TimeoutMilliseconds = 0
    )

    if ($null -ne $Session.PSObject.Properties['response_contaminated'] -and
        [bool]$Session.response_contaminated) {
        throw 'PHASE4_MCP_SESSION_CONTAMINATED'
    }
    if ([long]$Session.tool_call_count -ge $script:MaximumMcpToolCalls) {
        throw 'PHASE4_MCP_CALL_BUDGET_EXHAUSTED'
    }
    $requestId = [long]$Session.next_id
    $Session.next_id = $requestId + 1
    $Session.tool_call_count = [long]$Session.tool_call_count + 1
    Write-Phase4McpMessage -Session $Session -Message ([ordered]@{
        jsonrpc = '2.0'
        id = $requestId
        method = 'tools/call'
        params = [ordered]@{ name = $ToolName; arguments = $Arguments }
    })
    $response = Read-Phase4McpResponse -Session $Session -ExpectedId $requestId `
        -TimeoutSeconds $TimeoutSeconds -TimeoutMilliseconds $TimeoutMilliseconds
    if ([bool]$response.result.isError) {
        $code = [string]$response.result.structuredContent.code
        if ($code -cmatch '\A[A-Z][A-Z0-9_]{0,127}\z') { throw $code }
        throw 'PHASE4_MCP_TOOL_ERROR'
    }
    if ($null -eq $response.result.structuredContent) { throw 'PHASE4_MCP_RESULT_REJECTED' }
    return $response.result.structuredContent
}

function Invoke-Phase4FormalForemanCheckpoint {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][ValidateRange(15, 300)][int]$TimeoutSeconds
    )

    $checkpointId = 'phase4-foreman-' + $RunId.Substring(0, 16)
    $occurredAt = [DateTimeOffset]::UtcNow.ToString(
        'yyyy-MM-ddTHH:mm:ssZ',
        [Globalization.CultureInfo]::InvariantCulture
    )
    $checkpoint = Invoke-Phase4McpTool -Session $Session `
        -ToolName 'lattice_foreman_checkpoint' -Arguments ([ordered]@{
            checkpoint_id = $checkpointId
            generation = 1
            occurred_at = $occurredAt
            state = 'ACTIVE'
            blocker_ref = $null
            heartbeat_ref = 'heartbeat:sha256:' + (
                Get-Phase4StringSha256 -Value ('heartbeat:' + $RunId)
            )
            evidence_ref = 'evidence:sha256:' + (
                Get-Phase4StringSha256 -Value ('evidence:' + $RunId)
            )
        }) -TimeoutSeconds $TimeoutSeconds
    if ([string]$checkpoint.schema -cne 'lattice.foreman-checkpoint-result/1.0' -or
        [long]$checkpoint.generation -ne 1 -or
        [string]$checkpoint.status -cne 'RECORDED' -or
        [bool]$checkpoint.exact_retry -or
        [string]$checkpoint.checkpoint_digest -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_FOREMAN_CHECKPOINT_REJECTED'
    }
    return $checkpoint
}

function New-Phase4GeneralTaskStatusArguments {
    param([Parameter(Mandatory = $true)][string]$TaskRef)

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_TASK_REFERENCE_REJECTED'
    }
    return [ordered]@{ task_ref = $TaskRef }
}

function Stop-Phase4McpSession {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [switch]$SuppressFailure
    )

    $failure = $null
    $cleanupFailure = $null
    $cleanupProven = $false
    $stderr = ''
    try {
        $process = $Session.process
        if (-not $process.HasExited) {
            if ([IO.Path]::GetFullPath($process.Path) -cne [string]$Session.executable -or
                [long]$process.StartTime.ToUniversalTime().Ticks -ne
                [long]$Session.process_start_utc_ticks) {
                throw 'PHASE4_LATTICED_OWNERSHIP_REJECTED'
            }
            $process.StandardInput.Close()
        }
        # The product gives exact bridge reaping and terminal persistence their
        # own bounded 15-second phases. Allow both plus stdio bookkeeping.
        if (-not $process.WaitForExit(40000)) {
            Stop-Phase4OwnedProcessJob -OwnedProcess $process `
                -Failure 'PHASE4_LATTICED_STOP_FAILED'
            throw 'PHASE4_LATTICED_STOP_FAILED'
        }
        Close-Phase4OwnedProcessJob -OwnedProcess $process `
            -Failure 'PHASE4_LATTICED_STOP_FAILED'
        $stderr = $Session.stderr_task.GetAwaiter().GetResult()
        if ($stderr.Length -gt 8388608) { throw 'PHASE4_LATTICED_STDERR_REJECTED' }
        if ([int]$process.ExitCode -ne 0) {
            throw 'PHASE4_LATTICED_EXIT_REJECTED'
        }
        $cleanupProven = $true
        return [pscustomobject][ordered]@{
            process_id = [int]$Session.process_id
            process_start_utc_ticks = [long]$Session.process_start_utc_ticks
            tool_call_count = [long]$Session.tool_call_count
            stderr_byte_count = [long]$script:Utf8.GetByteCount([string]$stderr)
            stderr_sha256 = Get-Phase4StringSha256 -Value ([string]$stderr)
            job_empty = $true
            suppressed_failure = $false
        }
    }
    catch {
        $failure = $_
        try {
            $process = $Session.process
            if ([long]$process.ActiveProcessCount() -ne 0) {
                Stop-Phase4OwnedProcessJob -OwnedProcess $process `
                    -Failure 'PHASE4_LATTICED_STOP_PROOF_REJECTED'
            }
            if (-not $process.WaitForExit(10000)) {
                throw 'PHASE4_LATTICED_STOP_PROOF_REJECTED'
            }
            Close-Phase4OwnedProcessJob -OwnedProcess $process `
                -Failure 'PHASE4_LATTICED_STOP_PROOF_REJECTED'
            if ([long]$process.ActiveProcessCount() -ne 0) {
                throw 'PHASE4_LATTICED_STOP_PROOF_REJECTED'
            }
            $cleanupProven = $true
        }
        catch { $cleanupFailure = $_ }
    }
    finally {
        try { $Session.process.Dispose() } catch {}
    }
    if ($null -ne $cleanupFailure) { throw $cleanupFailure }
    if ($null -ne $failure -and -not $SuppressFailure) { throw $failure }
    if ($null -ne $failure -and $cleanupProven) {
        return [pscustomobject][ordered]@{
            process_id = [int]$Session.process_id
            process_start_utc_ticks = [long]$Session.process_start_utc_ticks
            tool_call_count = [long]$Session.tool_call_count
            stderr_byte_count = [long]0
            stderr_sha256 = Get-Phase4StringSha256 -Value ''
            job_empty = $true
            suppressed_failure = $true
        }
    }
    throw 'PHASE4_LATTICED_STOP_PROOF_REJECTED'
}

function Stop-Phase4McpSessionHard {
    param([Parameter(Mandatory = $true)]$Session)

    $process = $Session.process
    try {
        if ($process.HasExited -or
            [IO.Path]::GetFullPath($process.Path) -cne [string]$Session.executable -or
            [long]$process.StartTime.ToUniversalTime().Ticks -ne
            [long]$Session.process_start_utc_ticks) {
            throw 'PHASE4_LATTICED_OWNERSHIP_REJECTED'
        }
        Stop-Phase4OwnedProcessJob -OwnedProcess $process `
            -Failure 'PHASE4_LATTICED_HARD_STOP_FAILED'
        if (-not $process.WaitForExit(10000)) { throw 'PHASE4_LATTICED_HARD_STOP_FAILED' }
        Close-Phase4OwnedProcessJob -OwnedProcess $process `
            -Failure 'PHASE4_LATTICED_HARD_STOP_PROOF_REJECTED'
        $stderr = $Session.stderr_task.GetAwaiter().GetResult()
        if ($stderr.Length -gt 8388608) { throw 'PHASE4_LATTICED_STDERR_REJECTED' }
        if ([long]$process.ActiveProcessCount() -ne 0) {
            throw 'PHASE4_LATTICED_HARD_STOP_PROOF_REJECTED'
        }
        return [pscustomobject][ordered]@{
            process_id = [int]$Session.process_id
            process_start_utc_ticks = [long]$Session.process_start_utc_ticks
            tool_call_count = [long]$Session.tool_call_count
            hard_killed = $true
            exit_code = [int]$process.ExitCode
            stderr_byte_count = [long]$script:Utf8.GetByteCount([string]$stderr)
            stderr_sha256 = Get-Phase4StringSha256 -Value ([string]$stderr)
        }
    }
    finally {
        try { $process.Dispose() } catch {}
    }
}

function Assert-Phase4ProcessIdentityAbsent {
    param(
        [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][long]$ProcessId,
        [Parameter(Mandatory = $true)][long]$ProcessStartUtcTicks,
        [ValidateRange(1, 30)][int]$TimeoutSeconds = 15
    )

    if ($ProcessStartUtcTicks -le 0) { throw 'PHASE4_PROCESS_IDENTITY_REJECTED' }
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        try { $candidate = Get-Process -Id ([int]$ProcessId) -ErrorAction Stop }
        catch { $candidate = $null }
        if ($null -eq $candidate) {
            return [pscustomobject][ordered]@{
                process_id = $ProcessId
                process_start_utc_ticks = $ProcessStartUtcTicks
                absent = $true
                pid_reused = $false
            }
        }
        if ([long]$candidate.StartTime.ToUniversalTime().Ticks -ne $ProcessStartUtcTicks) {
            return [pscustomobject][ordered]@{
                process_id = $ProcessId
                process_start_utc_ticks = $ProcessStartUtcTicks
                absent = $true
                pid_reused = $true
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw 'PHASE4_EXACT_PROCESS_SURVIVED'
}

function New-Phase4ManagedScriptedFixture {
    param([Parameter(Mandatory = $true)][string]$FixtureId)

    if ($FixtureId -cnotmatch '\A[0-9a-f]{32}\z') {
        throw 'PHASE4_SCRIPTED_FIXTURE_ID_REJECTED'
    }
    $targetRoot = [IO.Path]::GetFullPath((Join-Path $script:RepositoryRoot 'target'))
    Assert-Phase4Directory -Path $targetRoot -Failure 'PHASE4_SCRIPTED_TARGET_REJECTED'
    $fixtureParent = [IO.Path]::GetFullPath((Join-Path $targetRoot 'lattice-delivery'))
    if (-not (Test-Path -LiteralPath $fixtureParent)) {
        [IO.Directory]::CreateDirectory($fixtureParent) | Out-Null
    }
    Assert-Phase4Directory -Path $fixtureParent -Failure 'PHASE4_SCRIPTED_PARENT_REJECTED'
    $fixtureRoot = Assert-Phase4ContainedPath -Root $fixtureParent `
        -Path (Join-Path $fixtureParent $FixtureId) `
        -Failure 'PHASE4_SCRIPTED_FIXTURE_CONTAINMENT_REJECTED'
    if (Test-Path -LiteralPath $fixtureRoot) { throw 'PHASE4_SCRIPTED_FIXTURE_EXISTS' }
    [IO.Directory]::CreateDirectory($fixtureRoot) | Out-Null
    foreach ($directoryName in @('codex-home', 'schema', 'delivery')) {
        [IO.Directory]::CreateDirectory((Join-Path $fixtureRoot $directoryName)) | Out-Null
    }

    $codexHome = Join-Path $fixtureRoot 'codex-home'
    [IO.File]::WriteAllText(
        (Join-Path $codexHome '.lattice-codex-home-v1'),
        "lattice.codex-home.v1`n",
        $script:Utf8
    )
    $codexConfig = Get-Phase4OwnedCodexConfig
    [IO.File]::WriteAllText((Join-Path $codexHome 'config.toml'), $codexConfig, $script:Utf8)

    $serverTemplate = Join-Path `
        $script:RepositoryRoot 'apps\lattice-runtime\src\fixtures\task032-scripted-codex.ps1'
    Assert-Phase4RegularFile -Path $serverTemplate -Failure 'PHASE4_SCRIPTED_SERVER_REJECTED'
    $serverPath = Join-Path $fixtureRoot 'scripted-codex.ps1'
    [IO.File]::WriteAllBytes($serverPath, [IO.File]::ReadAllBytes($serverTemplate))
    $serverSha256 = Get-Phase4FileSha256 -Path $serverPath
    $launcherPath = Join-Path $fixtureRoot 'scripted-codex.cmd'
    $launcherSource = ((@(
        '@echo off',
        'if "%~1"=="--version" if "%~2"=="" goto version',
        'if "%~1"=="app-server" if "%~2"=="generate-json-schema" if "%~3"=="--out" if "%~4" NEQ "" if "%~5"=="" goto schema',
        'if "%~1"=="app-server" if "%~2"=="--listen" if "%~3"=="stdio://" if "%~4"=="" goto server',
        'if "%~1"=="app-server" if "%~2"=="--stdio" if "%~3"=="" goto server',
        'exit /b 11',
        ':version',
        'echo codex-cli 0.144.6',
        'exit /b 0',
        ':schema',
        ('"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%~dp0scripted-codex.ps1" -ExpectedSelfSha256 "' + $serverSha256 + '" -Mode Schema -SchemaRoot "%~4"'),
        'exit /b %ERRORLEVEL%',
        ':server',
        'set LATTICE_DELIVERY_CODEX_MODE=SCRIPTED_ACCEPTANCE',
        ('"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%~dp0scripted-codex.ps1" -ExpectedSelfSha256 "' + $serverSha256 + '" -Mode Server'),
        'exit /b %ERRORLEVEL%'
    ) -join "`r`n") + "`r`n")
    [IO.File]::WriteAllText($launcherPath, $launcherSource, [Text.Encoding]::ASCII)
    $launcherSha256 = Get-Phase4FileSha256 -Path $launcherPath
    $activeMarkerPath = Join-Path $fixtureRoot '.lattice-managed-active-restart-v1'
    [IO.File]::WriteAllText(
        $activeMarkerPath,
        "lattice.phase4.scripted-active-restart.v1`n",
        [Text.Encoding]::ASCII
    )
    $fixtureMarkerPath = Join-Path $fixtureRoot '.lattice-delivery-fixture-v1.json'
    Write-Phase4JsonFile -Path $fixtureMarkerPath -Value ([ordered]@{
        kind = 'LATTICE_DELIVERY_SCRIPTED_ACCEPTANCE_V1'
        fixture_id = $FixtureId
        root = $fixtureRoot
        repository_root = $script:RepositoryRoot
        codex_mode = 'SCRIPTED_ACCEPTANCE'
        launcher_path = $launcherPath
        launcher_sha256 = $launcherSha256
        server_path = $serverPath
        server_sha256 = $serverSha256
    })
    return [pscustomobject][ordered]@{
        root = $fixtureRoot
        launcher = $launcherPath
        launcher_sha256 = $launcherSha256
        server = $serverPath
        server_sha256 = $serverSha256
        schema = Join-Path $fixtureRoot 'schema'
        delivery = Join-Path $fixtureRoot 'delivery'
        codex_home = $codexHome
        state = Join-Path $fixtureRoot 'managed-active-state.json'
        events = Join-Path $fixtureRoot 'managed-active-events.jsonl'
        generations = Join-Path $fixtureRoot 'managed-server-generations.jsonl'
        fixture_marker = $fixtureMarkerPath
        active_marker = $activeMarkerPath
    }
}

function Remove-Phase4ManagedScriptedFixture {
    param(
        [Parameter(Mandatory = $true)]$Fixture,
        [Parameter(Mandatory = $true)][string]$FixtureId
    )

    $fixtureParent = [IO.Path]::GetFullPath(
        (Join-Path $script:RepositoryRoot 'target\lattice-delivery')
    )
    $fixtureRoot = Assert-Phase4ContainedPath -Root $fixtureParent -Path ([string]$Fixture.root) `
        -Failure 'PHASE4_SCRIPTED_FIXTURE_CLEANUP_REJECTED'
    if ([IO.Path]::GetFileName($fixtureRoot) -cne $FixtureId) {
        throw 'PHASE4_SCRIPTED_FIXTURE_CLEANUP_REJECTED'
    }
    Assert-Phase4RegularFile -Path ([string]$Fixture.fixture_marker) `
        -Failure 'PHASE4_SCRIPTED_FIXTURE_CLEANUP_REJECTED'
    Assert-Phase4RegularFile -Path ([string]$Fixture.active_marker) `
        -Failure 'PHASE4_SCRIPTED_FIXTURE_CLEANUP_REJECTED'
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    if (Test-Path -LiteralPath $fixtureRoot) { throw 'PHASE4_SCRIPTED_FIXTURE_CLEANUP_REJECTED' }
}

function Assert-Phase4ManagedStatus {
    param(
        [Parameter(Mandatory = $true)]$Status,
        [Parameter(Mandatory = $true)][string]$ExpectedTaskRef,
        [switch]$Terminal
    )

    if ([string]$Status.task_ref -cne $ExpectedTaskRef -or
        [string]$Status.schema_version -cne 'lattice.task.status.v4' -or
        [string]$Status.ledger_head_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [long]$Status.foreman_generation -lt 1 -or
        [string]$Status.foreman_checkpoint_digest -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_MANAGED_STATUS_REJECTED'
    }
    if ($Terminal) {
        if ([string]$Status.status -cne 'AWAITING_MERGE_APPROVAL' -or
            [string]$Status.task_state -cne 'AWAITING_MERGE_APPROVAL' -or
            [bool]$Status.worker_running -or
            [int]$Status.attempt -ne 1 -or [int]$Status.retry_count -ne 0 -or
            [string]$Status.model -cne 'gpt-5.6-terra' -or
            [string]$Status.reasoning -cne 'medium' -or
            [string]$Status.thread_id -cnotmatch '\A[A-Za-z0-9][A-Za-z0-9._:-]{0,255}\z' -or
            [string]$Status.turn_id -cnotmatch '\A[A-Za-z0-9][A-Za-z0-9._:-]{0,255}\z' -or
            [string]$Status.verification_status -cne 'PASSED' -or
            [string]$Status.verification_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$Status.evidence_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            $null -eq $Status.resource_observation -or
            [string]$Status.resource_observation.external_cost_status -cne 'UNAVAILABLE') {
            throw 'PHASE4_TERMINAL_STATUS_REJECTED'
        }
    }
}

function Assert-Phase4DisabledDraftStatus {
    param(
        [Parameter(Mandatory = $true)]$Status,
        [Parameter(Mandatory = $true)][string]$ExpectedTaskRef,
        [Parameter(Mandatory = $true)][string]$ExpectedProjectId,
        [AllowNull()][string]$ExpectedLedgerHeadDigest = $null,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    $requiredProperties = @(
        'schema_version', 'status', 'task_state', 'task_ref',
        'ledger_head_digest', 'result_digest', 'failure_stage', 'failure_code',
        'objective_summary', 'objective_digest', 'project_id', 'project_name',
        'project_snapshot_id'
    )
    foreach ($name in $requiredProperties) {
        if ($null -eq $Status.PSObject.Properties[[string]$name]) { throw $Failure }
    }
    foreach ($name in @(
        'worker_running', 'attempt', 'retry_count', 'model', 'reasoning',
        'thread_id', 'turn_id', 'last_progress_at', 'blocker',
        'verification_status', 'verification_digest', 'evidence_digest',
        'resource_observation', 'next_action', 'foreman_generation',
        'foreman_checkpoint_digest'
    )) {
        if ($null -ne $Status.PSObject.Properties[[string]$name]) { throw $Failure }
    }
    if (@($Status.PSObject.Properties).Count -ne $requiredProperties.Count -or
        [string]$Status.schema_version -cne 'lattice.task.status.v5' -or
        [string]$Status.status -cne 'SUBMITTED' -or
        [string]$Status.task_state -cne 'DRAFT' -or
        [string]$Status.task_ref -cne $ExpectedTaskRef -or
        $ExpectedTaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$Status.ledger_head_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        $null -ne $Status.result_digest -or
        $null -ne $Status.failure_stage -or
        $null -ne $Status.failure_code -or
        [string]$Status.objective_summary -ceq '' -or
        [string]$Status.objective_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$Status.project_id -cne $ExpectedProjectId -or
        [string]::IsNullOrWhiteSpace([string]$Status.project_name) -or
        [string]::IsNullOrWhiteSpace([string]$Status.project_snapshot_id) -or
        (-not [string]::IsNullOrEmpty($ExpectedLedgerHeadDigest) -and
            [string]$Status.ledger_head_digest -cne $ExpectedLedgerHeadDigest)) {
        throw $Failure
    }
}

function Get-Phase4ManagedStatusDiagnostic {
    param([AllowNull()]$Status)

    if ($null -eq $Status) { return $null }
    $diagnostic = [ordered]@{}
    foreach ($name in @(
        'schema_version', 'task_state', 'status', 'task_ref',
        'ledger_head_digest', 'result_digest', 'failure_stage', 'failure_code',
        'objective_summary', 'objective_digest', 'project_id', 'project_name',
        'project_snapshot_id', 'worker_running', 'attempt', 'retry_count',
        'model', 'thread_id', 'turn_id', 'last_progress_at', 'blocker',
        'verification_status', 'evidence_digest', 'next_action'
    )) {
        $property = $Status.PSObject.Properties[[string]$name]
        if ($null -ne $property) { $diagnostic[[string]$name] = $property.Value }
    }
    $diagnostic['property_names'] = @(
        $Status.PSObject.Properties | ForEach-Object { [string]$_.Name }
    )
    return [pscustomobject]$diagnostic
}

function Invoke-Phase4McpStatusForGate {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][ValidateRange(1, 900)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][ValidateRange(1, 900000)][int]$TimeoutMilliseconds,
        [Parameter(Mandatory = $true)]
        [ValidatePattern('\APHASE4_[A-Z0-9_]{1,120}\z')][string]$TimeoutCode,
        [Parameter(Mandatory = $true)][string]$Stage,
        [Parameter(Mandatory = $true)][ValidateRange(1, 1000)][long]$PollOrdinal,
        [Parameter(Mandatory = $true)][DateTimeOffset]$PollOrigin,
        [Parameter(Mandatory = $true)][long]$RemainingAtDispatchMilliseconds,
        [AllowNull()]$LastCompletedStatus,
        [Parameter(Mandatory = $true)][ref]$TimeoutDiagnostic
    )

    if ($null -eq $Session.PSObject.Properties['response_contaminated']) {
        throw 'PHASE4_MCP_SESSION_SHAPE_REJECTED'
    }
    if ([bool]$Session.response_contaminated) {
        throw 'PHASE4_MCP_SESSION_CONTAMINATED'
    }
    try {
        return Invoke-Phase4McpTool -Session $Session -ToolName 'lattice_task_status' `
            -Arguments (New-Phase4GeneralTaskStatusArguments -TaskRef $TaskRef) `
            -TimeoutSeconds $TimeoutSeconds -TimeoutMilliseconds $TimeoutMilliseconds
    }
    catch {
        if ([string]$_.Exception.Message -ceq 'PHASE4_MCP_RESPONSE_TIMEOUT') {
            # A timed-out asynchronous ReadLine remains outstanding. Mark the
            # exact stdio session unusable before surfacing the typed gate code.
            $Session.response_contaminated = $true
            $TimeoutDiagnostic.Value = [ordered]@{
                stage = $Stage
                request_id = [long]$Session.next_id - 1
                poll_ordinal = [long]$PollOrdinal
                elapsed_at_timeout_milliseconds = [long][Math]::Ceiling(
                    ([DateTimeOffset]::UtcNow - $PollOrigin).TotalMilliseconds
                )
                remaining_at_dispatch_milliseconds = $RemainingAtDispatchMilliseconds
                configured_response_timeout_seconds = $TimeoutSeconds
                effective_response_timeout_milliseconds = [long][Math]::Min(
                    ([long]$TimeoutSeconds * 1000L), [long]$TimeoutMilliseconds
                )
                last_completed_candidate = Get-Phase4ManagedStatusDiagnostic -Status $LastCompletedStatus
            }
            throw $TimeoutCode
        }
        throw
    }
}

function Test-Phase4TransientActiveApprovalGate {
    param([Parameter(Mandatory = $true)]$Status)

    return [string]$Status.status -ceq 'BLOCKED' -and
        [string]$Status.task_state -ceq 'AWAITING_EXECUTION_APPROVAL' -and
        [string]$Status.failure_code -ceq 'LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED' -and
        $null -eq $Status.attempt -and -not [bool]$Status.worker_running -and
        $null -eq $Status.thread_id -and $null -eq $Status.turn_id
}

function Assert-Phase4ActiveManagedStatus {
    param(
        [Parameter(Mandatory = $true)]$Status,
        [Parameter(Mandatory = $true)][string]$ExpectedTaskRef
    )

    Assert-Phase4ManagedStatus -Status $Status -ExpectedTaskRef $ExpectedTaskRef
    if ([string]$Status.task_state -cne 'EXECUTING' -or
        [string]$Status.status -cne 'RUNNING' -or
        -not [bool]$Status.worker_running -or
        [int]$Status.attempt -ne 1 -or [int]$Status.retry_count -ne 0 -or
        [string]$Status.model -cne 'gpt-5.6-terra' -or
        [string]$Status.thread_id -cnotmatch '\A[A-Za-z0-9][A-Za-z0-9._:-]{0,255}\z' -or
        [string]$Status.turn_id -cnotmatch '\A[A-Za-z0-9][A-Za-z0-9._:-]{0,255}\z' -or
        -not [string]::IsNullOrEmpty([string]$Status.blocker)) {
        throw 'PHASE4_ACTIVE_STATUS_REJECTED'
    }
}

function Get-Phase4ActiveRestartEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$ProjectId,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ProjectId -cnotmatch '\A[a-z0-9][a-z0-9._-]{1,63}\z') {
        throw 'PHASE4_ACTIVE_EVIDENCE_IDENTITY_REJECTED'
    }
    $sql = @"
WITH target AS (SELECT decode('$TaskRef','hex') AS task_ref),
attempt AS (
    SELECT count(*) AS attempt_count, min(a.attempt_number) AS attempt_number,
           min(a.attempt_id) AS attempt_id, min(a.writer_fence) AS writer_fence,
           min(a.model::text) AS model, min(a.model_reason::text) AS model_reason,
           min(pg_catalog.encode(a.packet_digest,'hex')) AS packet_digest
      FROM ONLY foreman_execution.worker_attempts a, target t
     WHERE a.task_ref=t.task_ref
), observations AS (
    SELECT count(*) FILTER (WHERE o.observation_kind='THREAD_ACCEPTED') AS thread_count,
           count(*) FILTER (WHERE o.observation_kind='TURN_ACCEPTED') AS turn_count,
           count(*) FILTER (WHERE o.observation_kind='TURN_STARTED') AS turn_started_count,
           count(*) FILTER (WHERE o.observation_kind='RECONCILED') AS reconciled_count,
           count(*) FILTER (WHERE o.observation_kind LIKE 'TERMINAL_%') AS terminal_count,
           min(o.thread_id) FILTER (WHERE o.observation_kind='THREAD_ACCEPTED') AS thread_id,
           min(o.turn_id) FILTER (WHERE o.observation_kind='TURN_STARTED') AS turn_id
      FROM ONLY foreman_execution.worker_observations o, target t
     WHERE o.task_ref=t.task_ref
), dispatch AS (
    SELECT count(*) FILTER (WHERE d.operation_kind='WORKER_THREAD') AS worker_thread_count,
           count(*) FILTER (WHERE d.operation_kind='WORKER_TURN') AS worker_turn_count
      FROM ONLY foreman_execution.provider_dispatch_claims d, target t
     WHERE d.task_ref=t.task_ref
), writer AS (
    SELECT h.current_status::text AS current_status,
           h.current_attempt_id::text AS current_attempt_id,
           h.current_fencing_token AS current_fencing_token,
           h.current_holder_process_id,
           encode(h.current_holder_process_start_identity,'hex') AS current_process_start_identity,
           (SELECT count(*) FROM ONLY writer_lease.writer_lease_transitions x
             WHERE x.project_id=h.project_id AND x.transition_kind='PROCESS_HANDOFF') AS process_handoff_count,
           (SELECT jsonb_agg(convert_from(x.transition_bytes,'UTF8')::jsonb ORDER BY x.ordinal)
              FROM ONLY writer_lease.writer_lease_transitions x
             WHERE x.project_id=h.project_id AND x.transition_kind='PROCESS_HANDOFF') AS process_handoffs
      FROM ONLY writer_lease.writer_lease_heads h
     WHERE h.project_id='$ProjectId'
)
SELECT jsonb_build_object(
    'attempt_count', attempt.attempt_count,
    'attempt_number', attempt.attempt_number,
    'attempt_id', attempt.attempt_id,
    'writer_fence', attempt.writer_fence,
    'packet_digest', 'attempt-packet:sha256:' || attempt.packet_digest,
    'model', attempt.model,
    'model_reason', attempt.model_reason,
    'thread_count', observations.thread_count,
    'turn_count', observations.turn_count,
    'turn_started_count', observations.turn_started_count,
    'reconciled_count', observations.reconciled_count,
    'terminal_count', observations.terminal_count,
    'thread_id', observations.thread_id,
    'turn_id', observations.turn_id,
    'worker_thread_dispatch_count', dispatch.worker_thread_count,
    'worker_turn_dispatch_count', dispatch.worker_turn_count,
    'writer_status', writer.current_status,
    'writer_attempt_id', writer.current_attempt_id,
    'writer_current_fence', writer.current_fencing_token,
    'writer_process_id', writer.current_holder_process_id,
    'writer_process_start_identity', writer.current_process_start_identity,
    'process_handoff_count', writer.process_handoff_count,
    'process_handoffs', writer.process_handoffs
) FROM attempt CROSS JOIN observations CROSS JOIN dispatch CROSS JOIN writer;
"@
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database -Sql $sql `
        -WorkingDirectory $WorkingDirectory -Failure 'PHASE4_ACTIVE_EVIDENCE_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_ACTIVE_EVIDENCE_REJECTED' }
    return [pscustomobject][ordered]@{
        raw = $raw
        digest = Get-Phase4StringSha256 -Value $raw
        value = $value
    }
}

function Read-Phase4BoundedUtf8Lines {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][ValidateRange(1, 4194304)][long]$MaxBytes,
        [Parameter(Mandatory = $true)][ValidateRange(1, 4096)][int]$MaxLines,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65536)][int]$MaxLineBytes,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    Assert-Phase4RegularFile -Path $Path -Failure $Failure
    $item = Get-Item -LiteralPath $Path -Force
    if ([long]$item.Length -gt $MaxBytes) { throw $Failure }
    $stream = [IO.FileStream]::new(
        [IO.Path]::GetFullPath($Path),
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::ReadWrite,
        4096,
        [IO.FileOptions]::SequentialScan
    )
    $reader = [IO.StreamReader]::new($stream, $script:Utf8, $false, 4096, $false)
    $lines = [Collections.Generic.List[string]]::new()
    try {
        while (-not $reader.EndOfStream) {
            $line = $reader.ReadLine()
            if ($null -eq $line -or $script:Utf8.GetByteCount($line) -gt $MaxLineBytes -or
                $lines.Count -ge $MaxLines) {
                throw $Failure
            }
            $lines.Add($line)
        }
        if ([long]$stream.Position -gt $MaxBytes) { throw $Failure }
    }
    finally {
        $reader.Dispose()
    }
    return @($lines)
}

function Get-Phase4ScriptedEventEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$GenerationPath,
        [Parameter(Mandatory = $true)][ValidateRange(1, 32)][int]$ExpectedGenerationCount
    )

    $generationLines = @(Read-Phase4BoundedUtf8Lines -Path $GenerationPath `
        -MaxBytes 32768 -MaxLines 32 -MaxLineBytes 1024 `
        -Failure 'PHASE4_SCRIPTED_GENERATION_LOG_REJECTED')
    if ($generationLines.Count -lt $ExpectedGenerationCount) {
        throw 'PHASE4_SCRIPTED_GENERATION_LOG_MISSING'
    }
    if ($generationLines.Count -gt $ExpectedGenerationCount) {
        throw 'PHASE4_SCRIPTED_GENERATION_LOG_EXTRA'
    }
    $generationIdentities = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::Ordinal
    )
    foreach ($line in $generationLines) {
        try { $generation = $line | ConvertFrom-Json -ErrorAction Stop }
        catch { throw 'PHASE4_SCRIPTED_GENERATION_LOG_REJECTED' }
        if (@($generation.PSObject.Properties).Count -ne 3 -or
            [string]$generation.schema -cne 'lattice.phase4-scripted-server-generation.v1' -or
            [long]$generation.server_pid -le 0 -or
            [long]$generation.server_start_utc_ticks -le 0) {
            throw 'PHASE4_SCRIPTED_GENERATION_LOG_REJECTED'
        }
        $identity = '{0}:{1}' -f [long]$generation.server_pid,
            [long]$generation.server_start_utc_ticks
        if (-not $generationIdentities.Add($identity)) {
            throw 'PHASE4_SCRIPTED_GENERATION_LOG_REJECTED'
        }
    }

    $lines = @(Read-Phase4BoundedUtf8Lines -Path $Path -MaxBytes 1048576 `
        -MaxLines 128 -MaxLineBytes 8192 -Failure 'PHASE4_SCRIPTED_EVENT_LOG_REJECTED')
    $allowed = @(
        'ACCOUNT_READ', 'MODEL_LIST', 'THREAD_LIST', 'THREAD_START', 'TURN_START',
        'THREAD_RESUME', 'THREAD_READ', 'TURN_INTERRUPT',
        'TURN_TERMINAL_ACK', 'SERVER_EXIT'
    )
    $events = [Collections.Generic.List[object]]::new()
    foreach ($line in $lines) {
        try { $event = $line | ConvertFrom-Json -ErrorAction Stop }
        catch { throw 'PHASE4_SCRIPTED_EVENT_LOG_REJECTED' }
        $identity = '{0}:{1}' -f [long]$event.server_pid,
            [long]$event.server_start_utc_ticks
        if (@($event.PSObject.Properties).Count -ne 4 -or
            [string]$event.schema -cne 'lattice.phase4-scripted-server-event.v1' -or
            [string]$event.event -notin $allowed -or
            [long]$event.server_pid -le 0 -or
            [long]$event.server_start_utc_ticks -le 0 -or
            -not $generationIdentities.Contains($identity)) {
            throw 'PHASE4_SCRIPTED_EVENT_LOG_REJECTED'
        }
        $events.Add($event)
    }
    $threadStarts = @($events | Where-Object { [string]$_.event -ceq 'THREAD_START' })
    $resumes = @($events | Where-Object { [string]$_.event -ceq 'THREAD_RESUME' })
    $startIdentity = if ($threadStarts.Count -eq 1) {
        '{0}:{1}' -f [long]$threadStarts[0].server_pid,
            [long]$threadStarts[0].server_start_utc_ticks
    }
    else { $null }
    $resumeIdentities = @($resumes | ForEach-Object {
        '{0}:{1}' -f [long]$_.server_pid, [long]$_.server_start_utc_ticks
    } | Select-Object -Unique)
    $probeIdentities = @($generationIdentities | Where-Object {
        $_ -cne $startIdentity -and $_ -notin $resumeIdentities
    })
    $probeEvents = @($events | Where-Object {
        ('{0}:{1}' -f [long]$_.server_pid, [long]$_.server_start_utc_ticks) -in
        $probeIdentities
    })
    $roleIdentities = @(
        @($probeIdentities)
        $(if ($null -ne $startIdentity) { $startIdentity })
        @($resumeIdentities)
    ) | Select-Object -Unique
    if ($probeIdentities.Count -ne 1 -or $threadStarts.Count -ne 1 -or
        @($probeEvents | Where-Object { [string]$_.event -ceq 'MODEL_LIST' }).Count -lt 1 -or
        @($probeEvents | Where-Object {
            [string]$_.event -notin @('ACCOUNT_READ', 'MODEL_LIST', 'SERVER_EXIT')
        }).Count -ne 0 -or
        $roleIdentities.Count -ne $ExpectedGenerationCount -or
        @($generationIdentities | Where-Object { $_ -notin $roleIdentities }).Count -ne 0) {
        throw 'PHASE4_SCRIPTED_GENERATION_ROLE_REJECTED'
    }
    return [pscustomobject][ordered]@{
        digest = Get-Phase4StringSha256 -Value (($lines -join "`n") + "`n")
        generation_digest = Get-Phase4StringSha256 -Value (($generationLines -join "`n") + "`n")
        generation_count = $generationLines.Count
        probe_server_identity = [string]$probeIdentities[0]
        thread_start_count = $threadStarts.Count
        turn_start_count = @($events | Where-Object { [string]$_.event -ceq 'TURN_START' }).Count
        thread_resume_count = $resumes.Count
        thread_read_count = @($events | Where-Object { [string]$_.event -ceq 'THREAD_READ' }).Count
        turn_interrupt_count = @($events | Where-Object { [string]$_.event -ceq 'TURN_INTERRUPT' }).Count
        terminal_ack_count = @($events | Where-Object { [string]$_.event -ceq 'TURN_TERMINAL_ACK' }).Count
        server_exit_count = @($events | Where-Object { [string]$_.event -ceq 'SERVER_EXIT' }).Count
        start_server_identity = $startIdentity
        start_server_pid = $(if ($threadStarts.Count -eq 1) { [long]$threadStarts[0].server_pid } else { 0 })
        start_server_start_utc_ticks = $(
            if ($threadStarts.Count -eq 1) { [long]$threadStarts[0].server_start_utc_ticks } else { 0 }
        )
        resume_server_identities = $resumeIdentities
        events = @($events)
        line_count = $lines.Count
    }
}

function Get-Phase4GitControlEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Git,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    $head = Invoke-Phase4RepositoryGit -Git $Git -Argument @(
        '-C', $Repository, 'rev-parse', '--verify', 'HEAD'
    ) -Environment $Environment -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_GIT_CONTROL_HEAD_FAILED'
    $status = Invoke-Phase4RepositoryGit -Git $Git -Argument @(
        '-C', $Repository, 'status', '--porcelain=v1', '--untracked-files=all'
    ) -Environment $Environment -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_GIT_CONTROL_STATUS_FAILED'
    $remotes = Invoke-Phase4RepositoryGit -Git $Git -Argument @(
        '-C', $Repository, 'remote'
    ) -Environment $Environment -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_GIT_CONTROL_REMOTE_FAILED'
    $configuration = Invoke-Phase4RepositoryGit -Git $Git -Argument @(
        '-C', $Repository, 'config', '--local', '--null', '--list'
    ) -Environment $Environment -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_GIT_CONTROL_CONFIG_FAILED'
    $references = Invoke-Phase4RepositoryGit -Git $Git -Argument @(
        '-C', $Repository, 'for-each-ref',
        '--format=%(refname)%00%(objectname)%00%(objecttype)', '--',
        'refs/heads', 'refs/lattice'
    ) -Environment $Environment -WorkingDirectory $WorkingDirectory `
        -Failure 'PHASE4_GIT_CONTROL_REFS_FAILED'

    $headCommit = $head.stdout.Trim()
    if ($headCommit -cnotmatch '\A[0-9a-f]{40}\z') {
        throw 'PHASE4_GIT_CONTROL_HEAD_REJECTED'
    }
    $remoteNames = @($remotes.stdout -split '\r?\n' | Where-Object { $_.Length -gt 0 })
    if ($remoteNames.Count -gt 32) { throw 'PHASE4_GIT_CONTROL_REMOTE_REJECTED' }
    $refRows = [Collections.Generic.List[object]]::new()
    foreach ($line in @($references.stdout -split '\r?\n' | Where-Object { $_.Length -gt 0 })) {
        $parts = @($line -split "`0")
        if ($parts.Count -ne 3 -or
            [string]$parts[0] -cnotmatch '\Arefs/(heads|lattice)/[A-Za-z0-9._/-]{1,255}\z' -or
            [string]$parts[1] -cnotmatch '\A[0-9a-f]{40}\z' -or
            [string]$parts[2] -cne 'commit') {
            throw 'PHASE4_GIT_CONTROL_REFS_REJECTED'
        }
        $refRows.Add([pscustomobject][ordered]@{
            ref = [string]$parts[0]
            oid = [string]$parts[1]
            object_type = [string]$parts[2]
        })
    }
    if ($refRows.Count -lt 1 -or $refRows.Count -gt 8) {
        throw 'PHASE4_GIT_CONTROL_REFS_REJECTED'
    }
    $refProjection = @($refRows) | ConvertTo-Json -Compress -Depth 5
    $value = [ordered]@{
        schema = 'lattice.phase4-git-control-evidence/1.0'
        head_commit = $headCommit
        status_clean = [string]::IsNullOrEmpty($status.stdout)
        status_sha256 = Get-Phase4StringSha256 -Value $status.stdout
        remote_count = [long]$remoteNames.Count
        remote_sha256 = Get-Phase4StringSha256 -Value $remotes.stdout
        config_sha256 = Get-Phase4StringSha256 -Value $configuration.stdout
        ref_count = [long]$refRows.Count
        refs = @($refRows)
        refs_sha256 = Get-Phase4StringSha256 -Value $refProjection
    }
    $value.snapshot_digest = Get-Phase4StringSha256 -Value (
        $value | ConvertTo-Json -Compress -Depth 8
    )
    return [pscustomobject]$value
}

function Assert-Phase4GitControlEvidence {
    param(
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After,
        [Parameter(Mandatory = $true)]$Restart,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$BaseCommit,
        [Parameter(Mandatory = $true)][string]$ResultCommit
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $BaseCommit -cnotmatch '\A[0-9a-f]{40}\z' -or
        $ResultCommit -cnotmatch '\A[0-9a-f]{40}\z') {
        throw 'PHASE4_GIT_CONTROL_IDENTITY_REJECTED'
    }
    $taskBranch = 'refs/heads/lattice/task-' + $TaskRef.Substring(0, 59)
    $protectedRef = 'refs/lattice/managed/' + $TaskRef + '/attempt-1'
    $beforeExpected = @('refs/heads/main|' + $BaseCommit + '|commit')
    $afterExpected = @(
        'refs/heads/lattice/task-' + $TaskRef.Substring(0, 59) + '|' + $ResultCommit + '|commit',
        'refs/heads/main|' + $BaseCommit + '|commit',
        $protectedRef + '|' + $ResultCommit + '|commit'
    )
    $beforeActual = @($Before.refs | ForEach-Object {
        [string]$_.ref + '|' + [string]$_.oid + '|' + [string]$_.object_type
    })
    $afterActual = @($After.refs | ForEach-Object {
        [string]$_.ref + '|' + [string]$_.oid + '|' + [string]$_.object_type
    })
    $restartActual = @($Restart.refs | ForEach-Object {
        [string]$_.ref + '|' + [string]$_.oid + '|' + [string]$_.object_type
    })
    foreach ($evidence in @($Before, $After, $Restart)) {
        if ([string]$evidence.schema -cne 'lattice.phase4-git-control-evidence/1.0' -or
            [string]$evidence.head_commit -cne $BaseCommit -or
            $evidence.status_clean -isnot [bool] -or -not [bool]$evidence.status_clean -or
            [long]$evidence.remote_count -ne 0 -or
            [string]$evidence.status_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$evidence.remote_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$evidence.config_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$evidence.refs_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$evidence.snapshot_digest -cnotmatch '\A[0-9a-f]{64}\z') {
            throw 'PHASE4_GIT_CONTROL_PROOF_REJECTED'
        }
    }
    if ((Compare-Object $beforeExpected $beforeActual -SyncWindow 0).Count -ne 0 -or
        (Compare-Object $afterExpected $afterActual -SyncWindow 0).Count -ne 0 -or
        (Compare-Object $afterExpected $restartActual -SyncWindow 0).Count -ne 0 -or
        [string]$Before.config_sha256 -cne [string]$After.config_sha256 -or
        [string]$After.config_sha256 -cne [string]$Restart.config_sha256 -or
        [string]$Before.remote_sha256 -cne [string]$After.remote_sha256 -or
        [string]$After.remote_sha256 -cne [string]$Restart.remote_sha256 -or
        [string]$After.refs_sha256 -cne [string]$Restart.refs_sha256) {
        throw 'PHASE4_GIT_CONTROL_PROOF_REJECTED'
    }

    $proof = [ordered]@{
        schema = 'lattice.phase4-git-control-proof/1.0'
        evidence_scope = 'HEAD_STATUS_REMOTES_LOCAL_CONFIG_AND_REFS'
        source_head = $BaseCommit
        result_commit = $ResultCommit
        task_branch = $taskBranch
        protected_ref = $protectedRef
        before_snapshot_digest = [string]$Before.snapshot_digest
        terminal_snapshot_digest = [string]$After.snapshot_digest
        restart_snapshot_digest = [string]$Restart.snapshot_digest
        config_equal_at_each_snapshot = $true
        remote_count_at_each_snapshot = 0
        source_status_clean_at_each_snapshot = $true
        source_head_equal_at_each_snapshot = $true
        exact_expected_ref_count_at_terminal_and_restart = [long]$afterExpected.Count
    }
    $proof.git_control_proof_digest = Get-Phase4StringSha256 -Value (
        $proof | ConvertTo-Json -Compress -Depth 8
    )
    return [pscustomobject]$proof
}

function Get-Phase4TaskReplayDigest {
    param(
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$ReplayRows
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z' -or
        $ReplayRows.Count -lt 1 -or $ReplayRows.Count -gt 65536) {
        throw 'PHASE4_OWNER_REPLAY_DIGEST_REJECTED'
    }
    $stream = [IO.MemoryStream]::new()
    $writeFrame = {
        param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

        $bytes = $script:Utf8.GetBytes($Value)
        $lengthBytes = [BitConverter]::GetBytes([uint64]$bytes.Length)
        if ([BitConverter]::IsLittleEndian) { [Array]::Reverse($lengthBytes) }
        $stream.Write($lengthBytes, 0, $lengthBytes.Length)
        if ($bytes.Length -gt 0) { $stream.Write($bytes, 0, $bytes.Length) }
    }
    try {
        $domain = $script:Utf8.GetBytes("LATTICE_FOREMAN_TASK_REPLAY_V1`0")
        $stream.Write($domain, 0, $domain.Length)
        & $writeFrame $TaskRef
        & $writeFrame $ReplayRows.Count.ToString([Globalization.CultureInfo]::InvariantCulture)
        foreach ($row in $ReplayRows) {
            $record_state = [string]$row[1]
            $ledger_event_digest = [string]$row[7]
            $recordOrdinal = [uint64]0
            $ledgerEventSequence = [uint64]0
            if (@($row).Count -ne 9 -or
                [string]$row[0] -cnotmatch '\A[A-Z][A-Z0-9_]{0,63}\z' -or
                $record_state -cnotmatch '\A[A-Z][A-Z0-9_]{0,63}\z' -or
                -not [uint64]::TryParse([string]$row[3], [ref]$recordOrdinal) -or
                $recordOrdinal -eq 0 -or
                [string]$row[3] -cne
                $recordOrdinal.ToString([Globalization.CultureInfo]::InvariantCulture) -or
                [string]$row[4] -cnotmatch '\A[0-9a-f]{64}\z' -or
                [string]$row[5] -cnotmatch '\A[0-9a-f]{64}\z' -or
                -not [uint64]::TryParse([string]$row[6], [ref]$ledgerEventSequence) -or
                $ledgerEventSequence -eq 0 -or
                [string]$row[6] -cne
                $ledgerEventSequence.ToString([Globalization.CultureInfo]::InvariantCulture) -or
                $ledger_event_digest -cnotmatch '\A[0-9a-f]{64}\z') {
                throw 'PHASE4_OWNER_REPLAY_DIGEST_REJECTED'
            }
            $attempt = if ($null -eq $row[2]) { '-' } else { [string]$row[2] }
            if ($attempt -cnotmatch '\A(?:-|[1-9][0-9]{0,4})\z') {
                throw 'PHASE4_OWNER_REPLAY_DIGEST_REJECTED'
            }
            foreach ($frame in @(
                [string]$row[0], $record_state, $attempt,
                $recordOrdinal.ToString([Globalization.CultureInfo]::InvariantCulture),
                [string]$row[4], [string]$row[5],
                $ledgerEventSequence.ToString([Globalization.CultureInfo]::InvariantCulture),
                $ledger_event_digest
            )) {
                & $writeFrame $frame
            }
        }
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            return (($sha.ComputeHash($stream.ToArray()) |
                ForEach-Object { $_.ToString('x2') }) -join '')
        }
        finally { $sha.Dispose() }
    }
    finally { $stream.Dispose() }
}

function Get-Phase4DatabaseEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z') { throw 'PHASE4_TASK_REF_REJECTED' }
    $sql = @"
WITH target AS (SELECT decode('$TaskRef','hex') AS task_ref),
replay AS (
    SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_array(
        r.record_kind, r.record_state, r.attempt_number, r.record_ordinal,
        pg_catalog.encode(r.record_digest,'hex'),
        pg_catalog.encode(r.ledger_stream_id,'hex'), r.ledger_event_sequence::text,
        pg_catalog.encode(r.ledger_event_digest,'hex'), r.recorded_at
    ) ORDER BY r.ledger_event_sequence,
        CASE r.record_kind
            WHEN 'TASK_PROMOTION' THEN 1
            WHEN 'WORKER_ATTEMPT' THEN 2
            WHEN 'PROVIDER_DISPATCH_WORKER_THREAD' THEN 3
            WHEN 'PROVIDER_DISPATCH_WORKER_TURN' THEN 4
            WHEN 'PROVIDER_DISPATCH_REVIEW_THREAD' THEN 5
            WHEN 'PROVIDER_DISPATCH_REVIEW_TURN' THEN 6
            WHEN 'WORKER_OBSERVATION' THEN 7
            WHEN 'APPROVAL_EVIDENCE' THEN 8
            WHEN 'ARTIFACT_REFERENCE' THEN 9
            WHEN 'VERIFICATION' THEN 10
            ELSE 32767 END,
        r.record_ordinal, r.record_kind) AS rows
    FROM target t CROSS JOIN LATERAL foreman_execution.read_task_replay_v1(t.task_ref) r
),
provider_dispatch AS (
    SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
        'kind', dispatch.operation_kind,
        'attempt_number', dispatch.attempt_number,
        'attempt_id', dispatch.attempt_id,
        'binding_digest', pg_catalog.encode(dispatch.binding_digest,'hex'),
        'writer_fence', dispatch.writer_fence::text,
        'foreman_generation', dispatch.foreman_generation::text,
        'foreman_checkpoint_digest', pg_catalog.encode(dispatch.foreman_checkpoint_digest,'hex'),
        'anchor_digest', pg_catalog.encode(dispatch.anchor_digest,'hex'),
        'supporting_digest', pg_catalog.encode(dispatch.supporting_digest,'hex'),
        'subject_digest', pg_catalog.encode(dispatch.subject_digest,'hex'),
        'dispatch_digest', pg_catalog.encode(dispatch.dispatch_digest,'hex'),
        'claimed_at', dispatch.claimed_at::text,
        'record_ordinal', CASE dispatch.operation_kind
            WHEN 'WORKER_THREAD' THEN 101 WHEN 'WORKER_TURN' THEN 102
            WHEN 'REVIEW_THREAD' THEN 103 WHEN 'REVIEW_TURN' THEN 104 END,
        'ledger_stream_id', pg_catalog.encode(event.ledger_stream_id,'hex'),
        'ledger_event_sequence', event.ledger_event_sequence::text,
        'ledger_event_digest', pg_catalog.encode(event.ledger_event_digest,'hex'),
        'dependency_linked', CASE dispatch.operation_kind
            WHEN 'WORKER_THREAD' THEN
                dispatch.anchor_digest=attempt.payload_digest AND
                dispatch.supporting_digest=attempt.packet_digest
            WHEN 'WORKER_TURN' THEN EXISTS (
                SELECT 1 FROM ONLY foreman_execution.worker_observations observed
                WHERE observed.task_ref=dispatch.task_ref
                  AND observed.attempt_number=dispatch.attempt_number
                  AND observed.observation_kind='THREAD_ACCEPTED'
                  AND observed.turn_id IS NULL
                  AND observed.payload_digest=dispatch.anchor_digest
                  AND observed.evidence_digest=dispatch.supporting_digest
            ) AND EXISTS (
                SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims predecessor
                WHERE predecessor.task_ref=dispatch.task_ref
                  AND predecessor.attempt_number=dispatch.attempt_number
                  AND predecessor.operation_kind='WORKER_THREAD'
            )
            WHEN 'REVIEW_THREAD' THEN EXISTS (
                SELECT 1 FROM ONLY foreman_execution.worker_observations terminal
                WHERE terminal.task_ref=dispatch.task_ref
                  AND terminal.attempt_number=dispatch.attempt_number
                  AND terminal.observation_kind='TERMINAL_COMPLETED'
                  AND terminal.payload_digest=dispatch.anchor_digest
            ) AND EXISTS (
                SELECT 1 FROM ONLY foreman_execution.artifact_references snapshot
                WHERE snapshot.task_ref=dispatch.task_ref
                  AND snapshot.attempt_number=dispatch.attempt_number
                  AND snapshot.evidence_kind='GIT_SNAPSHOT'
                  AND snapshot.payload_schema='lattice.managed-git-snapshot/1.0'
                  AND snapshot.descriptor_digest=dispatch.supporting_digest
            )
            WHEN 'REVIEW_TURN' THEN EXISTS (
                SELECT 1 FROM ONLY foreman_execution.artifact_references lifecycle
                WHERE lifecycle.task_ref=dispatch.task_ref
                  AND lifecycle.attempt_number=dispatch.attempt_number
                  AND lifecycle.evidence_kind='WORKER_LIFECYCLE'
                  AND lifecycle.payload_schema='lattice.managed-review-lifecycle/1.0'
                  AND lifecycle.descriptor_digest=dispatch.anchor_digest
            ) AND EXISTS (
                SELECT 1 FROM ONLY foreman_execution.provider_dispatch_claims predecessor
                WHERE predecessor.task_ref=dispatch.task_ref
                  AND predecessor.attempt_number=dispatch.attempt_number
                  AND predecessor.operation_kind='REVIEW_THREAD'
                  AND predecessor.supporting_digest=dispatch.supporting_digest
            ) ELSE false END
    ) ORDER BY dispatch.attempt_number, CASE dispatch.operation_kind
        WHEN 'WORKER_THREAD' THEN 1 WHEN 'WORKER_TURN' THEN 2
        WHEN 'REVIEW_THREAD' THEN 3 WHEN 'REVIEW_TURN' THEN 4 END) AS rows
    FROM target t
    JOIN ONLY foreman_execution.provider_dispatch_claims dispatch
      ON dispatch.task_ref=t.task_ref
    JOIN ONLY foreman_execution.worker_attempts attempt
      ON attempt.task_ref=dispatch.task_ref
     AND attempt.attempt_number=dispatch.attempt_number
    JOIN ONLY foreman_execution.child_events event
      ON event.ledger_event_digest=attempt.ledger_event_digest
),
review_lifecycle AS (
    SELECT pg_catalog.convert_from(a.evidence_bytes,'UTF8')::pg_catalog.jsonb AS body
    FROM ONLY foreman_execution.artifact_references a, target t
    WHERE a.task_ref=t.task_ref
      AND a.evidence_kind='WORKER_LIFECYCLE'
      AND a.payload_schema='lattice.managed-review-lifecycle/1.0'
),
review_results AS (
    SELECT pg_catalog.convert_from(a.evidence_bytes,'UTF8')::pg_catalog.jsonb AS body
    FROM ONLY foreman_execution.artifact_references a, target t
    WHERE a.task_ref=t.task_ref
      AND a.evidence_kind='REVIEW_RESULT'
      AND a.payload_schema='lattice.managed-semantic-review-evidence/1.0'
),
resource_calls AS (
    SELECT pg_catalog.convert_from(a.evidence_bytes,'UTF8')::pg_catalog.jsonb AS body,
           a.payload_schema, pg_catalog.encode(a.descriptor_digest,'hex') AS descriptor_digest
    FROM ONLY foreman_execution.artifact_references a, target t
    WHERE a.task_ref=t.task_ref
      AND a.evidence_kind='RESOURCE_OBSERVATION'
      AND a.payload_schema IN (
          'lattice.codex-resource-observation/1.0',
          'lattice.codex-review-resource-observation/1.0'
      )
)
SELECT pg_catalog.jsonb_build_object(
    'promotion_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'promotion_task_spec_digest', (SELECT pg_catalog.encode(p.task_spec_digest,'hex') FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'promotion_approval_subject_digest', (SELECT pg_catalog.encode(p.approval_subject_digest,'hex') FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'promotion_budget_digest', (SELECT pg_catalog.encode(p.budget_digest,'hex') FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'promotion_binding_digest', (SELECT pg_catalog.encode(p.binding_digest,'hex') FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_global_active_limit', (SELECT p.global_active_limit FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_per_task_active_limit', (SELECT p.per_task_active_limit FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_repair_retry_limit', (SELECT p.repair_retry_limit FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_max_attempts', (SELECT p.repair_retry_limit + 1 FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_max_duration_seconds', (SELECT p.max_duration_seconds::text FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_max_total_tokens', (SELECT p.max_total_tokens::text FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_max_model_calls', (SELECT p.max_model_calls::text FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_external_cost_status', (SELECT p.external_cost_status FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_external_cost_limit_micros', (SELECT p.external_cost_limit_micros::text FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'budget_deadline_at', (SELECT p.deadline_at FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'approval_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref),
    'approval_task_spec_digest', (SELECT pg_catalog.encode(a.task_spec_digest,'hex') FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'approval_subject_digest', (SELECT pg_catalog.encode(a.approval_subject_digest,'hex') FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'approval_budget_digest', (SELECT pg_catalog.encode(a.budget_digest,'hex') FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'approval_authority_evidence_digest', (SELECT pg_catalog.encode(a.authority_evidence_digest,'hex') FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'approval_receipt_digest', (SELECT pg_catalog.encode(a.approval_receipt_digest,'hex') FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'approval_authority_digest', (SELECT pg_catalog.encode(a.authority_digest,'hex') FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'approval_capability', (SELECT a.capability FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'approval_issued_at', (SELECT a.issued_at FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'approval_expires_at', (SELECT a.expires_at FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'attempt_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref),
    'attempt_number', (SELECT a.attempt_number FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'attempt_id', (SELECT a.attempt_id FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'model', (SELECT a.model FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'reasoning', (SELECT a.reasoning FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'model_reason', (SELECT a.model_reason FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'attempt_task_spec_digest', (SELECT pg_catalog.encode(a.task_spec_digest,'hex') FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'attempt_binding_digest', (SELECT pg_catalog.encode(a.binding_digest,'hex') FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'attempt_budget_digest', (SELECT pg_catalog.encode(a.budget_digest,'hex') FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'attempt_authority_digest', (SELECT pg_catalog.encode(a.approval_receipt_digest,'hex') FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'attempt_claimed_at', (SELECT a.claimed_at FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'worktree_digest', (SELECT pg_catalog.encode(a.worktree_digest,'hex') FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'writer_fence', (SELECT a.writer_fence::text FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'foreman_generation', (SELECT a.foreman_generation::text FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1),
    'foreman_checkpoint_digest', (SELECT pg_catalog.encode(a.foreman_checkpoint_digest,'hex') FROM ONLY foreman_execution.worker_attempts a, target t WHERE a.task_ref=t.task_ref ORDER BY a.attempt_number DESC LIMIT 1)
) || pg_catalog.jsonb_build_object(
    'thread_count', (SELECT pg_catalog.count(DISTINCT o.thread_id) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref),
    'turn_count', (SELECT pg_catalog.count(DISTINCT o.turn_id) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref AND o.turn_id IS NOT NULL),
    'thread_id', (SELECT o.thread_id FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref ORDER BY o.observation_ordinal DESC LIMIT 1),
    'turn_id', (SELECT o.turn_id FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref AND o.turn_id IS NOT NULL ORDER BY o.observation_ordinal DESC LIMIT 1),
    'turn_started_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref AND o.observation_kind='TURN_STARTED'),
    'terminal_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref AND o.observation_kind IN ('TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')),
    'terminal_kind', (SELECT o.observation_kind FROM ONLY foreman_execution.worker_observations o, target t WHERE o.task_ref=t.task_ref AND o.observation_kind IN ('TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED') ORDER BY o.observation_ordinal DESC LIMIT 1),
    'review_lifecycle_count', (SELECT pg_catalog.count(*) FROM review_lifecycle),
    'review_turn_started_count', (SELECT pg_catalog.count(*) FROM review_lifecycle WHERE body->>'event_type'='TURN_STARTED'),
    'review_terminal_count', (SELECT pg_catalog.count(*) FROM review_lifecycle WHERE body->>'event_type'='TURN_TERMINAL'),
    'review_thread_count', (SELECT pg_catalog.count(DISTINCT body->>'thread_id') FROM review_lifecycle),
    'review_turn_count', (SELECT pg_catalog.count(DISTINCT body->>'turn_id') FROM review_lifecycle WHERE body->>'turn_id' IS NOT NULL),
    'review_thread_id', (SELECT body->>'reviewer_thread_id' FROM review_results LIMIT 1),
    'review_turn_id', (SELECT body->>'reviewer_turn_id' FROM review_results LIMIT 1),
    'review_result_count', (SELECT pg_catalog.count(*) FROM review_results),
    'review_model', (SELECT body->>'model' FROM review_results LIMIT 1),
    'review_reasoning', (SELECT body->>'reasoning' FROM review_results LIMIT 1),
    'review_model_reason', (SELECT body->>'model_reason' FROM review_results LIMIT 1),
    'review_model_call_identity', (SELECT body->>'model_call_identity' FROM review_results LIMIT 1),
    'review_started_at', (SELECT body->>'started_at' FROM review_results LIMIT 1),
    'review_terminal_at', (SELECT body->>'terminal_at' FROM review_results LIMIT 1),
    'review_terminal_status', (SELECT body->>'terminal_status' FROM review_results LIMIT 1),
    'review_resource_digest', (SELECT body->>'resource_digest' FROM review_results LIMIT 1),
    'worker_resource_count', (SELECT pg_catalog.count(*) FROM resource_calls WHERE payload_schema='lattice.codex-resource-observation/1.0'),
    'worker_terminal_resource_count', (SELECT pg_catalog.count(*) FROM resource_calls WHERE payload_schema='lattice.codex-resource-observation/1.0' AND body->>'usage_scope'='CUMULATIVE_TERMINAL'),
    'review_resource_count', (SELECT pg_catalog.count(*) FROM resource_calls WHERE payload_schema='lattice.codex-review-resource-observation/1.0'),
    'model_call_identity_count', (SELECT pg_catalog.count(DISTINCT body->>'model_call_identity') FROM resource_calls),
    'resource_calls', (SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
        'payload_schema', payload_schema, 'descriptor_digest', descriptor_digest, 'body', body
    ) ORDER BY payload_schema, descriptor_digest) FROM resource_calls),
    'verification_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref),
    'verification_outcome', (SELECT v.outcome FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'verification_digest', (SELECT pg_catalog.encode(v.result_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'result_commit_digest', (SELECT pg_catalog.encode(v.result_commit_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'tree_digest', (SELECT pg_catalog.encode(v.tree_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'diff_digest', (SELECT pg_catalog.encode(v.diff_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'review_digest', (SELECT pg_catalog.encode(v.review_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'verification_task_spec_digest', (SELECT pg_catalog.encode(v.task_spec_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'verification_binding_digest', (SELECT pg_catalog.encode(v.binding_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'verification_profile_digest', (SELECT pg_catalog.encode(v.verification_profile_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'verification_evidence_artifact_digest', (SELECT pg_catalog.encode(v.evidence_artifact_digest,'hex') FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1),
    'verification_verified_at', (SELECT v.verified_at FROM ONLY foreman_execution.verification_records v, target t WHERE v.task_ref=t.task_ref ORDER BY v.attempt_number DESC LIMIT 1)
) || pg_catalog.jsonb_build_object(
    'artifact_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.artifact_references a, target t WHERE a.task_ref=t.task_ref),
    'baseline_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.artifact_references a, target t WHERE a.task_ref=t.task_ref AND a.payload_schema='lattice.managed-worktree-baseline/1.0'),
    'baseline_content_digest', (SELECT pg_catalog.encode(a.content_digest,'hex') FROM ONLY foreman_execution.artifact_references a, target t WHERE a.task_ref=t.task_ref AND a.payload_schema='lattice.managed-worktree-baseline/1.0' ORDER BY a.attempt_number DESC LIMIT 1),
    'result_snapshot_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.artifact_references a, target t WHERE a.task_ref=t.task_ref AND a.payload_schema='lattice.managed-git-snapshot/1.0'),
    'protected_result_intent_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.artifact_references a, target t WHERE a.task_ref=t.task_ref AND a.payload_schema='lattice.managed-protected-result-intent/1.0'),
    'protected_result_receipt_count', (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.artifact_references a, target t WHERE a.task_ref=t.task_ref AND a.payload_schema='lattice.managed-protected-result/1.0'),
    'active_replay_count', (SELECT pg_catalog.count(*) FROM foreman_execution.list_active_task_refs_v1(256) a, target t WHERE a.task_ref=t.task_ref),
    'authority_source', (SELECT a.authority_source FROM ONLY foreman_execution.approval_evidence a, target t WHERE a.task_ref=t.task_ref ORDER BY a.authority_digest LIMIT 1),
    'task_created_action', (
        SELECT event.action_id
          FROM ONLY control.task_ledger_events AS event
          JOIN ONLY foreman_execution.task_promotions AS promotion
            ON promotion.successor_stream_id=event.stream_id
          JOIN target t ON t.task_ref=promotion.task_ref
         WHERE event.event_kind='TASK_CREATED'
         ORDER BY event.sequence LIMIT 1
    ),
    'task_state_actions', (
        SELECT pg_catalog.jsonb_agg(event.action_id ORDER BY event.sequence)
          FROM ONLY control.task_ledger_events AS event
          JOIN ONLY foreman_execution.task_promotions AS promotion
            ON promotion.successor_stream_id=event.stream_id
          JOIN target t ON t.task_ref=promotion.task_ref
         WHERE event.event_kind='STATE_TRANSITION'
    ),
    'base_ref', (SELECT p.base_ref FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'base_commit', (SELECT p.base_commit FROM ONLY foreman_execution.task_promotions p, target t WHERE p.task_ref=t.task_ref),
    'provider_dispatches', (SELECT rows FROM provider_dispatch),
    'replay', (SELECT rows FROM replay)
);
"@
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database -Sql $sql `
        -WorkingDirectory $WorkingDirectory -Failure 'PHASE4_DATABASE_EVIDENCE_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_DATABASE_EVIDENCE_REJECTED' }
    $ownerReplayDigest = Get-Phase4TaskReplayDigest -TaskRef $TaskRef -ReplayRows @($value.replay)
    return [pscustomobject][ordered]@{
        raw = $raw
        digest = Get-Phase4StringSha256 -Value $raw
        owner_replay_digest = $ownerReplayDigest
        value = $value
    }
}

function Get-Phase4GitSnapshotEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z') { throw 'PHASE4_TASK_REF_REJECTED' }
    $sql = "SELECT pg_catalog.convert_from(a.evidence_bytes,'UTF8') FROM ONLY " +
        "foreman_execution.artifact_references a WHERE a.task_ref=decode('$TaskRef','hex') " +
        "AND a.evidence_kind='GIT_SNAPSHOT' " +
        "AND a.payload_schema='lattice.managed-git-snapshot/1.0' " +
        "ORDER BY a.attempt_number DESC LIMIT 1;"
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database -Sql $sql `
        -WorkingDirectory $WorkingDirectory -Failure 'PHASE4_GIT_EVIDENCE_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_GIT_EVIDENCE_REJECTED' }
    if ([string]$value.schema -cne 'lattice.managed-git-snapshot/1.0' -or
        [string]$value.base_commit -cnotmatch '\A[0-9a-f]{40}\z' -or
        [string]$value.result_commit -cnotmatch '\A[0-9a-f]{40}\z' -or
        [string]$value.tree -cnotmatch '\A[0-9a-f]{40}\z' -or
        [string]$value.diff_digest -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_GIT_EVIDENCE_REJECTED'
    }
    $checks = @($value.checks)
    $diffCheck = @($checks | Where-Object {
        [string]$_.id -ceq 'git-diff-check-v1' -and
        $_.passed -is [bool] -and [bool]$_.passed
    })
    $focusedCheck = @($checks | Where-Object {
        [string]$_.id -ceq 'trusted-node-plan-v1' -and
        $_.passed -is [bool] -and [bool]$_.passed
    })
    $cargoCheck = @($checks | Where-Object {
        [string]$_.id -ceq 'cargo-test-locked-offline-v1' -and
        $_.passed -is [bool] -and [bool]$_.passed
    })
    $expectedCheckCount = if ($script:Wsl2LinuxLiveEnabled) { 3 } else { 2 }
    $expectedCargoCount = if ($script:Wsl2LinuxLiveEnabled) { 1 } else { 0 }
    if ($checks.Count -ne $expectedCheckCount -or $diffCheck.Count -ne 1 -or
        $focusedCheck.Count -ne 1 -or $cargoCheck.Count -ne $expectedCargoCount) {
        throw 'PHASE4_TRUSTED_VERIFICATION_EVIDENCE_REJECTED'
    }
    return $value
}

function Get-Phase4ManagedBaselineEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$TaskRef,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    if ($TaskRef -cnotmatch '\A[0-9a-f]{64}\z') { throw 'PHASE4_TASK_REF_REJECTED' }
    $sql = "SELECT pg_catalog.convert_from(a.evidence_bytes,'UTF8') FROM ONLY " +
        "foreman_execution.artifact_references a WHERE a.task_ref=decode('$TaskRef','hex') " +
        "AND a.evidence_kind='GIT_SNAPSHOT' " +
        "AND a.payload_schema='lattice.managed-worktree-baseline/1.0' " +
        "ORDER BY a.attempt_number DESC LIMIT 1;"
    $raw = Invoke-Phase4Psql -Password $Password -Port $Port -Database $Database -Sql $sql `
        -WorkingDirectory $WorkingDirectory -Failure 'PHASE4_BASELINE_EVIDENCE_QUERY_FAILED'
    try { $value = $raw | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE4_BASELINE_EVIDENCE_REJECTED' }
    if ([string]$value.schema -cne 'lattice.managed-worktree-baseline/1.0' -or
        [string]$value.task_ref -cne $TaskRef -or
        [string]$value.base_commit -cnotmatch '\A[0-9a-f]{40}\z' -or
        [string]$value.head_commit -cne [string]$value.base_commit -or
        [string]$value.base_tree -cne [string]$value.head_tree -or
        [string]$value.initial_worktree_state -cne 'CLEAN' -or
        [string]$value.index_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$value.git_control_digest -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'PHASE4_BASELINE_EVIDENCE_REJECTED'
    }
    return $value
}

function Assert-Phase4DatabaseEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)]$Status,
        [Parameter(Mandatory = $true)]$GitEvidence
    )

    $value = $Evidence.value
    $expectedTaskStateActions = @(
        'TASK_STATE_AWAITING_EXECUTION_APPROVAL',
        'TASK_STATE_PREPARING',
        'TASK_STATE_EXECUTING',
        'TASK_STATE_VERIFYING',
        'TASK_STATE_REVIEWING',
        'TASK_STATE_AWAITING_MERGE_APPROVAL'
    )
    $taskStateActions = @($value.task_state_actions)
    $digestFields = @(
        'promotion_task_spec_digest', 'promotion_approval_subject_digest',
        'promotion_budget_digest', 'promotion_binding_digest',
        'approval_task_spec_digest', 'approval_subject_digest',
        'approval_budget_digest', 'approval_authority_evidence_digest',
        'approval_authority_digest', 'attempt_task_spec_digest',
        'attempt_binding_digest', 'attempt_budget_digest', 'attempt_authority_digest',
        'verification_task_spec_digest', 'verification_binding_digest',
        'verification_profile_digest', 'verification_evidence_artifact_digest'
    )
    foreach ($field in $digestFields) {
        $digest = [string]$value.$field
        if ($digest -cnotmatch '\A[0-9a-f]{64}\z' -or $digest -ceq ('0' * 64)) {
            throw 'PHASE4_DATABASE_BINDING_REJECTED'
        }
    }
    if ([long]$value.promotion_count -ne 1 -or [long]$value.approval_count -ne 1 -or
        [long]$value.attempt_count -ne 1 -or [int]$value.attempt_number -ne 1 -or
        [string]$value.model -cne 'gpt-5.6-terra' -or
        [string]$value.reasoning -cne 'medium' -or
        [string]$value.model_reason -cne 'ROUTINE_ENGINEERING' -or
        [string]$value.review_model -cne 'gpt-5.6-terra' -or
        [string]$value.review_reasoning -cne 'medium' -or
        [string]$value.review_model_reason -cne 'INDEPENDENT_CODE_REVIEW' -or
        [string]$value.review_terminal_status -cne 'completed' -or
        [long]$value.writer_fence -lt 1 -or [long]$value.foreman_generation -lt 1 -or
        [long]$value.thread_count -ne 1 -or [long]$value.turn_count -ne 1 -or
        [long]$value.turn_started_count -ne 1 -or [long]$value.terminal_count -ne 1 -or
        [string]$value.terminal_kind -cne 'TERMINAL_COMPLETED' -or
        [long]$value.review_lifecycle_count -lt 3 -or
        [long]$value.review_turn_started_count -ne 1 -or
        [long]$value.review_terminal_count -ne 1 -or
        [long]$value.review_thread_count -ne 1 -or
        [long]$value.review_turn_count -ne 1 -or
        [string]$value.review_thread_id -cnotmatch '\A[A-Za-z0-9][A-Za-z0-9._:-]{0,255}\z' -or
        [string]$value.review_turn_id -cnotmatch '\A[A-Za-z0-9][A-Za-z0-9._:-]{0,255}\z' -or
        [string]$value.review_thread_id -ceq [string]$value.thread_id -or
        [string]$value.review_turn_id -ceq [string]$value.turn_id -or
        [long]$value.review_result_count -ne 1 -or
        [long]$value.worker_resource_count -lt 1 -or
        [long]$value.worker_terminal_resource_count -ne 1 -or
        [long]$value.review_resource_count -ne 1 -or
        [long]$value.model_call_identity_count -ne 2 -or
        [long]$value.verification_count -ne 1 -or
        [string]$value.verification_outcome -cne 'PASSED' -or
        [string]$value.review_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [long]$value.artifact_count -lt 3 -or [long]$value.baseline_count -ne 1 -or
        [long]$value.result_snapshot_count -ne 1 -or
        [long]$value.protected_result_intent_count -ne 1 -or
        [long]$value.protected_result_receipt_count -ne 1 -or
        [string]$value.worktree_digest -cne [string]$value.baseline_content_digest -or
        [long]$value.active_replay_count -ne 0 -or
        [string]$value.authority_source -cne 'CLOSED_POLICY_NO_APPROVAL_REQUIRED' -or
        [string]$value.approval_capability -cne 'LOCAL_REVERSIBLE_TASK_EXECUTION' -or
        $null -ne $value.approval_receipt_digest -or
        [string]$value.promotion_task_spec_digest -cne
        [string]$value.approval_task_spec_digest -or
        [string]$value.approval_task_spec_digest -cne
        [string]$value.attempt_task_spec_digest -or
        [string]$value.attempt_task_spec_digest -cne
        [string]$value.verification_task_spec_digest -or
        [string]$value.promotion_approval_subject_digest -cne
        [string]$value.approval_subject_digest -or
        [string]$value.promotion_budget_digest -cne [string]$value.approval_budget_digest -or
        [string]$value.approval_budget_digest -cne [string]$value.attempt_budget_digest -or
        [string]$value.promotion_binding_digest -cne [string]$value.attempt_binding_digest -or
        [string]$value.attempt_binding_digest -cne [string]$value.verification_binding_digest -or
        [string]$value.approval_authority_digest -cne
        [string]$value.attempt_authority_digest -or
        [int]$value.budget_global_active_limit -ne 4 -or
        [int]$value.budget_per_task_active_limit -ne 1 -or
        [int]$value.budget_repair_retry_limit -ne 2 -or
        [int]$value.budget_max_attempts -ne 3 -or
        [long]$value.budget_max_duration_seconds -ne 900 -or
        [long]$value.budget_max_total_tokens -ne 100000 -or
        [long]$value.budget_max_model_calls -ne 6 -or
        [string]$value.budget_external_cost_status -cne 'UNAVAILABLE' -or
        $null -ne $value.budget_external_cost_limit_micros -or
        [string]$value.task_created_action -cne 'MANAGED_GENERAL_TASK_V1' -or
        $taskStateActions.Count -ne $expectedTaskStateActions.Count -or
        (Compare-Object -ReferenceObject $expectedTaskStateActions -DifferenceObject $taskStateActions `
            -SyncWindow 0).Count -ne 0 -or
        [string]$value.thread_id -cne [string]$Status.thread_id -or
        [string]$value.turn_id -cne [string]$Status.turn_id -or
        [string]$value.verification_digest -cne [string]$Status.verification_digest -or
        [string]$Evidence.owner_replay_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$Evidence.owner_replay_digest -cne [string]$Status.evidence_digest -or
        [string]$value.foreman_checkpoint_digest -cne
        [string]$Status.foreman_checkpoint_digest -or
        [string]$value.base_commit -cne [string]$GitEvidence.base_commit -or
        [string]$value.diff_digest -cne [string]$GitEvidence.diff_digest -or
        [string]$value.result_commit_digest -cne
        (Get-Phase4StringSha256 -Value ([string]$GitEvidence.result_commit)) -or
        [string]$value.tree_digest -cne
        (Get-Phase4StringSha256 -Value ([string]$GitEvidence.tree))) {
        throw 'PHASE4_DATABASE_EVIDENCE_REJECTED'
    }

    $budgetDeadline = [DateTimeOffset]::MinValue
    $approvalIssued = [DateTimeOffset]::MinValue
    $approvalExpires = [DateTimeOffset]::MinValue
    $attemptClaimed = [DateTimeOffset]::MinValue
    $reviewStarted = [DateTimeOffset]::MinValue
    $reviewTerminal = [DateTimeOffset]::MinValue
    $verifiedAt = [DateTimeOffset]::MinValue
    $dateStyle = [Globalization.DateTimeStyles]::AssumeUniversal -bor
        [Globalization.DateTimeStyles]::AdjustToUniversal
    if (-not [DateTimeOffset]::TryParse(
            [string]$value.budget_deadline_at, [Globalization.CultureInfo]::InvariantCulture,
            $dateStyle, [ref]$budgetDeadline
        ) -or -not [DateTimeOffset]::TryParse(
            [string]$value.approval_issued_at, [Globalization.CultureInfo]::InvariantCulture,
            $dateStyle, [ref]$approvalIssued
        ) -or -not [DateTimeOffset]::TryParse(
            [string]$value.approval_expires_at, [Globalization.CultureInfo]::InvariantCulture,
            $dateStyle, [ref]$approvalExpires
        ) -or -not [DateTimeOffset]::TryParse(
            [string]$value.attempt_claimed_at, [Globalization.CultureInfo]::InvariantCulture,
            $dateStyle, [ref]$attemptClaimed
        ) -or -not [DateTimeOffset]::TryParse(
            [string]$value.review_started_at, [Globalization.CultureInfo]::InvariantCulture,
            $dateStyle, [ref]$reviewStarted
        ) -or -not [DateTimeOffset]::TryParse(
            [string]$value.review_terminal_at, [Globalization.CultureInfo]::InvariantCulture,
            $dateStyle, [ref]$reviewTerminal
        ) -or -not [DateTimeOffset]::TryParse(
            [string]$value.verification_verified_at, [Globalization.CultureInfo]::InvariantCulture,
            $dateStyle, [ref]$verifiedAt
        ) -or $approvalExpires -ne $budgetDeadline -or
        $approvalIssued -gt $attemptClaimed -or $attemptClaimed -gt $reviewStarted -or
        $reviewStarted -gt $reviewTerminal -or $reviewTerminal -gt $verifiedAt -or
        $verifiedAt -gt $budgetDeadline) {
        throw 'PHASE4_DATABASE_DEADLINE_REJECTED'
    }

    $resourceCalls = @($value.resource_calls)
    $workerResources = @($resourceCalls | Where-Object {
        [string]$_.payload_schema -ceq 'lattice.codex-resource-observation/1.0' -and
        [string]$_.body.usage_scope -ceq 'CUMULATIVE_TERMINAL'
    })
    $reviewResources = @($resourceCalls | Where-Object {
        [string]$_.payload_schema -ceq 'lattice.codex-review-resource-observation/1.0'
    })
    if ($resourceCalls.Count -ne (
            [long]$value.worker_resource_count + [long]$value.review_resource_count
        ) -or $workerResources.Count -ne 1 -or
        $reviewResources.Count -ne 1) {
        throw 'PHASE4_RESOURCE_EVIDENCE_REJECTED'
    }
    $workerResource = $workerResources[0]
    $reviewResource = $reviewResources[0]
    $workerBody = $workerResource.body
    $reviewBody = $reviewResource.body
    $workerIdentity = [string]$workerBody.model_call_identity
    $reviewIdentity = [string]$reviewBody.model_call_identity
    $workerTokens = 0L
    $reviewTokens = 0L
    if ([string]$workerResource.descriptor_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$reviewResource.descriptor_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$workerBody.schema -cne 'lattice.codex-resource-observation/1.0' -or
        [string]$workerBody.usage_scope -cne 'CUMULATIVE_TERMINAL' -or
        [string]$workerBody.external_cost_status -cne 'UNAVAILABLE' -or
        [string]$workerBody.event_evidence_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        $workerIdentity -cnotmatch '\Amodel-call:sha256:[0-9a-f]{64}\z' -or
        $null -eq $workerBody.total_tokens -or
        -not [long]::TryParse([string]$workerBody.total_tokens, [ref]$workerTokens) -or
        [string]$reviewBody.schema -cne
        'lattice.codex-review-resource-observation/1.0' -or
        [string]$reviewBody.external_cost_status -cne 'UNAVAILABLE' -or
        [string]$reviewBody.model_calls -cne '1' -or
        $reviewIdentity -cne ('managed-review-' + [string]$Status.task_ref + '-1') -or
        $reviewIdentity -cne [string]$value.review_model_call_identity -or
        [string]$reviewBody.review_evidence_digest -cne [string]$value.review_digest -or
        [string]$value.review_resource_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        $null -eq $reviewBody.total_tokens -or
        -not [long]::TryParse([string]$reviewBody.total_tokens, [ref]$reviewTokens) -or
        $workerIdentity -ceq $reviewIdentity) {
        throw 'PHASE4_RESOURCE_EVIDENCE_REJECTED'
    }
    $budgetTokens = [long]$value.budget_max_total_tokens
    $observedModelCalls = 2L
    if ($workerTokens -lt 0 -or $reviewTokens -lt 0 -or
        $workerTokens -gt $budgetTokens -or $reviewTokens -gt ($budgetTokens - $workerTokens) -or
        $observedModelCalls -gt [long]$value.budget_max_model_calls -or
        [long]$value.attempt_count -gt [long]$value.budget_max_attempts) {
        throw 'PHASE4_RESOURCE_BUDGET_REJECTED'
    }
    $resourceBudgetEvidence = [ordered]@{
        schema = 'lattice.phase4-resource-budget-evidence/1.0'
        worker_model_call_identity = $workerIdentity
        worker_terminal_resource_digest = [string]$workerResource.descriptor_digest
        worker_total_tokens = $workerTokens
        reviewer_model_call_identity = $reviewIdentity
        reviewer_terminal_resource_digest = [string]$reviewResource.descriptor_digest
        reviewer_total_tokens = $reviewTokens
        observed_total_tokens = $workerTokens + $reviewTokens
        observed_model_calls = $observedModelCalls
        budget_max_total_tokens = $budgetTokens
        budget_max_model_calls = [long]$value.budget_max_model_calls
        budget_max_attempts = [long]$value.budget_max_attempts
        budget_deadline_at = [string]$value.budget_deadline_at
        within_budget = $true
    }
    $resourceBudgetEvidence.evidence_digest = Get-Phase4StringSha256 -Value (
        $resourceBudgetEvidence | ConvertTo-Json -Compress -Depth 8
    )

    $dispatches = @($value.provider_dispatches)
    $replayRows = @($value.replay)
    $expectedKinds = @('WORKER_THREAD', 'WORKER_TURN', 'REVIEW_THREAD', 'REVIEW_TURN')
    $expectedOrdinals = @(101, 102, 103, 104)
    $attemptRows = @($replayRows | Where-Object { [string]$_[0] -ceq 'WORKER_ATTEMPT' })
    if ($dispatches.Count -ne 4 -or $attemptRows.Count -ne 1) {
        throw 'PHASE4_PROVIDER_DISPATCH_EVIDENCE_REJECTED'
    }
    $attemptRow = $attemptRows[0]
    $attemptIndex = -1
    $previousDispatchIndex = -1
    for ($rowIndex = 0; $rowIndex -lt $replayRows.Count; $rowIndex++) {
        if ([string]$replayRows[$rowIndex][0] -ceq 'WORKER_ATTEMPT') {
            $attemptIndex = $rowIndex
            break
        }
    }
    for ($index = 0; $index -lt 4; $index++) {
        $dispatch = $dispatches[$index]
        $expectedReplayKind = 'PROVIDER_DISPATCH_' + $expectedKinds[$index]
        if ([string]$dispatch.kind -cne $expectedKinds[$index] -or
            [int]$dispatch.attempt_number -ne 1 -or
            [string]$dispatch.attempt_id -cne [string]$value.attempt_id -or
            [long]$dispatch.writer_fence -ne [long]$value.writer_fence -or
            [long]$dispatch.foreman_generation -ne [long]$value.foreman_generation -or
            [string]$dispatch.foreman_checkpoint_digest -cne
            [string]$value.foreman_checkpoint_digest -or
            [int]$dispatch.record_ordinal -ne $expectedOrdinals[$index] -or
            [bool]$dispatch.dependency_linked -ne $true -or
            [string]$dispatch.binding_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$dispatch.binding_digest -cne [string]$value.attempt_binding_digest -or
            [string]$dispatch.anchor_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$dispatch.supporting_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$dispatch.subject_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$dispatch.dispatch_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$dispatch.ledger_stream_id -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$dispatch.ledger_event_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
            [long]$dispatch.ledger_event_sequence -lt 1) {
            throw 'PHASE4_PROVIDER_DISPATCH_EVIDENCE_REJECTED'
        }
        $matchingReplay = @($replayRows | Where-Object {
            [string]$_[0] -ceq $expectedReplayKind -and
            [int]$_[2] -eq 1 -and [int]$_[3] -eq $expectedOrdinals[$index] -and
            [string]$_[4] -ceq [string]$dispatch.dispatch_digest -and
            [string]$_[5] -ceq [string]$dispatch.ledger_stream_id -and
            [string]$_[6] -ceq [string]$dispatch.ledger_event_sequence -and
            [string]$_[7] -ceq [string]$dispatch.ledger_event_digest
        })
        if ($matchingReplay.Count -ne 1 -or
            [string]$attemptRow[5] -cne [string]$dispatch.ledger_stream_id -or
            [string]$attemptRow[6] -cne [string]$dispatch.ledger_event_sequence -or
            [string]$attemptRow[7] -cne [string]$dispatch.ledger_event_digest) {
            throw 'PHASE4_PROVIDER_DISPATCH_REPLAY_LINK_REJECTED'
        }
        $dispatchIndex = -1
        for ($rowIndex = 0; $rowIndex -lt $replayRows.Count; $rowIndex++) {
            if ([string]$replayRows[$rowIndex][0] -ceq $expectedReplayKind) {
                $dispatchIndex = $rowIndex
                break
            }
        }
        if ($attemptIndex -lt 0 -or $dispatchIndex -le $attemptIndex -or
            $dispatchIndex -le $previousDispatchIndex) {
            throw 'PHASE4_PROVIDER_DISPATCH_REPLAY_ORDER_REJECTED'
        }
        $previousDispatchIndex = $dispatchIndex
    }
    return [pscustomobject]$resourceBudgetEvidence
}

function Test-Phase4StaticSelf {
    $tokens = $null
    $errors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile(
        $PSCommandPath,
        [ref]$tokens,
        [ref]$errors
    )
    if (@($errors).Count -ne 0) { throw 'PHASE4_POWERSHELL_PARSER_REJECTED' }
    $transientApproval = [pscustomobject][ordered]@{
        status = 'BLOCKED'
        task_state = 'AWAITING_EXECUTION_APPROVAL'
        failure_code = 'LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED'
        attempt = $null
        worker_running = $false
        thread_id = $null
        turn_id = $null
    }
    $claimedApproval = $transientApproval.PSObject.Copy()
    $claimedApproval.attempt = 1
    $foreignBlocker = $transientApproval.PSObject.Copy()
    $foreignBlocker.failure_code = 'LATTICE_MANAGED_WORKTREE_NOT_CLEAN'
    if (-not (Test-Phase4TransientActiveApprovalGate -Status $transientApproval) -or
        (Test-Phase4TransientActiveApprovalGate -Status $claimedApproval) -or
        (Test-Phase4TransientActiveApprovalGate -Status $foreignBlocker)) {
        throw 'PHASE4_STATIC_ACTIVE_APPROVAL_GATE_REJECTED'
    }
    $testProcessTimeoutSeconds = 120
    $testAcceptanceTimeoutSeconds = 960
    $testActiveWindowSeconds = [int][Math]::Min(
        [long]$testAcceptanceTimeoutSeconds,
        ([long]$testProcessTimeoutSeconds * 2) +
            [long][Math]::Min([long]$testProcessTimeoutSeconds, 180) + 120
    )
    $testStatusResponseTimeoutSeconds = [int][Math]::Min(
        900,
        [long]$testActiveWindowSeconds
    )
    $testActiveWindowMilliseconds = [long]$testActiveWindowSeconds * 1000
    $testAvailableStatusCalls = $script:MaximumMcpToolCalls - 1
    $testFinalPollLeadMilliseconds = $script:MinimumMcpStatusResponseBudgetMilliseconds
    $testPollDelayMilliseconds =
        ($testActiveWindowMilliseconds - $testFinalPollLeadMilliseconds) /
        ([double]$testAvailableStatusCalls - 1.0)
    $testFinalPollOffsetMilliseconds = $testPollDelayMilliseconds *
        ([double]$testAvailableStatusCalls - 1.0)
    if ($testActiveWindowSeconds -ne 480 -or
        $testStatusResponseTimeoutSeconds -ne 480 -or
        $testFinalPollLeadMilliseconds -lt 5000 -or
        $testPollDelayMilliseconds -lt 1.0 -or
        [Math]::Abs(
            $testFinalPollOffsetMilliseconds -
            ($testActiveWindowMilliseconds - $testFinalPollLeadMilliseconds)
        ) -gt 0.001 -or
        $testFinalPollOffsetMilliseconds -ge $testActiveWindowMilliseconds) {
        throw 'PHASE4_STATIC_ACTIVE_STATUS_CALL_BUDGET_REJECTED'
    }
    $testReservedTerminalStatusCalls = [long]$script:MaximumMcpStatusPolls
    $testAvailableReconnectStatusCalls = [long]$script:MaximumMcpToolCalls -
        $testReservedTerminalStatusCalls
    $testReconnectWindowMilliseconds = 180000
    $testReconnectFinalPollLeadMilliseconds = $script:MinimumMcpStatusResponseBudgetMilliseconds
    $testReconnectPollDelayMilliseconds =
        ($testReconnectWindowMilliseconds - $testReconnectFinalPollLeadMilliseconds) /
        ([double]$testAvailableReconnectStatusCalls - 1.0)
    $testReconnectFinalPollOffsetMilliseconds = $testReconnectPollDelayMilliseconds *
        ([double]$testAvailableReconnectStatusCalls - 1.0)
    if ($testAvailableReconnectStatusCalls -lt 2 -or
        $testReconnectPollDelayMilliseconds -lt 1.0 -or
        [Math]::Abs(
            $testReconnectFinalPollOffsetMilliseconds -
            ($testReconnectWindowMilliseconds - $testReconnectFinalPollLeadMilliseconds)
        ) -gt 0.001 -or
        $testReconnectFinalPollOffsetMilliseconds -ge $testReconnectWindowMilliseconds -or
        ($testAvailableReconnectStatusCalls + $testReservedTerminalStatusCalls) -gt
            $script:MaximumMcpToolCalls) {
        throw 'PHASE4_STATIC_RECONNECT_STATUS_CALL_BUDGET_REJECTED'
    }
    $testTerminalWindowMilliseconds = 960000
    $testTerminalFinalPollLeadMilliseconds = $script:MinimumMcpStatusResponseBudgetMilliseconds
    $testTerminalPollDelayMilliseconds =
        ($testTerminalWindowMilliseconds - $testTerminalFinalPollLeadMilliseconds) /
        ([double]$script:MaximumMcpStatusPolls - 1.0)
    $testTerminalFinalPollOffsetMilliseconds = $testTerminalPollDelayMilliseconds *
        ([double]$script:MaximumMcpStatusPolls - 1.0)
    if ($testTerminalPollDelayMilliseconds -lt 1.0 -or
        [Math]::Abs(
            $testTerminalFinalPollOffsetMilliseconds -
            ($testTerminalWindowMilliseconds - $testTerminalFinalPollLeadMilliseconds)
        ) -gt 0.001 -or
        $testTerminalFinalPollOffsetMilliseconds -ge $testTerminalWindowMilliseconds) {
        throw 'PHASE4_STATIC_TERMINAL_STATUS_CALL_BUDGET_REJECTED'
    }
    $source = [IO.File]::ReadAllText($PSCommandPath, $script:Utf8)
    foreach ($required in @(
        'Start-Phase4McpSession', 'New-Phase4GeneralTaskStatusArguments',
        'lattice_foreman_checkpoint', 'lattice_task_submit',
        'lattice_task_status', 'AWAITING_MERGE_APPROVAL', 'TURN_STARTED',
        'list_active_task_refs_v1', 'LATTICE_MANAGED_FOREMAN_MODE',
        'LATTICE_DELIVERY_CODEX_HOME', 'LATTICE_MANAGED_WORKTREE_ROOT',
        'lattice.managed-worktree-baseline/1.0', 'refs/lattice/managed/',
        'LATTICE_MANAGED_NPM_EXE', 'verify-phase4-proof.mjs',
        'trusted-node-plan-v1',
        "body->>'event_type'='TURN_STARTED'", 'PROVIDER_DISPATCH_WORKER_THREAD',
        'PHASE4_PROVIDER_DISPATCH_REPLAY_LINK_REJECTED', 'dependency_linked',
        'lattice.managed-protected-result-intent/1.0',
        'lattice.managed-protected-result/1.0',
        "[string]`$value.model_reason -cne 'ROUTINE_ENGINEERING'",
        'ScriptedActiveRestart', 'PROCESS_HANDOFF', 'RECONCILED',
        'LATTICE_MANAGED_SCRIPTED_ACTIVE_RESTART',
        'Wsl2LinuxLive', 'Wsl2TechnicalPreflightOnly',
        'Invoke-Phase4Wsl2Materializer', 'Invoke-Phase4ManagedWorktreeBridge',
        'lattice.managed-worktree-command/1.1',
        '[AllowNull()]$ExpectedBaselineSha256',
        '[AllowNull()]$ExpectedExecutionEnvironmentRef',
        'expected_execution_environment_ref',
        'PHASE4_WSL2_SOURCE_WORKTREE_IDENTITY_REJECTED',
        'LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON',
        'lattice.phase4-managed-foreman.wsl2-technical-preflight.v1',
        'PHASE4_WSL2_PREFLIGHT_PROVIDER_EFFECT_REJECTED',
        'PHASE4_WSL2_SUBSTITUTION_NOT_REJECTED',
        'resealed_descriptor_substitution_rejected',
        'PHASE4_WSL2_FRESH_DRAFT_RECONSTRUCTION_REJECTED',
        'Assert-Phase4DisabledDraftStatus', 'lattice.task.status.v5',
        'Assert-Phase4NoCredentialShapedJsonStrings',
        'Get-Phase4WslProviderFenceEvidence',
        'Get-Phase4WslProviderSubtreeOpenEvidence',
        'Get-Phase4WslProviderSubtreeReconciliationEvidence',
        'Get-Phase4WslProviderSubtreeSegmentRef',
        'Get-Phase4WslReviewerSubtreeEvidence',
        'Invoke-Phase4WslFailureSubtreeCleanup',
        'durable_provider_effect_status', 'reconciliation_required',
        'foreman_process_tree_stopped', 'failure_subtree_cleanup',
        'provider_subtree_segment_ref', 'attempt-packet:sha256:',
        'PHASE4_WSL2_STALE_PROVIDER_FENCE_REJECTED',
        'PHASE4_WSL2_PROVIDER_EFFECTS_CHANGED_AFTER_HARD_STOP',
        'exact_old_processes_absent',
        'cargo-test-locked-offline-v1',
        'Read-Phase4BoundedUtf8Lines', 'PHASE4_SCRIPTED_GENERATION_ROLE_REJECTED',
        'ExpectedCodexLauncherVersion', 'PHASE4_OFFICIAL_CODEX_POLICY_VERSION_REJECTED',
        'real_codex = $false', 'Assert-Phase4ManagedCodexHome',
        'StaticSelfTestOnly', '[ValidateRange(900, 960)]',
        '[int]$AcceptanceTimeoutSeconds = 960'
    )) {
        if (-not $source.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_GUARD_REJECTED'
        }
    }
    foreach ($cargoIdentityLiteral in @(
        "`$script:ExpectedRustupToolchain = '1.97.1-x86_64-pc-windows-msvc'",
        "'cargo 1.97.1 (c980f4866 2026-06-30)'",
        "'release: 1.97.1'",
        "'commit-hash: c980f4866141969fab6254a680546a277789d6f0'",
        "'commit-date: 2026-06-30'",
        "'host: x86_64-pc-windows-msvc'"
    )) {
        if (-not $source.Contains($cargoIdentityLiteral, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_BUILD_TOOLCHAIN_REJECTED'
        }
    }
    $forbiddenWrapper = 'Invoke-' + 'LatticeMcp.ps1'
    $forbiddenPersonalRoot = 'One' + 'Drive'
    $forbiddenLegacyEvent = "body->>'event'=" + "'TURN_STARTED'"
    $forbiddenWslContinuation = 'run-phase4-wsl2-live-' + 'continuation.mjs'
    if ($source.Contains($forbiddenWrapper, [StringComparison]::OrdinalIgnoreCase) -or
        $source.Contains($forbiddenPersonalRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $source.Contains($forbiddenLegacyEvent, [StringComparison]::Ordinal) -or
        $source.Contains($forbiddenWslContinuation, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'PHASE4_STATIC_ISOLATION_REJECTED'
    }
    $legacyCheckId = 'npm.cmd' + '-run-verify-v1'
    if ($source.Contains($legacyCheckId, [StringComparison]::Ordinal)) {
        throw 'PHASE4_STATIC_LEGACY_VERIFICATION_ID_REJECTED'
    }
    $databaseFunctionMarker = 'function Get-Phase4Database' + 'Evidence'
    $replayFunctionMarker = 'function Get-Phase4TaskReplay' + 'Digest'
    $replayStart = $source.IndexOf($replayFunctionMarker, [StringComparison]::Ordinal)
    $databaseStart = $source.IndexOf($databaseFunctionMarker, [StringComparison]::Ordinal)
    if ($replayStart -lt 0 -or $databaseStart -le $replayStart) {
        throw 'PHASE4_STATIC_OWNER_REPLAY_DIGEST_REJECTED'
    }
    $replaySource = $source.Substring($replayStart, $databaseStart - $replayStart)
    foreach ($required in @(
        'LATTICE_FOREMAN_TASK_REPLAY_V1', '[uint64]', '[Array]::Reverse',
        'record_state', 'ledger_event_digest'
    )) {
        if (-not $replaySource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_OWNER_REPLAY_DIGEST_REJECTED'
        }
    }
    $gitControlMarker = 'function Get-Phase4GitControl' + 'Evidence'
    $gitControlStart = $source.IndexOf($gitControlMarker, [StringComparison]::Ordinal)
    if ($gitControlStart -lt 0 -or $replayStart -le $gitControlStart) {
        throw 'PHASE4_STATIC_GIT_CONTROL_PROOF_REJECTED'
    }
    $gitControlSource = $source.Substring($gitControlStart, $replayStart - $gitControlStart)
    foreach ($required in @(
        'config', '--null', '--list', 'remote', 'for-each-ref',
        'lattice.phase4-git-control-proof/1.0',
        "evidence_scope = 'HEAD_STATUS_REMOTES_LOCAL_CONFIG_AND_REFS'",
        'config_equal_at_each_snapshot', 'remote_count_at_each_snapshot',
        'source_status_clean_at_each_snapshot', 'source_head_equal_at_each_snapshot',
        'exact_expected_ref_count_at_terminal_and_restart',
        'git_control_proof_digest'
    )) {
        if (-not $gitControlSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_GIT_CONTROL_PROOF_REJECTED'
        }
    }
    foreach ($unmeasuredCount in @(
        'unauthorized_protected_effect_count', 'merge_effect_count',
        'push_effect_count', 'deploy_effect_count'
    )) {
        if ($gitControlSource -cmatch (
                [regex]::Escape($unmeasuredCount) + '\s*=\s*0'
            )) {
            throw 'PHASE4_STATIC_EFFECT_OVERCLAIM_REJECTED'
        }
    }
    $buildStart = $source.IndexOf(
        "            `$buildEnvironment = New-Phase4ClosedEnvironment -Values",
        [StringComparison]::Ordinal
    )
    $buildEnd = $source.IndexOf(
        "        `$failureStage = 'CREDENTIAL_READ_ISOLATION'", $buildStart,
        [StringComparison]::Ordinal
    )
    if ($buildStart -lt 0 -or $buildEnd -le $buildStart) {
        throw 'PHASE4_STATIC_BUILD_TOOLCHAIN_REJECTED'
    }
    $buildSource = $source.Substring($buildStart, $buildEnd - $buildStart)
    foreach ($required in @(
        "RUSTUP_TOOLCHAIN = `$script:ExpectedRustupToolchain",
        "-Argument @('-Vv')", 'Assert-Phase4CargoIdentity',
        "'PHASE4_CARGO_IDENTITY_REJECTED'"
    )) {
        if (-not $buildSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_BUILD_TOOLCHAIN_REJECTED'
        }
    }
    $assertDatabaseMarker = 'function Assert-Phase4Database' + 'Evidence'
    $assertDatabaseStart = $source.IndexOf($assertDatabaseMarker, [StringComparison]::Ordinal)
    $staticSelfMarker = 'function Test-Phase4Static' + 'Self'
    $staticSelfStart = $source.IndexOf($staticSelfMarker, [StringComparison]::Ordinal)
    if ($databaseStart -lt 0 -or $assertDatabaseStart -le $databaseStart -or
        $staticSelfStart -le $assertDatabaseStart) {
        throw 'PHASE4_STATIC_DATABASE_EVIDENCE_REJECTED'
    }
    $databaseSource = $source.Substring($databaseStart, $assertDatabaseStart - $databaseStart)
    foreach ($required in @(
        'r.record_state', 'promotion_task_spec_digest', 'approval_authority_digest',
        'budget_max_total_tokens', 'budget_max_model_calls', 'resource_calls',
        'owner_replay_digest'
    )) {
        if (-not $databaseSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_DATABASE_EVIDENCE_REJECTED'
        }
    }
    if ([regex]::Matches(
            $databaseSource, '\) \|\| pg_catalog\.jsonb_build_object\('
        ).Count -lt 2) {
        throw 'PHASE4_STATIC_DATABASE_EVIDENCE_REJECTED'
    }
    $wslDraftStart = $source.IndexOf("'WSL2_DRAFT_SUBMIT'", [StringComparison]::Ordinal)
    $wslMaterializationStart = $source.IndexOf(
        "`$failureStage = 'WSL2_BOOTSTRAP_MATERIALIZATION'",
        [StringComparison]::Ordinal
    )
    $wslCheckpointStart = $source.IndexOf(
        "`$failureStage = 'WSL2_FOREMAN_CHECKPOINT'",
        [StringComparison]::Ordinal
    )
    $wslActiveDescriptorStart = $source.IndexOf(
        "`$activeEnvironment['LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON'] =",
        [StringComparison]::Ordinal
    )
    if ($wslDraftStart -lt 0 -or $wslMaterializationStart -le $wslDraftStart -or
        $wslCheckpointStart -le $wslMaterializationStart -or
        $wslActiveDescriptorStart -le $wslCheckpointStart) {
        throw 'PHASE4_STATIC_WSL2_PROCESS_ORDER_REJECTED'
    }
    $wslActiveMainStart = $source.LastIndexOf(
        '$wslActiveRestartBefore = $null', [StringComparison]::Ordinal
    )
    $wslProviderCaptureStart = $source.IndexOf(
        "`$failureStage = 'WSL2_PROVIDER_FENCE_CAPTURE'", $wslActiveMainStart,
        [StringComparison]::Ordinal
    )
    $wslProviderHardStop = $source.IndexOf(
        '$hardStopped = Stop-Phase4McpSessionHard -Session $mcpSession',
        $wslProviderCaptureStart,
        [StringComparison]::Ordinal
    )
    $wslProviderTeardown = $source.IndexOf(
        "`$failureStage = 'WSL2_PROVIDER_FENCE_TEARDOWN'",
        $wslProviderHardStop,
        [StringComparison]::Ordinal
    )
    $wslProviderEffects = $source.IndexOf(
        '$wslProviderEffectsAfterFence = Get-Phase4WslDurableEvidence',
        $wslProviderTeardown,
        [StringComparison]::Ordinal
    )
    $wslProviderReconnect = $source.IndexOf(
        "`$mcpSession = Start-Phase4McpSession -Name 'managed-wsl2-reconnect'",
        $wslProviderEffects,
        [StringComparison]::Ordinal
    )
    $wslProviderPreflightAssignment = $source.IndexOf(
        '$wslProviderPreflightEvidence = Get-Phase4WslProviderPreflightEvidence',
        $wslProviderCaptureStart,
        [StringComparison]::Ordinal
    )
    $wslProviderOpenAssignment = $source.IndexOf(
        '$wslProviderOpenMarkerEvidence = Get-Phase4WslProviderSubtreeOpenEvidence',
        $wslProviderPreflightAssignment,
        [StringComparison]::Ordinal
    )
    $wslProviderReconciliationAssignment = $source.IndexOf(
        '$wslProviderSubtreeReconciliation =',
        $wslProviderReconnect,
        [StringComparison]::Ordinal
    )
    $wslProviderFreshReplayAssignment = $source.IndexOf(
        '$wslProviderSubtreeFreshReplay =',
        $wslProviderReconciliationAssignment,
        [StringComparison]::Ordinal
    )
    $wslReviewerSubtreeAssignment = $source.IndexOf(
        '$wslReviewerSubtreeEvidence = Get-Phase4WslReviewerSubtreeEvidence',
        $wslProviderFreshReplayAssignment,
        [StringComparison]::Ordinal
    )
    if ($wslActiveMainStart -lt 0 -or $wslProviderCaptureStart -lt 0 -or
        $wslProviderPreflightAssignment -le $wslProviderCaptureStart -or
        $wslProviderOpenAssignment -le $wslProviderPreflightAssignment -or
        $wslProviderHardStop -le $wslProviderOpenAssignment -or
        $wslProviderHardStop -le $wslProviderCaptureStart -or
        $wslProviderTeardown -le $wslProviderHardStop -or
        $wslProviderEffects -le $wslProviderTeardown -or
        $wslProviderReconnect -le $wslProviderEffects -or
        $wslProviderReconciliationAssignment -le $wslProviderReconnect -or
        $wslProviderFreshReplayAssignment -le $wslProviderReconciliationAssignment -or
        $wslReviewerSubtreeAssignment -le $wslProviderFreshReplayAssignment) {
        throw 'PHASE4_STATIC_WSL2_PROVIDER_FENCE_ORDER_REJECTED'
    }
    $wslProviderMainSource = $source.Substring(
        $wslProviderCaptureStart,
        $wslProviderFreshReplayAssignment - $wslProviderCaptureStart
    )
    foreach ($required in @(
        '-ExpectedPacketDigest ([string]$before.packet_digest)',
        '-ExpectedProducerDigest ([string]$before.writer_process_start_identity)',
        '-ExpectedReconcilerProducerDigest (',
        '[string]$after.writer_process_start_identity',
        '$wslProviderEffectsBeforeReconciliation = $wslProviderEffectsAfterFence'
    )) {
        if (-not $wslProviderMainSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_WSL2_PROVIDER_SUBTREE_ORDER_REJECTED'
        }
    }
    $forbiddenHarnessReconciler = 'Invoke-Phase4WslProviderSubtree' + 'Reconciliation'
    if ($source.Contains($forbiddenHarnessReconciler, [StringComparison]::Ordinal)) {
        throw 'PHASE4_STATIC_WSL2_PROVIDER_SUBTREE_ORDER_REJECTED'
    }
    $wslReviewerMainSource = $source.Substring(
        $wslReviewerSubtreeAssignment,
        [Math]::Min(4096, $source.Length - $wslReviewerSubtreeAssignment)
    )
    foreach ($required in @(
        '-ExpectedModelCallIdentity (',
        '[string]$resourceBudgetBefore.reviewer_model_call_identity',
        '-ExpectedProducerDigest (',
        '[string]$wslActiveRestartAfter.value.writer_process_start_identity',
        '-ExpectedProviderEffectCount ('
    )) {
        if (-not $wslReviewerMainSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_WSL2_REVIEWER_SUBTREE_ORDER_REJECTED'
        }
    }
    $assertDatabaseSource = $source.Substring(
        $assertDatabaseStart, $staticSelfStart - $assertDatabaseStart
    )
    foreach ($required in @(
        "[string]`$value.reasoning -cne 'medium'", 'promotion_task_spec_digest',
        'approval_authority_digest', 'budget_max_total_tokens',
        'owner_replay_digest', 'CUMULATIVE_TERMINAL'
    )) {
        if (-not $assertDatabaseSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_DATABASE_BINDING_REJECTED'
        }
    }
    $disposableStart = $source.IndexOf(
        "    `$failureStage = 'DISPOSABLE_GIT'", [StringComparison]::Ordinal
    )
    $disposableEnd = $source.IndexOf(
        "    `$failureStage = 'POSTGRES_INIT'", $disposableStart,
        [StringComparison]::Ordinal
    )
    if ($disposableStart -lt 0 -or $disposableEnd -le $disposableStart) {
        throw 'PHASE4_STATIC_MANAGED_SCOPE_POLICY_REJECTED'
    }
    $disposableSource = $source.Substring(
        $disposableStart, $disposableEnd - $disposableStart
    )
    foreach ($required in @(
        'lattice.managed-scope.json', 'lattice.managed-scope/1.0',
        '"allowed_paths":["phase4-proof.txt"]',
        'PHASE4_MANAGED_SCOPE_POLICY_BYTES_REJECTED'
    )) {
        if (-not $disposableSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_MANAGED_SCOPE_POLICY_REJECTED'
        }
    }
    $successReceiptMarker = 'if ($null -eq $failureCode -and $null -ne $terminal' + 'Status'
    $successStart = $source.IndexOf($successReceiptMarker, [StringComparison]::Ordinal)
    $failureStatusMarker = "    status = '" + "FAIL'"
    $failureReceiptStart = $source.IndexOf(
        $failureStatusMarker, $successStart, [StringComparison]::Ordinal
    )
    if ($successStart -lt 0 -or $failureReceiptStart -le $successStart) {
        throw 'PHASE4_STATIC_SUCCESS_RECEIPT_REJECTED'
    }
    $successSource = $source.Substring($successStart, $failureReceiptStart - $successStart)
    foreach ($required in @(
        'effect_evidence', 'control_evidence', 'observed_effects',
        "status = 'NOT_MEASURED'", "push = 'UNVERIFIED'", "deploy = 'UNVERIFIED'",
        "payment = 'UNVERIFIED'", "external_message = 'UNVERIFIED'",
        'owner_replay_digest', 'budget_max_total_tokens', 'worker_model_call_identity',
        'reviewer_model_call_identity'
    )) {
        if (-not $successSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_SUCCESS_RECEIPT_REJECTED'
        }
    }
    $ambientAuthCopy = 'Copy-' + 'Item -LiteralPath $SourceAuthPath'
    $forbiddenIoCopy = '[IO.File]::' + 'Copy'
    if ($source.Contains($ambientAuthCopy, [StringComparison]::Ordinal) -or
        $source.Contains($forbiddenIoCopy, [StringComparison]::Ordinal)) {
        throw 'PHASE4_STATIC_CREDENTIAL_COPY_REJECTED'
    }
    $credentialGateStart = $source.IndexOf(
        "    `$failureStage = 'CREDENTIAL_READ_ISOLATION'", [StringComparison]::Ordinal
    )
    if ($credentialGateStart -lt 0 -or $credentialGateStart -ge $disposableStart) {
        throw 'PHASE4_STATIC_CREDENTIAL_GATE_REJECTED'
    }
    $credentialGateSource = $source.Substring(
        $credentialGateStart, $disposableStart - $credentialGateStart
    )
    foreach ($required in @(
        'Assert-Phase4ManagedCodexHome -Path $codexProfileRoot',
        '$credentialReadIsolation = if ($Wsl2TechnicalPreflightOnly)',
        "'KEYRING_CONFIG_VERIFIED_READINESS_PENDING'",
        "'WSL2_AUTH_NOT_READ_ZERO_MODEL_PREFLIGHT'"
    )) {
        if (-not $credentialGateSource.Contains($required, [StringComparison]::Ordinal)) {
            throw 'PHASE4_STATIC_CREDENTIAL_GATE_REJECTED'
        }
    }
    $failureReceiptSource = $source.Substring($failureReceiptStart)
    if (-not $failureReceiptSource.Contains(
            'credential_read_isolation = $credentialReadIsolation',
            [StringComparison]::Ordinal
        )) {
        throw 'PHASE4_STATIC_CREDENTIAL_GATE_REJECTED'
    }
    $compositionPath = Join-Path $script:RepositoryRoot 'apps\lattice-runtime\src\composition.rs'
    Assert-Phase4RegularFile -Path $compositionPath `
        -Failure 'PHASE4_OFFICIAL_CODEX_POLICY_VERSION_REJECTED'
    $compositionSource = [IO.File]::ReadAllText($compositionPath, $script:Utf8)
    $policyStart = $compositionSource.IndexOf(
        'const OFFICIAL_BUNDLE_POLICY:', [StringComparison]::Ordinal
    )
    $policyEnd = $compositionSource.IndexOf("`n};", $policyStart, [StringComparison]::Ordinal)
    if ($policyStart -lt 0 -or $policyEnd -le $policyStart) {
        throw 'PHASE4_OFFICIAL_CODEX_POLICY_VERSION_REJECTED'
    }
    $officialPolicy = $compositionSource.Substring($policyStart, $policyEnd - $policyStart)
    if (-not $officialPolicy.Contains(
        ('version: "' + $script:ExpectedCodexLauncherVersion + '"'),
        [StringComparison]::Ordinal
    ) -or -not $officialPolicy.Contains(
        ('sha256: "' + $script:ExpectedCodexSha256 + '"'),
        [StringComparison]::Ordinal
    )) {
        throw 'PHASE4_OFFICIAL_CODEX_POLICY_VERSION_REJECTED'
    }
    $scriptedReaderStart = $source.IndexOf(
        'function Get-Phase4ScriptedEventEvidence', [StringComparison]::Ordinal
    )
    $scriptedReaderEnd = $source.IndexOf(
        'function Get-Phase4DatabaseEvidence', $scriptedReaderStart,
        [StringComparison]::Ordinal
    )
    if ($scriptedReaderStart -lt 0 -or $scriptedReaderEnd -le $scriptedReaderStart) {
        throw 'PHASE4_STATIC_BOUNDED_LOG_REJECTED'
    }
    $scriptedReader = $source.Substring(
        $scriptedReaderStart, $scriptedReaderEnd - $scriptedReaderStart
    )
    if ($scriptedReader.Contains('ReadAllLines', [StringComparison]::Ordinal)) {
        throw 'PHASE4_STATIC_BOUNDED_LOG_REJECTED'
    }
    $scriptedServerPath = Join-Path `
        $script:RepositoryRoot 'apps\lattice-runtime\src\fixtures\task032-scripted-codex.ps1'
    Assert-Phase4RegularFile -Path $scriptedServerPath `
        -Failure 'PHASE4_SCRIPTED_SERVER_REJECTED'
    $scriptedServerSource = [IO.File]::ReadAllText($scriptedServerPath, $script:Utf8)
    if (-not $scriptedServerSource.Contains(
            '[object[]]$turns = if ([bool]$State.turn_started)',
            [StringComparison]::Ordinal
        )) {
        throw 'PHASE4_SCRIPTED_ARRAY_SHAPE_REJECTED'
    }
}

if ($StaticReceiptPersistenceSelfTestOnly) {
    Test-Phase4StaticSelf
    Test-Phase4FailureReceiptPersistence | ConvertTo-Json -Compress
    return
}

if ($StaticMcpPollingSelfTestOnly) {
    Test-Phase4StaticSelf
    Test-Phase4McpStatusTimeoutBehavior | ConvertTo-Json -Compress
    return
}

if ($StaticSelfTestOnly) {
    Test-Phase4StaticSelf
    [ordered]@{
        schema = 'lattice.phase4-managed-foreman.acceptance.v1'
        status = 'PASS'
        mode = 'STATIC_SELF_CHECK'
        acceptance = $false
        powershell_parser = 'PASS'
        runtime_executed = $false
        real_codex_executed = $false
        credential_read_isolation = 'CREDENTIAL_READ_ISOLATION_NOT_VERIFIED'
    } | ConvertTo-Json -Compress
    return
}

if ($Wsl2TechnicalPreflightOnly -and -not $Wsl2LinuxLive) {
    throw 'PHASE4_WSL2_TECHNICAL_MODE_REQUIRES_WSL2_LIVE'
}
if ($Wsl2LinuxLive -and $ScriptedActiveRestart) {
    throw 'PHASE4_WSL2_SCRIPTED_MODE_CONFLICT'
}

$startedAt = [DateTimeOffset]::UtcNow
$runId = Get-Phase4RandomHex -ByteCount 16
$script:WslCommandUnitPrefix = 'lattice-phase4-command-' + $runId.Substring(0, 16)
$script:WslCommandUnitCounter = [long]0
$script:WslOpenCommandUnits = [Collections.Generic.HashSet[string]]::new(
    [StringComparer]::Ordinal
)
$tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd(
    [IO.Path]::DirectorySeparatorChar
)
$runRootName = 'lattice-phase4-managed-foreman-' + $runId
$runRoot = [IO.Path]::GetFullPath((Join-Path $tempParent $runRootName))
$expectedRunRoot = [IO.Path]::GetFullPath(
    $tempParent + [IO.Path]::DirectorySeparatorChar + $runRootName
)
$dataRoot = Join-Path $runRoot 'postgres-data'
$controlHome = Join-Path $runRoot 'control-home'
$wslSourceRepository = $Wsl2TaskRoot + '/managed-worktrees/source-' + $runId
$wslBootstrapRepository = $Wsl2TaskRoot + '/managed-worktrees/bootstrap-' + $runId
$wslManagedWorktreeParent = $Wsl2TaskRoot + '/managed-worktrees'
$wslManagedWorktreeRoot = $wslManagedWorktreeParent + '/owned-' + $runId
$wslHarnessRoot = $Wsl2TaskRoot + '/verifier-state/harness-' + $runId
$script:Wsl2HarnessHome = $wslHarnessRoot + '/home'
$script:Wsl2HarnessTemp = $wslHarnessRoot + '/tmp'
$script:Wsl2HarnessGitHooks = $wslHarnessRoot + '/git-hooks'
$projectRoot = if ($Wsl2LinuxLive) {
    ConvertTo-Phase4WslUncPath -LinuxPath $wslSourceRepository
}
else {
    Join-Path $runRoot 'repository'
}
$bootstrapProjectRoot = if ($Wsl2LinuxLive) {
    ConvertTo-Phase4WslUncPath -LinuxPath $wslBootstrapRepository
}
else {
    $projectRoot
}
$repositoryBuildRoot = $projectRoot
$managedWorktreeRoot = if ($Wsl2LinuxLive) {
    ConvertTo-Phase4WslUncPath -LinuxPath $wslManagedWorktreeRoot
}
else {
    Join-Path $runRoot 'managed-worktrees'
}
$markerPath = Join-Path $runRoot '.phase4-owner.json'
$passwordPath = Join-Path $runRoot '.initdb-password'
$databaseName = 'lattice_task019_' + $runId.Substring(0, 8) + '_base'
$projectName = 'phase4-disposable-' + $runId.Substring(0, 8)
$clientRequestId = 'phase4-managed-' + $runId.Substring(0, 16)
$objective = 'Create phase4-proof.txt in the repository root with exactly LATTICE_PHASE4_MANAGED_FOREMAN_OK followed by one newline. Make no other change.'
$postgresPort = $null
$controlPort = $null
$password = $null
$latticed = $null
$codex = $null
$node = $null
$npm = $null
$git = $null
$cargo = $null
$codexProfileRoot = $null
$foremanProcessTreeStopped = $true
$postgresRunning = $false
$script:postgresStartMayOwnProcess = $false
$script:postgresLauncherTerminalProven = $false
$script:postgresProcessIdentity = $null
$script:postgresOwnedProcessJob = $null
$script:LastPhase4ProcessTreeTerminationProven = $false
$controlSession = $null
$mcpSession = $null
$runRootCreated = $false
$cleanupSucceeded = $false
$listenerCleanup = $false
$failureCode = $null
$failureLine = 0
$failureException = 'NONE'
$failureStage = 'SETUP'
$checkpoint = $null
$submitted = $null
$realCodexAttempted = $false
$realCodexAttemptEvidence = 'NOT_ENTERED'
$terminalStatus = $null
$lastManagedStatus = $null
$mcpStatusTimeoutDiagnostic = $null
$restartStatus = $null
$databaseBefore = $null
$databaseAfter = $null
$gitEvidence = $null
$baselineEvidence = $null
$resourceBudgetBefore = $null
$resourceBudgetAfter = $null
$gitControlBefore = $null
$gitControlAfter = $null
$gitControlRestart = $null
$gitControlProof = $null
$managedWorkerRoot = $null
$protectedRef = $null
$firstPostgres = $null
$secondPostgres = $null
$firstControl = $null
$secondControl = $null
$firstForeman = $null
$secondForeman = $null
$mcpRecords = [Collections.Generic.List[object]]::new()
$physicalRestart = $false
$noDuplicateAgent = $false
$projectId = $null
$projectProjectionDigest = $null
$projectRestartDigest = $null
$systemIdentifierBefore = $null
$systemIdentifierAfter = $null
$scriptedFixture = $null
$scriptedEventEvidence = $null
$activeBeforeRestart = $null
$activeAfterRestart = $null
$firstActiveStatus = $null
$secondActiveStatus = $null
$firstScriptedServer = $null
$secondScriptedServer = $null
$probeScriptedServer = $null
$firstServerAbsence = $null
$secondServerAbsence = $null
$probeServerAbsence = $null
$scriptedFixtureCleanup = (-not $ScriptedActiveRestart)
$credentialReadIsolation = $(
    if ($ScriptedActiveRestart) { 'NOT_APPLICABLE_SCRIPTED_APP_SERVER' }
    elseif ($Wsl2TechnicalPreflightOnly) { 'WSL2_AUTH_NOT_READ_ZERO_MODEL_PREFLIGHT' }
    else { 'KEYRING_CONFIG_NOT_VERIFIED' }
)
$taskRef = $null
$baseCommit = $null
$bootstrapMaterialization = $null
$finalMaterialization = $null
$managedPrepare = $null
$managedVerify = $null
$wslZeroProviderEvidence = $null
$wslZeroProviderAfterMaterialization = $null
$wslZeroProviderAfterCheckpoint = $null
$wslEnvironmentBefore = $null
$wslEnvironmentAfter = $null
$wslTechnicalPreflightComplete = $false
$wslActiveRestartBefore = $null
$wslActiveRestartAfter = $null
$wslEnvironmentAtAcceptedStart = $null
$wslProviderPreflightEvidence = $null
$wslProviderOpenMarkerEvidence = $null
$wslProviderFenceBeforeHardStop = $null
$wslProviderEffectsBeforeReconciliation = $null
$wslProviderSubtreeReconciliation = $null
$wslProviderSubtreeFreshReplay = $null
$wslReviewerSubtreeEvidence = $null
$wslProviderFenceAfterHardStop = $null
$wslProviderEffectsAfterFence = $null
$wslFinalDurableEvidenceBeforeCleanup = $null
$wslFinalDurableEvidence = $null
$wslDurableProviderEffectStatus = 'NOT_OBSERVED'
$wslFailureSubtreeCleanup = $null
$wslFailureSubtreeCleanupCode = $null
$wslReconciliationRequired = $false
$wslReconnectForeman = $null
$taskSubmissionIdentity = $null
$draftStatus = $null
$freshDraftStatus = $null
$expectedManagedLinuxPath = $null
$expectedManagedUncPath = $null
$wslSubstitutionEvidence = $null
$wslSupervisorSourceSha256 = $null

try {
    Test-Phase4StaticSelf
    if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PHASE4_POWERSHELL_7_REQUIRED' }
    foreach ($binary in @(
        $script:InitDb, $script:PgCtl, $script:Postgres, $script:Psql,
        $script:Netstat, $script:ControlServer, $script:ManagedBridge
    )) {
        Assert-Phase4RegularFile -Path $binary -Failure 'PHASE4_REQUIRED_FILE_MISSING'
    }
    if ($Wsl2LinuxLive) {
        foreach ($binary in @(
            $script:Wsl, $script:ManagedWorktreeBridge, $script:Wsl2Materializer,
            $script:Wsl2PreflightBridge, $script:Wsl2ProviderSubtreeReconciler,
            $script:Wsl2SupervisorSource
        )) {
            Assert-Phase4RegularFile -Path $binary `
                -Failure 'PHASE4_WSL2_REQUIRED_FILE_MISSING'
        }
        $wslSupervisorSourceSha256 = Get-Phase4FileSha256 `
            -Path $script:Wsl2SupervisorSource
    }
    $node = [IO.Path]::GetFullPath((Get-Command node.exe -ErrorAction Stop).Source)
    $npm = [IO.Path]::GetFullPath((Get-Command npm.cmd -ErrorAction Stop).Source)
    $git = [IO.Path]::GetFullPath((Get-Command git.exe -ErrorAction Stop).Source)
    $cargo = [IO.Path]::GetFullPath((Get-Command cargo.exe -ErrorAction Stop).Source)
    foreach ($binary in @($node, $npm, $git, $cargo)) {
        Assert-Phase4RegularFile -Path $binary -Failure 'PHASE4_REQUIRED_BINARY_MISSING'
    }
    $taskUserProfileRoot = [IO.Path]::GetFullPath(
        [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    )
    if ([string]::IsNullOrWhiteSpace($BinaryPath)) {
        # The official bundle is hundreds of megabytes and every security
        # identity is intentionally SHA-bound. Run the real acceptance with
        # the deployment-shaped optimized binary so hashing cannot consume the
        # worker's bounded deadline before any Codex turn exists.
        $latticed = [IO.Path]::GetFullPath((Join-Path $script:RepositoryRoot 'target\release\latticed.exe'))
        if (-not $SkipBuild) {
            $failureStage = 'BUILD'
            $buildEnvironment = New-Phase4ClosedEnvironment -Values ([ordered]@{
                CARGO_NET_OFFLINE = 'true'
                RUSTUP_TOOLCHAIN = $script:ExpectedRustupToolchain
                CARGO_HOME = $(
                    $configured = [Environment]::GetEnvironmentVariable('CARGO_HOME', 'Process')
                    if ([string]::IsNullOrWhiteSpace($configured)) {
                        Join-Path $taskUserProfileRoot '.cargo'
                    }
                    else {
                        $configured
                    }
                )
                RUSTUP_HOME = $(
                    $configured = [Environment]::GetEnvironmentVariable('RUSTUP_HOME', 'Process')
                    if ([string]::IsNullOrWhiteSpace($configured)) {
                        Join-Path $taskUserProfileRoot '.rustup'
                    }
                    else {
                        $configured
                    }
                )
                USERPROFILE = $taskUserProfileRoot
            })
            $cargoIdentity = Invoke-Phase4Process -Executable $cargo -Argument @('-Vv') `
                -Environment $buildEnvironment -WorkingDirectory $script:RepositoryRoot `
                -StandardInput $null -TimeoutSeconds 30 `
                -Failure 'PHASE4_CARGO_IDENTITY_REJECTED'
            Assert-Phase4CargoIdentity -VerboseVersion $cargoIdentity.stdout
            $null = Invoke-Phase4Process -Executable $cargo -Argument @(
                'build', '--release', '-p', 'lattice-runtime', '--bin', 'latticed',
                '--locked', '--offline'
            ) -Environment $buildEnvironment -WorkingDirectory $script:RepositoryRoot `
                -StandardInput $null -TimeoutSeconds 900 -Failure 'PHASE4_BUILD_FAILED'
        }
    }
    else {
        if (-not [IO.Path]::IsPathRooted($BinaryPath)) { throw 'PHASE4_LATTICED_PATH_REJECTED' }
        $latticed = [IO.Path]::GetFullPath($BinaryPath)
    }
    Assert-Phase4RegularFile -Path $latticed -Failure 'PHASE4_LATTICED_BINARY_MISSING'
    if ([IO.Path]::GetFileName($latticed) -cne 'latticed.exe') {
        throw 'PHASE4_LATTICED_PATH_REJECTED'
    }
    if (-not $ScriptedActiveRestart) {
        if (-not [IO.Path]::IsPathRooted($CodexExecutablePath)) {
            throw 'PHASE4_CODEX_PATH_REJECTED'
        }
        $codex = [IO.Path]::GetFullPath($CodexExecutablePath)
        Assert-Phase4RegularFile -Path $codex -Failure 'PHASE4_CODEX_BINARY_MISSING'
        if ([IO.Path]::GetFileName($codex) -cne 'codex.exe' -or
            (Get-Phase4FileSha256 -Path $codex) -cne $script:ExpectedCodexSha256) {
            throw 'PHASE4_OFFICIAL_CODEX_IDENTITY_REJECTED'
        }
        $failureStage = 'CREDENTIAL_READ_ISOLATION'
        $codexProfileRoot = [IO.Path]::GetFullPath(
            (Join-Path $env:LOCALAPPDATA 'LATTICE\runtime-codex-home-keyring-v1')
        )
        $codexProfileRoot = Assert-Phase4ManagedCodexHome -Path $codexProfileRoot
        $credentialReadIsolation = if ($Wsl2TechnicalPreflightOnly) {
            'WSL2_AUTH_NOT_READ_ZERO_MODEL_PREFLIGHT'
        }
        elseif ($Wsl2LinuxLive) {
            'WSL2_LINUX_KEYRING_CONFIG_VERIFIED_READINESS_PENDING'
        }
        else {
            'KEYRING_CONFIG_VERIFIED_READINESS_PENDING'
        }
    }

    if ($runRoot -cne $expectedRunRoot -or -not $runRoot.StartsWith(
        $tempParent + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    ) -or (Test-Path -LiteralPath $runRoot)) {
        throw 'PHASE4_RUN_ROOT_REJECTED'
    }
    [IO.Directory]::CreateDirectory($runRoot) | Out-Null
    $runRootCreated = $true
    foreach ($directory in @($dataRoot, $controlHome)) {
        [IO.Directory]::CreateDirectory($directory) | Out-Null
        Assert-Phase4Directory -Path $directory -Failure 'PHASE4_RUN_DIRECTORY_REJECTED'
    }
    if ($Wsl2LinuxLive) {
        if ($Wsl2TaskRoot -cnotmatch '\A(?<home>/home/[^/]+)/') {
            throw 'PHASE4_WSL2_TASK_ROOT_REJECTED'
        }
        $wslOwnerHome = [string]$Matches.home
        $null = Invoke-Phase4WslProcess -Executable '/usr/bin/test' -Argument @(
            '-d', $wslManagedWorktreeParent
        ) -Environment ([ordered]@{}) -LinuxHome $wslOwnerHome -LinuxTemp $wslOwnerHome `
            -StandardInput $null -TimeoutSeconds 30 `
            -Failure 'PHASE4_WSL2_MANAGED_ROOT_REJECTED'
        foreach ($freshPath in @(
            $wslSourceRepository, $wslBootstrapRepository, $wslManagedWorktreeRoot,
            $wslHarnessRoot
        )) {
            $absence = Invoke-Phase4WslProcess -Executable '/usr/bin/test' -Argument @(
                '!', '-e', $freshPath
            ) -Environment ([ordered]@{}) -LinuxHome $wslOwnerHome `
                -LinuxTemp $wslOwnerHome -StandardInput $null -TimeoutSeconds 30 `
                -Failure 'PHASE4_WSL2_RUN_PATH_COLLISION' -AllowNonZeroExit
            if ([int]$absence.exit_code -ne 0) {
                throw 'PHASE4_WSL2_RUN_PATH_COLLISION'
            }
        }
        $null = Invoke-Phase4WslProcess -Executable '/usr/bin/install' -Argument @(
            '-d', '-m', '0700', $wslSourceRepository, $wslManagedWorktreeRoot,
            $wslHarnessRoot,
            $script:Wsl2HarnessHome, $script:Wsl2HarnessTemp,
            $script:Wsl2HarnessGitHooks
        ) -Environment ([ordered]@{}) -LinuxHome $wslOwnerHome -LinuxTemp $wslOwnerHome `
            -StandardInput $null -TimeoutSeconds 30 `
            -Failure 'PHASE4_WSL2_RUN_DIRECTORY_REJECTED'
        foreach ($directory in @($repositoryBuildRoot, $managedWorktreeRoot)) {
            Assert-Phase4Directory -Path $directory `
                -Failure 'PHASE4_WSL2_RUN_DIRECTORY_REJECTED'
        }
    }
    else {
        foreach ($directory in @($projectRoot, $managedWorktreeRoot)) {
            [IO.Directory]::CreateDirectory($directory) | Out-Null
            Assert-Phase4Directory -Path $directory -Failure 'PHASE4_RUN_DIRECTORY_REJECTED'
        }
    }
    if ($ScriptedActiveRestart) {
        $scriptedFixture = New-Phase4ManagedScriptedFixture -FixtureId $runId
        $codex = [string]$scriptedFixture.launcher
        $codexProfileRoot = [string]$scriptedFixture.codex_home
    }
    $postgresPort = New-Phase4AvailablePort -AdditionalForbidden @()
    $controlPort = New-Phase4AvailablePort -AdditionalForbidden @($postgresPort)
    $marker = [ordered]@{
        owner = $script:OwnerKind
        run_id = $runId
        root = $runRoot
        data_root = [IO.Path]::GetFullPath($dataRoot)
        control_home = [IO.Path]::GetFullPath($controlHome)
        project_root = [IO.Path]::GetFullPath($projectRoot)
        postgres_port = $postgresPort
        control_port = $controlPort
        postgres_executable = [IO.Path]::GetFullPath($script:Postgres)
        postgres_sha256 = Get-Phase4FileSha256 -Path $script:Postgres
        latticed_sha256 = Get-Phase4FileSha256 -Path $latticed
        codex_sha256 = Get-Phase4FileSha256 -Path $codex
    }
    Write-Phase4JsonFile -Path $markerPath -Value $marker
    $null = Get-Phase4OwnerMarker -RunRoot $runRoot -RunId $runId -MarkerPath $markerPath
    $failureStage = 'DISPOSABLE_GIT'
    $gitEnvironment = New-Phase4ClosedEnvironment -Values ([ordered]@{
        GIT_CONFIG_NOSYSTEM = '1'
        GIT_TERMINAL_PROMPT = '0'
    })
    $null = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        'init', '-b', 'main', $repositoryBuildRoot
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_GIT_INIT_FAILED'
    foreach ($config in @(
        @('user.name', 'LATTICE Phase 4'),
        @('user.email', 'lattice-phase4@invalid.example'),
        @('commit.gpgSign', 'false')
    )) {
        $null = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $repositoryBuildRoot, 'config', '--local', $config[0], $config[1]
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_GIT_CONFIG_FAILED'
    }
    [IO.File]::WriteAllText(
        (Join-Path $repositoryBuildRoot '.gitignore'),
        "*.credentials`n*.worker-sentinel`n",
        $script:Utf8
    )
    [IO.File]::WriteAllText(
        (Join-Path $repositoryBuildRoot 'README.md'),
        "# Phase 4 disposable acceptance repository`n",
        $script:Utf8
    )
    [IO.File]::WriteAllText(
        (Join-Path $repositoryBuildRoot 'package.json'),
        '{"name":"lattice-phase4-disposable","private":true,"scripts":{"verify":"node verify-phase4-proof.mjs"}}' + "`n",
        $script:Utf8
    )
    $managedScopePath = Join-Path $repositoryBuildRoot 'lattice.managed-scope.json'
    $managedScopeBytes =
        '{"schema":"lattice.managed-scope/1.0","allowed_paths":["phase4-proof.txt"]}' + "`n"
    [IO.File]::WriteAllText($managedScopePath, $managedScopeBytes, $script:Utf8)
    if ((Get-Phase4FileSha256 -Path $managedScopePath) -cne
        (Get-Phase4StringSha256 -Value $managedScopeBytes) -or
        [long](Get-Item -LiteralPath $managedScopePath).Length -ne
        [long]$script:Utf8.GetByteCount($managedScopeBytes)) {
        throw 'PHASE4_MANAGED_SCOPE_POLICY_BYTES_REJECTED'
    }
    $focusedVerifyScript = @'
import { readFileSync } from "node:fs";

const expected = Buffer.from("LATTICE_PHASE4_MANAGED_FOREMAN_OK\n", "utf8");
let actual;
try {
  actual = readFileSync("phase4-proof.txt");
} catch {
  process.exit(1);
}
if (!actual.equals(expected)) process.exit(1);
'@
    [IO.File]::WriteAllText(
        (Join-Path $repositoryBuildRoot 'verify-phase4-proof.mjs'),
        $focusedVerifyScript.Replace("`r`n", "`n") + "`n",
        $script:Utf8
    )
    $baselinePaths = [Collections.Generic.List[string]]::new()
    foreach ($path in @(
        '.gitignore', 'README.md', 'package.json', 'verify-phase4-proof.mjs',
        'lattice.managed-scope.json'
    )) { $baselinePaths.Add($path) }
    if ($Wsl2LinuxLive) {
        [IO.Directory]::CreateDirectory((Join-Path $repositoryBuildRoot 'src')) | Out-Null
        [IO.File]::WriteAllText(
            (Join-Path $repositoryBuildRoot 'Cargo.toml'),
            @"
[package]
name = "lattice-phase4-disposable"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"@.Replace("`r`n", "`n"),
            $script:Utf8
        )
        [IO.File]::WriteAllText(
            (Join-Path $repositoryBuildRoot 'Cargo.lock'),
            @"
# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 4

[[package]]
name = "lattice-phase4-disposable"
version = "0.1.0"
"@.Replace("`r`n", "`n"),
            $script:Utf8
        )
        [IO.File]::WriteAllText(
            (Join-Path $repositoryBuildRoot 'src\lib.rs'),
            @'
#[cfg(test)]
mod tests {
    #[test]
    fn exact_phase4_proof() {
        let proof = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/phase4-proof.txt"))
            .expect("phase4 proof must exist");
        assert_eq!(proof, b"LATTICE_PHASE4_MANAGED_FOREMAN_OK\n");
    }
}
'@.Replace("`r`n", "`n") + "`n",
            $script:Utf8
        )
        foreach ($path in @('Cargo.toml', 'Cargo.lock', 'src/lib.rs')) {
            $baselinePaths.Add($path)
        }
    }
    $null = Invoke-Phase4RepositoryGit -Git $git -Argument (
        @('-C', $repositoryBuildRoot, 'add', '--') + @($baselinePaths)
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_GIT_ADD_FAILED'
    $null = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        '-C', $repositoryBuildRoot, 'commit', '-m', 'chore: create disposable baseline'
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_GIT_COMMIT_FAILED'
    if ($Wsl2LinuxLive) {
        $seedHead = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $repositoryBuildRoot, 'rev-parse', '--verify', 'HEAD'
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_WSL2_SOURCE_WORKTREE_CREATE_FAILED'
        $null = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $repositoryBuildRoot, 'worktree', 'add', '--detach', $bootstrapProjectRoot, 'HEAD'
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_WSL2_SOURCE_WORKTREE_CREATE_FAILED'
        Assert-Phase4Directory -Path $bootstrapProjectRoot `
            -Failure 'PHASE4_WSL2_SOURCE_WORKTREE_IDENTITY_REJECTED'
        $sourceDotGit = Get-Item -LiteralPath (Join-Path $bootstrapProjectRoot '.git') -Force `
            -ErrorAction Stop
        if ($sourceDotGit.PSIsContainer -or $sourceDotGit.LinkType -or
            [long]$sourceDotGit.Length -lt 16 -or [long]$sourceDotGit.Length -gt 4096) {
            throw 'PHASE4_WSL2_SOURCE_WORKTREE_IDENTITY_REJECTED'
        }
        $sourceTopLevel = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $bootstrapProjectRoot, 'rev-parse', '--show-toplevel'
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_WSL2_SOURCE_WORKTREE_IDENTITY_REJECTED'
        $sourceGitDirectory = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $bootstrapProjectRoot, 'rev-parse', '--absolute-git-dir'
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_WSL2_SOURCE_WORKTREE_IDENTITY_REJECTED'
        $sourceCommonDirectory = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $bootstrapProjectRoot, 'rev-parse', '--path-format=absolute', '--git-common-dir'
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_WSL2_SOURCE_WORKTREE_IDENTITY_REJECTED'
        $sourceHead = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $bootstrapProjectRoot, 'rev-parse', '--verify', 'HEAD'
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_WSL2_SOURCE_WORKTREE_IDENTITY_REJECTED'
        $expectedGitDirectoryPrefix = $wslSourceRepository + '/.git/worktrees/'
        if ([string]$sourceTopLevel.stdout.Trim() -cne $wslBootstrapRepository -or
            -not [string]$sourceGitDirectory.stdout.Trim().StartsWith(
                $expectedGitDirectoryPrefix, [StringComparison]::Ordinal
            ) -or
            [string]$sourceGitDirectory.stdout.Trim().Substring(
                $expectedGitDirectoryPrefix.Length
            ) -cnotmatch '\A[A-Za-z0-9._-]{1,255}\z' -or
            [string]$sourceCommonDirectory.stdout.Trim() -cne
                ($wslSourceRepository + '/.git') -or
            [string]$sourceHead.stdout.Trim() -cne [string]$seedHead.stdout.Trim() -or
            [string]$sourceHead.stdout.Trim() -cnotmatch '\A[0-9a-f]{40}\z') {
            throw 'PHASE4_WSL2_SOURCE_WORKTREE_IDENTITY_REJECTED'
        }
    }
    [IO.File]::WriteAllText(
        (Join-Path $projectRoot 'registry-only.credentials'),
        "REGISTRY_ONLY_IGNORED_SENTINEL`n",
        $script:Utf8
    )
    $remote = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        '-C', $projectRoot, 'remote'
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_GIT_REMOTE_CHECK_FAILED'
    $initialStatus = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        '-C', $projectRoot, 'status', '--porcelain=v1', '--untracked-files=all'
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_GIT_STATUS_FAILED'
    if (-not [string]::IsNullOrWhiteSpace($remote.stdout) -or
        -not [string]::IsNullOrWhiteSpace($initialStatus.stdout)) {
        throw 'PHASE4_DISPOSABLE_GIT_REJECTED'
    }
    $gitControlBefore = Get-Phase4GitControlEvidence -Git $git -Repository $projectRoot `
        -Environment $gitEnvironment -WorkingDirectory $runRoot

    $failureStage = 'POSTGRES_INIT'
    $password = Get-Phase4RandomHex -ByteCount 32
    [IO.File]::WriteAllText($passwordPath, $password, [Text.Encoding]::ASCII)
    $null = Invoke-Phase4Process -Executable $script:InitDb -Argument @(
        '-D', $dataRoot, '-U', 'runtime_bootstrap', '--auth-host=scram-sha-256',
        '--auth-local=trust', ('--pwfile=' + $passwordPath), '--encoding=UTF8', '--locale=C',
        '--data-checksums'
    ) -Environment (New-Phase4ClosedEnvironment -Values ([ordered]@{})) `
        -WorkingDirectory $runRoot -StandardInput $null -TimeoutSeconds 120 `
        -Failure 'PHASE4_INITDB_FAILED' `
        -OutputEncodingCodePage $script:WindowsOemCodePage
    Remove-Item -LiteralPath $passwordPath -Force
    if (Test-Path -LiteralPath $passwordPath) { throw 'PHASE4_PASSWORD_FILE_CLEANUP_REJECTED' }
    $firstPostgres = Start-Phase4Postgres -RunRoot $runRoot -RunId $runId -Port $postgresPort `
        -DataRoot $dataRoot -MarkerPath $markerPath
    $postgresRunning = $true

    $failureStage = 'CONTROL_START'
    $controlSession = Start-Phase4Control -NodeExecutable $node -ControlHome $controlHome `
        -Port $controlPort
    $firstControl = [pscustomobject][ordered]@{
        process_id = [int]$controlSession.process_id
        process_start_utc_ticks = [long]$controlSession.process_start_utc_ticks
    }
    $registered = Invoke-Phase4ControlJson -Port $controlPort -Method POST -Path '/api/projects' `
        -Body ([ordered]@{ name = $projectName; rootPath = $projectRoot }) `
        -Failure 'PHASE4_CONTROL_PROJECT_REGISTER_FAILED'
    $projectId = [string]$registered.id
    if ($projectId -cnotmatch '\A[a-z0-9][a-z0-9._-]{1,63}\z' -or
        [string]$registered.name -cne $projectName -or
        [string]$registered.record_kind -cne 'CONTROL_LOCAL_CATALOG' -or
        [string]$registered.registry_authority -cne 'NONE' -or
        $null -ne $registered.registry_project_id -or
        -not [string]::Equals(
            [IO.Path]::GetFullPath([string]$registered.canonical_path),
            [IO.Path]::GetFullPath($projectRoot),
            [StringComparison]::OrdinalIgnoreCase
        )) {
        throw 'PHASE4_CONTROL_PROJECT_REJECTED'
    }
    $projectProjectionDigest = Get-Phase4ControlProjectDigest -Project $registered

    $authorityObservation = Get-Phase4StringSha256 -Value ('phase4-authority-observation:' + $runId)
    $authorityHead = Get-Phase4StringSha256 -Value ('phase4-authority-head:' + $runId)
    $ingressProfile = Get-Phase4StringSha256 -Value 'lattice.phase4.managed-foreman.local-acceptance.v1'
    $baseEnvironment = [ordered]@{
        LATTICE_FULL_CHAIN_RUN_MODE = 'RESUME_EXISTING'
        LATTICE_RUNTIME_INTEGRATION = 'CORE_ONLY'
        LATTICE_HERMES_MODE = 'TASK_ONLY'
        LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
        LATTICE_DELIVERY_TIMEOUT_SECONDS = [string]$ProcessTimeoutSeconds
        LATTICE_TASK019_HOST = '127.0.0.1'
        LATTICE_TASK019_PORT = [string]$postgresPort
        LATTICE_TASK019_RUN_ID = $runId
        LATTICE_TASK019_PASSWORD = $password
        LATTICE_STORE_DAEMON_INSTANCE_ID = ('phase4-managed-' + $runId.Substring(0, 12))
        LATTICE_STORE_DAEMON_EPOCH = '404'
        LATTICE_STORE_AUTHORITY_REVISION = '404'
        LATTICE_STORE_OBSERVATION_DIGEST = $authorityObservation
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = $authorityHead
        LATTICE_TASK_INGRESS_KIND = 'LOCAL_CANONICAL_MCP_ACCEPTANCE'
        LATTICE_TASK_INGRESS_PROFILE_SHA256 = $ingressProfile
        LATTICE_CONTROL_ORIGIN = "http://127.0.0.1:$controlPort"
        LATTICE_DELIVERY_GIT_EXE = $git
        LATTICE_DELIVERY_LAUNCHER = $codex
        LATTICE_DELIVERY_LAUNCHER_VERSION = $script:ExpectedCodexLauncherVersion
        LATTICE_DELIVERY_LAUNCHER_SHA256 = $(
            if ($ScriptedActiveRestart) { [string]$scriptedFixture.launcher_sha256 }
            else { $script:ExpectedCodexSha256 }
        )
        LATTICE_DELIVERY_CODEX_HOME = $codexProfileRoot
        LATTICE_GRAPHIFY_SOURCE_ROOT = $projectRoot
        LATTICE_MANAGED_NODE_EXE = $node
        LATTICE_MANAGED_NPM_EXE = $npm
        LATTICE_MANAGED_WORKER_BRIDGE = [IO.Path]::GetFullPath($script:ManagedBridge)
        LATTICE_MANAGED_WORKTREE_ROOT = [IO.Path]::GetFullPath($managedWorktreeRoot)
    }
    $failureStage = 'POSTGRES_BOOTSTRAP'
    $bootstrapEnvironment = [ordered]@{}
    foreach ($entry in $baseEnvironment.GetEnumerator()) {
        $bootstrapEnvironment[[string]$entry.Key] = [string]$entry.Value
    }
    $bootstrapEnvironment['LATTICE_MANAGED_FOREMAN_MODE'] = 'DISABLED'
    $closedBootstrap = New-Phase4ClosedEnvironment -Values $bootstrapEnvironment
    $null = Invoke-Phase4Process -Executable $latticed -Argument @('--postgres-initialize') `
        -Environment $closedBootstrap -WorkingDirectory $script:RepositoryRoot -StandardInput $null `
        -TimeoutSeconds 120 -Failure 'PHASE4_POSTGRES_INITIALIZE_FAILED'
    $null = Invoke-Phase4Process -Executable $latticed -Argument @('--postgres-bootstrap') `
        -Environment $closedBootstrap -WorkingDirectory $script:RepositoryRoot -StandardInput $null `
        -TimeoutSeconds 240 -Failure 'PHASE4_POSTGRES_BOOTSTRAP_FAILED'

    $failureStage = if ($Wsl2LinuxLive) { 'WSL2_DRAFT_SUBMIT' } else { 'FOREMAN_CHECKPOINT' }
    $mcpSession = Start-Phase4McpSession `
        -Name $(if ($Wsl2LinuxLive) { 'wsl2-draft-submit' } else { 'checkpoint' }) `
        -Latticed $latticed `
        -Environment $bootstrapEnvironment -TimeoutSeconds $ProcessTimeoutSeconds
    if ($Wsl2LinuxLive) {
        $submitted = Invoke-Phase4McpTool -Session $mcpSession `
            -ToolName 'lattice_task_submit' -Arguments ([ordered]@{
                client_request_id = $clientRequestId
                objective = $objective
                project_id = $projectId
            }) -TimeoutSeconds $ProcessTimeoutSeconds
        $lastManagedStatus = $submitted
        $taskRef = [string]$submitted.task_ref
        Assert-Phase4DisabledDraftStatus -Status $submitted `
            -ExpectedTaskRef $taskRef -ExpectedProjectId $projectId `
            -Failure 'PHASE4_WSL2_DRAFT_SUBMIT_REJECTED'
        $draftStatus = Invoke-Phase4McpTool -Session $mcpSession `
            -ToolName 'lattice_task_status' `
            -Arguments (New-Phase4GeneralTaskStatusArguments -TaskRef $taskRef) `
            -TimeoutSeconds $ProcessTimeoutSeconds
        $lastManagedStatus = $draftStatus
        Assert-Phase4DisabledDraftStatus -Status $draftStatus `
            -ExpectedTaskRef $taskRef -ExpectedProjectId $projectId `
            -ExpectedLedgerHeadDigest ([string]$submitted.ledger_head_digest) `
            -Failure 'PHASE4_WSL2_DRAFT_REPLAY_REJECTED'
        if ((Get-Phase4StringSha256 -Value (
                $submitted | ConvertTo-Json -Compress -Depth 30
            )) -cne (Get-Phase4StringSha256 -Value (
                $draftStatus | ConvertTo-Json -Compress -Depth 30
            ))) {
            throw 'PHASE4_WSL2_DRAFT_REPLAY_REJECTED'
        }
        $taskSubmissionIdentity = Get-Phase4TaskSubmissionIdentity -Password $password `
            -Port $postgresPort -Database $databaseName -TaskRef $taskRef `
            -WorkingDirectory $runRoot
        if ([string]$taskSubmissionIdentity.client_request_id -cne $clientRequestId -or
            [string]$taskSubmissionIdentity.project_id -cne $projectId) {
            throw 'PHASE4_TASK_SUBMISSION_IDENTITY_REJECTED'
        }
        $wslZeroProviderEvidence = Get-Phase4WslDurableEvidence -Password $password `
            -Port $postgresPort -Database $databaseName -TaskRef $taskRef `
            -WorkingDirectory $runRoot
        Assert-Phase4WslZeroProviderEvidence -Evidence $wslZeroProviderEvidence
    }
    else {
        $checkpoint = Invoke-Phase4FormalForemanCheckpoint -Session $mcpSession `
            -RunId $runId -TimeoutSeconds $ProcessTimeoutSeconds
    }
    $checkpointProcess = Stop-Phase4McpSession -Session $mcpSession
    $mcpRecords.Add($checkpointProcess)
    $mcpSession = $null

    if ($Wsl2LinuxLive) {
        $failureStage = 'WSL2_BOOTSTRAP_MATERIALIZATION'
        $sourceHead = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $projectRoot, 'rev-parse', '--verify', 'HEAD^{commit}'
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_WSL2_SOURCE_HEAD_READ_FAILED'
        $baseCommit = [string]$sourceHead.stdout.Trim()
        if ($baseCommit -cnotmatch '\A[0-9a-f]{40}\z') {
            throw 'PHASE4_WSL2_SOURCE_HEAD_REJECTED'
        }
        $bootstrapMaterialization = Invoke-Phase4Wsl2Materializer `
            -NodeExecutable $node -TaskRoot $Wsl2TaskRoot `
            -Repository $wslBootstrapRepository -TaskRef $taskRef `
            -ProcessFence (Get-Phase4RandomHex -ByteCount 32) `
            -WorktreeRef ('worktree:sha256:' + (Get-Phase4RandomHex -ByteCount 32)) `
            -ExpectedRepositoryHead $baseCommit `
            -ExpectedSupervisorSha256 $wslSupervisorSourceSha256
        if ([string]$bootstrapMaterialization.descriptor.linux.config_digest -cne
                ('codex-config:sha256:' + $script:ExpectedWsl2CodexConfigSha256) -or
            [string]$bootstrapMaterialization.descriptor.linux.supervisor_path -cne
                ($Wsl2TaskRoot + '/runtime-v4/wsl2-codex-supervisor.mjs') -or
            [string]$bootstrapMaterialization.descriptor.linux.supervisor_sha256 -cne
                $wslSupervisorSourceSha256) {
            throw 'PHASE4_WSL2_REVIEWED_BYTES_IDENTITY_REJECTED'
        }

        $failureStage = 'WSL2_MANAGED_WORKTREE_PREPARE'
        $managedPrepare = Invoke-Phase4ManagedWorktreeBridge -NodeExecutable $node `
            -Operation 'prepare' -RepositoryRoot $projectRoot `
            -WorktreeRoot $managedWorktreeRoot -GitExecutable $git `
            -TaskRef $taskRef -TaskId ([string]$taskSubmissionIdentity.task_id) `
            -BaseCommit $baseCommit -ExpectedBaselineSha256 $null `
            -ExpectedExecutionEnvironmentRef `
                ([string]$bootstrapMaterialization.record.execution_environment_ref) `
            -ExecutionEnvironmentJson ([string]$bootstrapMaterialization.descriptor_json)
        $expectedManagedLinuxPath = $wslManagedWorktreeRoot + '/work-' +
            $taskRef.Substring(0, 59)
        $expectedManagedUncPath = ConvertTo-Phase4WslUncPath `
            -LinuxPath $expectedManagedLinuxPath
        if ([bool]$managedPrepare.replayed -or
            [string]$managedPrepare.worktree_id -cne
                ('WORK-' + $taskRef.Substring(0, 59).ToUpperInvariant()) -or
            -not [string]::Equals(
                [IO.Path]::GetFullPath([string]$managedPrepare.worktree_path),
                [IO.Path]::GetFullPath($expectedManagedUncPath),
                [StringComparison]::OrdinalIgnoreCase
            ) -or
            [string]$managedPrepare.baseline_sha256 -cnotmatch '\A[0-9a-f]{64}\z') {
            throw 'PHASE4_WSL2_MANAGED_WORKTREE_PREPARE_REJECTED'
        }
        $managedHead = Invoke-Phase4RepositoryGit -Git $git -Argument @(
            '-C', $expectedManagedUncPath, 'rev-parse', '--verify', 'HEAD^{commit}'
        ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
            -Failure 'PHASE4_WSL2_MANAGED_HEAD_READ_FAILED'
        if ([string]$managedHead.stdout.Trim() -cne $baseCommit) {
            throw 'PHASE4_WSL2_MANAGED_HEAD_REJECTED'
        }

        $failureStage = 'WSL2_FINAL_MATERIALIZATION'
        $finalMaterialization = Invoke-Phase4Wsl2Materializer `
            -NodeExecutable $node -TaskRoot $Wsl2TaskRoot `
            -Repository $expectedManagedLinuxPath -TaskRef $taskRef `
            -ProcessFence (Get-Phase4RandomHex -ByteCount 32) `
            -WorktreeRef ('worktree:sha256:' + [string]$managedPrepare.baseline_sha256) `
            -ExpectedRepositoryHead $baseCommit `
            -ExpectedSupervisorSha256 $wslSupervisorSourceSha256
        if ([string]$finalMaterialization.record.execution_environment_ref -ceq
            [string]$bootstrapMaterialization.record.execution_environment_ref -or
            [string]$finalMaterialization.descriptor.linux.cwd -cne $expectedManagedLinuxPath -or
            [string]$finalMaterialization.descriptor.linux.config_digest -cne
                ('codex-config:sha256:' + $script:ExpectedWsl2CodexConfigSha256) -or
            [string]$finalMaterialization.descriptor.linux.supervisor_path -cne
                ($Wsl2TaskRoot + '/runtime-v4/wsl2-codex-supervisor.mjs') -or
            [string]$finalMaterialization.descriptor.linux.supervisor_sha256 -cne
                $wslSupervisorSourceSha256) {
            throw 'PHASE4_WSL2_FINAL_DESCRIPTOR_REJECTED'
        }
        $managedVerify = Invoke-Phase4ManagedWorktreeBridge -NodeExecutable $node `
            -Operation 'verify' -RepositoryRoot $projectRoot `
            -WorktreeRoot $managedWorktreeRoot -GitExecutable $git `
            -TaskRef $taskRef -TaskId ([string]$taskSubmissionIdentity.task_id) `
            -BaseCommit $baseCommit `
            -ExpectedBaselineSha256 ([string]$managedPrepare.baseline_sha256) `
            -ExpectedExecutionEnvironmentRef `
                ([string]$finalMaterialization.record.execution_environment_ref) `
            -ExecutionEnvironmentJson ([string]$finalMaterialization.descriptor_json)
        if (-not [bool]$managedVerify.replayed -or
            [string]$managedVerify.baseline_sha256 -cne
                [string]$managedPrepare.baseline_sha256 -or
            [string]$managedVerify.worktree_id -cne [string]$managedPrepare.worktree_id -or
            [string]$managedVerify.branch -cne [string]$managedPrepare.branch -or
            [string]$managedVerify.worktree_path -cne [string]$managedPrepare.worktree_path) {
            throw 'PHASE4_WSL2_MANAGED_WORKTREE_REPLAY_REJECTED'
        }
        $failureStage = 'WSL2_SUBSTITUTION_GATES'
        $baselineSubstitutionRejected = $false
        try {
            $null = Invoke-Phase4ManagedWorktreeBridge -NodeExecutable $node `
                -Operation 'verify' -RepositoryRoot $projectRoot `
                -WorktreeRoot $managedWorktreeRoot -GitExecutable $git `
                -TaskRef $taskRef -TaskId ([string]$taskSubmissionIdentity.task_id) `
                -BaseCommit $baseCommit -ExpectedBaselineSha256 (Get-Phase4RandomHex -ByteCount 32) `
                -ExpectedExecutionEnvironmentRef `
                    ([string]$finalMaterialization.record.execution_environment_ref) `
                -ExecutionEnvironmentJson ([string]$finalMaterialization.descriptor_json)
        }
        catch {
            if ([string]$_.Exception.Message -ceq
                'MANAGED_WORKTREE_BASELINE_SUBSTITUTION') {
                $baselineSubstitutionRejected = $true
            }
            else { throw }
        }
        $validDescriptorSubstitutionRejected = $false
        try {
            $null = Invoke-Phase4ManagedWorktreeBridge -NodeExecutable $node `
                -Operation 'verify' -RepositoryRoot $projectRoot `
                -WorktreeRoot $managedWorktreeRoot -GitExecutable $git `
                -TaskRef $taskRef -TaskId ([string]$taskSubmissionIdentity.task_id) `
                -BaseCommit $baseCommit `
                -ExpectedBaselineSha256 ([string]$managedPrepare.baseline_sha256) `
                -ExpectedExecutionEnvironmentRef `
                    ([string]$finalMaterialization.record.execution_environment_ref) `
                -ExecutionEnvironmentJson ([string]$bootstrapMaterialization.descriptor_json)
        }
        catch {
            if ([string]$_.Exception.Message -ceq
                'MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_BINDING_REJECTED') {
                $validDescriptorSubstitutionRejected = $true
            }
            else { throw }
        }
        $resealedDescriptor = New-Phase4ResealedExecutionEnvironmentSubstitution `
            -NodeExecutable $node `
            -DescriptorJson ([string]$finalMaterialization.descriptor_json)
        if ([string]$resealedDescriptor.execution_environment_ref -ceq
            [string]$finalMaterialization.record.execution_environment_ref) {
            throw 'PHASE4_WSL2_DESCRIPTOR_SUBSTITUTION_FIXTURE_REJECTED'
        }
        $resealedDescriptorSubstitutionRejected = $false
        try {
            $null = Invoke-Phase4ManagedWorktreeBridge -NodeExecutable $node `
                -Operation 'verify' -RepositoryRoot $projectRoot `
                -WorktreeRoot $managedWorktreeRoot -GitExecutable $git `
                -TaskRef $taskRef -TaskId ([string]$taskSubmissionIdentity.task_id) `
                -BaseCommit $baseCommit `
                -ExpectedBaselineSha256 ([string]$managedPrepare.baseline_sha256) `
                -ExpectedExecutionEnvironmentRef `
                    ([string]$finalMaterialization.record.execution_environment_ref) `
                -ExecutionEnvironmentJson ([string]$resealedDescriptor.descriptor_json)
        }
        catch {
            if ([string]$_.Exception.Message -ceq
                'MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_BINDING_REJECTED') {
                $resealedDescriptorSubstitutionRejected = $true
            }
            else { throw }
        }
        $cwdNeedle = '"cwd":"' + $expectedManagedLinuxPath + '"'
        $cwdReplacement = '"cwd":"' + $wslBootstrapRepository + '"'
        $mutatedDescriptorJson = ([string]$finalMaterialization.descriptor_json).Replace(
            $cwdNeedle,
            $cwdReplacement,
            [StringComparison]::Ordinal
        )
        if ($mutatedDescriptorJson -ceq [string]$finalMaterialization.descriptor_json) {
            throw 'PHASE4_WSL2_DESCRIPTOR_SUBSTITUTION_FIXTURE_REJECTED'
        }
        $descriptorSubstitutionRejected = $false
        try {
            $null = Invoke-Phase4ManagedWorktreeBridge -NodeExecutable $node `
                -Operation 'verify' -RepositoryRoot $projectRoot `
                -WorktreeRoot $managedWorktreeRoot -GitExecutable $git `
                -TaskRef $taskRef -TaskId ([string]$taskSubmissionIdentity.task_id) `
                -BaseCommit $baseCommit `
                -ExpectedBaselineSha256 ([string]$managedPrepare.baseline_sha256) `
                -ExpectedExecutionEnvironmentRef `
                    ([string]$finalMaterialization.record.execution_environment_ref) `
                -ExecutionEnvironmentJson $mutatedDescriptorJson
        }
        catch {
            if ([string]$_.Exception.Message -ceq
                'MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_REJECTED') {
                $descriptorSubstitutionRejected = $true
            }
            else { throw }
        }
        if (-not $baselineSubstitutionRejected -or
            -not $validDescriptorSubstitutionRejected -or
            -not $resealedDescriptorSubstitutionRejected -or
            -not $descriptorSubstitutionRejected) {
            throw 'PHASE4_WSL2_SUBSTITUTION_NOT_REJECTED'
        }
        $wslSubstitutionEvidence = [pscustomobject][ordered]@{
            baseline_digest_substitution_rejected = $baselineSubstitutionRejected
            valid_descriptor_substitution_rejected = $validDescriptorSubstitutionRejected
            resealed_descriptor_substitution_rejected =
                $resealedDescriptorSubstitutionRejected
            descriptor_path_substitution_rejected = $descriptorSubstitutionRejected
            provider_effect_count = 0
            artifact_outbox_count = 0
        }
        $wslZeroProviderAfterMaterialization = Get-Phase4WslDurableEvidence `
            -Password $password -Port $postgresPort -Database $databaseName `
            -TaskRef $taskRef -WorkingDirectory $runRoot
        Assert-Phase4WslZeroProviderEvidence -Evidence $wslZeroProviderAfterMaterialization
        if ([string]$wslZeroProviderAfterMaterialization.digest -cne
            [string]$wslZeroProviderEvidence.digest) {
            throw 'PHASE4_WSL2_PREFLIGHT_DURABLE_STATE_CHANGED'
        }
        $failureStage = 'WSL2_FOREMAN_CHECKPOINT'
        $wslCheckpointEnvironment = [ordered]@{}
        foreach ($entry in $bootstrapEnvironment.GetEnumerator()) {
            $wslCheckpointEnvironment[[string]$entry.Key] = [string]$entry.Value
        }
        $wslCheckpointEnvironment['LATTICE_GRAPHIFY_SOURCE_ROOT'] = $expectedManagedUncPath
        $wslCheckpointEnvironment['LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON'] =
            [string]$finalMaterialization.descriptor_json
        $mcpSession = Start-Phase4McpSession -Name 'wsl2-checkpoint' `
            -Latticed $latticed -Environment $wslCheckpointEnvironment `
            -TimeoutSeconds $ProcessTimeoutSeconds
        $freshDraftStatus = Invoke-Phase4McpTool -Session $mcpSession `
            -ToolName 'lattice_task_status' `
            -Arguments (New-Phase4GeneralTaskStatusArguments -TaskRef $taskRef) `
            -TimeoutSeconds $ProcessTimeoutSeconds
        $lastManagedStatus = $freshDraftStatus
        Assert-Phase4DisabledDraftStatus -Status $freshDraftStatus `
            -ExpectedTaskRef $taskRef -ExpectedProjectId $projectId `
            -ExpectedLedgerHeadDigest ([string]$draftStatus.ledger_head_digest) `
            -Failure 'PHASE4_WSL2_FRESH_DRAFT_RECONSTRUCTION_REJECTED'
        if ((Get-Phase4StringSha256 -Value (
                $freshDraftStatus | ConvertTo-Json -Compress -Depth 30
            )) -cne (Get-Phase4StringSha256 -Value (
                $draftStatus | ConvertTo-Json -Compress -Depth 30
            ))) {
            throw 'PHASE4_WSL2_FRESH_DRAFT_RECONSTRUCTION_REJECTED'
        }
        $checkpoint = Invoke-Phase4FormalForemanCheckpoint -Session $mcpSession `
            -RunId $runId -TimeoutSeconds $ProcessTimeoutSeconds
        $wslCheckpointProcess = Stop-Phase4McpSession -Session $mcpSession
        $mcpRecords.Add($wslCheckpointProcess)
        $mcpSession = $null
        $wslZeroProviderAfterCheckpoint = Get-Phase4WslDurableEvidence `
            -Password $password -Port $postgresPort -Database $databaseName `
            -TaskRef $taskRef -WorkingDirectory $runRoot
        Assert-Phase4WslZeroProviderEvidence -Evidence $wslZeroProviderAfterCheckpoint
        if ([string]$wslZeroProviderAfterCheckpoint.digest -cne
            [string]$wslZeroProviderAfterMaterialization.digest) {
            throw 'PHASE4_WSL2_CHECKPOINT_TASK_STATE_CHANGED'
        }
        if ($Wsl2TechnicalPreflightOnly) {
            $wslTechnicalPreflightComplete = $true
            throw 'PHASE4_WSL2_TECHNICAL_PREFLIGHT_COMPLETE'
        }
    }

    $failureStage = 'MANAGED_SUBMIT'
    $activeEnvironment = [ordered]@{}
    foreach ($entry in $baseEnvironment.GetEnumerator()) {
        $activeEnvironment[[string]$entry.Key] = [string]$entry.Value
    }
    $activeEnvironment['LATTICE_MANAGED_FOREMAN_MODE'] = 'ACTIVE'
    if ($Wsl2LinuxLive) {
        $activeEnvironment['LATTICE_GRAPHIFY_SOURCE_ROOT'] = $expectedManagedUncPath
        $activeEnvironment['LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON'] =
            [string]$finalMaterialization.descriptor_json
    }
    if ($ScriptedActiveRestart) {
        $activeEnvironment['LATTICE_DELIVERY_CODEX_MODE'] = 'SCRIPTED_ACCEPTANCE'
        $activeEnvironment['LATTICE_DELIVERY_LAUNCHER_VERSION'] = 'codex-cli 0.144.6'
        $activeEnvironment['LATTICE_DELIVERY_SCHEMA_DIR'] = [string]$scriptedFixture.schema
        $activeEnvironment['LATTICE_DELIVERY_ROOT'] = [string]$scriptedFixture.delivery
        $activeEnvironment['LATTICE_MANAGED_SCRIPTED_ACTIVE_RESTART'] = '1'
        $activeEnvironment['LATTICE_MANAGED_SCRIPTED_OWNER_MARKER'] = [IO.Path]::GetFullPath($markerPath)
    }
    if ($Wsl2LinuxLive -and -not $ScriptedActiveRestart) {
        # Entering the active runtime can cross the provider boundary before a
        # thread/turn row is durably observable.  Conservatively mark the real
        # attempt here so an ambiguous crash can never be reported as free to retry.
        $realCodexAttempted = $true
        $realCodexAttemptEvidence = 'ACTIVE_RUNTIME_DISPATCH_BOUNDARY_ENTERED'
    }
    $mcpSession = Start-Phase4McpSession -Name 'managed-live' -Latticed $latticed `
        -Environment $activeEnvironment -TimeoutSeconds $ProcessTimeoutSeconds
    $firstForeman = [pscustomobject][ordered]@{
        process_id = [int]$mcpSession.process_id
        process_start_utc_ticks = [long]$mcpSession.process_start_utc_ticks
    }
    if (-not $Wsl2LinuxLive) {
        $submitted = Invoke-Phase4McpTool -Session $mcpSession -ToolName 'lattice_task_submit' `
            -Arguments ([ordered]@{
                client_request_id = $clientRequestId
                objective = $objective
                project_id = $projectId
            }) -TimeoutSeconds $ProcessTimeoutSeconds
        $taskRef = [string]$submitted.task_ref
        if ($taskRef -cnotmatch '\A[0-9a-f]{64}\z') { throw 'PHASE4_TASK_SUBMIT_REJECTED' }
    }

    if ($ScriptedActiveRestart) {
        $failureStage = 'SCRIPTED_ACTIVE_START'
        $deadline = [DateTimeOffset]::UtcNow.AddSeconds($AcceptanceTimeoutSeconds)
        $poll = 0
        do {
            $poll++
            $candidate = Invoke-Phase4McpTool -Session $mcpSession `
                -ToolName 'lattice_task_status' `
                -Arguments (New-Phase4GeneralTaskStatusArguments -TaskRef $taskRef) `
                -TimeoutSeconds $ProcessTimeoutSeconds
            if ([string]$candidate.task_state -in @('BLOCKED', 'FAILED', 'REJECTED', 'CANCELLED') -or
                [string]$candidate.status -ceq 'BLOCKED') {
                throw 'PHASE4_SCRIPTED_ACTIVE_TASK_FAILED'
            }
            if ([string]$candidate.task_state -ceq 'EXECUTING' -and [bool]$candidate.worker_running -and
                -not [string]::IsNullOrWhiteSpace([string]$candidate.thread_id) -and
                -not [string]::IsNullOrWhiteSpace([string]$candidate.turn_id)) {
                $candidateEvidence = Get-Phase4ActiveRestartEvidence -Password $password `
                    -Port $postgresPort -Database $databaseName -TaskRef $taskRef `
                    -ProjectId $projectId -WorkingDirectory $runRoot
                $value = $candidateEvidence.value
                if ([long]$value.attempt_count -eq 1 -and [long]$value.attempt_number -eq 1 -and
                    [long]$value.thread_count -eq 1 -and [long]$value.turn_count -eq 1 -and
                    [long]$value.turn_started_count -eq 1 -and [long]$value.reconciled_count -eq 0 -and
                    [long]$value.terminal_count -eq 0 -and
                    [long]$value.worker_thread_dispatch_count -eq 1 -and
                    [long]$value.worker_turn_dispatch_count -eq 1 -and
                    [long]$value.process_handoff_count -eq 0) {
                    $firstActiveStatus = $candidate
                    $activeBeforeRestart = $candidateEvidence
                    break
                }
            }
            if ($poll -ge $script:MaximumMcpStatusPolls) { break }
            Start-Sleep -Milliseconds 500
        } while ([DateTimeOffset]::UtcNow -lt $deadline)
        if ($null -eq $firstActiveStatus -or $null -eq $activeBeforeRestart) {
            throw 'PHASE4_SCRIPTED_ACTIVE_START_TIMEOUT'
        }
        Assert-Phase4ActiveManagedStatus -Status $firstActiveStatus -ExpectedTaskRef $taskRef
        if ([long]$firstActiveStatus.foreman_generation -ne [long]$checkpoint.generation -or
            [string]$firstActiveStatus.foreman_checkpoint_digest -cne
            [string]$checkpoint.checkpoint_digest) {
            throw 'PHASE4_SCRIPTED_FOREMAN_CHECKPOINT_A_REJECTED'
        }
        $before = $activeBeforeRestart.value
        if ([string]$before.model -cne 'gpt-5.6-terra' -or
            [string]$before.model_reason -cne 'ROUTINE_ENGINEERING' -or
            [string]$before.thread_id -cne [string]$firstActiveStatus.thread_id -or
            [string]$before.turn_id -cne [string]$firstActiveStatus.turn_id -or
            [string]$before.writer_status -cne 'ACTIVE' -or
            [string]$before.writer_attempt_id -cne [string]$before.attempt_id -or
            [long]$before.writer_current_fence -ne [long]$before.writer_fence -or
            [long]$before.writer_process_id -ne [long]$firstForeman.process_id -or
            [string]$before.writer_process_start_identity -cnotmatch '\A[0-9a-f]{64}\z') {
            throw 'PHASE4_SCRIPTED_ACTIVE_START_EVIDENCE_REJECTED'
        }

        $failureStage = 'SCRIPTED_FOREMAN_HARD_RESTART'
        $preCrashEvents = Get-Phase4ScriptedEventEvidence `
            -Path ([string]$scriptedFixture.events) `
            -GenerationPath ([string]$scriptedFixture.generations) `
            -ExpectedGenerationCount 2
        if ([long]$preCrashEvents.thread_start_count -ne 1 -or
            [long]$preCrashEvents.turn_start_count -ne 1 -or
            [long]$preCrashEvents.turn_interrupt_count -ne 0 -or
            [long]$preCrashEvents.terminal_ack_count -ne 0 -or
            [long]$preCrashEvents.start_server_pid -le 0 -or
            [long]$preCrashEvents.start_server_start_utc_ticks -le 0) {
            throw 'PHASE4_SCRIPTED_PRECRASH_IDENTITY_REJECTED'
        }
        if ([string]$preCrashEvents.probe_server_identity -cnotmatch
            '\A(?<pid>[1-9][0-9]*):(?<start>[1-9][0-9]*)\z') {
            throw 'PHASE4_SCRIPTED_PROBE_IDENTITY_REJECTED'
        }
        $probeScriptedServer = [pscustomobject][ordered]@{
            process_id = [long]$Matches.pid
            process_start_utc_ticks = [long]$Matches.start
            identity = [string]$preCrashEvents.probe_server_identity
        }
        $probeServerAbsence = Assert-Phase4ProcessIdentityAbsent `
            -ProcessId ([long]$probeScriptedServer.process_id) `
            -ProcessStartUtcTicks ([long]$probeScriptedServer.process_start_utc_ticks)
        $firstScriptedServer = [pscustomobject][ordered]@{
            process_id = [long]$preCrashEvents.start_server_pid
            process_start_utc_ticks = [long]$preCrashEvents.start_server_start_utc_ticks
            identity = [string]$preCrashEvents.start_server_identity
        }
        $hardStopped = Stop-Phase4McpSessionHard -Session $mcpSession
        $mcpRecords.Add($hardStopped)
        $mcpSession = $null
        $firstServerAbsence = Assert-Phase4ProcessIdentityAbsent `
            -ProcessId ([long]$firstScriptedServer.process_id) `
            -ProcessStartUtcTicks ([long]$firstScriptedServer.process_start_utc_ticks)
        $mcpSession = Start-Phase4McpSession -Name 'managed-scripted-restart' `
            -Latticed $latticed -Environment $activeEnvironment `
            -TimeoutSeconds $ProcessTimeoutSeconds
        $secondForeman = [pscustomobject][ordered]@{
            process_id = [int]$mcpSession.process_id
            process_start_utc_ticks = [long]$mcpSession.process_start_utc_ticks
        }
        if ([long]$secondForeman.process_start_utc_ticks -eq
            [long]$firstForeman.process_start_utc_ticks -or
            [long]$secondForeman.process_id -eq [long]$firstForeman.process_id) {
            throw 'PHASE4_FOREMAN_PHYSICAL_RESTART_REJECTED'
        }

        $failureStage = 'SCRIPTED_ACTIVE_RECONCILE'
        $deadline = [DateTimeOffset]::UtcNow.AddSeconds($AcceptanceTimeoutSeconds)
        $poll = 0
        do {
            $poll++
            $candidate = Invoke-Phase4McpTool -Session $mcpSession `
                -ToolName 'lattice_task_status' `
                -Arguments (New-Phase4GeneralTaskStatusArguments -TaskRef $taskRef) `
                -TimeoutSeconds $ProcessTimeoutSeconds
            if ([string]$candidate.task_state -in @('BLOCKED', 'FAILED', 'REJECTED', 'CANCELLED') -or
                [string]$candidate.status -ceq 'BLOCKED') {
                throw 'PHASE4_SCRIPTED_RECONCILE_TASK_FAILED'
            }
            if ([string]$candidate.task_state -ceq 'EXECUTING' -and [bool]$candidate.worker_running) {
                $candidateEvidence = Get-Phase4ActiveRestartEvidence -Password $password `
                    -Port $postgresPort -Database $databaseName -TaskRef $taskRef `
                    -ProjectId $projectId -WorkingDirectory $runRoot
                $value = $candidateEvidence.value
                if ([long]$value.reconciled_count -eq 1 -and
                    [long]$value.process_handoff_count -eq 1 -and
                    [long]$value.writer_process_id -eq [long]$secondForeman.process_id) {
                    $secondActiveStatus = $candidate
                    $activeAfterRestart = $candidateEvidence
                    break
                }
            }
            if ($poll -ge $script:MaximumMcpStatusPolls) { break }
            Start-Sleep -Milliseconds 500
        } while ([DateTimeOffset]::UtcNow -lt $deadline)
        if ($null -eq $secondActiveStatus -or $null -eq $activeAfterRestart) {
            throw 'PHASE4_SCRIPTED_ACTIVE_RECONCILE_TIMEOUT'
        }
        Assert-Phase4ActiveManagedStatus -Status $secondActiveStatus -ExpectedTaskRef $taskRef
        if ([long]$secondActiveStatus.foreman_generation -ne [long]$checkpoint.generation -or
            [string]$secondActiveStatus.foreman_checkpoint_digest -cne
            [string]$checkpoint.checkpoint_digest) {
            throw 'PHASE4_SCRIPTED_FOREMAN_CHECKPOINT_B_REJECTED'
        }
        $after = $activeAfterRestart.value
        if ([long]$after.attempt_count -ne 1 -or [long]$after.attempt_number -ne 1 -or
            [string]$after.attempt_id -cne [string]$before.attempt_id -or
            [long]$after.writer_fence -ne [long]$before.writer_fence -or
            [string]$after.thread_id -cne [string]$before.thread_id -or
            [string]$after.turn_id -cne [string]$before.turn_id -or
            [string]$secondActiveStatus.thread_id -cne [string]$before.thread_id -or
            [string]$secondActiveStatus.turn_id -cne [string]$before.turn_id -or
            [long]$after.thread_count -ne 1 -or [long]$after.turn_count -ne 1 -or
            [long]$after.turn_started_count -ne 1 -or [long]$after.reconciled_count -ne 1 -or
            [long]$after.terminal_count -ne 0 -or
            [long]$after.worker_thread_dispatch_count -ne 1 -or
            [long]$after.worker_turn_dispatch_count -ne 1 -or
            [string]$after.writer_status -cne 'ACTIVE' -or
            [string]$after.writer_attempt_id -cne [string]$before.attempt_id -or
            [long]$after.writer_current_fence -ne [long]$before.writer_fence -or
            [long]$after.process_handoff_count -ne 1 -or
            [long]$after.writer_process_id -ne [long]$secondForeman.process_id -or
            [string]$after.writer_process_start_identity -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$after.writer_process_start_identity -ceq
            [string]$before.writer_process_start_identity -or
            @($after.process_handoffs).Count -ne 1) {
            throw 'PHASE4_SCRIPTED_ACTIVE_RECONCILE_EVIDENCE_REJECTED'
        }
        $scriptedEventEvidence = Get-Phase4ScriptedEventEvidence `
            -Path ([string]$scriptedFixture.events) `
            -GenerationPath ([string]$scriptedFixture.generations) `
            -ExpectedGenerationCount 3
        if ([long]$scriptedEventEvidence.thread_start_count -ne 1 -or
            [long]$scriptedEventEvidence.turn_start_count -ne 1 -or
            [long]$scriptedEventEvidence.thread_resume_count -lt 1 -or
            [long]$scriptedEventEvidence.thread_read_count -lt 1 -or
            [long]$scriptedEventEvidence.turn_interrupt_count -ne 0) {
            throw 'PHASE4_SCRIPTED_PROVIDER_REPLAY_REJECTED'
        }
        $resumeEvents = @($scriptedEventEvidence.events | Where-Object {
            [string]$_.event -ceq 'THREAD_RESUME'
        })
        if ($resumeEvents.Count -lt 1 -or
            @($scriptedEventEvidence.resume_server_identities).Count -ne 1) {
            throw 'PHASE4_SCRIPTED_RESTART_SERVER_IDENTITY_REJECTED'
        }
        $secondScriptedServer = [pscustomobject][ordered]@{
            process_id = [long]$resumeEvents[0].server_pid
            process_start_utc_ticks = [long]$resumeEvents[0].server_start_utc_ticks
            identity = [string]@($scriptedEventEvidence.resume_server_identities)[0]
        }
        if ([string]$secondScriptedServer.identity -ceq [string]$firstScriptedServer.identity) {
            throw 'PHASE4_SCRIPTED_RESTART_SERVER_IDENTITY_REJECTED'
        }
        $noDuplicateAgent = $true
        $physicalRestart = $true
        $restartMcpRecord = Stop-Phase4McpSession -Session $mcpSession
        $mcpRecords.Add($restartMcpRecord)
        $mcpSession = $null
        $scriptedEventEvidence = Get-Phase4ScriptedEventEvidence `
            -Path ([string]$scriptedFixture.events) `
            -GenerationPath ([string]$scriptedFixture.generations) `
            -ExpectedGenerationCount 3
        $secondIdentity = [string]$secondScriptedServer.identity
        $secondEvents = @($scriptedEventEvidence.events | Where-Object {
            ('{0}:{1}' -f [long]$_.server_pid, [long]$_.server_start_utc_ticks) -ceq
            $secondIdentity
        })
        $firstIdentity = [string]$firstScriptedServer.identity
        $firstEvents = @($scriptedEventEvidence.events | Where-Object {
            ('{0}:{1}' -f [long]$_.server_pid, [long]$_.server_start_utc_ticks) -ceq
            $firstIdentity
        })
        if ([long]$scriptedEventEvidence.thread_start_count -ne 1 -or
            [long]$scriptedEventEvidence.turn_start_count -ne 1 -or
            [long]$scriptedEventEvidence.thread_resume_count -lt 1 -or
            [long]$scriptedEventEvidence.thread_read_count -lt 1 -or
            [long]$scriptedEventEvidence.turn_interrupt_count -ne 1 -or
            [long]$scriptedEventEvidence.terminal_ack_count -ne 1 -or
            @($firstEvents | Where-Object {
                [string]$_.event -in @('TURN_INTERRUPT', 'TURN_TERMINAL_ACK', 'SERVER_EXIT')
            }).Count -ne 0 -or
            @($secondEvents | Where-Object { [string]$_.event -ceq 'TURN_INTERRUPT' }).Count -ne 1 -or
            @($secondEvents | Where-Object { [string]$_.event -ceq 'TURN_TERMINAL_ACK' }).Count -ne 1 -or
            @($secondEvents | Where-Object { [string]$_.event -ceq 'SERVER_EXIT' }).Count -ne 1) {
            throw 'PHASE4_SCRIPTED_SAFE_TEARDOWN_REJECTED'
        }
        $secondServerAbsence = Assert-Phase4ProcessIdentityAbsent `
            -ProcessId ([long]$secondScriptedServer.process_id) `
            -ProcessStartUtcTicks ([long]$secondScriptedServer.process_start_utc_ticks)
    }
    else {
    if ($Wsl2LinuxLive) {
        $failureStage = 'WSL2_ACTIVE_ACCEPTED_START'
        # Approval precedes three serial bounded preparation lanes: initial
        # worktree materialization, WSL2 zero-model preflight, and the attempt
        # baseline. Reserve the worker's first-heartbeat allowance as well.
        $activeStartWindowSeconds = [int][Math]::Min(
            [long]$AcceptanceTimeoutSeconds,
            ([long]$ProcessTimeoutSeconds * 2) +
                [long][Math]::Min([long]$ProcessTimeoutSeconds, 180) + 120
        )
        $activeStartWindowMilliseconds = [long]$activeStartWindowSeconds * 1000
        $activeStartPollOrigin = [DateTimeOffset]::UtcNow
        $deadline = $activeStartPollOrigin.AddMilliseconds($activeStartWindowMilliseconds)
        $availableActiveStatusCalls = [long]$script:MaximumMcpToolCalls -
            [long]$mcpSession.tool_call_count
        if ($availableActiveStatusCalls -lt 2) {
            throw 'PHASE4_WSL2_ACTIVE_STATUS_CALL_BUDGET_REJECTED'
        }
        # Use absolute scheduling and retain the final call until there is a
        # meaningful bounded response budget left. A one-second tail converted
        # normal gate exhaustion into an unrelated generic MCP timeout.
        $activeStartFinalPollLeadMilliseconds = $script:MinimumMcpStatusResponseBudgetMilliseconds
        $activeStartStatusResponseTimeoutSeconds = [int][Math]::Min(900, [long]$activeStartWindowSeconds)
        $activeStartPollDelayMilliseconds =
            ($activeStartWindowMilliseconds - $activeStartFinalPollLeadMilliseconds) /
            ([double]$availableActiveStatusCalls - 1.0)
        $poll = 0
        while ($poll -lt $availableActiveStatusCalls -and
            [DateTimeOffset]::UtcNow -lt $deadline) {
            $remainingActiveStartMilliseconds = [long][Math]::Ceiling(
                ($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
            )
            if ($remainingActiveStartMilliseconds -le 0) { break }
            $poll++
            $candidate = Invoke-Phase4McpStatusForGate -Session $mcpSession `
                -TaskRef $taskRef -TimeoutSeconds $activeStartStatusResponseTimeoutSeconds `
                -TimeoutMilliseconds ([int][Math]::Min(
                    ([long]$activeStartStatusResponseTimeoutSeconds * 1000),
                    $remainingActiveStartMilliseconds
                )) -TimeoutCode 'PHASE4_WSL2_ACTIVE_STATUS_RESPONSE_TIMEOUT' `
                -Stage 'WSL2_ACTIVE_ACCEPTED_START' -PollOrdinal $poll `
                -PollOrigin $activeStartPollOrigin `
                -RemainingAtDispatchMilliseconds $remainingActiveStartMilliseconds `
                -LastCompletedStatus $lastManagedStatus `
                -TimeoutDiagnostic ([ref]$mcpStatusTimeoutDiagnostic)
            # Retain every ACTIVE observation before evaluating it so a terminal
            # pre-dispatch status cannot be replaced by the earlier DRAFT receipt.
            $lastManagedStatus = $candidate
            # A response that completed outside this window is evidence, but it
            # cannot satisfy the bounded active-start acceptance gate.
            if ([DateTimeOffset]::UtcNow -ge $deadline) { break }
            $candidateBlocked = [string]$candidate.task_state -in @(
                    'BLOCKED', 'FAILED', 'REJECTED', 'CANCELLED'
                ) -or [string]$candidate.status -ceq 'BLOCKED'
            # Promotion is durable before the independently verified approval
            # evidence. The first ACTIVE poll can therefore observe this exact
            # no-attempt gate while the same authorized worker is still recording
            # that evidence. Keep polling inside the bounded start window; any
            # other blocker is terminal immediately, and a gate that never clears
            # is surfaced exactly when the window closes.
            $transientApprovalGate = Test-Phase4TransientActiveApprovalGate -Status $candidate
            if ($candidateBlocked -and -not $transientApprovalGate) {
                $managedFailureCode = [string]$candidate.failure_code
                if ($managedFailureCode -cmatch '\A[A-Z][A-Z0-9_]{0,127}\z') {
                    throw $managedFailureCode
                }
                throw 'PHASE4_WSL2_ACTIVE_START_FAILED'
            }
            if ([string]$candidate.task_state -ceq 'EXECUTING' -and
                [bool]$candidate.worker_running -and
                -not [string]::IsNullOrWhiteSpace([string]$candidate.thread_id) -and
                -not [string]::IsNullOrWhiteSpace([string]$candidate.turn_id)) {
                Assert-Phase4ActiveManagedStatus -Status $candidate -ExpectedTaskRef $taskRef
                $candidateEvidence = Get-Phase4ActiveRestartEvidence -Password $password `
                    -Port $postgresPort -Database $databaseName -TaskRef $taskRef `
                    -ProjectId $projectId -WorkingDirectory $runRoot
                if ([DateTimeOffset]::UtcNow -ge $deadline) { break }
                $value = $candidateEvidence.value
                if ([long]$value.attempt_count -eq 1 -and
                    [long]$value.attempt_number -eq 1 -and
                    [long]$value.thread_count -eq 1 -and
                    [long]$value.turn_count -eq 1 -and
                    [long]$value.turn_started_count -eq 1 -and
                    [long]$value.reconciled_count -eq 0 -and
                    [long]$value.terminal_count -eq 0 -and
                    [long]$value.worker_thread_dispatch_count -eq 1 -and
                    [long]$value.worker_turn_dispatch_count -eq 1 -and
                    [long]$value.process_handoff_count -eq 0 -and
                    [string]$value.thread_id -cne '' -and
                    [string]$value.turn_id -cne '') {
                    $firstActiveStatus = $candidate
                    $wslActiveRestartBefore = $candidateEvidence
                    break
                }
            }
            $remainingActiveStartMilliseconds = [long][Math]::Ceiling(
                ($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
            )
            if ($remainingActiveStartMilliseconds -le 0) { break }
            $activeStartNextPollAt = $activeStartPollOrigin.AddMilliseconds(
                $activeStartPollDelayMilliseconds * [double]$poll
            )
            if ($poll -ge $availableActiveStatusCalls) {
                $activeStartNextPollAt = $deadline
            }
            $activeStartSleepMilliseconds = [long][Math]::Ceiling(
                ($activeStartNextPollAt - [DateTimeOffset]::UtcNow).TotalMilliseconds
            )
            $activeStartSleepMilliseconds = [long][Math]::Min(
                $activeStartSleepMilliseconds,
                $remainingActiveStartMilliseconds
            )
            if ($activeStartSleepMilliseconds -le 0) { continue }
            Start-Sleep -Milliseconds ([int]$activeStartSleepMilliseconds)
            if ($poll -ge $availableActiveStatusCalls) { break }
        }
        if ($null -eq $firstActiveStatus -or $null -eq $wslActiveRestartBefore) {
            $managedFailureCode = [string]$lastManagedStatus.failure_code
            if ([string]$lastManagedStatus.status -ceq 'BLOCKED' -and
                $managedFailureCode -cmatch '\A[A-Z][A-Z0-9_]{0,127}\z') {
                throw $managedFailureCode
            }
            throw 'PHASE4_WSL2_ACTIVE_RESTART_WINDOW_MISSED'
        }
        if ([long]$firstActiveStatus.foreman_generation -ne [long]$checkpoint.generation -or
            [string]$firstActiveStatus.foreman_checkpoint_digest -cne
                [string]$checkpoint.checkpoint_digest) {
            throw 'PHASE4_WSL2_FOREMAN_CHECKPOINT_A_REJECTED'
        }
        $before = $wslActiveRestartBefore.value
        if ([string]$before.thread_id -cne [string]$firstActiveStatus.thread_id -or
            [string]$before.turn_id -cne [string]$firstActiveStatus.turn_id -or
            [string]$before.writer_status -cne 'ACTIVE' -or
            [string]$before.writer_attempt_id -cne [string]$before.attempt_id -or
            [long]$before.writer_current_fence -ne [long]$before.writer_fence -or
            [long]$before.writer_process_id -ne [long]$firstForeman.process_id) {
            throw 'PHASE4_WSL2_ACTIVE_ACCEPTED_EVIDENCE_REJECTED'
        }
        $wslEnvironmentAtAcceptedStart = Get-Phase4WslDurableEvidence `
            -Password $password -Port $postgresPort -Database $databaseName `
            -TaskRef $taskRef -WorkingDirectory $runRoot
        if ([long]$wslEnvironmentAtAcceptedStart.value.attempt_count -ne 1 -or
            [long]$wslEnvironmentAtAcceptedStart.value.environment_count -ne 1 -or
            [long]$wslEnvironmentAtAcceptedStart.value.validated_environment_count -ne 1 -or
            [long]$wslEnvironmentAtAcceptedStart.value.provider_effect_count -ne 2 -or
            [string]$wslEnvironmentAtAcceptedStart.value.environment_ref -cne
                [string]$finalMaterialization.record.execution_environment_ref -or
            [string]$wslEnvironmentAtAcceptedStart.value.canonical_descriptor -cne
                [string]$finalMaterialization.descriptor_json) {
            throw 'PHASE4_WSL2_ACTIVE_EXECUTION_ENVIRONMENT_REJECTED'
        }

        $realCodexAttempted = $true
        $realCodexAttemptEvidence = 'DURABLE_THREAD_TURN_AND_PROVIDER_EFFECT_OBSERVED'
        $failureStage = 'WSL2_PROVIDER_FENCE_CAPTURE'
        $wslProviderPreflightEvidence = Get-Phase4WslProviderPreflightEvidence `
            -Password $password -Port $postgresPort -Database $databaseName `
            -TaskRef $taskRef -ProjectId $projectId -WorkingDirectory $runRoot `
            -Materialization $finalMaterialization `
            -ExpectedWorktreeRef (
                'worktree:sha256:' + [string]$managedPrepare.baseline_sha256
            )
        if ([string]$before.packet_digest -cnotmatch
                '\Aattempt-packet:sha256:[0-9a-f]{64}\z' -or
            [string]$before.writer_process_start_identity -cnotmatch '\A[0-9a-f]{64}\z') {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_MARKER_IDENTITY_REJECTED'
        }
        $wslProviderOpenMarkerEvidence = Get-Phase4WslProviderSubtreeOpenEvidence `
            -Password $password -Port $postgresPort -Database $databaseName `
            -TaskRef $taskRef -ProjectId $projectId -WorkingDirectory $runRoot `
            -Materialization $finalMaterialization `
            -PreflightEvidence $wslProviderPreflightEvidence `
            -ExpectedWorktreeRef (
                'worktree:sha256:' + [string]$managedPrepare.baseline_sha256
            ) -ExpectedPacketDigest ([string]$before.packet_digest) `
            -ExpectedProducerDigest ([string]$before.writer_process_start_identity)
        $wslProviderFenceBeforeHardStop = Get-Phase4WslProviderFenceEvidence `
            -Materialization $finalMaterialization `
            -PreflightEvidence $wslProviderPreflightEvidence -Phase 'ACTIVE' `
            -OldMarkers $null
        $openProcessIdentityJson = $wslProviderOpenMarkerEvidence.process_identity |
            ConvertTo-Json -Compress -Depth 8
        $matchingOpenProcesses = @(
            $wslProviderFenceBeforeHardStop.value.process_markers | Where-Object {
                ($_ | ConvertTo-Json -Compress -Depth 8) -ceq $openProcessIdentityJson
            }
        )
        if ($matchingOpenProcesses.Count -ne 1) {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_PROCESS_FENCE_REJECTED'
        }

        $failureStage = 'WSL2_FOREMAN_HARD_RESTART'
        $hardStopped = Stop-Phase4McpSessionHard -Session $mcpSession
        $mcpRecords.Add($hardStopped)
        $mcpSession = $null
        $failureStage = 'WSL2_PROVIDER_FENCE_TEARDOWN'
        $providerFenceDeadline = [DateTimeOffset]::UtcNow.AddSeconds(20)
        $providerFenceFailureCode = $null
        do {
            try {
                $providerFenceFailureCode = $null
                $wslProviderFenceAfterHardStop = Get-Phase4WslProviderFenceEvidence `
                    -Materialization $finalMaterialization `
                    -PreflightEvidence $wslProviderPreflightEvidence -Phase 'CLOSED' `
                    -ExpectedCgroupPath (
                        [string]$wslProviderFenceBeforeHardStop.value.cgroup_path
                    ) -OldMarkers @($wslProviderFenceBeforeHardStop.value.process_markers)
                if ([bool]$wslProviderFenceAfterHardStop.value.gate_passed) { break }
            }
            catch {
                $providerFenceFailureCode = ConvertTo-Phase4FailureCode `
                    -Message ([string]$_.Exception.Message)
                if ($providerFenceFailureCode -cne
                    'PHASE4_WSL2_PROVIDER_CGROUP_QUERY_REJECTED') {
                    break
                }
            }
            Start-Sleep -Milliseconds 100
        } while ([DateTimeOffset]::UtcNow -lt $providerFenceDeadline)
        $wslProviderEffectsAfterFence = Get-Phase4WslDurableEvidence `
            -Password $password -Port $postgresPort -Database $databaseName `
            -TaskRef $taskRef -WorkingDirectory $runRoot
        $effectsBeforeFence = $wslEnvironmentAtAcceptedStart.value
        $effectsAfterFence = $wslProviderEffectsAfterFence.value
        if ([long]$effectsAfterFence.attempt_count -ne [long]$effectsBeforeFence.attempt_count -or
            [string]$effectsAfterFence.attempt_id -cne [string]$effectsBeforeFence.attempt_id -or
            [long]$effectsAfterFence.provider_effect_count -ne 2 -or
            [long]$effectsAfterFence.provider_effect_count -ne
                [long]$effectsBeforeFence.provider_effect_count -or
            [long]$effectsAfterFence.worker_thread_dispatch_count -ne
                [long]$effectsBeforeFence.worker_thread_dispatch_count -or
            [long]$effectsAfterFence.worker_turn_dispatch_count -ne
                [long]$effectsBeforeFence.worker_turn_dispatch_count -or
            [long]$effectsAfterFence.review_thread_dispatch_count -ne
                [long]$effectsBeforeFence.review_thread_dispatch_count -or
            [long]$effectsAfterFence.review_turn_dispatch_count -ne
                [long]$effectsBeforeFence.review_turn_dispatch_count -or
            [long]$effectsAfterFence.environment_count -ne
                [long]$effectsBeforeFence.environment_count -or
            [long]$effectsAfterFence.validated_environment_count -ne
                [long]$effectsBeforeFence.validated_environment_count -or
            [long]$effectsAfterFence.artifact_outbox_count -ne
                [long]$effectsBeforeFence.artifact_outbox_count -or
            [long]$effectsAfterFence.pending_worker_claim_count -ne
                [long]$effectsBeforeFence.pending_worker_claim_count -or
            [string]$effectsAfterFence.environment_ref -cne
                [string]$effectsBeforeFence.environment_ref -or
            [string]$effectsAfterFence.canonical_descriptor -cne
                [string]$effectsBeforeFence.canonical_descriptor) {
            throw 'PHASE4_WSL2_PROVIDER_EFFECTS_CHANGED_AFTER_HARD_STOP'
        }
        if ($null -ne $providerFenceFailureCode) { throw $providerFenceFailureCode }
        if ($null -eq $wslProviderFenceAfterHardStop -or
            -not [bool]$wslProviderFenceAfterHardStop.value.gate_passed -or
            -not [bool]$wslProviderFenceAfterHardStop.value.exact_old_processes_absent) {
            throw 'PHASE4_WSL2_STALE_PROVIDER_FENCE_REJECTED'
        }
        $wslProviderEffectsBeforeReconciliation = $wslProviderEffectsAfterFence
        $mcpSession = Start-Phase4McpSession -Name 'managed-wsl2-reconnect' `
            -Latticed $latticed -Environment $activeEnvironment `
            -TimeoutSeconds $ProcessTimeoutSeconds
        $wslReconnectForeman = [pscustomobject][ordered]@{
            process_id = [int]$mcpSession.process_id
            process_start_utc_ticks = [long]$mcpSession.process_start_utc_ticks
        }
        if ([long]$wslReconnectForeman.process_start_utc_ticks -eq
            [long]$firstForeman.process_start_utc_ticks -or
            [long]$wslReconnectForeman.process_id -eq [long]$firstForeman.process_id) {
            throw 'PHASE4_WSL2_FOREMAN_PHYSICAL_RESTART_REJECTED'
        }

        $failureStage = 'WSL2_ACTIVE_RECONNECT'
        $reconnectWindowSeconds = [int][Math]::Min(180, $AcceptanceTimeoutSeconds)
        $reconnectWindowMilliseconds = [long]$reconnectWindowSeconds * 1000
        $reconnectPollOrigin = [DateTimeOffset]::UtcNow
        $deadline = $reconnectPollOrigin.AddMilliseconds($reconnectWindowMilliseconds)
        # This session must retain the complete terminal-poll allowance after
        # reconciliation, so only the non-reserved calls may be used here.
        $reservedTerminalStatusCalls = [long]$script:MaximumMcpStatusPolls
        $availableReconnectStatusCalls = [long]$script:MaximumMcpToolCalls -
            [long]$mcpSession.tool_call_count - $reservedTerminalStatusCalls
        if ($availableReconnectStatusCalls -lt 2) {
            throw 'PHASE4_WSL2_RECONNECT_STATUS_CALL_BUDGET_REJECTED'
        }
        $reconnectFinalPollLeadMilliseconds = $script:MinimumMcpStatusResponseBudgetMilliseconds
        $reconnectStatusResponseTimeoutSeconds = [int][Math]::Min(900, [long]$reconnectWindowSeconds)
        $reconnectPollDelayMilliseconds =
            ($reconnectWindowMilliseconds - $reconnectFinalPollLeadMilliseconds) /
            ([double]$availableReconnectStatusCalls - 1.0)
        $poll = 0
        while ($poll -lt $availableReconnectStatusCalls -and
            [DateTimeOffset]::UtcNow -lt $deadline) {
            $remainingReconnectMilliseconds = [long][Math]::Ceiling(
                ($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
            )
            if ($remainingReconnectMilliseconds -le 0) { break }
            $poll++
            $candidate = Invoke-Phase4McpStatusForGate -Session $mcpSession `
                -TaskRef $taskRef -TimeoutSeconds $reconnectStatusResponseTimeoutSeconds `
                -TimeoutMilliseconds ([int][Math]::Min(
                    ([long]$reconnectStatusResponseTimeoutSeconds * 1000),
                    $remainingReconnectMilliseconds
                )) -TimeoutCode 'PHASE4_WSL2_RECONNECT_STATUS_RESPONSE_TIMEOUT' `
                -Stage 'WSL2_ACTIVE_RECONNECT' -PollOrdinal $poll `
                -PollOrigin $reconnectPollOrigin `
                -RemainingAtDispatchMilliseconds $remainingReconnectMilliseconds `
                -LastCompletedStatus $lastManagedStatus `
                -TimeoutDiagnostic ([ref]$mcpStatusTimeoutDiagnostic)
            $lastManagedStatus = $candidate
            if ([DateTimeOffset]::UtcNow -ge $deadline) { break }
            if ([string]$candidate.task_state -in @(
                    'BLOCKED', 'FAILED', 'REJECTED', 'CANCELLED'
                ) -or [string]$candidate.status -ceq 'BLOCKED') {
                throw 'PHASE4_WSL2_ACTIVE_RECONNECT_FAILED'
            }
            if ([string]$candidate.task_state -ceq 'EXECUTING' -and
                [string]$candidate.status -ceq 'RUNNING' -and
                [bool]$candidate.worker_running -and
                -not [string]::IsNullOrWhiteSpace([string]$candidate.thread_id) -and
                -not [string]::IsNullOrWhiteSpace([string]$candidate.turn_id)) {
                Assert-Phase4ActiveManagedStatus -Status $candidate -ExpectedTaskRef $taskRef
                $candidateEvidence = Get-Phase4ActiveRestartEvidence -Password $password `
                    -Port $postgresPort -Database $databaseName -TaskRef $taskRef `
                    -ProjectId $projectId -WorkingDirectory $runRoot
                if ([DateTimeOffset]::UtcNow -ge $deadline) { break }
                $value = $candidateEvidence.value
                if ([long]$value.attempt_count -eq 1 -and
                    [long]$value.attempt_number -eq 1 -and
                    [string]$value.attempt_id -ceq [string]$before.attempt_id -and
                    [long]$value.writer_fence -eq [long]$before.writer_fence -and
                    [string]$candidate.thread_id -ceq [string]$before.thread_id -and
                    [string]$candidate.turn_id -ceq [string]$before.turn_id -and
                    [string]$value.thread_id -ceq [string]$candidate.thread_id -and
                    [string]$value.turn_id -ceq [string]$candidate.turn_id -and
                    [long]$value.thread_count -eq 1 -and
                    [long]$value.turn_count -eq 1 -and
                    [long]$value.turn_started_count -eq 1 -and
                    [long]$value.reconciled_count -ge 1 -and
                    [long]$value.worker_thread_dispatch_count -eq 1 -and
                    [long]$value.worker_turn_dispatch_count -eq 1 -and
                    [string]$value.writer_status -ceq 'ACTIVE' -and
                    [string]$value.writer_attempt_id -ceq [string]$value.attempt_id -and
                    [long]$value.writer_current_fence -eq [long]$value.writer_fence -and
                    [long]$value.writer_process_id -eq [long]$wslReconnectForeman.process_id -and
                    [long]$value.process_handoff_count -ge 1) {
                    $secondActiveStatus = $candidate
                    $wslActiveRestartAfter = $candidateEvidence
                    break
                }
            }
            $remainingReconnectMilliseconds = [long][Math]::Ceiling(
                ($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
            )
            if ($remainingReconnectMilliseconds -le 0) { break }
            $reconnectNextPollAt = $reconnectPollOrigin.AddMilliseconds(
                $reconnectPollDelayMilliseconds * [double]$poll
            )
            if ($poll -ge $availableReconnectStatusCalls) {
                $reconnectNextPollAt = $deadline
            }
            $reconnectSleepMilliseconds = [long][Math]::Ceiling(
                ($reconnectNextPollAt - [DateTimeOffset]::UtcNow).TotalMilliseconds
            )
            $reconnectSleepMilliseconds = [long][Math]::Min(
                $reconnectSleepMilliseconds,
                $remainingReconnectMilliseconds
            )
            if ($reconnectSleepMilliseconds -le 0) { continue }
            Start-Sleep -Milliseconds ([int]$reconnectSleepMilliseconds)
            if ($poll -ge $availableReconnectStatusCalls) { break }
        }
        if ($null -eq $secondActiveStatus -or $null -eq $wslActiveRestartAfter) {
            throw 'PHASE4_WSL2_ACTIVE_RECONNECT_TIMEOUT'
        }
        if ([long]$secondActiveStatus.foreman_generation -ne [long]$checkpoint.generation -or
            [string]$secondActiveStatus.foreman_checkpoint_digest -cne
                [string]$checkpoint.checkpoint_digest) {
            throw 'PHASE4_WSL2_RECONNECT_FOREMAN_CHECKPOINT_REJECTED'
        }
        $after = $wslActiveRestartAfter.value
        if ([string]$after.packet_digest -cne [string]$before.packet_digest -or
            [string]$after.writer_process_start_identity -cnotmatch '\A[0-9a-f]{64}\z' -or
            [string]$after.writer_process_start_identity -ceq
                [string]$before.writer_process_start_identity) {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED'
        }
        $wslProviderSubtreeReconciliation =
            Get-Phase4WslProviderSubtreeReconciliationEvidence `
                -Password $password -Port $postgresPort -Database $databaseName `
                -TaskRef $taskRef -ProjectId $projectId -WorkingDirectory $runRoot `
                -Materialization $finalMaterialization `
                -PreflightEvidence $wslProviderPreflightEvidence `
                -OpenMarkerEvidence $wslProviderOpenMarkerEvidence `
                -ExpectedWorktreeRef (
                    'worktree:sha256:' + [string]$managedPrepare.baseline_sha256
                ) -ExpectedPacketDigest ([string]$before.packet_digest) `
                -ExpectedInitialProducerDigest (
                    [string]$before.writer_process_start_identity
                ) -ExpectedReconcilerProducerDigest (
                    [string]$after.writer_process_start_identity
                ) -ExpectedProviderEffectCount (
                    [int]$wslProviderEffectsBeforeReconciliation.value.provider_effect_count
                ) -RequireSuccessorOpen
        # Invoke-Phase4Psql creates a closed, one-shot process for every call.  This
        # second read is therefore a fresh-process exact replay, not an in-memory copy.
        $wslProviderSubtreeFreshReplay =
            Get-Phase4WslProviderSubtreeReconciliationEvidence `
                -Password $password -Port $postgresPort -Database $databaseName `
                -TaskRef $taskRef -ProjectId $projectId -WorkingDirectory $runRoot `
                -Materialization $finalMaterialization `
                -PreflightEvidence $wslProviderPreflightEvidence `
                -OpenMarkerEvidence $wslProviderOpenMarkerEvidence `
                -ExpectedWorktreeRef (
                    'worktree:sha256:' + [string]$managedPrepare.baseline_sha256
                ) -ExpectedPacketDigest ([string]$before.packet_digest) `
                -ExpectedInitialProducerDigest (
                    [string]$before.writer_process_start_identity
                ) -ExpectedReconcilerProducerDigest (
                    [string]$after.writer_process_start_identity
                ) -ExpectedProviderEffectCount (
                    [int]$wslProviderEffectsBeforeReconciliation.value.provider_effect_count
                ) -RequireSuccessorOpen
        if ([string]$wslProviderSubtreeFreshReplay.chain_binding_digest -cne
                [string]$wslProviderSubtreeReconciliation.chain_binding_digest -or
            [string]$wslProviderSubtreeFreshReplay.artifact_descriptor_digest -cne
                [string]$wslProviderSubtreeReconciliation.artifact_descriptor_digest -or
            [string]$wslProviderSubtreeFreshReplay.artifact_content_digest -cne
                [string]$wslProviderSubtreeReconciliation.artifact_content_digest -or
            [string]$wslProviderSubtreeFreshReplay.ledger_event_sequence -cne
                [string]$wslProviderSubtreeReconciliation.ledger_event_sequence -or
            [string]$wslProviderSubtreeFreshReplay.successor_preflight_artifact_ref -cne
                [string]$wslProviderSubtreeReconciliation.successor_preflight_artifact_ref -or
            [string]$wslProviderSubtreeFreshReplay.successor_marker_artifact_ref -cne
                [string]$wslProviderSubtreeReconciliation.successor_marker_artifact_ref) {
            throw 'PHASE4_WSL2_PROVIDER_SUBTREE_FRESH_REPLAY_REJECTED'
        }
        $physicalRestart = $true
    }
    $failureStage = 'MANAGED_POLL'
    $terminalWindowMilliseconds = [long]$AcceptanceTimeoutSeconds * 1000
    $terminalPollOrigin = [DateTimeOffset]::UtcNow
    $deadline = $terminalPollOrigin.AddMilliseconds($terminalWindowMilliseconds)
    $availableTerminalStatusCalls = [long]$script:MaximumMcpToolCalls -
        [long]$mcpSession.tool_call_count
    if ($Wsl2LinuxLive -and $availableTerminalStatusCalls -lt
        [long]$script:MaximumMcpStatusPolls) {
        throw 'PHASE4_WSL2_TERMINAL_STATUS_CALL_BUDGET_REJECTED'
    }
    $terminalFinalPollLeadMilliseconds = $script:MinimumMcpStatusResponseBudgetMilliseconds
    $terminalStatusResponseTimeoutSeconds = [int][Math]::Min(900, [long]$AcceptanceTimeoutSeconds)
    $terminalPollDelayMilliseconds =
        ($terminalWindowMilliseconds - $terminalFinalPollLeadMilliseconds) /
        ([double]$script:MaximumMcpStatusPolls - 1.0)
    $poll = 0
    while ($poll -lt [long]$script:MaximumMcpStatusPolls -and
        [DateTimeOffset]::UtcNow -lt $deadline) {
        $remainingTerminalMilliseconds = [long][Math]::Ceiling(
            ($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
        )
        if ($remainingTerminalMilliseconds -le 0) { break }
        $poll++
        $status = Invoke-Phase4McpStatusForGate -Session $mcpSession `
            -TaskRef $taskRef -TimeoutSeconds $terminalStatusResponseTimeoutSeconds `
            -TimeoutMilliseconds ([int][Math]::Min(
                ([long]$terminalStatusResponseTimeoutSeconds * 1000),
                $remainingTerminalMilliseconds
            )) -TimeoutCode 'PHASE4_MANAGED_TERMINAL_STATUS_RESPONSE_TIMEOUT' `
            -Stage 'MANAGED_POLL' -PollOrdinal $poll -PollOrigin $terminalPollOrigin `
            -RemainingAtDispatchMilliseconds $remainingTerminalMilliseconds `
            -LastCompletedStatus $lastManagedStatus `
            -TimeoutDiagnostic ([ref]$mcpStatusTimeoutDiagnostic)
        $terminalResponseWithinDeadline = [DateTimeOffset]::UtcNow -lt $deadline
        if ([string]$status.schema_version -ceq 'lattice.task.status.v4') {
            Assert-Phase4ManagedStatus -Status $status -ExpectedTaskRef $taskRef
            $lastManagedStatus = $status
            if (-not [string]::IsNullOrWhiteSpace([string]$status.thread_id) -and
                -not [string]::IsNullOrWhiteSpace([string]$status.turn_id)) {
                $realCodexAttempted = $true
                $realCodexAttemptEvidence = 'DURABLE_THREAD_AND_TURN_OBSERVED'
            }
            if ([string]$status.task_state -in @('BLOCKED', 'FAILED', 'REJECTED', 'CANCELLED') -or
                [string]$status.status -ceq 'BLOCKED') {
                $blocker = ConvertTo-Phase4FailureCode -Message ([string]$status.blocker)
                if ($blocker -ceq 'PHASE4_HARNESS_RUNTIME_ERROR') {
                    throw 'PHASE4_MANAGED_TASK_FAILED'
                }
                throw $blocker
            }
            if ($terminalResponseWithinDeadline -and
                [string]$status.task_state -ceq 'AWAITING_MERGE_APPROVAL') {
                $terminalStatus = $status
                break
            }
        }
        if (-not $terminalResponseWithinDeadline) { break }
        $remainingTerminalMilliseconds = [long][Math]::Ceiling(
            ($deadline - [DateTimeOffset]::UtcNow).TotalMilliseconds
        )
        if ($remainingTerminalMilliseconds -le 0) { break }
        $terminalNextPollAt = $terminalPollOrigin.AddMilliseconds(
            $terminalPollDelayMilliseconds * [double]$poll
        )
        if ($poll -ge [long]$script:MaximumMcpStatusPolls) {
            $terminalNextPollAt = $deadline
        }
        $terminalSleepMilliseconds = [long][Math]::Ceiling(
            ($terminalNextPollAt - [DateTimeOffset]::UtcNow).TotalMilliseconds
        )
        $terminalSleepMilliseconds = [long][Math]::Min(
            $terminalSleepMilliseconds,
            $remainingTerminalMilliseconds
        )
        if ($terminalSleepMilliseconds -le 0) { continue }
        Start-Sleep -Milliseconds ([int]$terminalSleepMilliseconds)
        if ($poll -ge [long]$script:MaximumMcpStatusPolls) { break }
    }
    if ($null -eq $terminalStatus) { throw 'PHASE4_MANAGED_TASK_TIMEOUT' }
    Assert-Phase4ManagedStatus -Status $terminalStatus -ExpectedTaskRef $taskRef -Terminal
    if (-not $realCodexAttempted) { throw 'CREDENTIAL_READ_ISOLATION_NOT_VERIFIED' }
    $credentialReadIsolation = if ($Wsl2LinuxLive) {
        'WSL2_LINUX_KEYRING_VERIFIED'
    }
    else {
        'VERIFIED'
    }
    if ([long]$terminalStatus.foreman_generation -ne [long]$checkpoint.generation -or
        [string]$terminalStatus.foreman_checkpoint_digest -cne
        [string]$checkpoint.checkpoint_digest) {
        throw 'PHASE4_FOREMAN_CHECKPOINT_BINDING_REJECTED'
    }

    $failureStage = 'DURABLE_EVIDENCE'
    $databaseBefore = Get-Phase4DatabaseEvidence -Password $password -Port $postgresPort `
        -Database $databaseName -TaskRef $taskRef -WorkingDirectory $runRoot
    $gitEvidence = Get-Phase4GitSnapshotEvidence -Password $password -Port $postgresPort `
        -Database $databaseName -TaskRef $taskRef -WorkingDirectory $runRoot
    $baselineEvidence = Get-Phase4ManagedBaselineEvidence -Password $password -Port $postgresPort `
        -Database $databaseName -TaskRef $taskRef -WorkingDirectory $runRoot
    $resourceBudgetBefore = Assert-Phase4DatabaseEvidence -Evidence $databaseBefore `
        -Status $terminalStatus `
        -GitEvidence $gitEvidence
    if ($Wsl2LinuxLive) {
        $wslEnvironmentBefore = Get-Phase4WslDurableEvidence -Password $password `
            -Port $postgresPort -Database $databaseName -TaskRef $taskRef `
            -WorkingDirectory $runRoot
        Assert-Phase4WslExecutionEnvironmentEvidence -Evidence $wslEnvironmentBefore `
            -Materialization $finalMaterialization -Repository $expectedManagedLinuxPath `
            -RepositoryHead $baseCommit -RequireReconciled
        $wslReviewerSubtreeEvidence = Get-Phase4WslReviewerSubtreeEvidence `
            -Password $password -Port $postgresPort -Database $databaseName `
            -TaskRef $taskRef -ProjectId $projectId -WorkingDirectory $runRoot `
            -Materialization $finalMaterialization `
            -ExpectedWorktreeRef (
                'worktree:sha256:' + [string]$managedPrepare.baseline_sha256
            ) -ExpectedModelCallIdentity (
                [string]$resourceBudgetBefore.reviewer_model_call_identity
            ) -ExpectedProducerDigest (
                [string]$wslActiveRestartAfter.value.writer_process_start_identity
            ) -ExpectedProviderEffectCount (
                [int]$wslEnvironmentBefore.value.review_thread_dispatch_count +
                [int]$wslEnvironmentBefore.value.review_turn_dispatch_count
            )
    }
    if ([string]$baselineEvidence.base_commit -cne [string]$gitEvidence.base_commit) {
        throw 'PHASE4_BASELINE_RESULT_LINEAGE_REJECTED'
    }
    $managedWorkerRoot = Assert-Phase4ContainedPath -Root $managedWorktreeRoot `
        -Path (Join-Path $managedWorktreeRoot ('work-' + $taskRef.Substring(0, 59))) `
        -Failure 'PHASE4_MANAGED_WORKTREE_CONTAINMENT_REJECTED'
    Assert-Phase4Directory -Path $managedWorkerRoot -Failure 'PHASE4_MANAGED_WORKTREE_MISSING'
    $proofPath = Join-Path $managedWorkerRoot 'phase4-proof.txt'
    Assert-Phase4RegularFile -Path $proofPath -Failure 'PHASE4_PROOF_FILE_MISSING'
    if ([IO.File]::ReadAllText($proofPath, $script:Utf8) -cne
        "LATTICE_PHASE4_MANAGED_FOREMAN_OK`n" -or
        @($gitEvidence.changed_paths).Count -ne 1 -or
        [string]$gitEvidence.changed_paths[0] -cne 'phase4-proof.txt') {
        throw 'PHASE4_PROOF_SCOPE_REJECTED'
    }
    if ((Test-Path -LiteralPath (Join-Path $projectRoot 'phase4-proof.txt')) -or
        (Test-Path -LiteralPath (Join-Path $managedWorkerRoot 'registry-only.credentials')) -or
        -not (Test-Path -LiteralPath (Join-Path $projectRoot 'registry-only.credentials') -PathType Leaf)) {
        throw 'PHASE4_MANAGED_WORKTREE_ISOLATION_REJECTED'
    }
    $sourceHead = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        '-C', $projectRoot, 'rev-parse', '--verify', 'HEAD^{commit}'
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_SOURCE_HEAD_READ_FAILED'
    $sourceStatus = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        '-C', $projectRoot, 'status', '--porcelain=v1', '--untracked-files=all'
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_SOURCE_STATUS_READ_FAILED'
    if ([string]$sourceHead.stdout.Trim() -cne [string]$gitEvidence.base_commit -or
        -not [string]::IsNullOrWhiteSpace([string]$sourceStatus.stdout)) {
        throw 'PHASE4_SOURCE_CHECKOUT_MUTATED'
    }
    $protectedRef = 'refs/lattice/managed/' + $taskRef + '/attempt-1'
    $protected = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        '-C', $projectRoot, 'show-ref', '--verify', '--hash', $protectedRef
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_PROTECTED_REF_READ_FAILED'
    $protectedProof = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        '-C', $projectRoot, 'show', ($protectedRef + ':phase4-proof.txt')
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_PROTECTED_REF_CONTENT_FAILED'
    if ([string]$protected.stdout.Trim() -cne [string]$gitEvidence.result_commit -or
        [string]$protectedProof.stdout -cne "LATTICE_PHASE4_MANAGED_FOREMAN_OK`n") {
        throw 'PHASE4_PROTECTED_REF_REJECTED'
    }
    $gitControlAfter = Get-Phase4GitControlEvidence -Git $git -Repository $projectRoot `
        -Environment $gitEnvironment -WorkingDirectory $runRoot
    $systemIdentifierBefore = Invoke-Phase4Psql -Password $password -Port $postgresPort `
        -Database 'postgres' -Sql 'SELECT system_identifier::text FROM pg_catalog.pg_control_system();' `
        -WorkingDirectory $runRoot -Failure 'PHASE4_POSTGRES_IDENTITY_READ_FAILED'
    if ($systemIdentifierBefore -cnotmatch '\A[1-9][0-9]+\z') {
        throw 'PHASE4_POSTGRES_IDENTITY_REJECTED'
    }

    $failureStage = 'FRESH_RESTART'
    $liveMcpRecord = Stop-Phase4McpSession -Session $mcpSession
    $mcpRecords.Add($liveMcpRecord)
    $mcpSession = $null
    $null = Stop-Phase4Control -Session $controlSession
    $controlSession = $null
    Stop-Phase4Postgres -RunRoot $runRoot -RunId $runId -Port $postgresPort `
        -DataRoot $dataRoot -MarkerPath $markerPath
    $postgresRunning = $false
    if (@(Get-Phase4ListenerPids -Port $postgresPort).Count -ne 0 -or
        @(Get-Phase4ListenerPids -Port $controlPort).Count -ne 0) {
        throw 'PHASE4_PRE_RESTART_STOP_PROOF_REJECTED'
    }

    $secondPostgres = Start-Phase4Postgres -RunRoot $runRoot -RunId $runId -Port $postgresPort `
        -DataRoot $dataRoot -MarkerPath $markerPath
    $postgresRunning = $true
    if ([long]$secondPostgres.process_start_utc_ticks -eq [long]$firstPostgres.process_start_utc_ticks) {
        throw 'PHASE4_POSTGRES_PHYSICAL_RESTART_REJECTED'
    }
    $systemIdentifierAfter = Invoke-Phase4Psql -Password $password -Port $postgresPort `
        -Database 'postgres' -Sql 'SELECT system_identifier::text FROM pg_catalog.pg_control_system();' `
        -WorkingDirectory $runRoot -Failure 'PHASE4_POSTGRES_IDENTITY_READ_FAILED'
    if ($systemIdentifierAfter -cne $systemIdentifierBefore) {
        throw 'PHASE4_POSTGRES_REPLAY_IDENTITY_REJECTED'
    }
    $controlSession = Start-Phase4Control -NodeExecutable $node -ControlHome $controlHome `
        -Port $controlPort
    $secondControl = [pscustomobject][ordered]@{
        process_id = [int]$controlSession.process_id
        process_start_utc_ticks = [long]$controlSession.process_start_utc_ticks
    }
    if ([long]$secondControl.process_start_utc_ticks -eq [long]$firstControl.process_start_utc_ticks) {
        throw 'PHASE4_CONTROL_PHYSICAL_RESTART_REJECTED'
    }
    $replayedProject = Invoke-Phase4ControlJson -Port $controlPort -Method GET `
        -Path ('/api/projects/' + [Uri]::EscapeDataString($projectId)) -Body $null `
        -Failure 'PHASE4_CONTROL_PROJECT_REPLAY_FAILED'
    $projectRestartDigest = Get-Phase4ControlProjectDigest -Project $replayedProject
    if ($projectRestartDigest -cne $projectProjectionDigest) {
        throw 'PHASE4_CONTROL_PROJECT_REPLAY_CHANGED'
    }

    $mcpSession = Start-Phase4McpSession -Name 'managed-restart' -Latticed $latticed `
        -Environment $activeEnvironment -TimeoutSeconds $ProcessTimeoutSeconds
    $secondForeman = [pscustomobject][ordered]@{
        process_id = [int]$mcpSession.process_id
        process_start_utc_ticks = [long]$mcpSession.process_start_utc_ticks
    }
    if ([long]$secondForeman.process_start_utc_ticks -eq [long]$firstForeman.process_start_utc_ticks) {
        throw 'PHASE4_FOREMAN_PHYSICAL_RESTART_REJECTED'
    }
    $restartStatus = Invoke-Phase4McpTool -Session $mcpSession -ToolName 'lattice_task_status' `
        -Arguments (New-Phase4GeneralTaskStatusArguments -TaskRef $taskRef) `
        -TimeoutSeconds $ProcessTimeoutSeconds
    Assert-Phase4ManagedStatus -Status $restartStatus -ExpectedTaskRef $taskRef -Terminal
    $databaseAfter = Get-Phase4DatabaseEvidence -Password $password -Port $postgresPort `
        -Database $databaseName -TaskRef $taskRef -WorkingDirectory $runRoot
    $resourceBudgetAfter = Assert-Phase4DatabaseEvidence -Evidence $databaseAfter `
        -Status $restartStatus `
        -GitEvidence $gitEvidence
    if ($Wsl2LinuxLive) {
        $wslEnvironmentAfter = Get-Phase4WslDurableEvidence -Password $password `
            -Port $postgresPort -Database $databaseName -TaskRef $taskRef `
            -WorkingDirectory $runRoot
        Assert-Phase4WslExecutionEnvironmentEvidence -Evidence $wslEnvironmentAfter `
            -Materialization $finalMaterialization -Repository $expectedManagedLinuxPath `
            -RepositoryHead $baseCommit -RequireReconciled
    }
    $baselineAfter = Get-Phase4ManagedBaselineEvidence -Password $password -Port $postgresPort `
        -Database $databaseName -TaskRef $taskRef -WorkingDirectory $runRoot
    $protectedAfter = Invoke-Phase4RepositoryGit -Git $git -Argument @(
        '-C', $projectRoot, 'show-ref', '--verify', '--hash', $protectedRef
    ) -Environment $gitEnvironment -WorkingDirectory $runRoot `
        -Failure 'PHASE4_PROTECTED_REF_REPLAY_FAILED'
    $gitControlRestart = Get-Phase4GitControlEvidence -Git $git -Repository $projectRoot `
        -Environment $gitEnvironment -WorkingDirectory $runRoot
    $gitControlProof = Assert-Phase4GitControlEvidence `
        -Before $gitControlBefore -After $gitControlAfter -Restart $gitControlRestart `
        -TaskRef $taskRef -BaseCommit ([string]$gitEvidence.base_commit) `
        -ResultCommit ([string]$gitEvidence.result_commit)
    $statusBeforeDigest = Get-Phase4StringSha256 -Value (
        $terminalStatus | ConvertTo-Json -Compress -Depth 30
    )
    $statusAfterDigest = Get-Phase4StringSha256 -Value (
        $restartStatus | ConvertTo-Json -Compress -Depth 30
    )
    if ($databaseAfter.digest -cne $databaseBefore.digest -or
        [string]$databaseAfter.owner_replay_digest -cne
        [string]$databaseBefore.owner_replay_digest -or
        [string]$resourceBudgetAfter.evidence_digest -cne
        [string]$resourceBudgetBefore.evidence_digest -or
        ($Wsl2LinuxLive -and [string]$wslEnvironmentAfter.digest -cne
            [string]$wslEnvironmentBefore.digest) -or
        $statusAfterDigest -cne $statusBeforeDigest -or
        [string]$databaseAfter.value.attempt_id -cne [string]$databaseBefore.value.attempt_id -or
        [string]$databaseAfter.value.thread_id -cne [string]$databaseBefore.value.thread_id -or
        [string]$databaseAfter.value.turn_id -cne [string]$databaseBefore.value.turn_id -or
        (Get-Phase4StringSha256 -Value ($baselineAfter | ConvertTo-Json -Compress -Depth 20)) -cne
        (Get-Phase4StringSha256 -Value ($baselineEvidence | ConvertTo-Json -Compress -Depth 20)) -or
        [string]$protectedAfter.stdout.Trim() -cne [string]$gitEvidence.result_commit) {
        throw 'PHASE4_RESTART_REPLAY_CHANGED'
    }
    $noDuplicateAgent = (
        [long]$databaseBefore.value.attempt_count -eq 1 -and
        [long]$databaseBefore.value.thread_count -eq 1 -and
        [long]$databaseBefore.value.turn_count -eq 1 -and
        [long]$databaseAfter.value.attempt_count -eq 1 -and
        [long]$databaseAfter.value.thread_count -eq 1 -and
        [long]$databaseAfter.value.turn_count -eq 1
    )
    if (-not $noDuplicateAgent) { throw 'PHASE4_DUPLICATE_AGENT_OBSERVED' }
    $restartMcpRecord = Stop-Phase4McpSession -Session $mcpSession
    $mcpRecords.Add($restartMcpRecord)
    $mcpSession = $null
    $physicalRestart = $true
    }
}
catch {
    if ($wslTechnicalPreflightComplete -and
        [string]$_.Exception.Message -ceq 'PHASE4_WSL2_TECHNICAL_PREFLIGHT_COMPLETE') {
        $failureStage = 'WSL2_TECHNICAL_PREFLIGHT_COMPLETE'
    }
    else {
        $failureLine = [int]$_.InvocationInfo.ScriptLineNumber
        $failureException = $_.Exception.GetType().Name
        $temporaryFailureDiagnostic = [regex]::Replace(
            [string]$_.Exception.Message,
            '(?i)(password|token|secret)=([^\s;]+)',
            '$1=[redacted]'
        )
        [Console]::Error.WriteLine('LATTICE_HARNESS_EXCEPTION:' + $temporaryFailureDiagnostic)
        $failureCode = ConvertTo-Phase4FailureCode -Message $_.Exception.Message
        if ($failureCode -ceq 'PHASE4_HARNESS_RUNTIME_ERROR') {
            $failureCode = 'PHASE4_STAGE_' + $failureStage + '_FAILED'
        }
    }
}
finally {
    if ($null -ne $mcpSession) {
        $stoppingSession = $mcpSession
        try {
            $stoppedSession = Stop-Phase4McpSession -Session $stoppingSession -SuppressFailure
            if ($null -ne $stoppedSession) {
                if (-not [bool]$stoppedSession.job_empty) {
                    throw 'PHASE4_LATTICED_STOP_PROOF_REJECTED'
                }
                $mcpRecords.Add($stoppedSession)
            }
            else {
                $null = Assert-Phase4ProcessIdentityAbsent `
                    -ProcessId ([long]$stoppingSession.process_id) `
                    -ProcessStartUtcTicks ([long]$stoppingSession.process_start_utc_ticks)
            }
        }
        catch {
            $foremanProcessTreeStopped = $false
            if ($null -eq $failureCode) { $failureCode = 'PHASE4_FOREMAN_TREE_CLEANUP_REJECTED' }
        }
        $mcpSession = $null
    }
    if ($null -ne $controlSession) {
        try {
            $controlCleanup = Stop-Phase4Control -Session $controlSession -SuppressFailure
            if ($null -eq $controlCleanup -or -not [bool]$controlCleanup.job_empty -or
                -not [bool]$controlCleanup.listener_absent) {
                throw 'PHASE4_CONTROL_STOP_PROOF_REJECTED'
            }
        }
        catch {
            $foremanProcessTreeStopped = $false
            if ($null -eq $failureCode) { $failureCode = 'PHASE4_FOREMAN_TREE_CLEANUP_REJECTED' }
        }
        $controlSession = $null
    }
    if ($Wsl2LinuxLive) {
        try { Close-Phase4WslOpenCommandUnits }
        catch {
            $foremanProcessTreeStopped = $false
            if ($null -eq $failureCode) {
                $failureCode = 'PHASE4_WSL2_COMMAND_UNIT_CLEANUP_REJECTED'
                $failureStage = 'WSL2_COMMAND_UNIT_CLEANUP'
            }
        }
    }
    $wslCleanupMaterialization = if ($null -ne $finalMaterialization) {
        $finalMaterialization
    }
    else { $bootstrapMaterialization }
    if ($Wsl2LinuxLive -and $taskRef -cmatch '\A[0-9a-f]{64}\z' -and
        $null -ne $finalMaterialization -and $null -ne $node -and
        $null -ne $password -and $null -ne $postgresPort -and
        @(Get-Phase4ListenerPids -Port $postgresPort).Count -ne 0) {
        try {
            $null = Assert-Phase4OwnedLivePostgres -RunRoot $runRoot -RunId $runId `
                -Port $postgresPort -DataRoot $dataRoot -MarkerPath $markerPath
            $wslFinalDurableEvidenceBeforeCleanup = Get-Phase4WslDurableEvidence `
                -Password $password -Port $postgresPort -Database $databaseName `
                -TaskRef $taskRef -WorkingDirectory $runRoot
            $finalBefore = $wslFinalDurableEvidenceBeforeCleanup.value
            if ([long]$finalBefore.provider_effect_count -gt 0 -or
                [long]$finalBefore.thread_count -gt 0 -or [long]$finalBefore.turn_count -gt 0) {
                $realCodexAttempted = $true
                $realCodexAttemptEvidence = 'FINAL_DURABLE_EFFECT_OR_IDENTITY_OBSERVED'
            }
            $wslFailureSubtreeCleanup = Invoke-Phase4WslFailureSubtreeCleanup `
                -Password $password -Port $postgresPort -Database $databaseName `
                -TaskRef $taskRef -ProjectId $projectId -WorkingDirectory $runRoot `
                -Materialization $finalMaterialization -NodeExecutable $node `
                -ProviderEffectCount ([int]$finalBefore.provider_effect_count)
            if ([bool]$wslFailureSubtreeCleanup.reconciliation_required -or
                -not [bool]$wslFailureSubtreeCleanup.gate_passed) {
                throw 'PHASE4_WSL2_FAILURE_SUBTREE_RECONCILIATION_REQUIRED'
            }
            $wslFinalDurableEvidence = Get-Phase4WslDurableEvidence `
                -Password $password -Port $postgresPort -Database $databaseName `
                -TaskRef $taskRef -WorkingDirectory $runRoot
            Assert-Phase4WslProviderEffectsUnchanged `
                -Before $wslFinalDurableEvidenceBeforeCleanup.value `
                -After $wslFinalDurableEvidence.value
            $wslDurableProviderEffectStatus = 'OBSERVED_FINAL'
        }
        catch {
            $wslFailureSubtreeCleanupCode = ConvertTo-Phase4FailureCode `
                -Message ([string]$_.Exception.Message)
            if ($wslFailureSubtreeCleanupCode -ceq 'PHASE4_HARNESS_RUNTIME_ERROR') {
                $wslFailureSubtreeCleanupCode =
                    'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_REJECTED'
            }
            $wslDurableProviderEffectStatus = 'UNKNOWN_RECONCILIATION_REQUIRED'
            $wslReconciliationRequired = $true
            try {
                $taskUnitCleanup = Close-Phase4WslTaskOwnedUnits `
                    -TaskRef $taskRef -Materialization $finalMaterialization
                if ($null -eq $wslFailureSubtreeCleanup) {
                    $wslFailureSubtreeCleanup = $taskUnitCleanup
                }
            }
            catch {
                $wslFailureSubtreeCleanupCode = 'PHASE4_WSL2_FAILURE_SUBTREE_RECONCILIATION_REQUIRED'
            }
            if ($null -eq $wslFinalDurableEvidence) {
                try {
                    $null = Assert-Phase4OwnedLivePostgres `
                        -RunRoot $runRoot -RunId $runId -Port $postgresPort `
                        -DataRoot $dataRoot -MarkerPath $markerPath
                    $wslFinalDurableEvidence = Get-Phase4WslDurableEvidence `
                        -Password $password -Port $postgresPort -Database $databaseName `
                        -TaskRef $taskRef -WorkingDirectory $runRoot
                }
                catch {}
            }
            if ($null -eq $failureCode) {
                $failureCode = $wslFailureSubtreeCleanupCode
                $failureStage = 'WSL2_FAILURE_CLEANUP'
            }
        }
    }
    elseif ($Wsl2LinuxLive -and $taskRef -cmatch '\A[0-9a-f]{64}\z' -and
        $null -ne $wslCleanupMaterialization) {
        $wslDurableProviderEffectStatus = 'UNKNOWN_RECONCILIATION_REQUIRED'
        $wslReconciliationRequired = $true
        $wslFailureSubtreeCleanupCode = 'PHASE4_WSL2_FAILURE_SUBTREE_CLEANUP_UNAVAILABLE'
        try {
            $wslFailureSubtreeCleanup = Close-Phase4WslTaskOwnedUnits `
                -TaskRef $taskRef -Materialization $wslCleanupMaterialization
        }
        catch {
            $wslFailureSubtreeCleanupCode =
                'PHASE4_WSL2_FAILURE_SUBTREE_RECONCILIATION_REQUIRED'
        }
        if ($null -eq $failureCode) {
            $failureCode = $wslFailureSubtreeCleanupCode
            $failureStage = 'WSL2_FAILURE_CLEANUP'
        }
    }
    if ($runRootCreated -and $null -ne $postgresPort -and
        $script:postgresStartMayOwnProcess) {
        try {
            Stop-Phase4Postgres -RunRoot $runRoot -RunId $runId -Port $postgresPort `
                -DataRoot $dataRoot -MarkerPath $markerPath
            $postgresRunning = $false
        }
        catch {
            if ($null -eq $failureCode) {
                $failureCode = ConvertTo-Phase4FailureCode -Message $_.Exception.Message
            }
            if ($null -ne $script:postgresOwnedProcessJob) {
                try { Close-Phase4PostgresOwnedJob -TerminateRemaining }
                catch {
                    if ($null -eq $failureCode) {
                        $failureCode = 'PHASE4_POSTGRES_STOP_PROOF_REJECTED'
                    }
                }
            }
        }
    }
    if ($runRootCreated -and (Test-Path -LiteralPath $passwordPath -PathType Leaf)) {
        try {
            $safePasswordPath = Assert-Phase4ContainedPath -Root $runRoot -Path $passwordPath `
                -Failure 'PHASE4_PASSWORD_PATH_REJECTED'
            Remove-Item -LiteralPath $safePasswordPath -Force
        }
        catch {
            if ($null -eq $failureCode) { $failureCode = 'PHASE4_PASSWORD_FILE_CLEANUP_REJECTED' }
        }
    }
    if ($null -ne $postgresPort -and $null -ne $controlPort) {
        $listenerCleanup = @(Get-Phase4ListenerPids -Port $postgresPort).Count -eq 0 -and
            @(Get-Phase4ListenerPids -Port $controlPort).Count -eq 0
        if (-not $listenerCleanup -and $null -eq $failureCode) {
            $failureCode = 'PHASE4_CLEANUP_LISTENER_SURVIVED'
        }
    }
    if ($runRootCreated -and $null -eq $failureCode -and -not $KeepArtifacts) {
        try {
            $deleteTarget = [IO.Path]::GetFullPath($runRoot)
            $null = Get-Phase4OwnerMarker -RunRoot $deleteTarget -RunId $runId `
                -MarkerPath $markerPath
            if ($deleteTarget -cne $expectedRunRoot -or
                -not $deleteTarget.StartsWith(
                    $tempParent + [IO.Path]::DirectorySeparatorChar,
                    [StringComparison]::OrdinalIgnoreCase
                ) -or [IO.Path]::GetFileName($deleteTarget) -cne $runRootName) {
                throw 'PHASE4_DELETE_TARGET_REJECTED'
            }
            Remove-Item -LiteralPath $deleteTarget -Recurse -Force
            if (Test-Path -LiteralPath $deleteTarget) { throw 'PHASE4_DELETE_INCOMPLETE' }
            $cleanupSucceeded = $true
        }
        catch {
            if ($null -eq $failureCode) {
                $failureCode = ConvertTo-Phase4FailureCode -Message $_.Exception.Message
            }
        }
    }
    elseif ($runRootCreated -and (Test-Path -LiteralPath $runRoot -PathType Container)) {
        $cleanupSucceeded = $false
    }
    if ($ScriptedActiveRestart -and $null -ne $scriptedFixture -and
        $null -eq $failureCode -and -not $KeepArtifacts) {
        try {
            Remove-Phase4ManagedScriptedFixture -Fixture $scriptedFixture -FixtureId $runId
            $scriptedFixtureCleanup = $true
        }
        catch {
            $failureCode = ConvertTo-Phase4FailureCode -Message $_.Exception.Message
        }
    }
    if ($Wsl2LinuxLive) {
        try { Close-Phase4WslOpenCommandUnits }
        catch {
            $foremanProcessTreeStopped = $false
            if ($null -eq $failureCode) {
                $failureCode = 'PHASE4_WSL2_COMMAND_UNIT_CLEANUP_REJECTED'
                $failureStage = 'WSL2_COMMAND_UNIT_CLEANUP'
            }
        }
    }
}

$elapsed = ([DateTimeOffset]::UtcNow - $startedAt).TotalMilliseconds
if ($Wsl2TechnicalPreflightOnly -and $wslTechnicalPreflightComplete -and
    $null -eq $failureCode -and $null -ne $taskRef -and
    $null -ne $bootstrapMaterialization -and $null -ne $finalMaterialization -and
    $null -ne $managedPrepare -and $null -ne $managedVerify -and
    $null -ne $wslSubstitutionEvidence -and
    $null -ne $wslZeroProviderEvidence -and
    $null -ne $wslZeroProviderAfterMaterialization -and
    $null -ne $wslZeroProviderAfterCheckpoint -and $null -ne $checkpoint -and
    $null -ne $draftStatus -and $null -ne $freshDraftStatus -and
    $listenerCleanup) {
    [ordered]@{
        schema = 'lattice.phase4-managed-foreman.wsl2-technical-preflight.v1'
        status = 'PASS'
        acceptance = $false
        technical_preflight = $true
        real_codex = $false
        real_codex_attempted = $false
        provider_effect_count = 0
        elapsed_ms = [long]$elapsed
        task_ref = $taskRef
        durable_task_state = 'DRAFT'
        draft_reconstruction = [ordered]@{
            initial_status_digest = Get-Phase4StringSha256 -Value (
                $draftStatus | ConvertTo-Json -Compress -Depth 30
            )
            fresh_process_status_digest = Get-Phase4StringSha256 -Value (
                $freshDraftStatus | ConvertTo-Json -Compress -Depth 30
            )
            ledger_head_digest = [string]$freshDraftStatus.ledger_head_digest
            exact = $true
        }
        source = [ordered]@{
            linux_path = $wslSourceRepository
            windows_path = $projectRoot
            repository_head = $baseCommit
            remote_count = [long]$gitControlBefore.remote_count
            clean = [bool]$gitControlBefore.status_clean
        }
        managed_worktree = [ordered]@{
            linux_path = $expectedManagedLinuxPath
            windows_path = $expectedManagedUncPath
            worktree_id = [string]$managedPrepare.worktree_id
            baseline_sha256 = [string]$managedPrepare.baseline_sha256
            prepare_replayed = [bool]$managedPrepare.replayed
            verify_replayed = [bool]$managedVerify.replayed
            prepare_process_evidence = $managedPrepare.harness_process_evidence
            verify_process_evidence = $managedVerify.harness_process_evidence
        }
        bootstrap = [ordered]@{
            persisted_as_attempt_environment = $false
            execution_environment_ref =
                [string]$bootstrapMaterialization.record.execution_environment_ref
            provider_effect_count = [long]$bootstrapMaterialization.record.provider_effect_count
        }
        execution_environment = [ordered]@{
            descriptor_schema = [string]$finalMaterialization.descriptor.schema
            execution_environment_ref =
                [string]$finalMaterialization.record.execution_environment_ref
            execution_domain_digest =
                [string]$finalMaterialization.record.execution_environment_ref
            distribution = [string]$finalMaterialization.descriptor.distribution
            distribution_identity_ref =
                [string]$finalMaterialization.descriptor.distribution_identity.identity_digest
            repository_identity_ref =
                [string]$finalMaterialization.descriptor.linux.repository_identity
            path_mapping_ref = [string]$finalMaterialization.descriptor.path_mapping.digest
            credential_authority_kind =
                [string]$finalMaterialization.descriptor.credential_authority.kind
            credential_authority_ref =
                [string]$finalMaterialization.descriptor.credential_authority.authority_digest
            process_fence_ref =
                [string]$finalMaterialization.descriptor.process_fence.identity_digest
            preflight_process_fence = [string]$finalMaterialization.process_fence
            verification_toolchain_ref =
                [string]$finalMaterialization.descriptor.verification_toolchain.identity_digest
            descriptor_path = [string]$finalMaterialization.descriptor_path
            evidence_directory = [string]$finalMaterialization.evidence_directory
        }
        toolchain = [ordered]@{
            launcher = [ordered]@{
                path = [string]$finalMaterialization.descriptor.linux.launcher_path
                version = [string]$finalMaterialization.descriptor.linux.launcher_version
                sha256 = [string]$finalMaterialization.descriptor.linux.launcher_sha256
            }
            node = $finalMaterialization.descriptor.verification_toolchain.npm
            cargo = $finalMaterialization.descriptor.verification_toolchain.cargo
            rustc = $finalMaterialization.descriptor.verification_toolchain.rustc
            git = [ordered]@{
                path = [string]$finalMaterialization.descriptor.linux.git_path
                version = [string]$finalMaterialization.descriptor.linux.git_version
                sha256 = [string]$finalMaterialization.descriptor.linux.git_sha256
            }
            supervisor = [ordered]@{
                path = [string]$finalMaterialization.descriptor.linux.supervisor_path
                sha256 = [string]$finalMaterialization.descriptor.linux.supervisor_sha256
            }
        }
        zero_model_preflight = [ordered]@{
            status = [string]$finalMaterialization.preflight.status
            connector_auth_ready = [bool]$finalMaterialization.preflight.connector_auth_ready
            provider_effect_count =
                [long]$finalMaterialization.preflight.effect_counters.provider_effect_count
            thread_start = [long]$finalMaterialization.preflight.effect_counters.thread_start
            turn_start = [long]$finalMaterialization.preflight.effect_counters.turn_start
            process_fence = [ordered]@{
                unit = [string]$finalMaterialization.preflight.process_fence.service_unit
                cgroup_path = [string]$finalMaterialization.preflight.process_fence.cgroup_path
                boot_id_digest =
                    [string]$finalMaterialization.preflight.process_fence.boot_id_digest
                unit_inactive =
                    ([string]$finalMaterialization.preflight.process_fence.outer_post_exit.active_state -ceq
                        'inactive')
                unit_dead =
                    ([string]$finalMaterialization.preflight.process_fence.outer_post_exit.sub_state -ceq
                        'dead')
                supervisor_zero_descendants =
                    [bool]$finalMaterialization.preflight.process_fence.supervisor_zero_descendants
                cgroup_closed = $(
                    $outer = $finalMaterialization.preflight.process_fence.outer_post_exit
                    ((-not [bool]$outer.cgroup_exists -and $null -eq $outer.populated) -or
                        ([bool]$outer.cgroup_exists -and $null -ne $outer.populated -and
                            [long]$outer.populated -eq 0))
                )
            }
            evidence_path = [string]$finalMaterialization.preflight_path
            materializer_process_evidence = $finalMaterialization.process_evidence
            durable_before_digest = [string]$wslZeroProviderEvidence.digest
            durable_after_digest = [string]$wslZeroProviderAfterMaterialization.digest
            durable_after_checkpoint_digest =
                [string]$wslZeroProviderAfterCheckpoint.digest
            durable_provider_effect_count =
                [long]$wslZeroProviderAfterCheckpoint.value.provider_effect_count
            durable_artifact_outbox_count =
                [long]$wslZeroProviderAfterCheckpoint.value.artifact_outbox_count
            durable_pending_worker_claim_count =
                [long]$wslZeroProviderAfterCheckpoint.value.pending_worker_claim_count
        }
        foreman_checkpoint = [ordered]@{
            status = [string]$checkpoint.status
            generation = [long]$checkpoint.generation
            checkpoint_digest = [string]$checkpoint.checkpoint_digest
            observed_linux_worktree = $expectedManagedLinuxPath
        }
        substitution_gates = $wslSubstitutionEvidence
        security = [ordered]@{
            credential_read_isolation = $credentialReadIsolation
            auth_read = $false
            lattice_owned_auth_copy = 'NOT_CREATED'
            source_under_linux_home = $true
            managed_worktree_under_linux_home = $true
            windows_mount_repository = $false
        }
        cleanup = [ordered]@{
            postgres_stopped = (-not $postgresRunning -and
                -not $script:postgresStartMayOwnProcess)
            listeners_absent = $listenerCleanup
            windows_temporary_root_removed = $cleanupSucceeded
            windows_artifacts_retained = (-not $cleanupSucceeded)
            wsl_task_assets_retained = $true
            windows_evidence_root = $(if ($cleanupSucceeded) { $null } else { $runRoot })
        }
    } | ConvertTo-Json -Compress -Depth 30
    return
}
if ($ScriptedActiveRestart -and $null -eq $failureCode -and
    $null -ne $firstActiveStatus -and $null -ne $secondActiveStatus -and
    $null -ne $activeBeforeRestart -and $null -ne $activeAfterRestart -and
    $null -ne $firstScriptedServer -and $null -ne $secondScriptedServer -and
    $null -ne $probeScriptedServer -and $null -ne $probeServerAbsence -and
    $null -ne $firstServerAbsence -and $null -ne $secondServerAbsence -and
    $physicalRestart -and $noDuplicateAgent -and $listenerCleanup -and
    ($KeepArtifacts -or $scriptedFixtureCleanup)) {
    [ordered]@{
        schema = 'lattice.phase4-managed-foreman.scripted-active-restart.v1'
        status = 'PASS'
        acceptance = $true
        real_codex = $false
        real_codex_attempted = $false
        scripted_app_server = $true
        scripted_app_server_attempted = $true
        elapsed_ms = [long]$elapsed
        task_ref = [string]$firstActiveStatus.task_ref
        task_state_at_proof = 'EXECUTING'
        worker = [ordered]@{
            thread_id = [string]$firstActiveStatus.thread_id
            turn_id = [string]$firstActiveStatus.turn_id
            attempt = [int]$firstActiveStatus.attempt
            retry_count = [int]$firstActiveStatus.retry_count
            model = [string]$firstActiveStatus.model
            model_reason = [string]$activeBeforeRestart.value.model_reason
            exact_turn_started = $true
            reconciled_active = $true
        }
        writer = [ordered]@{
            transition = 'PROCESS_HANDOFF'
            process_handoff_count = [long]$activeAfterRestart.value.process_handoff_count
            attempt_id = [string]$activeAfterRestart.value.attempt_id
            fence = [long]$activeAfterRestart.value.writer_fence
            first_process_id = [long]$firstForeman.process_id
            second_process_id = [long]$secondForeman.process_id
            first_process_start_identity = [string]$activeBeforeRestart.value.writer_process_start_identity
            second_process_start_identity = [string]$activeAfterRestart.value.writer_process_start_identity
        }
        foreman = [ordered]@{
            checkpoint_generation = [long]$checkpoint.generation
            checkpoint_digest = [string]$checkpoint.checkpoint_digest
            generation_a = [long]$firstActiveStatus.foreman_generation
            checkpoint_digest_a = [string]$firstActiveStatus.foreman_checkpoint_digest
            generation_b = [long]$secondActiveStatus.foreman_generation
            checkpoint_digest_b = [string]$secondActiveStatus.foreman_checkpoint_digest
        }
        replay = [ordered]@{
            attempt_count = [long]$activeAfterRestart.value.attempt_count
            worker_thread_dispatch_count = [long]$activeAfterRestart.value.worker_thread_dispatch_count
            worker_turn_dispatch_count = [long]$activeAfterRestart.value.worker_turn_dispatch_count
            turn_started_count = [long]$activeAfterRestart.value.turn_started_count
            reconciled_count = [long]$activeAfterRestart.value.reconciled_count
            terminal_count_at_proof = [long]$activeAfterRestart.value.terminal_count
            database_before_digest = [string]$activeBeforeRestart.digest
            database_after_digest = [string]$activeAfterRestart.digest
            scripted_event_digest = [string]$scriptedEventEvidence.digest
            provider_thread_start_count = [long]$scriptedEventEvidence.thread_start_count
            provider_turn_start_count = [long]$scriptedEventEvidence.turn_start_count
            provider_thread_resume_count = [long]$scriptedEventEvidence.thread_resume_count
            provider_thread_read_count = [long]$scriptedEventEvidence.thread_read_count
            exact_interrupt_cleanup_count = [long]$scriptedEventEvidence.turn_interrupt_count
            exact_terminal_ack_count = [long]$scriptedEventEvidence.terminal_ack_count
            server_exit_count = [long]$scriptedEventEvidence.server_exit_count
            server_generation_count = [long]$scriptedEventEvidence.generation_count
            server_generation_digest = [string]$scriptedEventEvidence.generation_digest
            generation_a_server_identity = [string]$firstScriptedServer.identity
            generation_b_server_identity = [string]$secondScriptedServer.identity
            probe_server_identity = [string]$probeScriptedServer.identity
            probe_generation_subtree_absent = [bool]$probeServerAbsence.absent
            generation_a_subtree_absent = [bool]$firstServerAbsence.absent
            generation_b_subtree_absent = [bool]$secondServerAbsence.absent
            no_duplicate_agent = $true
        }
        resource = [ordered]@{
            real_model_calls = 0
            real_model_cost = 'NOT_INCURRED_SCRIPTED_APP_SERVER'
            bounded_scripted_events = [long]$scriptedEventEvidence.line_count
        }
        cleanup = [ordered]@{
            postgres_stopped = (-not $postgresRunning -and
                -not $script:postgresStartMayOwnProcess)
            listeners_absent = $listenerCleanup
            temporary_root_removed = $cleanupSucceeded
            scripted_fixture_removed = $scriptedFixtureCleanup
            artifacts_retained = (-not $cleanupSucceeded -or -not $scriptedFixtureCleanup)
            evidence_root = $(if ($cleanupSucceeded) { $null } else { $runRoot })
            fixture_root = $(
                if ($scriptedFixtureCleanup) { $null } else { [string]$scriptedFixture.root }
            )
        }
    } | ConvertTo-Json -Compress -Depth 20
    return
}
if ($null -eq $failureCode -and $null -ne $terminalStatus -and $null -ne $restartStatus -and
    $null -ne $resourceBudgetBefore -and $null -ne $gitControlProof -and
    $foremanProcessTreeStopped -and
    (($Wsl2LinuxLive -and
        $credentialReadIsolation -ceq 'WSL2_LINUX_KEYRING_VERIFIED') -or
     (-not $Wsl2LinuxLive -and $credentialReadIsolation -ceq 'VERIFIED')) -and
    (-not $Wsl2LinuxLive -or
        ($null -ne $wslEnvironmentBefore -and $null -ne $wslEnvironmentAfter -and
         $null -ne $wslActiveRestartBefore -and $null -ne $wslActiveRestartAfter -and
          $null -ne $finalMaterialization -and $null -ne $wslSubstitutionEvidence -and
          $null -ne $wslProviderPreflightEvidence -and
          $null -ne $wslProviderOpenMarkerEvidence -and
          $null -ne $wslProviderEffectsBeforeReconciliation -and
          $null -ne $wslProviderSubtreeReconciliation -and
          $null -ne $wslProviderSubtreeFreshReplay -and
          $null -ne $wslReviewerSubtreeEvidence -and
          $null -ne $wslProviderFenceBeforeHardStop -and
         $null -ne $wslProviderFenceAfterHardStop -and
         [bool]$wslProviderFenceAfterHardStop.value.gate_passed -and
         [bool]$wslProviderFenceAfterHardStop.value.exact_old_processes_absent -and
          $null -ne $wslProviderEffectsAfterFence -and
          [long]$wslProviderEffectsAfterFence.value.provider_effect_count -eq 2 -and
          $null -ne $wslFailureSubtreeCleanup -and
          [bool]$wslFailureSubtreeCleanup.gate_passed -and
          -not $wslReconciliationRequired)) -and
    $physicalRestart -and $noDuplicateAgent -and $listenerCleanup) {
    [ordered]@{
        schema = 'lattice.phase4-managed-foreman.acceptance.v1'
        status = 'PASS'
        acceptance = $true
        real_codex = $true
        elapsed_ms = [long]$elapsed
        task_ref = [string]$terminalStatus.task_ref
        task_state = [string]$terminalStatus.task_state
        worker = [ordered]@{
            thread_id = [string]$terminalStatus.thread_id
            turn_id = [string]$terminalStatus.turn_id
            attempt = [int]$terminalStatus.attempt
            retry_count = [int]$terminalStatus.retry_count
            model = [string]$terminalStatus.model
            reasoning = [string]$terminalStatus.reasoning
            model_reason = [string]$databaseBefore.value.model_reason
            exact_turn_started = $true
            terminal = [string]$databaseBefore.value.terminal_kind
        }
        reviewer = [ordered]@{
            thread_id = [string]$databaseBefore.value.review_thread_id
            turn_id = [string]$databaseBefore.value.review_turn_id
            exact_turn_started = $true
            terminal = [string]$databaseBefore.value.review_terminal_status
            model = [string]$databaseBefore.value.review_model
            reasoning = [string]$databaseBefore.value.review_reasoning
            model_reason = [string]$databaseBefore.value.review_model_reason
            model_call_identity_count = [long]$databaseBefore.value.model_call_identity_count
        }
        foreman = [ordered]@{
            generation = [long]$terminalStatus.foreman_generation
            checkpoint_digest = [string]$terminalStatus.foreman_checkpoint_digest
            checkpoint_status = [string]$checkpoint.status
        }
        authorization = [ordered]@{
            execution = [string]$databaseBefore.value.authority_source
            capability = [string]$databaseBefore.value.approval_capability
            task_spec_digest = [string]$databaseBefore.value.approval_task_spec_digest
            approval_subject_digest = [string]$databaseBefore.value.approval_subject_digest
            budget_digest = [string]$databaseBefore.value.approval_budget_digest
            authority_evidence_digest = [string]$databaseBefore.value.approval_authority_evidence_digest
            authority_digest = [string]$databaseBefore.value.approval_authority_digest
            issued_at = [string]$databaseBefore.value.approval_issued_at
            expires_at = [string]$databaseBefore.value.approval_expires_at
            merge = 'NOT_GRANTED'
            push = 'NOT_GRANTED'
            deploy = 'NOT_GRANTED'
        }
        exact_binding = [ordered]@{
            promotion_task_spec_digest = [string]$databaseBefore.value.promotion_task_spec_digest
            promotion_approval_subject_digest = [string]$databaseBefore.value.promotion_approval_subject_digest
            promotion_budget_digest = [string]$databaseBefore.value.promotion_budget_digest
            promotion_binding_digest = [string]$databaseBefore.value.promotion_binding_digest
            attempt_task_spec_digest = [string]$databaseBefore.value.attempt_task_spec_digest
            attempt_budget_digest = [string]$databaseBefore.value.attempt_budget_digest
            attempt_binding_digest = [string]$databaseBefore.value.attempt_binding_digest
            attempt_authority_digest = [string]$databaseBefore.value.attempt_authority_digest
            writer_fence = [long]$databaseBefore.value.writer_fence
        }
        budget = [ordered]@{
            global_active_limit = [long]$databaseBefore.value.budget_global_active_limit
            per_task_active_limit = [long]$databaseBefore.value.budget_per_task_active_limit
            repair_retry_limit = [long]$databaseBefore.value.budget_repair_retry_limit
            max_attempts = [long]$databaseBefore.value.budget_max_attempts
            max_duration_seconds = [long]$databaseBefore.value.budget_max_duration_seconds
            budget_max_total_tokens = [long]$databaseBefore.value.budget_max_total_tokens
            budget_max_model_calls = [long]$databaseBefore.value.budget_max_model_calls
            external_cost_status = [string]$databaseBefore.value.budget_external_cost_status
            deadline_at = [string]$databaseBefore.value.budget_deadline_at
        }
        verification = [ordered]@{
            status = [string]$terminalStatus.verification_status
            result_digest = [string]$terminalStatus.verification_digest
            evidence_digest = [string]$terminalStatus.evidence_digest
            owner_replay_digest = [string]$databaseBefore.owner_replay_digest
            base_commit = [string]$gitEvidence.base_commit
            result_commit = [string]$gitEvidence.result_commit
            tree = [string]$gitEvidence.tree
            diff_digest = [string]$gitEvidence.diff_digest
            changed_paths = @($gitEvidence.changed_paths)
            checks = @($gitEvidence.checks)
            review_digest = [string]$databaseBefore.value.review_digest
        }
        managed_worktree = [ordered]@{
            root = [string]$managedWorkerRoot
            baseline_digest = [string]$databaseBefore.value.baseline_content_digest
            attempt_worktree_digest = [string]$databaseBefore.value.worktree_digest
            source_checkout_unchanged = $true
            source_ignored_credentials_absent = $true
            protected_ref = [string]$protectedRef
            protected_commit = [string]$gitEvidence.result_commit
        }
        execution_environment = $(
            if ($Wsl2LinuxLive) {
                [ordered]@{
                    kind = 'WSL2_LINUX'
                    execution_environment_ref =
                        [string]$finalMaterialization.record.execution_environment_ref
                    canonical_descriptor_digest = Get-Phase4StringSha256 `
                        -Value ([string]$finalMaterialization.descriptor_json)
                    distribution = [string]$finalMaterialization.descriptor.distribution
                    distribution_identity_ref =
                        [string]$finalMaterialization.descriptor.distribution_identity.identity_digest
                    linux_repository_path =
                        [string]$finalMaterialization.descriptor.linux.cwd
                    repository_head =
                        [string]$finalMaterialization.descriptor.linux.repository_head
                    repository_identity_ref =
                        [string]$finalMaterialization.descriptor.linux.repository_identity
                    path_mapping_ref =
                        [string]$finalMaterialization.descriptor.path_mapping.digest
                    credential_authority_kind =
                        [string]$finalMaterialization.descriptor.credential_authority.kind
                    credential_authority_ref =
                        [string]$finalMaterialization.descriptor.credential_authority.authority_digest
                    process_fence_ref =
                        [string]$finalMaterialization.descriptor.process_fence.identity_digest
                    preflight_process_fence = [string]$finalMaterialization.process_fence
                    verification_toolchain_ref =
                        [string]$finalMaterialization.descriptor.verification_toolchain.identity_digest
                    provider_effect_count =
                        [long]$wslEnvironmentBefore.value.provider_effect_count
                    artifact_outbox_count =
                        [long]$wslEnvironmentBefore.value.artifact_outbox_count
                    attempt_count = [long]$wslEnvironmentBefore.value.attempt_count
                    attempt_limit = 3
                    bootstrap_descriptor_persisted = $false
                    substitution_gates = $wslSubstitutionEvidence
                    final_descriptor_exact_replay =
                        ([string]$wslEnvironmentAfter.digest -ceq
                            [string]$wslEnvironmentBefore.digest)
                    durable_evidence_digest = [string]$wslEnvironmentBefore.digest
                    fresh_process_evidence_digest = [string]$wslEnvironmentAfter.digest
                    evidence_directory = [string]$finalMaterialization.evidence_directory
                    materializer_process_evidence = $finalMaterialization.process_evidence
                    reviewer_subtree = $wslReviewerSubtreeEvidence
                }
            }
            else {
                [ordered]@{ kind = 'NATIVE_WINDOWS' }
            }
        )
        resource = [ordered]@{
            worker_model_call_identity = [string]$resourceBudgetBefore.worker_model_call_identity
            worker_terminal_resource_digest = [string]$resourceBudgetBefore.worker_terminal_resource_digest
            worker_total_tokens = [long]$resourceBudgetBefore.worker_total_tokens
            reviewer_model_call_identity = [string]$resourceBudgetBefore.reviewer_model_call_identity
            reviewer_terminal_resource_digest = [string]$resourceBudgetBefore.reviewer_terminal_resource_digest
            reviewer_total_tokens = [long]$resourceBudgetBefore.reviewer_total_tokens
            observed_total_tokens = [long]$resourceBudgetBefore.observed_total_tokens
            observed_model_calls = [long]$resourceBudgetBefore.observed_model_calls
            within_budget = [bool]$resourceBudgetBefore.within_budget
            evidence_digest = [string]$resourceBudgetBefore.evidence_digest
            status_observation = $terminalStatus.resource_observation
        }
        security = [ordered]@{
            credential_read_isolation = $credentialReadIsolation
            lattice_owned_auth_copy = 'NOT_CREATED'
        }
        effect_evidence = [ordered]@{
            control_evidence = $gitControlProof
            observed_effects = [ordered]@{
                schema = 'lattice.phase4-observed-effects/1.0'
                status = 'NOT_MEASURED'
                reason = 'NO_OS_LEVEL_EFFECT_ATTESTATION'
                push = 'UNVERIFIED'
                deploy = 'UNVERIFIED'
                payment = 'UNVERIFIED'
                external_message = 'UNVERIFIED'
            }
        }
        restart_replay = [ordered]@{
            postgres_system_identifier = $systemIdentifierAfter
            postgres_process_changed = $true
            control_process_changed = $true
            foreman_process_changed = $true
            status_digest = Get-Phase4StringSha256 -Value (
                $restartStatus | ConvertTo-Json -Compress -Depth 30
            )
            database_replay_digest = [string]$databaseAfter.digest
            owner_replay_digest = [string]$databaseAfter.owner_replay_digest
            git_control_proof_digest = [string]$gitControlProof.git_control_proof_digest
            control_project_digest = [string]$projectRestartDigest
            attempt_count = [long]$databaseAfter.value.attempt_count
            thread_count = [long]$databaseAfter.value.thread_count
            turn_count = [long]$databaseAfter.value.turn_count
            no_duplicate_agent = $noDuplicateAgent
            baseline_exact_replay = $true
            protected_ref_exact_replay = $true
            active_provider_reconnect = $(
                if ($Wsl2LinuxLive) {
                    [ordered]@{
                        attempt_id = [string]$wslActiveRestartBefore.value.attempt_id
                        writer_fence = [long]$wslActiveRestartBefore.value.writer_fence
                        thread_id = [string]$wslActiveRestartBefore.value.thread_id
                        turn_id = [string]$wslActiveRestartBefore.value.turn_id
                        durable_thread_accepted =
                            ([long]$wslActiveRestartBefore.value.thread_count -eq 1)
                        durable_turn_accepted =
                            ([long]$wslActiveRestartBefore.value.turn_count -eq 1)
                        durable_turn_started =
                            ([long]$wslActiveRestartBefore.value.turn_started_count -eq 1)
                        reconciled_count =
                            [long]$wslActiveRestartAfter.value.reconciled_count
                        process_handoff_count =
                            [long]$wslActiveRestartAfter.value.process_handoff_count
                        worker_thread_dispatch_count =
                            [long]$wslActiveRestartAfter.value.worker_thread_dispatch_count
                        worker_turn_dispatch_count =
                            [long]$wslActiveRestartAfter.value.worker_turn_dispatch_count
                        same_attempt_thread_turn_and_fence = $true
                        prior_provider_fence = [ordered]@{
                            preflight_artifact_ref =
                                [string]$wslProviderPreflightEvidence.artifact_ref
                            preflight_receipt_digest =
                                [string]$wslProviderPreflightEvidence.receipt_digest
                            unit = [string]$wslProviderFenceBeforeHardStop.value.provider_unit
                            process_fence =
                                [string]$wslProviderFenceBeforeHardStop.value.process_fence
                            cgroup_path =
                                [string]$wslProviderFenceBeforeHardStop.value.cgroup_path
                            process_marker_count = @(
                                $wslProviderFenceBeforeHardStop.value.process_markers
                            ).Count
                            process_markers = @(
                                $wslProviderFenceBeforeHardStop.value.process_markers
                            )
                            active_evidence_digest =
                                [string]$wslProviderFenceBeforeHardStop.evidence_digest
                            closed_evidence_digest =
                                [string]$wslProviderFenceAfterHardStop.evidence_digest
                            unit_inactive =
                                ([string]$wslProviderFenceAfterHardStop.value.active_state -ceq
                                    'inactive')
                            cgroup_closed =
                                [bool]$wslProviderFenceAfterHardStop.value.gate_passed
                            exact_old_processes_absent =
                                [bool]$wslProviderFenceAfterHardStop.value.exact_old_processes_absent
                            provider_effect_count_before =
                                [long]$wslEnvironmentAtAcceptedStart.value.provider_effect_count
                            provider_effect_count_after =
                                [long]$wslProviderEffectsAfterFence.value.provider_effect_count
                            provider_open_artifact_ref =
                                [string]$wslProviderOpenMarkerEvidence.artifact_ref
                            provider_open_marker_digest =
                                [string]$wslProviderOpenMarkerEvidence.marker_digest
                            provider_subtree_segment_ref =
                                [string]$wslProviderOpenMarkerEvidence.provider_subtree_segment_ref
                            provider_open_producer_digest =
                                [string]$wslProviderOpenMarkerEvidence.producer_digest
                            reconciliation_artifact_ref =
                                [string]$wslProviderSubtreeReconciliation.artifact_ref
                            reconciliation_digest =
                                [string]$wslProviderSubtreeReconciliation.value.reconciliation_digest
                            reconciliation_producer_digest =
                                [string]$wslProviderSubtreeReconciliation.producer_digest
                            reconciliation_ledger_event_sequence =
                                [string]$wslProviderSubtreeReconciliation.ledger_event_sequence
                            successor_preflight_artifact_ref =
                                [string]$wslProviderSubtreeReconciliation.successor_preflight_artifact_ref
                            successor_preflight_ledger_event_sequence =
                                [string]$wslProviderSubtreeReconciliation.successor_preflight_ledger_event_sequence
                            successor_marker_artifact_ref =
                                [string]$wslProviderSubtreeReconciliation.successor_marker_artifact_ref
                            successor_marker_ledger_event_sequence =
                                [string]$wslProviderSubtreeReconciliation.successor_marker_ledger_event_sequence
                            fresh_process_exact_replay =
                                ([string]$wslProviderSubtreeFreshReplay.chain_binding_digest -ceq
                                    [string]$wslProviderSubtreeReconciliation.chain_binding_digest)
                        }
                    }
                }
                else { $null }
            )
        }
        identities = [ordered]@{
            latticed_sha256 = Get-Phase4FileSha256 -Path $latticed
            codex_sha256 = Get-Phase4FileSha256 -Path $codex
            project_id = $projectId
            objective_sha256 = Get-Phase4StringSha256 -Value $objective
        }
        mcp = [ordered]@{
            long_lived_session_count = $mcpRecords.Count
            total_tool_calls = [long](
                ($mcpRecords | ForEach-Object { $_.tool_call_count } | Measure-Object -Sum).Sum
            )
        }
        cleanup = [ordered]@{
            postgres_stopped = (-not $postgresRunning -and
                -not $script:postgresStartMayOwnProcess)
            listeners_absent = $listenerCleanup
            foreman_process_tree_stopped = $foremanProcessTreeStopped
            failure_subtree_cleanup = $wslFailureSubtreeCleanup
            temporary_root_removed = $cleanupSucceeded
            artifacts_retained = (-not $cleanupSucceeded)
            evidence_root = $(if ($cleanupSucceeded) { $null } else { $runRoot })
        }
    } | ConvertTo-Json -Compress -Depth 20
    return
}

$failureReceiptPath = $null
$failureReceiptDigestPath = $null
$failureReceiptPersistence = [ordered]@{
    receipt_status = 'NOT_AVAILABLE'
    receipt_code = $null
    path = $null
    digest_sidecar_path = $null
}
if ($runRootCreated -and (Test-Path -LiteralPath $runRoot -PathType Container)) {
    try {
        $null = Get-Phase4OwnerMarker -RunRoot $runRoot -RunId $runId -MarkerPath $markerPath
        $failureReceiptPath = Assert-Phase4ContainedPath -Root $runRoot `
            -Path (Join-Path $runRoot 'final-failure-receipt.json') `
            -Failure 'PHASE4_FAILURE_RECEIPT_PATH_REJECTED'
        $failureReceiptDigestPath = Assert-Phase4ContainedPath -Root $runRoot `
            -Path (Join-Path $runRoot 'final-failure-receipt.sha256') `
            -Failure 'PHASE4_FAILURE_RECEIPT_PATH_REJECTED'
        $failureReceiptPersistence.receipt_status = 'PENDING'
        $failureReceiptPersistence.path = $failureReceiptPath
        $failureReceiptPersistence.digest_sidecar_path = $failureReceiptDigestPath
    }
    catch {
        $failureReceiptPersistence.receipt_status = 'FAILED'
        $failureReceiptPersistence.receipt_code =
            'PHASE4_FAILURE_RECEIPT_OWNERSHIP_REJECTED'
    }
}
$failureReceipt = [ordered]@{
    schema = 'lattice.phase4-managed-foreman.acceptance.v1'
    status = 'FAIL'
    acceptance = $false
    real_codex = $false
    real_codex_attempted = (-not $ScriptedActiveRestart -and $realCodexAttempted)
    real_codex_attempt_evidence = $realCodexAttemptEvidence
    scripted_app_server_attempted = ($ScriptedActiveRestart -and $null -ne $firstActiveStatus)
    failure_stage = $failureStage
    failure_code = $(if ($null -eq $failureCode) { 'PHASE4_ACCEPTANCE_INCOMPLETE' } else { $failureCode })
    failure_line = $failureLine
    failure_exception = $failureException
    security = [ordered]@{
        credential_read_isolation = $credentialReadIsolation
        real_codex_dispatch_allowed = $false
        lattice_owned_auth_copy = 'NOT_CREATED'
    }
    last_status = Get-Phase4ManagedStatusDiagnostic -Status $lastManagedStatus
    mcp_status_timeout = $mcpStatusTimeoutDiagnostic
    failure_receipt_persistence = $failureReceiptPersistence
    task_ref = $(if ($null -eq $submitted) { $null } else { [string]$submitted.task_ref })
    wsl2 = $(
        if ($Wsl2LinuxLive) {
            [ordered]@{
                source_linux_path = $wslSourceRepository
                source_windows_path = $projectRoot
                managed_linux_path = $expectedManagedLinuxPath
                managed_windows_path = $expectedManagedUncPath
                final_execution_environment_ref = $(
                    if ($null -eq $finalMaterialization) { $null }
                    else { [string]$finalMaterialization.record.execution_environment_ref }
                )
                final_evidence_directory = $(
                    if ($null -eq $finalMaterialization) { $null }
                    else { [string]$finalMaterialization.evidence_directory }
                )
                durable_provider_effect_count = $(
                    if ($null -ne $wslFinalDurableEvidence) {
                        [long]$wslFinalDurableEvidence.value.provider_effect_count
                    }
                    elseif ($null -ne $wslFinalDurableEvidenceBeforeCleanup) {
                        [long]$wslFinalDurableEvidenceBeforeCleanup.value.provider_effect_count
                    }
                    else { $null }
                )
                durable_provider_effect_status = $wslDurableProviderEffectStatus
                last_observed_provider_effect_count = $(
                    if ($null -ne $wslFinalDurableEvidence) {
                        [long]$wslFinalDurableEvidence.value.provider_effect_count
                    }
                    elseif ($null -ne $wslFinalDurableEvidenceBeforeCleanup) {
                        [long]$wslFinalDurableEvidenceBeforeCleanup.value.provider_effect_count
                    }
                    elseif ($null -ne $wslEnvironmentAfter) {
                        [long]$wslEnvironmentAfter.value.provider_effect_count
                    }
                    elseif ($null -ne $wslEnvironmentBefore) {
                        [long]$wslEnvironmentBefore.value.provider_effect_count
                    }
                    elseif ($null -ne $wslProviderEffectsAfterFence) {
                        [long]$wslProviderEffectsAfterFence.value.provider_effect_count
                    }
                    elseif ($null -ne $wslEnvironmentAtAcceptedStart) {
                        [long]$wslEnvironmentAtAcceptedStart.value.provider_effect_count
                    }
                    elseif ($null -ne $wslZeroProviderAfterCheckpoint) {
                        [long]$wslZeroProviderAfterCheckpoint.value.provider_effect_count
                    }
                    elseif ($null -ne $wslZeroProviderAfterMaterialization) {
                        [long]$wslZeroProviderAfterMaterialization.value.provider_effect_count
                    }
                    elseif ($null -ne $wslZeroProviderEvidence) {
                        [long]$wslZeroProviderEvidence.value.provider_effect_count
                    }
                    else { $null }
                )
                reconciliation_required = $wslReconciliationRequired
                failure_subtree_cleanup = $wslFailureSubtreeCleanup
                failure_subtree_cleanup_code = $wslFailureSubtreeCleanupCode
                provider_fence = $(
                    if ($null -ne $wslProviderPreflightEvidence -or
                        $null -ne $wslProviderFenceBeforeHardStop -or
                        $null -ne $wslProviderFenceAfterHardStop) {
                        [ordered]@{
                            preflight_artifact_ref = $(
                                if ($null -eq $wslProviderPreflightEvidence) { $null }
                                else { [string]$wslProviderPreflightEvidence.artifact_ref }
                            )
                            unit = $(
                                if ($null -eq $wslProviderPreflightEvidence) { $null }
                                else { [string]$wslProviderPreflightEvidence.provider_unit }
                            )
                            process_fence = $(
                                if ($null -eq $wslProviderPreflightEvidence) { $null }
                                else { [string]$wslProviderPreflightEvidence.process_fence }
                            )
                            cgroup_path = $(
                                if ($null -eq $wslProviderFenceBeforeHardStop) { $null }
                                else { [string]$wslProviderFenceBeforeHardStop.value.cgroup_path }
                            )
                            process_marker_count = $(
                                if ($null -eq $wslProviderFenceBeforeHardStop) { 0 }
                                else { @($wslProviderFenceBeforeHardStop.value.process_markers).Count }
                            )
                            process_markers = $(
                                if ($null -eq $wslProviderFenceBeforeHardStop) { @() }
                                else { @($wslProviderFenceBeforeHardStop.value.process_markers) }
                            )
                            active_evidence_digest = $(
                                if ($null -eq $wslProviderFenceBeforeHardStop) { $null }
                                else { [string]$wslProviderFenceBeforeHardStop.evidence_digest }
                            )
                            closed_evidence_digest = $(
                                if ($null -eq $wslProviderFenceAfterHardStop) { $null }
                                else { [string]$wslProviderFenceAfterHardStop.evidence_digest }
                            )
                            unit_inactive = $(
                                if ($null -eq $wslProviderFenceAfterHardStop) { $false }
                                else {
                                    [string]$wslProviderFenceAfterHardStop.value.active_state -ceq
                                        'inactive'
                                }
                            )
                            cgroup_closed = $(
                                if ($null -eq $wslProviderFenceAfterHardStop) { $false }
                                else { [bool]$wslProviderFenceAfterHardStop.value.gate_passed }
                            )
                            exact_old_processes_absent = $(
                                if ($null -eq $wslProviderFenceAfterHardStop) { $false }
                                else {
                                    [bool]$wslProviderFenceAfterHardStop.value.exact_old_processes_absent
                                }
                            )
                            provider_effect_count_before = $(
                                if ($null -eq $wslEnvironmentAtAcceptedStart) { $null }
                                else {
                                    [long]$wslEnvironmentAtAcceptedStart.value.provider_effect_count
                                }
                            )
                            provider_effect_count_after = $(
                                if ($null -eq $wslProviderEffectsAfterFence) { $null }
                                else {
                                    [long]$wslProviderEffectsAfterFence.value.provider_effect_count
                                }
                            )
                        }
                    }
                    else { $null }
                )
                wsl_task_assets_retained = $true
            }
        }
        else { $null }
    )
    retained_evidence_root = $(
        if ($runRootCreated -and (Test-Path -LiteralPath $runRoot -PathType Container)) {
            $runRoot
        }
        else {
            $null
        }
    )
    resource_usage = [ordered]@{
        status = $(
            if ($null -ne $resourceBudgetBefore) { 'OBSERVED_TERMINAL' }
            elseif ($realCodexAttempted) { 'UNKNOWN_RECONCILIATION_REQUIRED' }
            else { 'NOT_INCURRED' }
        )
        worker_total_tokens = $(
            if ($null -eq $resourceBudgetBefore) { $null }
            else { [long]$resourceBudgetBefore.worker_total_tokens }
        )
        reviewer_total_tokens = $(
            if ($null -eq $resourceBudgetBefore) { $null }
            else { [long]$resourceBudgetBefore.reviewer_total_tokens }
        )
        observed_total_tokens = $(
            if ($null -eq $resourceBudgetBefore) { $null }
            else { [long]$resourceBudgetBefore.observed_total_tokens }
        )
        observed_model_calls = $(
            if ($null -eq $resourceBudgetBefore) { $null }
            else { [long]$resourceBudgetBefore.observed_model_calls }
        )
    }
    cleanup = [ordered]@{
        postgres_stopped = (-not $postgresRunning -and
            -not $script:postgresStartMayOwnProcess)
        listeners_absent = $listenerCleanup
        plaintext_password_file_absent = (-not (Test-Path -LiteralPath $passwordPath))
        foreman_process_tree_stopped = $foremanProcessTreeStopped
        failure_subtree_cleanup = $wslFailureSubtreeCleanup
    }
}
$failureReceiptJson = $null
if ([string]$failureReceiptPersistence.receipt_status -ceq 'PENDING') {
    # This status describes only the create-new receipt file. The optional
    # digest sidecar makes its own existence claim and is never predeclared PASS.
    $failureReceiptPersistence.receipt_status = 'PASS'
    $failureReceiptJson = $failureReceipt | ConvertTo-Json -Compress -Depth 12
    $failureReceiptWritten = $false
    try {
        Write-Phase4AtomicCreateNewUtf8File -Path $failureReceiptPath `
            -Content ($failureReceiptJson + "`n")
        $failureReceiptWritten = $true
        $failureReceiptSha256 = Get-Phase4FileSha256 -Path $failureReceiptPath
        Write-Phase4AtomicCreateNewUtf8File -Path $failureReceiptDigestPath `
            -Content ($failureReceiptSha256 + '  final-failure-receipt.json' + "`n")
    }
    catch {
        if ($failureReceiptWritten) {
            [Console]::Error.WriteLine(
                'PHASE4_FAILURE_RECEIPT_DIGEST_PERSISTENCE_REJECTED'
            )
        }
        else {
            $failureReceiptPersistence.receipt_status = 'FAILED'
            $failureReceiptPersistence.receipt_code =
                'PHASE4_FAILURE_RECEIPT_PERSISTENCE_REJECTED'
            $failureReceiptJson = $failureReceipt | ConvertTo-Json -Compress -Depth 12
            [Console]::Error.WriteLine('PHASE4_FAILURE_RECEIPT_PERSISTENCE_REJECTED')
        }
    }
}
if ($null -eq $failureReceiptJson) {
    $failureReceiptJson = $failureReceipt | ConvertTo-Json -Compress -Depth 12
}
$failureReceiptJson
exit 1
