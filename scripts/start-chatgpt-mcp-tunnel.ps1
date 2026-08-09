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
    $item = Get-Item -LiteralPath $resolved -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [IO.FileInfo]) -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw $FailureCode
    }
    return $resolved
}

function Get-FileSha256 {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    try {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
    }
    catch {
        throw $FailureCode
    }
}

function Get-StringSha256 {
    param([Parameter(Mandatory = $true)][string]$Value)

    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Get-ByteArraySha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Get-LiveTaskIngressProfileDigest {
    param(
        [Parameter(Mandatory = $true)][string]$ProfileRoot,
        [Parameter(Mandatory = $true)][string]$ProfileName,
        [Parameter(Mandatory = $true)][string]$TunnelClient
    )

    $profileRootItem = Get-Item -LiteralPath $ProfileRoot -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $profileRootItem -or
        -not $profileRootItem.PSIsContainer -or
        -not ($profileRootItem -is [IO.DirectoryInfo]) -or
        ($profileRootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $profilePath = [IO.Path]::GetFullPath((Join-Path $ProfileRoot ($ProfileName + '.yaml')))
    $profileItem = Get-Item -LiteralPath $profilePath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $profileItem -or
        $profileItem.PSIsContainer -or
        -not ($profileItem -is [IO.FileInfo]) -or
        ($profileItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $profileItem.Length -lt 1 -or
        $profileItem.Length -gt 65536
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }

    try {
        $profileBytes = [IO.File]::ReadAllBytes($profilePath)
        $profileText = [Text.UTF8Encoding]::new($false, $true).GetString($profileBytes)
    }
    catch {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    if (
        $profileBytes.Length -ne $profileItem.Length -or
        -not $profileText.EndsWith("`n", [StringComparison]::Ordinal) -or
        $profileText.IndexOf("`r", [StringComparison]::Ordinal) -ge 0
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $profileLines = $profileText.Split([string[]]@("`n"), [StringSplitOptions]::None)
    if ($profileLines.Count -ne 23 -or $profileLines[22] -ne '') {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $tunnelMatch = [regex]::Match(
        $profileLines[4],
        '^  tunnel_id: "(?<value>tunnel_[0-9a-f]{32})"$',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    $commandMatch = [regex]::Match(
        $profileLines[21],
        '^      command: (?<value>"(?:[^"\\]|\\.)*")$',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if (-not $tunnelMatch.Success -or -not $commandMatch.Success) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $commandLiteral = $commandMatch.Groups['value'].Value
    try {
        $quotedCommand = ConvertFrom-Json -InputObject $commandLiteral
    }
    catch {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $canonicalCommandLiteral = '"' + $quotedCommand.Replace('\', '\\').Replace('"', '\"') + '"'
    if (
        -not ($quotedCommand -is [string]) -or
        $quotedCommand.Length -lt 3 -or
        -not $quotedCommand.StartsWith("'", [StringComparison]::Ordinal) -or
        -not $quotedCommand.EndsWith("'", [StringComparison]::Ordinal) -or
        $quotedCommand.Substring(1, $quotedCommand.Length - 2).Contains("'") -or
        -not [String]::Equals($commandLiteral, $canonicalCommandLiteral, [StringComparison]::Ordinal)
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $expectedProfileText = ((@(
        'config_version: 1',
        'control_plane:',
        '  base_url: "https://api.openai.com"',
        '',
        ('  tunnel_id: "' + $tunnelMatch.Groups['value'].Value + '"'),
        '  api_key: "env:CONTROL_PLANE_API_KEY"',
        'health:',
        '  # Keep a fixed port when you want a stable local admin URL.',
        '  # For concurrent or clean-room runs, switch listen_addr to "127.0.0.1:0" and',
        '  # set url_file so another process can discover the resolved /healthz, /readyz,',
        '  # /metrics, and /ui base URL.',
        '  listen_addr: "127.0.0.1:8080"',
        '  # url_file: "/tmp/tunnel-client-health.url"',
        'admin_ui:',
        '  open_browser: false',
        'log:',
        '  level: info',
        '  format: json',
        'mcp:',
        '  commands:',
        '    - channel: main',
        ('      command: ' + $canonicalCommandLiteral)
    ) -join "`n") + "`n")
    if (-not [String]::Equals($profileText, $expectedProfileText, [StringComparison]::Ordinal)) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $latticed = Resolve-RequiredLeafPath `
        -Path $quotedCommand.Substring(1, $quotedCommand.Length - 2) `
        -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED'
    $tunnelClientSha256 = Get-FileSha256 `
        -Path $TunnelClient `
        -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED'
    $latticedSha256 = Get-FileSha256 `
        -Path $latticed `
        -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED'
    $profileSha256 = Get-ByteArraySha256 -Bytes $profileBytes
    $commitment = @(
        'lattice.task-ingress-profile.v2',
        ('profile_name=' + $ProfileName),
        ('profile_sha256=' + $profileSha256),
        ('tunnel_id=' + $tunnelMatch.Groups['value'].Value),
        'channel=main',
        ('tunnel_client_sha256=' + $tunnelClientSha256),
        ('latticed_sha256=' + $latticedSha256)
    ) -join "`n"
    return Get-StringSha256 -Value $commitment
}

$tunnelClient = Resolve-RequiredLeafPath -Path $TunnelClientExecutable -FailureCode 'TASK038_TUNNEL_CLIENT_REJECTED'
if (-not [IO.Path]::IsPathRooted($ProfileDirectory)) {
    throw 'TASK038_TUNNEL_PROFILE_DIRECTORY_REJECTED'
}
$profileRoot = [IO.Path]::GetFullPath($ProfileDirectory)
$taskIngressKind = $null
$taskIngressProfileDigest = $null

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
        $taskIngressKind = 'CHATGPT_SECURE_MCP_TUNNEL'
        $taskIngressProfileDigest = Get-LiveTaskIngressProfileDigest `
            -ProfileRoot $profileRoot `
            -ProfileName $ProfileName `
            -TunnelClient $tunnelClient
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
    if ($Mode -eq 'Run') {
        [Environment]::SetEnvironmentVariable(
            'LATTICE_TASK_INGRESS_KIND',
            $taskIngressKind,
            'Process'
        )
        [Environment]::SetEnvironmentVariable(
            'LATTICE_TASK_INGRESS_PROFILE_SHA256',
            $taskIngressProfileDigest,
            'Process'
        )
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
