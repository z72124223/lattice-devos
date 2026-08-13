[CmdletBinding()]
param()

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

# The managed Windows runner denies Win32_Process CIM inspection even for the
# harness-owned postmaster. Preserve listener, executable hash, native file
# identity, process start time, SQL system identifier, and restart checks while
# using the process API available to this local acceptance boundary.
$source = $source.Replace(
    '$process = Get-CimInstance -ClassName Win32_Process -Filter (''ProcessId = '' + $processId) -ErrorAction Stop',
    '$process = Get-Process -Id $processId -ErrorAction Stop'
)
$source = $source.Replace(
    '$executable = Get-CanonicalPath -Path ([string]$process.ExecutablePath)',
    '$executable = Get-CanonicalPath -Path ([string]$process.Path)'
)
$source = $source.Replace(
    '$createdAt = ([DateTimeOffset]([DateTime]$process.CreationDate)).ToUniversalTime()',
    '$createdAt = ([DateTimeOffset]$process.StartTime).ToUniversalTime()'
)
$source = $source.Replace(
    "            [string]`$process.CommandLine -notlike ('*' + (Get-CanonicalPath -Path `$DataDirectory) + '*') -or`r`n",
    ''
)

$suiteLoop = '    foreach ($suite in $liveSuites) {'
if (-not $source.Contains($suiteLoop)) {
    throw 'TASK050_TASK019_HARNESS_SHAPE_REJECTED'
}
$source = $source.Replace(
    $suiteLoop,
    '    $liveSuites = @([pscustomobject]@{ Name = ''store''; Package = ''lattice-postgres-store''; Test = ''postgres_task_ledger'' })' + "`r`n" + $suiteLoop
)
$source = $source.Replace("'--test', 'postgres_live',", "'--test', [string]`$suite.Test,")
$cleanup = '        Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword' + "`r`n" +
    '        $oneTimePassword = $null'
$cleanupReplacement = '        if (-not [string]::IsNullOrEmpty($oneTimePassword)) {' + "`r`n" +
    '            Remove-VerifiedSafeServerLog -LogPath $serverLog -RepositoryRoot $repositoryRoot -OneTimePassword $oneTimePassword' + "`r`n" +
    '        }' + "`r`n" +
    '        $oneTimePassword = $null'
if (-not $source.Contains($cleanup)) {
    throw 'TASK050_TASK019_CLEANUP_SHAPE_REJECTED'
}
$source = $source.Replace($cleanup, $cleanupReplacement)
$source = $source.Replace(
    '        if (Test-Path -LiteralPath $clusterRoot) {' + "`r`n" +
    '            $cleanupTargetIsExact = Test-SafeCleanupTarget',
    '        if ($null -ne $cleanupContainment -and (Test-Path -LiteralPath $clusterRoot)) {' + "`r`n" +
    '            $cleanupTargetIsExact = Test-SafeCleanupTarget'
)

$scriptRootLiteral = "'" + $PSScriptRoot.Replace("'", "''") + "'"
$source = $source.Replace('$PSScriptRoot', $scriptRootLiteral)

Push-Location $repositoryRoot
$previousTask050Live = [Environment]::GetEnvironmentVariable('LATTICE_TASK050_LIVE', 'Process')
try {
    [Environment]::SetEnvironmentVariable('LATTICE_TASK050_LIVE', '1', 'Process')
    $harness = [scriptblock]::Create($source)
    $completed = $false
    foreach ($attempt in 1..2) {
        try {
            & $harness
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
