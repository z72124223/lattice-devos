[CmdletBinding()]
param(
    [switch]$RunLatticeDeliveryHook,
    [switch]$RunFullChainAcceptanceHook,
    [switch]$RunTask038AcceptanceHook,
    [switch]$RunTask038TunnelHook,
    [string]$Task038OfficialCodexExecutable,
    [string]$Task038CodexAuthHome,
    [string]$Task038TunnelClientExecutable,
    [string]$Task038TunnelProfileDirectory,
    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,63}$')]
    [string]$Task038TunnelProfileName = 'lattice-local',
    [ValidateRange(60, 1800)]
    [int]$HolderTtlSeconds = 900,
    [switch]$RunTask068HermesReplayGate,
    [switch]$MemoryOnly,
    [switch]$StoreOnly,
    [switch]$RunTask075MemoryGate,
    [switch]$MeasureTask075Catalog,
    [switch]$MeasureTask075CurrentCatalog,
    [switch]$SelfTestOnly
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
    throw 'TASK019_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}
try {
    $nativeIdentityHelperSource = [Text.UTF8Encoding]::new($false, $true).GetString(
        [IO.File]::ReadAllBytes($nativeIdentityHelperItem.FullName)
    )
    . ([scriptblock]::Create($nativeIdentityHelperSource))
    Initialize-LatticeWindowsNativePathIdentity
}
catch {
    throw 'TASK019_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}
$task075CatalogMeasurementRequested = $MeasureTask075Catalog -or $MeasureTask075CurrentCatalog
if ($MeasureTask075Catalog -and $MeasureTask075CurrentCatalog) {
    throw 'TASK075_CATALOG_MEASUREMENT_MODE_CONFLICT'
}
if ($task075CatalogMeasurementRequested) {
    if (
        $MemoryOnly -or $RunTask075MemoryGate -or $RunLatticeDeliveryHook -or
        $RunFullChainAcceptanceHook -or
        $RunTask038AcceptanceHook -or $RunTask038TunnelHook -or $RunTask068HermesReplayGate
    ) {
        throw 'TASK075_CATALOG_MEASUREMENT_PROFILE_CONFLICT'
    }
    $StoreOnly = $true
}
if (
    $SelfTestOnly -and (
        $MemoryOnly -or $StoreOnly -or $RunTask075MemoryGate -or
        $task075CatalogMeasurementRequested -or
        $RunLatticeDeliveryHook -or $RunFullChainAcceptanceHook -or
        $RunTask038AcceptanceHook -or $RunTask038TunnelHook -or
        $RunTask068HermesReplayGate
    )
) {
    throw 'TASK019_SELF_TEST_PROFILE_CONFLICT'
}
function Test-Task019ProfileModeSelection {
    param(
        [bool]$MemoryOnlySelected,
        [bool]$StoreOnlySelected,
        [bool]$Task075MemoryGateSelected
    )

    return @(
        @($MemoryOnlySelected, $StoreOnlySelected, $Task075MemoryGateSelected) |
            Where-Object { $_ }
    ).Count -le 1
}
if (-not (Test-Task019ProfileModeSelection `
        -MemoryOnlySelected $MemoryOnly `
        -StoreOnlySelected $StoreOnly `
        -Task075MemoryGateSelected $RunTask075MemoryGate)) {
    throw 'TASK019_HARNESS_PROFILE_SELECTION_CONFLICT'
}
if ($StoreOnly -and ($RunLatticeDeliveryHook -or $RunFullChainAcceptanceHook -or $RunTask038AcceptanceHook)) {
    throw 'TASK019_STORE_ONLY_HOOK_FORBIDDEN'
}
if (
    $RunTask075MemoryGate -and (
        $RunLatticeDeliveryHook -or $RunFullChainAcceptanceHook -or
        $RunTask038AcceptanceHook -or $RunTask038TunnelHook -or
        $RunTask068HermesReplayGate
    )
) {
    throw 'TASK075_MEMORY_GATE_HOOK_FORBIDDEN'
}
$extensionHookRequested =
    $RunLatticeDeliveryHook -or $RunFullChainAcceptanceHook -or $RunTask038AcceptanceHook
if (
    $extensionHookRequested -and -not $MemoryOnly -and -not $StoreOnly -and
    -not $RunTask075MemoryGate
) {
    # Preserve the established hook CLI while pinning every extension hook to
    # the separately governed frozen V3 Memory profile.
    $MemoryOnly = $true
}

# Bare schema V5 and the frozen V3 Memory profile are intentionally distinct
# databases. The no-hook default is the formal composite gate and must never
# install a V3 extension into the StoreOnly V5 successor.
if (
    -not $MemoryOnly -and
    -not $StoreOnly -and
    -not $RunTask075MemoryGate -and
    -not $SelfTestOnly -and
    -not $extensionHookRequested
) {
    & $PSCommandPath -StoreOnly
    & $PSCommandPath -MemoryOnly
    Write-Output 'TASK019_COMPOSITE_PROFILE_HARNESS=PASS'
    return
}

$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$requiredExecutables = @(
    'initdb.exe',
    'pg_ctl.exe',
    'pg_isready.exe',
    'postgres.exe',
    'psql.exe'
)
$serviceName = 'postgresql-x64-17'
$markerName = '.lattice-task019-disposable.json'
$expectedPostgresVersion = '17.10'
$expectedPostgresExecutableSha256 = '882a5a073a88817f6c6d4c8827df1e4269ff226d52cf6f47c9883e91088c6345'
$expectedPsqlExecutableSha256 = 'e43adb9c5032e7efc63eebb44c5d32b142b34e5f4207666fed2dc7a51d43b630'
$expectedPgCtlExecutableSha256 = 'abe89b0767a8cd0f956059aa5a5a93cd1042efc6194d000c2501da3e23babbd2'
$task075GlobalV5ManifestSha256 = 'f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d'
$task075MemoryV3ManifestSha256 = 'd4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0'
$harnessUser = 'task019_harness'
$reservedPostgresPorts = [Collections.Generic.HashSet[int]]::new()
foreach ($reservedPort in @(5432, 64272, 55432)) {
    $null = $reservedPostgresPorts.Add([int]$reservedPort)
}
$environmentNames = @(
    'LATTICE_TASK019_LIVE',
    'LATTICE_TASK019_PHASE',
    'LATTICE_TASK019_HOST',
    'LATTICE_TASK019_PORT',
    'LATTICE_TASK019_PASSWORD',
    'LATTICE_TASK019_RUN_ID',
    'LATTICE_TASK019_EXPECTED_UUID',
    'LATTICE_TASK019_EXPECTED_MANIFEST',
    'LATTICE_TASK075_CURRENT_CATALOG_ONLY',
    'LATTICE_TASK068_EXPECTED_RECEIPT_SHA256',
    'LATTICE_TASK038_POSTGRES_PASSWORD',
    'LATTICE_WRITER_LEASE_MIGRATOR_URL',
    'LATTICE_WRITER_LEASE_RUNTIME_URL',
    'LATTICE_WRITER_LEASE_ADMIN_URL',
    'LATTICE_WRITER_LEASE_DATABASE_NAME',
    'LATTICE_WRITER_LEASE_DATABASE_IDENTITY_SHA256',
    'LATTICE_WRITER_LEASE_GLOBAL_MANIFEST_SHA256',
    'LATTICE_WRITER_LEASE_MEMORY_MANIFEST_SHA256',
    'LATTICE_WRITER_LEASE_DAEMON_INSTANCE_ID',
    'LATTICE_WRITER_LEASE_DAEMON_EPOCH',
    'LATTICE_WRITER_LEASE_AUTHORITY_REVISION',
    'LATTICE_WRITER_LEASE_ADMISSION_OBSERVATION_SHA256',
    'LATTICE_WRITER_LEASE_AUTHORITY_HEAD_SHA256',
    'LATTICE_STORE_PROFILE_LIVE',
    'LATTICE_STORE_PROFILE_EXPECTED',
    'LATTICE_STORE_PROFILE_RUNTIME_URL',
    'LATTICE_STORE_PROFILE_MIGRATOR_URL',
    'LATTICE_STORE_CATALOG_SIGNATURE_URL',
    'LATTICE_MEMORY_CATALOG_SIGNATURE_URL'
    'LATTICE_FULL_CHAIN_RUN_MODE'
    'LATTICE_DELIVERY_CODEX_MODE'
    'LATTICE_DELIVERY_TIMEOUT_SECONDS'
    'LATTICE_STORE_DAEMON_INSTANCE_ID'
    'LATTICE_STORE_DAEMON_EPOCH'
    'LATTICE_STORE_AUTHORITY_REVISION'
    'LATTICE_STORE_OBSERVATION_DIGEST'
    'LATTICE_STORE_AUTHORITY_HEAD_DIGEST'
    'LATTICE_TASK019_HOLDER_RECEIPT_PATH'
    'LATTICE_TASK019_HOLDER_SESSION_ID'
    'LATTICE_TASK019_HOLDER_NONCE'
    'LATTICE_TASK019_HOLDER_NONCE_COMMITMENT'
    'LATTICE_TASK019_HOLDER_CONSUMER_SESSION_ID'
    'LATTICE_TASK019_HOLDER_DEADLINE_UTC'
    'LATTICE_P0_CONSUMER_SESSION_ID'
)

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
        throw 'TASK-019 path is outside the repository boundary.'
    }

    $current = $canonicalPath
    while ($true) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw 'TASK-019 path has an existing reparse-point ancestor.'
            }
        }
        if (Test-ExactPath -Actual $current -Expected $canonicalBoundary) {
            break
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrWhiteSpace($parent) -or (Test-ExactPath -Actual $parent -Expected $current)) {
            throw 'TASK-019 path ancestry could not be proved.'
        }
        $current = $parent
    }
}

function Get-LatticeDeliveryHookPath {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptDirectory,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalScriptDirectory = Get-CanonicalPath -Path $ScriptDirectory
    $expectedPath = Get-CanonicalPath -Path (Join-Path $canonicalScriptDirectory 'run-lattice-delivery.ps1')
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $expectedPath) -Expected $canonicalScriptDirectory)) {
        throw 'TASK019_DELIVERY_HOOK_NOT_EXACT_SIBLING'
    }

    Assert-NoReparseAncestor -Path $expectedPath -Boundary $RepositoryRoot
    $item = Get-Item -LiteralPath $expectedPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $expectedPath -PathType Leaf) -or
        -not (Test-ExactPath -Actual $item.FullName -Expected $expectedPath)
    ) {
        throw 'TASK019_DELIVERY_HOOK_NOT_REGULAR_LEAF'
    }

    return $expectedPath
}

function Get-LatticeFullChainAcceptanceHookPath {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptDirectory,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalScriptDirectory = Get-CanonicalPath -Path $ScriptDirectory
    $expectedPath = Get-CanonicalPath -Path (Join-Path $canonicalScriptDirectory 'run-task037-full-chain-verification.ps1')
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $expectedPath) -Expected $canonicalScriptDirectory)) {
        throw 'TASK037_FULL_CHAIN_HOOK_NOT_EXACT_SIBLING'
    }

    Assert-NoReparseAncestor -Path $expectedPath -Boundary $RepositoryRoot
    $item = Get-Item -LiteralPath $expectedPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $expectedPath -PathType Leaf) -or
        -not (Test-ExactPath -Actual $item.FullName -Expected $expectedPath)
    ) {
        throw 'TASK037_FULL_CHAIN_HOOK_NOT_REGULAR_LEAF'
    }

    return $expectedPath
}

function Get-LatticeTask038AcceptanceHookPath {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptDirectory,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalScriptDirectory = Get-CanonicalPath -Path $ScriptDirectory
    $expectedPath = Get-CanonicalPath -Path (Join-Path $canonicalScriptDirectory 'run-task038-task-submit.ps1')
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $expectedPath) -Expected $canonicalScriptDirectory)) {
        throw 'TASK038_ACCEPTANCE_HOOK_NOT_EXACT_SIBLING'
    }
    Assert-NoReparseAncestor -Path $expectedPath -Boundary $RepositoryRoot
    $item = Get-Item -LiteralPath $expectedPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $expectedPath -PathType Leaf) -or
        -not (Test-ExactPath -Actual $item.FullName -Expected $expectedPath)
    ) {
        throw 'TASK038_ACCEPTANCE_HOOK_NOT_REGULAR_LEAF'
    }
    return $expectedPath
}

function Get-LatticeTask038TunnelHookPath {
    param(
        [Parameter(Mandatory = $true)][string]$ScriptDirectory,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $canonicalScriptDirectory = Get-CanonicalPath -Path $ScriptDirectory
    $expectedPath = Get-CanonicalPath -Path (Join-Path $canonicalScriptDirectory 'start-chatgpt-mcp-tunnel.ps1')
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $expectedPath) -Expected $canonicalScriptDirectory)) {
        throw 'TASK038_TUNNEL_HOOK_NOT_EXACT_SIBLING'
    }
    Assert-NoReparseAncestor -Path $expectedPath -Boundary $RepositoryRoot
    $item = Get-Item -LiteralPath $expectedPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [IO.FileInfo]) -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $expectedPath -PathType Leaf) -or
        -not (Test-ExactPath -Actual $item.FullName -Expected $expectedPath)
    ) {
        throw 'TASK038_TUNNEL_HOOK_NOT_REGULAR_LEAF'
    }
    return $expectedPath
}

function Test-PgCtlStatusCodeIsStopped {
    param([Parameter(Mandatory = $true)][int]$StatusCode)

    return ($StatusCode -eq 3)
}

function Test-StoreProfileLiveGateOutput {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][object[]]$Output,
        [Parameter(Mandatory = $true)][string]$ExpectedProfile
    )

    $allowedProfiles = @(
        'V5', 'V5_MEMORY_V3', 'V3_MEMORY_V2', 'V3_MEMORY_V2_WRITER_LEASE_V1'
    )
    if ($ExpectedProfile -notin $allowedProfiles) {
        return $false
    }
    $text = @($Output | ForEach-Object { [string]$_ }) -join "`n"
    $escapedProfile = [regex]::Escape($ExpectedProfile)
    $passPattern = "(?m)(?:^|[^\S\r\n])PASS: Store live profile $escapedProfile accepted with exact fail-closed matrix[ `t]*$"
    $skipPattern = '(?m)(?:^|[^\S\r\n])SKIP:'
    return (
        $ExitCode -eq 0 -and
        $text -match $passPattern -and
        $text -notmatch $skipPattern
    )
}

function Get-Task019AllowlistedDiagnosticTokens {
    param([Parameter(Mandatory = $true)][object[]]$Output)

    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $tokens = @()
    foreach ($item in $Output) {
        foreach ($match in [regex]::Matches(
            [string]$item,
            '(?<![A-Z0-9_])(?:TASK019|TASK075|STORE|POSTGRES_TASK_LEDGER|POSTGRES_PROJECT_REGISTRY|MEMORY|OPENCLAW)_[A-Z0-9_]{1,63}(?![A-Z0-9_])'
        )) {
            if ($seen.Add($match.Value)) {
                $tokens += $match.Value
            }
        }
    }
    return $tokens
}

function Get-Task019SafeDiagnosticSummary {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Tokens)

    if ($Tokens.Count -eq 0) {
        return 'No allowlisted static diagnostic was emitted.'
    }
    return (@($Tokens | ForEach-Object { [string]$_ }) -join ' | ')
}

function Get-Task075LastIncompleteStageToken {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Tokens)

    $openStages = [Collections.Generic.List[string]]::new()
    foreach ($item in $Tokens) {
        $token = [string]$item
        $enter = [regex]::Match($token, '\ATASK075_STAGE_ENTER_(?<name>[A-Z0-9_]{1,48})\z')
        if ($enter.Success) {
            $openStages.Add($token)
            continue
        }
        $pass = [regex]::Match($token, '\ATASK075_STAGE_PASS_(?<name>[A-Z0-9_]{1,48})\z')
        if ($pass.Success) {
            $expectedEnter = 'TASK075_STAGE_ENTER_' + $pass.Groups['name'].Value
            for ($index = $openStages.Count - 1; $index -ge 0; $index--) {
                if ($openStages[$index] -ceq $expectedEnter) {
                    $openStages.RemoveAt($index)
                    break
                }
            }
        }
    }
    if ($openStages.Count -eq 0) {
        return $null
    }
    return $openStages[$openStages.Count - 1]
}

