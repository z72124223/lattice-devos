[CmdletBinding()]
param(
    [ValidateSet('FullChainPreStatus', 'FullChainRun', 'FullChainStatus')]
    [string]$InternalPhase,
    [string]$CodexAuthHome,
    [switch]$HarnessSelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$officialCodexVersion = 'codex-cli 0.146.0'
$officialCodexSha256 = 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb'
$openClawPackageTarballSha256 = '5bb525f36f471a41239615d321c441778c7e1c007018ed6d84b795be77803276'
$openClawEntrypointSha256 = 'f643b005d6db233a0b45204e8d8e943256874ccc6897b8a6e0cf42a9b376a188'
$openClawVersion = '2026.7.1-2'
$hermesRuntimeGuestRoot = '/var/tmp/lattice-runtime-targets/hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-errorfix-v1'
$hermesRuntimeManifestSha256 = 'e3a3272b6cead30cd2df1af755df031766475595fdacfb080d0886671b6d1fbb'
$taskBinding = [ordered]@{
    project_id = 'task032-delivery'
    project_snapshot_id = 'task032-delivery:snapshot:1'
    task_id = 'TASK-032'
    revision = '1'
    task_spec_digest = 'b70aa1a7445ea7e7ebe466154d13ea1039f963b5ac4ffe1f7d5094dd8c949e0e'
}
$deliveryConfigBytes = [System.Text.UTF8Encoding]::new($false).GetBytes((@(
    'approval_policy = "never"',
    'sandbox_mode = "workspace-write"',
    'model = "gpt-5.6-sol"',
    'model_reasoning_effort = "low"',
    '',
    '[windows]',
    'sandbox = "unelevated"'
) -join "`n") + "`n")
$codexMarkerBytes = [System.Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$task037MaxEvidenceFileBytes = 1048576
$task037MaxRetainedLogBytes = 65536

function Get-CanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $trimCharacters = [char[]]@(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    return [System.IO.Path]::GetFullPath($Path).TrimEnd($trimCharacters)
}

function Test-ExactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    return [string]::Equals(
        (Get-CanonicalPath -Path $Actual),
        (Get-CanonicalPath -Path $Expected),
        [System.StringComparison]::OrdinalIgnoreCase
    )
}

function Assert-NoReparseAncestor {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Boundary
    )

    $canonicalPath = Get-CanonicalPath -Path $Path
    $canonicalBoundary = Get-CanonicalPath -Path $Boundary
    $boundaryPrefix = $canonicalBoundary + [System.IO.Path]::DirectorySeparatorChar
    if (-not (Test-ExactPath -Actual $canonicalPath -Expected $canonicalBoundary) -and
        -not $canonicalPath.StartsWith($boundaryPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'TASK037_PATH_OUTSIDE_REPOSITORY'
    }

    $current = $canonicalPath
    while ($true) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw 'TASK037_REPARSE_ANCESTOR_REJECTED'
            }
        }
        if (Test-ExactPath -Actual $current -Expected $canonicalBoundary) {
            break
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrWhiteSpace($parent) -or (Test-ExactPath -Actual $parent -Expected $current)) {
            throw 'TASK037_PATH_ANCESTRY_REJECTED'
        }
        $current = $parent
    }
}

function Assert-RegularFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $Path -PathType Leaf)
    ) {
        throw ('TASK037_FILE_REJECTED|' + $Path)
    }
}

function ConvertTo-LowerHex {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    return -join ($Bytes | ForEach-Object { $_.ToString('x2') })
}

function New-RandomBytes {
    param([Parameter(Mandatory = $true)][int]$Count)

    $bytes = New-Object byte[] $Count
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($bytes)
        return $bytes
    }
    finally {
        $rng.Dispose()
    }
}

function New-RandomHex {
    param([Parameter(Mandatory = $true)][int]$Bytes)

    return ConvertTo-LowerHex (New-RandomBytes -Count $Bytes)
}

function Get-FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-StringSha256 {
    param([Parameter(Mandatory = $true)][string]$Text)

    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($Text)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return ConvertTo-LowerHex ($sha256.ComputeHash($bytes))
    }
    finally {
        $sha256.Dispose()
    }
}

function Write-JsonEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force:$false | Out-Null
    }
    [System.IO.File]::WriteAllText(
        $Path,
        ($Value | ConvertTo-Json -Depth 16) + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Get-SafeFailureEvidence {
    param([string]$Message)

    if ([string]::IsNullOrEmpty($Message)) {
        return $null
    }
    $fixedCode = if ($Message -match '^(TASK037|TASK019|LATTICE|HERMES|GRAPHIFY|OPENCLAW)_[A-Z0-9_]+') {
        $Matches[0]
    } else {
        'TASK037_UNCLASSIFIED_FAILURE'
    }
    return [ordered]@{
        code = $fixedCode
        sha256 = Get-StringSha256 $Message
        byte_count = [Text.Encoding]::UTF8.GetByteCount($Message)
    }
}

function Throw-Task037SafeFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$FallbackCode = 'TASK037_UNCLASSIFIED_FAILURE'
    )

    $evidence = Get-SafeFailureEvidence -Message $Message
    if ($null -ne $evidence -and -not [string]::IsNullOrWhiteSpace([string]$evidence.code)) {
        throw ([string]$evidence.code)
    }
    throw $FallbackCode
}

function Invoke-Task037GitText {
    param(
        [Parameter(Mandatory = $true)][string]$GitExe,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $output = @(& $GitExe @Arguments 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw $FailureCode
    }
    return ($output -join "`n")
}

function Assert-BoundedTask037EvidenceFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Code,
        [int64]$MaxBytes = $task037MaxEvidenceFileBytes
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return
    }
    $length = (Get-Item -LiteralPath $Path).Length
    if ($length -gt $MaxBytes) {
        throw $Code
    }
}

function Redact-Task037LogText {
    param([string]$Text)

    if ([string]::IsNullOrEmpty($Text)) {
        return ''
    }
    $redacted = $Text
    $redacted = [regex]::Replace($redacted, '(?i)(authorization\s*[:=]\s*Bearer\s+)[^\s"''\\]+', '${1}[REDACTED]')
    $redacted = [regex]::Replace($redacted, '(?i)("?(?:api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|gateway[_-]?token|token|password|secret|credential|auth)"?\s*[:=]\s*")[^"\r\n]+(")', '${1}[REDACTED]${2}')
    $redacted = [regex]::Replace($redacted, '(?i)((?:api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|gateway[_-]?token|token|password|secret|credential|auth)\s*=\s*)[^\s\r\n&]+', '${1}[REDACTED]')
    $redacted = [regex]::Replace($redacted, '(?i)([?&](?:api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|gatewayToken|gateway[_-]?token|token)=)[^\s\r\n&"''<>]+', '${1}[REDACTED]')
    $limit = $task037MaxRetainedLogBytes
    if ($redacted.Length -gt $limit) {
        $redacted = $redacted.Substring(0, $limit) + "`n[TASK037_LOG_TRUNCATED]"
    }
    return $redacted
}

function Protect-Task037LogEvidence {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return [ordered]@{
            path = $Path
            exists = $false
        }
    }
    $item = Get-Item -LiteralPath $Path
    $originalSha256 = Get-FileSha256 $Path
    $readLimit = [Math]::Min([int64]$item.Length, [int64]$task037MaxRetainedLogBytes)
    $buffer = New-Object byte[] ([int]$readLimit)
    $stream = [IO.File]::OpenRead($Path)
    try {
        $bytesRead = if ($buffer.Length -eq 0) { 0 } else { $stream.Read($buffer, 0, $buffer.Length) }
    }
    finally {
        $stream.Dispose()
    }
    $text = [Text.Encoding]::UTF8.GetString($buffer, 0, $bytesRead)
    if ($item.Length -gt $task037MaxRetainedLogBytes) {
        $text += "`n[TASK037_LOG_TRUNCATED]"
    }
    $redacted = Redact-Task037LogText -Text $text
    [IO.File]::WriteAllText($Path, $redacted, [System.Text.UTF8Encoding]::new($false))
    return [ordered]@{
        path = $Path
        exists = $true
        original_sha256 = $originalSha256
        original_byte_count = $item.Length
        retained_sha256 = Get-FileSha256 $Path
        retained_byte_count = (Get-Item -LiteralPath $Path).Length
    }
}

