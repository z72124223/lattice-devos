#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PurgeBinary,
    [Parameter(Mandatory = $true)][string]$SeedBinary,
    [Parameter(Mandatory = $true)][string]$RuntimeBinary,
    [string]$EpochFixtureBinary,
    [string]$LegacyPurgeBinary,
    [string]$NodeBinary = (Get-Command node.exe -ErrorAction Stop).Source,
    [ValidateSet('all','main','interleaved','survivor-reference','epoch','epoch-reference','coordinator','coordinator-absent','inventory','bot-inventory','graph-ownership','upgrade')][string]$Scenario = 'all'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fixtureBase = Join-Path $repo '.lattice\project-purge-fixture'
$runId = [Guid]::NewGuid().ToString('N')
$runRoot = [IO.Path]::GetFullPath((Join-Path $fixtureBase $runId))
$cluster = Join-Path $runRoot 'cluster'
$markerPath = Join-Path $runRoot 'fixture-owner.json'
$pgBin = 'C:\Program Files\PostgreSQL\17\bin'
$pgCtl = Join-Path $pgBin 'pg_ctl.exe'
$postgresBinary = Join-Path $pgBin 'postgres.exe'
$psql = Join-Path $pgBin 'psql.exe'
$initialized = $false
$serverIdentity = $null
$result = [ordered]@{ schema = 'lattice.project-purge-live-fixture.v1'; runId = $runId; status = 'RUNNING'; runRoot = $runRoot; productionDatabaseAccess = $false }