function Get-StoreProfileForLiveSuitePhase {
    param(
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][string]$SuiteName,
        [switch]$Task075MemoryGate
    )

    if (
        $Task075MemoryGate -and $SuiteName -eq 'memory' -and
        $Phase -in @('initial', 'restart')
    ) {
        return 'V5_MEMORY_V3'
    }
    if ($Phase -eq 'initial' -and $SuiteName -eq 'store') {
        return 'V5'
    }
    if ($Phase -eq 'initial' -and $SuiteName -eq 'memory') {
        return 'V3_MEMORY_V2'
    }
    return $null
}

function Test-Task075MemoryGateOutput {
    param(
        [Parameter(Mandatory = $true)][ValidateSet('initial', 'restart')][string]$Phase,
        [Parameter(Mandatory = $true)][object[]]$Output
    )

    $text = @($Output | ForEach-Object { [string]$_ }) -join "`n"
    if ($Phase -eq 'restart') {
        return (
            [regex]::Matches(
                $text,
                '(?m)(?:^|\s)MEMORY_EXTENSION_RESTART_PROFILE_OK[ `t]*$'
            ).Count -eq 1 -and
            $text -notmatch '(?m)(?:^|\s)(?:TASK075_MEMORY_V5_SETUP_OK|MEMORY_EXTENSION_INITIAL_OK)'
        )
    }

    $evidence = [regex]::Matches(
        $text,
        '(?m)(?:^|\s)TASK019_EVIDENCE database_uuid=(?<uuid>[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}) manifest_sha256=(?<manifest>[0-9a-f]{64})[ `t]*$'
    )
    $memory = [regex]::Matches(
        $text,
        '(?m)(?:^|\s)MEMORY_EXTENSION_INITIAL_OK database_uuid=(?<uuid>[0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}) extension_manifest=(?<manifest>[0-9a-f]{64})[ `t]*$'
    )
    return (
        [regex]::Matches($text, '(?m)(?:^|\s)TASK075_MEMORY_V5_SETUP_OK[ `t]*$').Count -eq 1 -and
        $evidence.Count -eq 1 -and
        $memory.Count -eq 1 -and
        $evidence[0].Groups['uuid'].Value -ceq $memory[0].Groups['uuid'].Value -and
        $evidence[0].Groups['manifest'].Value -ceq $task075GlobalV5ManifestSha256 -and
        $memory[0].Groups['manifest'].Value -ceq $task075MemoryV3ManifestSha256 -and
        $text -notmatch '(?m)(?:^|\s)(?:TASK019_MEMORY_SETUP_OK|MEMORY_EXTENSION_RESTART_PROFILE_OK|WRITER_LEASE_)'
    )
}

function Test-Task075CatalogMeasurementShape {
    param(
        [Parameter(Mandatory = $true)][object[]]$Output,
        [switch]$CurrentOnly
    )

    $expectedCount = if ($CurrentOnly) { 17 } else { 43 }
    if ($Output.Count -ne $expectedCount) {
        return $false
    }
    $metrics = @(
        'RELATION', 'COLUMN', 'CONSTRAINT', 'INDEX',
        'FUNCTION', 'TABLE_ACL', 'FUNCTION_ACL', 'SCHEMA_ACL'
    )
    $groups = if ($CurrentOnly) {
        @('V5_MEMORY_V3|STORE', 'V5_MEMORY_V3|MEMORY')
    }
    else {
        @(
            'V5_BARE|STORE',
            'V5_MEMORY_V2|STORE',
            'V5_MEMORY_V2|MEMORY',
            'V5_MEMORY_V3|STORE',
            'V5_MEMORY_V3|MEMORY'
        )
    }
    $pattern = '\ATASK075_(?<profile>V5_BARE|V5_MEMORY_V2|V5_MEMORY_V3)_(?<source>STORE|MEMORY)_CATALOG_(?<metric>RELATION|COLUMN|CONSTRAINT|INDEX|FUNCTION|TABLE_ACL|FUNCTION_ACL|SCHEMA_ACL|AUTONOMY)_SIGNATURE=(?<digest>[a-f0-9]{64})\z'
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($item in $Output) {
        if ($item -isnot [string]) {
            return $false
        }
        $match = [regex]::Match([string]$item, $pattern)
        if (-not $match.Success) {
            return $false
        }
        $group = $match.Groups['profile'].Value + '|' + $match.Groups['source'].Value
        if ($group -notin $groups) {
            return $false
        }
        $metric = $match.Groups['metric'].Value
        if ($metric -eq 'AUTONOMY' -and $match.Groups['source'].Value -ne 'STORE') {
            return $false
        }
        if (-not $seen.Add($group + '|' + $metric)) {
            return $false
        }
    }
    foreach ($group in $groups) {
        foreach ($metric in $metrics) {
            if (-not $seen.Contains($group + '|' + $metric)) {
                return $false
            }
        }
        if ($group.EndsWith('|STORE') -and -not $seen.Contains($group + '|AUTONOMY')) {
            return $false
        }
    }
    return $true
}

function Invoke-HarnessSelfTest {
    if (
        -not (Test-Task019ProfileModeSelection $false $false $false) -or
        -not (Test-Task019ProfileModeSelection $false $false $true) -or
        (Test-Task019ProfileModeSelection $true $false $true) -or
        (Test-Task019ProfileModeSelection $false $true $true) -or
        (Test-Task019ProfileModeSelection $true $true $true)
    ) {
        throw 'TASK075_MEMORY_GATE_MODE_SELECTION_SELF_TEST_REJECTED'
    }
    if (-not (Test-PgCtlStatusCodeIsStopped -StatusCode 3)) {
        throw 'TASK-019 stopped-state contract rejected exit 3.'
    }
    foreach ($statusCode in @(0, 1, 2, 4, 5, 127)) {
        if (Test-PgCtlStatusCodeIsStopped -StatusCode $statusCode) {
            throw 'TASK-019 stopped-state contract accepted an unknown status.'
        }
    }
    $profilePass = @(
        'test postgres_setup::tests::live_store_profile ... PASS: Store live profile V5 accepted with exact fail-closed matrix'
    )
    if (-not (Test-StoreProfileLiveGateOutput -ExitCode 0 -Output $profilePass -ExpectedProfile 'V5')) {
        throw 'TASK019_STORE_PROFILE_OUTPUT_SELF_TEST_REJECTED_PASS'
    }
    $memoryProfilePass = @(
        'test postgres_setup::tests::live_store_profile ... PASS: Store live profile V5_MEMORY_V3 accepted with exact fail-closed matrix'
    )
    if (-not (Test-StoreProfileLiveGateOutput `
            -ExitCode 0 `
            -Output $memoryProfilePass `
            -ExpectedProfile 'V5_MEMORY_V3')) {
        throw 'TASK075_MEMORY_PROFILE_OUTPUT_SELF_TEST_REJECTED_PASS'
    }
    foreach ($rejected in @(
        [pscustomobject]@{ ExitCode = 1; Output = $profilePass; Profile = 'V5' },
        [pscustomobject]@{ ExitCode = 0; Output = @('SKIP: LATTICE_STORE_PROFILE_LIVE is not enabled'); Profile = 'V5' },
        [pscustomobject]@{ ExitCode = 0; Output = $profilePass; Profile = 'V3_MEMORY_V2' },
        [pscustomobject]@{ ExitCode = 0; Output = $profilePass; Profile = 'UNKNOWN' }
    )) {
        if (Test-StoreProfileLiveGateOutput `
                -ExitCode $rejected.ExitCode `
                -Output $rejected.Output `
                -ExpectedProfile $rejected.Profile) {
            throw 'TASK019_STORE_PROFILE_OUTPUT_SELF_TEST_ACCEPTED_REJECTION'
        }
    }
    $rawDiagnosticSentinel = 'raw-password-must-not-survive'
    $safeDiagnosticTokens = @(Get-Task019AllowlistedDiagnosticTokens -Output @(
        'TASK075_STAGE_ENTER_FRESH_V5_RECONCILIATION',
        'TASK075_STAGE_PASS_FRESH_V5_RECONCILIATION',
        'TASK075_STAGE_ENTER_GLOBAL_V5_PENDING',
        "panic TASK075_MEMORY_V3_LEDGER_FK_NOT_EXACT $rawDiagnosticSentinel",
        'STORE_MIGRATION_HISTORY_MISMATCH TASK075_MEMORY_V3_LEDGER_FK_NOT_EXACT'
    ))
    $safeDiagnosticSummary = Get-Task019SafeDiagnosticSummary -Tokens $safeDiagnosticTokens
    $lastIncompleteStage = Get-Task075LastIncompleteStageToken -Tokens $safeDiagnosticTokens
    if (
        $safeDiagnosticTokens.Count -ne 5 -or
        'TASK075_MEMORY_V3_LEDGER_FK_NOT_EXACT' -notin $safeDiagnosticTokens -or
        'STORE_MIGRATION_HISTORY_MISMATCH' -notin $safeDiagnosticTokens -or
        $lastIncompleteStage -cne 'TASK075_STAGE_ENTER_GLOBAL_V5_PENDING' -or
        $safeDiagnosticSummary -match [regex]::Escape($rawDiagnosticSentinel)
    ) {
        throw 'TASK075_FAILURE_DIAGNOSTIC_SELF_TEST_REJECTED_ALLOWLIST'
    }
    $zeroTokenSummary = Get-Task019SafeDiagnosticSummary `
        -Tokens @(Get-Task019AllowlistedDiagnosticTokens -Output @($rawDiagnosticSentinel))
    if ($zeroTokenSummary -cne 'No allowlisted static diagnostic was emitted.') {
        throw 'TASK075_FAILURE_DIAGNOSTIC_SELF_TEST_REJECTED_ZERO_TOKEN_FALLBACK'
    }
    if (
        (Get-StoreProfileForLiveSuitePhase -Phase 'initial' -SuiteName 'store') -ne 'V5' -or
        (Get-StoreProfileForLiveSuitePhase -Phase 'initial' -SuiteName 'memory') -ne 'V3_MEMORY_V2' -or
        (Get-StoreProfileForLiveSuitePhase -Phase 'initial' -SuiteName 'memory' -Task075MemoryGate) -ne 'V5_MEMORY_V3' -or
        (Get-StoreProfileForLiveSuitePhase -Phase 'restart' -SuiteName 'memory' -Task075MemoryGate) -ne 'V5_MEMORY_V3' -or
        $null -ne (Get-StoreProfileForLiveSuitePhase -Phase 'memory_setup' -SuiteName 'store') -or
        $null -ne (Get-StoreProfileForLiveSuitePhase -Phase 'task075_memory_setup' -SuiteName 'store' -Task075MemoryGate) -or
        $null -ne (Get-StoreProfileForLiveSuitePhase -Phase 'restart' -SuiteName 'store') -or
        $null -ne (Get-StoreProfileForLiveSuitePhase -Phase 'initial' -SuiteName 'unknown')
    ) {
        throw 'TASK019_STORE_PROFILE_PHASE_MAPPING_SELF_TEST_REJECTED'
    }
    $task075FixtureUuid = '00000000-0000-8000-8000-000000000000'
    $task075InitialOutput = @(
        "TASK019_EVIDENCE database_uuid=$task075FixtureUuid manifest_sha256=$task075GlobalV5ManifestSha256",
        'TASK075_MEMORY_V5_SETUP_OK',
        "MEMORY_EXTENSION_INITIAL_OK database_uuid=$task075FixtureUuid extension_manifest=$task075MemoryV3ManifestSha256"
    )
    if (
        -not (Test-Task075MemoryGateOutput -Phase 'initial' -Output $task075InitialOutput) -or
        -not (Test-Task075MemoryGateOutput -Phase 'restart' -Output @('MEMORY_EXTENSION_RESTART_PROFILE_OK')) -or
        (Test-Task075MemoryGateOutput -Phase 'initial' -Output @($task075InitialOutput + 'WRITER_LEASE_OWNER_LIVE_OK')) -or
        (Test-Task075MemoryGateOutput -Phase 'restart' -Output @('MEMORY_EXTENSION_INITIAL_OK'))
    ) {
        throw 'TASK075_MEMORY_GATE_OUTPUT_SHAPE_SELF_TEST_REJECTED'
    }
    $catalogShape = @(
        foreach ($group in @(
            @('V5_BARE', 'STORE'),
            @('V5_MEMORY_V2', 'STORE'),
            @('V5_MEMORY_V2', 'MEMORY'),
            @('V5_MEMORY_V3', 'STORE'),
            @('V5_MEMORY_V3', 'MEMORY')
        )) {
            $shapeMetrics = @(
                'RELATION', 'COLUMN', 'CONSTRAINT', 'INDEX',
                'FUNCTION', 'TABLE_ACL', 'FUNCTION_ACL', 'SCHEMA_ACL'
            )
            if ($group[1] -eq 'STORE') {
                $shapeMetrics += 'AUTONOMY'
            }
            foreach ($metric in $shapeMetrics) {
                'TASK075_{0}_{1}_CATALOG_{2}_SIGNATURE={3}' -f `
                    $group[0], $group[1], $metric, ('a' * 64)
            }
        }
    )
    if (-not (Test-Task075CatalogMeasurementShape -Output $catalogShape)) {
        throw 'TASK075_CATALOG_MEASUREMENT_SHAPE_SELF_TEST_REJECTED_FLAT'
    }
    if (Test-Task075CatalogMeasurementShape -Output @($catalogShape[0..41])) {
        throw 'TASK075_CATALOG_MEASUREMENT_SHAPE_SELF_TEST_ACCEPTED_PARTIAL'
    }
    $currentCatalogShape = @($catalogShape | Where-Object {
        $_ -cmatch '^TASK075_V5_MEMORY_V3_'
    })
    if (-not (Test-Task075CatalogMeasurementShape -Output $currentCatalogShape -CurrentOnly)) {
        throw 'TASK075_CURRENT_CATALOG_MEASUREMENT_SHAPE_SELF_TEST_REJECTED'
    }
    if (Test-Task075CatalogMeasurementShape -Output $catalogShape -CurrentOnly) {
        throw 'TASK075_CURRENT_CATALOG_MEASUREMENT_SHAPE_SELF_TEST_ACCEPTED_FULL'
    }
    $catalogDatabases = @(
        'lattice_task019_aaaaaaaa_catalog_bare',
        'lattice_task019_aaaaaaaa_catalog_vtwo',
        'lattice_task019_aaaaaaaa_catalog_vthree'
    )
    $catalogAccessQuery = New-Task075CatalogDatabaseAccessQuery `
        -CurrentDatabase $catalogDatabases[0] `
        -TargetDatabases $catalogDatabases
    if (
        [regex]::Matches($catalogAccessQuery, '(?im)^GRANT CONNECT ON DATABASE ').Count -ne 1 -or
        $catalogAccessQuery -cnotmatch 'GRANT CONNECT ON DATABASE "lattice_task019_aaaaaaaa_catalog_bare" TO' -or
        $catalogAccessQuery -cmatch 'REVOKE ALL ON DATABASE[^;]+FROM\s+lattice_(?:migrator|runtime|guardian|readonly),' -or
        @($catalogDatabases | Where-Object { $catalogAccessQuery -cnotmatch [regex]::Escape('"' + $_ + '"') }).Count -ne 0
    ) {
        throw 'TASK075_CATALOG_DATABASE_ACCESS_SELF_TEST_REJECTED_EXACT_TARGET'
    }
    $rejectedCatalogAccess = $false
    try {
        $null = New-Task075CatalogDatabaseAccessQuery `
            -CurrentDatabase 'lattice_task019_aaaaaaaa_catalog_other' `
            -TargetDatabases $catalogDatabases
    }
    catch {
        $rejectedCatalogAccess = $_.Exception.Message -eq 'TASK075_CATALOG_DATABASE_ACCESS_TARGET_REJECTED'
    }
    if (-not $rejectedCatalogAccess) {
        throw 'TASK075_CATALOG_DATABASE_ACCESS_SELF_TEST_ACCEPTED_FOREIGN_TARGET'
    }
    $currentCatalogAccessQuery = New-Task075CatalogDatabaseAccessQuery `
        -CurrentDatabase $catalogDatabases[2] `
        -TargetDatabases @($catalogDatabases[2]) `
        -CurrentOnly
    if (
        [regex]::Matches($currentCatalogAccessQuery, '(?im)^GRANT CONNECT ON DATABASE ').Count -ne 1 -or
        $currentCatalogAccessQuery -cnotmatch 'GRANT CONNECT ON DATABASE "lattice_task019_aaaaaaaa_catalog_vthree" TO' -or
        $currentCatalogAccessQuery -cmatch 'catalog_(?:bare|vtwo)'
    ) {
        throw 'TASK075_CURRENT_CATALOG_DATABASE_ACCESS_SELF_TEST_REJECTED'
    }
    Write-Output 'TASK019_HARNESS_SELF_TEST=PASS'
}

function Invoke-NativeChecked {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Operation
    )

    $stdoutPath = Join-Path $clusterRoot '.native-stdout.log'
    $stderrPath = Join-Path $clusterRoot '.native-stderr.log'
    Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    $process = $null
    $nativeExitCode = $null
    try {
        $startParameters = @{
            FilePath = $Executable
            ArgumentList = $Arguments
            RedirectStandardOutput = $stdoutPath
            RedirectStandardError = $stderrPath
            WindowStyle = 'Hidden'
            PassThru = $true
        }
        $process = Start-Process @startParameters
        $null = $process.Handle
        $process.WaitForExit()
        $nativeExitCode = $process.ExitCode
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    }
    if ($null -eq $nativeExitCode -or $nativeExitCode -ne 0) {
        throw "$Operation failed with exit code $nativeExitCode. Native output was suppressed."
    }
}

function Invoke-HarnessPsqlRows {
    param(
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][string]$DatabaseName,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$Query,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $sqlPath = Join-Path $clusterRoot '.writer-lease-owner.sql'
    $stdoutPath = Join-Path $clusterRoot '.writer-lease-psql-stdout.log'
    $stderrPath = Join-Path $clusterRoot '.writer-lease-psql-stderr.log'
    $originalPassword = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    $process = $null
    $exitCode = $null
    $rows = @()
    try {
        Set-Content -LiteralPath $sqlPath -Value $Query -Encoding utf8
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $process = Start-Process -FilePath $Psql -ArgumentList @(
            '-X', '-q', '-A', '-t', '--no-password', '--set', 'ON_ERROR_STOP=1',
            '--field-separator=|', '-h', '127.0.0.1', '-p', [string]$Port,
            '-U', 'lattice_migrator_login', '-d', $DatabaseName, '--file', $sqlPath
        ) -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath `
            -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $process.WaitForExit()
        $exitCode = $process.ExitCode
        if ($exitCode -eq 0 -and (Test-Path -LiteralPath $stdoutPath -PathType Leaf)) {
            $rows = @(
                Get-Content -LiteralPath $stdoutPath -Encoding utf8 |
                    Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) }
            )
        }
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $originalPassword, 'Process')
        foreach ($path in @($sqlPath, $stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }
    if ($exitCode -ne 0) {
        throw $FailureCode
    }
    return ,$rows
}