function Read-JsonEvidence {
    param([Parameter(Mandatory = $true)][string]$Path)

    return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Get-U32BigEndian {
    param([Parameter(Mandatory = $true)][uint32]$Value)

    return [byte[]]@(
        [byte](($Value -shr 24) -band 0xff),
        [byte](($Value -shr 16) -band 0xff),
        [byte](($Value -shr 8) -band 0xff),
        [byte]($Value -band 0xff)
    )
}

function Add-Bytes {
    param(
        [Collections.Generic.List[byte]]$Target,
        [Parameter(Mandatory = $true)][byte[]]$Bytes
    )

    foreach ($byte in $Bytes) {
        $Target.Add($byte)
    }
}

function Add-AttestationField {
    param(
        [Collections.Generic.List[byte]]$Target,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][byte[]]$Value
    )

    $nameBytes = [Text.Encoding]::UTF8.GetBytes($Name)
    Add-Bytes -Target $Target -Bytes (Get-U32BigEndian ([uint32]$nameBytes.Length))
    Add-Bytes -Target $Target -Bytes $nameBytes
    Add-Bytes -Target $Target -Bytes (Get-U32BigEndian ([uint32]$Value.Length))
    Add-Bytes -Target $Target -Bytes $Value
}

function Get-AttestationTag {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Key,
        [Parameter(Mandatory = $true)][string]$LaunchRecordId,
        [Parameter(Mandatory = $true)][uint32]$ProcessId,
        [Parameter(Mandatory = $true)][byte[]]$ProcessNonce,
        [Parameter(Mandatory = $true)][string]$ProfileSha256
    )

    $payload = [Collections.Generic.List[byte]]::new()
    Add-Bytes -Target $payload -Bytes ([Text.Encoding]::UTF8.GetBytes('lattice-openclaw-launch-attestation-v1' + [char]0))
    Add-AttestationField -Target $payload -Name 'launch_record_id' -Value ([Text.Encoding]::UTF8.GetBytes($LaunchRecordId))
    Add-AttestationField -Target $payload -Name 'process_id' -Value (Get-U32BigEndian $ProcessId)
    Add-AttestationField -Target $payload -Name 'process_start_nonce' -Value $ProcessNonce
    Add-AttestationField -Target $payload -Name 'package_name' -Value ([Text.Encoding]::UTF8.GetBytes('openclaw'))
    Add-AttestationField -Target $payload -Name 'package_version' -Value ([Text.Encoding]::UTF8.GetBytes($openClawVersion))
    Add-AttestationField -Target $payload -Name 'source_commit' -Value ([Text.Encoding]::UTF8.GetBytes('0790d9f593ad30c940ed93b5872a8cf6d6f3cf8c'))
    Add-AttestationField -Target $payload -Name 'package_license' -Value ([Text.Encoding]::UTF8.GetBytes('MIT'))
    Add-AttestationField -Target $payload -Name 'package_integrity' -Value ([Text.Encoding]::UTF8.GetBytes('sha512-ycF3yPcbjN6bUPeaUx6Mh6vze1hQWoD3CT/wWcmD7a8xaHHHRUaAlaq+lFxMHf1ssEgODVAwjlzYqp2twkYZ7g=='))
    Add-AttestationField -Target $payload -Name 'entrypoint' -Value ([Text.Encoding]::UTF8.GetBytes('openclaw.mjs'))
    Add-AttestationField -Target $payload -Name 'package_tarball_digest' -Value ([Text.Encoding]::UTF8.GetBytes($openClawPackageTarballSha256))
    Add-AttestationField -Target $payload -Name 'entrypoint_digest' -Value ([Text.Encoding]::UTF8.GetBytes($openClawEntrypointSha256))
    Add-AttestationField -Target $payload -Name 'isolated_profile_digest' -Value ([Text.Encoding]::UTF8.GetBytes($ProfileSha256))
    $hmac = [Security.Cryptography.HMACSHA256]::new($Key)
    try {
        return ConvertTo-LowerHex ($hmac.ComputeHash($payload.ToArray()))
    }
    finally {
        $hmac.Dispose()
    }
}

function Get-UnreservedLoopbackPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    try {
        $listener.Start()
        return ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        $listener.Stop()
    }
}

function Test-LoopbackPort {
    param([Parameter(Mandatory = $true)][int]$Port)

    $client = [Net.Sockets.TcpClient]::new()
    try {
        $async = $client.BeginConnect('127.0.0.1', $Port, $null, $null)
        if (-not $async.AsyncWaitHandle.WaitOne(500)) {
            return $false
        }
        $client.EndConnect($async)
        return $true
    }
    catch {
        return $false
    }
    finally {
        $client.Dispose()
    }
}

function Stop-OwnedProcessTree {
    param([Parameter(Mandatory = $true)][int]$ProcessId)

    if ($ProcessId -le 0 -or $null -eq (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) {
        return
    }
    $taskkill = Join-Path $env:SystemRoot 'System32\taskkill.exe'
    $null = & $taskkill '/PID' ([string]$ProcessId) '/T' '/F' 2>&1
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (
        [DateTime]::UtcNow -lt $deadline -and
        $null -ne (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)
    ) {
        Start-Sleep -Milliseconds 100
    }
}

function Wait-LoopbackPortClosed {
    param([Parameter(Mandatory = $true)][int]$Port)

    if ($Port -le 0) {
        return
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (
        [DateTime]::UtcNow -lt $deadline -and
        @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue).Count -gt 0
    ) {
        Start-Sleep -Milliseconds 100
    }
}

function Remove-SafeTempRoot {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return $true
    }
    $canonical = Get-CanonicalPath -Path $Path
    $tempBase = Get-CanonicalPath -Path ([IO.Path]::GetTempPath())
    $prefix = $tempBase + [IO.Path]::DirectorySeparatorChar + 'lattice-'
    if (-not $canonical.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'TASK037_TEMP_CLEANUP_SCOPE_REJECTED'
    }
    Remove-Item -LiteralPath $canonical -Recurse -Force -ErrorAction SilentlyContinue
    return -not (Test-Path -LiteralPath $canonical)
}

function Invoke-Task037HarnessSelfTest {
    $tempBase = Get-CanonicalPath -Path ([IO.Path]::GetTempPath())
    $testRoot = Join-Path $tempBase ('lattice-task037-harness-' + [Guid]::NewGuid().ToString('N'))
    $markerPath = Join-Path $testRoot 'owned-marker.txt'
    $child = $null
    $childExited = $false

    try {
        New-Item -ItemType Directory -Path $testRoot -Force:$false | Out-Null
        Assert-NoReparseAncestor -Path $testRoot -Boundary $tempBase
        [IO.File]::WriteAllText($markerPath, "task037-harness-owned`n", [Text.UTF8Encoding]::new($false))
        Assert-RegularFile -Path $markerPath

        $powerShell = Join-Path $PSHOME 'powershell.exe'
        Assert-RegularFile -Path $powerShell
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $powerShell
        $startInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -Command "[Console]::Out.Write(''TASK037_HARNESS_CHILD=PASS'')"'
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.EnvironmentVariables.Clear()
        foreach ($entry in ([ordered]@{
            SystemRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
            WINDIR = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
            ComSpec = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)) 'System32\cmd.exe'
            TEMP = $testRoot
            TMP = $testRoot
            PSModuleAnalysisCachePath = 'NUL'
        }).GetEnumerator()) {
            $startInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
        }
        $child = [Diagnostics.Process]::new()
        $child.StartInfo = $startInfo
        if (-not $child.Start()) { throw 'TASK037_HARNESS_CHILD_START_REJECTED' }
        $stdout = $child.StandardOutput.ReadToEndAsync()
        $stderr = $child.StandardError.ReadToEndAsync()
        if (-not $child.WaitForExit(5000)) { throw 'TASK037_HARNESS_CHILD_TIMEOUT_REJECTED' }
        $childExited = $true
        $output = $stdout.GetAwaiter().GetResult()
        $errorOutput = $stderr.GetAwaiter().GetResult()
        if (
            $child.ExitCode -ne 0 -or
            $output -cne 'TASK037_HARNESS_CHILD=PASS' -or
            -not [string]::IsNullOrEmpty($errorOutput)
        ) {
            throw 'TASK037_HARNESS_CHILD_OUTPUT_REJECTED'
        }
    }
    finally {
        if ($null -ne $child) {
            if (-not $childExited -and -not $child.HasExited) {
                Stop-OwnedProcessTree -ProcessId $child.Id
            }
            $child.Dispose()
        }
        if (Test-Path -LiteralPath $testRoot) {
            Assert-NoReparseAncestor -Path $testRoot -Boundary $tempBase
            if (-not (Remove-SafeTempRoot -Path $testRoot)) {
                throw 'TASK037_HARNESS_CLEANUP_REJECTED'
            }
        }
    }

    Write-Output 'TASK037_HARNESS_SELF_TEST=PASS'
}

