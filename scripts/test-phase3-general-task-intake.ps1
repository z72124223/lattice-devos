#requires -Version 7.0

[CmdletBinding()]
param(
    [string]$BinaryPath,

    [ValidateRange(5, 300)]
    [int]$ProcessTimeoutSeconds = 90,

    [switch]$StaticSelfTestOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:RepositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$script:WrapperPath = [IO.Path]::GetFullPath((Join-Path $script:RepositoryRoot 'tools\lattice-mcp-kit\direct-stdio\Invoke-LatticeMcp.ps1'))
$script:PostgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$script:InitDb = Join-Path $script:PostgresBin 'initdb.exe'
$script:PgCtl = Join-Path $script:PostgresBin 'pg_ctl.exe'
$script:Postgres = Join-Path $script:PostgresBin 'postgres.exe'
$script:Psql = Join-Path $script:PostgresBin 'psql.exe'
$script:Netstat = Join-Path `
    ([Environment]::GetFolderPath([Environment+SpecialFolder]::System)) 'netstat.exe'
$script:ControlOrigin = 'http://127.0.0.1:4317'
$script:ProjectId = '5fbaf1af-dcf8-42fb-8327-ea3bcd7c580f'
$script:ProjectName = 'AI 劇本'
$script:ProjectCanonicalPath = [IO.Path]::GetFullPath('C:\Users\f7212\OneDrive\文件\AI 劇本')
$script:Objective = '完成角色系統'
$script:OwnerKind = 'LATTICE_PHASE3_GENERAL_TASK_INTAKE_POSTGRES_V1'
$script:ForbiddenPorts = @(4317, 5432, 55432, 58743, 64272)
$script:PsqlMaxAttempts = 3
$script:PsqlTransientRetryCount = 0
$script:LastPsqlDiagnostic = $null

function Get-Phase3RandomHex {
    param([Parameter(Mandatory = $true)][ValidateRange(1, 1024)][int]$ByteCount)

    $bytes = [byte[]]::new($ByteCount)
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Get-Phase3StringSha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return (($sha.ComputeHash($script:Utf8.GetBytes($Value)) |
            ForEach-Object { $_.ToString('x2') }) -join '')
    }
    finally {
        $sha.Dispose()
    }
}

function Get-Phase3FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-Phase3PsqlExitClass {
    param([Parameter(Mandatory = $true)][int]$ExitCode)

    switch ($ExitCode) {
        0 { 'SUCCESS' }
        1 { 'CLIENT_FATAL' }
        2 { 'CONNECTION_LOST' }
        3 { 'SQL_REJECTED' }
        default { 'UNKNOWN' }
    }
}

function Test-Phase3PsqlRetryAllowed {
    param(
        [Parameter(Mandatory = $true)][int]$ExitCode,
        [Parameter(Mandatory = $true)][ValidateRange(1, 3)][int]$Attempt
    )

    return $ExitCode -eq 2 -and $Attempt -lt $script:PsqlMaxAttempts
}

function New-Phase3PsqlDiagnostic {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][string]$Failure,
        [Parameter(Mandatory = $true)][ValidateRange(1, 3)][int]$Attempt
    )

    if ($Failure -cnotmatch '\A[A-Z][A-Z0-9_]{0,95}\z') {
        throw 'PHASE3_PSQL_FAILURE_CODE_REJECTED'
    }
    $stderr = [string]$Result.stderr
    $stderrByteCount = [long]$script:Utf8.GetByteCount($stderr)
    return [pscustomobject][ordered]@{
        schema = 'lattice.phase3.psql-diagnostic.v1'
        failure = $Failure
        attempt = $Attempt
        max_attempts = $script:PsqlMaxAttempts
        exit_code = [int]$Result.exit_code
        exit_class = Get-Phase3PsqlExitClass -ExitCode ([int]$Result.exit_code)
        stderr_byte_count = $stderrByteCount
        stderr_sha256 = Get-Phase3StringSha256 -Value $stderr
    }
}

function Write-Phase3JsonFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    [IO.File]::WriteAllText(
        [IO.Path]::GetFullPath($Path),
        (($Value | ConvertTo-Json -Depth 20) + "`n"),
        $script:Utf8
    )
}

function Get-Phase3ControlCatalogProjection {
    param([Parameter(Mandatory = $true)][string]$Content)

    try {
        $state = $Content | ConvertFrom-Json -ErrorAction Stop
        if ($null -eq $state.PSObject.Properties['projects']) {
            throw 'projects missing'
        }
        $projects = @(
            foreach ($project in @($state.projects)) {
                if ([string]$project.schema_version -cne 'lattice.control.project-catalog.v1' -or
                    [string]$project.record_kind -cne 'CONTROL_LOCAL_CATALOG' -or
                    [string]$project.registry_authority -cne 'NONE' -or
                    $null -ne $project.registry_project_id -or
                    [string]$project.control_project_id -cne [string]$project.id) {
                    continue
                }
                [pscustomobject][ordered]@{
                    id = $project.id
                    name = $project.name
                    canonical_path = $project.canonical_path
                    schema_version = $project.schema_version
                    record_kind = $project.record_kind
                    registry_authority = $project.registry_authority
                    registry_project_id = $project.registry_project_id
                    control_project_id = $project.control_project_id
                }
            }
        ) | Sort-Object -Property id, name, canonical_path, schema_version, record_kind,
            registry_authority, registry_project_id, control_project_id
        return (ConvertTo-Json -InputObject @($projects) -Depth 4 -Compress)
    }
    catch {
        throw 'PHASE3_LIVE_CONTROL_PROJECT_REJECTED'
    }
}

function ConvertTo-Phase3FailureCode {
    param([Parameter(Mandatory = $true)][string]$Message)

    if ($Message -cmatch '\A[A-Z][A-Z0-9_]{0,127}\z') {
        return $Message
    }
    return 'PHASE3_HARNESS_RUNTIME_ERROR'
}

function Assert-Phase3RegularFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw $Failure }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw $Failure }
}

function Assert-Phase3Directory {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { throw $Failure }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw $Failure }
}

function Assert-Phase3ContainedPath {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    $canonicalRoot = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $canonicalPath = [IO.Path]::GetFullPath($Path)
    if (-not $canonicalPath.StartsWith(
        $canonicalRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw $Failure
    }
    return $canonicalPath
}

function Get-Phase3ListenerPids {
    param([Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port)

    Assert-Phase3RegularFile -Path $script:Netstat -Failure 'PHASE3_NETSTAT_BINARY_MISSING'
    $environment = New-Phase3ClosedEnvironment -Values ([ordered]@{})
    $result = Invoke-Phase3Process -Executable $script:Netstat -Argument @('-ano', '-p', 'tcp') `
        -Environment $environment -StandardInput $null -TimeoutSeconds 5 `
        -Failure 'PHASE3_NETSTAT_FAILED'
    if ($result.stdout.Length -gt 16777216) { throw 'PHASE3_NETSTAT_OUTPUT_REJECTED' }

    $listenerPids = [Collections.Generic.HashSet[int]]::new()
    foreach ($line in @($result.stdout -split '\r?\n')) {
        if ($line -cnotmatch (
            '\A\s*TCP\s+(?<local>\S+):(?<local_port>[0-9]{1,5})\s+' +
            '(?<remote>\S+):(?<remote_port>[0-9]{1,5})\s+\S+\s+' +
            '(?<process_id>[0-9]+)\s*\z'
        )) {
            continue
        }
        $localPort = [int]$Matches.local_port
        $remotePort = [int]$Matches.remote_port
        $processId = [int]$Matches.process_id
        # A Windows TCP listener has the wildcard remote endpoint with port 0.
        # This avoids depending on the localized spelling of the LISTENING state.
        if ($localPort -eq $Port -and $remotePort -eq 0 -and $processId -gt 0) {
            $null = $listenerPids.Add($processId)
        }
    }
    return @($listenerPids | Sort-Object)
}

function New-Phase3AvailablePort {
    for ($attempt = 0; $attempt -lt 32; $attempt++) {
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
        try {
            $listener.Start()
            $port = [int]([Net.IPEndPoint]$listener.LocalEndpoint).Port
        }
        finally {
            $listener.Stop()
        }
        if ($port -notin $script:ForbiddenPorts -and @(Get-Phase3ListenerPids -Port $port).Count -eq 0) {
            return $port
        }
    }
    throw 'PHASE3_POSTGRES_PORT_UNAVAILABLE'
}

function New-Phase3ClosedEnvironment {
    param([Parameter(Mandatory = $true)][Collections.IDictionary]$Values)

    $environment = [ordered]@{}
    foreach ($name in @('SystemRoot', 'WINDIR', 'TEMP', 'TMP', 'PATH', 'ComSpec')) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (-not [string]::IsNullOrWhiteSpace($value)) { $environment[$name] = $value }
    }
    foreach ($entry in $Values.GetEnumerator()) {
        $environment[[string]$entry.Key] = [string]$entry.Value
    }
    $environment['NO_COLOR'] = '1'
    return $environment
}

function Invoke-Phase3Process {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Argument,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [AllowNull()][AllowEmptyString()][string]$StandardInput,
        [Parameter(Mandatory = $true)][ValidateRange(1, 600)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][string]$Failure,
        [switch]$AllowNonZeroExit,
        [switch]$DoNotWaitForRedirectEof
    )

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardInputEncoding = $script:Utf8
    $startInfo.StandardOutputEncoding = $script:Utf8
    $startInfo.StandardErrorEncoding = $script:Utf8
    $startInfo.WorkingDirectory = $script:RepositoryRoot
    foreach ($value in $Argument) { $null = $startInfo.ArgumentList.Add($value) }
    $startInfo.Environment.Clear()
    foreach ($entry in $Environment.GetEnumerator()) {
        $startInfo.Environment[[string]$entry.Key] = [string]$entry.Value
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw $Failure }
        $stdoutTask = $null
        $stderrTask = $null
        if (-not $DoNotWaitForRedirectEof) {
            $stdoutTask = $process.StandardOutput.ReadToEndAsync()
            $stderrTask = $process.StandardError.ReadToEndAsync()
        }
        if ($null -ne $StandardInput) { $process.StandardInput.Write($StandardInput) }
        $process.StandardInput.Close()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            try { $process.Kill($true) } catch {}
            try { $null = $process.WaitForExit(5000) } catch {}
            throw $Failure
        }
        $stdout = if ($DoNotWaitForRedirectEof) { '' } else { $stdoutTask.GetAwaiter().GetResult() }
        $stderr = if ($DoNotWaitForRedirectEof) { '' } else { $stderrTask.GetAwaiter().GetResult() }
        if ($process.ExitCode -ne 0 -and -not $AllowNonZeroExit) { throw $Failure }
        return [pscustomobject]@{
            exit_code = [int]$process.ExitCode
            stdout = [string]$stdout
            stderr = [string]$stderr
        }
    }
    finally {
        $process.Dispose()
    }
}