function Invoke-WriterLeaseOwnerLiveGate {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$RunId
    )

    if ($RunId -notmatch '^[0-9a-f]{32}$') {
        throw 'TASK019_WRITER_LEASE_OWNER_RUN_ID_REJECTED'
    }
    $databaseName = 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
    $daemonInstanceId = 'task022-harness-' + $RunId
    $activateQuery = @"
SET ROLE lattice_migrator;
UPDATE ONLY control.runtime_admission
SET admission_mode = 'ACTIVE',
    daemon_instance_id = '$daemonInstanceId',
    daemon_epoch = 1,
    authority_revision = 1,
    observation_digest = pg_catalog.decode(pg_catalog.repeat('a1', 32), 'hex'),
    authority_head_digest = pg_catalog.decode(pg_catalog.repeat('a2', 32), 'hex'),
    updated_at = pg_catalog.clock_timestamp()
WHERE singleton = true;
SELECT pg_catalog.btrim(e.database_identity_sha256::text),
       pg_catalog.btrim(e.global_manifest_sha256::text),
       pg_catalog.btrim(e.extension_manifest_sha256::text),
       a.daemon_instance_id,
       a.daemon_epoch::text,
       a.authority_revision::text,
       pg_catalog.encode(a.observation_digest, 'hex'),
       pg_catalog.encode(a.authority_head_digest, 'hex')
FROM ONLY memory.codebase_memory_extension_identity AS e
CROSS JOIN ONLY control.runtime_admission AS a
WHERE e.singleton = true AND a.singleton = true;
RESET ROLE;
"@
    $stopQuery = @"
SET ROLE lattice_migrator;
UPDATE ONLY control.runtime_admission
SET admission_mode = 'STOPPED',
    daemon_instance_id = NULL,
    daemon_epoch = NULL,
    authority_revision = 0,
    observation_digest = NULL,
    authority_head_digest = NULL,
    updated_at = pg_catalog.clock_timestamp()
WHERE singleton = true;
RESET ROLE;
"@
    $identityRows = Invoke-HarnessPsqlRows -Psql $Psql -DatabaseName $databaseName `
        -Port $Port -Password $Password -Query $activateQuery `
        -FailureCode 'TASK019_WRITER_LEASE_OWNER_ACTIVATION_REJECTED'
    if ($identityRows.Count -ne 1) {
        throw 'TASK019_WRITER_LEASE_OWNER_IDENTITY_SHAPE_REJECTED'
    }
    $identity = @(([string]$identityRows[0]) -split '\|', -1)
    if (
        $identity.Count -ne 8 -or
        $identity[0] -notmatch '^[0-9a-f]{64}$' -or
        $identity[1] -notmatch '^[0-9a-f]{64}$' -or
        $identity[2] -notmatch '^[0-9a-f]{64}$' -or
        $identity[3] -ne $daemonInstanceId -or
        $identity[4] -ne '1' -or
        $identity[5] -ne '1' -or
        $identity[6] -notmatch '^[0-9a-f]{64}$' -or
        $identity[7] -notmatch '^[0-9a-f]{64}$'
    ) {
        throw 'TASK019_WRITER_LEASE_OWNER_IDENTITY_REJECTED'
    }

    $encodedPassword = [Uri]::EscapeDataString($Password)
    $values = [ordered]@{
        LATTICE_WRITER_LEASE_MIGRATOR_URL = ('postgresql://lattice_migrator_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $Port, $databaseName)
        LATTICE_WRITER_LEASE_RUNTIME_URL = ('postgresql://lattice_runtime_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $Port, $databaseName)
        LATTICE_WRITER_LEASE_ADMIN_URL = ('postgresql://task019_harness:{0}@127.0.0.1:{1}/postgres' -f $encodedPassword, $Port)
        LATTICE_WRITER_LEASE_DATABASE_NAME = $databaseName
        LATTICE_WRITER_LEASE_DATABASE_IDENTITY_SHA256 = $identity[0]
        LATTICE_WRITER_LEASE_GLOBAL_MANIFEST_SHA256 = $identity[1]
        LATTICE_WRITER_LEASE_MEMORY_MANIFEST_SHA256 = $identity[2]
        LATTICE_WRITER_LEASE_DAEMON_INSTANCE_ID = $identity[3]
        LATTICE_WRITER_LEASE_DAEMON_EPOCH = $identity[4]
        LATTICE_WRITER_LEASE_AUTHORITY_REVISION = $identity[5]
        LATTICE_WRITER_LEASE_ADMISSION_OBSERVATION_SHA256 = $identity[6]
        LATTICE_WRITER_LEASE_AUTHORITY_HEAD_SHA256 = $identity[7]
    }
    $original = @{}
    $stdoutPath = Join-Path $clusterRoot '.cargo-writer-lease-owner-stdout.log'
    $stderrPath = Join-Path $clusterRoot '.cargo-writer-lease-owner-stderr.log'
    $process = $null
    $exitCode = $null
    $testOutput = @()
    try {
        foreach ($entry in $values.GetEnumerator()) {
            $original[[string]$entry.Key] = [Environment]::GetEnvironmentVariable([string]$entry.Key, 'Process')
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        $process = Start-Process -FilePath $Cargo -ArgumentList @(
            'test', '-p', 'lattice-postgres-writer-lease', '--test', 'postgres_live',
            '--locked', '--', '--nocapture', '--test-threads=1'
        ) -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $process.WaitForExit()
        $exitCode = $process.ExitCode
        foreach ($path in @($stdoutPath, $stderrPath)) {
            if (Test-Path -LiteralPath $path -PathType Leaf) {
                $testOutput += @(Get-Content -LiteralPath $path -Encoding utf8)
            }
        }
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        foreach ($entry in $original.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, $entry.Value, 'Process')
        }
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
        $null = Invoke-HarnessPsqlRows -Psql $Psql -DatabaseName $databaseName `
            -Port $Port -Password $Password -Query $stopQuery `
            -FailureCode 'TASK019_WRITER_LEASE_OWNER_STOP_REJECTED'
    }
    $text = @($testOutput | ForEach-Object { [string]$_ }) -join "`n"
    if (
        $exitCode -ne 0 -or
        $text -match '(?m)(?:^|[^\S\r\n])SKIP:' -or
        $text -notmatch '(?m)^test live_postgres_acquire_restarts_and_replays_authority_when_provisioned \.\.\. ok\s*$'
    ) {
        throw 'TASK019_WRITER_LEASE_OWNER_LIVE_GATE_REJECTED'
    }
    $script:writerLeaseOwnerProfileProved = $true
}

function Get-PgIsReadyExitCode {
    param([Parameter(Mandatory = $true)][string]$PgIsReady)

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $null = & $PgIsReady '-h' '127.0.0.1' '-p' '5432' '-t' '2' '-q' 2>&1
        return [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
}

function Get-InstalledPostgresSnapshot {
    param([Parameter(Mandatory = $true)][string]$PgIsReady)

    $service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    return [pscustomobject]@{
        ServicePresent = ($null -ne $service)
        ServiceStatus = if ($null -eq $service) { 'ABSENT' } else { [string]$service.Status }
        PgIsReady5432 = Get-PgIsReadyExitCode -PgIsReady $PgIsReady
    }
}

function Test-SameInstalledPostgresSnapshot {
    param(
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After
    )

    return (
        $Before.ServicePresent -eq $After.ServicePresent -and
        $Before.ServiceStatus -eq $After.ServiceStatus -and
        $Before.PgIsReady5432 -eq $After.PgIsReady5432
    )
}

function New-OneTimePassword {
    $bytes = New-Object byte[] 48
    $generator = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $generator.GetBytes($bytes)
        return [Convert]::ToBase64String($bytes)
    }
    finally {
        $generator.Dispose()
        [Array]::Clear($bytes, 0, $bytes.Length)
    }
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

function Get-Task019HmacSha256 {
    param(
        [Parameter(Mandatory = $true)][string]$Key,
        [Parameter(Mandatory = $true)][string]$Value
    )

    $algorithm = [Security.Cryptography.HMACSHA256]::new(
        [Text.UTF8Encoding]::new($false).GetBytes($Key)
    )
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash(
            [Text.UTF8Encoding]::new($false).GetBytes($Value)
        ))).Replace('-', '').ToLowerInvariant()
    }
    finally { $algorithm.Dispose() }
}

function Set-Task019OwnerOnlyAcl {
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
    catch { throw 'TASK019_HOLDER_ACL_REJECTED' }
}