function Copy-CodexCredentialFiles {
    param(
        [Parameter(Mandatory = $true)][string]$SourceHome,
        [Parameter(Mandatory = $true)][string]$DestinationHome,
        [switch]$ForBroker
    )

    $source = Get-CanonicalPath -Path $SourceHome
    $auth = Join-Path $source 'auth.json'
    Assert-RegularFile -Path $auth
    if (-not (Test-Path -LiteralPath $DestinationHome)) {
        New-Item -ItemType Directory -Path $DestinationHome -Force:$false | Out-Null
    }
    [IO.File]::WriteAllBytes((Join-Path $DestinationHome '.lattice-codex-home-v1'), $codexMarkerBytes)
    [IO.File]::WriteAllBytes((Join-Path $DestinationHome 'auth.json'), [IO.File]::ReadAllBytes($auth))
    foreach ($name in @('cap_sid', 'installation_id', 'models_cache.json')) {
        $candidate = Join-Path $source $name
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            [IO.File]::WriteAllBytes((Join-Path $DestinationHome $name), [IO.File]::ReadAllBytes($candidate))
        }
    }
    if (-not $ForBroker) {
        [IO.File]::WriteAllBytes((Join-Path $DestinationHome 'config.toml'), $deliveryConfigBytes)
    }
}

function New-OpenClawProfile {
    param(
        [Parameter(Mandatory = $true)][string]$ProfileRoot,
        [Parameter(Mandatory = $true)][string]$PluginRoot
    )

    foreach ($directory in @($ProfileRoot, (Join-Path $ProfileRoot 'appdata'), (Join-Path $ProfileRoot 'localappdata'), (Join-Path $ProfileRoot 'temp'))) {
        if (-not (Test-Path -LiteralPath $directory)) {
            New-Item -ItemType Directory -Path $directory -Force:$false | Out-Null
        }
    }
    $profile = [ordered]@{
        agents = [ordered]@{
            defaults = [ordered]@{
                heartbeat = [ordered]@{ every = '0m' }
                memorySearch = [ordered]@{ enabled = $false }
            }
        }
        cron = [ordered]@{ enabled = $false }
        discovery = [ordered]@{ mdns = [ordered]@{ mode = 'off' } }
        gateway = [ordered]@{
            auth = [ordered]@{ mode = 'token' }
            bind = 'loopback'
            mode = 'local'
            tailscale = [ordered]@{ mode = 'off' }
            terminal = [ordered]@{ enabled = $false }
        }
        hooks = [ordered]@{ enabled = $false }
        plugins = [ordered]@{
            allow = @('lattice-devos')
            entries = [ordered]@{ 'lattice-devos' = [ordered]@{ enabled = $true } }
            load = [ordered]@{ paths = @($PluginRoot) }
            slots = [ordered]@{ memory = 'none' }
        }
        update = [ordered]@{
            auto = [ordered]@{ enabled = $false }
            checkOnStart = $false
        }
    }
    Write-JsonEvidence -Path (Join-Path $ProfileRoot 'openclaw.json') -Value $profile
    return Get-FileSha256 (Join-Path $ProfileRoot 'openclaw.json')
}

function Start-IsolatedOpenClaw {
    param(
        [Parameter(Mandatory = $true)][string]$NodeExe,
        [Parameter(Mandatory = $true)][string]$OpenClawCli,
        [Parameter(Mandatory = $true)][string]$OpenClawWorkingDirectory,
        [Parameter(Mandatory = $true)][string]$ProfileRoot,
        [Parameter(Mandatory = $true)][string]$GatewayToken,
        [Parameter(Mandatory = $true)][string]$TransportKeyHex,
        [Parameter(Mandatory = $true)][string]$ProcessNonceHex,
        [Parameter(Mandatory = $true)][string]$LaunchRecordId,
        [Parameter(Mandatory = $true)][int]$WsPort,
        [Parameter(Mandatory = $true)][int]$RustPort,
        [Parameter(Mandatory = $true)][string]$StdoutPath,
        [Parameter(Mandatory = $true)][string]$StderrPath
    )

    $environment = @{
        APPDATA = (Join-Path $ProfileRoot 'appdata')
        LOCALAPPDATA = (Join-Path $ProfileRoot 'localappdata')
        HOME = $ProfileRoot
        USERPROFILE = $ProfileRoot
        TEMP = (Join-Path $ProfileRoot 'temp')
        TMP = (Join-Path $ProfileRoot 'temp')
        OPENCLAW_CONFIG_PATH = (Join-Path $ProfileRoot 'openclaw.json')
        OPENCLAW_STATE_DIR = $ProfileRoot
        OPENCLAW_GATEWAY_TOKEN = $GatewayToken
        OPENCLAW_DISABLE_BONJOUR = '1'
        OPENCLAW_SKIP_GMAIL_WATCHER = '1'
        CI = '1'
        NO_COLOR = '1'
        LATTICE_OPENCLAW_AUTH_KEY_HEX = $TransportKeyHex
        LATTICE_OPENCLAW_DEADLINE_MS = '10000'
        LATTICE_OPENCLAW_GATEWAY_PORT = [string]$RustPort
        LATTICE_OPENCLAW_LAUNCH_RECORD_ID = $LaunchRecordId
        LATTICE_OPENCLAW_PROCESS_START_NONCE = $ProcessNonceHex
    }
    $originalEnvironment = @{}
    foreach ($entry in $environment.GetEnumerator()) {
        $originalEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable([string]$entry.Key, 'Process')
    }
    try {
        foreach ($entry in $environment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        return Start-Process -FilePath $NodeExe -ArgumentList @(
            $OpenClawCli,
            'gateway', 'run',
            '--bind', 'loopback',
            '--auth', 'token',
            '--token', $GatewayToken,
            '--port', [string]$WsPort,
            '--ws-log', 'compact'
        ) -WorkingDirectory $OpenClawWorkingDirectory -RedirectStandardOutput $StdoutPath -RedirectStandardError $StderrPath -WindowStyle Hidden -PassThru
    }
    finally {
        foreach ($entry in $environment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, $originalEnvironment[$entry.Key], 'Process')
        }
    }
}

function Get-McpInputBytes {
    param([Parameter(Mandatory = $true)][string]$ToolName)

    $frames = @(
        [ordered]@{
            jsonrpc = '2.0'
            id = 1
            method = 'initialize'
            params = [ordered]@{
                protocolVersion = '2025-11-25'
                capabilities = [ordered]@{}
                clientInfo = [ordered]@{ name = 'task037-full-chain-verifier'; version = '1' }
            }
        },
        [ordered]@{
            jsonrpc = '2.0'
            method = 'notifications/initialized'
            params = [ordered]@{}
        },
        [ordered]@{
            jsonrpc = '2.0'
            id = 2
            method = 'tools/call'
            params = [ordered]@{
                name = $ToolName
                arguments = $taskBinding
            }
        }
    )
    $text = (($frames | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 12 }) -join [Environment]::NewLine) + [Environment]::NewLine
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($text)
    if ($bytes.Length -eq 0 -or $bytes[0] -ne 0x7b) {
        throw 'TASK037_MCP_INPUT_ENCODING_REJECTED'
    }
    return $bytes
}

function Get-StructuredToolContent {
    param([Parameter(Mandatory = $true)]$Response)

    $result = $Response.result
    if ($null -ne $result.PSObject.Properties['structuredContent'] -and $null -ne $result.structuredContent) {
        return $result.structuredContent
    }
    $content = @($result.content)
    if ($content.Count -ne 1 -or [string]$content[0].type -ne 'text') {
        throw 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
    }
    return ([string]$content[0].text) | ConvertFrom-Json
}

function Invoke-PostgresMemoryProbe {
    param(
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$HostName,
        [Parameter(Mandatory = $true)][string]$Port,
        [Parameter(Mandatory = $true)][string]$OutputPath
    )

    $psql = Join-Path $postgresBin 'psql.exe'
    Assert-RegularFile -Path $psql
    $database = 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
    $query = @"
SELECT
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_analyses) AS analyses,
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_receipts) AS receipts,
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_retrieval_audits) AS retrieval_audits,
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_records) AS records,
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_reflections) AS reflections,
  (SELECT count(*)::int FROM ONLY memory.openclaw_gateway_commands) AS openclaw_commands,
  COALESCE((SELECT encode(reflection_receipt_digest, 'hex') FROM ONLY memory.codebase_memory_reflections ORDER BY reflection_receipt_digest LIMIT 1), '') AS reflection_receipt_digest,
  COALESCE((SELECT encode(graph_receipt_digest, 'hex') FROM ONLY memory.codebase_memory_reflections ORDER BY reflection_receipt_digest LIMIT 1), '') AS graph_receipt_digest,
  COALESCE((SELECT reflection_status FROM ONLY memory.codebase_memory_reflections ORDER BY reflection_receipt_digest LIMIT 1), '') AS reflection_status;
