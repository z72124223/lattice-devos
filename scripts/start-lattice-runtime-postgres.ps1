[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateScript({ [IO.Path]::IsPathRooted($_) -and (Test-Path -LiteralPath $_ -PathType Leaf) })]
    [string]$LatticedPath,

    [ValidateScript({ [IO.Path]::IsPathRooted($_) })]
    [string]$StateRoot = (Join-Path $env:LOCALAPPDATA 'LATTICE\runtime-postgres'),

    [ValidateScript({ [IO.Path]::IsPathRooted($_) -and (Test-Path -LiteralPath $_ -PathType Leaf) })]
    [string]$ConfigPath = (Join-Path $env:USERPROFILE '.codex\config.toml'),

    [switch]$Background
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Background) {
    $logRoot = Join-Path $StateRoot 'bootstrap'
    New-Item -ItemType Directory -Path $logRoot -Force | Out-Null
    $stdout = Join-Path $logRoot 'stdout.log'
    $stderr = Join-Path $logRoot 'stderr.log'
    Remove-Item -LiteralPath $stdout, $stderr -Force -ErrorAction SilentlyContinue
    $arguments = @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $PSCommandPath,
        '-LatticedPath', $LatticedPath,
        '-StateRoot', $StateRoot,
        '-ConfigPath', $ConfigPath
    )
    $process = Start-Process -FilePath 'powershell.exe' -ArgumentList $arguments `
        -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    Write-Output "LATTICE_RUNTIME_POSTGRES_BACKGROUND_STARTED:$($process.Id)"
    exit 0
}

function Get-FreeLoopbackPort {
    do {
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
        try {
            $listener.Start()
            $candidate = ([Net.IPEndPoint]$listener.LocalEndpoint).Port
        }
        finally {
            $listener.Stop()
        }
    } while ($candidate -eq 5432 -or $candidate -eq 58743)
    return $candidate
}

function Get-RandomHex {
    param([Parameter(Mandatory = $true)][ValidateRange(1, 64)][int]$ByteCount)
    $bytes = [byte[]]::new($ByteCount)
    $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $generator.GetBytes($bytes) }
    finally { $generator.Dispose() }
    return (([BitConverter]::ToString($bytes) -replace '-', '')).ToLowerInvariant()
}

function Set-LatticeConfigEnvironment {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][hashtable]$Values
    )
    $lines = [Collections.Generic.List[string]](Get-Content -LiteralPath $Path)
    $start = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ($lines[$index] -eq '[mcp_servers.lattice.env]') {
            $start = $index + 1
            break
        }
    }
    if ($start -lt 0) { throw 'LATTICE_RUNTIME_CONFIG_SECTION_MISSING' }
    $end = $lines.Count
    for ($index = $start; $index -lt $lines.Count; $index++) {
        if ($lines[$index] -match '^\[') { $end = $index; break }
    }
    foreach ($name in $Values.Keys) {
        $tomlValue = ([string]$Values[$name]).Replace('\', '\\').Replace('"', '\"')
        $replacement = "$name = `"$tomlValue`""
        $found = $false
        for ($index = $start; $index -lt $end; $index++) {
            if ($lines[$index] -match ('^' + [regex]::Escape($name) + '\s*=')) {
                $lines[$index] = $replacement
                $found = $true
                break
            }
        }
        if (-not $found) {
            $lines.Insert($end, $replacement)
            $end++
        }
    }
    $temporary = "$Path.runtime-postgres.tmp"
    [IO.File]::WriteAllLines($temporary, $lines, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Import-LatticeConfigEnvironment {
    param([Parameter(Mandatory = $true)][string]$Path)
    $inside = $false
    foreach ($line in Get-Content -LiteralPath $Path) {
        if ($line -eq '[mcp_servers.lattice.env]') { $inside = $true; continue }
        if ($inside -and $line -match '^\[') { break }
        if ($inside -and $line -match '^([A-Z0-9_]+)\s*=\s*"(.*)"\s*$') {
            $value = $Matches[2].Replace('\\', '\')
            [Environment]::SetEnvironmentVariable($Matches[1], $value, 'Process')
        }
    }
}

function Initialize-LatticeRuntimeCodexHome {
    param([Parameter(Mandatory = $true)][string]$Path)

    $markerPath = Join-Path $Path '.lattice-codex-home-v1'
    $configPath = Join-Path $Path 'config.toml'
    $authPath = Join-Path $Path 'auth.json'
    $markerBytes = [Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
    $configBytes = [Text.UTF8Encoding]::new($false).GetBytes((@(
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

    $items = @(Get-ChildItem -LiteralPath $Path -Force)
    if ($items.Count -eq 0) {
        [IO.File]::WriteAllBytes($markerPath, $markerBytes)
        [IO.File]::WriteAllBytes($configPath, $configBytes)
    }
    if (Test-Path -LiteralPath $authPath) {
        throw 'LATTICE_RUNTIME_CODEX_PLAINTEXT_AUTH_DENIED'
    }
    $markerItem = Get-Item -LiteralPath $markerPath -Force -ErrorAction SilentlyContinue
    $configItem = Get-Item -LiteralPath $configPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $markerItem -or $markerItem.PSIsContainer -or
        ($markerItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $null -eq $configItem -or $configItem.PSIsContainer -or
        ($configItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        [Convert]::ToBase64String([IO.File]::ReadAllBytes($markerPath)) -ne
            [Convert]::ToBase64String($markerBytes) -or
        [Convert]::ToBase64String([IO.File]::ReadAllBytes($configPath)) -ne
            [Convert]::ToBase64String($configBytes)
    ) {
        throw 'LATTICE_RUNTIME_CODEX_HOME_REJECTED'
    }
}

function Start-LatticePostgres {
    param(
        [Parameter(Mandatory = $true)][string]$PgCtl,
        [Parameter(Mandatory = $true)][string]$Cluster,
        [Parameter(Mandatory = $true)][string]$LogPath
    )
    Start-Process -FilePath $PgCtl -ArgumentList @(
        'start', '-D', $Cluster, '-l', $LogPath, '-w', '-t', '30'
    ) -WindowStyle Hidden | Out-Null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Seconds 1
        & $PgCtl status -D $Cluster *> $null
        if ($LASTEXITCODE -eq 0) { return }
    }
    throw 'LATTICE_RUNTIME_POSTGRES_START_REJECTED'
}

function Write-RuntimeMetadata {
    param([Parameter(Mandatory = $true)][string]$Path)
    $port = [int]$env:LATTICE_TASK019_PORT
    $runId = [string]$env:LATTICE_TASK019_RUN_ID
    [IO.File]::WriteAllText(
        $Path,
        ('{"schema":"lattice.runtime-postgres.v1","host":"127.0.0.1","port":' + $port + ',"run_id":"' + $runId + '"}'),
        [Text.UTF8Encoding]::new($false)
    )
}

$clusterRoot = Join-Path $StateRoot 'cluster'
$metadataPath = Join-Path $StateRoot 'runtime-postgres.json'
$postgresLog = Join-Path $StateRoot 'postgres.log'
$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$initdb = Join-Path $postgresBin 'initdb.exe'
$pgCtl = Join-Path $postgresBin 'pg_ctl.exe'
if (-not (Test-Path -LiteralPath $initdb -PathType Leaf) -or -not (Test-Path -LiteralPath $pgCtl -PathType Leaf)) {
    throw 'LATTICE_RUNTIME_POSTGRES_BINARIES_MISSING'
}
$gitExecutable = (Get-Command git.exe -ErrorAction Stop).Source
if (-not [IO.Path]::IsPathRooted($gitExecutable) -or -not (Test-Path -LiteralPath $gitExecutable -PathType Leaf)) {
    throw 'LATTICE_RUNTIME_GIT_MISSING'
}
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$dirty = & $gitExecutable -C $repositoryRoot status --porcelain
if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrEmpty($dirty)) {
    throw 'LATTICE_RUNTIME_GRAPH_SOURCE_REJECTED'
}
$graphWorkRoot = Join-Path $env:LOCALAPPDATA 'LATTICE\runtime-graphify'
$dependencyWorktreeRoot = Join-Path $env:LOCALAPPDATA 'LATTICE\dependency-worktrees'
$runtimeTaskRoot = Join-Path $env:LOCALAPPDATA 'LATTICE\runtime-delivery'
$runtimeTaskCodexHome = Join-Path $env:LOCALAPPDATA 'LATTICE\runtime-codex-home-keyring-v1'
New-Item -ItemType Directory -Path $graphWorkRoot, $dependencyWorktreeRoot, $runtimeTaskRoot, $runtimeTaskCodexHome -Force | Out-Null
Initialize-LatticeRuntimeCodexHome -Path $runtimeTaskCodexHome
$runtimeConfig = @{
    LATTICE_DELIVERY_GIT_EXE = $gitExecutable
    LATTICE_GRAPHIFY_SOURCE_ROOT = $repositoryRoot
    LATTICE_GRAPHIFY_WORK_ROOT = $graphWorkRoot
    LATTICE_DEPENDENCY_WORKTREE_ROOT = $dependencyWorktreeRoot
    LATTICE_FULL_CHAIN_RUN_MODE = 'FRESH'
    LATTICE_DELIVERY_ROOT = $runtimeTaskRoot
    LATTICE_DELIVERY_CODEX_HOME = $runtimeTaskCodexHome
}
if (Test-Path -LiteralPath $metadataPath) {
    Set-LatticeConfigEnvironment -Path $ConfigPath -Values $runtimeConfig
    Import-LatticeConfigEnvironment -Path $ConfigPath
    & $pgCtl status -D $clusterRoot *> $null
    if ($LASTEXITCODE -ne 0) {
        Start-LatticePostgres -PgCtl $pgCtl -Cluster $clusterRoot -LogPath $postgresLog
    }
    & $LatticedPath --postgres-bootstrap
    if ($LASTEXITCODE -ne 0) { throw 'LATTICE_RUNTIME_POSTGRES_BOOTSTRAP_REJECTED' }
    Write-Output 'LATTICE_RUNTIME_POSTGRES_READY'
    exit 0
}

if (Test-Path -LiteralPath $clusterRoot) {
    Set-LatticeConfigEnvironment -Path $ConfigPath -Values $runtimeConfig
    Import-LatticeConfigEnvironment -Path $ConfigPath
    Start-LatticePostgres -PgCtl $pgCtl -Cluster $clusterRoot -LogPath $postgresLog
    & $LatticedPath --postgres-initialize
    if ($LASTEXITCODE -ne 0) { throw 'LATTICE_RUNTIME_POSTGRES_INITIALIZE_REJECTED' }
    & $LatticedPath --postgres-bootstrap
    if ($LASTEXITCODE -ne 0) { throw 'LATTICE_RUNTIME_POSTGRES_BOOTSTRAP_REJECTED' }
    Write-RuntimeMetadata -Path $metadataPath
    Write-Output 'LATTICE_RUNTIME_POSTGRES_READY'
    exit 0
}

New-Item -ItemType Directory -Path $StateRoot -Force | Out-Null
$port = Get-FreeLoopbackPort
$runId = Get-RandomHex -ByteCount 16
$password = Get-RandomHex -ByteCount 32
$passwordFile = Join-Path $StateRoot '.initdb-password'
try {
    [IO.File]::WriteAllText($passwordFile, $password, [Text.UTF8Encoding]::new($false))
    & $initdb -D $clusterRoot -U runtime_bootstrap --auth-host=scram-sha-256 --auth-local=trust --pwfile=$passwordFile -E UTF8 --no-locale --data-checksums *> $null
    if ($LASTEXITCODE -ne 0) { throw 'LATTICE_RUNTIME_POSTGRES_INITDB_REJECTED' }
}
finally {
    Remove-Item -LiteralPath $passwordFile -Force -ErrorAction SilentlyContinue
}
[IO.File]::AppendAllText(
    (Join-Path $clusterRoot 'postgresql.conf'),
    ("`n# LATTICE-owned local Runtime only`nlisten_addresses = '127.0.0.1'`nport = $port`n"),
    [Text.UTF8Encoding]::new($false)
)
Start-LatticePostgres -PgCtl $pgCtl -Cluster $clusterRoot -LogPath $postgresLog

try {
    Set-LatticeConfigEnvironment -Path $ConfigPath -Values @{
        LATTICE_TASK019_HOST = '127.0.0.1'
        LATTICE_TASK019_PORT = [string]$port
        LATTICE_TASK019_RUN_ID = $runId
        LATTICE_TASK019_PASSWORD = $password
        LATTICE_DELIVERY_GIT_EXE = $gitExecutable
        LATTICE_GRAPHIFY_SOURCE_ROOT = $repositoryRoot
        LATTICE_GRAPHIFY_WORK_ROOT = $graphWorkRoot
        LATTICE_DEPENDENCY_WORKTREE_ROOT = $dependencyWorktreeRoot
    }
    Import-LatticeConfigEnvironment -Path $ConfigPath
    & $LatticedPath --postgres-initialize
    if ($LASTEXITCODE -ne 0) { throw 'LATTICE_RUNTIME_POSTGRES_INITIALIZE_REJECTED' }
    & $LatticedPath --postgres-bootstrap
    if ($LASTEXITCODE -ne 0) { throw 'LATTICE_RUNTIME_POSTGRES_BOOTSTRAP_REJECTED' }
}
catch {
    & $pgCtl stop -D $clusterRoot -m fast *> $null
    throw
}

Write-RuntimeMetadata -Path $metadataPath
Write-Output 'LATTICE_RUNTIME_POSTGRES_READY'