function Get-Phase3OwnerMarker {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    Assert-Phase3Directory -Path $RunRoot -Failure 'PHASE3_OWNER_ROOT_REJECTED'
    Assert-Phase3Directory -Path $DataRoot -Failure 'PHASE3_OWNER_DATA_ROOT_REJECTED'
    Assert-Phase3RegularFile -Path $MarkerPath -Failure 'PHASE3_OWNER_MARKER_REJECTED'
    $marker = try {
        [Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($MarkerPath)) |
            ConvertFrom-Json -ErrorAction Stop
    }
    catch { throw 'PHASE3_OWNER_MARKER_REJECTED' }
    if (
        [string]$marker.owner -cne $script:OwnerKind -or
        [string]$marker.run_id -cne $RunId -or
        [string]$marker.root -cne [IO.Path]::GetFullPath($RunRoot) -or
        [string]$marker.data_root -cne [IO.Path]::GetFullPath($DataRoot) -or
        [int]$marker.port -ne $Port -or
        [string]$marker.postgres_executable -cne [IO.Path]::GetFullPath($script:Postgres) -or
        [string]$marker.postgres_sha256 -cne (Get-Phase3FileSha256 -Path $script:Postgres)
    ) {
        throw 'PHASE3_OWNER_MARKER_REJECTED'
    }
    return $marker
}

function Write-Phase3OwnerMarker {
    param(
        [Parameter(Mandatory = $true)][string]$MarkerPath,
        [Parameter(Mandatory = $true)]$Marker
    )

    Write-Phase3JsonFile -Path $MarkerPath -Value $Marker
    Assert-Phase3RegularFile -Path $MarkerPath -Failure 'PHASE3_OWNER_MARKER_WRITE_REJECTED'
}

function Start-Phase3Postgres {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    $marker = Get-Phase3OwnerMarker -RunRoot $RunRoot -RunId $RunId -Port $Port `
        -DataRoot $DataRoot -MarkerPath $MarkerPath
    if (@(Get-Phase3ListenerPids -Port $Port).Count -ne 0) {
        throw 'PHASE3_POSTGRES_PORT_COLLISION'
    }
    $postgresLog = Assert-Phase3ContainedPath -Root $RunRoot `
        -Path (Join-Path $RunRoot 'postgres.log') -Failure 'PHASE3_POSTGRES_LOG_REJECTED'
    $options = "-p $Port -h 127.0.0.1 -c ssl=off -c fsync=on -c synchronous_commit=on " +
        '-c full_page_writes=on -c max_prepared_transactions=0'
    $environment = New-Phase3ClosedEnvironment -Values ([ordered]@{})
    $null = Invoke-Phase3Process -Executable $script:PgCtl -Argument @(
        '-D', $DataRoot, '-l', $postgresLog, '-o', $options, '-W', 'start'
    ) -Environment $environment -StandardInput $null -TimeoutSeconds 60 `
        -Failure 'PHASE3_POSTGRES_START_FAILED' -DoNotWaitForRedirectEof

    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(15)
    $process = $null
    $listeners = @()
    $postgresPid = $null
    do {
        $postmasterPath = Join-Path $DataRoot 'postmaster.pid'
        if (Test-Path -LiteralPath $postmasterPath -PathType Leaf) {
            $pidText = (Get-Content -LiteralPath $postmasterPath -TotalCount 1).Trim()
            if ($pidText -match '\A[1-9][0-9]*\z') {
                $postgresPid = [int]$pidText
                $process = Get-Process -Id $postgresPid -ErrorAction SilentlyContinue
                $listeners = @(Get-Phase3ListenerPids -Port $Port)
                if ($null -ne $process -and $listeners.Count -eq 1 -and $listeners[0] -eq $postgresPid) {
                    break
                }
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTimeOffset]::UtcNow -lt $deadline)

    if ($null -eq $process -or $listeners.Count -ne 1 -or $listeners[0] -ne $postgresPid -or
        [IO.Path]::GetFullPath($process.Path) -cne [IO.Path]::GetFullPath($script:Postgres)) {
        throw 'PHASE3_POSTGRES_OWNERSHIP_REJECTED'
    }
    $startTicks = $process.StartTime.ToUniversalTime().Ticks
    $marker.state = 'RUNNING'
    $marker.process_id = $postgresPid
    $marker.process_start_utc_ticks = $startTicks
    Write-Phase3OwnerMarker -MarkerPath $MarkerPath -Marker $marker
    return [pscustomobject]@{ process_id = $postgresPid; process_start_utc_ticks = $startTicks }
}

function Assert-Phase3OwnedLivePostgres {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    $marker = Get-Phase3OwnerMarker -RunRoot $RunRoot -RunId $RunId -Port $Port `
        -DataRoot $DataRoot -MarkerPath $MarkerPath
    $postmasterPath = Join-Path $DataRoot 'postmaster.pid'
    Assert-Phase3RegularFile -Path $postmasterPath -Failure 'PHASE3_POSTGRES_OWNERSHIP_REJECTED'
    $pidText = (Get-Content -LiteralPath $postmasterPath -TotalCount 1).Trim()
    if ($pidText -cnotmatch '\A[1-9][0-9]*\z') { throw 'PHASE3_POSTGRES_OWNERSHIP_REJECTED' }
    $postgresPid = [int]$pidText
    $process = Get-Process -Id $postgresPid -ErrorAction SilentlyContinue
    $listeners = @(Get-Phase3ListenerPids -Port $Port)
    if (
        $null -eq $process -or
        [IO.Path]::GetFullPath($process.Path) -cne [IO.Path]::GetFullPath($script:Postgres) -or
        $listeners.Count -ne 1 -or $listeners[0] -ne $postgresPid
    ) {
        throw 'PHASE3_POSTGRES_OWNERSHIP_REJECTED'
    }
    $markerStateAccepted = [string]$marker.state -ceq 'RUNNING' -or (
        [string]$marker.state -ceq 'CREATED' -and
        $null -eq $marker.process_id -and
        $null -eq $marker.process_start_utc_ticks
    )
    $materializedMarkerMatches = [string]$marker.state -cne 'RUNNING' -or (
        [int]$marker.process_id -eq $postgresPid -and
        [long]$marker.process_start_utc_ticks -eq $process.StartTime.ToUniversalTime().Ticks
    )
    if (
        -not $markerStateAccepted -or -not $materializedMarkerMatches
    ) {
        throw 'PHASE3_POSTGRES_OWNERSHIP_REJECTED'
    }
    if ([string]$marker.state -ceq 'CREATED') {
        $marker.state = 'RUNNING'
        $marker.process_id = $postgresPid
        $marker.process_start_utc_ticks = $process.StartTime.ToUniversalTime().Ticks
        Write-Phase3OwnerMarker -MarkerPath $MarkerPath -Marker $marker
    }
    return [pscustomobject]@{ marker = $marker; process = $process }
}