"@
    $originalPassword = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $output = @(& $psql '--no-psqlrc' '--quiet' '--csv' '-h' $HostName '-p' $Port '-U' 'task019_harness' '-d' $database '-v' 'ON_ERROR_STOP=1' '-c' $query 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $originalPassword, 'Process')
    }
    if ($exitCode -ne 0) {
        Write-JsonEvidence -Path ($OutputPath + '.failure.json') -Value ([ordered]@{
            schema_version = 'lattice.task037.command-failure.v1'
            command = 'psql-memory-probe'
            code = 'TASK037_MEMORY_PROBE_FAILED'
            output_sha256 = Get-StringSha256 (($output | Out-String))
            output_line_count = @($output).Count
        })
        throw 'TASK037_MEMORY_PROBE_FAILED'
    }
    $rows = @($output | ConvertFrom-Csv)
    if ($rows.Count -ne 1) {
        throw 'TASK037_MEMORY_PROBE_SHAPE_REJECTED'
    }
    $probe = [ordered]@{
        schema_version = 'lattice.task037.memory-probe.v1'
        postgres_run_id = $RunId
        database_name = $database
        analyses = [int]$rows[0].analyses
        receipts = [int]$rows[0].receipts
        retrieval_audits = [int]$rows[0].retrieval_audits
        records = [int]$rows[0].records
        reflections = [int]$rows[0].reflections
        openclaw_commands = [int]$rows[0].openclaw_commands
        reflection_receipt_digest = [string]$rows[0].reflection_receipt_digest
        graph_receipt_digest = [string]$rows[0].graph_receipt_digest
        reflection_status = [string]$rows[0].reflection_status
    }
    Write-JsonEvidence -Path $OutputPath -Value $probe
    return $probe
}

function Reset-PostgresMemoryRows {
    param(
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$HostName,
        [Parameter(Mandatory = $true)][string]$Port,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot
    )

    $before = Invoke-PostgresMemoryProbe `
        -RunId $RunId `
        -Password $Password `
        -HostName $HostName `
        -Port $Port `
        -OutputPath (Join-Path $EvidenceRoot 'memory-reset-before.json')
    $psql = Join-Path $postgresBin 'psql.exe'
    Assert-RegularFile -Path $psql
    $database = 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
    $query = @"
DELETE FROM ONLY memory.codebase_memory_reflections;
DELETE FROM ONLY memory.codebase_memory_receipts;
DELETE FROM ONLY memory.codebase_memory_retrieval_audits;
DELETE FROM ONLY memory.codebase_memory_records;
DELETE FROM ONLY memory.codebase_memory_analyses;
DELETE FROM ONLY memory.openclaw_gateway_commands;
"@
    $originalPassword = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $output = @(& $psql '--no-psqlrc' '--quiet' '-h' $HostName '-p' $Port '-U' 'task019_harness' '-d' $database '-v' 'ON_ERROR_STOP=1' '-c' $query 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $originalPassword, 'Process')
    }
    if ($exitCode -ne 0) {
        Write-JsonEvidence -Path (Join-Path $EvidenceRoot 'memory-reset.failure.json') -Value ([ordered]@{
            schema_version = 'lattice.task037.command-failure.v1'
            command = 'psql-memory-reset'
            code = 'TASK037_MEMORY_RESET_FAILED'
            output_sha256 = Get-StringSha256 (($output | Out-String))
            output_line_count = @($output).Count
        })
        throw 'TASK037_MEMORY_RESET_FAILED'
    }
    $after = Invoke-PostgresMemoryProbe `
        -RunId $RunId `
        -Password $Password `
        -HostName $HostName `
        -Port $Port `
        -OutputPath (Join-Path $EvidenceRoot 'memory-reset-after.json')
    if (
        $after.analyses -ne 0 -or
        $after.receipts -ne 0 -or
        $after.retrieval_audits -ne 0 -or
        $after.records -ne 0 -or
        $after.reflections -ne 0 -or
        $after.openclaw_commands -ne 0
    ) {
        throw 'TASK037_MEMORY_RESET_REJECTED'
    }
    Write-JsonEvidence -Path (Join-Path $EvidenceRoot 'memory-reset.json') -Value ([ordered]@{
        schema_version = 'lattice.task037.memory-reset.v1'
        postgres_run_id = $RunId
        database_name = $database
        reset_scope = @(
            'memory.codebase_memory_reflections',
            'memory.codebase_memory_receipts',
            'memory.codebase_memory_retrieval_audits',
            'memory.codebase_memory_records',
            'memory.codebase_memory_analyses',
            'memory.openclaw_gateway_commands'
        )
        before = $before
        after = $after
    })
}

function Assert-InternalEnvironment {
    $acceptanceId = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_ACCEPTANCE_ID', 'Process')
    $runId = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_RUN_ID', 'Process')
    $hostName = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_HOST', 'Process')
    $port = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PORT', 'Process')
    $password = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PASSWORD', 'Process')
    if (
        [string]::IsNullOrWhiteSpace($acceptanceId) -or
        $acceptanceId -notmatch '^[0-9a-f]{32}$' -or
        $runId -notmatch '^[0-9a-f]{32}$' -or
        $hostName -ne '127.0.0.1' -or
        $port -notmatch '^[0-9]{1,5}$' -or
        [int]$port -eq 0 -or
        [int]$port -eq 5432 -or
        [string]::IsNullOrWhiteSpace($password) -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_LIVE', 'Process') -ne '1' -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PHASE', 'Process') -ne 'restart'
    ) {
        throw 'TASK037_INTERNAL_ENVIRONMENT_REJECTED'
    }
}

function Assert-FullChainSuccess {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$DeliveryRoot
    )

    $repositoryPath = Get-CanonicalPath -Path ([string]$Evidence.repository_path)
    $expectedRepositoryPath = Get-CanonicalPath -Path (Join-Path $DeliveryRoot 'repo')
    $expectedRequestId = 'task032-request-' + $RunId
    if (
        [string]$Evidence.status -ne 'COMPLETED' -or
        [string]$Evidence.entrypoint -ne 'codex-app-mcp' -or
        [string]$Evidence.entrypoint_classification -ne 'official-codex-app-live' -or
        [string]$Evidence.entrypoint_runtime_kind -ne 'Live' -or
        [string]$Evidence.hermes_status -ne 'INFERENCE_CANDIDATE' -or
        [string]$Evidence.hermes_schema_version -ne 'lattice.hermes.reflection.v1' -or
        [string]$Evidence.hermes_provenance_status -ne 'INFERENCE_CANDIDATE' -or
        [string]$Evidence.request_id -ne $expectedRequestId -or
        [string]$Evidence.graph_project_id -ne 'task032-delivery' -or
        -not (Test-ExactPath -Actual $repositoryPath -Expected $expectedRepositoryPath) -or
        [string]$Evidence.full_chain_receipt_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.hermes_reflection_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.hermes_identity_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.hermes_input_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.hermes_graph_receipt_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.graph_receipt_digest -notmatch '^[0-9a-f]{64}$'
    ) {
        throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
    }
    $answerPath = Join-Path $repositoryPath 'answer.txt'
    Assert-RegularFile -Path $answerPath
    $expectedAnswer = [System.Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n")
    $answer = [System.IO.File]::ReadAllBytes($answerPath)
    if ([Convert]::ToBase64String($answer) -ne [Convert]::ToBase64String($expectedAnswer)) {
        throw 'TASK037_DELIVERY_ANSWER_BYTES_REJECTED'
    }
}

