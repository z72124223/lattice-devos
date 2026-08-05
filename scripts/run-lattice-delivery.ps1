[CmdletBinding()]
param(
    [ValidateSet('DeliveryRun', 'DeliveryStatus')]
    [string]$InternalPhase,
    [switch]$DiagnoseOfficialCodex,
    [switch]$OfficialCodex,
    [switch]$ScriptedDeadlineRegression,
    [switch]$TestRuntimeTerminalEnvelope,
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
$deadlineRegression = if (-not [string]::IsNullOrEmpty($InternalPhase)) {
    [Environment]::GetEnvironmentVariable('LATTICE_DELIVERY_DEADLINE_REGRESSION', 'Process') -eq '1'
} else {
    [bool]$ScriptedDeadlineRegression
}
if ($deadlineRegression -and ($codexMode -ne 'SCRIPTED_ACCEPTANCE' -or $OfficialCodex -or $DiagnoseOfficialCodex)) {
    throw 'LATTICE_DELIVERY_DEADLINE_REGRESSION_MODE_REJECTED'
}
$officialCodexVersion = '0.146.0'
$officialLauncherSha256 = 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb'
$officialSandboxSetupSha256 = 'c12d225b34e7f82cdab6bbc714797abed661f40e158104694953889750121cef'
$officialCommandRunnerSha256 = '0102fa1820ecd03bb03a991fd2303a1a484118f7da8a71864f88ec94bca61d6d'
$officialSignerThumbprint = '0B7C30C11BF7250EC1ECD3254AC781D9E13D62F8'
$codexHomeConfigBytes = [System.Text.UTF8Encoding]::new($false).GetBytes((@(
    'approval_policy = "never"',
    'sandbox_mode = "workspace-write"',
    'model = "gpt-5.6-sol"',
    'model_reasoning_effort = "low"',
    '',
    '[windows]',
    'sandbox = "elevated"'
) -join "`n") + "`n")
$scriptedDeliveryTimeoutSeconds = if ($deadlineRegression) { '40' } else { '120' }
$officialDeliveryTimeoutSeconds = '600'
$deliveryTimeoutSeconds = if ($codexMode -eq 'OFFICIAL_CODEX_APP_SERVER') {
    $officialDeliveryTimeoutSeconds
} else {
    $scriptedDeliveryTimeoutSeconds
}
$officialCodexLiveEnabled = $false
$officialCodexBlockedDiagnostic = 'FAILED_DIAGNOSTIC: OFFICIAL_CODEX_DISABLED_PENDING_LOCAL_ACCEPTANCE'
if ($codexMode -eq 'OFFICIAL_CODEX_APP_SERVER' -and -not $DiagnoseOfficialCodex -and -not $officialCodexLiveEnabled) {
    throw $officialCodexBlockedDiagnostic
}
$officialLauncherVersion = "codex-cli $officialCodexVersion"
$scriptedLauncherVersion = 'codex-cli 0.144.6'
$launcherVersion = if ($codexMode -eq 'OFFICIAL_CODEX_APP_SERVER' -or $DiagnoseOfficialCodex) {
    $officialLauncherVersion
} else {
    $scriptedLauncherVersion
}
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
    'LATTICE_DELIVERY_DEADLINE_REGRESSION',
    'LATTICE_DELIVERY_SCRIPTED_DELAY_MILLISECONDS',
    'LATTICE_DELIVERY_SCRIPTED_INVOCATION_LOG'
)
$graphEvidenceProperties = @(
    'graph_status',
    'graph_project_id',
    'graph_commit_sha',
    'graph_query_digest',
    'graph_analysis_digest',
    'graph_record_count',
    'graph_persistence_digest',
    'graph_retrieval_digest',
    'graph_result_count',
    'graph_receipt_digest',
    'graph_database_identity_digest',
    'graph_extension_manifest_digest'
)
$deliveryEvidenceProperties = @(
    'status', 'component', 'launcher_path', 'version', 'launcher_sha256',
    'schema_bundle_sha256', 'schema_file_count', 'repository_path',
    'changed_paths', 'test', 'test_command_id', 'baseline_commit', 'parent_sha',
    'commit_sha', 'thread_id', 'turn_id', 'codex_runtime', 'intent_digest',
    'outcome_digest', 'profile', 'request_id', 'configuration_digest',
    'receipt_digest'
) + $graphEvidenceProperties
$finalEvidenceProperties = @(
    'status', 'component', 'codex_mode', 'postgres_restarted_before_status',
    'fixture_id', 'postgres_run_id', 'repository_path', 'changed_paths',
    'test_command_id', 'baseline_commit', 'commit_sha', 'codex_runtime', 'profile',
    'request_id', 'configuration_digest', 'launcher_sha256',
    'schema_bundle_sha256', 'intent_digest', 'outcome_digest', 'receipt_digest',
    'graph_execution_footprint_unchanged_during_status',
    'graph_execution_footprint_digest', 'answer_sha256'
) + $graphEvidenceProperties

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

function Assert-NoReparsePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $currentPath = Get-CanonicalPath -Path $Path
    while ($true) {
        if (Test-Path -LiteralPath $currentPath) {
            $item = Get-Item -LiteralPath $currentPath -Force
            if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw 'LATTICE_DELIVERY_REPARSE_PATH_REJECTED'
            }
        }
        $parent = Split-Path -Parent $currentPath
        if ([string]::IsNullOrWhiteSpace($parent) -or (Test-ExactPath -Actual $parent -Expected $currentPath)) {
            break
        }
        $currentPath = $parent
    }
}

function Get-RestrictedDirectoryAclEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [switch]$RequireProtected
    )

    $acl = Get-Acl -LiteralPath $Path
    $currentIdentity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
    $currentSid = $currentIdentity.User.Value
    $ownerSid = ([System.Security.Principal.NTAccount]$acl.Owner).Translate(
        [System.Security.Principal.SecurityIdentifier]
    ).Value
    if (($RequireProtected -and -not $acl.AreAccessRulesProtected) -or $ownerSid -ne $currentSid) {
        throw 'LATTICE_DELIVERY_RESTRICTED_ACL_REJECTED'
    }
    $allowedSids = @($currentSid, 'S-1-5-18', 'S-1-5-32-544')
    $observedSids = @()
    foreach ($rule in $acl.Access) {
        $sid = $rule.IdentityReference.Translate(
            [System.Security.Principal.SecurityIdentifier]
        ).Value
        $observedSids += $sid
        if (
            $sid -notin $allowedSids -or
            $rule.AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow -or
            ($RequireProtected -and $rule.IsInherited) -or
            ($rule.FileSystemRights -band [System.Security.AccessControl.FileSystemRights]::FullControl) -ne
                [System.Security.AccessControl.FileSystemRights]::FullControl
        ) {
            throw 'LATTICE_DELIVERY_RESTRICTED_ACL_REJECTED'
        }
    }
    foreach ($requiredSid in $allowedSids) {
        if ($requiredSid -notin $observedSids) {
            throw 'LATTICE_DELIVERY_RESTRICTED_ACL_REJECTED'
        }
    }
    return [ordered]@{
        owner_sid = $ownerSid
        acl_protected = [bool]$acl.AreAccessRulesProtected
        allowed_sids = $allowedSids
    }
}

function Assert-SingleLinkFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    $fsutil = Join-Path $env:SystemRoot 'System32\fsutil.exe'
    Assert-RegularFile -Path $fsutil
    $links = @(& $fsutil 'hardlink' 'list' $Path 2>$null)
    if ($LASTEXITCODE -ne 0 -or $links.Count -ne 1) {
        throw 'LATTICE_DELIVERY_HARDLINK_REJECTED'
    }
}