function Stop-Phase3Postgres {
    param(
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    $owned = Assert-Phase3OwnedLivePostgres -RunRoot $RunRoot -RunId $RunId -Port $Port `
        -DataRoot $DataRoot -MarkerPath $MarkerPath
    $environment = New-Phase3ClosedEnvironment -Values ([ordered]@{})
    $null = Invoke-Phase3Process -Executable $script:PgCtl -Argument @(
        '-D', $DataRoot, '-m', 'fast', '-w', 'stop'
    ) -Environment $environment -StandardInput $null -TimeoutSeconds 60 `
        -Failure 'PHASE3_POSTGRES_STOP_FAILED'
    if (@(Get-Phase3ListenerPids -Port $Port).Count -ne 0 -or
        $null -ne (Get-Process -Id $owned.process.Id -ErrorAction SilentlyContinue)) {
        throw 'PHASE3_POSTGRES_STOP_INCOMPLETE'
    }
    $marker = $owned.marker
    $marker.state = 'STOPPED'
    Write-Phase3OwnerMarker -MarkerPath $MarkerPath -Marker $marker
}

function Invoke-Phase3Psql {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][int]$Port,
        [Parameter(Mandatory = $true)][string]$Database,
        [Parameter(Mandatory = $true)][string]$Sql,
        [Parameter(Mandatory = $true)][string]$Failure,
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$DataRoot,
        [Parameter(Mandatory = $true)][string]$MarkerPath
    )

    if ($Failure -cnotmatch '\A[A-Z][A-Z0-9_]{0,95}\z') {
        throw 'PHASE3_PSQL_FAILURE_CODE_REJECTED'
    }
    $environment = New-Phase3ClosedEnvironment -Values ([ordered]@{
        PGPASSWORD = $Password
        PGCONNECT_TIMEOUT = '10'
    })
    for ($attempt = 1; $attempt -le $script:PsqlMaxAttempts; $attempt++) {
        $null = Assert-Phase3OwnedLivePostgres -RunRoot $RunRoot -RunId $RunId -Port $Port `
            -DataRoot $DataRoot -MarkerPath $MarkerPath
        $result = Invoke-Phase3Process -Executable $script:Psql -Argument @(
            '--no-psqlrc', '--no-password', '--quiet', '--tuples-only', '--no-align',
            '--host', '127.0.0.1', '--port', [string]$Port, '--username', 'runtime_bootstrap',
            '--dbname', $Database, '--set', 'ON_ERROR_STOP=1'
        ) -Environment $environment -StandardInput ($Sql + "`n") -TimeoutSeconds 60 `
            -Failure $Failure -AllowNonZeroExit
        if ([int]$result.exit_code -eq 0) {
            return $result.stdout.Trim()
        }

        $diagnostic = New-Phase3PsqlDiagnostic -Result $result -Failure $Failure -Attempt $attempt
        $script:LastPsqlDiagnostic = $diagnostic
        if (Test-Phase3PsqlRetryAllowed -ExitCode ([int]$result.exit_code) -Attempt $attempt) {
            $script:PsqlTransientRetryCount++
            Start-Sleep -Milliseconds (100 * $attempt)
            continue
        }

        throw ($Failure + '_PSQL_' + [string]$diagnostic.exit_class)
    }
    throw ($Failure + '_PSQL_CONNECTION_LOST')
}

function Get-Phase3StrictJsonLines {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    Assert-Phase3RegularFile -Path $Path -Failure $Failure
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -eq 0 -or $bytes.Length -gt 1048576 -or
        ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf)) {
        throw $Failure
    }
    try { $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes) }
    catch { throw $Failure }
    if (-not $text.EndsWith("`n", [StringComparison]::Ordinal) -or $text.Contains("`r")) {
        throw $Failure
    }
    $lines = @($text.Split("`n", [StringSplitOptions]::None))
    if ($lines[-1] -cne '') { throw $Failure }
    $records = [Collections.Generic.List[object]]::new()
    foreach ($line in $lines[0..($lines.Count - 2)]) {
        if ($line -ceq '') { throw $Failure }
        try { $records.Add(($line | ConvertFrom-Json -ErrorAction Stop)) }
        catch { throw $Failure }
    }
    return @($records)
}