function Invoke-FullChainInternalPhase {
    param([Parameter(Mandatory = $true)][string]$Phase)

    Assert-InternalEnvironment
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $acceptanceId = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_ACCEPTANCE_ID', 'Process')
    $runId = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_RUN_ID', 'Process')
    $hostName = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_HOST', 'Process')
    $port = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PORT', 'Process')
    $postgresPassword = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PASSWORD', 'Process')
    $evidenceRoot = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_EVIDENCE_ROOT', 'Process'))
    $fixtureRoot = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_FIXTURE_ROOT', 'Process'))
    $fullChainExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_FULL_CHAIN_EXE', 'Process'))
    $brokerExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_HERMES_BROKER_EXE', 'Process'))
    $runtimeManifest = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_RUNTIME_MANIFEST', 'Process'))
    $deliveryLauncher = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_DELIVERY_LAUNCHER', 'Process'))
    $deliveryCodexHome = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_DELIVERY_CODEX_HOME', 'Process'))
    $schemaDirectory = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_SCHEMA_DIR', 'Process'))
    $deliveryRoot = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_DELIVERY_ROOT', 'Process'))
    $gitExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_GIT_EXE', 'Process'))
    $nodeExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_NODE_EXE', 'Process'))
    $openClawCli = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_OPENCLAW_CLI', 'Process'))
    $codexAuthHome = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CODEX_AUTH_HOME', 'Process'))
    $wslExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_WSL_EXE', 'Process'))

    foreach ($path in @(
        $evidenceRoot, $fixtureRoot, $fullChainExe, $brokerExe, $runtimeManifest,
        $deliveryLauncher, $schemaDirectory, $deliveryRoot, $openClawCli
    )) {
        Assert-NoReparseAncestor -Path $path -Boundary $repositoryRoot
    }
    foreach ($path in @($fullChainExe, $brokerExe, $runtimeManifest, $deliveryLauncher, $gitExe, $nodeExe, $openClawCli, $wslExe)) {
        Assert-RegularFile -Path $path
    }
    if ((Get-FileSha256 $runtimeManifest) -ne $hermesRuntimeManifestSha256) {
        throw 'TASK037_HERMES_RUNTIME_MANIFEST_REJECTED'
    }
    if ((Get-FileSha256 $deliveryLauncher) -ne $officialCodexSha256) {
        throw 'TASK037_DELIVERY_LAUNCHER_REJECTED'
    }
    if ((Get-FileSha256 $openClawCli) -ne $openClawEntrypointSha256) {
        throw 'TASK037_OPENCLAW_ENTRYPOINT_REJECTED'
    }

    $toolName = if ($Phase -eq 'FullChainRun') { 'lattice_delivery_run' } else { 'lattice_delivery_status' }
    $runMode = if ($Phase -eq 'FullChainRun') { 'FRESH' } else { 'RESUME_EXISTING' }
    $expectError = $Phase -eq 'FullChainPreStatus'
    $phaseName = switch ($Phase) {
        'FullChainPreStatus' { 'pre-status' }
        'FullChainRun' { 'run' }
        'FullChainStatus' { 'status' }
    }
    $phaseId = $acceptanceId + '-' + $phaseName + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $tempBase = Get-CanonicalPath -Path ([IO.Path]::GetTempPath())
    Assert-NoReparseAncestor -Path $deliveryCodexHome -Boundary $tempBase
    $profileRoot = Join-Path $tempBase ('lattice-openclaw-' + $phaseId)
    $hermesIsolationRoot = Join-Path $tempBase ('lattice-hermes-' + $phaseId)
    $brokerIsolationRoot = Join-Path $tempBase ('lattice-hermes-broker-' + $phaseId)
    $brokerCodexHome = Join-Path $tempBase ('lattice-hermes-broker-codex-home-' + $phaseId)
    foreach ($freshPath in @($profileRoot, $hermesIsolationRoot, $brokerIsolationRoot, $brokerCodexHome)) {
        if (Test-Path -LiteralPath $freshPath) {
            throw ('TASK037_FRESH_TEMP_REJECTED|' + $freshPath)
        }
    }
    $openClawStdout = Join-Path $evidenceRoot ($phaseName + '.openclaw.stdout.log')
    $openClawStderr = Join-Path $evidenceRoot ($phaseName + '.openclaw.stderr.log')
    $inputPath = Join-Path $evidenceRoot ($phaseName + '.input.ndjson')
    $stdoutPath = Join-Path $evidenceRoot ($phaseName + '.response.ndjson')
    $stderrPath = Join-Path $evidenceRoot ($phaseName + '.stderr.log')
    $metaPath = Join-Path $evidenceRoot ($phaseName + '.meta.json')
    $cleanupPath = Join-Path $evidenceRoot ($phaseName + '.cleanup.json')
    $outputJsonPath = Join-Path $evidenceRoot ($phaseName + '.json')
    $memoryProbePath = Join-Path $evidenceRoot ('memory-' + $phaseName + '.json')

    $openClawProcess = $null
    $fullChainProcess = $null
    $openClawPid = 0
    $fullChainPid = 0
    $wsPort = 0
    $rustPort = 0
    $failure = $null
    $failureEvidence = $null
    $toolIsError = $null
    $toolCode = $null
    $responseDigest = $null
    $phaseSucceeded = $false
    try {
        if ($Phase -eq 'FullChainPreStatus') {
            Reset-PostgresMemoryRows `
                -RunId $runId `
                -Password $postgresPassword `
                -HostName $hostName `
                -Port $port `
                -EvidenceRoot $evidenceRoot
        }
        $pluginRoot = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'integrations\openclaw-lattice')
        $openClawWorkingDirectory = Get-CanonicalPath -Path (Split-Path -Parent $openClawCli)
        $profileSha256 = New-OpenClawProfile -ProfileRoot $profileRoot -PluginRoot $pluginRoot
        $gatewayToken = New-RandomHex -Bytes 32
        $openClawAuthKey = New-RandomBytes -Count 32
        $openClawAttestationKey = New-RandomBytes -Count 32
        $openClawNonce = New-RandomBytes -Count 16
        $sessionEpochBytes = New-RandomBytes -Count 7
        $sessionEpoch = [Convert]::ToUInt64((ConvertTo-LowerHex $sessionEpochBytes), 16)
        if ($sessionEpoch -eq 0) {
            $sessionEpoch = 1
        }
        $launchRecordId = 'lattice-' + $phaseId
        $wsPort = Get-UnreservedLoopbackPort
        do {
            $rustPort = Get-UnreservedLoopbackPort
        } while ($rustPort -eq $wsPort)
        Copy-CodexCredentialFiles -SourceHome $codexAuthHome -DestinationHome $brokerCodexHome -ForBroker

        $openClawProcess = Start-IsolatedOpenClaw `
            -NodeExe $nodeExe `
            -OpenClawCli $openClawCli `
            -OpenClawWorkingDirectory $openClawWorkingDirectory `
            -ProfileRoot $profileRoot `
            -GatewayToken $gatewayToken `
            -TransportKeyHex (ConvertTo-LowerHex $openClawAuthKey) `
            -ProcessNonceHex (ConvertTo-LowerHex $openClawNonce) `
            -LaunchRecordId $launchRecordId `
            -WsPort $wsPort `
            -RustPort $rustPort `
            -StdoutPath $openClawStdout `
            -StderrPath $openClawStderr
        $openClawPid = $openClawProcess.Id
        $null = $openClawProcess.Handle
        $gatewayDeadline = [DateTime]::UtcNow.AddSeconds(90)
        while ([DateTime]::UtcNow -lt $gatewayDeadline) {
            if ($openClawProcess.HasExited) {
                throw ('TASK037_OPENCLAW_GATEWAY_EARLY_EXIT_' + $openClawProcess.ExitCode)
            }
            if (Test-LoopbackPort -Port $wsPort) {
                break
            }
            Start-Sleep -Milliseconds 250
        }
        if (-not (Test-LoopbackPort -Port $wsPort)) {
            throw 'TASK037_OPENCLAW_GATEWAY_NOT_READY'
        }

        $attestationTag = Get-AttestationTag `
            -Key $openClawAttestationKey `
            -LaunchRecordId $launchRecordId `
            -ProcessId ([uint32]$openClawProcess.Id) `
            -ProcessNonce $openClawNonce `
            -ProfileSha256 $profileSha256

        [IO.File]::WriteAllBytes($inputPath, (Get-McpInputBytes -ToolName $toolName))
        $environment = [ordered]@{
            CI = '1'
            NO_COLOR = '1'
            LATTICE_FULL_CHAIN_RUN_MODE = $runMode
            LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
            LATTICE_DELIVERY_LAUNCHER = $deliveryLauncher
            LATTICE_DELIVERY_LAUNCHER_VERSION = $officialCodexVersion
            LATTICE_DELIVERY_LAUNCHER_SHA256 = $officialCodexSha256
            LATTICE_DELIVERY_SCHEMA_DIR = $schemaDirectory
            LATTICE_DELIVERY_CODEX_HOME = $deliveryCodexHome
            LATTICE_DELIVERY_ROOT = $deliveryRoot
            LATTICE_DELIVERY_GIT_EXE = $gitExe
            LATTICE_DELIVERY_TIMEOUT_SECONDS = '300'
            LATTICE_TASK019_HOST = $hostName
            LATTICE_TASK019_PORT = $port
            LATTICE_TASK019_RUN_ID = $runId
            LATTICE_TASK019_PASSWORD = $postgresPassword
            LATTICE_HERMES_RUNTIME_MANIFEST = $runtimeManifest
            LATTICE_HERMES_RUNTIME_GUEST_ROOT = $hermesRuntimeGuestRoot
            LATTICE_HERMES_API_KEY = (New-RandomHex -Bytes 32)
            LATTICE_HERMES_PRODUCT_ROOT = $repositoryRoot
            LATTICE_HERMES_WSL_EXE = $wslExe
            LATTICE_HERMES_ISOLATION_ROOT = $hermesIsolationRoot
            LATTICE_HERMES_BROKER_HELPER = $brokerExe
            LATTICE_HERMES_BROKER_HELPER_SHA256 = (Get-FileSha256 $brokerExe)
            LATTICE_HERMES_CODEX_LAUNCHER = $deliveryLauncher
            LATTICE_HERMES_CODEX_HOME = $brokerCodexHome
            LATTICE_HERMES_BROKER_ISOLATION_ROOT = $brokerIsolationRoot
            LATTICE_HERMES_DEADLINE_SECONDS = '300'
            LATTICE_OPENCLAW_AUTH_KEY_HEX = (ConvertTo-LowerHex $openClawAuthKey)
            LATTICE_OPENCLAW_LAUNCH_ATTESTATION_KEY_HEX = (ConvertTo-LowerHex $openClawAttestationKey)
            LATTICE_OPENCLAW_LAUNCH_ATTESTATION_TAG_HEX = $attestationTag
            LATTICE_OPENCLAW_PROCESS_START_NONCE = (ConvertTo-LowerHex $openClawNonce)
            LATTICE_OPENCLAW_LAUNCH_RECORD_ID = $launchRecordId
            LATTICE_OPENCLAW_PROCESS_ID = [string]$openClawProcess.Id
            LATTICE_OPENCLAW_PACKAGE_TARBALL_SHA256 = $openClawPackageTarballSha256
            LATTICE_OPENCLAW_ENTRYPOINT_SHA256 = $openClawEntrypointSha256
            LATTICE_OPENCLAW_PROFILE_SHA256 = $profileSha256
            LATTICE_OPENCLAW_SESSION_EPOCH = $sessionEpoch.ToString([Globalization.CultureInfo]::InvariantCulture)
            LATTICE_OPENCLAW_GATEWAY_INSTANCE_ID = 'official-' + $phaseId
            LATTICE_OPENCLAW_ACTOR_ID = 'user-' + $phaseId
            LATTICE_OPENCLAW_CHANNEL_ID = 'codex-app'
            LATTICE_OPENCLAW_SESSION_ID = 'agent:main:main'
            LATTICE_OPENCLAW_GATEWAY_PORT = [string]$rustPort
            LATTICE_OPENCLAW_DEADLINE_MS = '10000'
        }
        $originalFullChainEnvironment = @{}
        foreach ($entry in $environment.GetEnumerator()) {
            $originalFullChainEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, 'Process')
            [Environment]::SetEnvironmentVariable($entry.Key, [string]$entry.Value, 'Process')
        }
        try {
            $fullChainProcess = Start-Process -FilePath $fullChainExe `
                -WorkingDirectory $repositoryRoot `
                -RedirectStandardInput $inputPath `
                -RedirectStandardOutput $stdoutPath `
                -RedirectStandardError $stderrPath `
                -WindowStyle Hidden `
                -PassThru
            $null = $fullChainProcess.Handle
        }
        finally {
            foreach ($entry in $environment.GetEnumerator()) {
                [Environment]::SetEnvironmentVariable($entry.Key, $originalFullChainEnvironment[$entry.Key], 'Process')
            }
        }
        $fullChainPid = $fullChainProcess.Id
        $timeoutMilliseconds = if ($Phase -eq 'FullChainRun') { 540000 } else { 360000 }
        if (-not $fullChainProcess.WaitForExit($timeoutMilliseconds)) {
            try { $fullChainProcess.Kill() } catch {}
            $fullChainProcess.WaitForExit()
            throw ('TASK037_' + $Phase.ToUpperInvariant() + '_TIMEOUT')
        }
        $exitCode = $fullChainProcess.ExitCode
        if ($exitCode -ne 0) {
            throw ('TASK037_' + $Phase.ToUpperInvariant() + '_EXIT_' + $exitCode)
        }
        Assert-BoundedTask037EvidenceFile -Path $stdoutPath -Code 'TASK037_FULL_CHAIN_STDOUT_OVERSIZE_REJECTED'
        Assert-BoundedTask037EvidenceFile -Path $stderrPath -Code 'TASK037_FULL_CHAIN_STDERR_OVERSIZE_REJECTED'
        Assert-BoundedTask037EvidenceFile -Path $openClawStdout -Code 'TASK037_OPENCLAW_STDOUT_OVERSIZE_REJECTED'
        Assert-BoundedTask037EvidenceFile -Path $openClawStderr -Code 'TASK037_OPENCLAW_STDERR_OVERSIZE_REJECTED'
        $stdout = [IO.File]::ReadAllText($stdoutPath, [Text.Encoding]::UTF8)
        $stderr = [IO.File]::ReadAllText($stderrPath, [Text.Encoding]::UTF8)
        $responseLines = @($stdout -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $responses = @($responseLines | ForEach-Object { $_ | ConvertFrom-Json })
        if ($responses.Count -ne 2 -or [int]$responses[0].id -ne 1 -or [int]$responses[1].id -ne 2) {
            throw 'TASK037_MCP_RESPONSE_SHAPE_REJECTED'
        }
        $readyLines = @($stderr -split '\r?\n' | Where-Object { $_ -like '*"event":"ready"*' -and $_ -like ('*"endpoint":"127.0.0.1:' + $rustPort + '"*') })
        if ($readyLines.Count -ne 1) {
            throw 'TASK037_FULL_CHAIN_READY_EVIDENCE_REJECTED'
        }
        $toolIsError = [bool]$responses[1].result.isError
        $structured = Get-StructuredToolContent -Response $responses[1]
        $toolCode = if ($null -ne $structured.PSObject.Properties['code']) { [string]$structured.code } else { $null }
        $responseDigest = Get-FileSha256 $stdoutPath
        $meta = [ordered]@{
            schema_version = 'lattice.task037.phase-meta.v1'
            acceptance_id = $acceptanceId
            phase = $Phase
            phase_id = $phaseId
            run_mode = $runMode
            tool_name = $toolName
            postgres_run_id = $runId
            process_exit_code = $exitCode
            tool_is_error = $toolIsError
            tool_code = $toolCode
            response_sha256 = $responseDigest
            openclaw = [ordered]@{
                process_id = $openClawPid
                ws_port = $wsPort
                rust_port = $rustPort
                profile_sha256 = $profileSha256
                package_version = $openClawVersion
                runtime_kind = 'Fake'
            }
            hermes = [ordered]@{
                runtime_manifest_sha256 = Get-FileSha256 $runtimeManifest
                broker_helper_sha256 = Get-FileSha256 $brokerExe
                deadline_seconds = 300
            }
            binary = [ordered]@{
                path = $fullChainExe
                sha256 = Get-FileSha256 $fullChainExe
                bytes = (Get-Item -LiteralPath $fullChainExe).Length
            }
        }
        Write-JsonEvidence -Path $metaPath -Value $meta
        Write-JsonEvidence -Path $outputJsonPath -Value $structured

        if ($expectError) {
            if (-not $toolIsError -or [string]$structured.status -ne 'ERROR' -or $toolCode -notin @('LATTICE_DELIVERY_RECONCILIATION_REQUIRED', 'LATTICE_HERMES_MEMORY_RECEIPT_REJECTED')) {
                throw 'TASK037_PRE_STATUS_FAIL_CLOSED_REJECTED'
            }
            $probe = Invoke-PostgresMemoryProbe -RunId $runId -Password $postgresPassword -HostName $hostName -Port $port -OutputPath $memoryProbePath
            if (
                $probe.analyses -ne 0 -or
                $probe.receipts -ne 0 -or
                $probe.retrieval_audits -ne 0 -or
                $probe.records -ne 0 -or
                $probe.reflections -ne 0 -or
                $probe.openclaw_commands -ne 0
            ) {
                throw 'TASK037_PRE_STATUS_MEMORY_NOT_EMPTY'
            }
            $phaseSucceeded = $true
            return
        }
        if ($toolIsError) {
            throw ('TASK037_' + $Phase.ToUpperInvariant() + '_TOOL_ERROR')
        }
        Assert-FullChainSuccess -Evidence $structured -RunId $runId -DeliveryRoot $deliveryRoot
        $probe = Invoke-PostgresMemoryProbe -RunId $runId -Password $postgresPassword -HostName $hostName -Port $port -OutputPath $memoryProbePath
        if (
            $probe.analyses -ne 1 -or
            $probe.receipts -ne 1 -or
            $probe.retrieval_audits -ne 1 -or
            $probe.records -le 0 -or
            $probe.reflections -ne 1 -or
            $probe.reflection_status -ne 'INFERENCE_CANDIDATE' -or
            $probe.reflection_receipt_digest -ne [string]$structured.full_chain_receipt_digest -or
            $probe.graph_receipt_digest -ne [string]$structured.graph_receipt_digest
        ) {
            throw 'TASK037_MEMORY_PROBE_REJECTED'
        }
        if ($Phase -eq 'FullChainStatus') {
            $runEvidence = Read-JsonEvidence -Path (Join-Path $evidenceRoot 'run.json')
            $runProbe = Read-JsonEvidence -Path (Join-Path $evidenceRoot 'memory-run.json')
            if (
                [string]$structured.full_chain_receipt_digest -ne [string]$runEvidence.full_chain_receipt_digest -or
                [string]$structured.hermes_reflection_digest -ne [string]$runEvidence.hermes_reflection_digest -or
                [string]$structured.hermes_identity_digest -ne [string]$runEvidence.hermes_identity_digest -or
                [string]$structured.hermes_input_digest -ne [string]$runEvidence.hermes_input_digest -or
                [string]$structured.graph_receipt_digest -ne [string]$runEvidence.graph_receipt_digest -or
                $probe.analyses -ne [int]$runProbe.analyses -or
                $probe.receipts -ne [int]$runProbe.receipts -or
                $probe.retrieval_audits -ne [int]$runProbe.retrieval_audits -or
                $probe.records -ne [int]$runProbe.records -or
                $probe.reflections -ne [int]$runProbe.reflections -or
                [string]$probe.reflection_receipt_digest -ne [string]$runProbe.reflection_receipt_digest
            ) {
                throw 'TASK037_STATUS_REPLAY_REJECTED'
            }
            $preStatus = Read-JsonEvidence -Path (Join-Path $evidenceRoot 'pre-status.json')
            $preMeta = Read-JsonEvidence -Path (Join-Path $evidenceRoot 'pre-status.meta.json')
            $final = [ordered]@{
                status = 'PASS'
                component = 'task037-full-chain-verification'
                acceptance_id = $acceptanceId
                postgres_run_id = $runId
                repository_path = Get-CanonicalPath -Path ([string]$structured.repository_path)
                codex_mode = 'OFFICIAL_CODEX_APP_SERVER'
                entrypoint = [string]$structured.entrypoint
                entrypoint_runtime_kind = [string]$structured.entrypoint_runtime_kind
                hermes_status = [string]$structured.hermes_status
                hermes_schema_version = [string]$structured.hermes_schema_version
                full_chain_receipt_digest = [string]$structured.full_chain_receipt_digest
                hermes_reflection_digest = [string]$structured.hermes_reflection_digest
                graph_receipt_digest = [string]$structured.graph_receipt_digest
                memory_reflection_rows = $probe.reflections
                memory_receipt_rows = $probe.receipts
                memory_retrieval_audit_rows = $probe.retrieval_audits
                memory_record_rows = $probe.records
                memory_analysis_rows = $probe.analyses
                status_replayed_after_postgres_restart = $true
                memory_counts_unchanged_during_status = $true
                pre_status_fail_closed = ([bool]$preMeta.tool_is_error -and [string]$preStatus.status -eq 'ERROR')
                pre_status_error_code = [string]$preStatus.code
                run_response_sha256 = Get-FileSha256 (Join-Path $evidenceRoot 'run.response.ndjson')
                status_response_sha256 = Get-FileSha256 (Join-Path $evidenceRoot 'status.response.ndjson')
                runtime_manifest_sha256 = Get-FileSha256 $runtimeManifest
                full_chain_binary_sha256 = Get-FileSha256 $fullChainExe
                broker_binary_sha256 = Get-FileSha256 $brokerExe
            }
            Write-JsonEvidence -Path (Join-Path $evidenceRoot 'final.json') -Value $final
        }
        $phaseSucceeded = $true
    }
    catch {
        $failure = $_.Exception.Message
        $failureEvidence = Get-SafeFailureEvidence -Message $failure
    }
    finally {
        if ($null -ne $fullChainProcess) {
            Stop-OwnedProcessTree -ProcessId $fullChainPid
            $fullChainProcess.Dispose()
        }
        if ($null -ne $openClawProcess) {
            try {
                Stop-OwnedProcessTree -ProcessId $openClawPid
            }
            finally {
                $openClawProcess.Dispose()
            }
        }
        Wait-LoopbackPortClosed -Port $wsPort
        Wait-LoopbackPortClosed -Port $rustPort
        $profileRemoved = Remove-SafeTempRoot -Path $profileRoot
        $brokerCodexHomeRemoved = Remove-SafeTempRoot -Path $brokerCodexHome
        $hermesIsolationRemoved = if ($phaseSucceeded) { Remove-SafeTempRoot -Path $hermesIsolationRoot } else { -not (Test-Path -LiteralPath $hermesIsolationRoot) }
        $brokerIsolationRemoved = if ($phaseSucceeded) { Remove-SafeTempRoot -Path $brokerIsolationRoot } else { -not (Test-Path -LiteralPath $brokerIsolationRoot) }
        $logEvidence = @(
            Protect-Task037LogEvidence -Path $openClawStdout
            Protect-Task037LogEvidence -Path $openClawStderr
            Protect-Task037LogEvidence -Path $stdoutPath
            Protect-Task037LogEvidence -Path $stderrPath
        )
        $cleanup = [ordered]@{
            schema_version = 'lattice.task037.cleanup.v1'
            acceptance_id = $acceptanceId
            phase = $Phase
            full_chain_process_stopped = if ($fullChainPid -eq 0) { $true } else { $null -eq (Get-Process -Id $fullChainPid -ErrorAction SilentlyContinue) }
            openclaw_process_stopped = if ($openClawPid -eq 0) { $true } else { $null -eq (Get-Process -Id $openClawPid -ErrorAction SilentlyContinue) }
            ws_listener_count = if ($wsPort -eq 0) { 0 } else { @(Get-NetTCPConnection -LocalPort $wsPort -State Listen -ErrorAction SilentlyContinue).Count }
            rust_listener_count = if ($rustPort -eq 0) { 0 } else { @(Get-NetTCPConnection -LocalPort $rustPort -State Listen -ErrorAction SilentlyContinue).Count }
            openclaw_profile_removed = $profileRemoved
            broker_codex_home_removed = $brokerCodexHomeRemoved
            hermes_isolation_removed_on_success = $hermesIsolationRemoved
            broker_isolation_removed_on_success = $brokerIsolationRemoved
            failure = $failureEvidence
            retained_log_evidence = $logEvidence
        }
        Write-JsonEvidence -Path $cleanupPath -Value $cleanup
    }
    if ($null -ne $failure) {
        Throw-Task037SafeFailure -Message $failure
    }
}

function Invoke-DefaultVerification {
    if ([string]::IsNullOrWhiteSpace($CodexAuthHome)) {
        $CodexAuthHome = Join-Path $env:USERPROFILE '.codex'
    }
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
    $acceptanceId = [Guid]::NewGuid().ToString('N')
    $acceptanceRoot = Get-CanonicalPath -Path (Join-Path $repositoryTarget ('full-chain-acceptance\' + $acceptanceId))
    $fixtureRoot = Get-CanonicalPath -Path (Join-Path $repositoryTarget ('lattice-delivery\' + $acceptanceId))
    $evidenceRoot = Join-Path $acceptanceRoot 'evidence'
    $runtimeManifest = Join-Path $acceptanceRoot 'offline-runtime-manifest.json'
    $sourceManifest = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'full-chain-acceptance\4fe82cdeaa154c28bbe18f6a\offline-runtime-manifest.json')
    $tempBase = Get-CanonicalPath -Path ([IO.Path]::GetTempPath())
    $deliveryCodexHome = Join-Path $tempBase ('lattice-task037-delivery-codex-home-' + $acceptanceId)
    $schemaDirectory = Join-Path $fixtureRoot 'schema'
    $deliveryRoot = Join-Path $fixtureRoot 'delivery'
    $deliveryLauncher = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe')
    $openClawCli = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'integrations\openclaw-lattice\node_modules\openclaw\openclaw.mjs')
    $wslExe = Get-CanonicalPath -Path (Join-Path $env:SystemRoot 'System32\wsl.exe')
    $git = @(Get-Command 'git.exe' -CommandType Application -ErrorAction Stop)[0]
    $node = @(Get-Command 'node.exe' -CommandType Application -ErrorAction Stop)[0]
    $cargo = @(Get-Command 'cargo.exe' -CommandType Application -ErrorAction Stop)[0]

    foreach ($path in @($acceptanceRoot, $fixtureRoot)) {
        if (Test-Path -LiteralPath $path) {
            throw ('TASK037_FRESH_ROOT_REJECTED|' + $path)
        }
    }
    foreach ($path in @($repositoryTarget, $sourceManifest, $deliveryLauncher, $openClawCli)) {
        Assert-NoReparseAncestor -Path $path -Boundary $repositoryRoot
    }
    Assert-NoReparseAncestor -Path $deliveryCodexHome -Boundary $tempBase
    foreach ($path in @($sourceManifest, $deliveryLauncher, $openClawCli, $wslExe, $git.Source, $node.Source, $cargo.Source)) {
        Assert-RegularFile -Path $path
    }
    if ((Get-FileSha256 $sourceManifest) -ne $hermesRuntimeManifestSha256) {
        throw 'TASK037_SOURCE_MANIFEST_REJECTED'
    }
    if ((Get-FileSha256 $deliveryLauncher) -ne $officialCodexSha256) {
        throw 'TASK037_OFFICIAL_CODEX_REJECTED'
    }
    if ((Get-FileSha256 $openClawCli) -ne $openClawEntrypointSha256) {
        throw 'TASK037_OPENCLAW_REJECTED'
    }

    Push-Location $repositoryRoot
    try {
        $branch = Invoke-Task037GitText -GitExe $git.Source -Arguments @('rev-parse', '--abbrev-ref', 'HEAD') -FailureCode 'TASK037_GIT_BRANCH_FAILED'
        $head = Invoke-Task037GitText -GitExe $git.Source -Arguments @('rev-parse', 'HEAD') -FailureCode 'TASK037_GIT_HEAD_FAILED'
        $status = Invoke-Task037GitText -GitExe $git.Source -Arguments @('status', '--short') -FailureCode 'TASK037_GIT_STATUS_FAILED'
    }
    finally {
        Pop-Location
    }
    if (-not [string]::IsNullOrWhiteSpace($status)) {
        throw 'TASK037_WORKTREE_DIRTY_REJECTED'
    }
    if (Test-Path -LiteralPath $deliveryCodexHome) {
        throw ('TASK037_FRESH_TEMP_REJECTED|' + $deliveryCodexHome)
    }

    New-Item -ItemType Directory -Path $acceptanceRoot -Force:$false | Out-Null
    New-Item -ItemType Directory -Path $evidenceRoot -Force:$false | Out-Null
    New-Item -ItemType Directory -Path $fixtureRoot -Force:$false | Out-Null
    Copy-Item -LiteralPath $sourceManifest -Destination $runtimeManifest -Force:$false

    Push-Location $repositoryRoot
    try {
        & $cargo.Source 'build' '-p' 'lattice-runtime' '--bin' 'lattice-full-chain' '--locked'
        if ($LASTEXITCODE -ne 0) {
            throw 'TASK037_FULL_CHAIN_BUILD_FAILED'
        }
        & $cargo.Source 'build' '-p' 'lattice-hermes-adapter' '--bin' 'lattice-hermes-broker' '--locked'
        if ($LASTEXITCODE -ne 0) {
            throw 'TASK037_HERMES_BROKER_BUILD_FAILED'
        }
        $metadataText = (& $cargo.Source 'metadata' '--no-deps' '--format-version' '1' '--locked') -join "`n"
        if ($LASTEXITCODE -ne 0) {
            throw 'TASK037_CARGO_METADATA_FAILED'
        }
    }
    finally {
        Pop-Location
    }
    $metadata = $metadataText | ConvertFrom-Json
    $targetDirectory = Get-CanonicalPath -Path ([string]$metadata.target_directory)
    $fullChainExe = Get-CanonicalPath -Path (Join-Path $targetDirectory 'debug\lattice-full-chain.exe')
    $brokerExe = Get-CanonicalPath -Path (Join-Path $targetDirectory 'debug\lattice-hermes-broker.exe')
    Assert-RegularFile -Path $fullChainExe
    Assert-RegularFile -Path $brokerExe

    $params = [ordered]@{
        schema_version = 'lattice.task037.params.v1'
        acceptance_id = $acceptanceId
        branch = $branch.Trim()
        head = $head.Trim()
        working_tree_clean_at_start = $true
        repository_root = $repositoryRoot
        acceptance_root = $acceptanceRoot
        evidence_root = $evidenceRoot
        fixture_root = $fixtureRoot
        delivery_root = $deliveryRoot
        delivery_codex_home = $deliveryCodexHome
        delivery_codex_home_scope = 'temp'
        runtime_manifest_sha256 = Get-FileSha256 $runtimeManifest
        full_chain_binary_sha256 = Get-FileSha256 $fullChainExe
        broker_binary_sha256 = Get-FileSha256 $brokerExe
        delivery_launcher_sha256 = Get-FileSha256 $deliveryLauncher
        openclaw_entrypoint_sha256 = Get-FileSha256 $openClawCli
        codex_auth_source = Get-CanonicalPath -Path $CodexAuthHome
    }
    Write-JsonEvidence -Path (Join-Path $acceptanceRoot 'params.json') -Value $params

    $environmentValues = [ordered]@{
        LATTICE_TASK037_ACCEPTANCE_ID = $acceptanceId
        LATTICE_TASK037_EVIDENCE_ROOT = $evidenceRoot
        LATTICE_TASK037_FIXTURE_ROOT = $fixtureRoot
        LATTICE_TASK037_FULL_CHAIN_EXE = $fullChainExe
        LATTICE_TASK037_HERMES_BROKER_EXE = $brokerExe
        LATTICE_TASK037_RUNTIME_MANIFEST = $runtimeManifest
        LATTICE_TASK037_DELIVERY_LAUNCHER = $deliveryLauncher
        LATTICE_TASK037_DELIVERY_CODEX_HOME = $deliveryCodexHome
        LATTICE_TASK037_SCHEMA_DIR = $schemaDirectory
        LATTICE_TASK037_DELIVERY_ROOT = $deliveryRoot
        LATTICE_TASK037_GIT_EXE = Get-CanonicalPath -Path $git.Source
        LATTICE_TASK037_NODE_EXE = Get-CanonicalPath -Path $node.Source
        LATTICE_TASK037_OPENCLAW_CLI = $openClawCli
        LATTICE_TASK037_CODEX_AUTH_HOME = Get-CanonicalPath -Path $CodexAuthHome
        LATTICE_TASK037_WSL_EXE = $wslExe
    }
    $originalEnvironment = @{}
    $driverFailure = $null
    $deliveryCodexHomeRemoved = $false
    foreach ($entry in $environmentValues.GetEnumerator()) {
        $originalEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, 'Process')
    }
    try {
        Copy-CodexCredentialFiles -SourceHome $CodexAuthHome -DestinationHome $deliveryCodexHome
        foreach ($entry in $environmentValues.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable($entry.Key, [string]$entry.Value, 'Process')
        }
        $postgresHarness = Get-CanonicalPath -Path (Join-Path $PSScriptRoot 'run-task019-postgres.ps1')
        Assert-RegularFile -Path $postgresHarness
        & $postgresHarness -RunFullChainAcceptanceHook
    }
    catch {
        $driverFailure = $_.Exception.Message
        Throw-Task037SafeFailure -Message $driverFailure
    }
    finally {
        foreach ($entry in $environmentValues.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable($entry.Key, $originalEnvironment[$entry.Key], 'Process')
        }
        $deliveryCodexHomeRemoved = Remove-SafeTempRoot -Path $deliveryCodexHome
        Write-JsonEvidence -Path (Join-Path $evidenceRoot 'driver.cleanup.json') -Value ([ordered]@{
            schema_version = 'lattice.task037.driver-cleanup.v1'
            acceptance_id = $acceptanceId
            delivery_codex_home = $deliveryCodexHome
            delivery_codex_home_removed = $deliveryCodexHomeRemoved
            failure = Get-SafeFailureEvidence -Message $driverFailure
        })
    }
    if (-not $deliveryCodexHomeRemoved) {
        throw 'TASK037_DELIVERY_CODEX_HOME_CLEANUP_REJECTED'
    }

    $finalPath = Join-Path $evidenceRoot 'final.json'
    $final = Read-JsonEvidence -Path $finalPath
    if (
        [string]$final.status -ne 'PASS' -or
        [string]$final.component -ne 'task037-full-chain-verification' -or
        [string]$final.acceptance_id -ne $acceptanceId -or
        [bool]$final.status_replayed_after_postgres_restart -ne $true -or
        [bool]$final.memory_counts_unchanged_during_status -ne $true -or
        [bool]$final.pre_status_fail_closed -ne $true
    ) {
        throw 'TASK037_FINAL_EVIDENCE_REJECTED'
    }

    Write-Output 'TASK037_FULL_CHAIN_VERIFICATION=PASS'
    Write-Output (([ordered]@{
        status = 'PASS'
        component = 'task037-full-chain-verification'
        acceptance_id = $acceptanceId
        evidence_path = $finalPath
        postgres_run_id = [string]$final.postgres_run_id
        full_chain_receipt_digest = [string]$final.full_chain_receipt_digest
        hermes_reflection_digest = [string]$final.hermes_reflection_digest
    }) | ConvertTo-Json -Compress)
}

if ($HarnessSelfTest) {
    Invoke-Task037HarnessSelfTest
    return
}

if (-not [string]::IsNullOrEmpty($InternalPhase)) {
    Invoke-FullChainInternalPhase -Phase $InternalPhase
    return
}

Invoke-DefaultVerification
