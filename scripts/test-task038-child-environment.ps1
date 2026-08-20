[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$helper = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'task038-local-process-environment.ps1'))
$item = Get-Item -LiteralPath $helper -Force -ErrorAction SilentlyContinue
if ($null -eq $item -or $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'TASK038_CHILD_ENVIRONMENT_HELPER_REJECTED'
}
. $helper

$hostile = [ordered]@{
    HERMES_HOME = 'fixture-hermes-home'
    HERMES_PROFILE = 'fixture-hermes-profile'
    OPENCLAW_HOME = 'fixture-openclaw-home'
    OPENCLAW_STATE = 'fixture-openclaw-state'
    LATTICE_HERMES_API_KEY = 'fixture-hermes-key'
    CONTROL_PLANE_API_KEY = 'fixture-control-plane-key'
    AWS_SECRET_ACCESS_KEY = 'fixture-aws-key'
    PGPASSWORD = 'fixture-postgres-password'
}
$original = @{}
try {
    foreach ($entry in $hostile.GetEnumerator()) {
        $original[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, 'Process')
        [Environment]::SetEnvironmentVariable($entry.Key, [string]$entry.Value, 'Process')
    }
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = (@(Get-Command 'powershell.exe' -CommandType Application -ErrorAction Stop)[0].Source)
    $startInfo.Arguments = '-NoProfile -Command "if ($env:HERMES_HOME -or $env:HERMES_PROFILE -or $env:OPENCLAW_HOME -or $env:OPENCLAW_STATE -or $env:LATTICE_HERMES_API_KEY -or $env:CONTROL_PLANE_API_KEY -or $env:AWS_SECRET_ACCESS_KEY -or $env:PGPASSWORD) { exit 91 }; if ($env:LATTICE_TASK_INGRESS_KIND -ne ''LOCAL_CANONICAL_MCP_ACCEPTANCE'') { exit 92 }; exit 0"'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    Set-Task038ClosedChildEnvironment -StartInfo $startInfo -EnvironmentValues ([ordered]@{
        LATTICE_TASK_INGRESS_KIND = 'LOCAL_CANONICAL_MCP_ACCEPTANCE'
    })
    $process = [Diagnostics.Process]::Start($startInfo)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    if (-not $process.WaitForExit(10000)) {
        try { $process.Kill() } catch { }
        throw 'TASK038_CHILD_ENVIRONMENT_PROBE_TIMEOUT'
    }
    $exitCode = [int]$process.ExitCode
    $process.Dispose()
    if ($exitCode -ne 0 -or -not [string]::IsNullOrEmpty($stdout) -or -not [string]::IsNullOrEmpty($stderr)) {
        throw ('TASK038_CHILD_ENVIRONMENT_PROBE_REJECTED_' + $exitCode)
    }

    $encodingProbe = [Diagnostics.ProcessStartInfo]::new()
    $encodingProbe.FileName = $startInfo.FileName
    $encodingProbe.Arguments = '-NoProfile -Command "$bytes=New-Object byte[] 4;$count=[Console]::OpenStandardInput().Read($bytes,0,4);[BitConverter]::ToString($bytes,0,$count)"'
    $encodingProbe.UseShellExecute = $false
    $encodingProbe.CreateNoWindow = $true
    $encodingProbe.RedirectStandardInput = $true
    $encodingProbe.RedirectStandardOutput = $true
    $originalInputEncoding = [Console]::InputEncoding
    try {
        [Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
        $encodingProcess = [Diagnostics.Process]::Start($encodingProbe)
        $encodingProcess.StandardInput.Write('{"js')
        $encodingProcess.StandardInput.Close()
    }
    finally {
        [Console]::InputEncoding = $originalInputEncoding
    }
    $firstBytes = $encodingProcess.StandardOutput.ReadToEnd().Trim()
    $encodingProcess.WaitForExit()
    $encodingProcess.Dispose()
    if ($firstBytes -ne '7B-22-6A-73') {
        throw 'TASK038_CHILD_STDIN_UTF8_REJECTED'
    }
}
finally {
    foreach ($entry in $original.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, $entry.Value, 'Process')
    }
}

Write-Output 'TASK038_CHILD_ENVIRONMENT=PASS'