function Get-SignedOfficialFileEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedSha256
    )

    Assert-RegularFile -Path $Path
    $sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if (-not [string]::Equals($sha256, $ExpectedSha256, [System.StringComparison]::Ordinal)) {
        throw 'LATTICE_DELIVERY_OFFICIAL_BINARY_DIGEST_REJECTED'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if (
        [string]$signature.Status -ne 'Valid' -or
        $null -eq $signature.SignerCertificate -or
        -not [string]::Equals(
            [string]$signature.SignerCertificate.Thumbprint,
            $officialSignerThumbprint,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -or
        [string]$signature.SignerCertificate.Subject -notlike '*OpenAI OpCo, LLC*'
    ) {
        throw 'LATTICE_DELIVERY_OFFICIAL_BINARY_SIGNATURE_REJECTED'
    }
    return [ordered]@{
        path = (Get-CanonicalPath -Path $Path)
        sha256 = $sha256
        signer_thumbprint = [string]$signature.SignerCertificate.Thumbprint
    }
}

function Get-OfficialCodexBundleEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$LauncherPath,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $installRoot = Join-Path $RepositoryRoot "target\codex-official\$officialCodexVersion"
    $platformRoot = Join-Path $installRoot 'node_modules\@openai\codex-win32-x64'
    $bundleRoot = Join-Path $platformRoot 'vendor\x86_64-pc-windows-msvc'
    $expectedLauncher = Join-Path $bundleRoot 'bin\codex.exe'
    if (-not (Test-ExactPath -Actual $LauncherPath -Expected $expectedLauncher)) {
        throw 'LATTICE_DELIVERY_OFFICIAL_LAUNCHER_LAYOUT_REJECTED'
    }
    Assert-NoReparseAncestor -Path $bundleRoot -Boundary $RepositoryRoot
    $packageAcl = Get-RestrictedDirectoryAclEvidence -Path $installRoot -RequireProtected

    $manifestPath = Join-Path $bundleRoot 'codex-package.json'
    Assert-RegularFile -Path $manifestPath
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (
        [int]$manifest.layoutVersion -ne 1 -or
        [string]$manifest.version -ne $officialCodexVersion -or
        [string]$manifest.target -ne 'x86_64-pc-windows-msvc' -or
        [string]$manifest.variant -ne 'codex' -or
        [string]$manifest.entrypoint -ne 'bin/codex.exe' -or
        [string]$manifest.resourcesDir -ne 'codex-resources'
    ) {
        throw 'LATTICE_DELIVERY_OFFICIAL_PACKAGE_MANIFEST_REJECTED'
    }

    $platformPackagePath = Join-Path $platformRoot 'package.json'
    $mainPackagePath = Join-Path $installRoot 'node_modules\@openai\codex\package.json'
    Assert-RegularFile -Path $platformPackagePath
    Assert-RegularFile -Path $mainPackagePath
    $platformPackage = Get-Content -LiteralPath $platformPackagePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $mainPackage = Get-Content -LiteralPath $mainPackagePath -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($package in @($platformPackage, $mainPackage)) {
        if (
            [string]$package.name -ne '@openai/codex' -or
            [string]$package.repository.url -ne 'git+https://github.com/openai/codex.git' -or
            [string]$package.repository.directory -ne 'codex-cli'
        ) {
            throw 'LATTICE_DELIVERY_OFFICIAL_PACKAGE_SOURCE_REJECTED'
        }
    }
    if (
        [string]$mainPackage.version -ne $officialCodexVersion -or
        [string]$platformPackage.version -ne "$officialCodexVersion-win32-x64"
    ) {
        throw 'LATTICE_DELIVERY_OFFICIAL_PACKAGE_VERSION_REJECTED'
    }

    $launcher = Get-SignedOfficialFileEvidence -Path $expectedLauncher -ExpectedSha256 $officialLauncherSha256
    $sandboxSetup = Get-SignedOfficialFileEvidence `
        -Path (Join-Path $bundleRoot 'codex-resources\codex-windows-sandbox-setup.exe') `
        -ExpectedSha256 $officialSandboxSetupSha256
    $commandRunner = Get-SignedOfficialFileEvidence `
        -Path (Join-Path $bundleRoot 'codex-resources\codex-command-runner.exe') `
        -ExpectedSha256 $officialCommandRunnerSha256

    $versionOutput = @(& $expectedLauncher '--version' 2>&1)
    if ($LASTEXITCODE -ne 0 -or $versionOutput.Count -ne 1 -or [string]$versionOutput[0] -ne $officialLauncherVersion) {
        throw 'LATTICE_DELIVERY_OFFICIAL_VERSION_REJECTED'
    }
    return [ordered]@{
        version = $officialCodexVersion
        launcher = $launcher
        sandbox_setup = $sandboxSetup
        command_runner = $commandRunner
        package_manifest_sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        source = 'git+https://github.com/openai/codex.git'
        package_acl = $packageAcl
    }
}

function Get-OfficialCodexHomeEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$CodexHomePath,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot
    )

    $codexStateDir = Get-CanonicalPath -Path $CodexHomePath
    Assert-NoReparsePath -Path $codexStateDir
    $stateItem = Get-Item -LiteralPath $codexStateDir -Force -ErrorAction SilentlyContinue
    if ($null -eq $stateItem -or -not $stateItem.PSIsContainer) {
        throw 'LATTICE_DELIVERY_OFFICIAL_CODEX_HOME_REJECTED'
    }
    $repositoryPrefix = (Get-CanonicalPath -Path $RepositoryRoot) + [System.IO.Path]::DirectorySeparatorChar
    if ($codexStateDir.StartsWith($repositoryPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'LATTICE_DELIVERY_OFFICIAL_CODEX_HOME_OVERLAP'
    }
    $ambientHomes = @()
    if (-not [string]::IsNullOrWhiteSpace($env:CODEX_HOME)) {
        $ambientHomes += $env:CODEX_HOME
    }
    if (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) {
        $ambientHomes += (Join-Path $env:USERPROFILE '.codex')
    }
    foreach ($ambientPath in $ambientHomes) {
        if (Test-ExactPath -Actual $codexStateDir -Expected $ambientPath) {
            throw 'LATTICE_DELIVERY_OFFICIAL_AMBIENT_CODEX_HOME_REJECTED'
        }
    }

    $ownershipMarker = Join-Path $codexStateDir '.lattice-codex-home-v1'
    Assert-RegularFile -Path $ownershipMarker
    $expectedOwnershipMarker = [System.Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
    $actualOwnershipMarker = [System.IO.File]::ReadAllBytes($ownershipMarker)
    if ([Convert]::ToBase64String($actualOwnershipMarker) -ne [Convert]::ToBase64String($expectedOwnershipMarker)) {
        throw 'LATTICE_DELIVERY_OFFICIAL_CODEX_HOME_MARKER_REJECTED'
    }

    $authStatePath = Join-Path $codexStateDir 'auth.json'
    $configPath = Join-Path $codexStateDir 'config.toml'
    Assert-RegularFile -Path $authStatePath
    Assert-RegularFile -Path $configPath
    foreach ($isolatedFile in @($ownershipMarker, $authStatePath, $configPath)) {
        Assert-SingleLinkFile -Path $isolatedFile
        $null = Get-RestrictedDirectoryAclEvidence -Path $isolatedFile
    }
    $actualConfigBytes = [System.IO.File]::ReadAllBytes($configPath)
    if ([Convert]::ToBase64String($actualConfigBytes) -ne [Convert]::ToBase64String($codexHomeConfigBytes)) {
        throw 'LATTICE_DELIVERY_OFFICIAL_CODEX_HOME_SAFETY_REJECTED'
    }

    $aclEvidence = Get-RestrictedDirectoryAclEvidence -Path $codexStateDir -RequireProtected
    return [ordered]@{
        path = $codexStateDir
        owner_sid = [string]$aclEvidence.owner_sid
        acl_protected = [bool]$aclEvidence.acl_protected
        auth_present = $true
        sandbox_mode = 'workspace-write'
        windows_sandbox = 'elevated'
    }
}

function Invoke-OfficialCodexDiagnostic {
    if ($OfficialCodex) {
        throw 'LATTICE_DELIVERY_DIAGNOSTIC_LIVE_CONFLICT'
    }
    if (
        [string]::IsNullOrWhiteSpace($OfficialLauncherPath) -or
        [string]::IsNullOrWhiteSpace($OfficialCodexHomePath)
    ) {
        throw 'LATTICE_DELIVERY_OFFICIAL_CONFIGURATION_MISSING'
    }
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $bundle = Get-OfficialCodexBundleEvidence `
        -LauncherPath $OfficialLauncherPath `
        -RepositoryRoot $repositoryRoot
    $codexState = Get-OfficialCodexHomeEvidence `
        -CodexHomePath $OfficialCodexHomePath `
        -RepositoryRoot $repositoryRoot
    Write-Output 'LATTICE_OFFICIAL_CODEX_DIAGNOSTIC=PASS'
    Write-Output (([ordered]@{
        status = 'PASS'
        component = 'official-codex-windows-diagnostic'
        delivery_timeout_seconds = [int]$officialDeliveryTimeoutSeconds
        bundle = $bundle
        codex_home = $codexState
    }) | ConvertTo-Json -Depth 6 -Compress)
}

function New-OfficialLiveAttemptLatch {
    param(
        [Parameter(Mandatory = $true)][string]$FixtureParent,
        [Parameter(Mandatory = $true)][string]$FixtureId,
        [Parameter(Mandatory = $true)]$BundleEvidence,
        [Parameter(Mandatory = $true)]$CodexHomeEvidence
    )

    $latchPath = Join-Path $FixtureParent '.official-codex-live-attempt-v1.json'
    $latch = [ordered]@{
        kind = 'LATTICE_OFFICIAL_CODEX_LIVE_ATTEMPT_V1'
        fixture_id = $FixtureId
        started_at_utc = [DateTime]::UtcNow.ToString('o')
        launcher_path = [string]$BundleEvidence.launcher.path
        launcher_sha256 = [string]$BundleEvidence.launcher.sha256
        sandbox_setup_sha256 = [string]$BundleEvidence.sandbox_setup.sha256
        command_runner_sha256 = [string]$BundleEvidence.command_runner.sha256
        codex_home = [string]$CodexHomeEvidence.path
        auth_present = [bool]$CodexHomeEvidence.auth_present
        safety = 'workspace-write;elevated;stdio-only'
    }
    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes(
        (($latch | ConvertTo-Json -Depth 5 -Compress) + "`n")
    )
    try {
        $stream = [System.IO.File]::Open(
            $latchPath,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::Write,
            [System.IO.FileShare]::None
        )
        try {
            $stream.Write($bytes, 0, $bytes.Length)
            $stream.Flush($true)
        }
        finally {
            $stream.Dispose()
        }
    }
    catch [System.IO.IOException] {
        throw 'LATTICE_DELIVERY_OFFICIAL_ATTEMPT_ALREADY_RECORDED'
    }
    return $latchPath
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

function Assert-ExactEvidenceProperties {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][string[]]$Allowed,
        [Parameter(Mandatory = $true)][string]$RejectionCode
    )

    $actual = @($Evidence.PSObject.Properties.Name | Sort-Object -CaseSensitive)
    $expected = @($Allowed | Sort-Object -CaseSensitive)
    if ($actual.Count -ne $expected.Count) {
        throw $RejectionCode
    }
    for ($index = 0; $index -lt $expected.Count; $index++) {
        if (-not [string]::Equals(
            [string]$actual[$index],
            [string]$expected[$index],
            [System.StringComparison]::Ordinal
        )) {
            throw $RejectionCode
        }
    }
}

