[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OfficialCodexExecutable,
    [Parameter(Mandatory = $true)]
    [string]$CodexAuthHome,
    [ValidatePattern('^127\.0\.0\.1$')]
    [string]$PostgresHost = '127.0.0.1',
    [Parameter(Mandatory = $true)]
    [ValidateRange(1, 65535)]
    [int]$PostgresPort,
    [Parameter(Mandatory = $true)]
    [ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })]
    [string]$PostgresRunId,
    [Parameter(Mandatory = $true)]
    [string]$PsqlExecutable,
    [Parameter(Mandatory = $true)]
    [string]$PostgresDataDirectory,
    [string]$DatabaseSecretVariable = 'LATTICE_TASK038_POSTGRES_PASSWORD',
    [string]$MigratorDsnVariable = 'LATTICE_WRITER_LEASE_MIGRATOR_URL',
    [string]$RuntimeDsnVariable = 'LATTICE_WRITER_LEASE_RUNTIME_URL',
    [string]$AdminDsnVariable = 'LATTICE_WRITER_LEASE_ADMIN_URL',
    [ValidateRange(60, 300)]
    [int]$TimeoutSeconds = 300
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$expectedPostgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$expectedPostgresExecutableSha256 = '882a5a073a88817f6c6d4c8827df1e4269ff226d52cf6f47c9883e91088c6345'
$expectedPsqlExecutableSha256 = 'e43adb9c5032e7efc63eebb44c5d32b142b34e5f4207666fed2dc7a51d43b630'
$expectedPgCtlExecutableSha256 = 'abe89b0767a8cd0f956059aa5a5a93cd1042efc6194d000c2501da3e23babbd2'

$script:SecretValues = [Collections.Generic.List[string]]::new()
$environmentHelper = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'task038-local-process-environment.ps1'))
$environmentHelperItem = Get-Item -LiteralPath $environmentHelper -Force -ErrorAction SilentlyContinue
if (
    $null -eq $environmentHelperItem -or
    $environmentHelperItem.PSIsContainer -or
    ($environmentHelperItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
) {
    throw 'TASK038_CHILD_ENVIRONMENT_HELPER_REJECTED'
}
. $environmentHelper
$nativeIdentityHelper = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'windows-native-path-identity.ps1'))
$nativeIdentityHelperItem = Get-Item -LiteralPath $nativeIdentityHelper -Force -ErrorAction SilentlyContinue
if (
    $null -eq $nativeIdentityHelperItem -or
    $nativeIdentityHelperItem.PSIsContainer -or
    ($nativeIdentityHelperItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
) {
    throw 'TASK038_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}
. $nativeIdentityHelper

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
    $leftPrefix = $canonicalLeft + [IO.Path]::DirectorySeparatorChar
    $rightPrefix = $canonicalRight + [IO.Path]::DirectorySeparatorChar
    return (
        (Test-ExactPath -Actual $canonicalLeft -Expected $canonicalRight) -or
        $canonicalLeft.StartsWith($rightPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $canonicalRight.StartsWith($leftPrefix, [StringComparison]::OrdinalIgnoreCase)
    )
}

function Assert-RegularFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [IO.FileInfo]) -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $Path -PathType Leaf)
    ) {
        throw $FailureCode
    }
}

function Assert-NoReparseAncestor {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Boundary,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $canonicalPath = Get-CanonicalPath -Path $Path
    $canonicalBoundary = Get-CanonicalPath -Path $Boundary
    $prefix = $canonicalBoundary + [IO.Path]::DirectorySeparatorChar
    if (
        -not (Test-ExactPath -Actual $canonicalPath -Expected $canonicalBoundary) -and
        -not $canonicalPath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw $FailureCode
    }
    $current = $canonicalPath
    while ($true) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw $FailureCode
            }
        }
        if (Test-ExactPath -Actual $current -Expected $canonicalBoundary) {
            break
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrWhiteSpace($parent) -or (Test-ExactPath -Actual $parent -Expected $current)) {
            throw $FailureCode
        }
        $current = $parent
    }
}

function Assert-NoReparsePath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $current = Get-CanonicalPath -Path $Path
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
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

function Get-FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    Assert-RegularFile -Path $Path -FailureCode 'TASK038_FILE_DIGEST_TARGET_REJECTED'
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Read-Task038StrictUtf8Text {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    Assert-RegularFile -Path $Path -FailureCode $FailureCode
    try {
        $bytes = [IO.File]::ReadAllBytes($Path)
        if (
            $bytes.Length -ge 3 -and
            $bytes[0] -eq 0xef -and
            $bytes[1] -eq 0xbb -and
            $bytes[2] -eq 0xbf
        ) {
            throw $FailureCode
        }
        return [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    }
    catch {
        throw $FailureCode
    }
}

function Get-Task019ProductionDatabaseName {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })]
        [string]$RunId
    )

    return 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
}

function Assert-Task038PostgresNativeIdentity {
    param([Parameter(Mandatory = $true)][string]$FailureCode)

    Assert-LatticeWindowsNativeContainmentSnapshot `
        -Snapshot $script:PostgresContainmentSnapshot `
        -FailureCode $FailureCode
    if (-not (Test-LatticeWindowsNativePathIdentity `
        -Path $script:PostgresData `
        -Directory $true `
        -ExpectedToken $script:PostgresDataIdentity)) {
        throw $FailureCode
    }
    foreach ($entry in @(
        @($script:Psql, $script:PsqlNativeIdentity, $expectedPsqlExecutableSha256),
        @($script:PgCtl, $script:PgCtlNativeIdentity, $expectedPgCtlExecutableSha256),
        @($script:PostgresExecutable, $script:PostgresExecutableNativeIdentity, $expectedPostgresExecutableSha256)
    )) {
        if (
            -not (Test-LatticeWindowsNativePathIdentity `
                -Path ([string]$entry[0]) `
                -Directory $false `
                -ExpectedToken ([string]$entry[1])) -or
            (Get-FileSha256 -Path ([string]$entry[0])) -cne [string]$entry[2]
        ) {
            throw $FailureCode
        }
    }
}

function Get-Task038FailureClassification {
    param([Parameter(Mandatory = $true)]$ErrorRecord)

    $message = [string]$ErrorRecord.Exception.Message
    if ($message -match '^TASK038_[A-Z0-9_]{1,95}(?:\|(?:TASK038_|LATTICE_)?[A-Z0-9_]{1,95}|\|[0-9a-f]{64})*$') {
        return $message
    }
    $match = [regex]::Match($message, '(?<![A-Z0-9_])TASK038_[A-Z0-9_]{1,95}(?![A-Z0-9_])')
    if ($match.Success) {
        return $match.Value
    }
    return 'TASK038_UNCLASSIFIED_REJECTED'
}

function Write-Task038ExclusiveBytes {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $stream = $null
    try {
        $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $stream.Write($Bytes, 0, $Bytes.Length)
        $stream.Flush($true)
    }
    catch {
        throw $FailureCode
    }
    finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
    }
}

function Assert-SecretFreeText {
    param(
        [AllowEmptyString()][string]$Text,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    foreach ($secret in $script:SecretValues) {
        if (-not [string]::IsNullOrEmpty($secret) -and
            $Text.IndexOf($secret, [StringComparison]::Ordinal) -ge 0) {
            throw $FailureCode
        }
    }
}

function Write-JsonEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    $json = ($Value | ConvertTo-Json -Depth 16)
    Assert-SecretFreeText -Text $json -FailureCode 'TASK038_EVIDENCE_SECRET_REJECTED'
    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Path $parent -Force:$false | Out-Null
    }
    [IO.File]::WriteAllText($Path, ($json + "`n"), [Text.UTF8Encoding]::new($false))
}

function Write-McpResponseSummary {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$ResponseText
    )

    $lineCount = @($ResponseText -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count
    Write-JsonEvidence -Path $Path -Value ([ordered]@{
        schema_version = 'lattice.task038.mcp-response-summary.v1'
        byte_count = [Text.UTF8Encoding]::new($false).GetByteCount($ResponseText)
        line_count = $lineCount
        response_sha256 = Get-StringSha256 -Value $ResponseText
        raw_response_retained = $false
    })
}

function Get-RequiredSecretEnvironment {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$FailureCode,
        [ValidateRange(1, 16384)][int]$MinimumLength = 1
    )

    if ($Name -notmatch '^[A-Z][A-Z0-9_]{0,127}$') {
        throw $FailureCode
    }
    $value = [Environment]::GetEnvironmentVariable($Name, 'Process')
    if (
        [string]::IsNullOrEmpty($value) -or
        $value.Length -lt $MinimumLength -or
        $value.Length -gt 16384 -or
        $value.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0
    ) {
        throw $FailureCode
    }
    $script:SecretValues.Add($value)
    return $value
}

function Assert-LocalPostgresDsn {
    param(
        [Parameter(Mandatory = $true)][string]$Value,
        [Parameter(Mandatory = $true)][string]$ExpectedDatabase,
        [switch]$AllowMaintenanceDatabase,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $uri = $null
    if (-not [Uri]::TryCreate($Value, [UriKind]::Absolute, [ref]$uri) -or
        $uri.Scheme -notin @('postgres', 'postgresql') -or
        $uri.Host -ne '127.0.0.1' -or
        $uri.Port -ne $PostgresPort -or
        [string]::IsNullOrWhiteSpace($uri.UserInfo) -or
        -not [string]::IsNullOrEmpty($uri.Query) -or
        -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw $FailureCode
    }
    $rawUserInfo = [string]$uri.UserInfo
    $decodedUserInfo = [Uri]::UnescapeDataString($rawUserInfo)
    $separator = $decodedUserInfo.IndexOf(':')
    if ($separator -lt 1 -or $separator + 1 -ge $decodedUserInfo.Length) {
        throw $FailureCode
    }
    $decodedPassword = $decodedUserInfo.Substring($separator + 1)
    $rawSeparator = $rawUserInfo.IndexOf(':')
    $rawPassword = if ($rawSeparator -ge 0) { $rawUserInfo.Substring($rawSeparator + 1) } else { [string]::Empty }
    if ($decodedPassword.Length -lt 16 -or $rawPassword.Length -lt 16) {
        throw $FailureCode
    }
    foreach ($credentialFragment in @($rawUserInfo, $decodedUserInfo, $rawPassword, $decodedPassword)) {
        if (-not [string]::IsNullOrEmpty($credentialFragment) -and -not $script:SecretValues.Contains($credentialFragment)) {
            $script:SecretValues.Add($credentialFragment)
        }
    }
    $database = [Uri]::UnescapeDataString($uri.AbsolutePath.TrimStart('/'))
    if ($AllowMaintenanceDatabase) {
        if ($database -notin @($ExpectedDatabase, 'postgres')) {
            throw $FailureCode
        }
    }
    elseif ($database -ne $ExpectedDatabase) {
        throw $FailureCode
    }
}

function Invoke-NativeText {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$WorkingDirectory
    )

    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) {
            $output = @(& $Executable @Arguments 2>&1)
        }
        else {
            Push-Location $WorkingDirectory
            try {
                $output = @(& $Executable @Arguments 2>&1)
            }
            finally {
                Pop-Location
            }
        }
        $exitCode = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    $text = [string]::Join([Environment]::NewLine, @($output | ForEach-Object { [string]$_ }))
    return [pscustomobject]@{ ExitCode = $exitCode; Text = $text; LineCount = @($output).Count }
}

function New-FreshCodexExecutionHome {
    param(
        [Parameter(Mandatory = $true)][string]$CredentialSource,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })][string]$AcceptanceId
    )

    $source = Get-CanonicalPath -Path $CredentialSource
    $sourceItem = Get-Item -LiteralPath $source -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $sourceItem -or
        -not $sourceItem.PSIsContainer -or
        ($sourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_CODEX_CREDENTIAL_SOURCE_REJECTED'
    }
    Assert-NoReparsePath -Path $source -FailureCode 'TASK038_CODEX_CREDENTIAL_SOURCE_REJECTED'
    $canonicalRepository = Get-CanonicalPath -Path $RepositoryRoot
    if (Test-PathOverlap -Left $source -Right $canonicalRepository) {
        throw 'TASK038_CODEX_CREDENTIAL_SOURCE_REPOSITORY_OVERLAP'
    }
    $sourceAuth = Join-Path $source 'auth.json'
    Assert-RegularFile -Path $sourceAuth -FailureCode 'TASK038_CODEX_CREDENTIAL_SOURCE_REJECTED'

    $executionParent = Get-CanonicalPath -Path (Join-Path (Split-Path -Parent $source) 'task038-execution-homes')
    if (
        (Test-PathOverlap -Left $executionParent -Right $source) -or
        (Test-PathOverlap -Left $executionParent -Right $canonicalRepository)
    ) {
        throw 'TASK038_CODEX_EXECUTION_PARENT_REJECTED'
    }
    foreach ($ambient in @(
        $env:CODEX_HOME,
        $(if ($env:USERPROFILE) { Join-Path $env:USERPROFILE '.codex' }),
        $(if ($env:HOME) { Join-Path $env:HOME '.codex' })
    )) {
        if (
            -not [string]::IsNullOrWhiteSpace($ambient) -and
            ((Test-PathOverlap -Left $source -Right $ambient) -or
             (Test-PathOverlap -Left $executionParent -Right $ambient))
        ) {
            throw 'TASK038_AMBIENT_CODEX_HOME_REJECTED'
        }
    }
    if (-not (Test-Path -LiteralPath $executionParent)) {
        [IO.Directory]::CreateDirectory($executionParent) | Out-Null
    }
    $executionParentItem = Get-Item -LiteralPath $executionParent -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $executionParentItem -or
        -not $executionParentItem.PSIsContainer -or
        ($executionParentItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_CODEX_EXECUTION_PARENT_REJECTED'
    }
    Assert-NoReparsePath -Path $executionParent -FailureCode 'TASK038_CODEX_EXECUTION_PARENT_REJECTED'
    $destination = Get-CanonicalPath -Path (Join-Path $executionParent $AcceptanceId)
    if (Test-Path -LiteralPath $destination) {
        throw 'TASK038_CODEX_EXECUTION_HOME_NOT_FRESH'
    }
    if (
        (Test-PathOverlap -Left $destination -Right $source) -or
        (Test-PathOverlap -Left $destination -Right $canonicalRepository)
    ) {
        throw 'TASK038_CODEX_EXECUTION_HOME_REPOSITORY_OVERLAP'
    }

    try {
        $createdDirectory = New-Item -ItemType Directory -Path $destination -ErrorAction Stop
    }
    catch {
        throw 'TASK038_CODEX_EXECUTION_HOME_NOT_FRESH'
    }
    if (
        $null -eq $createdDirectory -or
        -not $createdDirectory.PSIsContainer -or
        ($createdDirectory.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        -not (Test-ExactPath -Actual $createdDirectory.FullName -Expected $destination) -or
        @(Get-ChildItem -LiteralPath $destination -Force).Count -ne 0
    ) {
        throw 'TASK038_CODEX_EXECUTION_HOME_NOT_FRESH'
    }
    Assert-NoReparseAncestor `
        -Path $destination `
        -Boundary $executionParent `
        -FailureCode 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
    $ownerPath = Join-Path $destination '.lattice-task038-execution-owner-v1'
    $markerPath = Join-Path $destination '.lattice-codex-home-v1'
    $configPath = Join-Path $destination 'config.toml'
    $authPath = Join-Path $destination 'auth.json'
    $markerBytes = [Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
    $configBytes = [Text.UTF8Encoding]::new($false).GetBytes((@(
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
    ) -join "`n") + "`n"
    )
    try {
        $ownerBytes = [Text.Encoding]::UTF8.GetBytes(("lattice.task038-execution-home.v1:" + $AcceptanceId + "`n"))
        Write-Task038ExclusiveBytes `
            -Path $ownerPath `
            -Bytes $ownerBytes `
            -FailureCode 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
        Write-Task038ExclusiveBytes `
            -Path $markerPath `
            -Bytes $markerBytes `
            -FailureCode 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
        Write-Task038ExclusiveBytes `
            -Path $configPath `
            -Bytes $configBytes `
            -FailureCode 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
        Assert-NoReparseAncestor `
            -Path $destination `
            -Boundary $executionParent `
            -FailureCode 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
        $preCredentialItems = @(Get-ChildItem -LiteralPath $destination -Force)
        if (
            $preCredentialItems.Count -ne 3 -or
            @($preCredentialItems | Where-Object {
                $_.PSIsContainer -or ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)
            }).Count -ne 0
        ) {
            throw 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
        }
        [IO.File]::Copy($sourceAuth, $authPath, $false)
        foreach ($path in @($ownerPath, $markerPath, $configPath, $authPath)) {
            Assert-RegularFile -Path $path -FailureCode 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
        }
        if (
            (Get-FileSha256 -Path $configPath) -ne '1a9bc2b325476a4679e5ad9202329c97952ed8ea958162bd0ffadd2196833189' -or
            [Convert]::ToBase64String([IO.File]::ReadAllBytes($ownerPath)) -ne
            [Convert]::ToBase64String($ownerBytes) -or
            [Convert]::ToBase64String([IO.File]::ReadAllBytes($markerPath)) -ne
            [Convert]::ToBase64String($markerBytes)
        ) {
            throw 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
        }
        $provisionedItems = @(Get-ChildItem -LiteralPath $destination -Force)
        if ($provisionedItems.Count -ne 4) {
            throw 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
        }
        Assert-NoReparseAncestor `
            -Path $destination `
            -Boundary $executionParent `
            -FailureCode 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
    }
    catch {
        if (Test-Path -LiteralPath $authPath) {
            try { [IO.File]::Delete($authPath) } catch { }
        }
        try {
            if (Test-Path -LiteralPath $destination -PathType Container) {
                if (Test-Path -LiteralPath $ownerPath -PathType Leaf) {
                    Remove-FreshCodexExecutionHome `
                        -Path $destination `
                        -ExpectedParent $executionParent `
                        -AcceptanceId $AcceptanceId
                }
                else {
                    $partialItems = @(Get-ChildItem -LiteralPath $destination -Force)
                    if ($partialItems.Count -ne 0) {
                        throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
                    }
                    [IO.Directory]::Delete($destination, $false)
                }
            }
        }
        catch {
            throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
        }
        throw 'TASK038_CODEX_EXECUTION_HOME_PROVISION_REJECTED'
    }
    return [pscustomobject]@{ Path = $destination; Parent = $executionParent }
}

