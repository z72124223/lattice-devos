[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$InstallerPath,
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedSha256,
    [Parameter(Mandatory = $true)][string]$TrialRoot,
    [Parameter(Mandatory = $true)][string]$Output
)

# Read-only host gate; only the supplied, hash-checked installer changes the fresh trial.
function Assert-LatticeEvidence($Condition, [string]$Phase, [string]$Code) {
    if (-not $Condition) { throw "$Phase|$Code" }
}

function Resolve-LatticeRegularPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $cursor = $full
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            Assert-LatticeEvidence (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) 'INPUT' 'REPARSE_PATH_REJECTED'
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
    return $full
}

function Get-LatticeWslReadiness {
    if ([Environment]::OSVersion.Platform -ne 'Win32NT') { return @{ status = 'BLOCKED'; code = 'WINDOWS_REQUIRED' } }
    $os = Get-CimInstance Win32_OperatingSystem
    $hostInfo = @{ build = [int]$os.BuildNumber; product_type = [int]$os.ProductType }
    if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        return @{ status = 'BLOCKED'; code = 'WINDOWS_X64_REQUIRED'; host = $hostInfo }
    }
    if ($hostInfo.build -lt 19041) { return @{ status = 'BLOCKED'; code = 'WINDOWS_UPDATE_REQUIRED'; host = $hostInfo } }
    foreach ($key in @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending',
                        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired')) {
        if (Test-Path -LiteralPath $key) { return @{ status = 'BLOCKED'; code = 'WINDOWS_REBOOT_PENDING'; host = $hostInfo } }
    }
    $wsl = Join-Path ([Environment]::SystemDirectory) 'wsl.exe'
    $machine = Get-CimInstance Win32_ComputerSystem
    $feature = Get-CimInstance Win32_OptionalFeature -Filter "Name='VirtualMachinePlatform'"
    if (-not (Test-Path -LiteralPath $wsl -PathType Leaf) -or -not $machine.HypervisorPresent -or $feature.InstallState -ne 1) {
        return @{ status = 'BLOCKED'; code = 'WSL_HOST_NOT_READY'; host = $hostInfo }
    }
    $info = [Diagnostics.ProcessStartInfo]::new($wsl, '--status')
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = [Diagnostics.Process]::Start($info)
    $process.BeginOutputReadLine()
    $process.BeginErrorReadLine()
    if (-not $process.WaitForExit(60000)) { return @{ status = 'BLOCKED'; code = 'WSL_STATUS_TIMEOUT'; host = $hostInfo } }
    $code = $process.ExitCode
    $process.Dispose()
    if ($code -ne 0) { return @{ status = 'BLOCKED'; code = 'WSL_STATUS_FAILED'; host = $hostInfo } }
    return @{ status = 'READY'; code = 'WSL_HOST_READY'; host = $hostInfo }
}

function Read-LatticeEvidence([string]$Path, [string]$Phase, [long]$MaximumBytes = 2097152) {
    $file = Resolve-LatticeRegularPath $Path
    Assert-LatticeEvidence (Test-Path -LiteralPath $file -PathType Leaf) $Phase 'REPORT_MISSING'
    Assert-LatticeEvidence ((Get-Item -LiteralPath $file).Length -le $MaximumBytes) $Phase 'REPORT_TOO_LARGE'
    try { return Get-Content -LiteralPath $file -Raw -Encoding UTF8 | ConvertFrom-Json -ErrorAction Stop }
    catch { throw "$Phase|REPORT_INVALID_JSON" }
}

