[CmdletBinding()]
param(
    [switch]$SelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$task019Path = Join-Path $PSScriptRoot 'run-task019-postgres.ps1'
if (-not (Test-Path -LiteralPath $task019Path -PathType Leaf)) {
    throw 'TASK050_TASK019_HARNESS_MISSING'
}

$source = [Text.UTF8Encoding]::new($false, $true).GetString(
    [IO.File]::ReadAllBytes($task019Path)
)

$sourceWithoutCrLf = $source.Replace("`r`n", '')
$hasCrLf = $source.Contains("`r`n")
$hasBareLf = $sourceWithoutCrLf.Contains("`n")
$hasBareCr = $sourceWithoutCrLf.Contains("`r")
if (
    $hasBareCr -or
    ($hasCrLf -and $hasBareLf) -or
    (-not $hasCrLf -and -not $hasBareLf)
) {
    throw 'TASK050_TASK019_HARNESS_NEWLINE_REJECTED'
}
$sourceNewline = if ($hasCrLf) { "`r`n" } else { "`n" }

function Replace-Task050ExactSourceShape {
    param(
        [Parameter(Mandatory = $true)][string]$InputSource,
        [Parameter(Mandatory = $true)][string]$OldShape,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$NewShape,
        [Parameter(Mandatory = $true)][ValidateRange(1, 64)][int]$ExpectedCount,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $actualCount = [regex]::Matches(
        $InputSource,
        [regex]::Escape($OldShape),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count
    if ($actualCount -ne $ExpectedCount) {
        throw $FailureCode
    }
    $replaced = $InputSource.Replace($OldShape, $NewShape)
    $expectedResidualCount = $ExpectedCount * [regex]::Matches(
        $NewShape,
        [regex]::Escape($OldShape),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count
    $actualResidualCount = [regex]::Matches(
        $replaced,
        [regex]::Escape($OldShape),
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    ).Count
    if ($actualResidualCount -ne $expectedResidualCount) {
        throw $FailureCode
    }
    return $replaced
}

# The managed Windows runner denies Win32_Process CIM inspection even for the
# harness-owned postmaster. Preserve listener, executable hash, native file
# identity, process start time, SQL system identifier, and restart checks while
# using the process API available to this local acceptance boundary.
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape '$process = Get-CimInstance -ClassName Win32_Process -Filter (''ProcessId = '' + $processId) -ErrorAction Stop' `
    -NewShape '$process = Get-Process -Id $processId -ErrorAction Stop' `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_PROCESS_SHAPE_REJECTED'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape '$executable = Get-CanonicalPath -Path ([string]$process.ExecutablePath)' `
    -NewShape '$executable = Get-CanonicalPath -Path ([string]$process.Path)' `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_EXECUTABLE_SHAPE_REJECTED'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape '$createdAt = ([DateTimeOffset]([DateTime]$process.CreationDate)).ToUniversalTime()' `
    -NewShape '$createdAt = ([DateTimeOffset]$process.StartTime).ToUniversalTime()' `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_PROCESS_TIME_SHAPE_REJECTED'
if ($source.Contains('$process.CommandLine')) {
    throw 'TASK050_TASK019_COMMAND_LINE_SHAPE_REJECTED'
}

$suiteLoop = '    foreach ($suite in $liveSuites) {'
if (-not $source.Contains($suiteLoop)) {
    throw 'TASK050_TASK019_HARNESS_SHAPE_REJECTED'
}
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape $suiteLoop `
    -NewShape ('    $liveSuites = @([pscustomobject]@{ Name = ''store''; Package = ''lattice-postgres-store''; Test = ''postgres_task_ledger'' })' + $sourceNewline + $suiteLoop) `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_HARNESS_SHAPE_REJECTED'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape "'--test', 'postgres_live'," `
    -NewShape "'--test', [string]`$suite.Test," `
    -ExpectedCount 3 `
    -FailureCode 'TASK050_TASK019_TEST_SHAPE_REJECTED'
$cleanup = '        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword' + $sourceNewline +
    '        $oneTimePassword = $null'
$cleanupReplacement = '        if (-not [string]::IsNullOrEmpty($oneTimePassword)) {' + $sourceNewline +
    '            Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword' + $sourceNewline +
    '        }' + $sourceNewline +
    '        $oneTimePassword = $null'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape $cleanup `
    -NewShape $cleanupReplacement `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_CLEANUP_SHAPE_REJECTED'
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape ('        if (Test-Path -LiteralPath $clusterRoot) {' + $sourceNewline +
        '            $cleanupTargetIsExact = Test-SafeCleanupTarget') `
    -NewShape ('        if ($null -ne $cleanupContainment -and (Test-Path -LiteralPath $clusterRoot)) {' + $sourceNewline +
        '            $cleanupTargetIsExact = Test-SafeCleanupTarget') `
    -ExpectedCount 1 `
    -FailureCode 'TASK050_TASK019_CONTAINMENT_SHAPE_REJECTED'

$scriptRootLiteral = "'" + $PSScriptRoot.Replace("'", "''") + "'"
$source = Replace-Task050ExactSourceShape `
    -InputSource $source `
    -OldShape '$PSScriptRoot' `
    -NewShape $scriptRootLiteral `
    -ExpectedCount 13 `
    -FailureCode 'TASK050_TASK019_SCRIPT_ROOT_SHAPE_REJECTED'

if ($SelfTestOnly) {
    [scriptblock]::Create($source) | Out-Null
    Write-Output 'TASK050_TASK019_SOURCE_TRANSFORM_SELF_TEST=PASS'
    return
}

Push-Location $repositoryRoot
$previousTask050Live = [Environment]::GetEnvironmentVariable('LATTICE_TASK050_LIVE', 'Process')
try {
    [Environment]::SetEnvironmentVariable('LATTICE_TASK050_LIVE', '1', 'Process')
    $harness = [scriptblock]::Create($source)
    $completed = $false
    foreach ($attempt in 1..2) {
        try {
            & $harness -StoreOnly
            if ($LASTEXITCODE -ne 0) {
                throw 'TASK050_POSTGRES_HARNESS_REJECTED'
            }
            $completed = $true
            break
        }
        catch {
            if ($attempt -eq 2 -or $_.Exception.Message -cne 'TASK019_HOLDER_ACL_REJECTED') {
                throw
            }
            Start-Sleep -Milliseconds 200
        }
    }
    if (-not $completed) {
        throw 'TASK050_POSTGRES_HARNESS_REJECTED'
    }
}
finally {
    [Environment]::SetEnvironmentVariable('LATTICE_TASK050_LIVE', $previousTask050Live, 'Process')
    Pop-Location
}

Write-Output 'TASK050_POSTGRES_FRESH_PROCESS_ACCEPTANCE=PASS'
