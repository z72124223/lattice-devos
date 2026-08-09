[CmdletBinding()]
param(
    [ValidateSet('Init', 'Doctor', 'Run')]
    [string]$Mode = 'Doctor',
    [Parameter(Mandatory = $true)]
    [string]$TunnelClientExecutable,
    [Parameter(Mandatory = $true)]
    [string]$ProfileDirectory,
    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,63}$')]
    [string]$ProfileName = 'lattice-local',
    [string]$TunnelId,
    [string]$LatticedExecutable
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Resolve-RequiredLeafPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if (-not [IO.Path]::IsPathRooted($Path)) {
        throw $FailureCode
    }
    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw $FailureCode
    }
    return $resolved
}

$tunnelClient = Resolve-RequiredLeafPath -Path $TunnelClientExecutable -FailureCode 'TASK038_TUNNEL_CLIENT_REJECTED'
if (-not [IO.Path]::IsPathRooted($ProfileDirectory)) {
    throw 'TASK038_TUNNEL_PROFILE_DIRECTORY_REJECTED'
}
$profileRoot = [IO.Path]::GetFullPath($ProfileDirectory)

$arguments = switch ($Mode) {
    'Init' {
        if ($TunnelId -notmatch '^tunnel_[0-9a-f]{32}$') {
            throw 'TASK038_TUNNEL_ID_REJECTED'
        }
        $latticed = Resolve-RequiredLeafPath -Path $LatticedExecutable -FailureCode 'TASK038_LATTICED_EXECUTABLE_REJECTED'
        if ($latticed.IndexOfAny([char[]]@("'", "`r", "`n")) -ge 0) {
            throw 'TASK038_LATTICED_COMMAND_REJECTED'
        }
        [IO.Directory]::CreateDirectory($profileRoot) | Out-Null
        @(
            'init',
            '--sample', 'sample_mcp_stdio_local',
            '--profile', $ProfileName,
            '--profile-dir', $profileRoot,
            '--tunnel-id', $TunnelId,
            '--mcp-command', ("'" + $latticed + "'")
        )
        break
    }
    'Doctor' {
        @('doctor', '--profile', $ProfileName, '--profile-dir', $profileRoot, '--explain')
        break
    }
    'Run' {
        if ([string]::IsNullOrWhiteSpace($env:CONTROL_PLANE_API_KEY)) {
            throw 'TASK038_TUNNEL_RUNTIME_KEY_REQUIRED'
        }
        @('run', '--profile', $ProfileName, '--profile-dir', $profileRoot)
        break
    }
}

$allowedEnvironmentNames = [Collections.Generic.HashSet[string]]::new(
    [StringComparer]::OrdinalIgnoreCase
)
@(
    'ALLUSERSPROFILE',
    'APPDATA',
    'CommonProgramFiles',
    'CommonProgramFiles(x86)',
    'CommonProgramW6432',
    'ComSpec',
    'CONTROL_PLANE_API_KEY',
    'DriverData',
    'HOMEDRIVE',
    'HOMEPATH',
    'LOCALAPPDATA',
    'NUMBER_OF_PROCESSORS',
    'OS',
    'Path',
    'PATHEXT',
    'PROCESSOR_ARCHITECTURE',
    'PROCESSOR_IDENTIFIER',
    'PROCESSOR_LEVEL',
    'PROCESSOR_REVISION',
    'ProgramData',
    'ProgramFiles',
    'ProgramFiles(x86)',
    'ProgramW6432',
    'PSModulePath',
    'SystemDrive',
    'SystemRoot',
    'TEMP',
    'TMP',
    'USERDOMAIN',
    'USERDOMAIN_ROAMINGPROFILE',
    'USERNAME',
    'USERPROFILE',
    'windir'
) | ForEach-Object { [void]$allowedEnvironmentNames.Add($_) }

$originalProcessEnvironment = [Environment]::GetEnvironmentVariables('Process')
$clientExitCode = 1
try {
    foreach ($entry in $originalProcessEnvironment.GetEnumerator()) {
        $name = [string]$entry.Key
        $isLatticeConfiguration = $name.StartsWith(
            'LATTICE_',
            [StringComparison]::OrdinalIgnoreCase
        )
        if (-not $isLatticeConfiguration -and -not $allowedEnvironmentNames.Contains($name)) {
            [Environment]::SetEnvironmentVariable($name, $null, 'Process')
        }
    }
    & $tunnelClient @arguments
    $clientExitCode = $LASTEXITCODE
}
finally {
    foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, $null, 'Process')
    }
    foreach ($entry in $originalProcessEnvironment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable(
            [string]$entry.Key,
            [string]$entry.Value,
            'Process'
        )
    }
}
if ($clientExitCode -ne 0) {
    throw ('TASK038_TUNNEL_CLIENT_FAILED_' + $Mode.ToUpperInvariant())
}
