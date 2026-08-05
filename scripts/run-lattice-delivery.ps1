[CmdletBinding()]
param(
    [ValidateSet('DeliveryRun', 'DeliveryStatus')]
    [string]$InternalPhase,
    [switch]$OfficialCodex,
    [string]$OfficialLauncherPath,
    [string]$OfficialCodexHomePath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$codexMode = if (-not [string]::IsNullOrEmpty($InternalPhase)) {
    [Environment]::GetEnvironmentVariable('LATTICE_DELIVERY_CODEX_MODE', 'Process')
} elseif ($OfficialCodex) {
    'OFFICIAL_CODEX_APP_SERVER'
} else {
    'SCRIPTED_ACCEPTANCE'
}
if ($codexMode -notin @('SCRIPTED_ACCEPTANCE', 'OFFICIAL_CODEX_APP_SERVER')) {
    throw 'LATTICE_DELIVERY_CODEX_MODE_REJECTED'
}
$officialCodexLiveEnabled = $false
$officialCodexBlockedDiagnostic = 'FAILED_DIAGNOSTIC: OFFICIAL_CODEX_DISABLED_UPSTREAM_WINDOWS_SANDBOX_HELPER_REGRESSION; https://github.com/openai/codex/issues/29952; https://github.com/openai/codex/issues/29200'
if ($codexMode -eq 'OFFICIAL_CODEX_APP_SERVER' -and -not $officialCodexLiveEnabled) {
    throw $officialCodexBlockedDiagnostic
}
$launcherVersion = 'codex-cli 0.144.6'
$deliveryEnvironmentNames = @(
    'LATTICE_DELIVERY_CODEX_MODE',
    'LATTICE_DELIVERY_FIXTURE_ID',
    'LATTICE_DELIVERY_FIXTURE_ROOT',
    'LATTICE_DELIVERY_RUNTIME_EXE',
    'LATTICE_DELIVERY_LAUNCHER',
    'LATTICE_DELIVERY_LAUNCHER_VERSION',
    'LATTICE_DELIVERY_LAUNCHER_SHA256',
    'LATTICE_DELIVERY_SCHEMA_DIR',
    'LATTICE_DELIVERY_CODEX_HOME',
    'LATTICE_DELIVERY_ROOT',
    'LATTICE_DELIVERY_GIT_EXE',
    'LATTICE_DELIVERY_RUN_EVIDENCE',
    'LATTICE_DELIVERY_STATUS_EVIDENCE',
    'LATTICE_DELIVERY_FINAL_EVIDENCE'
)

function Get-CanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $trimCharacters = [char[]]@(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    return [System.IO.Path]::GetFullPath($Path).TrimEnd($trimCharacters)
}

function Test-ExactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    return [string]::Equals(
        (Get-CanonicalPath -Path $Actual),
        (Get-CanonicalPath -Path $Expected),
        [System.StringComparison]::OrdinalIgnoreCase
    )
}

function Assert-NoReparseAncestor {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Boundary
    )

    $canonicalPath = Get-CanonicalPath -Path $Path
    $canonicalBoundary = Get-CanonicalPath -Path $Boundary
    $boundaryPrefix = $canonicalBoundary + [System.IO.Path]::DirectorySeparatorChar
    if (-not (Test-ExactPath -Actual $canonicalPath -Expected $canonicalBoundary) -and
        -not $canonicalPath.StartsWith($boundaryPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'LATTICE_DELIVERY_PATH_OUTSIDE_REPOSITORY'
    }

    $current = $canonicalPath
    while ($true) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw 'LATTICE_DELIVERY_REPARSE_PATH_REJECTED'
            }
        }
        if (Test-ExactPath -Actual $current -Expected $canonicalBoundary) {
            break
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrWhiteSpace($parent) -or (Test-ExactPath -Actual $parent -Expected $current)) {
            throw 'LATTICE_DELIVERY_PATH_ANCESTRY_UNPROVED'
        }
        $current = $parent
    }
}

function Assert-RegularFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [System.IO.FileInfo]) -or
        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $Path -PathType Leaf)
    ) {
        throw 'LATTICE_DELIVERY_REGULAR_FILE_REQUIRED'
    }
}

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )

    [System.IO.File]::WriteAllText($Path, $Content, [System.Text.UTF8Encoding]::new($false))
}

function Write-JsonEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    if (Test-Path -LiteralPath $Path) {
        throw 'LATTICE_DELIVERY_EVIDENCE_ALREADY_EXISTS'
    }
    $json = $Value | ConvertTo-Json -Depth 12 -Compress
    if ($json.Length -gt 32768) {
        throw 'LATTICE_DELIVERY_EVIDENCE_TOO_LARGE'
    }
    Write-Utf8NoBom -Path $Path -Content ($json + "`n")
    Assert-RegularFile -Path $Path
}