function Enable-Task038TunnelStoreAuthority {
    param(
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })][string]$RunId
    )

    if ($RunId -cnotmatch '\A[0-9a-f]{32}\z') {
        throw 'TASK038_TUNNEL_STORE_AUTHORITY_REJECTED'
    }
    $unixSeconds = [long](([DateTime]::UtcNow - [DateTime]'1970-01-01T00:00:00Z').TotalSeconds)
    $authority = [ordered]@{
        daemon_instance_id = 'task038-tunnel-' + $RunId
        daemon_epoch = $unixSeconds
        authority_revision = $unixSeconds
        observation_digest = Get-StringSha256 -Value ('task038-tunnel-observation|' + $RunId)
        head_digest = Get-StringSha256 -Value ('task038-tunnel-authority|' + $RunId + '|' + $unixSeconds)
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
       authority_revision::text, encode(observation_digest, 'hex'),
       encode(authority_head_digest, 'hex')
FROM ONLY control.runtime_admission WHERE singleton = true;
"@
    $privateNames = @('PGPASSWORD', 'PGCONNECT_TIMEOUT', 'PGSSLMODE', 'PGSERVICE', 'PGSERVICEFILE', 'PGPASSFILE', 'PGOPTIONS')
    $original = @{}
    foreach ($name in $privateNames) {
        $original[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    }
    $output = @()
    $exitCode = $null
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        [Environment]::SetEnvironmentVariable('PGCONNECT_TIMEOUT', '5', 'Process')
        [Environment]::SetEnvironmentVariable('PGSSLMODE', 'disable', 'Process')
        foreach ($name in @('PGSERVICE', 'PGSERVICEFILE', 'PGPASSFILE', 'PGOPTIONS')) {
            [Environment]::SetEnvironmentVariable($name, $null, 'Process')
        }
        $previousErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $output = @(& $Psql `
                '--no-psqlrc' '--no-password' '--quiet' '--tuples-only' '--no-align' `
                '--field-separator' '|' '-h' '127.0.0.1' '-p' ([string]$Port) `
                '-U' $harnessUser '-d' $databaseName '-v' 'ON_ERROR_STOP=1' '-c' $query 2>&1 |
                ForEach-Object { [string]$_ })
            $exitCode = [int]$LASTEXITCODE
        }
        finally {
            $ErrorActionPreference = $previousErrorActionPreference
        }
    }
    finally {
        foreach ($name in $privateNames) {
            [Environment]::SetEnvironmentVariable($name, $original[$name], 'Process')
        }
    }
    $text = $output -join "`n"
    if ($text.IndexOf($Password, [StringComparison]::Ordinal) -ge 0) {
        throw 'TASK038_TUNNEL_STORE_AUTHORITY_OUTPUT_REJECTED'
    }
    $expected = @(
        'ACTIVE',
        [string]$authority.daemon_instance_id,
        [string]$authority.daemon_epoch,
        [string]$authority.authority_revision,
        [string]$authority.observation_digest,
        [string]$authority.head_digest
    ) -join '|'
    $rows = @($output | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($exitCode -ne 0 -or $rows.Count -ne 1 -or [string]$rows[0] -cne $expected) {
        throw 'TASK038_TUNNEL_STORE_AUTHORITY_REJECTED'
    }
    return [pscustomobject]$authority
}

function Get-UnreservedLoopbackPort {
    do {
        $listener = [System.Net.Sockets.TcpListener]::new(
            [System.Net.IPAddress]::Loopback,
            0
        )
        try {
            $listener.Start()
            $candidate = ([System.Net.IPEndPoint]$listener.LocalEndpoint).Port
        }
        finally {
            $listener.Stop()
        }
    } while ($reservedPostgresPorts.Contains([int]$candidate))

    return [int]$candidate
}

function Set-HarnessEnvironment {
    param(
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][string]$HostName,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$RunId
    )

    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_LIVE', '1', 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PHASE', $Phase, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_HOST', $HostName, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PORT', [string]$Port, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PASSWORD', $Password, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_RUN_ID', $RunId, 'Process')
    [Environment]::SetEnvironmentVariable(
        'LATTICE_TASK075_CURRENT_CATALOG_ONLY',
        $(if ($MeasureTask075CurrentCatalog) { '1' } else { $null }),
        'Process'
    )
}

function Invoke-StoreProfileLiveGate {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$ExpectedProfile,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$RunId
    )

    if (
        $ExpectedProfile -notin @(
            'V5', 'V5_MEMORY_V3', 'V3_MEMORY_V2', 'V3_MEMORY_V2_WRITER_LEASE_V1'
        )
    ) {
        throw 'TASK019_STORE_PROFILE_EXPECTATION_REJECTED'
    }
    if ($RunId -cnotmatch '\A[0-9a-f]{32}\z') {
        throw 'TASK019_STORE_PROFILE_RUN_ID_REJECTED'
    }

    $databaseName = 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
    $encodedPassword = [Uri]::EscapeDataString($Password)
    $profileEnvironment = [ordered]@{
        LATTICE_STORE_PROFILE_LIVE = '1'
        LATTICE_STORE_PROFILE_EXPECTED = $ExpectedProfile
        LATTICE_STORE_PROFILE_RUNTIME_URL = ('postgresql://lattice_runtime_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $Port, $databaseName)
        LATTICE_STORE_PROFILE_MIGRATOR_URL = ('postgresql://lattice_migrator_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $Port, $databaseName)
    }
    $original = @{}
    $stdoutPath = Join-Path $clusterRoot ".cargo-store-profile-$ExpectedProfile-stdout.log"
    $stderrPath = Join-Path $clusterRoot ".cargo-store-profile-$ExpectedProfile-stderr.log"
    $process = $null
    $testExitCode = $null
    $testOutput = @()
    Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    try {
        foreach ($entry in $profileEnvironment.GetEnumerator()) {
            $original[[string]$entry.Key] = [Environment]::GetEnvironmentVariable([string]$entry.Key, 'Process')
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        $process = Start-Process -FilePath $Cargo -ArgumentList @(
            'test',
            '-p', 'lattice-postgres-store',
            '--lib',
            'live_store_profile_accepts_exact_profiles_and_rejects_writer_lease_drift_when_provisioned',
            '--locked',
            '--',
            '--nocapture',
            '--test-threads=1'
        ) -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $process.WaitForExit()
        $testExitCode = $process.ExitCode
        if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
            $testOutput += @(Get-Content -LiteralPath $stdoutPath -Encoding utf8)
        }
        if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
            $testOutput += @(Get-Content -LiteralPath $stderrPath -Encoding utf8)
        }
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        foreach ($entry in $original.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, $entry.Value, 'Process')
        }
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $path) {
                throw 'TASK019_STORE_PROFILE_OUTPUT_DELETE_FAILED'
            }
        }
    }

    if (-not (Test-StoreProfileLiveGateOutput `
            -ExitCode $testExitCode `
            -Output $testOutput `
            -ExpectedProfile $ExpectedProfile)) {
        $safeTokens = @(Get-Task019AllowlistedDiagnosticTokens -Output $testOutput)
        $safeSummary = Get-Task019SafeDiagnosticSummary -Tokens $safeTokens
        throw "TASK019_STORE_PROFILE_LIVE_GATE_REJECTED_$ExpectedProfile diagnostics: $safeSummary"
    }
}

function Invoke-LiveTest {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    $testOutput = @()
    $liveSuites = if ($StoreOnly) {
        @([pscustomobject]@{ Name = 'store'; Package = 'lattice-postgres-store' })
    }
    elseif (($MemoryOnly -or $RunTask075MemoryGate) -and $Phase -eq 'restart') {
        @([pscustomobject]@{ Name = 'memory'; Package = 'lattice-postgres-codebase-memory' })
    }
    else {
        @(
            [pscustomobject]@{ Name = 'store'; Package = 'lattice-postgres-store' },
            [pscustomobject]@{ Name = 'memory'; Package = 'lattice-postgres-codebase-memory' }
        )
    }
    foreach ($suite in $liveSuites) {
        $suitePhase = if (($MemoryOnly -or $RunTask075MemoryGate) -and $suite.Name -eq 'store') {
            if ($RunTask075MemoryGate) { 'task075_memory_setup' } else { 'memory_setup' }
        }
        else {
            $Phase
        }
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PHASE', $suitePhase, 'Process')
        $stdoutPath = Join-Path $clusterRoot ".cargo-$Phase-$($suite.Name)-stdout.log"
        $stderrPath = Join-Path $clusterRoot ".cargo-$Phase-$($suite.Name)-stderr.log"
        $process = $null
        $testExitCode = $null
        $suiteOutput = @()
        Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
        try {
            $cargoArguments = if ($task075CatalogMeasurementRequested) {
                @(
                    'test',
                    '-p', 'lattice-postgres-store',
                    '--test', 'postgres_live',
                    'task075_catalog_signature_fixture',
                    '--locked',
                    '--',
                    '--ignored',
                    '--exact',
                    '--nocapture',
                    '--test-threads=1'
                )
            }
            else {
                @(
                    'test',
                    '-p', [string]$suite.Package,
                    '--test', 'postgres_live',
                    '--locked',
                    '--',
                    '--nocapture',
                    '--test-threads=1'
                )
            }
            $process = Start-Process -FilePath $Cargo -ArgumentList $cargoArguments `
                -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
                -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
            $null = $process.Handle
            $process.WaitForExit()
            $testExitCode = $process.ExitCode
            if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
                $suiteOutput += @(Get-Content -LiteralPath $stdoutPath -Encoding utf8)
            }
            if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
                $suiteOutput += @(Get-Content -LiteralPath $stderrPath -Encoding utf8)
            }
        }
        finally {
            if ($null -ne $process) {
                $process.Dispose()
            }
            foreach ($path in @($stdoutPath, $stderrPath)) {
                Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
                if (Test-Path -LiteralPath $path) {
                    throw 'TASK019_CARGO_OUTPUT_DELETE_FAILED'
                }
            }
        }
        if ($testExitCode -ne 0) {
            $safeTokens = @(Get-Task019AllowlistedDiagnosticTokens -Output $suiteOutput)
            $safeSummary = Get-Task019SafeDiagnosticSummary -Tokens $safeTokens
            $lastIncompleteTask075Stage = Get-Task075LastIncompleteStageToken -Tokens $safeTokens
            if ($null -ne $holderReceipt -and -not [bool]$holderReceipt.closed) {
                Add-Task019HolderEvent `
                    -Receipt $holderReceipt `
                    -EventType 'LIVE_GATE_FAILED' `
                    -Payload ([ordered]@{
                        suite = [string]$suite.Name
                        phase = $Phase
                        exit_code = [long]$testExitCode
                        diagnostics = $safeTokens
                        diagnostic_summary = $safeSummary
                        last_incomplete_task075_stage = $lastIncompleteTask075Stage
                    })
            }
            throw "$($suite.Name) postgres_live $Phase phase failed with exit code $testExitCode. Allowlisted diagnostics: $safeSummary"
        }
        $testOutput += $suiteOutput
        if ($task075CatalogMeasurementRequested -and $Phase -eq 'initial') {
            $testOutput += @(
                Invoke-Task075CatalogMeasurements `
                    -Cargo $Cargo `
                    -Psql (Join-Path $postgresBin 'psql.exe') `
                    -RepositoryRoot $RepositoryRoot `
                    -Port $port `
                    -Password $oneTimePassword `
                    -RunId $runId `
                    -CurrentOnly:$MeasureTask075CurrentCatalog
            )
        }
        $storeProfile = Get-StoreProfileForLiveSuitePhase `
            -Phase $suitePhase `
            -SuiteName $suite.Name `
            -Task075MemoryGate:$RunTask075MemoryGate
        if ($null -ne $storeProfile -and -not $task075CatalogMeasurementRequested) {
            Invoke-StoreProfileLiveGate `
                -Cargo $Cargo `
                -RepositoryRoot $RepositoryRoot `
                -ExpectedProfile $storeProfile `
                -Port $port `
                -Password $oneTimePassword `
                -RunId $runId
            if (
                $MemoryOnly -and -not $RunTask075MemoryGate -and
                $suitePhase -eq 'initial' -and $suite.Name -eq 'memory'
            ) {
                $testOutput += @(
                    Invoke-WriterLeaseOwnerLiveGate `
                        -Cargo $Cargo `
                        -Psql (Join-Path $postgresBin 'psql.exe') `
                        -RepositoryRoot $RepositoryRoot `
                        -Port $port `
                        -Password $oneTimePassword `
                        -RunId $runId
                )
                Invoke-StoreProfileLiveGate `
                    -Cargo $Cargo `
                    -RepositoryRoot $RepositoryRoot `
                    -ExpectedProfile 'V3_MEMORY_V2_WRITER_LEASE_V1' `
                    -Port $port `
                    -Password $oneTimePassword `
                    -RunId $runId
            }
        }
    }
    if (
        $RunTask075MemoryGate -and
        -not (Test-Task075MemoryGateOutput -Phase $Phase -Output $testOutput)
    ) {
        throw "TASK075_MEMORY_GATE_${Phase}_OUTPUT_REJECTED"
    }
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PHASE', $Phase, 'Process')
    return ,$testOutput
}

function Invoke-Task075CatalogSignatureCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$EnvironmentName,
        [Parameter(Mandatory = $true)][string]$ConnectionUrl,
        [Parameter(Mandatory = $true)][string[]]$CargoArguments,
        [Parameter(Mandatory = $true)][string]$ProfileLabel,
        [switch]$RecordPartial
    )

    $originalValue = [Environment]::GetEnvironmentVariable($EnvironmentName, 'Process')
    $stdoutPath = Join-Path $clusterRoot ".cargo-task075-$ProfileLabel-stdout.log"
    $stderrPath = Join-Path $clusterRoot ".cargo-task075-$ProfileLabel-stderr.log"
    $process = $null
    $exitCode = $null
    $output = @()
    Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    try {
        [Environment]::SetEnvironmentVariable($EnvironmentName, $ConnectionUrl, 'Process')
        $process = Start-Process -FilePath $Cargo -ArgumentList $CargoArguments `
            -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $process.WaitForExit()
        $exitCode = $process.ExitCode
        if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
            $output += @(Get-Content -LiteralPath $stdoutPath -Encoding utf8)
        }
        if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
            $output += @(Get-Content -LiteralPath $stderrPath -Encoding utf8)
        }
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        [Environment]::SetEnvironmentVariable($EnvironmentName, $originalValue, 'Process')
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }
    if ($exitCode -ne 0) {
        $diagnostics = @(
            $output | ForEach-Object {
                foreach ($match in [regex]::Matches(
                    [string]$_,
                    'TASK075_CATALOG_DIAGNOSTIC_[A-Z_]+_STORE_[A-Z0-9_]+'
                )) {
                    $match.Value
                }
            } | Sort-Object -Unique
        )
        $safeDiagnostic = if ($diagnostics.Count -eq 1) {
            [string]$diagnostics[0]
        }
        else {
            'TASK075_CATALOG_DIAGNOSTIC_UNAVAILABLE'
        }
        $partialSignatures = @(
            $output | ForEach-Object {
                foreach ($match in [regex]::Matches(
                    [string]$_,
                    '(?:STORE|MEMORY)_CATALOG_[A-Z_]+_SIGNATURE=[a-f0-9]{64}'
                )) {
                    $match.Value
                }
            } | Sort-Object -Unique
        )
        if ($null -ne $holderReceipt -and -not [bool]$holderReceipt.closed) {
            Add-Task019HolderEvent `
                -Receipt $holderReceipt `
                -EventType 'CATALOG_DIAGNOSTIC_FAILED' `
                -Payload ([ordered]@{
                    profile = $ProfileLabel
                    diagnostic = $safeDiagnostic
                    signatures = $partialSignatures
                })
        }
        throw "TASK075_CATALOG_MEASUREMENT_FAILED_$ProfileLabel $safeDiagnostic"
    }
    $signatures = @(
        $output | ForEach-Object {
            foreach ($match in [regex]::Matches(
                [string]$_,
                '(?:STORE|MEMORY)_CATALOG_[A-Z_]+_SIGNATURE=[a-f0-9]{64}'
            )) {
                $match.Value
            }
        } | Sort-Object -Unique
    )
    $expectedSignatureCount = if ($EnvironmentName -eq 'LATTICE_STORE_CATALOG_SIGNATURE_URL') {
        9
    }
    else {
        8
    }
    if ($signatures.Count -ne $expectedSignatureCount) {
        throw "TASK075_CATALOG_MEASUREMENT_OUTPUT_REJECTED_$ProfileLabel"
    }
    $profileSignatures = @($signatures | ForEach-Object { "TASK075_${ProfileLabel}_$_" })
    if ($RecordPartial -and $null -ne $holderReceipt -and -not [bool]$holderReceipt.closed) {
        Add-Task019HolderEvent `
            -Receipt $holderReceipt `
            -EventType 'CATALOG_SIGNATURES_PARTIAL' `
            -Payload ([ordered]@{
                profile = $ProfileLabel
                source = $(if ($EnvironmentName -eq 'LATTICE_STORE_CATALOG_SIGNATURE_URL') {
                    'STORE'
                } else {
                    'MEMORY'
                })
                signatures = $profileSignatures
            })
    }
    return $profileSignatures
}

function Invoke-Task075CatalogMeasurements {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$RunId,
        [switch]$CurrentOnly
    )

    $encodedPassword = [Uri]::EscapeDataString($Password)
    $prefix = 'lattice_task019_' + $RunId.Substring(0, 8)
    $storeArguments = @(
        'test', '-p', 'lattice-postgres-store', '--lib',
        'postgres_setup::tests::measure_catalog_signatures', '--locked', '--',
        '--ignored', '--exact', '--nocapture', '--test-threads=1'
    )
    $memoryArguments = @(
        'test', '-p', 'lattice-postgres-codebase-memory', '--lib',
        'setup::tests::measure_catalog_signatures', '--locked', '--',
        '--ignored', '--exact', '--nocapture', '--test-threads=1'
    )
    $profiles = if ($CurrentOnly) {
        @([pscustomobject]@{ Label = 'V5_MEMORY_V3'; Database = "${prefix}_catalog_vthree" })
    }
    else {
        @(
            [pscustomobject]@{ Label = 'V5_BARE'; Database = "${prefix}_catalog_bare" },
            [pscustomobject]@{ Label = 'V5_MEMORY_V2'; Database = "${prefix}_catalog_vtwo" },
            [pscustomobject]@{ Label = 'V5_MEMORY_V3'; Database = "${prefix}_catalog_vthree" }
        )
    }
    $catalogDatabases = @($profiles | ForEach-Object { [string]$_.Database })
    $accessDatabase = "${prefix}_catalog_vthree"
    $results = @()
    foreach ($profile in $profiles) {
        $accessQuery = New-Task075CatalogDatabaseAccessQuery `
            -CurrentDatabase ([string]$profile.Database) `
            -TargetDatabases $catalogDatabases `
            -CurrentOnly:$CurrentOnly
        $null = Invoke-HarnessPsqlRows `
            -Psql $Psql `
            -DatabaseName $accessDatabase `
            -Port $Port `
            -Password $Password `
            -Query $accessQuery `
            -FailureCode 'TASK075_CATALOG_DATABASE_ACCESS_SWITCH_REJECTED'
        $accessDatabase = [string]$profile.Database
        $url = 'postgresql://task019_harness:{0}@127.0.0.1:{1}/{2}' -f `
            $encodedPassword, $Port, $profile.Database
        if ($CurrentOnly) {
            $results += @(
                Invoke-Task075CatalogSignatureCommand `
                    -Cargo $Cargo `
                    -RepositoryRoot $RepositoryRoot `
                    -EnvironmentName 'LATTICE_MEMORY_CATALOG_SIGNATURE_URL' `
                    -ConnectionUrl $url `
                    -CargoArguments $memoryArguments `
                    -ProfileLabel ([string]$profile.Label) `
                    -RecordPartial
            )
        }
        $results += @(
            Invoke-Task075CatalogSignatureCommand `
                -Cargo $Cargo `
                -RepositoryRoot $RepositoryRoot `
                -EnvironmentName 'LATTICE_STORE_CATALOG_SIGNATURE_URL' `
                -ConnectionUrl $url `
                -CargoArguments $storeArguments `
                -ProfileLabel ([string]$profile.Label) `
                -RecordPartial:$CurrentOnly
        )
        if (-not $CurrentOnly -and $profile.Label -ne 'V5_BARE') {
            $results += @(
                Invoke-Task075CatalogSignatureCommand `
                    -Cargo $Cargo `
                    -RepositoryRoot $RepositoryRoot `
                    -EnvironmentName 'LATTICE_MEMORY_CATALOG_SIGNATURE_URL' `
                    -ConnectionUrl $url `
                    -CargoArguments $memoryArguments `
                    -ProfileLabel ([string]$profile.Label)
            )
        }
    }
    return $results
}

