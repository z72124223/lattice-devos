[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9a-f]{32}$')]
    [string]$RunId,

    [Parameter(Mandatory)]
    [ValidateRange(1, 65535)]
    [int]$Port,

    [Parameter(Mandatory)]
    [string]$RepositoryRoot,

    [switch]$CleanupInterrupted
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($Port -in @(5432, 58743)) {
    throw 'TASK094_FORBIDDEN_PORT'
}

function Get-Task094ListenerPid {
    $pattern = '^\s*TCP\s+127\.0\.0\.1:' + $Port + '\s+\S+\s+LISTENING\s+(\d+)\s*$'
    @(
        & "$env:SystemRoot\System32\netstat.exe" -ano -p tcp |
            ForEach-Object {
                if ($_ -match $pattern) {
                    [int]$Matches[1]
                }
            }
    )
}

$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$initdb = Join-Path $postgresBin 'initdb.exe'
$pgCtl = Join-Path $postgresBin 'pg_ctl.exe'
$postgres = Join-Path $postgresBin 'postgres.exe'
foreach ($binary in @($initdb, $pgCtl, $postgres)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw 'TASK094_POSTGRES_BINARY_MISSING'
    }
}

$tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')
$runRoot = [IO.Path]::GetFullPath((Join-Path $tempRoot "lattice-task094-pg-$RunId"))
$expectedRoot = "$tempRoot\lattice-task094-pg-$RunId"
if ($runRoot -cne $expectedRoot -or [IO.Path]::GetFileName($runRoot) -cne "lattice-task094-pg-$RunId") {
    throw 'TASK094_RUN_ROOT_REJECTED'
}
$repository = [IO.Path]::GetFullPath($RepositoryRoot)
if (-not (Test-Path -LiteralPath (Join-Path $repository 'Cargo.toml') -PathType Leaf)) {
    throw 'TASK094_REPOSITORY_REJECTED'
}
if ($CleanupInterrupted) {
    if (-not (Test-Path -LiteralPath $runRoot -PathType Container)) {
        throw 'TASK094_INTERRUPTED_ROOT_MISSING'
    }
    $owner = Get-Content -LiteralPath (Join-Path $runRoot 'TASK094_OWNER.json') -Raw |
        ConvertFrom-Json
    if (
        $owner.owner -cne 'TASK-094' -or
        $owner.run_id -cne $RunId -or
        [int]$owner.port -ne $Port
    ) {
        throw 'TASK094_INTERRUPTED_MARKER_MISMATCH'
    }
    & $pgCtl -D (Join-Path $runRoot 'data') status *> $null
    if ($LASTEXITCODE -ne 3) {
        throw 'TASK094_INTERRUPTED_SERVER_NOT_STOPPED'
    }
    if (@(Get-Task094ListenerPid).Count -ne 0) {
        throw 'TASK094_INTERRUPTED_LISTENER_PRESENT'
    }
    Remove-Item -LiteralPath $runRoot -Recurse -Force
    Write-Output "TASK094_INTERRUPTED_ROOT_REMOVED root_absent=$(-not (Test-Path -LiteralPath $runRoot))"
    exit 0
}
if (Test-Path -LiteralPath $runRoot) {
    throw 'TASK094_RUN_ROOT_COLLISION'
}
if (@(Get-Task094ListenerPid).Count -ne 0) {
    throw 'TASK094_PORT_COLLISION'
}

$dataRoot = Join-Path $runRoot 'data'
$logPath = Join-Path $runRoot 'postgres.log'
$pgCtlStartLog = Join-Path $runRoot 'pg_ctl-start.log'
$pgCtlStartError = Join-Path $runRoot 'pg_ctl-start.err'
$pgCtlStopLog = Join-Path $runRoot 'pg_ctl-stop.log'
$pgCtlStopError = Join-Path $runRoot 'pg_ctl-stop.err'
$markerPath = Join-Path $runRoot 'TASK094_OWNER.json'
$passwordPath = Join-Path $runRoot '.initdb-password'
$started = $false
$password = [Convert]::ToHexString(
    [Security.Cryptography.RandomNumberGenerator]::GetBytes(24)
).ToLowerInvariant()

