[CmdletBinding()]
param(
    [ValidateSet('Init', 'Doctor', 'Run', 'ManagedRun')]
    [string]$Mode = 'Doctor',
    [Parameter(Mandatory = $true)]
    [string]$TunnelClientExecutable,
    [Parameter(Mandatory = $true)]
    [string]$ProfileDirectory,
    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,63}$')]
    [string]$ProfileName = 'lattice-local',
    [string]$TunnelId,
    [string]$LatticedExecutable,
    [string]$OfficialCodexExecutable,
    [string]$CodexAuthHome,
    [string]$PsqlExecutable,
    [string]$PostgresDataDirectory,
    [string]$ManagedEvidenceDirectory,
    [string]$CanonicalAcceptanceReceipt,
    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string]$CanonicalAcceptanceReceiptSha256
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$nativeIdentityHelperPath = Join-Path $PSScriptRoot 'windows-native-path-identity.ps1'
$nativeIdentityHelperItem = Get-Item -LiteralPath $nativeIdentityHelperPath -Force -ErrorAction SilentlyContinue
if (
    $null -eq $nativeIdentityHelperItem -or
    $nativeIdentityHelperItem.PSIsContainer -or
    -not ($nativeIdentityHelperItem -is [IO.FileInfo]) -or
    ($nativeIdentityHelperItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
) {
    throw 'TASK038_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}
try {
    $nativeIdentityHelperSource = [Text.UTF8Encoding]::new($false, $true).GetString(
        [IO.File]::ReadAllBytes($nativeIdentityHelperItem.FullName)
    )
    . ([scriptblock]::Create($nativeIdentityHelperSource))
    Initialize-LatticeWindowsNativePathIdentity
}
catch {
    throw 'TASK038_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}

$script:ManagedSecretValues = [Collections.Generic.List[string]]::new()
$script:ManagedCodexAuthPath = $null
$script:ManagedCodexHome = $null
$script:ManagedRuntimeId = $null
$script:ManagedClusterRoot = $null
$script:ManagedCredentialSourceAuth = $null
$script:ManagedCredentialSourceSha256 = $null

function Get-CanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return [IO.Path]::GetFullPath($Path).TrimEnd([char[]]@(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    ))
}

function Test-ExactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    return [string]::Equals(
        (Get-CanonicalPath -Path $Actual),
        (Get-CanonicalPath -Path $Expected),
        [StringComparison]::OrdinalIgnoreCase
    )
}

function Test-PathOverlap {
    param(
        [Parameter(Mandatory = $true)][string]$Left,
        [Parameter(Mandatory = $true)][string]$Right
    )

    $canonicalLeft = Get-CanonicalPath -Path $Left
    $canonicalRight = Get-CanonicalPath -Path $Right
    return (
        (Test-ExactPath -Actual $canonicalLeft -Expected $canonicalRight) -or
        $canonicalLeft.StartsWith(($canonicalRight + [IO.Path]::DirectorySeparatorChar), [StringComparison]::OrdinalIgnoreCase) -or
        $canonicalRight.StartsWith(($canonicalLeft + [IO.Path]::DirectorySeparatorChar), [StringComparison]::OrdinalIgnoreCase)
    )
}

function Assert-NoReparsePath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $current = Get-CanonicalPath -Path $Path
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw $FailureCode
            }
        }
        $parentInfo = [IO.Directory]::GetParent($current)
        if ($null -eq $parentInfo) { break }
        $parent = $parentInfo.FullName
        if ([string]::Equals($parent, $current, [StringComparison]::OrdinalIgnoreCase)) { break }
        $current = $parent
    }
}

function Resolve-RequiredLeafPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if (-not [IO.Path]::IsPathRooted($Path)) {
        throw $FailureCode
    }
    $resolved = [IO.Path]::GetFullPath($Path)
    $item = Get-Item -LiteralPath $resolved -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [IO.FileInfo]) -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw $FailureCode
    }
    return $resolved
}

function Get-FileSha256 {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    try {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
    }
    catch {
        throw $FailureCode
    }
}