function Assert-GraphEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][string]$RejectionCode
    )

    [uint32]$recordCount = 0
    [uint32]$resultCount = 0
    $recordCountValid = [uint32]::TryParse(
        [string]$Evidence.graph_record_count,
        [System.Globalization.NumberStyles]::None,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [ref]$recordCount
    )
    $resultCountValid = [uint32]::TryParse(
        [string]$Evidence.graph_result_count,
        [System.Globalization.NumberStyles]::None,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [ref]$resultCount
    )
    if (
        [string]$Evidence.graph_status -ne 'COMPLETED' -or
        [string]$Evidence.graph_project_id -ne 'task032-delivery' -or
        [string]$Evidence.graph_commit_sha -ne [string]$Evidence.commit_sha -or
        [string]$Evidence.graph_commit_sha -notmatch '^[0-9a-f]{40}([0-9a-f]{24})?$' -or
        [string]$Evidence.graph_query_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.graph_analysis_digest -notmatch '^[0-9a-f]{64}$' -or
        -not $recordCountValid -or
        $recordCount -eq 0 -or
        [string]$Evidence.graph_persistence_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.graph_retrieval_digest -notmatch '^[0-9a-f]{64}$' -or
        -not $resultCountValid -or
        $resultCount -eq 0 -or
        $resultCount -gt $recordCount -or
        [string]$Evidence.graph_receipt_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.graph_database_identity_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.graph_extension_manifest_digest -notmatch '^[0-9a-f]{64}$'
    ) {
        throw $RejectionCode
    }
}