function New-Task075CatalogDatabaseAccessQuery {
    param(
        [Parameter(Mandatory = $true)][string]$CurrentDatabase,
        [Parameter(Mandatory = $true)][string[]]$TargetDatabases,
        [switch]$CurrentOnly
    )

    $expectedPattern = '\Alattice_task019_[0-9a-f]{8}_catalog_(?:bare|vtwo|vthree)\z'
    $targets = @($TargetDatabases | Sort-Object -Unique)
    $expectedTargetCount = if ($CurrentOnly) { 1 } else { 3 }
    if (
        $targets.Count -ne $expectedTargetCount -or
        $CurrentDatabase -notin $targets -or
        @($targets | Where-Object { $_ -cnotmatch $expectedPattern }).Count -ne 0
    ) {
        throw 'TASK075_CATALOG_DATABASE_ACCESS_TARGET_REJECTED'
    }
    $quotedTargets = @($targets | ForEach-Object { '"' + $_ + '"' })
    $quotedCurrent = '"' + $CurrentDatabase + '"'
    return @"
SET ROLE lattice_migrator;
REVOKE ALL ON DATABASE $($quotedTargets -join ', ') FROM
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
GRANT CONNECT ON DATABASE $quotedCurrent TO
    lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly,
    lattice_migrator_login, lattice_runtime_login,
    lattice_guardian_login, lattice_readonly_login;
RESET ROLE;
"@
}

function Invoke-Task068HermesReplayGate {
    param(
        [Parameter(Mandatory = $true)][string]$Cargo,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][ValidateSet('initial', 'restart')][string]$Phase
    )

    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_PHASE', $Phase, 'Process')
    $stdoutPath = Join-Path $clusterRoot ".cargo-$Phase-task068-stdout.log"
    $stderrPath = Join-Path $clusterRoot ".cargo-$Phase-task068-stderr.log"
    $process = $null
    $testExitCode = $null
    $testOutput = @()
    Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
    try {
        $process = Start-Process -FilePath $Cargo -ArgumentList @(
            'test',
            '-p', 'lattice-runtime',
            '--lib',
            'composition::tests::canonical_hermes_reflection_survives_postgres_restart_when_provisioned',
            '--locked',
            '--',
            '--ignored',
            '--exact',
            '--nocapture',
            '--test-threads=1'
        ) -WorkingDirectory $RepositoryRoot -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $process.WaitForExit()
        $testExitCode = $process.ExitCode
        if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
            $testOutput += @(Get-Content -LiteralPath $stdoutPath -Encoding utf8)
        }
        if (Test-Path -LiteralPath $stderrPath -PathType Leaf) {
            $testOutput += @(Get-Content -LiteralPath $stderrPath -Encoding utf8)
        }
    }
    finally {
        if ($null -ne $process) {
            $process.Dispose()
        }
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $path) {
                throw 'TASK068_CARGO_OUTPUT_DELETE_FAILED'
            }
        }
    }

    if ($testExitCode -ne 0) {
        $safeTokens = @(
            $testOutput | ForEach-Object {
                foreach ($match in [regex]::Matches(
                    [string]$_,
                    '(?<![A-Z0-9_])(?:TASK068|MEMORY)_[A-Z0-9_]{1,63}(?![A-Z0-9_])'
                )) {
                    $match.Value
                }
            }
        )
        $safeTokens = @($safeTokens | Sort-Object -Unique)
        $safeSummary = if ($safeTokens.Count -eq 0) {
            'No allowlisted static diagnostic was emitted.'
        }
        else {
            $safeTokens -join ' | '
        }
        throw "TASK068 postgres replay $Phase phase failed with exit code $testExitCode. Allowlisted diagnostics: $safeSummary"
    }
    return ,$testOutput
}

function Get-Task068HermesReplayEvidence {
    param(
        [Parameter(Mandatory = $true)][object[]]$TestOutput,
        [Parameter(Mandatory = $true)][ValidateSet('initial', 'restart')][string]$Phase
    )

    $evidence = @()
    foreach ($item in $TestOutput) {
        $line = [string]$item
        if ($line -cmatch '(?:^|\s)TASK068_HERMES_POSTGRES_REPLAY_OK phase=(initial|restart) receipt_sha256=([0-9a-f]{64}) ready_calls=([01]) research_calls=([01]) persist_calls=([01])$') {
            $evidence += [pscustomobject]@{
                Phase = $Matches[1]
                ReceiptSha256 = $Matches[2]
                ReadyCalls = [int]$Matches[3]
                ResearchCalls = [int]$Matches[4]
                PersistCalls = [int]$Matches[5]
            }
        }
    }
    if ($evidence.Count -ne 1 -or $evidence[0].Phase -cne $Phase) {
        throw "TASK068_HERMES_REPLAY_EVIDENCE_MISSING_$Phase"
    }
    $expectedCalls = if ($Phase -ceq 'initial') { 1 } else { 0 }
    if (
        $evidence[0].ReadyCalls -ne $expectedCalls -or
        $evidence[0].ResearchCalls -ne $expectedCalls -or
        $evidence[0].PersistCalls -ne $expectedCalls
    ) {
        throw "TASK068_HERMES_REPLAY_EFFECT_COUNT_REJECTED_$Phase"
    }
    return $evidence[0]
}

function Get-RestartEvidence {
    param([Parameter(Mandatory = $true)][object[]]$TestOutput)

    $databaseId = $null
    $manifestHash = $null
    foreach ($item in $TestOutput) {
        $line = [string]$item
        # libtest may prefix the first uncaptured test line with `test <name> ... `.
        if ($line -match '(?:^|\s)TASK019_EVIDENCE database_uuid=([0-9a-f]{8}-[0-9a-f]{4}-8[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}) manifest_sha256=([0-9a-f]{64})$') {
            $databaseId = $Matches[1]
            $manifestHash = $Matches[2]
        }
    }

    if ($null -eq $databaseId -or $null -eq $manifestHash) {
        throw 'postgres_live initial phase did not emit the exact safe restart UUID/hash evidence.'
    }
    return [pscustomobject]@{
        DatabaseId = $databaseId
        ManifestHash = $manifestHash
    }
}

