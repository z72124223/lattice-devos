#Requires -Version 7.5
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Start','Status','Stop')][string]$Action,
    [Parameter(Mandatory)][string]$StatePath,
    [Parameter(Mandatory)][string]$ClusterPath,
    [string]$PostgresBin = 'C:\Program Files\PostgreSQL\17\bin'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
function Safe-Path([string]$Value, [switch]$MissingLeaf) {
    if (-not [IO.Path]::IsPathFullyQualified($Value)) { throw 'SHARED_POSTGRES_PATH_INVALID' }
    $full = [IO.Path]::GetFullPath($Value)
    $current = if ($MissingLeaf) { [IO.Path]::GetDirectoryName($full) } else { $full }
    while ($current) {
        if ((Get-Item -LiteralPath $current -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'SHARED_POSTGRES_PATH_ALIAS' }
        $current = [IO.Path]::GetDirectoryName($current)
    }
    return $full
}
function Cluster-Identity([string]$Root) {
    if ((Get-Content -LiteralPath (Join-Path $Root 'PG_VERSION') -Raw).Trim() -cne '17') { throw 'SHARED_POSTGRES_VERSION_UNSUPPORTED' }
    $control = Safe-Path (Join-Path $Root 'global\pg_control')
    $stream = [IO.File]::Open($control, 'Open', 'Read', 'ReadWrite')
    try { $reader = [IO.BinaryReader]::new($stream); return $reader.ReadUInt64().ToString() } finally { $stream.Dispose() }
}
function Process-Identity([int]$Number, [string]$Executable, [string]$Root) {
    $p = Get-CimInstance Win32_Process -Filter "ProcessId=$Number"
    if (-not $p -or $p.ExecutablePath -ine $Executable) { throw 'SHARED_POSTGRES_PROCESS_MISMATCH' }
    $rootPattern = [regex]::Escape($Root)
    $commandLine = $p.CommandLine.Replace('/','\')
    if ($commandLine -notmatch ('(?i)(?:^|\s)-D\s+(?:"'+$rootPattern+'"|'+$rootPattern+'(?=\s|$))')) { throw 'SHARED_POSTGRES_CLUSTER_PROCESS_MISMATCH' }
    $owner = Invoke-CimMethod -InputObject $p -MethodName GetOwnerSid
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if ($owner.ReturnValue -ne 0 -or $owner.Sid -cne $sid) { throw 'SHARED_POSTGRES_OWNER_MISMATCH' }
    return [ordered]@{pid=$Number;createdAt=$p.CreationDate.ToUniversalTime().ToString('o');executable=$p.ExecutablePath;ownerSid=$sid}
}
function Save-State($Value, [switch]$First) {
    $destination = if ($First) { $StatePath } else { $StatePath+'.next.'+[guid]::NewGuid().ToString('N') }
    $stream=[IO.File]::Open($destination,'CreateNew','Write','None')
    try {$bytes=[Text.Encoding]::UTF8.GetBytes(($Value|ConvertTo-Json -Depth 5));$stream.Write($bytes);$stream.Flush($true)} finally {$stream.Dispose()}
    if(-not $First){[IO.File]::Move($destination,$StatePath,$true)}
}
$StatePath = Safe-Path $StatePath -MissingLeaf
$ClusterPath = Safe-Path $ClusterPath
$PostgresBin = Safe-Path $PostgresBin
$postgres = Safe-Path (Join-Path $PostgresBin 'postgres.exe')
$ctl = Safe-Path (Join-Path $PostgresBin 'pg_ctl.exe')
if ($Action -eq 'Start') {
    if (Test-Path -LiteralPath $StatePath) { throw 'SHARED_POSTGRES_STATE_EXISTS' }
    $systemId = Cluster-Identity $ClusterPath
    $listen = & $postgres -D $ClusterPath -C listen_addresses
    if ($LASTEXITCODE -ne 0 -or $listen.Trim() -notin @('127.0.0.1','localhost')) { throw 'SHARED_POSTGRES_LOOPBACK_REQUIRED' }
    & $ctl -D $ClusterPath status *> $null
    if ($LASTEXITCODE -eq 0) { throw 'SHARED_POSTGRES_ALREADY_RUNNING_UNOWNED' }
    if ($LASTEXITCODE -ne 3) { throw 'SHARED_POSTGRES_STATUS_FAILED' }
    $state=[ordered]@{schema='lattice.shared-postgres.v1';status='STARTING';cluster=$ClusterPath;systemId=$systemId;pgCtl=$ctl;postgres=$postgres;identity=$null;automaticRestart=$false}
    Save-State $state -First
    # pg_ctl waits only for readiness, then exits. PostgreSQL redirects its own
    # output to this exact file; no persistent Codex command holds it open.
    $arguments = '-D "{0}" -l "{1}" -w -t 30 start' -f $ClusterPath, ($StatePath+'.postgres.log')
    $launcher = Start-Process -FilePath $ctl -ArgumentList $arguments -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput ($StatePath+'.start.log') -RedirectStandardError ($StatePath+'.start.err.log')
    # Wait for pg_ctl itself, not PowerShell's descendant process tree.
    if (-not $launcher.WaitForExit(35000) -or $launcher.ExitCode -ne 0) { $state.status='START_OUTCOME_UNKNOWN';Save-State $state;throw 'SHARED_POSTGRES_START_FAILED' }
    try {
        $pidFile = Safe-Path (Join-Path $ClusterPath 'postmaster.pid')
        $number = [int](Get-Content -LiteralPath $pidFile -TotalCount 1)
        $state.identity = Process-Identity $number $postgres $ClusterPath
        if ((Cluster-Identity $ClusterPath) -cne $systemId) { throw 'SHARED_POSTGRES_CLUSTER_CHANGED' }
    } catch { $state.status='START_OUTCOME_UNKNOWN';Save-State $state;throw }
    $state.status='RUNNING'; Save-State $state
} else {
    $null = Safe-Path $StatePath
    $state = Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json -AsHashtable -DateKind String
    if ($state.schema -cne 'lattice.shared-postgres.v1') { throw 'SHARED_POSTGRES_STATE_INVALID' }
    # Scope and binaries come from explicit arguments, never from a mutable receipt.
    if($state.cluster -ine $ClusterPath -or $state.pgCtl -ine $ctl -or $state.postgres -ine $postgres){throw 'SHARED_POSTGRES_STATE_SCOPE_CHANGED'}
    if ((Cluster-Identity $state.cluster) -cne $state.systemId) { throw 'SHARED_POSTGRES_CLUSTER_CHANGED' }
    if ($state.status -eq 'STOPPED') {
        & $ctl -D $ClusterPath status *> $null
        if($LASTEXITCODE -ne 3){throw 'SHARED_POSTGRES_STOP_READBACK_FAILED'}
        $state|ConvertTo-Json -Depth 5; return
    }
    if ($state.status -eq 'STOP_PENDING') {
        & $ctl -D $ClusterPath status *> $null
        if($LASTEXITCODE -eq 3 -and -not (Test-Path -LiteralPath (Join-Path $ClusterPath 'postmaster.pid'))){
            $priorProcess=Get-CimInstance Win32_Process -Filter ('ProcessId='+$state.identity.pid)
            if($priorProcess -and $priorProcess.CreationDate.ToUniversalTime().ToString('o') -ceq $state.identity.createdAt){throw 'SHARED_POSTGRES_STOP_READBACK_FAILED'}
            $state.status='STOPPED';$state.stoppedAt=[DateTime]::UtcNow.ToString('o');Save-State $state
            $state|ConvertTo-Json -Depth 5;return
        }
    }
    if ($state.status -notin @('RUNNING','STOP_PENDING') -or -not $state.identity) { throw 'SHARED_POSTGRES_STATE_UNVERIFIED' }
    $observed = Process-Identity ([int]$state.identity.pid) $state.postgres $state.cluster
    foreach ($key in @('pid','createdAt','executable','ownerSid')) { if ($observed[$key] -cne $state.identity[$key]) { throw 'SHARED_POSTGRES_IDENTITY_CHANGED' } }
    if ([int](Get-Content -LiteralPath (Safe-Path (Join-Path $state.cluster 'postmaster.pid')) -TotalCount 1) -ne $state.identity.pid) { throw 'SHARED_POSTGRES_IDENTITY_CHANGED' }
    if ($Action -eq 'Stop') {
        # Explicit stop only; smart mode waits for clients instead of killing them.
        $state.status='STOP_PENDING';Save-State $state
        & $ctl -D $ClusterPath -m smart -w -t 30 stop *> ($StatePath+'.stop.log')
        if ($LASTEXITCODE -ne 0) { throw 'SHARED_POSTGRES_STOP_INCOMPLETE' }
        & $ctl -D $ClusterPath status *> $null
        if ($LASTEXITCODE -ne 3) { throw 'SHARED_POSTGRES_STOP_READBACK_FAILED' }
        $state.status='STOPPED'; $state.stoppedAt=[DateTime]::UtcNow.ToString('o'); Save-State $state
    }
}
$state | ConvertTo-Json -Depth 5