function Get-GraphExecutionFootprintDigest {
    param([Parameter(Mandatory = $true)][string]$FixtureRoot)

    $canonicalFixture = Get-CanonicalPath -Path $FixtureRoot
    $entries = [System.Collections.Generic.List[string]]::new()
    foreach ($relativeRoot in @('graph-memory\snapshots', 'graph-memory\staging')) {
        $root = Get-CanonicalPath -Path (Join-Path $canonicalFixture $relativeRoot)
        Assert-NoReparseAncestor -Path $root -Boundary $canonicalFixture
        $rootItem = Get-Item -LiteralPath $root -Force -ErrorAction SilentlyContinue
        if (
            $null -eq $rootItem -or
            -not $rootItem.PSIsContainer -or
            ($rootItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
        ) {
            throw 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_REJECTED'
        }
        $normalizedRoot = $relativeRoot.Replace('\', '/')
        $entries.Add(('D|{0}|{1}' -f $normalizedRoot, $rootItem.LastWriteTimeUtc.Ticks))

        $pending = [System.Collections.Generic.Stack[string]]::new()
        $pending.Push($root)
        while ($pending.Count -gt 0) {
            $directory = $pending.Pop()
            foreach ($item in @(Get-ChildItem -LiteralPath $directory -Force)) {
                if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                    throw 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_REJECTED'
                }
                $fullPath = Get-CanonicalPath -Path $item.FullName
                $fixturePrefix = $canonicalFixture + [System.IO.Path]::DirectorySeparatorChar
                if (-not $fullPath.StartsWith(
                    $fixturePrefix,
                    [System.StringComparison]::OrdinalIgnoreCase
                )) {
                    throw 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_REJECTED'
                }
                $relativePath = $fullPath.Substring($fixturePrefix.Length).Replace('\', '/')
                if ($item.PSIsContainer) {
                    $entries.Add(('D|{0}|{1}' -f $relativePath, $item.LastWriteTimeUtc.Ticks))
                    $pending.Push($fullPath)
                    continue
                }
                if (
                    -not ($item -is [System.IO.FileInfo]) -or
                    -not (Test-Path -LiteralPath $fullPath -PathType Leaf)
                ) {
                    throw 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_REJECTED'
                }
                $sha256 = (Get-FileHash -LiteralPath $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
                $entries.Add((
                    'F|{0}|{1}|{2}|{3}' -f
                        $relativePath,
                        $item.Length,
                        $item.LastWriteTimeUtc.Ticks,
                        $sha256
                ))
            }
        }
    }

    $entries.Sort([System.StringComparer]::Ordinal)
    $bytes = [System.Text.Encoding]::UTF8.GetBytes(($entries -join "`n") + "`n")
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        return [System.BitConverter]::ToString(
            $hasher.ComputeHash($bytes)
        ).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
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

function Stop-RuntimeProcessTree {
    param([Parameter(Mandatory = $true)]$Process)

    if ($Process.HasExited) {
        return
    }
    $taskkill = Join-Path $env:SystemRoot 'System32\taskkill.exe'
    if (Test-Path -LiteralPath $taskkill -PathType Leaf) {
        $killer = Start-Process `
            -FilePath $taskkill `
            -ArgumentList @('/PID', [string]$Process.Id, '/T', '/F') `
            -WindowStyle Hidden `
            -PassThru
        if (-not $killer.WaitForExit(5000)) {
            try { $killer.Kill() } catch {}
        }
        $killer.Dispose()
    }
    if (-not $Process.HasExited) {
        try { $Process.Kill() } catch {}
    }
    $null = $Process.WaitForExit(5000)
}

function ConvertFrom-UniqueJsonObject {
    param([Parameter(Mandatory = $true)][string]$Json)

    try {
        $null = Add-Type -AssemblyName System.Runtime.Serialization -ErrorAction Stop
        $settings = New-Object System.Runtime.Serialization.Json.DataContractJsonSerializerSettings
        $settings.UseSimpleDictionaryFormat = $true
        $dictionaryType = [System.Collections.Generic.Dictionary[string, object]]
        $serializer = New-Object System.Runtime.Serialization.Json.DataContractJsonSerializer(
            $dictionaryType,
            $settings
        )
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Json)
        $stream = New-Object System.IO.MemoryStream(, $bytes)
        try {
            $dictionary = $serializer.ReadObject($stream)
        }
        finally {
            $stream.Dispose()
        }
        if ($null -eq $dictionary) {
            throw 'JSON_OBJECT_REQUIRED'
        }
        return $Json | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw 'LATTICE_DELIVERY_RUNTIME_JSON_OBJECT_REJECTED'
    }
}

function Assert-ExactJsonEvidenceTypes {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][string[]]$StringFields,
        [Parameter(Mandatory = $true)][string[]]$IntegerFields,
        [Parameter(Mandatory = $true)][string[]]$StringArrayFields,
        [Parameter(Mandatory = $true)][string]$RejectionCode
    )

    foreach ($name in $StringFields) {
        if (-not ($Evidence.PSObject.Properties[$name].Value -is [string])) {
            throw $RejectionCode
        }
    }
    foreach ($name in $IntegerFields) {
        $value = $Evidence.PSObject.Properties[$name].Value
        if (-not ($value -is [int]) -and -not ($value -is [long])) {
            throw $RejectionCode
        }
    }
    foreach ($name in $StringArrayFields) {
        $value = $Evidence.PSObject.Properties[$name].Value
        if (-not ($value -is [System.Array]) -or $value.Rank -ne 1) {
            throw $RejectionCode
        }
        foreach ($item in $value) {
            if (-not ($item -is [string])) {
                throw $RejectionCode
            }
        }
    }
}

function Invoke-RuntimeJson {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [int]$TimeoutMilliseconds = 615000,
        [switch]$AllowReconciliationEnvelope,
        [switch]$AllowCompletedDeliveryEnvelope,
        [switch]$AllowCompletedStatusEnvelope,
        [string]$ExpectedLauncher,
        [string]$ExpectedLauncherSha256,
        [string]$ExpectedDeliveryRoot,
        [ValidateSet('', 'SCRIPTED_ACCEPTANCE', 'OFFICIAL_CODEX_APP_SERVER')]
        [string]$ExpectedCodexMode = ''
    )

    Assert-RegularFile -Path $Executable
    if ($Arguments -contains '--password') {
        throw 'LATTICE_DELIVERY_PASSWORD_ARGUMENT_FORBIDDEN'
    }
    if ($TimeoutMilliseconds -lt 1000 -or $TimeoutMilliseconds -gt 615000) {
        throw 'LATTICE_DELIVERY_RUNTIME_TIMEOUT_INVALID'
    }
    $fixtureRoot = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FIXTURE_ROOT')
    $evidenceRoot = Join-Path $fixtureRoot 'evidence'
    Assert-NoReparseAncestor -Path $evidenceRoot -Boundary (Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..'))
    $captureId = [Guid]::NewGuid().ToString('N')
    $stdoutPath = Join-Path $evidenceRoot "runtime-$captureId.stdout"
    $stderrPath = Join-Path $evidenceRoot "runtime-$captureId.stderr"
    $quotedArguments = foreach ($argument in $Arguments) {
        if ($argument -match '["\r\n]' -or $argument.EndsWith('\')) {
            throw 'LATTICE_DELIVERY_RUNTIME_ARGUMENT_REJECTED'
        }
        '"' + $argument + '"'
    }
    $process = Start-Process `
        -FilePath $Executable `
        -ArgumentList $quotedArguments `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath `
        -WindowStyle Hidden `
        -PassThru
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    try {
        while (-not $process.HasExited) {
            foreach ($capturePath in @($stdoutPath, $stderrPath)) {
                if ((Test-Path -LiteralPath $capturePath) -and (Get-Item -LiteralPath $capturePath).Length -gt 32768) {
                    Stop-RuntimeProcessTree -Process $process
                    throw 'LATTICE_DELIVERY_RUNTIME_OUTPUT_INVALID'
                }
            }
            if ([DateTime]::UtcNow -ge $deadline) {
                Stop-RuntimeProcessTree -Process $process
                throw 'LATTICE_DELIVERY_RUNTIME_WATCHDOG_TIMEOUT'
            }
            Start-Sleep -Milliseconds 100
            $process.Refresh()
        }
        $process.WaitForExit()
        $exitCode = $process.ExitCode
    }
    finally {
        $process.Dispose()
    }
    $stdout = if (Test-Path -LiteralPath $stdoutPath) { [IO.File]::ReadAllText($stdoutPath) } else { '' }
    $stderr = if (Test-Path -LiteralPath $stderrPath) { [IO.File]::ReadAllText($stderrPath) } else { '' }
    $raw = (($stdout.Trim(), $stderr.Trim()) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join "`n"
    if ($exitCode -ne 0) {
        $password = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PASSWORD', 'Process')
        if ($raw.Length -gt 32768 -or (-not [string]::IsNullOrEmpty($password) -and $raw.Contains($password))) {
            throw 'LATTICE_DELIVERY_RUNTIME_FAILURE_OUTPUT_REJECTED'
        }
        if ($AllowReconciliationEnvelope) {
            try {
                $reconciliation = ConvertFrom-UniqueJsonObject -Json $raw
                if ([string]$reconciliation.status -eq 'RECONCILIATION_REQUIRED') {
                    return $reconciliation
                }
            }
            catch {}
        }
        if ($AllowCompletedDeliveryEnvelope) {
            if (
                [string]::IsNullOrWhiteSpace($ExpectedLauncher) -or
                [string]::IsNullOrWhiteSpace($ExpectedLauncherSha256) -or
                [string]::IsNullOrWhiteSpace($ExpectedDeliveryRoot) -or
                [string]::IsNullOrWhiteSpace($ExpectedCodexMode)
            ) {
                throw 'LATTICE_DELIVERY_COMPLETED_ENVELOPE_CONTEXT_MISSING'
            }
            try {
                $completed = ConvertFrom-UniqueJsonObject -Json $raw
                Assert-DeliveryRunEvidence `
                    -Evidence $completed `
                    -Launcher $ExpectedLauncher `
                    -LauncherSha256 $ExpectedLauncherSha256 `
                    -DeliveryRoot $ExpectedDeliveryRoot `
                    -CodexMode $ExpectedCodexMode
                return $completed
            }
            catch {
                throw 'LATTICE_DELIVERY_COMPLETED_ENVELOPE_REJECTED'
            }
        }
        if ($AllowCompletedStatusEnvelope) {
            if (
                [string]::IsNullOrWhiteSpace($ExpectedLauncher) -or
                [string]::IsNullOrWhiteSpace($ExpectedLauncherSha256) -or
                [string]::IsNullOrWhiteSpace($ExpectedDeliveryRoot) -or
                [string]::IsNullOrWhiteSpace($ExpectedCodexMode)
            ) {
                throw 'LATTICE_DELIVERY_COMPLETED_STATUS_CONTEXT_MISSING'
            }
            try {
                $completedStatus = ConvertFrom-UniqueJsonObject -Json $raw
                Assert-DurableStatusEvidence `
                    -Evidence $completedStatus `
                    -Launcher $ExpectedLauncher `
                    -LauncherSha256 $ExpectedLauncherSha256 `
                    -DeliveryRoot $ExpectedDeliveryRoot `
                    -CodexMode $ExpectedCodexMode
                return $completedStatus
            }
            catch {
                throw 'LATTICE_DELIVERY_COMPLETED_STATUS_REJECTED'
            }
        }
        if ([string]::IsNullOrWhiteSpace($raw)) {
            throw "LATTICE_DELIVERY_RUNTIME_FAILED_EXIT_$exitCode"
        }
        throw "LATTICE_DELIVERY_RUNTIME_FAILED: $raw"
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
        return ConvertFrom-UniqueJsonObject -Json $raw
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

    Assert-ExactEvidenceProperties `
        -Evidence $Evidence `
        -Allowed $deliveryEvidenceProperties `
        -RejectionCode 'LATTICE_DELIVERY_RUN_EVIDENCE_ALLOWLIST_REJECTED'
    Assert-ExactJsonEvidenceTypes `
        -Evidence $Evidence `
        -StringFields @($deliveryEvidenceProperties | Where-Object { $_ -notin @('schema_file_count', 'changed_paths', 'graph_record_count', 'graph_result_count') }) `
        -IntegerFields @('schema_file_count', 'graph_record_count', 'graph_result_count') `
        -StringArrayFields @('changed_paths') `
        -RejectionCode 'LATTICE_DELIVERY_RUN_EVIDENCE_TYPE_REJECTED'
    $expectedRequestId = 'task032-request-' + (Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID')

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
        [string]$Evidence.request_id -ne $expectedRequestId -or
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
    Assert-GraphEvidence `
        -Evidence $Evidence `
        -RejectionCode 'LATTICE_DELIVERY_RUN_GRAPH_EVIDENCE_REJECTED'
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

    Assert-ExactEvidenceProperties `
        -Evidence $Evidence `
        -Allowed $deliveryEvidenceProperties `
        -RejectionCode 'LATTICE_DELIVERY_STATUS_EVIDENCE_ALLOWLIST_REJECTED'
    Assert-ExactJsonEvidenceTypes `
        -Evidence $Evidence `
        -StringFields @($deliveryEvidenceProperties | Where-Object { $_ -notin @('schema_file_count', 'changed_paths', 'graph_record_count', 'graph_result_count') }) `
        -IntegerFields @('schema_file_count', 'graph_record_count', 'graph_result_count') `
        -StringArrayFields @('changed_paths') `
        -RejectionCode 'LATTICE_DELIVERY_STATUS_EVIDENCE_TYPE_REJECTED'
    $expectedRequestId = 'task032-request-' + (Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID')
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
        [string]$Evidence.request_id -ne $expectedRequestId -or
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
    Assert-GraphEvidence `
        -Evidence $Evidence `
        -RejectionCode 'LATTICE_DELIVERY_STATUS_GRAPH_EVIDENCE_REJECTED'
}

function Assert-DeliveryRestartCrossBinding {
    param(
        [Parameter(Mandatory = $true)]$RunEvidence,
        [Parameter(Mandatory = $true)]$StatusEvidence
    )

    foreach ($name in (@(
        'repository_path', 'test', 'test_command_id', 'commit_sha', 'parent_sha',
        'baseline_commit', 'launcher_path', 'version', 'launcher_sha256',
        'schema_bundle_sha256', 'schema_file_count', 'thread_id', 'turn_id',
        'codex_runtime', 'intent_digest', 'outcome_digest', 'profile', 'request_id',
        'configuration_digest', 'receipt_digest'
    ) + $graphEvidenceProperties)) {
        if ([string]$StatusEvidence.$name -ne [string]$RunEvidence.$name) {
            throw 'LATTICE_DELIVERY_RESTART_CROSS_BINDING_REJECTED'
        }
    }
}

function Assert-InternalEnvironment {
    $mode = Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_CODEX_MODE'
    $regressionMode = Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_DEADLINE_REGRESSION'
    $hostName = Get-RequiredEnvironment -Name 'LATTICE_TASK019_HOST'
    $port = Get-RequiredEnvironment -Name 'LATTICE_TASK019_PORT'
    $postgresRunId = Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID'
    $null = Get-RequiredEnvironment -Name 'LATTICE_TASK019_PASSWORD'
    if (
        $mode -ne $codexMode -or
        $regressionMode -notin @('0', '1') -or
        (($regressionMode -eq '1') -ne $deadlineRegression) -or
        ($deadlineRegression -and $mode -ne 'SCRIPTED_ACCEPTANCE') -or
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
    $graphFootprintEvidencePath = Get-CanonicalPath -Path (Join-Path (Split-Path -Parent $runEvidencePath) 'graph-execution-footprint.json')

    $repositoryOwnedPaths = @(
        $fixtureRoot, $runtime, $schemaDirectory, $deliveryRoot,
        $runEvidencePath, $graphFootprintEvidencePath
    )
    if ($codexMode -eq 'SCRIPTED_ACCEPTANCE') {
        $repositoryOwnedPaths += @($launcher, $codexHome)
    }
    foreach ($path in $repositoryOwnedPaths) {
        Assert-NoReparseAncestor -Path $path -Boundary $repositoryRoot
    }
    Assert-RegularFile -Path $runtime
    Assert-RegularFile -Path $launcher
    Assert-RegularFile -Path $gitExe
    if (
        (Test-Path -LiteralPath $schemaDirectory) -or
        (Test-Path -LiteralPath $deliveryRoot) -or
        (Test-Path -LiteralPath $runEvidencePath) -or
        (Test-Path -LiteralPath $graphFootprintEvidencePath)
    ) {
        throw 'LATTICE_DELIVERY_RUN_TARGET_NOT_FRESH'
    }
    if ($launcherSha256 -notmatch '^[0-9a-f]{64}$') {
        throw 'LATTICE_DELIVERY_LAUNCHER_DIGEST_INVALID'
    }

    $officialBundleEvidence = $null
    $officialCodexHomeEvidence = $null
    if ($codexMode -eq 'OFFICIAL_CODEX_APP_SERVER') {
        $officialBundleEvidence = Get-OfficialCodexBundleEvidence `
            -LauncherPath $launcher `
            -RepositoryRoot $repositoryRoot
        $officialCodexHomeEvidence = Get-OfficialCodexHomeEvidence `
            -CodexHomePath $codexHome `
            -RepositoryRoot $repositoryRoot
        if (-not [string]::Equals(
            $launcherSha256,
            [string]$officialBundleEvidence.launcher.sha256,
            [System.StringComparison]::Ordinal
        )) {
            throw 'LATTICE_DELIVERY_OFFICIAL_LAUNCHER_DIGEST_REJECTED'
        }
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
        '--timeout-seconds', $deliveryTimeoutSeconds,
        '--postgres-host', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_HOST'),
        '--postgres-port', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_PORT'),
        '--postgres-run-id', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID')
    )
    if ($codexMode -eq 'OFFICIAL_CODEX_APP_SERVER') {
        $fixtureParent = Get-CanonicalPath -Path (Split-Path -Parent $fixtureRoot)
        Assert-NoReparseAncestor -Path $fixtureParent -Boundary $repositoryRoot
        $null = New-OfficialLiveAttemptLatch `
            -FixtureParent $fixtureParent `
            -FixtureId (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FIXTURE_ID') `
            -BundleEvidence $officialBundleEvidence `
            -CodexHomeEvidence $officialCodexHomeEvidence
    }

    if ($deadlineRegression) {
        $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        $observedFailure = $null
        try {
            $null = Invoke-RuntimeJson -Executable $runtime -TimeoutMilliseconds 45000 -Arguments $arguments
        }
        catch {
            $observedFailure = [string]$_.Exception.Message
        }
        finally {
            $stopwatch.Stop()
        }
        $failurePrefix = 'LATTICE_DELIVERY_RUNTIME_FAILED: '
        $failureEnvelope = $null
        if ($null -ne $observedFailure -and $observedFailure.StartsWith($failurePrefix, [System.StringComparison]::Ordinal)) {
            try {
                $failureEnvelope = $observedFailure.Substring($failurePrefix.Length) | ConvertFrom-Json -ErrorAction Stop
            }
            catch {
                $failureEnvelope = $null
            }
        }
        if (
            $null -eq $failureEnvelope -or
            @($failureEnvelope.PSObject.Properties.Name).Count -ne 3 -or
            [string]$failureEnvelope.code -ne 'LATTICE_DELIVERY_RECONCILIATION_REQUIRED' -or
            [string]$failureEnvelope.message -ne 'LATTICE_DELIVERY_RECONCILIATION_REQUIRED' -or
            [string]$failureEnvelope.status -ne 'ERROR'
        ) {
            throw 'LATTICE_DELIVERY_DEADLINE_REGRESSION_WRONG_RUN_OUTCOME'
        }
        if ($stopwatch.ElapsedMilliseconds -ge 40000) {
            throw 'LATTICE_DELIVERY_FINALIZATION_RESERVE_EXHAUSTED'
        }
        Write-JsonEvidence -Path $runEvidencePath -Value ([ordered]@{
            status = 'RECONCILIATION_REQUIRED'
            component = 'lattice-delivery-deadline-regression-run'
            failure_stage = 'CODEX'
            failure_code = 'CODEX_APP_SERVER_TIMEOUT'
            effect_budget_seconds = 10
            finalization_reserve_seconds = 30
            elapsed_milliseconds = $stopwatch.ElapsedMilliseconds
        })
        return
    }

    $evidence = Invoke-RuntimeJson `
        -Executable $runtime `
        -Arguments $arguments `
        -AllowCompletedDeliveryEnvelope `
        -ExpectedLauncher $launcher `
        -ExpectedLauncherSha256 $launcherSha256 `
        -ExpectedDeliveryRoot $deliveryRoot `
        -ExpectedCodexMode $codexMode
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
    $head = Invoke-BoundedSecretFreeGitHead `
        -Executable $gitExe `
        -RepositoryPath ([string]$evidence.repository_path) `
        -OutputPrefix $runEvidencePath
    if ($head -ne [string]$evidence.commit_sha) {
        throw 'LATTICE_DELIVERY_RUN_COMMIT_BINDING_REJECTED'
    }
    $graphExecutionFootprintDigest = Get-GraphExecutionFootprintDigest -FixtureRoot $fixtureRoot
    Write-JsonEvidence -Path $graphFootprintEvidencePath -Value ([ordered]@{
        kind = 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_V1'
        graph_receipt_digest = [string]$evidence.graph_receipt_digest
        graph_execution_footprint_digest = $graphExecutionFootprintDigest
    })
    Write-JsonEvidence -Path $runEvidencePath -Value $evidence
}

function Invoke-DeliveryStatusPhase {
    Assert-InternalEnvironment
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $fixtureRoot = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FIXTURE_ROOT')
    $runtime = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_RUNTIME_EXE')
    $launcher = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_LAUNCHER')
    $launcherSha256 = Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_LAUNCHER_SHA256'
    $deliveryRoot = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_ROOT')
    $gitExe = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_GIT_EXE')
    $runEvidencePath = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_RUN_EVIDENCE')
    $statusEvidencePath = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_STATUS_EVIDENCE')
    $finalEvidencePath = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FINAL_EVIDENCE')
    $graphFootprintEvidencePath = Get-CanonicalPath -Path (Join-Path (Split-Path -Parent $runEvidencePath) 'graph-execution-footprint.json')

    $repositoryOwnedPaths = @(
        $fixtureRoot, $runtime, $deliveryRoot, $runEvidencePath, $statusEvidencePath,
        $finalEvidencePath, $graphFootprintEvidencePath
    )
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
    if ($deadlineRegression) {
        if (
            [string]$runEvidence.status -ne 'RECONCILIATION_REQUIRED' -or
            [string]$runEvidence.component -ne 'lattice-delivery-deadline-regression-run' -or
            [string]$runEvidence.failure_stage -ne 'CODEX' -or
            [string]$runEvidence.failure_code -ne 'CODEX_APP_SERVER_TIMEOUT' -or
            [int]$runEvidence.effect_budget_seconds -ne 10 -or
            [int]$runEvidence.finalization_reserve_seconds -ne 30 -or
            [long]$runEvidence.elapsed_milliseconds -ge 40000
        ) {
            throw 'LATTICE_DELIVERY_DEADLINE_RUN_EVIDENCE_REJECTED'
        }
        $status = Invoke-RuntimeJson -Executable $runtime -TimeoutMilliseconds 60000 -AllowReconciliationEnvelope -Arguments @(
            'delivery-status',
            '--postgres-host', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_HOST'),
            '--postgres-port', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_PORT'),
            '--postgres-run-id', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID')
        )
        Assert-ReconciliationStatusEvidence -Evidence $status

        $fixtureRoot = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FIXTURE_ROOT')
        $invocationLog = Get-CanonicalPath -Path (Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_SCRIPTED_INVOCATION_LOG')
        Assert-NoReparseAncestor -Path $invocationLog -Boundary $repositoryRoot
        if (-not (Test-ExactPath -Actual $invocationLog -Expected (Join-Path $fixtureRoot 'scripted-server-invocations.log'))) {
            throw 'LATTICE_DELIVERY_INVOCATION_LOG_PATH_REJECTED'
        }
        Assert-RegularFile -Path $invocationLog
        $invocationBytes = [System.IO.File]::ReadAllBytes($invocationLog)
        $expectedInvocationBytes = [System.Text.Encoding]::ASCII.GetBytes("server`n")
        if ([Convert]::ToBase64String($invocationBytes) -ne [Convert]::ToBase64String($expectedInvocationBytes)) {
            throw 'LATTICE_DELIVERY_SCRIPTED_CHILD_RESENT'
        }
        if (Test-Path -LiteralPath (Join-Path $deliveryRoot 'repo\answer.txt')) {
            throw 'LATTICE_DELIVERY_TIMED_OUT_CHILD_COMPLETED_EFFECT'
        }

        Write-JsonEvidence -Path $statusEvidencePath -Value $status
        Write-JsonEvidence -Path $finalEvidencePath -Value ([ordered]@{
            status = 'PASS'
            component = 'lattice-delivery-deadline-regression'
            codex_mode = $codexMode
            durable_status = [string]$status.status
            failure_stage = [string]$status.failure_stage
            failure_code = [string]$status.failure_code
            postgres_restarted_before_status = $true
            fresh_status_replayed = $true
            no_resend = $true
            app_server_invocation_count = 1
            elapsed_milliseconds = [long]$runEvidence.elapsed_milliseconds
            fixture_id = Get-RequiredEnvironment -Name 'LATTICE_DELIVERY_FIXTURE_ID'
            postgres_run_id = Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID'
            request_id = [string]$status.request_id
            configuration_digest = [string]$status.configuration_digest
            intent_digest = [string]$status.intent_digest
            outcome_digest = [string]$status.outcome_digest
            receipt_digest = [string]$status.receipt_digest
        })
        return
    }

    Assert-DeliveryRunEvidence `
        -Evidence $runEvidence `
        -Launcher $launcher `
        -LauncherSha256 $launcherSha256 `
        -DeliveryRoot $deliveryRoot `
        -CodexMode $codexMode
    $graphFootprintEvidence = Read-JsonEvidence -Path $graphFootprintEvidencePath
    Assert-ExactEvidenceProperties `
        -Evidence $graphFootprintEvidence `
        -Allowed @('kind', 'graph_receipt_digest', 'graph_execution_footprint_digest') `
        -RejectionCode 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_MARKER_REJECTED'
    if (
        [string]$graphFootprintEvidence.kind -ne 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_V1' -or
        [string]$graphFootprintEvidence.graph_receipt_digest -ne [string]$runEvidence.graph_receipt_digest -or
        [string]$graphFootprintEvidence.graph_execution_footprint_digest -notmatch '^[0-9a-f]{64}$'
    ) {
        throw 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_MARKER_REJECTED'
    }
    $graphFootprintBeforeStatus = Get-GraphExecutionFootprintDigest -FixtureRoot $fixtureRoot
    if ($graphFootprintBeforeStatus -ne [string]$graphFootprintEvidence.graph_execution_footprint_digest) {
        throw 'LATTICE_GRAPH_EXECUTION_FOOTPRINT_CHANGED_BEFORE_STATUS'
    }
    $status = Invoke-RuntimeJson `
        -Executable $runtime `
        -TimeoutMilliseconds 60000 `
        -AllowCompletedStatusEnvelope `
        -ExpectedLauncher $launcher `
        -ExpectedLauncherSha256 $launcherSha256 `
        -ExpectedDeliveryRoot $deliveryRoot `
        -ExpectedCodexMode $codexMode `
        -Arguments @(
        'delivery-status',
        '--postgres-host', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_HOST'),
        '--postgres-port', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_PORT'),
        '--postgres-run-id', (Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID')
    )
    $graphFootprintAfterStatus = Get-GraphExecutionFootprintDigest -FixtureRoot $fixtureRoot
    if ($graphFootprintAfterStatus -ne $graphFootprintBeforeStatus) {
        throw 'LATTICE_GRAPHIFY_REEXECUTED_DURING_FRESH_STATUS'
    }
    Assert-DurableStatusEvidence `
        -Evidence $status `
        -Launcher $launcher `
        -LauncherSha256 $launcherSha256 `
        -DeliveryRoot $deliveryRoot `
        -CodexMode $codexMode

    Assert-DeliveryRestartCrossBinding -RunEvidence $runEvidence -StatusEvidence $status

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
        graph_status = [string]$status.graph_status
        graph_project_id = [string]$status.graph_project_id
        graph_commit_sha = [string]$status.graph_commit_sha
        graph_query_digest = [string]$status.graph_query_digest
        graph_analysis_digest = [string]$status.graph_analysis_digest
        graph_record_count = [uint32]$status.graph_record_count
        graph_persistence_digest = [string]$status.graph_persistence_digest
        graph_retrieval_digest = [string]$status.graph_retrieval_digest
        graph_result_count = [uint32]$status.graph_result_count
        graph_receipt_digest = [string]$status.graph_receipt_digest
        graph_database_identity_digest = [string]$status.graph_database_identity_digest
        graph_extension_manifest_digest = [string]$status.graph_extension_manifest_digest
        graph_execution_footprint_unchanged_during_status = $true
        graph_execution_footprint_digest = $graphFootprintAfterStatus
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

    $evidenceRoot = Join-Path $fixtureRoot 'evidence'
    New-Item -ItemType Directory -Path $evidenceRoot -Force:$false | Out-Null
    $fixtureMarker = Join-Path $fixtureRoot '.lattice-delivery-fixture-v1.json'
    $officialBundleEvidence = $null
    $officialCodexHomeEvidence = $null
    if ($OfficialCodex) {
        $launcherPath = Get-CanonicalPath -Path $OfficialLauncherPath
        $codexHome = Get-CanonicalPath -Path $OfficialCodexHomePath
        $officialBundleEvidence = Get-OfficialCodexBundleEvidence `
            -LauncherPath $launcherPath `
            -RepositoryRoot $repositoryRoot
        $officialCodexHomeEvidence = Get-OfficialCodexHomeEvidence `
            -CodexHomePath $codexHome `
            -RepositoryRoot $repositoryRoot
        $launcherSha256 = [string]$officialBundleEvidence.launcher.sha256
        Write-JsonEvidence -Path $fixtureMarker -Value ([ordered]@{
            kind = 'LATTICE_DELIVERY_OFFICIAL_CODEX_ACCEPTANCE_V1'
            fixture_id = $fixtureId
            root = $fixtureRoot
            repository_root = $repositoryRoot
            codex_mode = 'OFFICIAL_CODEX_APP_SERVER'
            launcher_path = [string]$officialBundleEvidence.launcher.path
            launcher_sha256 = $launcherSha256
            sandbox_setup_sha256 = [string]$officialBundleEvidence.sandbox_setup.sha256
            command_runner_sha256 = [string]$officialBundleEvidence.command_runner.sha256
            codex_home = [string]$officialCodexHomeEvidence.path
            auth_present = [bool]$officialCodexHomeEvidence.auth_present
        })
    }
    else {
        $codexHome = Join-Path $fixtureRoot 'codex-home'
        New-Item -ItemType Directory -Path $codexHome -Force:$false | Out-Null
        [System.IO.File]::WriteAllBytes(
            (Join-Path $codexHome '.lattice-codex-home-v1'),
            [System.Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
        )
        [System.IO.File]::WriteAllBytes(
            (Join-Path $codexHome 'auth.json'),
            [System.Text.Encoding]::ASCII.GetBytes("{}`n")
        )
        [System.IO.File]::WriteAllBytes(
            (Join-Path $codexHome 'config.toml'),
            $codexHomeConfigBytes
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
            ('echo ' + $launcherVersion),
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
    }

    $schemaDirectory = Join-Path $fixtureRoot 'schema'
    $deliveryRoot = Join-Path $fixtureRoot 'delivery'
    $runEvidencePath = Join-Path $evidenceRoot 'delivery-run.json'
    $statusEvidencePath = Join-Path $evidenceRoot 'delivery-status.json'
    $finalEvidencePath = Join-Path $evidenceRoot 'final.json'
    $scriptedInvocationLog = Join-Path $fixtureRoot 'scripted-server-invocations.log'
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
        LATTICE_DELIVERY_DEADLINE_REGRESSION = $(if ($deadlineRegression) { '1' } else { '0' })
        LATTICE_DELIVERY_SCRIPTED_DELAY_MILLISECONDS = $(if ($deadlineRegression) { '20000' } else { '' })
        LATTICE_DELIVERY_SCRIPTED_INVOCATION_LOG = $(if ($deadlineRegression) { $scriptedInvocationLog } else { '' })
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
        if ($deadlineRegression) {
            if (
                [string]$final.status -ne 'PASS' -or
                [string]$final.component -ne 'lattice-delivery-deadline-regression' -or
                [string]$final.codex_mode -ne 'SCRIPTED_ACCEPTANCE' -or
                [string]$final.durable_status -ne 'RECONCILIATION_REQUIRED' -or
                [string]$final.failure_stage -ne 'CODEX' -or
                [string]$final.failure_code -ne 'CODEX_APP_SERVER_TIMEOUT' -or
                [bool]$final.postgres_restarted_before_status -ne $true -or
                [bool]$final.fresh_status_replayed -ne $true -or
                [bool]$final.no_resend -ne $true -or
                [int]$final.app_server_invocation_count -ne 1 -or
                [long]$final.elapsed_milliseconds -ge 40000 -or
                [string]$final.fixture_id -ne $fixtureId
            ) {
                throw 'LATTICE_DELIVERY_DEADLINE_FINAL_EVIDENCE_REJECTED'
            }
            Write-Output 'LATTICE_DELIVERY_DEADLINE_REGRESSION=PASS'
            Write-Output (([ordered]@{
                status = 'PASS'
                component = 'lattice-delivery-deadline-regression'
                evidence_path = $finalEvidencePath
                durable_status = [string]$final.durable_status
                elapsed_milliseconds = [long]$final.elapsed_milliseconds
                fresh_status_replayed = [bool]$final.fresh_status_replayed
                no_resend = [bool]$final.no_resend
            }) | ConvertTo-Json -Compress)
            return
        }
        Assert-ExactEvidenceProperties `
            -Evidence $final `
            -Allowed $finalEvidenceProperties `
            -RejectionCode 'LATTICE_DELIVERY_FINAL_EVIDENCE_ALLOWLIST_REJECTED'
        Assert-GraphEvidence `
            -Evidence $final `
            -RejectionCode 'LATTICE_DELIVERY_FINAL_GRAPH_EVIDENCE_REJECTED'
        if (
            [string]$final.status -ne 'COMPLETED' -or
            [string]$final.component -ne 'lattice-delivery-acceptance' -or
            [string]$final.codex_mode -ne $codexMode -or
            [bool]$final.postgres_restarted_before_status -ne $true -or
            [string]$final.fixture_id -ne $fixtureId -or
            [bool]$final.graph_execution_footprint_unchanged_during_status -ne $true -or
            [string]$final.graph_execution_footprint_digest -notmatch '^[0-9a-f]{64}$'
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
            graph_receipt_digest = [string]$final.graph_receipt_digest
        }) | ConvertTo-Json -Compress)
    }
    finally {
        foreach ($name in $deliveryEnvironmentNames) {
            [Environment]::SetEnvironmentVariable($name, $originalEnvironment[$name], 'Process')
        }
    }
}

function Invoke-RuntimeTerminalEnvelopeSelfTest {
    if ($OfficialCodex -or $DiagnoseOfficialCodex -or $ScriptedDeadlineRegression -or -not [string]::IsNullOrEmpty($InternalPhase)) {
        throw 'LATTICE_RUNTIME_TERMINAL_ENVELOPE_SELF_TEST_MODE_REJECTED'
    }
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $testParent = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target\lattice-delivery-envelope-selftest')
    if (-not (Test-Path -LiteralPath $testParent)) {
        New-Item -ItemType Directory -Path $testParent -Force:$false | Out-Null
    }
    Assert-NoReparseAncestor -Path $testParent -Boundary $repositoryRoot
    $testRoot = Get-CanonicalPath -Path (Join-Path $testParent ([Guid]::NewGuid().ToString('N')))
    New-Item -ItemType Directory -Path $testRoot -Force:$false | Out-Null
    $evidenceRoot = Join-Path $testRoot 'evidence'
    New-Item -ItemType Directory -Path $evidenceRoot -Force:$false | Out-Null
    $deliveryRoot = Join-Path $testRoot 'delivery'
    $cmd = Get-CanonicalPath -Path (Join-Path $env:SystemRoot 'System32\cmd.exe')
    $launcherSha256 = (Get-FileHash -LiteralPath $cmd -Algorithm SHA256).Hash.ToLowerInvariant()
    $runId = '11111111111111111111111111111111'
    $originalFixtureRoot = [Environment]::GetEnvironmentVariable('LATTICE_DELIVERY_FIXTURE_ROOT', 'Process')
    $originalRunId = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_RUN_ID', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('LATTICE_DELIVERY_FIXTURE_ROOT', $testRoot, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_RUN_ID', $runId, 'Process')
        $envelope = [ordered]@{
            status = 'COMPLETED'
            component = 'lattice-delivery'
            launcher_path = $cmd
            version = $launcherVersion
            launcher_sha256 = $launcherSha256
            schema_bundle_sha256 = ('2' * 64)
            schema_file_count = 1
            repository_path = (Join-Path $deliveryRoot 'repo')
            changed_paths = @('answer.txt')
            test = 'FIXED_TEST_PASSED'
            test_command_id = 'git-diff-no-index-exact-answer-v1'
            baseline_commit = ('a' * 40)
            parent_sha = ('a' * 40)
            commit_sha = ('b' * 40)
            thread_id = 'thread-task032-scripted'
            turn_id = 'turn-task032-scripted'
            codex_runtime = 'SCRIPTED_ACCEPTANCE'
            intent_digest = ('3' * 64)
            outcome_digest = ('4' * 64)
            profile = 'task032-codex-postgres-v1'
            request_id = "task032-request-$runId"
            configuration_digest = ('5' * 64)
            receipt_digest = ('6' * 64)
        }

        function Invoke-FakeTerminal {
            param(
                [AllowNull()]$Value,
                [Parameter(Mandatory = $true)][string]$Name,
                [string]$RawJson,
                [switch]$StatusEnvelope
            )
            $fake = Join-Path $testRoot "$Name.cmd"
            $json = if ($PSBoundParameters.ContainsKey('RawJson')) {
                $RawJson
            }
            else {
                $Value | ConvertTo-Json -Depth 8 -Compress
            }
            [System.IO.File]::WriteAllText(
                $fake,
                "@echo off`r`necho $json`r`nexit /b 2`r`n",
                [System.Text.Encoding]::ASCII
            )
            $common = @{
                Executable = $cmd
                Arguments = @('/d', '/c', $fake)
                TimeoutMilliseconds = 10000
                ExpectedLauncher = $cmd
                ExpectedLauncherSha256 = $launcherSha256
                ExpectedDeliveryRoot = $deliveryRoot
                ExpectedCodexMode = 'SCRIPTED_ACCEPTANCE'
            }
            if ($StatusEnvelope) {
                return Invoke-RuntimeJson @common -AllowCompletedStatusEnvelope
            }
            return Invoke-RuntimeJson @common -AllowCompletedDeliveryEnvelope
        }

        $accepted = Invoke-FakeTerminal -Value $envelope -Name 'valid-completed'
        if ([string]$accepted.status -ne 'COMPLETED' -or [string]$accepted.commit_sha -ne ('b' * 40)) {
            throw 'LATTICE_RUNTIME_TERMINAL_ENVELOPE_SELF_TEST_ACCEPT_FAILED'
        }

        $statusEnvelope = [ordered]@{}
        foreach ($entry in $envelope.GetEnumerator()) {
            $statusEnvelope[$entry.Key] = $entry.Value
        }
        $statusEnvelope.component = 'delivery-ledger'
        $acceptedStatus = Invoke-FakeTerminal -Value $statusEnvelope -Name 'valid-completed-status' -StatusEnvelope
        Assert-DeliveryRestartCrossBinding -RunEvidence $accepted -StatusEvidence $acceptedStatus
        $crossBoundTamper = [ordered]@{}
        foreach ($entry in $statusEnvelope.GetEnumerator()) {
            $crossBoundTamper[$entry.Key] = $entry.Value
        }
        $crossBoundTamper.configuration_digest = ('9' * 64)
        $crossBindingRejected = $false
        try {
            Assert-DeliveryRestartCrossBinding -RunEvidence $accepted -StatusEvidence $crossBoundTamper
        }
        catch {
            $crossBindingRejected = [string]$_.Exception.Message -eq 'LATTICE_DELIVERY_RESTART_CROSS_BINDING_REJECTED'
        }
        if (-not $crossBindingRejected) {
            throw 'LATTICE_RUNTIME_TERMINAL_ENVELOPE_SELF_TEST_CROSS_BINDING_FAILED'
        }

        $tamperedCases = @(
            @{ name = 'wrong-request'; field = 'request_id'; value = 'task032-request-22222222222222222222222222222222' },
            @{ name = 'wrong-commit'; field = 'commit_sha'; value = ('a' * 40) },
            @{ name = 'wrong-digest'; field = 'launcher_sha256'; value = ('9' * 64) },
            @{ name = 'extra-field'; field = 'unexpected'; value = 'rejected' }
        )
        foreach ($case in $tamperedCases) {
            $tampered = [ordered]@{}
            foreach ($entry in $envelope.GetEnumerator()) {
                $tampered[$entry.Key] = $entry.Value
            }
            $tampered[$case.field] = $case.value
            $rejected = $false
            try {
                $null = Invoke-FakeTerminal -Value $tampered -Name $case.name
            }
            catch {
                $rejected = [string]$_.Exception.Message -eq 'LATTICE_DELIVERY_COMPLETED_ENVELOPE_REJECTED'
            }
            if (-not $rejected) {
                throw 'LATTICE_RUNTIME_TERMINAL_ENVELOPE_SELF_TEST_REJECTION_FAILED'
            }
        }

        $singletonArrayStatus = [ordered]@{}
        $scalarChangedPaths = [ordered]@{}
        foreach ($entry in $envelope.GetEnumerator()) {
            $singletonArrayStatus[$entry.Key] = $entry.Value
            $scalarChangedPaths[$entry.Key] = $entry.Value
        }
        $singletonArrayStatus.status = [object[]]@('COMPLETED')
        $scalarChangedPaths.changed_paths = 'answer.txt'
        $validRaw = $envelope | ConvertTo-Json -Depth 8 -Compress
        $duplicateStatusRaw = $validRaw.Substring(0, $validRaw.Length - 1) + ',"status":"COMPLETED"}'
        $ambiguousCases = @(
            @{ name = 'duplicate-status'; raw = $duplicateStatusRaw },
            @{ name = 'singleton-array-status'; value = $singletonArrayStatus },
            @{ name = 'scalar-changed-paths'; value = $scalarChangedPaths }
        )
        foreach ($case in $ambiguousCases) {
            $rejected = $false
            try {
                if ($case.ContainsKey('raw')) {
                    $null = Invoke-FakeTerminal -Name $case.name -RawJson $case.raw
                }
                else {
                    $null = Invoke-FakeTerminal -Name $case.name -Value $case.value
                }
            }
            catch {
                $rejected = [string]$_.Exception.Message -eq 'LATTICE_DELIVERY_COMPLETED_ENVELOPE_REJECTED'
            }
            if (-not $rejected) {
                throw 'LATTICE_RUNTIME_TERMINAL_ENVELOPE_SELF_TEST_AMBIGUOUS_JSON_ACCEPTED'
            }
        }
        Write-Output 'LATTICE_RUNTIME_TERMINAL_ENVELOPE_SELF_TEST=PASS'
    }
    finally {
        [Environment]::SetEnvironmentVariable('LATTICE_DELIVERY_FIXTURE_ROOT', $originalFixtureRoot, 'Process')
        [Environment]::SetEnvironmentVariable('LATTICE_TASK019_RUN_ID', $originalRunId, 'Process')
        if ((Test-Path -LiteralPath $testRoot) -and
            (Test-ExactPath -Actual (Split-Path -Parent $testRoot) -Expected $testParent)) {
            Remove-Item -LiteralPath $testRoot -Recurse -Force
        }
    }
}

if ($TestRuntimeTerminalEnvelope) {
    Invoke-RuntimeTerminalEnvelopeSelfTest
    return
}

if ($DiagnoseOfficialCodex) {
    Invoke-OfficialCodexDiagnostic
    return
}

function Assert-ReconciliationStatusEvidence {
    param([Parameter(Mandatory = $true)]$Evidence)

    $required = @(
        'status', 'component', 'profile', 'request_id', 'configuration_digest',
        'intent_digest', 'outcome_digest', 'receipt_digest', 'failure_stage',
        'failure_code'
    )
    foreach ($name in $required) {
        if ($name -notin $Evidence.PSObject.Properties.Name) {
            throw 'LATTICE_DELIVERY_RECONCILIATION_EVIDENCE_INCOMPLETE'
        }
    }
    $expectedRequestId = 'task032-request-' + (Get-RequiredEnvironment -Name 'LATTICE_TASK019_RUN_ID')
    if (
        [string]$Evidence.status -ne 'RECONCILIATION_REQUIRED' -or
        [string]$Evidence.component -ne 'delivery-ledger' -or
        [string]$Evidence.profile -ne 'task032-codex-postgres-v1' -or
        [string]$Evidence.request_id -ne $expectedRequestId -or
        [string]$Evidence.configuration_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.intent_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.outcome_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.receipt_digest -notmatch '^[0-9a-f]{64}$' -or
        [string]$Evidence.failure_stage -ne 'CODEX' -or
        [string]$Evidence.failure_code -ne 'CODEX_APP_SERVER_TIMEOUT'
    ) {
        throw 'LATTICE_DELIVERY_RECONCILIATION_EVIDENCE_REJECTED'
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