function Test-ClusterStopped {
    param(
        [Parameter(Mandatory = $true)][string]$PgCtl,
        [Parameter(Mandatory = $true)][string]$DataDirectory
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $null = & $PgCtl '-D' $DataDirectory 'status' 2>&1
        $statusExitCode = [int]$LASTEXITCODE
        return ($statusExitCode -eq 3)
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
}

function Wait-Task019ClusterStopped {
    param(
        [Parameter(Mandatory = $true)][string]$PgCtl,
        [Parameter(Mandatory = $true)][string]$DataDirectory
    )

    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        if (Test-ClusterStopped -PgCtl $PgCtl -DataDirectory $DataDirectory) {
            return $true
        }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

function Stop-TestCluster {
    param(
        [Parameter(Mandatory = $true)][string]$PgCtl,
        [Parameter(Mandatory = $true)][string]$DataDirectory
    )

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $null = & $PgCtl '-D' $DataDirectory '-m' 'fast' '-w' '-t' '30' 'stop' 2>&1
        $stopExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    return (Wait-Task019ClusterStopped -PgCtl $PgCtl -DataDirectory $DataDirectory)
}

function Remove-VerifiedSafeServerLog {
    param(
        [Parameter(Mandatory = $true)][string]$LogPath,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$OneTimePassword
    )

    if (-not (Test-Path -LiteralPath $LogPath -PathType Leaf)) {
        return
    }
    $content = Get-Content -LiteralPath $LogPath -Raw -Encoding utf8
    if ($null -eq $content) {
        $content = ''
    }
    $forbidden = @(
        $RepositoryRoot,
        $OneTimePassword,
        'intentional task019 rollback',
        'forbidden_table',
        'task019_ghost',
        'task019_unexpected',
        '11111111-1111-8111-8111-111111111111'
    )
    foreach ($value in $forbidden) {
        if (-not [string]::IsNullOrEmpty($value) -and
            $content.IndexOf($value, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
            try {
                [System.IO.File]::WriteAllBytes($LogPath, [byte[]]@())
            }
            catch {
                try {
                    Set-Content -LiteralPath $LogPath -Value $null -Encoding utf8 -NoNewline -ErrorAction Stop
                }
                catch {
                    throw 'TASK019_SERVER_LOG_SANITIZE_FAILED'
                }
            }
            if ((Get-Item -LiteralPath $LogPath -Force).Length -ne 0) {
                throw 'TASK019_SERVER_LOG_SANITIZE_FAILED'
            }
            try {
                Remove-Item -LiteralPath $LogPath -Force -ErrorAction Stop
            }
            catch {
                throw 'TASK019_SERVER_LOG_DELETE_FAILED'
            }
            if (Test-Path -LiteralPath $LogPath) {
                throw 'TASK019_SERVER_LOG_DELETE_FAILED'
            }
            throw 'TASK019_SERVER_LOG_REJECTED'
        }
    }
    Remove-Item -LiteralPath $LogPath -Force
    if (Test-Path -LiteralPath $LogPath) {
        throw 'TASK019_SERVER_LOG_DELETE_FAILED'
    }
}

function Remove-HarnessOutputFiles {
    param([Parameter(Mandatory = $true)][string]$Root)

    foreach ($outputPath in @(
        (Join-Path $Root '.native-stdout.log'),
        (Join-Path $Root '.native-stderr.log'),
        (Join-Path $Root '.cargo-initial-stdout.log'),
        (Join-Path $Root '.cargo-initial-stderr.log'),
        (Join-Path $Root '.cargo-restart-stdout.log'),
        (Join-Path $Root '.cargo-restart-stderr.log')
    )) {
        Remove-Item -LiteralPath $outputPath -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $outputPath) {
            throw 'TASK019_PROCESS_OUTPUT_DELETE_FAILED'
        }
    }
}

function Test-SafeCleanupTarget {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$ExpectedParent,
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)]$ContainmentSnapshot
    )

    if ($RunId -cnotmatch '\A[0-9a-f]{32}\z') {
        return $false
    }

    $canonicalRoot = Get-CanonicalPath -Path $Root
    $canonicalParent = Get-CanonicalPath -Path $ExpectedParent
    $canonicalRepositoryTarget = Get-CanonicalPath -Path $RepositoryTarget
    $expectedRoot = Get-CanonicalPath -Path (Join-Path $canonicalParent $RunId)
    if (-not (Test-ExactPath -Actual $canonicalRoot -Expected $expectedRoot)) {
        return $false
    }
    if (-not (Test-ExactPath -Actual (Split-Path -Parent $canonicalRoot) -Expected $canonicalParent)) {
        return $false
    }

    $targetPrefix = $canonicalRepositoryTarget + [System.IO.Path]::DirectorySeparatorChar
    if (-not $canonicalRoot.StartsWith($targetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }

    $rootItem = Get-Item -LiteralPath $canonicalRoot -Force -ErrorAction SilentlyContinue
    if ($null -eq $rootItem -or ($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
        return $false
    }

    $parentItem = Get-Item -LiteralPath $canonicalParent -Force -ErrorAction SilentlyContinue
    $targetItem = Get-Item -LiteralPath $canonicalRepositoryTarget -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $parentItem -or
        $null -eq $targetItem -or
        ($parentItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        ($targetItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
    ) {
        return $false
    }

    $markerPath = Join-Path $canonicalRoot $markerName
    $markerItem = Get-Item -LiteralPath $markerPath -Force -ErrorAction SilentlyContinue
    if ($null -eq $markerItem -or ($markerItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
        return $false
    }

    if (
        -not (Test-ExactPath -Actual ([string]$ContainmentSnapshot.parent_path) -Expected $canonicalParent) -or
        -not (Test-ExactPath -Actual ([string]$ContainmentSnapshot.root_path) -Expected $canonicalRoot) -or
        -not (Test-ExactPath -Actual ([string]$ContainmentSnapshot.marker_path) -Expected $markerPath) -or
        -not (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $ContainmentSnapshot)
    ) {
        return $false
    }

    try {
        $marker = Get-Content -LiteralPath $markerPath -Raw -Encoding utf8 | ConvertFrom-Json
    }
    catch {
        return $false
    }

    $requiredMarkerProperties = @('kind', 'run_id', 'root', 'parent', 'repository_target')
    foreach ($propertyName in $requiredMarkerProperties) {
        if ($propertyName -notin $marker.PSObject.Properties.Name) {
            return $false
        }
    }

    return (
        [string]$marker.kind -eq 'LATTICE_TASK019_DISPOSABLE_POSTGRES_V1' -and
        [string]$marker.run_id -eq $RunId -and
        (Test-ExactPath -Actual ([string]$marker.root) -Expected $canonicalRoot) -and
        (Test-ExactPath -Actual ([string]$marker.parent) -Expected $canonicalParent) -and
        (Test-ExactPath -Actual ([string]$marker.repository_target) -Expected $canonicalRepositoryTarget)
    )
}

$repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
$repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
$clusterParent = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'task019-postgres')
$runId = [Guid]::NewGuid().ToString('N')
$clusterRoot = Get-CanonicalPath -Path (Join-Path $clusterParent $runId)
$dataDirectory = Join-Path $clusterRoot 'data'
$passwordFile = Join-Path $clusterRoot '.initdb-password'
$serverLog = Join-Path $clusterRoot 'postgres.log'
$port = Get-UnreservedLoopbackPort
$oneTimePassword = $null
$clusterStarted = $false
$harnessCompleted = $false
$writerLeaseOwnerProfileProved = $false
$installedBefore = $null
$installedAfter = $null
$originalEnvironment = @{}
$cleanupContainment = $null
$fullSerializationMutex = $null
$fullSerializationMutexOwned = $false
$deliveryHookPath = $null
$fullChainHookPath = $null
$task038HookPath = $null
$task038TunnelHookPath = $null
$holderReceipt = $null
$holderFinalEvidence = $null
$holderConsumerStarted = $false
$holderConsumerExited = $false
$postgresProcessIdentity = $null
$restartedPostgresIdentity = $null

$selectedHookCount = @(
    $RunLatticeDeliveryHook,
    $RunFullChainAcceptanceHook,
    $RunTask038AcceptanceHook,
    $RunTask038TunnelHook
) |
    Where-Object { [bool]$_ }
if (@($selectedHookCount).Count -gt 1) {
    throw 'TASK019_HOOK_MODE_REJECTED'
}
if (
    $RunTask068HermesReplayGate -and
    (-not $MemoryOnly -or @($selectedHookCount).Count -ne 0)
) {
    throw 'TASK068_HARNESS_MODE_REJECTED'
}
if ($RunLatticeDeliveryHook) {
    $deliveryHookPath = Get-LatticeDeliveryHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
}
if ($RunFullChainAcceptanceHook) {
    $fullChainHookPath = Get-LatticeFullChainAcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
}
if ($RunTask038AcceptanceHook) {
    if ([string]::IsNullOrWhiteSpace($Task038OfficialCodexExecutable) -or [string]::IsNullOrWhiteSpace($Task038CodexAuthHome)) {
        throw 'TASK038_ACCEPTANCE_INPUT_REJECTED'
    }
    $task038HookPath = Get-LatticeTask038AcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
}

function Get-Task019PostgresProcessIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$RunId
    )

    if ($RunId -cnotmatch '\A[0-9a-f]{32}\z') {
        throw 'TASK019_POSTGRES_PROCESS_IDENTITY_REJECTED'
    }
    $databaseName = 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
    $originalPassword = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $previousErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $rows = @(& $Psql `
                '--no-psqlrc' '--no-password' '--quiet' '--tuples-only' '--no-align' `
                '--field-separator' '|' '-h' '127.0.0.1' '-p' ([string]$Port) `
                '-U' $harnessUser '-d' 'postgres' '-v' 'ON_ERROR_STOP=1' `
                '-c' "SELECT system_identifier::text, pg_postmaster_start_time()::text, current_setting('data_directory') FROM pg_control_system();" `
                2>&1 | ForEach-Object { [string]$_ })
            $exitCode = [int]$LASTEXITCODE
        }
        finally {
            $ErrorActionPreference = $previousErrorActionPreference
        }
    }
    finally {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $originalPassword, 'Process')
    }
    $rows = @($rows | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if (
        $exitCode -ne 0 -or
        $rows.Count -ne 1 -or
        [string]$rows[0] -cnotmatch '\A([0-9]{1,20})\|([^|]+)\|([^|]+)\z'
    ) {
        throw 'TASK019_POSTGRES_PROCESS_IDENTITY_REJECTED'
    }
    return [pscustomobject]@{
        system_identifier = [string]$Matches[1]
        postmaster_started_at = [string]$Matches[2]
        data_directory = [string]$Matches[3]
    }
}

function Get-Task019PostmasterRuntimeEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Psql,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$DataDirectory,
        [Parameter(Mandatory = $true)][string]$PostgresExecutable,
        [Parameter(Mandatory = $true)][string]$ExpectedNativeIdentity,
        [Parameter(Mandatory = $true)][string]$ExpectedSha256
    )

    $identity = Get-Task019PostgresProcessIdentity -Psql $Psql -Port $Port -Password $Password -RunId $RunId
    try {
        $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction Stop | Where-Object {
            [string]$_.LocalAddress -in @('127.0.0.1', '::ffff:127.0.0.1')
        })
    }
    catch { throw 'TASK019_POSTMASTER_LISTENER_REJECTED' }
    if ($listeners.Count -ne 1 -or [int]$listeners[0].OwningProcess -lt 1) {
        throw 'TASK019_POSTMASTER_LISTENER_REJECTED'
    }
    try {
        $processId = [int]$listeners[0].OwningProcess
        $process = Get-CimInstance -ClassName Win32_Process -Filter ('ProcessId = ' + $processId) -ErrorAction Stop
        if ($null -eq $process) {
            throw 'TASK019_POSTMASTER_PROCESS_MISSING'
        }
        $executable = Get-CanonicalPath -Path ([string]$process.ExecutablePath)
        $createdAt = ([DateTimeOffset]([DateTime]$process.CreationDate)).ToUniversalTime()
        if (-not (Test-ExactPath -Actual $executable -Expected $PostgresExecutable)) {
            throw 'TASK019_POSTMASTER_PROCESS_PATH_REJECTED'
        }
        if (-not (Test-ExactPath -Actual ([string]$identity.data_directory) -Expected $DataDirectory)) {
            throw 'TASK019_POSTMASTER_PROCESS_ARGUMENT_REJECTED'
        }
        if ((Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ExpectedSha256) {
            throw 'TASK019_POSTMASTER_PROCESS_HASH_REJECTED'
        }
        if ((Get-LatticeWindowsNativePathIdentityToken -Path $executable -Directory $false) -cne $ExpectedNativeIdentity) {
            throw 'TASK019_POSTMASTER_PROCESS_NATIVE_IDENTITY_REJECTED'
        }
    }
    catch {
        if ([string]$_.Exception.Message -cmatch '\ATASK019_POSTMASTER_PROCESS_[A-Z_]+\z') {
            throw [string]$_.Exception.Message
        }
        throw 'TASK019_POSTMASTER_PROCESS_IDENTITY_REJECTED'
    }
    return [pscustomobject][ordered]@{
        system_identifier = [string]$identity.system_identifier
        postmaster_started_at = [string]$identity.postmaster_started_at
        listener_process_id = [long]$processId
        listener_process_creation_time = $createdAt.ToFileTime().ToString()
        listener_process_creation_time_source = 'WINDOWS_PROCESS_TIMES'
        listener_process_created_at_utc = $createdAt.ToString('o')
        listener_executable_path = $executable
        listener_executable_sha256 = $ExpectedSha256
        listener_executable_native_identity = $ExpectedNativeIdentity
        listener_data_directory = Get-CanonicalPath -Path $DataDirectory
        listener_host = '127.0.0.1'
        listener_port = [long]$Port
    }
}

function New-Task019HolderReceipt {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$ClusterRoot,
        [Parameter(Mandatory = $true)][string]$DataDirectory,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][int]$TtlSeconds,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$ToolIdentity
    )

    if ($RunId -cnotmatch '\A[0-9a-f]{32}\z' -or $Port -in @(5432, 64272, 55432)) {
        throw 'TASK019_HOLDER_CONFIG_REJECTED'
    }
    $root = Get-CanonicalPath -Path (Join-Path $RepositoryTarget 'task019-holder-receipts')
    if (-not (Test-Path -LiteralPath $root -PathType Container)) {
        New-Item -ItemType Directory -Path $root -Force:$false | Out-Null
    }
    Assert-NoReparseAncestor -Path $root -Boundary (Split-Path -Parent $RepositoryTarget)
    Set-Task019OwnerOnlyAcl -Path $root -Directory $true
    $path = Get-CanonicalPath -Path (Join-Path $root ($RunId + '.jsonl'))
    $stream = $null
    try {
        $stream = [IO.File]::Open(
            $path,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::ReadWrite
        )
        Set-Task019OwnerOnlyAcl -Path $path -Directory $false
        $nonce = [Guid]::NewGuid().ToString('N') + [Guid]::NewGuid().ToString('N')
        $consumerSessionId = [Guid]::NewGuid().ToString('N')
        $sessionId = [Guid]::NewGuid().ToString('N')
        $createdAt = [DateTimeOffset]::UtcNow
        $deadline = $createdAt.AddSeconds($TtlSeconds)
        $owner = Get-Process -Id $PID -ErrorAction Stop
        $ownerExecutable = Get-CanonicalPath -Path ([string]$owner.Path)
        $receipt = [pscustomobject][ordered]@{
            stream = $stream
            path = $path
            native_identity = Get-LatticeWindowsNativePathIdentityToken -Path $path -Directory $false
            nonce = $nonce
            nonce_commitment = Get-StringSha256 -Value (@(
                'lattice.task019.postgres-holder-nonce.v1', $sessionId, $consumerSessionId,
                $RunId, [string]$Port, $nonce
            ) -join "`n")
            session_id = $sessionId
            consumer_session_id = $consumerSessionId
            run_id = $RunId
            cluster_root = Get-CanonicalPath -Path $ClusterRoot
            data_directory = Get-CanonicalPath -Path $DataDirectory
            port = [long]$Port
            excluded_ports = @(5432, 64272, 55432)
            created_at_utc = $createdAt.ToString('o')
            deadline_utc = $deadline.ToString('o')
            ordinal = 0L
            previous_hmac_sha256 = '0' * 64
            tool_identity = $ToolIdentity
            owner_process_id = [long]$PID
            owner_process_creation_time = $owner.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()
            owner_process_executable = $ownerExecutable
            owner_process_executable_sha256 = (Get-FileHash -LiteralPath $ownerExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
            owner_process_executable_native_identity = Get-LatticeWindowsNativePathIdentityToken -Path $ownerExecutable -Directory $false
            closed = $false
        }
        Add-Task019HolderEvent -Receipt $receipt -EventType 'HOLDER_OPEN' -Payload ([ordered]@{
            owner_process_id = [long]$receipt.owner_process_id
            owner_process_creation_time = [string]$receipt.owner_process_creation_time
            owner_process_creation_time_source = 'WINDOWS_PROCESS_TIMES'
            owner_process_executable = [string]$receipt.owner_process_executable
            owner_process_executable_sha256 = [string]$receipt.owner_process_executable_sha256
            owner_process_executable_native_identity = [string]$receipt.owner_process_executable_native_identity
            cluster_root = [string]$receipt.cluster_root
            data_directory = [string]$receipt.data_directory
            host = '127.0.0.1'
            port = [long]$receipt.port
            excluded_ports = @($receipt.excluded_ports)
            ttl_seconds = [long]$TtlSeconds
            deadline_utc = [string]$receipt.deadline_utc
            tool_identity = $receipt.tool_identity
            authority_receipt_path = [string]$receipt.path
            authority_receipt_native_identity = [string]$receipt.native_identity
        })
        return $receipt
    }
    catch {
        if ($null -ne $stream) { $stream.Dispose() }
        throw 'TASK019_HOLDER_RECEIPT_REJECTED'
    }
}

function Add-Task019HolderEvent {
    param(
        [Parameter(Mandatory = $true)]$Receipt,
        [Parameter(Mandatory = $true)][ValidateSet(
            'HOLDER_OPEN', 'MARKER_CREATED', 'INITIAL_POSTMASTER_READY',
            'INITIAL_POSTMASTER_STOPPED', 'RESTART_POSTMASTER_READY',
            'CONSUMER_STARTED', 'CONSUMER_EXITED', 'HOLDER_STOP_REQUESTED',
            'HOLDER_STOPPED', 'CATALOG_SIGNATURES_MEASURED', 'CATALOG_SIGNATURES_PARTIAL',
            'CATALOG_DIAGNOSTIC_FAILED', 'LIVE_GATE_FAILED',
            'CLEANUP_REQUESTED',
            'CLEANUP_COMPLETED', 'RECEIPT_CLOSED'
        )][string]$EventType,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Payload
    )

    if (
        [bool]$Receipt.closed -or $null -eq $Receipt.stream -or -not $Receipt.stream.CanWrite -or
        -not (Test-LatticeWindowsNativePathIdentity -Path ([string]$Receipt.path) -Directory $false -ExpectedToken ([string]$Receipt.native_identity))
    ) { throw 'TASK019_HOLDER_RECEIPT_REJECTED' }
    if ($EventType -in @('RESTART_POSTMASTER_READY', 'CONSUMER_STARTED') -and [DateTimeOffset]::UtcNow -gt [DateTimeOffset]::Parse([string]$Receipt.deadline_utc)) {
        throw 'TASK019_HOLDER_TTL_EXPIRED'
    }
    $Receipt.ordinal = [long]$Receipt.ordinal + 1L
    $observedAt = [DateTimeOffset]::UtcNow.ToString('o')
    $payloadJson = $Payload | ConvertTo-Json -Compress -Depth 20
    $payloadSha256 = Get-StringSha256 -Value $payloadJson
    $hmacInput = @(
        'lattice.task019.postgres-holder-hmac.v1', [string]$Receipt.previous_hmac_sha256,
        [string]$Receipt.session_id, [string]$Receipt.consumer_session_id,
        [string]$Receipt.run_id, [string]$Receipt.port, [string]$Receipt.nonce_commitment,
        [string]$Receipt.ordinal, $EventType, $observedAt, $payloadSha256
    ) -join "`n"
    $eventHmac = Get-Task019HmacSha256 -Key ([string]$Receipt.nonce) -Value $hmacInput
    $record = [ordered]@{
        schema = 'lattice.task019.postgres-holder-authority.v1'
        event_type = $EventType
        session_id = [string]$Receipt.session_id
        consumer_session_id = [string]$Receipt.consumer_session_id
        run_id = [string]$Receipt.run_id
        host = '127.0.0.1'
        port = [long]$Receipt.port
        excluded_ports = @($Receipt.excluded_ports)
        deadline_utc = [string]$Receipt.deadline_utc
        nonce_commitment = [string]$Receipt.nonce_commitment
        ordinal = [long]$Receipt.ordinal
        observed_at_utc = $observedAt
        payload = $Payload
        payload_sha256 = $payloadSha256
        previous_hmac_sha256 = [string]$Receipt.previous_hmac_sha256
        event_hmac_sha256 = $eventHmac
    }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes((($record | ConvertTo-Json -Compress -Depth 24) + "`n"))
    $Receipt.stream.Write($bytes, 0, $bytes.Length)
    $Receipt.stream.Flush($true)
    $Receipt.previous_hmac_sha256 = $eventHmac
}

function Close-Task019HolderReceipt {
    param([Parameter(Mandatory = $true)]$Receipt)

    if (-not [bool]$Receipt.closed) {
        Add-Task019HolderEvent -Receipt $Receipt -EventType 'RECEIPT_CLOSED' -Payload ([ordered]@{
            final_event_count_before_close = [long]$Receipt.ordinal
            cleanup_complete = $true
        })
        $Receipt.closed = $true
        $Receipt.stream.Flush($true)
        $Receipt.stream.Dispose()
        $Receipt.nonce = $null
    }
    return [pscustomobject][ordered]@{
        path = [string]$Receipt.path
        native_identity = [string]$Receipt.native_identity
        raw_sha256 = (Get-FileHash -LiteralPath ([string]$Receipt.path) -Algorithm SHA256).Hash.ToLowerInvariant()
        byte_count = [long](Get-Item -LiteralPath ([string]$Receipt.path)).Length
        event_count = [long]$Receipt.ordinal
        final_hmac_sha256 = [string]$Receipt.previous_hmac_sha256
        session_id = [string]$Receipt.session_id
        consumer_session_id = [string]$Receipt.consumer_session_id
        nonce_commitment = [string]$Receipt.nonce_commitment
        deadline_utc = [string]$Receipt.deadline_utc
        authority_scope = 'LIVE_HOLDER_PROCESS_PRIVATE_HMAC'
    }
}

function Write-Task019IdentityMarker {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    $bytes = [Text.UTF8Encoding]::new($false).GetBytes((($Value | ConvertTo-Json -Depth 8) + "`n"))
    $stream = $null
    try {
        $stream = [IO.File]::Open($Path, [IO.FileMode]::Truncate, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    }
    catch {
        throw 'TASK019_POSTGRES_IDENTITY_MARKER_REJECTED'
    }
    finally {
        if ($null -ne $stream) { $stream.Dispose() }
    }
}
if ($RunTask038TunnelHook) {
    if (
        [string]::IsNullOrWhiteSpace($Task038TunnelClientExecutable) -or
        [string]::IsNullOrWhiteSpace($Task038TunnelProfileDirectory)
    ) {
        throw 'TASK038_TUNNEL_RUNTIME_INPUT_REJECTED'
    }
    foreach ($name in @(
        'CONTROL_PLANE_API_KEY',
        'LATTICE_DELIVERY_LAUNCHER',
        'LATTICE_DELIVERY_LAUNCHER_VERSION',
        'LATTICE_DELIVERY_LAUNCHER_SHA256',
        'LATTICE_DELIVERY_SCHEMA_DIR',
        'LATTICE_DELIVERY_CODEX_HOME',
        'LATTICE_DELIVERY_ROOT',
        'LATTICE_DELIVERY_GIT_EXE'
    )) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (
            [string]::IsNullOrWhiteSpace($value) -or
            $value.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0
        ) {
            throw 'TASK038_TUNNEL_RUNTIME_INPUT_REJECTED'
        }
    }
    $task038TunnelHookPath = Get-LatticeTask038TunnelHookPath `
        -ScriptDirectory $PSScriptRoot `
        -RepositoryRoot $repositoryRoot
}