function Read-JsonEvidence {
    param([Parameter(Mandatory = $true)][string]$Path)

    Assert-RegularFile -Path $Path
    $raw = Get-Content -LiteralPath $Path -Raw -Encoding utf8
    if ([string]::IsNullOrWhiteSpace($raw) -or $raw.Length -gt 32768) {
        throw 'LATTICE_DELIVERY_EVIDENCE_INVALID'
    }
    try {
        return $raw | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw 'LATTICE_DELIVERY_EVIDENCE_INVALID'
    }
}

function Get-RequiredEnvironment {
    param([Parameter(Mandatory = $true)][string]$Name)

    $value = [Environment]::GetEnvironmentVariable($Name, 'Process')
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw 'LATTICE_DELIVERY_REQUIRED_ENVIRONMENT_MISSING'
    }
    return $value
}

function Invoke-RuntimeJson {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    Assert-RegularFile -Path $Executable
    if ($Arguments -contains '--password') {
        throw 'LATTICE_DELIVERY_PASSWORD_ARGUMENT_FORBIDDEN'
    }
    $output = @(& $Executable @Arguments 2>&1 | ForEach-Object { [string]$_ })
    $exitCode = $LASTEXITCODE
    $raw = ($output -join "`n").Trim()
    if ($exitCode -ne 0) {
        throw 'LATTICE_DELIVERY_RUNTIME_FAILED'
    }
    if ([string]::IsNullOrWhiteSpace($raw) -or $raw.Length -gt 32768) {
        throw 'LATTICE_DELIVERY_RUNTIME_OUTPUT_INVALID'
    }
    $password = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PASSWORD', 'Process')
    if (-not [string]::IsNullOrEmpty($password) -and
        $raw.IndexOf($password, [System.StringComparison]::Ordinal) -ge 0) {
        throw 'LATTICE_DELIVERY_RUNTIME_OUTPUT_SECRET_REJECTED'
    }
    try {
        return $raw | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw 'LATTICE_DELIVERY_RUNTIME_OUTPUT_INVALID'
    }
}

function Invoke-BoundedSecretFreeGitHead {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string]$RepositoryPath,
        [Parameter(Mandatory = $true)][string]$OutputPrefix
    )

    Assert-RegularFile -Path $Executable
    if ($RepositoryPath.Contains('"')) {
        throw 'LATTICE_DELIVERY_GIT_PATH_REJECTED'
    }
    $stdoutPath = $OutputPrefix + '.git.stdout'
    $stderrPath = $OutputPrefix + '.git.stderr'
    if ((Test-Path -LiteralPath $stdoutPath) -or (Test-Path -LiteralPath $stderrPath)) {
        throw 'LATTICE_DELIVERY_GIT_OUTPUT_NOT_FRESH'
    }

    $protectedNames = @(
        'LATTICE_TASK019_PASSWORD', 'PGPASSWORD', 'PGPASSFILE', 'GIT_ASKPASS',
        'SSH_ASKPASS', 'OPENAI_API_KEY', 'CODEX_API_KEY'
    )
    $originalValues = @{}
    $process = $null
    try {
        foreach ($name in $protectedNames) {
            $originalValues[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
            [Environment]::SetEnvironmentVariable($name, $null, 'Process')
        }
        $argumentLine = '-C "' + $RepositoryPath + '" rev-parse --verify HEAD'
        $process = Start-Process -FilePath $Executable -ArgumentList $argumentLine `
            -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath `
            -WindowStyle Hidden -PassThru
        $null = $process.Handle
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not $process.WaitForExit(100)) {
            foreach ($path in @($stdoutPath, $stderrPath)) {
                if ((Test-Path -LiteralPath $path) -and (Get-Item -LiteralPath $path).Length -gt 32768) {
                    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                    throw 'LATTICE_DELIVERY_GIT_OUTPUT_TOO_LARGE'
                }
            }
            if ([DateTime]::UtcNow -ge $deadline) {
                Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                throw 'LATTICE_DELIVERY_GIT_TIMEOUT'
            }
        }
        $process.WaitForExit()
        foreach ($path in @($stdoutPath, $stderrPath)) {
            if ((Get-Item -LiteralPath $path).Length -gt 32768) {
                throw 'LATTICE_DELIVERY_GIT_OUTPUT_TOO_LARGE'
            }
        }
        $stdoutValue = Get-Content -LiteralPath $stdoutPath -Raw -Encoding utf8
        $stderrValue = Get-Content -LiteralPath $stderrPath -Raw -Encoding utf8
        $stdout = if ($null -eq $stdoutValue) { [string]::Empty } else { [string]$stdoutValue }
        $stderr = if ($null -eq $stderrValue) { [string]::Empty } else { [string]$stderrValue }
        $stdout = $stdout.Trim()
        $stderr = $stderr.Trim()
        if ($process.ExitCode -ne 0 -or -not [string]::IsNullOrEmpty($stderr)) {
            throw 'LATTICE_DELIVERY_GIT_REPLAY_FAILED'
        }
        return $stdout
    }
    finally {
        if ($null -ne $process) {
            if (-not $process.HasExited) {
                Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            }
            $process.Dispose()
        }
        foreach ($name in $protectedNames) {
            [Environment]::SetEnvironmentVariable($name, $originalValues[$name], 'Process')
        }
        foreach ($path in @($stdoutPath, $stderrPath)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }
}

