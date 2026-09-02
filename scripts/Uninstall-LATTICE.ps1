[CmdletBinding()]
param(
    [string]$TestSandboxRoot = '',
    [string]$TestRegistryId = '',
    [switch]$Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Text.UTF8Encoding]::new($false)

. (Join-Path $PSScriptRoot 'LatticeDesktopInstaller.Common.ps1')

Invoke-LatticeDesktopUninstall `
    -TestSandboxRoot $TestSandboxRoot `
    -TestRegistryId $TestRegistryId `
    -Quiet:$Quiet | ConvertTo-Json -Depth 6