try {
    New-Item -ItemType Directory -Path $runRoot | Out-Null
    [ordered]@{
        owner = 'TASK-094'
        run_id = $RunId
        port = $Port
        created_utc = [DateTime]::UtcNow.ToString('o')
        postgres_executable = $postgres
    } | ConvertTo-Json | Set-Content -LiteralPath $markerPath -Encoding utf8NoBOM
    Set-Content -LiteralPath $passwordPath -Value $password -Encoding ascii -NoNewline

    & $initdb -D $dataRoot -U task019_harness -A scram-sha-256 `
        --pwfile=$passwordPath --encoding=UTF8 --locale=C --data-checksums | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw 'TASK094_INITDB_FAILED'
    }
    Remove-Item -LiteralPath $passwordPath -Force

    $postgresOptions = "-p $Port -h 127.0.0.1 -c ssl=off -c fsync=on " +
        '-c synchronous_commit=on -c full_page_writes=on -c max_prepared_transactions=0'
    $startArguments = "-D `"$dataRoot`" -l `"$logPath`" -o `"$postgresOptions`" start"
    $startProcess = Start-Process -FilePath $pgCtl -ArgumentList $startArguments -PassThru `
        -WindowStyle Hidden -RedirectStandardOutput $pgCtlStartLog `
        -RedirectStandardError $pgCtlStartError
    $null = $startProcess.Handle
    $startProcess.WaitForExit()
    Get-Content -LiteralPath $pgCtlStartLog | Out-Host
    Get-Content -LiteralPath $pgCtlStartError -ErrorAction SilentlyContinue | Out-Host
    if ($startProcess.ExitCode -ne 0) {
        throw 'TASK094_PG_START_FAILED'
    }
    $started = $true

    $pidPath = Join-Path $dataRoot 'postmaster.pid'
    Write-Host 'TASK094_OWNERSHIP_CHECK_PID_ENTER'
    $postgresPid = [int]((Get-Content -LiteralPath $pidPath -TotalCount 1).Trim())
    $process = Get-Process -Id $postgresPid -ErrorAction Stop
    Write-Host 'TASK094_OWNERSHIP_CHECK_LISTENER_ENTER'
    $listenerPids = @(Get-Task094ListenerPid)
    Write-Host 'TASK094_OWNERSHIP_CHECK_MARKER_ENTER'
    $owner = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    if (
        $listenerPids.Count -ne 1 -or
        $listenerPids[0] -ne $postgresPid -or
        [IO.Path]::GetFullPath($process.Path) -cne [IO.Path]::GetFullPath($postgres) -or
        $owner.owner -cne 'TASK-094' -or
        $owner.run_id -cne $RunId -or
        [int]$owner.port -ne $Port
    ) {
        throw 'TASK094_LIVE_OWNERSHIP_REJECTED'
    }
    Write-Output "TASK094_LIVE_OWNERSHIP_OK run_root=$runRoot port=$Port pid=$postgresPid"

    $env:LATTICE_TASK019_LIVE = '1'
    $env:LATTICE_TASK019_HOST = '127.0.0.1'
    $env:LATTICE_TASK019_PORT = [string]$Port
    $env:LATTICE_TASK019_PASSWORD = $password
    $env:LATTICE_TASK019_RUN_ID = $RunId
    $env:LATTICE_TASK019_PHASE = 'task094_transition'
    Write-Host 'TASK094_CARGO_TEST_ENTER'
    Push-Location -LiteralPath $repository
    try {
        & cargo test -p lattice-runtime --test task094_writer_v3_transition `
            task094_writer_v3_transition_composition --locked -- --nocapture
        if ($LASTEXITCODE -ne 0) {
            throw 'TASK094_FOCUSED_LIVE_TEST_FAILED'
        }
    }
    finally {
        Pop-Location
    }
    Write-Output 'TASK094_FOCUSED_LIVE_GATE_PASS'
}
catch {
    if (Test-Path -LiteralPath $logPath -PathType Leaf) {
        Write-Host 'TASK094_POSTGRES_FAILURE_LOG_ENTER'
        Get-Content -LiteralPath $logPath |
            Select-String -Pattern 'ERROR:|CONTEXT:|DETAIL:|HINT:' -Context 2,6 |
            Select-Object -Last 80 |
            Out-Host
        Write-Host 'TASK094_POSTGRES_FAILURE_LOG_EXIT'
    }
    throw
}
finally {
    Remove-Item Env:LATTICE_TASK019_PASSWORD -ErrorAction SilentlyContinue
    if ($started -and (Test-Path -LiteralPath $dataRoot -PathType Container)) {
        $pidPath = Join-Path $dataRoot 'postmaster.pid'
        if (Test-Path -LiteralPath $pidPath -PathType Leaf) {
            $ownedPid = [int]((Get-Content -LiteralPath $pidPath -TotalCount 1).Trim())
            $ownedProcess = Get-Process -Id $ownedPid -ErrorAction SilentlyContinue
            if (
                $null -eq $ownedProcess -or
                [IO.Path]::GetFullPath($ownedProcess.Path) -cne [IO.Path]::GetFullPath($postgres)
            ) {
                throw 'TASK094_TEARDOWN_OWNERSHIP_LOST'
            }
        }
        $stopArguments = "-D `"$dataRoot`" -m fast -w stop"
        $stopProcess = Start-Process -FilePath $pgCtl -ArgumentList $stopArguments -PassThru `
            -WindowStyle Hidden -RedirectStandardOutput $pgCtlStopLog `
            -RedirectStandardError $pgCtlStopError
        $null = $stopProcess.Handle
        $stopProcess.WaitForExit()
        Get-Content -LiteralPath $pgCtlStopLog | Out-Host
        Get-Content -LiteralPath $pgCtlStopError -ErrorAction SilentlyContinue | Out-Host
        if ($stopProcess.ExitCode -ne 0) {
            throw 'TASK094_PG_STOP_FAILED'
        }
    }
    $listenerSurvivors = @(Get-Task094ListenerPid).Count
    if ($listenerSurvivors -ne 0) {
        throw 'TASK094_LISTENER_SURVIVOR'
    }
    if (Test-Path -LiteralPath $runRoot) {
        $deleteTarget = [IO.Path]::GetFullPath($runRoot)
        if ($deleteTarget -cne $expectedRoot) {
            throw 'TASK094_DELETE_TARGET_MISMATCH'
        }
        Remove-Item -LiteralPath $deleteTarget -Recurse -Force
    }
    Write-Output "TASK094_TEARDOWN_OK root_absent=$(-not (Test-Path -LiteralPath $runRoot)) listener_survivors=$listenerSurvivors"
}