function Assert-CodexModeEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)]
        [ValidateSet('SCRIPTED_ACCEPTANCE', 'OFFICIAL_CODEX_APP_SERVER')]
        [string]$CodexMode,
        [Parameter(Mandatory = $true)][string]$RejectionCode
    )

    [int]$schemaFileCount = 0
    $schemaCountValid = [int]::TryParse(
        [string]$Evidence.schema_file_count,
        [System.Globalization.NumberStyles]::None,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [ref]$schemaFileCount
    )
    if (-not $schemaCountValid) {
        throw $RejectionCode
    }

    if ($CodexMode -eq 'SCRIPTED_ACCEPTANCE') {
        if (
            [string]$Evidence.codex_runtime -ne 'SCRIPTED_ACCEPTANCE' -or
            $schemaFileCount -ne 1 -or
            [string]$Evidence.thread_id -ne 'thread-task032-scripted' -or
            [string]$Evidence.turn_id -ne 'turn-task032-scripted'
        ) {
            throw $RejectionCode
        }
        return
    }

    if (
        [string]$Evidence.codex_runtime -ne 'OFFICIAL_CODEX_APP_SERVER' -or
        $schemaFileCount -lt 1 -or
        $schemaFileCount -gt 4096 -or
        [string]$Evidence.thread_id -notmatch '^[A-Za-z0-9][A-Za-z0-9._:-]{0,255}$' -or
        [string]$Evidence.turn_id -notmatch '^[A-Za-z0-9][A-Za-z0-9._:-]{0,255}$'
    ) {
        throw $RejectionCode
    }
}

