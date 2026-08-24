[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^[0-9a-f]{32}$')]
    [string]$RunId,

    [Parameter(Mandatory)]
    [ValidateRange(1, 65535)]
    [int]$Port,

    [Parameter(Mandatory)]
    [string]$RepositoryRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($Port -in @(4317, 5432, 58743)) {
    throw 'TASK105_FORBIDDEN_PORT'
}

function Get-Task105ListenerPid {
    $pattern = '^\s*TCP\s+127\.0\.0\.1:' + $Port + '\s+\S+\s+LISTENING\s+(\d+)\s*$'
    @(
        & "$env:SystemRoot\System32\netstat.exe" -ano -p tcp |
            ForEach-Object {
                if ($_ -match $pattern) { [int]$Matches[1] }
            }
    )
}

$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$initdb = Join-Path $postgresBin 'initdb.exe'
$pgCtl = Join-Path $postgresBin 'pg_ctl.exe'
$postgres = Join-Path $postgresBin 'postgres.exe'
foreach ($binary in @($initdb, $pgCtl, $postgres)) {
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw 'TASK105_POSTGRES_BINARY_MISSING'
    }
}

$repository = [IO.Path]::GetFullPath($RepositoryRoot)
if (-not (Test-Path -LiteralPath (Join-Path $repository 'Cargo.toml') -PathType Leaf)) {
    throw 'TASK105_REPOSITORY_REJECTED'
}
$git = (Get-Command git.exe -ErrorAction Stop).Source
$dirty = & $git -C $repository status --porcelain
if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrEmpty($dirty)) {
    throw 'TASK105_REPOSITORY_NOT_CLEAN'
}

$tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')
$runRoot = [IO.Path]::GetFullPath((Join-Path $tempRoot "lattice-task105-pg-$RunId"))
$expectedRoot = "$tempRoot\lattice-task105-pg-$RunId"
if ($runRoot -cne $expectedRoot -or (Test-Path -LiteralPath $runRoot)) {
    throw 'TASK105_RUN_ROOT_REJECTED'
}
if (@(Get-Task105ListenerPid).Count -ne 0) {
    throw 'TASK105_PORT_COLLISION'
}

$dataRoot = Join-Path $runRoot 'data'
$markerPath = Join-Path $runRoot 'TASK105_OWNER.json'
$passwordPath = Join-Path $runRoot '.initdb-password'
$postgresLog = Join-Path $runRoot 'postgres.log'
$startOut = Join-Path $runRoot 'pg-start.out'
$startErr = Join-Path $runRoot 'pg-start.err'
$stopOut = Join-Path $runRoot 'pg-stop.out'
$stopErr = Join-Path $runRoot 'pg-stop.err'
$passwordBytes = New-Object byte[] 24
$passwordGenerator = [Security.Cryptography.RandomNumberGenerator]::Create()
try {
    $passwordGenerator.GetBytes($passwordBytes)
}
finally {
    $passwordGenerator.Dispose()
}
$password = ([BitConverter]::ToString($passwordBytes)).Replace('-', '').ToLowerInvariant()
$started = $false