function Get-StringSha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Remove-ManagedCodexAuthCredential {
    param(
        [Parameter(Mandatory = $true)][string]$ClusterRoot,
        [Parameter(Mandatory = $true)][string]$CodexHome,
        [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{32}$')][string]$RuntimeId
    )

    $canonicalCluster = Get-CanonicalPath -Path $ClusterRoot
    $canonicalHome = Get-CanonicalPath -Path $CodexHome
    $expectedHome = Get-CanonicalPath -Path (Join-Path $canonicalCluster 'task038-tunnel-codex-home')
    if (
        -not (Test-ExactPath -Actual $canonicalHome -Expected $expectedHome) -or
        -not (Test-ExactPath -Actual (Split-Path -Parent $canonicalHome) -Expected $canonicalCluster)
    ) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
    Assert-NoReparsePath -Path $canonicalHome -FailureCode 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    $homeItem = Get-Item -LiteralPath $canonicalHome -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $homeItem -or
        -not $homeItem.PSIsContainer -or
        -not ($homeItem -is [IO.DirectoryInfo]) -or
        ($homeItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
    $homeMarker = Join-Path $canonicalHome '.lattice-codex-home-v1'
    $ownerMarker = Join-Path $canonicalHome '.lattice-task038-tunnel-owner-v1'
    foreach ($marker in @($homeMarker, $ownerMarker)) {
        $markerItem = Get-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
        if (
            $null -eq $markerItem -or
            $markerItem.PSIsContainer -or
            -not ($markerItem -is [IO.FileInfo]) -or
            ($markerItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
        ) {
            throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
        }
    }
    $homeContainment = New-LatticeWindowsNativeContainmentSnapshot `
        -ParentPath $canonicalCluster `
        -RootPath $canonicalHome `
        -MarkerPath $homeMarker
    $ownerMarkerIdentity = Get-LatticeWindowsNativePathIdentityToken `
        -Path $ownerMarker `
        -Directory $false
    if (
        [IO.File]::ReadAllText($homeMarker, [Text.Encoding]::UTF8) -cne "lattice.codex-home.v1`n" -or
        [IO.File]::ReadAllText($ownerMarker, [Text.Encoding]::UTF8) -cne
            ('lattice.task038-tunnel-home.v1:' + $RuntimeId + "`n")
    ) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
    $authPath = Join-Path $canonicalHome 'auth.json'
    if (-not (Test-Path -LiteralPath $authPath)) { return }
    Assert-NoReparsePath -Path $authPath -FailureCode 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    $authItem = Get-Item -LiteralPath $authPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $authItem -or
        $authItem.PSIsContainer -or
        -not ($authItem -is [IO.FileInfo]) -or
        ($authItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
    $authIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $authPath -Directory $false
    Assert-LatticeWindowsNativeContainmentSnapshot `
        -Snapshot $homeContainment `
        -FailureCode 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    if (
        -not (Test-LatticeWindowsNativePathIdentity `
            -Path $ownerMarker `
            -Directory $false `
            -ExpectedToken $ownerMarkerIdentity) -or
        -not (Test-LatticeWindowsNativePathIdentity `
            -Path $authPath `
            -Directory $false `
            -ExpectedToken $authIdentity)
    ) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
    [IO.File]::Delete($authPath)
    if (Test-Path -LiteralPath $authPath) {
        throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
    }
}

function Get-ByteArraySha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Initialize-Task038Utf8DiagnosticType {
    if ($null -ne ('Lattice.Task038.Utf8DiagnosticDecoder' -as [type])) { return }

    Add-Type -TypeDefinition @'
using System;
using System.Text;

namespace Lattice.Task038
{
    public sealed class Utf8DiagnosticResult
    {
        public string Text { get; private set; }
        public int ReplacementCount { get; private set; }
        public int FirstInvalidByteOffset { get; private set; }

        public Utf8DiagnosticResult(string text, int replacementCount, int firstInvalidByteOffset)
        {
            Text = text;
            ReplacementCount = replacementCount;
            FirstInvalidByteOffset = firstInvalidByteOffset;
        }
    }

    internal sealed class CountingDecoderFallback : DecoderFallback
    {
        private int count;
        private int firstOffset = -1;

        public int Count { get { return count; } }
        public int FirstOffset { get { return firstOffset; } }
        public override int MaxCharCount { get { return 1; } }

        internal void Record(int index)
        {
            if (firstOffset < 0) { firstOffset = index; }
            count++;
        }

        public override DecoderFallbackBuffer CreateFallbackBuffer()
        {
            return new CountingDecoderFallbackBuffer(this);
        }
    }

    internal sealed class CountingDecoderFallbackBuffer : DecoderFallbackBuffer
    {
        private readonly CountingDecoderFallback owner;
        private bool pending;

        public CountingDecoderFallbackBuffer(CountingDecoderFallback owner)
        {
            this.owner = owner;
        }

        public override bool Fallback(byte[] bytesUnknown, int index)
        {
            if (pending) { return false; }
            owner.Record(index);
            pending = true;
            return true;
        }

        public override char GetNextChar()
        {
            if (!pending) { return '\0'; }
            pending = false;
            return '\ufffd';
        }

        public override bool MovePrevious()
        {
            if (pending) { return false; }
            pending = true;
            return true;
        }

        public override int Remaining { get { return pending ? 1 : 0; } }

        public override void Reset()
        {
            pending = false;
            base.Reset();
        }
    }

    public static class Utf8DiagnosticDecoder
    {
        public static Utf8DiagnosticResult Decode(byte[] bytes)
        {
            if (bytes == null) { throw new ArgumentNullException("bytes"); }
            CountingDecoderFallback fallback = new CountingDecoderFallback();
            Encoding encoding = (Encoding)new UTF8Encoding(false, false).Clone();
            encoding.DecoderFallback = fallback;
            Decoder decoder = encoding.GetDecoder();
            char[] characters = new char[bytes.Length];
            int characterCount = decoder.GetChars(bytes, 0, bytes.Length, characters, 0, true);
            string text = new string(characters, 0, characterCount);
            return new Utf8DiagnosticResult(text, fallback.Count, fallback.FirstOffset);
        }
    }
}
'@ -Language CSharp -ErrorAction Stop
}

function Get-ClosedTunnelChildPath {
    $windows = [IO.Path]::GetFullPath(
        [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
    )
    $directories = @(
        $windows,
        (Join-Path $windows 'System32'),
        (Join-Path $windows 'System32\Wbem'),
        (Join-Path $windows 'System32\WindowsPowerShell\v1.0')
    )
    $resolved = [Collections.Generic.List[string]]::new()
    foreach ($directory in $directories) {
        $item = Get-Item -LiteralPath $directory -Force -ErrorAction SilentlyContinue
        if (
            $null -eq $item -or
            -not $item.PSIsContainer -or
            -not ($item -is [IO.DirectoryInfo]) -or
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
        ) {
            throw 'TASK038_TUNNEL_CLOSED_PATH_REJECTED'
        }
        foreach ($leaf in @('codex.exe', 'codex.cmd', 'codex.bat', 'codex.com')) {
            if (Test-Path -LiteralPath (Join-Path $item.FullName $leaf)) {
                throw 'TASK038_TUNNEL_CLOSED_PATH_REJECTED'
            }
        }
        $resolved.Add([IO.Path]::GetFullPath($item.FullName))
    }
    return ($resolved -join [IO.Path]::PathSeparator)
}

function Get-ManagedRuntimeEnvironmentDigest {
    param([Parameter(Mandatory = $true)][Collections.IDictionary]$Environment)

    $lines = [Collections.Generic.List[string]]::new()
    foreach ($name in @($Environment.Keys | Sort-Object -CaseSensitive)) {
        $value = [string]$Environment[$name]
        if (
            [string]::IsNullOrWhiteSpace([string]$name) -or
            $value.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0
        ) {
            throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        }
        $lines.Add(([string]$name + '=' + $value))
    }
    return Get-StringSha256 -Value ($lines -join "`n")
}

function Test-ManagedTunnelReconnectRecords {
    param(
        [Parameter(Mandatory = $true)][object[]]$Records,
        [Parameter(Mandatory = $true)][string]$ExpectedCommand,
        [Parameter(Mandatory = $true)][string]$ExpectedTunnelId
    )

    $health = [Collections.Generic.List[int]]::new()
    $pidWritten = [Collections.Generic.List[int]]::new()
    $started = [Collections.Generic.List[int]]::new()
    $deadline = [Collections.Generic.List[int]]::new()
    $ttl = [Collections.Generic.List[int]]::new()
    $exited = [Collections.Generic.List[int]]::new()
    $shutdown = [Collections.Generic.List[int]]::new()
    $allShutdownRequests = 0
    $clientInstanceId = $null
    $requestId = $null
    for ($index = 0; $index -lt $Records.Count; $index++) {
        $record = $Records[$index]
        if ($null -eq $record -or $record.PSObject.Properties.Name -notcontains 'msg') {
            return $false
        }
        $message = [string]$record.msg
        $recordClientInstanceId = [string]$record.client_instance_id
        $recordTunnelId = [string]$record.tunnel_id
        if (
            $recordClientInstanceId -cnotmatch '^[0-9a-f]{32}$' -or
            $recordTunnelId -cne $ExpectedTunnelId
        ) {
            return $false
        }
        if ($null -eq $clientInstanceId) { $clientInstanceId = $recordClientInstanceId }
        if ($recordClientInstanceId -cne $clientInstanceId) { return $false }
        if ($message -eq 'stdio MCP command failed; requesting tunnel-client shutdown') {
            $allShutdownRequests++
        }
        switch -CaseSensitive ($message) {
            'health server listening' {
                if ([string]$record.component -ceq 'health') { $health.Add($index) }
            }
            'pid file written' {
                if ([string]$record.component -ceq 'process' -and [int]$record.pid -gt 0) {
                    $pidWritten.Add($index)
                }
            }
            'stdio MCP command started' {
                if (
                    [string]$record.component -ceq 'mcpclient' -and
                    [string]$record.transport -ceq 'stdio' -and
                    [string]$record.command -ceq $ExpectedCommand
                ) { $started.Add($index) }
            }
            'command response deadline reached; dropping without posting a response' {
                if (
                    [string]$record.component -ceq 'dispatcher' -and
                    [string]$record.request_id -cnotmatch '^\s*$'
                ) {
                    $deadline.Add($index)
                    $requestId = [string]$record.request_id
                }
            }
            'MCP connection TTL reached; stopping response forwarding' {
                if (
                    [string]$record.component -ceq 'dispatcher' -and
                    [string]$record.request_id -cnotmatch '^\s*$'
                ) { $ttl.Add($index) }
            }
            'stdio MCP command exited' {
                if (
                    [string]$record.component -ceq 'mcpclient' -and
                    [string]$record.transport -ceq 'stdio' -and
                    [string]$record.command -ceq $ExpectedCommand
                ) { $exited.Add($index) }
            }
            'stdio MCP command failed; requesting tunnel-client shutdown' {
                if (
                    [string]$record.component -ceq 'mcpclient' -and
                    [string]$record.transport -ceq 'stdio' -and
                    [string]$record.reason -ceq 'stdio MCP command exited' -and
                    [string]$record.command -ceq $ExpectedCommand
                ) {
                    $shutdown.Add($index)
                }
            }
        }
    }
    return (
        $health.Count -eq 1 -and
        $pidWritten.Count -eq 1 -and
        $started.Count -eq 1 -and
        $deadline.Count -eq 1 -and
        $ttl.Count -eq 1 -and
        $exited.Count -eq 1 -and
        $shutdown.Count -eq 1 -and
        $allShutdownRequests -eq 1 -and
        [string]$Records[$ttl[0]].request_id -ceq $requestId -and
        $health[0] -lt $deadline[0] -and
        $pidWritten[0] -lt $deadline[0] -and
        $started[0] -lt $deadline[0] -and
        $started[0] -lt $ttl[0] -and
        $deadline[0] -lt $exited[0] -and
        $ttl[0] -lt $exited[0] -and
        $exited[0] -lt $shutdown[0]
    )
}

function Test-ManagedTunnelReconnectLog {
    param(
        [Parameter(Mandatory = $true)]$LogDiagnostic,
        [Parameter(Mandatory = $true)][string]$ExpectedCommand,
        [Parameter(Mandatory = $true)][string]$ExpectedTunnelId
    )

    if (
        $null -eq $LogDiagnostic -or
        $LogDiagnostic.PSObject.Properties.Name -notcontains 'canonical_text' -or
        $LogDiagnostic.PSObject.Properties.Name -notcontains 'raw_sha256' -or
        [string]$LogDiagnostic.raw_sha256 -cnotmatch '^[0-9a-f]{64}$'
    ) {
        return $false
    }
    $text = [string]$LogDiagnostic.canonical_text
    Assert-ManagedSecretFreeText -Text $text
    try {
        $records = @(ConvertFrom-ManagedTunnelJsonLines `
            -Text $text `
            -FailureCode 'TASK038_TUNNEL_SESSION_LOG_JSON_REJECTED')
    }
    catch { return $false }
    if ($records.Count -lt 5) { return $false }
    return Test-ManagedTunnelReconnectRecords `
        -Records @($records) `
        -ExpectedCommand $ExpectedCommand `
        -ExpectedTunnelId $ExpectedTunnelId
}

function Get-ManagedTunnelLogDiagnostic {
    param(
        [Parameter(Mandatory = $true)][string]$LogPath,
        [string]$DiagnosticPath = '',
        [string]$DiagnosticRoot = ''
    )

    $item = Get-Item -LiteralPath $LogPath -Force -ErrorAction SilentlyContinue
    if ($null -eq $item) { throw 'TASK038_TUNNEL_SESSION_LOG_MISSING' }
    if (
        $item.PSIsContainer -or
        -not ($item -is [IO.FileInfo]) -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) { throw 'TASK038_TUNNEL_SESSION_LOG_PATH_REJECTED' }
    if ($item.Length -lt 1) { throw 'TASK038_TUNNEL_SESSION_LOG_EMPTY' }
    if ($item.Length -gt 16777216) { throw 'TASK038_TUNNEL_SESSION_LOG_TOO_LARGE' }

    $rawStream = $null
    $rawBytes = $null
    $readFailureCode = ''
    try {
        $rawStream = [IO.FileStream]::new(
            $item.FullName,
            [IO.FileMode]::Open,
            [IO.FileAccess]::Read,
            ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
        )
        $snapshotLength = [long]$rawStream.Length
        if ($snapshotLength -lt 1 -or $snapshotLength -gt 16777216) {
            $readFailureCode = 'TASK038_TUNNEL_SESSION_LOG_SIZE_CHANGED'
        }
        else {
            $rawBytes = [byte[]]::new([int]$snapshotLength)
            $offset = 0
            while ($offset -lt $rawBytes.Length) {
                $read = $rawStream.Read($rawBytes, $offset, $rawBytes.Length - $offset)
                if ($read -lt 1) {
                    $readFailureCode = 'TASK038_TUNNEL_SESSION_LOG_SIZE_CHANGED'
                    break
                }
                $offset += $read
            }
            if (
                [string]::IsNullOrEmpty($readFailureCode) -and
                [long]$rawStream.Length -ne $snapshotLength
            ) {
                $readFailureCode = 'TASK038_TUNNEL_SESSION_LOG_SIZE_CHANGED'
            }
        }
    }
    catch {
        $readFailureCode = 'TASK038_TUNNEL_SESSION_LOG_READ_REJECTED'
    }
    finally {
        if ($null -ne $rawStream) { $rawStream.Dispose() }
    }
    if (-not [string]::IsNullOrEmpty($readFailureCode)) {
        throw $readFailureCode
    }

    if (
        $rawBytes.Length -ge 3 -and
        $rawBytes[0] -eq 0xef -and
        $rawBytes[1] -eq 0xbb -and
        $rawBytes[2] -eq 0xbf
    ) {
        throw 'TASK038_TUNNEL_SESSION_LOG_BOM_REJECTED'
    }
    try {
        $decoded = [Lattice.Task038.Utf8DiagnosticDecoder]::Decode($rawBytes)
        $canonicalText = [string]$decoded.Text
        $replacementCount = [int]$decoded.ReplacementCount
        $firstInvalidByteOffset = [int]$decoded.FirstInvalidByteOffset
    }
    catch {
        throw 'TASK038_TUNNEL_SESSION_LOG_CANONICALIZATION_REJECTED'
    }
    if (
        $replacementCount -lt 0 -or
        ($replacementCount -eq 0 -and $firstInvalidByteOffset -ne -1) -or
        ($replacementCount -gt 0 -and ($firstInvalidByteOffset -lt 0 -or $firstInvalidByteOffset -ge $rawBytes.Length))
    ) {
        throw 'TASK038_TUNNEL_SESSION_LOG_CANONICALIZATION_REJECTED'
    }

    Assert-ManagedSecretFreeText -Text $canonicalText
    $canonicalBytes = [Text.UTF8Encoding]::new($false).GetBytes($canonicalText)
    if ($canonicalBytes.LongLength -gt 50331648) {
        throw 'TASK038_TUNNEL_SESSION_LOG_CANONICALIZATION_REJECTED'
    }
    $rawSha256 = Get-ByteArraySha256 -Bytes $rawBytes
    $canonicalSha256 = Get-ByteArraySha256 -Bytes $canonicalBytes
    $resolvedDiagnosticPath = ''
    if (-not [string]::IsNullOrWhiteSpace($DiagnosticPath)) {
        if (
            -not [IO.Path]::IsPathRooted($DiagnosticPath) -or
            [string]::IsNullOrWhiteSpace($DiagnosticRoot) -or
            -not [IO.Path]::IsPathRooted($DiagnosticRoot)
        ) {
            throw 'TASK038_TUNNEL_DIAGNOSTIC_COPY_REJECTED'
        }
        $resolvedDiagnosticPath = [IO.Path]::GetFullPath($DiagnosticPath)
        $diagnosticParent = Split-Path -Parent $resolvedDiagnosticPath
        $resolvedDiagnosticRoot = [IO.Path]::GetFullPath($DiagnosticRoot)
        $diagnosticParentItem = Get-Item -LiteralPath $diagnosticParent -Force -ErrorAction SilentlyContinue
        if (
            -not [string]::Equals($diagnosticParent, $resolvedDiagnosticRoot, [StringComparison]::OrdinalIgnoreCase) -or
            $null -eq $diagnosticParentItem -or
            -not $diagnosticParentItem.PSIsContainer -or
            -not ($diagnosticParentItem -is [IO.DirectoryInfo]) -or
            ($diagnosticParentItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            (Test-Path -LiteralPath $resolvedDiagnosticPath)
        ) {
            throw 'TASK038_TUNNEL_DIAGNOSTIC_COPY_REJECTED'
        }
        Assert-NoReparsePath -Path $diagnosticParent -FailureCode 'TASK038_TUNNEL_DIAGNOSTIC_COPY_REJECTED'
        $temporaryPath = $resolvedDiagnosticPath + '.tmp'
        $stream = $null
        try {
            $stream = [IO.FileStream]::new(
                $temporaryPath,
                [IO.FileMode]::CreateNew,
                [IO.FileAccess]::Write,
                [IO.FileShare]::None
            )
            $stream.Write($canonicalBytes, 0, $canonicalBytes.Length)
            $stream.Flush($true)
            $stream.Dispose()
            $stream = $null
            [IO.File]::Move($temporaryPath, $resolvedDiagnosticPath)
            $writtenBytes = [IO.File]::ReadAllBytes($resolvedDiagnosticPath)
            $null = [Text.UTF8Encoding]::new($false, $true).GetString($writtenBytes)
            if (
                $writtenBytes.LongLength -ne $canonicalBytes.LongLength -or
                (Get-ByteArraySha256 -Bytes $writtenBytes) -cne $canonicalSha256
            ) {
                throw 'TASK038_TUNNEL_DIAGNOSTIC_COPY_REJECTED'
            }
        }
        catch {
            throw 'TASK038_TUNNEL_DIAGNOSTIC_COPY_REJECTED'
        }
        finally {
            if ($null -ne $stream) { $stream.Dispose() }
            if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
                [IO.File]::Delete($temporaryPath)
            }
        }
    }

    return [pscustomobject]@{
        schema = 'lattice.task038.tunnel-log-diagnostic.v1'
        canonical_text = $canonicalText
        raw_sha256 = $rawSha256
        raw_byte_count = [long]$rawBytes.LongLength
        strict_utf8 = ($replacementCount -eq 0)
        replacement_count = [int]$replacementCount
        first_invalid_byte_offset = [int]$firstInvalidByteOffset
        canonical_sha256 = $canonicalSha256
        canonical_byte_count = [long]$canonicalBytes.LongLength
        diagnostic_path = $resolvedDiagnosticPath
    }
}

function ConvertFrom-ManagedTunnelJsonLines {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if (
        [string]::IsNullOrEmpty($Text) -or
        -not $Text.EndsWith("`n", [StringComparison]::Ordinal) -or
        $Text.Contains("`r") -or
        $Text.Contains([string][char]0)
    ) {
        throw $FailureCode
    }
    $lines = $Text.Split([char]10)
    if (
        $lines.Count -lt 2 -or
        $lines.Count -gt 10001 -or
        -not [string]::IsNullOrEmpty([string]$lines[$lines.Count - 1])
    ) {
        throw $FailureCode
    }
    $records = [Collections.Generic.List[object]]::new()
    for ($index = 0; $index -lt $lines.Count - 1; $index++) {
        $line = [string]$lines[$index]
        if ([string]::IsNullOrWhiteSpace($line)) { throw $FailureCode }
        try { $record = $line | ConvertFrom-Json }
        catch { throw $FailureCode }
        if (
            $null -eq $record -or
            -not ($record -is [Management.Automation.PSCustomObject]) -or
            @($record.PSObject.Properties).Count -lt 1
        ) {
            throw $FailureCode
        }
        $records.Add($record)
    }
    return @($records)
}

function Write-ManagedTunnelLogDiagnosticEvidence {
    param(
        [Parameter(Mandatory = $true)][ValidateRange(1, 2)][int]$Session,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)]$LogDiagnostic
    )

    Write-ManagedJsonEvidence `
        -Path (Join-Path $EvidenceRoot "session-$Session-tunnel-log-diagnostic.json") `
        -Value ([ordered]@{
            schema = 'lattice.task038.tunnel-log-diagnostic-evidence.v1'
            session = $Session
            raw_log_sha256 = [string]$LogDiagnostic.raw_sha256
            raw_log_byte_count = [long]$LogDiagnostic.raw_byte_count
            raw_log_strict_utf8 = [bool]$LogDiagnostic.strict_utf8
            utf8_replacement_count = [int]$LogDiagnostic.replacement_count
            first_invalid_byte_offset = [int]$LogDiagnostic.first_invalid_byte_offset
            canonical_log_sha256 = [string]$LogDiagnostic.canonical_sha256
            canonical_log_byte_count = [long]$LogDiagnostic.canonical_byte_count
        })
}

function Assert-ManagedTunnelLogSecretFree {
    param([Parameter(Mandatory = $true)][string]$LogPath)

    $null = Get-ManagedTunnelLogDiagnostic -LogPath $LogPath
}

function Invoke-ManagedTunnelClassifierSelfTest {
    $command = "'C:\controlled\latticed.exe'"
    $clientInstanceId = '0123456789abcdef0123456789abcdef'
    $tunnelId = 'tunnel_0123456789abcdef0123456789abcdef'
    $requestId = 'request-0123456789abcdef'
    $valid = @(
        [pscustomobject]@{ msg = 'stdio MCP command started'; command = $command; component = 'mcpclient'; transport = 'stdio'; client_instance_id = $clientInstanceId; tunnel_id = $tunnelId },
        [pscustomobject]@{ msg = 'health server listening'; component = 'health'; client_instance_id = $clientInstanceId; tunnel_id = $tunnelId },
        [pscustomobject]@{ msg = 'pid file written'; component = 'process'; pid = 42; client_instance_id = $clientInstanceId; tunnel_id = $tunnelId },
        [pscustomobject]@{ msg = 'MCP connection TTL reached; stopping response forwarding'; component = 'dispatcher'; request_id = $requestId; client_instance_id = $clientInstanceId; tunnel_id = $tunnelId },
        [pscustomobject]@{ msg = 'command response deadline reached; dropping without posting a response'; component = 'dispatcher'; request_id = $requestId; client_instance_id = $clientInstanceId; tunnel_id = $tunnelId },
        [pscustomobject]@{ msg = 'stdio MCP command exited'; command = $command; component = 'mcpclient'; transport = 'stdio'; client_instance_id = $clientInstanceId; tunnel_id = $tunnelId },
        [pscustomobject]@{ msg = 'stdio MCP command failed; requesting tunnel-client shutdown'; reason = 'stdio MCP command exited'; command = $command; component = 'mcpclient'; transport = 'stdio'; client_instance_id = $clientInstanceId; tunnel_id = $tunnelId }
    )
    if (-not (Test-ManagedTunnelReconnectRecords -Records $valid -ExpectedCommand $command -ExpectedTunnelId $tunnelId)) {
        throw 'TASK038_TUNNEL_RECONNECT_CLASSIFIER_SELF_TEST_REJECTED'
    }
    $normalStop = @($valid[0], $valid[1], $valid[2], $valid[5], $valid[6])
    if (Test-ManagedTunnelReconnectRecords -Records $normalStop -ExpectedCommand $command -ExpectedTunnelId $tunnelId) {
        throw 'TASK038_TUNNEL_RECONNECT_CLASSIFIER_SELF_TEST_REJECTED'
    }
    $wrongReason = @($valid)
    $wrongReason[6] = [pscustomobject]@{
        msg = 'stdio MCP command failed; requesting tunnel-client shutdown'
        reason = 'stdio MCP command stdout closed'
        command = $command
        component = 'mcpclient'
        transport = 'stdio'
        client_instance_id = $clientInstanceId
        tunnel_id = $tunnelId
    }
    if (Test-ManagedTunnelReconnectRecords -Records $wrongReason -ExpectedCommand $command -ExpectedTunnelId $tunnelId) {
        throw 'TASK038_TUNNEL_RECONNECT_CLASSIFIER_SELF_TEST_REJECTED'
    }
    $duplicateShutdown = @($valid + $valid[6])
    if (Test-ManagedTunnelReconnectRecords -Records $duplicateShutdown -ExpectedCommand $command -ExpectedTunnelId $tunnelId) {
        throw 'TASK038_TUNNEL_RECONNECT_CLASSIFIER_SELF_TEST_REJECTED'
    }
}

function Assert-PinnedTunnelClientIdentity {
    param([Parameter(Mandatory = $true)][string]$TunnelClient)

    $sha256 = Get-FileSha256 `
        -Path $TunnelClient `
        -FailureCode 'TASK038_TUNNEL_CLIENT_IDENTITY_REJECTED'
    if ($sha256 -cne '7d3c7d492ce84b52835e11865a835a8a5bcd4a669dee84e169aa11b314dc952a') {
        throw 'TASK038_TUNNEL_CLIENT_IDENTITY_REJECTED'
    }
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $versionOutput = @(& $TunnelClient '--version' 2>&1)
        $versionExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if (
        $versionExitCode -ne 0 -or
        ($versionOutput -join "`n").Trim() -cne
            '0.0.11+8d55683eeef80bc5e360d95abf4692454fafc615 (git sha: 8d55683eeef80bc5e360d95abf4692454fafc615)'
    ) {
        throw 'TASK038_TUNNEL_CLIENT_IDENTITY_REJECTED'
    }
}

function Get-ManagedRequiredEnvironment {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][ValidateRange(1, 16384)][int]$MinimumLength,
        [switch]$Sensitive
    )

    $value = [Environment]::GetEnvironmentVariable($Name, 'Process')
    if (
        [string]::IsNullOrEmpty($value) -or
        $value.Length -lt $MinimumLength -or
        $value.Length -gt 16384 -or
        $value.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0
    ) {
        throw 'TASK038_TUNNEL_MANAGED_ENVIRONMENT_REJECTED'
    }
    if ($Sensitive) {
        $script:ManagedSecretValues.Add($value)
    }
    return $value
}

function Assert-ManagedSecretFreeText {
    param([AllowEmptyString()][string]$Text)

    foreach ($secret in $script:ManagedSecretValues) {
        if (-not [string]::IsNullOrEmpty($secret) -and $Text.Contains($secret)) {
            throw 'TASK038_TUNNEL_MANAGED_OUTPUT_SECRET_REJECTED'
        }
    }
}

function Add-ManagedAuthCredentialSecrets {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [IO.FileInfo]) -or
        $item.Length -lt 2 -or
        $item.Length -gt 1048576
    ) {
        throw 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED'
    }
    try {
        $auth = [Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($item.FullName)) | ConvertFrom-Json
    }
    catch {
        throw 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED'
    }
    $pending = [Collections.Generic.Stack[object]]::new()
    $pending.Push($auth)
    $secretCount = 0
    while ($pending.Count -gt 0) {
        $current = $pending.Pop()
        if ($null -eq $current) { continue }
        if ($current -is [Collections.IDictionary]) {
            foreach ($key in $current.Keys) {
                $value = $current[$key]
                $keyName = [Convert]::ToString($key, [Globalization.CultureInfo]::InvariantCulture)
                if (
                    $value -is [string] -and
                    $keyName -match '(?i)(?:^|_)(?:access_token|refresh_token|id_token|api_key|secret|token)(?:$|_)' -and
                    ([string]$value).Length -ge 8
                ) {
                    if (-not $script:ManagedSecretValues.Contains([string]$value)) {
                        $script:ManagedSecretValues.Add([string]$value)
                    }
                    $secretCount++
                }
                elseif ($null -ne $value -and -not ($value -is [string])) {
                    $pending.Push($value)
                }
            }
            continue
        }
        if ($current -is [Collections.IEnumerable] -and -not ($current -is [string])) {
            foreach ($value in $current) {
                if ($null -ne $value) { $pending.Push($value) }
            }
            continue
        }
        foreach ($property in $current.PSObject.Properties) {
            $value = $property.Value
            $propertyName = [Convert]::ToString($property.Name, [Globalization.CultureInfo]::InvariantCulture)
            if (
                $value -is [string] -and
                $propertyName -match '(?i)(?:^|_)(?:access_token|refresh_token|id_token|api_key|secret|token)(?:$|_)' -and
                ([string]$value).Length -ge 8
            ) {
                if (-not $script:ManagedSecretValues.Contains([string]$value)) {
                    $script:ManagedSecretValues.Add([string]$value)
                }
                $secretCount++
            }
            elseif ($null -ne $value -and -not ($value -is [string])) {
                $pending.Push($value)
            }
        }
    }
    if ($secretCount -lt 1) {
        throw 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED'
    }
}