function Remove-FreshCodexExecutionHome {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedParent,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })][string]$AcceptanceId
    )

    $parent = Get-CanonicalPath -Path $ExpectedParent
    $executionHome = Get-CanonicalPath -Path $Path
    $expected = Get-CanonicalPath -Path (Join-Path $parent $AcceptanceId)
    if (
        -not (Test-ExactPath -Actual $executionHome -Expected $expected) -or
        -not (Test-Path -LiteralPath $executionHome -PathType Container)
    ) {
        throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    }
    Assert-NoReparseAncestor `
        -Path $executionHome `
        -Boundary $parent `
        -FailureCode 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    $authPath = Join-Path $executionHome 'auth.json'
    if (Test-Path -LiteralPath $authPath) {
        [IO.File]::Delete($authPath)
    }
    if (Test-Path -LiteralPath $authPath) {
        throw 'TASK038_CODEX_EXECUTION_HOME_SECRET_CLEANUP_REJECTED'
    }
    $ownerPath = Join-Path $executionHome '.lattice-task038-execution-owner-v1'
    Assert-RegularFile -Path $ownerPath -FailureCode 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    if (
        [IO.File]::ReadAllText($ownerPath, [Text.Encoding]::UTF8) -cne
        ("lattice.task038-execution-home.v1:" + $AcceptanceId + "`n")
    ) {
        throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    }
    $items = @(Get-ChildItem -LiteralPath $executionHome -Recurse -Force)
    if (@($items | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count -ne 0) {
        throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    }
    foreach ($file in @($items | Where-Object { -not $_.PSIsContainer })) {
        $probe = $null
        try {
            $probe = [IO.File]::Open($file.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
        }
        catch {
            throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
        }
        finally {
            if ($null -ne $probe) { $probe.Dispose() }
        }
    }
    Remove-Item -LiteralPath $executionHome -Recurse -Force
    if (Test-Path -LiteralPath $executionHome) {
        throw 'TASK038_CODEX_EXECUTION_HOME_CLEANUP_REJECTED'
    }
}

function Invoke-BoundedNativeFileCapture {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$OutputDirectory,
        [Parameter(Mandatory = $true)][ValidatePattern('^[A-Z][A-Z0-9_]{0,63}$')][string]$Operation,
        [Parameter(Mandatory = $true)][ValidateRange(1, 120)][int]$TimeoutSeconds,
        [switch]$DiscardOutput
    )

    if ($DiscardOutput -and $Operation -ne 'PG_CTL_START') {
        throw 'TASK038_NATIVE_OUTPUT_DISCARD_REJECTED'
    }

    $canonicalExecutable = Get-CanonicalPath -Path $Executable
    Assert-RegularFile -Path $canonicalExecutable -FailureCode 'TASK038_NATIVE_EXECUTABLE_REJECTED'
    $canonicalOutputDirectory = Get-CanonicalPath -Path $OutputDirectory
    $outputDirectoryItem = Get-Item -LiteralPath $canonicalOutputDirectory -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $outputDirectoryItem -or
        -not $outputDirectoryItem.PSIsContainer -or
        ($outputDirectoryItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_NATIVE_OUTPUT_DIRECTORY_REJECTED'
    }
    Assert-NoReparseAncestor `
        -Path $canonicalOutputDirectory `
        -Boundary $script:RepositoryRoot `
        -FailureCode 'TASK038_NATIVE_OUTPUT_DIRECTORY_REJECTED'

    $captureId = [Guid]::NewGuid().ToString('N')
    $stdoutPath = Join-Path $canonicalOutputDirectory ('.task038-native-' + $captureId + '.stdout.log')
    $stderrPath = Join-Path $canonicalOutputDirectory ('.task038-native-' + $captureId + '.stderr.log')
    $process = $null
    $completed = $false
    $exitCode = $null
    $startFailed = $false
    $stdout = [string]::Empty
    $stderr = [string]::Empty
    $outputTooLarge = $false
    try {
        $startParameters = @{
            FilePath = $canonicalExecutable
            ArgumentList = $Arguments
            WorkingDirectory = $canonicalOutputDirectory
            WindowStyle = 'Hidden'
            PassThru = $true
        }
        if ($DiscardOutput) {
            # PostgreSQL is the long-lived child of `pg_ctl start`. Bind both
            # inherited output handles to non-blocking OS devices so the
            # server cannot retain this harness's temporary capture files.
            $startParameters.RedirectStandardOutput = 'NUL'
            $startParameters.RedirectStandardError = '\\.\NUL'
        }
        else {
            $startParameters.RedirectStandardOutput = $stdoutPath
            $startParameters.RedirectStandardError = $stderrPath
        }
        try {
            $process = Start-Process @startParameters
            $null = $process.Handle
        }
        catch {
            $startFailed = $true
        }
        if (-not $startFailed) {
            $completed = $process.WaitForExit($TimeoutSeconds * 1000)
            if ($completed) {
                $exitCode = [int]$process.ExitCode
            }
            else {
                try { $process.Kill() } catch { }
                $null = $process.WaitForExit(5000)
            }
        }
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        foreach ($path in $(if ($DiscardOutput) { @() } else { @($stdoutPath, $stderrPath) })) {
            if (Test-Path -LiteralPath $path -PathType Leaf) {
                if ((Get-Item -LiteralPath $path -Force).Length -gt 65536) {
                    $outputTooLarge = $true
                }
                elseif (Test-ExactPath -Actual $path -Expected $stdoutPath) {
                    $stdout = [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8)
                }
                else {
                    $stderr = [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8)
                }
                [IO.File]::Delete($path)
                if (Test-Path -LiteralPath $path) {
                    throw 'TASK038_NATIVE_OUTPUT_DELETE_REJECTED'
                }
            }
        }
    }

    $text = if ([string]::IsNullOrEmpty($stdout)) {
        $stderr
    }
    elseif ([string]::IsNullOrEmpty($stderr)) {
        $stdout
    }
    else {
        $stdout + [Environment]::NewLine + $stderr
    }
    Assert-SecretFreeText -Text $text -FailureCode 'TASK038_NATIVE_OUTPUT_SECRET_REJECTED'
    if ($outputTooLarge) {
        throw ('TASK038_NATIVE_OUTPUT_SIZE_REJECTED|' + $Operation)
    }
    if ($startFailed) {
        throw ('TASK038_NATIVE_PROCESS_START_REJECTED|' + $Operation + '|' + (Get-StringSha256 -Value $text))
    }
    if (-not $completed) {
        throw ('TASK038_NATIVE_PROCESS_TIMEOUT|' + $Operation + '|' + (Get-StringSha256 -Value $text))
    }
    return [pscustomobject]@{
        ExitCode = $exitCode
        Text = $text
        Stdout = $stdout
        Stderr = $stderr
    }
}

function Invoke-PsqlRows {
    param(
        [Parameter(Mandatory = $true)][string]$Query,
        [Parameter(Mandatory = $true)][string]$Header,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$DatabaseName,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $original = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $result = Invoke-NativeText -Executable $script:Psql -Arguments @(
            '--no-psqlrc', '--no-password', '--quiet', '--csv',
            '-h', $PostgresHost, '-p', [string]$PostgresPort,
            '-U', 'task019_harness', '-d', $DatabaseName,
            '-v', 'ON_ERROR_STOP=1', '-c', $Query
        )
    }
    finally {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $original, 'Process')
    }
    Assert-SecretFreeText -Text $result.Text -FailureCode 'TASK038_PSQL_OUTPUT_SECRET_REJECTED'
    if ($result.ExitCode -ne 0) {
        throw ($FailureCode + '|' + (Get-StringSha256 -Value $result.Text))
    }
    $lines = @($result.Text -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
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
    $csv = [string]::Join([Environment]::NewLine, $lines[$headerIndex..($lines.Count - 1)])
    try {
        return @($csv | ConvertFrom-Csv)
    }
    catch {
        throw ($FailureCode + '_CSV')
    }
}

function Get-PostgresProcessEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$DatabaseName
    )

    $rows = @(Invoke-PsqlRows `
        -Query "SELECT pg_postmaster_start_time()::text AS postmaster_started_at, system_identifier::text FROM pg_control_system();" `
        -Header 'postmaster_started_at' `
        -Password $Password `
        -DatabaseName $DatabaseName `
        -FailureCode 'TASK038_POSTGRES_PROCESS_EVIDENCE_REJECTED')
    if (
        $rows.Count -ne 1 -or
        [string]::IsNullOrWhiteSpace([string]$rows[0].postmaster_started_at) -or
        [string]$rows[0].system_identifier -notmatch '^[0-9]{1,20}$'
    ) {
        throw 'TASK038_POSTGRES_PROCESS_EVIDENCE_REJECTED_SHAPE'
    }
    return [ordered]@{
        postmaster_started_at = [string]$rows[0].postmaster_started_at
        system_identifier = [string]$rows[0].system_identifier
    }
}

function Restart-DisposablePostgres {
    param(
        [Parameter(Mandatory = $true)][string]$DataDirectory,
        [Parameter(Mandatory = $true)][string]$ServerLog
    )

    $captureRoot = Get-CanonicalPath -Path (Split-Path -Parent $DataDirectory)
    Assert-NoReparseAncestor `
        -Path $captureRoot `
        -Boundary $script:RepositoryRoot `
        -FailureCode 'TASK038_POSTGRES_RESTART_ROOT_REJECTED'
    if (-not (Test-ExactPath -Actual $ServerLog -Expected (Join-Path $captureRoot 'postgres.log'))) {
        throw 'TASK038_POSTGRES_RESTART_LOG_REJECTED'
    }

    try {
        $stop = Invoke-BoundedNativeFileCapture -Executable $script:PgCtl -Arguments @(
            '-D', $DataDirectory, '-w', '-t', '30', '-m', 'fast', 'stop'
        ) -OutputDirectory $captureRoot -Operation 'PG_CTL_STOP' -TimeoutSeconds 45
    }
    catch {
        $classification = Get-Task038FailureClassification -ErrorRecord $_
        if ($classification -eq 'TASK038_UNCLASSIFIED_REJECTED') {
            throw 'TASK038_POSTGRES_STOP_EXECUTION_REJECTED'
        }
        throw $classification
    }
    Assert-SecretFreeText -Text $stop.Text -FailureCode 'TASK038_POSTGRES_STOP_OUTPUT_SECRET_REJECTED'
    if ($stop.ExitCode -ne 0) {
        throw ('TASK038_POSTGRES_STOP_REJECTED|' + (Get-StringSha256 -Value $stop.Text))
    }
    try {
        $status = Invoke-BoundedNativeFileCapture `
            -Executable $script:PgCtl `
            -Arguments @('-D', $DataDirectory, 'status') `
            -OutputDirectory $captureRoot `
            -Operation 'PG_CTL_STATUS' `
            -TimeoutSeconds 15
    }
    catch {
        $classification = Get-Task038FailureClassification -ErrorRecord $_
        if ($classification -eq 'TASK038_UNCLASSIFIED_REJECTED') {
            throw 'TASK038_POSTGRES_STATUS_EXECUTION_REJECTED'
        }
        throw $classification
    }
    Assert-SecretFreeText -Text $status.Text -FailureCode 'TASK038_POSTGRES_STATUS_OUTPUT_SECRET_REJECTED'
    if ($status.ExitCode -ne 3) {
        throw 'TASK038_POSTGRES_STOP_NOT_PROVED'
    }

    try {
        $start = Invoke-BoundedNativeFileCapture -Executable $script:PgCtl -Arguments @(
            '-D', $DataDirectory, '-l', $ServerLog, '-w', '-t', '30', 'start'
        ) -OutputDirectory $captureRoot -Operation 'PG_CTL_START' -TimeoutSeconds 45 -DiscardOutput
    }
    catch {
        $classification = Get-Task038FailureClassification -ErrorRecord $_
        if ($classification -eq 'TASK038_UNCLASSIFIED_REJECTED') {
            throw 'TASK038_POSTGRES_START_EXECUTION_REJECTED'
        }
        throw $classification
    }
    Assert-SecretFreeText -Text $start.Text -FailureCode 'TASK038_POSTGRES_START_OUTPUT_SECRET_REJECTED'
    if ($start.ExitCode -ne 0) {
        throw ('TASK038_POSTGRES_START_REJECTED|' + (Get-StringSha256 -Value $start.Text))
    }
}

function Get-DatabaseIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$DatabaseName
    )

    $query = @"
SET ROLE lattice_migrator;
SELECT btrim(database_identity_sha256)::text AS database_identity_sha256,
       btrim(global_manifest_sha256)::text AS global_manifest_sha256,
       btrim(extension_manifest_sha256)::text AS memory_manifest_sha256
FROM ONLY memory.codebase_memory_extension_identity
WHERE singleton;
"@
    $rows = @(Invoke-PsqlRows -Query $query -Header 'database_identity_sha256' -Password $Password -DatabaseName $DatabaseName -FailureCode 'TASK038_DATABASE_IDENTITY_REJECTED')
    if ($rows.Count -ne 1) {
        throw 'TASK038_DATABASE_IDENTITY_REJECTED_SHAPE'
    }
    foreach ($name in @('database_identity_sha256', 'global_manifest_sha256', 'memory_manifest_sha256')) {
        if ([string]$rows[0].$name -notmatch '^[0-9a-f]{64}$') {
            throw 'TASK038_DATABASE_IDENTITY_REJECTED_VALUE'
        }
    }
    return [ordered]@{
        database_identity_sha256 = [string]$rows[0].database_identity_sha256
        global_manifest_sha256 = [string]$rows[0].global_manifest_sha256
        memory_manifest_sha256 = [string]$rows[0].memory_manifest_sha256
    }
}

function Enable-StoreAuthority {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$DatabaseName,
        [Parameter(Mandatory = $true)][string]$AcceptanceId
    )

    $unixSeconds = [long](([DateTime]::UtcNow - [DateTime]'1970-01-01T00:00:00Z').TotalSeconds)
    $authority = [ordered]@{
        daemon_instance_id = ('task038-local-' + $AcceptanceId)
        daemon_epoch = $unixSeconds
        authority_revision = $unixSeconds
        observation_digest = Get-StringSha256 -Value ('task038-local-observation|' + $AcceptanceId)
        head_digest = Get-StringSha256 -Value ('task038-local-authority|' + $AcceptanceId + '|' + $unixSeconds)
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
    $rows = @(Invoke-PsqlRows -Query $query -Header 'admission_mode' -Password $Password -DatabaseName $DatabaseName -FailureCode 'TASK038_STORE_AUTHORITY_ACTIVATION_REJECTED')
    if (
        $rows.Count -ne 1 -or
        [string]$rows[0].admission_mode -ne 'ACTIVE' -or
        [string]$rows[0].daemon_instance_id -ne [string]$authority.daemon_instance_id -or
        [string]$rows[0].daemon_epoch -ne [string]$authority.daemon_epoch -or
        [string]$rows[0].authority_revision -ne [string]$authority.authority_revision -or
        [string]$rows[0].observation_digest -ne [string]$authority.observation_digest -or
        [string]$rows[0].head_digest -ne [string]$authority.head_digest
    ) {
        throw 'TASK038_STORE_AUTHORITY_ACTIVATION_REJECTED'
    }
    return $authority
}

function Get-PreMutationDatabaseFootprint {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$DatabaseName
    )

    $query = @"
SET ROLE lattice_migrator;
SELECT
  (SELECT count(*)::text
     FROM ONLY control.task_ledger_streams s
    WHERE s.project_id='task038-controlled-canary'
      AND s.project_snapshot_id='task038-controlled-canary:snapshot:1'
      AND s.task_id='TASK-038-CANARY'
      AND s.task_revision=1) AS task_streams,
  (SELECT count(*)::text
     FROM ONLY control.task_ledger_events e
     JOIN ONLY control.task_ledger_streams s ON s.stream_id=e.stream_id
    WHERE s.project_id='task038-controlled-canary'
      AND s.project_snapshot_id='task038-controlled-canary:snapshot:1'
      AND s.task_id='TASK-038-CANARY'
      AND s.task_revision=1) AS task_events,
  (SELECT count(*)::text FROM ONLY writer_lease.writer_lease_commands c
    WHERE c.project_id='task038-controlled-canary') AS writer_commands,
  (SELECT count(*)::text FROM ONLY writer_lease.writer_lease_transitions t
    WHERE t.project_id='task038-controlled-canary') AS writer_transitions,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_analyses) AS memory_analyses,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_receipts) AS memory_receipts,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_retrieval_audits) AS memory_retrieval_audits,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_records) AS memory_records,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_reflections) AS memory_reflections,
  (SELECT count(*)::text FROM ONLY memory.openclaw_gateway_commands) AS openclaw_commands;
"@
    $rows = @(Invoke-PsqlRows -Query $query -Header 'task_streams' -Password $Password -DatabaseName $DatabaseName -FailureCode 'TASK038_PRE_MUTATION_FOOTPRINT_REJECTED')
    if ($rows.Count -ne 1) { throw 'TASK038_PRE_MUTATION_FOOTPRINT_REJECTED_SHAPE' }
    $footprint = [ordered]@{
        task_streams = [int]$rows[0].task_streams
        task_events = [int]$rows[0].task_events
        writer_commands = [int]$rows[0].writer_commands
        writer_transitions = [int]$rows[0].writer_transitions
        memory_analyses = [int]$rows[0].memory_analyses
        memory_receipts = [int]$rows[0].memory_receipts
        memory_retrieval_audits = [int]$rows[0].memory_retrieval_audits
        memory_records = [int]$rows[0].memory_records
        memory_reflections = [int]$rows[0].memory_reflections
        openclaw_commands = [int]$rows[0].openclaw_commands
    }
    foreach ($name in @('task_streams', 'task_events', 'writer_commands', 'writer_transitions')) {
        if ([int]$footprint[$name] -ne 0) { throw 'TASK038_DATABASE_NOT_FRESH' }
    }
    return $footprint
}

function Invoke-WriterLeaseLiveSuite {
    param(
        [Parameter(Mandatory = $true)]$Identity,
        [Parameter(Mandatory = $true)]$Authority,
        [Parameter(Mandatory = $true)][string]$DatabaseName,
        [Parameter(Mandatory = $true)][string]$MigratorDsn,
        [Parameter(Mandatory = $true)][string]$RuntimeDsn,
        [Parameter(Mandatory = $true)][string]$AdminDsn,
        [Parameter(Mandatory = $true)][string]$EvidencePath
    )

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
    $original = @{}
    try {
        foreach ($entry in $values.GetEnumerator()) {
            $original[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, 'Process')
            [Environment]::SetEnvironmentVariable($entry.Key, [string]$entry.Value, 'Process')
        }
        $result = Invoke-NativeText -Executable $script:Cargo -WorkingDirectory $script:RepositoryRoot -Arguments @(
            'test', '-p', 'lattice-postgres-writer-lease', '--test', 'postgres_live', '--locked',
            '--', '--nocapture', '--test-threads=1'
        )
    }
    finally {
        foreach ($entry in $original.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, $entry.Value, 'Process')
        }
    }
    Assert-SecretFreeText -Text $result.Text -FailureCode 'TASK038_WRITER_LEASE_OUTPUT_SECRET_REJECTED'
    $skipped = $result.Text.IndexOf('SKIP:', [StringComparison]::Ordinal) -ge 0
    $testPassed = $result.Text -match '(?m)^test live_postgres_acquire_restarts_and_replays_authority_when_provisioned \.\.\. ok\s*$'
    if ($result.ExitCode -ne 0 -or $skipped -or -not $testPassed) {
        throw ('TASK038_WRITER_LEASE_LIVE_REJECTED|' + (Get-StringSha256 -Value $result.Text))
    }
    Write-JsonEvidence -Path $EvidencePath -Value ([ordered]@{
        schema_version = 'lattice.task038.writer-lease-live.v1'
        status = 'PASS'
        test = 'live_postgres_acquire_restarts_and_replays_authority_when_provisioned'
        skipped = $false
        admin_fault_path_exercised = $true
        output_sha256 = Get-StringSha256 -Value $result.Text
        output_line_count = [int]$result.LineCount
    })
}