function Test-LatticeInstallerReports([string]$Root, [long]$Started) {
    $base = Join-Path $Root 'appdata\LATTICE'
    $receipt = Read-LatticeEvidence "$base\sfx-last-result.json" 'SFX'
    Assert-LatticeEvidence ($receipt.schema -eq 'lattice.sfx-result.v1' -and
        $receipt.run_id -match '^\d{10,}$' -and $receipt.started_at -ge $Started -and
        $receipt.finished_at -ge $receipt.started_at -and
        $receipt.finished_at -le [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() + 5) 'SFX' 'FRESH_COMPLETE_RECEIPT_REQUIRED'
    if ($receipt.status -eq 'SETUP_FAILED') {
        $code = if ($receipt.code -cmatch '^[A-Z][A-Z0-9_]{0,95}$') { $receipt.code } else { 'SETUP_COULD_NOT_START' }
        throw "SFX|$code"
    }
    $report = Read-LatticeEvidence "$base\one-click-report.json" 'THREE_CORES'
    if ($report.status -ne 'INSTALLED') {
        # run_install failures live inside installation; top-level preflight codes
        # may be empty. Preserve only a bounded symbolic code, never raw detail.
        $blocked = @((@($report.installation.code) + @($report.blocked_codes)) |
            Where-Object { $_ -is [string] -and $_ -cmatch '^[A-Z][A-Z0-9_]{0,95}$' })
        $code = if ($blocked.Count) { $blocked[0] } else { 'INSTALL_NOT_VERIFIED' }
        throw "THREE_CORES|$code"
    }
    Assert-LatticeEvidence ($report.schema -eq 'lattice.one-click-install.v1' -and
        $report.created_at -ge $Started -and $report.wsl_host.status -eq 'READY' -and
        $report.installation.status -eq 'INSTALLED') 'THREE_CORES' 'INSTALL_EVIDENCE_INVALID'
    $steps = @($report.installation.post_install_steps)
    $install = @($steps | Where-Object { $_.action -eq 'install' })
    $graph = @($steps | Where-Object { $_.action -eq 'graphify-refresh' })
    $mcp = @($steps | Where-Object { $_.action -eq 'mcp-acceptance' })
    $hook = @($steps | Where-Object { $_.action -eq 'global-startup-hook' })
    $connect = @($steps | Where-Object { $_.action -eq 'connect' })
    Assert-LatticeEvidence ($install.Count -eq 1 -and $install[0].status -eq 'RUNNING_IDENTITY_VERIFIED' -and
        $install[0].exit_code -eq 0 -and $connect.Count -eq 1 -and
        $connect[0].status -eq 'RUNNING_IDENTITY_VERIFIED' -and $connect[0].exit_code -eq 0) 'RUNTIME' 'RUNTIME_NOT_VERIFIED'
    Assert-LatticeEvidence ($graph.Count -eq 1 -and $graph[0].exit_code -eq 0 -and
        $graph[0].operation_evidence.component -eq 'graphify' -and
        $graph[0].operation_evidence.status -eq 'PERSISTED' -and
        $graph[0].operation_evidence.record_count -gt 0) 'GRAPHIFY' 'GRAPH_NOT_PERSISTED'
    $evidence = $graph[0].operation_evidence
    Assert-LatticeEvidence ($mcp.Count -eq 1 -and $mcp[0].status -eq 'VERIFIED' -and
        $mcp[0].relation_count -gt 0 -and $mcp[0].source_receipt_digest -match '^[a-f0-9]{64}$' -and
        $mcp[0].source_receipt_digest -eq $evidence.receipt_digest -and
        $mcp[0].commit -eq $evidence.commit -and $mcp[0].commit -match '^(?:[a-f0-9]{40}|[a-f0-9]{64})$') 'MCP' 'MCP_GRAPH_READBACK_NOT_VERIFIED'
    Assert-LatticeEvidence ($mcp[0].runtime_process_restart -eq 'VERIFIED' -and
        $mcp[0].postgres_process_restart -eq 'VERIFIED') 'RESTART' 'RUNTIME_POSTGRES_RESTART_NOT_VERIFIED'
    Assert-LatticeEvidence ($hook.Count -eq 1 -and $hook[0].status -eq 'INSTALLED') 'GLOBAL_HOOK' 'GLOBAL_HOOK_NOT_INSTALLED'
    $preferences = Read-LatticeEvidence "$base\preferences-report.json" 'PREFERENCES'
    Assert-LatticeEvidence ($preferences.status -eq 'PREFERENCES_APPLIED') 'PREFERENCES' 'PREFERENCES_NOT_APPLIED'
    $profile = Read-LatticeEvidence "$base\codex-portable-profile.json" 'PROFILE'
    Assert-LatticeEvidence ($profile.schema -eq 'lattice.codex-portable-profile.v1' -and
        $profile.secrets_included -eq $false -and $profile.absolute_paths_included -eq $false) 'PROFILE' 'PORTABLE_PROFILE_INVALID'
    $environment = Read-LatticeEvidence "$base\required-environment.json" 'ENVIRONMENT'
    Assert-LatticeEvidence ($environment.schema -eq 'lattice.required-environment.v1' -and
        $environment.status -eq 'COLLECTED' -and $environment.secrets_included -eq $false) 'ENVIRONMENT' 'ENVIRONMENT_NOT_COLLECTED'
    Assert-LatticeEvidence (@($environment.components.bundle_cores).Count -eq 3 -and
        @($environment.components.bundle_cores | Where-Object { $_ -in @('control', 'postgresql', 'graphify') } | Select-Object -Unique).Count -eq 3) 'ENVIRONMENT' 'THREE_CORE_MANIFEST_REQUIRED'
    foreach ($name in @('git', 'node', 'python', 'wsl')) {
        $command = @($environment.commands | Where-Object { $_.name -eq $name })
        $source = if ($name -eq 'wsl') { 'SYSTEM' } else { 'BUNDLED' }
        Assert-LatticeEvidence ($command.Count -eq 1 -and $command[0].status -eq 'PASS' -and
            $command[0].source -eq $source) 'ENVIRONMENT' ('REQUIRED_' + $name.ToUpperInvariant() + '_NOT_VERIFIED')
    }
    Assert-LatticeEvidence ($receipt.status -eq 'SETUP_EXITED' -and $receipt.setup_exit_code -eq 0 -and
        $receipt.reboot_requested -eq $false) 'SFX' 'WHOLE_CMD_NOT_SUCCESSFUL'
    $hashes = @{}
    foreach ($name in @('sfx-last-result.json', 'one-click-report.json', 'preferences-report.json',
                        'codex-portable-profile.json', 'required-environment.json')) {
        $hashes[$name] = (Get-FileHash -LiteralPath "$base\$name" -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return @{ three_cores = 'VERIFIED'; graph_records = $evidence.record_count; mcp_relations = $mcp[0].relation_count;
        runtime_restart = 'VERIFIED'; postgres_restart = 'VERIFIED'; global_hook = 'INSTALLED';
        preferences = 'PREFERENCES_APPLIED'; environment = 'VERIFIED'; reports_sha256 = $hashes }
}

function Get-LatticeLoadedCrt([string]$Root, $EnvironmentInfo) {
    $state = Join-Path $Root 'appdata\LATTICE\runtime'
    $config = Read-LatticeEvidence "$state\installation.json" 'LIVE_CRT' 8388608
    $python = Resolve-LatticeRegularPath $config.python
    Assert-LatticeEvidence ($python.StartsWith("$Root\appdata\LATTICE\bundles\", [StringComparison]::OrdinalIgnoreCase) -and
        $python.EndsWith('\python\python.exe', [StringComparison]::OrdinalIgnoreCase)) 'LIVE_CRT' 'BUNDLED_PYTHON_PATH_REQUIRED'
    $EnvironmentInfo.FileName = $python
    $EnvironmentInfo.Arguments = '-I -B -S "' + (Join-Path $PSScriptRoot 'verify-lattice-loaded-crt.py') + '" --state "' + $state + '"'
    $process = [Diagnostics.Process]::Start($EnvironmentInfo)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $process.BeginErrorReadLine()
    Assert-LatticeEvidence ($process.WaitForExit(600000)) 'LIVE_CRT' 'CRT_CHECK_TIMEOUT_PARTIAL_PRESERVED'
    $payload = $stdout.GetAwaiter().GetResult()
    $code = $process.ExitCode
    $process.Dispose()
    Assert-LatticeEvidence ($payload.Length -le 100000) 'LIVE_CRT' 'CRT_REPORT_TOO_LARGE'
    try { $report = $payload | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'LIVE_CRT|CRT_REPORT_INVALID_JSON' }
    $failure = if ($report.code -cmatch '^[A-Z][A-Z0-9_]{0,95}$') { $report.code } else { 'CRT_LIVE_CHECK_FAILED' }
    Assert-LatticeEvidence ($code -eq 0 -and $report.schema -eq 'lattice.loaded-crt.v1' -and
        $report.status -eq 'VERIFIED' -and $report.mcp_closed -eq $true -and $report.postgres_preserved -eq $true) 'LIVE_CRT' $failure
    return $report
}

function Invoke-LatticeInstallerVerification {
    param([string]$InstallerPath, [string]$ExpectedSha256, [string]$TrialRoot, [string]$Output)
    $ErrorActionPreference = 'Stop'
    $result = [ordered]@{ schema = 'lattice.windows-installer-verification.v1'; status = 'BLOCKED'; phase = 'INPUT';
        code = $null; started_at = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds(); finished_at = $null;
        installer_sha256 = $null; scope = 'LOCAL_WINDOWS_ISOLATED_INSTALLATION';
        limitations = @('NOT_ORDINARY_WINDOWS_10_11_CLEAN_MACHINE', 'UAC_FIRST_BOOT_AND_REBOOT_NOT_TESTED',
            'CODEX_DESKTOP_LOGIN_AND_AI_TURN_NOT_TESTED'); sfx_exit_code = $null; setup_exit_code = $null }
    $outputReady = $false
    try {
        $Output = Resolve-LatticeRegularPath $Output
        Assert-LatticeEvidence (-not (Test-Path -LiteralPath $Output)) 'INPUT' 'OUTPUT_ALREADY_EXISTS'
        $TrialRoot = Resolve-LatticeRegularPath $TrialRoot
        Assert-LatticeEvidence (-not (Test-Path -LiteralPath $TrialRoot)) 'INPUT' 'TRIAL_ROOT_MUST_BE_FRESH'
        Assert-LatticeEvidence ($Output -ine $TrialRoot -and
            -not $Output.StartsWith($TrialRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) 'INPUT' 'OUTPUT_INSIDE_TRIAL_REJECTED'
        $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Output))
        $outputReady = $true
        $InstallerPath = Resolve-LatticeRegularPath $InstallerPath
        Assert-LatticeEvidence ((Test-Path -LiteralPath $InstallerPath -PathType Leaf) -and
            [IO.Path]::GetExtension($InstallerPath) -ieq '.exe') 'INPUT' 'INSTALLER_EXE_REQUIRED'
        $result.phase = 'HASH'
        $result.installer_sha256 = (Get-FileHash -LiteralPath $InstallerPath -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-LatticeEvidence ($ExpectedSha256 -match '^[a-fA-F0-9]{64}$' -and
            $result.installer_sha256 -eq $ExpectedSha256) 'HASH' 'INSTALLER_SHA256_MISMATCH'
        $result.phase = 'WSL_HOST'
        $gate = Get-LatticeWslReadiness
        $result.host = $gate.host
        Assert-LatticeEvidence ($gate.status -eq 'READY') 'WSL_HOST' $gate.code
        if ($env:GITHUB_ACTIONS -eq 'true' -and $env:RUNNER_ENVIRONMENT -eq 'github-hosted' -and
            $gate.host.build -eq 26100 -and $gate.host.product_type -ne 1) {
            $result.scope = 'INDEPENDENT_GITHUB_WINDOWS_SERVER_2025_VM'
        }
        foreach ($child in @('appdata', 'roaming', 'home', 'codex', 'tmp')) { $null = [IO.Directory]::CreateDirectory((Join-Path $TrialRoot $child)) }
        $result.phase = 'SFX'
        $info = [Diagnostics.ProcessStartInfo]::new($InstallerPath, '-y')
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        # Whitelist OS identity only; no runner tokens, provider keys, PG*, or user tool paths.
        $info.EnvironmentVariables.Clear()
        foreach ($key in @('SystemRoot', 'WINDIR', 'COMSPEC', 'USERPROFILE', 'USERNAME', 'USERDOMAIN',
                            'APPDATA', 'ProgramData', 'ProgramFiles', 'ProgramFiles(x86)', 'CommonProgramFiles',
                            'PROCESSOR_ARCHITECTURE', 'NUMBER_OF_PROCESSORS', 'OS')) {
            $value = [Environment]::GetEnvironmentVariable($key)
            if ($null -ne $value) { $info.EnvironmentVariables[$key] = $value }
        }
        $system = [Environment]::SystemDirectory
        $info.EnvironmentVariables['PATH'] = "$system;$env:SystemRoot;$system\Wbem;$system\WindowsPowerShell\v1.0"
        $info.EnvironmentVariables['LOCALAPPDATA'] = "$TrialRoot\appdata"
        $info.EnvironmentVariables['APPDATA'] = "$TrialRoot\roaming"
        $info.EnvironmentVariables['USERPROFILE'] = "$TrialRoot\home"
        $info.EnvironmentVariables['CODEX_HOME'] = "$TrialRoot\codex"
        $info.EnvironmentVariables['TEMP'] = "$TrialRoot\tmp"
        $info.EnvironmentVariables['TMP'] = "$TrialRoot\tmp"
        $info.EnvironmentVariables['LATTICE_INSTALL_UNATTENDED'] = '1'
        $process = [Diagnostics.Process]::Start($info)
        $process.BeginOutputReadLine()
        $process.BeginErrorReadLine()
        # The timed overload waits only this PID, not PostgreSQL descendants or inherited pipes.
        Assert-LatticeEvidence ($process.WaitForExit(3000000)) 'SFX' 'INSTALLER_TIMEOUT_PARTIAL_PRESERVED'
        $result.sfx_exit_code = $process.ExitCode
        $process.Dispose()
        $receiptPath = "$TrialRoot\appdata\LATTICE\sfx-last-result.json"
        if (Test-Path -LiteralPath $receiptPath) {
            $receipt = Read-LatticeEvidence $receiptPath 'SFX'
            if ($receipt.setup_exit_code -is [int] -or $receipt.setup_exit_code -is [long]) { $result.setup_exit_code = $receipt.setup_exit_code }
        }
        $result.checks = Test-LatticeInstallerReports $TrialRoot $result.started_at
        Assert-LatticeEvidence ($result.sfx_exit_code -eq 0) 'SFX' 'SFX_EXIT_FAILED'
        $result.phase = 'LIVE_CRT'
        $result.checks.loaded_crt = Get-LatticeLoadedCrt $TrialRoot $info
        $result.status = 'VERIFIED'
        $result.phase = 'COMPLETE'
        $result.code = 'WHOLE_INSTALLER_VERIFIED'
    } catch {
        $failure = $_.Exception.Message
        if ($failure -cmatch '^([A-Z_]+)\|([A-Z][A-Z0-9_]{0,95})$') {
            $result.phase = $Matches[1]; $result.code = $Matches[2]
        } else { $result.code = 'VERIFICATION_OPERATION_FAILED' }
        # Never echo arbitrary exception text, stderr, auth, runtime configuration, or databases.
    }
    $result.finished_at = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $json = $result | ConvertTo-Json -Depth 8
    if ($outputReady) {
        try {
            $stream = [IO.File]::Open($Output, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
            try {
                $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json + "`n")
                $stream.Write($bytes, 0, $bytes.Length)
            } finally { $stream.Dispose() }
        } catch {
            $result.status = 'BLOCKED'; $result.phase = 'OUTPUT'; $result.code = 'OUTPUT_WRITE_FAILED'
            $json = $result | ConvertTo-Json -Depth 8
        }
    }
    Write-Output $json
    if ($result.status -eq 'VERIFIED') { return 0 }
    return 2
}

# Dot-sourcing exposes the focused validators for small fixture tests without installing.
if ($MyInvocation.InvocationName -ne '.') {
    $items = @(Invoke-LatticeInstallerVerification @PSBoundParameters)
    $code = $items[-1]
    $items | Select-Object -SkipLast 1 | Write-Output
    exit $code
}
