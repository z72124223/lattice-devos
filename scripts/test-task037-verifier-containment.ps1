[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$harness = Join-Path $PSScriptRoot 'run-task037-full-chain-verification.ps1'
$secret = 'task097-offline-secret-' + [Guid]::NewGuid().ToString('N')
$previous = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_HARNESS_SECRET', 'Process')
$process = $null
try {
    [Environment]::SetEnvironmentVariable('LATTICE_TASK037_HARNESS_SECRET', $secret, 'Process')
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = Join-Path $PSHOME 'powershell.exe'
    $startInfo.Arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $harness + '" -HarnessSelfTest'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw 'TASK097_HARNESS_CHILD_START_REJECTED' }
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(10000)) {
        $process.Kill()
        $process.WaitForExit()
        throw 'TASK097_HARNESS_TIMEOUT_REJECTED'
    }
    $output = $stdout.GetAwaiter().GetResult().TrimEnd("`r", "`n")
    $errorOutput = $stderr.GetAwaiter().GetResult()
    $exitCode = $process.ExitCode
    if ($exitCode -ne 0 -or $output -cne 'TASK037_HARNESS_SELF_TEST=PASS') {
        throw 'TASK097_HARNESS_SELF_TEST_REJECTED'
    }
    if (
        $output.IndexOf($secret, [StringComparison]::Ordinal) -ge 0 -or
        $errorOutput.IndexOf($secret, [StringComparison]::Ordinal) -ge 0
    ) {
        throw 'TASK097_HARNESS_SECRET_LEAKED'
    }
}
finally {
    if ($null -ne $process) {
        if (-not $process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
        $process.Dispose()
    }
    [Environment]::SetEnvironmentVariable('LATTICE_TASK037_HARNESS_SECRET', $previous, 'Process')
}

Write-Output 'TASK097_VERIFIER_CONTAINMENT=PASS'