function Get-Phase3WrapperSession {
    param(
        [Parameter(Mandatory = $true)][string]$Role,
        [Parameter(Mandatory = $true)][ValidateSet('Discovery', 'TaskSubmit', 'TaskStatus')][string]$Action,
        [Parameter(Mandatory = $true)][string]$RunRoot,
        [Parameter(Mandatory = $true)][string]$Latticed,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$RuntimeEnvironment,
        [AllowNull()][string]$ClientRequestId,
        [AllowNull()][string]$Objective,
        [AllowNull()][string]$ProjectId,
        [AllowNull()][string]$ProjectName,
        [AllowNull()][string]$TaskRef
    )

    $sessionRoot = Assert-Phase3ContainedPath -Root $RunRoot -Path (Join-Path $RunRoot ('mcp-' + $Role)) `
        -Failure 'PHASE3_MCP_SESSION_ROOT_REJECTED'
    if (Test-Path -LiteralPath $sessionRoot) { throw 'PHASE3_MCP_SESSION_ROOT_REJECTED' }
    [IO.Directory]::CreateDirectory($sessionRoot) | Out-Null
    $acceptancePath = Join-Path $sessionRoot 'acceptance.jsonl'
    $effectPath = Join-Path $sessionRoot 'observed-effects.jsonl'
    $environmentPath = Join-Path $sessionRoot 'environment.json'
    foreach ($path in @($acceptancePath, $effectPath)) {
        $stream = [IO.File]::Open($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $stream.Dispose()
    }
    $sessionId = Get-Phase3RandomHex -ByteCount 16
    $nonce = Get-Phase3RandomHex -ByteCount 32
    $safeConfig = Get-Phase3StringSha256 -Value (
        'lattice.phase3.general-task-intake.evidence.v1' + "`n" + $Role + "`n" +
        [string]$RuntimeEnvironment.LATTICE_TASK019_RUN_ID + "`n" +
        [string]$RuntimeEnvironment.LATTICE_TASK019_PORT + "`n" + (Get-Phase3FileSha256 -Path $Latticed)
    )
    $environment = [ordered]@{}
    foreach ($entry in $RuntimeEnvironment.GetEnumerator()) {
        $environment[[string]$entry.Key] = [string]$entry.Value
    }
    $environment['LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH'] = $acceptancePath
    $environment['LATTICE_MCP_ACCEPTANCE_SESSION_ID'] = $sessionId
    $environment['LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256'] = $safeConfig
    $environment['LATTICE_MCP_OBSERVED_EFFECT_PATH'] = $effectPath
    $environment['LATTICE_MCP_OBSERVED_EFFECT_NONCE'] = $nonce
    Write-Phase3JsonFile -Path $environmentPath -Value $environment

    $parameters = [ordered]@{
        BinaryPath = $Latticed
        EnvironmentFile = $environmentPath
        Action = $Action
        TimeoutSeconds = $ProcessTimeoutSeconds
        ToolCallTimeoutSeconds = $ProcessTimeoutSeconds
        OutputDirectory = (Join-Path $sessionRoot 'wrapper-output')
    }
    if ($PSBoundParameters.ContainsKey('ClientRequestId')) {
        $parameters.ClientRequestId = $ClientRequestId
    }
    if ($PSBoundParameters.ContainsKey('Objective')) { $parameters.Objective = $Objective }
    if ($PSBoundParameters.ContainsKey('ProjectId')) { $parameters.ProjectId = $ProjectId }
    if ($PSBoundParameters.ContainsKey('ProjectName')) { $parameters.ProjectName = $ProjectName }
    if ($PSBoundParameters.ContainsKey('TaskRef')) { $parameters.TaskRef = $TaskRef }
    $output = @(& $script:WrapperPath @parameters)
    $summaryText = ($output | Out-String).Trim()
    if ([string]::IsNullOrWhiteSpace($summaryText)) { throw 'PHASE3_MCP_SUMMARY_MISSING' }
    try { $summary = $summaryText | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE3_MCP_SUMMARY_REJECTED' }
    if ([string]$summary.schema -cne 'lattice.direct-stdio-client.v2') {
        throw 'PHASE3_MCP_SUMMARY_SCHEMA_REJECTED'
    }
    if ([string]$summary.action -cne $Action) { throw 'PHASE3_MCP_SUMMARY_ACTION_REJECTED' }
    if (-not [bool]$summary.process.started) {
        $setupFailure = ConvertTo-Phase3FailureCode -Message ([string]$summary.failure_message)
        if ($setupFailure -cne 'PHASE3_HARNESS_RUNTIME_ERROR') { throw $setupFailure }
        throw 'PHASE3_MCP_PROCESS_NOT_STARTED'
    }
    if ([int]$summary.process.exit_code -ne 0) { throw 'PHASE3_MCP_PROCESS_FAILED' }

    $stdoutPath = Assert-Phase3ContainedPath -Root $sessionRoot -Path ([string]$summary.artifacts.stdout) `
        -Failure 'PHASE3_MCP_ARTIFACT_REJECTED'
    $responses = Get-Phase3StrictJsonLines -Path $stdoutPath -Failure 'PHASE3_MCP_STDOUT_REJECTED'
    $toolsResponses = @($responses | Where-Object { [string]$_.id -ceq '2' })
    if ($toolsResponses.Count -ne 1 -or $null -eq $toolsResponses[0].result.tools) {
        throw 'PHASE3_MCP_DISCOVERY_REJECTED'
    }

    $acceptance = Get-Phase3StrictJsonLines -Path $acceptancePath `
        -Failure 'PHASE3_MCP_ACCEPTANCE_EVIDENCE_REJECTED'
    $effects = Get-Phase3StrictJsonLines -Path $effectPath `
        -Failure 'PHASE3_MCP_EFFECT_EVIDENCE_REJECTED'
    if ([string]$acceptance[0].record_type -cne 'SESSION_OPEN' -or
        [string]$acceptance[-1].record_type -cne 'SESSION_CLOSED' -or
        [string]$effects[0].record_type -cne 'SESSION_OPEN' -or
        [string]$effects[-1].record_type -cne 'SESSION_CLOSED') {
        throw 'PHASE3_MCP_EVIDENCE_NOT_CLOSED'
    }
    foreach ($record in $effects) {
        if ([string]$record.schema -cne 'lattice.mcp.observed-effect.v1' -or
            [string]$record.session_id -cne $sessionId -or
            [string]$record.safe_config_sha256 -cne $safeConfig -or
            [long]$record.session_counters.codex -ne 0) {
            throw 'PHASE3_CODEX_EFFECT_REJECTED'
        }
    }
    $counters = $effects[-1].session_counters
    $expectedDispatch = if ($Action -ceq 'Discovery') { 0 } else { 1 }
    if ([long]$counters.dispatch -ne $expectedDispatch) { throw 'PHASE3_EFFECT_DISPATCH_REJECTED' }

    return [pscustomobject]@{
        role = $Role
        summary = $summary
        responses = $responses
        tools = @($toolsResponses[0].result.tools)
        effect_counters = [pscustomobject]@{
            dispatch = [long]$counters.dispatch
            database = [long]$counters.database
            filesystem = [long]$counters.filesystem
            process = [long]$counters.process
            network = [long]$counters.network
            codex = [long]$counters.codex
        }
    }
}

function Get-Phase3StructuredContent {
    param(
        [Parameter(Mandatory = $true)]$Session,
        [Parameter(Mandatory = $true)][string]$Failure
    )

    $calls = @($Session.responses | Where-Object { [string]$_.id -ceq '3' })
    if ($calls.Count -ne 1 -or [bool]$calls[0].result.isError -or
        $null -eq $calls[0].result.structuredContent) {
        throw $Failure
    }
    return $calls[0].result.structuredContent
}

function Get-Phase3ToolErrorCode {
    param([Parameter(Mandatory = $true)]$Session)

    $calls = @($Session.responses | Where-Object { [string]$_.id -ceq '3' })
    if ($calls.Count -ne 1 -or -not [bool]$calls[0].result.isError) {
        throw 'PHASE3_EXPECTED_TOOL_ERROR_MISSING'
    }
    $candidates = [Collections.Generic.List[string]]::new()
    if ($null -ne $Session.summary.call -and
        $Session.summary.call.PSObject.Properties.Name -contains 'error_code') {
        $candidates.Add([string]$Session.summary.call.error_code)
    }
    $structured = $calls[0].result.structuredContent
    if ($null -ne $structured) {
        foreach ($name in @('code', 'error_code', 'errorCode')) {
            if ($structured.PSObject.Properties.Name -contains $name) {
                $candidates.Add([string]$structured.$name)
            }
        }
    }
    foreach ($candidate in $candidates) {
        if ($candidate -cmatch '\A[A-Z][A-Z0-9_]{0,127}\z') { return $candidate }
    }
    throw 'PHASE3_TOOL_ERROR_CODE_MISSING'
}

function Assert-Phase3GeneralSubmitSchema {
    param([Parameter(Mandatory = $true)]$Session)

    $tools = @($Session.tools | Where-Object { [string]$_.name -ceq 'lattice_task_submit' })
    if ($tools.Count -ne 1) { throw 'PHASE3_GENERAL_SCHEMA_REJECTED' }
    $variants = @($tools[0].inputSchema.oneOf)
    if ($variants.Count -ne 3) { throw 'PHASE3_GENERAL_SCHEMA_REJECTED' }
    $canary = @($variants | Where-Object {
        if (@($_.required) -notcontains 'intent') { return $false }
        $intentProperty = $_.properties.PSObject.Properties['intent']
        if ($null -eq $intentProperty) { return $false }
        $enumProperty = $intentProperty.Value.PSObject.Properties['enum']
        $null -ne $enumProperty -and @($enumProperty.Value).Count -eq 1 -and
            [string]$enumProperty.Value[0] -ceq 'CONTROLLED_CODEX_CANARY'
    })
    $objective = @($variants | Where-Object {
        @($_.required) -contains 'objective' -and
        $_.properties.PSObject.Properties.Name -contains 'project_id' -and
        $_.properties.PSObject.Properties.Name -contains 'project_name'
    })
    $generalIntent = @($variants | Where-Object {
        if (@($_.required) -notcontains 'intent') { return $false }
        $intentProperty = $_.properties.PSObject.Properties['intent']
        $null -ne $intentProperty -and
            $intentProperty.Value.PSObject.Properties.Name -contains 'not' -and
            $_.properties.PSObject.Properties.Name -contains 'project_id' -and
            $_.properties.PSObject.Properties.Name -contains 'project_name'
    })
    if ($canary.Count -ne 1 -or $objective.Count -ne 1 -or $generalIntent.Count -ne 1 -or
        [int]$objective[0].properties.objective.maxLength -ne 512 -or
        [bool]$objective[0].additionalProperties -or
        [bool]$generalIntent[0].additionalProperties) {
        throw 'PHASE3_GENERAL_SCHEMA_REJECTED'
    }
}

function Assert-Phase3SubmittedStatus {
    param(
        [Parameter(Mandatory = $true)]$Status,
        [AllowNull()][string]$ExpectedTaskRef
    )

    if (
        [string]$Status.schema_version -cne 'lattice.task.status.v3' -or
        [string]$Status.status -cne 'SUBMITTED' -or
        [string]$Status.task_state -cne 'DRAFT' -or
        [string]$Status.objective -cne $script:Objective -or
        [string]$Status.project_id -cne $script:ProjectId -or
        [string]$Status.project_name -cne $script:ProjectName -or
        [string]$Status.task_ref -cnotmatch '\A[0-9a-f]{64}\z' -or
        [string]$Status.ledger_head_digest -cnotmatch '\A[0-9a-f]{64}\z' -or
        $null -ne $Status.result_digest -or $null -ne $Status.failure_stage -or
        $null -ne $Status.failure_code -or
        (-not [string]::IsNullOrEmpty($ExpectedTaskRef) -and
            [string]$Status.task_ref -cne $ExpectedTaskRef)
    ) {
        throw 'PHASE3_GENERAL_STATUS_REJECTED'
    }
}

function Test-Phase3StaticSelf {
    $tokens = $null
    $parseErrors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile(
        $PSCommandPath,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if (@($parseErrors).Count -ne 0) { throw 'PHASE3_POWERSHELL_PARSE_REJECTED' }
    $source = [IO.File]::ReadAllText($PSCommandPath, [Text.Encoding]::UTF8)
    foreach ($required in @(
        '--postgres-initialize', '--postgres-bootstrap', 'Invoke-LatticeMcp.ps1',
        'GENERAL_TASK_INTAKE', 'CONTROLLED_CODEX_CANARY', 'LATTICE_MCP_OBSERVED_EFFECT_PATH',
        'PHASE3_CODEX_EFFECT_REJECTED', 'Remove-Item -LiteralPath $deleteTarget -Recurse -Force',
        'netstat.exe', 'PHASE3_NETSTAT_FAILED', 'PHASE3_NETSTAT_OUTPUT_REJECTED',
        "'-W', 'start'", 'PSQL_CONNECTION_LOST', 'PsqlMaxAttempts = 3',
        'Test-Phase3PsqlRetryAllowed'
    )) {
        $occurrences = 0
        $offset = 0
        while (($offset = $source.IndexOf($required, $offset, [StringComparison]::Ordinal)) -ge 0) {
            $occurrences++
            $offset += $required.Length
        }
        # One occurrence is the guard literal itself; a second must exist in
        # the executable harness body.
        if ($occurrences -lt 2) {
            throw 'PHASE3_STATIC_GUARD_REJECTED'
        }
    }

    $fakePowerShell = Join-Path $PSHOME 'pwsh.exe'
    Assert-Phase3RegularFile -Path $fakePowerShell -Failure 'PHASE3_FAKE_PROCESS_BINARY_MISSING'
    $fakeEnvironment = New-Phase3ClosedEnvironment -Values ([ordered]@{})
    $expectedClasses = @('SUCCESS', 'CLIENT_FATAL', 'CONNECTION_LOST', 'SQL_REJECTED')
    for ($exitCode = 0; $exitCode -le 3; $exitCode++) {
        $fakeStderr = 'phase3-fake-stderr-' + $exitCode
        $command = "[Console]::Out.Write('phase3-fake-stdout-$exitCode'); " +
            "[Console]::Error.Write('$fakeStderr'); exit $exitCode"
        $result = Invoke-Phase3Process -Executable $fakePowerShell -Argument @(
            '-NoLogo', '-NoProfile', '-NonInteractive', '-Command', $command
        ) -Environment $fakeEnvironment -StandardInput $null -TimeoutSeconds 15 `
            -Failure 'PHASE3_FAKE_PROCESS_FAILED' -AllowNonZeroExit
        if ([int]$result.exit_code -ne $exitCode -or
            (Get-Phase3PsqlExitClass -ExitCode $exitCode) -cne $expectedClasses[$exitCode]) {
            throw 'PHASE3_FAKE_PROCESS_EXIT_CLASS_REJECTED'
        }
        $diagnostic = New-Phase3PsqlDiagnostic -Result $result `
            -Failure 'PHASE3_FAKE_PSQL_FAILED' -Attempt 1
        $diagnosticJson = $diagnostic | ConvertTo-Json -Compress
        if ([string]$diagnostic.exit_class -cne $expectedClasses[$exitCode] -or
            [long]$diagnostic.stderr_byte_count -ne $script:Utf8.GetByteCount($fakeStderr) -or
            [string]$diagnostic.stderr_sha256 -cne (Get-Phase3StringSha256 -Value $fakeStderr) -or
            $diagnosticJson.Contains($fakeStderr, [StringComparison]::Ordinal)) {
            throw 'PHASE3_FAKE_PROCESS_DIAGNOSTIC_REJECTED'
        }
    }
    if (-not (Test-Phase3PsqlRetryAllowed -ExitCode 2 -Attempt 1) -or
        -not (Test-Phase3PsqlRetryAllowed -ExitCode 2 -Attempt 2) -or
        (Test-Phase3PsqlRetryAllowed -ExitCode 2 -Attempt 3) -or
        (Test-Phase3PsqlRetryAllowed -ExitCode 1 -Attempt 1) -or
        (Test-Phase3PsqlRetryAllowed -ExitCode 3 -Attempt 1)) {
        throw 'PHASE3_PSQL_RETRY_CLOSURE_REJECTED'
    }
}

if ($StaticSelfTestOnly) {
    Test-Phase3StaticSelf
    [ordered]@{
        schema = 'lattice.phase3-general-task-intake.acceptance.v1'
        status = 'PASS'
        mode = 'STATIC_SELF_CHECK'
        acceptance = $false
        powershell_parser = 'PASS'
        runtime_executed = $false
    } | ConvertTo-Json -Compress
    return
}

$startedAt = [DateTimeOffset]::UtcNow
$runId = Get-Phase3RandomHex -ByteCount 16
$clientRequestId = 'phase3-general-' + $runId.Substring(0, 16)
$tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar)
$runRoot = [IO.Path]::GetFullPath((Join-Path $tempParent ('lattice-phase3-general-intake-' + $runId)))
$expectedRoot = [IO.Path]::GetFullPath(($tempParent + [IO.Path]::DirectorySeparatorChar +
    'lattice-phase3-general-intake-' + $runId))
$dataRoot = Join-Path $runRoot 'data'
$markerPath = Join-Path $runRoot '.phase3-owner.json'
$passwordPath = Join-Path $runRoot '.initdb-password'
$databaseName = 'lattice_task019_' + $runId.Substring(0, 8) + '_base'
$port = $null
$password = $null
$latticed = $null
$git = $null
$runtimeEnvironment = $null
$postgresRunning = $false
$runRootCreated = $false
$cleanupSucceeded = $false
$listenerAbsent = $false
$failureCode = $null
$failureLine = 0
$failureException = 'NONE'
$accepted = $null
$databaseEvidence = $null
$registryRootMatches = $false
$sessions = [Collections.Generic.List[object]]::new()
$physicalRestart = $false
$controlVerified = $false
$failureStage = 'SETUP'

try {
    Test-Phase3StaticSelf
    if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PHASE3_POWERSHELL_7_REQUIRED' }
    Assert-Phase3RegularFile -Path $script:WrapperPath -Failure 'PHASE3_MCP_WRAPPER_MISSING'
    Assert-Phase3RegularFile -Path $script:Netstat -Failure 'PHASE3_NETSTAT_BINARY_MISSING'
    foreach ($binary in @($script:InitDb, $script:PgCtl, $script:Postgres, $script:Psql)) {
        Assert-Phase3RegularFile -Path $binary -Failure 'PHASE3_POSTGRES_17_BINARY_MISSING'
    }
    if ([string]::IsNullOrWhiteSpace($BinaryPath)) {
        $latticed = [IO.Path]::GetFullPath((Join-Path $script:RepositoryRoot 'target\debug\latticed.exe'))
    }
    else {
        if (-not [IO.Path]::IsPathRooted($BinaryPath)) { throw 'PHASE3_LATTICED_PATH_REJECTED' }
        $latticed = [IO.Path]::GetFullPath($BinaryPath)
    }
    Assert-Phase3RegularFile -Path $latticed -Failure 'PHASE3_LATTICED_BINARY_MISSING'
    if ([IO.Path]::GetFileName($latticed) -cne 'latticed.exe') { throw 'PHASE3_LATTICED_PATH_REJECTED' }
    $git = [IO.Path]::GetFullPath((Get-Command git.exe -ErrorAction Stop).Source)
    Assert-Phase3RegularFile -Path $git -Failure 'PHASE3_GIT_BINARY_MISSING'

    if ($runRoot -cne $expectedRoot -or -not $runRoot.StartsWith(
        $tempParent + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase
    ) -or (Test-Path -LiteralPath $runRoot)) {
        throw 'PHASE3_RUN_ROOT_REJECTED'
    }
    $port = New-Phase3AvailablePort
    if ($port -in $script:ForbiddenPorts -or @(Get-Phase3ListenerPids -Port $port).Count -ne 0) {
        throw 'PHASE3_POSTGRES_PORT_COLLISION'
    }
    [IO.Directory]::CreateDirectory($runRoot) | Out-Null
    $runRootCreated = $true
    Assert-Phase3Directory -Path $runRoot -Failure 'PHASE3_RUN_ROOT_REJECTED'
    [IO.Directory]::CreateDirectory($dataRoot) | Out-Null

    $marker = [ordered]@{
        owner = $script:OwnerKind
        run_id = $runId
        root = $runRoot
        data_root = [IO.Path]::GetFullPath($dataRoot)
        port = $port
        postgres_executable = [IO.Path]::GetFullPath($script:Postgres)
        postgres_sha256 = Get-Phase3FileSha256 -Path $script:Postgres
        state = 'CREATED'
        process_id = $null
        process_start_utc_ticks = $null
    }
    Write-Phase3OwnerMarker -MarkerPath $markerPath -Marker $marker
    $failureStage = 'POSTGRES_INIT'
    $password = Get-Phase3RandomHex -ByteCount 32
    [IO.File]::WriteAllText($passwordPath, $password, [Text.Encoding]::ASCII)
    $initEnvironment = New-Phase3ClosedEnvironment -Values ([ordered]@{})
    $null = Invoke-Phase3Process -Executable $script:InitDb -Argument @(
        '-D', $dataRoot, '-U', 'runtime_bootstrap', '--auth-host=scram-sha-256',
        '--auth-local=trust', ('--pwfile=' + $passwordPath), '--encoding=UTF8', '--locale=C',
        '--data-checksums'
    ) -Environment $initEnvironment -StandardInput $null -TimeoutSeconds 120 `
        -Failure 'PHASE3_INITDB_FAILED'
    Remove-Item -LiteralPath $passwordPath -Force
    if (Test-Path -LiteralPath $passwordPath) { throw 'PHASE3_PASSWORD_FILE_CLEANUP_REJECTED' }

    $started = Start-Phase3Postgres -RunRoot $runRoot -RunId $RunId -Port $port `
        -DataRoot $dataRoot -MarkerPath $markerPath
    $postgresRunning = $true
    $firstPostgresStartTicks = [long]$started.process_start_utc_ticks

    $failureStage = 'CONTROL_CATALOG'
    $controlStateResponseOne = Invoke-WebRequest -Uri ($script:ControlOrigin + '/api/state') `
        -Method Get -TimeoutSec 10
    $controlStateResponseTwo = Invoke-WebRequest -Uri ($script:ControlOrigin + '/api/state') `
        -Method Get -TimeoutSec 10
    $controlCatalogOne = Get-Phase3ControlCatalogProjection `
        -Content ([string]$controlStateResponseOne.Content)
    $controlCatalogTwo = Get-Phase3ControlCatalogProjection `
        -Content ([string]$controlStateResponseTwo.Content)
    if ($controlCatalogOne -cne $controlCatalogTwo) {
        throw 'PHASE3_LIVE_CONTROL_CATALOG_CHANGED'
    }
    try {
        $controlState = [string]$controlStateResponseOne.Content |
            ConvertFrom-Json -ErrorAction Stop
    }
    catch { throw 'PHASE3_LIVE_CONTROL_PROJECT_REJECTED' }
    $controlProjects = @($controlState.projects | Where-Object { [string]$_.id -ceq $script:ProjectId })
    $controlCanonicalPath = if ($controlProjects.Count -eq 1) {
        [IO.Path]::GetFullPath([string]$controlProjects[0].canonical_path)
    }
    else {
        $null
    }
    if ($controlProjects.Count -ne 1 -or
        [string]$controlProjects[0].name -cne $script:ProjectName -or
        -not [string]::Equals(
            $controlCanonicalPath,
            $script:ProjectCanonicalPath,
            [StringComparison]::OrdinalIgnoreCase
        ) -or
        [string]$controlProjects[0].record_kind -cne 'CONTROL_LOCAL_CATALOG' -or
        [string]$controlProjects[0].registry_authority -cne 'NONE' -or
        $null -ne $controlProjects[0].registry_project_id) {
        throw 'PHASE3_LIVE_CONTROL_PROJECT_REJECTED'
    }
    $controlVerified = $true

    $authorityObservation = Get-Phase3StringSha256 -Value ('phase3-authority-observation:' + $runId)
    $authorityHead = Get-Phase3StringSha256 -Value ('phase3-authority-head:' + $runId)
    $ingressProfile = Get-Phase3StringSha256 -Value 'lattice.phase3.general-task-intake.local-acceptance.v1'
    $runtimeEnvironment = [ordered]@{
        LATTICE_FULL_CHAIN_RUN_MODE = 'RESUME_EXISTING'
        LATTICE_RUNTIME_INTEGRATION = 'CORE_ONLY'
        LATTICE_HERMES_MODE = 'TASK_ONLY'
        LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
        LATTICE_DELIVERY_TIMEOUT_SECONDS = [string]$ProcessTimeoutSeconds
        LATTICE_TASK019_HOST = '127.0.0.1'
        LATTICE_TASK019_PORT = [string]$port
        LATTICE_TASK019_RUN_ID = $runId
        LATTICE_TASK019_PASSWORD = $password
        LATTICE_STORE_DAEMON_INSTANCE_ID = ('phase3-general-' + $runId.Substring(0, 12))
        LATTICE_STORE_DAEMON_EPOCH = '303'
        LATTICE_STORE_AUTHORITY_REVISION = '303'
        LATTICE_STORE_OBSERVATION_DIGEST = $authorityObservation
        LATTICE_STORE_AUTHORITY_HEAD_DIGEST = $authorityHead
        LATTICE_TASK_INGRESS_KIND = 'LOCAL_CANONICAL_MCP_ACCEPTANCE'
        LATTICE_TASK_INGRESS_PROFILE_SHA256 = $ingressProfile
        LATTICE_CONTROL_ORIGIN = $script:ControlOrigin
        LATTICE_DELIVERY_GIT_EXE = $git
    }
    $failureStage = 'POSTGRES_BOOTSTRAP'
    $commandEnvironment = New-Phase3ClosedEnvironment -Values $runtimeEnvironment
    $null = Invoke-Phase3Process -Executable $latticed -Argument @('--postgres-initialize') `
        -Environment $commandEnvironment -StandardInput $null -TimeoutSeconds 120 `
        -Failure 'PHASE3_POSTGRES_INITIALIZE_FAILED'
    $null = Invoke-Phase3Process -Executable $latticed -Argument @('--postgres-bootstrap') `
        -Environment $commandEnvironment -StandardInput $null -TimeoutSeconds 180 `
        -Failure 'PHASE3_POSTGRES_BOOTSTRAP_FAILED'

    $failureStage = 'MCP_DISCOVERY'
    $discovery = Get-Phase3WrapperSession -Role '01-discovery' -Action Discovery `
        -RunRoot $runRoot -Latticed $latticed -RuntimeEnvironment $runtimeEnvironment
    $sessions.Add($discovery)
    if (-not [bool]$discovery.summary.success -or
        [string]$discovery.summary.classification -cne 'DISCOVERY_OK') {
        throw 'PHASE3_MCP_DISCOVERY_REJECTED'
    }
    Assert-Phase3GeneralSubmitSchema -Session $discovery

    $failureStage = 'MCP_SUBMIT'
    $submit = Get-Phase3WrapperSession -Role '02-submit' -Action TaskSubmit `
        -RunRoot $runRoot -Latticed $latticed -RuntimeEnvironment $runtimeEnvironment `
        -ClientRequestId $clientRequestId -Objective $script:Objective -ProjectId $script:ProjectId
    $sessions.Add($submit)
    if (-not [bool]$submit.summary.success -or [string]$submit.summary.classification -cne 'CALL_OK') {
        throw 'PHASE3_GENERAL_SUBMIT_FAILED'
    }
    $accepted = Get-Phase3StructuredContent -Session $submit -Failure 'PHASE3_GENERAL_SUBMIT_FAILED'
    Assert-Phase3SubmittedStatus -Status $accepted -ExpectedTaskRef $null
    $taskRef = [string]$accepted.task_ref

    $failureStage = 'PRE_RESTART_STATUS'
    $preRestartStatusSession = Get-Phase3WrapperSession -Role '03-pre-restart-status' -Action TaskStatus `
        -RunRoot $runRoot -Latticed $latticed -RuntimeEnvironment $runtimeEnvironment `
        -TaskRef $taskRef
    $sessions.Add($preRestartStatusSession)
    if (-not [bool]$preRestartStatusSession.summary.success) {
        throw 'PHASE3_PRE_RESTART_STATUS_FAILED'
    }
    $preRestartStatus = Get-Phase3StructuredContent -Session $preRestartStatusSession `
        -Failure 'PHASE3_PRE_RESTART_STATUS_FAILED'
    Assert-Phase3SubmittedStatus -Status $preRestartStatus -ExpectedTaskRef $taskRef
    if ([string]$preRestartStatus.ledger_head_digest -cne [string]$accepted.ledger_head_digest) {
        throw 'PHASE3_PRE_RESTART_STATUS_CHANGED_LEDGER'
    }

    $failureStage = 'IDEMPOTENCY_REPLAY'
    $replay = Get-Phase3WrapperSession -Role '04-exact-replay' -Action TaskSubmit `
        -RunRoot $runRoot -Latticed $latticed -RuntimeEnvironment $runtimeEnvironment `
        -ClientRequestId $clientRequestId -Objective $script:Objective -ProjectId $script:ProjectId
    $sessions.Add($replay)
    if (-not [bool]$replay.summary.success) { throw 'PHASE3_EXACT_REPLAY_FAILED' }
    $replayed = Get-Phase3StructuredContent -Session $replay -Failure 'PHASE3_EXACT_REPLAY_FAILED'
    Assert-Phase3SubmittedStatus -Status $replayed -ExpectedTaskRef $taskRef
    if ([string]$replayed.ledger_head_digest -cne [string]$accepted.ledger_head_digest) {
        throw 'PHASE3_EXACT_REPLAY_CHANGED_LEDGER'
    }

    $failureStage = 'IDEMPOTENCY_CONFLICT'
    $objectiveConflict = Get-Phase3WrapperSession -Role '05-objective-conflict' -Action TaskSubmit `
        -RunRoot $runRoot -Latticed $latticed -RuntimeEnvironment $runtimeEnvironment `
        -ClientRequestId $clientRequestId -Objective '完成角色系統（衝突）' -ProjectId $script:ProjectId
    $sessions.Add($objectiveConflict)
    if ((Get-Phase3ToolErrorCode -Session $objectiveConflict) -cne 'LATTICE_TASK_IDEMPOTENCY_CONFLICT') {
        throw 'PHASE3_OBJECTIVE_CONFLICT_NOT_REJECTED'
    }

    $projectConflict = Get-Phase3WrapperSession -Role '06-project-conflict' -Action TaskSubmit `
        -RunRoot $runRoot -Latticed $latticed -RuntimeEnvironment $runtimeEnvironment `
        -ClientRequestId $clientRequestId -Objective $script:Objective `
        -ProjectName 'phase3-conflicting-project-selector'
    $sessions.Add($projectConflict)
    if ((Get-Phase3ToolErrorCode -Session $projectConflict) -cne 'LATTICE_TASK_IDEMPOTENCY_CONFLICT') {
        throw 'PHASE3_PROJECT_CONFLICT_NOT_REJECTED'
    }

    $failureStage = 'POSTGRES_RESTART'
    $systemIdentifierBefore = Invoke-Phase3Psql -Password $password -Port $port -Database 'postgres' `
        -Sql 'SELECT system_identifier::text FROM pg_catalog.pg_control_system();' `
        -Failure 'PHASE3_POSTGRES_IDENTITY_READ_FAILED' -RunRoot $runRoot -RunId $RunId `
        -DataRoot $dataRoot -MarkerPath $markerPath
    if ($systemIdentifierBefore -cnotmatch '\A[1-9][0-9]+\z') {
        throw 'PHASE3_POSTGRES_IDENTITY_REJECTED'
    }
    Stop-Phase3Postgres -RunRoot $runRoot -RunId $RunId -Port $port `
        -DataRoot $dataRoot -MarkerPath $markerPath
    $postgresRunning = $false
    if (@(Get-Phase3ListenerPids -Port $port).Count -ne 0) {
        throw 'PHASE3_RESTART_STOP_PROOF_REJECTED'
    }
    $restarted = Start-Phase3Postgres -RunRoot $runRoot -RunId $RunId -Port $port `
        -DataRoot $dataRoot -MarkerPath $markerPath
    $postgresRunning = $true
    $systemIdentifierAfter = Invoke-Phase3Psql -Password $password -Port $port -Database 'postgres' `
        -Sql 'SELECT system_identifier::text FROM pg_catalog.pg_control_system();' `
        -Failure 'PHASE3_POSTGRES_IDENTITY_READ_FAILED' -RunRoot $runRoot -RunId $RunId `
        -DataRoot $dataRoot -MarkerPath $markerPath
    if ($systemIdentifierAfter -cne $systemIdentifierBefore -or
        [long]$restarted.process_start_utc_ticks -eq $firstPostgresStartTicks) {
        throw 'PHASE3_PHYSICAL_RESTART_REJECTED'
    }
    $physicalRestart = $true

    $failureStage = 'RESTART_STATUS'
    $status = Get-Phase3WrapperSession -Role '07-restart-status' -Action TaskStatus `
        -RunRoot $runRoot -Latticed $latticed -RuntimeEnvironment $runtimeEnvironment `
        -TaskRef $taskRef
    $sessions.Add($status)
    if (-not [bool]$status.summary.success) { throw 'PHASE3_RESTART_STATUS_FAILED' }
    $restartStatus = Get-Phase3StructuredContent -Session $status -Failure 'PHASE3_RESTART_STATUS_FAILED'
    Assert-Phase3SubmittedStatus -Status $restartStatus -ExpectedTaskRef $taskRef
    if ([string]$restartStatus.ledger_head_digest -cne [string]$accepted.ledger_head_digest) {
        throw 'PHASE3_RESTART_STATUS_CHANGED_LEDGER'
    }

    $failureStage = 'DATABASE_EVIDENCE'
    $sql = @"
SELECT pg_catalog.jsonb_build_object(
    'envelope_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_submission_envelopes x WHERE x.task_ref='$taskRef'),
    'claim_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ingress_claims x WHERE x.stream_id=s.stream_id AND x.request_kind='GENERAL_TASK'),
    'subject_kind', l.task_subject_kind::text,
    'subject_digest_matches_intake', l.task_subject_digest=s.intake_digest,
    'task_spec_digest_is_null', l.task_spec_digest IS NULL,
    'accounting_currency_is_null', l.accounting_currency IS NULL,
    'stream_sequence', l.sequence::text,
    'stream_event_count', l.event_count::text,
    'stream_command_count', l.command_count::text,
    'stream_outbox_count', l.outbox_count::text,
    'stream_resource_revision', l.resource_revision::text,
    'stream_active_agents', l.active_agents::text,
    'stream_active_implementers', l.active_implementers::text,
    'stream_used_model_calls', l.used_model_calls::text,
    'stream_used_external_cost', l.used_external_cost::text,
    'event_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id),
    'task_created_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='TASK_CREATED'),
    'resource_snapshot_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND (e.has_resource_snapshot OR e.event_kind='RESOURCE_SNAPSHOT')),
    'command_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_commands c WHERE c.stream_id=s.stream_id),
    'autonomy_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_autonomy_receipts a WHERE a.stream_id=s.stream_id),
    'outbox_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_outbox o WHERE o.stream_id=s.stream_id),
    'non_task_ledger_terminal_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_commands c JOIN ONLY control.terminal_transactions t ON t.transaction_id=c.store_transaction_id WHERE c.stream_id=s.stream_id AND t.repository_owner<>'TASK_LEDGER'),
    'task_ledger_terminal_count', (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_commands c JOIN ONLY control.terminal_transactions t ON t.transaction_id=c.store_transaction_id WHERE c.stream_id=s.stream_id AND t.repository_owner='TASK_LEDGER'),
    'objective', s.objective::text,
    'project_id', s.project_id::text,
    'project_name', s.project_display_name::text,
    'project_registry_active_count', (SELECT pg_catalog.count(*) FROM ONLY control.project_registry_projects p WHERE p.project_id=s.project_id AND p.authority_runtime='LIVE' AND p.authority_lifecycle='ACTIVE' AND p.pending_observation_digest IS NULL AND NOT p.drift_canonical_root AND NOT p.drift_repository AND NOT p.drift_file AND NOT p.drift_primary_ref_name AND NOT p.drift_primary_ref_storage),
    'project_registry_root', (SELECT o.canonical_root FROM ONLY control.project_registry_projects p JOIN ONLY control.project_registry_observations o ON o.observation_digest=p.accepted_observation_digest WHERE p.project_id=s.project_id)
)
FROM ONLY control.task_submission_envelopes s
JOIN ONLY control.task_ledger_streams l ON l.stream_id=s.stream_id
WHERE s.task_ref='$taskRef';
"@
    $databaseJson = Invoke-Phase3Psql -Password $password -Port $port -Database $databaseName `
        -Sql $sql -Failure 'PHASE3_DATABASE_EVIDENCE_QUERY_FAILED' -RunRoot $runRoot `
        -RunId $RunId -DataRoot $dataRoot -MarkerPath $markerPath
    try { $databaseEvidence = $databaseJson | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'PHASE3_DATABASE_EVIDENCE_REJECTED' }
    $registryCanonicalRoot = [string]$databaseEvidence.project_registry_root
    if ($registryCanonicalRoot.StartsWith('\\?\', [StringComparison]::Ordinal)) {
        $registryCanonicalRoot = $registryCanonicalRoot.Substring(4)
    }
    $registryRootMatches = -not [string]::IsNullOrWhiteSpace($registryCanonicalRoot) -and
        [string]::Equals(
            [IO.Path]::GetFullPath($registryCanonicalRoot),
            $script:ProjectCanonicalPath,
            [StringComparison]::OrdinalIgnoreCase
        )
    if (
        [long]$databaseEvidence.envelope_count -ne 1 -or
        [long]$databaseEvidence.claim_count -ne 1 -or
        [string]$databaseEvidence.subject_kind -cne 'GENERAL_TASK_INTAKE' -or
        -not [bool]$databaseEvidence.subject_digest_matches_intake -or
        -not [bool]$databaseEvidence.task_spec_digest_is_null -or
        -not [bool]$databaseEvidence.accounting_currency_is_null -or
        [string]$databaseEvidence.stream_sequence -cne '1' -or
        [string]$databaseEvidence.stream_event_count -cne '1' -or
        [string]$databaseEvidence.stream_command_count -cne '1' -or
        [string]$databaseEvidence.stream_outbox_count -cne '0' -or
        [string]$databaseEvidence.stream_resource_revision -cne '0' -or
        [string]$databaseEvidence.stream_active_agents -cne '0' -or
        [string]$databaseEvidence.stream_active_implementers -cne '0' -or
        [string]$databaseEvidence.stream_used_model_calls -cne '0' -or
        [string]$databaseEvidence.stream_used_external_cost -cne '0' -or
        [long]$databaseEvidence.event_count -ne 1 -or
        [long]$databaseEvidence.task_created_count -ne 1 -or
        [long]$databaseEvidence.resource_snapshot_count -ne 0 -or
        [long]$databaseEvidence.command_count -ne 1 -or
        [long]$databaseEvidence.autonomy_count -ne 0 -or
        [long]$databaseEvidence.outbox_count -ne 0 -or
        [long]$databaseEvidence.non_task_ledger_terminal_count -ne 0 -or
        [long]$databaseEvidence.task_ledger_terminal_count -ne 1 -or
        [string]$databaseEvidence.objective -cne $script:Objective -or
        [string]$databaseEvidence.project_id -cne $script:ProjectId -or
        [string]$databaseEvidence.project_name -cne $script:ProjectName -or
        [long]$databaseEvidence.project_registry_active_count -ne 1 -or
        -not $registryRootMatches
    ) {
        throw 'PHASE3_DATABASE_EVIDENCE_REJECTED'
    }

    $failureStage = 'EFFECT_EVIDENCE'
    $processEffects = @($sessions | Where-Object { $_.effect_counters.process -ne 0 })
    $filesystemEffects = @($sessions | Where-Object { $_.effect_counters.filesystem -ne 0 })
    $codexEffects = [long](($sessions | ForEach-Object { $_.effect_counters.codex } | Measure-Object -Sum).Sum)
    $totalProcessEffects = [long](($sessions | ForEach-Object { $_.effect_counters.process } | Measure-Object -Sum).Sum)
    $totalFilesystemEffects = [long](($sessions | ForEach-Object { $_.effect_counters.filesystem } | Measure-Object -Sum).Sum)
    $expectedEffectVectors = @{
        '01-discovery' = '0,0,0,0,0,0'
        '02-submit' = '1,4,1,1,4,0'
        '03-pre-restart-status' = '1,2,0,0,2,0'
        '04-exact-replay' = '1,2,0,0,2,0'
        '05-objective-conflict' = '1,1,0,0,1,0'
        '06-project-conflict' = '1,1,0,0,1,0'
        '07-restart-status' = '1,2,0,0,2,0'
    }
    foreach ($session in $sessions) {
        $actualEffectVector = '{0},{1},{2},{3},{4},{5}' -f @(
            [long]$session.effect_counters.dispatch,
            [long]$session.effect_counters.database,
            [long]$session.effect_counters.filesystem,
            [long]$session.effect_counters.process,
            [long]$session.effect_counters.network,
            [long]$session.effect_counters.codex
        )
        if (-not $expectedEffectVectors.ContainsKey([string]$session.role) -or
            $actualEffectVector -cne $expectedEffectVectors[[string]$session.role]) {
            throw 'PHASE3_UNEXPECTED_SESSION_EFFECT_REJECTED'
        }
    }
    if ($codexEffects -ne 0 -or $totalProcessEffects -ne 1 -or $totalFilesystemEffects -ne 1 -or
        $processEffects.Count -ne 1 -or [string]$processEffects[0].role -cne '02-submit' -or
        $filesystemEffects.Count -ne 1 -or [string]$filesystemEffects[0].role -cne '02-submit') {
        throw 'PHASE3_UNEXPECTED_EXTERNAL_EFFECT_REJECTED'
    }
}
catch {
    $failureLine = [int]$_.InvocationInfo.ScriptLineNumber
    $failureException = $_.Exception.GetType().Name
    $failureCode = ConvertTo-Phase3FailureCode -Message $_.Exception.Message
    if ($failureCode -ceq 'PHASE3_HARNESS_RUNTIME_ERROR') {
        $failureCode = 'PHASE3_STAGE_' + $failureStage + '_FAILED'
    }
}
finally {
    if ($runRootCreated -and (Test-Path -LiteralPath $runRoot -PathType Container)) {
        try {
            $null = Get-Phase3OwnerMarker -RunRoot $runRoot -RunId $RunId -Port $port `
                -DataRoot $dataRoot -MarkerPath $markerPath
            $listeners = @(Get-Phase3ListenerPids -Port $port)
            if ($listeners.Count -ne 0) {
                $null = Assert-Phase3OwnedLivePostgres -RunRoot $runRoot -RunId $RunId -Port $port `
                    -DataRoot $dataRoot -MarkerPath $markerPath
                Stop-Phase3Postgres -RunRoot $runRoot -RunId $RunId -Port $port `
                    -DataRoot $dataRoot -MarkerPath $markerPath
                $postgresRunning = $false
            }
            $listenerAbsent = @(Get-Phase3ListenerPids -Port $port).Count -eq 0
            if (-not $listenerAbsent) { throw 'PHASE3_CLEANUP_LISTENER_SURVIVED' }
            $deleteTarget = [IO.Path]::GetFullPath($runRoot)
            if ($deleteTarget -cne $expectedRoot -or
                -not $deleteTarget.StartsWith(
                    $tempParent + [IO.Path]::DirectorySeparatorChar,
                    [StringComparison]::OrdinalIgnoreCase
                ) -or
                [IO.Path]::GetFileName($deleteTarget) -cne ('lattice-phase3-general-intake-' + $runId)) {
                throw 'PHASE3_DELETE_TARGET_REJECTED'
            }
            $null = Get-Phase3OwnerMarker -RunRoot $deleteTarget -RunId $RunId -Port $port `
                -DataRoot $dataRoot -MarkerPath $markerPath
            Remove-Item -LiteralPath $deleteTarget -Recurse -Force
            if (Test-Path -LiteralPath $deleteTarget) { throw 'PHASE3_DELETE_INCOMPLETE' }
            $cleanupSucceeded = $true
        }
        catch {
            $cleanupSucceeded = $false
            if ($null -eq $failureCode) {
                $failureCode = ConvertTo-Phase3FailureCode -Message $_.Exception.Message
            }
        }
    }
}

$elapsed = ([DateTimeOffset]::UtcNow - $startedAt).TotalMilliseconds
if ($null -eq $failureCode -and $null -ne $accepted -and $cleanupSucceeded) {
    $totalCounters = [ordered]@{
        dispatch = [long](($sessions | ForEach-Object { $_.effect_counters.dispatch } | Measure-Object -Sum).Sum)
        database = [long](($sessions | ForEach-Object { $_.effect_counters.database } | Measure-Object -Sum).Sum)
        filesystem = [long](($sessions | ForEach-Object { $_.effect_counters.filesystem } | Measure-Object -Sum).Sum)
        process = [long](($sessions | ForEach-Object { $_.effect_counters.process } | Measure-Object -Sum).Sum)
        network = [long](($sessions | ForEach-Object { $_.effect_counters.network } | Measure-Object -Sum).Sum)
        codex = [long](($sessions | ForEach-Object { $_.effect_counters.codex } | Measure-Object -Sum).Sum)
    }
    [ordered]@{
        schema = 'lattice.phase3-general-task-intake.acceptance.v1'
        status = 'PASS'
        acceptance = $true
        elapsed_ms = [long]$elapsed
        binary_sha256 = Get-Phase3FileSha256 -Path $latticed
        discovery_general_schema = $true
        control_catalog = [ordered]@{
            live = $controlVerified
            canonical_path_exact = $true
            record_kind = 'CONTROL_LOCAL_CATALOG'
            registry_authority = 'NONE'
        }
        task_ref = [string]$accepted.task_ref
        status_value = [string]$accepted.status
        task_state = [string]$accepted.task_state
        objective = $script:Objective
        project_id = $script:ProjectId
        project_name = $script:ProjectName
        idempotency = [ordered]@{
            exact_replay_same_task_ref = $true
            changed_objective_rejected = $true
            changed_project_rejected = $true
        }
        restart = [ordered]@{
            physical_postgres_stop_start = $physicalRestart
            fresh_process_status_exact = $true
        }
        psql = [ordered]@{
            max_attempts = $script:PsqlMaxAttempts
            transient_retry_count = $script:PsqlTransientRetryCount
            raw_stderr_persisted = $false
        }
        database = [ordered]@{
            formal_project_registry_active = $true
            formal_project_registry_root_exact = $registryRootMatches
            subject_kind = [string]$databaseEvidence.subject_kind
            task_spec_digest_is_null = [bool]$databaseEvidence.task_spec_digest_is_null
            accounting_currency_is_null = [bool]$databaseEvidence.accounting_currency_is_null
            event_count = [long]$databaseEvidence.event_count
            command_count = [long]$databaseEvidence.command_count
            autonomy_count = [long]$databaseEvidence.autonomy_count
            resource_snapshot_count = [long]$databaseEvidence.resource_snapshot_count
            outbox_count = [long]$databaseEvidence.outbox_count
            model_calls = [long]$databaseEvidence.stream_used_model_calls
            external_cost = [string]$databaseEvidence.stream_used_external_cost
        }
        observed_effects = [ordered]@{
            fresh_process_sessions = $sessions.Count
            counters = $totalCounters
            project_bridge_process_probe_only = $true
            codex_or_model_dispatch = $false
        }
        cleanup = [ordered]@{
            postgres_stopped = (-not $postgresRunning)
            listener_absent = $listenerAbsent
            temporary_root_removed = $cleanupSucceeded
        }
    } | ConvertTo-Json -Compress -Depth 12
    return
}

[ordered]@{
    schema = 'lattice.phase3-general-task-intake.acceptance.v1'
    status = 'FAIL'
    acceptance = $false
    failure_code = $(if ($null -eq $failureCode) { 'PHASE3_ACCEPTANCE_INCOMPLETE' } else { $failureCode })
    failure_line = $failureLine
    failure_exception = $failureException
    psql_diagnostic = $(
        if ($null -ne $script:LastPsqlDiagnostic -and
            [string]$failureCode -match '_PSQL_') {
            $script:LastPsqlDiagnostic
        }
        else {
            $null
        }
    )
    cleanup = [ordered]@{
        postgres_stopped = (-not $postgresRunning)
        listener_absent = $listenerAbsent
        temporary_root_removed = $cleanupSucceeded
    }
} | ConvertTo-Json -Compress -Depth 6
exit 1