Invoke-HarnessSelfTest
if ($SelfTestOnly) {
    return
}
Assert-NoReparseAncestor -Path $clusterRoot -Boundary $repositoryRoot

foreach ($name in $environmentNames) {
    $originalEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

foreach ($executable in $requiredExecutables) {
    $path = Join-Path $postgresBin $executable
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required PostgreSQL 17.10 executable is missing: $executable"
    }
}

$initdb = Join-Path $postgresBin 'initdb.exe'
$pgCtl = Join-Path $postgresBin 'pg_ctl.exe'
$pgIsReady = Join-Path $postgresBin 'pg_isready.exe'
$postgres = Join-Path $postgresBin 'postgres.exe'
$psql = Join-Path $postgresBin 'psql.exe'
$cargoCommand = Get-Command 'cargo.exe' -ErrorAction Stop

$postgresExecutableNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $postgres -Directory $false
$psqlExecutableNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $psql -Directory $false
$pgCtlExecutableNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $pgCtl -Directory $false
if (
    (Get-FileHash -LiteralPath $postgres -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedPostgresExecutableSha256 -or
    (Get-FileHash -LiteralPath $psql -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedPsqlExecutableSha256 -or
    (Get-FileHash -LiteralPath $pgCtl -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedPgCtlExecutableSha256
) {
    throw 'TASK019_POSTGRES_EXECUTABLE_IDENTITY_REJECTED'
}

$previousErrorActionPreference = $ErrorActionPreference
try {
    $ErrorActionPreference = 'Continue'
    $versionOutput = @(& $postgres '--version' 2>&1)
    $versionExitCode = $LASTEXITCODE
}
finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
if ($versionExitCode -ne 0 -or (($versionOutput -join "`n") -notmatch "postgres \(PostgreSQL\) $([regex]::Escape($expectedPostgresVersion))(?:\s|$)")) {
    throw "The harness requires PostgreSQL $expectedPostgresVersion exactly."
}

$installedBefore = Get-InstalledPostgresSnapshot -PgIsReady $pgIsReady

try {
    if (@($selectedHookCount).Count -eq 1) {
        $fullSerializationMutex = [Threading.Mutex]::new($false, 'Local\Lattice.Task019.SerializedFull.v1')
        try {
            $fullSerializationMutexOwned = $fullSerializationMutex.WaitOne(0)
        }
        catch [Threading.AbandonedMutexException] {
            $fullSerializationMutexOwned = $true
        }
        if (-not $fullSerializationMutexOwned) {
            throw 'TASK019_FULL_SERIALIZATION_REJECTED'
        }
    }

    New-Item -ItemType Directory -Path $clusterRoot -Force:$false | Out-Null
    Assert-NoReparseAncestor -Path $clusterRoot -Boundary $repositoryRoot
    $holderReceipt = New-Task019HolderReceipt `
        -RepositoryTarget $repositoryTarget `
        -RunId $runId `
        -ClusterRoot $clusterRoot `
        -DataDirectory $dataDirectory `
        -Port $port `
        -TtlSeconds $HolderTtlSeconds `
        -ToolIdentity ([ordered]@{
            postgres_version = $expectedPostgresVersion
            postgres_path = $postgres
            postgres_sha256 = $expectedPostgresExecutableSha256
            postgres_native_identity = $postgresExecutableNativeIdentity
            psql_path = $psql
            psql_sha256 = $expectedPsqlExecutableSha256
            psql_native_identity = $psqlExecutableNativeIdentity
            pg_ctl_path = $pgCtl
            pg_ctl_sha256 = $expectedPgCtlExecutableSha256
            pg_ctl_native_identity = $pgCtlExecutableNativeIdentity
        })
    $marker = [ordered]@{
        kind = 'LATTICE_TASK019_DISPOSABLE_POSTGRES_V1'
        run_id = $runId
        created_at_utc = [DateTime]::UtcNow.ToString('o')
        root = $clusterRoot
        parent = $clusterParent
        repository_target = $repositoryTarget
        postgres_version = $expectedPostgresVersion
        host = '127.0.0.1'
        port = $port
        excluded_ports = @(5432, 64272, 55432)
        identity_materialized = $false
    }
    $markerPath = Join-Path $clusterRoot $markerName
    $markerBytes = [Text.UTF8Encoding]::new($false).GetBytes((($marker | ConvertTo-Json -Depth 4) + "`n"))
    [IO.File]::WriteAllBytes($markerPath, $markerBytes)
    $cleanupContainment = New-LatticeWindowsNativeContainmentSnapshot `
        -ParentPath $clusterParent `
        -RootPath $clusterRoot `
        -MarkerPath $markerPath
    Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'MARKER_CREATED' -Payload ([ordered]@{
        marker_path = $markerPath
        marker_native_identity = Get-LatticeWindowsNativePathIdentityToken -Path $markerPath -Directory $false
        marker_raw_sha256 = (Get-FileHash -LiteralPath $markerPath -Algorithm SHA256).Hash.ToLowerInvariant()
        marker_created_at_utc = [string]$marker.created_at_utc
        marker_identity_materialized = $false
    })

    $oneTimePassword = New-OneTimePassword
    try {
        Set-Content -LiteralPath $passwordFile -Value $oneTimePassword -Encoding ascii -NoNewline
        $null = Invoke-NativeChecked -Executable $initdb -Arguments @(
            '--pgdata', $dataDirectory,
            '--encoding', 'UTF8',
            '--locale', 'C',
            '--data-checksums',
            '--username', $harnessUser,
            '--pwfile', $passwordFile,
            '--auth-host', 'scram-sha-256',
            '--auth-local', 'scram-sha-256'
        ) -Operation 'initdb'
    }
    finally {
        if (Test-Path -LiteralPath $passwordFile) {
            Remove-Item -LiteralPath $passwordFile -Force
        }
    }

    @(
        "listen_addresses = '127.0.0.1'"
        "port = $port"
        'ssl = off'
        'fsync = on'
        'synchronous_commit = on'
        'full_page_writes = on'
        'max_prepared_transactions = 0'
        'password_encryption = scram-sha-256'
        'logging_collector = off'
        "log_min_messages = 'panic'"
        "log_min_error_statement = 'panic'"
        'log_parameter_max_length_on_error = 0'
        "log_error_verbosity = 'terse'"
        "log_connections = off"
        "log_disconnections = off"
        "log_statement = 'none'"
    ) | Add-Content -LiteralPath (Join-Path $dataDirectory 'postgresql.conf') -Encoding ascii

    $clusterStarted = $true
    $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
        '-D', $dataDirectory,
        '-l', $serverLog,
        '-w',
        '-t', '30',
        'start'
    ) -Operation 'PostgreSQL test-cluster start'
    Set-HarnessEnvironment -Phase 'initial' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
    foreach ($entry in @(
        @($postgres, $postgresExecutableNativeIdentity, $expectedPostgresExecutableSha256),
        @($psql, $psqlExecutableNativeIdentity, $expectedPsqlExecutableSha256),
        @($pgCtl, $pgCtlExecutableNativeIdentity, $expectedPgCtlExecutableSha256)
    )) {
        if (
            -not (Test-LatticeWindowsNativePathIdentity `
                -Path ([string]$entry[0]) `
                -Directory $false `
                -ExpectedToken ([string]$entry[1])) -or
            (Get-FileHash -LiteralPath ([string]$entry[0]) -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$entry[2]
        ) {
            throw 'TASK019_POSTGRES_EXECUTABLE_IDENTITY_REJECTED'
        }
    }
    $postgresProcessIdentity = Get-Task019PostmasterRuntimeEvidence `
        -Psql $psql `
        -Port $port `
        -Password $oneTimePassword `
        -RunId $runId `
        -DataDirectory $dataDirectory `
        -PostgresExecutable $postgres `
        -ExpectedNativeIdentity $postgresExecutableNativeIdentity `
        -ExpectedSha256 $expectedPostgresExecutableSha256
    $marker['identity_materialized'] = $true
    $marker['system_identifier'] = [string]$postgresProcessIdentity.system_identifier
    $marker['initial_postmaster_started_at'] = [string]$postgresProcessIdentity.postmaster_started_at
    $marker['data_native_identity'] = Get-LatticeWindowsNativePathIdentityToken -Path $dataDirectory -Directory $true
    $marker['postgres_executable_path'] = $postgres
    $marker['postgres_executable_raw_sha256'] = $expectedPostgresExecutableSha256
    $marker['postgres_executable_native_identity'] = $postgresExecutableNativeIdentity
    $marker['psql_executable_path'] = $psql
    $marker['psql_executable_raw_sha256'] = $expectedPsqlExecutableSha256
    $marker['psql_executable_native_identity'] = $psqlExecutableNativeIdentity
    $marker['pg_ctl_executable_path'] = $pgCtl
    $marker['pg_ctl_executable_raw_sha256'] = $expectedPgCtlExecutableSha256
    $marker['pg_ctl_executable_native_identity'] = $pgCtlExecutableNativeIdentity
    Write-Task019IdentityMarker -Path $markerPath -Value $marker
    if (-not (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $cleanupContainment)) {
        throw 'TASK019_POSTGRES_IDENTITY_MARKER_REJECTED'
    }
    Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'INITIAL_POSTMASTER_READY' -Payload ([ordered]@{
        system_identifier = [string]$postgresProcessIdentity.system_identifier
        postmaster_started_at = [string]$postgresProcessIdentity.postmaster_started_at
        listener_process_id = [long]$postgresProcessIdentity.listener_process_id
        listener_process_creation_time = [string]$postgresProcessIdentity.listener_process_creation_time
        listener_process_creation_time_source = [string]$postgresProcessIdentity.listener_process_creation_time_source
        listener_process_created_at_utc = [string]$postgresProcessIdentity.listener_process_created_at_utc
        listener_executable_path = [string]$postgresProcessIdentity.listener_executable_path
        listener_executable_sha256 = [string]$postgresProcessIdentity.listener_executable_sha256
        listener_executable_native_identity = [string]$postgresProcessIdentity.listener_executable_native_identity
        listener_data_directory = [string]$postgresProcessIdentity.listener_data_directory
        listener_host = [string]$postgresProcessIdentity.listener_host
        listener_port = [long]$postgresProcessIdentity.listener_port
        data_native_identity = [string]$marker.data_native_identity
    })
    $initialOutput = Invoke-LiveTest -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -Phase 'initial'
    $restartEvidence = Get-RestartEvidence -TestOutput $initialOutput
    if ($task075CatalogMeasurementRequested) {
        $catalogSignatures = @($initialOutput | Where-Object {
            [string]$_ -cmatch '^TASK075_[A-Z0-9_]+_(?:STORE|MEMORY)_CATALOG_[A-Z_]+_SIGNATURE=[a-f0-9]{64}$'
        })
        if (-not (Test-Task075CatalogMeasurementShape `
                -Output $catalogSignatures `
                -CurrentOnly:$MeasureTask075CurrentCatalog)) {
            throw 'TASK075_CATALOG_MEASUREMENT_SHAPE_REJECTED'
        }
        Add-Task019HolderEvent `
            -Receipt $holderReceipt `
            -EventType 'CATALOG_SIGNATURES_MEASURED' `
            -Payload ([ordered]@{ signatures = $catalogSignatures })
        $catalogSignatures | Write-Output
    }
    $task068InitialEvidence = $null
    if ($RunTask068HermesReplayGate) {
        $task068InitialOutput = Invoke-Task068HermesReplayGate `
            -Cargo $cargoCommand.Source `
            -RepositoryRoot $repositoryRoot `
            -Phase 'initial'
        $task068InitialEvidence = Get-Task068HermesReplayEvidence `
            -TestOutput $task068InitialOutput `
            -Phase 'initial'
        [Environment]::SetEnvironmentVariable(
            'LATTICE_TASK068_EXPECTED_RECEIPT_SHA256',
            [string]$task068InitialEvidence.ReceiptSha256,
            'Process'
        )
    }

    if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
        throw 'Could not prove the disposable PostgreSQL cluster stopped after the initial phase.'
    }
    $clusterStarted = $false
    if (@(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue).Count -ne 0) {
        throw 'TASK019_POSTMASTER_LISTENER_STILL_PRESENT'
    }
    Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'INITIAL_POSTMASTER_STOPPED' -Payload ([ordered]@{
        initial_listener_process_id = [long]$postgresProcessIdentity.listener_process_id
        pg_ctl_status_stopped = $true
        port_listener_absent = $true
    })
    Remove-HarnessOutputFiles -Root $clusterRoot
    Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword

    $clusterStarted = $true
    $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
        '-D', $dataDirectory,
        '-l', $serverLog,
        '-w',
        '-t', '30',
        'start'
    ) -Operation 'PostgreSQL test-cluster restart'
    Set-HarnessEnvironment -Phase 'restart' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
    $restartedPostgresIdentity = Get-Task019PostmasterRuntimeEvidence `
        -Psql $psql `
        -Port $port `
        -Password $oneTimePassword `
        -RunId $runId `
        -DataDirectory $dataDirectory `
        -PostgresExecutable $postgres `
        -ExpectedNativeIdentity $postgresExecutableNativeIdentity `
        -ExpectedSha256 $expectedPostgresExecutableSha256
    if (
        [string]$restartedPostgresIdentity.system_identifier -cne [string]$marker.system_identifier -or
        [string]$restartedPostgresIdentity.postmaster_started_at -ceq [string]$marker.initial_postmaster_started_at -or
        (
            [long]$restartedPostgresIdentity.listener_process_id -eq [long]$postgresProcessIdentity.listener_process_id -and
            [string]$restartedPostgresIdentity.listener_process_creation_time -ceq [string]$postgresProcessIdentity.listener_process_creation_time
        )
    ) {
        throw 'TASK019_POSTGRES_RESTART_IDENTITY_REJECTED'
    }
    $marker['restart_postmaster_started_at'] = [string]$restartedPostgresIdentity.postmaster_started_at
    $marker['restart_identity_verified'] = $true
    Write-Task019IdentityMarker -Path $markerPath -Value $marker
    if (-not (Test-LatticeWindowsNativeContainmentSnapshot -Snapshot $cleanupContainment)) {
        throw 'TASK019_POSTGRES_IDENTITY_MARKER_REJECTED'
    }
    Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'RESTART_POSTMASTER_READY' -Payload ([ordered]@{
        system_identifier = [string]$restartedPostgresIdentity.system_identifier
        initial_postmaster_started_at = [string]$postgresProcessIdentity.postmaster_started_at
        restart_postmaster_started_at = [string]$restartedPostgresIdentity.postmaster_started_at
        initial_listener_process_id = [long]$postgresProcessIdentity.listener_process_id
        initial_listener_process_creation_time = [string]$postgresProcessIdentity.listener_process_creation_time
        listener_process_id = [long]$restartedPostgresIdentity.listener_process_id
        listener_process_creation_time = [string]$restartedPostgresIdentity.listener_process_creation_time
        listener_process_creation_time_source = [string]$restartedPostgresIdentity.listener_process_creation_time_source
        listener_process_created_at_utc = [string]$restartedPostgresIdentity.listener_process_created_at_utc
        listener_executable_path = [string]$restartedPostgresIdentity.listener_executable_path
        listener_executable_sha256 = [string]$restartedPostgresIdentity.listener_executable_sha256
        listener_executable_native_identity = [string]$restartedPostgresIdentity.listener_executable_native_identity
        listener_data_directory = [string]$restartedPostgresIdentity.listener_data_directory
        listener_host = [string]$restartedPostgresIdentity.listener_host
        listener_port = [long]$restartedPostgresIdentity.listener_port
        marker_raw_sha256 = (Get-FileHash -LiteralPath $markerPath -Algorithm SHA256).Hash.ToLowerInvariant()
        marker_native_identity = Get-LatticeWindowsNativePathIdentityToken -Path $markerPath -Directory $false
        restart_identity_distinct = $true
    })
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_UUID', $restartEvidence.DatabaseId, 'Process')
    [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_MANIFEST', $restartEvidence.ManifestHash, 'Process')
    $null = Invoke-LiveTest -Cargo $cargoCommand.Source -RepositoryRoot $repositoryRoot -Phase 'restart'
    if ($RunTask068HermesReplayGate) {
        $task068RestartOutput = Invoke-Task068HermesReplayGate `
            -Cargo $cargoCommand.Source `
            -RepositoryRoot $repositoryRoot `
            -Phase 'restart'
        $task068RestartEvidence = Get-Task068HermesReplayEvidence `
            -TestOutput $task068RestartOutput `
            -Phase 'restart'
        if (
            [string]$task068RestartEvidence.ReceiptSha256 -cne
            [string]$task068InitialEvidence.ReceiptSha256
        ) {
            throw 'TASK068_HERMES_REPLAY_RECEIPT_SUBSTITUTION_REJECTED'
        }
    }

    if (@($selectedHookCount).Count -eq 1) {
        foreach ($entry in ([ordered]@{
            LATTICE_TASK019_HOLDER_RECEIPT_PATH = [string]$holderReceipt.path
            LATTICE_TASK019_HOLDER_SESSION_ID = [string]$holderReceipt.session_id
            LATTICE_TASK019_HOLDER_NONCE = [string]$holderReceipt.nonce
            LATTICE_TASK019_HOLDER_NONCE_COMMITMENT = [string]$holderReceipt.nonce_commitment
            LATTICE_TASK019_HOLDER_CONSUMER_SESSION_ID = [string]$holderReceipt.consumer_session_id
            LATTICE_TASK019_HOLDER_DEADLINE_UTC = [string]$holderReceipt.deadline_utc
        }).GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'CONSUMER_STARTED' -Payload ([ordered]@{
            consumer_session_id = [string]$holderReceipt.consumer_session_id
            consumer_kind = $(if ($RunTask038AcceptanceHook) { 'TASK038_LOCAL_ACCEPTANCE' } elseif ($RunTask038TunnelHook) { 'TASK038_TUNNEL' } elseif ($RunFullChainAcceptanceHook) { 'TASK037_FULL_CHAIN' } else { 'LATTICE_DELIVERY' })
            restart_postmaster_started_at = [string]$restartedPostgresIdentity.postmaster_started_at
            listener_process_id = [long]$restartedPostgresIdentity.listener_process_id
            listener_process_creation_time = [string]$restartedPostgresIdentity.listener_process_creation_time
            listener_executable_sha256 = [string]$restartedPostgresIdentity.listener_executable_sha256
            listener_data_directory = [string]$restartedPostgresIdentity.listener_data_directory
            holder_process_id = [long]$holderReceipt.owner_process_id
            holder_process_creation_time = [string]$holderReceipt.owner_process_creation_time
        })
        $holderConsumerStarted = $true
    }

    if ($RunLatticeDeliveryHook) {
        $deliveryHookPath = Get-LatticeDeliveryHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $deliveryHookPath -InternalPhase 'DeliveryRun'

        if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw 'Could not prove the disposable PostgreSQL cluster stopped after the delivery-run phase.'
        }
        $clusterStarted = $false
        Remove-HarnessOutputFiles -Root $clusterRoot
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword

        $clusterStarted = $true
        $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
            '-D', $dataDirectory,
            '-l', $serverLog,
            '-w',
            '-t', '30',
            'start'
        ) -Operation 'PostgreSQL test-cluster delivery restart'
        Set-HarnessEnvironment -Phase 'restart' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_UUID', $restartEvidence.DatabaseId, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_MANIFEST', $restartEvidence.ManifestHash, 'Process')

        $deliveryHookPath = Get-LatticeDeliveryHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $deliveryHookPath -InternalPhase 'DeliveryStatus'
    }
    elseif ($RunFullChainAcceptanceHook) {
        $fullChainHookPath = Get-LatticeFullChainAcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $fullChainHookPath -InternalPhase 'FullChainPreStatus'

        if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw 'Could not prove the disposable PostgreSQL cluster stopped after the full-chain pre-status phase.'
        }
        $clusterStarted = $false
        Remove-HarnessOutputFiles -Root $clusterRoot
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword

        $clusterStarted = $true
        $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
            '-D', $dataDirectory,
            '-l', $serverLog,
            '-w',
            '-t', '30',
            'start'
        ) -Operation 'PostgreSQL test-cluster full-chain run restart'
        Set-HarnessEnvironment -Phase 'restart' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_UUID', $restartEvidence.DatabaseId, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_MANIFEST', $restartEvidence.ManifestHash, 'Process')

        $fullChainHookPath = Get-LatticeFullChainAcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $fullChainHookPath -InternalPhase 'FullChainRun'

        if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw 'Could not prove the disposable PostgreSQL cluster stopped after the full-chain run phase.'
        }
        $clusterStarted = $false
        Remove-HarnessOutputFiles -Root $clusterRoot
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword

        $clusterStarted = $true
        $null = Invoke-NativeChecked -Executable $pgCtl -Arguments @(
            '-D', $dataDirectory,
            '-l', $serverLog,
            '-w',
            '-t', '30',
            'start'
        ) -Operation 'PostgreSQL test-cluster full-chain status restart'
        Set-HarnessEnvironment -Phase 'restart' -HostName '127.0.0.1' -Port $port -Password $oneTimePassword -RunId $runId
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_UUID', $restartEvidence.DatabaseId, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_EXPECTED_MANIFEST', $restartEvidence.ManifestHash, 'Process')

        $fullChainHookPath = Get-LatticeFullChainAcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        & $fullChainHookPath -InternalPhase 'FullChainStatus'
    }
    elseif ($RunTask038TunnelHook) {
        [Environment]::SetEnvironmentVariable(
            'LATTICE_P0_CONSUMER_SESSION_ID',
            [string]$holderReceipt.consumer_session_id,
            'Process'
        )
        $task038TunnelHookPath = Get-LatticeTask038TunnelHookPath `
            -ScriptDirectory $PSScriptRoot `
            -RepositoryRoot $repositoryRoot
        $authority = Enable-Task038TunnelStoreAuthority `
            -Psql (Join-Path $postgresBin 'psql.exe') `
            -Port $port `
            -Password $oneTimePassword `
            -RunId $runId
        $tunnelEnvironment = [ordered]@{
            LATTICE_FULL_CHAIN_RUN_MODE = 'FRESH'
            LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
            LATTICE_DELIVERY_TIMEOUT_SECONDS = '300'
            LATTICE_STORE_DAEMON_INSTANCE_ID = [string]$authority.daemon_instance_id
            LATTICE_STORE_DAEMON_EPOCH = [string]$authority.daemon_epoch
            LATTICE_STORE_AUTHORITY_REVISION = [string]$authority.authority_revision
            LATTICE_STORE_OBSERVATION_DIGEST = [string]$authority.observation_digest
            LATTICE_STORE_AUTHORITY_HEAD_DIGEST = [string]$authority.head_digest
        }
        foreach ($entry in $tunnelEnvironment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        & $task038TunnelHookPath `
            -Mode 'ManagedRun' `
            -TunnelClientExecutable $Task038TunnelClientExecutable `
            -ProfileDirectory $Task038TunnelProfileDirectory `
            -ProfileName $Task038TunnelProfileName
        Invoke-StoreProfileLiveGate `
            -Cargo $cargoCommand.Source `
            -RepositoryRoot $repositoryRoot `
            -ExpectedProfile 'V3_MEMORY_V2_WRITER_LEASE_V1' `
            -Port $port `
            -Password $oneTimePassword `
            -RunId $runId
    }
    elseif ($RunTask038AcceptanceHook) {
        $task038HookPath = Get-LatticeTask038AcceptanceHookPath -ScriptDirectory $PSScriptRoot -RepositoryRoot $repositoryRoot
        $task038DatabaseName = 'lattice_task019_' + $runId.Substring(0, 8) + '_base'
        $encodedPassword = [Uri]::EscapeDataString($oneTimePassword)
        $task038Environment = [ordered]@{
            LATTICE_TASK038_POSTGRES_PASSWORD = $oneTimePassword
            LATTICE_WRITER_LEASE_MIGRATOR_URL = ('postgresql://lattice_migrator_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $port, $task038DatabaseName)
            LATTICE_WRITER_LEASE_RUNTIME_URL = ('postgresql://lattice_runtime_login:{0}@127.0.0.1:{1}/{2}' -f $encodedPassword, $port, $task038DatabaseName)
            LATTICE_WRITER_LEASE_ADMIN_URL = ('postgresql://task019_harness:{0}@127.0.0.1:{1}/postgres' -f $encodedPassword, $port)
        }
        foreach ($entry in $task038Environment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value, 'Process')
        }
        & $task038HookPath `
            -OfficialCodexExecutable $Task038OfficialCodexExecutable `
            -CodexAuthHome $Task038CodexAuthHome `
            -PostgresPort $port `
            -PostgresRunId $runId `
            -PsqlExecutable (Join-Path $postgresBin 'psql.exe') `
            -PostgresDataDirectory $dataDirectory
        Invoke-StoreProfileLiveGate `
            -Cargo $cargoCommand.Source `
            -RepositoryRoot $repositoryRoot `
            -ExpectedProfile 'V3_MEMORY_V2_WRITER_LEASE_V1' `
            -Port $port `
            -Password $oneTimePassword `
            -RunId $runId
        foreach ($entry in $task038Environment.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable([string]$entry.Key, $originalEnvironment[[string]$entry.Key], 'Process')
        }
    }
    if ($holderConsumerStarted) {
        Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'CONSUMER_EXITED' -Payload ([ordered]@{
            consumer_session_id = [string]$holderReceipt.consumer_session_id
            consumer_exit_classification = 'COMPLETED'
            restart_listener_process_id = [long]$restartedPostgresIdentity.listener_process_id
            restart_listener_still_present = (@(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction Stop).Count -eq 1)
        })
        $holderConsumerExited = $true
    }
    $harnessCompleted = $true
}
finally {
    try {
        foreach ($name in $environmentNames) {
            [Environment]::SetEnvironmentVariable($name, $originalEnvironment[$name], 'Process')
        }

        if (Test-Path -LiteralPath $passwordFile) {
            Remove-Item -LiteralPath $passwordFile -Force -ErrorAction SilentlyContinue
        }
        if ($null -ne $holderReceipt -and -not [bool]$holderReceipt.closed) {
            $stopListenerProcessId = if ($null -ne $restartedPostgresIdentity) {
                [long]$restartedPostgresIdentity.listener_process_id
            }
            elseif ($null -ne $postgresProcessIdentity) {
                [long]$postgresProcessIdentity.listener_process_id
            }
            else {
                0L
            }
            Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'HOLDER_STOP_REQUESTED' -Payload ([ordered]@{
                holder_process_id = [long]$holderReceipt.owner_process_id
                listener_process_id = $stopListenerProcessId
                consumer_started = [bool]$holderConsumerStarted
                consumer_exited = [bool]$holderConsumerExited
                harness_completed = [bool]$harnessCompleted
            })
        }
        if ($clusterStarted) {
            if (-not (Stop-TestCluster -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
                throw "Disposable cluster could not be proved stopped; preserving $clusterRoot"
            }
            $clusterStarted = $false
        }
        elseif ((Test-Path -LiteralPath $dataDirectory) -and -not (Test-ClusterStopped -PgCtl $pgCtl -DataDirectory $dataDirectory)) {
            throw "Disposable cluster status is not safely stopped; preserving $clusterRoot"
        }
        if ($null -ne $holderReceipt -and -not [bool]$holderReceipt.closed) {
            Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'HOLDER_STOPPED' -Payload ([ordered]@{
                pg_ctl_status_stopped = $true
                listener_absent = (@(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue).Count -eq 0)
                data_directory = $dataDirectory
                harness_completed = [bool]$harnessCompleted
            })
        }

        Remove-HarnessOutputFiles -Root $clusterRoot
        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword
        $oneTimePassword = $null

        if (Test-Path -LiteralPath $clusterRoot) {
            $cleanupTargetIsExact = Test-SafeCleanupTarget `
                -Root $clusterRoot `
                -ExpectedParent $clusterParent `
                -RepositoryTarget $repositoryTarget `
                -RunId $runId `
                -ContainmentSnapshot $cleanupContainment
            if (-not $cleanupTargetIsExact) {
                throw "Disposable cluster cleanup gate did not pass; preserving $clusterRoot"
            }
            if ($null -ne $holderReceipt -and -not [bool]$holderReceipt.closed) {
                Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'CLEANUP_REQUESTED' -Payload ([ordered]@{
                    cluster_root = $clusterRoot
                    cleanup_containment_verified = $true
                    marker_native_identity = [string]$cleanupContainment.marker_identity
                    harness_completed = [bool]$harnessCompleted
                })
            }
            Remove-Item -LiteralPath $clusterRoot -Recurse -Force
            if (Test-Path -LiteralPath $clusterRoot) {
                throw "Disposable cluster cleanup could not be proved complete; preserving $clusterRoot"
            }
        }
        if ($null -ne $holderReceipt -and -not [bool]$holderReceipt.closed) {
            Add-Task019HolderEvent -Receipt $holderReceipt -EventType 'CLEANUP_COMPLETED' -Payload ([ordered]@{
                cluster_root = $clusterRoot
                cluster_root_absent = (-not (Test-Path -LiteralPath $clusterRoot))
                listener_absent = (@(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue).Count -eq 0)
                harness_completed = [bool]$harnessCompleted
            })
            $holderFinalEvidence = Close-Task019HolderReceipt -Receipt $holderReceipt
        }
    }
    finally {
        if ($null -ne $holderReceipt -and -not [bool]$holderReceipt.closed) {
            try { $holderReceipt.stream.Dispose() } catch {}
            $holderReceipt.nonce = $null
        }
        if ($fullSerializationMutexOwned -and $null -ne $fullSerializationMutex) {
            $fullSerializationMutex.ReleaseMutex()
            $fullSerializationMutexOwned = $false
        }
        if ($null -ne $fullSerializationMutex) {
            $fullSerializationMutex.Dispose()
            $fullSerializationMutex = $null
        }
        $installedAfter = Get-InstalledPostgresSnapshot -PgIsReady $pgIsReady
        if (-not (Test-SameInstalledPostgresSnapshot -Before $installedBefore -After $installedAfter)) {
            throw 'Installed postgresql-x64-17 service or its read-only 127.0.0.1:5432 readiness snapshot changed during the harness.'
        }
    }
}