if ($Scenario -eq 'upgrade' -and [string]::IsNullOrWhiteSpace($LegacyPurgeBinary)) { throw 'FIXTURE_LEGACY_BINARY_REQUIRED' }
if ($Scenario -in @('epoch','epoch-reference','bot-inventory','graph-ownership','all') -and [string]::IsNullOrWhiteSpace($EpochFixtureBinary)) { $EpochFixtureBinary = Join-Path (Split-Path -Parent $SeedBinary) 'project_purge_epoch_fixture.exe' }
$inputBinaries = @($PurgeBinary, $SeedBinary, $RuntimeBinary, $NodeBinary, $pgCtl, $postgresBinary, $psql, (Join-Path $pgBin 'initdb.exe'))
if (-not [string]::IsNullOrWhiteSpace($LegacyPurgeBinary)) { $inputBinaries += $LegacyPurgeBinary }
if (-not [string]::IsNullOrWhiteSpace($EpochFixtureBinary)) { $inputBinaries += $EpochFixtureBinary }
foreach ($file in $inputBinaries) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "FIXTURE_BINARY_MISSING: $file" }
}
if (-not $runRoot.StartsWith($fixtureBase + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'FIXTURE_PATH_REJECTED' }
[IO.Directory]::CreateDirectory($runRoot) | Out-Null
$binRoot = Join-Path $runRoot 'bin'
[IO.Directory]::CreateDirectory($binRoot) | Out-Null
$copies = @{}
$binaryEntries = @(@{name='purge';source=$PurgeBinary},@{name='seed';source=$SeedBinary},@{name='runtime';source=$RuntimeBinary})
if (-not [string]::IsNullOrWhiteSpace($LegacyPurgeBinary)) { $binaryEntries += @{name='legacy';source=$LegacyPurgeBinary} }
if (-not [string]::IsNullOrWhiteSpace($EpochFixtureBinary)) { $binaryEntries += @{name='epoch';source=$EpochFixtureBinary} }
foreach ($entry in $binaryEntries) {
    $source = [IO.Path]::GetFullPath($entry.source)
    $copyName = if ($entry.name -eq 'legacy') { 'legacy-lattice-project-purge.exe' } else { [IO.Path]::GetFileName($source) }
    $copy = Join-Path $binRoot $copyName
    $before = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    [IO.File]::Copy($source, $copy, $false)
    $after = (Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash
    if ($before -cne $after -or $before -cne (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash) { throw 'FIXTURE_BINARY_CHANGED_DURING_COPY' }
    $copies[$entry.name] = @{ source=$source; path=$copy; sha256=$after.ToLowerInvariant() }
}
$PurgeBinary=$copies.purge.path; $SeedBinary=$copies.seed.path; $RuntimeBinary=$copies.runtime.path
if ($copies.ContainsKey('legacy')) { $LegacyPurgeBinary = $copies.legacy.path }
if ($copies.ContainsKey('epoch')) { $EpochFixtureBinary = $copies.epoch.path }
$result.binaryCopies = $copies
if ($Scenario -eq 'all') {
    $result.children = @()
    try {
        $stages = @('main','interleaved','survivor-reference','epoch','epoch-reference','coordinator','coordinator-absent','inventory','bot-inventory','graph-ownership')
        if ($copies.ContainsKey('legacy')) { $stages += 'upgrade' }
        foreach ($stage in $stages) {
            $childOutput = @(& $PSCommandPath -PurgeBinary $PurgeBinary -SeedBinary $SeedBinary -RuntimeBinary $RuntimeBinary -EpochFixtureBinary $EpochFixtureBinary -LegacyPurgeBinary $LegacyPurgeBinary -NodeBinary $NodeBinary -Scenario $stage)
            $child = $childOutput[-1] | ConvertFrom-Json
            if ($child.status -cne 'PASS' -or -not $child.fixtureStopped) { throw "FIXTURE_STAGE_FAILED: $stage" }
            foreach ($key in $copies.Keys) {
                if ($child.binaryCopies.$key.sha256 -cne $copies[$key].sha256) { throw 'FIXTURE_MATRIX_BINARY_MISMATCH' }
            }
            $result.children += @{scenario=$stage;runRoot=$child.runRoot;status=$child.status;fixtureStopped=$child.fixtureStopped}
            Write-Output "PROJECT_PURGE_FIXTURE_STAGE_PASS: $stage"
        }
        $result.status = 'PASS'
        $result.fixtureStopped = $true
    } catch { $result.status='FAIL'; $result.error=$_.Exception.Message; throw }
    finally {
        $result.finishedAt=[DateTimeOffset]::UtcNow.ToString('o')
        $result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $runRoot 'harness-result.json') -Encoding utf8NoBOM
        Write-Output ($result | ConvertTo-Json -Depth 12 -Compress)
    }
    return
}
$result.scenario = $Scenario
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()
if ($port -in @(4317, 5432, 55432, 58743, 64272)) { throw 'FIXTURE_RESERVED_PORT_REJECTED' }
@{ kind = 'LATTICE_PROJECT_PURGE_SYNTHETIC_FIXTURE'; runId = $runId; cluster = $cluster; port = $port } | ConvertTo-Json | Set-Content -LiteralPath $markerPath -Encoding utf8NoBOM
$result.port = $port

try {
    & (Join-Path $pgBin 'initdb.exe') -D $cluster -U runtime_bootstrap --auth-host=trust --auth-local=trust --encoding=UTF8 --no-locale --data-checksums *> (Join-Path $runRoot 'initdb.log')
    if ($LASTEXITCODE -ne 0) { throw 'FIXTURE_INITDB_FAILED' }
    $initialized = $true
    @"
listen_addresses = '127.0.0.1'
port = $port
max_connections = 15
shared_buffers = '16MB'
log_statement = 'none'
"@ | Add-Content -LiteralPath (Join-Path $cluster 'postgresql.conf') -Encoding utf8NoBOM
    # Wait only for pg_ctl itself. Start-Process -Wait would wait for its child tree.
    $arguments = '-D "{0}" -l "{1}" -w -t 30 start' -f $cluster, (Join-Path $runRoot 'postgres.log')
    $launcher = Start-Process -FilePath $pgCtl -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $runRoot 'pgctl-start.out.log') -RedirectStandardError (Join-Path $runRoot 'pgctl-start.err.log')
    if (-not $launcher.WaitForExit(35000) -or $launcher.ExitCode -ne 0) { throw 'FIXTURE_START_FAILED' }
    $postmaster = [int](Get-Content -LiteralPath (Join-Path $cluster 'postmaster.pid') -TotalCount 1)
    $process = Get-Process -Id $postmaster
    if ($process.Path -cne $postgresBinary) { throw 'FIXTURE_PROCESS_IDENTITY_REJECTED' }
    $serverIdentity = @{ pid = $postmaster; startTicks = $process.StartTime.ToUniversalTime().Ticks }
    $dataDirectory = (& $psql -X -A -t -h 127.0.0.1 -p $port -U runtime_bootstrap -d postgres -v ON_ERROR_STOP=1 -c 'SHOW data_directory').Trim()
    if ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFullPath($dataDirectory) -cne $cluster) { throw 'FIXTURE_DATABASE_IDENTITY_REJECTED' }
    $nodeArguments = @((Join-Path $PSScriptRoot 'test-project-purge-postgres.mjs'), '--binary', $PurgeBinary, '--seed-binary', $SeedBinary, '--runtime-binary', $RuntimeBinary, '--port', $port, '--run-root', $runRoot, '--psql', $psql, '--scenario', $Scenario)
    if ($copies.ContainsKey('legacy')) { $nodeArguments += @('--legacy-binary', $LegacyPurgeBinary) }
    if ($copies.ContainsKey('epoch')) { $nodeArguments += @('--epoch-binary', $EpochFixtureBinary) }
    & $NodeBinary @nodeArguments
    if ($LASTEXITCODE -ne 0) { throw 'FIXTURE_SCENARIOS_FAILED' }
    foreach ($copy in $copies.Values) {
        if ((Get-FileHash -LiteralPath $copy.path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $copy.sha256) { throw 'FIXTURE_BINARY_CHANGED_DURING_RUN' }
    }
    $result.status = 'PASS'
} catch {
    $result.status = 'FAIL'
    $result.error = $_.Exception.Message
    throw
} finally {
    # No recursive filesystem deletion: preserve this complete synthetic fixture as evidence.
    $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    $pidPath = Join-Path $cluster 'postmaster.pid'
    if ($initialized -and (Test-Path -LiteralPath $pidPath)) {
        if ($marker.kind -cne 'LATTICE_PROJECT_PURGE_SYNTHETIC_FIXTURE' -or $marker.runId -cne $runId -or $marker.cluster -cne $cluster -or $marker.port -ne $port) { throw 'FIXTURE_STOP_OWNERSHIP_REJECTED' }
        $livePid = [int](Get-Content -LiteralPath $pidPath -TotalCount 1)
        $liveProcess = Get-Process -Id $livePid
        if ($liveProcess.Path -cne $postgresBinary -or ($serverIdentity -and ($livePid -ne $serverIdentity.pid -or $liveProcess.StartTime.ToUniversalTime().Ticks -ne $serverIdentity.startTicks))) { throw 'FIXTURE_STOP_PROCESS_CHANGED' }
        & $pgCtl -D $cluster -m fast -w -t 30 stop *> (Join-Path $runRoot 'pgctl-stop.log')
        $result.fixtureStopped = ($LASTEXITCODE -eq 0 -and -not (Test-Path -LiteralPath $pidPath))
        if (-not $result.fixtureStopped) { $result.status = 'FAIL'; $result.error = 'FIXTURE_STOP_FAILED' }
    }
    $result.finishedAt = [DateTimeOffset]::UtcNow.ToString('o')
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'harness-result.json') -Encoding utf8NoBOM
    Write-Output ($result | ConvertTo-Json -Compress)
    if ($result.Contains('fixtureStopped') -and -not $result.fixtureStopped) { throw 'FIXTURE_STOP_FAILED' }
}