function Assert-DeliveryRunEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][string]$Launcher,
        [Parameter(Mandatory = $true)][string]$LauncherSha256,
        [Parameter(Mandatory = $true)][string]$DeliveryRoot,
        [Parameter(Mandatory = $true)]
        [ValidateSet('SCRIPTED_ACCEPTANCE', 'OFFICIAL_CODEX_APP_SERVER')]
        [string]$CodexMode
    )

    $required = @(
        'status', 'component', 'launcher_path', 'version', 'launcher_sha256',
        'schema_bundle_sha256', 'schema_file_count', 'repository_path',
        'changed_paths', 'test', 'test_command_id', 'baseline_commit', 'parent_sha',
        'commit_sha', 'thread_id', 'turn_id', 'codex_runtime', 'intent_digest',
        'outcome_digest', 'profile', 'request_id', 'configuration_digest',
        'receipt_digest'
    )
    foreach ($name in $required) {
        if ($name -notin $Evidence.PSObject.Properties.Name) {
            throw 'LATTICE_DELIVERY_RUN_EVIDENCE_INCOMPLETE'
        }
    }

    $changedPaths = @($Evidence.changed_paths)
    if (
        [string]$Evidence.status -ne 'COMPLETED' -or
        [string]$Evidence.component -ne 'lattice-delivery' -or
        -not (Test-ExactPath -Actual ([string]$Evidence.launcher_path) -Expected $Launcher) -or
        [string]$Evidence.version -ne $launcherVersion -or
        [string]$Evidence.launcher_sha256 -ne $LauncherSha256 -or
        [string]$Evidence.schema_bundle_sha256 -notmatch '^[0-9a-f]{64}$' -or
        -not (Test-ExactPath -Actual ([string]$Evidence.repository_path) -Expected (Join-Path $DeliveryRoot 'repo')) -or
        $changedPaths.Count -ne 1 -or
        [string]$changedPaths[0] -ne 'answer.txt' -or
        [string]$Evidence.test -ne 'FIXED_TEST_PASSED' -or
        [string]$Evidence.test_command_id -ne 'git-diff-no-index-exact-answer-v1' -or
        [string]$Evidence.baseline_commit -notmatch '^[0-9a-f]{40}([0-9a-f]{24})?$' -or
        [string]$Evidence.parent_sha -ne [string]$Evidence.baseline_commit -or
        [string]$Evidence.commit_sha -notmatch '^[0-9a-f]{40}([0-9a-f]{24})?$' -or
        [string]$Evidence.commit_sha -eq [string]$Evidence.baseline_commit -or
        [string]$Evidence.profile -ne 'task032-codex-postgres-v1' -or
        [string]$Evidence.request_id -notmatch '^task032-request-[0-9a-f]{32}$' -or
        [string]$Evidence.configuration_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.intent_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.outcome_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.receipt_digest -notmatch '^[0-9a-f]{64}$'
    ) {
        throw 'LATTICE_DELIVERY_RUN_EVIDENCE_REJECTED'
    }
    Assert-CodexModeEvidence `
        -Evidence $Evidence `
        -CodexMode $CodexMode `
        -RejectionCode 'LATTICE_DELIVERY_RUN_EVIDENCE_REJECTED'
}

function Assert-DurableStatusEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][string]$Launcher,
        [Parameter(Mandatory = $true)][string]$LauncherSha256,
        [Parameter(Mandatory = $true)][string]$DeliveryRoot,
        [Parameter(Mandatory = $true)]
        [ValidateSet('SCRIPTED_ACCEPTANCE', 'OFFICIAL_CODEX_APP_SERVER')]
        [string]$CodexMode
    )

    $required = @(
        'status', 'component', 'repository_path', 'changed_paths', 'test',
        'test_command_id', 'commit_sha', 'parent_sha', 'baseline_commit',
        'launcher_path', 'version', 'launcher_sha256', 'schema_bundle_sha256',
        'schema_file_count', 'thread_id', 'turn_id', 'codex_runtime',
        'intent_digest', 'outcome_digest', 'profile', 'request_id',
        'configuration_digest', 'receipt_digest'
    )
    foreach ($name in $required) {
        if ($name -notin $Evidence.PSObject.Properties.Name) {
            throw 'LATTICE_DELIVERY_STATUS_EVIDENCE_INCOMPLETE'
        }
    }
    $changedPaths = @($Evidence.changed_paths)
    if (
        [string]$Evidence.status -ne 'COMPLETED' -or
        [string]$Evidence.component -ne 'delivery-ledger' -or
        -not (Test-ExactPath -Actual ([string]$Evidence.repository_path) -Expected (Join-Path $DeliveryRoot 'repo')) -or
        $changedPaths.Count -ne 1 -or
        [string]$changedPaths[0] -ne 'answer.txt' -or
        [string]$Evidence.test -ne 'FIXED_TEST_PASSED' -or
        [string]$Evidence.test_command_id -ne 'git-diff-no-index-exact-answer-v1' -or
        [string]$Evidence.commit_sha -notmatch '^[0-9a-f]{40}([0-9a-f]{24})?$' -or
        [string]$Evidence.parent_sha -notmatch '^[0-9a-f]{40}([0-9a-f]{24})?$' -or
        [string]$Evidence.commit_sha -eq [string]$Evidence.parent_sha -or
        [string]$Evidence.baseline_commit -ne [string]$Evidence.parent_sha -or
        -not (Test-ExactPath -Actual ([string]$Evidence.launcher_path) -Expected $Launcher) -or
        [string]$Evidence.version -ne $launcherVersion -or
        [string]$Evidence.launcher_sha256 -ne $LauncherSha256 -or
        [string]$Evidence.schema_bundle_sha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.profile -ne 'task032-codex-postgres-v1' -or
        [string]$Evidence.request_id -notmatch '^task032-request-[0-9a-f]{32}$' -or
        [string]$Evidence.configuration_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.intent_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.outcome_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.receipt_digest -notmatch '^[0-9a-f]{64}$'
    ) {
        throw 'LATTICE_DELIVERY_STATUS_EVIDENCE_REJECTED'
    }
    Assert-CodexModeEvidence `
        -Evidence $Evidence `
        -CodexMode $CodexMode `
        -RejectionCode 'LATTICE_DELIVERY_STATUS_EVIDENCE_REJECTED'
}

function Assert-InternalEnvironment {
    $mode = Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_CODEX_MODE'
    $hostName = Get-RequiredEnvironment -Name 'LATTICE_TASK019_HOST'
    $port = Get-RequiredEnvironment -Name 'LATTICE_TASK019_PORT'
    $postgresRunId = Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID'
    $null = Get-RequiredEnvironment -Name 'LATTICE_TASK019_PASSWORD'
    if (
        $mode -ne $codexMode -or
        (Get-RequiredEnvironment -Name 'LATTICE_TASK019_LIVE') -ne '1' -or
        (Get-RequiredEnvironment -Name 'LATTICE_TASK019_PHASE') -ne 'restart' -or
        $hostName -ne '127.0.0.1' -or
        $port -notmatch '^[0-9]{1,5}$' -or
        [int]$port -eq 0 -or
        [int]$port -eq 5432 -or
        $postgresRunId -notmatch '^[0-9a-f]{32}$'
    ) {
        throw 'LATTICE_DELIVERY_INTERNAL_ENVIRONMENT_REJECTED'
    }
}