try {
    New-Item -ItemType Directory -Path $runRoot | Out-Null
    [ordered]@{
        owner = 'TASK-105'
        run_id = $RunId
        port = $Port
        created_utc = [DateTime]::UtcNow.ToString('o')
        postgres_executable = $postgres
    } | ConvertTo-Json | Set-Content -LiteralPath $markerPath -Encoding utf8
    Set-Content -LiteralPath $passwordPath -Value $password -Encoding ascii -NoNewline

    & $initdb -D $dataRoot -U runtime_bootstrap --auth-host=scram-sha-256 `
        --auth-local=trust --pwfile=$passwordPath --encoding=UTF8 --locale=C `
        --data-checksums | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'TASK105_INITDB_FAILED' }
    Remove-Item -LiteralPath $passwordPath -Force

    $options = "-p $Port -h 127.0.0.1 -c ssl=off -c fsync=on " +
        '-c synchronous_commit=on -c full_page_writes=on -c max_prepared_transactions=0'
    $arguments = "-D `"$dataRoot`" -l `"$postgresLog`" -o `"$options`" start"
    $start = Start-Process -FilePath $pgCtl -ArgumentList $arguments -PassThru `
        -WindowStyle Hidden -RedirectStandardOutput $startOut -RedirectStandardError $startErr
    $null = $start.Handle
    $start.WaitForExit()
    if ($start.ExitCode -ne 0) { throw 'TASK105_POSTGRES_START_FAILED' }
    $started = $true

    $postgresPid = [int]((Get-Content -LiteralPath (Join-Path $dataRoot 'postmaster.pid') -TotalCount 1).Trim())
    $process = Get-Process -Id $postgresPid -ErrorAction Stop
    $listeners = @(Get-Task105ListenerPid)
    $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    if (
        $listeners.Count -ne 1 -or $listeners[0] -ne $postgresPid -or
        [IO.Path]::GetFullPath($process.Path) -cne [IO.Path]::GetFullPath($postgres) -or
        $marker.owner -cne 'TASK-105' -or $marker.run_id -cne $RunId -or
        [int]$marker.port -ne $Port
    ) {
        throw 'TASK105_LIVE_OWNERSHIP_REJECTED'
    }
    Write-Output "TASK105_LIVE_OWNERSHIP_OK port=$Port pid=$postgresPid"

    $env:LATTICE_TASK105_LIVE = '1'
    $env:LATTICE_TASK105_PHASE = 'durable_foreman_restart'
    $env:LATTICE_TASK019_HOST = '127.0.0.1'
    $env:LATTICE_TASK019_PORT = [string]$Port
    $env:LATTICE_TASK019_PASSWORD = $password
    $env:LATTICE_TASK019_RUN_ID = $RunId
    $env:LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
    $env:LATTICE_FULL_CHAIN_RUN_MODE = 'RESUME_EXISTING'
    $env:LATTICE_RUNTIME_INTEGRATION = 'CORE_ONLY'
    $env:LATTICE_DELIVERY_TIMEOUT_SECONDS = '30'
    $env:LATTICE_DELIVERY_GIT_EXE = $git
    $env:LATTICE_GRAPHIFY_SOURCE_ROOT = $repository
    $env:LATTICE_STORE_DAEMON_INSTANCE_ID = 'task105-live-daemon'
    $env:LATTICE_STORE_DAEMON_EPOCH = '105'
    $env:LATTICE_STORE_AUTHORITY_REVISION = '105'
    $env:LATTICE_STORE_OBSERVATION_DIGEST = 'a' * 64
    $env:LATTICE_STORE_AUTHORITY_HEAD_DIGEST = 'b' * 64
    $env:LATTICE_TASK_INGRESS_KIND = 'LOCAL_CANONICAL_MCP_ACCEPTANCE'
    $env:LATTICE_TASK_INGRESS_PROFILE_SHA256 = 'c' * 64

    Push-Location -LiteralPath $repository
    try {
        & cargo test -p lattice-runtime --test task105_durable_foreman_runtime `
            --locked -- --nocapture --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'TASK105_LIVE_TEST_FAILED' }
    }
    finally {
        Pop-Location
    }
    Write-Output 'TASK105_DURABLE_FOREMAN_LIVE_GATE=PASS'
}
catch {
    if (Test-Path -LiteralPath $postgresLog -PathType Leaf) {
        Get-Content -LiteralPath $postgresLog |
            Select-String -Pattern 'ERROR:|CONTEXT:|DETAIL:|HINT:' -Context 2,5 |
            Select-Object -Last 60 | Out-Host
    }
    throw
}
finally {
    Remove-Item Env:LATTICE_TASK019_PASSWORD -ErrorAction SilentlyContinue
    if ($started -and (Test-Path -LiteralPath $dataRoot -PathType Container)) {
        $ownedPid = [int]((Get-Content -LiteralPath (Join-Path $dataRoot 'postmaster.pid') -TotalCount 1).Trim())
        $ownedProcess = Get-Process -Id $ownedPid -ErrorAction SilentlyContinue
        if ($null -eq $ownedProcess -or [IO.Path]::GetFullPath($ownedProcess.Path) -cne [IO.Path]::GetFullPath($postgres)) {
            throw 'TASK105_TEARDOWN_OWNERSHIP_LOST'
        }
        $arguments = "-D `"$dataRoot`" -m fast -w stop"
        $stop = Start-Process -FilePath $pgCtl -ArgumentList $arguments -PassThru `
            -WindowStyle Hidden -RedirectStandardOutput $stopOut -RedirectStandardError $stopErr
        $null = $stop.Handle
        $stop.WaitForExit()
        if ($stop.ExitCode -ne 0) { throw 'TASK105_POSTGRES_STOP_FAILED' }
    }
    if (@(Get-Task105ListenerPid).Count -ne 0) { throw 'TASK105_LISTENER_SURVIVOR' }
    if (Test-Path -LiteralPath $runRoot) {
        $deleteTarget = [IO.Path]::GetFullPath($runRoot)
        if ($deleteTarget -cne $expectedRoot) { throw 'TASK105_DELETE_TARGET_MISMATCH' }
        Remove-Item -LiteralPath $deleteTarget -Recurse -Force
    }
    Write-Output "TASK105_TEARDOWN_OK root_absent=$(-not (Test-Path -LiteralPath $runRoot)) listener_absent=$(@(Get-Task105ListenerPid).Count -eq 0)"
}
