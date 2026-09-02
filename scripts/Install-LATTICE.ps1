[CmdletBinding(DefaultParameterSetName = 'Install')]
param(
    [Parameter(ParameterSetName = 'Install')]
    [string]$BundleRoot = '',

    [Parameter(Mandatory, ParameterSetName = 'Rollback')]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string]$RollbackToCommit,

    [string]$TestSandboxRoot = '',
    [string]$TestRegistryId = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Text.UTF8Encoding]::new($false)

. (Join-Path $PSScriptRoot 'LatticeDesktopInstaller.Common.ps1')

$arguments = @{
    TestSandboxRoot = $TestSandboxRoot
    TestRegistryId = $TestRegistryId
}
if ($PSCmdlet.ParameterSetName -ceq 'Rollback') {
    $arguments.RollbackToCommit = $RollbackToCommit
}
else {
    $arguments.BundleRoot = if ([string]::IsNullOrWhiteSpace($BundleRoot)) {
        $PSScriptRoot
    }
    else {
        $BundleRoot
    }
}

Invoke-LatticeDesktopInstall @arguments | ConvertTo-Json -Depth 6