if (-not $harnessCompleted) {
    throw 'TASK-019 live phases did not complete.'
}
if ($writerLeaseOwnerProfileProved) {
    Write-Output 'TASK019_WRITER_LEASE_OWNER_PROFILE=PASS'
}
Write-Output 'TASK019_POSTGRES_HARNESS=PASS'
Write-Output "POSTGRES_VERSION=$expectedPostgresVersion"
Write-Output 'ENDPOINT=127.0.0.1:<dynamic-excludes-5432-64272-55432>'
Write-Output 'PHASES=initial,restart'
if ($RunTask068HermesReplayGate) {
    Write-Output 'TASK068_HERMES_POSTGRES_REPLAY=PASS'
}
if ($null -ne $holderFinalEvidence) {
    Write-Output ('HOLDER_RECEIPT_PATH=' + [string]$holderFinalEvidence.path)
    Write-Output ('HOLDER_RECEIPT_RAW_SHA256=' + [string]$holderFinalEvidence.raw_sha256)
    Write-Output ('HOLDER_RECEIPT_FINAL_HMAC_SHA256=' + [string]$holderFinalEvidence.final_hmac_sha256)
    Write-Output ('HOLDER_RECEIPT_EVENT_COUNT=' + [string]$holderFinalEvidence.event_count)
}