function Get-ManagedPsqlRows {
    param(
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][string]$HostName,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$DatabaseName,
        [Parameter(Mandatory = $true)][string]$Query,
        [Parameter(Mandatory = $true)][string]$Header,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $originalPassword = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $ErrorActionPreference = 'Continue'
        $output = @(& $Psql '--no-psqlrc' '--no-password' '--quiet' '--csv' `
            '-h' $HostName '-p' ([string]$Port) '-U' 'task019_harness' '-d' $DatabaseName `
            '-v' 'ON_ERROR_STOP=1' '-c' $Query 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $originalPassword, 'Process')
    }
    $text = $output -join "`n"
    Assert-ManagedSecretFreeText -Text $text
    if ($exitCode -ne 0) {
        throw $FailureCode
    }
    $lines = @($text -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $headerIndex = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ([string]$lines[$index] -eq $Header -or [string]$lines[$index] -like ($Header + ',*')) {
            $headerIndex = $index
            break
        }
    }
    if ($headerIndex -lt 0 -or $headerIndex + 1 -ge $lines.Count) {
        throw ($FailureCode + '_SHAPE')
    }
    try {
        return @([string]::Join([Environment]::NewLine, $lines[$headerIndex..($lines.Count - 1)]) | ConvertFrom-Csv)
    }
    catch {
        throw ($FailureCode + '_CSV')
    }
}

function Write-ManagedJsonEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    $resolvedPath = [IO.Path]::GetFullPath($Path)
    $parent = Split-Path -Parent $resolvedPath
    $parentItem = Get-Item -LiteralPath $parent -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $parentItem -or
        -not $parentItem.PSIsContainer -or
        -not ($parentItem -is [IO.DirectoryInfo]) -or
        ($parentItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        (Test-Path -LiteralPath $resolvedPath)
    ) {
        throw 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
    }
    Assert-NoReparsePath -Path $parent -FailureCode 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
    $json = ($Value | ConvertTo-Json -Depth 10 -Compress) + "`n"
    Assert-ManagedSecretFreeText -Text $json
    $temporaryPath = $resolvedPath + '.tmp'
    $stream = $null
    try {
        $stream = [IO.FileStream]::new(
            $temporaryPath,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None
        )
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
        $stream.Dispose()
        $stream = $null
        $temporaryItem = Get-Item -LiteralPath $temporaryPath -Force -ErrorAction Stop
        if (
            $temporaryItem.PSIsContainer -or
            -not ($temporaryItem -is [IO.FileInfo]) -or
            ($temporaryItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
        ) {
            throw 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
        }
        [IO.File]::Move($temporaryPath, $resolvedPath)
    }
    catch [IO.IOException] {
        throw 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
    }
    finally {
        if ($null -ne $stream) { $stream.Dispose() }
        if (Test-Path -LiteralPath $temporaryPath -PathType Leaf) {
            [IO.File]::Delete($temporaryPath)
        }
    }
}

function Get-ManagedTunnelLifecycleEvidence {
    param([Parameter(Mandatory = $true)]$LogDiagnostic)

    if (
        $null -eq $LogDiagnostic -or
        $LogDiagnostic.PSObject.Properties.Name -notcontains 'canonical_text' -or
        $LogDiagnostic.PSObject.Properties.Name -notcontains 'raw_sha256' -or
        $LogDiagnostic.PSObject.Properties.Name -notcontains 'canonical_sha256' -or
        $LogDiagnostic.PSObject.Properties.Name -notcontains 'first_invalid_byte_offset' -or
        [string]$LogDiagnostic.raw_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
        [string]$LogDiagnostic.canonical_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
        [long]$LogDiagnostic.raw_byte_count -lt 1 -or
        [long]$LogDiagnostic.raw_byte_count -gt 16777216 -or
        [long]$LogDiagnostic.canonical_byte_count -lt 1 -or
        [long]$LogDiagnostic.canonical_byte_count -gt 50331648 -or
        [int]$LogDiagnostic.replacement_count -lt 0 -or
        ([int]$LogDiagnostic.replacement_count -eq 0 -and [int]$LogDiagnostic.first_invalid_byte_offset -ne -1) -or
        ([int]$LogDiagnostic.replacement_count -gt 0 -and
            ([int]$LogDiagnostic.first_invalid_byte_offset -lt 0 -or
             [int]$LogDiagnostic.first_invalid_byte_offset -ge [long]$LogDiagnostic.raw_byte_count))
    ) {
        throw 'TASK038_TUNNEL_DURABLE_LIFECYCLE_EVIDENCE_REJECTED'
    }
    $text = [string]$LogDiagnostic.canonical_text
    Assert-ManagedSecretFreeText -Text $text
    $allowedMessages = @(
        'health server listening',
        'pid file written',
        'stdio MCP command started',
        'command response deadline reached; dropping without posting a response',
        'MCP connection TTL reached; stopping response forwarding',
        'stdio MCP command exited',
        'stdio MCP command failed; requesting tunnel-client shutdown'
    )
    $messageCounts = [ordered]@{}
    foreach ($message in $allowedMessages) { $messageCounts[$message] = 0 }
    $clientInstanceIds = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $tunnelIds = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $requestIds = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $commandDigests = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $recordCount = 0
    $records = @(ConvertFrom-ManagedTunnelJsonLines `
        -Text $text `
        -FailureCode 'TASK038_TUNNEL_DURABLE_LIFECYCLE_EVIDENCE_REJECTED')
    foreach ($record in $records) {
        $messageProperty = $record.PSObject.Properties['msg']
        $message = if ($null -eq $messageProperty) { '' } else { [string]$messageProperty.Value }
        if ($message -notin $allowedMessages) { continue }
        $recordCount++
        $messageCounts[$message] = [int]$messageCounts[$message] + 1
        $clientInstanceProperty = $record.PSObject.Properties['client_instance_id']
        $tunnelProperty = $record.PSObject.Properties['tunnel_id']
        $requestProperty = $record.PSObject.Properties['request_id']
        $commandProperty = $record.PSObject.Properties['command']
        $clientInstanceId = if ($null -eq $clientInstanceProperty) { '' } else { [string]$clientInstanceProperty.Value }
        $tunnelId = if ($null -eq $tunnelProperty) { '' } else { [string]$tunnelProperty.Value }
        $requestId = if ($null -eq $requestProperty) { '' } else { [string]$requestProperty.Value }
        $command = if ($null -eq $commandProperty) { '' } else { [string]$commandProperty.Value }
        if (-not [string]::IsNullOrEmpty($clientInstanceId)) {
            if ($clientInstanceId -cnotmatch '^[0-9a-f]{32}$') {
                throw 'TASK038_TUNNEL_DURABLE_LIFECYCLE_EVIDENCE_REJECTED'
            }
            [void]$clientInstanceIds.Add($clientInstanceId)
        }
        if (-not [string]::IsNullOrEmpty($tunnelId)) {
            if ($tunnelId -cnotmatch '^tunnel_[0-9a-f]{32}$') {
                throw 'TASK038_TUNNEL_DURABLE_LIFECYCLE_EVIDENCE_REJECTED'
            }
            [void]$tunnelIds.Add($tunnelId)
        }
        if (-not [string]::IsNullOrEmpty($requestId)) {
            if ($requestId.Length -gt 256 -or $requestId.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0) {
                throw 'TASK038_TUNNEL_DURABLE_LIFECYCLE_EVIDENCE_REJECTED'
            }
            [void]$requestIds.Add($requestId)
        }
        if (-not [string]::IsNullOrEmpty($command)) {
            [void]$commandDigests.Add((Get-StringSha256 -Value $command))
        }
    }
    return [ordered]@{
        schema = 'lattice.task038.tunnel-lifecycle-evidence.v2'
        raw_log_sha256 = [string]$LogDiagnostic.raw_sha256
        raw_log_byte_count = [long]$LogDiagnostic.raw_byte_count
        raw_log_strict_utf8 = [bool]$LogDiagnostic.strict_utf8
        utf8_replacement_count = [int]$LogDiagnostic.replacement_count
        first_invalid_byte_offset = [int]$LogDiagnostic.first_invalid_byte_offset
        canonical_log_sha256 = [string]$LogDiagnostic.canonical_sha256
        canonical_log_byte_count = [long]$LogDiagnostic.canonical_byte_count
        record_count = $recordCount
        message_counts = $messageCounts
        client_instance_ids = @($clientInstanceIds | Sort-Object)
        tunnel_ids = @($tunnelIds | Sort-Object)
        request_ids = @($requestIds | Sort-Object)
        command_sha256 = @($commandDigests | Sort-Object)
    }
}

function Get-ManagedObservationDatabaseFootprint {
    param(
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$RuntimeEnvironment
    )

    $runId = [string]$RuntimeEnvironment['LATTICE_TASK019_RUN_ID']
    if ($runId -cnotmatch '^[0-9a-f]{32}$') {
        throw 'TASK038_TUNNEL_ACCEPTANCE_DATABASE_REJECTED'
    }
    $databaseName = 'lattice_task019_' + $runId.Substring(0, 8) + '_base'
    $rows = @(Get-ManagedPsqlRows `
        -Psql $Psql `
        -HostName ([string]$RuntimeEnvironment['LATTICE_TASK019_HOST']) `
        -Port ([int]$RuntimeEnvironment['LATTICE_TASK019_PORT']) `
        -Password ([string]$RuntimeEnvironment['LATTICE_TASK019_PASSWORD']) `
        -DatabaseName $databaseName `
        -Query @"
SET ROLE lattice_migrator;
SELECT
  COALESCE(s.sequence, 0)::text AS sequence,
  COALESCE(s.event_count, 0)::text AS event_count,
  COALESCE(s.command_count, 0)::text AS command_count,
  COALESCE(encode(s.task_spec_digest, 'hex'), '') AS task_ref,
  COALESCE(encode(s.head_digest, 'hex'), '') AS ledger_head_digest,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), 0)::text AS task_created,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='STATE_TRANSITION'), 0)::text AS state_transitions,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EFFECT_INTENT'), 0)::text AS codex_intents,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EFFECT_OUTCOME'), 0)::text AS verified_outcomes,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EVIDENCE_RECORDED' AND e.action_id='TASK_RESULT'), 0)::text AS task_results,
  COALESCE((SELECT e.command_id FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_command_id,
  COALESCE((SELECT encode(e.subject_digest, 'hex') FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EVIDENCE_RECORDED' AND e.action_id='TASK_RESULT'), '') AS result_digest,
  COALESCE((SELECT string_agg(encode(e.event_digest, 'hex'), ':' ORDER BY e.sequence) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id), '') AS event_digest_chain,
  COALESCE(w.fencing_high_water, 0)::text AS writer_fencing_high_water,
  COALESCE(w.command_high_water, 0)::text AS writer_command_count,
  COALESCE((SELECT count(*) FROM ONLY writer_lease.writer_lease_transitions t WHERE t.project_id='task038-controlled-canary'), 0)::text AS writer_transition_count,
  COALESCE(w.current_status, '') AS current_writer_status,
  COALESCE((SELECT string_agg(encode(c.receipt_digest, 'hex'), ':' ORDER BY c.ordinal) FROM ONLY writer_lease.writer_lease_commands c WHERE c.project_id='task038-controlled-canary'), '') AS writer_receipt_chain,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_analyses) AS memory_analyses,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_receipts) AS memory_receipts,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_retrieval_audits) AS memory_retrieval_audits,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_records) AS memory_records,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_reflections) AS memory_reflections,
  (SELECT count(*)::text FROM ONLY memory.openclaw_gateway_commands) AS openclaw_commands
FROM (SELECT 1) AS anchor
LEFT JOIN ONLY control.task_ledger_streams s
  ON s.project_id='task038-controlled-canary'
 AND s.project_snapshot_id='task038-controlled-canary:snapshot:1'
 AND s.task_id='TASK-038-CANARY'
 AND s.task_revision=1
LEFT JOIN ONLY writer_lease.writer_lease_heads w ON w.project_id='task038-controlled-canary';
"@ `
        -Header 'sequence' `
        -FailureCode 'TASK038_TUNNEL_ACCEPTANCE_DATABASE_REJECTED')
    if ($rows.Count -ne 1) {
        throw 'TASK038_TUNNEL_ACCEPTANCE_DATABASE_REJECTED'
    }
    $row = $rows[0]
    foreach ($digestName in @('task_ref', 'ledger_head_digest', 'result_digest')) {
        $value = [string]$row.$digestName
        if (-not [string]::IsNullOrEmpty($value) -and $value -cnotmatch '^[0-9a-f]{64}$') {
            throw 'TASK038_TUNNEL_ACCEPTANCE_DATABASE_REJECTED'
        }
    }
    $commandId = [string]$row.created_command_id
    if ($commandId.Length -gt 256 -or $commandId.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0) {
        throw 'TASK038_TUNNEL_ACCEPTANCE_DATABASE_REJECTED'
    }
    return [ordered]@{
        sequence = [int]$row.sequence
        event_count = [int]$row.event_count
        command_count = [int]$row.command_count
        task_ref = [string]$row.task_ref
        ledger_head_digest = [string]$row.ledger_head_digest
        task_created = [int]$row.task_created
        state_transitions = [int]$row.state_transitions
        codex_intents = [int]$row.codex_intents
        verified_outcomes = [int]$row.verified_outcomes
        task_results = [int]$row.task_results
        created_command_id_sha256 = Get-StringSha256 -Value $commandId
        result_digest = [string]$row.result_digest
        writer_fencing_high_water = [int]$row.writer_fencing_high_water
        writer_command_count = [int]$row.writer_command_count
        writer_transition_count = [int]$row.writer_transition_count
        current_writer_status = [string]$row.current_writer_status
        memory_analyses = [int]$row.memory_analyses
        memory_receipts = [int]$row.memory_receipts
        memory_retrieval_audits = [int]$row.memory_retrieval_audits
        memory_records = [int]$row.memory_records
        memory_reflections = [int]$row.memory_reflections
        openclaw_commands = [int]$row.openclaw_commands
        ledger_fingerprint = Get-StringSha256 -Value ([string]$row.event_digest_chain)
        writer_fingerprint = Get-StringSha256 -Value ([string]$row.writer_receipt_chain)
    }
}

function Invoke-ManagedGitText {
    param(
        [Parameter(Mandatory = $true)][string]$Git,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $Git '-C' $Repository @Arguments 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $text = ($output -join "`n").Trim()
    Assert-ManagedSecretFreeText -Text $text
    if ($exitCode -ne 0) { throw $FailureCode }
    return $text
}

