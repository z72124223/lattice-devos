#Requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$NodePath,
    [Parameter(Mandatory)][string]$DatabasePath,
    [Parameter(Mandatory)][string]$StatePath,
    [string]$RuntimeConfigPath,
    [ValidateRange(0,65535)][int]$Port = 4317
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$entry = Join-Path $repository 'apps\lattice-control\src\shared-control.mjs'
function Exact-Path([string]$Value) {
    if (-not [IO.Path]::IsPathFullyQualified($Value) -or $Value.Contains('"')) { throw 'SHARED_CONTROL_PATH_INVALID' }
    return [IO.Path]::GetFullPath($Value)
}
$NodePath = Exact-Path $NodePath
$DatabasePath = Exact-Path $DatabasePath
$StatePath = Exact-Path $StatePath
if ($Port -gt 0 -and $Port -lt 1024) { throw 'SHARED_CONTROL_PORT_INVALID' }
if (-not (Test-Path -LiteralPath $NodePath -PathType Leaf) -or -not (Test-Path -LiteralPath $entry -PathType Leaf)) { throw 'SHARED_CONTROL_COMPONENT_MISSING' }
if (-not (Test-Path -LiteralPath ([IO.Path]::GetDirectoryName($StatePath)) -PathType Container)) { throw 'SHARED_CONTROL_STATE_DIRECTORY_REQUIRED' }
if (Test-Path -LiteralPath $StatePath) {
    & $NodePath $entry status --state $StatePath
    if ($LASTEXITCODE -ne 0) { throw 'SHARED_CONTROL_EXISTING_STATE_UNVERIFIED' }
    return
}
if ($Port -ne 0 -and @([Net.NetworkInformation.IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners() | Where-Object Port -eq $Port).Count) { throw 'SHARED_CONTROL_PORT_OCCUPIED' }
$environment = @{
    LATTICE_CONTROL_PORT = [string]$Port
    LATTICE_CONTROL_DATABASE_PATH = $DatabasePath
    LATTICE_CONTROL_DESKTOP_OWNED = '0'
}
if ($RuntimeConfigPath) { $environment.LATTICE_RUNTIME_CONFIG_PATH = Exact-Path $RuntimeConfigPath }
$argsLine = '"' + $entry + '" host --state "' + $StatePath + '"'
$process = Start-Process -FilePath $NodePath -ArgumentList $argsLine -WorkingDirectory $repository -Environment $environment `
    -WindowStyle Hidden -RedirectStandardOutput ($StatePath + '.stdout.log') -RedirectStandardError ($StatePath + '.stderr.log') -PassThru
# No WaitForExit: the command must finish normally while the shared service lives.
# This does not change Windows Jobs or claim that every host supports detachment.
[ordered]@{ status='STARTED_PENDING_READBACK'; pid=$process.Id; statePath=$StatePath;
    automaticRestart=$false; scheduled=$false; startupMode='explicit-only' } | ConvertTo-Json -Compress