function Get-DatabaseFootprint {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$DatabaseName,
        [AllowEmptyString()][string]$TaskRef
    )

    if (-not [string]::IsNullOrEmpty($TaskRef) -and $TaskRef -notmatch '^[0-9a-f]{64}$') {
        throw 'TASK038_TASK_REFERENCE_REJECTED'
    }
    $taskRefFilter = if ([string]::IsNullOrEmpty($TaskRef)) { [string]::Empty } else { " AND encode(s.task_spec_digest, 'hex') = '$TaskRef'" }
    $query = @"
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
  COALESCE((SELECT e.diagnostic->>'status' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EFFECT_OUTCOME'), '') AS delivery_status,
  COALESCE((SELECT e.diagnostic->>'failure_stage' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EFFECT_OUTCOME'), '') AS delivery_failure_stage,
  COALESCE((SELECT e.diagnostic->>'failure_code' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EFFECT_OUTCOME'), '') AS delivery_failure_code,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EVIDENCE_RECORDED' AND e.action_id='TASK_RESULT'), 0)::text AS task_results,
  COALESCE((SELECT e.command_id FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_command_id,
  COALESCE((SELECT e.actor_id FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_actor_id,
  COALESCE((SELECT e.action_id FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_action_id,
  COALESCE((SELECT e.reason_code FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_reason_code,
  COALESCE((SELECT e.diagnostic->>'schema' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_audit_schema,
  COALESCE((SELECT e.diagnostic->>'client_kind' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_client_kind,
  COALESCE((SELECT e.diagnostic->>'actor_kind' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_actor_kind,
  COALESCE((SELECT e.diagnostic->>'adapter_id' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_adapter_id,
  COALESCE((SELECT e.diagnostic->>'profile_adapter_commitment' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_profile_adapter_commitment,
  COALESCE((SELECT e.diagnostic->>'process_start_authority_digest' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_process_start_authority_digest,
  COALESCE((SELECT e.diagnostic->>'admission_observation_commitment' FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'), '') AS created_admission_observation_commitment,
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
 AND s.task_revision=1$taskRefFilter
LEFT JOIN ONLY writer_lease.writer_lease_heads w ON w.project_id='task038-controlled-canary';
"@
    $rows = @(Invoke-PsqlRows -Query $query -Header 'sequence' -Password $Password -DatabaseName $DatabaseName -FailureCode 'TASK038_DATABASE_FOOTPRINT_REJECTED')
    if ($rows.Count -ne 1) {
        throw 'TASK038_DATABASE_FOOTPRINT_REJECTED_SHAPE'
    }
    $row = $rows[0]
    $deliveryFailureCode = [string]$row.delivery_failure_code
    if (
        [string]$row.delivery_status -notin @('', 'COMPLETED', 'FAILED', 'RECONCILIATION_REQUIRED') -or
        [string]$row.delivery_failure_stage -notin @(
            '',
            'NONE',
            'INTENT',
            'WORKSPACE_PREPARE',
            'CODEX',
            'SCOPE_VERIFICATION',
            'FIXED_TEST',
            'GIT_COMMIT',
            'OUTCOME',
            'RECEIPT'
        ) -or
        ($deliveryFailureCode -ne '' -and $deliveryFailureCode -notmatch '^(?:NONE|[A-Z][A-Z0-9_]{2,127})$')
    ) {
        throw 'TASK038_DATABASE_FAILURE_PROJECTION_REJECTED'
    }
    $safe = [ordered]@{
        sequence = [int]$row.sequence
        event_count = [int]$row.event_count
        command_count = [int]$row.command_count
        task_ref = [string]$row.task_ref
        ledger_head_digest = [string]$row.ledger_head_digest
        task_created = [int]$row.task_created
        state_transitions = [int]$row.state_transitions
        codex_intents = [int]$row.codex_intents
        verified_outcomes = [int]$row.verified_outcomes
        delivery_status = [string]$row.delivery_status
        delivery_failure_stage = [string]$row.delivery_failure_stage
        delivery_failure_code_sha256 = Get-StringSha256 -Value $deliveryFailureCode
        task_results = [int]$row.task_results
        created_command_id = [string]$row.created_command_id
        created_actor_id = [string]$row.created_actor_id
        created_action_id = [string]$row.created_action_id
        created_reason_code = [string]$row.created_reason_code
        created_audit_schema = [string]$row.created_audit_schema
        created_client_kind = [string]$row.created_client_kind
        created_actor_kind = [string]$row.created_actor_kind
        created_adapter_id = [string]$row.created_adapter_id
        created_profile_adapter_commitment = [string]$row.created_profile_adapter_commitment
        created_process_start_authority_digest = [string]$row.created_process_start_authority_digest
        created_admission_observation_commitment = [string]$row.created_admission_observation_commitment
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
    return $safe
}

function Get-DirectoryFootprint {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [ValidateRange(1, 4096)][int]$MaxChildEntries = 4096,
        [ValidateRange(1, 134217728)][long]$MaxTotalBytes = 134217728,
        [ValidateRange(1, 67108864)][long]$MaxFileBytes = 67108864,
        [Diagnostics.Stopwatch]$DeadlineStopwatch,
        [long]$DeadlineMilliseconds = 0
    )

    if ($null -ne $DeadlineStopwatch -and $DeadlineMilliseconds -le 0) {
        throw 'TASK038_DIRECTORY_FOOTPRINT_DEADLINE_REJECTED'
    }

    $canonicalRoot = Get-CanonicalPath -Path $Root
    $rootItem = Get-Item -LiteralPath $canonicalRoot -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $rootItem -or
        -not $rootItem.PSIsContainer -or
        ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_DIRECTORY_FOOTPRINT_ROOT_REJECTED'
    }
    $records = [Collections.Generic.List[string]]::new()
    $items = [Collections.Generic.List[System.IO.FileSystemInfo]]::new()
    [long]$totalBytes = 0
    if ($null -ne $DeadlineStopwatch -and $DeadlineStopwatch.ElapsedMilliseconds -ge $DeadlineMilliseconds) {
        throw 'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT'
    }
    Get-ChildItem -LiteralPath $canonicalRoot -Recurse -Force | ForEach-Object {
        if ($null -ne $DeadlineStopwatch -and $DeadlineStopwatch.ElapsedMilliseconds -ge $DeadlineMilliseconds) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT'
        }
        if ($items.Count -ge $MaxChildEntries) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_ENTRY_LIMIT_REJECTED'
        }
        if ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_REPARSE_REJECTED'
        }
        if (-not $_.PSIsContainer) {
            if ($_.Length -gt $MaxFileBytes -or $totalBytes -gt ($MaxTotalBytes - $_.Length)) {
                throw 'TASK038_DIRECTORY_FOOTPRINT_BYTE_LIMIT_REJECTED'
            }
            $totalBytes += $_.Length
        }
        $items.Add($_)
    }
    $records.Add((
        'R|' + [int64]$rootItem.Attributes + '|' +
        $rootItem.CreationTimeUtc.Ticks + '|' + $rootItem.LastWriteTimeUtc.Ticks
    ))
    $directories = @($items | Where-Object { $_.PSIsContainer } | Sort-Object FullName)
    foreach ($directory in $directories) {
        $relative = $directory.FullName.Substring($canonicalRoot.Length).TrimStart([char[]]@('\', '/')).Replace('\', '/')
        $records.Add((
            'D|' + $relative + '|' + [int64]$directory.Attributes + '|' +
            $directory.CreationTimeUtc.Ticks + '|' + $directory.LastWriteTimeUtc.Ticks
        ))
    }
    $files = @($items | Where-Object { -not $_.PSIsContainer } | Sort-Object FullName)
    foreach ($file in $files) {
        if ($null -ne $DeadlineStopwatch -and $DeadlineStopwatch.ElapsedMilliseconds -ge $DeadlineMilliseconds) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT'
        }
        $relative = $file.FullName.Substring($canonicalRoot.Length).TrimStart([char[]]@('\', '/')).Replace('\', '/')
        $sha = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($null -ne $DeadlineStopwatch -and $DeadlineStopwatch.ElapsedMilliseconds -ge $DeadlineMilliseconds) {
            throw 'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT'
        }
        $records.Add((
            'F|' + $relative + '|' + [int64]$file.Attributes + '|' + $file.Length + '|' +
            $file.CreationTimeUtc.Ticks + '|' + $file.LastWriteTimeUtc.Ticks + '|' + $sha
        ))
    }
    if ($null -ne $DeadlineStopwatch -and $DeadlineStopwatch.ElapsedMilliseconds -ge $DeadlineMilliseconds) {
        throw 'TASK038_DIRECTORY_FOOTPRINT_SCAN_TIMEOUT'
    }
    return [ordered]@{
        file_count = $files.Count
        directory_count = $directories.Count
        entry_count = $records.Count
        total_bytes = $totalBytes
        digest = Get-StringSha256 -Value ([string]::Join("`n", $records))
    }
}

function Get-StableDirectoryFootprint {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [ValidateRange(1, 30)][int]$TimeoutSeconds = 15,
        [ValidateRange(50, 5000)][int]$QuietMilliseconds = 2000
    )

    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    $deadlineMilliseconds = [long]$TimeoutSeconds * 1000
    $previous = Get-DirectoryFootprint `
        -Root $Root `
        -DeadlineStopwatch $stopwatch `
        -DeadlineMilliseconds $deadlineMilliseconds
    if ($stopwatch.ElapsedMilliseconds -ge $deadlineMilliseconds) {
        throw 'TASK038_DIRECTORY_FOOTPRINT_NOT_STABLE'
    }
    $previousJson = $previous | ConvertTo-Json -Compress -Depth 8
    $stableSince = $stopwatch.ElapsedMilliseconds
    while ($stopwatch.ElapsedMilliseconds -lt $deadlineMilliseconds) {
        $remainingMilliseconds = $deadlineMilliseconds - $stopwatch.ElapsedMilliseconds
        Start-Sleep -Milliseconds ([Math]::Min(100, [Math]::Max(1, $remainingMilliseconds)))
        if ($stopwatch.ElapsedMilliseconds -ge $deadlineMilliseconds) {
            break
        }
        $current = Get-DirectoryFootprint `
            -Root $Root `
            -DeadlineStopwatch $stopwatch `
            -DeadlineMilliseconds $deadlineMilliseconds
        if ($stopwatch.ElapsedMilliseconds -ge $deadlineMilliseconds) {
            break
        }
        $currentJson = $current | ConvertTo-Json -Compress -Depth 8
        if ($currentJson -ne $previousJson) {
            $previous = $current
            $previousJson = $currentJson
            $stableSince = $stopwatch.ElapsedMilliseconds
            continue
        }
        if (($stopwatch.ElapsedMilliseconds - $stableSince) -ge $QuietMilliseconds) {
            return $current
        }
    }
    throw 'TASK038_DIRECTORY_FOOTPRINT_NOT_STABLE'
}

function Assert-CredentialSourceUnchanged {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$ExpectedFootprint,
        [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedAuthSha256
    )

    $actual = Get-DirectoryFootprint -Root $Root
    if (
        ($ExpectedFootprint | ConvertTo-Json -Compress -Depth 8) -ne
        ($actual | ConvertTo-Json -Compress -Depth 8) -or
        (Get-FileSha256 -Path (Join-Path $Root 'auth.json')) -ne $ExpectedAuthSha256
    ) {
        throw 'TASK038_CODEX_CREDENTIAL_SOURCE_MUTATED'
    }
}

function Invoke-GitText {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $result = Invoke-NativeText -Executable $script:Git -Arguments (@('-C', $Repository) + $Arguments)
    Assert-SecretFreeText -Text $result.Text -FailureCode 'TASK038_GIT_OUTPUT_SECRET_REJECTED'
    if ($result.ExitCode -ne 0) {
        throw ($FailureCode + '|' + (Get-StringSha256 -Value $result.Text))
    }
    return $result.Text.Trim()
}

function Get-GitFootprint {
    param([Parameter(Mandatory = $true)][string]$Repository)

    $head = Invoke-GitText -Repository $Repository -Arguments @('rev-parse', '--verify', 'HEAD') -FailureCode 'TASK038_GIT_HEAD_REJECTED'
    $parent = Invoke-GitText -Repository $Repository -Arguments @('rev-parse', '--verify', 'HEAD^') -FailureCode 'TASK038_GIT_PARENT_REJECTED'
    $tree = Invoke-GitText -Repository $Repository -Arguments @('rev-parse', 'HEAD^{tree}') -FailureCode 'TASK038_GIT_TREE_REJECTED'
    $commitCount = Invoke-GitText -Repository $Repository -Arguments @('rev-list', '--count', 'HEAD') -FailureCode 'TASK038_GIT_COUNT_REJECTED'
    $status = Invoke-GitText -Repository $Repository -Arguments @('status', '--porcelain=v1', '--untracked-files=all') -FailureCode 'TASK038_GIT_STATUS_REJECTED'
    $changed = Invoke-GitText -Repository $Repository -Arguments @('diff-tree', '--no-commit-id', '--name-only', '-r', '--no-renames', 'HEAD') -FailureCode 'TASK038_GIT_DIFF_REJECTED'
    $reflog = Invoke-GitText -Repository $Repository -Arguments @('reflog', '--format=%H') -FailureCode 'TASK038_GIT_REFLOG_REJECTED'
    $answerPath = Join-Path $Repository 'answer.txt'
    Assert-RegularFile -Path $answerPath -FailureCode 'TASK038_ANSWER_REJECTED'
    $expected = [Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n")
    $actual = [IO.File]::ReadAllBytes($answerPath)
    if ([Convert]::ToBase64String($actual) -ne [Convert]::ToBase64String($expected)) {
        throw 'TASK038_ANSWER_REJECTED'
    }
    if (
        $head -notmatch '^[0-9a-f]{40,64}$' -or
        $parent -notmatch '^[0-9a-f]{40,64}$' -or
        $tree -notmatch '^[0-9a-f]{40,64}$' -or
        [int]$commitCount -ne 2 -or
        -not [string]::IsNullOrEmpty($status) -or
        $changed -ne 'answer.txt'
    ) {
        throw 'TASK038_GIT_FOOTPRINT_REJECTED'
    }
    return [ordered]@{
        git_head = $head
        git_parent = $parent
        git_tree = $tree
        commit_count = [int]$commitCount
        clean = $true
        changed_path = $changed
        reflog_count = @($reflog -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count
        reflog_digest = Get-StringSha256 -Value $reflog
        answer_sha256 = Get-FileSha256 -Path $answerPath
    }
}

function New-McpInput {
    param([Parameter(Mandatory = $true)][object[]]$Frames)

    return ((@($Frames | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 12 }) -join "`n") + "`n")
}

function Initialize-Task038JobObjectInterop {
    if ($null -ne ('LatticeTask038JobObjectInterop' -as [type])) {
        return
    }
    try {
        Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Collections;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class LatticeTask038JobObjectInterop
{
    private const UInt32 JobObjectExtendedLimitInformation = 9;
    private const UInt32 JobObjectLimitKillOnJobClose = 0x00002000;

    [StructLayout(LayoutKind.Sequential)]
    private struct IoCounters
    {
        public UInt64 ReadOperationCount;
        public UInt64 WriteOperationCount;
        public UInt64 OtherOperationCount;
        public UInt64 ReadTransferCount;
        public UInt64 WriteTransferCount;
        public UInt64 OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BasicLimitInformation
    {
        public Int64 PerProcessUserTimeLimit;
        public Int64 PerJobUserTimeLimit;
        public UInt32 LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public UInt32 ActiveProcessLimit;
        public UIntPtr Affinity;
        public UInt32 PriorityClass;
        public UInt32 SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ExtendedLimitInformation
    {
        public BasicLimitInformation BasicLimitInformation;
        public IoCounters IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BasicAccountingInformation
    {
        public Int64 TotalUserTime;
        public Int64 TotalKernelTime;
        public Int64 ThisPeriodTotalUserTime;
        public Int64 ThisPeriodTotalKernelTime;
        public UInt32 TotalPageFaultCount;
        public UInt32 TotalProcesses;
        public UInt32 ActiveProcesses;
        public UInt32 TotalTerminatedProcesses;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateJobObject(IntPtr jobAttributes, string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool SetInformationJobObject(
        IntPtr job,
        UInt32 informationClass,
        IntPtr information,
        UInt32 informationLength);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool QueryInformationJobObject(
        IntPtr job,
        UInt32 informationClass,
        out BasicAccountingInformation information,
        UInt32 informationLength,
        IntPtr returnLength);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool TerminateJobObject(IntPtr job, UInt32 exitCode);

    public static IntPtr CreateKillOnClose()
    {
        IntPtr job = CreateJobObject(IntPtr.Zero, null);
        if (job == IntPtr.Zero)
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        var information = new ExtendedLimitInformation();
        information.BasicLimitInformation.LimitFlags = JobObjectLimitKillOnJobClose;
        int size = Marshal.SizeOf(typeof(ExtendedLimitInformation));
        IntPtr buffer = Marshal.AllocHGlobal(size);
        try
        {
            Marshal.StructureToPtr(information, buffer, false);
            if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, buffer, (UInt32)size))
            {
                int error = Marshal.GetLastWin32Error();
                CloseHandle(job);
                throw new Win32Exception(error);
            }
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
        return job;
    }

    public static void Assign(IntPtr job, IntPtr process)
    {
        if (job == IntPtr.Zero || process == IntPtr.Zero || !AssignProcessToJobObject(job, process))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }

    public static UInt32 ActiveProcessCount(IntPtr job)
    {
        BasicAccountingInformation information;
        UInt32 size = (UInt32)Marshal.SizeOf(typeof(BasicAccountingInformation));
        if (job == IntPtr.Zero ||
            !QueryInformationJobObject(job, 1, out information, size, IntPtr.Zero))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        return information.ActiveProcesses;
    }

    public static void Terminate(IntPtr job)
    {
        if (job == IntPtr.Zero || !TerminateJobObject(job, 1))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }

    public static void Close(IntPtr job)
    {
        if (job != IntPtr.Zero && !CloseHandle(job))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }
}
'@
    }
    catch {
        throw 'TASK038_LATTICED_JOB_OBJECT_REJECTED'
    }
}

function Get-Task038CandidateSourceLinkage {
    param([Parameter(Mandatory = $true)][string]$Repository)

    $commit = Invoke-GitText `
        -Repository $Repository `
        -Arguments @('rev-parse', '--verify', 'HEAD') `
        -FailureCode 'TASK038_CANDIDATE_COMMIT_REJECTED'
    $tree = Invoke-GitText `
        -Repository $Repository `
        -Arguments @('rev-parse', 'HEAD^{tree}') `
        -FailureCode 'TASK038_CANDIDATE_TREE_REJECTED'
    $status = Invoke-GitText `
        -Repository $Repository `
        -Arguments @('status', '--porcelain=v1', '--untracked-files=all') `
        -FailureCode 'TASK038_CANDIDATE_STATUS_REJECTED'
    if (
        $commit -cnotmatch '\A[0-9a-f]{40}\z' -or
        $tree -cnotmatch '\A[0-9a-f]{40}\z' -or
        -not [string]::IsNullOrEmpty($status)
    ) {
        throw 'TASK038_CANDIDATE_SOURCE_REJECTED'
    }
    $ownedPaths = [ordered]@{
        p0_05 = @(
            'apps/lattice-runtime/src/mcp.rs',
            'apps/lattice-runtime/tests/mcp.rs',
            'scripts/windows-native-path-identity.ps1',
            'scripts/run-task019-postgres.ps1',
            'scripts/run-task038-task-submit.ps1',
            ('scripts/start-' + 'chatgpt-mcp-tunnel.ps1'),
            'scripts/test-task038-local-acceptance.ps1',
            'scripts/test-chatgpt-mcp-tunnel-entrypoint.ps1'
        )
        p0_07 = @(
            'Cargo.lock',
            'apps/lattice-runtime/Cargo.toml',
            'apps/lattice-runtime/src/composition.rs',
            'apps/lattice-runtime/tests/composition.rs'
        )
        p0_06 = @(
            'scripts/run-task038-four-tool-acceptance.ps1',
            'scripts/test-task038-four-tool-acceptance.ps1'
        )
    }
    $entries = [Collections.Generic.List[object]]::new()
    foreach ($owner in $ownedPaths.Keys) {
        foreach ($path in $ownedPaths[$owner]) {
            $blob = Invoke-GitText `
                -Repository $Repository `
                -Arguments @('rev-parse', ($commit + ':' + $path)) `
                -FailureCode 'TASK038_CANDIDATE_BLOB_REJECTED'
            $lastChangeCommit = Invoke-GitText `
                -Repository $Repository `
                -Arguments @('log', '-1', '--format=%H', '--', $path) `
                -FailureCode 'TASK038_CANDIDATE_PATH_COMMIT_REJECTED'
            if (
                $blob -cnotmatch '\A[0-9a-f]{40}\z' -or
                $lastChangeCommit -cnotmatch '\A[0-9a-f]{40}\z'
            ) {
                throw 'TASK038_CANDIDATE_LINKAGE_REJECTED'
            }
            $entries.Add([ordered]@{
                owner = [string]$owner
                path = [string]$path
                blob = $blob
                last_change_commit = $lastChangeCommit
            })
        }
    }
    $entryCommitment = Get-StringSha256 -Value (@($entries) | ConvertTo-Json -Compress -Depth 6)
    return [ordered]@{
        schema_version = 'lattice.task038.candidate-source-linkage.v1'
        source_commit = $commit
        source_tree = $tree
        source_status_clean = $true
        exact_path_count = [int]$entries.Count
        exact_path_entries = @($entries)
        exact_path_entries_sha256 = $entryCommitment
    }
}

function Set-Task038OwnerOnlyAcl {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$Directory
    )

    try {
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
        if ($Directory) {
            $security = [Security.AccessControl.DirectorySecurity]::new()
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                $sid,
                [Security.AccessControl.FileSystemRights]::FullControl,
                [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
                [Security.AccessControl.PropagationFlags]::None,
                [Security.AccessControl.AccessControlType]::Allow
            )
            $security.SetOwner($sid)
            $security.SetAccessRuleProtection($true, $false)
            [void]$security.AddAccessRule($rule)
            [IO.Directory]::SetAccessControl($Path, $security)
        }
        else {
            $security = [Security.AccessControl.FileSecurity]::new()
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                $sid,
                [Security.AccessControl.FileSystemRights]::FullControl,
                [Security.AccessControl.AccessControlType]::Allow
            )
            $security.SetOwner($sid)
            $security.SetAccessRuleProtection($true, $false)
            [void]$security.AddAccessRule($rule)
            [IO.File]::SetAccessControl($Path, $security)
        }
    }
    catch {
        throw 'TASK038_MCP_ACCEPTANCE_EVIDENCE_ACL_REJECTED'
    }
}

function New-Task038McpAcceptanceEvidenceSink {
    param(
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })][string]$SessionId
    )

    $root = Get-CanonicalPath -Path (Join-Path $EvidenceRoot 'mcp-dispatch')
    if (-not (Test-Path -LiteralPath $root -PathType Container)) {
        New-Item -ItemType Directory -Path $root -Force:$false | Out-Null
        Set-Task038OwnerOnlyAcl -Path $root -Directory $true
    }
    Assert-NoReparseAncestor `
        -Path $root `
        -Boundary $script:RepositoryRoot `
        -FailureCode 'TASK038_MCP_ACCEPTANCE_EVIDENCE_PATH_REJECTED'
    $path = Get-CanonicalPath -Path (Join-Path $root ($SessionId + '.jsonl'))
    Write-Task038ExclusiveBytes `
        -Path $path `
        -Bytes ([byte[]]::new(0)) `
        -FailureCode 'TASK038_MCP_ACCEPTANCE_EVIDENCE_NOT_FRESH'
    Set-Task038OwnerOnlyAcl -Path $path -Directory $false
    return [pscustomobject]@{
        path = $path
        native_identity = Get-LatticeWindowsNativePathIdentityToken -Path $path -Directory $false
    }
}

function Read-Task038McpAcceptanceEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedNativeIdentity,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })][string]$SessionId,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{64}\z' })][string]$SafeConfigSha256,
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][ValidateRange(0, 64)][int]$ExpectedDispatchCount
    )

    $failureCode = 'TASK038_MCP_ACCEPTANCE_EVIDENCE_REJECTED'
    if (-not (Test-LatticeWindowsNativePathIdentity `
            -Path $Path `
            -Directory $false `
            -ExpectedToken $ExpectedNativeIdentity)) {
        throw $failureCode
    }
    $bytes = [IO.File]::ReadAllBytes($Path)
    if (
        $bytes.Length -lt 1 -or
        $bytes.Length -gt 1048576 -or
        ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf)
    ) {
        throw $failureCode
    }
    try { $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes) }
    catch { throw $failureCode }
    if (-not $text.EndsWith("`n", [StringComparison]::Ordinal) -or $text.Contains("`r")) {
        throw $failureCode
    }
    $lines = @($text.Split([string[]]@("`n"), [StringSplitOptions]::None))
    if ($lines[-1] -cne '') { throw $failureCode }
    $lines = @($lines[0..($lines.Count - 2)])
    if ($lines.Count -ne ($ExpectedDispatchCount + 2)) { throw $failureCode }
    $previousEventSha256 = '0' * 64
    $dispatchCount = 0
    $records = [Collections.Generic.List[object]]::new()
    for ($index = 0; $index -lt $lines.Count; $index++) {
        try { $record = $lines[$index] | ConvertFrom-Json -ErrorAction Stop }
        catch { throw $failureCode }
        $keys = @($record.PSObject.Properties.Name | Sort-Object)
        $expectedKeys = @(
            'dispatch_accepted_count', 'event_sha256', 'observed_at_unix_nanos',
            'ordinal', 'previous_event_sha256', 'process_id', 'record_type',
            'request_id_sha256', 'safe_config_sha256', 'schema', 'session_id', 'tool_name'
        ) | Sort-Object
        if (($keys -join "`n") -cne ($expectedKeys -join "`n")) { throw $failureCode }
        $expectedType = if ($index -eq 0) {
            'SESSION_OPEN'
        }
        elseif ($index -eq $lines.Count - 1) {
            'SESSION_CLOSED'
        }
        else {
            'DISPATCH_ACCEPTED'
        }
        if ($expectedType -ceq 'DISPATCH_ACCEPTED') { $dispatchCount++ }
        if (
            [string]$record.schema -cne 'lattice.mcp.acceptance-dispatch.v1' -or
            [string]$record.record_type -cne $expectedType -or
            [string]$record.session_id -cne $SessionId -or
            [string]$record.safe_config_sha256 -cne $SafeConfigSha256 -or
            [long]$record.process_id -ne $ProcessId -or
            [long]$record.ordinal -ne ($index + 1) -or
            [long]$record.dispatch_accepted_count -ne $dispatchCount -or
            [string]$record.observed_at_unix_nanos -cnotmatch '\A[1-9][0-9]*\z' -or
            [string]$record.previous_event_sha256 -cne $previousEventSha256 -or
            [string]$record.event_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
        ) {
            throw $failureCode
        }
        if ($expectedType -ceq 'DISPATCH_ACCEPTED') {
            if (
                [string]$record.tool_name -cnotmatch '\A(?:lattice_delivery_run|lattice_delivery_status|lattice_task_submit|lattice_task_status)\z' -or
                [string]$record.request_id_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
            ) { throw $failureCode }
            $toolName = [string]$record.tool_name
            $requestIdSha256 = [string]$record.request_id_sha256
        }
        else {
            if ($null -ne $record.tool_name -or $null -ne $record.request_id_sha256) { throw $failureCode }
            $toolName = 'null'
            $requestIdSha256 = 'null'
        }
        $hashInput = @(
            'lattice.mcp.acceptance-dispatch-hash.v1',
            $previousEventSha256,
            $SessionId,
            $SafeConfigSha256,
            $expectedType,
            [string]($index + 1),
            [string]$ProcessId,
            $toolName,
            $requestIdSha256,
            [string]$dispatchCount,
            [string]$record.observed_at_unix_nanos
        ) -join "`n"
        $eventSha256 = Get-StringSha256 -Value $hashInput
        if ([string]$record.event_sha256 -cne $eventSha256) { throw $failureCode }
        $previousEventSha256 = $eventSha256
        $records.Add($record)
    }
    return [pscustomobject]@{
        schema = 'lattice.task038.mcp-acceptance-dispatch-evidence.v1'
        path = $Path
        raw_sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
        byte_count = [long]$bytes.Length
        strict_utf8 = $true
        session_id = $SessionId
        safe_config_sha256 = $SafeConfigSha256
        process_id = $ProcessId
        record_count = [int]$lines.Count
        dispatch_accepted_count = $dispatchCount
        final_event_sha256 = $previousEventSha256
        normal_close_complete = $true
        native_identity = $ExpectedNativeIdentity
        records = @($records)
    }
}

function Initialize-Task038SuspendedProcessInterop {
    if ($null -ne ('LatticeTask038SuspendedProcessFactory' -as [type])) {
        return
    }
    try {
        Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Collections;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public sealed class LatticeTask038SuspendedProcess : IDisposable
{
    private IntPtr threadHandle;
    private bool resumed;

    internal LatticeTask038SuspendedProcess(
        Process process,
        IntPtr threadHandle,
        StreamWriter standardInput,
        StreamReader standardOutput,
        StreamReader standardError)
    {
        Process = process;
        this.threadHandle = threadHandle;
        StandardInput = standardInput;
        StandardOutput = standardOutput;
        StandardError = standardError;
    }

    public Process Process { get; private set; }
    public StreamWriter StandardInput { get; private set; }
    public StreamReader StandardOutput { get; private set; }
    public StreamReader StandardError { get; private set; }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern UInt32 ResumeThread(IntPtr thread);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    public void Resume()
    {
        if (resumed || threadHandle == IntPtr.Zero)
        {
            throw new InvalidOperationException("Suspended process already resumed.");
        }
        if (ResumeThread(threadHandle) == UInt32.MaxValue)
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        resumed = true;
        if (!CloseHandle(threadHandle))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        threadHandle = IntPtr.Zero;
    }

    public void Dispose()
    {
        if (threadHandle != IntPtr.Zero)
        {
            CloseHandle(threadHandle);
            threadHandle = IntPtr.Zero;
        }
        if (StandardInput != null) { StandardInput.Dispose(); StandardInput = null; }
        if (StandardOutput != null) { StandardOutput.Dispose(); StandardOutput = null; }
        if (StandardError != null) { StandardError.Dispose(); StandardError = null; }
        if (Process != null) { Process.Dispose(); Process = null; }
    }
}

public static class LatticeTask038SuspendedProcessFactory
{
    private const UInt32 CREATE_SUSPENDED = 0x00000004;
    private const UInt32 CREATE_NO_WINDOW = 0x08000000;
    private const UInt32 CREATE_UNICODE_ENVIRONMENT = 0x00000400;
    private const UInt32 STARTF_USESTDHANDLES = 0x00000100;
    private const UInt32 HANDLE_FLAG_INHERIT = 0x00000001;

    [StructLayout(LayoutKind.Sequential)]
    private struct SecurityAttributes
    {
        public Int32 Length;
        public IntPtr SecurityDescriptor;
        [MarshalAs(UnmanagedType.Bool)] public bool InheritHandle;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct StartupInfo
    {
        public Int32 Size;
        public string Reserved;
        public string Desktop;
        public string Title;
        public UInt32 X;
        public UInt32 Y;
        public UInt32 XSize;
        public UInt32 YSize;
        public UInt32 XCountChars;
        public UInt32 YCountChars;
        public UInt32 FillAttribute;
        public UInt32 Flags;
        public UInt16 ShowWindow;
        public UInt16 Reserved2;
        public IntPtr Reserved2Pointer;
        public IntPtr StandardInput;
        public IntPtr StandardOutput;
        public IntPtr StandardError;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInformation
    {
        public IntPtr Process;
        public IntPtr Thread;
        public UInt32 ProcessId;
        public UInt32 ThreadId;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CreatePipe(
        out IntPtr readPipe,
        out IntPtr writePipe,
        ref SecurityAttributes pipeAttributes,
        UInt32 size);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetHandleInformation(IntPtr handle, UInt32 mask, UInt32 flags);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CreateProcessW(
        string applicationName,
        StringBuilder commandLine,
        IntPtr processAttributes,
        IntPtr threadAttributes,
        [MarshalAs(UnmanagedType.Bool)] bool inheritHandles,
        UInt32 creationFlags,
        IntPtr environment,
        string currentDirectory,
        ref StartupInfo startupInfo,
        out ProcessInformation processInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool TerminateProcess(IntPtr process, UInt32 exitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr handle);

    private static void CloseIfPresent(ref IntPtr handle)
    {
        if (handle != IntPtr.Zero)
        {
            CloseHandle(handle);
            handle = IntPtr.Zero;
        }
    }

    private static IntPtr BuildEnvironmentBlock(StringDictionary environment)
    {
        var values = new List<string>();
        foreach (DictionaryEntry entry in environment)
        {
            string name = Convert.ToString(entry.Key);
            string value = Convert.ToString(entry.Value);
            if (String.IsNullOrEmpty(name) || name.IndexOf('=') >= 0 ||
                name.IndexOf('\0') >= 0 || value.IndexOf('\0') >= 0)
            {
                throw new InvalidOperationException("Invalid child environment entry.");
            }
            values.Add(name + "=" + value);
        }
        values.Sort(StringComparer.OrdinalIgnoreCase);
        string block = String.Join("\0", values.ToArray()) + "\0\0";
        return Marshal.StringToHGlobalUni(block);
    }

    public static LatticeTask038SuspendedProcess Start(
        string executable,
        string arguments,
        StringDictionary environment)
    {
        if (String.IsNullOrWhiteSpace(executable) || executable.IndexOf('"') >= 0 || environment == null)
        {
            throw new ArgumentException("Invalid suspended-process input.");
        }

        IntPtr childStdinRead = IntPtr.Zero;
        IntPtr parentStdinWrite = IntPtr.Zero;
        IntPtr parentStdoutRead = IntPtr.Zero;
        IntPtr childStdoutWrite = IntPtr.Zero;
        IntPtr parentStderrRead = IntPtr.Zero;
        IntPtr childStderrWrite = IntPtr.Zero;
        IntPtr environmentBlock = IntPtr.Zero;
        var processInformation = new ProcessInformation();
        bool created = false;
        try
        {
            var attributes = new SecurityAttributes {
                Length = Marshal.SizeOf(typeof(SecurityAttributes)),
                SecurityDescriptor = IntPtr.Zero,
                InheritHandle = true
            };
            if (!CreatePipe(out childStdinRead, out parentStdinWrite, ref attributes, 0) ||
                !SetHandleInformation(parentStdinWrite, HANDLE_FLAG_INHERIT, 0) ||
                !CreatePipe(out parentStdoutRead, out childStdoutWrite, ref attributes, 0) ||
                !SetHandleInformation(parentStdoutRead, HANDLE_FLAG_INHERIT, 0) ||
                !CreatePipe(out parentStderrRead, out childStderrWrite, ref attributes, 0) ||
                !SetHandleInformation(parentStderrRead, HANDLE_FLAG_INHERIT, 0))
            {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }

            var startupInfo = new StartupInfo {
                Size = Marshal.SizeOf(typeof(StartupInfo)),
                Flags = STARTF_USESTDHANDLES,
                StandardInput = childStdinRead,
                StandardOutput = childStdoutWrite,
                StandardError = childStderrWrite
            };
            environmentBlock = BuildEnvironmentBlock(environment);
            var commandLine = new StringBuilder("\"" + executable + "\" " + (arguments ?? String.Empty));
            if (!CreateProcessW(
                executable,
                commandLine,
                IntPtr.Zero,
                IntPtr.Zero,
                true,
                CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                environmentBlock,
                null,
                ref startupInfo,
                out processInformation))
            {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
            created = true;
            CloseIfPresent(ref childStdinRead);
            CloseIfPresent(ref childStdoutWrite);
            CloseIfPresent(ref childStderrWrite);

            var stdinHandle = new SafeFileHandle(parentStdinWrite, true);
            parentStdinWrite = IntPtr.Zero;
            var stdoutHandle = new SafeFileHandle(parentStdoutRead, true);
            parentStdoutRead = IntPtr.Zero;
            var stderrHandle = new SafeFileHandle(parentStderrRead, true);
            parentStderrRead = IntPtr.Zero;
            var strictUtf8 = new UTF8Encoding(false, true);
            var standardInput = new StreamWriter(new FileStream(stdinHandle, FileAccess.Write, 4096, false), strictUtf8);
            standardInput.AutoFlush = true;
            var standardOutput = new StreamReader(new FileStream(stdoutHandle, FileAccess.Read, 4096, false), strictUtf8, false, 4096, false);
            var standardError = new StreamReader(new FileStream(stderrHandle, FileAccess.Read, 4096, false), strictUtf8, false, 4096, false);
            var process = Process.GetProcessById((Int32)processInformation.ProcessId);
            CloseIfPresent(ref processInformation.Process);
            IntPtr thread = processInformation.Thread;
            processInformation.Thread = IntPtr.Zero;
            return new LatticeTask038SuspendedProcess(process, thread, standardInput, standardOutput, standardError);
        }
        catch
        {
            if (created && processInformation.Process != IntPtr.Zero)
            {
                TerminateProcess(processInformation.Process, 1);
            }
            throw;
        }
        finally
        {
            if (environmentBlock != IntPtr.Zero) { Marshal.FreeHGlobal(environmentBlock); }
            CloseIfPresent(ref childStdinRead);
            CloseIfPresent(ref parentStdinWrite);
            CloseIfPresent(ref parentStdoutRead);
            CloseIfPresent(ref childStdoutWrite);
            CloseIfPresent(ref parentStderrRead);
            CloseIfPresent(ref childStderrWrite);
            CloseIfPresent(ref processInformation.Process);
            CloseIfPresent(ref processInformation.Thread);
        }
    }
}
'@
    }
    catch {
        throw 'TASK038_LATTICED_PROCESS_INTEROP_REJECTED'
    }
}

function New-Task038KillOnCloseJob {
    Initialize-Task038JobObjectInterop
    try {
        return [LatticeTask038JobObjectInterop]::CreateKillOnClose()
    }
    catch {
        throw 'TASK038_LATTICED_JOB_OBJECT_REJECTED'
    }
}

function Add-Task038ProcessToJob {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process
    )

    try {
        [LatticeTask038JobObjectInterop]::Assign($Job, $Process.Handle)
    }
    catch {
        throw 'TASK038_LATTICED_JOB_ASSIGN_REJECTED'
    }
}

function Start-Task038SuspendedProcess {
    param([Parameter(Mandatory = $true)][Diagnostics.ProcessStartInfo]$StartInfo)

    Initialize-Task038SuspendedProcessInterop
    if (
        $StartInfo.UseShellExecute -or
        -not $StartInfo.RedirectStandardInput -or
        -not $StartInfo.RedirectStandardOutput -or
        -not $StartInfo.RedirectStandardError
    ) {
        throw 'TASK038_LATTICED_START_REJECTED'
    }
    try {
        return [LatticeTask038SuspendedProcessFactory]::Start(
            $StartInfo.FileName,
            $StartInfo.Arguments,
            $StartInfo.EnvironmentVariables
        )
    }
    catch {
        throw 'TASK038_LATTICED_START_REJECTED'
    }
}

function Resume-Task038SuspendedProcess {
    param([Parameter(Mandatory = $true)][LatticeTask038SuspendedProcess]$SuspendedProcess)

    try {
        $SuspendedProcess.Resume()
    }
    catch {
        throw 'TASK038_LATTICED_RESUME_REJECTED'
    }
}

function Close-Task038Job {
    param([Parameter(Mandatory = $true)][IntPtr]$Job)

    try {
        [LatticeTask038JobObjectInterop]::Close($Job)
    }
    catch {
        throw 'TASK038_LATTICED_JOB_CLEANUP_REJECTED'
    }
}

function Stop-Task038Job {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [ValidateRange(1, 10000)][int]$TimeoutMilliseconds = 5000
    )

    try {
        $active = [uint32][LatticeTask038JobObjectInterop]::ActiveProcessCount($Job)
        if ($active -gt 0) {
            [LatticeTask038JobObjectInterop]::Terminate($Job)
        }
        $stopwatch = [Diagnostics.Stopwatch]::StartNew()
        while ($stopwatch.ElapsedMilliseconds -lt $TimeoutMilliseconds) {
            $active = [uint32][LatticeTask038JobObjectInterop]::ActiveProcessCount($Job)
            if ($active -eq 0) {
                return
            }
            Start-Sleep -Milliseconds 10
        }
    }
    catch {
        throw 'TASK038_LATTICED_JOB_TERMINATION_REJECTED'
    }
    throw 'TASK038_LATTICED_JOB_TERMINATION_REJECTED'
}

function Stop-Task038ProcessTree {
    param([Parameter(Mandatory = $true)][Diagnostics.Process]$Process)

    try {
        if ($Process.HasExited) {
            return
        }
    }
    catch {
        throw 'TASK038_LATTICED_PROCESS_CLEANUP_REJECTED'
    }

    try {
        $Process.Kill()
    }
    catch {
        try {
            if ($Process.HasExited) { return }
        }
        catch { }
        throw 'TASK038_LATTICED_PROCESS_CLEANUP_REJECTED'
    }

    if (-not $Process.WaitForExit(5000)) {
        throw 'TASK038_LATTICED_PROCESS_CLEANUP_REJECTED'
    }
}

function Invoke-LatticedSession {
    param(
        [Parameter(Mandatory = $true)][string]$InputText,
        [Parameter(Mandatory = $true)][ValidateSet('FRESH', 'RESUME_EXISTING')][string]$RunMode,
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [Parameter(Mandatory = $true)][string]$MetaPath,
        [Parameter(Mandatory = $true)]$Authority,
        [Parameter(Mandatory = $true)][string]$DatabasePassword,
        [Parameter(Mandatory = $true)][string]$DeliveryRoot,
        [Parameter(Mandatory = $true)][string]$SchemaDirectory,
        [Parameter(Mandatory = $true)][string]$LauncherSha256,
        [Parameter(Mandatory = $true)][string]$LauncherVersion,
        [Parameter(Mandatory = $true)][string]$AcceptanceEvidencePath,
        [Parameter(Mandatory = $true)][string]$AcceptanceEvidenceNativeIdentity,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })][string]$AcceptanceSessionId,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{64}\z' })][string]$AcceptanceSafeConfigSha256,
        [Parameter(Mandatory = $true)][ValidateRange(0, 64)][int]$ExpectedDispatchCount
    )

    $latticedPidPath = Get-CanonicalPath -Path ($MetaPath + '.latticed-pid.tmp')
    if (Test-Path -LiteralPath $latticedPidPath) {
        throw 'TASK038_LATTICED_PID_EVIDENCE_NOT_FRESH'
    }
    $pidParent = Get-CanonicalPath -Path (Split-Path -Parent $latticedPidPath)
    Assert-NoReparseAncestor `
        -Path $pidParent `
        -Boundary $script:RepositoryRoot `
        -FailureCode 'TASK038_LATTICED_PID_EVIDENCE_REJECTED'
    $wrapperSource = @'
$ErrorActionPreference = 'Stop'
$child = $null
try {
    $executable = [Environment]::GetEnvironmentVariable('LATTICE_TASK038_WRAPPED_EXECUTABLE', 'Process')
    $pidPath = [Environment]::GetEnvironmentVariable('LATTICE_TASK038_WRAPPED_PID_PATH', 'Process')
    $inputText = [Console]::In.ReadToEnd()
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    foreach ($name in @(
        'LATTICE_TASK038_WRAPPED_EXECUTABLE',
        'LATTICE_TASK038_WRAPPED_PID_PATH'
    )) {
        $startInfo.EnvironmentVariables.Remove($name)
    }
    $child = [Diagnostics.Process]::new()
    $child.StartInfo = $startInfo
    if (-not $child.Start()) { exit 112 }
    $stdoutTask = $child.StandardOutput.ReadToEndAsync()
    $stderrTask = $child.StandardError.ReadToEndAsync()
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(([string]$child.Id + "`n"))
    $stream = [IO.File]::Open($pidPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    }
    finally {
        $stream.Dispose()
    }
    $child.StandardInput.Write($inputText)
    $child.StandardInput.Close()
    $child.WaitForExit()
    if (-not $stdoutTask.Wait(5000) -or -not $stderrTask.Wait(5000)) { exit 114 }
    [Console]::Out.Write([string]$stdoutTask.Result)
    [Console]::Error.Write([string]$stderrTask.Result)
    $exitCode = [int]$child.ExitCode
    exit $exitCode
}
catch {
    exit 113
}
finally {
    if ($null -ne $child) { $child.Dispose() }
}
'@
    $wrapperEncodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($wrapperSource))

    $environmentValues = [ordered]@{
        LATTICE_FULL_CHAIN_RUN_MODE = $RunMode
        LATTICE_TASK_INGRESS_KIND = 'LOCAL_CANONICAL_MCP_ACCEPTANCE'
        LATTICE_TASK_INGRESS_PROFILE_SHA256 = $script:IngressProfileDigest
        LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
        LATTICE_DELIVERY_TIMEOUT_SECONDS = [string]$TimeoutSeconds
        LATTICE_TASK019_HOST = $PostgresHost
        LATTICE_TASK019_PORT = [string]$PostgresPort
        LATTICE_TASK019_RUN_ID = $PostgresRunId
        LATTICE_TASK019_PASSWORD = $DatabasePassword
        LATTICE_STORE_DAEMON_INSTANCE_ID = [string]$Authority.daemon_instance_id
        LATTICE_STORE_DAEMON_EPOCH = [string]$Authority.daemon_epoch
        LATTICE_STORE_AUTHORITY_REVISION = [string]$Authority.authority_revision
        LATTICE_STORE_OBSERVATION_DIGEST = [string]$Authority.observation_digest
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = [string]$Authority.head_digest
        LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH = $AcceptanceEvidencePath
        LATTICE_MCP_ACCEPTANCE_SESSION_ID = $AcceptanceSessionId
        LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256 = $AcceptanceSafeConfigSha256
        LATTICE_TASK038_WRAPPED_EXECUTABLE = $script:Latticed
        LATTICE_TASK038_WRAPPED_PID_PATH = $latticedPidPath
    }
    if ($RunMode -eq 'FRESH') {
        $environmentValues.LATTICE_DELIVERY_LAUNCHER = $script:OfficialCodex
        $environmentValues.LATTICE_DELIVERY_LAUNCHER_VERSION = $LauncherVersion
        $environmentValues.LATTICE_DELIVERY_LAUNCHER_SHA256 = $LauncherSha256
        $environmentValues.LATTICE_DELIVERY_SCHEMA_DIR = $SchemaDirectory
        $environmentValues.LATTICE_DELIVERY_CODEX_HOME = $script:CodexHome
        $environmentValues.LATTICE_DELIVERY_ROOT = $DeliveryRoot
        $environmentValues.LATTICE_DELIVERY_GIT_EXE = $script:Git
    }

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $script:PowerShell
    $startInfo.Arguments = ('-NoProfile -NonInteractive -EncodedCommand ' + $wrapperEncodedCommand)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    Set-Task038ClosedChildEnvironment -StartInfo $startInfo -EnvironmentValues $environmentValues

    $process = $null
    $suspendedProcess = $null
    $startedAt = [DateTime]::UtcNow
    $originalConsoleInputEncoding = [Console]::InputEncoding
    $started = $false
    $stdoutTask = $null
    $stderrTask = $null
    $stdout = [string]::Empty
    $stderr = [string]::Empty
    $exitCode = $null
    $controllerProcessId = 0
    $childProcessId = 0
    $primaryFailure = $null
    $cleanupFailure = $null
    $jobHandle = [IntPtr]::Zero
    $jobAssigned = $false
    $resumedAfterJobAssignment = $false
    $jobProcessCountZero = $false
    try {
        [Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
        $jobHandle = New-Task038KillOnCloseJob
        $suspendedProcess = Start-Task038SuspendedProcess -StartInfo $startInfo
        $process = $suspendedProcess.Process
        $started = $true
        $controllerProcessId = [int]$process.Id
        Add-Task038ProcessToJob -Job $jobHandle -Process $process
        $jobAssigned = $true
        Resume-Task038SuspendedProcess -SuspendedProcess $suspendedProcess
        $resumedAfterJobAssignment = $true
        try {
            $stdoutTask = $suspendedProcess.StandardOutput.ReadToEndAsync()
            $stderrTask = $suspendedProcess.StandardError.ReadToEndAsync()
            $suspendedProcess.StandardInput.Write($InputText)
            $suspendedProcess.StandardInput.Close()
        }
        catch {
            throw 'TASK038_LATTICED_STDIN_REJECTED'
        }
        $watchdogSeconds = $TimeoutSeconds + 15
        if (-not $process.WaitForExit($watchdogSeconds * 1000)) {
            throw 'TASK038_LATTICED_TIMEOUT'
        }
        $exitCode = [int]$process.ExitCode
    }
    catch {
        $primaryFailure = Get-Task038FailureClassification -ErrorRecord $_
    }
    finally {
        [Console]::InputEncoding = $originalConsoleInputEncoding
        if ($jobHandle -ne [IntPtr]::Zero) {
            try {
                Stop-Task038Job -Job $jobHandle
                $jobProcessCountZero = $true
            }
            catch {
                $cleanupFailure = Get-Task038FailureClassification -ErrorRecord $_
            }
            try {
                Close-Task038Job -Job $jobHandle
                $jobHandle = [IntPtr]::Zero
            }
            catch {
                if ($null -eq $cleanupFailure) {
                    $cleanupFailure = 'TASK038_LATTICED_JOB_CLEANUP_REJECTED'
                }
            }
        }
        if ($started) {
            try {
                if (-not $process.HasExited) {
                    Stop-Task038ProcessTree -Process $process
                }
                elseif (-not $process.WaitForExit(5000)) {
                    throw 'TASK038_LATTICED_PROCESS_CLEANUP_REJECTED'
                }
                if ($null -ne $stdoutTask) {
                    if (-not $stdoutTask.Wait(5000)) {
                        throw 'TASK038_LATTICED_PROCESS_CLEANUP_REJECTED'
                    }
                    $stdout = [string]$stdoutTask.Result
                }
                if ($null -ne $stderrTask) {
                    if (-not $stderrTask.Wait(5000)) {
                        throw 'TASK038_LATTICED_PROCESS_CLEANUP_REJECTED'
                    }
                    $stderr = [string]$stderrTask.Result
                }
                if ($null -eq $exitCode -and $process.HasExited) {
                    $exitCode = [int]$process.ExitCode
                }
                if (Test-Path -LiteralPath $latticedPidPath -PathType Leaf) {
                    Assert-RegularFile `
                        -Path $latticedPidPath `
                        -FailureCode 'TASK038_LATTICED_PID_EVIDENCE_REJECTED'
                    $pidText = (Read-Task038StrictUtf8Text `
                        -Path $latticedPidPath `
                        -FailureCode 'TASK038_LATTICED_PID_EVIDENCE_REJECTED').Trim()
                    if (-not [int]::TryParse($pidText, [ref]$childProcessId) -or $childProcessId -le 0) {
                        throw 'TASK038_LATTICED_PID_EVIDENCE_REJECTED'
                    }
                }
            }
            catch {
                if ($null -eq $cleanupFailure) {
                    $cleanupFailure = 'TASK038_LATTICED_PROCESS_CLEANUP_REJECTED'
                }
            }
        }
        try {
            if (Test-Path -LiteralPath $latticedPidPath) {
                [IO.File]::Delete($latticedPidPath)
            }
            if (Test-Path -LiteralPath $latticedPidPath) {
                throw 'TASK038_LATTICED_PID_EVIDENCE_CLEANUP_REJECTED'
            }
        }
        catch {
            if ($null -eq $cleanupFailure) {
                $cleanupFailure = 'TASK038_LATTICED_PID_EVIDENCE_CLEANUP_REJECTED'
            }
        }
        if ($null -ne $suspendedProcess) {
            try { $suspendedProcess.Dispose() } catch {
                if ($null -eq $cleanupFailure) {
                    $cleanupFailure = 'TASK038_LATTICED_PROCESS_CLEANUP_REJECTED'
                }
            }
        }
    }
    if ($null -ne $cleanupFailure) {
        if ($null -ne $primaryFailure) {
            throw ($cleanupFailure + '|' + $primaryFailure)
        }
        throw $cleanupFailure
    }
    if ($null -ne $primaryFailure) {
        throw $primaryFailure
    }
    if (
        $childProcessId -le 0 -or
        -not $jobAssigned -or
        -not $resumedAfterJobAssignment -or
        -not $jobProcessCountZero
    ) {
        throw 'TASK038_LATTICED_PID_EVIDENCE_REJECTED'
    }
    Assert-SecretFreeText -Text ($stdout + "`n" + $stderr) -FailureCode 'TASK038_LATTICED_OUTPUT_SECRET_REJECTED'
    if ($stdout.Length -gt 1048576 -or $stderr.Length -gt 65536) {
        throw 'TASK038_LATTICED_OUTPUT_SIZE_REJECTED'
    }
    if ($exitCode -ne 0 -or -not [string]::IsNullOrWhiteSpace($stderr)) {
        $code = [regex]::Match($stderr, '(?<![A-Z0-9_])LATTICE[A-Z0-9_]{1,95}(?![A-Z0-9_])').Value
        if ([string]::IsNullOrEmpty($code)) { $code = 'NO_ALLOWLISTED_CODE' }
        throw ('TASK038_LATTICED_SESSION_REJECTED|' + $code + '|' + (Get-StringSha256 -Value $stderr))
    }
    $acceptanceEvidence = Read-Task038McpAcceptanceEvidence `
        -Path $AcceptanceEvidencePath `
        -ExpectedNativeIdentity $AcceptanceEvidenceNativeIdentity `
        -SessionId $AcceptanceSessionId `
        -SafeConfigSha256 $AcceptanceSafeConfigSha256 `
        -ProcessId $childProcessId `
        -ExpectedDispatchCount $ExpectedDispatchCount
    Write-McpResponseSummary -Path $OutputPath -ResponseText $stdout
    Write-JsonEvidence -Path $MetaPath -Value ([ordered]@{
        schema_version = 'lattice.task038.local-mcp-process.v1'
        run_mode = $RunMode
        process_id = $childProcessId
        controller_process_id = $controllerProcessId
        create_suspended = $true
        job_assigned_before_resume = $true
        resumed_after_job_assignment = $true
        job_active_processes_after_cleanup = 0
        started_at_utc = $startedAt.ToString('o')
        exited_at_utc = [DateTime]::UtcNow.ToString('o')
        exit_code = $exitCode
        response_sha256 = Get-StringSha256 -Value $stdout
        stderr_sha256 = Get-StringSha256 -Value $stderr
        hermes_or_openclaw_environment_supplied = $false
        acceptance_dispatch_evidence_schema = [string]$acceptanceEvidence.schema
        acceptance_session_id = [string]$acceptanceEvidence.session_id
        acceptance_safe_config_sha256 = [string]$acceptanceEvidence.safe_config_sha256
        acceptance_evidence_raw_sha256 = [string]$acceptanceEvidence.raw_sha256
        acceptance_evidence_byte_count = [long]$acceptanceEvidence.byte_count
        acceptance_dispatch_accepted_count = [int]$acceptanceEvidence.dispatch_accepted_count
        acceptance_final_event_sha256 = [string]$acceptanceEvidence.final_event_sha256
        acceptance_normal_close_complete = [bool]$acceptanceEvidence.normal_close_complete
        acceptance_evidence_native_identity = [string]$acceptanceEvidence.native_identity
    })
    return [pscustomobject]@{
        ProcessId = $childProcessId
        Output = $stdout
        AcceptanceEvidence = $acceptanceEvidence
    }
}

function Get-McpResponses {
    param([Parameter(Mandatory = $true)][string]$Output)

    $responses = [Collections.Generic.List[object]]::new()
    foreach ($line in @($Output -split '\r?\n')) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        try {
            $responses.Add(($line | ConvertFrom-Json))
        }
        catch {
            throw 'TASK038_MCP_RESPONSE_JSON_REJECTED'
        }
    }
    return @($responses)
}

function Get-McpResponse {
    param(
        [Parameter(Mandatory = $true)][object[]]$Responses,
        [Parameter(Mandatory = $true)][int]$Id
    )

    $matches = @($Responses | Where-Object { [int]$_.id -eq $Id })
    if (
        $matches.Count -ne 1 -or
        $null -ne $matches[0].PSObject.Properties['error'] -or
        @($matches[0].PSObject.Properties.Name | Sort-Object) -join ',' -cne 'id,jsonrpc,result' -or
        -not ($matches[0].jsonrpc -is [string]) -or
        [string]$matches[0].jsonrpc -cne '2.0'
    ) {
        throw 'TASK038_MCP_RESPONSE_SHAPE_REJECTED'
    }
    return $matches[0]
}

function Assert-Task038ServerMeta {
    param(
        [Parameter(Mandatory = $true)]$Meta,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if ($null -eq $Meta) {
        throw $FailureCode
    }
    $serverInfoProperty = $Meta.PSObject.Properties['io.modelcontextprotocol/serverInfo']
    if (
        @($Meta.PSObject.Properties).Count -ne 1 -or
        $null -eq $serverInfoProperty -or
        @($serverInfoProperty.Value.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'name,title,version' -or
        -not ($serverInfoProperty.Value.name -is [string]) -or
        [string]$serverInfoProperty.Value.name -cne 'latticed' -or
        -not ($serverInfoProperty.Value.title -is [string]) -or
        [string]$serverInfoProperty.Value.title -cne 'LATTICE DevOS' -or
        -not ($serverInfoProperty.Value.version -is [string]) -or
        [string]$serverInfoProperty.Value.version -cne '1.0.0'
    ) {
        throw $FailureCode
    }
}

function Assert-LegacyInitializeResponse {
    param([Parameter(Mandatory = $true)]$Response)

    $result = $Response.result
    if ($null -eq $result) {
        throw 'TASK038_MCP_INITIALIZE_RESPONSE_REJECTED'
    }
    if (
        @($result.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'capabilities,instructions,protocolVersion,serverInfo' -or
        -not ($result.protocolVersion -is [string]) -or
        [string]$result.protocolVersion -cne '2025-11-25' -or
        -not ($result.instructions -is [string]) -or
        [string]$result.instructions -cne 'Four bounded LATTICE tools. Authority, task binding, orchestration, and execution configuration remain server-owned.' -or
        @($result.capabilities.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'tools' -or
        @($result.capabilities.tools.PSObject.Properties).Count -ne 0 -or
        @($result.serverInfo.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'name,title,version' -or
        -not ($result.serverInfo.name -is [string]) -or
        [string]$result.serverInfo.name -cne 'latticed' -or
        -not ($result.serverInfo.title -is [string]) -or
        [string]$result.serverInfo.title -cne 'LATTICE DevOS' -or
        -not ($result.serverInfo.version -is [string]) -or
        [string]$result.serverInfo.version -cne '1.0.0'
    ) {
        throw 'TASK038_MCP_INITIALIZE_RESPONSE_REJECTED'
    }
}

function Assert-StatelessDiscoverResponse {
    param([Parameter(Mandatory = $true)]$Response)

    $result = $Response.result
    if ($null -eq $result) {
        throw 'TASK038_MCP_DISCOVER_RESPONSE_REJECTED'
    }
    if (
        @($result.PSObject.Properties.Name | Sort-Object) -join ',' -cne '_meta,cacheScope,capabilities,instructions,resultType,supportedVersions,ttlMs' -or
        -not ($result.resultType -is [string]) -or
        [string]$result.resultType -cne 'complete' -or
        -not ($result.cacheScope -is [string]) -or
        [string]$result.cacheScope -cne 'private' -or
        -not ($result.ttlMs -is [int]) -or
        [int]$result.ttlMs -ne 0 -or
        @($result.supportedVersions).Count -ne 1 -or
        -not ($result.supportedVersions[0] -is [string]) -or
        [string]$result.supportedVersions[0] -cne '2026-07-28' -or
        -not ($result.instructions -is [string]) -or
        [string]$result.instructions -cne 'Four bounded LATTICE tools. Authority, task binding, orchestration, and execution configuration remain server-owned.' -or
        @($result.capabilities.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'tools' -or
        @($result.capabilities.tools.PSObject.Properties).Count -ne 0
    ) {
        throw 'TASK038_MCP_DISCOVER_RESPONSE_REJECTED'
    }
    Assert-Task038ServerMeta -Meta $result._meta -FailureCode 'TASK038_MCP_DISCOVER_RESPONSE_REJECTED'
}

function Assert-ToolResultEnvelope {
    param(
        [Parameter(Mandatory = $true)]$Response,
        [Parameter(Mandatory = $true)][ValidateSet('TASK_STATUS', 'TASK_ERROR')][string]$ExpectedKind,
        [ValidateSet('LEGACY', 'STATELESS')][string]$Protocol = 'LEGACY'
    )

    $resultNames = @($Response.result.PSObject.Properties.Name | Sort-Object) -join ','
    $expectedResultNames = if ($Protocol -eq 'STATELESS') {
        '_meta,content,isError,resultType,structuredContent'
    }
    else {
        'content,isError,structuredContent'
    }
    if (
        @($Response.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'id,jsonrpc,result' -or
        -not ($Response.jsonrpc -is [string]) -or
        [string]$Response.jsonrpc -cne '2.0' -or
        $null -eq $Response.result -or
        $resultNames -cne $expectedResultNames -or
        -not ($Response.result.isError -is [bool])
    ) {
        throw 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    if ($Protocol -eq 'STATELESS') {
        $serverInfoProperty = $Response.result._meta.PSObject.Properties['io.modelcontextprotocol/serverInfo']
        if (
            -not ($Response.result.resultType -is [string]) -or
            [string]$Response.result.resultType -cne 'complete' -or
            @($Response.result._meta.PSObject.Properties).Count -ne 1 -or
            $null -eq $serverInfoProperty -or
            @($serverInfoProperty.Value.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'name,title,version' -or
            [string]$serverInfoProperty.Value.name -cne 'latticed' -or
            [string]$serverInfoProperty.Value.title -cne 'LATTICE DevOS' -or
            [string]$serverInfoProperty.Value.version -cne '1.0.0'
        ) {
            throw 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED'
        }
    }
    $content = @($Response.result.content)
    if (
        $content.Count -ne 1 -or
        @($content[0].PSObject.Properties.Name | Sort-Object) -join ',' -cne 'text,type' -or
        -not ($content[0].type -is [string]) -or
        [string]$content[0].type -cne 'text' -or
        -not ($content[0].text -is [string]) -or
        $null -eq $Response.result.structuredContent
    ) {
        throw 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    try {
        $textContent = [string]$content[0].text | ConvertFrom-Json
    }
    catch {
        throw 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    if ($ExpectedKind -eq 'TASK_STATUS') {
        if ([bool]$Response.result.isError) {
            throw 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED'
        }
        Assert-PublicTaskStatusShape -Value $Response.result.structuredContent
        Assert-PublicTaskStatusShape -Value $textContent
        Assert-SamePublicTaskStatus -Expected $Response.result.structuredContent -Actual $textContent
    }
    else {
        if (-not [bool]$Response.result.isError) {
            throw 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED'
        }
        foreach ($value in @($Response.result.structuredContent, $textContent)) {
            if (
                @($value.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'code,status' -or
                -not ($value.status -is [string]) -or
                [string]$value.status -cne 'ERROR' -or
                -not ($value.code -is [string]) -or
                [string]$value.code -notmatch '^LATTICE_[A-Z0-9_]{1,95}$'
            ) {
                throw 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED'
            }
        }
        if (
            [string]$Response.result.structuredContent.status -cne [string]$textContent.status -or
            [string]$Response.result.structuredContent.code -cne [string]$textContent.code
        ) {
            throw 'TASK038_TOOL_RESULT_ENVELOPE_REJECTED'
        }
    }
}

function Get-ToolStructuredContent {
    param(
        [Parameter(Mandatory = $true)]$Response,
        [Parameter(Mandatory = $true)][ValidateSet('TASK_STATUS', 'TASK_ERROR')][string]$ExpectedKind,
        [ValidateSet('LEGACY', 'STATELESS')][string]$Protocol = 'LEGACY'
    )

    Assert-ToolResultEnvelope -Response $Response -ExpectedKind $ExpectedKind -Protocol $Protocol
    return $Response.result.structuredContent
}

function Assert-ToolDiscovery {
    param([Parameter(Mandatory = $true)]$Response)

    $resultNames = @($Response.result.PSObject.Properties.Name | Sort-Object) -join ','
    $stateless = $null -ne $Response.result.PSObject.Properties['resultType']
    if (
        (-not $stateless -and $resultNames -cne 'tools') -or
        ($stateless -and $resultNames -cne '_meta,cacheScope,resultType,tools,ttlMs')
    ) {
        throw 'TASK038_TOOL_DISCOVERY_ENVELOPE_REJECTED'
    }
    if ($stateless -and (
        [string]$Response.result.resultType -cne 'complete' -or
        [string]$Response.result.cacheScope -cne 'private' -or
        [int]$Response.result.ttlMs -ne 0
    )) {
        throw 'TASK038_TOOL_DISCOVERY_ENVELOPE_REJECTED'
    }
    if ($stateless) {
        Assert-Task038ServerMeta -Meta $Response.result._meta -FailureCode 'TASK038_TOOL_DISCOVERY_ENVELOPE_REJECTED'
    }
    $tools = @($Response.result.tools)
    $names = @($tools | ForEach-Object { [string]$_.name } | Sort-Object)
    $expected = @('lattice_delivery_run', 'lattice_delivery_status', 'lattice_task_status', 'lattice_task_submit')
    if ($tools.Count -ne 4 -or @(Compare-Object -ReferenceObject $expected -DifferenceObject $names).Count -ne 0) {
        throw 'TASK038_TOOL_DISCOVERY_REJECTED'
    }
    foreach ($name in @('lattice_delivery_run', 'lattice_delivery_status')) {
        $tool = @($tools | Where-Object { [string]$_.name -eq $name })[0]
        $propertiesMember = $tool.inputSchema.PSObject.Properties['properties']
        $propertyCount = if ($null -eq $propertiesMember) {
            0
        }
        else {
            @($propertiesMember.Value.PSObject.Properties).Count
        }
        if (
            [bool]$tool.inputSchema.additionalProperties -ne $false -or
            $propertyCount -ne 0 -or
            $null -ne $tool.PSObject.Properties['outputSchema']
        ) {
            throw 'TASK038_DELIVERY_SCHEMA_REJECTED'
        }
    }
    $submit = @($tools | Where-Object { [string]$_.name -eq 'lattice_task_submit' })[0].inputSchema
    $status = @($tools | Where-Object { [string]$_.name -eq 'lattice_task_status' })[0].inputSchema
    $submitProperties = @($submit.properties.PSObject.Properties.Name | Sort-Object)
    $submitRequired = @($submit.required | Sort-Object)
    $statusProperties = @($status.properties.PSObject.Properties.Name | Sort-Object)
    $statusRequired = @($status.required | Sort-Object)
    if (
        [bool]$submit.additionalProperties -ne $false -or
        @(Compare-Object @('client_request_id', 'intent') $submitProperties).Count -ne 0 -or
        @(Compare-Object @('client_request_id', 'intent') $submitRequired).Count -ne 0 -or
        [bool]$status.additionalProperties -ne $false -or
        @(Compare-Object @('task_ref') $statusProperties).Count -ne 0 -or
        @(Compare-Object @('task_ref') $statusRequired).Count -ne 0
    ) {
        throw 'TASK038_TASK_SCHEMA_REJECTED'
    }
    $expectedOutputProperties = @(
        'ledger_head_digest', 'result_digest', 'schema_version', 'status', 'task_ref', 'task_state'
    )
    $expectedStatuses = @('COMPLETED', 'FAILED', 'NOT_SUBMITTED', 'RECONCILIATION_REQUIRED')
    $expectedStates = @(
        'AWAITING_EXECUTION_APPROVAL', 'AWAITING_MERGE_APPROVAL', 'BLOCKED', 'CANCELLED',
        'COMPLETED', 'DRAFT', 'EXECUTING', 'FAILED', 'MERGING', 'NOT_SUBMITTED', 'PREPARING',
        'REJECTED', 'REVIEWING', 'STOPPING', 'VERIFYING'
    )
    foreach ($toolName in @('lattice_task_submit', 'lattice_task_status')) {
        $output = @($tools | Where-Object { [string]$_.name -eq $toolName })[0].outputSchema
        $outputProperties = @($output.properties.PSObject.Properties.Name | Sort-Object)
        $outputRequired = @($output.required | Sort-Object)
        if (
            [string]$output.type -cne 'object' -or
            [bool]$output.additionalProperties -ne $false -or
            @(Compare-Object $expectedOutputProperties $outputProperties).Count -ne 0 -or
            @(Compare-Object $expectedOutputProperties $outputRequired).Count -ne 0 -or
            @(Compare-Object $expectedStatuses @($output.properties.status.enum | Sort-Object)).Count -ne 0 -or
            @(Compare-Object $expectedStates @($output.properties.task_state.enum | Sort-Object)).Count -ne 0 -or
            [string]$output.properties.task_ref.pattern -cne '^[0-9a-f]{64}$' -or
            [string]$output.properties.ledger_head_digest.pattern -cne '^[0-9a-f]{64}$' -or
            @($output.properties.result_digest.anyOf).Count -ne 2
        ) {
            throw 'TASK038_TASK_OUTPUT_SCHEMA_REJECTED'
        }
    }
}

function Assert-PublicTaskStatusShape {
    param([Parameter(Mandatory = $true)]$Value)

    $names = @($Value.PSObject.Properties.Name | Sort-Object)
    $expected = @('ledger_head_digest', 'result_digest', 'schema_version', 'status', 'task_ref', 'task_state')
    $terminalFailureStates = @('BLOCKED', 'CANCELLED', 'FAILED', 'REJECTED')
    $reconciliationStates = @(
        'AWAITING_EXECUTION_APPROVAL', 'AWAITING_MERGE_APPROVAL', 'DRAFT', 'EXECUTING',
        'MERGING', 'PREPARING', 'REVIEWING', 'STOPPING', 'VERIFYING'
    )
    $resultDigestValid = (
        $null -eq $Value.result_digest -or
        (($Value.result_digest -is [string]) -and [string]$Value.result_digest -match '^[0-9a-f]{64}$')
    )
    $statusMappingValid = switch ([string]$Value.status) {
        'COMPLETED' {
            [string]$Value.task_state -ceq 'COMPLETED' -and
            ($Value.result_digest -is [string])
            break
        }
        'FAILED' {
            $terminalFailureStates -ccontains [string]$Value.task_state
            break
        }
        'RECONCILIATION_REQUIRED' {
            $reconciliationStates -ccontains [string]$Value.task_state
            break
        }
        'NOT_SUBMITTED' {
            [string]$Value.task_state -ceq 'NOT_SUBMITTED' -and $null -eq $Value.result_digest
            break
        }
        default { $false }
    }
    if (
        @(Compare-Object -ReferenceObject $expected -DifferenceObject $names).Count -ne 0 -or
        -not ($Value.schema_version -is [string]) -or
        [string]$Value.schema_version -cne 'lattice.task.status.v1' -or
        -not ($Value.status -is [string]) -or
        -not ($Value.task_state -is [string]) -or
        -not ($Value.task_ref -is [string]) -or
        [string]$Value.task_ref -notmatch '^[0-9a-f]{64}$' -or
        -not ($Value.ledger_head_digest -is [string]) -or
        [string]$Value.ledger_head_digest -notmatch '^[0-9a-f]{64}$' -or
        -not $resultDigestValid -or
        -not $statusMappingValid
    ) {
        throw 'TASK038_PUBLIC_STATUS_SHAPE_REJECTED'
    }
}

function Assert-CompletedTaskStatus {
    param([Parameter(Mandatory = $true)]$Value)

    Assert-PublicTaskStatusShape -Value $Value
    if (
        [string]$Value.status -cne 'COMPLETED' -or
        [string]$Value.task_state -cne 'COMPLETED' -or
        -not ($Value.result_digest -is [string]) -or
        [string]$Value.result_digest -notmatch '^[0-9a-f]{64}$'
    ) {
        throw 'TASK038_TASK_NOT_COMPLETED'
    }
}

function Assert-SamePublicTaskStatus {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual
    )

    foreach ($name in @('ledger_head_digest', 'result_digest', 'schema_version', 'status', 'task_ref', 'task_state')) {
        if ([string]$Expected.$name -ne [string]$Actual.$name) {
            throw 'TASK038_PUBLIC_STATUS_REPLAY_REJECTED'
        }
    }
}

function Assert-CompletedDatabaseFootprint {
    param(
        [Parameter(Mandatory = $true)]$Footprint,
        [Parameter(Mandatory = $true)]$PublicStatus,
        [Parameter(Mandatory = $true)][string]$ExpectedCommandId,
        [Parameter(Mandatory = $true)]$Baseline
    )

    if (
        $Footprint.sequence -ne 12 -or $Footprint.event_count -ne 12 -or $Footprint.command_count -ne 12 -or
        $Footprint.task_ref -ne [string]$PublicStatus.task_ref -or
        $Footprint.ledger_head_digest -ne [string]$PublicStatus.ledger_head_digest -or
        $Footprint.result_digest -ne [string]$PublicStatus.result_digest -or
        $Footprint.task_created -ne 1 -or $Footprint.state_transitions -ne 8 -or
        $Footprint.codex_intents -ne 1 -or $Footprint.verified_outcomes -ne 1 -or $Footprint.task_results -ne 1 -or
        $Footprint.created_command_id -ne $ExpectedCommandId -or
        $Footprint.created_actor_id -ne 'local-canonical-mcp-acceptance-profile' -or
        $Footprint.created_action_id -ne 'CONTROLLED_CODEX_CANARY' -or
        $Footprint.created_reason_code -ne 'TASK038_TASK_ACCEPTED' -or
        $Footprint.created_audit_schema -ne 'lattice.task-created-ingress-audit.v1' -or
        $Footprint.created_client_kind -ne 'LOCAL_CANONICAL_MCP_ACCEPTANCE' -or
        $Footprint.created_actor_kind -ne 'LOCAL_ACCEPTANCE_HARNESS' -or
        $Footprint.created_adapter_id -ne 'lattice-local-canonical-mcp-acceptance' -or
        $Footprint.created_profile_adapter_commitment -notmatch '^[0-9a-f]{64}$' -or
        $Footprint.created_process_start_authority_digest -notmatch '^[0-9a-f]{64}$' -or
        $Footprint.created_admission_observation_commitment -notmatch '^[0-9a-f]{64}$' -or
        $Footprint.writer_fencing_high_water -ne 1 -or $Footprint.writer_command_count -ne 2 -or
        $Footprint.writer_transition_count -ne 2 -or -not [string]::IsNullOrEmpty($Footprint.current_writer_status) -or
        $Footprint.memory_analyses -ne $Baseline.memory_analyses -or
        $Footprint.memory_receipts -ne $Baseline.memory_receipts -or
        $Footprint.memory_retrieval_audits -ne $Baseline.memory_retrieval_audits -or
        $Footprint.memory_records -ne $Baseline.memory_records -or
        $Footprint.memory_reflections -ne $Baseline.memory_reflections -or
        $Footprint.openclaw_commands -ne $Baseline.openclaw_commands
    ) {
        throw 'TASK038_COMPLETED_DATABASE_FOOTPRINT_REJECTED'
    }
}

$script:RepositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
$repositoryTarget = Get-CanonicalPath -Path (Join-Path $script:RepositoryRoot 'target')
if (-not (Test-Path -LiteralPath $repositoryTarget -PathType Container)) {
    New-Item -ItemType Directory -Path $repositoryTarget -Force:$false | Out-Null
}
Assert-NoReparseAncestor -Path $repositoryTarget -Boundary $script:RepositoryRoot -FailureCode 'TASK038_TARGET_REJECTED'
$task038CargoTarget = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'task038-main')
if (-not (Test-Path -LiteralPath $task038CargoTarget -PathType Container)) {
    New-Item -ItemType Directory -Path $task038CargoTarget -Force:$false | Out-Null
}
Assert-NoReparseAncestor -Path $task038CargoTarget -Boundary $script:RepositoryRoot -FailureCode 'TASK038_CARGO_TARGET_REJECTED'
foreach ($cargoVariable in @('CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET')) {
    if (-not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($cargoVariable, 'Process'))) {
        throw 'TASK038_AMBIENT_CARGO_TARGET_REJECTED'
    }
}

$reservedPostgresPorts = [Collections.Generic.HashSet[int]]::new()
foreach ($reservedPort in @(5432, 64272, 55432)) {
    [void]$reservedPostgresPorts.Add($reservedPort)
}
if ($reservedPostgresPorts.Contains($PostgresPort)) {
    throw 'TASK038_LOCAL_POSTGRES_RESERVED_PORT_REJECTED'
}
$databaseName = Get-Task019ProductionDatabaseName -RunId $PostgresRunId
$databasePassword = Get-RequiredSecretEnvironment -Name $DatabaseSecretVariable -MinimumLength 16 -FailureCode 'TASK038_DATABASE_SECRET_REQUIRED'
$migratorDsn = Get-RequiredSecretEnvironment -Name $MigratorDsnVariable -MinimumLength 24 -FailureCode 'TASK038_MIGRATOR_DSN_REQUIRED'
$runtimeDsn = Get-RequiredSecretEnvironment -Name $RuntimeDsnVariable -MinimumLength 24 -FailureCode 'TASK038_RUNTIME_DSN_REQUIRED'
$adminDsn = Get-RequiredSecretEnvironment -Name $AdminDsnVariable -MinimumLength 24 -FailureCode 'TASK038_ADMIN_DSN_REQUIRED'
Assert-LocalPostgresDsn -Value $migratorDsn -ExpectedDatabase $databaseName -FailureCode 'TASK038_MIGRATOR_DSN_REJECTED'
Assert-LocalPostgresDsn -Value $runtimeDsn -ExpectedDatabase $databaseName -FailureCode 'TASK038_RUNTIME_DSN_REJECTED'
Assert-LocalPostgresDsn -Value $adminDsn -ExpectedDatabase $databaseName -AllowMaintenanceDatabase -FailureCode 'TASK038_ADMIN_DSN_REJECTED'

$script:Psql = Get-CanonicalPath -Path $PsqlExecutable
if (-not (Test-ExactPath -Actual $script:Psql -Expected (Join-Path $expectedPostgresBin 'psql.exe'))) {
    throw 'TASK038_PSQL_IDENTITY_REJECTED'
}
Assert-RegularFile -Path $script:Psql -FailureCode 'TASK038_PSQL_REJECTED'
$script:PsqlNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $script:Psql -Directory $false
if ((Get-FileSha256 -Path $script:Psql) -cne $expectedPsqlExecutableSha256) {
    throw 'TASK038_PSQL_IDENTITY_REJECTED'
}
$psqlVersion = Invoke-NativeText -Executable $script:Psql -Arguments @('--version')
if ($psqlVersion.ExitCode -ne 0 -or $psqlVersion.Text -notmatch '^psql \(PostgreSQL\) 17\.10(?:\s|$)') {
    throw 'TASK038_POSTGRES_VERSION_REJECTED'
}
$script:PgCtl = Get-CanonicalPath -Path (Join-Path (Split-Path -Parent $script:Psql) 'pg_ctl.exe')
Assert-RegularFile -Path $script:PgCtl -FailureCode 'TASK038_PG_CTL_REJECTED'
$script:PgCtlNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $script:PgCtl -Directory $false
if ((Get-FileSha256 -Path $script:PgCtl) -cne $expectedPgCtlExecutableSha256) {
    throw 'TASK038_PG_CTL_IDENTITY_REJECTED'
}
$pgCtlVersion = Invoke-NativeText -Executable $script:PgCtl -Arguments @('--version')
if ($pgCtlVersion.ExitCode -ne 0 -or $pgCtlVersion.Text -notmatch '^pg_ctl \(PostgreSQL\) 17\.10(?:\s|$)') {
    throw 'TASK038_PG_CTL_VERSION_REJECTED'
}
$script:PostgresExecutable = Get-CanonicalPath -Path (Join-Path (Split-Path -Parent $script:Psql) 'postgres.exe')
Assert-RegularFile -Path $script:PostgresExecutable -FailureCode 'TASK038_POSTGRES_EXECUTABLE_REJECTED'
$script:PostgresExecutableNativeIdentity = Get-LatticeWindowsNativePathIdentityToken `
    -Path $script:PostgresExecutable `
    -Directory $false
if ((Get-FileSha256 -Path $script:PostgresExecutable) -cne $expectedPostgresExecutableSha256) {
    throw 'TASK038_POSTGRES_EXECUTABLE_IDENTITY_REJECTED'
}
$postgresVersion = Invoke-NativeText -Executable $script:PostgresExecutable -Arguments @('--version')
if ($postgresVersion.ExitCode -ne 0 -or $postgresVersion.Text -notmatch '^postgres \(PostgreSQL\) 17\.10(?:\s|$)') {
    throw 'TASK038_POSTGRES_EXECUTABLE_VERSION_REJECTED'
}

$script:PostgresData = Get-CanonicalPath -Path $PostgresDataDirectory
$dataItem = Get-Item -LiteralPath $script:PostgresData -Force -ErrorAction SilentlyContinue
if ($null -eq $dataItem -or -not $dataItem.PSIsContainer -or ($dataItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'TASK038_POSTGRES_DATA_REJECTED'
}
Assert-NoReparseAncestor -Path $script:PostgresData -Boundary $script:RepositoryRoot -FailureCode 'TASK038_POSTGRES_DATA_REJECTED'
$clusterRoot = Get-CanonicalPath -Path (Split-Path -Parent $script:PostgresData)
$clusterMarkerPath = Join-Path $clusterRoot '.lattice-task019-disposable.json'
Assert-RegularFile -Path $clusterMarkerPath -FailureCode 'TASK038_POSTGRES_MARKER_REJECTED'
try {
    $clusterMarkerRawSha256 = Get-FileSha256 -Path $clusterMarkerPath
    $clusterMarker = Read-Task038StrictUtf8Text `
        -Path $clusterMarkerPath `
        -FailureCode 'TASK038_POSTGRES_MARKER_REJECTED' |
        ConvertFrom-Json
}
catch {
    throw 'TASK038_POSTGRES_MARKER_REJECTED'
}
if (
    [string]$clusterMarker.kind -cne 'LATTICE_TASK019_DISPOSABLE_POSTGRES_V1' -or
    [string]$clusterMarker.run_id -cne $PostgresRunId -or
    [string]$clusterMarker.postgres_version -cne '17.10' -or
    [string]$clusterMarker.host -cne '127.0.0.1' -or
    [int]$clusterMarker.port -ne $PostgresPort -or
    -not [bool]$clusterMarker.identity_materialized -or
    -not [bool]$clusterMarker.restart_identity_verified -or
    [string]$clusterMarker.system_identifier -cnotmatch '\A[0-9]{1,20}\z' -or
    [string]::IsNullOrWhiteSpace([string]$clusterMarker.initial_postmaster_started_at) -or
    [string]::IsNullOrWhiteSpace([string]$clusterMarker.restart_postmaster_started_at) -or
    [string]$clusterMarker.initial_postmaster_started_at -ceq [string]$clusterMarker.restart_postmaster_started_at -or
    (@($clusterMarker.excluded_ports | ForEach-Object { [int]$_ }) -join ',') -cne '5432,64272,55432' -or
    -not (Test-ExactPath -Actual ([string]$clusterMarker.psql_executable_path) -Expected $script:Psql) -or
    -not (Test-ExactPath -Actual ([string]$clusterMarker.pg_ctl_executable_path) -Expected $script:PgCtl) -or
    -not (Test-ExactPath -Actual ([string]$clusterMarker.postgres_executable_path) -Expected $script:PostgresExecutable) -or
    [string]$clusterMarker.psql_executable_raw_sha256 -cne $expectedPsqlExecutableSha256 -or
    [string]$clusterMarker.pg_ctl_executable_raw_sha256 -cne $expectedPgCtlExecutableSha256 -or
    [string]$clusterMarker.postgres_executable_raw_sha256 -cne $expectedPostgresExecutableSha256 -or
    [string]$clusterMarker.psql_executable_native_identity -cne $script:PsqlNativeIdentity -or
    [string]$clusterMarker.pg_ctl_executable_native_identity -cne $script:PgCtlNativeIdentity -or
    [string]$clusterMarker.postgres_executable_native_identity -cne $script:PostgresExecutableNativeIdentity -or
    -not (Test-ExactPath -Actual ([string]$clusterMarker.root) -Expected $clusterRoot) -or
    -not (Test-ExactPath -Actual ([string]$clusterMarker.repository_target) -Expected $repositoryTarget)
) {
    throw 'TASK038_POSTGRES_MARKER_REJECTED'
}
$script:PostgresContainmentSnapshot = New-LatticeWindowsNativeContainmentSnapshot `
    -ParentPath $repositoryTarget `
    -RootPath $clusterRoot `
    -MarkerPath $clusterMarkerPath
$script:PostgresDataIdentity = Get-LatticeWindowsNativePathIdentityToken `
    -Path $script:PostgresData `
    -Directory $true
if ([string]$clusterMarker.data_native_identity -cne $script:PostgresDataIdentity) {
    throw 'TASK038_POSTGRES_MARKER_REJECTED'
}
Assert-Task038PostgresNativeIdentity -FailureCode 'TASK038_POSTGRES_NATIVE_IDENTITY_REJECTED'
$postgresServerLog = Get-CanonicalPath -Path (Join-Path $clusterRoot 'postgres.log')

$script:Cargo = Get-CanonicalPath -Path (@(Get-Command 'cargo.exe' -CommandType Application -ErrorAction Stop)[0].Source)
$script:Git = Get-CanonicalPath -Path (@(Get-Command 'git.exe' -CommandType Application -ErrorAction Stop)[0].Source)
$windowsDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
$script:PowerShell = Get-CanonicalPath -Path (Join-Path $windowsDirectory 'System32\WindowsPowerShell\v1.0\powershell.exe')
$currentPowerShell = Get-CanonicalPath -Path ([Diagnostics.Process]::GetCurrentProcess().MainModule.FileName)
Assert-RegularFile -Path $script:Cargo -FailureCode 'TASK038_CARGO_REJECTED'
Assert-RegularFile -Path $script:Git -FailureCode 'TASK038_GIT_REJECTED'
Assert-RegularFile -Path $script:PowerShell -FailureCode 'TASK038_POWERSHELL_REJECTED'
if (
    -not (Test-ExactPath -Actual $script:PowerShell -Expected $currentPowerShell) -or
    -not (Test-ExactPath -Actual $script:PowerShell -Expected (Join-Path $PSHOME 'powershell.exe'))
) {
    throw 'TASK038_POWERSHELL_IDENTITY_REJECTED'
}
$powerShellSignature = Get-AuthenticodeSignature -LiteralPath $script:PowerShell
if (
    [string]$powerShellSignature.Status -ne 'Valid' -or
    $null -eq $powerShellSignature.SignerCertificate -or
    [string]$powerShellSignature.SignerCertificate.Subject -notlike 'CN=Microsoft Windows,*O=Microsoft Corporation*'
) {
    throw 'TASK038_POWERSHELL_SIGNATURE_REJECTED'
}

$script:OfficialCodex = Get-CanonicalPath -Path $OfficialCodexExecutable
Assert-RegularFile -Path $script:OfficialCodex -FailureCode 'TASK038_OFFICIAL_CODEX_REJECTED'
Assert-NoReparsePath -Path $script:OfficialCodex -FailureCode 'TASK038_OFFICIAL_CODEX_REPARSE_REJECTED'
$codexVersion = Invoke-NativeText -Executable $script:OfficialCodex -Arguments @('--version')
if ($codexVersion.ExitCode -ne 0 -or $codexVersion.Text.Trim() -ne 'codex-cli 0.146.0') {
    throw 'TASK038_OFFICIAL_CODEX_VERSION_REJECTED'
}
$signature = Get-AuthenticodeSignature -LiteralPath $script:OfficialCodex
if (
    [string]$signature.Status -ne 'Valid' -or
    $null -eq $signature.SignerCertificate -or
    [string]$signature.SignerCertificate.Thumbprint -ne '0B7C30C11BF7250EC1ECD3254AC781D9E13D62F8' -or
    [string]$signature.SignerCertificate.Subject -notlike '*OpenAI OpCo, LLC*'
) {
    throw 'TASK038_OFFICIAL_CODEX_SIGNATURE_REJECTED'
}
$launcherSha256 = Get-FileSha256 -Path $script:OfficialCodex
if ($launcherSha256 -ne 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb') {
    throw 'TASK038_OFFICIAL_CODEX_DIGEST_REJECTED'
}

$script:CodexCredentialSource = Get-CanonicalPath -Path $CodexAuthHome
$credentialSourceItem = Get-Item -LiteralPath $script:CodexCredentialSource -Force -ErrorAction SilentlyContinue
if ($null -eq $credentialSourceItem -or -not $credentialSourceItem.PSIsContainer -or ($credentialSourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'TASK038_CODEX_CREDENTIAL_SOURCE_REJECTED'
}
if (Test-PathOverlap -Left $script:CodexCredentialSource -Right $script:RepositoryRoot) {
    throw 'TASK038_CODEX_CREDENTIAL_SOURCE_REPOSITORY_OVERLAP'
}
Assert-NoReparsePath -Path $script:CodexCredentialSource -FailureCode 'TASK038_CODEX_CREDENTIAL_SOURCE_REJECTED'
foreach ($ambient in @(
    $env:CODEX_HOME,
    $(if ($env:USERPROFILE) { Join-Path $env:USERPROFILE '.codex' }),
    $(if ($env:HOME) { Join-Path $env:HOME '.codex' })
)) {
    if (
        -not [string]::IsNullOrWhiteSpace($ambient) -and
        (Test-PathOverlap -Left $script:CodexCredentialSource -Right $ambient)
    ) {
        throw 'TASK038_AMBIENT_CODEX_CREDENTIAL_SOURCE_REJECTED'
    }
}
$sourceMarker = Join-Path $script:CodexCredentialSource '.lattice-codex-home-v1'
$sourceAuth = Join-Path $script:CodexCredentialSource 'auth.json'
Assert-RegularFile -Path $sourceMarker -FailureCode 'TASK038_CODEX_CREDENTIAL_SOURCE_REJECTED'
Assert-RegularFile -Path $sourceAuth -FailureCode 'TASK038_CODEX_CREDENTIAL_SOURCE_REJECTED'
$expectedMarker = [Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($sourceMarker)) -ne [Convert]::ToBase64String($expectedMarker)) {
    throw 'TASK038_CODEX_CREDENTIAL_SOURCE_REJECTED'
}
$credentialSourceBefore = Get-DirectoryFootprint -Root $script:CodexCredentialSource
$credentialSourceAuthSha256 = Get-FileSha256 -Path $sourceAuth
$acceptanceId = [Guid]::NewGuid().ToString('N')
$executionHome = $null
$executionHomeParent = $null
$finalPath = $null
$final = $null
$primaryFailure = $null
$homeCleanupFailure = $null
$sourceCheckFailure = $null

try {
$executionHome = New-FreshCodexExecutionHome `
    -CredentialSource $script:CodexCredentialSource `
    -RepositoryRoot $script:RepositoryRoot `
    -AcceptanceId $acceptanceId
$script:CodexHome = [string]$executionHome.Path
$executionHomeParent = [string]$executionHome.Parent
Assert-CredentialSourceUnchanged `
    -Root $script:CodexCredentialSource `
    -ExpectedFootprint $credentialSourceBefore `
    -ExpectedAuthSha256 $credentialSourceAuthSha256
$cargoHostTarget = 'x86_64-pc-windows-msvc'
$build = Invoke-NativeText -Executable $script:Cargo -WorkingDirectory $script:RepositoryRoot -Arguments @(
    'build', '-p', 'lattice-runtime', '--bin', 'latticed', '--locked',
    '--target-dir', $task038CargoTarget, '--target', $cargoHostTarget
)
Assert-SecretFreeText -Text $build.Text -FailureCode 'TASK038_BUILD_OUTPUT_SECRET_REJECTED'
if ($build.ExitCode -ne 0) {
    throw ('TASK038_LATTICED_BUILD_REJECTED|' + (Get-StringSha256 -Value $build.Text))
}
$script:Latticed = Get-CanonicalPath -Path (Join-Path $task038CargoTarget ($cargoHostTarget + '\debug\latticed.exe'))
Assert-RegularFile -Path $script:Latticed -FailureCode 'TASK038_LATTICED_REJECTED'

$script:IngressProfileDigest = Get-StringSha256 -Value 'lattice-task038-local-canonical-mcp-acceptance-profile-v1'
$fixtureParent = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'lattice-delivery')
if (-not (Test-Path -LiteralPath $fixtureParent -PathType Container)) {
    New-Item -ItemType Directory -Path $fixtureParent -Force:$false | Out-Null
}
Assert-NoReparseAncestor -Path $fixtureParent -Boundary $script:RepositoryRoot -FailureCode 'TASK038_FIXTURE_PARENT_REJECTED'
$fixtureRoot = Get-CanonicalPath -Path (Join-Path $fixtureParent $acceptanceId)
if (Test-Path -LiteralPath $fixtureRoot) {
    throw 'TASK038_FIXTURE_NOT_FRESH'
}
New-Item -ItemType Directory -Path $fixtureRoot -Force:$false | Out-Null
$evidenceRoot = Join-Path $fixtureRoot 'evidence'
New-Item -ItemType Directory -Path $evidenceRoot -Force:$false | Out-Null
$candidateSourceLinkage = Get-Task038CandidateSourceLinkage -Repository $script:RepositoryRoot
$candidateLatticedSha256 = Get-FileSha256 -Path $script:Latticed
$candidateSourceLinkage['canonical_latticed_sha256'] = $candidateLatticedSha256
$candidateSourceLinkage['cluster_marker_raw_sha256'] = $clusterMarkerRawSha256
$candidateSourceLinkage['postgres_system_identifier'] = [string]$clusterMarker.system_identifier
$candidateSourceLinkagePath = Join-Path $evidenceRoot 'candidate-source-linkage.json'
Write-JsonEvidence -Path $candidateSourceLinkagePath -Value $candidateSourceLinkage
$candidateSourceLinkageRawSha256 = Get-FileSha256 -Path $candidateSourceLinkagePath
$deliveryRoot = Join-Path $fixtureRoot 'delivery'
$schemaDirectory = Join-Path $fixtureRoot 'schema'

Assert-Task038PostgresNativeIdentity -FailureCode 'TASK038_POSTGRES_NATIVE_IDENTITY_REJECTED'
$postgresRuntimeBinding = Get-PostgresProcessEvidence -Password $databasePassword -DatabaseName $databaseName
if (
    [string]$postgresRuntimeBinding.system_identifier -cne [string]$clusterMarker.system_identifier -or
    [string]$postgresRuntimeBinding.postmaster_started_at -cne [string]$clusterMarker.restart_postmaster_started_at
) {
    throw 'TASK038_POSTGRES_RUNTIME_BINDING_REJECTED'
}
$identity = Get-DatabaseIdentity -Password $databasePassword -DatabaseName $databaseName
$authority = Enable-StoreAuthority -Password $databasePassword -DatabaseName $databaseName -AcceptanceId $acceptanceId
Invoke-WriterLeaseLiveSuite -Identity $identity -Authority $authority -DatabaseName $databaseName -MigratorDsn $migratorDsn -RuntimeDsn $runtimeDsn -AdminDsn $adminDsn -EvidencePath (Join-Path $evidenceRoot 'writer-lease-live.json')
# The live suite deliberately proves stale-daemon rejection by replacing the
# admission head. Re-establish this acceptance process as the current daemon
# before the controlled task obtains its own lease.
$authority = Enable-StoreAuthority -Password $databasePassword -DatabaseName $databaseName -AcceptanceId $acceptanceId
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'database-binding.json') -Value ([ordered]@{
    schema_version = 'lattice.task038.database-binding.v1'
    database_name = $databaseName
    postgres_host = $PostgresHost
    postgres_port = $PostgresPort
    postgres_run_id = $PostgresRunId
    cluster_marker_raw_sha256 = $clusterMarkerRawSha256
    cluster_parent_native_identity = [string]$script:PostgresContainmentSnapshot.parent_identity
    cluster_root_native_identity = [string]$script:PostgresContainmentSnapshot.root_identity
    cluster_marker_native_identity = [string]$script:PostgresContainmentSnapshot.marker_identity
    postgres_data_native_identity = [string]$script:PostgresDataIdentity
    postgres_runtime_binding = $postgresRuntimeBinding
    psql_executable_path = $script:Psql
    psql_executable_raw_sha256 = $expectedPsqlExecutableSha256
    psql_executable_native_identity = $script:PsqlNativeIdentity
    pg_ctl_executable_path = $script:PgCtl
    pg_ctl_executable_raw_sha256 = $expectedPgCtlExecutableSha256
    pg_ctl_executable_native_identity = $script:PgCtlNativeIdentity
    postgres_executable_path = $script:PostgresExecutable
    postgres_executable_raw_sha256 = $expectedPostgresExecutableSha256
    postgres_executable_native_identity = $script:PostgresExecutableNativeIdentity
    excluded_ports = @(5432, 64272, 55432)
    marker_restart_identity_verified = $true
    identity = $identity
    authority = $authority
    task_ingress_kind = 'LOCAL_CANONICAL_MCP_ACCEPTANCE'
    task_ingress_profile_sha256 = $script:IngressProfileDigest
})
$preMutation = Get-PreMutationDatabaseFootprint -Password $databasePassword -DatabaseName $databaseName
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'database-pre-mutation.json') -Value $preMutation

$before = Get-DatabaseFootprint -Password $databasePassword -DatabaseName $databaseName -TaskRef ''
if (
    $before.event_count -ne 0 -or $before.command_count -ne 0 -or
    $before.writer_command_count -ne 0 -or $before.writer_transition_count -ne 0 -or
    $before.memory_analyses -ne $preMutation.memory_analyses -or
    $before.memory_receipts -ne $preMutation.memory_receipts -or
    $before.memory_retrieval_audits -ne $preMutation.memory_retrieval_audits -or
    $before.memory_records -ne $preMutation.memory_records -or
    $before.memory_reflections -ne $preMutation.memory_reflections -or
    $before.openclaw_commands -ne $preMutation.openclaw_commands
) {
    throw 'TASK038_DATABASE_NOT_FRESH'
}
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'database-before.json') -Value $before

$sameClientRequestId = 'task038-' + $acceptanceId
$differentClientRequestId = $sameClientRequestId + '-different'
$legacyFrames = @(
    [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = [ordered]@{ protocolVersion = '2025-11-25'; capabilities = [ordered]@{}; clientInfo = [ordered]@{ name = 'task038-local-submit'; version = '1' } } },
    [ordered]@{ jsonrpc = '2.0'; method = 'notifications/initialized'; params = [ordered]@{} },
    [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/list'; params = [ordered]@{} },
    [ordered]@{ jsonrpc = '2.0'; id = 3; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = $sameClientRequestId; intent = 'CONTROLLED_CODEX_CANARY' } } },
    [ordered]@{ jsonrpc = '2.0'; id = 4; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = $sameClientRequestId; intent = 'CONTROLLED_CODEX_CANARY' } } },
    [ordered]@{ jsonrpc = '2.0'; id = 5; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = $differentClientRequestId; intent = 'CONTROLLED_CODEX_CANARY' } } }
)
$submitInput = New-McpInput -Frames $legacyFrames
$submitAcceptanceSessionId = [Guid]::NewGuid().ToString('N')
$submitAcceptanceSink = New-Task038McpAcceptanceEvidenceSink `
    -EvidenceRoot $evidenceRoot `
    -SessionId $submitAcceptanceSessionId
$submitAcceptanceSafeConfigSha256 = Get-StringSha256 -Value (@(
    'lattice.task038.mcp-acceptance-safe-config.v1',
    $acceptanceId,
    'FRESH',
    (Get-StringSha256 -Value $submitInput),
    $candidateSourceLinkageRawSha256,
    $candidateLatticedSha256,
    $clusterMarkerRawSha256,
    [string]$clusterMarker.system_identifier,
    [string]$authority.authority_revision
) -join "`n")
$submitSession = Invoke-LatticedSession `
    -InputText $submitInput `
    -RunMode 'FRESH' `
    -OutputPath (Join-Path $evidenceRoot 'submit.response-summary.json') `
    -MetaPath (Join-Path $evidenceRoot 'submit.process.json') `
    -Authority $authority `
    -DatabasePassword $databasePassword `
    -DeliveryRoot $deliveryRoot `
    -SchemaDirectory $schemaDirectory `
    -LauncherSha256 $launcherSha256 `
    -LauncherVersion $codexVersion.Text.Trim() `
    -AcceptanceEvidencePath ([string]$submitAcceptanceSink.path) `
    -AcceptanceEvidenceNativeIdentity ([string]$submitAcceptanceSink.native_identity) `
    -AcceptanceSessionId $submitAcceptanceSessionId `
    -AcceptanceSafeConfigSha256 $submitAcceptanceSafeConfigSha256 `
    -ExpectedDispatchCount 3
Assert-CredentialSourceUnchanged `
    -Root $script:CodexCredentialSource `
    -ExpectedFootprint $credentialSourceBefore `
    -ExpectedAuthSha256 $credentialSourceAuthSha256
$submitResponses = @(Get-McpResponses -Output $submitSession.Output)
if ($submitResponses.Count -ne 5) { throw 'TASK038_SUBMIT_RESPONSE_COUNT_REJECTED' }
Assert-LegacyInitializeResponse -Response (Get-McpResponse -Responses $submitResponses -Id 1)
Assert-ToolDiscovery -Response (Get-McpResponse -Responses $submitResponses -Id 2)
$submitResponse = Get-McpResponse -Responses $submitResponses -Id 3
$retryResponse = Get-McpResponse -Responses $submitResponses -Id 4
$differentResponse = Get-McpResponse -Responses $submitResponses -Id 5
if ([bool]$submitResponse.result.isError -or [bool]$retryResponse.result.isError -or -not [bool]$differentResponse.result.isError) {
    throw 'TASK038_IDEMPOTENCY_RESPONSE_REJECTED'
}
$submitted = Get-ToolStructuredContent -Response $submitResponse -ExpectedKind 'TASK_STATUS'
$retried = Get-ToolStructuredContent -Response $retryResponse -ExpectedKind 'TASK_STATUS'
$different = Get-ToolStructuredContent -Response $differentResponse -ExpectedKind 'TASK_ERROR'
Assert-PublicTaskStatusShape -Value $submitted
Assert-PublicTaskStatusShape -Value $retried
Assert-SamePublicTaskStatus -Expected $submitted -Actual $retried
if ([string]$different.code -ne 'LATTICE_TASK_REQUEST_SUBSTITUTED') {
    throw 'TASK038_DIFFERENT_KEY_DENIAL_REJECTED'
}
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'submit.json') -Value $submitted
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'same-key-retry.json') -Value $retried
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'different-key-denial.json') -Value ([ordered]@{ code = [string]$different.code; is_error = $true })
$databaseAfterSubmit = Get-DatabaseFootprint -Password $databasePassword -DatabaseName $databaseName -TaskRef ([string]$submitted.task_ref)
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'database-after-submit.json') -Value $databaseAfterSubmit
Assert-CompletedTaskStatus -Value $submitted
Assert-CompletedTaskStatus -Value $retried

$repository = Get-CanonicalPath -Path (Join-Path $deliveryRoot 'repo')
$gitAfterSubmit = Get-GitFootprint -Repository $repository
$codexAfterSubmit = Get-StableDirectoryFootprint -Root $script:CodexHome
Assert-CompletedDatabaseFootprint -Footprint $databaseAfterSubmit -PublicStatus $submitted -ExpectedCommandId ('mcp-submit:' + $sameClientRequestId) -Baseline $before
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'git-after-submit.json') -Value $gitAfterSubmit
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'codex-home-after-submit.json') -Value ([ordered]@{ codex_home_footprint = $codexAfterSubmit })

try {
    $postgresBeforeRestart = Get-PostgresProcessEvidence -Password $databasePassword -DatabaseName $databaseName
}
catch {
    $classification = Get-Task038FailureClassification -ErrorRecord $_
    if ($classification -eq 'TASK038_UNCLASSIFIED_REJECTED') {
        throw 'TASK038_POSTGRES_BEFORE_RESTART_EVIDENCE_REJECTED'
    }
    throw $classification
}
Assert-Task038PostgresNativeIdentity -FailureCode 'TASK038_POSTGRES_NATIVE_IDENTITY_REJECTED'
Restart-DisposablePostgres -DataDirectory $script:PostgresData -ServerLog $postgresServerLog
Assert-Task038PostgresNativeIdentity -FailureCode 'TASK038_POSTGRES_NATIVE_IDENTITY_REJECTED'
try {
    $postgresAfterRestart = Get-PostgresProcessEvidence -Password $databasePassword -DatabaseName $databaseName
}
catch {
    $classification = Get-Task038FailureClassification -ErrorRecord $_
    if ($classification -eq 'TASK038_UNCLASSIFIED_REJECTED') {
        throw 'TASK038_POSTGRES_AFTER_RESTART_EVIDENCE_REJECTED'
    }
    throw $classification
}
if (
    [string]$postgresBeforeRestart.system_identifier -ne [string]$postgresAfterRestart.system_identifier -or
    [string]$postgresBeforeRestart.postmaster_started_at -eq [string]$postgresAfterRestart.postmaster_started_at
) {
    throw 'TASK038_POSTGRES_RESTART_EVIDENCE_REJECTED'
}
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'postgres-restart.json') -Value ([ordered]@{
    schema_version = 'lattice.task038.postgres-restart.v1'
    status = 'PASS'
    system_identifier = [string]$postgresAfterRestart.system_identifier
    before_postmaster_started_at = [string]$postgresBeforeRestart.postmaster_started_at
    after_postmaster_started_at = [string]$postgresAfterRestart.postmaster_started_at
})

$modernMeta = [ordered]@{
    'io.modelcontextprotocol/protocolVersion' = '2026-07-28'
    'io.modelcontextprotocol/clientCapabilities' = [ordered]@{}
}
$statusFrames = @(
    [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'server/discover'; params = [ordered]@{ _meta = $modernMeta } },
    [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/list'; params = [ordered]@{ _meta = $modernMeta } },
    [ordered]@{ jsonrpc = '2.0'; id = 3; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_status'; arguments = [ordered]@{ task_ref = [string]$submitted.task_ref }; _meta = $modernMeta } }
)
$statusInput = New-McpInput -Frames $statusFrames
$statusAcceptanceSessionId = [Guid]::NewGuid().ToString('N')
$statusAcceptanceSink = New-Task038McpAcceptanceEvidenceSink `
    -EvidenceRoot $evidenceRoot `
    -SessionId $statusAcceptanceSessionId
$statusAcceptanceSafeConfigSha256 = Get-StringSha256 -Value (@(
    'lattice.task038.mcp-acceptance-safe-config.v1',
    $acceptanceId,
    'RESUME_EXISTING',
    (Get-StringSha256 -Value $statusInput),
    $candidateSourceLinkageRawSha256,
    $candidateLatticedSha256,
    $clusterMarkerRawSha256,
    [string]$clusterMarker.system_identifier,
    [string]$authority.authority_revision
) -join "`n")
$statusSession = Invoke-LatticedSession `
    -InputText $statusInput `
    -RunMode 'RESUME_EXISTING' `
    -OutputPath (Join-Path $evidenceRoot 'status.response-summary.json') `
    -MetaPath (Join-Path $evidenceRoot 'status.process.json') `
    -Authority $authority `
    -DatabasePassword $databasePassword `
    -DeliveryRoot $deliveryRoot `
    -SchemaDirectory $schemaDirectory `
    -LauncherSha256 $launcherSha256 `
    -LauncherVersion $codexVersion.Text.Trim() `
    -AcceptanceEvidencePath ([string]$statusAcceptanceSink.path) `
    -AcceptanceEvidenceNativeIdentity ([string]$statusAcceptanceSink.native_identity) `
    -AcceptanceSessionId $statusAcceptanceSessionId `
    -AcceptanceSafeConfigSha256 $statusAcceptanceSafeConfigSha256 `
    -ExpectedDispatchCount 1
Assert-CredentialSourceUnchanged `
    -Root $script:CodexCredentialSource `
    -ExpectedFootprint $credentialSourceBefore `
    -ExpectedAuthSha256 $credentialSourceAuthSha256
$statusResponses = @(Get-McpResponses -Output $statusSession.Output)
if ($statusResponses.Count -ne 3) { throw 'TASK038_STATUS_RESPONSE_COUNT_REJECTED' }
Assert-StatelessDiscoverResponse -Response (Get-McpResponse -Responses $statusResponses -Id 1)
Assert-ToolDiscovery -Response (Get-McpResponse -Responses $statusResponses -Id 2)
$statusResponse = Get-McpResponse -Responses $statusResponses -Id 3
if ([bool]$statusResponse.result.isError) { throw 'TASK038_STATUS_TOOL_REJECTED' }
$status = Get-ToolStructuredContent -Response $statusResponse -ExpectedKind 'TASK_STATUS' -Protocol 'STATELESS'
Assert-PublicTaskStatusShape -Value $status
Assert-CompletedTaskStatus -Value $status
Assert-SamePublicTaskStatus -Expected $submitted -Actual $status
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'status.json') -Value $status

$gitAfterStatus = Get-GitFootprint -Repository $repository
$codexAfterStatus = Get-StableDirectoryFootprint -Root $script:CodexHome
$databaseAfterStatus = Get-DatabaseFootprint -Password $databasePassword -DatabaseName $databaseName -TaskRef ([string]$submitted.task_ref)
Assert-CompletedDatabaseFootprint -Footprint $databaseAfterStatus -PublicStatus $status -ExpectedCommandId ('mcp-submit:' + $sameClientRequestId) -Baseline $before
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'git-after-status.json') -Value $gitAfterStatus
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'codex-home-after-status.json') -Value ([ordered]@{ codex_home_footprint = $codexAfterStatus })
Write-JsonEvidence -Path (Join-Path $evidenceRoot 'database-after-status.json') -Value $databaseAfterStatus
if (($gitAfterSubmit | ConvertTo-Json -Compress -Depth 8) -ne ($gitAfterStatus | ConvertTo-Json -Compress -Depth 8)) {
    throw 'TASK038_FRESH_STATUS_GIT_FOOTPRINT_REJECTED'
}
if (($codexAfterSubmit | ConvertTo-Json -Compress -Depth 8) -ne ($codexAfterStatus | ConvertTo-Json -Compress -Depth 8)) {
    throw 'TASK038_FRESH_STATUS_CODEX_HOME_FOOTPRINT_REJECTED'
}
if (($databaseAfterSubmit | ConvertTo-Json -Compress -Depth 8) -ne ($databaseAfterStatus | ConvertTo-Json -Compress -Depth 8)) {
    throw 'TASK038_FRESH_STATUS_DATABASE_FOOTPRINT_REJECTED'
}

$submitProcessPresentAfter = $null -ne (Get-Process -Id ([int]$submitSession.ProcessId) -ErrorAction SilentlyContinue)
$statusProcessPresentAfter = $null -ne (Get-Process -Id ([int]$statusSession.ProcessId) -ErrorAction SilentlyContinue)
$tcpOwnersAfter = @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object {
    [int]$_.OwningProcess -in @([int]$submitSession.ProcessId, [int]$statusSession.ProcessId)
}).Count
$udpOwnersAfter = @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue | Where-Object {
    [int]$_.OwningProcess -in @([int]$submitSession.ProcessId, [int]$statusSession.ProcessId)
}).Count
if ($submitProcessPresentAfter -or $statusProcessPresentAfter -or $tcpOwnersAfter -ne 0 -or $udpOwnersAfter -ne 0) {
    throw 'TASK038_SESSION_EFFECT_CLEANUP_REJECTED'
}
$effectObservationPath = Join-Path $evidenceRoot 'production-effect-observation.json'
$effectEvidenceFiles = [ordered]@{}
foreach ($name in @(
    'candidate-source-linkage.json',
    'database-binding.json',
    'database-before.json',
    'database-after-submit.json',
    'database-after-status.json',
    'git-after-submit.json',
    'git-after-status.json',
    'codex-home-after-submit.json',
    'codex-home-after-status.json',
    'submit.process.json',
    'status.process.json'
)) {
    $path = Join-Path $evidenceRoot $name
    $effectEvidenceFiles[$name] = Get-FileSha256 -Path $path
}
$effectObservation = [ordered]@{
    schema_version = 'lattice.task038.production-effect-observation.v1'
    acceptance_id = $acceptanceId
    candidate_source_linkage_raw_sha256 = $candidateSourceLinkageRawSha256
    source_commit = [string]$candidateSourceLinkage.source_commit
    source_tree = [string]$candidateSourceLinkage.source_tree
    canonical_latticed_sha256 = $candidateLatticedSha256
    postgres_run_id = $PostgresRunId
    postgres_system_identifier = [string]$clusterMarker.system_identifier
    postgres_port = $PostgresPort
    postgres_marker_raw_sha256 = $clusterMarkerRawSha256
    process_private_dispatch_sink = $true
    dispatch_sessions = @(
        [ordered]@{
            phase = 'SUBMIT'
            session_id = [string]$submitSession.AcceptanceEvidence.session_id
            safe_config_sha256 = [string]$submitSession.AcceptanceEvidence.safe_config_sha256
            raw_sha256 = [string]$submitSession.AcceptanceEvidence.raw_sha256
            final_event_sha256 = [string]$submitSession.AcceptanceEvidence.final_event_sha256
            dispatch_accepted_count = [int]$submitSession.AcceptanceEvidence.dispatch_accepted_count
            normal_close_complete = [bool]$submitSession.AcceptanceEvidence.normal_close_complete
        },
        [ordered]@{
            phase = 'STATUS'
            session_id = [string]$statusSession.AcceptanceEvidence.session_id
            safe_config_sha256 = [string]$statusSession.AcceptanceEvidence.safe_config_sha256
            raw_sha256 = [string]$statusSession.AcceptanceEvidence.raw_sha256
            final_event_sha256 = [string]$statusSession.AcceptanceEvidence.final_event_sha256
            dispatch_accepted_count = [int]$statusSession.AcceptanceEvidence.dispatch_accepted_count
            normal_close_complete = [bool]$statusSession.AcceptanceEvidence.normal_close_complete
        }
    )
    database_before_sha256 = [string]$effectEvidenceFiles['database-before.json']
    database_after_submit_sha256 = [string]$effectEvidenceFiles['database-after-submit.json']
    database_after_status_sha256 = [string]$effectEvidenceFiles['database-after-status.json']
    filesystem_git_after_submit_sha256 = [string]$effectEvidenceFiles['git-after-submit.json']
    filesystem_git_after_status_sha256 = [string]$effectEvidenceFiles['git-after-status.json']
    filesystem_codex_home_after_submit_sha256 = [string]$effectEvidenceFiles['codex-home-after-submit.json']
    filesystem_codex_home_after_status_sha256 = [string]$effectEvidenceFiles['codex-home-after-status.json']
    process_job_active_count_after_cleanup = 0
    process_session_pids_present_after_cleanup = 0
    network_tcp_owner_rows_after_cleanup = $tcpOwnersAfter
    network_udp_owner_rows_after_cleanup = $udpOwnersAfter
    codex_invocation_count = [int]$databaseAfterStatus.codex_intents
    downstream_graphify_hermes_memory_effect_delta = 0
    status_database_footprint_unchanged = $true
    status_git_footprint_unchanged = $true
    status_codex_home_footprint_unchanged = $true
    evidence_file_sha256 = $effectEvidenceFiles
}
Write-JsonEvidence -Path $effectObservationPath -Value $effectObservation
$effectObservationRawSha256 = Get-FileSha256 -Path $effectObservationPath

$finalPath = Join-Path $evidenceRoot 'final.json'
$final = [ordered]@{
    schema_version = 'lattice.task038.local-canonical-mcp-acceptance.v1'
    status = 'PASS'
    component = 'task038-local-canonical-mcp-acceptance'
    scope = 'LOCAL_CANONICAL_MCP_NOT_CHATGPT_TUNNEL'
    acceptance_id = $acceptanceId
    task_ref = [string]$status.task_ref
    task_state = [string]$status.task_state
    result_digest = [string]$status.result_digest
    ledger_head_digest = [string]$status.ledger_head_digest
    canonical_latticed_sha256 = Get-FileSha256 -Path $script:Latticed
    candidate_source_commit = [string]$candidateSourceLinkage.source_commit
    candidate_source_tree = [string]$candidateSourceLinkage.source_tree
    candidate_source_exact_path_entries_sha256 = [string]$candidateSourceLinkage.exact_path_entries_sha256
    candidate_source_linkage_raw_sha256 = $candidateSourceLinkageRawSha256
    production_effect_observation_schema = 'lattice.task038.production-effect-observation.v1'
    production_effect_observation_raw_sha256 = $effectObservationRawSha256
    submit_dispatch_evidence_raw_sha256 = [string]$submitSession.AcceptanceEvidence.raw_sha256
    submit_dispatch_final_event_sha256 = [string]$submitSession.AcceptanceEvidence.final_event_sha256
    status_dispatch_evidence_raw_sha256 = [string]$statusSession.AcceptanceEvidence.raw_sha256
    status_dispatch_final_event_sha256 = [string]$statusSession.AcceptanceEvidence.final_event_sha256
    official_codex_sha256 = $launcherSha256
    writer_lease_live_suite_passed_without_skip = $true
    legacy_discovery_and_submit = $true
    same_key_retry_exact = $true
    different_key_substitution_denied = $true
    fresh_process_status = $true
    postgres_restart_between_submit_and_status = $true
    submit_process_id = [int]$submitSession.ProcessId
    status_process_id = [int]$statusSession.ProcessId
    postgres_authoritative_replay = $true
    codex_invocation_count = [int]$databaseAfterStatus.codex_intents
    verification_outcome_count = [int]$databaseAfterStatus.verified_outcomes
    git_head = [string]$gitAfterStatus.git_head
    git_commit_count = [int]$gitAfterStatus.commit_count
    ledger_fingerprint = [string]$databaseAfterStatus.ledger_fingerprint
    codex_home_footprint = [string]$codexAfterStatus.digest
    status_zero_rerun = $true
    downstream_graphify_hermes_memory_effect_delta = 0
    chatgpt_tunnel_claimed = $false
    evidence_root = $evidenceRoot
}
}
catch {
    $primaryFailure = Get-Task038FailureClassification -ErrorRecord $_
}
finally {
    if ($null -ne $executionHome) {
        try {
            Remove-FreshCodexExecutionHome `
                -Path ([string]$executionHome.Path) `
                -ExpectedParent ([string]$executionHome.Parent) `
                -AcceptanceId $acceptanceId
        }
        catch {
            $homeCleanupFailure = Get-Task038FailureClassification -ErrorRecord $_
        }
    }
    try {
        Assert-CredentialSourceUnchanged `
            -Root $script:CodexCredentialSource `
            -ExpectedFootprint $credentialSourceBefore `
            -ExpectedAuthSha256 $credentialSourceAuthSha256
    }
    catch {
        $sourceCheckFailure = Get-Task038FailureClassification -ErrorRecord $_
    }
}
$finalizationFailures = [Collections.Generic.List[string]]::new()
if ($null -ne $primaryFailure) {
    $finalizationFailures.Add(('PRIMARY=' + ([string]$primaryFailure -split '\|')[0]))
}
if ($null -ne $homeCleanupFailure) {
    $finalizationFailures.Add(('HOME=' + ([string]$homeCleanupFailure -split '\|')[0]))
}
if ($null -ne $sourceCheckFailure) {
    $finalizationFailures.Add(('SOURCE=' + ([string]$sourceCheckFailure -split '\|')[0]))
}
if ($null -ne $homeCleanupFailure -or $null -ne $sourceCheckFailure) {
    throw ('TASK038_ACCEPTANCE_FINALIZATION_REJECTED|' + [string]::Join('|', $finalizationFailures))
}
if ($null -ne $primaryFailure) {
    throw $primaryFailure
}
if ($null -eq $final -or [string]::IsNullOrWhiteSpace($finalPath)) {
    throw 'TASK038_ACCEPTANCE_FINAL_EVIDENCE_REJECTED'
}
$final.execution_home_removed = $true
$final.credential_source_unchanged = $true
Write-JsonEvidence -Path $finalPath -Value $final
Write-Output 'LATTICED_LOCAL_MCP_ACCEPTANCE=PASS'
Write-Output (([ordered]@{
    status = 'PASS'
    scope = 'LOCAL_CANONICAL_MCP_NOT_CHATGPT_TUNNEL'
    task_ref = [string]$status.task_ref
    evidence_path = $finalPath
}) | ConvertTo-Json -Compress)