function Invoke-ManagedGitObservation {
    param(
        [Parameter(Mandatory = $true)][string]$Git,
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    try {
        return [pscustomobject]@{
            Success = $true
            Text = Invoke-ManagedGitText `
                -Git $Git `
                -Repository $Repository `
                -Arguments $Arguments `
                -FailureCode 'TASK038_TUNNEL_OBSERVATION_GIT_COMMAND_FAILED'
        }
    }
    catch {
        if ([string]$_.Exception.Message -cne 'TASK038_TUNNEL_OBSERVATION_GIT_COMMAND_FAILED') { throw }
        return [pscustomobject]@{ Success = $false; Text = '' }
    }
}

function Get-ManagedObservationGitFootprint {
    param([Parameter(Mandatory = $true)][Collections.IDictionary]$RuntimeEnvironment)

    $deliveryRoot = Get-CanonicalPath -Path ([string]$RuntimeEnvironment['LATTICE_DELIVERY_ROOT'])
    $repository = Join-Path $deliveryRoot 'repo'
    if (-not (Test-Path -LiteralPath $repository)) {
        return [ordered]@{
            present = $false
            observation_complete = $true
            state = 'ABSENT'
            head = ''
            parent = ''
            commit_count = 0
            status_clean = $false
            changed_paths = @()
            answer_sha256 = ''
            answer_byte_length = 0
            footprint_sha256 = Get-StringSha256 -Value 'lattice.task038.git-footprint.absent.v1'
        }
    }
    $repositoryItem = Get-Item -LiteralPath $repository -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $repositoryItem -or
        -not $repositoryItem.PSIsContainer -or
        -not ($repositoryItem -is [IO.DirectoryInfo]) -or
        ($repositoryItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED'
    }
    Assert-NoReparsePath -Path $repository -FailureCode 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED'
    $git = Resolve-RequiredLeafPath `
        -Path ([string]$RuntimeEnvironment['LATTICE_DELIVERY_GIT_EXE']) `
        -FailureCode 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED'
    $headResult = Invoke-ManagedGitObservation -Git $git -Repository $repository -Arguments @('rev-parse', '--verify', 'HEAD')
    $countResult = Invoke-ManagedGitObservation -Git $git -Repository $repository -Arguments @('rev-list', '--count', 'HEAD')
    $statusResult = Invoke-ManagedGitObservation -Git $git -Repository $repository -Arguments @('status', '--porcelain=v1', '--untracked-files=all')
    $head = if ($headResult.Success) { [string]$headResult.Text } else { '' }
    $commitCountText = if ($countResult.Success) { [string]$countResult.Text } else { '0' }
    if ($head -and $head -cnotmatch '^[0-9a-f]{40,64}$') { throw 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED' }
    if ($commitCountText -cnotmatch '^[0-9]{1,4}$') { throw 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED' }
    $commitCount = [int]$commitCountText
    $parentResult = if ($commitCount -ge 2) {
        Invoke-ManagedGitObservation -Git $git -Repository $repository -Arguments @('rev-parse', '--verify', 'HEAD^')
    }
    else { [pscustomobject]@{ Success = $true; Text = '' } }
    $changedResult = if ($headResult.Success) {
        Invoke-ManagedGitObservation -Git $git -Repository $repository -Arguments @('diff-tree', '--no-commit-id', '--name-only', '-r', '--no-renames', 'HEAD')
    }
    else { [pscustomobject]@{ Success = $true; Text = '' } }
    $parent = if ($parentResult.Success) { [string]$parentResult.Text } else { '' }
    $status = if ($statusResult.Success) { [string]$statusResult.Text } else { '' }
    $changedText = if ($changedResult.Success) { [string]$changedResult.Text } else { '' }
    if ($parent -and $parent -cnotmatch '^[0-9a-f]{40,64}$') { throw 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED' }
    $changedPaths = @($changedText -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    foreach ($changedPath in $changedPaths) {
        if ($changedPath.Length -gt 512 -or $changedPath.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0) {
            throw 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED'
        }
    }
    $answerPath = Join-Path $repository 'answer.txt'
    $answerItem = Get-Item -LiteralPath $answerPath -Force -ErrorAction SilentlyContinue
    $answerSha256 = ''
    $answerLength = 0
    if ($null -ne $answerItem) {
        if (
            $answerItem.PSIsContainer -or
            -not ($answerItem -is [IO.FileInfo]) -or
            ($answerItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            $answerItem.Length -gt 1048576
        ) {
            throw 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED'
        }
        $answerSha256 = Get-FileSha256 -Path $answerItem.FullName -FailureCode 'TASK038_TUNNEL_OBSERVATION_GIT_REJECTED'
        $answerLength = [long]$answerItem.Length
    }
    $observationComplete = (
        $headResult.Success -and $countResult.Success -and $statusResult.Success -and
        $parentResult.Success -and $changedResult.Success
    )
    $state = if (-not $headResult.Success) { 'INITIALIZED_NO_HEAD' }
        elseif (-not $observationComplete) { 'PARTIAL' }
        else { 'HEAD_OBSERVED' }
    $fingerprint = @(
        'lattice.task038.git-footprint.v1',
        ('head=' + $head),
        ('parent=' + $parent),
        ('commit_count=' + [string]$commitCount),
        ('status_sha256=' + (Get-StringSha256 -Value $status)),
        ('changed_paths=' + ($changedPaths -join ':')),
        ('answer_sha256=' + $answerSha256),
        ('answer_byte_length=' + [string]$answerLength)
    ) -join "`n"
    return [ordered]@{
        present = $true
        observation_complete = $observationComplete
        state = $state
        head = $head
        parent = $parent
        commit_count = $commitCount
        status_clean = [string]::IsNullOrEmpty($status)
        changed_paths = $changedPaths
        answer_sha256 = $answerSha256
        answer_byte_length = $answerLength
        footprint_sha256 = Get-StringSha256 -Value $fingerprint
    }
}

function Write-ManagedSessionObservation {
    param(
        [Parameter(Mandatory = $true)][ValidateRange(1, 2)][int]$Session,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$RuntimeEnvironment,
        [Parameter(Mandatory = $true)][string]$PostgresContinuityDigest,
        [Parameter(Mandatory = $true)]$LogDiagnostic
    )

    $evidenceRootItem = Get-Item -LiteralPath $EvidenceRoot -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $evidenceRootItem -or
        -not $evidenceRootItem.PSIsContainer -or
        -not ($evidenceRootItem -is [IO.DirectoryInfo]) -or
        ($evidenceRootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $PostgresContinuityDigest -cnotmatch '^[0-9a-f]{64}$'
    ) {
        throw 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
    }
    Assert-NoReparsePath -Path $EvidenceRoot -FailureCode 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
    $lifecycle = Get-ManagedTunnelLifecycleEvidence -LogDiagnostic $LogDiagnostic
    $snapshot = [ordered]@{
        schema = 'lattice.task038.managed-session-observation.v1'
        acceptance_scope = 'OBSERVATION_ONLY_NOT_ACCEPTED'
        session = $Session
        postgres_continuity_digest = $PostgresContinuityDigest
        database_footprint = Get-ManagedObservationDatabaseFootprint -Psql $Psql -RuntimeEnvironment $RuntimeEnvironment
        git_footprint = Get-ManagedObservationGitFootprint -RuntimeEnvironment $RuntimeEnvironment
    }
    Write-ManagedJsonEvidence `
        -Path (Join-Path $EvidenceRoot "session-$session-tunnel-lifecycle.json") `
        -Value $lifecycle
    Write-ManagedJsonEvidence `
        -Path (Join-Path $EvidenceRoot "session-$session-observation.json") `
        -Value $snapshot
    return $snapshot
}

function Assert-ManagedCanonicalRestartReceipt {
    param(
        [Parameter(Mandatory = $true)][string]$ReceiptPath,
        [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedReceiptSha256,
        [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedLatticedSha256
    )

    $receipt = Resolve-RequiredLeafPath `
        -Path $ReceiptPath `
        -FailureCode 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED'
    Assert-NoReparsePath `
        -Path $receipt `
        -FailureCode 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED'
    $receiptSha256 = Get-FileSha256 `
        -Path $receipt `
        -FailureCode 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED'
    if ($receiptSha256 -cne $ExpectedReceiptSha256) {
        throw 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED'
    }
    try {
        $value = [Text.UTF8Encoding]::new($false, $true).GetString(
            [IO.File]::ReadAllBytes($receipt)
        ) | ConvertFrom-Json
    }
    catch { throw 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED' }
    $requiredProperties = @(
        'schema_version', 'status', 'mode', 'failure_code', 'completed_stages',
        'binary_sha256', 'source_commit', 'discovered_tools',
        'delivery_run_invoked', 'hermes_run_invoked',
        'tool_level_is_error_checked', 'cross_session_status_checked',
        'postgres_evidence_checked', 'postgres_restart_required',
        'postgres_restart_observed', 'process_cleanup_checked',
        'process_cleanup_status', 'process_cleanup_basis', 'exact_pass_marker',
        'exact_pass_marker_count', 'skip_marker_count', 'not_run'
    )
    if (@($requiredProperties | Where-Object {
        $null -eq $value.PSObject.Properties[$_]
    }).Count -ne 0) {
        throw 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED'
    }
    $expectedTools = @(
        'lattice_delivery_run',
        'lattice_delivery_status',
        'lattice_task_submit',
        'lattice_task_status'
    )
    $observedTools = @($value.discovered_tools | ForEach-Object { [string]$_ } | Sort-Object)
    $requiredStages = @(
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
    $completedStages = @($value.completed_stages | ForEach-Object { [string]$_ })
    if (
        [string]$value.schema_version -cne 'lattice.task038.four-tool-acceptance.v1' -or
        [string]$value.status -cne 'PASS' -or
        [string]$value.mode -cne 'FULL' -or
        -not [string]::IsNullOrEmpty([string]$value.failure_code) -or
        [string]$value.binary_sha256 -cne $ExpectedLatticedSha256 -or
        [string]$value.source_commit -notmatch '^[0-9a-f]{40}$' -or
        @(Compare-Object -ReferenceObject ($expectedTools | Sort-Object) -DifferenceObject $observedTools).Count -ne 0 -or
        @($requiredStages | Where-Object { $completedStages -notcontains $_ }).Count -ne 0 -or
        [bool]$value.delivery_run_invoked -or
        [bool]$value.hermes_run_invoked -or
        -not [bool]$value.tool_level_is_error_checked -or
        -not [bool]$value.cross_session_status_checked -or
        -not [bool]$value.postgres_evidence_checked -or
        -not [bool]$value.postgres_restart_required -or
        -not [bool]$value.postgres_restart_observed -or
        -not [bool]$value.process_cleanup_checked -or
        [string]$value.process_cleanup_status -cne 'PASS' -or
        [string]$value.process_cleanup_basis -cne 'WINDOWS_JOB_OBJECT_NATIVE_HANDLE' -or
        [string]$value.exact_pass_marker -cne 'TASK038_FOUR_TOOL_ACCEPTANCE=PASS' -or
        [int]$value.exact_pass_marker_count -ne 1 -or
        [int]$value.skip_marker_count -ne 0 -or
        [bool]$value.not_run
    ) {
        throw 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED'
    }
    return [pscustomobject]@{
        Path = $receipt
        Sha256 = $receiptSha256
        SourceCommit = [string]$value.source_commit
    }
}

function Enable-ManagedTunnelAuthority {
    param(
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][string]$HostName,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$DatabaseName,
        [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{32}$')][string]$RuntimeId
    )

    $unixSeconds = [long](([DateTime]::UtcNow - [DateTime]'1970-01-01T00:00:00Z').TotalSeconds)
    $authority = [ordered]@{
        daemon_instance_id = ('task038-tunnel-' + $RuntimeId)
        daemon_epoch = $unixSeconds
        authority_revision = $unixSeconds
        observation_digest = Get-StringSha256 -Value ('task038-tunnel-observation|' + $RuntimeId)
        head_digest = Get-StringSha256 -Value ('task038-tunnel-authority|' + $RuntimeId + '|' + $unixSeconds)
    }
    $query = @"
SET ROLE lattice_migrator;
UPDATE ONLY control.runtime_admission
SET admission_mode = 'ACTIVE',
    daemon_instance_id = '$($authority.daemon_instance_id)',
    daemon_epoch = $($authority.daemon_epoch),
    authority_revision = $($authority.authority_revision),
    observation_digest = decode('$($authority.observation_digest)', 'hex'),
    authority_head_digest = decode('$($authority.head_digest)', 'hex'),
    updated_at = clock_timestamp()
WHERE singleton = true;
SELECT admission_mode, daemon_instance_id, daemon_epoch::text,
       authority_revision::text, encode(observation_digest, 'hex') AS observation_digest,
       encode(authority_head_digest, 'hex') AS head_digest
FROM ONLY control.runtime_admission WHERE singleton = true;
"@
    $rows = @(Get-ManagedPsqlRows -Psql $Psql -HostName $HostName -Port $Port `
        -Password $Password -DatabaseName $DatabaseName -Query $query -Header 'admission_mode' `
        -FailureCode 'TASK038_TUNNEL_STORE_AUTHORITY_REJECTED')
    if (
        $rows.Count -ne 1 -or
        [string]$rows[0].admission_mode -cne 'ACTIVE' -or
        [string]$rows[0].daemon_instance_id -cne [string]$authority.daemon_instance_id -or
        [string]$rows[0].daemon_epoch -cne [string]$authority.daemon_epoch -or
        [string]$rows[0].authority_revision -cne [string]$authority.authority_revision -or
        [string]$rows[0].observation_digest -cne [string]$authority.observation_digest -or
        [string]$rows[0].head_digest -cne [string]$authority.head_digest
    ) {
        throw 'TASK038_TUNNEL_STORE_AUTHORITY_REJECTED'
    }
    return $authority
}

function Invoke-ManagedWriterLeaseSetup {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$CaptureRoot,
        [Parameter(Mandatory = $true)][string]$DatabaseName,
        [Parameter(Mandatory = $true)]$Identity,
        [Parameter(Mandatory = $true)]$Authority,
        [Parameter(Mandatory = $true)][string]$MigratorDsn,
        [Parameter(Mandatory = $true)][string]$RuntimeDsn,
        [Parameter(Mandatory = $true)][string]$AdminDsn
    )

    $cargo = @(Get-Command 'cargo.exe' -CommandType Application -ErrorAction Stop)[0].Source
    $values = [ordered]@{
        LATTICE_WRITER_LEASE_MIGRATOR_URL = $MigratorDsn
        LATTICE_WRITER_LEASE_RUNTIME_URL = $RuntimeDsn
        LATTICE_WRITER_LEASE_ADMIN_URL = $AdminDsn
        LATTICE_WRITER_LEASE_DATABASE_NAME = $DatabaseName
        LATTICE_WRITER_LEASE_DATABASE_IDENTITY_SHA256 = [string]$Identity.database_identity_sha256
        LATTICE_WRITER_LEASE_GLOBAL_MANIFEST_SHA256 = [string]$Identity.global_manifest_sha256
        LATTICE_WRITER_LEASE_MEMORY_MANIFEST_SHA256 = [string]$Identity.memory_manifest_sha256
        LATTICE_WRITER_LEASE_DAEMON_INSTANCE_ID = [string]$Authority.daemon_instance_id
        LATTICE_WRITER_LEASE_DAEMON_EPOCH = [string]$Authority.daemon_epoch
        LATTICE_WRITER_LEASE_AUTHORITY_REVISION = [string]$Authority.authority_revision
        LATTICE_WRITER_LEASE_ADMISSION_OBSERVATION_SHA256 = [string]$Authority.observation_digest
        LATTICE_WRITER_LEASE_AUTHORITY_HEAD_SHA256 = [string]$Authority.head_digest
    }
    $stdoutPath = Join-Path $CaptureRoot '.task038-tunnel-writer.stdout.log'
    $stderrPath = Join-Path $CaptureRoot '.task038-tunnel-writer.stderr.log'
    $original = @{}
    $process = $null
    try {
        foreach ($entry in $values.GetEnumerator()) {
            $original[[string]$entry.Key] = [Environment]::GetEnvironmentVariable([string]$entry.Key, 'Process')
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        $process = Start-Process -FilePath $cargo -ArgumentList @(
            'test', '-p', 'lattice-postgres-writer-lease', '--test', 'postgres_live', '--locked',
            '--', '--nocapture', '--test-threads=1'
        ) -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $process.WaitForExit()
        $exitCode = $process.ExitCode
        $text = @(
            $(if (Test-Path -LiteralPath $stdoutPath) { Get-Content -LiteralPath $stdoutPath -Raw -Encoding UTF8 }),
            $(if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw -Encoding UTF8 })
        ) -join "`n"
    }
    finally {
        if ($null -ne $process) { $process.Dispose() }
        foreach ($entry in $values.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, $original[[string]$entry.Key], 'Process')
        }
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }
    Assert-ManagedSecretFreeText -Text $text
    if (
        $exitCode -ne 0 -or
        $text.IndexOf('SKIP:', [StringComparison]::Ordinal) -ge 0 -or
        $text -notmatch '(?m)^test live_postgres_acquire_restarts_and_replays_authority_when_provisioned \.\.\. ok\s*$'
    ) {
        throw 'TASK038_TUNNEL_WRITER_LEASE_SETUP_REJECTED'
    }
}

function Initialize-ManagedTunnelRuntime {
    param(
        [Parameter(Mandatory = $true)][string]$OfficialCodex,
        [Parameter(Mandatory = $true)][string]$CredentialSource,
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][string]$PostgresData
    )

    $repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    $repositoryTarget = [IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target'))
    $hostName = Get-ManagedRequiredEnvironment -Name 'LATTICE_TASK019_HOST' -MinimumLength 1
    $portText = Get-ManagedRequiredEnvironment -Name 'LATTICE_TASK019_PORT' -MinimumLength 1
    $runId = Get-ManagedRequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID' -MinimumLength 32
    $databasePassword = Get-ManagedRequiredEnvironment -Name 'LATTICE_TASK038_POSTGRES_PASSWORD' -MinimumLength 16 -Sensitive
    $migratorDsn = Get-ManagedRequiredEnvironment -Name 'LATTICE_WRITER_LEASE_MIGRATOR_URL' -MinimumLength 24 -Sensitive
    $runtimeDsn = Get-ManagedRequiredEnvironment -Name 'LATTICE_WRITER_LEASE_RUNTIME_URL' -MinimumLength 24 -Sensitive
    $adminDsn = Get-ManagedRequiredEnvironment -Name 'LATTICE_WRITER_LEASE_ADMIN_URL' -MinimumLength 24 -Sensitive
    $port = 0
    if (
        $hostName -cne '127.0.0.1' -or
        -not [int]::TryParse($portText, [ref]$port) -or
        $port -lt 1 -or $port -gt 65535 -or $port -eq 5432 -or
        $runId -cnotmatch '^[0-9a-f]{32}$'
    ) {
        throw 'TASK038_TUNNEL_MANAGED_ENVIRONMENT_REJECTED'
    }

    $psqlPath = Resolve-RequiredLeafPath -Path $Psql -FailureCode 'TASK038_TUNNEL_PSQL_REJECTED'
    $psqlVersion = @(& $psqlPath '--version' 2>&1) -join "`n"
    if ($LASTEXITCODE -ne 0 -or $psqlVersion -notmatch '^psql \(PostgreSQL\) 17\.10(?:\s|$)') {
        throw 'TASK038_TUNNEL_PSQL_REJECTED'
    }
    $dataDirectory = [IO.Path]::GetFullPath($PostgresData)
    $clusterRoot = [IO.Path]::GetFullPath((Split-Path -Parent $dataDirectory))
    $markerPath = Join-Path $clusterRoot '.lattice-task019-disposable.json'
    if (-not (Test-Path -LiteralPath $dataDirectory -PathType Container) -or -not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        throw 'TASK038_TUNNEL_POSTGRES_MARKER_REJECTED'
    }
    try { $marker = Get-Content -LiteralPath $markerPath -Raw -Encoding UTF8 | ConvertFrom-Json } catch { throw 'TASK038_TUNNEL_POSTGRES_MARKER_REJECTED' }
    if (
        [string]$marker.kind -cne 'LATTICE_TASK019_DISPOSABLE_POSTGRES_V1' -or
        [string]$marker.run_id -cne $runId -or
        [string]$marker.postgres_version -cne '17.10' -or
        -not [string]::Equals([IO.Path]::GetFullPath([string]$marker.root), $clusterRoot, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([IO.Path]::GetFullPath([string]$marker.repository_target), $repositoryTarget, [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw 'TASK038_TUNNEL_POSTGRES_MARKER_REJECTED'
    }

    $officialCodexPath = Resolve-RequiredLeafPath -Path $OfficialCodex -FailureCode 'TASK038_TUNNEL_OFFICIAL_CODEX_REJECTED'
    $codexSha256 = Get-FileSha256 -Path $officialCodexPath -FailureCode 'TASK038_TUNNEL_OFFICIAL_CODEX_REJECTED'
    $signature = Get-AuthenticodeSignature -LiteralPath $officialCodexPath
    if (
        $codexSha256 -cne 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb' -or
        [string]$signature.Status -cne 'Valid' -or $null -eq $signature.SignerCertificate -or
        [string]$signature.SignerCertificate.Thumbprint -cne '0B7C30C11BF7250EC1ECD3254AC781D9E13D62F8'
    ) {
        throw 'TASK038_TUNNEL_OFFICIAL_CODEX_REJECTED'
    }
    $codexVersion = @(& $officialCodexPath '--version' 2>&1) -join "`n"
    if ($LASTEXITCODE -ne 0 -or $codexVersion.Trim() -cne 'codex-cli 0.146.0') {
        throw 'TASK038_TUNNEL_OFFICIAL_CODEX_REJECTED'
    }

    $credentialRoot = Get-CanonicalPath -Path $CredentialSource
    $credentialMarker = Join-Path $credentialRoot '.lattice-codex-home-v1'
    $credentialAuth = Join-Path $credentialRoot 'auth.json'
    $credentialRootItem = Get-Item -LiteralPath $credentialRoot -Force -ErrorAction SilentlyContinue
    $credentialMarkerItem = Get-Item -LiteralPath $credentialMarker -Force -ErrorAction SilentlyContinue
    $credentialAuthItem = Get-Item -LiteralPath $credentialAuth -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $credentialRootItem -or
        -not $credentialRootItem.PSIsContainer -or
        -not ($credentialRootItem -is [IO.DirectoryInfo]) -or
        ($credentialRootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $null -eq $credentialMarkerItem -or
        $credentialMarkerItem.PSIsContainer -or
        -not ($credentialMarkerItem -is [IO.FileInfo]) -or
        ($credentialMarkerItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $null -eq $credentialAuthItem -or
        $credentialAuthItem.PSIsContainer -or
        -not ($credentialAuthItem -is [IO.FileInfo]) -or
        ($credentialAuthItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        [IO.File]::ReadAllText($credentialMarker, [Text.Encoding]::UTF8) -cne "lattice.codex-home.v1`n"
    ) {
        throw 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED'
    }
    Assert-NoReparsePath -Path $credentialRoot -FailureCode 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED'
    if (Test-PathOverlap -Left $credentialRoot -Right $repositoryRoot) {
        throw 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED'
    }
    foreach ($ambient in @(
        $env:CODEX_HOME,
        $(if ($env:USERPROFILE) { Join-Path $env:USERPROFILE '.codex' }),
        $(if ($env:HOME) { Join-Path $env:HOME '.codex' })
    )) {
        if (
            -not [string]::IsNullOrWhiteSpace($ambient) -and
            (Test-PathOverlap -Left $credentialRoot -Right $ambient)
        ) {
            throw 'TASK038_TUNNEL_AMBIENT_CODEX_HOME_REJECTED'
        }
    }
    $script:ManagedCredentialSourceAuth = $credentialAuth
    $script:ManagedCredentialSourceSha256 = Get-FileSha256 -Path $credentialAuth -FailureCode 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED'
    Add-ManagedAuthCredentialSecrets -Path $credentialAuth

    $runtimeId = [Guid]::NewGuid().ToString('N')
    $script:ManagedRuntimeId = $runtimeId
    $script:ManagedClusterRoot = $clusterRoot
    $databaseName = 'lattice_task019_' + $runId.Substring(0, 8) + '_base'
    $identityQuery = @"
SET ROLE lattice_migrator;
SELECT btrim(database_identity_sha256)::text AS database_identity_sha256,
       btrim(global_manifest_sha256)::text AS global_manifest_sha256,
       btrim(extension_manifest_sha256)::text AS memory_manifest_sha256
FROM ONLY memory.codebase_memory_extension_identity WHERE singleton;
"@
    $identityRows = @(Get-ManagedPsqlRows -Psql $psqlPath -HostName $hostName -Port $port `
        -Password $databasePassword -DatabaseName $databaseName -Query $identityQuery `
        -Header 'database_identity_sha256' -FailureCode 'TASK038_TUNNEL_DATABASE_IDENTITY_REJECTED')
    if ($identityRows.Count -ne 1) { throw 'TASK038_TUNNEL_DATABASE_IDENTITY_REJECTED' }
    $identity = [ordered]@{}
    foreach ($name in @('database_identity_sha256', 'global_manifest_sha256', 'memory_manifest_sha256')) {
        if ([string]$identityRows[0].$name -cnotmatch '^[0-9a-f]{64}$') {
            throw 'TASK038_TUNNEL_DATABASE_IDENTITY_REJECTED'
        }
        $identity[$name] = [string]$identityRows[0].$name
    }
    $authority = Enable-ManagedTunnelAuthority -Psql $psqlPath -HostName $hostName -Port $port `
        -Password $databasePassword -DatabaseName $databaseName -RuntimeId $runtimeId
    Invoke-ManagedWriterLeaseSetup -RepositoryRoot $repositoryRoot -CaptureRoot $clusterRoot `
        -DatabaseName $databaseName -Identity $identity -Authority $authority `
        -MigratorDsn $migratorDsn -RuntimeDsn $runtimeDsn -AdminDsn $adminDsn
    $authority = Enable-ManagedTunnelAuthority -Psql $psqlPath -HostName $hostName -Port $port `
        -Password $databasePassword -DatabaseName $databaseName -RuntimeId $runtimeId

    $codexHome = Join-Path $clusterRoot 'task038-tunnel-codex-home'
    $script:ManagedCodexHome = $codexHome
    foreach ($ambient in @(
        $env:CODEX_HOME,
        $(if ($env:USERPROFILE) { Join-Path $env:USERPROFILE '.codex' }),
        $(if ($env:HOME) { Join-Path $env:HOME '.codex' })
    )) {
        if (
            -not [string]::IsNullOrWhiteSpace($ambient) -and
            (Test-PathOverlap -Left $codexHome -Right $ambient)
        ) {
            throw 'TASK038_TUNNEL_AMBIENT_CODEX_HOME_REJECTED'
        }
    }
    if (Test-Path -LiteralPath $codexHome) { throw 'TASK038_TUNNEL_CODEX_HOME_NOT_FRESH' }
    [IO.Directory]::CreateDirectory($codexHome) | Out-Null
    [IO.File]::WriteAllText((Join-Path $codexHome '.lattice-codex-home-v1'), "lattice.codex-home.v1`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $codexHome '.lattice-task038-tunnel-owner-v1'), ('lattice.task038-tunnel-home.v1:' + $runtimeId + "`n"), [Text.UTF8Encoding]::new($false))
    $configText = ((@(
        'approval_policy = "never"',
        'sandbox_mode = "workspace-write"',
        'model = "gpt-5.6-sol"',
        'model_reasoning_effort = "low"',
        '',
        '[windows]',
        'sandbox = "unelevated"',
        '',
        '[features]',
        'plugins = false'
    ) -join "`n") + "`n")
    [IO.File]::WriteAllText((Join-Path $codexHome 'config.toml'), $configText, [Text.UTF8Encoding]::new($false))
    $script:ManagedCodexAuthPath = Join-Path $codexHome 'auth.json'
    [IO.File]::Copy($credentialAuth, $script:ManagedCodexAuthPath, $false)
    if ((Get-FileSha256 -Path $script:ManagedCodexAuthPath -FailureCode 'TASK038_TUNNEL_CODEX_HOME_REJECTED') -cne $script:ManagedCredentialSourceSha256) {
        throw 'TASK038_TUNNEL_CODEX_HOME_REJECTED'
    }
    $fixtureParent = Join-Path $repositoryTarget 'lattice-delivery'
    if (-not (Test-Path -LiteralPath $fixtureParent)) { [IO.Directory]::CreateDirectory($fixtureParent) | Out-Null }
    $fixtureRoot = Join-Path $fixtureParent $runtimeId
    if (Test-Path -LiteralPath $fixtureRoot) { throw 'TASK038_TUNNEL_FIXTURE_NOT_FRESH' }
    [IO.Directory]::CreateDirectory($fixtureRoot) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $fixtureRoot 'evidence')) | Out-Null

    return [ordered]@{
        LATTICE_FULL_CHAIN_RUN_MODE = 'FRESH'
        LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
        LATTICE_DELIVERY_TIMEOUT_SECONDS = '300'
        LATTICE_TASK019_HOST = $hostName
        LATTICE_TASK019_PORT = [string]$port
        LATTICE_TASK019_RUN_ID = $runId
        LATTICE_TASK019_PASSWORD = $databasePassword
        LATTICE_STORE_DAEMON_INSTANCE_ID = [string]$authority.daemon_instance_id
        LATTICE_STORE_DAEMON_EPOCH = [string]$authority.daemon_epoch
        LATTICE_STORE_AUTHORITY_REVISION = [string]$authority.authority_revision
        LATTICE_STORE_OBSERVATION_DIGEST = [string]$authority.observation_digest
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = [string]$authority.head_digest
        LATTICE_DELIVERY_LAUNCHER = $officialCodexPath
        LATTICE_DELIVERY_LAUNCHER_VERSION = $codexVersion.Trim()
        LATTICE_DELIVERY_LAUNCHER_SHA256 = $codexSha256
        LATTICE_DELIVERY_SCHEMA_DIR = (Join-Path $fixtureRoot 'schema')
        LATTICE_DELIVERY_CODEX_HOME = $codexHome
        LATTICE_DELIVERY_ROOT = (Join-Path $fixtureRoot 'delivery')
        LATTICE_DELIVERY_GIT_EXE = @(Get-Command 'git.exe' -CommandType Application -ErrorAction Stop)[0].Source
    }
}

function Get-LiveTaskIngressProfileDigest {
    param(
        [Parameter(Mandatory = $true)][string]$ProfileRoot,
        [Parameter(Mandatory = $true)][string]$ProfileName,
        [Parameter(Mandatory = $true)][string]$TunnelClient,
        [switch]$PassThru
    )

    $profileRootItem = Get-Item -LiteralPath $ProfileRoot -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $profileRootItem -or
        -not $profileRootItem.PSIsContainer -or
        -not ($profileRootItem -is [IO.DirectoryInfo]) -or
        ($profileRootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $profilePath = [IO.Path]::GetFullPath((Join-Path $ProfileRoot ($ProfileName + '.yaml')))
    $profileItem = Get-Item -LiteralPath $profilePath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $profileItem -or
        $profileItem.PSIsContainer -or
        -not ($profileItem -is [IO.FileInfo]) -or
        ($profileItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $profileItem.Length -lt 1 -or
        $profileItem.Length -gt 65536
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }

    try {
        $profileBytes = [IO.File]::ReadAllBytes($profilePath)
        $profileText = [Text.UTF8Encoding]::new($false, $true).GetString($profileBytes)
    }
    catch {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    if (
        $profileBytes.Length -ne $profileItem.Length -or
        -not $profileText.EndsWith("`n", [StringComparison]::Ordinal) -or
        $profileText.IndexOf("`r", [StringComparison]::Ordinal) -ge 0
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $profileLines = $profileText.Split([string[]]@("`n"), [StringSplitOptions]::None)
    if ($profileLines.Count -ne 23 -or $profileLines[22] -ne '') {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $tunnelMatch = [regex]::Match(
        $profileLines[4],
        '^  tunnel_id: "(?<value>tunnel_[0-9a-f]{32})"$',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    $commandMatch = [regex]::Match(
        $profileLines[21],
        '^      command: (?<value>"(?:[^"\\]|\\.)*")$',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if (-not $tunnelMatch.Success -or -not $commandMatch.Success) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $commandLiteral = $commandMatch.Groups['value'].Value
    try {
        $quotedCommand = ConvertFrom-Json -InputObject $commandLiteral
    }
    catch {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $canonicalCommandLiteral = '"' + $quotedCommand.Replace('\', '\\').Replace('"', '\"') + '"'
    if (
        -not ($quotedCommand -is [string]) -or
        $quotedCommand.Length -lt 3 -or
        -not $quotedCommand.StartsWith("'", [StringComparison]::Ordinal) -or
        -not $quotedCommand.EndsWith("'", [StringComparison]::Ordinal) -or
        $quotedCommand.Substring(1, $quotedCommand.Length - 2).Contains("'") -or
        -not [String]::Equals($commandLiteral, $canonicalCommandLiteral, [StringComparison]::Ordinal)
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $expectedProfileText = ((@(
        'config_version: 1',
        'control_plane:',
        '  base_url: "https://api.openai.com"',
        '',
        ('  tunnel_id: "' + $tunnelMatch.Groups['value'].Value + '"'),
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
        ('      command: ' + $canonicalCommandLiteral)
    ) -join "`n") + "`n")
    if (-not [String]::Equals($profileText, $expectedProfileText, [StringComparison]::Ordinal)) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $latticed = Resolve-RequiredLeafPath `
        -Path $quotedCommand.Substring(1, $quotedCommand.Length - 2) `
        -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED'
    $tunnelClientSha256 = Get-FileSha256 `
        -Path $TunnelClient `
        -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED'
    $latticedSha256 = Get-FileSha256 `
        -Path $latticed `
        -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED'
    $profileSha256 = Get-ByteArraySha256 -Bytes $profileBytes
    $commitment = @(
        'lattice.task-ingress-profile.v2',
        ('profile_name=' + $ProfileName),
        ('profile_sha256=' + $profileSha256),
        ('tunnel_id=' + $tunnelMatch.Groups['value'].Value),
        'channel=main',
        ('tunnel_client_sha256=' + $tunnelClientSha256),
        ('latticed_sha256=' + $latticedSha256)
    ) -join "`n"
    $digest = Get-StringSha256 -Value $commitment
    if ($PassThru) {
        return [pscustomobject]@{
            Digest = $digest
            CommandLabel = $quotedCommand
            LatticedExecutable = $latticed
            ProfileSha256 = $profileSha256
            TunnelId = $tunnelMatch.Groups['value'].Value
        }
    }
    return $digest
}

function Assert-LiveTaskRuntimeEnvironment {
    param([switch]$Reconnect)

    $requiredNames = @(
        'LATTICE_FULL_CHAIN_RUN_MODE',
        'LATTICE_DELIVERY_CODEX_MODE',
        'LATTICE_DELIVERY_TIMEOUT_SECONDS',
        'LATTICE_TASK019_HOST',
        'LATTICE_TASK019_PORT',
        'LATTICE_TASK019_RUN_ID',
        'LATTICE_TASK019_PASSWORD',
        'LATTICE_STORE_DAEMON_INSTANCE_ID',
        'LATTICE_STORE_DAEMON_EPOCH',
        'LATTICE_STORE_AUTHORITY_REVISION',
        'LATTICE_STORE_OBSERVATION_DIGEST',
        'LATTICE_STORE_AUTHORITY_HEAD_DIGEST',
        'LATTICE_DELIVERY_LAUNCHER',
        'LATTICE_DELIVERY_LAUNCHER_VERSION',
        'LATTICE_DELIVERY_LAUNCHER_SHA256',
        'LATTICE_DELIVERY_SCHEMA_DIR',
        'LATTICE_DELIVERY_CODEX_HOME',
        'LATTICE_DELIVERY_ROOT',
        'LATTICE_DELIVERY_GIT_EXE'
    )
    $values = @{}
    foreach ($name in $requiredNames) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (
            [string]::IsNullOrWhiteSpace($value) -or
            $value.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0
        ) {
            throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        }
        $values[$name] = $value
    }

    $port = 0
    $timeout = 0
    $daemonEpoch = 0L
    $authorityRevision = 0L
    if (
        $values.LATTICE_FULL_CHAIN_RUN_MODE -cne 'FRESH' -or
        $values.LATTICE_DELIVERY_CODEX_MODE -cne 'OFFICIAL_CODEX_APP_SERVER' -or
        -not [int]::TryParse($values.LATTICE_DELIVERY_TIMEOUT_SECONDS, [ref]$timeout) -or
        $timeout -lt 60 -or $timeout -gt 300 -or
        $values.LATTICE_TASK019_HOST -cne '127.0.0.1' -or
        -not [int]::TryParse($values.LATTICE_TASK019_PORT, [ref]$port) -or
        $port -lt 1 -or $port -gt 65535 -or $port -eq 5432 -or
        $values.LATTICE_TASK019_RUN_ID -cnotmatch '^[0-9a-f]{32}$' -or
        $values.LATTICE_TASK019_PASSWORD.Length -lt 16 -or
        $values.LATTICE_STORE_DAEMON_INSTANCE_ID -cnotmatch '^task038-(?:local|tunnel)-[0-9a-f]{32}$' -or
        -not [long]::TryParse($values.LATTICE_STORE_DAEMON_EPOCH, [ref]$daemonEpoch) -or
        -not [long]::TryParse($values.LATTICE_STORE_AUTHORITY_REVISION, [ref]$authorityRevision) -or
        $daemonEpoch -lt 1 -or $authorityRevision -lt 1 -or
        $values.LATTICE_STORE_OBSERVATION_DIGEST -cnotmatch '^[0-9a-f]{64}$' -or
        $values.LATTICE_STORE_AUTHORITY_HEAD_DIGEST -cnotmatch '^[0-9a-f]{64}$' -or
        $values.LATTICE_DELIVERY_LAUNCHER_SHA256 -cnotmatch '^[0-9a-f]{64}$'
    ) {
        throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }

    foreach ($name in @('LATTICE_DELIVERY_LAUNCHER', 'LATTICE_DELIVERY_GIT_EXE')) {
        $null = Resolve-RequiredLeafPath `
            -Path $values[$name] `
            -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }
    $ownedRuntimePaths = [Collections.Generic.List[string]]::new()
    foreach ($name in @('LATTICE_DELIVERY_SCHEMA_DIR', 'LATTICE_DELIVERY_ROOT')) {
        if (-not [IO.Path]::IsPathRooted($values[$name])) {
            throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        }
        $runtimePath = [IO.Path]::GetFullPath($values[$name])
        $expectedLeaf = if ($name -eq 'LATTICE_DELIVERY_SCHEMA_DIR') { 'schema' } else { 'delivery' }
        $parent = Get-Item -LiteralPath (Split-Path -Parent $runtimePath) -Force -ErrorAction SilentlyContinue
        $runtimeItem = Get-Item -LiteralPath $runtimePath -Force -ErrorAction SilentlyContinue
        $pathStateRejected = if ($Reconnect) {
            $null -ne $runtimeItem -and (
                -not $runtimeItem.PSIsContainer -or
                -not ($runtimeItem -is [IO.DirectoryInfo]) -or
                ($runtimeItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
            )
        }
        else {
            $null -ne $runtimeItem
        }
        if (
            [IO.Path]::GetFileName($runtimePath) -cne $expectedLeaf -or
            $pathStateRejected -or
            $null -eq $parent -or
            -not $parent.PSIsContainer -or
            -not ($parent -is [IO.DirectoryInfo]) -or
            ($parent.Attributes -band [IO.FileAttributes]::ReparsePoint)
        ) {
            throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        }
        Assert-NoReparsePath -Path $runtimePath -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        $ownedRuntimePaths.Add($runtimePath)
    }
    if (
        -not (Test-ExactPath `
            -Actual (Split-Path -Parent $ownedRuntimePaths[0]) `
            -Expected (Split-Path -Parent $ownedRuntimePaths[1]))
    ) {
        throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }
    $codexHome = Get-Item -LiteralPath ([IO.Path]::GetFullPath($values.LATTICE_DELIVERY_CODEX_HOME)) -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $codexHome -or
        -not $codexHome.PSIsContainer -or
        -not ($codexHome -is [IO.DirectoryInfo]) -or
        ($codexHome.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }
}

function Get-CurrentManagedRuntimeEnvironment {
    param([Parameter(Mandatory = $true)][Collections.IDictionary]$Template)

    $current = [ordered]@{}
    foreach ($name in $Template.Keys) {
        $current[[string]$name] = [Environment]::GetEnvironmentVariable([string]$name, 'Process')
    }
    return $current
}

function Get-ExactExecutableProcessIds {
    param([Parameter(Mandatory = $true)][string]$Executable)

    $expected = [IO.Path]::GetFullPath($Executable)
    $ids = [Collections.Generic.List[int]]::new()
    foreach ($process in @(Get-Process -ErrorAction SilentlyContinue)) {
        try { $path = $process.Path } catch { continue }
        if (
            -not [string]::IsNullOrWhiteSpace($path) -and
            [string]::Equals([IO.Path]::GetFullPath($path), $expected, [StringComparison]::OrdinalIgnoreCase)
        ) {
            $ids.Add([int]$process.Id)
        }
    }
    return @($ids | Sort-Object)
}

function ConvertTo-WindowsCommandLineArgument {
    param([AllowEmptyString()][Parameter(Mandatory = $true)][string]$Value)

    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') { return $Value }
    $builder = [Text.StringBuilder]::new()
    [void]$builder.Append('"')
    $backslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $backslashes++
            continue
        }
        if ($character -eq '"') {
            [void]$builder.Append(('\' * (($backslashes * 2) + 1)))
            [void]$builder.Append('"')
            $backslashes = 0
            continue
        }
        if ($backslashes -gt 0) {
            [void]$builder.Append(('\' * $backslashes))
            $backslashes = 0
        }
        [void]$builder.Append($character)
    }
    if ($backslashes -gt 0) {
        [void]$builder.Append(('\' * ($backslashes * 2)))
    }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Initialize-Task038OwnedProcessType {
    if ($null -ne ('Lattice.Task038.OwnedProcessTree' -as [type])) { return }

    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

namespace Lattice.Task038
{
    public sealed class OwnedProcessTree : IDisposable
    {
        private const uint CREATE_SUSPENDED = 0x00000004;
        private const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
        private const uint WAIT_OBJECT_0 = 0x00000000;
        private const uint WAIT_TIMEOUT = 0x00000102;
        private const uint INFINITE = 0xffffffff;
        private IntPtr jobHandle;
        private Process rootProcess;
        private bool closed;

        [StructLayout(LayoutKind.Sequential)]
        private struct STARTUPINFO
        {
            public uint cb;
            public IntPtr lpReserved;
            public IntPtr lpDesktop;
            public IntPtr lpTitle;
            public uint dwX;
            public uint dwY;
            public uint dwXSize;
            public uint dwYSize;
            public uint dwXCountChars;
            public uint dwYCountChars;
            public uint dwFillAttribute;
            public uint dwFlags;
            public ushort wShowWindow;
            public ushort cbReserved2;
            public IntPtr lpReserved2;
            public IntPtr hStdInput;
            public IntPtr hStdOutput;
            public IntPtr hStdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct PROCESS_INFORMATION
        {
            public IntPtr hProcess;
            public IntPtr hThread;
            public uint dwProcessId;
            public uint dwThreadId;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_LIMIT_INFORMATION
        {
            public long PerProcessUserTimeLimit;
            public long PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct IO_COUNTERS
        {
            public ulong ReadOperationCount;
            public ulong WriteOperationCount;
            public ulong OtherOperationCount;
            public ulong ReadTransferCount;
            public ulong WriteTransferCount;
            public ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
            public IO_COUNTERS IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateJobObject(IntPtr jobAttributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetInformationJobObject(
            IntPtr job,
            int informationClass,
            ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION information,
            uint informationLength);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CreateProcess(
            string applicationName,
            StringBuilder commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            bool inheritHandles,
            uint creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref STARTUPINFO startupInfo,
            out PROCESS_INFORMATION processInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint ResumeThread(IntPtr thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateProcess(IntPtr process, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateJobObject(IntPtr job, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CloseHandle(IntPtr handle);

        private OwnedProcessTree(IntPtr job, Process process)
        {
            jobHandle = job;
            rootProcess = process;
        }

        public Process RootProcess { get { return rootProcess; } }

        public static OwnedProcessTree Start(string executable, string arguments, string workingDirectory)
        {
            IntPtr job = CreateJobObject(IntPtr.Zero, null);
            if (job == IntPtr.Zero)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_JOB_CREATE_REJECTED");

            JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if (!SetInformationJobObject(job, 9, ref limits, (uint)Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION))))
            {
                int error = Marshal.GetLastWin32Error();
                CloseHandle(job);
                throw new Win32Exception(error, "TASK038_TUNNEL_JOB_LIMIT_REJECTED");
            }

            STARTUPINFO startup = new STARTUPINFO();
            startup.cb = (uint)Marshal.SizeOf(typeof(STARTUPINFO));
            PROCESS_INFORMATION created;
            StringBuilder commandLine = new StringBuilder();
            commandLine.Append('"').Append(executable).Append('"');
            if (!String.IsNullOrWhiteSpace(arguments)) commandLine.Append(' ').Append(arguments);
            if (!CreateProcess(
                    executable,
                    commandLine,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    false,
                    CREATE_SUSPENDED,
                    IntPtr.Zero,
                    workingDirectory,
                    ref startup,
                    out created))
            {
                int error = Marshal.GetLastWin32Error();
                CloseHandle(job);
                throw new Win32Exception(error, "TASK038_TUNNEL_CLIENT_START_REJECTED");
            }

            try
            {
                if (!AssignProcessToJobObject(job, created.hProcess))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_JOB_ASSIGN_REJECTED");
                if (ResumeThread(created.hThread) == UInt32.MaxValue)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_CLIENT_RESUME_REJECTED");
                Process process = Process.GetProcessById((int)created.dwProcessId);
                return new OwnedProcessTree(job, process);
            }
            catch
            {
                TerminateProcess(created.hProcess, 1);
                CloseHandle(job);
                throw;
            }
            finally
            {
                CloseHandle(created.hThread);
                CloseHandle(created.hProcess);
            }
        }

        public bool TerminateAndWait(int milliseconds)
        {
            if (closed) return true;
            bool terminated = TerminateJobObject(jobHandle, 1);
            if (!terminated && Marshal.GetLastWin32Error() != 5) return false;
            uint waitResult = WaitForSingleObject(jobHandle, milliseconds < 0 ? INFINITE : (uint)milliseconds);
            CloseHandle(jobHandle);
            jobHandle = IntPtr.Zero;
            closed = true;
            return waitResult == WAIT_OBJECT_0;
        }

        public void Dispose()
        {
            try { TerminateAndWait(15000); }
            finally
            {
                if (jobHandle != IntPtr.Zero) CloseHandle(jobHandle);
                jobHandle = IntPtr.Zero;
                closed = true;
                if (rootProcess != null) rootProcess.Dispose();
                rootProcess = null;
            }
        }
    }
}
'@ -Language CSharp -ErrorAction Stop
}

function Invoke-ManagedTunnelClientSession {
    param(
        [Parameter(Mandatory = $true)][string]$TunnelClient,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string]$OwnerPidPath,
        [Parameter(Mandatory = $true)][string]$LogPath,
        [switch]$BoundAfterResponseDeadline,
        [ValidateRange(60, 1800)][int]$MaximumLifetimeSeconds = 900,
        [string]$StopSignalPath
    )

    $commandLineArguments = (@($Arguments | ForEach-Object {
        ConvertTo-WindowsCommandLineArgument -Value ([string]$_)
    }) -join ' ')
    $ownedTree = $null
    $process = $null
    $responseDeadlineObservedAt = $null
    $exitCode = $null
    $started = $false
    $stopRequested = $false
    $sessionDeadline = [DateTime]::UtcNow.AddSeconds($MaximumLifetimeSeconds)
    try {
        $ownedTree = [Lattice.Task038.OwnedProcessTree]::Start(
            $TunnelClient,
            $commandLineArguments,
            $WorkingDirectory
        )
        $process = $ownedTree.RootProcess
        $started = $true
        [IO.File]::WriteAllText(
            $OwnerPidPath,
            ([string]$process.Id + "`n"),
            [Text.UTF8Encoding]::new($false)
        )
        while (-not $process.WaitForExit(1000)) {
            if (-not [string]::IsNullOrWhiteSpace($StopSignalPath) -and (Test-Path -LiteralPath $StopSignalPath)) {
                Assert-NoReparsePath -Path $StopSignalPath -FailureCode 'TASK038_TUNNEL_STOP_SIGNAL_REJECTED'
                $stopItem = Get-Item -LiteralPath $StopSignalPath -Force -ErrorAction SilentlyContinue
                if (
                    $null -eq $stopItem -or $stopItem.PSIsContainer -or
                    -not ($stopItem -is [IO.FileInfo]) -or
                    ($stopItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
                    [IO.File]::ReadAllText($StopSignalPath, [Text.Encoding]::UTF8) -cne
                        "STOP_TASK038_TUNNEL_SESSION_2`n"
                ) {
                    throw 'TASK038_TUNNEL_STOP_SIGNAL_REJECTED'
                }
                [IO.File]::Delete($StopSignalPath)
                $stopRequested = $true
                $exitCode = 0
                break
            }
            if (
                (Test-Path -LiteralPath $LogPath -PathType Leaf) -and
                (Get-Item -LiteralPath $LogPath -Force).Length -gt 0
            ) {
                Assert-ManagedTunnelLogSecretFree -LogPath $LogPath
            }
            if (
                $BoundAfterResponseDeadline -and
                $null -eq $responseDeadlineObservedAt -and
                (Test-Path -LiteralPath $LogPath -PathType Leaf)
            ) {
                try {
                    $logTail = [IO.File]::ReadAllText($LogPath, [Text.Encoding]::UTF8)
                    if ($logTail.Contains('"msg":"command response deadline reached; dropping without posting a response"')) {
                        $responseDeadlineObservedAt = [DateTime]::UtcNow
                    }
                }
                catch {}
            }
            if (
                $null -ne $responseDeadlineObservedAt -and
                [DateTime]::UtcNow -ge $responseDeadlineObservedAt.AddSeconds(240)
            ) {
                throw 'TASK038_TUNNEL_FIRST_SESSION_SHUTDOWN_TIMEOUT'
            }
            if ([DateTime]::UtcNow -ge $sessionDeadline) {
                throw 'TASK038_TUNNEL_SESSION_LIFETIME_TIMEOUT'
            }
        }
        if (-not $stopRequested) { $exitCode = $process.ExitCode }
    }
    finally {
        try {
            if ($null -ne $ownedTree -and -not $ownedTree.TerminateAndWait(15000)) {
                throw 'TASK038_TUNNEL_PROCESS_TREE_NOT_REAPED'
            }
        }
        finally {
            if (Test-Path -LiteralPath $OwnerPidPath -PathType Leaf) {
                [IO.File]::Delete($OwnerPidPath)
            }
            if ($null -ne $ownedTree) { $ownedTree.Dispose() }
        }
    }
    return [int]$exitCode
}

function Test-SameProcessIdSet {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][int[]]$Before,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][int[]]$After
    )

    return [string]::Equals(
        (@($Before | Sort-Object) -join ','),
        (@($After | Sort-Object) -join ','),
        [StringComparison]::Ordinal
    )
}

function Get-ManagedPostgresContinuityDigest {
    param(
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$RuntimeEnvironment,
        [Parameter(Mandatory = $true)][string]$ExpectedDataDirectory
    )

    $runId = [string]$RuntimeEnvironment['LATTICE_TASK019_RUN_ID']
    $databaseName = 'lattice_task019_' + $runId.Substring(0, 8) + '_base'
    $connection = @{
        Psql = $Psql
        HostName = [string]$RuntimeEnvironment['LATTICE_TASK019_HOST']
        Port = [int]$RuntimeEnvironment['LATTICE_TASK019_PORT']
        Password = [string]$RuntimeEnvironment['LATTICE_TASK019_PASSWORD']
        DatabaseName = $databaseName
    }
    $physicalRows = @(Get-ManagedPsqlRows @connection `
        -Query @"
SELECT current_setting('data_directory') AS data_directory,
       system_identifier::text AS system_identifier,
       extract(epoch FROM pg_postmaster_start_time())::bigint::text AS postmaster_started_epoch
FROM pg_control_system();
"@ `
        -Header 'data_directory' `
        -FailureCode 'TASK038_TUNNEL_POSTGRES_CONTINUITY_REJECTED')
    if (
        $physicalRows.Count -ne 1 -or
        -not (Test-ExactPath -Actual ([string]$physicalRows[0].data_directory) -Expected $ExpectedDataDirectory) -or
        [string]$physicalRows[0].system_identifier -cnotmatch '^[0-9]{10,24}$' -or
        [string]$physicalRows[0].postmaster_started_epoch -cnotmatch '^[0-9]{9,16}$'
    ) {
        throw 'TASK038_TUNNEL_POSTGRES_CONTINUITY_REJECTED'
    }
    $identityRows = @(Get-ManagedPsqlRows @connection `
        -Query @"
SET ROLE lattice_migrator;
SELECT btrim(database_identity_sha256)::text AS database_identity_sha256,
       btrim(global_manifest_sha256)::text AS global_manifest_sha256,
       btrim(extension_manifest_sha256)::text AS memory_manifest_sha256
FROM ONLY memory.codebase_memory_extension_identity WHERE singleton;
"@ `
        -Header 'database_identity_sha256' `
        -FailureCode 'TASK038_TUNNEL_POSTGRES_CONTINUITY_REJECTED')
    if ($identityRows.Count -ne 1) {
        throw 'TASK038_TUNNEL_POSTGRES_CONTINUITY_REJECTED'
    }
    foreach ($name in @('database_identity_sha256', 'global_manifest_sha256', 'memory_manifest_sha256')) {
        if ([string]$identityRows[0].$name -cnotmatch '^[0-9a-f]{64}$') {
            throw 'TASK038_TUNNEL_POSTGRES_CONTINUITY_REJECTED'
        }
    }
    $commitment = @(
        'lattice.task038.postgres-continuity.v1',
        ('data_directory=' + (Get-CanonicalPath -Path ([string]$physicalRows[0].data_directory)).ToLowerInvariant()),
        ('system_identifier=' + [string]$physicalRows[0].system_identifier),
        ('postmaster_started_epoch=' + [string]$physicalRows[0].postmaster_started_epoch),
        ('database_identity_sha256=' + [string]$identityRows[0].database_identity_sha256),
        ('global_manifest_sha256=' + [string]$identityRows[0].global_manifest_sha256),
        ('memory_manifest_sha256=' + [string]$identityRows[0].memory_manifest_sha256)
    ) -join "`n"
    return Get-StringSha256 -Value $commitment
}

function Assert-ManagedTunnelContinuity {
    param(
        [Parameter(Mandatory = $true)][string]$TunnelClient,
        [Parameter(Mandatory = $true)][string]$ProfileRoot,
        [Parameter(Mandatory = $true)][string]$ProfileName,
        [Parameter(Mandatory = $true)][string]$ExpectedProfileDigest,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$RuntimeEnvironment,
        [Parameter(Mandatory = $true)][string]$ExpectedRuntimeEnvironmentDigest,
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][string]$PostgresData,
        [Parameter(Mandatory = $true)][string]$ExpectedPostgresMarkerSha256,
        [Parameter(Mandatory = $true)][string]$ExpectedPostgresContinuityDigest,
        [switch]$Reconnect
    )

    Assert-PinnedTunnelClientIdentity -TunnelClient $TunnelClient
    $profile = Get-LiveTaskIngressProfileDigest `
        -ProfileRoot $ProfileRoot `
        -ProfileName $ProfileName `
        -TunnelClient $TunnelClient `
        -PassThru
    if ([string]$profile.Digest -cne $ExpectedProfileDigest) {
        throw 'TASK038_TUNNEL_RECONNECT_PROFILE_CHANGED'
    }
    Assert-LiveTaskRuntimeEnvironment -Reconnect:$Reconnect
    $currentEnvironment = Get-CurrentManagedRuntimeEnvironment -Template $RuntimeEnvironment
    if ((Get-ManagedRuntimeEnvironmentDigest -Environment $currentEnvironment) -cne $ExpectedRuntimeEnvironmentDigest) {
        throw 'TASK038_TUNNEL_RECONNECT_ENVIRONMENT_CHANGED'
    }
    if (
        $null -eq $script:ManagedCredentialSourceAuth -or
        $null -eq $script:ManagedCodexAuthPath -or
        (Get-FileSha256 -Path $script:ManagedCredentialSourceAuth -FailureCode 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED') -cne
            $script:ManagedCredentialSourceSha256 -or
        (Get-FileSha256 -Path $script:ManagedCodexAuthPath -FailureCode 'TASK038_TUNNEL_CODEX_HOME_REJECTED') -cne
            $script:ManagedCredentialSourceSha256
    ) {
        throw 'TASK038_TUNNEL_RECONNECT_CREDENTIAL_CHANGED'
    }

    $dataDirectory = [IO.Path]::GetFullPath($PostgresData)
    $markerPath = Join-Path ([IO.Path]::GetFullPath((Split-Path -Parent $dataDirectory))) '.lattice-task019-disposable.json'
    if (
        -not (Test-Path -LiteralPath $dataDirectory -PathType Container) -or
        (Get-FileSha256 -Path $markerPath -FailureCode 'TASK038_TUNNEL_POSTGRES_MARKER_REJECTED') -cne
            $ExpectedPostgresMarkerSha256
    ) {
        throw 'TASK038_TUNNEL_RECONNECT_POSTGRES_CHANGED'
    }
    if (
        (Get-ManagedPostgresContinuityDigest `
            -Psql $Psql `
            -RuntimeEnvironment $RuntimeEnvironment `
            -ExpectedDataDirectory $dataDirectory) -cne
        $ExpectedPostgresContinuityDigest
    ) {
        throw 'TASK038_TUNNEL_RECONNECT_POSTGRES_REJECTED'
    }
    return $profile
}

Initialize-Task038Utf8DiagnosticType
Initialize-Task038OwnedProcessType
Invoke-ManagedTunnelClassifierSelfTest

$tunnelClient = Resolve-RequiredLeafPath -Path $TunnelClientExecutable -FailureCode 'TASK038_TUNNEL_CLIENT_REJECTED'
if (-not [IO.Path]::IsPathRooted($ProfileDirectory)) {
    throw 'TASK038_TUNNEL_PROFILE_DIRECTORY_REJECTED'
}
$profileRoot = [IO.Path]::GetFullPath($ProfileDirectory)
$taskIngressKind = $null
$taskIngressProfileDigest = $null
$originalProcessEnvironment = [Environment]::GetEnvironmentVariables('Process')
$managedRuntimeEnvironment = $null
$managedProfileEvidence = $null
$managedRuntimeEnvironmentDigest = $null
$managedPostgresMarkerSha256 = $null
$managedPostgresContinuityDigest = $null
$managedRuntimeRoot = $null
$managedObservationEvidenceRoot = $null
$managedObservationSnapshots = [Collections.Generic.List[object]]::new()
$managedClosedPath = $null
$managedCanonicalAcceptance = $null

$arguments = switch ($Mode) {
    'Init' {
        if ($TunnelId -notmatch '^tunnel_[0-9a-f]{32}$') {
            throw 'TASK038_TUNNEL_ID_REJECTED'
        }
        $latticed = Resolve-RequiredLeafPath -Path $LatticedExecutable -FailureCode 'TASK038_LATTICED_EXECUTABLE_REJECTED'
        if ($latticed.IndexOfAny([char[]]@("'", "`r", "`n")) -ge 0) {
            throw 'TASK038_LATTICED_COMMAND_REJECTED'
        }
        [IO.Directory]::CreateDirectory($profileRoot) | Out-Null
        @(
            'init',
            '--sample', 'sample_mcp_stdio_local',
            '--profile', $ProfileName,
            '--profile-dir', $profileRoot,
            '--tunnel-id', $TunnelId,
            '--mcp-command', ("'" + $latticed + "'")
        )
        break
    }
    'Doctor' {
        @('doctor', '--profile', $ProfileName, '--profile-dir', $profileRoot, '--explain')
        break
    }
    'Run' {
        if ([string]::IsNullOrWhiteSpace($env:CONTROL_PLANE_API_KEY)) {
            throw 'TASK038_TUNNEL_RUNTIME_KEY_REQUIRED'
        }
        Assert-LiveTaskRuntimeEnvironment
        $taskIngressKind = 'CHATGPT_SECURE_MCP_TUNNEL'
        $taskIngressProfileDigest = Get-LiveTaskIngressProfileDigest `
            -ProfileRoot $profileRoot `
            -ProfileName $ProfileName `
            -TunnelClient $tunnelClient
        Assert-PinnedTunnelClientIdentity -TunnelClient $tunnelClient
        @('run', '--profile', $ProfileName, '--profile-dir', $profileRoot)
        break
    }
    'ManagedRun' {
        if ([string]::IsNullOrWhiteSpace($env:CONTROL_PLANE_API_KEY)) {
            throw 'TASK038_TUNNEL_RUNTIME_KEY_REQUIRED'
        }
        $null = Get-ManagedRequiredEnvironment `
            -Name 'CONTROL_PLANE_API_KEY' `
            -MinimumLength 20 `
            -Sensitive
        if (
            [string]::IsNullOrWhiteSpace($OfficialCodexExecutable) -or
            [string]::IsNullOrWhiteSpace($CodexAuthHome) -or
            [string]::IsNullOrWhiteSpace($PsqlExecutable) -or
            [string]::IsNullOrWhiteSpace($PostgresDataDirectory) -or
            [string]::IsNullOrWhiteSpace($CanonicalAcceptanceReceipt) -or
            [string]::IsNullOrWhiteSpace($CanonicalAcceptanceReceiptSha256)
        ) {
            throw 'TASK038_TUNNEL_MANAGED_INPUT_REJECTED'
        }
        Assert-PinnedTunnelClientIdentity -TunnelClient $tunnelClient
        if ([string]::IsNullOrWhiteSpace($ManagedEvidenceDirectory)) {
            throw 'TASK038_TUNNEL_MANAGED_INPUT_REJECTED'
        }
        $managedRuntimeEnvironment = Initialize-ManagedTunnelRuntime `
            -OfficialCodex $OfficialCodexExecutable `
            -CredentialSource $CodexAuthHome `
            -Psql $PsqlExecutable `
            -PostgresData $PostgresDataDirectory
        foreach ($entry in $managedRuntimeEnvironment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        Assert-LiveTaskRuntimeEnvironment
        $taskIngressKind = 'CHATGPT_SECURE_MCP_TUNNEL'
        $managedProfileEvidence = Get-LiveTaskIngressProfileDigest `
            -ProfileRoot $profileRoot `
            -ProfileName $ProfileName `
            -TunnelClient $tunnelClient `
            -PassThru
        $managedCanonicalAcceptance = Assert-ManagedCanonicalRestartReceipt `
            -ReceiptPath $CanonicalAcceptanceReceipt `
            -ExpectedReceiptSha256 $CanonicalAcceptanceReceiptSha256.ToLowerInvariant() `
            -ExpectedLatticedSha256 (Get-FileSha256 `
                -Path ([string]$managedProfileEvidence.LatticedExecutable) `
                -FailureCode 'TASK038_TUNNEL_CANONICAL_RESTART_RECEIPT_REJECTED')
        $taskIngressProfileDigest = [string]$managedProfileEvidence.Digest
        $managedRuntimeEnvironmentDigest = Get-ManagedRuntimeEnvironmentDigest `
            -Environment $managedRuntimeEnvironment
        $managedPostgresMarkerSha256 = Get-FileSha256 `
            -Path (Join-Path ([IO.Path]::GetFullPath((Split-Path -Parent $PostgresDataDirectory))) '.lattice-task019-disposable.json') `
            -FailureCode 'TASK038_TUNNEL_POSTGRES_MARKER_REJECTED'
        $managedPostgresContinuityDigest = Get-ManagedPostgresContinuityDigest `
            -Psql $PsqlExecutable `
            -RuntimeEnvironment $managedRuntimeEnvironment `
            -ExpectedDataDirectory $PostgresDataDirectory
        $managedRuntimeRoot = Join-Path `
            ([IO.Path]::GetFullPath((Split-Path -Parent $PostgresDataDirectory))) `
            'task038-tunnel-runtime'
        if (-not [IO.Path]::IsPathRooted($ManagedEvidenceDirectory)) {
            throw 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
        }
        $managedObservationEvidenceRoot = [IO.Path]::GetFullPath($ManagedEvidenceDirectory)
        $allowedEvidenceRoot = [IO.Path]::GetFullPath((Join-Path (Join-Path $PSScriptRoot '..') 'target\task038-live'))
        $allowedEvidencePrefix = $allowedEvidenceRoot.TrimEnd('\') + '\'
        $managedObservationEvidenceItem = Get-Item -LiteralPath $managedObservationEvidenceRoot -Force -ErrorAction SilentlyContinue
        if (
            -not $managedObservationEvidenceRoot.StartsWith($allowedEvidencePrefix, [StringComparison]::OrdinalIgnoreCase) -or
            $null -eq $managedObservationEvidenceItem -or
            -not $managedObservationEvidenceItem.PSIsContainer -or
            -not ($managedObservationEvidenceItem -is [IO.DirectoryInfo]) -or
            ($managedObservationEvidenceItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            @(Get-ChildItem -LiteralPath $managedObservationEvidenceRoot -Force).Count -ne 0
        ) {
            throw 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
        }
        Assert-NoReparsePath `
            -Path $managedObservationEvidenceRoot `
            -FailureCode 'TASK038_TUNNEL_OBSERVATION_PATH_REJECTED'
        if (Test-Path -LiteralPath $managedRuntimeRoot) {
            throw 'TASK038_TUNNEL_RUNTIME_ROOT_NOT_FRESH'
        }
        [IO.Directory]::CreateDirectory($managedRuntimeRoot) | Out-Null
        $managedClosedPath = Get-ClosedTunnelChildPath
        @('run', '--profile', $ProfileName, '--profile-dir', $profileRoot)
        break
    }
}

$allowedEnvironmentNames = [Collections.Generic.HashSet[string]]::new(
    [StringComparer]::OrdinalIgnoreCase
)
@(
    'ALLUSERSPROFILE',
    'APPDATA',
    'CommonProgramFiles',
    'CommonProgramFiles(x86)',
    'CommonProgramW6432',
    'ComSpec',
    'CONTROL_PLANE_API_KEY',
    'DriverData',
    'HOMEDRIVE',
    'HOMEPATH',
    'LOCALAPPDATA',
    'NUMBER_OF_PROCESSORS',
    'OS',
    'Path',
    'PATHEXT',
    'PROCESSOR_ARCHITECTURE',
    'PROCESSOR_IDENTIFIER',
    'PROCESSOR_LEVEL',
    'PROCESSOR_REVISION',
    'ProgramData',
    'ProgramFiles',
    'ProgramFiles(x86)',
    'ProgramW6432',
    'PSModulePath',
    'SystemDrive',
    'SystemRoot',
    'TEMP',
    'TMP',
    'USERDOMAIN',
    'USERDOMAIN_ROAMINGPROFILE',
    'USERNAME',
    'USERPROFILE',
    'windir'
) | ForEach-Object { [void]$allowedEnvironmentNames.Add($_) }

$allowedLatticeEnvironmentNames = [Collections.Generic.HashSet[string]]::new(
    [StringComparer]::Ordinal
)
@(
    'LATTICE_FULL_CHAIN_RUN_MODE',
    'LATTICE_DELIVERY_CODEX_MODE',
    'LATTICE_DELIVERY_TIMEOUT_SECONDS',
    'LATTICE_TASK019_HOST',
    'LATTICE_TASK019_PORT',
    'LATTICE_TASK019_RUN_ID',
    'LATTICE_TASK019_PASSWORD',
    'LATTICE_STORE_DAEMON_INSTANCE_ID',
    'LATTICE_STORE_DAEMON_EPOCH',
    'LATTICE_STORE_AUTHORITY_REVISION',
    'LATTICE_STORE_OBSERVATION_DIGEST',
    'LATTICE_STORE_AUTHORITY_HEAD_DIGEST',
    'LATTICE_DELIVERY_LAUNCHER',
    'LATTICE_DELIVERY_LAUNCHER_VERSION',
    'LATTICE_DELIVERY_LAUNCHER_SHA256',
    'LATTICE_DELIVERY_SCHEMA_DIR',
    'LATTICE_DELIVERY_CODEX_HOME',
    'LATTICE_DELIVERY_ROOT',
    'LATTICE_DELIVERY_GIT_EXE'
) | ForEach-Object { [void]$allowedLatticeEnvironmentNames.Add($_) }

$clientExitCode = 1
$managedFinalizationFailure = $null
$managedReconnectAuthorized = $false
$managedExecutionFailure = $null
$managedExecutionFailureCode = ''
try {
    foreach ($entry in $originalProcessEnvironment.GetEnumerator()) {
        $name = [string]$entry.Key
        $isLatticeConfiguration = $allowedLatticeEnvironmentNames.Contains($name)
        if (-not $isLatticeConfiguration -and -not $allowedEnvironmentNames.Contains($name)) {
            [Environment]::SetEnvironmentVariable($name, $null, 'Process')
        }
    }
    if ($Mode -in @('Run', 'ManagedRun')) {
        [Environment]::SetEnvironmentVariable(
            'LATTICE_TASK_INGRESS_KIND',
            $taskIngressKind,
            'Process'
        )
        [Environment]::SetEnvironmentVariable(
            'LATTICE_TASK_INGRESS_PROFILE_SHA256',
            $taskIngressProfileDigest,
            'Process'
        )
    }
    if ($Mode -eq 'ManagedRun') {
        [Environment]::SetEnvironmentVariable('Path', $managedClosedPath, 'Process')
        [Environment]::SetEnvironmentVariable('PSModulePath', $null, 'Process')
        [Environment]::SetEnvironmentVariable('PATHEXT', '.EXE;.COM;.BAT;.CMD', 'Process')
        [Environment]::SetEnvironmentVariable('NoDefaultCurrentDirectoryInExePath', '1', 'Process')
        $pidRoot = Join-Path `
            ([IO.Path]::GetFullPath((Split-Path -Parent $PostgresDataDirectory))) `
            'task038-tunnel-pids'
        if (Test-Path -LiteralPath $pidRoot) {
            throw 'TASK038_TUNNEL_PID_ROOT_NOT_FRESH'
        }
        [IO.Directory]::CreateDirectory($pidRoot) | Out-Null
        $pidRootMarker = Join-Path $pidRoot '.lattice-task038-tunnel-pid-root-v1'
        [IO.File]::WriteAllText(
            $pidRootMarker,
            ('lattice.task038-tunnel-pid-root.v1:' + $script:ManagedRuntimeId + "`n"),
            [Text.UTF8Encoding]::new($false)
        )
        $pidRootContainment = New-LatticeWindowsNativeContainmentSnapshot `
            -ParentPath ([IO.Path]::GetFullPath((Split-Path -Parent $PostgresDataDirectory))) `
            -RootPath $pidRoot `
            -MarkerPath $pidRootMarker
        $managedStopSignalPath = Join-Path $managedRuntimeRoot 'stop-session-2.signal'
        if (Test-Path -LiteralPath $managedStopSignalPath) {
            throw 'TASK038_TUNNEL_STOP_SIGNAL_REJECTED'
        }
        Write-Output ('TASK038_TUNNEL_STOP_SIGNAL=' + $managedStopSignalPath)
        for ($session = 1; $session -le 2; $session++) {
            $profileEvidence = Assert-ManagedTunnelContinuity `
                -TunnelClient $tunnelClient `
                -ProfileRoot $profileRoot `
                -ProfileName $ProfileName `
                -ExpectedProfileDigest $taskIngressProfileDigest `
                -RuntimeEnvironment $managedRuntimeEnvironment `
                -ExpectedRuntimeEnvironmentDigest $managedRuntimeEnvironmentDigest `
                -Psql $PsqlExecutable `
                -PostgresData $PostgresDataDirectory `
                -ExpectedPostgresMarkerSha256 $managedPostgresMarkerSha256 `
                -ExpectedPostgresContinuityDigest $managedPostgresContinuityDigest `
                -Reconnect:($session -eq 2)
            $sessionRoot = Join-Path $managedRuntimeRoot ('session-' + $session)
            $workingDirectory = Join-Path $sessionRoot 'working'
            $userProfile = Join-Path $sessionRoot 'user'
            $roaming = Join-Path $userProfile 'AppData\Roaming'
            $local = Join-Path $userProfile 'AppData\Local'
            $temp = Join-Path $local 'Temp'
            foreach ($directory in @($sessionRoot, $workingDirectory, $roaming, $local, $temp)) {
                [IO.Directory]::CreateDirectory($directory) | Out-Null
                Assert-NoReparsePath `
                    -Path $directory `
                    -FailureCode 'TASK038_TUNNEL_SESSION_DIRECTORY_REJECTED'
            }
            $sessionMarker = Join-Path $sessionRoot '.lattice-task038-tunnel-session-v1'
            [IO.File]::WriteAllText(
                $sessionMarker,
                ('lattice.task038-tunnel-session.v1:' + $script:ManagedRuntimeId + ':' + $session + "`n"),
                [Text.UTF8Encoding]::new($false)
            )
            $sessionContainment = New-LatticeWindowsNativeContainmentSnapshot `
                -ParentPath $managedRuntimeRoot `
                -RootPath $sessionRoot `
                -MarkerPath $sessionMarker
            if (@(Get-ChildItem -LiteralPath $workingDirectory -Force).Count -ne 0) {
                throw 'TASK038_TUNNEL_WORKING_DIRECTORY_NOT_FRESH'
            }
            $homeDrive = Split-Path -Qualifier $userProfile
            $homePath = $userProfile.Substring($homeDrive.Length)
            [Environment]::SetEnvironmentVariable('APPDATA', $roaming, 'Process')
            [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $local, 'Process')
            [Environment]::SetEnvironmentVariable('TEMP', $temp, 'Process')
            [Environment]::SetEnvironmentVariable('TMP', $temp, 'Process')
            [Environment]::SetEnvironmentVariable('USERPROFILE', $userProfile, 'Process')
            [Environment]::SetEnvironmentVariable('HOMEDRIVE', $homeDrive, 'Process')
            [Environment]::SetEnvironmentVariable('HOMEPATH', $homePath, 'Process')
            [Environment]::SetEnvironmentVariable('Path', $managedClosedPath, 'Process')
            [Environment]::SetEnvironmentVariable('PATHEXT', '.EXE;.COM;.BAT;.CMD', 'Process')
            [Environment]::SetEnvironmentVariable('NoDefaultCurrentDirectoryInExePath', '1', 'Process')

            $logPath = Join-Path $sessionRoot 'tunnel-client.jsonl'
            $diagnosticLogPath = Join-Path `
                $managedObservationEvidenceRoot `
                ('session-' + $session + '-tunnel-client.canonical.jsonl')
            $diagnosticEvidencePath = Join-Path `
                $managedObservationEvidenceRoot `
                ('session-' + $session + '-tunnel-log-diagnostic.json')
            $healthPath = Join-Path $sessionRoot 'health.url'
            $pidPath = Join-Path $pidRoot ('task038-tunnel-session-' + $session + '.pid')
            $ownerPidPath = Join-Path $pidRoot ('task038-tunnel-session-' + $session + '.owner.pid')
            foreach ($path in @($logPath, $diagnosticLogPath, $diagnosticEvidencePath, $healthPath, $pidPath, $ownerPidPath)) {
                if (Test-Path -LiteralPath $path) {
                    throw 'TASK038_TUNNEL_SESSION_EVIDENCE_NOT_FRESH'
                }
            }
            $sessionArguments = @($arguments) + @(
                '--log.format', 'json',
                '--log.file', $logPath,
                '--health.listen-addr', '127.0.0.1:0',
                '--health.url-file', $healthPath,
                '--pid.file', $pidPath
            )
            $tunnelProcessesBefore = @(Get-ExactExecutableProcessIds -Executable $tunnelClient)
            $latticedProcessesBefore = @(Get-ExactExecutableProcessIds -Executable ([string]$profileEvidence.LatticedExecutable))
            $sessionFailure = $null
            $sessionLogDiagnostic = $null
            try {
                $clientExitCode = Invoke-ManagedTunnelClientSession `
                    -TunnelClient $tunnelClient `
                    -Arguments $sessionArguments `
                    -WorkingDirectory $workingDirectory `
                    -OwnerPidPath $ownerPidPath `
                    -LogPath $logPath `
                    -BoundAfterResponseDeadline:($session -eq 1) `
                    -MaximumLifetimeSeconds 900 `
                    -StopSignalPath $(if ($session -eq 2) { $managedStopSignalPath } else { '' })
            }
            catch {
                $sessionFailure = $_
                $clientExitCode = 1
            }
            finally {
                $sessionLogDiagnostic = Get-ManagedTunnelLogDiagnostic `
                    -LogPath $logPath `
                    -DiagnosticPath $diagnosticLogPath `
                    -DiagnosticRoot $managedObservationEvidenceRoot
                Write-ManagedTunnelLogDiagnosticEvidence `
                    -Session $session `
                    -EvidenceRoot $managedObservationEvidenceRoot `
                    -LogDiagnostic $sessionLogDiagnostic
            }
            $tunnelProcessesAfter = @(Get-ExactExecutableProcessIds -Executable $tunnelClient)
            $latticedProcessesAfter = @(Get-ExactExecutableProcessIds -Executable ([string]$profileEvidence.LatticedExecutable))
            if (-not (Test-SameProcessIdSet -Before $tunnelProcessesBefore -After $tunnelProcessesAfter)) {
                throw 'TASK038_TUNNEL_SESSION_TUNNEL_PROCESS_SET_CHANGED'
            }
            if (-not (Test-SameProcessIdSet -Before $latticedProcessesBefore -After $latticedProcessesAfter)) {
                throw 'TASK038_TUNNEL_SESSION_LATTICED_PROCESS_SET_CHANGED'
            }
            foreach ($ownedSessionFile in @(
                [pscustomobject]@{ Path = $pidPath; Containment = $pidRootContainment },
                [pscustomobject]@{ Path = $healthPath; Containment = $sessionContainment }
            )) {
                if (Test-Path -LiteralPath $ownedSessionFile.Path -PathType Leaf) {
                    $ownedSessionFileIdentity = Get-LatticeWindowsNativePathIdentityToken `
                        -Path $ownedSessionFile.Path `
                        -Directory $false
                    Assert-LatticeWindowsNativeContainmentSnapshot `
                        -Snapshot $ownedSessionFile.Containment `
                        -FailureCode 'TASK038_TUNNEL_SESSION_CLEANUP_CONTAINMENT_REJECTED'
                    if (-not (Test-LatticeWindowsNativePathIdentity `
                        -Path $ownedSessionFile.Path `
                        -Directory $false `
                        -ExpectedToken $ownedSessionFileIdentity)) {
                        throw 'TASK038_TUNNEL_SESSION_CLEANUP_CONTAINMENT_REJECTED'
                    }
                    [IO.File]::Delete($ownedSessionFile.Path)
                }
            }
            if (Test-Path -LiteralPath $pidPath) {
                throw 'TASK038_TUNNEL_SESSION_PID_FILE_RETAINED'
            }
            if (Test-Path -LiteralPath $healthPath) {
                throw 'TASK038_TUNNEL_SESSION_HEALTH_FILE_RETAINED'
            }
            if (Test-Path -LiteralPath $ownerPidPath) {
                throw 'TASK038_TUNNEL_SESSION_OWNER_PID_RETAINED'
            }
            if (Test-Path -LiteralPath $managedStopSignalPath) {
                throw 'TASK038_TUNNEL_SESSION_STOP_SIGNAL_RETAINED'
            }
            if (@(Get-ChildItem -LiteralPath $workingDirectory -Force).Count -ne 0) {
                throw 'TASK038_TUNNEL_SESSION_WORKING_DIRECTORY_NOT_EMPTY'
            }
            $managedObservationSnapshots.Add((Write-ManagedSessionObservation `
                -Session $session `
                -EvidenceRoot $managedObservationEvidenceRoot `
                -Psql $PsqlExecutable `
                -RuntimeEnvironment $managedRuntimeEnvironment `
                -PostgresContinuityDigest $managedPostgresContinuityDigest `
                -LogDiagnostic $sessionLogDiagnostic))
            if ($null -ne $sessionFailure) { throw $sessionFailure }
            if (
                $session -eq 1 -and
                $clientExitCode -eq 0 -and
                (Test-ManagedTunnelReconnectLog `
                    -LogDiagnostic $sessionLogDiagnostic `
                    -ExpectedCommand ([string]$profileEvidence.CommandLabel) `
                    -ExpectedTunnelId ([string]$profileEvidence.TunnelId))
            ) {
                $managedReconnectAuthorized = $true
                Write-Output 'TASK038_TUNNEL_RECONNECT=AUTHORIZED_ONCE'
                continue
            }
            break
        }
    }
    else {
        & $tunnelClient @arguments
        $clientExitCode = $LASTEXITCODE
    }
}
catch {
    $managedExecutionFailure = $_
    $candidateCode = [string]$_.Exception.Message
    $managedExecutionFailureCode = if ($candidateCode -cmatch '^TASK038_[A-Z0-9_]+$') {
        $candidateCode
    }
    else {
        'TASK038_TUNNEL_UNCLASSIFIED_FAILURE'
    }
}
finally {
    try {
        if ($null -ne $script:ManagedCodexAuthPath) {
            if (
                $null -eq $script:ManagedClusterRoot -or
                $null -eq $script:ManagedCodexHome -or
                $null -eq $script:ManagedRuntimeId -or
                -not (Test-ExactPath `
                    -Actual $script:ManagedCodexAuthPath `
                    -Expected (Join-Path $script:ManagedCodexHome 'auth.json'))
            ) {
                throw 'TASK038_TUNNEL_CODEX_AUTH_CLEANUP_REJECTED'
            }
            Remove-ManagedCodexAuthCredential `
                -ClusterRoot $script:ManagedClusterRoot `
                -CodexHome $script:ManagedCodexHome `
                -RuntimeId $script:ManagedRuntimeId
        }
        if (
            $null -ne $script:ManagedCredentialSourceAuth -and
            (Get-FileSha256 -Path $script:ManagedCredentialSourceAuth -FailureCode 'TASK038_TUNNEL_CREDENTIAL_SOURCE_REJECTED') -cne
                $script:ManagedCredentialSourceSha256
        ) {
            $managedFinalizationFailure = 'TASK038_TUNNEL_CREDENTIAL_SOURCE_CHANGED'
        }
    }
    catch {
        $managedFinalizationFailure = 'TASK038_TUNNEL_MANAGED_CLEANUP_REJECTED'
    }
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
}
if ($Mode -eq 'ManagedRun' -and $null -ne $managedObservationEvidenceRoot) {
    $lastManagedObservation = if ($managedObservationSnapshots.Count -gt 0) {
        $managedObservationSnapshots[$managedObservationSnapshots.Count - 1]
    }
    else { $null }
    $sessionArtifacts = [Collections.Generic.List[object]]::new()
    for ($artifactSession = 1; $artifactSession -le $managedObservationSnapshots.Count; $artifactSession++) {
        $lifecyclePath = Join-Path $managedObservationEvidenceRoot "session-$artifactSession-tunnel-lifecycle.json"
        $observationPath = Join-Path $managedObservationEvidenceRoot "session-$artifactSession-observation.json"
        $sessionArtifacts.Add([ordered]@{
            session = $artifactSession
            lifecycle_sha256 = Get-FileSha256 `
                -Path $lifecyclePath `
                -FailureCode 'TASK038_TUNNEL_OBSERVATION_ARTIFACT_REJECTED'
            observation_sha256 = Get-FileSha256 `
                -Path $observationPath `
                -FailureCode 'TASK038_TUNNEL_OBSERVATION_ARTIFACT_REJECTED'
            canonical_log_sha256 = Get-FileSha256 `
                -Path (Join-Path $managedObservationEvidenceRoot "session-$artifactSession-tunnel-client.canonical.jsonl") `
                -FailureCode 'TASK038_TUNNEL_OBSERVATION_ARTIFACT_REJECTED'
            log_diagnostic_sha256 = Get-FileSha256 `
                -Path (Join-Path $managedObservationEvidenceRoot "session-$artifactSession-tunnel-log-diagnostic.json") `
                -FailureCode 'TASK038_TUNNEL_OBSERVATION_ARTIFACT_REJECTED'
        })
    }
    $footprintsEqual = $false
    $candidateComplete = $false
    if ($managedObservationSnapshots.Count -eq 2) {
        $firstDatabase = $managedObservationSnapshots[0].database_footprint | ConvertTo-Json -Depth 8 -Compress
        $secondDatabase = $managedObservationSnapshots[1].database_footprint | ConvertTo-Json -Depth 8 -Compress
        $firstGit = $managedObservationSnapshots[0].git_footprint | ConvertTo-Json -Depth 8 -Compress
        $secondGit = $managedObservationSnapshots[1].git_footprint | ConvertTo-Json -Depth 8 -Compress
        $footprintsEqual = (
            [String]::Equals($firstDatabase, $secondDatabase, [StringComparison]::Ordinal) -and
            [String]::Equals($firstGit, $secondGit, [StringComparison]::Ordinal)
        )
        $database = $managedObservationSnapshots[1].database_footprint
        $git = $managedObservationSnapshots[1].git_footprint
        $candidateComplete = (
            $managedReconnectAuthorized -and $footprintsEqual -and
            [int]$database.sequence -eq 12 -and
            [int]$database.event_count -eq 12 -and
            [int]$database.command_count -eq 12 -and
            [int]$database.task_created -eq 1 -and
            [int]$database.state_transitions -eq 8 -and
            [int]$database.codex_intents -eq 1 -and
            [int]$database.verified_outcomes -eq 1 -and
            [int]$database.task_results -eq 1 -and
            [int]$database.writer_fencing_high_water -eq 1 -and
            [int]$database.writer_command_count -eq 2 -and
            [int]$database.writer_transition_count -eq 2 -and
            [string]::IsNullOrEmpty([string]$database.current_writer_status) -and
            [int]$database.memory_analyses -eq 0 -and
            [int]$database.memory_receipts -eq 0 -and
            [int]$database.memory_retrieval_audits -eq 0 -and
            [int]$database.memory_records -eq 0 -and
            [int]$database.memory_reflections -eq 0 -and
            [int]$database.openclaw_commands -eq 0 -and
            [bool]$git.present -and [bool]$git.observation_complete -and
            [int]$git.commit_count -eq 2 -and [bool]$git.status_clean -and
            @($git.changed_paths).Count -eq 1 -and [string]$git.changed_paths[0] -ceq 'answer.txt' -and
            [string]$git.answer_sha256 -ceq '1dab4dcc0e1d27971ce5c2f3cccd2631f4ddae0a82284588ee5e3437cde030ae' -and
            [long]$git.answer_byte_length -eq 20
        )
    }
    Write-ManagedJsonEvidence `
        -Path (Join-Path $managedObservationEvidenceRoot 'managed-run-observation.json') `
        -Value ([ordered]@{
            schema = 'lattice.task038.managed-run-observation.v2'
            acceptance_scope = 'PROVIDER_SESSION_OBSERVATION_ONLY_NOT_ACCEPTED'
            accepted = $false
            candidate_complete = $candidateComplete
            external_chatgpt_tool_receipt_bound = $false
            postgres_restart_claimed = $false
            canonical_four_tool_acceptance_receipt_sha256 = [string]$managedCanonicalAcceptance.Sha256
            canonical_four_tool_acceptance_source_commit = [string]$managedCanonicalAcceptance.SourceCommit
            session_count = $managedObservationSnapshots.Count
            session_footprints_equal = $footprintsEqual
            reconnect_authorized = $managedReconnectAuthorized
            client_exit_code = $clientExitCode
            execution_failure_code = $managedExecutionFailureCode
            credential_cleanup_succeeded = ($null -eq $managedFinalizationFailure)
            profile_digest = $taskIngressProfileDigest
            runtime_environment_digest = $managedRuntimeEnvironmentDigest
            postgres_continuity_digest = $managedPostgresContinuityDigest
            session_artifacts = @($sessionArtifacts)
            task_ref = $(if ($null -ne $lastManagedObservation) { [string]$lastManagedObservation.database_footprint.task_ref } else { '' })
            ledger_head_digest = $(if ($null -ne $lastManagedObservation) { [string]$lastManagedObservation.database_footprint.ledger_head_digest } else { '' })
            result_digest = $(if ($null -ne $lastManagedObservation) { [string]$lastManagedObservation.database_footprint.result_digest } else { '' })
        })
}
if ($null -ne $managedFinalizationFailure) {
    throw $managedFinalizationFailure
}
if ($null -ne $managedExecutionFailure) {
    throw $managedExecutionFailure
}
if ($clientExitCode -ne 0) {
    throw ('TASK038_TUNNEL_CLIENT_FAILED_' + $Mode.ToUpperInvariant())
}
if ($Mode -eq 'ManagedRun') {
    throw 'TASK038_TUNNEL_STOPPED'
}