function Invoke-DeliveryRunPhase {
    Assert-InternalEnvironment
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $fixtureRoot = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FIXTURE_ROOT')
    $runtime = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_RUNTIME_EXE')
    $launcher = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_LAUNCHER')
    $launcherSha256 = Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_LAUNCHER_SHA256'
    $schemaDirectory = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_SCHEMA_DIR')
    $codexHome = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_CODEX_HOME')
    $deliveryRoot = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_ROOT')
    $gitExe = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_GIT_EXE')
    $runEvidencePath = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_RUN_EVIDENCE')

    $repositoryOwnedPaths = @($fixtureRoot, $runtime, $schemaDirectory, $codexHome, $deliveryRoot, $runEvidencePath)
    if ($codexMode -eq 'SCRIPTED_ACCEPTANCE') {
        $repositoryOwnedPaths += $launcher
    }
    foreach ($path in $repositoryOwnedPaths) {
        Assert-NoReparseAncestor -Path $path -Boundary $repositoryRoot
    }
    Assert-RegularFile -Path $runtime
    Assert-RegularFile -Path $launcher
    Assert-RegularFile -Path $gitExe
    if ((Test-Path -LiteralPath $schemaDirectory) -or (Test-Path -LiteralPath $deliveryRoot) -or (Test-Path -LiteralPath $runEvidencePath)) {
        throw 'LATTICE_DELIVERY_RUN_TARGET_NOT_FRESH'
    }
    if ($launcherSha256 -notmatch '^[0-9a-f]{64}$') {
        throw 'LATTICE_DELIVERY_LAUNCHER_DIGEST_INVALID'
    }

    $arguments = @(
        'delivery-run',
        '--launcher', $launcher,
        '--version', (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_LAUNCHER_VERSION'),
        '--sha256', $launcherSha256,
        '--schema-dir', $schemaDirectory,
        '--codex-home', $codexHome,
        '--delivery-root', $deliveryRoot,
        '--git-exe', $gitExe,
        '--timeout-seconds', '120',
        '--postgres-host', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_HOST'),
        '--postgres-port', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_PORT'),
        '--postgres-run-id', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID')
    )
    $evidence = Invoke-RuntimeJson -Executable $runtime -Arguments $arguments
    Assert-DeliveryRunEvidence `
        -Evidence $evidence `
        -Launcher $launcher `
        -LauncherSha256 $launcherSha256 `
        -DeliveryRoot $deliveryRoot `
        -CodexMode $codexMode

    $answerPath = Join-Path ([string]$evidence.repository_path) 'answer.txt'
    Assert-RegularFile -Path $answerPath
    $expectedAnswer = [System.Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n")
    $answer = [System.IO.File]::ReadAllBytes($answerPath)
    if ([Convert]::ToBase64String($answer) -ne [Convert]::ToBase64String($expectedAnswer)) {
        throw 'LATTICE_DELIVERY_ANSWER_BYTES_REJECTED'
    }
    Write-JsonEvidence -Path $runEvidencePath -Value $evidence
}

function Invoke-DeliveryStatusPhase {
    Assert-InternalEnvironment
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $runtime = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_RUNTIME_EXE')
    $launcher = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_LAUNCHER')
    $launcherSha256 = Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_LAUNCHER_SHA256'
    $deliveryRoot = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_ROOT')
    $gitExe = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_GIT_EXE')
    $runEvidencePath = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_RUN_EVIDENCE')
    $statusEvidencePath = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_STATUS_EVIDENCE')
    $finalEvidencePath = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FINAL_EVIDENCE')

    $repositoryOwnedPaths = @($runtime, $deliveryRoot, $runEvidencePath, $statusEvidencePath, $finalEvidencePath)
    if ($codexMode -eq 'SCRIPTED_ACCEPTANCE') {
        $repositoryOwnedPaths += $launcher
    }
    foreach ($path in $repositoryOwnedPaths) {
        Assert-NoReparseAncestor -Path $path -Boundary $repositoryRoot
    }
    Assert-RegularFile -Path $runtime
    Assert-RegularFile -Path $gitExe
    if ((Test-Path -LiteralPath $statusEvidencePath) -or (Test-Path -LiteralPath $finalEvidencePath)) {
        throw 'LATTICE_DELIVERY_STATUS_TARGET_NOT_FRESH'
    }

    $runEvidence = Read-JsonEvidence -Path $runEvidencePath
    Assert-DeliveryRunEvidence `
        -Evidence $runEvidence `
        -Launcher $launcher `
        -LauncherSha256 $launcherSha256 `
        -DeliveryRoot $deliveryRoot `
        -CodexMode $codexMode
    $status = Invoke-RuntimeJson -Executable $runtime -Arguments @(
        'delivery-status',
        '--postgres-host', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_HOST'),
        '--postgres-port', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_PORT'),
        '--postgres-run-id', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID')
    )
    Assert-DurableStatusEvidence `
        -Evidence $status `
        -Launcher $launcher `
        -LauncherSha256 $launcherSha256 `
        -DeliveryRoot $deliveryRoot `
        -CodexMode $codexMode

    foreach ($name in @(
        'repository_path', 'test', 'test_command_id', 'commit_sha', 'parent_sha',
        'baseline_commit',
        'launcher_path', 'version', 'launcher_sha256', 'schema_bundle_sha256',
        'schema_file_count', 'thread_id', 'turn_id', 'codex_runtime',
        'intent_digest', 'outcome_digest', 'profile', 'request_id',
        'configuration_digest', 'receipt_digest'
    )) {
        if ([string]$status.$name -ne [string]$runEvidence.$name) {
            throw 'LATTICE_DELIVERY_RESTART_CROSS_BINDING_REJECTED'
        }
    }

    $repositoryPath = Get-CanonicalPath -Path ([string]$status.repository_path)
    if (-not (Test-ExactPath -Actual $repositoryPath -Expected (Join-Path $deliveryRoot 'repo'))) {
        throw 'LATTICE_DELIVERY_REPOSITORY_PATH_REJECTED'
    }
    Assert-NoReparseAncestor -Path $repositoryPath -Boundary $repositoryRoot
    $answerPath = Join-Path $repositoryPath 'answer.txt'
    Assert-RegularFile -Path $answerPath
    $expectedAnswer = [System.Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n")
    $answer = [System.IO.File]::ReadAllBytes($answerPath)
    if ([Convert]::ToBase64String($answer) -ne [Convert]::ToBase64String($expectedAnswer)) {
        throw 'LATTICE_DELIVERY_ANSWER_BYTES_REJECTED'
    }

    $head = Invoke-BoundedSecretFreeGitHead -Executable $gitExe -RepositoryPath $repositoryPath -OutputPrefix $statusEvidencePath
    if ($head -notmatch '^[0-9a-f]{40}([0-9a-f]{24})?$' -or $head -ne [string]$status.commit_sha) {
        throw 'LATTICE_DELIVERY_COMMIT_REPLAY_REJECTED'
    }

    Write-JsonEvidence -Path $statusEvidencePath -Value $status
    $final = [ordered]@{
        status = 'COMPLETED'
        component = 'lattice-delivery-acceptance'
        codex_mode = $codexMode
        postgres_restarted_before_status = $true
        fixture_id = Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FIXTURE_ID'
        postgres_run_id = Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID'
        repository_path = $repositoryPath
        changed_paths = @('answer.txt')
        test_command_id = [string]$status.test_command_id
        baseline_commit = [string]$status.baseline_commit
        commit_sha = [string]$status.commit_sha
        codex_runtime = [string]$status.codex_runtime
        profile = [string]$status.profile
        request_id = [string]$status.request_id
        configuration_digest = [string]$status.configuration_digest
        launcher_sha256 = [string]$status.launcher_sha256
        schema_bundle_sha256 = [string]$status.schema_bundle_sha256
        intent_digest = [string]$status.intent_digest
        outcome_digest = [string]$status.outcome_digest
        receipt_digest = [string]$status.receipt_digest
        answer_sha256 = (Get-FileHash -LiteralPath $answerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    Write-JsonEvidence -Path $finalEvidencePath -Value $final
}

function Invoke-DefaultAcceptance {
    if ($OfficialCodex -and (
        [string]::IsNullOrWhiteSpace($OfficialLauncherPath) -or
        [string]::IsNullOrWhiteSpace($OfficialCodexHomePath)
    )) {
        throw 'LATTICE_DELIVERY_OFFICIAL_CONFIGURATION_MISSING'
    }
    if (-not $OfficialCodex -and (
        -not [string]::IsNullOrEmpty($OfficialLauncherPath) -or
        -not [string]::IsNullOrEmpty($OfficialCodexHomePath)
    )) {
        throw 'LATTICE_DELIVERY_UNUSED_OFFICIAL_CONFIGURATION'
    }
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
    $fixtureParent = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'lattice-delivery')
    Assert-NoReparseAncestor -Path $fixtureParent -Boundary $repositoryRoot

    $cargo = @(Get-Command 'cargo.exe' -CommandType Application -ErrorAction Stop)[0]
    & $cargo.Source 'build' '-p' 'lattice-runtime' '--locked'
    if ($LASTEXITCODE -ne 0) {
        throw 'LATTICE_DELIVERY_BUILD_FAILED'
    }
    $runtime = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'debug\lattice-runtime.exe')
    Assert-RegularFile -Path $runtime
    $git = @(Get-Command 'git.exe' -CommandType Application -ErrorAction Stop)[0]
    $gitExe = Get-CanonicalPath -Path $git.Source
    Assert-RegularFile -Path $gitExe

    if (-not (Test-Path -LiteralPath $fixtureParent)) {
        New-Item -ItemType Directory -Path $fixtureParent -Force:$false | Out-Null
    }
    Assert-NoReparseAncestor -Path $fixtureParent -Boundary $repositoryRoot
    $fixtureId = [Guid]::NewGuid().ToString('N')
    $fixtureRoot = Get-CanonicalPath -Path (Join-Path $fixtureParent $fixtureId)
    New-Item -ItemType Directory -Path $fixtureRoot -Force:$false | Out-Null
    Assert-NoReparseAncestor -Path $fixtureRoot -Boundary $repositoryRoot

    $codexHome = Join-Path $fixtureRoot 'codex-home'
    $evidenceRoot = Join-Path $fixtureRoot 'evidence'
    New-Item -ItemType Directory -Path $codexHome -Force:$false | Out-Null
    New-Item -ItemType Directory -Path $evidenceRoot -Force:$false | Out-Null
    $fixtureMarker = Join-Path $fixtureRoot '.lattice-delivery-fixture-v1.json'
    [System.IO.File]::WriteAllBytes(
        (Join-Path $codexHome '.lattice-codex-home-v1'),
        [System.Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
    )

    $serverPath = Join-Path $fixtureRoot 'scripted-codex.ps1'
    $serverTemplatePath = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'apps\lattice-runtime\src\fixtures\task032-scripted-codex.ps1')
    Assert-NoReparseAncestor -Path $serverTemplatePath -Boundary $repositoryRoot
    Assert-RegularFile -Path $serverTemplatePath
    [System.IO.File]::WriteAllBytes(
        $serverPath,
        [System.IO.File]::ReadAllBytes($serverTemplatePath)
    )
    Assert-RegularFile -Path $serverPath
    $serverSha256 = (Get-FileHash -LiteralPath $serverPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $launcherPath = Join-Path $fixtureRoot 'scripted-codex.cmd'
    $launcherSource = (@(
        '@echo off',
        'if "%~1"=="--version" if "%~2"=="" goto version',
        'if "%~1"=="app-server" if "%~2"=="generate-json-schema" if "%~3"=="--out" if "%~4" NEQ "" if "%~5"=="" goto schema',
        'if "%~1"=="app-server" if "%~2"=="--listen" if "%~3"=="stdio://" if "%~4"=="" goto server',
        'exit /b 11',
        ':version',
        'echo codex-cli 0.144.6',
        'exit /b 0',
        ':schema',
        ('"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%~dp0scripted-codex.ps1" -ExpectedSelfSha256 "' + $serverSha256 + '" -Mode Schema -SchemaRoot "%~4"'),
        'exit /b %ERRORLEVEL%',
        ':server',
        ('"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%~dp0scripted-codex.ps1" -ExpectedSelfSha256 "' + $serverSha256 + '" -Mode Server'),
        'exit /b %ERRORLEVEL%'
    ) -join "`r`n") + "`r`n"
    [System.IO.File]::WriteAllText($launcherPath, $launcherSource, [System.Text.Encoding]::ASCII)
    Assert-RegularFile -Path $launcherPath
    $launcherSha256 = (Get-FileHash -LiteralPath $launcherPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-JsonEvidence -Path $fixtureMarker -Value ([ordered]@{
        kind = 'LATTICE_DELIVERY_SCRIPTED_ACCEPTANCE_V1'
        fixture_id = $fixtureId
        root = $fixtureRoot
        repository_root = $repositoryRoot
        codex_mode = 'SCRIPTED_ACCEPTANCE'
        launcher_path = (Get-CanonicalPath -Path $launcherPath)
        launcher_sha256 = $launcherSha256
        server_path = (Get-CanonicalPath -Path $serverPath)
        server_sha256 = $serverSha256
    })

    if ($OfficialCodex) {
        $launcherPath = Get-CanonicalPath -Path $OfficialLauncherPath
        $codexHome = Get-CanonicalPath -Path $OfficialCodexHomePath
        Assert-RegularFile -Path $launcherPath
        $launcherItem = Get-Item -LiteralPath $launcherPath -Force
        if ($launcherItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
            throw 'LATTICE_DELIVERY_OFFICIAL_LAUNCHER_REPARSE_REJECTED'
        }
        Assert-NoReparseAncestor -Path $codexHome -Boundary $repositoryRoot
        $homeItem = Get-Item -LiteralPath $codexHome -Force
        if (-not $homeItem.PSIsContainer -or ($homeItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw 'LATTICE_DELIVERY_OFFICIAL_CODEX_HOME_REJECTED'
        }
        $ownershipMarker = Join-Path $codexHome '.lattice-codex-home-v1'
        Assert-RegularFile -Path $ownershipMarker
        $expectedOwnershipMarker = [System.Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
        $actualOwnershipMarker = [System.IO.File]::ReadAllBytes($ownershipMarker)
        if ([Convert]::ToBase64String($actualOwnershipMarker) -ne [Convert]::ToBase64String($expectedOwnershipMarker)) {
            throw 'LATTICE_DELIVERY_OFFICIAL_CODEX_HOME_MARKER_REJECTED'
        }
        $launcherSha256 = (Get-FileHash -LiteralPath $launcherPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }

    $schemaDirectory = Join-Path $fixtureRoot 'schema'
    $deliveryRoot = Join-Path $fixtureRoot 'delivery'
    $runEvidencePath = Join-Path $evidenceRoot 'delivery-run.json'
    $statusEvidencePath = Join-Path $evidenceRoot 'delivery-status.json'
    $finalEvidencePath = Join-Path $evidenceRoot 'final.json'
    $environmentValues = [ordered]@{
        LATTICE_DELIVERY_CODEX_MODE = $codexMode
        LATTICE_DELIVERY_FIXTURE_ID = $fixtureId
        LATTICE_DELIVERY_FIXTURE_ROOT = $fixtureRoot
        LATTICE_DELIVERY_RUNTIME_EXE = $runtime
        LATTICE_DELIVERY_LAUNCHER = $launcherPath
        LATTICE_DELIVERY_LAUNCHER_VERSION = $launcherVersion
        LATTICE_DELIVERY_LAUNCHER_SHA256 = $launcherSha256
        LATTICE_DELIVERY_SCHEMA_DIR = $schemaDirectory
        LATTICE_DELIVERY_CODEX_HOME = $codexHome
        LATTICE_DELIVERY_ROOT = $deliveryRoot
        LATTICE_DELIVERY_GIT_EXE = $gitExe
        LATTICE_DELIVERY_RUN_EVIDENCE = $runEvidencePath
        LATTICE_DELIVERY_STATUS_EVIDENCE = $statusEvidencePath
        LATTICE_DELIVERY_FINAL_EVIDENCE = $finalEvidencePath
    }
    $originalEnvironment = @{}
    foreach ($name in $deliveryEnvironmentNames) {
        $originalEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    }

    try {
        foreach ($entry in $environmentValues.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable($entry.Key, [string]$entry.Value, 'Process')
        }
        $postgresHarness = Get-CanonicalPath -Path (Join-Path $PSScriptRoot 'run-task019-postgres.ps1')
        Assert-NoReparseAncestor -Path $postgresHarness -Boundary $repositoryRoot
        Assert-RegularFile -Path $postgresHarness
        & $postgresHarness -RunLatticeDeliveryHook

        $final = Read-JsonEvidence -Path $finalEvidencePath
        if (
            [string]$final.status -ne 'COMPLETED' -or
            [string]$final.component -ne 'lattice-delivery-acceptance' -or
            [string]$final.codex_mode -ne $codexMode -or
            [bool]$final.postgres_restarted_before_status -ne $true -or
            [string]$final.fixture_id -ne $fixtureId
        ) {
            throw 'LATTICE_DELIVERY_FINAL_EVIDENCE_REJECTED'
        }

        Write-Output 'LATTICE_DELIVERY_HARNESS=PASS'
        Write-Output ("CODEX_MODE=$codexMode")
        Write-Output (([ordered]@{
            status = 'PASS'
            component = 'lattice-delivery-harness'
            codex_mode = $codexMode
            evidence_path = $finalEvidencePath
            commit_sha = [string]$final.commit_sha
            outcome_digest = [string]$final.outcome_digest
        }) | ConvertTo-Json -Compress)
    }
    finally {
        foreach ($name in $deliveryEnvironmentNames) {
            [Environment]::SetEnvironmentVariable($name, $originalEnvironment[$name], 'Process')
        }
    }
}

if (-not [string]::IsNullOrEmpty($InternalPhase)) {
    switch ($InternalPhase) {
        'DeliveryRun' { Invoke-DeliveryRunPhase }
        'DeliveryStatus' { Invoke-DeliveryStatusPhase }
        default { throw 'LATTICE_DELIVERY_INTERNAL_PHASE_REJECTED' }
    }
    return
}

Invoke-DefaultAcceptance
