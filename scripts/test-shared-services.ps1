#Requires -Version 7.5
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Start','Verify','Stop')][string]$Action,
    [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{32}$')][string]$RunId,
    [Parameter(Mandatory)][string]$NodePath,
    [string]$PostgresBin = 'C:\Program Files\PostgreSQL\17\bin'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Join-Path $env:LOCALAPPDATA ('LATTICE\shared-service-tests\'+$RunId)
$marker = Join-Path $root 'fixture.json'
$controlState = Join-Path $root 'control.json'
$postgresState = Join-Path $root 'postgres.json'
$entry = Join-Path $PSScriptRoot '..\apps\lattice-control\src\shared-control.mjs'
$pgScript = Join-Path $PSScriptRoot 'LatticeSharedPostgres.ps1'
$psql = Join-Path $PostgresBin 'psql.exe'
function Query([string]$Sql) {
    $value = & $psql -X -A -t -h 127.0.0.1 -p $fixture.port -U lifecycle_fixture -d postgres -v ON_ERROR_STOP=1 -c $Sql
    if ($LASTEXITCODE -ne 0) { throw 'FIXTURE_QUERY_FAILED' }
    return ($value -join "`n").Trim()
}
if ($Action -eq 'Start') {
    if (Test-Path -LiteralPath $root) { throw 'FIXTURE_ALREADY_EXISTS' }
    New-Item -ItemType Directory -Path $root | Out-Null
    $listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
    $listener.Start();$port=$listener.LocalEndpoint.Port;$listener.Stop()
    if ($port -in @(4317,5432,50214,58743)) { throw 'FIXTURE_PORT_REJECTED' }
    $cluster=Join-Path $root 'cluster'
    $fixture=[ordered]@{schema='lattice.shared-services-fixture.v1';runId=$RunId;port=$port;cluster=$cluster;status='STARTING';control=$null;postgres=$null}
    $fixture|ConvertTo-Json -Depth 8|Set-Content -LiteralPath $marker -Encoding utf8NoBOM
    # Synthetic data only, on a new loopback-only cluster, matching other PG fixtures.
    & (Join-Path $PostgresBin 'initdb.exe') -D $cluster -U lifecycle_fixture --auth-host=trust --auth-local=trust --encoding=UTF8 --no-locale --data-checksums *> (Join-Path $root 'initdb.log')
    if ($LASTEXITCODE -ne 0) { throw 'FIXTURE_INITDB_FAILED' }
    "listen_addresses='127.0.0.1'`nport=$port`nmax_connections=15`nshared_buffers='16MB'" | Add-Content -LiteralPath (Join-Path $cluster 'postgresql.conf') -Encoding utf8NoBOM
    $fixture.postgres = (& $pgScript -Action Start -StatePath $postgresState -ClusterPath $cluster -PostgresBin $PostgresBin | ConvertFrom-Json -AsHashtable -DateKind String).identity
    $null=Query ("CREATE TABLE lifecycle_probe (run_id text PRIMARY KEY); INSERT INTO lifecycle_probe VALUES ('"+$RunId+"')")
    Set-Content -LiteralPath (Join-Path $root 'isolated.toml') -Value '# isolated fixture; production runtime is not configured' -Encoding utf8NoBOM
    $launch=& (Join-Path $PSScriptRoot 'Start-LatticeSharedControl.ps1') -NodePath $NodePath -DatabasePath (Join-Path $root 'control.sqlite') -StatePath $controlState -RuntimeConfigPath (Join-Path $root 'isolated.toml') -Port 0 | ConvertFrom-Json
    $ready=$false
    for($attempt=0;$attempt -lt 30;$attempt++){
        if(Test-Path -LiteralPath $controlState){
            $record=Get-Content -LiteralPath $controlState -Raw|ConvertFrom-Json -AsHashtable -DateKind String
            if($record.status -eq 'RUNNING'){$ready=$true;break}
            if($record.status -eq 'START_FAILED'){throw 'FIXTURE_CONTROL_START_FAILED'}
        }
        Start-Sleep -Milliseconds 200
    }
    if(-not $ready -or $record.identity.pid -ne $launch.pid){throw 'FIXTURE_CONTROL_NOT_READY'}
    $fixture.control=$record.identity
    $fixture.status='RUNNING'
    $fixture|ConvertTo-Json -Depth 8|Set-Content -LiteralPath $marker -Encoding utf8NoBOM
} else {
    $fixture=Get-Content -LiteralPath $marker -Raw|ConvertFrom-Json -AsHashtable -DateKind String
    if($fixture.schema -cne 'lattice.shared-services-fixture.v1' -or $fixture.runId -cne $RunId -or $fixture.cluster -ine (Join-Path $root 'cluster')){throw 'FIXTURE_SCOPE_INVALID'}
    $pg=& $pgScript -Action Status -StatePath $postgresState | ConvertFrom-Json -AsHashtable -DateKind String
    $control=& $NodePath $entry status --state $controlState | ConvertFrom-Json -AsHashtable -DateKind String
    if($LASTEXITCODE -ne 0 -or $pg.status -ne 'RUNNING' -or $control.status -ne 'RUNNING'){throw 'FIXTURE_NOT_RUNNING'}
    foreach($field in @('pid','createdAt','executable','ownerSid')){
        if($pg.identity[$field] -cne $fixture.postgres[$field] -or $control.identity[$field] -cne $fixture.control[$field]){throw 'FIXTURE_PROCESS_CHANGED'}
    }
    if((Query 'SELECT run_id FROM lifecycle_probe') -cne $RunId){throw 'FIXTURE_DURABLE_READBACK_FAILED'}
    $http=Invoke-RestMethod ('http://127.0.0.1:'+$control.port+'/api/state')
    if(@($http.projects).Count -ne 0){throw 'FIXTURE_UNEXPECTED_PROJECTS'}
    $result=[ordered]@{schema='lattice.shared-services-verification.v1';runId=$RunId;checkedAt=[DateTime]::UtcNow.ToString('o');sameProcessInstances=$true;postgresReadback=$true;controlHttpReadback=$true;nativeArchive='NOT_ASSERTED';control=$control.identity;postgres=$pg.identity}
    if($Action -eq 'Stop'){
        $stopped=& $NodePath $entry stop --state $controlState | ConvertFrom-Json
        if($LASTEXITCODE -ne 0 -or $stopped.status -ne 'STOPPED'){throw 'FIXTURE_CONTROL_STOP_INCOMPLETE'}
        $pgStopped=& $pgScript -Action Stop -StatePath $postgresState | ConvertFrom-Json
        if($pgStopped.status -ne 'STOPPED'){throw 'FIXTURE_POSTGRES_STOP_INCOMPLETE'}
        foreach($prior in @($fixture.control,$fixture.postgres)){
            $process=Get-CimInstance Win32_Process -Filter ('ProcessId='+$prior.pid)
            if($process -and $process.CreationDate.ToUniversalTime().ToString('o') -ceq $prior.createdAt){throw 'FIXTURE_PROCESS_STILL_ALIVE'}
        }
        $result.normalStopVerified=$true
        $fixture.status='STOPPED'
        $fixture|ConvertTo-Json -Depth 8|Set-Content -LiteralPath $marker -Encoding utf8NoBOM
    }
    $result|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $root ($Action.ToLowerInvariant()+'-result.json')) -Encoding utf8NoBOM
}
[ordered]@{runId=$RunId;action=$Action;status=$fixture.status;root=$root}|ConvertTo-Json -Compress
