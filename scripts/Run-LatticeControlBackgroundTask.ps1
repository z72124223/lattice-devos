[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$NodePath,
    [Parameter(Mandatory = $true)][string]$DatabasePath,
    [ValidateRange(1024, 65535)][int]$Port = 4317
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$serverPath = Join-Path $repositoryRoot 'apps\lattice-control\src\server.mjs'
$logDirectory = Join-Path ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($DatabasePath))) 'background-service'
New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
$runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + $PID
$stdoutPath = Join-Path $logDirectory ($runId + '.stdout.log')
$stderrPath = Join-Path $logDirectory ($runId + '.stderr.log')
$observationPath = Join-Path $logDirectory ($runId + '.json')
$serverProcess = $null
try {
    $env:LATTICE_CONTROL_PORT = [string]$Port
    $env:LATTICE_CONTROL_DATABASE_PATH = [IO.Path]::GetFullPath($DatabasePath)
    $env:LATTICE_CONTROL_DESKTOP_OWNED = '0'
    $serverProcess = Start-Process -FilePath $NodePath -ArgumentList ('"' + $serverPath + '"') `
        -WorkingDirectory $repositoryRoot -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    # Keep the handle open so the real child exit code reaches Task Scheduler.
    $serverHandle = $serverProcess.Handle
    $observation = [ordered]@{
        started_at = [DateTime]::UtcNow.ToString('o')
        runner_pid = $PID
        server_pid = $serverProcess.Id
        server_path = $serverPath
        node_path = $NodePath
        database_path = $env:LATTICE_CONTROL_DATABASE_PATH
        port = $Port
        stdout_path = $stdoutPath
        stderr_path = $stderrPath
        exit_code = $null
    }
    $observation | ConvertTo-Json | Set-Content -LiteralPath $observationPath -Encoding UTF8
    $serverProcess.WaitForExit()
    $observation.exit_code = $serverProcess.ExitCode
    $observation | ConvertTo-Json | Set-Content -LiteralPath $observationPath -Encoding UTF8
    exit $serverProcess.ExitCode
}
catch {
    if ($null -ne $serverProcess -and -not $serverProcess.HasExited) {
        $serverProcess.Kill()
        $serverProcess.WaitForExit()
    }
    [IO.File]::AppendAllText($stderrPath, [DateTime]::UtcNow.ToString('o') + ' ' + $_.Exception.Message + [Environment]::NewLine)
    exit 1
}
