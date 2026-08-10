[CmdletBinding()]
param(
    [ValidateSet('FullChainPreStatus', 'FullChainRun', 'FullChainStatus')]
    [string]$InternalPhase,
    [string]$CodexAuthHome,
    [switch]$HarnessSelfTest,
    [switch]$InternalController,
    [switch]$InternalAdmissionProbe,
    [switch]$HarnessControllerPreflightProbe,
    [switch]$InternalControllerPreflightOnly,
    [switch]$HarnessPublicFailureProbe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$officialCodexVersion = 'codex-cli 0.146.0'
$officialCodexSha256 = 'bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb'
$openClawPackageTarballSha256 = '5bb525f36f471a41239615d321c441778c7e1c007018ed6d84b795be77803276'
$openClawEntrypointSha256 = 'f643b005d6db233a0b45204e8d8e943256874ccc6897b8a6e0cf42a9b376a188'
$openClawVersion = '2026.7.1-2'
$officialNodeVersion = 'v24.16.0'
$officialNodeSha256 = 'b3094d0b49f9ad602262a9921551737bb97637c05dd357a06ae98188d7290aa3'
$officialGitSha256 = '81ef35ae005ca9318018d18e3327578ce939fb99feaad6b2d7c8ab15f3de8db5'
$task037RustToolchain = '1.97.1-x86_64-pc-windows-msvc'
$officialCargoVersion = 'cargo 1.97.1 (c980f4866 2026-06-30)'
$officialCargoSha256 = 'ddfbad20b31b918d3439d070945ec59bbfe037a6ec0ab5b584459e69c8b37d1b'
$officialRustcVersion = 'rustc 1.97.1 (8bab26f4f 2026-07-14)'
$officialRustcSha256 = 'cf79cfd77b0a144c56a0a6af6bf10bcdf095a73718cd4bf2b9d4fe2d2cbded55'
$officialRustdocSha256 = '0c6ccda72363b77afc177b2c9a75ced422e3b88c0930ec1aab552e83a40e381e'
$hermesRuntimeGuestRoot = '/var/tmp/lattice-runtime-targets/hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-errorfix-v1'
$hermesRuntimeManifestSha256 = 'e3a3272b6cead30cd2df1af755df031766475595fdacfb080d0886671b6d1fbb'
$taskBinding = [ordered]@{
    project_id = 'task032-delivery'
    project_snapshot_id = 'task032-delivery:snapshot:1'
    task_id = 'TASK-032'
    revision = '1'
    task_spec_digest = 'b70aa1a7445ea7e7ebe466154d13ea1039f963b5ac4ffe1f7d5094dd8c949e0e'
}
$deliveryConfigBytes = [System.Text.UTF8Encoding]::new($false).GetBytes((@(
    'approval_policy = "never"',
    'sandbox_mode = "workspace-write"',
    'model = "gpt-5.6-sol"',
    'model_reasoning_effort = "low"',
    '',
    '[windows]',
    'sandbox = "unelevated"'
) -join "`n") + "`n")
$codexMarkerBytes = [System.Text.Encoding]::UTF8.GetBytes("lattice.codex-home.v1`n")
$postgresBin = 'C:\Program Files\PostgreSQL\17\bin'
$task037MaxEvidenceFileBytes = 1048576
$script:Task037KnownSecrets = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
$script:Task037AdmissionStages = @(
    'recovery_preflight',
    'supervision_create',
    'gated_controller_start',
    'supervisor_binding',
    'git_identity_preflight',
    'node_identity_preflight',
    'node_runtime_preflight',
    'rust_toolchain_preflight',
    'artifact_identity_preflight',
    'worktree_preflight',
    'inner_pre_acceptance'
)
$script:Task037PhaseGates = @(
    'phase_setup',
    'openclaw_job_baseline',
    'full_chain_gated_start',
    'process_wait',
    'process_exit_snapshot',
    'wrapper_reap',
    'wrapper_exit_match',
    'openclaw_liveness',
    'child_exit_success',
    'process_handle_release',
    'job_cardinality',
    'raw_bounds',
    'response_json',
    'mcp_envelope',
    'ready_evidence',
    'tool_content',
    'pre_status_semantics',
    'success_semantics',
    'phase_meta_projection',
    'success_tool_result',
    'success_tool_error_configuration',
    'success_tool_error_delivery',
    'success_tool_error_graph',
    'success_tool_error_hermes',
    'success_tool_error_hermes_hint_auth',
    'success_tool_error_hermes_hint_quota',
    'success_tool_error_hermes_hint_transport',
    'success_tool_error_hermes_hint_app_server_timeout',
    'success_tool_error_hermes_hint_app_server_turn_status',
    'success_tool_error_hermes_hint_schema',
    'success_tool_error_hermes_hint_app_server',
    'success_tool_error_hermes_app_server_empty_output',
    'success_tool_error_hermes_runtime',
    'success_tool_error_hermes_memory_receipt',
    'success_tool_error_hermes_production_child_exited',
    'success_tool_error_hermes_production_deadline_exceeded',
    'success_tool_error_hermes_diagnostic_rejected',
    'success_tool_error_hermes_unclassified',
    'success_tool_error_transport',
    'success_tool_error_unclassified',
    'success_launcher_binding',
    'success_delivery_contract',
    'success_graph_contract',
    'success_hermes_contract',
    'success_digest_contract',
    'success_finding_items',
    'success_action_items',
    'success_answer_bytes',
    'memory_empty',
    'memory_expected',
    'status_replay',
    'evidence_commit'
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

function Test-PathOverlap {
    param(
        [Parameter(Mandatory = $true)][string]$Left,
        [Parameter(Mandatory = $true)][string]$Right
    )

    $canonicalLeft = Get-CanonicalPath -Path $Left
    $canonicalRight = Get-CanonicalPath -Path $Right
    $leftPrefix = $canonicalLeft + [IO.Path]::DirectorySeparatorChar
    $rightPrefix = $canonicalRight + [IO.Path]::DirectorySeparatorChar
    return (
        (Test-ExactPath -Actual $canonicalLeft -Expected $canonicalRight) -or
        $canonicalLeft.StartsWith($rightPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $canonicalRight.StartsWith($leftPrefix, [StringComparison]::OrdinalIgnoreCase)
    )
}

function Assert-NoReparsePath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $current = Get-CanonicalPath -Path $Path
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw $FailureCode
            }
        }
        $parentInfo = [IO.Directory]::GetParent($current)
        if ($null -eq $parentInfo) { break }
        $parent = $parentInfo.FullName
        if ([string]::Equals($parent, $current, [StringComparison]::OrdinalIgnoreCase)) { break }
        $current = $parent
    }
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
        throw 'TASK037_PATH_OUTSIDE_REPOSITORY'
    }

    $current = $canonicalPath
    while ($true) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw 'TASK037_REPARSE_ANCESTOR_REJECTED'
            }
        }
        if (Test-ExactPath -Actual $current -Expected $canonicalBoundary) {
            break
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrWhiteSpace($parent) -or (Test-ExactPath -Actual $parent -Expected $current)) {
            throw 'TASK037_PATH_ANCESTRY_REJECTED'
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
        throw ('TASK037_FILE_REJECTED|' + $Path)
    }
}

function Get-Task037RestrictedAclEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [switch]$RequireProtected
    )

    $acl = Get-Acl -LiteralPath $Path
    $currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $currentSid = $currentIdentity.User.Value
    $ownerSid = ([Security.Principal.NTAccount]$acl.Owner).Translate(
        [Security.Principal.SecurityIdentifier]
    ).Value
    if (($RequireProtected -and -not $acl.AreAccessRulesProtected) -or $ownerSid -ne $currentSid) {
        throw 'TASK037_RESTRICTED_ACL_REJECTED'
    }
    $allowedSids = @($currentSid, 'S-1-5-18', 'S-1-5-32-544')
    $rules = @($acl.Access)
    if ($rules.Count -ne $allowedSids.Count) {
        throw 'TASK037_RESTRICTED_ACL_REJECTED'
    }
    $observedSids = @()
    foreach ($rule in $rules) {
        $sid = $rule.IdentityReference.Translate(
            [Security.Principal.SecurityIdentifier]
        ).Value
        $observedSids += $sid
        if (
            $sid -notin $allowedSids -or
            $rule.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow -or
            ($RequireProtected -and $rule.IsInherited) -or
            ($rule.FileSystemRights -band [Security.AccessControl.FileSystemRights]::FullControl) -ne
                [Security.AccessControl.FileSystemRights]::FullControl
        ) {
            throw 'TASK037_RESTRICTED_ACL_REJECTED'
        }
    }
    foreach ($requiredSid in $allowedSids) {
        if ($requiredSid -notin $observedSids) {
            throw 'TASK037_RESTRICTED_ACL_REJECTED'
        }
    }
    return [ordered]@{
        owner_sid = $ownerSid
        acl_protected = [bool]$acl.AreAccessRulesProtected
        allowed_sids = $allowedSids
    }
}

function Set-Task037RestrictedDirectoryAcl {
    param([Parameter(Mandatory = $true)][string]$Path)

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        -not $item.PSIsContainer -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK037_RESTRICTED_ACL_REJECTED'
    }
    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $security = [Security.AccessControl.DirectorySecurity]::new()
    $security.SetOwner($currentSid)
    $security.SetAccessRuleProtection($true, $false)
    $inheritance = (
        [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor
        [Security.AccessControl.InheritanceFlags]::ObjectInherit
    )
    foreach ($sidText in @($currentSid.Value, 'S-1-5-18', 'S-1-5-32-544')) {
        $sid = [Security.Principal.SecurityIdentifier]::new($sidText)
        $rule = [Security.AccessControl.FileSystemAccessRule]::new(
            $sid,
            [Security.AccessControl.FileSystemRights]::FullControl,
            $inheritance,
            [Security.AccessControl.PropagationFlags]::None,
            [Security.AccessControl.AccessControlType]::Allow
        )
        $null = $security.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $Path -AclObject $security
    return Get-Task037RestrictedAclEvidence -Path $Path -RequireProtected
}

function Assert-Task037SingleLinkFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    Assert-RegularFile -Path $Path
    $fsutil = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)) 'System32\fsutil.exe'
    Assert-RegularFile -Path $fsutil
    $links = @(& $fsutil 'hardlink' 'list' $Path 2>$null)
    if ($LASTEXITCODE -ne 0 -or $links.Count -ne 1) {
        throw 'TASK037_HARDLINK_REJECTED'
    }
}

function ConvertTo-LowerHex {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    return -join ($Bytes | ForEach-Object { $_.ToString('x2') })
}

function New-RandomBytes {
    param([Parameter(Mandatory = $true)][int]$Count)

    $bytes = New-Object byte[] $Count
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($bytes)
        return $bytes
    }
    finally {
        $rng.Dispose()
    }
}

function New-RandomHex {
    param([Parameter(Mandatory = $true)][int]$Bytes)

    return ConvertTo-LowerHex (New-RandomBytes -Count $Bytes)
}

function Get-FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-StringSha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text)

    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($Text)
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return ConvertTo-LowerHex ($sha256.ComputeHash($bytes))
    }
    finally {
        $sha256.Dispose()
    }
}

function Add-Task037KnownSecret {
    param([string]$Value)

    if (-not [string]::IsNullOrEmpty($Value) -and $Value.Length -ge 8) {
        $null = $script:Task037KnownSecrets.Add($Value)
    }
}

function Add-Task037SecretScalars {
    param($Value)

    if ($null -eq $Value) { return }
    if ($Value -is [string]) {
        Add-Task037KnownSecret -Value ([string]$Value)
        return
    }
    if ($Value -is [Collections.IDictionary]) {
        foreach ($entry in $Value.GetEnumerator()) {
            Add-Task037SecretScalars -Value $entry.Value
        }
        return
    }
    if ($Value -is [Collections.IEnumerable]) {
        foreach ($entry in $Value) { Add-Task037SecretScalars -Value $entry }
        return
    }
    foreach ($property in @($Value.PSObject.Properties)) {
        Add-Task037SecretScalars -Value $property.Value
    }
}

function Register-Task037AuthSecrets {
    param([Parameter(Mandatory = $true)][string]$AuthPath)

    Assert-RegularFile -Path $AuthPath
    $raw = [IO.File]::ReadAllText($AuthPath, [Text.Encoding]::UTF8)
    if ([Text.Encoding]::UTF8.GetByteCount($raw) -gt 4194304) {
        throw 'TASK037_CODEX_CREDENTIAL_SOURCE_REJECTED'
    }
    try {
        Add-Task037SecretScalars -Value ($raw | ConvertFrom-Json)
    }
    catch {
        throw 'TASK037_CODEX_CREDENTIAL_SOURCE_REJECTED'
    }
}

function Assert-Task037NoKnownSecretText {
    param([string]$Text)

    if ([string]::IsNullOrEmpty($Text)) { return }
    foreach ($secret in $script:Task037KnownSecrets) {
        if ($Text.IndexOf($secret, [StringComparison]::Ordinal) -ge 0) {
            throw 'TASK037_SECRET_PERSISTENCE_REJECTED'
        }
    }
}

function Assert-Task037NoKnownSecretValue {
    param($Value)

    if ($null -eq $Value) { return }
    Assert-Task037NoKnownSecretText -Text ($Value | ConvertTo-Json -Compress -Depth 16)
}

function Write-JsonEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force:$false | Out-Null
    }
    $json = ($Value | ConvertTo-Json -Depth 16) + [Environment]::NewLine
    Assert-Task037NoKnownSecretText -Text $json
    [System.IO.File]::WriteAllText($Path, $json, [System.Text.UTF8Encoding]::new($false))
}

function Promote-Task037FinalEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$PendingPath,
        [Parameter(Mandatory = $true)][string]$FinalPath
    )

    if (
        -not (Test-Path -LiteralPath $PendingPath -PathType Leaf) -or
        (Test-Path -LiteralPath $FinalPath)
    ) {
        throw 'TASK037_FINAL_EVIDENCE_NOT_FRESH'
    }
    Assert-RegularFile -Path $PendingPath
    try {
        [IO.File]::Move($PendingPath, $FinalPath)
    }
    catch {
        throw 'TASK037_FINAL_EVIDENCE_PROMOTION_REJECTED'
    }
    if (
        (Test-Path -LiteralPath $PendingPath) -or
        -not (Test-Path -LiteralPath $FinalPath -PathType Leaf)
    ) {
        throw 'TASK037_FINAL_EVIDENCE_CLEANUP_REJECTED'
    }
}

function Get-SafeFailureEvidence {
    param([string]$Message)

    if ([string]::IsNullOrEmpty($Message)) {
        return $null
    }
    $fixedCode = switch -Regex ($Message) {
        '^TASK037_[A-Z0-9_]+$' { 'TASK037_CONTROLLED_FAILURE'; break }
        '^TASK019_[A-Z0-9_]+$' { 'TASK037_POSTGRES_HARNESS_FAILURE'; break }
        '^LATTICE_[A-Z0-9_]+$' { 'TASK037_LATTICE_COMPONENT_FAILURE'; break }
        '^HERMES_[A-Z0-9_]+$' { 'TASK037_HERMES_COMPONENT_FAILURE'; break }
        '^GRAPHIFY_[A-Z0-9_]+$' { 'TASK037_GRAPHIFY_COMPONENT_FAILURE'; break }
        '^OPENCLAW_[A-Z0-9_]+$' { 'TASK037_OPENCLAW_COMPONENT_FAILURE'; break }
        default { 'TASK037_UNCLASSIFIED_FAILURE' }
    }
    return [ordered]@{
        code = $fixedCode
    }
}

function Throw-Task037SafeFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Message,
        [string]$FallbackCode = 'TASK037_UNCLASSIFIED_FAILURE'
    )

    $evidence = Get-SafeFailureEvidence -Message $Message
    if ($null -ne $evidence -and -not [string]::IsNullOrWhiteSpace([string]$evidence.code)) {
        throw ([string]$evidence.code)
    }
    throw $FallbackCode
}

function Invoke-Task037GitText {
    param(
        [Parameter(Mandatory = $true)][string]$GitExe,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $output = @(& $GitExe @Arguments 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw $FailureCode
    }
    return ($output -join "`n")
}

function Test-Task037PhaseGateName {
    param([AllowNull()]$Gate)

    if ($null -eq $Gate) { return $true }
    return (
        $Gate -is [string] -and
        $script:Task037PhaseGates -ccontains [string]$Gate
    )
}

function Get-Task037ToolFailureGate {
    param([AllowNull()]$Code)

    if ($Code -isnot [string]) { return 'success_tool_error_unclassified' }
    switch -CaseSensitive ([string]$Code) {
        { @(
            'LATTICED_CONFIGURATION_REJECTED',
            'LATTICED_DATABASE_SECRET_MISSING',
            'LATTICED_DATABASE_CONNECT_REJECTED',
            'LATTICED_LEDGER_CONFIGURATION_REJECTED',
            'LATTICED_WORKSPACE_CONFIGURATION_REJECTED',
            'LATTICED_CODEX_CONFIGURATION_REJECTED',
            'LATTICE_OFFICIAL_CODEX_IDENTITY_REJECTED',
            'LATTICE_SCRIPTED_FIXTURE_REJECTED'
        ) -ccontains $_ } { return 'success_tool_error_configuration' }
        { @(
            'LATTICE_DELIVERY_CONTRACT_REJECTED',
            'LATTICE_DELIVERY_INTENT_REJECTED',
            'LATTICE_DELIVERY_OUTCOME_PERSIST_REJECTED',
            'LATTICE_DELIVERY_RECEIPT_REJECTED',
            'LATTICE_DELIVERY_RECEIPT_MISMATCH',
            'LATTICE_DELIVERY_FAILED',
            'LATTICE_DELIVERY_RECONCILIATION_REQUIRED'
        ) -ccontains $_ } { return 'success_tool_error_delivery' }
        { @(
            'LATTICE_GRAPH_MEMORY_CONFIGURATION_REJECTED',
            'LATTICE_GRAPH_MEMORY_RUN_REJECTED',
            'LATTICE_GRAPH_MEMORY_RECEIPT_REJECTED'
        ) -ccontains $_ } { return 'success_tool_error_graph' }
        { @(
            'LATTICE_HERMES_PRODUCTION_RUNNER_REQUIRED',
            'LATTICE_HERMES_REFLECTION_REJECTED',
            'LATTICE_HERMES_MEMORY_RECEIPT_REJECTED'
        ) -ccontains $_ } { return 'success_tool_error_hermes' }
        'LATTICED_STDIO_REJECTED' { return 'success_tool_error_transport' }
        default { return 'success_tool_error_unclassified' }
    }
}

function Get-Task037HermesFailureClassificationGate {
    param([AllowNull()]$Classification)

    if ($Classification -isnot [string]) { return 'success_tool_error_hermes_unclassified' }
    switch -CaseSensitive ([string]$Classification) {
        'HERMES_RUN_FAILED_HINT_AUTH' { return 'success_tool_error_hermes_hint_auth' }
        'HERMES_RUN_FAILED_HINT_QUOTA' { return 'success_tool_error_hermes_hint_quota' }
        'HERMES_RUN_FAILED_HINT_TRANSPORT' { return 'success_tool_error_hermes_hint_transport' }
        'HERMES_RUN_FAILED_HINT_APP_SERVER_TIMEOUT' { return 'success_tool_error_hermes_hint_app_server_timeout' }
        'HERMES_RUN_FAILED_HINT_APP_SERVER_TURN_STATUS' { return 'success_tool_error_hermes_hint_app_server_turn_status' }
        'HERMES_RUN_FAILED_HINT_SCHEMA' { return 'success_tool_error_hermes_hint_schema' }
        'HERMES_RUN_FAILED_HINT_APP_SERVER' { return 'success_tool_error_hermes_hint_app_server' }
        'HERMES_CODEX_APP_SERVER_EMPTY_OUTPUT' { return 'success_tool_error_hermes_app_server_empty_output' }
        default { return 'success_tool_error_hermes_unclassified' }
    }
}

function Get-Task037HermesFailureStage {
    param([AllowNull()]$Classification)

    if ($Classification -isnot [string]) { return $null }
    switch -CaseSensitive ([string]$Classification) {
        'HERMES_RUN_FAILED_HINT_AUTH' { return 'auth' }
        'HERMES_RUN_FAILED_HINT_QUOTA' { return 'quota' }
        'HERMES_RUN_FAILED_HINT_TRANSPORT' { return 'transport' }
        'HERMES_RUN_FAILED_HINT_APP_SERVER_TIMEOUT' { return 'app_server_timeout' }
        'HERMES_RUN_FAILED_HINT_APP_SERVER_TURN_STATUS' { return 'app_server_turn_status' }
        'HERMES_RUN_FAILED_HINT_SCHEMA' { return 'schema' }
        'HERMES_RUN_FAILED_HINT_APP_SERVER' { return 'app_server' }
        'HERMES_RUN_FAILED' { return 'unknown' }
        default { return $null }
    }
}

function Test-Task037HermesReflectionErrorCode {
    param([AllowNull()]$ErrorCode)

    if ($ErrorCode -isnot [string]) { return $false }
    return @(
        'HERMES_RUN_FAILED',
        'HERMES_RUN_FAILED_HINT_AUTH',
        'HERMES_RUN_FAILED_HINT_QUOTA',
        'HERMES_RUN_FAILED_HINT_TRANSPORT',
        'HERMES_RUN_FAILED_HINT_APP_SERVER_TIMEOUT',
        'HERMES_RUN_FAILED_HINT_APP_SERVER_TURN_STATUS',
        'HERMES_RUN_FAILED_HINT_SCHEMA',
        'HERMES_RUN_FAILED_HINT_APP_SERVER',
        'HERMES_CODEX_APP_SERVER_RUN_FAILED',
        'HERMES_PRODUCTION_CHILD_EXITED',
        'HERMES_PRODUCTION_DEADLINE_EXCEEDED'
    ) -ccontains [string]$ErrorCode
}

function Get-Task037HermesToolFailureGate {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$StderrText)

    $unclassified = 'success_tool_error_hermes_unclassified'
    $diagnosticRejected = 'success_tool_error_hermes_diagnostic_rejected'
    try {
        if ([Text.Encoding]::UTF8.GetByteCount($StderrText) -gt $task037MaxEvidenceFileBytes) {
            return $diagnosticRejected
        }
        $lines = @($StderrText -split '\r?\n')
        $reflectionLines = @($lines | Where-Object {
            $_.IndexOf('"event":"reflection_rejected"', [StringComparison]::Ordinal) -ge 0
        })
        $detailLines = @($lines | Where-Object {
            $_.IndexOf('"event":"run_failure_detail"', [StringComparison]::Ordinal) -ge 0
        })
        if ($reflectionLines.Count -ne 1 -or $detailLines.Count -gt 1) {
            return $diagnosticRejected
        }

        $reflection = ConvertFrom-Task037McpJson -Json ([string]$reflectionLines[0])
        if ($reflection -isnot [pscustomobject]) { return $diagnosticRejected }
        Assert-Task037ExactPropertySet `
            -Value $reflection `
            -Expected @('component', 'error_code', 'event') `
            -FailureCode 'TASK037_HERMES_FAILURE_DIAGNOSTIC_REJECTED'
        if (
            $reflection.component -isnot [string] -or
            $reflection.event -isnot [string] -or
            $reflection.error_code -isnot [string] -or
            [string]$reflection.component -cne 'Hermes' -or
            [string]$reflection.event -cne 'reflection_rejected'
        ) {
            return $diagnosticRejected
        }
        if (-not (Test-Task037HermesReflectionErrorCode -ErrorCode $reflection.error_code)) {
            return $unclassified
        }

        $errorCode = [string]$reflection.error_code
        if ($errorCode -ceq 'HERMES_PRODUCTION_CHILD_EXITED') {
            if ($detailLines.Count -ne 0) { return $diagnosticRejected }
            return 'success_tool_error_hermes_production_child_exited'
        }
        if ($errorCode -ceq 'HERMES_PRODUCTION_DEADLINE_EXCEEDED') {
            if ($detailLines.Count -ne 0) { return $diagnosticRejected }
            return 'success_tool_error_hermes_production_deadline_exceeded'
        }
        if ($detailLines.Count -ne 1) { return $diagnosticRejected }

        $detail = ConvertFrom-Task037McpJson -Json ([string]$detailLines[0])
        if ($detail -isnot [pscustomobject]) { return $diagnosticRejected }
        if (
            $detail.component -isnot [string] -or
            $detail.classification -isnot [string] -or
            $detail.event -isnot [string] -or
            [string]$detail.event -cne 'run_failure_detail' -or
            $detail.source -isnot [string]
        ) {
            return $diagnosticRejected
        }

        $source = [string]$detail.source
        $classification = [string]$detail.classification
        switch -CaseSensitive ($source) {
            'status' {
                if ($null -ne $detail.PSObject.Properties['detail_present']) {
                    Assert-Task037ExactPropertySet `
                        -Value $detail `
                        -Expected @('classification', 'component', 'detail_present', 'event', 'source') `
                        -FailureCode 'TASK037_HERMES_FAILURE_DIAGNOSTIC_REJECTED'
                    if (
                        [string]$detail.component -cne 'Hermes' -or
                        $detail.detail_present -isnot [bool] -or
                        [bool]$detail.detail_present -or
                        $classification -cne 'HERMES_RUN_FAILED'
                    ) {
                        return $diagnosticRejected
                    }
                }
                else {
                    Assert-Task037ExactPropertySet `
                        -Value $detail `
                        -Expected @(
                            'classification', 'component', 'detail_byte_count', 'detail_redacted',
                            'detail_sha256', 'event', 'failure_stage', 'source'
                        ) `
                        -FailureCode 'TASK037_HERMES_FAILURE_DIAGNOSTIC_REJECTED'
                    $expectedStage = Get-Task037HermesFailureStage -Classification $classification
                    if (
                        [string]$detail.component -cne 'Hermes' -or
                        -not (Test-Task037JsonInteger -Value $detail.detail_byte_count -Minimum 0 -Maximum 2097152) -or
                        $detail.detail_redacted -isnot [bool] -or
                        -not [bool]$detail.detail_redacted -or
                        $detail.detail_sha256 -isnot [string] -or
                        [string]$detail.detail_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
                        $detail.failure_stage -isnot [string] -or
                        $null -eq $expectedStage -or
                        [string]$detail.failure_stage -cne [string]$expectedStage
                    ) {
                        return $diagnosticRejected
                    }
                }
            }
            'events' {
                Assert-Task037ExactPropertySet `
                    -Value $detail `
                    -Expected @(
                        'classification', 'component', 'detail_byte_count', 'detail_redacted',
                        'detail_sha256', 'event', 'failure_stage', 'source'
                    ) `
                    -FailureCode 'TASK037_HERMES_FAILURE_DIAGNOSTIC_REJECTED'
                $expectedStage = Get-Task037HermesFailureStage -Classification $classification
                if (
                    [string]$detail.component -cne 'Hermes' -or
                    -not (Test-Task037JsonInteger -Value $detail.detail_byte_count -Minimum 0 -Maximum 2097152) -or
                    $detail.detail_redacted -isnot [bool] -or
                    -not [bool]$detail.detail_redacted -or
                    $detail.detail_sha256 -isnot [string] -or
                    [string]$detail.detail_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
                    $detail.failure_stage -isnot [string] -or
                    $null -eq $expectedStage -or
                    [string]$detail.failure_stage -cne [string]$expectedStage
                ) {
                    return $diagnosticRejected
                }
            }
            'completed_output' {
                Assert-Task037ExactPropertySet `
                    -Value $detail `
                    -Expected @(
                        'classification', 'component', 'detail_byte_count', 'detail_redacted',
                        'detail_sha256', 'event', 'failure_stage', 'source'
                    ) `
                    -FailureCode 'TASK037_HERMES_FAILURE_DIAGNOSTIC_REJECTED'
                $expectedStage = Get-Task037HermesFailureStage -Classification $classification
                if (
                    [string]$detail.component -cne 'Codex app-server' -or
                    -not (Test-Task037JsonInteger -Value $detail.detail_byte_count -Minimum 0 -Maximum 2097152) -or
                    $detail.detail_redacted -isnot [bool] -or
                    -not [bool]$detail.detail_redacted -or
                    $detail.detail_sha256 -isnot [string] -or
                    [string]$detail.detail_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
                    $detail.failure_stage -isnot [string] -or
                    $null -eq $expectedStage -or
                    [string]$detail.failure_stage -cne [string]$expectedStage
                ) {
                    return $diagnosticRejected
                }
            }
            'empty' {
                Assert-Task037ExactPropertySet `
                    -Value $detail `
                    -Expected @('classification', 'component', 'event', 'source') `
                    -FailureCode 'TASK037_HERMES_FAILURE_DIAGNOSTIC_REJECTED'
                if (
                    [string]$detail.component -cne 'Codex app-server' -or
                    $classification -cne 'HERMES_CODEX_APP_SERVER_EMPTY_OUTPUT'
                ) {
                    return $diagnosticRejected
                }
            }
            default { return $diagnosticRejected }
        }

        if ($errorCode -ceq 'HERMES_CODEX_APP_SERVER_RUN_FAILED') {
            if ([string]$detail.component -cne 'Codex app-server') { return $diagnosticRejected }
            return Get-Task037HermesFailureClassificationGate -Classification $classification
        }
        if ($errorCode -ceq 'HERMES_RUN_FAILED') {
            if (
                [string]$detail.component -cne 'Hermes' -or
                $classification -cne 'HERMES_RUN_FAILED'
            ) {
                return $diagnosticRejected
            }
            return $unclassified
        }
        if ($errorCode -cne $classification -or [string]$detail.component -cne 'Hermes') {
            return $diagnosticRejected
        }
        return Get-Task037HermesFailureClassificationGate -Classification $classification
    }
    catch {
        return $diagnosticRejected
    }
}

function Get-Task037HermesComponentFailureGate {
    param(
        [AllowNull()]$Code,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$StderrText
    )

    if ($Code -isnot [string]) { return 'success_tool_error_hermes_unclassified' }
    switch -CaseSensitive ([string]$Code) {
        'LATTICE_HERMES_REFLECTION_REJECTED' {
            return Get-Task037HermesToolFailureGate -StderrText $StderrText
        }
        'LATTICE_HERMES_PRODUCTION_RUNNER_REQUIRED' {
            return 'success_tool_error_hermes_runtime'
        }
        'LATTICE_HERMES_MEMORY_RECEIPT_REJECTED' {
            return 'success_tool_error_hermes_memory_receipt'
        }
        default { return 'success_tool_error_hermes_unclassified' }
    }
}

function Enter-Task037PhaseGate {
    param(
        [Parameter(Mandatory = $true)][string]$Gate,
        [Parameter(Mandatory = $true)][ref]$ActiveGate,
        [Parameter(Mandatory = $true)][ref]$LastCompletedGate
    )

    if (-not (Test-Task037PhaseGateName -Gate $Gate)) {
        throw 'TASK037_PHASE_GATE_REJECTED'
    }
    if ($null -ne $ActiveGate.Value) {
        if (-not (Test-Task037PhaseGateName -Gate $ActiveGate.Value)) {
            throw 'TASK037_PHASE_GATE_REJECTED'
        }
        $LastCompletedGate.Value = [string]$ActiveGate.Value
    }
    $ActiveGate.Value = $Gate
}

function Complete-Task037PhaseGate {
    param(
        [Parameter(Mandatory = $true)][ref]$ActiveGate,
        [Parameter(Mandatory = $true)][ref]$LastCompletedGate
    )

    if (-not (Test-Task037PhaseGateName -Gate $ActiveGate.Value)) {
        throw 'TASK037_PHASE_GATE_REJECTED'
    }
    if ($null -ne $ActiveGate.Value) {
        $LastCompletedGate.Value = [string]$ActiveGate.Value
        $ActiveGate.Value = $null
    }
}

function Invoke-Task037ProcessContractCheck {
    param(
        [Parameter(Mandatory = $true)][string]$Gate,
        [Parameter(Mandatory = $true)][scriptblock]$Check,
        [Parameter(Mandatory = $true)][ref]$ActiveGate,
        [Parameter(Mandatory = $true)][ref]$LastCompletedGate
    )

    $contractGates = @(
        'process_exit_snapshot',
        'wrapper_reap',
        'wrapper_exit_match',
        'openclaw_liveness',
        'child_exit_success',
        'process_handle_release',
        'job_cardinality'
    )
    if ($contractGates -cnotcontains $Gate) {
        throw 'TASK037_PHASE_GATE_REJECTED'
    }
    Enter-Task037PhaseGate `
        -Gate $Gate `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    try {
        $result = @(& $Check)
        if (
            $result.Count -ne 1 -or
            $result[0] -isnot [bool] -or
            -not [bool]$result[0]
        ) {
            throw 'TASK037_PROCESS_CONTRACT_REJECTED'
        }
    }
    catch {
        throw 'TASK037_PROCESS_CONTRACT_REJECTED'
    }
}

function Invoke-Task037ClosedGitText {
    param(
        [Parameter(Mandatory = $true)][string]$GitExe,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$IsolationRoot,
        [Parameter(Mandatory = $true)]
        [ValidateSet(
            'Root',
            'Branch',
            'Head',
            'Status',
            'InitializeSynthetic',
            'AddSynthetic',
            'CommitSynthetic'
        )]
        [string]$Operation,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $fixedConfiguration = @(
        '-c', 'core.fsmonitor=false',
        '-c', 'core.untrackedCache=false',
        '-c', 'core.excludesFile=NUL',
        '-c', 'status.showUntrackedFiles=all'
    )
    $arguments = switch ($Operation) {
        'Root' { @($fixedConfiguration + @('rev-parse', '--show-toplevel')) }
        'Branch' { @($fixedConfiguration + @('rev-parse', '--abbrev-ref', 'HEAD')) }
        'Head' { @($fixedConfiguration + @('rev-parse', 'HEAD')) }
        'Status' {
            @(
                $fixedConfiguration + @(
                    'status', '--porcelain=v1', '--untracked-files=all', '--ignore-submodules=none'
                )
            )
        }
        'InitializeSynthetic' {
            @(
                $fixedConfiguration + @(
                    '-c', 'init.defaultBranch=task037-selftest',
                    'init', '--quiet', '--initial-branch=task037-selftest', '--template=', '.'
                )
            )
        }
        'AddSynthetic' {
            @($fixedConfiguration + @('add', '--', 'baseline.txt'))
        }
        'CommitSynthetic' {
            @(
                $fixedConfiguration + @(
                    '-c', 'user.name=task037-selftest',
                    '-c', 'user.email=task037-selftest@invalid',
                    '-c', 'commit.gpgSign=false',
                    'commit', '--quiet', '--no-gpg-sign', '--no-verify',
                    '-m', 'task037-selftest'
                )
            )
        }
    }
    $canonicalGit = Get-CanonicalPath -Path $GitExe
    $canonicalRepository = Get-CanonicalPath -Path $RepositoryRoot
    $canonicalIsolation = Get-CanonicalPath -Path $IsolationRoot
    foreach ($path in @($canonicalGit, $canonicalRepository, $canonicalIsolation)) {
        Assert-NoReparsePath -Path $path -FailureCode $FailureCode
    }
    Assert-RegularFile -Path $canonicalGit
    foreach ($directory in @(
        $canonicalRepository,
        $canonicalIsolation,
        (Join-Path $canonicalIsolation 'home'),
        (Join-Path $canonicalIsolation 'temp')
    )) {
        $item = Get-Item -LiteralPath $directory -Force -ErrorAction SilentlyContinue
        if (
            $null -eq $item -or
            -not $item.PSIsContainer -or
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
        ) {
            throw $FailureCode
        }
    }

    $windows = Get-CanonicalPath -Path (
        [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
    )
    $system32 = Get-CanonicalPath -Path (Join-Path $windows 'System32')
    $gitDirectory = Get-CanonicalPath -Path (Split-Path -Parent $canonicalGit)
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $canonicalGit
    $startInfo.Arguments = [string]::Join(' ', $arguments)
    $startInfo.WorkingDirectory = $canonicalRepository
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.EnvironmentVariables.Clear()
    foreach ($entry in ([ordered]@{
        SystemRoot = $windows
        WINDIR = $windows
        ComSpec = Join-Path $system32 'cmd.exe'
        PATH = [string]::Join(';', @($gitDirectory, $system32))
        HOME = Join-Path $canonicalIsolation 'home'
        USERPROFILE = Join-Path $canonicalIsolation 'home'
        XDG_CONFIG_HOME = Join-Path $canonicalIsolation 'home'
        TEMP = Join-Path $canonicalIsolation 'temp'
        TMP = Join-Path $canonicalIsolation 'temp'
        GIT_CONFIG_NOSYSTEM = '1'
        GIT_CONFIG_GLOBAL = 'NUL'
        GIT_CONFIG_SYSTEM = 'NUL'
        GIT_TERMINAL_PROMPT = '0'
        GIT_OPTIONAL_LOCKS = '0'
        GIT_NO_REPLACE_OBJECTS = '1'
        NO_COLOR = '1'
    }).GetEnumerator()) {
        $startInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }

    $process = $null
    try {
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw $FailureCode }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(15000)) {
            try { $process.Kill() } catch { }
            try { $process.WaitForExit() } catch { }
            throw $FailureCode
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if (
            [int]$process.ExitCode -ne 0 -or
            -not [string]::IsNullOrEmpty($stderr) -or
            $stdout.Length -gt 65536 -or
            $stderr.Length -gt 65536
        ) {
            throw $FailureCode
        }
        return $stdout.TrimEnd([char[]]"`r`n")
    }
    catch {
        throw $FailureCode
    }
    finally {
        if ($null -ne $process) { $process.Dispose() }
    }
}

function Get-Task037ClosedGitSnapshot {
    param(
        [Parameter(Mandatory = $true)][string]$GitExe,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$AuthorizedTempBase,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $tempBase = Resolve-Task037AuthorizedTempBase `
        -AuthorizedTempBase $AuthorizedTempBase `
        -FailureCode $FailureCode
    $isolationRoot = Join-Path $tempBase (
        'lattice-task037-git-inspection-' + [Guid]::NewGuid().ToString('N')
    )
    try {
        New-Item -ItemType Directory -Path $isolationRoot -Force:$false | Out-Null
        $null = Set-Task037RestrictedDirectoryAcl -Path $isolationRoot
        foreach ($leaf in @('home', 'temp')) {
            New-Item -ItemType Directory -Path (Join-Path $isolationRoot $leaf) -Force:$false | Out-Null
        }
        $root = Invoke-Task037ClosedGitText `
            -GitExe $GitExe `
            -RepositoryRoot $RepositoryRoot `
            -IsolationRoot $isolationRoot `
            -Operation Root `
            -FailureCode $FailureCode
        $branch = Invoke-Task037ClosedGitText `
            -GitExe $GitExe `
            -RepositoryRoot $RepositoryRoot `
            -IsolationRoot $isolationRoot `
            -Operation Branch `
            -FailureCode $FailureCode
        $head = Invoke-Task037ClosedGitText `
            -GitExe $GitExe `
            -RepositoryRoot $RepositoryRoot `
            -IsolationRoot $isolationRoot `
            -Operation Head `
            -FailureCode $FailureCode
        $status = Invoke-Task037ClosedGitText `
            -GitExe $GitExe `
            -RepositoryRoot $RepositoryRoot `
            -IsolationRoot $isolationRoot `
            -Operation Status `
            -FailureCode $FailureCode
        if (
            -not (Test-ExactPath -Actual $root -Expected $RepositoryRoot) -or
            [string]::IsNullOrWhiteSpace($branch) -or
            $branch.IndexOf("`n", [StringComparison]::Ordinal) -ge 0 -or
            $head -cnotmatch '^[0-9a-f]{40}$'
        ) {
            throw $FailureCode
        }
        return [pscustomobject]@{
            RepositoryRoot = Get-CanonicalPath -Path $root
            Branch = $branch
            Head = $head
            Clean = [string]::IsNullOrEmpty($status)
        }
    }
    finally {
        if (Test-Path -LiteralPath $isolationRoot) {
            $removed = Remove-SafeTempRoot `
                -Path $isolationRoot `
                -AuthorizedTempBase $tempBase
            if (-not $removed) { throw $FailureCode }
        }
    }
}

function Get-Task037RepositoryDirectoryShape {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [string]$FailureCode = 'TASK037_REPOSITORY_DIRECTORY_SHAPE_REJECTED',
        [int]$MaxDirectoryCount = 8192,
        [int64]$MaxSerializedBytes = 4194304
    )

    try {
        $root = Get-CanonicalPath -Path $RepositoryRoot
        $rootItem = Get-Item -LiteralPath $root -Force -ErrorAction Stop
        if (
            -not $rootItem.PSIsContainer -or
            ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            $MaxDirectoryCount -le 0 -or
            $MaxSerializedBytes -le 0
        ) {
            throw $FailureCode
        }
        $rootPrefix = $root + [IO.Path]::DirectorySeparatorChar
        $queue = [Collections.Generic.Queue[string]]::new()
        $relativePaths = [Collections.Generic.List[string]]::new()
        $queue.Enqueue($root)
        while ($queue.Count -gt 0) {
            $current = $queue.Dequeue()
            $isRoot = Test-ExactPath -Actual $current -Expected $root
            foreach ($child in @(Get-ChildItem -LiteralPath $current -Directory -Force -ErrorAction Stop)) {
                if ($isRoot -and [string]$child.Name -in @('.git', 'target')) {
                    continue
                }
                if ($child.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                    throw $FailureCode
                }
                $canonicalChild = Get-CanonicalPath -Path $child.FullName
                if (-not $canonicalChild.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                    throw $FailureCode
                }
                $relative = $canonicalChild.Substring($rootPrefix.Length) -replace '\\', '/'
                if ([string]::IsNullOrWhiteSpace($relative)) {
                    throw $FailureCode
                }
                $relativePaths.Add($relative)
                if ($relativePaths.Count -gt $MaxDirectoryCount) {
                    throw $FailureCode
                }
                $queue.Enqueue($canonicalChild)
            }
        }
        [string[]]$ordered = @($relativePaths.ToArray())
        [Array]::Sort($ordered, [StringComparer]::Ordinal)
        $serialized = [Text.StringBuilder]::new()
        [int64]$serializedBytes = 0
        foreach ($relative in $ordered) {
            $relativeBytes = [Text.Encoding]::UTF8.GetByteCount($relative)
            $record = (
                $relativeBytes.ToString([Globalization.CultureInfo]::InvariantCulture) +
                ':' + $relative + "`n"
            )
            $serializedBytes += [Text.Encoding]::UTF8.GetByteCount($record)
            if ($serializedBytes -gt $MaxSerializedBytes) {
                throw $FailureCode
            }
            $null = $serialized.Append($record)
        }
        return [pscustomobject]@{
            RepositoryRoot = $root
            DirectoryCount = [int]$ordered.Count
            DirectoryShapeSha256 = Get-StringSha256 $serialized.ToString()
        }
    }
    catch {
        throw $FailureCode
    }
}

function Assert-Task037RepositoryDirectoryShapeUnchanged {
    param(
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After,
        [string]$FailureCode = 'TASK037_REPOSITORY_DIRECTORY_SHAPE_REJECTED'
    )

    if (
        [string]$Before.DirectoryShapeSha256 -notmatch '^[0-9a-f]{64}$' -or
        [string]$After.DirectoryShapeSha256 -notmatch '^[0-9a-f]{64}$' -or
        -not (Test-ExactPath -Actual ([string]$Before.RepositoryRoot) -Expected ([string]$After.RepositoryRoot)) -or
        [int]$Before.DirectoryCount -ne [int]$After.DirectoryCount -or
        [string]$Before.DirectoryShapeSha256 -cne [string]$After.DirectoryShapeSha256
    ) {
        throw $FailureCode
    }
}

function Assert-BoundedTask037EvidenceFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Code,
        [int64]$MaxBytes = $task037MaxEvidenceFileBytes
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return
    }
    $length = (Get-Item -LiteralPath $Path).Length
    if ($length -gt $MaxBytes) {
        throw $Code
    }
}

function Get-Task037RawArtifactReceipt {
    param(
        [Parameter(Mandatory = $true)][string]$SourcePath,
        [Parameter(Mandatory = $true)][string]$ArtifactId
    )

    if ($ArtifactId -notmatch '^[a-z0-9][a-z0-9._-]{0,127}$') {
        throw 'TASK037_RAW_ARTIFACT_ID_REJECTED'
    }
    if (-not (Test-Path -LiteralPath $SourcePath -PathType Leaf)) {
        return [ordered]@{
            artifact_id = $ArtifactId
            exists = $false
            content_retained = $false
        }
    }
    Assert-RegularFile -Path $SourcePath
    $item = Get-Item -LiteralPath $SourcePath
    if ($item.Length -gt $task037MaxEvidenceFileBytes) {
        throw 'TASK037_RAW_ARTIFACT_OVERSIZE_REJECTED'
    }
    return [ordered]@{
        artifact_id = $ArtifactId
        exists = $true
        sha256 = Get-FileSha256 $SourcePath
        byte_count = [int64]$item.Length
        content_retained = $false
    }
}

function Read-JsonEvidence {
    param([Parameter(Mandatory = $true)][string]$Path)

    return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Assert-Task037ExactPropertySet {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [Parameter(Mandatory = $true)][string[]]$Expected,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if ($null -eq $Value -or $null -eq $Value.PSObject) {
        throw $FailureCode
    }
    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $wanted = @($Expected | Sort-Object)
    if (
        $actual.Count -ne $wanted.Count -or
        [string]::Join("`n", $actual) -cne [string]::Join("`n", $wanted)
    ) {
        throw $FailureCode
    }
}

function Get-U32BigEndian {
    param([Parameter(Mandatory = $true)][uint32]$Value)

    return [byte[]]@(
        [byte](($Value -shr 24) -band 0xff),
        [byte](($Value -shr 16) -band 0xff),
        [byte](($Value -shr 8) -band 0xff),
        [byte]($Value -band 0xff)
    )
}

function Add-Bytes {
    param(
        [Collections.Generic.List[byte]]$Target,
        [Parameter(Mandatory = $true)][byte[]]$Bytes
    )

    foreach ($byte in $Bytes) {
        $Target.Add($byte)
    }
}

function Add-AttestationField {
    param(
        [Collections.Generic.List[byte]]$Target,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][byte[]]$Value
    )

    $nameBytes = [Text.Encoding]::UTF8.GetBytes($Name)
    Add-Bytes -Target $Target -Bytes (Get-U32BigEndian ([uint32]$nameBytes.Length))
    Add-Bytes -Target $Target -Bytes $nameBytes
    Add-Bytes -Target $Target -Bytes (Get-U32BigEndian ([uint32]$Value.Length))
    Add-Bytes -Target $Target -Bytes $Value
}

function Get-AttestationTag {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Key,
        [Parameter(Mandatory = $true)][string]$LaunchRecordId,
        [Parameter(Mandatory = $true)][uint32]$ProcessId,
        [Parameter(Mandatory = $true)][byte[]]$ProcessNonce,
        [Parameter(Mandatory = $true)][string]$ProfileSha256
    )

    $payload = [Collections.Generic.List[byte]]::new()
    Add-Bytes -Target $payload -Bytes ([Text.Encoding]::UTF8.GetBytes('lattice-openclaw-launch-attestation-v1' + [char]0))
    Add-AttestationField -Target $payload -Name 'launch_record_id' -Value ([Text.Encoding]::UTF8.GetBytes($LaunchRecordId))
    Add-AttestationField -Target $payload -Name 'process_id' -Value (Get-U32BigEndian $ProcessId)
    Add-AttestationField -Target $payload -Name 'process_start_nonce' -Value $ProcessNonce
    Add-AttestationField -Target $payload -Name 'package_name' -Value ([Text.Encoding]::UTF8.GetBytes('openclaw'))
    Add-AttestationField -Target $payload -Name 'package_version' -Value ([Text.Encoding]::UTF8.GetBytes($openClawVersion))
    Add-AttestationField -Target $payload -Name 'source_commit' -Value ([Text.Encoding]::UTF8.GetBytes('0790d9f593ad30c940ed93b5872a8cf6d6f3cf8c'))
    Add-AttestationField -Target $payload -Name 'package_license' -Value ([Text.Encoding]::UTF8.GetBytes('MIT'))
    Add-AttestationField -Target $payload -Name 'package_integrity' -Value ([Text.Encoding]::UTF8.GetBytes('sha512-ycF3yPcbjN6bUPeaUx6Mh6vze1hQWoD3CT/wWcmD7a8xaHHHRUaAlaq+lFxMHf1ssEgODVAwjlzYqp2twkYZ7g=='))
    Add-AttestationField -Target $payload -Name 'entrypoint' -Value ([Text.Encoding]::UTF8.GetBytes('openclaw.mjs'))
    Add-AttestationField -Target $payload -Name 'package_tarball_digest' -Value ([Text.Encoding]::UTF8.GetBytes($openClawPackageTarballSha256))
    Add-AttestationField -Target $payload -Name 'entrypoint_digest' -Value ([Text.Encoding]::UTF8.GetBytes($openClawEntrypointSha256))
    Add-AttestationField -Target $payload -Name 'isolated_profile_digest' -Value ([Text.Encoding]::UTF8.GetBytes($ProfileSha256))
    $hmac = [Security.Cryptography.HMACSHA256]::new($Key)
    try {
        return ConvertTo-LowerHex ($hmac.ComputeHash($payload.ToArray()))
    }
    finally {
        $hmac.Dispose()
    }
}

function Get-UnreservedLoopbackPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    try {
        $listener.Start()
        return ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        $listener.Stop()
    }
}

function Test-LoopbackPort {
    param([Parameter(Mandatory = $true)][int]$Port)

    $client = [Net.Sockets.TcpClient]::new()
    try {
        $async = $client.BeginConnect('127.0.0.1', $Port, $null, $null)
        if (-not $async.AsyncWaitHandle.WaitOne(500)) {
            return $false
        }
        $client.EndConnect($async)
        return $true
    }
    catch {
        return $false
    }
    finally {
        $client.Dispose()
    }
}

function Initialize-Task037JobObjectInterop {
    if ($null -ne ('LatticeTask037JobObjectInterop' -as [type])) {
        return
    }
    try {
        Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;

public static class LatticeTask037JobObjectInterop
{
    private const UInt32 JobObjectExtendedLimitInformation = 9;
    private const UInt32 JobObjectBasicProcessIdList = 3;
    private const UInt32 JobObjectLimitKillOnJobClose = 0x00002000;
    private const UInt32 JobObjectAssignProcess = 0x0001;
    private const UInt32 JobObjectSetAttributes = 0x0002;
    private const UInt32 JobObjectQuery = 0x0004;
    private const UInt32 JobObjectTerminate = 0x0008;
    private const UInt32 WriteDac = 0x00040000;
    private const UInt32 WriteOwner = 0x00080000;
    private const UInt32 SecurityDescriptorRevision = 1;
    private const Int32 ErrorAccessDenied = 5;
    private const Int32 ErrorAlreadyExists = 183;

    [StructLayout(LayoutKind.Sequential)]
    private struct SecurityAttributes
    {
        public Int32 Length;
        public IntPtr SecurityDescriptor;
        public Int32 InheritHandle;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct IoCounters
    {
        public UInt64 ReadOperationCount;
        public UInt64 WriteOperationCount;
        public UInt64 OtherOperationCount;
        public UInt64 ReadTransferCount;
        public UInt64 WriteTransferCount;
        public UInt64 OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BasicLimitInformation
    {
        public Int64 PerProcessUserTimeLimit;
        public Int64 PerJobUserTimeLimit;
        public UInt32 LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public UInt32 ActiveProcessLimit;
        public UIntPtr Affinity;
        public UInt32 PriorityClass;
        public UInt32 SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ExtendedLimitInformation
    {
        public BasicLimitInformation BasicLimitInformation;
        public IoCounters IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BasicAccountingInformation
    {
        public Int64 TotalUserTime;
        public Int64 TotalKernelTime;
        public Int64 ThisPeriodTotalUserTime;
        public Int64 ThisPeriodTotalKernelTime;
        public UInt32 TotalPageFaultCount;
        public UInt32 TotalProcesses;
        public UInt32 ActiveProcesses;
        public UInt32 TotalTerminatedProcesses;
    }

    [DllImport("kernel32.dll", EntryPoint = "CreateJobObjectW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateJobObjectWithoutSecurity(IntPtr jobAttributes, string name);

    [DllImport("kernel32.dll", EntryPoint = "CreateJobObjectW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateJobObjectWithSecurity(ref SecurityAttributes jobAttributes, string name);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr OpenJobObject(UInt32 desiredAccess, bool inheritHandle, string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool SetInformationJobObject(
        IntPtr job,
        UInt32 informationClass,
        IntPtr information,
        UInt32 informationLength);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool QueryInformationJobObject(
        IntPtr job,
        UInt32 informationClass,
        out BasicAccountingInformation information,
        UInt32 informationLength,
        IntPtr returnLength);

    [DllImport("kernel32.dll", EntryPoint = "QueryInformationJobObject", SetLastError = true)]
    private static extern bool QueryInformationJobObjectBuffer(
        IntPtr job,
        UInt32 informationClass,
        IntPtr information,
        UInt32 informationLength,
        IntPtr returnLength);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool TerminateJobObject(IntPtr job, UInt32 exitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool result);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool ConvertStringSecurityDescriptorToSecurityDescriptor(
        string stringSecurityDescriptor,
        UInt32 stringSDRevision,
        out IntPtr securityDescriptor,
        out UInt32 securityDescriptorSize);

    [DllImport("kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr memory);

    public static IntPtr CreateKillOnClose()
    {
        IntPtr job = CreateJobObjectWithoutSecurity(IntPtr.Zero, null);
        int createError = Marshal.GetLastWin32Error();
        if (job == IntPtr.Zero)
        {
            throw new Win32Exception(createError);
        }
        var information = new ExtendedLimitInformation();
        information.BasicLimitInformation.LimitFlags = JobObjectLimitKillOnJobClose;
        int size = Marshal.SizeOf(typeof(ExtendedLimitInformation));
        IntPtr buffer = Marshal.AllocHGlobal(size);
        try
        {
            Marshal.StructureToPtr(information, buffer, false);
            if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, buffer, (UInt32)size))
            {
                int error = Marshal.GetLastWin32Error();
                CloseHandle(job);
                throw new Win32Exception(error);
            }
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
        return job;
    }

    public static IntPtr CreateQueryOnlyAttestationNamed(string name)
    {
        if (String.IsNullOrEmpty(name))
        {
            throw new ArgumentException("job name required", "name");
        }
        string sid;
        using (WindowsIdentity identity = WindowsIdentity.GetCurrent())
        {
            if (identity.User == null)
            {
                throw new InvalidOperationException("current process has no user SID");
            }
            sid = identity.User.Value;
        }
        IntPtr securityDescriptor;
        UInt32 securityDescriptorSize;
        string sddl = "D:P(A;;0x00000004;;;" + sid + ")(A;;0x00000004;;;OW)";
        if (!ConvertStringSecurityDescriptorToSecurityDescriptor(
                sddl,
                SecurityDescriptorRevision,
                out securityDescriptor,
                out securityDescriptorSize))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        IntPtr job;
        int createError;
        try
        {
            var attributes = new SecurityAttributes();
            attributes.Length = Marshal.SizeOf(typeof(SecurityAttributes));
            attributes.SecurityDescriptor = securityDescriptor;
            attributes.InheritHandle = 0;
            job = CreateJobObjectWithSecurity(ref attributes, name);
            createError = Marshal.GetLastWin32Error();
        }
        finally
        {
            LocalFree(securityDescriptor);
        }
        if (job == IntPtr.Zero)
        {
            throw new Win32Exception(createError);
        }
        if (createError == ErrorAlreadyExists)
        {
            CloseHandle(job);
            throw new InvalidOperationException("named job already exists");
        }
        try
        {
            VerifyEffectOpenDenied(name);
            IntPtr query = OpenForQuery(name);
            Close(query);
        }
        catch
        {
            CloseHandle(job);
            throw;
        }
        return job;
    }

    public static void VerifyEffectOpenDenied(string name)
    {
        if (String.IsNullOrEmpty(name))
        {
            throw new ArgumentException("job name required", "name");
        }
        UInt32[] deniedAccess = new UInt32[] {
            JobObjectAssignProcess,
            JobObjectSetAttributes,
            JobObjectTerminate,
            WriteDac,
            WriteOwner
        };
        foreach (UInt32 access in deniedAccess)
        {
            IntPtr job = OpenJobObject(access, false, name);
            if (job != IntPtr.Zero)
            {
                CloseHandle(job);
                throw new InvalidOperationException("named attestation job permits effect access");
            }
            int error = Marshal.GetLastWin32Error();
            if (error != ErrorAccessDenied)
            {
                throw new Win32Exception(error);
            }
        }
    }

    public static IntPtr OpenForQuery(string name)
    {
        if (String.IsNullOrEmpty(name))
        {
            throw new ArgumentException("job name required", "name");
        }
        IntPtr job = OpenJobObject(JobObjectQuery, false, name);
        if (job == IntPtr.Zero)
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        return job;
    }

    public static void Assign(IntPtr job, IntPtr process)
    {
        if (job == IntPtr.Zero || process == IntPtr.Zero || !AssignProcessToJobObject(job, process))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }

    public static UInt32 ActiveProcessCount(IntPtr job)
    {
        BasicAccountingInformation information;
        UInt32 size = (UInt32)Marshal.SizeOf(typeof(BasicAccountingInformation));
        if (job == IntPtr.Zero ||
            !QueryInformationJobObject(job, 1, out information, size, IntPtr.Zero))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        return information.ActiveProcesses;
    }

    public static UInt64 ProcessCounts(IntPtr job)
    {
        BasicAccountingInformation information;
        UInt32 size = (UInt32)Marshal.SizeOf(typeof(BasicAccountingInformation));
        if (job == IntPtr.Zero ||
            !QueryInformationJobObject(job, 1, out information, size, IntPtr.Zero))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        return ((UInt64)information.TotalProcesses << 32) | (UInt64)information.ActiveProcesses;
    }

    public static UInt64[] ProcessIds(IntPtr job, UInt32 capacity)
    {
        if (job == IntPtr.Zero || capacity == 0 || capacity > 64)
        {
            throw new ArgumentOutOfRangeException("capacity");
        }
        int size = checked(8 + checked((int)capacity * IntPtr.Size));
        IntPtr buffer = Marshal.AllocHGlobal(size);
        try
        {
            if (!QueryInformationJobObjectBuffer(
                    job,
                    JobObjectBasicProcessIdList,
                    buffer,
                    (UInt32)size,
                    IntPtr.Zero))
            {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
            UInt32 assigned = unchecked((UInt32)Marshal.ReadInt32(buffer, 0));
            UInt32 listed = unchecked((UInt32)Marshal.ReadInt32(buffer, 4));
            if (assigned != listed || listed > capacity)
            {
                throw new InvalidOperationException("job process list is incomplete");
            }
            var result = new UInt64[listed];
            for (UInt32 index = 0; index < listed; index++)
            {
                int offset = checked(8 + checked((int)index * IntPtr.Size));
                UInt64 value = IntPtr.Size == 8
                    ? unchecked((UInt64)Marshal.ReadInt64(buffer, offset))
                    : unchecked((UInt32)Marshal.ReadInt32(buffer, offset));
                if (value == 0 || value > UInt32.MaxValue)
                {
                    throw new InvalidOperationException("job process id is invalid");
                }
                result[index] = value;
            }
            return result;
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
    }

    public static void Terminate(IntPtr job)
    {
        if (job == IntPtr.Zero || !TerminateJobObject(job, 1))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }

    public static bool IsMember(IntPtr job, IntPtr process)
    {
        bool result;
        if (job == IntPtr.Zero || process == IntPtr.Zero || !IsProcessInJob(process, job, out result))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        return result;
    }

    public static void Close(IntPtr job)
    {
        if (job != IntPtr.Zero && !CloseHandle(job))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }
}
'@
    }
    catch {
        throw 'TASK037_JOB_OBJECT_REJECTED'
    }
}

function New-Task037KillOnCloseJob {
    Initialize-Task037JobObjectInterop
    try {
        return [LatticeTask037JobObjectInterop]::CreateKillOnClose()
    }
    catch {
        throw 'TASK037_JOB_OBJECT_REJECTED'
    }
}

function New-Task037QueryOnlyAttestationJob {
    param([Parameter(Mandatory = $true)][string]$Name)

    Initialize-Task037JobObjectInterop
    try {
        return [LatticeTask037JobObjectInterop]::CreateQueryOnlyAttestationNamed($Name)
    }
    catch {
        throw 'TASK037_JOB_OBJECT_REJECTED'
    }
}

function Add-Task037ProcessToJob {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process
    )

    try {
        [LatticeTask037JobObjectInterop]::Assign($Job, $Process.Handle)
    }
    catch {
        throw 'TASK037_JOB_ASSIGN_REJECTED'
    }
}

function Get-Task037JobActiveProcessCount {
    param([Parameter(Mandatory = $true)][IntPtr]$Job)

    try {
        return [uint32][LatticeTask037JobObjectInterop]::ActiveProcessCount($Job)
    }
    catch {
        throw 'TASK037_JOB_ACCOUNTING_REJECTED'
    }
}

function Test-Task037JsonInteger {
    param(
        [AllowNull()]$Value,
        [Parameter(Mandatory = $true)][int64]$Minimum,
        [Parameter(Mandatory = $true)][int64]$Maximum
    )

    if ($Value -isnot [int32]) { return $false }
    $integer = [int32]$Value
    return $integer -ge $Minimum -and $integer -le $Maximum
}

function Test-Task037JsonArray {
    param([AllowNull()]$Value)

    return $Value -is [object[]]
}

function Get-Task037JobProcessCountSnapshot {
    param([Parameter(Mandatory = $true)][IntPtr]$Job)

    try {
        $packed = [uint64][LatticeTask037JobObjectInterop]::ProcessCounts($Job)
        return [pscustomobject]@{
            TotalProcesses = [uint32]($packed -shr 32)
            ActiveProcesses = [uint32]($packed -band [uint64]4294967295)
        }
    }
    catch {
        throw 'TASK037_JOB_ACCOUNTING_REJECTED'
    }
}

function Test-Task037ValidProcessIdSet {
    param([AllowEmptyCollection()][uint32[]]$ProcessIds = @())

    $seen = [Collections.Generic.HashSet[uint32]]::new()
    foreach ($processId in $ProcessIds) {
        if ($processId -eq 0 -or -not $seen.Add([uint32]$processId)) {
            return $false
        }
    }
    return $true
}

function Test-Task037ProcessIdSetEqual {
    param(
        [AllowEmptyCollection()][uint32[]]$Left = @(),
        [AllowEmptyCollection()][uint32[]]$Right = @()
    )

    if (
        $Left.Count -ne $Right.Count -or
        -not (Test-Task037ValidProcessIdSet -ProcessIds $Left) -or
        -not (Test-Task037ValidProcessIdSet -ProcessIds $Right)
    ) {
        return $false
    }
    $rightSet = [Collections.Generic.HashSet[uint32]]::new()
    foreach ($processId in $Right) { $null = $rightSet.Add([uint32]$processId) }
    foreach ($processId in $Left) {
        if (-not $rightSet.Contains([uint32]$processId)) { return $false }
    }
    return $true
}

function Test-Task037ProcessIdSetSubset {
    param(
        [AllowEmptyCollection()][uint32[]]$Candidate = @(),
        [AllowEmptyCollection()][uint32[]]$Allowed = @()
    )

    if (
        -not (Test-Task037ValidProcessIdSet -ProcessIds $Candidate) -or
        -not (Test-Task037ValidProcessIdSet -ProcessIds $Allowed)
    ) {
        return $false
    }
    $allowedSet = [Collections.Generic.HashSet[uint32]]::new()
    foreach ($processId in $Allowed) { $null = $allowedSet.Add([uint32]$processId) }
    foreach ($processId in $Candidate) {
        if (-not $allowedSet.Contains([uint32]$processId)) { return $false }
    }
    return $true
}

function Get-Task037CoherentJobProcessSnapshot {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [ValidateRange(1, 5000)][int]$TimeoutMilliseconds = 250
    )

    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    while ($stopwatch.ElapsedMilliseconds -lt $TimeoutMilliseconds) {
        try {
            $before = Get-Task037JobProcessCountSnapshot -Job $Job
            $rawProcessIds = @([LatticeTask037JobObjectInterop]::ProcessIds($Job, 64))
            $after = Get-Task037JobProcessCountSnapshot -Job $Job
            $processIds = [uint32[]]@($rawProcessIds | ForEach-Object { [uint32]$_ } | Sort-Object)
            if (
                [uint32]$before.TotalProcesses -eq [uint32]$after.TotalProcesses -and
                [uint32]$before.ActiveProcesses -eq [uint32]$after.ActiveProcesses -and
                $processIds.Count -eq [uint32]$before.ActiveProcesses -and
                (Test-Task037ValidProcessIdSet -ProcessIds $processIds)
            ) {
                return [pscustomobject]@{
                    TotalProcesses = [uint32]$before.TotalProcesses
                    ActiveProcesses = [uint32]$before.ActiveProcesses
                    ProcessIds = $processIds
                }
            }
        }
        catch { }
        if ($stopwatch.ElapsedMilliseconds -lt $TimeoutMilliseconds) {
            Start-Sleep -Milliseconds 5
        }
    }
    throw 'TASK037_JOB_ACCOUNTING_REJECTED'
}

function Test-Task037ProcessIdentityCohortLiveInJob {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [AllowEmptyCollection()][object[]]$Identities = @()
    )

    try {
        foreach ($identity in $Identities) {
            if (
                $null -eq $identity -or
                $identity.PSObject.Properties.Name -cnotcontains 'Process' -or
                $identity.PSObject.Properties.Name -cnotcontains 'ProcessId' -or
                $identity.PSObject.Properties.Name -cnotcontains 'ProcessStartTicks' -or
                $identity.ProcessId -isnot [uint32] -or
                $identity.ProcessStartTicks -isnot [int64] -or
                [uint32]$identity.ProcessId -eq 0 -or
                [int64]$identity.ProcessStartTicks -le 0
            ) {
                return $false
            }
            $process = $identity.Process
            if (
                $null -eq $process -or
                $process -isnot [Diagnostics.Process] -or
                [uint32]$process.Id -ne [uint32]$identity.ProcessId -or
                [int64]$process.StartTime.ToUniversalTime().Ticks -ne
                    [int64]$identity.ProcessStartTicks -or
                $process.HasExited -or
                -not [LatticeTask037JobObjectInterop]::IsMember($Job, $process.Handle)
            ) {
                return $false
            }
        }
        return $true
    }
    catch {
        return $false
    }
}

function Get-Task037OpenClawJobBaseline {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$OpenClawProcess,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$OpenClawControllerProcess,
        [ValidateRange(1, 30000)][int]$TimeoutMilliseconds = 5000
    )

    try {
        if ($OpenClawProcess.Id -eq $OpenClawControllerProcess.Id) {
            throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
        }
        $requiredProcessIds = [uint32[]]@(
            [uint32]$OpenClawProcess.Id,
            [uint32]$OpenClawControllerProcess.Id
        )
        $primaryIdentities = @(
            [pscustomobject]@{
                Process = $OpenClawProcess
                ProcessId = [uint32]$OpenClawProcess.Id
                ProcessStartTicks = [int64]$OpenClawProcess.StartTime.ToUniversalTime().Ticks
            },
            [pscustomobject]@{
                Process = $OpenClawControllerProcess
                ProcessId = [uint32]$OpenClawControllerProcess.Id
                ProcessStartTicks = [int64]$OpenClawControllerProcess.StartTime.ToUniversalTime().Ticks
            }
        )
        $stopwatch = [Diagnostics.Stopwatch]::StartNew()
        $previous = $null
        $stableSnapshot = $null
        while ($stopwatch.ElapsedMilliseconds -lt $TimeoutMilliseconds) {
            if (-not (Test-Task037ProcessIdentityCohortLiveInJob `
                -Job $Job `
                -Identities $primaryIdentities)) {
                throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
            }
            $remainingMilliseconds = [Math]::Max(
                1,
                [Math]::Min(250, $TimeoutMilliseconds - [int]$stopwatch.ElapsedMilliseconds)
            )
            $snapshot = Get-Task037CoherentJobProcessSnapshot `
                -Job $Job `
                -TimeoutMilliseconds $remainingMilliseconds
            if (
                [uint32]$snapshot.ActiveProcesses -lt 2 -or
                [uint32]$snapshot.ActiveProcesses -gt 16 -or
                -not (Test-Task037ProcessIdSetSubset `
                    -Candidate $requiredProcessIds `
                    -Allowed $snapshot.ProcessIds)
            ) {
                throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
            }
            if (
                $null -ne $previous -and
                [uint32]$snapshot.TotalProcesses -eq [uint32]$previous.TotalProcesses -and
                (Test-Task037ProcessIdSetEqual `
                    -Left $snapshot.ProcessIds `
                    -Right $previous.ProcessIds)
            ) {
                $stableSnapshot = $snapshot
                break
            }
            $previous = $snapshot
            Start-Sleep -Milliseconds 100
        }
        if ($null -eq $stableSnapshot) {
            throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
        }

        $extraProcesses = [Collections.Generic.List[Diagnostics.Process]]::new()
        $identities = [Collections.Generic.List[object]]::new()
        try {
            foreach ($processId in $stableSnapshot.ProcessIds) {
                $process = $null
                if ([uint32]$processId -eq [uint32]$OpenClawProcess.Id) {
                    $process = $OpenClawProcess
                }
                elseif ([uint32]$processId -eq [uint32]$OpenClawControllerProcess.Id) {
                    $process = $OpenClawControllerProcess
                }
                else {
                    $process = [Diagnostics.Process]::GetProcessById([int]$processId)
                    $null = $process.Handle
                    $extraProcesses.Add($process)
                }
                $identities.Add([pscustomobject]@{
                    Process = $process
                    ProcessId = [uint32]$process.Id
                    ProcessStartTicks = [int64]$process.StartTime.ToUniversalTime().Ticks
                })
            }
            if (-not (Test-Task037ProcessIdentityCohortLiveInJob `
                -Job $Job `
                -Identities $identities.ToArray())) {
                throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
            }
            if ($stopwatch.ElapsedMilliseconds -ge $TimeoutMilliseconds) {
                throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
            }
            $remainingMilliseconds = [Math]::Max(
                1,
                [Math]::Min(250, $TimeoutMilliseconds - [int]$stopwatch.ElapsedMilliseconds)
            )
            $confirmed = Get-Task037CoherentJobProcessSnapshot `
                -Job $Job `
                -TimeoutMilliseconds $remainingMilliseconds
            if (
                [uint32]$confirmed.TotalProcesses -ne [uint32]$stableSnapshot.TotalProcesses -or
                -not (Test-Task037ProcessIdSetEqual `
                    -Left $confirmed.ProcessIds `
                    -Right $stableSnapshot.ProcessIds) -or
                -not (Test-Task037ProcessIdentityCohortLiveInJob `
                    -Job $Job `
                    -Identities $identities.ToArray())
            ) {
                throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
            }
            return [pscustomobject]@{
                TotalProcesses = [uint32]$confirmed.TotalProcesses
                ActiveProcesses = [uint32]$confirmed.ActiveProcesses
                ProcessIds = [uint32[]]@($confirmed.ProcessIds)
                ProcessIdentities = [object[]]$identities.ToArray()
                ExtraProcesses = [Diagnostics.Process[]]$extraProcesses.ToArray()
            }
        }
        catch {
            foreach ($extraProcess in $extraProcesses) {
                try { $extraProcess.Dispose() } catch { }
            }
            throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
        }
    }
    catch {
        throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
    }
    throw 'TASK037_OPENCLAW_JOB_BASELINE_REJECTED'
}

function Wait-Task037JobProcessBaseline {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [Parameter(Mandatory = $true)]$InitialSnapshot,
        [AllowEmptyCollection()][uint32[]]$BaselineProcessIds = @(),
        [AllowEmptyCollection()][object[]]$RequiredProcessIdentities = @(),
        [ValidateRange(1, 30000)][int]$TimeoutMilliseconds = 5000
    )

    try {
        $requiredIdentityProcessIds = [uint32[]]@(
            $RequiredProcessIdentities | ForEach-Object { [uint32]$_.ProcessId }
        )
        if (
            $null -eq $InitialSnapshot -or
            $InitialSnapshot.PSObject.Properties.Name -cnotcontains 'TotalProcesses' -or
            $InitialSnapshot.PSObject.Properties.Name -cnotcontains 'ActiveProcesses' -or
            $InitialSnapshot.PSObject.Properties.Name -cnotcontains 'ProcessIds' -or
            -not (Test-Task037ValidProcessIdSet -ProcessIds $BaselineProcessIds) -or
            -not (Test-Task037ProcessIdSetEqual `
                -Left $requiredIdentityProcessIds `
                -Right $BaselineProcessIds) -or
            -not (Test-Task037ValidProcessIdSet -ProcessIds $InitialSnapshot.ProcessIds) -or
            [uint32]$InitialSnapshot.ActiveProcesses -ne @($InitialSnapshot.ProcessIds).Count -or
            -not (Test-Task037ProcessIdSetSubset `
                -Candidate $BaselineProcessIds `
                -Allowed $InitialSnapshot.ProcessIds)
        ) {
            return $false
        }
        $expectedTotalProcesses = [uint32]$InitialSnapshot.TotalProcesses
        $previousProcessIds = [uint32[]]@($InitialSnapshot.ProcessIds)
        $stopwatch = [Diagnostics.Stopwatch]::StartNew()
        while ($stopwatch.ElapsedMilliseconds -lt $TimeoutMilliseconds) {
            if (-not (Test-Task037ProcessIdentityCohortLiveInJob `
                -Job $Job `
                -Identities $RequiredProcessIdentities)) {
                return $false
            }
            $remainingMilliseconds = [Math]::Max(
                1,
                [Math]::Min(250, $TimeoutMilliseconds - [int]$stopwatch.ElapsedMilliseconds)
            )
            $snapshot = Get-Task037CoherentJobProcessSnapshot `
                -Job $Job `
                -TimeoutMilliseconds $remainingMilliseconds
            if (
                [uint32]$snapshot.TotalProcesses -ne $expectedTotalProcesses -or
                -not (Test-Task037ProcessIdSetSubset `
                    -Candidate $BaselineProcessIds `
                    -Allowed $snapshot.ProcessIds) -or
                -not (Test-Task037ProcessIdSetSubset `
                    -Candidate $snapshot.ProcessIds `
                    -Allowed $previousProcessIds)
            ) {
                return $false
            }
            if (Test-Task037ProcessIdSetEqual `
                -Left $snapshot.ProcessIds `
                -Right $BaselineProcessIds) {
                Start-Sleep -Milliseconds 10
                if ($stopwatch.ElapsedMilliseconds -ge $TimeoutMilliseconds) { return $false }
                if (-not (Test-Task037ProcessIdentityCohortLiveInJob `
                    -Job $Job `
                    -Identities $RequiredProcessIdentities)) {
                    return $false
                }
                $remainingMilliseconds = [Math]::Max(
                    1,
                    [Math]::Min(250, $TimeoutMilliseconds - [int]$stopwatch.ElapsedMilliseconds)
                )
                $confirmed = Get-Task037CoherentJobProcessSnapshot `
                    -Job $Job `
                    -TimeoutMilliseconds $remainingMilliseconds
                return (
                    [uint32]$confirmed.TotalProcesses -eq $expectedTotalProcesses -and
                    (Test-Task037ProcessIdSetEqual `
                        -Left $confirmed.ProcessIds `
                        -Right $BaselineProcessIds) -and
                    (Test-Task037ProcessIdentityCohortLiveInJob `
                        -Job $Job `
                        -Identities $RequiredProcessIdentities)
                )
            }
            $previousProcessIds = [uint32[]]@($snapshot.ProcessIds)
            Start-Sleep -Milliseconds 10
        }
        return $false
    }
    catch {
        return $false
    }
}

function Release-Task037ExitedProcessHandles {
    param([Parameter(Mandatory = $true)]$State)

    $succeeded = $true
    foreach ($propertyName in @('ChildProcess', 'ControllerProcess')) {
        if ($State.PSObject.Properties.Name -cnotcontains $propertyName) {
            $succeeded = $false
            continue
        }
        $process = $State.$propertyName
        try {
            if ($null -eq $process -or $process -isnot [Diagnostics.Process]) {
                $succeeded = $false
            }
            else {
                if (-not $process.HasExited) { $succeeded = $false }
                $process.Dispose()
            }
        }
        catch {
            $succeeded = $false
        }
        finally {
            $State.$propertyName = $null
        }
    }
    return [bool]$succeeded
}

function Assert-Task037ProcessInJob {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [Parameter(Mandatory = $true)][Diagnostics.Process]$Process
    )

    try {
        if (-not [LatticeTask037JobObjectInterop]::IsMember($Job, $Process.Handle)) {
            throw 'TASK037_GATED_PROCESS_NOT_IN_JOB'
        }
    }
    catch {
        throw 'TASK037_GATED_PROCESS_NOT_IN_JOB'
    }
}

function Assert-Task037CurrentProcessInNamedJob {
    param([Parameter(Mandatory = $true)][string]$Name)

    Initialize-Task037JobObjectInterop
    $process = [Diagnostics.Process]::GetCurrentProcess()
    $job = [IntPtr]::Zero
    try {
        $job = [LatticeTask037JobObjectInterop]::OpenForQuery($Name)
        if (-not [LatticeTask037JobObjectInterop]::IsMember($job, $process.Handle)) {
            throw 'TASK037_SUPERVISOR_BINDING_REJECTED'
        }
    }
    catch { throw 'TASK037_SUPERVISOR_BINDING_REJECTED' }
    finally {
        if ($job -ne [IntPtr]::Zero) { try { [LatticeTask037JobObjectInterop]::Close($job) } catch { } }
        $process.Dispose()
    }
}

function Stop-Task037Job {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [ValidateRange(1, 30000)][int]$TimeoutMilliseconds = 10000
    )

    try {
        if ((Get-Task037JobActiveProcessCount -Job $Job) -gt 0) {
            [LatticeTask037JobObjectInterop]::Terminate($Job)
        }
        $stopwatch = [Diagnostics.Stopwatch]::StartNew()
        while ($stopwatch.ElapsedMilliseconds -lt $TimeoutMilliseconds) {
            if ((Get-Task037JobActiveProcessCount -Job $Job) -eq 0) {
                return
            }
            Start-Sleep -Milliseconds 10
        }
    }
    catch {
        throw 'TASK037_JOB_TERMINATION_REJECTED'
    }
    throw 'TASK037_JOB_TERMINATION_REJECTED'
}

function Close-Task037Job {
    param([Parameter(Mandatory = $true)][IntPtr]$Job)

    try {
        [LatticeTask037JobObjectInterop]::Close($Job)
    }
    catch {
        throw 'TASK037_JOB_CLEANUP_REJECTED'
    }
}

function Get-Task037TrustedTempBase {
    $localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    if ([string]::IsNullOrWhiteSpace($localAppData)) {
        throw 'TASK037_RECOVERY_SCOPE_REJECTED'
    }
    $base = Get-CanonicalPath -Path (Join-Path $localAppData 'Temp')
    Assert-NoReparsePath -Path $base -FailureCode 'TASK037_RECOVERY_SCOPE_REJECTED'
    $item = Get-Item -LiteralPath $base -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        -not $item.PSIsContainer -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK037_RECOVERY_SCOPE_REJECTED'
    }
    return $base
}

function Get-Task037SupervisedTempRoot {
    param(
        [Parameter(Mandatory = $true)][string]$AcceptanceId,
        [string]$TempBase
    )

    if ($AcceptanceId -notmatch '^[0-9a-f]{32}$') {
        throw 'TASK037_RECOVERY_BINDING_REJECTED'
    }
    $trustedBase = Get-Task037TrustedTempBase
    $base = if ([string]::IsNullOrWhiteSpace($TempBase)) {
        $trustedBase
    }
    else {
        Get-CanonicalPath -Path $TempBase
    }
    if (-not (Test-ExactPath -Actual $base -Expected $trustedBase)) {
        throw 'TASK037_RECOVERY_SCOPE_REJECTED'
    }
    return Get-Task037BoundTempRoot -AcceptanceId $AcceptanceId -TempBase $base
}

function Get-Task037BoundTempRoot {
    param(
        [Parameter(Mandatory = $true)][string]$AcceptanceId,
        [Parameter(Mandatory = $true)][string]$TempBase
    )

    if ($AcceptanceId -notmatch '^[0-9a-f]{32}$') {
        throw 'TASK037_RECOVERY_BINDING_REJECTED'
    }
    $base = Get-CanonicalPath -Path $TempBase
    return Get-CanonicalPath -Path (Join-Path $base ('lattice-task037-' + $AcceptanceId))
}

function Get-Task037RecoveryManifestPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][string]$AcceptanceId
    )

    if ($AcceptanceId -notmatch '^[0-9a-f]{32}$') {
        throw 'TASK037_RECOVERY_BINDING_REJECTED'
    }
    return Join-Path (Join-Path $RepositoryTarget 'full-chain-recovery') ($AcceptanceId + '.json')
}

function Get-Task037AdmissionReceiptPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][string]$AcceptanceId
    )

    if ($AcceptanceId -notmatch '^[0-9a-f]{32}$') {
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }
    return Join-Path (Join-Path $RepositoryTarget 'full-chain-admission') ($AcceptanceId + '.json')
}

function ConvertFrom-Task037AdmissionTimestamp {
    param([Parameter(Mandatory = $true)][string]$Value)

    try {
        $parsed = [DateTimeOffset]::ParseExact(
            $Value,
            'O',
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::None
        )
    }
    catch {
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }
    if (
        $parsed.Offset -ne [TimeSpan]::Zero -or
        $parsed.ToString('O', [Globalization.CultureInfo]::InvariantCulture) -cne $Value
    ) {
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }
    return $parsed
}

function Read-Task037AdmissionStageReceipt {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$AcceptanceId
    )

    Assert-RegularFile -Path $Path
    if ((Get-Item -LiteralPath $Path).Length -gt 4096) {
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }
    try { $receipt = Read-JsonEvidence -Path $Path }
    catch { throw 'TASK037_ADMISSION_RECEIPT_REJECTED' }
    Assert-Task037ExactPropertySet `
        -Value $receipt `
        -Expected @('acceptance_id', 'class', 'observed_at_utc', 'stage', 'started_at_utc') `
        -FailureCode 'TASK037_ADMISSION_RECEIPT_REJECTED'
    $stage = [string]$receipt.stage
    $started = ConvertFrom-Task037AdmissionTimestamp -Value ([string]$receipt.started_at_utc)
    $observed = ConvertFrom-Task037AdmissionTimestamp -Value ([string]$receipt.observed_at_utc)
    if (
        [string]$receipt.acceptance_id -cne $AcceptanceId -or
        [string]$receipt.class -cne 'lattice.task037.admission-stage.v2' -or
        [Array]::IndexOf($script:Task037AdmissionStages, $stage) -lt 0 -or
        $observed -lt $started
    ) {
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }
    return $receipt
}

function Write-Task037AdmissionStageReceipt {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][string]$AcceptanceId,
        [Parameter(Mandatory = $true)][string]$StartedAtUtc,
        [Parameter(Mandatory = $true)]
        [ValidateSet(
            'recovery_preflight',
            'supervision_create',
            'gated_controller_start',
            'supervisor_binding',
            'git_identity_preflight',
            'node_identity_preflight',
            'node_runtime_preflight',
            'rust_toolchain_preflight',
            'artifact_identity_preflight',
            'worktree_preflight',
            'inner_pre_acceptance'
        )]
        [string]$Stage
    )

    $null = ConvertFrom-Task037AdmissionTimestamp -Value $StartedAtUtc
    $path = Get-Task037AdmissionReceiptPath `
        -RepositoryTarget $RepositoryTarget `
        -AcceptanceId $AcceptanceId
    $parent = Split-Path -Parent $path
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force:$false | Out-Null
        $null = Set-Task037RestrictedDirectoryAcl -Path $parent
    }
    Assert-NoReparsePath -Path $parent -FailureCode 'TASK037_ADMISSION_RECEIPT_REJECTED'
    $null = Get-Task037RestrictedAclEvidence -Path $parent -RequireProtected

    $desiredIndex = [Array]::IndexOf($script:Task037AdmissionStages, $Stage)
    $existing = $null
    if (Test-Path -LiteralPath $path -PathType Leaf) {
        $existing = Read-Task037AdmissionStageReceipt -Path $path -AcceptanceId $AcceptanceId
        $existingIndex = [Array]::IndexOf($script:Task037AdmissionStages, [string]$existing.stage)
        if (
            $desiredIndex -ne ($existingIndex + 1) -or
            [string]$existing.started_at_utc -cne $StartedAtUtc
        ) {
            throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
        }
    }
    elseif ($desiredIndex -ne 0) {
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }

    $observedAtUtc = [DateTimeOffset]::UtcNow.ToString(
        'O',
        [Globalization.CultureInfo]::InvariantCulture
    )
    $value = [ordered]@{
        acceptance_id = $AcceptanceId
        class = 'lattice.task037.admission-stage.v2'
        stage = $Stage
        started_at_utc = $StartedAtUtc
        observed_at_utc = $observedAtUtc
    }
    $json = ($value | ConvertTo-Json -Depth 4) + [Environment]::NewLine
    Assert-Task037NoKnownSecretText -Text $json
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json)
    $pending = $path + '.pending-' + [Guid]::NewGuid().ToString('N')
    $backup = $path + '.backup-' + [Guid]::NewGuid().ToString('N')
    $stream = $null
    try {
        $stream = [IO.FileStream]::new(
            $pending,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None,
            4096,
            [IO.FileOptions]::WriteThrough
        )
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
        $stream.Dispose()
        $stream = $null
        if ($null -eq $existing) {
            [IO.File]::Move($pending, $path)
        }
        else {
            [IO.File]::Replace($pending, $path, $backup, $true)
            [IO.File]::Delete($backup)
            if (Test-Path -LiteralPath $backup) {
                throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
            }
        }
    }
    catch {
        if ($null -ne $stream) { $stream.Dispose() }
        if (Test-Path -LiteralPath $pending -PathType Leaf) {
            try { [IO.File]::Delete($pending) } catch { }
        }
        if (Test-Path -LiteralPath $backup -PathType Leaf) {
            try { [IO.File]::Delete($backup) } catch { }
        }
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }
    $written = Read-Task037AdmissionStageReceipt -Path $path -AcceptanceId $AcceptanceId
    if (
        [string]$written.stage -cne $Stage -or
        [string]$written.started_at_utc -cne $StartedAtUtc
    ) {
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }
    return $path
}

function Write-Task037DurableRecoveryManifest {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force:$false | Out-Null
    }
    Assert-NoReparsePath -Path $parent -FailureCode 'TASK037_RECOVERY_MANIFEST_REJECTED'
    if (Test-Path -LiteralPath $Path) {
        throw 'TASK037_RECOVERY_MANIFEST_NOT_FRESH'
    }
    $pending = $Path + '.pending-' + [Guid]::NewGuid().ToString('N')
    $stream = $null
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 8) + [Environment]::NewLine)
        $stream = [IO.FileStream]::new(
            $pending,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None,
            4096,
            [IO.FileOptions]::WriteThrough
        )
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
        $stream.Dispose()
        $stream = $null
        [IO.File]::Move($pending, $Path)
    }
    catch {
        if ($null -ne $stream) { $stream.Dispose() }
        if (Test-Path -LiteralPath $pending -PathType Leaf) {
            try { [IO.File]::Delete($pending) } catch { }
        }
        throw 'TASK037_RECOVERY_MANIFEST_REJECTED'
    }
    Assert-RegularFile -Path $Path
}

function Remove-Task037SupervisedTempRoot {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$AcceptanceId,
        [Parameter(Mandatory = $true)][string]$OwnerToken,
        [string]$TempBase
    )

    $expectedRoot = Get-Task037SupervisedTempRoot `
        -AcceptanceId $AcceptanceId `
        -TempBase $TempBase
    if (-not (Test-ExactPath -Actual $Root -Expected $expectedRoot)) {
        throw 'TASK037_RECOVERY_SCOPE_REJECTED'
    }
    if (-not (Test-Path -LiteralPath $expectedRoot)) { return $true }
    Assert-NoReparsePath -Path $expectedRoot -FailureCode 'TASK037_RECOVERY_SCOPE_REJECTED'
    $rootItem = Get-Item -LiteralPath $expectedRoot -Force
    if (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'TASK037_RECOVERY_SCOPE_REJECTED'
    }
    $markerPath = Join-Path $expectedRoot '.lattice-task037-cleanup-root-v1'
    $expectedMarker = "lattice.task037.cleanup-root.v1|$AcceptanceId|$OwnerToken`n"
    Assert-RegularFile -Path $markerPath
    if ([IO.File]::ReadAllText($markerPath, [Text.Encoding]::UTF8) -cne $expectedMarker) {
        throw 'TASK037_RECOVERY_MARKER_REJECTED'
    }
    $scopeRejected = $false
    $knownAuthPaths = [Collections.Generic.List[string]]::new()
    $knownAuthPaths.Add((Join-Path $expectedRoot 'delivery-codex-home\auth.json'))
    $topLevelItems = @(Get-ChildItem -LiteralPath $expectedRoot -Force)
    $phasePattern = '^phase-' + [regex]::Escape($AcceptanceId) + '-(pre-status|run|status)-[0-9a-f]{8}$'
    foreach ($item in $topLevelItems) {
        if ([string]$item.Name -match $phasePattern) {
            if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                $scopeRejected = $true
                continue
            }
            $knownAuthPaths.Add((Join-Path $item.FullName 'broker-codex-home\auth.json'))
        }
    }

    # Delete only fixed, derived credential paths before inspecting attacker-controlled children.
    foreach ($authPath in $knownAuthPaths) {
        $parent = Split-Path -Parent $authPath
        if (-not (Test-Path -LiteralPath $parent)) { continue }
        try {
            Assert-NoReparsePath -Path $parent -FailureCode 'TASK037_RECOVERY_SCOPE_REJECTED'
            $parentItem = Get-Item -LiteralPath $parent -Force
            if (-not $parentItem.PSIsContainer -or ($parentItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                $scopeRejected = $true
                continue
            }
            if (Test-Path -LiteralPath $authPath) {
                Assert-RegularFile -Path $authPath
                [IO.File]::Delete($authPath)
                if (Test-Path -LiteralPath $authPath) {
                    throw 'TASK037_RECOVERY_SECRET_CLEANUP_REJECTED'
                }
            }
        }
        catch {
            if ($_.Exception.Message -ceq 'TASK037_RECOVERY_SECRET_CLEANUP_REJECTED') { throw }
            $scopeRejected = $true
        }
    }

    $items = [Collections.Generic.List[object]]::new()
    $directories = [Collections.Generic.Queue[string]]::new()
    $directories.Enqueue($expectedRoot)
    while ($directories.Count -gt 0) {
        $directory = $directories.Dequeue()
        foreach ($item in @(Get-ChildItem -LiteralPath $directory -Force)) {
            $items.Add($item)
            if ($items.Count -gt 65536) {
                $scopeRejected = $true
                break
            }
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                $scopeRejected = $true
                continue
            }
            if ($item.PSIsContainer) { $directories.Enqueue($item.FullName) }
        }
        if ($items.Count -gt 65536) { break }
    }
    foreach ($authFile in @($items | Where-Object { -not $_.PSIsContainer -and $_.Name -ceq 'auth.json' })) {
        Assert-RegularFile -Path $authFile.FullName
        [IO.File]::Delete($authFile.FullName)
        if (Test-Path -LiteralPath $authFile.FullName) {
            throw 'TASK037_RECOVERY_SECRET_CLEANUP_REJECTED'
        }
        $scopeRejected = $true
    }
    foreach ($file in @($items | Where-Object {
        -not $_.PSIsContainer -and $_.Name -cne 'auth.json' -and
        -not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)
    })) {
        $probe = $null
        try {
            $probe = [IO.File]::Open($file.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
        }
        catch {
            throw 'TASK037_RECOVERY_FILE_LOCK_REJECTED'
        }
        finally {
            if ($null -ne $probe) { $probe.Dispose() }
        }
    }
    if ($scopeRejected) {
        throw 'TASK037_RECOVERY_SCOPE_REJECTED'
    }
    Remove-Item -LiteralPath $expectedRoot -Recurse -Force
    return -not (Test-Path -LiteralPath $expectedRoot)
}

function Test-Task037ExactProcessIdentityAlive {
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][int64]$ProcessStartTicks
    )

    $process = $null
    try {
        $process = [Diagnostics.Process]::GetProcessById($ProcessId)
        $null = $process.Handle
        return $process.StartTime.ToUniversalTime().Ticks -eq $ProcessStartTicks
    }
    catch { return $false }
    finally { if ($null -ne $process) { $process.Dispose() } }
}

function Read-Task037RecoveryManifest {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [switch]$RequireTrustedTempBase
    )

    Assert-RegularFile -Path $Path
    $record = Read-JsonEvidence -Path $Path
    Assert-Task037ExactPropertySet `
        -Value $record `
        -Expected @(
            'acceptance_id', 'controller_process_id', 'controller_process_start_ticks',
            'marker_sha256', 'owner_token', 'schema_version', 'temp_base', 'temp_root'
        ) `
        -FailureCode 'TASK037_RECOVERY_MANIFEST_REJECTED'
    $acceptanceId = [string]$record.acceptance_id
    $expectedPath = Get-Task037RecoveryManifestPath -RepositoryTarget $RepositoryTarget -AcceptanceId $acceptanceId
    try {
        $tempBase = Get-CanonicalPath -Path ([string]$record.temp_base)
        $tempRoot = Get-CanonicalPath -Path ([string]$record.temp_root)
        $expectedTempRoot = Get-Task037BoundTempRoot `
            -AcceptanceId $acceptanceId `
            -TempBase $tempBase
    }
    catch {
        throw 'TASK037_RECOVERY_MANIFEST_REJECTED'
    }
    if (
        [string]$record.schema_version -cne 'lattice.task037.recovery.v2' -or
        -not (Test-ExactPath -Actual $Path -Expected $expectedPath) -or
        -not (Test-ExactPath -Actual $tempRoot -Expected $expectedTempRoot) -or
        [string]$record.owner_token -notmatch '^[0-9a-f]{64}$' -or
        [string]$record.marker_sha256 -notmatch '^[0-9a-f]{64}$' -or
        [int]$record.controller_process_id -le 0 -or
        [int64]$record.controller_process_start_ticks -le 0
    ) {
        throw 'TASK037_RECOVERY_MANIFEST_REJECTED'
    }
    if (
        $RequireTrustedTempBase -and
        -not (Test-ExactPath -Actual $tempBase -Expected (Get-Task037TrustedTempBase))
    ) {
        throw 'TASK037_RECOVERY_MANIFEST_REJECTED'
    }
    return $record
}

function Assert-Task037SupervisorBinding {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][string]$AcceptanceId,
        [Parameter(Mandatory = $true)][string]$TempRoot
    )

    $ownerToken = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_SUPERVISOR_OWNER_TOKEN', 'Process')
    $manifestPath = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_SUPERVISOR_MANIFEST_PATH', 'Process')
    $tempBaseValue = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_SUPERVISOR_TEMP_BASE', 'Process')
    $expectedManifest = Get-Task037RecoveryManifestPath -RepositoryTarget $RepositoryTarget -AcceptanceId $AcceptanceId
    if ([string]::IsNullOrWhiteSpace($tempBaseValue)) {
        throw 'TASK037_SUPERVISOR_BINDING_TEMP_BASE_REJECTED'
    }
    if ($ownerToken -notmatch '^[0-9a-f]{64}$') {
        throw 'TASK037_SUPERVISOR_BINDING_OWNER_TOKEN_REJECTED'
    }
    if (
        [string]::IsNullOrWhiteSpace($manifestPath) -or
        -not (Test-ExactPath -Actual $manifestPath -Expected $expectedManifest)
    ) {
        throw 'TASK037_SUPERVISOR_BINDING_MANIFEST_PATH_REJECTED'
    }
    $record = Read-Task037RecoveryManifest -Path $expectedManifest -RepositoryTarget $RepositoryTarget
    if (-not (Test-ExactPath -Actual $tempBaseValue -Expected ([string]$record.temp_base))) {
        throw 'TASK037_SUPERVISOR_BINDING_TEMP_BASE_REJECTED'
    }
    if (
        -not (Test-ExactPath -Actual $TempRoot -Expected ([string]$record.temp_root)) -or
        -not (Test-ExactPath `
            -Actual ([string]$record.temp_root) `
            -Expected (Get-Task037BoundTempRoot `
                -AcceptanceId $AcceptanceId `
                -TempBase ([string]$record.temp_base)))
    ) {
        throw 'TASK037_SUPERVISOR_BINDING_TEMP_ROOT_REJECTED'
    }
    $expectedTempRoot = Get-CanonicalPath -Path ([string]$record.temp_root)
    $markerPath = Join-Path $expectedTempRoot '.lattice-task037-cleanup-root-v1'
    $expectedMarker = "lattice.task037.cleanup-root.v1|$AcceptanceId|$ownerToken`n"
    if (
        [string]$record.acceptance_id -cne $AcceptanceId -or
        [string]$record.owner_token -cne $ownerToken -or
        -not (Test-Task037ExactProcessIdentityAlive `
            -ProcessId ([int]$record.controller_process_id) `
            -ProcessStartTicks ([int64]$record.controller_process_start_ticks)) -or
        (Get-FileSha256 $markerPath) -cne [string]$record.marker_sha256 -or
        [IO.File]::ReadAllText($markerPath, [Text.Encoding]::UTF8) -cne $expectedMarker
    ) {
        throw 'TASK037_SUPERVISOR_BINDING_AUTHORITY_REJECTED'
    }
    foreach ($path in @($expectedManifest, $markerPath)) {
        $probe = $null
        $writeDenied = $false
        try {
            $probe = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::None)
        }
        catch [IO.IOException] { $writeDenied = $true }
        finally { if ($null -ne $probe) { $probe.Dispose() } }
        if (-not $writeDenied) { throw 'TASK037_SUPERVISOR_BINDING_LOCK_REJECTED' }
    }
    try {
        Assert-Task037CurrentProcessInNamedJob -Name ('LatticeTask037-' + $ownerToken)
    }
    catch {
        throw 'TASK037_SUPERVISOR_BINDING_JOB_REJECTED'
    }
}

function Resolve-Task037AuthorizedTempBase {
    param(
        [string]$AuthorizedTempBase,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $acceptanceId = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_SUPERVISOR_ACCEPTANCE_ID',
        'Process'
    )
    $manifestPath = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_SUPERVISOR_MANIFEST_PATH',
        'Process'
    )
    $ownerToken = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_SUPERVISOR_OWNER_TOKEN',
        'Process'
    )
    $boundTempBase = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_SUPERVISOR_TEMP_BASE',
        'Process'
    )
    $tempRoot = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_SUPERVISOR_TEMP_ROOT',
        'Process'
    )
    $bindingValues = @($acceptanceId, $manifestPath, $ownerToken, $boundTempBase, $tempRoot)
    $hasBinding = @($bindingValues | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count -ne 0
    if ($hasBinding) {
        try {
            if (
                [string]::IsNullOrWhiteSpace($AuthorizedTempBase) -or
                @($bindingValues | Where-Object { [string]::IsNullOrWhiteSpace($_) }).Count -ne 0
            ) {
                throw $FailureCode
            }
            $canonical = Get-CanonicalPath -Path $AuthorizedTempBase
            Assert-Task037SupervisorBinding `
                -RepositoryTarget (Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..\target')) `
                -AcceptanceId $acceptanceId `
                -TempRoot $tempRoot
            if (-not (Test-ExactPath -Actual $canonical -Expected $boundTempBase)) {
                throw $FailureCode
            }
            return $canonical
        }
        catch {
            throw $FailureCode
        }
    }

    $trustedBase = Get-Task037TrustedTempBase
    if ([string]::IsNullOrWhiteSpace($AuthorizedTempBase)) {
        return $trustedBase
    }
    $canonical = Get-CanonicalPath -Path $AuthorizedTempBase
    if (-not (Test-ExactPath -Actual $canonical -Expected $trustedBase)) {
        throw $FailureCode
    }
    return $canonical
}

function Invoke-Task037RecoveryOnly {
    param([Parameter(Mandatory = $true)][string]$RepositoryTarget)

    $recoveryRoot = Join-Path $RepositoryTarget 'full-chain-recovery'
    if (-not (Test-Path -LiteralPath $recoveryRoot -PathType Container)) { return 0 }
    Assert-NoReparsePath -Path $recoveryRoot -FailureCode 'TASK037_RECOVERY_SCOPE_REJECTED'
    $manifests = @(Get-ChildItem -LiteralPath $recoveryRoot -File -Force | Where-Object { $_.Name -match '^[0-9a-f]{32}\.json$' })
    if ($manifests.Count -gt 64) { throw 'TASK037_RECOVERY_SCOPE_REJECTED' }
    $removed = 0
    foreach ($manifest in $manifests) {
        $record = Read-Task037RecoveryManifest `
            -Path $manifest.FullName `
            -RepositoryTarget $RepositoryTarget `
            -RequireTrustedTempBase
        if (Test-Task037ExactProcessIdentityAlive `
            -ProcessId ([int]$record.controller_process_id) `
            -ProcessStartTicks ([int64]$record.controller_process_start_ticks)
        ) {
            throw 'TASK037_RECOVERY_ACTIVE_OWNER_REJECTED'
        }
        $tempBase = Get-CanonicalPath -Path ([string]$record.temp_base)
        $tempRoot = Get-CanonicalPath -Path ([string]$record.temp_root)
        $markerPath = Join-Path $tempRoot '.lattice-task037-cleanup-root-v1'
        if (
            (Test-Path -LiteralPath $tempRoot) -and
            ((Get-FileSha256 $markerPath) -cne [string]$record.marker_sha256)
        ) {
            throw 'TASK037_RECOVERY_MARKER_REJECTED'
        }
        if (-not (Remove-Task037SupervisedTempRoot `
            -Root $tempRoot `
            -AcceptanceId ([string]$record.acceptance_id) `
            -OwnerToken ([string]$record.owner_token) `
            -TempBase $tempBase
        )) {
            throw 'TASK037_RECOVERY_CLEANUP_REJECTED'
        }
        [IO.File]::Delete($manifest.FullName)
        if (Test-Path -LiteralPath $manifest.FullName) {
            throw 'TASK037_RECOVERY_MANIFEST_CLEANUP_REJECTED'
        }
        $removed++
    }
    return $removed
}

function New-Task037CleanupSupervision {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryTarget,
        [Parameter(Mandatory = $true)][string]$AcceptanceId,
        [string]$TempBase
    )

    $canonicalTempBase = if ([string]::IsNullOrWhiteSpace($TempBase)) {
        Get-Task037TrustedTempBase
    }
    else {
        Get-CanonicalPath -Path $TempBase
    }
    $tempRoot = Get-Task037SupervisedTempRoot `
        -AcceptanceId $AcceptanceId `
        -TempBase $canonicalTempBase
    if (Test-Path -LiteralPath $tempRoot) { throw 'TASK037_SUPERVISOR_ROOT_NOT_FRESH' }
    $ownerToken = New-RandomHex -Bytes 32
    $jobName = 'LatticeTask037-' + $ownerToken
    $markerPath = Join-Path $tempRoot '.lattice-task037-cleanup-root-v1'
    $manifestPath = Get-Task037RecoveryManifestPath -RepositoryTarget $RepositoryTarget -AcceptanceId $AcceptanceId
    $controller = $null
    $manifestLock = $null
    $markerLock = $null
    $job = [IntPtr]::Zero
    $attestationJob = [IntPtr]::Zero
    $rootCreated = $false
    try {
        New-Item -ItemType Directory -Path $tempRoot -Force:$false | Out-Null
        $rootCreated = $true
        $null = Set-Task037RestrictedDirectoryAcl -Path $tempRoot
        [IO.File]::WriteAllText(
            $markerPath,
            "lattice.task037.cleanup-root.v1|$AcceptanceId|$ownerToken`n",
            [Text.UTF8Encoding]::new($false)
        )
        Assert-Task037SingleLinkFile -Path $markerPath
        $null = Get-Task037RestrictedAclEvidence -Path $markerPath
        $controller = [Diagnostics.Process]::GetCurrentProcess()
        Write-Task037DurableRecoveryManifest -Path $manifestPath -Value ([ordered]@{
            schema_version = 'lattice.task037.recovery.v2'
            acceptance_id = $AcceptanceId
            owner_token = $ownerToken
            marker_sha256 = Get-FileSha256 $markerPath
            controller_process_id = [int]$controller.Id
            controller_process_start_ticks = [int64]$controller.StartTime.ToUniversalTime().Ticks
            temp_base = $canonicalTempBase
            temp_root = $tempRoot
        })
        $manifestLock = [IO.File]::Open($manifestPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        $markerLock = [IO.File]::Open($markerPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        $job = New-Task037KillOnCloseJob
        $attestationJob = New-Task037QueryOnlyAttestationJob -Name $jobName
        return [pscustomobject]@{
            AcceptanceId = $AcceptanceId
            OwnerToken = $ownerToken
            TempRoot = $tempRoot
            TempBase = $canonicalTempBase
            MarkerPath = $markerPath
            MarkerLock = $markerLock
            ManifestPath = $manifestPath
            ManifestLock = $manifestLock
            Job = $job
            AttestationJob = $attestationJob
            JobName = $jobName
        }
    }
    catch {
        if ($attestationJob -ne [IntPtr]::Zero) { try { Close-Task037Job -Job $attestationJob } catch { } }
        if ($job -ne [IntPtr]::Zero) { try { Close-Task037Job -Job $job } catch { } }
        if ($null -ne $markerLock) { try { $markerLock.Dispose() } catch { } }
        if ($null -ne $manifestLock) { try { $manifestLock.Dispose() } catch { } }
        if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
            try { [IO.File]::Delete($manifestPath) } catch { }
        }
        if ($rootCreated -and (Test-Path -LiteralPath $tempRoot)) {
            try {
                if (Test-Path -LiteralPath $markerPath -PathType Leaf) {
                    $null = Remove-Task037SupervisedTempRoot `
                        -Root $tempRoot `
                        -AcceptanceId $AcceptanceId `
                        -OwnerToken $ownerToken
                }
                else {
                    Remove-Item -LiteralPath $tempRoot -Recurse -Force
                }
            }
            catch { }
        }
        throw 'TASK037_SUPERVISOR_INITIALIZATION_REJECTED'
    }
    finally {
        if ($null -ne $controller) { $controller.Dispose() }
    }
}

function Complete-Task037CleanupSupervision {
    param([Parameter(Mandatory = $true)]$Supervision)

    Stop-Task037Job -Job $Supervision.Job
    if ((Get-Task037JobActiveProcessCount -Job $Supervision.Job) -ne 0) {
        throw 'TASK037_SUPERVISOR_JOB_ZERO_REJECTED'
    }
    if ((Get-Task037JobActiveProcessCount -Job $Supervision.AttestationJob) -ne 0) {
        throw 'TASK037_SUPERVISOR_ATTESTATION_JOB_ZERO_REJECTED'
    }
    Close-Task037Job -Job $Supervision.Job
    $Supervision.Job = [IntPtr]::Zero
    Close-Task037Job -Job $Supervision.AttestationJob
    $Supervision.AttestationJob = [IntPtr]::Zero
    $Supervision.MarkerLock.Dispose()
    $Supervision.MarkerLock = $null
    if (-not (Remove-Task037SupervisedTempRoot `
        -Root $Supervision.TempRoot `
        -AcceptanceId $Supervision.AcceptanceId `
        -OwnerToken $Supervision.OwnerToken `
        -TempBase $Supervision.TempBase
    )) {
        throw 'TASK037_SUPERVISOR_ROOT_CLEANUP_REJECTED'
    }
    $Supervision.ManifestLock.Dispose()
    $Supervision.ManifestLock = $null
    [IO.File]::Delete($Supervision.ManifestPath)
    if (Test-Path -LiteralPath $Supervision.ManifestPath) {
        throw 'TASK037_RECOVERY_MANIFEST_CLEANUP_REJECTED'
    }
}

function Wait-LoopbackPortClosed {
    param([Parameter(Mandatory = $true)][int]$Port)

    if ($Port -le 0) {
        return
    }
    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
    while (
        $stopwatch.ElapsedMilliseconds -lt 10000 -and
        @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue).Count -gt 0
    ) {
        Start-Sleep -Milliseconds 100
    }
}

function Remove-SafeTempRoot {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [switch]$CredentialHome,
        $Ownership,
        [string]$AuthorizedTempBase
    )

    $canonical = Get-CanonicalPath -Path $Path
    $tempBase = Resolve-Task037AuthorizedTempBase `
        -AuthorizedTempBase $AuthorizedTempBase `
        -FailureCode 'TASK037_TEMP_CLEANUP_SCOPE_REJECTED'
    $prefix = $tempBase + [IO.Path]::DirectorySeparatorChar + 'lattice-'
    if (-not $canonical.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'TASK037_TEMP_CLEANUP_SCOPE_REJECTED'
    }
    if (-not (Test-Path -LiteralPath $Path)) {
        if ($CredentialHome -and $null -ne $Ownership) {
            if ($null -ne $Ownership.OwnerRecordLock) {
                $Ownership.OwnerRecordLock.Dispose()
                $Ownership.OwnerRecordLock = $null
            }
            if (Test-Path -LiteralPath ([string]$Ownership.OwnerRecordPath) -PathType Leaf) {
                [IO.File]::Delete([string]$Ownership.OwnerRecordPath)
            }
            if (Test-Path -LiteralPath ([string]$Ownership.OwnerRecordPath)) {
                throw 'TASK037_TEMP_CLEANUP_OWNER_REJECTED'
            }
        }
        return $true
    }
    $rootItem = Get-Item -LiteralPath $canonical -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $rootItem -or
        -not $rootItem.PSIsContainer -or
        ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK037_TEMP_CLEANUP_SHAPE_REJECTED'
    }
    if ($CredentialHome) {
        if ($null -eq $Ownership) {
            throw 'TASK037_TEMP_CLEANUP_OWNER_REJECTED'
        }
        if (
            $null -eq $Ownership.OwnerRecordLock -or
            -not $Ownership.OwnerRecordLock.CanRead
        ) {
            throw 'TASK037_TEMP_CLEANUP_OWNER_REJECTED'
        }
        $ownerRecordPath = Get-CanonicalPath -Path ([string]$Ownership.OwnerRecordPath)
        Assert-NoReparsePath -Path $ownerRecordPath -FailureCode 'TASK037_TEMP_CLEANUP_OWNER_REJECTED'
        Assert-RegularFile -Path $ownerRecordPath
        $record = Read-JsonEvidence -Path $ownerRecordPath
        if (
            [string]$record.schema_version -cne 'lattice.task037.credential-owner.v1' -or
            -not (Test-ExactPath -Actual ([string]$record.destination) -Expected $canonical) -or
            [string]$record.owner_token -cne [string]$Ownership.OwnerToken -or
            [int]$record.controller_process_id -ne [int]$Ownership.ControllerProcessId -or
            [int64]$record.controller_process_start_ticks -ne [int64]$Ownership.ControllerProcessStartTicks -or
            [string]$record.source_auth_sha256 -cne [string]$Ownership.SourceAuthSha256 -or
            [int64]$record.source_auth_bytes -ne [int64]$Ownership.SourceAuthBytes
        ) {
            throw 'TASK037_TEMP_CLEANUP_OWNER_REJECTED'
        }
        $authPath = Join-Path $canonical 'auth.json'
        if (Test-Path -LiteralPath $authPath) {
            Assert-RegularFile -Path $authPath
            [IO.File]::Delete($authPath)
        }
        if (Test-Path -LiteralPath $authPath) {
            throw 'TASK037_CODEX_HOME_SECRET_CLEANUP_REJECTED'
        }
    }
    $items = @(Get-ChildItem -LiteralPath $canonical -Recurse -Force)
    if (
        $items.Count -gt 4096 -or
        @($items | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count -ne 0
    ) {
        throw 'TASK037_TEMP_CLEANUP_SHAPE_REJECTED'
    }
    foreach ($file in @($items | Where-Object { -not $_.PSIsContainer })) {
        $probe = $null
        try {
            $probe = [IO.File]::Open($file.FullName, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
        }
        catch {
            throw 'TASK037_TEMP_CLEANUP_FILE_LOCK_REJECTED'
        }
        finally {
            if ($null -ne $probe) { $probe.Dispose() }
        }
    }
    Remove-Item -LiteralPath $canonical -Recurse -Force
    if (Test-Path -LiteralPath $canonical) {
        return $false
    }
    if ($CredentialHome) {
        $Ownership.OwnerRecordLock.Dispose()
        [IO.File]::Delete([string]$Ownership.OwnerRecordPath)
        if (Test-Path -LiteralPath ([string]$Ownership.OwnerRecordPath)) {
            throw 'TASK037_TEMP_CLEANUP_OWNER_REJECTED'
        }
    }
    return $true
}

function Get-CodexCredentialSourceEvidence {
    param([Parameter(Mandatory = $true)][string]$SourceHome)

    $source = Get-CanonicalPath -Path $SourceHome
    Assert-NoReparsePath -Path $source -FailureCode 'TASK037_CODEX_CREDENTIAL_SOURCE_REJECTED'
    $sourceItem = Get-Item -LiteralPath $source -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $sourceItem -or
        -not $sourceItem.PSIsContainer -or
        ($sourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK037_CODEX_CREDENTIAL_SOURCE_REJECTED'
    }
    $auth = Join-Path $source 'auth.json'
    Assert-NoReparseAncestor -Path $auth -Boundary $source
    Assert-RegularFile -Path $auth
    $authItem = Get-Item -LiteralPath $auth -Force
    if ($authItem.Length -le 0 -or $authItem.Length -gt 4194304) {
        throw 'TASK037_CODEX_CREDENTIAL_SOURCE_REJECTED'
    }
    return [pscustomobject]@{
        Source = $source
        AuthPath = $auth
        AuthBytes = [int64]$authItem.Length
        AuthSha256 = Get-FileSha256 $auth
    }
}

function Assert-CodexCredentialSourceUnchanged {
    param(
        [Parameter(Mandatory = $true)][string]$SourceHome,
        [Parameter(Mandatory = $true)]$Expected
    )

    $actual = Get-CodexCredentialSourceEvidence -SourceHome $SourceHome
    if (
        -not (Test-ExactPath -Actual $actual.Source -Expected $Expected.Source) -or
        [int64]$actual.AuthBytes -ne [int64]$Expected.AuthBytes -or
        [string]$actual.AuthSha256 -cne [string]$Expected.AuthSha256
    ) {
        throw 'TASK037_CODEX_CREDENTIAL_SOURCE_MUTATED'
    }
}

function Start-Task037CredentialSourceMonitor {
    param([Parameter(Mandatory = $true)][string]$SourceHome)

    $source = Get-CanonicalPath -Path $SourceHome
    $queue = [Collections.Concurrent.ConcurrentQueue[string]]::new()
    $watcher = [IO.FileSystemWatcher]::new($source)
    $watcher.Filter = 'auth.json'
    $watcher.IncludeSubdirectories = $false
    $watcher.NotifyFilter = (
        [IO.NotifyFilters]::FileName -bor [IO.NotifyFilters]::LastWrite -bor
        [IO.NotifyFilters]::Size -bor [IO.NotifyFilters]::Security
    )
    $watcher.InternalBufferSize = 65536
    $prefix = 'LatticeTask037Source-' + [Guid]::NewGuid().ToString('N')
    $subscriptions = @()
    try {
        foreach ($eventName in @('Changed', 'Created', 'Deleted', 'Renamed', 'Error')) {
            $subscriptions += @(Register-ObjectEvent `
                -InputObject $watcher `
                -EventName $eventName `
                -SourceIdentifier ($prefix + '-' + $eventName) `
                -MessageData $queue `
                -Action { $event.MessageData.Enqueue([string]$event.SourceEventArgs.GetType().Name) })
        }
        $watcher.EnableRaisingEvents = $true
        return [pscustomobject]@{
            Source = $source
            Queue = $queue
            Watcher = $watcher
            Subscriptions = $subscriptions
        }
    }
    catch {
        foreach ($subscription in $subscriptions) {
            try { Unregister-Event -SourceIdentifier $subscription.Name -Force } catch { }
            try { Remove-Job -Id $subscription.Id -Force } catch { }
        }
        $watcher.Dispose()
        throw 'TASK037_CODEX_CREDENTIAL_SOURCE_MONITOR_REJECTED'
    }
}

function Stop-Task037CredentialSourceMonitor {
    param(
        [Parameter(Mandatory = $true)]$Monitor,
        [Parameter(Mandatory = $true)][string]$SourceHome,
        [Parameter(Mandatory = $true)]$Expected
    )

    Start-Sleep -Milliseconds 250
    $Monitor.Watcher.EnableRaisingEvents = $false
    Start-Sleep -Milliseconds 100
    foreach ($subscription in @($Monitor.Subscriptions)) {
        try { Unregister-Event -SourceIdentifier $subscription.Name -Force } catch { }
        try { Remove-Job -Id $subscription.Id -Force } catch { }
    }
    $Monitor.Watcher.Dispose()
    Assert-CodexCredentialSourceUnchanged -SourceHome $SourceHome -Expected $Expected
    if ($Monitor.Queue.Count -ne 0) {
        throw 'TASK037_CODEX_CREDENTIAL_SOURCE_MUTATED'
    }
}

function Copy-CodexCredentialFiles {
    param(
        [Parameter(Mandatory = $true)][string]$SourceHome,
        [Parameter(Mandatory = $true)][string]$DestinationHome,
        [Parameter(Mandatory = $true)][string]$OwnerToken,
        [Parameter(Mandatory = $true)][string]$OwnerRecordPath,
        [switch]$ForBroker,
        [string]$AuthorizedTempBase
    )

    if ($OwnerToken -notmatch '^[a-z0-9][a-z0-9-]{0,127}$') {
        throw 'TASK037_CODEX_HOME_OWNER_REJECTED'
    }
    $sourceEvidence = Get-CodexCredentialSourceEvidence -SourceHome $SourceHome
    $destination = Get-CanonicalPath -Path $DestinationHome
    $ownerRecord = Get-CanonicalPath -Path $OwnerRecordPath
    $tempBase = Resolve-Task037AuthorizedTempBase `
        -AuthorizedTempBase $AuthorizedTempBase `
        -FailureCode 'TASK037_CODEX_HOME_SCOPE_REJECTED'
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $repositoryPrefix = $repositoryRoot + [IO.Path]::DirectorySeparatorChar
    $ambientCodexHome = [Environment]::GetEnvironmentVariable('CODEX_HOME', 'Process')
    if (
        (Test-PathOverlap -Left $sourceEvidence.Source -Right $destination) -or
        (Test-PathOverlap -Left $sourceEvidence.Source -Right $repositoryRoot) -or
        (Test-PathOverlap -Left $destination -Right $repositoryRoot) -or
        (Test-PathOverlap -Left $ownerRecord -Right $destination) -or
        (Test-PathOverlap -Left $ownerRecord -Right $sourceEvidence.Source) -or
        -not $ownerRecord.StartsWith($repositoryPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        (
            -not [string]::IsNullOrWhiteSpace($ambientCodexHome) -and
            (Test-PathOverlap -Left $destination -Right (Get-CanonicalPath -Path $ambientCodexHome))
        ) -or
        -not $destination.StartsWith(
            ($tempBase + [IO.Path]::DirectorySeparatorChar + 'lattice-'),
            [StringComparison]::OrdinalIgnoreCase
        ) -or
        (Test-Path -LiteralPath $ownerRecord)
    ) {
        throw 'TASK037_CODEX_HOME_SCOPE_REJECTED'
    }
    Assert-NoReparsePath -Path (Split-Path -Parent $ownerRecord) -FailureCode 'TASK037_CODEX_HOME_SCOPE_REJECTED'
    Assert-NoReparsePath -Path (Split-Path -Parent $destination) -FailureCode 'TASK037_CODEX_HOME_SCOPE_REJECTED'
    if (Test-Path -LiteralPath $DestinationHome) {
        throw 'TASK037_CODEX_HOME_NOT_FRESH'
    }
    $created = New-Item -ItemType Directory -Path $DestinationHome -ErrorAction Stop
    if (
        $null -eq $created -or
        -not $created.PSIsContainer -or
        ($created.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        @(Get-ChildItem -LiteralPath $DestinationHome -Force).Count -ne 0
    ) {
        throw 'TASK037_CODEX_HOME_NOT_FRESH'
    }
    $null = Set-Task037RestrictedDirectoryAcl -Path $DestinationHome
    [IO.File]::WriteAllBytes((Join-Path $DestinationHome '.lattice-codex-home-v1'), $codexMarkerBytes)
    if (-not $ForBroker) {
        [IO.File]::WriteAllBytes((Join-Path $DestinationHome 'config.toml'), $deliveryConfigBytes)
    }
    $controller = [Diagnostics.Process]::GetCurrentProcess()
    $ownership = [pscustomobject]@{
        OwnerRecordPath = $ownerRecord
        OwnerToken = $OwnerToken
        ControllerProcessId = [int]$controller.Id
        ControllerProcessStartTicks = [int64]$controller.StartTime.ToUniversalTime().Ticks
        Destination = $destination
        SourceAuthSha256 = [string]$sourceEvidence.AuthSha256
        SourceAuthBytes = [int64]$sourceEvidence.AuthBytes
        OwnerRecordLock = $null
    }
    Write-JsonEvidence -Path $ownerRecord -Value ([ordered]@{
        schema_version = 'lattice.task037.credential-owner.v1'
        owner_token = $OwnerToken
        destination = $destination
        controller_process_id = $ownership.ControllerProcessId
        controller_process_start_ticks = $ownership.ControllerProcessStartTicks
        source_auth_sha256 = $ownership.SourceAuthSha256
        source_auth_bytes = $ownership.SourceAuthBytes
    })
    $recordLock = $null
    try {
        $recordLock = [IO.File]::Open(
            $ownerRecord,
            [IO.FileMode]::Open,
            [IO.FileAccess]::Read,
            [IO.FileShare]::Read
        )
        $ownership.OwnerRecordLock = $recordLock
        [IO.File]::Copy($sourceEvidence.AuthPath, (Join-Path $DestinationHome 'auth.json'), $false)
        $expectedCount = if ($ForBroker) { 2 } else { 3 }
        $items = @(Get-ChildItem -LiteralPath $DestinationHome -Force)
        if (
            $items.Count -ne $expectedCount -or
            @($items | Where-Object {
                $_.PSIsContainer -or ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)
            }).Count -ne 0 -or
            (Get-FileSha256 (Join-Path $DestinationHome 'auth.json')) -cne $sourceEvidence.AuthSha256
        ) {
            throw 'TASK037_CODEX_HOME_PROVISION_REJECTED'
        }
        $null = Get-Task037RestrictedAclEvidence -Path $DestinationHome -RequireProtected
        foreach ($file in $items) {
            Assert-Task037SingleLinkFile -Path $file.FullName
            $null = Get-Task037RestrictedAclEvidence -Path $file.FullName
        }
        return $ownership
    }
    catch {
        if ($null -ne $recordLock) { $recordLock.Dispose() }
        throw
    }
}

function Get-Task037CredentialOwnershipFromRecord {
    param([Parameter(Mandatory = $true)][string]$OwnerRecordPath)

    $canonical = Get-CanonicalPath -Path $OwnerRecordPath
    Assert-NoReparsePath -Path $canonical -FailureCode 'TASK037_TEMP_CLEANUP_OWNER_REJECTED'
    Assert-RegularFile -Path $canonical
    $record = Read-JsonEvidence -Path $canonical
    if (
        [string]$record.schema_version -cne 'lattice.task037.credential-owner.v1' -or
        [string]$record.owner_token -notmatch '^[a-z0-9][a-z0-9-]{0,127}$' -or
        [string]$record.source_auth_sha256 -notmatch '^[0-9a-f]{64}$' -or
        [int]$record.controller_process_id -le 0 -or
        [int64]$record.controller_process_start_ticks -le 0 -or
        [int64]$record.source_auth_bytes -le 0
    ) {
        throw 'TASK037_TEMP_CLEANUP_OWNER_REJECTED'
    }
    $destination = Get-CanonicalPath -Path ([string]$record.destination)
    $recordLock = [IO.File]::Open(
        $canonical,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::Read
    )
    return [pscustomobject]@{
        OwnerRecordPath = $canonical
        OwnerToken = [string]$record.owner_token
        ControllerProcessId = [int]$record.controller_process_id
        ControllerProcessStartTicks = [int64]$record.controller_process_start_ticks
        Destination = $destination
        SourceAuthSha256 = [string]$record.source_auth_sha256
        SourceAuthBytes = [int64]$record.source_auth_bytes
        OwnerRecordLock = $recordLock
    }
}

function Test-Task037StartedPhaseJobsZero {
    param(
        [Parameter(Mandatory = $true)][string]$EvidenceRoot,
        [Parameter(Mandatory = $true)][string]$AcceptanceId
    )

    foreach ($phaseName in @('pre-status', 'run', 'status')) {
        $startedPath = Join-Path $EvidenceRoot ($phaseName + '.started.json')
        if (-not (Test-Path -LiteralPath $startedPath)) { continue }
        Assert-RegularFile -Path $startedPath
        $started = Read-JsonEvidence -Path $startedPath
        $cleanupPath = Join-Path $EvidenceRoot ($phaseName + '.cleanup.json')
        if (-not (Test-Path -LiteralPath $cleanupPath -PathType Leaf)) { return $false }
        Assert-RegularFile -Path $cleanupPath
        $cleanup = Read-JsonEvidence -Path $cleanupPath
        if (
            [string]$started.schema_version -cne 'lattice.task037.phase-started.v2' -or
            [string]$started.acceptance_id -cne $AcceptanceId -or
            [string]$cleanup.schema_version -cne 'lattice.task037.cleanup.v2' -or
            [string]$cleanup.acceptance_id -cne $AcceptanceId -or
            -not (Test-Task037PhaseGateName -Gate $cleanup.failure_gate) -or
            -not (Test-Task037PhaseGateName -Gate $cleanup.last_completed_gate) -or
            $cleanup.job_active_process_count_zero -isnot [bool] -or
            $cleanup.full_chain_process_stopped -isnot [bool] -or
            $cleanup.openclaw_process_stopped -isnot [bool] -or
            $cleanup.full_chain_controller_stopped -isnot [bool] -or
            $cleanup.openclaw_controller_stopped -isnot [bool] -or
            -not $cleanup.job_active_process_count_zero -or
            -not $cleanup.full_chain_process_stopped -or
            -not $cleanup.openclaw_process_stopped -or
            -not $cleanup.full_chain_controller_stopped -or
            -not $cleanup.openclaw_controller_stopped
        ) {
            return $false
        }
    }
    return $true
}

function New-OpenClawProfile {
    param(
        [Parameter(Mandatory = $true)][string]$ProfileRoot,
        [Parameter(Mandatory = $true)][string]$PluginRoot
    )

    if (-not (Test-Path -LiteralPath $ProfileRoot)) {
        New-Item -ItemType Directory -Path $ProfileRoot -Force:$false | Out-Null
    }
    $profile = [ordered]@{
        agents = [ordered]@{
            defaults = [ordered]@{
                heartbeat = [ordered]@{ every = '0m' }
                memorySearch = [ordered]@{ enabled = $false }
            }
        }
        cron = [ordered]@{ enabled = $false }
        discovery = [ordered]@{ mdns = [ordered]@{ mode = 'off' } }
        gateway = [ordered]@{
            auth = [ordered]@{ mode = 'token' }
            bind = 'loopback'
            mode = 'local'
            tailscale = [ordered]@{ mode = 'off' }
            terminal = [ordered]@{ enabled = $false }
        }
        hooks = [ordered]@{ enabled = $false }
        plugins = [ordered]@{
            allow = @('lattice-devos')
            entries = [ordered]@{ 'lattice-devos' = [ordered]@{ enabled = $true } }
            load = [ordered]@{ paths = @($PluginRoot) }
            slots = [ordered]@{ memory = 'none' }
        }
        update = [ordered]@{
            auto = [ordered]@{ enabled = $false }
            checkOnStart = $false
        }
    }
    Write-JsonEvidence -Path (Join-Path $ProfileRoot 'openclaw.json') -Value $profile
    return Get-FileSha256 (Join-Path $ProfileRoot 'openclaw.json')
}

function Get-TrustedTask037PowerShell {
    $current = Get-CanonicalPath -Path ([Diagnostics.Process]::GetCurrentProcess().MainModule.FileName)
    $expected = Get-CanonicalPath -Path (Join-Path $PSHOME 'powershell.exe')
    Assert-RegularFile -Path $current
    if (-not (Test-ExactPath -Actual $current -Expected $expected)) {
        throw 'TASK037_POWERSHELL_IDENTITY_REJECTED'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $current
    if (
        [string]$signature.Status -ne 'Valid' -or
        $null -eq $signature.SignerCertificate -or
        [string]$signature.SignerCertificate.Subject -notlike 'CN=Microsoft Windows,*O=Microsoft Corporation*'
    ) {
        throw 'TASK037_POWERSHELL_SIGNATURE_REJECTED'
    }
    return $current
}

function Get-TrustedTask037Node {
    param(
        [Parameter(Mandatory = $true)][string]$Candidate,
        [Parameter(Mandatory = $true)][string]$ProgramDataAuthority,
        [scriptblock]$RuntimeStageWriter
    )

    $canonical = Get-CanonicalPath -Path $Candidate
    $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    $expected = Get-CanonicalPath -Path (Join-Path $programFiles 'nodejs\node.exe')
    Assert-RegularFile -Path $canonical
    if (
        -not (Test-ExactPath -Actual $canonical -Expected $expected) -or
        (Get-FileSha256 $canonical) -cne $officialNodeSha256
    ) {
        throw 'TASK037_NODE_IDENTITY_REJECTED'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $canonical
    if (
        [string]$signature.Status -ne 'Valid' -or
        $null -eq $signature.SignerCertificate -or
        [string]$signature.SignerCertificate.Subject -notlike 'CN=OpenJS Foundation,*O=OpenJS Foundation*'
    ) {
        throw 'TASK037_NODE_IDENTITY_REJECTED'
    }
    if ($null -ne $RuntimeStageWriter) {
        & $RuntimeStageWriter 'node_runtime_preflight'
    }
    $nodeProbeTempBase = Resolve-Task037AuthorizedTempBase `
        -AuthorizedTempBase ([Environment]::GetEnvironmentVariable(
            'LATTICE_TASK037_SUPERVISOR_TEMP_BASE',
            'Process'
        )) `
        -FailureCode 'TASK037_NODE_IDENTITY_REJECTED'
    $probeRoot = Get-CanonicalPath -Path (
        Join-Path ([IO.Path]::GetTempPath()) ('lattice-task037-node-probe-' + [Guid]::NewGuid().ToString('N'))
    )
    if (-not $probeRoot.StartsWith(
        $nodeProbeTempBase + [IO.Path]::DirectorySeparatorChar + 'lattice-',
        [StringComparison]::OrdinalIgnoreCase
    )) {
        throw 'TASK037_NODE_IDENTITY_REJECTED'
    }
    $job = [IntPtr]::Zero
    $owned = $null
    try {
        New-Item -ItemType Directory -Path $probeRoot -Force:$false | Out-Null
        $probeScriptPath = Join-Path $probeRoot 'node-identity-probe.js'
        $probeReleasePath = Join-Path $probeRoot 'node-identity-probe.release'
        $probeScript = @'
'use strict';
const fs = require('node:fs');
const release = process.argv[2];
process.stdout.write(`${process.version}\n`);
const deadline = Date.now() + 10000;
const timer = setInterval(() => {
  if (fs.existsSync(release)) {
    clearInterval(timer);
    process.exit(0);
  }
  if (Date.now() >= deadline) {
    clearInterval(timer);
    process.exit(124);
  }
}, 10);
'@
        $probeScriptBytes = [Text.UTF8Encoding]::new($false).GetBytes($probeScript)
        $probeScriptStream = [IO.FileStream]::new(
            $probeScriptPath,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None,
            4096,
            [IO.FileOptions]::WriteThrough
        )
        try {
            $probeScriptStream.Write($probeScriptBytes, 0, $probeScriptBytes.Length)
            $probeScriptStream.Flush($true)
        }
        finally {
            $probeScriptStream.Dispose()
        }
        $job = New-Task037KillOnCloseJob
        $owned = Start-Task037GatedProcess `
            -Job $job `
            -PowerShellExecutable (Get-TrustedTask037PowerShell) `
            -Executable $canonical `
            -Arguments @($probeScriptPath, $probeReleasePath) `
            -WorkingDirectory $probeRoot `
            -EnvironmentValues ([ordered]@{}) `
            -ChildPidPath (Join-Path $probeRoot 'node.pid') `
            -ChildStdoutPath (Join-Path $probeRoot 'node.stdout') `
            -ChildStderrPath (Join-Path $probeRoot 'node.stderr') `
            -WrapperStdoutPath (Join-Path $probeRoot 'wrapper.stdout') `
            -WrapperStderrPath (Join-Path $probeRoot 'wrapper.stderr') `
            -RuntimeIsolationRoot (Join-Path $probeRoot 'runtime') `
            -TrustedProgramData $ProgramDataAuthority
        $probeReleaseBytes = [Text.UTF8Encoding]::new($false).GetBytes("release`n")
        $probeReleaseStream = [IO.FileStream]::new(
            $probeReleasePath,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None,
            4096,
            [IO.FileOptions]::WriteThrough
        )
        try {
            $probeReleaseStream.Write($probeReleaseBytes, 0, $probeReleaseBytes.Length)
            $probeReleaseStream.Flush($true)
        }
        finally {
            $probeReleaseStream.Dispose()
        }
        if (
            -not $owned.ChildProcess.WaitForExit(10000) -or
            -not $owned.Process.WaitForExit(5000)
        ) {
            throw 'TASK037_NODE_IDENTITY_REJECTED'
        }
        $exitCode = [int]$owned.ChildProcess.ExitCode
        Stop-Task037Job -Job $job
        Complete-Task037GatedProcessEvidence -OwnedProcess $owned
        $stdout = [IO.File]::ReadAllText((Join-Path $probeRoot 'node.stdout'), [Text.Encoding]::UTF8).Trim()
        $stderr = [IO.File]::ReadAllText((Join-Path $probeRoot 'node.stderr'), [Text.Encoding]::UTF8)
        if ($exitCode -ne 0 -or $stdout -cne $officialNodeVersion -or -not [string]::IsNullOrEmpty($stderr)) {
            throw 'TASK037_NODE_IDENTITY_REJECTED'
        }
    }
    finally {
        if ($job -ne [IntPtr]::Zero) {
            try { Stop-Task037Job -Job $job } catch { }
            try { Close-Task037Job -Job $job } catch { }
        }
        if ($null -ne $owned) {
            try { $owned.ChildProcess.Dispose() } catch { }
            try { $owned.Process.Dispose() } catch { }
        }
        if (Test-Path -LiteralPath $probeRoot) {
            Remove-SafeTempRoot `
                -Path $probeRoot `
                -AuthorizedTempBase $nodeProbeTempBase | Out-Null
        }
    }
    return $canonical
}

function Get-TrustedTask037Git {
    param([Parameter(Mandatory = $true)][string]$Candidate)

    $canonical = Get-CanonicalPath -Path $Candidate
    $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    $expected = Get-CanonicalPath -Path (Join-Path $programFiles 'Git\cmd\git.exe')
    Assert-RegularFile -Path $canonical
    if (
        -not (Test-ExactPath -Actual $canonical -Expected $expected) -or
        (Get-FileSha256 $canonical) -cne $officialGitSha256
    ) {
        throw 'TASK037_GIT_IDENTITY_REJECTED'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $canonical
    if (
        [string]$signature.Status -ne 'Valid' -or
        $null -eq $signature.SignerCertificate -or
        [string]$signature.SignerCertificate.Subject -notlike 'CN=Johannes Schindelin,*O=Johannes Schindelin*'
    ) {
        throw 'TASK037_GIT_IDENTITY_REJECTED'
    }
    return $canonical
}

function Assert-Task037CargoHomeConfigurationAbsent {
    param([Parameter(Mandatory = $true)][string]$CargoHome)

    $canonical = Get-CanonicalPath -Path $CargoHome
    Assert-NoReparsePath -Path $canonical -FailureCode 'TASK037_CARGO_CONFIGURATION_REJECTED'
    $item = Get-Item -LiteralPath $canonical -Force -ErrorAction SilentlyContinue
    if ($null -eq $item -or -not $item.PSIsContainer) {
        throw 'TASK037_CARGO_CONFIGURATION_REJECTED'
    }
    foreach ($name in @('config', 'config.toml', 'credentials', 'credentials.toml')) {
        if (Test-Path -LiteralPath (Join-Path $canonical $name)) {
            throw 'TASK037_CARGO_CONFIGURATION_REJECTED'
        }
    }
}

function Assert-Task037CargoConfigurationClosure {
    param(
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$CargoHome
    )

    Assert-Task037CargoHomeConfigurationAbsent -CargoHome $CargoHome
    $current = Get-CanonicalPath -Path $RepositoryRoot
    Assert-NoReparsePath -Path $current -FailureCode 'TASK037_CARGO_CONFIGURATION_REJECTED'
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        $cargoDirectory = Join-Path $current '.cargo'
        foreach ($name in @('config', 'config.toml')) {
            if (Test-Path -LiteralPath (Join-Path $cargoDirectory $name)) {
                throw 'TASK037_CARGO_CONFIGURATION_REJECTED'
            }
        }
        $parentInfo = [IO.Directory]::GetParent($current)
        if ($null -eq $parentInfo) { break }
        $parent = $parentInfo.FullName
        if (Test-ExactPath -Actual $parent -Expected $current) { break }
        $current = $parent
    }
}

function Get-TrustedTask037RustToolchain {
    param(
        [Parameter(Mandatory = $true)][string]$CargoCandidate,
        [Parameter(Mandatory = $true)][string]$RustcCandidate,
        [Parameter(Mandatory = $true)][string]$RustdocCandidate,
        [Parameter(Mandatory = $true)][string]$CargoHome
    )

    $userProfile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    $expectedCargoHome = Get-CanonicalPath -Path (Join-Path $userProfile '.cargo')
    $rustupHome = Get-CanonicalPath -Path (Join-Path $userProfile '.rustup')
    $toolchainBin = Get-CanonicalPath -Path (
        Join-Path $rustupHome ('toolchains\' + $task037RustToolchain + '\bin')
    )
    $expectedCargo = Join-Path $toolchainBin 'cargo.exe'
    $expectedRustc = Join-Path $toolchainBin 'rustc.exe'
    $expectedRustdoc = Join-Path $toolchainBin 'rustdoc.exe'
    $cargo = Get-CanonicalPath -Path $CargoCandidate
    $rustc = Get-CanonicalPath -Path $RustcCandidate
    $rustdoc = Get-CanonicalPath -Path $RustdocCandidate
    foreach ($path in @($cargo, $rustc, $rustdoc)) {
        Assert-NoReparsePath -Path $path -FailureCode 'TASK037_RUST_TOOLCHAIN_IDENTITY_REJECTED'
        Assert-RegularFile -Path $path
    }
    if (
        -not (Test-ExactPath -Actual $cargo -Expected $expectedCargo) -or
        -not (Test-ExactPath -Actual $rustc -Expected $expectedRustc) -or
        -not (Test-ExactPath -Actual $rustdoc -Expected $expectedRustdoc) -or
        (Get-FileSha256 $cargo) -cne $officialCargoSha256 -or
        (Get-FileSha256 $rustc) -cne $officialRustcSha256 -or
        (Get-FileSha256 $rustdoc) -cne $officialRustdocSha256
    ) {
        throw 'TASK037_RUST_TOOLCHAIN_IDENTITY_REJECTED'
    }
    if (-not (Test-ExactPath -Actual $CargoHome -Expected $expectedCargoHome)) {
        throw 'TASK037_CARGO_CONFIGURATION_REJECTED'
    }
    Assert-Task037CargoHomeConfigurationAbsent -CargoHome $expectedCargoHome
    return [pscustomobject]@{
        Cargo = $cargo
        Rustc = $rustc
        Rustdoc = $rustdoc
        CargoHome = $expectedCargoHome
        RustupHome = $rustupHome
        ToolchainBin = $toolchainBin
    }
}

function Assert-Task037RustToolchainVersion {
    param([Parameter(Mandatory = $true)]$Toolchain)

    $cargoOutput = @(& $Toolchain.Cargo '--version' 2>&1)
    if ($LASTEXITCODE -ne 0 -or $cargoOutput.Count -ne 1 -or [string]$cargoOutput[0] -cne $officialCargoVersion) {
        throw 'TASK037_RUST_TOOLCHAIN_VERSION_REJECTED'
    }
    $rustcOutput = @(& $Toolchain.Rustc '--version' 2>&1)
    if ($LASTEXITCODE -ne 0 -or $rustcOutput.Count -ne 1 -or [string]$rustcOutput[0] -cne $officialRustcVersion) {
        throw 'TASK037_RUST_TOOLCHAIN_VERSION_REJECTED'
    }
}

function Set-Task037ClosedChildEnvironment {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.ProcessStartInfo]$StartInfo,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$EnvironmentValues,
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string]$IsolationRoot,
        [string]$TrustedProgramData
    )

    $windows = Get-CanonicalPath -Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Windows))
    $system32 = Get-CanonicalPath -Path (Join-Path $windows 'System32')
    $isolation = Get-CanonicalPath -Path $IsolationRoot
    Assert-NoReparsePath -Path (Split-Path -Parent $isolation) -FailureCode 'TASK037_CHILD_ENVIRONMENT_SCOPE_REJECTED'
    foreach ($directory in @(
        $isolation,
        (Join-Path $isolation 'home'),
        (Join-Path $isolation 'home\AppData'),
        (Join-Path $isolation 'home\AppData\Roaming'),
        (Join-Path $isolation 'home\AppData\Local'),
        (Join-Path $isolation 'temp')
    )) {
        if (-not (Test-Path -LiteralPath $directory)) {
            New-Item -ItemType Directory -Path $directory -Force:$false | Out-Null
        }
        $item = Get-Item -LiteralPath $directory -Force
        if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'TASK037_CHILD_ENVIRONMENT_SCOPE_REJECTED'
        }
    }
    $userProfile = Join-Path $isolation 'home'
    $temp = Join-Path $isolation 'temp'
    $appData = Join-Path $userProfile 'AppData\Roaming'
    $localAppData = Join-Path $userProfile 'AppData\Local'
    $programDataValue = $TrustedProgramData
    if ([string]::IsNullOrWhiteSpace($programDataValue)) {
        $programDataValue = [Environment]::GetFolderPath([Environment+SpecialFolder]::CommonApplicationData)
    }
    $programData = Get-CanonicalPath -Path $programDataValue
    Assert-NoReparsePath -Path $programData -FailureCode 'TASK037_CHILD_ENVIRONMENT_SCOPE_REJECTED'
    $programDataItem = Get-Item -LiteralPath $programData -Force -ErrorAction SilentlyContinue
    if ($null -eq $programDataItem -or -not $programDataItem.PSIsContainer) {
        throw 'TASK037_CHILD_ENVIRONMENT_SCOPE_REJECTED'
    }
    $programFiles = Get-CanonicalPath -Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles))
    $programFilesX86 = Get-CanonicalPath -Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFilesX86))
    $executableDirectory = Get-CanonicalPath -Path (Split-Path -Parent $Executable)
    $powerShellDirectory = Get-CanonicalPath -Path $PSHOME
    $powerShellModulePath = Get-CanonicalPath -Path (Join-Path $powerShellDirectory 'Modules')
    $pathValue = [string]::Join(';', (@(
        $executableDirectory,
        $system32,
        $windows,
        (Join-Path $system32 'Wbem'),
        $powerShellDirectory
    ) | Select-Object -Unique))
    $baseline = [ordered]@{
        SystemRoot = $windows
        WINDIR = $windows
        ComSpec = (Join-Path $system32 'cmd.exe')
        PATH = $pathValue
        PATHEXT = '.COM;.EXE;.BAT;.CMD'
        PSModulePath = $powerShellModulePath
        PSModuleAnalysisCachePath = 'NUL'
        OS = 'Windows_NT'
        TEMP = $temp
        TMP = $temp
        USERPROFILE = $userProfile
        HOME = $userProfile
        APPDATA = $appData
        LOCALAPPDATA = $localAppData
        ALLUSERSPROFILE = $programData
        ProgramData = $programData
        ProgramFiles = $programFiles
        'ProgramFiles(x86)' = $programFilesX86
        NUMBER_OF_PROCESSORS = [Environment]::ProcessorCount.ToString([Globalization.CultureInfo]::InvariantCulture)
        PROCESSOR_ARCHITECTURE = [Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITECTURE', 'Process')
    }
    $StartInfo.EnvironmentVariables.Clear()
    foreach ($entry in $baseline.GetEnumerator()) {
        if (-not [string]::IsNullOrWhiteSpace([string]$entry.Value)) {
            $StartInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
        }
    }
    foreach ($entry in $EnvironmentValues.GetEnumerator()) {
        if (
            [string]$entry.Key -notmatch '^[A-Z][A-Z0-9_]{0,127}$' -or
            $null -eq $entry.Value -or
            [string]$entry.Key -in @(
                'PROGRAMDATA',
                'ALLUSERSPROFILE',
                'PSMODULEANALYSISCACHEPATH',
                'USERPROFILE',
                'HOME',
                'APPDATA',
                'LOCALAPPDATA',
                'TEMP',
                'TMP'
            )
        ) {
            throw 'TASK037_CHILD_ENVIRONMENT_VALUE_REJECTED'
        }
        $StartInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }
    $StartInfo.EnvironmentVariables['NO_COLOR'] = '1'
}

function Start-Task037GatedProcess {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [IntPtr]$AttestationJob = [IntPtr]::Zero,
        [Parameter(Mandatory = $true)][string]$PowerShellExecutable,
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$EnvironmentValues,
        [Parameter(Mandatory = $true)][string]$ChildPidPath,
        [Parameter(Mandatory = $true)][string]$ChildStdoutPath,
        [Parameter(Mandatory = $true)][string]$ChildStderrPath,
        [Parameter(Mandatory = $true)][string]$WrapperStdoutPath,
        [Parameter(Mandatory = $true)][string]$WrapperStderrPath,
        [Parameter(Mandatory = $true)][string]$RuntimeIsolationRoot,
        [string]$TrustedProgramData,
        [string]$StandardInputPath,
        [scriptblock]$BeforeGate
    )

    foreach ($path in @($PowerShellExecutable, $Executable)) {
        Assert-RegularFile -Path $path
    }
    $workingItem = Get-Item -LiteralPath $WorkingDirectory -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $workingItem -or
        -not $workingItem.PSIsContainer -or
        ($workingItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK037_GATED_PROCESS_WORKING_DIRECTORY_REJECTED'
    }
    if (-not [string]::IsNullOrEmpty($StandardInputPath)) {
        Assert-RegularFile -Path $StandardInputPath
    }
    foreach ($freshPath in @(
        $ChildPidPath,
        $ChildStdoutPath,
        $ChildStderrPath,
        $WrapperStdoutPath,
        $WrapperStderrPath
    )) {
        if (Test-Path -LiteralPath $freshPath) {
            throw 'TASK037_GATED_PROCESS_EVIDENCE_NOT_FRESH'
        }
    }

    $gateName = 'LatticeTask037-' + [Guid]::NewGuid().ToString('N')
    $payloadEnvironmentStartInfo = [Diagnostics.ProcessStartInfo]::new()
    Set-Task037ClosedChildEnvironment `
        -StartInfo $payloadEnvironmentStartInfo `
        -EnvironmentValues $EnvironmentValues `
        -Executable $Executable `
        -IsolationRoot $RuntimeIsolationRoot `
        -TrustedProgramData $TrustedProgramData
    $payloadEnvironment = [ordered]@{}
    foreach ($name in @($payloadEnvironmentStartInfo.EnvironmentVariables.Keys | Sort-Object)) {
        $payloadEnvironment[[string]$name] = [string]$payloadEnvironmentStartInfo.EnvironmentVariables[[string]$name]
    }
    $payload = [ordered]@{
        schema = 'lattice.task037.gated-process.v1'
        executable = Get-CanonicalPath -Path $Executable
        working_directory = Get-CanonicalPath -Path $WorkingDirectory
        arguments = @($Arguments)
        pid_path = Get-CanonicalPath -Path $ChildPidPath
        stdin_path = if ([string]::IsNullOrEmpty($StandardInputPath)) { '' } else { Get-CanonicalPath -Path $StandardInputPath }
        stdout_path = Get-CanonicalPath -Path $ChildStdoutPath
        stderr_path = Get-CanonicalPath -Path $ChildStderrPath
        environment = $payloadEnvironment
    }
    $payloadJson = $payload | ConvertTo-Json -Compress -Depth 8
    $payloadBase64 = [Convert]::ToBase64String([Text.UTF8Encoding]::new($false).GetBytes($payloadJson))
    if (
        $payloadBase64 -notmatch '^[A-Za-z0-9+/]+={0,2}$' -or
        ($payloadBase64.Length % 4) -ne 0
    ) {
        throw 'TASK037_GATED_PROCESS_PAYLOAD_REJECTED'
    }
    $wrapperSource = @'
$ErrorActionPreference = 'Stop'
$gate = $null
$child = $null
$stage = 'PAYLOAD'
try {
    $gateName = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_JOB_GATE', 'Process')
    $gate = [Threading.EventWaitHandle]::OpenExisting($gateName)
    [Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
    $payloadBase64 = [Console]::In.ReadLine()
    if ($null -ne $payloadBase64) { $payloadBase64 = $payloadBase64.TrimStart([char]0xFEFF) }
    if ([string]::IsNullOrWhiteSpace($payloadBase64)) { exit 114 }
    $stage = 'GATE'
    if (-not $gate.WaitOne(30000)) { exit 111 }
    $stage = 'DECODE'
    if ($payloadBase64 -notmatch '^[A-Za-z0-9+/]+={0,2}$') {
        $invalidIndex = -1
        $invalidCode = 0
        for ($index = 0; $index -lt $payloadBase64.Length; $index++) {
            $character = $payloadBase64[$index]
            if ([string]$character -notmatch '^[A-Za-z0-9+/=]$') {
                $invalidIndex = $index
                $invalidCode = [int][char]$character
                break
            }
        }
        [Console]::Error.WriteLine(
            'TASK037_WRAPPER_PAYLOAD_ALPHABET_REJECTED_' +
            $invalidCode.ToString('X4') + '_' +
            $invalidIndex.ToString([Globalization.CultureInfo]::InvariantCulture) + '_' +
            ([Console]::InputEncoding.WebName -replace '[^A-Za-z0-9-]', '')
        )
        exit 116
    }
    if (($payloadBase64.Length % 4) -ne 0) {
        [Console]::Error.WriteLine('TASK037_WRAPPER_PAYLOAD_LENGTH_REJECTED')
        exit 117
    }
    $payloadJson = [Text.UTF8Encoding]::new($false).GetString([Convert]::FromBase64String($payloadBase64))
    $payload = $payloadJson | ConvertFrom-Json
    if ([string]$payload.schema -cne 'lattice.task037.gated-process.v1') { exit 115 }
    $stage = 'ENVIRONMENT'
    $payloadNames = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    foreach ($property in $payload.environment.PSObject.Properties) {
        $name = [string]$property.Name
        if (-not $payloadNames.Add($name)) { exit 118 }
        [Environment]::SetEnvironmentVariable($name, [string]$property.Value, 'Process')
    }
    foreach ($name in @([Environment]::GetEnvironmentVariables('Process').Keys)) {
        if (-not $payloadNames.Contains([string]$name)) {
            [Environment]::SetEnvironmentVariable([string]$name, $null, 'Process')
        }
    }
    $applicationData = [Environment]::GetFolderPath(
        [Environment+SpecialFolder]::ApplicationData
    )
    $localApplicationData = [Environment]::GetFolderPath(
        [Environment+SpecialFolder]::LocalApplicationData
    )
    if (
        [string]::IsNullOrWhiteSpace($applicationData) -or
        [string]::IsNullOrWhiteSpace($localApplicationData) -or
        -not [string]::Equals(
            [IO.Path]::GetFullPath($applicationData),
            [IO.Path]::GetFullPath([string]$payload.environment.APPDATA),
            [StringComparison]::OrdinalIgnoreCase
        ) -or
        -not [string]::Equals(
            [IO.Path]::GetFullPath($localApplicationData),
            [IO.Path]::GetFullPath([string]$payload.environment.LOCALAPPDATA),
            [StringComparison]::OrdinalIgnoreCase
        )
    ) {
        [Console]::Error.WriteLine('TASK037_CHILD_SPECIAL_FOLDER_REJECTED')
        exit 119
    }
    $arguments = @($payload.arguments | ForEach-Object { [string]$_ })
    $start = @{
        FilePath = [string]$payload.executable
        WorkingDirectory = [string]$payload.working_directory
        RedirectStandardOutput = [string]$payload.stdout_path
        RedirectStandardError = [string]$payload.stderr_path
        WindowStyle = 'Hidden'
        PassThru = $true
    }
    if ($arguments.Count -gt 0) {
        $start.ArgumentList = $arguments
    }
    if (-not [string]::IsNullOrEmpty([string]$payload.stdin_path)) {
        $start.RedirectStandardInput = [string]$payload.stdin_path
    }
    $stage = 'START'
    $child = Start-Process @start
    $stage = 'IDENTITY'
    $identity = [string]$child.Id + '|' + [string]$child.StartTime.ToUniversalTime().Ticks + "`n"
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($identity)
    $stream = [IO.File]::Open([string]$payload.pid_path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    }
    finally {
        $stream.Dispose()
    }
    $stage = 'WAIT'
    $child.WaitForExit()
    exit ([int]$child.ExitCode)
}
catch {
    $failureType = $_.Exception.GetType().Name
    $innerType = if ($null -eq $_.Exception.InnerException) { 'None' } else { $_.Exception.InnerException.GetType().Name }
    [Console]::Error.WriteLine('TASK037_WRAPPER_FAILED_' + $stage + '_' + $failureType + '_' + $innerType)
    exit 113
}
finally {
    if ($null -ne $child) { $child.Dispose() }
    if ($null -ne $gate) { $gate.Dispose() }
}
'@
    $encodedWrapper = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($wrapperSource))
    $launchEnvironment = [ordered]@{ LATTICE_TASK037_JOB_GATE = $gateName }

    $gate = $null
    $process = $null
    $childProcess = $null
    $started = $false
    $assigned = $false
    $wrapperStdoutTask = $null
    $wrapperStderrTask = $null
    try {
        $gateCreated = $false
        $gate = [Threading.EventWaitHandle]::new(
            $false,
            [Threading.EventResetMode]::ManualReset,
            $gateName,
            [ref]$gateCreated
        )
        if (-not $gateCreated) {
            throw 'TASK037_JOB_GATE_REJECTED'
        }
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $PowerShellExecutable
        $startInfo.Arguments = ('-NoLogo -NoProfile -NonInteractive -EncodedCommand ' + $encodedWrapper)
        $startInfo.WorkingDirectory = $WorkingDirectory
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.RedirectStandardInput = $true
        $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
        $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
        Set-Task037ClosedChildEnvironment `
            -StartInfo $startInfo `
            -EnvironmentValues $launchEnvironment `
            -Executable $PowerShellExecutable `
            -IsolationRoot $RuntimeIsolationRoot `
            -TrustedProgramData $TrustedProgramData
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        try {
            if (-not $process.Start()) {
                throw 'TASK037_GATED_PROCESS_START_REJECTED'
            }
            $started = $true
            $null = $process.Handle
            $wrapperStdoutTask = $process.StandardOutput.ReadToEndAsync()
            $wrapperStderrTask = $process.StandardError.ReadToEndAsync()
        }
        catch {
            throw 'TASK037_GATED_PROCESS_START_REJECTED'
        }
        Add-Task037ProcessToJob -Job $Job -Process $process
        $assigned = $true
        if ($AttestationJob -ne [IntPtr]::Zero) {
            Add-Task037ProcessToJob -Job $AttestationJob -Process $process
        }
        if ($null -ne $BeforeGate) {
            & $BeforeGate
        }
        $process.StandardInput.WriteLine($payloadBase64)
        $process.StandardInput.Close()
        if (-not $gate.Set()) {
            throw 'TASK037_JOB_GATE_REJECTED'
        }
        $stopwatch = [Diagnostics.Stopwatch]::StartNew()
        $childPidText = $null
        while ($stopwatch.ElapsedMilliseconds -lt 30000 -and $null -eq $childPidText) {
            if (Test-Path -LiteralPath $ChildPidPath -PathType Leaf) {
                $pidStream = $null
                $pidReader = $null
                try {
                    Assert-RegularFile -Path $ChildPidPath
                    $pidStream = [IO.File]::Open(
                        $ChildPidPath,
                        [IO.FileMode]::Open,
                        [IO.FileAccess]::Read,
                        [IO.FileShare]::Read
                    )
                    $pidReader = [IO.StreamReader]::new($pidStream, [Text.Encoding]::UTF8, $true)
                    $candidate = $pidReader.ReadToEnd().Trim()
                    if (-not [string]::IsNullOrEmpty($candidate)) {
                        $childPidText = $candidate
                    }
                }
                catch [IO.IOException] { }
                finally {
                    if ($null -ne $pidReader) { $pidReader.Dispose() }
                    elseif ($null -ne $pidStream) { $pidStream.Dispose() }
                }
            }
            if ($null -eq $childPidText -and $process.HasExited) {
                if ($null -ne $wrapperStderrTask -and $wrapperStderrTask.Wait(2000)) {
                    $wrapperFailure = ([string]$wrapperStderrTask.Result).Trim()
                    if (
                        $wrapperFailure -eq 'TASK037_WRAPPER_PAYLOAD_LENGTH_REJECTED' -or
                        $wrapperFailure -eq 'TASK037_CHILD_SPECIAL_FOLDER_REJECTED' -or
                        $wrapperFailure -match '^TASK037_WRAPPER_PAYLOAD_ALPHABET_REJECTED_[0-9A-F]{4}_[0-9]+_[A-Za-z0-9-]+$'
                    ) {
                        throw $wrapperFailure
                    }
                    if ($wrapperFailure -match '^TASK037_WRAPPER_FAILED_(GATE|PAYLOAD|DECODE|ENVIRONMENT|START|IDENTITY|WAIT)_[A-Za-z]+_[A-Za-z]+$') {
                        throw $wrapperFailure
                    }
                }
                throw 'TASK037_GATED_PROCESS_EARLY_EXIT'
            }
            if ($null -eq $childPidText) { Start-Sleep -Milliseconds 10 }
        }
        $identityParts = @($childPidText -split '\|')
        $childProcessId = 0
        [int64]$childProcessStartTicks = 0
        if (
            $null -eq $childPidText -or $identityParts.Count -ne 2 -or
            -not [int]::TryParse($identityParts[0], [ref]$childProcessId) -or
            -not [int64]::TryParse($identityParts[1], [ref]$childProcessStartTicks) -or
            $childProcessId -le 0
        ) {
            throw 'TASK037_GATED_PROCESS_PID_REJECTED'
        }
        $childProcess = [Diagnostics.Process]::GetProcessById($childProcessId)
        $null = $childProcess.Handle
        if ($childProcess.StartTime.ToUniversalTime().Ticks -ne $childProcessStartTicks) {
            throw 'TASK037_GATED_PROCESS_IDENTITY_REJECTED'
        }
        Assert-Task037ProcessInJob -Job $Job -Process $childProcess
        if ($AttestationJob -ne [IntPtr]::Zero) {
            Assert-Task037ProcessInJob -Job $AttestationJob -Process $childProcess
        }
        return [pscustomobject]@{
            Process = $process
            ChildProcess = $childProcess
            ChildProcessId = $childProcessId
            PidPath = $ChildPidPath
            WrapperStdoutPath = $WrapperStdoutPath
            WrapperStderrPath = $WrapperStderrPath
            WrapperStdoutTask = $wrapperStdoutTask
            WrapperStderrTask = $wrapperStderrTask
        }
    }
    catch {
        if ($assigned) {
            try { Stop-Task037Job -Job $Job } catch { }
        }
        elseif ($started -and $null -ne $process) {
            try {
                if (-not $process.HasExited) { $process.Kill() }
                $null = $process.WaitForExit(5000)
            }
            catch { }
        }
        try {
            if ($null -ne $wrapperStdoutTask -and $wrapperStdoutTask.Wait(5000)) {
                [IO.File]::WriteAllText($WrapperStdoutPath, [string]$wrapperStdoutTask.Result, [Text.UTF8Encoding]::new($false))
            }
            if ($null -ne $wrapperStderrTask -and $wrapperStderrTask.Wait(5000)) {
                [IO.File]::WriteAllText($WrapperStderrPath, [string]$wrapperStderrTask.Result, [Text.UTF8Encoding]::new($false))
            }
        }
        catch { }
        if ($null -ne $childProcess) { $childProcess.Dispose() }
        if ($null -ne $process) { $process.Dispose() }
        throw
    }
    finally {
        if ($null -ne $gate) { $gate.Dispose() }
    }
}

function Complete-Task037GatedProcessEvidence {
    param([Parameter(Mandatory = $true)]$OwnedProcess)

    foreach ($stream in @(
        [pscustomobject]@{
            Task = $OwnedProcess.WrapperStdoutTask
            Path = [string]$OwnedProcess.WrapperStdoutPath
        },
        [pscustomobject]@{
            Task = $OwnedProcess.WrapperStderrTask
            Path = [string]$OwnedProcess.WrapperStderrPath
        }
    )) {
        if ($null -eq $stream.Task -or -not $stream.Task.Wait(5000)) {
            throw 'TASK037_GATED_PROCESS_EVIDENCE_REJECTED'
        }
        $value = [string]$stream.Task.Result
        if ([Text.Encoding]::UTF8.GetByteCount($value) -gt 65536) {
            throw 'TASK037_GATED_PROCESS_EVIDENCE_REJECTED'
        }
        if (Test-Path -LiteralPath $stream.Path) {
            throw 'TASK037_GATED_PROCESS_EVIDENCE_NOT_FRESH'
        }
        [IO.File]::WriteAllText($stream.Path, $value, [Text.UTF8Encoding]::new($false))
    }
}

function Start-IsolatedOpenClaw {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Job,
        [Parameter(Mandatory = $true)][string]$PowerShellExecutable,
        [Parameter(Mandatory = $true)][string]$NodeExe,
        [Parameter(Mandatory = $true)][string]$OpenClawCli,
        [Parameter(Mandatory = $true)][string]$OpenClawWorkingDirectory,
        [Parameter(Mandatory = $true)][string]$ProfileRoot,
        [Parameter(Mandatory = $true)][string]$GatewayToken,
        [Parameter(Mandatory = $true)][string]$TransportKeyHex,
        [Parameter(Mandatory = $true)][string]$ProcessNonceHex,
        [Parameter(Mandatory = $true)][string]$LaunchRecordId,
        [Parameter(Mandatory = $true)][int]$WsPort,
        [Parameter(Mandatory = $true)][int]$RustPort,
        [Parameter(Mandatory = $true)][string]$ChildPidPath,
        [Parameter(Mandatory = $true)][string]$StdoutPath,
        [Parameter(Mandatory = $true)][string]$StderrPath,
        [Parameter(Mandatory = $true)][string]$WrapperStdoutPath,
        [Parameter(Mandatory = $true)][string]$WrapperStderrPath,
        [Parameter(Mandatory = $true)][string]$RuntimeIsolationRoot,
        [Parameter(Mandatory = $true)][string]$TrustedProgramData
    )

    $environment = @{
        OPENCLAW_CONFIG_PATH = (Join-Path $ProfileRoot 'openclaw.json')
        OPENCLAW_STATE_DIR = $ProfileRoot
        OPENCLAW_GATEWAY_TOKEN = $GatewayToken
        OPENCLAW_DISABLE_BONJOUR = '1'
        OPENCLAW_SKIP_GMAIL_WATCHER = '1'
        CI = '1'
        NO_COLOR = '1'
        LATTICE_OPENCLAW_AUTH_KEY_HEX = $TransportKeyHex
        LATTICE_OPENCLAW_DEADLINE_MS = '10000'
        LATTICE_OPENCLAW_GATEWAY_PORT = [string]$RustPort
        LATTICE_OPENCLAW_LAUNCH_RECORD_ID = $LaunchRecordId
        LATTICE_OPENCLAW_PROCESS_START_NONCE = $ProcessNonceHex
    }
    return Start-Task037GatedProcess `
        -Job $Job `
        -PowerShellExecutable $PowerShellExecutable `
        -Executable $NodeExe `
        -Arguments @(
            $OpenClawCli,
            'gateway', 'run',
            '--bind', 'loopback',
            '--auth', 'token',
            '--port', [string]$WsPort,
            '--ws-log', 'compact'
        ) `
        -WorkingDirectory $OpenClawWorkingDirectory `
        -EnvironmentValues $environment `
        -ChildPidPath $ChildPidPath `
        -ChildStdoutPath $StdoutPath `
        -ChildStderrPath $StderrPath `
        -WrapperStdoutPath $WrapperStdoutPath `
        -WrapperStderrPath $WrapperStderrPath `
        -RuntimeIsolationRoot $RuntimeIsolationRoot `
        -TrustedProgramData $TrustedProgramData
}

function Get-McpInputBytes {
    param([Parameter(Mandatory = $true)][string]$ToolName)

    $frames = @(
        [ordered]@{
            jsonrpc = '2.0'
            id = 1
            method = 'initialize'
            params = [ordered]@{
                protocolVersion = '2025-11-25'
                capabilities = [ordered]@{}
                clientInfo = [ordered]@{ name = 'task037-full-chain-verifier'; version = '1' }
            }
        },
        [ordered]@{
            jsonrpc = '2.0'
            method = 'notifications/initialized'
            params = [ordered]@{}
        },
        [ordered]@{
            jsonrpc = '2.0'
            id = 2
            method = 'tools/call'
            params = [ordered]@{
                name = $ToolName
                arguments = $taskBinding
            }
        }
    )
    $text = (($frames | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 12 }) -join [Environment]::NewLine) + [Environment]::NewLine
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($text)
    if ($bytes.Length -eq 0 -or $bytes[0] -ne 0x7b) {
        throw 'TASK037_MCP_INPUT_ENCODING_REJECTED'
    }
    return $bytes
}

function ConvertFrom-Task037McpJson {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Json)

    try {
        if ([string]::IsNullOrWhiteSpace($Json)) {
            throw 'TASK037_MCP_RESPONSE_JSON_REJECTED'
        }
        return $Json | ConvertFrom-Json
    }
    catch {
        throw 'TASK037_MCP_RESPONSE_JSON_REJECTED'
    }
}

function Get-StructuredToolContent {
    param([Parameter(Mandatory = $true)]$Response)

    $result = $Response.result
    if ($result -isnot [pscustomobject]) {
        throw 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
    }
    Assert-Task037ExactPropertySet `
        -Value $result `
        -Expected @('content', 'structuredContent', 'isError') `
        -FailureCode 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
    if ($result.content -isnot [object[]]) {
        throw 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
    }
    $content = $result.content
    if (
        $content.Count -ne 1 -or
        $content[0] -isnot [pscustomobject] -or
        $content[0].type -isnot [string] -or
        [string]$content[0].type -cne 'text' -or
        $content[0].text -isnot [string] -or
        $result.structuredContent -isnot [pscustomobject] -or
        $result.isError -isnot [bool]
    ) {
        throw 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
    }
    Assert-Task037ExactPropertySet `
        -Value $content[0] `
        -Expected @('text', 'type') `
        -FailureCode 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
    $textValue = ConvertFrom-Task037McpJson -Json $content[0].text
    if ($textValue -isnot [pscustomobject]) {
        throw 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
    }
    if (
        ($textValue | ConvertTo-Json -Compress -Depth 20) -cne
        ($result.structuredContent | ConvertTo-Json -Compress -Depth 20)
    ) {
        throw 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
    }
    return $result.structuredContent
}

function Invoke-PostgresMemoryProbe {
    param(
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$HostName,
        [Parameter(Mandatory = $true)][string]$Port,
        [Parameter(Mandatory = $true)][string]$OutputPath
    )

    $psql = Join-Path $postgresBin 'psql.exe'
    Assert-RegularFile -Path $psql
    $database = 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
    $query = @"
SELECT
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_analyses) AS analyses,
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_receipts) AS receipts,
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_retrieval_audits) AS retrieval_audits,
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_records) AS records,
  (SELECT count(*)::int FROM ONLY memory.codebase_memory_reflections) AS reflections,
  (SELECT count(*)::int FROM ONLY memory.openclaw_gateway_commands) AS openclaw_commands,
  COALESCE((SELECT encode(reflection_receipt_digest, 'hex') FROM ONLY memory.codebase_memory_reflections ORDER BY reflection_receipt_digest LIMIT 1), '') AS reflection_receipt_digest,
  COALESCE((SELECT encode(graph_receipt_digest, 'hex') FROM ONLY memory.codebase_memory_reflections ORDER BY reflection_receipt_digest LIMIT 1), '') AS graph_receipt_digest,
  COALESCE((SELECT reflection_status FROM ONLY memory.codebase_memory_reflections ORDER BY reflection_receipt_digest LIMIT 1), '') AS reflection_status;
"@
    $originalPassword = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $output = @(& $psql '--no-psqlrc' '--quiet' '--csv' '-h' $HostName '-p' $Port '-U' 'task019_harness' '-d' $database '-v' 'ON_ERROR_STOP=1' '-c' $query 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $originalPassword, 'Process')
    }
    if ($exitCode -ne 0) {
        Write-JsonEvidence -Path ($OutputPath + '.failure.json') -Value ([ordered]@{
            schema_version = 'lattice.task037.command-failure.v1'
            command = 'psql-memory-probe'
            code = 'TASK037_MEMORY_PROBE_FAILED'
            output_sha256 = Get-StringSha256 (($output | Out-String))
            output_line_count = @($output).Count
        })
        throw 'TASK037_MEMORY_PROBE_FAILED'
    }
    $rows = @($output | ConvertFrom-Csv)
    if ($rows.Count -ne 1) {
        throw 'TASK037_MEMORY_PROBE_SHAPE_REJECTED'
    }
    $probe = [ordered]@{
        schema_version = 'lattice.task037.memory-probe.v1'
        postgres_run_id = $RunId
        database_name = $database
        analyses = [int]$rows[0].analyses
        receipts = [int]$rows[0].receipts
        retrieval_audits = [int]$rows[0].retrieval_audits
        records = [int]$rows[0].records
        reflections = [int]$rows[0].reflections
        openclaw_commands = [int]$rows[0].openclaw_commands
        reflection_receipt_digest = [string]$rows[0].reflection_receipt_digest
        graph_receipt_digest = [string]$rows[0].graph_receipt_digest
        reflection_status = [string]$rows[0].reflection_status
    }
    Write-JsonEvidence -Path $OutputPath -Value $probe
    return $probe
}

function Reset-PostgresMemoryRows {
    param(
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$Password,
        [Parameter(Mandatory = $true)][string]$HostName,
        [Parameter(Mandatory = $true)][string]$Port,
        [Parameter(Mandatory = $true)][string]$EvidenceRoot
    )

    $before = Invoke-PostgresMemoryProbe `
        -RunId $RunId `
        -Password $Password `
        -HostName $HostName `
        -Port $Port `
        -OutputPath (Join-Path $EvidenceRoot 'memory-reset-before.json')
    $psql = Join-Path $postgresBin 'psql.exe'
    Assert-RegularFile -Path $psql
    $database = 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
    $query = @"
DELETE FROM ONLY memory.codebase_memory_reflections;
DELETE FROM ONLY memory.codebase_memory_receipts;
DELETE FROM ONLY memory.codebase_memory_retrieval_audits;
DELETE FROM ONLY memory.codebase_memory_records;
DELETE FROM ONLY memory.codebase_memory_analyses;
DELETE FROM ONLY memory.openclaw_gateway_commands;
"@
    $originalPassword = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $output = @(& $psql '--no-psqlrc' '--quiet' '-h' $HostName '-p' $Port '-U' 'task019_harness' '-d' $database '-v' 'ON_ERROR_STOP=1' '-c' $query 2>&1)
        $exitCode = $LASTEXITCODE
    }
    finally {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $originalPassword, 'Process')
    }
    if ($exitCode -ne 0) {
        Write-JsonEvidence -Path (Join-Path $EvidenceRoot 'memory-reset.failure.json') -Value ([ordered]@{
            schema_version = 'lattice.task037.command-failure.v1'
            command = 'psql-memory-reset'
            code = 'TASK037_MEMORY_RESET_FAILED'
            output_sha256 = Get-StringSha256 (($output | Out-String))
            output_line_count = @($output).Count
        })
        throw 'TASK037_MEMORY_RESET_FAILED'
    }
    $after = Invoke-PostgresMemoryProbe `
        -RunId $RunId `
        -Password $Password `
        -HostName $HostName `
        -Port $Port `
        -OutputPath (Join-Path $EvidenceRoot 'memory-reset-after.json')
    if (
        $after.analyses -ne 0 -or
        $after.receipts -ne 0 -or
        $after.retrieval_audits -ne 0 -or
        $after.records -ne 0 -or
        $after.reflections -ne 0 -or
        $after.openclaw_commands -ne 0
    ) {
        throw 'TASK037_MEMORY_RESET_REJECTED'
    }
    Write-JsonEvidence -Path (Join-Path $EvidenceRoot 'memory-reset.json') -Value ([ordered]@{
        schema_version = 'lattice.task037.memory-reset.v1'
        postgres_run_id = $RunId
        database_name = $database
        reset_scope = @(
            'memory.codebase_memory_reflections',
            'memory.codebase_memory_receipts',
            'memory.codebase_memory_retrieval_audits',
            'memory.codebase_memory_records',
            'memory.codebase_memory_analyses',
            'memory.openclaw_gateway_commands'
        )
        before = $before
        after = $after
    })
}

function Assert-InternalEnvironment {
    $acceptanceId = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_ACCEPTANCE_ID', 'Process')
    $runId = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_RUN_ID', 'Process')
    $hostName = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_HOST', 'Process')
    $port = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PORT', 'Process')
    $password = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PASSWORD', 'Process')
    if (
        [string]::IsNullOrWhiteSpace($acceptanceId) -or
        $acceptanceId -notmatch '^[0-9a-f]{32}$' -or
        $runId -notmatch '^[0-9a-f]{32}$' -or
        $hostName -ne '127.0.0.1' -or
        $port -notmatch '^[0-9]{1,5}$' -or
        [int]$port -eq 0 -or
        [int]$port -eq 5432 -or
        [string]::IsNullOrWhiteSpace($password) -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_LIVE', 'Process') -ne '1' -or
        [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PHASE', 'Process') -ne 'restart'
    ) {
        throw 'TASK037_INTERNAL_ENVIRONMENT_REJECTED'
    }
}

function Assert-Task037MemoryReplayUnchanged {
    param(
        [Parameter(Mandatory = $true)]$RunProbe,
        [Parameter(Mandatory = $true)]$StatusProbe
    )

    if (
        [int]$StatusProbe.analyses -ne [int]$RunProbe.analyses -or
        [int]$StatusProbe.receipts -ne [int]$RunProbe.receipts -or
        [int]$StatusProbe.retrieval_audits -ne [int]$RunProbe.retrieval_audits -or
        [int]$StatusProbe.records -ne [int]$RunProbe.records -or
        [int]$StatusProbe.reflections -ne [int]$RunProbe.reflections -or
        [int]$StatusProbe.openclaw_commands -ne [int]$RunProbe.openclaw_commands -or
        [string]$StatusProbe.reflection_receipt_digest -cne [string]$RunProbe.reflection_receipt_digest
    ) {
        throw 'TASK037_STATUS_REPLAY_REJECTED'
    }
}

function Assert-Task037TrustedLauncherPath {
    param(
        [Parameter(Mandatory = $true)][string]$Actual,
        [Parameter(Mandatory = $true)][string]$Expected
    )

    try {
        if (-not (Test-ExactPath -Actual $Actual -Expected $Expected)) {
            throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        }
    }
    catch { throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED' }
}

function Assert-FullChainSuccess {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][string]$RunId,
        [Parameter(Mandatory = $true)][string]$DeliveryRoot,
        [Parameter(Mandatory = $true)][string]$DeliveryLauncher,
        [Parameter(Mandatory = $true)][ref]$ActiveGate,
        [Parameter(Mandatory = $true)][ref]$LastCompletedGate
    )

    Enter-Task037PhaseGate `
        -Gate 'success_launcher_binding' `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    if ($Evidence.launcher_path -isnot [string]) {
        throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
    }
    Assert-Task037TrustedLauncherPath -Actual $Evidence.launcher_path -Expected $DeliveryLauncher

    Enter-Task037PhaseGate `
        -Gate 'success_delivery_contract' `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    if ($Evidence.repository_path -isnot [string]) {
        throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
    }
    $repositoryPath = Get-CanonicalPath -Path $Evidence.repository_path
    $expectedRepositoryPath = Get-CanonicalPath -Path (Join-Path $DeliveryRoot 'repo')
    $expectedRequestId = 'task032-request-' + $RunId
    $deliveryStrings = [ordered]@{
        status = 'COMPLETED'
        profile = 'task032-codex-postgres-v1'
        entrypoint = 'codex-app-mcp'
        entrypoint_classification = 'official-codex-app-live'
        entrypoint_runtime_kind = 'Live'
        request_id = $expectedRequestId
        codex_runtime = 'OFFICIAL_CODEX_APP_SERVER'
        version = $officialCodexVersion
        launcher_sha256 = $officialCodexSha256
        test = 'FIXED_TEST_PASSED'
        test_command_id = 'git-diff-no-index-exact-answer-v1'
    }
    foreach ($name in @($deliveryStrings.Keys)) {
        $value = $Evidence.PSObject.Properties[$name].Value
        if ($value -isnot [string] -or [string]$value -cne [string]$deliveryStrings[$name]) {
            throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        }
    }
    if (
        $Evidence.component -isnot [string] -or
        @('lattice-delivery', 'delivery-ledger') -cnotcontains [string]$Evidence.component -or
        -not (Test-Task037JsonInteger -Value $Evidence.schema_file_count -Minimum 1 -Maximum ([int32]::MaxValue)) -or
        -not (Test-Task037JsonArray -Value $Evidence.changed_paths)
    ) {
        throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
    }
    $changedPaths = $Evidence.changed_paths
    if (
        $changedPaths.Count -ne 1 -or
        $changedPaths[0] -isnot [string] -or
        [string]$changedPaths[0] -cne 'answer.txt' -or
        -not (Test-ExactPath -Actual $repositoryPath -Expected $expectedRepositoryPath)
    ) {
        throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
    }

    Enter-Task037PhaseGate `
        -Gate 'success_graph_contract' `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    if (
        $Evidence.graph_project_id -isnot [string] -or
        [string]$Evidence.graph_project_id -cne 'task032-delivery' -or
        $Evidence.graph_status -isnot [string] -or
        [string]$Evidence.graph_status -cne 'COMPLETED' -or
        -not (Test-Task037JsonInteger -Value $Evidence.graph_record_count -Minimum 1 -Maximum ([int32]::MaxValue)) -or
        -not (Test-Task037JsonInteger -Value $Evidence.graph_result_count -Minimum 1 -Maximum ([int32]::MaxValue))
    ) {
        throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
    }

    Enter-Task037PhaseGate `
        -Gate 'success_hermes_contract' `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    $hermesStrings = [ordered]@{
        hermes_status = 'INFERENCE_CANDIDATE'
        hermes_schema_version = 'lattice.hermes.reflection.v1'
        hermes_provenance_status = 'PERSISTED_STRUCTURED_INFERENCE'
    }
    foreach ($name in @($hermesStrings.Keys)) {
        $value = $Evidence.PSObject.Properties[$name].Value
        if ($value -isnot [string] -or [string]$value -cne [string]$hermesStrings[$name]) {
            throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        }
    }
    if (
        $Evidence.thread_id -isnot [string] -or
        [string]::IsNullOrWhiteSpace($Evidence.thread_id) -or
        $Evidence.thread_id.Length -gt 256 -or
        $Evidence.turn_id -isnot [string] -or
        [string]::IsNullOrWhiteSpace($Evidence.turn_id) -or
        $Evidence.turn_id.Length -gt 256 -or
        $Evidence.hermes_summary -isnot [string] -or
        [string]::IsNullOrWhiteSpace($Evidence.hermes_summary) -or
        $Evidence.hermes_summary.Length -gt 8192 -or
        -not (Test-Task037JsonArray -Value $Evidence.hermes_findings) -or
        -not (Test-Task037JsonArray -Value $Evidence.hermes_next_actions)
    ) {
        throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
    }
    $findings = $Evidence.hermes_findings
    $nextActions = $Evidence.hermes_next_actions
    if (
        $findings.Count -lt 1 -or $findings.Count -gt 16 -or
        $nextActions.Count -lt 1 -or $nextActions.Count -gt 16
    ) {
        throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
    }

    Enter-Task037PhaseGate `
        -Gate 'success_digest_contract' `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    $sha256Fields = @(
        'configuration_digest', 'intent_digest', 'outcome_digest', 'receipt_digest',
        'schema_bundle_sha256', 'graph_query_digest', 'graph_analysis_digest',
        'graph_persistence_digest', 'graph_retrieval_digest',
        'graph_database_identity_digest', 'graph_extension_manifest_digest',
        'full_chain_receipt_digest', 'hermes_reflection_digest', 'hermes_identity_digest',
        'hermes_input_digest', 'hermes_graph_receipt_digest', 'graph_receipt_digest'
    )
    foreach ($name in $sha256Fields) {
        $value = $Evidence.PSObject.Properties[$name].Value
        if ($value -isnot [string] -or [string]$value -cnotmatch '^[0-9a-f]{64}$') {
            throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        }
    }
    foreach ($name in @('baseline_commit', 'parent_sha', 'commit_sha', 'graph_commit_sha')) {
        $value = $Evidence.PSObject.Properties[$name].Value
        if ($value -isnot [string] -or [string]$value -cnotmatch '^[0-9a-f]{40}$') {
            throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        }
    }

    Enter-Task037PhaseGate `
        -Gate 'success_finding_items' `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    foreach ($finding in $findings) {
        if ($finding -isnot [pscustomobject]) {
            throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        }
        Assert-Task037ExactPropertySet `
            -Value $finding `
            -Expected @('evidence_digest', 'statement') `
            -FailureCode 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        if (
            $finding.evidence_digest -isnot [string] -or
            [string]$finding.evidence_digest -cnotmatch '^[0-9a-f]{64}$' -or
            $finding.statement -isnot [string] -or
            [string]::IsNullOrWhiteSpace($finding.statement) -or
            $finding.statement.Length -gt 8192
        ) {
            throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        }
    }

    Enter-Task037PhaseGate `
        -Gate 'success_action_items' `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    foreach ($action in $nextActions) {
        if (
            $action -isnot [string] -or
            [string]::IsNullOrWhiteSpace($action) -or
            $action.Length -gt 8192
        ) {
            throw 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED'
        }
    }

    Enter-Task037PhaseGate `
        -Gate 'success_answer_bytes' `
        -ActiveGate $ActiveGate `
        -LastCompletedGate $LastCompletedGate
    $answerPath = Join-Path $repositoryPath 'answer.txt'
    Assert-RegularFile -Path $answerPath
    $expectedAnswer = [System.Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n")
    $answer = [System.IO.File]::ReadAllBytes($answerPath)
    if ([Convert]::ToBase64String($answer) -cne [Convert]::ToBase64String($expectedAnswer)) {
        throw 'TASK037_DELIVERY_ANSWER_BYTES_REJECTED'
    }
}

function New-Task037SafeToolEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][bool]$IsError
    )

    if ($IsError) {
        return [ordered]@{
            code = [string]$Evidence.code
            status = [string]$Evidence.status
        }
    }
    $safe = [ordered]@{}
    foreach ($property in @($Evidence.PSObject.Properties)) {
        switch -CaseSensitive ([string]$property.Name) {
            'hermes_summary' {
                $value = [string]$property.Value
                $safe.hermes_summary_sha256 = Get-StringSha256 $value
                $safe.hermes_summary_byte_count = [Text.Encoding]::UTF8.GetByteCount($value)
            }
            'hermes_findings' {
                $safe.hermes_findings = @($property.Value | ForEach-Object {
                    $statement = [string]$_.statement
                    [ordered]@{
                        evidence_digest = [string]$_.evidence_digest
                        statement_sha256 = Get-StringSha256 $statement
                        statement_byte_count = [Text.Encoding]::UTF8.GetByteCount($statement)
                    }
                })
            }
            'hermes_next_actions' {
                $safe.hermes_next_actions = @($property.Value | ForEach-Object {
                    $value = [string]$_
                    [ordered]@{
                        sha256 = Get-StringSha256 $value
                        byte_count = [Text.Encoding]::UTF8.GetByteCount($value)
                    }
                })
            }
            'thread_id' {
                $value = [string]$property.Value
                $safe.thread_id_sha256 = Get-StringSha256 $value
                $safe.thread_id_byte_count = [Text.Encoding]::UTF8.GetByteCount($value)
            }
            'turn_id' {
                $value = [string]$property.Value
                $safe.turn_id_sha256 = Get-StringSha256 $value
                $safe.turn_id_byte_count = [Text.Encoding]::UTF8.GetByteCount($value)
            }
            'launcher_path' {
                $value = [string]$property.Value
                $safe.launcher_path_sha256 = Get-StringSha256 $value
                $safe.launcher_path_byte_count = [Text.Encoding]::UTF8.GetByteCount($value)
            }
            default { $safe[[string]$property.Name] = $property.Value }
        }
    }
    return $safe
}

function Write-Task037ValidatedPhaseEvidence {
    param(
        [Parameter(Mandatory = $true)]$Evidence,
        [Parameter(Mandatory = $true)][bool]$IsError,
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [Parameter(Mandatory = $true)]$Meta,
        [Parameter(Mandatory = $true)][string]$MetaPath
    )

    Assert-Task037NoKnownSecretValue -Value $Evidence
    $safe = New-Task037SafeToolEvidence -Evidence $Evidence -IsError $IsError
    Write-JsonEvidence -Path $OutputPath -Value $safe
    $digest = Get-FileSha256 $OutputPath
    $Meta.response_sha256 = $digest
    Write-JsonEvidence -Path $MetaPath -Value $Meta
    return $digest
}

function Invoke-Task037HarnessSelfTest {
    $powerShellExecutable = Get-TrustedTask037PowerShell
    $tempBase = Get-Task037TrustedTempBase
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')

    $aclProbeRoot = Join-Path $tempBase ('lattice-task037-acl-selftest-' + [Guid]::NewGuid().ToString('N'))
    $unprotectedProbeRoot = Join-Path $tempBase ('lattice-task037-acl-negative-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $aclProbeRoot -Force:$false | Out-Null
        $null = Set-Task037RestrictedDirectoryAcl -Path $aclProbeRoot
        $aclProbeFile = Join-Path $aclProbeRoot 'probe.txt'
        [IO.File]::WriteAllText($aclProbeFile, 'acl-probe', [Text.UTF8Encoding]::new($false))
        Assert-Task037SingleLinkFile -Path $aclProbeFile
        $null = Get-Task037RestrictedAclEvidence -Path $aclProbeFile
        New-Item -ItemType Directory -Path $unprotectedProbeRoot -Force:$false | Out-Null
        $aclFailure = $null
        try { $null = Get-Task037RestrictedAclEvidence -Path $unprotectedProbeRoot -RequireProtected }
        catch { $aclFailure = $_.Exception.Message }
        if ($aclFailure -cne 'TASK037_RESTRICTED_ACL_REJECTED') {
            throw 'TASK037_SELF_TEST_RESTRICTED_ACL_REJECTED'
        }
    }
    finally {
        foreach ($path in @($aclProbeRoot, $unprotectedProbeRoot)) {
            if (Test-Path -LiteralPath $path) { try { Remove-Item -LiteralPath $path -Recurse -Force } catch { } }
        }
    }

    for ($iteration = 0; $iteration -lt 20; $iteration++) {
        $testId = [Guid]::NewGuid().ToString('N')
        $testRoot = Join-Path $tempBase ('lattice-task037-containment-selftest-' + $testId)
        $sourceHome = Join-Path $testRoot 'source'
        $credentialHome = Join-Path $testRoot 'execution-home'
        $credentialOwnerRecord = Join-Path $repositoryRoot ('target\task037-selftest-owner-' + $testId + '.json')
        $ownerToken = 'selftest-' + $testId
        $childPidPath = Join-Path $testRoot 'child.pid'
        $childStdoutPath = Join-Path $testRoot 'child.stdout'
        $childStderrPath = Join-Path $testRoot 'child.stderr'
        $wrapperStdoutPath = Join-Path $testRoot 'wrapper.stdout'
        $wrapperStderrPath = Join-Path $testRoot 'wrapper.stderr'
        $sentinelPath = Join-Path $testRoot 'escaped.sentinel'
        $readyName = 'LatticeTask037SelfTestReady-' + $testId
        $rootReleaseName = 'LatticeTask037SelfTestRoot-' + $testId
        $descendantTriggerName = 'LatticeTask037SelfTestTrigger-' + $testId
        $descendantAckName = 'LatticeTask037SelfTestAck-' + $testId
        $ready = $null
        $rootRelease = $null
        $descendantTrigger = $null
        $descendantAck = $null
        $owned = $null
        $credentialOwnership = $null
        $ambientCanaries = @(
            'CI_JOB_JWT',
            'DOCKER_AUTH_CONFIG',
            'HTTPS_PROXY',
            'KUBECONFIG',
            'NODE_OPTIONS'
        )
        $ambientOriginal = @{}
        $job = [IntPtr]::Zero
        try {
            New-Item -ItemType Directory -Path $sourceHome -Force:$false | Out-Null
            [IO.File]::WriteAllText(
                (Join-Path $sourceHome 'auth.json'),
                "{}`n",
                [Text.UTF8Encoding]::new($false)
            )
            $sourceBefore = Get-CodexCredentialSourceEvidence -SourceHome $sourceHome
            $credentialOwnership = Copy-CodexCredentialFiles `
                -SourceHome $sourceHome `
                -DestinationHome $credentialHome `
                -OwnerToken $ownerToken `
                -OwnerRecordPath $credentialOwnerRecord `
                -ForBroker
            $ownerMutationDenied = $false
            $ownerMutationProbe = $null
            try {
                $ownerMutationProbe = [IO.File]::Open(
                    $credentialOwnerRecord,
                    [IO.FileMode]::Open,
                    [IO.FileAccess]::Write,
                    [IO.FileShare]::None
                )
            }
            catch [IO.IOException] {
                $ownerMutationDenied = $true
            }
            finally {
                if ($null -ne $ownerMutationProbe) { $ownerMutationProbe.Dispose() }
            }
            if (-not $ownerMutationDenied) {
                throw 'TASK037_SELF_TEST_OWNER_RECORD_LOCK_REJECTED'
            }

            $created = $false
            $ready = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $readyName, [ref]$created)
            if (-not $created) { throw 'TASK037_SELF_TEST_EVENT_REJECTED' }
            $created = $false
            $rootRelease = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $rootReleaseName, [ref]$created)
            if (-not $created) { throw 'TASK037_SELF_TEST_EVENT_REJECTED' }
            $created = $false
            $descendantTrigger = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $descendantTriggerName, [ref]$created)
            if (-not $created) { throw 'TASK037_SELF_TEST_EVENT_REJECTED' }
            $created = $false
            $descendantAck = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $descendantAckName, [ref]$created)
            if (-not $created) { throw 'TASK037_SELF_TEST_EVENT_REJECTED' }

            $descendantSource = @'
$ErrorActionPreference = 'Stop'
$ready = [Threading.EventWaitHandle]::OpenExisting($env:TASK037_SELFTEST_READY)
$trigger = [Threading.EventWaitHandle]::OpenExisting($env:TASK037_SELFTEST_TRIGGER)
$ack = [Threading.EventWaitHandle]::OpenExisting($env:TASK037_SELFTEST_ACK)
$auth = [IO.File]::Open($env:TASK037_SELFTEST_AUTH, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
try {
    $null = $ready.Set()
    if ($trigger.WaitOne(30000)) {
        [IO.File]::WriteAllText($env:TASK037_SELFTEST_SENTINEL, 'escaped')
        $null = $ack.Set()
    }
}
finally {
    $auth.Dispose()
    $ready.Dispose()
    $trigger.Dispose()
    $ack.Dispose()
}
'@
            $descendantEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($descendantSource))
            $rootSource = @'
$ErrorActionPreference = 'Stop'
$forbiddenAmbient = @('CI_JOB_JWT', 'DOCKER_AUTH_CONFIG', 'HTTPS_PROXY', 'KUBECONFIG', 'NODE_OPTIONS')
foreach ($name in $forbiddenAmbient) {
    if ($null -ne [Environment]::GetEnvironmentVariable($name, 'Process')) { exit 122 }
}
$release = [Threading.EventWaitHandle]::OpenExisting($env:TASK037_SELFTEST_ROOT_RELEASE)
try {
    $child = Start-Process `
        -FilePath $env:TASK037_SELFTEST_POWERSHELL `
        -ArgumentList @('-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', $env:TASK037_SELFTEST_DESCENDANT_B64) `
        -WindowStyle Hidden `
        -PassThru
    $child.Dispose()
    if (-not $release.WaitOne(30000)) { exit 121 }
}
finally {
    $release.Dispose()
}
'@
            $rootEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($rootSource))
            $environment = [ordered]@{
                TASK037_SELFTEST_POWERSHELL = $powerShellExecutable
                TASK037_SELFTEST_DESCENDANT_B64 = $descendantEncoded
                TASK037_SELFTEST_READY = $readyName
                TASK037_SELFTEST_ROOT_RELEASE = $rootReleaseName
                TASK037_SELFTEST_TRIGGER = $descendantTriggerName
                TASK037_SELFTEST_ACK = $descendantAckName
                TASK037_SELFTEST_AUTH = (Join-Path $credentialHome 'auth.json')
                TASK037_SELFTEST_SENTINEL = $sentinelPath
            }
            $job = New-Task037KillOnCloseJob
            foreach ($name in $ambientCanaries) {
                $ambientOriginal[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
                [Environment]::SetEnvironmentVariable($name, 'task037-secret-canary', 'Process')
            }
            $beforeGate = {
                if (
                    (Test-Path -LiteralPath $childPidPath) -or
                    (Test-Path -LiteralPath $sentinelPath) -or
                    $ready.WaitOne(0)
                ) {
                    throw 'TASK037_SELF_TEST_PRE_ASSIGN_EFFECT_REJECTED'
                }
            }.GetNewClosure()
            $owned = Start-Task037GatedProcess `
                -Job $job `
                -PowerShellExecutable $powerShellExecutable `
                -Executable $powerShellExecutable `
                -Arguments @('-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', $rootEncoded) `
                -WorkingDirectory $testRoot `
                -EnvironmentValues $environment `
                -ChildPidPath $childPidPath `
                -ChildStdoutPath $childStdoutPath `
                -ChildStderrPath $childStderrPath `
                -WrapperStdoutPath $wrapperStdoutPath `
                -WrapperStderrPath $wrapperStderrPath `
                -RuntimeIsolationRoot (Join-Path $testRoot 'runtime') `
                -BeforeGate $beforeGate
            foreach ($name in $ambientCanaries) {
                [Environment]::SetEnvironmentVariable($name, $ambientOriginal[$name], 'Process')
            }
            if ($owned.Process.Id -eq $owned.ChildProcessId) {
                throw 'TASK037_SELF_TEST_CHILD_IDENTITY_REJECTED'
            }
            if (-not $ready.WaitOne(10000)) {
                throw 'TASK037_SELF_TEST_DESCENDANT_READY_REJECTED'
            }
            $lockProbe = $null
            try {
                $lockProbe = [IO.File]::Open(
                    (Join-Path $credentialHome 'auth.json'),
                    [IO.FileMode]::Open,
                    [IO.FileAccess]::Read,
                    [IO.FileShare]::None
                )
                throw 'TASK037_SELF_TEST_CREDENTIAL_LOCK_REJECTED'
            }
            catch [IO.IOException] { }
            finally {
                if ($null -ne $lockProbe) { $lockProbe.Dispose() }
            }
            $null = $rootRelease.Set()
            if (
                -not $owned.ChildProcess.WaitForExit(10000) -or
                -not $owned.Process.WaitForExit(10000) -or
                (Get-Task037JobActiveProcessCount -Job $job) -eq 0
            ) {
                throw 'TASK037_SELF_TEST_DESCENDANT_OWNERSHIP_REJECTED'
            }
            Stop-Task037Job -Job $job
            if ((Get-Task037JobActiveProcessCount -Job $job) -ne 0) {
                throw 'TASK037_SELF_TEST_JOB_ZERO_REJECTED'
            }
            $null = $descendantTrigger.Set()
            if ($descendantAck.WaitOne(500) -or (Test-Path -LiteralPath $sentinelPath)) {
                throw 'TASK037_SELF_TEST_ESCAPED_EFFECT_REJECTED'
            }
            Complete-Task037GatedProcessEvidence -OwnedProcess $owned
            Assert-CodexCredentialSourceUnchanged -SourceHome $sourceHome -Expected $sourceBefore
            if (-not (Remove-SafeTempRoot -Path $credentialHome -CredentialHome -Ownership $credentialOwnership)) {
                throw 'TASK037_SELF_TEST_CREDENTIAL_CLEANUP_REJECTED'
            }
            $rawLogPath = Join-Path $testRoot 'synthetic.raw.log'
            $safeLogPath = Join-Path $testRoot 'synthetic.safe.log'
            [IO.File]::WriteAllText(
                $rawLogPath,
                'authorization: Bearer task037-synthetic-secret',
                [Text.UTF8Encoding]::new($false)
            )
            $logReceipt = Get-Task037RawArtifactReceipt `
                -SourcePath $rawLogPath `
                -ArtifactId 'selftest.raw'
            if (
                -not [bool]$logReceipt.exists -or
                [bool]$logReceipt.content_retained -or
                (Test-Path -LiteralPath $safeLogPath)
            ) {
                throw 'TASK037_SELF_TEST_RAW_TEXT_RETENTION_REJECTED'
            }
            $pendingProbe = Join-Path $testRoot 'final.pending.probe.json'
            $finalProbe = Join-Path $testRoot 'final.probe.json'
            [IO.File]::WriteAllText($pendingProbe, "{}`n", [Text.UTF8Encoding]::new($false))
            $pendingLock = [IO.File]::Open(
                $pendingProbe,
                [IO.FileMode]::Open,
                [IO.FileAccess]::Read,
                [IO.FileShare]::Read
            )
            $promotionFailure = $null
            try {
                Promote-Task037FinalEvidence -PendingPath $pendingProbe -FinalPath $finalProbe
            }
            catch {
                $promotionFailure = $_.Exception.Message
            }
            finally {
                $pendingLock.Dispose()
            }
            if (
                $promotionFailure -cne 'TASK037_FINAL_EVIDENCE_PROMOTION_REJECTED' -or
                (Test-Path -LiteralPath $finalProbe)
            ) {
                throw 'TASK037_SELF_TEST_FINAL_PROMOTION_REJECTED'
            }
            [IO.File]::Delete($pendingProbe)
            Close-Task037Job -Job $job
            $job = [IntPtr]::Zero
            $owned.ChildProcess.Dispose()
            $owned.Process.Dispose()
            $owned = $null
            foreach ($eventHandle in @($ready, $rootRelease, $descendantTrigger, $descendantAck)) {
                $eventHandle.Dispose()
            }
            $ready = $null
            $rootRelease = $null
            $descendantTrigger = $null
            $descendantAck = $null
            if (-not (Remove-SafeTempRoot -Path $testRoot)) {
                throw 'TASK037_SELF_TEST_ROOT_CLEANUP_REJECTED'
            }
        }
        finally {
            foreach ($name in $ambientCanaries) {
                if ($ambientOriginal.ContainsKey($name)) {
                    [Environment]::SetEnvironmentVariable($name, $ambientOriginal[$name], 'Process')
                }
            }
            if ($job -ne [IntPtr]::Zero) {
                try { Stop-Task037Job -Job $job } catch { }
                try { Close-Task037Job -Job $job } catch { }
            }
            if ($null -ne $owned) {
                try { $owned.ChildProcess.Dispose() } catch { }
                try { $owned.Process.Dispose() } catch { }
            }
            foreach ($eventHandle in @($ready, $rootRelease, $descendantTrigger, $descendantAck)) {
                if ($null -ne $eventHandle) { try { $eventHandle.Dispose() } catch { } }
            }
            if (Test-Path -LiteralPath $testRoot) {
                Remove-SafeTempRoot -Path $testRoot | Out-Null
            }
            if (Test-Path -LiteralPath $credentialOwnerRecord -PathType Leaf) {
                if ($null -ne $credentialOwnership -and $null -ne $credentialOwnership.OwnerRecordLock) {
                    try { $credentialOwnership.OwnerRecordLock.Dispose() } catch { }
                }
                try { [IO.File]::Delete($credentialOwnerRecord) } catch { }
            }
        }
    }

    $supervisedAcceptanceId = [Guid]::NewGuid().ToString('N')
    $supervised = $null
    $supervisedOwned = $null
    $supervisedQuery = [IntPtr]::Zero
    try {
        $supervised = New-Task037CleanupSupervision `
            -RepositoryTarget (Join-Path $repositoryRoot 'target') `
            -AcceptanceId $supervisedAcceptanceId
        try { [LatticeTask037JobObjectInterop]::VerifyEffectOpenDenied($supervised.JobName) }
        catch { throw 'TASK037_SELF_TEST_JOB_EFFECT_DACL_REJECTED' }
        $bindingEnvironmentNames = @(
            'LATTICE_TASK037_SUPERVISOR_MANIFEST_PATH',
            'LATTICE_TASK037_SUPERVISOR_OWNER_TOKEN',
            'LATTICE_TASK037_SUPERVISOR_TEMP_BASE'
        )
        $bindingEnvironmentOriginal = @{}
        foreach ($name in $bindingEnvironmentNames) {
            $bindingEnvironmentOriginal[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        }
        try {
            [Environment]::SetEnvironmentVariable(
                'LATTICE_TASK037_SUPERVISOR_MANIFEST_PATH', $supervised.ManifestPath, 'Process'
            )
            [Environment]::SetEnvironmentVariable(
                'LATTICE_TASK037_SUPERVISOR_OWNER_TOKEN', $supervised.OwnerToken, 'Process'
            )
            [Environment]::SetEnvironmentVariable(
                'LATTICE_TASK037_SUPERVISOR_TEMP_BASE', (Split-Path -Parent $supervised.TempRoot), 'Process'
            )
            $wrongJobFailure = $null
            try {
                Assert-Task037SupervisorBinding `
                    -RepositoryTarget (Join-Path $repositoryRoot 'target') `
                    -AcceptanceId $supervisedAcceptanceId `
                    -TempRoot $supervised.TempRoot
            }
            catch { $wrongJobFailure = $_.Exception.Message }
            if ($wrongJobFailure -cne 'TASK037_SUPERVISOR_BINDING_JOB_REJECTED') {
                throw 'TASK037_SELF_TEST_EXACT_JOB_BINDING_REJECTED'
            }
            $alternateBindingBase = Join-Path $tempBase ('lattice-task037-binding-alt-' + [Guid]::NewGuid().ToString('N'))
            [Environment]::SetEnvironmentVariable(
                'LATTICE_TASK037_SUPERVISOR_TEMP_BASE', $alternateBindingBase, 'Process'
            )
            $alternateBindingFailure = $null
            try {
                Assert-Task037SupervisorBinding `
                    -RepositoryTarget (Join-Path $repositoryRoot 'target') `
                    -AcceptanceId $supervisedAcceptanceId `
                    -TempRoot (Join-Path $alternateBindingBase ('lattice-task037-' + $supervisedAcceptanceId))
            }
            catch { $alternateBindingFailure = $_.Exception.Message }
            if (
                $alternateBindingFailure -cne 'TASK037_SUPERVISOR_BINDING_TEMP_BASE_REJECTED' -or
                -not (Test-Path -LiteralPath $supervised.MarkerPath -PathType Leaf) -or
                -not (Test-Path -LiteralPath $supervised.ManifestPath -PathType Leaf)
            ) {
                throw 'TASK037_SELF_TEST_ALTERNATE_ROOT_BINDING_REJECTED'
            }
        }
        finally {
            foreach ($name in $bindingEnvironmentNames) {
                [Environment]::SetEnvironmentVariable($name, $bindingEnvironmentOriginal[$name], 'Process')
            }
        }
        $deliveryHome = Join-Path $supervised.TempRoot 'delivery-codex-home'
        New-Item -ItemType Directory -Path $deliveryHome -Force:$false | Out-Null
        [IO.File]::WriteAllBytes((Join-Path $deliveryHome '.lattice-codex-home-v1'), $codexMarkerBytes)
        [IO.File]::WriteAllBytes((Join-Path $deliveryHome 'config.toml'), $deliveryConfigBytes)
        [IO.File]::WriteAllText((Join-Path $deliveryHome 'auth.json'), '{"synthetic":"credential"}', [Text.UTF8Encoding]::new($false))
        $phaseRoot = Join-Path $supervised.TempRoot ('phase-' + $supervisedAcceptanceId + '-run-1234abcd')
        $brokerHome = Join-Path $phaseRoot 'broker-codex-home'
        $rawRoot = Join-Path $phaseRoot 'raw'
        New-Item -ItemType Directory -Path $brokerHome -Force:$false | Out-Null
        New-Item -ItemType Directory -Path $rawRoot -Force:$false | Out-Null
        [IO.File]::WriteAllBytes((Join-Path $brokerHome '.lattice-codex-home-v1'), $codexMarkerBytes)
        [IO.File]::WriteAllText((Join-Path $brokerHome 'auth.json'), '{"synthetic":"broker"}', [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $rawRoot 'opaque.log'), 'bare-opaque-secret', [Text.UTF8Encoding]::new($false))
        $sleepSource = 'Start-Sleep -Seconds 30'
        $sleepEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($sleepSource))
        $supervisedOwned = Start-Task037GatedProcess `
            -Job $supervised.Job `
            -AttestationJob $supervised.AttestationJob `
            -PowerShellExecutable $powerShellExecutable `
            -Executable $powerShellExecutable `
            -Arguments @('-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', $sleepEncoded) `
            -WorkingDirectory $supervised.TempRoot `
            -EnvironmentValues ([ordered]@{}) `
            -ChildPidPath (Join-Path $supervised.TempRoot 'crash.child.pid') `
            -ChildStdoutPath (Join-Path $supervised.TempRoot 'crash.child.stdout') `
            -ChildStderrPath (Join-Path $supervised.TempRoot 'crash.child.stderr') `
            -WrapperStdoutPath (Join-Path $supervised.TempRoot 'crash.wrapper.stdout') `
            -WrapperStderrPath (Join-Path $supervised.TempRoot 'crash.wrapper.stderr') `
            -RuntimeIsolationRoot (Join-Path $supervised.TempRoot 'crash-runtime')
        $supervisedQuery = [LatticeTask037JobObjectInterop]::OpenForQuery($supervised.JobName)
        Stop-Task037Job -Job $supervised.Job
        if (
            (Get-Task037JobActiveProcessCount -Job $supervised.Job) -ne 0 -or
            (Get-Task037JobActiveProcessCount -Job $supervised.AttestationJob) -ne 0
        ) {
            throw 'TASK037_SELF_TEST_ATTESTATION_HANDLE_ISOLATION_REJECTED'
        }
        Complete-Task037GatedProcessEvidence -OwnedProcess $supervisedOwned
        Complete-Task037CleanupSupervision -Supervision $supervised
        [LatticeTask037JobObjectInterop]::Close($supervisedQuery)
        $supervisedQuery = [IntPtr]::Zero
        if (
            (Test-Path -LiteralPath $supervised.TempRoot) -or
            (Test-Path -LiteralPath $supervised.ManifestPath)
        ) {
            throw 'TASK037_SELF_TEST_SUPERVISOR_CLEANUP_REJECTED'
        }
    }
    finally {
        if ($null -ne $supervisedOwned) {
            try { $supervisedOwned.ChildProcess.Dispose() } catch { }
            try { $supervisedOwned.Process.Dispose() } catch { }
        }
        if ($null -ne $supervised) {
            if ($supervised.Job -ne [IntPtr]::Zero) {
                try { Stop-Task037Job -Job $supervised.Job } catch { }
                try { Close-Task037Job -Job $supervised.Job } catch { }
            }
            if ($supervised.AttestationJob -ne [IntPtr]::Zero) {
                try { Close-Task037Job -Job $supervised.AttestationJob } catch { }
            }
            if ($null -ne $supervised.MarkerLock) { try { $supervised.MarkerLock.Dispose() } catch { } }
            if ($null -ne $supervised.ManifestLock) { try { $supervised.ManifestLock.Dispose() } catch { } }
        }
        if ($supervisedQuery -ne [IntPtr]::Zero) {
            try { [LatticeTask037JobObjectInterop]::Close($supervisedQuery) } catch { }
        }
    }

    $admissionAcceptanceId = [Guid]::NewGuid().ToString('N')
    $admissionStartedAtUtc = [DateTimeOffset]::UtcNow.ToString(
        'O',
        [Globalization.CultureInfo]::InvariantCulture
    )
    $admissionRepositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
    $admissionReceiptPath = Get-Task037AdmissionReceiptPath `
        -RepositoryTarget $admissionRepositoryTarget `
        -AcceptanceId $admissionAcceptanceId
    $admissionSupervision = $null
    $admissionOwned = $null
    $admissionRelease = $null
    $admissionCredentialSource = $null
    $admissionCredentialDestination = $null
    $admissionCredentialOwnerRecord = $null
    try {
        $null = Write-Task037AdmissionStageReceipt `
            -RepositoryTarget $admissionRepositoryTarget `
            -AcceptanceId $admissionAcceptanceId `
            -StartedAtUtc $admissionStartedAtUtc `
            -Stage 'recovery_preflight'
        $admissionSupervision = New-Task037CleanupSupervision `
            -RepositoryTarget $admissionRepositoryTarget `
            -AcceptanceId $admissionAcceptanceId
        $admissionCredentialSource = Join-Path $admissionSupervision.TempRoot 'admission-credential-source'
        $admissionCredentialDestination = Join-Path $admissionSupervision.TempRoot 'admission-execution-home'
        $admissionCredentialOwnerRecord = Join-Path $admissionRepositoryTarget (
            'task037-admission-probe-owner-' + $admissionAcceptanceId + '.json'
        )
        New-Item -ItemType Directory -Path $admissionCredentialSource -Force:$false | Out-Null
        [IO.File]::WriteAllText(
            (Join-Path $admissionCredentialSource 'auth.json'),
            "{}`n",
            [Text.UTF8Encoding]::new($false)
        )
        $null = Write-Task037AdmissionStageReceipt `
            -RepositoryTarget $admissionRepositoryTarget `
            -AcceptanceId $admissionAcceptanceId `
            -StartedAtUtc $admissionStartedAtUtc `
            -Stage 'supervision_create'
        $admissionReleaseName = 'LatticeTask037AdmissionRelease-' + $admissionAcceptanceId
        $admissionReleaseCreated = $false
        $admissionRelease = [Threading.EventWaitHandle]::new(
            $false,
            [Threading.EventResetMode]::ManualReset,
            $admissionReleaseName,
            [ref]$admissionReleaseCreated
        )
        if (-not $admissionReleaseCreated) {
            throw 'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED'
        }
        $beforeAdmissionGate = {
            $null = Write-Task037AdmissionStageReceipt `
                -RepositoryTarget $admissionRepositoryTarget `
                -AcceptanceId $admissionAcceptanceId `
                -StartedAtUtc $admissionStartedAtUtc `
                -Stage 'gated_controller_start'
        }.GetNewClosure()
        $admissionOwned = Start-Task037GatedProcess `
            -Job $admissionSupervision.Job `
            -AttestationJob $admissionSupervision.AttestationJob `
            -PowerShellExecutable $powerShellExecutable `
            -Executable $powerShellExecutable `
            -Arguments @(
                '-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
                '-File', $PSCommandPath, '-InternalAdmissionProbe'
            ) `
            -WorkingDirectory $repositoryRoot `
            -EnvironmentValues ([ordered]@{
                LATTICE_TASK037_SUPERVISOR_ACCEPTANCE_ID = $admissionAcceptanceId
                LATTICE_TASK037_SUPERVISOR_MANIFEST_PATH = $admissionSupervision.ManifestPath
                LATTICE_TASK037_SUPERVISOR_OWNER_TOKEN = $admissionSupervision.OwnerToken
                LATTICE_TASK037_SUPERVISOR_TEMP_BASE = $tempBase
                LATTICE_TASK037_SUPERVISOR_TEMP_ROOT = $admissionSupervision.TempRoot
                LATTICE_TASK037_ADMISSION_STARTED_AT_UTC = $admissionStartedAtUtc
                LATTICE_TASK037_ADMISSION_PROBE_RELEASE = $admissionReleaseName
                LATTICE_TASK037_ADMISSION_PROBE_CREDENTIAL_SOURCE = $admissionCredentialSource
                LATTICE_TASK037_ADMISSION_PROBE_CREDENTIAL_DESTINATION = $admissionCredentialDestination
                LATTICE_TASK037_ADMISSION_PROBE_CREDENTIAL_OWNER_RECORD = $admissionCredentialOwnerRecord
            }) `
            -ChildPidPath (Join-Path $admissionSupervision.TempRoot 'admission.child.pid') `
            -ChildStdoutPath (Join-Path $admissionSupervision.TempRoot 'admission.child.stdout') `
            -ChildStderrPath (Join-Path $admissionSupervision.TempRoot 'admission.child.stderr') `
            -WrapperStdoutPath (Join-Path $admissionSupervision.TempRoot 'admission.wrapper.stdout') `
            -WrapperStderrPath (Join-Path $admissionSupervision.TempRoot 'admission.wrapper.stderr') `
            -RuntimeIsolationRoot (Join-Path $admissionSupervision.TempRoot 'admission-runtime') `
            -BeforeGate $beforeAdmissionGate
        $null = $admissionRelease.Set()
        if (
            -not $admissionOwned.ChildProcess.WaitForExit(30000) -or
            [int]$admissionOwned.ChildProcess.ExitCode -eq 0 -or
            -not $admissionOwned.Process.WaitForExit(10000)
        ) {
            throw 'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED'
        }
        Complete-Task037GatedProcessEvidence -OwnedProcess $admissionOwned
        if (
            (Get-Task037JobActiveProcessCount -Job $admissionSupervision.Job) -ne 0 -or
            (Get-Task037JobActiveProcessCount -Job $admissionSupervision.AttestationJob) -ne 0
        ) {
            throw 'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED'
        }
        $admissionSourceAuth = Join-Path $admissionCredentialSource 'auth.json'
        if (Test-Path -LiteralPath $admissionSourceAuth -PathType Leaf) {
            [IO.File]::Delete($admissionSourceAuth)
        }
        if (
            (Test-Path -LiteralPath $admissionSourceAuth) -or
            @(Get-ChildItem -LiteralPath $admissionCredentialSource -Force).Count -ne 0
        ) {
            throw 'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED'
        }
        [IO.Directory]::Delete($admissionCredentialSource)
        $admissionProvisioningSentinel = Join-Path `
            $admissionSupervision.TempRoot `
            'admission-provisioning-complete.v1'
        Assert-RegularFile -Path $admissionProvisioningSentinel
        if ([IO.File]::ReadAllText(
            $admissionProvisioningSentinel,
            [Text.Encoding]::UTF8
        ) -cne "lattice.task037.admission-provisioning.v1|$admissionAcceptanceId`n") {
            throw 'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED'
        }
        $admissionReceipt = Read-Task037AdmissionStageReceipt `
            -Path $admissionReceiptPath `
            -AcceptanceId $admissionAcceptanceId
        $safeAdmissionFailure = Get-SafeFailureEvidence `
            -Message 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
        if (
            [string]$admissionReceipt.stage -cne 'supervisor_binding' -or
            [string]$admissionReceipt.started_at_utc -cne $admissionStartedAtUtc -or
            [string]$safeAdmissionFailure.code -cne 'TASK037_CONTROLLED_FAILURE'
        ) {
            throw 'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED'
        }
        Complete-Task037CleanupSupervision -Supervision $admissionSupervision
        if (
            (Test-Path -LiteralPath $admissionSupervision.TempRoot) -or
            (Test-Path -LiteralPath $admissionSupervision.ManifestPath) -or
            -not (Test-Path -LiteralPath $admissionReceiptPath -PathType Leaf) -or
            (Test-Path -LiteralPath $admissionCredentialSource) -or
            (Test-Path -LiteralPath $admissionCredentialDestination) -or
            (Test-Path -LiteralPath $admissionCredentialOwnerRecord) -or
            (Test-Path -LiteralPath (
                Join-Path $admissionRepositoryTarget ('full-chain-acceptance\' + $admissionAcceptanceId)
            ))
        ) {
            throw 'TASK037_SELF_TEST_ADMISSION_RECEIPT_REJECTED'
        }
    }
    finally {
        if ($null -ne $admissionRelease) { try { $admissionRelease.Dispose() } catch { } }
        if ($null -ne $admissionOwned) {
            try { $admissionOwned.ChildProcess.Dispose() } catch { }
            try { $admissionOwned.Process.Dispose() } catch { }
        }
        if ($null -ne $admissionSupervision) {
            if ($admissionSupervision.Job -ne [IntPtr]::Zero) {
                try { Stop-Task037Job -Job $admissionSupervision.Job } catch { }
                try { Close-Task037Job -Job $admissionSupervision.Job } catch { }
            }
            if ($admissionSupervision.AttestationJob -ne [IntPtr]::Zero) {
                try { Close-Task037Job -Job $admissionSupervision.AttestationJob } catch { }
            }
            if ($null -ne $admissionSupervision.MarkerLock) {
                try { $admissionSupervision.MarkerLock.Dispose() } catch { }
            }
            if ($null -ne $admissionSupervision.ManifestLock) {
                try { $admissionSupervision.ManifestLock.Dispose() } catch { }
            }
        }
        if (Test-Path -LiteralPath $admissionReceiptPath -PathType Leaf) {
            try { [IO.File]::Delete($admissionReceiptPath) } catch { }
        }
        if (
            -not [string]::IsNullOrWhiteSpace($admissionCredentialOwnerRecord) -and
            (Test-Path -LiteralPath $admissionCredentialOwnerRecord -PathType Leaf)
        ) {
            try { [IO.File]::Delete($admissionCredentialOwnerRecord) } catch { }
        }
        if (
            $null -ne $admissionSupervision -and
            -not [string]::IsNullOrWhiteSpace($admissionCredentialSource) -and
            (Test-Path -LiteralPath $admissionCredentialSource)
        ) {
            $sourceAuth = Join-Path $admissionCredentialSource 'auth.json'
            if (Test-Path -LiteralPath $sourceAuth -PathType Leaf) {
                try { [IO.File]::Delete($sourceAuth) } catch { }
            }
            if (Test-Path -LiteralPath $admissionCredentialSource) {
                try { [IO.Directory]::Delete($admissionCredentialSource, $true) } catch { }
            }
        }
        $admissionDirectory = Split-Path -Parent $admissionReceiptPath
        if (
            (Test-Path -LiteralPath $admissionDirectory -PathType Container) -and
            @(Get-ChildItem -LiteralPath $admissionDirectory -Force).Count -eq 0
        ) {
            try { [IO.Directory]::Delete($admissionDirectory) } catch { }
        }
    }

    $recoveryAcceptanceId = [Guid]::NewGuid().ToString('N')
    $recoveryToken = New-RandomHex -Bytes 32
    $recoveryRoot = Get-Task037SupervisedTempRoot -AcceptanceId $recoveryAcceptanceId
    $recoveryManifest = Get-Task037RecoveryManifestPath `
        -RepositoryTarget (Join-Path $repositoryRoot 'target') `
        -AcceptanceId $recoveryAcceptanceId
    New-Item -ItemType Directory -Path $recoveryRoot -Force:$false | Out-Null
    $recoveryMarker = Join-Path $recoveryRoot '.lattice-task037-cleanup-root-v1'
    [IO.File]::WriteAllText(
        $recoveryMarker,
        "lattice.task037.cleanup-root.v1|$recoveryAcceptanceId|$recoveryToken`n",
        [Text.UTF8Encoding]::new($false)
    )
    $recoveryDelivery = Join-Path $recoveryRoot 'delivery-codex-home'
    New-Item -ItemType Directory -Path $recoveryDelivery -Force:$false | Out-Null
    [IO.File]::WriteAllText((Join-Path $recoveryDelivery 'auth.json'), '{"synthetic":"recovery"}', [Text.UTF8Encoding]::new($false))
    Write-Task037DurableRecoveryManifest -Path $recoveryManifest -Value ([ordered]@{
        schema_version = 'lattice.task037.recovery.v2'
        acceptance_id = $recoveryAcceptanceId
        owner_token = $recoveryToken
        marker_sha256 = Get-FileSha256 $recoveryMarker
        controller_process_id = 2147483647
        controller_process_start_ticks = 1
        temp_base = $tempBase
        temp_root = $recoveryRoot
    })
    $alternateTemp = Join-Path $tempBase ('lattice-task037-alternate-temp-' + [Guid]::NewGuid().ToString('N'))
    $alternateCanary = Join-Path $alternateTemp 'must-survive.txt'
    $originalTemp = [Environment]::GetEnvironmentVariable('TEMP', 'Process')
    $originalTmp = [Environment]::GetEnvironmentVariable('TMP', 'Process')
    try {
        New-Item -ItemType Directory -Path $alternateTemp -Force:$false | Out-Null
        [IO.File]::WriteAllText($alternateCanary, 'must-survive', [Text.UTF8Encoding]::new($false))
        [Environment]::SetEnvironmentVariable('TEMP', $alternateTemp, 'Process')
        [Environment]::SetEnvironmentVariable('TMP', $alternateTemp, 'Process')
        $recovered = Invoke-Task037RecoveryOnly -RepositoryTarget (Join-Path $repositoryRoot 'target')
        if (
            $recovered -lt 1 -or
            (Test-Path -LiteralPath $recoveryRoot) -or
            (Test-Path -LiteralPath $recoveryManifest) -or
            -not (Test-Path -LiteralPath $alternateCanary -PathType Leaf) -or
            [IO.File]::ReadAllText($alternateCanary, [Text.Encoding]::UTF8) -cne 'must-survive'
        ) {
            throw 'TASK037_SELF_TEST_RECOVERY_REJECTED'
        }
    }
    finally {
        [Environment]::SetEnvironmentVariable('TEMP', $originalTemp, 'Process')
        [Environment]::SetEnvironmentVariable('TMP', $originalTmp, 'Process')
        if (Test-Path -LiteralPath $alternateTemp) {
            try {
                Remove-SafeTempRoot `
                    -Path $alternateTemp `
                    -AuthorizedTempBase $tempBase | Out-Null
            }
            catch { }
        }
    }

    $crossTempAcceptanceId = [Guid]::NewGuid().ToString('N')
    $crossTempSupervision = $null
    $crossTempAlternate = Join-Path $tempBase ('lattice-task037-cross-temp-' + [Guid]::NewGuid().ToString('N'))
    $crossTempCanary = Join-Path $crossTempAlternate 'must-survive.txt'
    $crossTempOriginalTemp = [Environment]::GetEnvironmentVariable('TEMP', 'Process')
    $crossTempOriginalTmp = [Environment]::GetEnvironmentVariable('TMP', 'Process')
    try {
        $crossTempSupervision = New-Task037CleanupSupervision `
            -RepositoryTarget (Join-Path $repositoryRoot 'target') `
            -AcceptanceId $crossTempAcceptanceId `
            -TempBase $tempBase
        New-Item -ItemType Directory -Path $crossTempAlternate -Force:$false | Out-Null
        [IO.File]::WriteAllText($crossTempCanary, 'must-survive', [Text.UTF8Encoding]::new($false))
        [Environment]::SetEnvironmentVariable('TEMP', $crossTempAlternate, 'Process')
        [Environment]::SetEnvironmentVariable('TMP', $crossTempAlternate, 'Process')
        Complete-Task037CleanupSupervision -Supervision $crossTempSupervision
        if (
            (Test-Path -LiteralPath $crossTempSupervision.TempRoot) -or
            (Test-Path -LiteralPath $crossTempSupervision.ManifestPath) -or
            -not (Test-Path -LiteralPath $crossTempCanary -PathType Leaf) -or
            [IO.File]::ReadAllText($crossTempCanary, [Text.Encoding]::UTF8) -cne 'must-survive'
        ) {
            throw 'TASK037_SELF_TEST_CROSS_TEMP_CLEANUP_REJECTED'
        }
    }
    finally {
        [Environment]::SetEnvironmentVariable('TEMP', $crossTempOriginalTemp, 'Process')
        [Environment]::SetEnvironmentVariable('TMP', $crossTempOriginalTmp, 'Process')
        if ($null -ne $crossTempSupervision) {
            if ($crossTempSupervision.Job -ne [IntPtr]::Zero) {
                try { Stop-Task037Job -Job $crossTempSupervision.Job } catch { }
                try { Close-Task037Job -Job $crossTempSupervision.Job } catch { }
            }
            if ($crossTempSupervision.AttestationJob -ne [IntPtr]::Zero) {
                try { Close-Task037Job -Job $crossTempSupervision.AttestationJob } catch { }
            }
            if ($null -ne $crossTempSupervision.MarkerLock) {
                try { $crossTempSupervision.MarkerLock.Dispose() } catch { }
            }
            if ($null -ne $crossTempSupervision.ManifestLock) {
                try { $crossTempSupervision.ManifestLock.Dispose() } catch { }
            }
            if (Test-Path -LiteralPath $crossTempSupervision.TempRoot) {
                try {
                    $null = Remove-Task037SupervisedTempRoot `
                        -Root $crossTempSupervision.TempRoot `
                        -AcceptanceId $crossTempSupervision.AcceptanceId `
                        -OwnerToken $crossTempSupervision.OwnerToken `
                        -TempBase $tempBase
                }
                catch { }
            }
            if (Test-Path -LiteralPath $crossTempSupervision.ManifestPath -PathType Leaf) {
                try { [IO.File]::Delete($crossTempSupervision.ManifestPath) } catch { }
            }
        }
        if (Test-Path -LiteralPath $crossTempAlternate) {
            try { Remove-SafeTempRoot -Path $crossTempAlternate -AuthorizedTempBase $tempBase | Out-Null } catch { }
        }
    }

    $secretFirstAcceptanceId = [Guid]::NewGuid().ToString('N')
    $secretFirstToken = New-RandomHex -Bytes 32
    $secretFirstRoot = Get-Task037SupervisedTempRoot -AcceptanceId $secretFirstAcceptanceId
    $secretFirstCanaryRoot = Join-Path $tempBase ('lattice-task037-secret-first-canary-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $secretFirstRoot -Force:$false | Out-Null
        $secretFirstMarker = Join-Path $secretFirstRoot '.lattice-task037-cleanup-root-v1'
        [IO.File]::WriteAllText(
            $secretFirstMarker,
            "lattice.task037.cleanup-root.v1|$secretFirstAcceptanceId|$secretFirstToken`n",
            [Text.UTF8Encoding]::new($false)
        )
        $deliveryAuth = Join-Path $secretFirstRoot 'delivery-codex-home\auth.json'
        $brokerAuth = Join-Path $secretFirstRoot ("phase-$secretFirstAcceptanceId-run-1234abcd\broker-codex-home\auth.json")
        $unknownAuth = Join-Path $secretFirstRoot 'aaa\auth.json'
        foreach ($path in @($deliveryAuth, $brokerAuth, $unknownAuth)) {
            New-Item -ItemType Directory -Path (Split-Path -Parent $path) -Force:$false | Out-Null
            [IO.File]::WriteAllText($path, '{"synthetic":"credential"}', [Text.UTF8Encoding]::new($false))
        }
        New-Item -ItemType Directory -Path $secretFirstCanaryRoot -Force:$false | Out-Null
        $secretFirstCanary = Join-Path $secretFirstCanaryRoot 'canary.txt'
        [IO.File]::WriteAllText($secretFirstCanary, 'must-survive', [Text.UTF8Encoding]::new($false))
        $secretFirstJunction = Join-Path $secretFirstRoot 'zzz-junction'
        New-Item -ItemType Junction -Path $secretFirstJunction -Target $secretFirstCanaryRoot | Out-Null
        [IO.File]::WriteAllText($secretFirstMarker, 'corrupt-marker', [Text.UTF8Encoding]::new($false))
        $markerFailure = $null
        try {
            $null = Remove-Task037SupervisedTempRoot `
                -Root $secretFirstRoot `
                -AcceptanceId $secretFirstAcceptanceId `
                -OwnerToken $secretFirstToken
        }
        catch { $markerFailure = $_.Exception.Message }
        if (
            $markerFailure -cne 'TASK037_RECOVERY_MARKER_REJECTED' -or
            -not (Test-Path -LiteralPath $deliveryAuth -PathType Leaf) -or
            -not (Test-Path -LiteralPath $brokerAuth -PathType Leaf) -or
            -not (Test-Path -LiteralPath $unknownAuth -PathType Leaf) -or
            -not (Test-Path -LiteralPath $secretFirstCanary -PathType Leaf)
        ) {
            throw 'TASK037_SELF_TEST_MARKER_AUTHORITY_REJECTED'
        }
        [IO.File]::WriteAllText(
            $secretFirstMarker,
            "lattice.task037.cleanup-root.v1|$secretFirstAcceptanceId|$secretFirstToken`n",
            [Text.UTF8Encoding]::new($false)
        )
        $secretFirstFailure = $null
        try {
            $null = Remove-Task037SupervisedTempRoot `
                -Root $secretFirstRoot `
                -AcceptanceId $secretFirstAcceptanceId `
                -OwnerToken $secretFirstToken
        }
        catch { $secretFirstFailure = $_.Exception.Message }
        if (
            $secretFirstFailure -cne 'TASK037_RECOVERY_SCOPE_REJECTED' -or
            (Test-Path -LiteralPath $deliveryAuth) -or
            (Test-Path -LiteralPath $brokerAuth) -or
            (Test-Path -LiteralPath $unknownAuth) -or
            -not (Test-Path -LiteralPath $secretFirstCanary -PathType Leaf) -or
            [IO.File]::ReadAllText($secretFirstCanary, [Text.Encoding]::UTF8) -cne 'must-survive'
        ) {
            throw 'TASK037_SELF_TEST_SECRET_FIRST_CLEANUP_REJECTED'
        }
        [IO.Directory]::Delete($secretFirstJunction)
        Remove-Item -LiteralPath (Split-Path -Parent $unknownAuth) -Recurse -Force
        $null = Remove-Task037SupervisedTempRoot `
            -Root $secretFirstRoot `
            -AcceptanceId $secretFirstAcceptanceId `
            -OwnerToken $secretFirstToken
    }
    finally {
        if (Test-Path -LiteralPath $secretFirstRoot) {
            $junction = Join-Path $secretFirstRoot 'zzz-junction'
            if (Test-Path -LiteralPath $junction) { try { [IO.Directory]::Delete($junction) } catch { } }
            try { Remove-Item -LiteralPath $secretFirstRoot -Recurse -Force } catch { }
        }
        if (Test-Path -LiteralPath $secretFirstCanaryRoot) {
            try { Remove-SafeTempRoot -Path $secretFirstCanaryRoot | Out-Null } catch { }
        }
    }

    $forgedAcceptanceId = [Guid]::NewGuid().ToString('N')
    $forgedToken = New-RandomHex -Bytes 32
    $forgedRoot = Get-Task037SupervisedTempRoot -AcceptanceId $forgedAcceptanceId
    $forgedManifest = Get-Task037RecoveryManifestPath `
        -RepositoryTarget (Join-Path $repositoryRoot 'target') `
        -AcceptanceId $forgedAcceptanceId
    $forgedCanaryRoot = Join-Path $tempBase ('lattice-task037-forged-canary-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $forgedRoot -Force:$false | Out-Null
        New-Item -ItemType Directory -Path $forgedCanaryRoot -Force:$false | Out-Null
        $forgedMarker = Join-Path $forgedRoot '.lattice-task037-cleanup-root-v1'
        $forgedAuth = Join-Path (Join-Path $forgedRoot 'delivery-codex-home') 'auth.json'
        New-Item -ItemType Directory -Path (Split-Path -Parent $forgedAuth) -Force:$false | Out-Null
        [IO.File]::WriteAllText(
            $forgedMarker,
            "lattice.task037.cleanup-root.v1|$forgedAcceptanceId|$forgedToken`n",
            [Text.UTF8Encoding]::new($false)
        )
        [IO.File]::WriteAllText($forgedAuth, '{"synthetic":"forged"}', [Text.UTF8Encoding]::new($false))
        $forgedCanary = Join-Path $forgedCanaryRoot 'canary.txt'
        [IO.File]::WriteAllText($forgedCanary, 'must-survive', [Text.UTF8Encoding]::new($false))
        Write-Task037DurableRecoveryManifest -Path $forgedManifest -Value ([ordered]@{
            schema_version = 'lattice.task037.recovery.v2'
            acceptance_id = $forgedAcceptanceId
            owner_token = $forgedToken
            marker_sha256 = Get-FileSha256 $forgedMarker
            controller_process_id = 2147483647
            controller_process_start_ticks = 1
            temp_base = $tempBase
            temp_root = $forgedRoot
            destination = $forgedCanaryRoot
        })
        $forgedFailure = $null
        try { $null = Invoke-Task037RecoveryOnly -RepositoryTarget (Join-Path $repositoryRoot 'target') }
        catch { $forgedFailure = $_.Exception.Message }
        if (
            $forgedFailure -cne 'TASK037_RECOVERY_MANIFEST_REJECTED' -or
            -not (Test-Path -LiteralPath $forgedAuth -PathType Leaf) -or
            -not (Test-Path -LiteralPath $forgedCanary -PathType Leaf) -or
            [IO.File]::ReadAllText($forgedCanary, [Text.Encoding]::UTF8) -cne 'must-survive'
        ) {
            throw 'TASK037_SELF_TEST_FORGED_RECOVERY_SCOPE_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $forgedManifest -PathType Leaf) {
            try { [IO.File]::Delete($forgedManifest) } catch { }
        }
        if (Test-Path -LiteralPath $forgedRoot) {
            try {
                $null = Remove-Task037SupervisedTempRoot `
                    -Root $forgedRoot `
                    -AcceptanceId $forgedAcceptanceId `
                    -OwnerToken $forgedToken
            }
            catch { }
        }
        if (Test-Path -LiteralPath $forgedCanaryRoot) {
            try { $null = Remove-SafeTempRoot -Path $forgedCanaryRoot } catch { }
        }
    }

    $secretRoot = Join-Path $tempBase ('lattice-task037-secret-selftest-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $secretRoot -Force:$false | Out-Null
        $script:Task037KnownSecrets = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        Add-Task037KnownSecret -Value 'bare-opaque-secret'
        $secretFailure = $null
        try {
            $null = Write-Task037ValidatedPhaseEvidence `
                -Evidence ([pscustomobject]@{ hermes_summary = 'bare-opaque-secret' }) `
                -IsError $false `
                -OutputPath (Join-Path $secretRoot 'unsafe.json') `
                -Meta ([ordered]@{ response_sha256 = $null }) `
                -MetaPath (Join-Path $secretRoot 'unsafe.meta.json')
        }
        catch { $secretFailure = $_.Exception.Message }
        if (
            $secretFailure -cne 'TASK037_SECRET_PERSISTENCE_REJECTED' -or
            (Test-Path -LiteralPath (Join-Path $secretRoot 'unsafe.json')) -or
            (Test-Path -LiteralPath (Join-Path $secretRoot 'unsafe.meta.json'))
        ) {
            throw 'TASK037_SELF_TEST_SECRET_PERSISTENCE_REJECTED'
        }
        $safeSource = [pscustomobject]@{
            status = 'OK'
            launcher_path = 'C:\synthetic\trusted\codex.exe'
            hermes_summary = 'bounded summary'
            hermes_findings = @([pscustomobject]@{
                evidence_digest = ('b' * 64)
                statement = 'bounded finding'
            })
            hermes_next_actions = @('bounded action')
            thread_id = 'thread-sensitive-value'
            turn_id = 'turn-sensitive-value'
        }
        $safePath = Join-Path $secretRoot 'safe.json'
        $safeMetaPath = Join-Path $secretRoot 'safe.meta.json'
        $safeDigest = Write-Task037ValidatedPhaseEvidence `
            -Evidence $safeSource `
            -IsError $false `
            -OutputPath $safePath `
            -Meta ([ordered]@{ response_sha256 = $null }) `
            -MetaPath $safeMetaPath
        $safeEvidence = Read-JsonEvidence -Path $safePath
        $safeText = [IO.File]::ReadAllText($safePath, [Text.Encoding]::UTF8)
        Assert-Task037ExactPropertySet `
            -Value $safeEvidence `
            -Expected @(
                'hermes_findings', 'hermes_next_actions', 'hermes_summary_byte_count',
                'hermes_summary_sha256', 'launcher_path_byte_count', 'launcher_path_sha256',
                'status', 'thread_id_byte_count', 'thread_id_sha256',
                'turn_id_byte_count', 'turn_id_sha256'
            ) `
            -FailureCode 'TASK037_SELF_TEST_SAFE_PROJECTION_REJECTED'
        if (
            $safeDigest -cne (Get-FileSha256 $safePath) -or
            $safeText.IndexOf('bounded summary', [StringComparison]::Ordinal) -ge 0 -or
            $safeText.IndexOf('bounded finding', [StringComparison]::Ordinal) -ge 0 -or
            $safeText.IndexOf('bounded action', [StringComparison]::Ordinal) -ge 0 -or
            $safeText.IndexOf('C:\synthetic\trusted\codex.exe', [StringComparison]::Ordinal) -ge 0 -or
            $safeText.IndexOf('thread-sensitive-value', [StringComparison]::Ordinal) -ge 0 -or
            $safeText.IndexOf('turn-sensitive-value', [StringComparison]::Ordinal) -ge 0 -or
            [string]$safeEvidence.hermes_summary_sha256 -cne (Get-StringSha256 'bounded summary')
        ) {
            throw 'TASK037_SELF_TEST_SAFE_PROJECTION_REJECTED'
        }
    }
    finally {
        $script:Task037KnownSecrets = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        if (Test-Path -LiteralPath $secretRoot) { Remove-SafeTempRoot -Path $secretRoot | Out-Null }
    }

    $forgedFailure = Get-SafeFailureEvidence -Message 'LATTICE_BARE_OPAQUE_SECRET_CANARY'
    $forgedThrown = $null
    try { Throw-Task037SafeFailure -Message 'LATTICE_BARE_OPAQUE_SECRET_CANARY' }
    catch { $forgedThrown = $_.Exception.Message }
    if (
        [string]$forgedFailure.code -cne 'TASK037_LATTICE_COMPONENT_FAILURE' -or
        @($forgedFailure.Keys).Count -ne 1 -or
        $forgedThrown -cne 'TASK037_LATTICE_COMPONENT_FAILURE' -or
        (($forgedFailure | ConvertTo-Json -Compress).IndexOf('BARE_OPAQUE_SECRET_CANARY', [StringComparison]::Ordinal) -ge 0)
    ) {
        throw 'TASK037_SELF_TEST_FAILURE_CLASSIFICATION_REJECTED'
    }

    $launcherFailure = $null
    try {
        Assert-Task037TrustedLauncherPath `
            -Actual 'C:\synthetic\untrusted\codex.exe' `
            -Expected 'C:\synthetic\trusted\codex.exe'
    }
    catch { $launcherFailure = $_.Exception.Message }
    if ($launcherFailure -cne 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED') {
        throw 'TASK037_SELF_TEST_LAUNCHER_BINDING_REJECTED'
    }
    Assert-Task037TrustedLauncherPath `
        -Actual 'C:\synthetic\trusted\codex.exe' `
        -Expected 'C:\synthetic\trusted\codex.exe'

    $toolFailureCases = @(
        [pscustomobject]@{ Code = 'LATTICED_CONFIGURATION_REJECTED'; Gate = 'success_tool_error_configuration' },
        [pscustomobject]@{ Code = 'LATTICE_DELIVERY_FAILED'; Gate = 'success_tool_error_delivery' },
        [pscustomobject]@{ Code = 'LATTICE_GRAPH_MEMORY_RUN_REJECTED'; Gate = 'success_tool_error_graph' },
        [pscustomobject]@{ Code = 'LATTICE_HERMES_REFLECTION_REJECTED'; Gate = 'success_tool_error_hermes' },
        [pscustomobject]@{ Code = 'LATTICED_STDIO_REJECTED'; Gate = 'success_tool_error_transport' },
        [pscustomobject]@{ Code = 'lattice_hermes_reflection_rejected'; Gate = 'success_tool_error_unclassified' },
        [pscustomobject]@{ Code = 'TASK037_UNKNOWN_TOOL_FAILURE'; Gate = 'success_tool_error_unclassified' }
    )
    foreach ($toolFailureCase in $toolFailureCases) {
        $toolFailureGate = Get-Task037ToolFailureGate -Code $toolFailureCase.Code
        if (
            $toolFailureGate -cne [string]$toolFailureCase.Gate
        ) {
            throw 'TASK037_SELF_TEST_TOOL_FAILURE_GATE_REJECTED'
        }
        $toolFailureActiveGate = 'phase_meta_projection'
        $toolFailureLastCompletedGate = 'success_semantics'
        Enter-Task037PhaseGate `
            -Gate $toolFailureGate `
            -ActiveGate ([ref]$toolFailureActiveGate) `
            -LastCompletedGate ([ref]$toolFailureLastCompletedGate)
        if (
            $toolFailureActiveGate -cne [string]$toolFailureCase.Gate -or
            $toolFailureLastCompletedGate -cne 'phase_meta_projection'
        ) {
            throw 'TASK037_SELF_TEST_TOOL_FAILURE_GATE_REJECTED'
        }
    }
    if ((Get-Task037ToolFailureGate -Code $true) -cne 'success_tool_error_unclassified') {
        throw 'TASK037_SELF_TEST_TOOL_FAILURE_GATE_REJECTED'
    }

    $hermesFailureCases = @(
        [pscustomobject]@{ Classification = 'HERMES_RUN_FAILED_HINT_AUTH'; Stage = 'auth'; Gate = 'success_tool_error_hermes_hint_auth' },
        [pscustomobject]@{ Classification = 'HERMES_RUN_FAILED_HINT_QUOTA'; Stage = 'quota'; Gate = 'success_tool_error_hermes_hint_quota' },
        [pscustomobject]@{ Classification = 'HERMES_RUN_FAILED_HINT_TRANSPORT'; Stage = 'transport'; Gate = 'success_tool_error_hermes_hint_transport' },
        [pscustomobject]@{ Classification = 'HERMES_RUN_FAILED_HINT_APP_SERVER_TIMEOUT'; Stage = 'app_server_timeout'; Gate = 'success_tool_error_hermes_hint_app_server_timeout' },
        [pscustomobject]@{ Classification = 'HERMES_RUN_FAILED_HINT_APP_SERVER_TURN_STATUS'; Stage = 'app_server_turn_status'; Gate = 'success_tool_error_hermes_hint_app_server_turn_status' },
        [pscustomobject]@{ Classification = 'HERMES_RUN_FAILED_HINT_SCHEMA'; Stage = 'schema'; Gate = 'success_tool_error_hermes_hint_schema' },
        [pscustomobject]@{ Classification = 'HERMES_RUN_FAILED_HINT_APP_SERVER'; Stage = 'app_server'; Gate = 'success_tool_error_hermes_hint_app_server' }
    )
    foreach ($hermesSource in @('status', 'events')) {
        foreach ($hermesFailureCase in $hermesFailureCases) {
            $detailLine = [ordered]@{
                component = 'Hermes'
                classification = [string]$hermesFailureCase.Classification
                detail_byte_count = [int32]12
                detail_redacted = $true
                detail_sha256 = ('a' * 64)
                event = 'run_failure_detail'
                failure_stage = [string]$hermesFailureCase.Stage
                source = [string]$hermesSource
            } | ConvertTo-Json -Compress
            $reflectionLine = [ordered]@{
                component = 'Hermes'
                error_code = [string]$hermesFailureCase.Classification
                event = 'reflection_rejected'
            } | ConvertTo-Json -Compress
            $diagnosticText = $detailLine + "`n" + $reflectionLine + "`n"
            $hermesGateResult = @(Get-Task037HermesToolFailureGate -StderrText $diagnosticText)
            if (
                $hermesGateResult.Count -ne 1 -or
                $hermesGateResult[0] -isnot [string] -or
                [string]$hermesGateResult[0] -cne [string]$hermesFailureCase.Gate
            ) {
                throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
            }
        }
    }

    $codexDetailLine = [ordered]@{
        component = 'Codex app-server'
        classification = 'HERMES_RUN_FAILED_HINT_AUTH'
        detail_byte_count = [int32]12
        detail_redacted = $true
        detail_sha256 = ('b' * 64)
        event = 'run_failure_detail'
        failure_stage = 'auth'
        source = 'completed_output'
    } | ConvertTo-Json -Compress
    $codexReflectionLine = [ordered]@{
        component = 'Hermes'
        error_code = 'HERMES_CODEX_APP_SERVER_RUN_FAILED'
        event = 'reflection_rejected'
    } | ConvertTo-Json -Compress
    if (
        (Get-Task037HermesToolFailureGate -StderrText ($codexDetailLine + "`n" + $codexReflectionLine)) -cne
        'success_tool_error_hermes_hint_auth'
    ) {
        throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
    }

    $emptyDetailLine = [ordered]@{
        component = 'Codex app-server'
        classification = 'HERMES_CODEX_APP_SERVER_EMPTY_OUTPUT'
        event = 'run_failure_detail'
        source = 'empty'
    } | ConvertTo-Json -Compress
    if (
        (Get-Task037HermesToolFailureGate -StderrText ($emptyDetailLine + "`n" + $codexReflectionLine)) -cne
        'success_tool_error_hermes_app_server_empty_output'
    ) {
        throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
    }

    $missingDetailLine = [ordered]@{
        component = 'Hermes'
        classification = 'HERMES_RUN_FAILED'
        detail_present = $false
        event = 'run_failure_detail'
        source = 'status'
    } | ConvertTo-Json -Compress
    $genericReflectionLine = [ordered]@{
        component = 'Hermes'
        error_code = 'HERMES_RUN_FAILED'
        event = 'reflection_rejected'
    } | ConvertTo-Json -Compress
    $genericEmptyDetailLine = [ordered]@{
        component = 'Hermes'
        classification = 'HERMES_RUN_FAILED'
        detail_byte_count = [int32]0
        detail_redacted = $true
        detail_sha256 = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
        event = 'run_failure_detail'
        failure_stage = 'unknown'
        source = 'events'
    } | ConvertTo-Json -Compress
    $unclassifiedDiagnostics = @(
        ($missingDetailLine + "`n" + $genericReflectionLine),
        ($genericEmptyDetailLine + "`n" + $genericReflectionLine),
        ('{"component":"Hermes","error_code":"HERMES_UNKNOWN_CODE","event":"reflection_rejected"}'),
        ($codexDetailLine + "`n" + ($codexReflectionLine -replace 'HERMES_CODEX_APP_SERVER_RUN_FAILED', 'hermes_codex_app_server_run_failed'))
    )
    foreach ($unclassifiedDiagnostic in $unclassifiedDiagnostics) {
        if (
            (Get-Task037HermesToolFailureGate -StderrText $unclassifiedDiagnostic) -cne
            'success_tool_error_hermes_unclassified'
        ) {
            throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
        }
    }

    $authEventsDetailLine = [ordered]@{
        component = 'Hermes'
        classification = 'HERMES_RUN_FAILED_HINT_AUTH'
        detail_byte_count = [int32]12
        detail_redacted = $true
        detail_sha256 = ('c' * 64)
        event = 'run_failure_detail'
        failure_stage = 'auth'
        source = 'events'
    } | ConvertTo-Json -Compress
    $authReflectionLine = [ordered]@{
        component = 'Hermes'
        error_code = 'HERMES_RUN_FAILED_HINT_AUTH'
        event = 'reflection_rejected'
    } | ConvertTo-Json -Compress
    $quotaReflectionLine = [ordered]@{
        component = 'Hermes'
        error_code = 'HERMES_RUN_FAILED_HINT_QUOTA'
        event = 'reflection_rejected'
    } | ConvertTo-Json -Compress
    $extraKeyDetailLine = $authEventsDetailLine.Substring(0, $authEventsDetailLine.Length - 1) + ',"extra":true}'
    $diagnosticRejectedCases = @(
        $genericReflectionLine,
        $codexDetailLine,
        ($codexDetailLine + "`n" + $codexDetailLine + "`n" + $codexReflectionLine),
        ($codexDetailLine + "`n" + $codexReflectionLine + "`n" + $codexReflectionLine),
        (($codexDetailLine -replace 'HERMES_RUN_FAILED_HINT_AUTH', 'HERMES_RUN_FAILED_HINT_QUOTA') + "`n" + $codexReflectionLine),
        (($missingDetailLine -replace '"source":"status"', '"source":"events"') + "`n" + $genericReflectionLine),
        (($codexDetailLine -replace '"source":"completed_output"', '"source":"unknown"') + "`n" + $codexReflectionLine),
        (($codexDetailLine -replace '"detail_sha256":"[0-9a-f]+"', '"detail_sha256":"ABC"') + "`n" + $codexReflectionLine),
        (($authEventsDetailLine -replace '"detail_byte_count":12', '"detail_byte_count":true') + "`n" + $authReflectionLine),
        (($authEventsDetailLine -replace '"detail_byte_count":12', '"detail_byte_count":-1') + "`n" + $authReflectionLine),
        (($authEventsDetailLine -replace '"detail_byte_count":12', '"detail_byte_count":1.5') + "`n" + $authReflectionLine),
        ($extraKeyDetailLine + "`n" + $authReflectionLine),
        ($authEventsDetailLine + "`n" + $quotaReflectionLine),
        ('{"component":true,"error_code":"HERMES_RUN_FAILED_HINT_AUTH","event":"reflection_rejected"}' + "`n" + $authEventsDetailLine),
        ('{"component":"Other","error_code":"HERMES_RUN_FAILED_HINT_AUTH","event":"reflection_rejected"}' + "`n" + $authEventsDetailLine),
        ('{"component":"Hermes","error_code":true,"event":"reflection_rejected"}' + "`n" + $authEventsDetailLine),
        ('{"event":"reflection_rejected"' + "`n" + $codexDetailLine),
        ('{"message":"\"event\":\"reflection_rejected\""}' + "`n" + $codexDetailLine)
    )
    foreach ($diagnosticRejectedCase in $diagnosticRejectedCases) {
        if (
            (Get-Task037HermesToolFailureGate -StderrText $diagnosticRejectedCase) -cne
            'success_tool_error_hermes_diagnostic_rejected'
        ) {
            throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
        }
    }
    $oversizeHermesDiagnostic = 'x' * ($task037MaxEvidenceFileBytes + 1)
    if (
        (Get-Task037HermesToolFailureGate -StderrText $oversizeHermesDiagnostic) -cne
        'success_tool_error_hermes_diagnostic_rejected'
    ) {
        throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
    }

    $parserSecretCanary = 'TASK037_SYNTHETIC_HERMES_PARSER_SECRET_CANARY'
    $parserCanaryResult = @(Get-Task037HermesToolFailureGate -StderrText (
        $parserSecretCanary + "`n" + $authEventsDetailLine + "`n" + $authReflectionLine
    ))
    if (
        $parserCanaryResult.Count -ne 1 -or
        $parserCanaryResult[0] -isnot [string] -or
        [string]$parserCanaryResult[0] -cne 'success_tool_error_hermes_hint_auth' -or
        [string]$parserCanaryResult[0] -like ('*' + $parserSecretCanary + '*')
    ) {
        throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
    }

    $adapterFailureCases = @(
        [pscustomobject]@{
            ErrorCode = 'HERMES_PRODUCTION_CHILD_EXITED'
            Gate = 'success_tool_error_hermes_production_child_exited'
        },
        [pscustomobject]@{
            ErrorCode = 'HERMES_PRODUCTION_DEADLINE_EXCEEDED'
            Gate = 'success_tool_error_hermes_production_deadline_exceeded'
        }
    )
    foreach ($adapterFailureCase in $adapterFailureCases) {
        $adapterReflectionLine = [ordered]@{
            component = 'Hermes'
            error_code = [string]$adapterFailureCase.ErrorCode
            event = 'reflection_rejected'
        } | ConvertTo-Json -Compress
        if (
            (Get-Task037HermesToolFailureGate -StderrText $adapterReflectionLine) -cne
            [string]$adapterFailureCase.Gate
        ) {
            throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
        }
    }

    $hermesComponentCases = @(
        [pscustomobject]@{
            Code = 'LATTICE_HERMES_REFLECTION_REJECTED'
            StderrText = $codexDetailLine + "`n" + $codexReflectionLine
            Gate = 'success_tool_error_hermes_hint_auth'
        },
        [pscustomobject]@{
            Code = 'LATTICE_HERMES_PRODUCTION_RUNNER_REQUIRED'
            StderrText = $codexDetailLine + "`n" + $codexReflectionLine
            Gate = 'success_tool_error_hermes_runtime'
        },
        [pscustomobject]@{
            Code = 'LATTICE_HERMES_MEMORY_RECEIPT_REJECTED'
            StderrText = $codexDetailLine + "`n" + $codexReflectionLine
            Gate = 'success_tool_error_hermes_memory_receipt'
        },
        [pscustomobject]@{
            Code = 'lattice_hermes_reflection_rejected'
            StderrText = $codexDetailLine + "`n" + $codexReflectionLine
            Gate = 'success_tool_error_hermes_unclassified'
        }
    )
    foreach ($hermesComponentCase in $hermesComponentCases) {
        $hermesComponentResult = @(Get-Task037HermesComponentFailureGate `
            -Code $hermesComponentCase.Code `
            -StderrText $hermesComponentCase.StderrText)
        if (
            $hermesComponentResult.Count -ne 1 -or
            $hermesComponentResult[0] -isnot [string] -or
            [string]$hermesComponentResult[0] -cne [string]$hermesComponentCase.Gate
        ) {
            throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
        }
    }

    $hermesSecretCanary = 'TASK037_SYNTHETIC_HERMES_SECRET_CANARY'
    $hermesNonParserResult = @(Get-Task037HermesComponentFailureGate `
        -Code 'LATTICE_HERMES_PRODUCTION_RUNNER_REQUIRED' `
        -StderrText $hermesSecretCanary)
    if (
        $hermesNonParserResult.Count -ne 1 -or
        $hermesNonParserResult[0] -isnot [string] -or
        [string]$hermesNonParserResult[0] -cne 'success_tool_error_hermes_runtime' -or
        [string]$hermesNonParserResult[0] -like ('*' + $hermesSecretCanary + '*')
    ) {
        throw 'TASK037_SELF_TEST_HERMES_FAILURE_GATE_REJECTED'
    }

    $structuredPayload = '{"status":"OK"}' | ConvertFrom-Json
    $structuredText = $structuredPayload | ConvertTo-Json -Compress
    $structuredContentItem = [pscustomobject]@{ type = 'text'; text = $structuredText }
    $structuredResponse = [pscustomobject]@{
        result = [pscustomobject]@{
            content = [object[]]@($structuredContentItem)
            structuredContent = $structuredPayload
            isError = $false
        }
    }
    $parsedStructured = Get-StructuredToolContent -Response $structuredResponse
    if ($parsedStructured.status -isnot [string] -or [string]$parsedStructured.status -cne 'OK') {
        throw 'TASK037_SELF_TEST_TOOL_CONTENT_TYPE_REJECTED'
    }
    foreach ($invalidStructuredResponse in @(
        [pscustomobject]@{
            result = [pscustomobject]@{
                content = $structuredContentItem
                structuredContent = $structuredPayload
                isError = $false
            }
        },
        [pscustomobject]@{
            result = [pscustomobject]@{
                content = [object[]]@([pscustomobject]@{ type = 'text'; text = $true })
                structuredContent = $structuredPayload
                isError = $false
            }
        },
        [pscustomobject]@{
            result = [pscustomobject]@{
                content = [object[]]@($structuredContentItem)
                structuredContent = $true
                isError = $false
            }
        }
    )) {
        $invalidToolContentFailure = $null
        try { $null = Get-StructuredToolContent -Response $invalidStructuredResponse }
        catch { $invalidToolContentFailure = $_.Exception.Message }
        if ($invalidToolContentFailure -cne 'TASK037_TOOL_CONTENT_SHAPE_REJECTED') {
            throw 'TASK037_SELF_TEST_TOOL_CONTENT_TYPE_REJECTED'
        }
    }

    $semanticRoot = Join-Path $tempBase ('lattice-task037-success-semantics-' + [Guid]::NewGuid().ToString('N'))
    try {
        $semanticDeliveryRoot = Join-Path $semanticRoot 'delivery'
        $semanticRepository = Join-Path $semanticDeliveryRoot 'repo'
        $semanticLauncher = Join-Path $semanticRoot 'codex.exe'
        New-Item -ItemType Directory -Path $semanticRepository -Force:$false | Out-Null
        [IO.File]::WriteAllBytes(
            (Join-Path $semanticRepository 'answer.txt'),
            [Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n")
        )
        $syntheticSuccess = [pscustomobject][ordered]@{
            baseline_commit = ('a' * 40)
            changed_paths = [object[]]@('answer.txt')
            codex_runtime = 'OFFICIAL_CODEX_APP_SERVER'
            commit_sha = ('b' * 40)
            component = 'lattice-delivery'
            configuration_digest = ('a' * 64)
            entrypoint = 'codex-app-mcp'
            entrypoint_classification = 'official-codex-app-live'
            entrypoint_runtime_kind = 'Live'
            full_chain_receipt_digest = ('b' * 64)
            graph_analysis_digest = ('c' * 64)
            graph_commit_sha = ('c' * 40)
            graph_database_identity_digest = ('d' * 64)
            graph_extension_manifest_digest = ('e' * 64)
            graph_persistence_digest = ('f' * 64)
            graph_project_id = 'task032-delivery'
            graph_query_digest = ('1' * 64)
            graph_receipt_digest = ('2' * 64)
            graph_record_count = [int32]1
            graph_result_count = [int32]1
            graph_retrieval_digest = ('3' * 64)
            graph_status = 'COMPLETED'
            hermes_findings = [object[]]@([pscustomobject]@{
                evidence_digest = ('4' * 64)
                statement = 'bounded finding'
            })
            hermes_graph_receipt_digest = ('5' * 64)
            hermes_identity_digest = ('6' * 64)
            hermes_input_digest = ('7' * 64)
            hermes_next_actions = [object[]]@('bounded action')
            hermes_provenance_status = 'PERSISTED_STRUCTURED_INFERENCE'
            hermes_reflection_digest = ('8' * 64)
            hermes_schema_version = 'lattice.hermes.reflection.v1'
            hermes_status = 'INFERENCE_CANDIDATE'
            hermes_summary = 'bounded summary'
            intent_digest = ('9' * 64)
            launcher_path = $semanticLauncher
            launcher_sha256 = $officialCodexSha256
            outcome_digest = ('a' * 64)
            parent_sha = ('d' * 40)
            profile = 'task032-codex-postgres-v1'
            receipt_digest = ('b' * 64)
            repository_path = $semanticRepository
            request_id = 'task032-request-synthetic'
            schema_bundle_sha256 = ('c' * 64)
            schema_file_count = [int32]1
            status = 'COMPLETED'
            test = 'FIXED_TEST_PASSED'
            test_command_id = 'git-diff-no-index-exact-answer-v1'
            thread_id = 'synthetic-thread'
            turn_id = 'synthetic-turn'
            version = $officialCodexVersion
        }
        $semanticActiveGate = 'success_tool_result'
        $semanticLastCompletedGate = 'phase_meta_projection'
        Assert-FullChainSuccess `
            -Evidence $syntheticSuccess `
            -RunId 'synthetic' `
            -DeliveryRoot $semanticDeliveryRoot `
            -DeliveryLauncher $semanticLauncher `
            -ActiveGate ([ref]$semanticActiveGate) `
            -LastCompletedGate ([ref]$semanticLastCompletedGate)
        if (
            $semanticActiveGate -cne 'success_answer_bytes' -or
            $semanticLastCompletedGate -cne 'success_action_items'
        ) {
            throw 'TASK037_SELF_TEST_SUCCESS_SEMANTICS_REJECTED'
        }

        $successSemanticCases = @(
            [pscustomobject]@{ Gate = 'success_launcher_binding'; Last = 'success_tool_result'; Mutate = { param($e) $e.launcher_path = 'C:\synthetic\wrong.exe' } },
            [pscustomobject]@{ Gate = 'success_delivery_contract'; Last = 'success_launcher_binding'; Mutate = { param($e) $e.status = 'completed' } },
            [pscustomobject]@{ Gate = 'success_delivery_contract'; Last = 'success_launcher_binding'; Mutate = { param($e) $e.schema_file_count = $true } },
            [pscustomobject]@{ Gate = 'success_delivery_contract'; Last = 'success_launcher_binding'; Mutate = { param($e) $e.schema_file_count = '1' } },
            [pscustomobject]@{ Gate = 'success_delivery_contract'; Last = 'success_launcher_binding'; Mutate = { param($e) $e.schema_file_count = 0.6 } },
            [pscustomobject]@{ Gate = 'success_delivery_contract'; Last = 'success_launcher_binding'; Mutate = { param($e) $e.changed_paths = 'answer.txt' } },
            [pscustomobject]@{ Gate = 'success_graph_contract'; Last = 'success_delivery_contract'; Mutate = { param($e) $e.graph_record_count = '1' } },
            [pscustomobject]@{ Gate = 'success_hermes_contract'; Last = 'success_graph_contract'; Mutate = { param($e) $e.hermes_status = 'inference_candidate' } },
            [pscustomobject]@{ Gate = 'success_hermes_contract'; Last = 'success_graph_contract'; Mutate = { param($e) $e.thread_id = $true } },
            [pscustomobject]@{ Gate = 'success_hermes_contract'; Last = 'success_graph_contract'; Mutate = { param($e) $e.hermes_findings = $e.hermes_findings[0] } },
            [pscustomobject]@{ Gate = 'success_hermes_contract'; Last = 'success_graph_contract'; Mutate = { param($e) $e.hermes_next_actions = 'bounded action' } },
            [pscustomobject]@{ Gate = 'success_digest_contract'; Last = 'success_hermes_contract'; Mutate = { param($e) $e.configuration_digest = ('A' * 64) } },
            [pscustomobject]@{ Gate = 'success_finding_items'; Last = 'success_digest_contract'; Mutate = { param($e) $e.hermes_findings[0].statement = $true } },
            [pscustomobject]@{ Gate = 'success_action_items'; Last = 'success_finding_items'; Mutate = { param($e) $e.hermes_next_actions[0] = $true } }
        )
        foreach ($semanticCase in $successSemanticCases) {
            $caseEvidence = ($syntheticSuccess | ConvertTo-Json -Compress -Depth 20) | ConvertFrom-Json
            & $semanticCase.Mutate $caseEvidence
            $caseActiveGate = 'success_tool_result'
            $caseLastCompletedGate = 'phase_meta_projection'
            $caseFailure = $null
            try {
                Assert-FullChainSuccess `
                    -Evidence $caseEvidence `
                    -RunId 'synthetic' `
                    -DeliveryRoot $semanticDeliveryRoot `
                    -DeliveryLauncher $semanticLauncher `
                    -ActiveGate ([ref]$caseActiveGate) `
                    -LastCompletedGate ([ref]$caseLastCompletedGate)
            }
            catch { $caseFailure = $_.Exception.Message }
            if (
                $caseFailure -cne 'TASK037_FULL_CHAIN_EVIDENCE_REJECTED' -or
                $caseActiveGate -cne [string]$semanticCase.Gate -or
                $caseLastCompletedGate -cne [string]$semanticCase.Last
            ) {
                throw 'TASK037_SELF_TEST_SUCCESS_SEMANTICS_REJECTED'
            }
        }

        [IO.File]::WriteAllBytes(
            (Join-Path $semanticRepository 'answer.txt'),
            [Text.Encoding]::ASCII.GetBytes("WRONG`n")
        )
        $answerActiveGate = 'success_tool_result'
        $answerLastCompletedGate = 'phase_meta_projection'
        $answerFailure = $null
        try {
            Assert-FullChainSuccess `
                -Evidence $syntheticSuccess `
                -RunId 'synthetic' `
                -DeliveryRoot $semanticDeliveryRoot `
                -DeliveryLauncher $semanticLauncher `
                -ActiveGate ([ref]$answerActiveGate) `
                -LastCompletedGate ([ref]$answerLastCompletedGate)
        }
        catch { $answerFailure = $_.Exception.Message }
        if (
            $answerFailure -cne 'TASK037_DELIVERY_ANSWER_BYTES_REJECTED' -or
            $answerActiveGate -cne 'success_answer_bytes' -or
            $answerLastCompletedGate -cne 'success_action_items'
        ) {
            throw 'TASK037_SELF_TEST_SUCCESS_SEMANTICS_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $semanticRoot) {
            Remove-SafeTempRoot -Path $semanticRoot | Out-Null
        }
    }

    $monitorRoot = Join-Path $tempBase ('lattice-task037-source-monitor-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $monitorRoot -Force:$false | Out-Null
        [IO.File]::WriteAllText((Join-Path $monitorRoot 'auth.json'), "{}`n", [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $monitorRoot 'config.toml'), "before`n", [Text.UTF8Encoding]::new($false))
        $monitorBefore = Get-CodexCredentialSourceEvidence -SourceHome $monitorRoot
        $monitor = Start-Task037CredentialSourceMonitor -SourceHome $monitorRoot
        [IO.File]::WriteAllText((Join-Path $monitorRoot 'config.toml'), "after`n", [Text.UTF8Encoding]::new($false))
        Stop-Task037CredentialSourceMonitor -Monitor $monitor -SourceHome $monitorRoot -Expected $monitorBefore
        $monitor = Start-Task037CredentialSourceMonitor -SourceHome $monitorRoot
        [IO.File]::WriteAllText((Join-Path $monitorRoot 'auth.json'), "{`"changed`":true}`n", [Text.UTF8Encoding]::new($false))
        $monitorWait = [Diagnostics.Stopwatch]::StartNew()
        while ($monitor.Queue.Count -eq 0 -and $monitorWait.ElapsedMilliseconds -lt 2000) { Start-Sleep -Milliseconds 10 }
        $monitorFailure = $null
        try { Stop-Task037CredentialSourceMonitor -Monitor $monitor -SourceHome $monitorRoot -Expected $monitorBefore }
        catch { $monitorFailure = $_.Exception.Message }
        if ($monitorFailure -cne 'TASK037_CODEX_CREDENTIAL_SOURCE_MUTATED') {
            throw 'TASK037_SELF_TEST_SOURCE_MONITOR_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $monitorRoot) { Remove-SafeTempRoot -Path $monitorRoot | Out-Null }
    }

    $phaseGateActive = 'phase_setup'
    $phaseGateLastCompleted = $null
    Enter-Task037PhaseGate `
        -Gate 'full_chain_gated_start' `
        -ActiveGate ([ref]$phaseGateActive) `
        -LastCompletedGate ([ref]$phaseGateLastCompleted)
    if (
        $phaseGateActive -cne 'full_chain_gated_start' -or
        $phaseGateLastCompleted -cne 'phase_setup'
    ) {
        throw 'TASK037_SELF_TEST_PHASE_GATE_REJECTED'
    }
    Complete-Task037PhaseGate `
        -ActiveGate ([ref]$phaseGateActive) `
        -LastCompletedGate ([ref]$phaseGateLastCompleted)
    if ($null -ne $phaseGateActive -or $phaseGateLastCompleted -cne 'full_chain_gated_start') {
        throw 'TASK037_SELF_TEST_PHASE_GATE_REJECTED'
    }
    $invalidPhaseGateFailure = $null
    try {
        Enter-Task037PhaseGate `
            -Gate 'forged_phase_gate' `
            -ActiveGate ([ref]$phaseGateActive) `
            -LastCompletedGate ([ref]$phaseGateLastCompleted)
    }
    catch { $invalidPhaseGateFailure = $_.Exception.Message }
    if ($invalidPhaseGateFailure -cne 'TASK037_PHASE_GATE_REJECTED') {
        throw 'TASK037_SELF_TEST_PHASE_GATE_REJECTED'
    }

    foreach ($invalidMcpJson in @('', '   ', '{')) {
        $invalidMcpJsonFailure = $null
        try { $null = ConvertFrom-Task037McpJson -Json $invalidMcpJson }
        catch { $invalidMcpJsonFailure = $_.Exception.Message }
        if ($invalidMcpJsonFailure -cne 'TASK037_MCP_RESPONSE_JSON_REJECTED') {
            throw 'TASK037_SELF_TEST_MCP_JSON_REJECTED'
        }
    }

    $runProbe = [pscustomobject]@{
        analyses = 1; receipts = 1; retrieval_audits = 1; records = 2; reflections = 1
        openclaw_commands = 1; reflection_receipt_digest = ('a' * 64)
    }
    $statusProbe = [pscustomobject]@{
        analyses = 1; receipts = 1; retrieval_audits = 1; records = 2; reflections = 1
        openclaw_commands = 2; reflection_receipt_digest = ('a' * 64)
    }
    $replayFailure = $null
    try { Assert-Task037MemoryReplayUnchanged -RunProbe $runProbe -StatusProbe $statusProbe }
    catch { $replayFailure = $_.Exception.Message }
    if ($replayFailure -cne 'TASK037_STATUS_REPLAY_REJECTED') {
        throw 'TASK037_SELF_TEST_MEMORY_REPLAY_REJECTED'
    }

    $processContractGates = @(
        'process_exit_snapshot',
        'wrapper_reap',
        'wrapper_exit_match',
        'openclaw_liveness',
        'child_exit_success',
        'process_handle_release',
        'job_cardinality'
    )
    for ($failureIndex = 0; $failureIndex -lt $processContractGates.Count; $failureIndex++) {
        $activeContractGate = 'process_wait'
        $lastCompletedContractGate = 'full_chain_gated_start'
        $contractFailure = $null
        for ($gateIndex = 0; $gateIndex -le $failureIndex; $gateIndex++) {
            $shouldPass = $gateIndex -lt $failureIndex
            try {
                Invoke-Task037ProcessContractCheck `
                    -Gate $processContractGates[$gateIndex] `
                    -Check { return [bool]$shouldPass } `
                    -ActiveGate ([ref]$activeContractGate) `
                    -LastCompletedGate ([ref]$lastCompletedContractGate)
            }
            catch {
                $contractFailure = $_.Exception.Message
                break
            }
        }
        $expectedLastCompleted = if ($failureIndex -eq 0) {
            'process_wait'
        }
        else {
            $processContractGates[$failureIndex - 1]
        }
        if (
            $contractFailure -cne 'TASK037_PROCESS_CONTRACT_REJECTED' -or
            $activeContractGate -cne $processContractGates[$failureIndex] -or
            $lastCompletedContractGate -cne $expectedLastCompleted
        ) {
            throw 'TASK037_SELF_TEST_PROCESS_CONTRACT_REJECTED'
        }
    }
    $activeContractGate = 'process_wait'
    $lastCompletedContractGate = 'full_chain_gated_start'
    foreach ($contractGate in $processContractGates) {
        Invoke-Task037ProcessContractCheck `
            -Gate $contractGate `
            -Check { return $true } `
            -ActiveGate ([ref]$activeContractGate) `
            -LastCompletedGate ([ref]$lastCompletedContractGate)
    }
    if (
        $activeContractGate -cne 'job_cardinality' -or
        $lastCompletedContractGate -cne 'process_handle_release'
    ) {
        throw 'TASK037_SELF_TEST_PROCESS_CONTRACT_REJECTED'
    }
    $invalidContractChecks = @(
        { return },
        { Write-Output $true; Write-Output $true },
        { return 'true' },
        { throw 'TASK037_SYNTHETIC_PROCESS_CONTRACT_THROW' }
    )
    foreach ($invalidContractCheck in $invalidContractChecks) {
        $activeContractGate = 'process_wait'
        $lastCompletedContractGate = 'full_chain_gated_start'
        $contractFailure = $null
        try {
            Invoke-Task037ProcessContractCheck `
                -Gate 'process_exit_snapshot' `
                -Check $invalidContractCheck `
                -ActiveGate ([ref]$activeContractGate) `
                -LastCompletedGate ([ref]$lastCompletedContractGate)
        }
        catch {
            $contractFailure = $_.Exception.Message
        }
        if (
            $contractFailure -cne 'TASK037_PROCESS_CONTRACT_REJECTED' -or
            $activeContractGate -cne 'process_exit_snapshot' -or
            $lastCompletedContractGate -cne 'process_wait'
        ) {
            throw 'TASK037_SELF_TEST_PROCESS_CONTRACT_REJECTED'
        }
    }
    $processContract = [pscustomobject]@{ ChildExitCode = 0 }
    $processContractMetaProbe = [ordered]@{
        process_exit_code = [int]$processContract.ChildExitCode
    }
    $processContractMetaJson = $processContractMetaProbe | ConvertTo-Json -Compress
    $processContractMetaRoundTrip = $processContractMetaJson | ConvertFrom-Json
    if (
        @($processContractMetaRoundTrip.PSObject.Properties).Count -ne 1 -or
        $processContractMetaRoundTrip.process_exit_code -isnot [int] -or
        [int]$processContractMetaRoundTrip.process_exit_code -ne 0
    ) {
        throw 'TASK037_SELF_TEST_PROCESS_CONTRACT_REJECTED'
    }

    $boolProbeRoot = Join-Path $tempBase ('lattice-task037-bool-probe-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $boolProbeRoot -Force:$false | Out-Null
        $boolAcceptanceId = 'a' * 32
        Write-JsonEvidence -Path (Join-Path $boolProbeRoot 'pre-status.started.json') -Value ([ordered]@{
            schema_version = 'lattice.task037.phase-started.v2'
            acceptance_id = $boolAcceptanceId
        })
        Write-JsonEvidence -Path (Join-Path $boolProbeRoot 'pre-status.cleanup.json') -Value ([ordered]@{
            schema_version = 'lattice.task037.cleanup.v2'
            acceptance_id = $boolAcceptanceId
            failure_gate = 'process_wait'
            last_completed_gate = 'full_chain_gated_start'
            job_active_process_count_zero = 'false'
            full_chain_process_stopped = 'true'
            openclaw_process_stopped = 'true'
            full_chain_controller_stopped = 'true'
            openclaw_controller_stopped = 'true'
        })
        if (Test-Task037StartedPhaseJobsZero -EvidenceRoot $boolProbeRoot -AcceptanceId $boolAcceptanceId) {
            throw 'TASK037_SELF_TEST_NATIVE_BOOLEAN_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $boolProbeRoot) { Remove-SafeTempRoot -Path $boolProbeRoot | Out-Null }
    }

    $assignFailureRoot = Join-Path $tempBase ('lattice-task037-assign-selftest-' + [Guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $assignFailureRoot -Force:$false | Out-Null
        $assignSentinel = Join-Path $assignFailureRoot 'assign.sentinel'
        $assignSource = '[IO.File]::WriteAllText($env:TASK037_SELFTEST_ASSIGN_SENTINEL, ''spawned'')'
        $assignEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($assignSource))
        $assignFailure = $null
        try {
            $null = Start-Task037GatedProcess `
                -Job ([IntPtr]::Zero) `
                -PowerShellExecutable $powerShellExecutable `
                -Executable $powerShellExecutable `
                -Arguments @('-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', $assignEncoded) `
                -WorkingDirectory $assignFailureRoot `
                -EnvironmentValues ([ordered]@{ TASK037_SELFTEST_ASSIGN_SENTINEL = $assignSentinel }) `
                -ChildPidPath (Join-Path $assignFailureRoot 'child.pid') `
                -ChildStdoutPath (Join-Path $assignFailureRoot 'child.stdout') `
                -ChildStderrPath (Join-Path $assignFailureRoot 'child.stderr') `
                -WrapperStdoutPath (Join-Path $assignFailureRoot 'wrapper.stdout') `
                -WrapperStderrPath (Join-Path $assignFailureRoot 'wrapper.stderr') `
                -RuntimeIsolationRoot (Join-Path $assignFailureRoot 'runtime')
        }
        catch {
            $assignFailure = $_.Exception.Message
        }
        if (
            $assignFailure -cne 'TASK037_JOB_ASSIGN_REJECTED' -or
            (Test-Path -LiteralPath $assignSentinel) -or
            (Test-Path -LiteralPath (Join-Path $assignFailureRoot 'child.pid'))
        ) {
            throw 'TASK037_SELF_TEST_ASSIGN_FAILURE_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $assignFailureRoot) {
            Remove-SafeTempRoot -Path $assignFailureRoot | Out-Null
        }
    }

    $ambientNodeOptions = [Environment]::GetEnvironmentVariable('NODE_OPTIONS', 'Process')
    try {
        [Environment]::SetEnvironmentVariable(
            'NODE_OPTIONS',
            '--require=C:\lattice-task037-node-options-must-not-run.js',
            'Process'
        )
        $expectedNode = Join-Path `
            ([Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles) `
            ) `
            'nodejs\node.exe'
        $expectedProgramData = [Environment]::GetFolderPath(
            [Environment+SpecialFolder]::CommonApplicationData
        )
        $trustedNode = Get-TrustedTask037Node `
            -Candidate $expectedNode `
            -ProgramDataAuthority $expectedProgramData
        if (-not (Test-ExactPath -Actual $trustedNode -Expected $expectedNode)) {
            throw 'TASK037_SELF_TEST_NODE_CLOSED_ENVIRONMENT_REJECTED'
        }
    }
    finally {
        [Environment]::SetEnvironmentVariable('NODE_OPTIONS', $ambientNodeOptions, 'Process')
    }

    $profileOverrideRoot = Join-Path $tempBase (
        'lattice-task037-profile-override-' + [Guid]::NewGuid().ToString('N')
    )
    try {
        $profileOverrideFailure = $null
        try {
            $profileOverrideInfo = [Diagnostics.ProcessStartInfo]::new()
            Set-Task037ClosedChildEnvironment `
                -StartInfo $profileOverrideInfo `
                -EnvironmentValues ([ordered]@{ APPDATA = 'C:\forged-appdata' }) `
                -Executable $powerShellExecutable `
                -IsolationRoot $profileOverrideRoot `
                -TrustedProgramData $expectedProgramData
        }
        catch { $profileOverrideFailure = $_.Exception.Message }
        if ($profileOverrideFailure -cne 'TASK037_CHILD_ENVIRONMENT_VALUE_REJECTED') {
            throw 'TASK037_SELF_TEST_PROFILE_OVERRIDE_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $profileOverrideRoot) {
            Remove-SafeTempRoot -Path $profileOverrideRoot | Out-Null
        }
    }

    $expectedGit = Join-Path `
        ([Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles) `
        ) `
        'Git\cmd\git.exe'
    $expectedCargoHome = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)) '.cargo'
    $expectedRustBin = Join-Path `
        ([Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)) `
        ('.rustup\toolchains\' + $task037RustToolchain + '\bin')
    $trustedRust = Get-TrustedTask037RustToolchain `
        -CargoCandidate (Join-Path $expectedRustBin 'cargo.exe') `
        -RustcCandidate (Join-Path $expectedRustBin 'rustc.exe') `
        -RustdocCandidate (Join-Path $expectedRustBin 'rustdoc.exe') `
        -CargoHome $expectedCargoHome
    Assert-Task037RustToolchainVersion -Toolchain $trustedRust
    if (
        -not (Test-ExactPath -Actual (Get-TrustedTask037Git -Candidate $expectedGit) -Expected $expectedGit) -or
        -not (Test-ExactPath -Actual $trustedRust.Cargo -Expected (Join-Path $expectedRustBin 'cargo.exe'))
    ) {
        throw 'TASK037_SELF_TEST_CONTROLLER_TOOL_IDENTITY_REJECTED'
    }

    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $closedGitBaseline = Get-Task037ClosedGitSnapshot `
        -GitExe $expectedGit `
        -RepositoryRoot $repositoryRoot `
        -AuthorizedTempBase $tempBase `
        -FailureCode 'TASK037_SELF_TEST_CLOSED_GIT_REJECTED'
    $ambientGitValues = @{}
    foreach ($name in @('GIT_CONFIG_COUNT', 'GIT_CONFIG_KEY_0', 'GIT_CONFIG_VALUE_0', 'GIT_DIR')) {
        $ambientGitValues[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    }
    try {
        [Environment]::SetEnvironmentVariable('GIT_CONFIG_COUNT', '1', 'Process')
        [Environment]::SetEnvironmentVariable('GIT_CONFIG_KEY_0', 'core.fsmonitor', 'Process')
        [Environment]::SetEnvironmentVariable(
            'GIT_CONFIG_VALUE_0',
            'C:\lattice-task037-forged-fsmonitor.exe',
            'Process'
        )
        [Environment]::SetEnvironmentVariable(
            'GIT_DIR',
            'C:\lattice-task037-forged-git-dir',
            'Process'
        )
        $closedGitInjected = Get-Task037ClosedGitSnapshot `
            -GitExe $expectedGit `
            -RepositoryRoot $repositoryRoot `
            -AuthorizedTempBase $tempBase `
            -FailureCode 'TASK037_SELF_TEST_CLOSED_GIT_REJECTED'
        if (
            -not (Test-ExactPath `
                -Actual $closedGitInjected.RepositoryRoot `
                -Expected $closedGitBaseline.RepositoryRoot) -or
            [string]$closedGitInjected.Branch -cne [string]$closedGitBaseline.Branch -or
            [string]$closedGitInjected.Head -cne [string]$closedGitBaseline.Head -or
            [bool]$closedGitInjected.Clean -ne [bool]$closedGitBaseline.Clean
        ) {
            throw 'TASK037_SELF_TEST_CLOSED_GIT_REJECTED'
        }
    }
    finally {
        foreach ($name in $ambientGitValues.Keys) {
            [Environment]::SetEnvironmentVariable($name, $ambientGitValues[$name], 'Process')
        }
    }

    $directoryShapeProbeRoot = Join-Path $tempBase (
        'lattice-task037-directory-shape-' + [Guid]::NewGuid().ToString('N')
    )
    $directoryShapeRepository = Join-Path $directoryShapeProbeRoot 'repository'
    $directoryShapeIsolation = Join-Path $directoryShapeProbeRoot 'git-isolation'
    try {
        New-Item -ItemType Directory -Path $directoryShapeProbeRoot -Force:$false | Out-Null
        $null = Set-Task037RestrictedDirectoryAcl -Path $directoryShapeProbeRoot
        foreach ($directory in @(
            $directoryShapeRepository,
            $directoryShapeIsolation,
            (Join-Path $directoryShapeIsolation 'home'),
            (Join-Path $directoryShapeIsolation 'temp')
        )) {
            New-Item -ItemType Directory -Path $directory -Force:$false | Out-Null
        }
        [IO.File]::WriteAllText(
            (Join-Path $directoryShapeRepository 'baseline.txt'),
            "task037-directory-shape`n",
            [Text.UTF8Encoding]::new($false)
        )
        foreach ($operation in @('InitializeSynthetic', 'AddSynthetic', 'CommitSynthetic')) {
            $output = Invoke-Task037ClosedGitText `
                -GitExe $expectedGit `
                -RepositoryRoot $directoryShapeRepository `
                -IsolationRoot $directoryShapeIsolation `
                -Operation $operation `
                -FailureCode 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
            if (-not [string]::IsNullOrEmpty($output)) {
                throw 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
            }
        }
        $closedSyntheticBaseline = Get-Task037ClosedGitSnapshot `
            -GitExe $expectedGit `
            -RepositoryRoot $directoryShapeRepository `
            -AuthorizedTempBase $tempBase `
            -FailureCode 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
        if (-not [bool]$closedSyntheticBaseline.Clean) {
            throw 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
        }
        $directoryShapeBaseline = Get-Task037RepositoryDirectoryShape `
            -RepositoryRoot $directoryShapeRepository `
            -FailureCode 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
        New-Item `
            -ItemType Directory `
            -Path (Join-Path $directoryShapeRepository 'Microsoft\Windows\PowerShell\ScheduledJobs') `
            -Force:$false | Out-Null
        $gitWithEmptyDirectory = Get-Task037ClosedGitSnapshot `
            -GitExe $expectedGit `
            -RepositoryRoot $directoryShapeRepository `
            -AuthorizedTempBase $tempBase `
            -FailureCode 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
        if (
            [string]$gitWithEmptyDirectory.Branch -cne [string]$closedSyntheticBaseline.Branch -or
            [string]$gitWithEmptyDirectory.Head -cne [string]$closedSyntheticBaseline.Head -or
            -not [bool]$gitWithEmptyDirectory.Clean
        ) {
            throw 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
        }
        $directoryShapeWithEmpty = Get-Task037RepositoryDirectoryShape `
            -RepositoryRoot $directoryShapeRepository `
            -FailureCode 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
        $directoryShapeFailure = $null
        try {
            Assert-Task037RepositoryDirectoryShapeUnchanged `
                -Before $directoryShapeBaseline `
                -After $directoryShapeWithEmpty `
                -FailureCode 'TASK037_SELF_TEST_DIRECTORY_SHAPE_CHANGED'
        }
        catch { $directoryShapeFailure = $_.Exception.Message }
        if ($directoryShapeFailure -cne 'TASK037_SELF_TEST_DIRECTORY_SHAPE_CHANGED') {
            throw 'TASK037_SELF_TEST_DIRECTORY_SHAPE_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $directoryShapeProbeRoot) {
            $removed = Remove-SafeTempRoot `
                -Path $directoryShapeProbeRoot `
                -AuthorizedTempBase $tempBase
            if (-not $removed) {
                throw 'TASK037_SELF_TEST_DIRECTORY_SHAPE_CLEANUP_REJECTED'
            }
        }
    }

    $cargoConfigProbeRoot = Join-Path $tempBase ('lattice-task037-cargo-config-' + [Guid]::NewGuid().ToString('N'))
    try {
        $cargoConfigProbeRepository = Join-Path $cargoConfigProbeRoot 'parent\repository'
        $cargoConfigProbeHome = Join-Path $cargoConfigProbeRoot 'cargo-home'
        $cargoConfigDirectory = Join-Path $cargoConfigProbeRoot 'parent\.cargo'
        foreach ($directory in @($cargoConfigProbeRepository, $cargoConfigProbeHome, $cargoConfigDirectory)) {
            New-Item -ItemType Directory -Path $directory -Force:$false | Out-Null
        }
        [IO.File]::WriteAllText(
            (Join-Path $cargoConfigDirectory 'config.toml'),
            "[build]`nrustc-wrapper = 'forged'`n",
            [Text.UTF8Encoding]::new($false)
        )
        $cargoConfigFailure = $null
        try {
            Assert-Task037CargoConfigurationClosure `
                -RepositoryRoot $cargoConfigProbeRepository `
                -CargoHome $cargoConfigProbeHome
        }
        catch { $cargoConfigFailure = $_.Exception.Message }
        if ($cargoConfigFailure -cne 'TASK037_CARGO_CONFIGURATION_REJECTED') {
            throw 'TASK037_SELF_TEST_CARGO_CONFIGURATION_REJECTED'
        }
    }
    finally {
        if (Test-Path -LiteralPath $cargoConfigProbeRoot) {
            Remove-SafeTempRoot -Path $cargoConfigProbeRoot | Out-Null
        }
    }

    $emptyArgumentsRoot = Join-Path $tempBase (
        'lattice-task037-empty-arguments-' + [Guid]::NewGuid().ToString('N')
    )
    $emptyArgumentsJob = [IntPtr]::Zero
    $emptyArgumentsOwned = $null
    $emptyArgumentsLateOwned = $null
    $emptyArgumentsBaseline = $null
    try {
        New-Item -ItemType Directory -Path $emptyArgumentsRoot -Force:$false | Out-Null
        $emptyArgumentsReleasePath = Join-Path $emptyArgumentsRoot 'release.signal'
        $emptyArgumentsInput = Join-Path $emptyArgumentsRoot 'stdin.js'
        $emptyArgumentsSource = @'
const fs = require('fs');
const releasePath = process.env.TASK037_SELFTEST_EMPTY_ARGUMENTS_RELEASE;
const deadline = Date.now() + 10000;
const waitCell = new Int32Array(new SharedArrayBuffer(4));
while (!fs.existsSync(releasePath)) {
    if (Date.now() >= deadline) process.exit(124);
    Atomics.wait(waitCell, 0, 0, 10);
}
if (fs.readFileSync(releasePath, 'utf8') !== 'release\n') process.exit(125);
'@
        [IO.File]::WriteAllText(
            $emptyArgumentsInput,
            $emptyArgumentsSource,
            [Text.UTF8Encoding]::new($false)
        )
        $emptyArgumentsJob = New-Task037KillOnCloseJob
        $emptyArgumentsOwned = Start-Task037GatedProcess `
            -Job $emptyArgumentsJob `
            -PowerShellExecutable $powerShellExecutable `
            -Executable $trustedNode `
            -Arguments @() `
            -WorkingDirectory $emptyArgumentsRoot `
            -EnvironmentValues ([ordered]@{
                TASK037_SELFTEST_EMPTY_ARGUMENTS_RELEASE = $emptyArgumentsReleasePath
            }) `
            -ChildPidPath (Join-Path $emptyArgumentsRoot 'child.pid') `
            -ChildStdoutPath (Join-Path $emptyArgumentsRoot 'child.stdout') `
            -ChildStderrPath (Join-Path $emptyArgumentsRoot 'child.stderr') `
            -WrapperStdoutPath (Join-Path $emptyArgumentsRoot 'wrapper.stdout') `
            -WrapperStderrPath (Join-Path $emptyArgumentsRoot 'wrapper.stderr') `
            -RuntimeIsolationRoot (Join-Path $emptyArgumentsRoot 'runtime') `
            -TrustedProgramData $expectedProgramData `
            -StandardInputPath $emptyArgumentsInput
        $emptyArgumentsLiveSnapshot = Get-Task037CoherentJobProcessSnapshot -Job $emptyArgumentsJob
        $emptyArgumentsBaseline = Get-Task037OpenClawJobBaseline `
            -Job $emptyArgumentsJob `
            -OpenClawProcess $emptyArgumentsOwned.ChildProcess `
            -OpenClawControllerProcess $emptyArgumentsOwned.Process `
            -TimeoutMilliseconds 1000
        $emptyArgumentsForgedIdentity = [pscustomobject]@{
            Process = $emptyArgumentsBaseline.ProcessIdentities[0].Process
            ProcessId = [uint32]$emptyArgumentsBaseline.ProcessIdentities[0].ProcessId
            ProcessStartTicks = [int64]$emptyArgumentsBaseline.ProcessIdentities[0].ProcessStartTicks + 1
        }
        if (
            [uint32]$emptyArgumentsLiveSnapshot.ActiveProcesses -lt 2 -or
            -not (Test-Task037ProcessIdSetEqual `
                -Left $emptyArgumentsBaseline.ProcessIds `
                -Right $emptyArgumentsLiveSnapshot.ProcessIds) -or
            (Test-Task037ProcessIdentityCohortLiveInJob `
                -Job $emptyArgumentsJob `
                -Identities @($emptyArgumentsForgedIdentity)) -or
            -not (Wait-Task037JobProcessBaseline `
                -Job $emptyArgumentsJob `
                -InitialSnapshot $emptyArgumentsLiveSnapshot `
                -BaselineProcessIds $emptyArgumentsLiveSnapshot.ProcessIds `
                -RequiredProcessIdentities $emptyArgumentsBaseline.ProcessIdentities `
                -TimeoutMilliseconds 1000) -or
            (Wait-Task037JobProcessBaseline `
                -Job $emptyArgumentsJob `
                -InitialSnapshot $emptyArgumentsLiveSnapshot `
                -BaselineProcessIds @() `
                -TimeoutMilliseconds 50)
        ) {
            throw 'TASK037_SELF_TEST_JOB_DRAIN_BARRIER_REJECTED'
        }
        $emptyArgumentsForgedTotal = [pscustomobject]@{
            TotalProcesses = [uint32]($emptyArgumentsLiveSnapshot.TotalProcesses + 1)
            ActiveProcesses = [uint32]$emptyArgumentsLiveSnapshot.ActiveProcesses
            ProcessIds = [uint32[]]@($emptyArgumentsLiveSnapshot.ProcessIds)
        }
        $emptyArgumentsMissingProcessIds = [uint32[]]@(
            $emptyArgumentsLiveSnapshot.ProcessIds |
                Select-Object -First ($emptyArgumentsLiveSnapshot.ProcessIds.Count - 1)
        )
        $emptyArgumentsForgedSet = [pscustomobject]@{
            TotalProcesses = [uint32]$emptyArgumentsLiveSnapshot.TotalProcesses
            ActiveProcesses = [uint32]$emptyArgumentsMissingProcessIds.Count
            ProcessIds = $emptyArgumentsMissingProcessIds
        }
        if (
            (Wait-Task037JobProcessBaseline `
                -Job $emptyArgumentsJob `
                -InitialSnapshot $emptyArgumentsForgedTotal `
                -BaselineProcessIds @() `
                -TimeoutMilliseconds 50) -or
            (Wait-Task037JobProcessBaseline `
                -Job $emptyArgumentsJob `
                -InitialSnapshot $emptyArgumentsForgedSet `
                -BaselineProcessIds @() `
                -TimeoutMilliseconds 50) -or
            (Test-Task037ValidProcessIdSet -ProcessIds @(
                [uint32]$emptyArgumentsLiveSnapshot.ProcessIds[0],
                [uint32]$emptyArgumentsLiveSnapshot.ProcessIds[0]
            ))
        ) {
            throw 'TASK037_SELF_TEST_JOB_DRAIN_AUTHORITY_REJECTED'
        }

        $emptyArgumentsLateReleasePath = Join-Path $emptyArgumentsRoot 'late-release.signal'
        $emptyArgumentsLateOwned = Start-Task037GatedProcess `
            -Job $emptyArgumentsJob `
            -PowerShellExecutable $powerShellExecutable `
            -Executable $trustedNode `
            -Arguments @() `
            -WorkingDirectory $emptyArgumentsRoot `
            -EnvironmentValues ([ordered]@{
                TASK037_SELFTEST_EMPTY_ARGUMENTS_RELEASE = $emptyArgumentsLateReleasePath
            }) `
            -ChildPidPath (Join-Path $emptyArgumentsRoot 'late-child.pid') `
            -ChildStdoutPath (Join-Path $emptyArgumentsRoot 'late-child.stdout') `
            -ChildStderrPath (Join-Path $emptyArgumentsRoot 'late-child.stderr') `
            -WrapperStdoutPath (Join-Path $emptyArgumentsRoot 'late-wrapper.stdout') `
            -WrapperStderrPath (Join-Path $emptyArgumentsRoot 'late-wrapper.stderr') `
            -RuntimeIsolationRoot (Join-Path $emptyArgumentsRoot 'late-runtime') `
            -TrustedProgramData $expectedProgramData `
            -StandardInputPath $emptyArgumentsInput
        if (Wait-Task037JobProcessBaseline `
            -Job $emptyArgumentsJob `
            -InitialSnapshot $emptyArgumentsLiveSnapshot `
            -BaselineProcessIds $emptyArgumentsBaseline.ProcessIds `
            -RequiredProcessIdentities $emptyArgumentsBaseline.ProcessIdentities `
            -TimeoutMilliseconds 50) {
            throw 'TASK037_SELF_TEST_JOB_DRAIN_TOTAL_INCREASE_REJECTED'
        }
        [IO.File]::WriteAllText(
            $emptyArgumentsLateReleasePath,
            "release`n",
            [Text.UTF8Encoding]::new($false)
        )
        Assert-NoReparsePath `
            -Path $emptyArgumentsLateReleasePath `
            -FailureCode 'TASK037_SELF_TEST_JOB_DRAIN_TOTAL_INCREASE_REJECTED'
        Assert-RegularFile -Path $emptyArgumentsLateReleasePath
        if (
            -not $emptyArgumentsLateOwned.ChildProcess.WaitForExit(10000) -or
            [int]$emptyArgumentsLateOwned.ChildProcess.ExitCode -ne 0 -or
            -not $emptyArgumentsLateOwned.Process.WaitForExit(10000)
        ) {
            throw 'TASK037_SELF_TEST_JOB_DRAIN_TOTAL_INCREASE_REJECTED'
        }
        Complete-Task037GatedProcessEvidence -OwnedProcess $emptyArgumentsLateOwned
        $emptyArgumentsLateReleaseState = [pscustomobject]@{
            ChildProcess = $emptyArgumentsLateOwned.ChildProcess
            ControllerProcess = $emptyArgumentsLateOwned.Process
        }
        if (-not (Release-Task037ExitedProcessHandles -State $emptyArgumentsLateReleaseState)) {
            throw 'TASK037_SELF_TEST_JOB_DRAIN_TOTAL_INCREASE_REJECTED'
        }
        $emptyArgumentsLateOwned.ChildProcess = $null
        $emptyArgumentsLateOwned.Process = $null
        $emptyArgumentsLiveAfterLateProcess = Get-Task037CoherentJobProcessSnapshot -Job $emptyArgumentsJob
        if (
            [uint32]$emptyArgumentsLiveAfterLateProcess.TotalProcesses -le
                [uint32]$emptyArgumentsLiveSnapshot.TotalProcesses -or
            -not (Test-Task037ProcessIdSetEqual `
                -Left $emptyArgumentsLiveAfterLateProcess.ProcessIds `
                -Right $emptyArgumentsBaseline.ProcessIds) -or
            -not (Test-Task037ProcessIdentityCohortLiveInJob `
                -Job $emptyArgumentsJob `
                -Identities $emptyArgumentsBaseline.ProcessIdentities)
        ) {
            throw 'TASK037_SELF_TEST_JOB_DRAIN_TOTAL_INCREASE_REJECTED'
        }
        $emptyArgumentsLiveSnapshot = $emptyArgumentsLiveAfterLateProcess

        [IO.File]::WriteAllText(
            $emptyArgumentsReleasePath,
            "release`n",
            [Text.UTF8Encoding]::new($false)
        )
        Assert-NoReparsePath `
            -Path $emptyArgumentsReleasePath `
            -FailureCode 'TASK037_SELF_TEST_EMPTY_ARGUMENTS_REJECTED'
        Assert-RegularFile -Path $emptyArgumentsReleasePath
        if (-not (Wait-Task037JobProcessBaseline `
            -Job $emptyArgumentsJob `
            -InitialSnapshot $emptyArgumentsLiveSnapshot `
            -BaselineProcessIds @() `
            -TimeoutMilliseconds 10000)) {
            throw 'TASK037_SELF_TEST_JOB_DRAIN_REJECTED'
        }
        if (
            -not $emptyArgumentsOwned.ChildProcess.WaitForExit(10000) -or
            [int]$emptyArgumentsOwned.ChildProcess.ExitCode -ne 0 -or
            -not $emptyArgumentsOwned.Process.WaitForExit(10000)
        ) {
            throw 'TASK037_SELF_TEST_EMPTY_ARGUMENTS_REJECTED'
        }
        foreach ($outputPath in @(
            (Join-Path $emptyArgumentsRoot 'child.stdout'),
            (Join-Path $emptyArgumentsRoot 'child.stderr')
        )) {
            Assert-RegularFile -Path $outputPath
            if ((Get-Item -LiteralPath $outputPath -Force).Length -ne 0) {
                throw 'TASK037_SELF_TEST_EMPTY_ARGUMENTS_REJECTED'
            }
        }
        $emptyArgumentsDrainSnapshot = Get-Task037CoherentJobProcessSnapshot -Job $emptyArgumentsJob
        if (
            [uint32]$emptyArgumentsDrainSnapshot.TotalProcesses -ne
                [uint32]$emptyArgumentsLiveSnapshot.TotalProcesses -or
            [uint32]$emptyArgumentsDrainSnapshot.ActiveProcesses -ne 0 -or
            (Test-Task037ProcessIdentityCohortLiveInJob `
                -Job $emptyArgumentsJob `
                -Identities $emptyArgumentsBaseline.ProcessIdentities)
        ) {
            throw 'TASK037_SELF_TEST_JOB_DRAIN_REJECTED'
        }
        Complete-Task037GatedProcessEvidence -OwnedProcess $emptyArgumentsOwned
        $emptyArgumentsReleaseState = [pscustomobject]@{
            ChildProcess = $emptyArgumentsOwned.ChildProcess
            ControllerProcess = $emptyArgumentsOwned.Process
        }
        if (-not (Release-Task037ExitedProcessHandles -State $emptyArgumentsReleaseState)) {
            throw 'TASK037_SELF_TEST_PROCESS_HANDLE_RELEASE_REJECTED'
        }
        $emptyArgumentsOwned.ChildProcess = $null
        $emptyArgumentsOwned.Process = $null
        foreach ($extraProcess in @($emptyArgumentsBaseline.ExtraProcesses)) {
            try { $extraProcess.Dispose() }
            catch { throw 'TASK037_SELF_TEST_PROCESS_HANDLE_RELEASE_REJECTED' }
        }
        $emptyArgumentsBaseline.ExtraProcesses = @()
        if (-not (Wait-Task037JobProcessBaseline `
            -Job $emptyArgumentsJob `
            -InitialSnapshot $emptyArgumentsDrainSnapshot `
            -BaselineProcessIds @())) {
            throw 'TASK037_SELF_TEST_PROCESS_HANDLE_RELEASE_REJECTED'
        }
        foreach ($specialFolder in @(
            (Join-Path $emptyArgumentsRoot 'runtime\home\AppData\Roaming'),
            (Join-Path $emptyArgumentsRoot 'runtime\home\AppData\Local')
        )) {
            $specialFolderItem = Get-Item -LiteralPath $specialFolder -Force -ErrorAction SilentlyContinue
            if (
                $null -eq $specialFolderItem -or
                -not $specialFolderItem.PSIsContainer -or
                ($specialFolderItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
            ) {
                throw 'TASK037_SELF_TEST_EMPTY_ARGUMENTS_REJECTED'
            }
        }
        if (
            (Get-Task037JobActiveProcessCount -Job $emptyArgumentsJob) -ne 0 -or
            (Test-Path -LiteralPath (Join-Path $emptyArgumentsRoot 'Microsoft'))
        ) {
            throw 'TASK037_SELF_TEST_EMPTY_ARGUMENTS_REJECTED'
        }
    }
    finally {
        if ($emptyArgumentsJob -ne [IntPtr]::Zero) {
            try { Stop-Task037Job -Job $emptyArgumentsJob } catch { }
            try { Close-Task037Job -Job $emptyArgumentsJob } catch { }
        }
        if ($null -ne $emptyArgumentsOwned) {
            try { $emptyArgumentsOwned.ChildProcess.Dispose() } catch { }
            try { $emptyArgumentsOwned.Process.Dispose() } catch { }
        }
        if ($null -ne $emptyArgumentsLateOwned) {
            try { $emptyArgumentsLateOwned.ChildProcess.Dispose() } catch { }
            try { $emptyArgumentsLateOwned.Process.Dispose() } catch { }
        }
        if ($null -ne $emptyArgumentsBaseline) {
            foreach ($extraProcess in @($emptyArgumentsBaseline.ExtraProcesses)) {
                try { $extraProcess.Dispose() } catch { }
            }
        }
        if (Test-Path -LiteralPath $emptyArgumentsRoot) {
            Remove-SafeTempRoot -Path $emptyArgumentsRoot | Out-Null
        }
    }

    Write-Output 'TASK037_VERIFIER_CONTAINMENT_SELF_TEST=PASS'
}

function Invoke-FullChainInternalPhase {
    param([Parameter(Mandatory = $true)][string]$Phase)

    Assert-InternalEnvironment
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
    $acceptanceId = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_ACCEPTANCE_ID', 'Process')
    $tempAcceptanceRootValue = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_TEMP_ACCEPTANCE_ROOT', 'Process')
    if ([string]::IsNullOrWhiteSpace($tempAcceptanceRootValue)) {
        throw 'TASK037_SUPERVISOR_BINDING_REJECTED'
    }
    $tempAcceptanceRoot = Get-CanonicalPath -Path $tempAcceptanceRootValue
    Assert-Task037SupervisorBinding `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -TempRoot $tempAcceptanceRoot
    $runId = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_RUN_ID', 'Process')
    $hostName = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_HOST', 'Process')
    $port = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PORT', 'Process')
    $postgresPassword = [Environment]::GetEnvironmentVariable('LATTICE_TASK019_PASSWORD', 'Process')
    $evidenceRoot = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_EVIDENCE_ROOT', 'Process'))
    $fixtureRoot = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_FIXTURE_ROOT', 'Process'))
    $fullChainExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_FULL_CHAIN_EXE', 'Process'))
    $brokerExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_HERMES_BROKER_EXE', 'Process'))
    $runtimeManifest = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_RUNTIME_MANIFEST', 'Process'))
    $deliveryLauncher = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_DELIVERY_LAUNCHER', 'Process'))
    $deliveryCodexHome = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_DELIVERY_CODEX_HOME', 'Process'))
    $schemaDirectory = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_SCHEMA_DIR', 'Process'))
    $deliveryRoot = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_DELIVERY_ROOT', 'Process'))
    $gitExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_GIT_EXE', 'Process'))
    $nodeExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_NODE_EXE', 'Process'))
    $openClawCli = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_OPENCLAW_CLI', 'Process'))
    $codexAuthHome = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CODEX_AUTH_HOME', 'Process'))
    $wslExe = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_WSL_EXE', 'Process'))

    foreach ($path in @(
        $evidenceRoot, $fixtureRoot, $fullChainExe, $brokerExe, $runtimeManifest,
        $deliveryLauncher, $schemaDirectory, $deliveryRoot, $openClawCli
    )) {
        Assert-NoReparseAncestor -Path $path -Boundary $repositoryRoot
    }
    foreach ($path in @($fullChainExe, $brokerExe, $runtimeManifest, $deliveryLauncher, $gitExe, $nodeExe, $openClawCli, $wslExe)) {
        Assert-RegularFile -Path $path
    }
    $programDataAuthority = Get-CanonicalPath -Path (
        [Environment]::GetEnvironmentVariable(
            'LATTICE_TASK037_PROGRAM_DATA',
            'Process'
        )
    )
    $nodeExe = Get-TrustedTask037Node `
        -Candidate $nodeExe `
        -ProgramDataAuthority $programDataAuthority
    if ((Get-FileSha256 $runtimeManifest) -ne $hermesRuntimeManifestSha256) {
        throw 'TASK037_HERMES_RUNTIME_MANIFEST_REJECTED'
    }
    if ((Get-FileSha256 $deliveryLauncher) -ne $officialCodexSha256) {
        throw 'TASK037_DELIVERY_LAUNCHER_REJECTED'
    }
    if ((Get-FileSha256 $openClawCli) -ne $openClawEntrypointSha256) {
        throw 'TASK037_OPENCLAW_ENTRYPOINT_REJECTED'
    }

    $toolName = if ($Phase -eq 'FullChainRun') { 'lattice_delivery_run' } else { 'lattice_delivery_status' }
    $runMode = if ($Phase -eq 'FullChainRun') { 'FRESH' } else { 'RESUME_EXISTING' }
    $expectError = $Phase -eq 'FullChainPreStatus'
    $phaseName = switch ($Phase) {
        'FullChainPreStatus' { 'pre-status' }
        'FullChainRun' { 'run' }
        'FullChainStatus' { 'status' }
    }
    $phaseId = $acceptanceId + '-' + $phaseName + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $tempBaseValue = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_SUPERVISOR_TEMP_BASE', 'Process')
    if ([string]::IsNullOrWhiteSpace($tempBaseValue)) {
        throw 'TASK037_SUPERVISOR_BINDING_TEMP_BASE_REJECTED'
    }
    $tempBase = Get-CanonicalPath -Path $tempBaseValue
    $expectedTempAcceptanceRoot = Get-Task037BoundTempRoot `
        -AcceptanceId $acceptanceId `
        -TempBase $tempBase
    if (-not (Test-ExactPath -Actual $tempAcceptanceRoot -Expected $expectedTempAcceptanceRoot)) {
        throw 'TASK037_SUPERVISOR_BINDING_REJECTED'
    }
    Assert-NoReparseAncestor -Path $deliveryCodexHome -Boundary $tempBase
    $phaseRoot = Join-Path $tempAcceptanceRoot ('phase-' + $phaseId)
    $profileRoot = Join-Path $phaseRoot 'openclaw'
    $hermesIsolationRoot = Join-Path $phaseRoot 'hermes'
    $brokerIsolationRoot = Join-Path $phaseRoot 'broker'
    $brokerCodexHome = Join-Path $phaseRoot 'broker-codex-home'
    $rawLogRoot = Join-Path $phaseRoot 'raw'
    $fullChainEnvironmentRoot = Join-Path $phaseRoot 'full-chain-environment'
    if (Test-Path -LiteralPath $phaseRoot) {
        throw ('TASK037_FRESH_TEMP_REJECTED|' + $phaseRoot)
    }
    New-Item -ItemType Directory -Path $phaseRoot -ErrorAction Stop | Out-Null
    $null = Get-Task037RestrictedAclEvidence -Path $phaseRoot
    foreach ($freshPath in @(
        $profileRoot, $hermesIsolationRoot, $brokerIsolationRoot, $brokerCodexHome,
        $rawLogRoot, $fullChainEnvironmentRoot
    )) {
        if (Test-Path -LiteralPath $freshPath) {
            throw ('TASK037_FRESH_TEMP_REJECTED|' + $freshPath)
        }
    }
    New-Item -ItemType Directory -Path $rawLogRoot -ErrorAction Stop | Out-Null
    $null = Get-Task037RestrictedAclEvidence -Path $rawLogRoot
    $openClawStdout = Join-Path $rawLogRoot 'openclaw.stdout.log'
    $openClawStderr = Join-Path $rawLogRoot 'openclaw.stderr.log'
    $inputPath = Join-Path $evidenceRoot ($phaseName + '.input.ndjson')
    $stdoutPath = Join-Path $rawLogRoot 'response.ndjson'
    $stderrPath = Join-Path $rawLogRoot 'full-chain.stderr.log'
    $metaPath = Join-Path $evidenceRoot ($phaseName + '.meta.json')
    $cleanupPath = Join-Path $evidenceRoot ($phaseName + '.cleanup.json')
    $phaseStartedPath = Join-Path $evidenceRoot ($phaseName + '.started.json')
    $brokerOwnerRecordPath = Join-Path $evidenceRoot ($phaseName + '.broker.credential-owner.json')
    $outputJsonPath = Join-Path $evidenceRoot ($phaseName + '.json')
    $memoryProbePath = Join-Path $evidenceRoot ('memory-' + $phaseName + '.json')
    $openClawChildPidPath = Join-Path $evidenceRoot ($phaseName + '.openclaw.child.pid')
    $openClawWrapperStdout = Join-Path $rawLogRoot 'openclaw.wrapper.stdout.log'
    $openClawWrapperStderr = Join-Path $rawLogRoot 'openclaw.wrapper.stderr.log'
    $fullChainChildPidPath = Join-Path $evidenceRoot ($phaseName + '.full-chain.child.pid')
    $fullChainWrapperStdout = Join-Path $rawLogRoot 'full-chain.wrapper.stdout.log'
    $fullChainWrapperStderr = Join-Path $rawLogRoot 'full-chain.wrapper.stderr.log'
    $logPublicationPlan = @(
        [pscustomobject]@{ Source = $openClawStdout; ArtifactId = 'openclaw.stdout' },
        [pscustomobject]@{ Source = $openClawStderr; ArtifactId = 'openclaw.stderr' },
        [pscustomobject]@{ Source = $openClawWrapperStdout; ArtifactId = 'openclaw.wrapper.stdout' },
        [pscustomobject]@{ Source = $openClawWrapperStderr; ArtifactId = 'openclaw.wrapper.stderr' },
        [pscustomobject]@{ Source = $stdoutPath; ArtifactId = 'full-chain.response' },
        [pscustomobject]@{ Source = $stderrPath; ArtifactId = 'full-chain.stderr' },
        [pscustomobject]@{ Source = $fullChainWrapperStdout; ArtifactId = 'full-chain.wrapper.stdout' },
        [pscustomobject]@{ Source = $fullChainWrapperStderr; ArtifactId = 'full-chain.wrapper.stderr' }
    )

    $phaseJob = [IntPtr]::Zero
    $jobProcessCountZero = $false
    $openClawOwned = $null
    $fullChainOwned = $null
    $fullChainEvidenceCompleted = $false
    $openClawControllerProcess = $null
    $fullChainControllerProcess = $null
    $openClawProcess = $null
    $fullChainProcess = $null
    $openClawPid = 0
    $fullChainPid = 0
    $openClawJobBaseline = $null
    $openClawBaselineExtraProcesses = @()
    $wsPort = 0
    $rustPort = 0
    $failure = $null
    $failureEvidence = $null
    $toolIsError = $null
    $toolCode = $null
    $responseDigest = $null
    $phaseCleanupFailure = $null
    $activeGate = 'phase_setup'
    $lastCompletedGate = $null
    $brokerCredentialSourceBefore = $null
    $brokerCredentialOwnership = $null
    $rawLogRootRemoved = $false
    $phaseController = [Diagnostics.Process]::GetCurrentProcess()
    Write-JsonEvidence -Path $phaseStartedPath -Value ([ordered]@{
        schema_version = 'lattice.task037.phase-started.v2'
        acceptance_id = $acceptanceId
        phase = $Phase
        phase_id = $phaseId
        controller_process_id = [int]$phaseController.Id
        controller_process_start_ticks = [int64]$phaseController.StartTime.ToUniversalTime().Ticks
        raw_log_root = $rawLogRoot
    })
    $phaseController.Dispose()
    try {
        if ($Phase -eq 'FullChainPreStatus') {
            Reset-PostgresMemoryRows `
                -RunId $runId `
                -Password $postgresPassword `
                -HostName $hostName `
                -Port $port `
                -EvidenceRoot $evidenceRoot
        }
        $pluginRoot = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'integrations\openclaw-lattice')
        $openClawWorkingDirectory = Get-CanonicalPath -Path (Split-Path -Parent $openClawCli)
        $profileSha256 = New-OpenClawProfile -ProfileRoot $profileRoot -PluginRoot $pluginRoot
        $gatewayToken = New-RandomHex -Bytes 32
        $openClawAuthKey = New-RandomBytes -Count 32
        $openClawAttestationKey = New-RandomBytes -Count 32
        $openClawNonce = New-RandomBytes -Count 16
        $hermesApiKey = New-RandomHex -Bytes 32
        Register-Task037AuthSecrets -AuthPath (Join-Path $codexAuthHome 'auth.json')
        foreach ($secret in @(
            $postgresPassword,
            $gatewayToken,
            (ConvertTo-LowerHex $openClawAuthKey),
            (ConvertTo-LowerHex $openClawAttestationKey),
            (ConvertTo-LowerHex $openClawNonce),
            $hermesApiKey
        )) {
            Add-Task037KnownSecret -Value ([string]$secret)
        }
        $sessionEpochBytes = New-RandomBytes -Count 7
        $sessionEpoch = [Convert]::ToUInt64((ConvertTo-LowerHex $sessionEpochBytes), 16)
        if ($sessionEpoch -eq 0) {
            $sessionEpoch = 1
        }
        $launchRecordId = 'lattice-' + $phaseId
        $wsPort = Get-UnreservedLoopbackPort
        do {
            $rustPort = Get-UnreservedLoopbackPort
        } while ($rustPort -eq $wsPort)
        $brokerCredentialSourceBefore = Get-CodexCredentialSourceEvidence -SourceHome $codexAuthHome
        $brokerCredentialOwnership = Copy-CodexCredentialFiles `
            -SourceHome $codexAuthHome `
            -DestinationHome $brokerCodexHome `
            -OwnerToken ($phaseId + '-broker') `
            -OwnerRecordPath $brokerOwnerRecordPath `
            -ForBroker `
            -AuthorizedTempBase $tempBase
        $powerShellExecutable = Get-TrustedTask037PowerShell
        $phaseJob = New-Task037KillOnCloseJob

        $openClawOwned = Start-IsolatedOpenClaw `
            -Job $phaseJob `
            -PowerShellExecutable $powerShellExecutable `
            -NodeExe $nodeExe `
            -OpenClawCli $openClawCli `
            -OpenClawWorkingDirectory $openClawWorkingDirectory `
            -ProfileRoot $profileRoot `
            -GatewayToken $gatewayToken `
            -TransportKeyHex (ConvertTo-LowerHex $openClawAuthKey) `
            -ProcessNonceHex (ConvertTo-LowerHex $openClawNonce) `
            -LaunchRecordId $launchRecordId `
            -WsPort $wsPort `
            -RustPort $rustPort `
            -ChildPidPath $openClawChildPidPath `
            -StdoutPath $openClawStdout `
            -StderrPath $openClawStderr `
            -WrapperStdoutPath $openClawWrapperStdout `
            -WrapperStderrPath $openClawWrapperStderr `
            -RuntimeIsolationRoot $profileRoot `
            -TrustedProgramData $programDataAuthority
        $openClawControllerProcess = $openClawOwned.Process
        $openClawProcess = $openClawOwned.ChildProcess
        $openClawPid = $openClawOwned.ChildProcessId
        $null = $openClawProcess.Handle
        $gatewayStopwatch = [Diagnostics.Stopwatch]::StartNew()
        while ($gatewayStopwatch.ElapsedMilliseconds -lt 90000) {
            if ($openClawProcess.HasExited) {
                throw ('TASK037_OPENCLAW_GATEWAY_EARLY_EXIT_' + $openClawProcess.ExitCode)
            }
            if (Test-LoopbackPort -Port $wsPort) {
                break
            }
            Start-Sleep -Milliseconds 250
        }
        if (-not (Test-LoopbackPort -Port $wsPort)) {
            throw 'TASK037_OPENCLAW_GATEWAY_NOT_READY'
        }

        Enter-Task037PhaseGate `
            -Gate 'openclaw_job_baseline' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $openClawJobBaseline = Get-Task037OpenClawJobBaseline `
            -Job $phaseJob `
            -OpenClawProcess $openClawProcess `
            -OpenClawControllerProcess $openClawControllerProcess
        $openClawBaselineExtraProcesses = @($openClawJobBaseline.ExtraProcesses)

        $attestationTag = Get-AttestationTag `
            -Key $openClawAttestationKey `
            -LaunchRecordId $launchRecordId `
            -ProcessId ([uint32]$openClawPid) `
            -ProcessNonce $openClawNonce `
            -ProfileSha256 $profileSha256

        [IO.File]::WriteAllBytes($inputPath, (Get-McpInputBytes -ToolName $toolName))
        $environment = [ordered]@{
            CI = '1'
            NO_COLOR = '1'
            LATTICE_FULL_CHAIN_RUN_MODE = $runMode
            LATTICE_DELIVERY_CODEX_MODE = 'OFFICIAL_CODEX_APP_SERVER'
            LATTICE_DELIVERY_LAUNCHER = $deliveryLauncher
            LATTICE_DELIVERY_LAUNCHER_VERSION = $officialCodexVersion
            LATTICE_DELIVERY_LAUNCHER_SHA256 = $officialCodexSha256
            LATTICE_DELIVERY_SCHEMA_DIR = $schemaDirectory
            LATTICE_DELIVERY_CODEX_HOME = $deliveryCodexHome
            LATTICE_DELIVERY_ROOT = $deliveryRoot
            LATTICE_DELIVERY_GIT_EXE = $gitExe
            LATTICE_DELIVERY_TIMEOUT_SECONDS = '300'
            LATTICE_TASK019_HOST = $hostName
            LATTICE_TASK019_PORT = $port
            LATTICE_TASK019_RUN_ID = $runId
            LATTICE_TASK019_PASSWORD = $postgresPassword
            LATTICE_HERMES_RUNTIME_MANIFEST = $runtimeManifest
            LATTICE_HERMES_RUNTIME_GUEST_ROOT = $hermesRuntimeGuestRoot
            LATTICE_HERMES_API_KEY = $hermesApiKey
            LATTICE_HERMES_PRODUCT_ROOT = $repositoryRoot
            LATTICE_HERMES_WSL_EXE = $wslExe
            LATTICE_HERMES_ISOLATION_ROOT = $hermesIsolationRoot
            LATTICE_HERMES_BROKER_HELPER = $brokerExe
            LATTICE_HERMES_BROKER_HELPER_SHA256 = (Get-FileSha256 $brokerExe)
            LATTICE_HERMES_CODEX_LAUNCHER = $deliveryLauncher
            LATTICE_HERMES_CODEX_HOME = $brokerCodexHome
            LATTICE_HERMES_BROKER_ISOLATION_ROOT = $brokerIsolationRoot
            LATTICE_HERMES_DEADLINE_SECONDS = '300'
            LATTICE_OPENCLAW_AUTH_KEY_HEX = (ConvertTo-LowerHex $openClawAuthKey)
            LATTICE_OPENCLAW_LAUNCH_ATTESTATION_KEY_HEX = (ConvertTo-LowerHex $openClawAttestationKey)
            LATTICE_OPENCLAW_LAUNCH_ATTESTATION_TAG_HEX = $attestationTag
            LATTICE_OPENCLAW_PROCESS_START_NONCE = (ConvertTo-LowerHex $openClawNonce)
            LATTICE_OPENCLAW_LAUNCH_RECORD_ID = $launchRecordId
            LATTICE_OPENCLAW_PROCESS_ID = [string]$openClawPid
            LATTICE_OPENCLAW_PACKAGE_TARBALL_SHA256 = $openClawPackageTarballSha256
            LATTICE_OPENCLAW_ENTRYPOINT_SHA256 = $openClawEntrypointSha256
            LATTICE_OPENCLAW_PROFILE_SHA256 = $profileSha256
            LATTICE_OPENCLAW_SESSION_EPOCH = $sessionEpoch.ToString([Globalization.CultureInfo]::InvariantCulture)
            LATTICE_OPENCLAW_GATEWAY_INSTANCE_ID = 'official-' + $phaseId
            LATTICE_OPENCLAW_ACTOR_ID = 'user-' + $phaseId
            LATTICE_OPENCLAW_CHANNEL_ID = 'codex-app'
            LATTICE_OPENCLAW_SESSION_ID = 'agent:main:main'
            LATTICE_OPENCLAW_GATEWAY_PORT = [string]$rustPort
            LATTICE_OPENCLAW_DEADLINE_MS = '10000'
            GIT_CONFIG_NOSYSTEM = '1'
            GIT_CONFIG_GLOBAL = 'NUL'
            GIT_CONFIG_SYSTEM = 'NUL'
            GIT_TERMINAL_PROMPT = '0'
        }
        Enter-Task037PhaseGate `
            -Gate 'full_chain_gated_start' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $fullChainOwned = Start-Task037GatedProcess `
            -Job $phaseJob `
            -PowerShellExecutable $powerShellExecutable `
            -Executable $fullChainExe `
            -Arguments @() `
            -WorkingDirectory $repositoryRoot `
            -EnvironmentValues $environment `
            -ChildPidPath $fullChainChildPidPath `
            -ChildStdoutPath $stdoutPath `
            -ChildStderrPath $stderrPath `
            -WrapperStdoutPath $fullChainWrapperStdout `
            -WrapperStderrPath $fullChainWrapperStderr `
            -RuntimeIsolationRoot $fullChainEnvironmentRoot `
            -TrustedProgramData $programDataAuthority `
            -StandardInputPath $inputPath
        $fullChainControllerProcess = $fullChainOwned.Process
        $fullChainProcess = $fullChainOwned.ChildProcess
        $fullChainPid = $fullChainOwned.ChildProcessId
        $null = $fullChainProcess.Handle
        Enter-Task037PhaseGate `
            -Gate 'process_wait' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $timeoutMilliseconds = if ($Phase -eq 'FullChainRun') { 540000 } else { 360000 }
        if (-not $fullChainProcess.WaitForExit($timeoutMilliseconds)) {
            throw ('TASK037_' + $Phase.ToUpperInvariant() + '_TIMEOUT')
        }
        $processContract = [pscustomobject]@{
            ChildExitCode = 0
            ChildProcess = $fullChainProcess
            ControllerProcess = $fullChainControllerProcess
            DrainSnapshot = $null
            EvidenceCompleted = $false
        }
        Invoke-Task037ProcessContractCheck `
            -Gate 'process_exit_snapshot' `
            -Check {
                $processContract.ChildExitCode = [int]$fullChainProcess.ExitCode
                return $true
            } `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        Invoke-Task037ProcessContractCheck `
            -Gate 'wrapper_reap' `
            -Check { return [bool]$fullChainControllerProcess.WaitForExit(5000) } `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        Invoke-Task037ProcessContractCheck `
            -Gate 'wrapper_exit_match' `
            -Check {
                return (
                    [int]$fullChainControllerProcess.ExitCode -eq
                    [int]$processContract.ChildExitCode
                )
            } `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        Invoke-Task037ProcessContractCheck `
            -Gate 'openclaw_liveness' `
            -Check {
                return (
                    -not $openClawProcess.HasExited -and
                    -not $openClawControllerProcess.HasExited
                )
            } `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        Invoke-Task037ProcessContractCheck `
            -Gate 'child_exit_success' `
            -Check { return [int]$processContract.ChildExitCode -eq 0 } `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        try {
            Invoke-Task037ProcessContractCheck `
                -Gate 'process_handle_release' `
                -Check {
                    Complete-Task037GatedProcessEvidence -OwnedProcess $fullChainOwned
                    $processContract.EvidenceCompleted = $true
                    $processContract.DrainSnapshot = Get-Task037CoherentJobProcessSnapshot -Job $phaseJob
                    return Release-Task037ExitedProcessHandles -State $processContract
                } `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
        }
        finally {
            $fullChainProcess = $processContract.ChildProcess
            $fullChainControllerProcess = $processContract.ControllerProcess
            $fullChainEvidenceCompleted = [bool]$processContract.EvidenceCompleted
            if ($null -ne $fullChainOwned) {
                $fullChainOwned.ChildProcess = $processContract.ChildProcess
                $fullChainOwned.Process = $processContract.ControllerProcess
            }
        }
        Invoke-Task037ProcessContractCheck `
            -Gate 'job_cardinality' `
            -Check {
                return Wait-Task037JobProcessBaseline `
                    -Job $phaseJob `
                    -InitialSnapshot $processContract.DrainSnapshot `
                    -BaselineProcessIds $openClawJobBaseline.ProcessIds `
                    -RequiredProcessIdentities $openClawJobBaseline.ProcessIdentities
            } `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        Enter-Task037PhaseGate `
            -Gate 'raw_bounds' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        Assert-BoundedTask037EvidenceFile -Path $stdoutPath -Code 'TASK037_FULL_CHAIN_STDOUT_OVERSIZE_REJECTED'
        Assert-BoundedTask037EvidenceFile -Path $stderrPath -Code 'TASK037_FULL_CHAIN_STDERR_OVERSIZE_REJECTED'
        Assert-BoundedTask037EvidenceFile -Path $openClawStdout -Code 'TASK037_OPENCLAW_STDOUT_OVERSIZE_REJECTED'
        Assert-BoundedTask037EvidenceFile -Path $openClawStderr -Code 'TASK037_OPENCLAW_STDERR_OVERSIZE_REJECTED'
        $stdout = [IO.File]::ReadAllText($stdoutPath, [Text.Encoding]::UTF8)
        $stderr = [IO.File]::ReadAllText($stderrPath, [Text.Encoding]::UTF8)
        Enter-Task037PhaseGate `
            -Gate 'response_json' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $responseLines = @($stdout -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $responses = @($responseLines | ForEach-Object { ConvertFrom-Task037McpJson -Json ([string]$_) })
        Enter-Task037PhaseGate `
            -Gate 'mcp_envelope' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        if (
            $responses.Count -ne 2 -or
            ($responses[0].id -isnot [int] -and $responses[0].id -isnot [long]) -or
            ($responses[1].id -isnot [int] -and $responses[1].id -isnot [long]) -or
            [int64]$responses[0].id -ne 1 -or
            [int64]$responses[1].id -ne 2
        ) {
            throw 'TASK037_MCP_RESPONSE_SHAPE_REJECTED'
        }
        foreach ($response in $responses) {
            Assert-Task037ExactPropertySet `
                -Value $response `
                -Expected @('id', 'jsonrpc', 'result') `
                -FailureCode 'TASK037_MCP_RESPONSE_SHAPE_REJECTED'
            if ([string]$response.jsonrpc -cne '2.0') {
                throw 'TASK037_MCP_RESPONSE_SHAPE_REJECTED'
            }
        }
        Assert-Task037ExactPropertySet `
            -Value $responses[0].result `
            -Expected @('capabilities', 'instructions', 'protocolVersion', 'serverInfo') `
            -FailureCode 'TASK037_MCP_RESPONSE_SHAPE_REJECTED'
        Enter-Task037PhaseGate `
            -Gate 'ready_evidence' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $readyLines = @($stderr -split '\r?\n' | Where-Object { $_ -like '*"event":"ready"*' -and $_ -like ('*"endpoint":"127.0.0.1:' + $rustPort + '"*') })
        if ($readyLines.Count -ne 1) {
            throw 'TASK037_FULL_CHAIN_READY_EVIDENCE_REJECTED'
        }
        if ($responses[1].result.isError -isnot [bool]) {
            throw 'TASK037_MCP_RESPONSE_SHAPE_REJECTED'
        }
        Enter-Task037PhaseGate `
            -Gate 'tool_content' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $toolIsError = [bool]$responses[1].result.isError
        $structured = Get-StructuredToolContent -Response $responses[1]
        $errorProperties = @('code', 'status')
        $successProperties = @(
            'baseline_commit', 'changed_paths', 'codex_runtime', 'commit_sha', 'component',
            'configuration_digest', 'entrypoint', 'entrypoint_classification',
            'entrypoint_runtime_kind', 'full_chain_receipt_digest', 'graph_analysis_digest',
            'graph_commit_sha', 'graph_database_identity_digest',
            'graph_extension_manifest_digest', 'graph_persistence_digest', 'graph_project_id',
            'graph_query_digest', 'graph_receipt_digest', 'graph_record_count',
            'graph_result_count', 'graph_retrieval_digest', 'graph_status', 'hermes_findings',
            'hermes_graph_receipt_digest', 'hermes_identity_digest', 'hermes_input_digest',
            'hermes_next_actions', 'hermes_provenance_status', 'hermes_reflection_digest',
            'hermes_schema_version', 'hermes_status', 'hermes_summary', 'intent_digest',
            'launcher_path', 'launcher_sha256', 'outcome_digest', 'parent_sha', 'profile',
            'receipt_digest', 'repository_path', 'request_id', 'schema_bundle_sha256',
            'schema_file_count', 'status', 'test', 'test_command_id', 'thread_id', 'turn_id',
            'version'
        )
        Assert-Task037ExactPropertySet `
            -Value $structured `
            -Expected $(if ($toolIsError) { $errorProperties } else { $successProperties }) `
            -FailureCode 'TASK037_TOOL_CONTENT_SHAPE_REJECTED'
        Enter-Task037PhaseGate `
            -Gate $(if ($expectError) { 'pre_status_semantics' } else { 'success_semantics' }) `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $toolCodeValue = if ($null -ne $structured.PSObject.Properties['code']) { $structured.code } else { $null }
        $toolCode = if ($toolCodeValue -is [string]) { [string]$toolCodeValue } else { $null }
        if (
            $expectError -and (
                -not $toolIsError -or
                $structured.status -isnot [string] -or
                [string]$structured.status -cne 'ERROR' -or
                $toolCodeValue -isnot [string] -or
                @(
                    'LATTICE_DELIVERY_RECONCILIATION_REQUIRED',
                    'LATTICE_HERMES_MEMORY_RECEIPT_REJECTED'
                ) -cnotcontains [string]$toolCodeValue
            )
        ) {
            throw 'TASK037_PRE_STATUS_FAIL_CLOSED_REJECTED'
        }
        Enter-Task037PhaseGate `
            -Gate 'phase_meta_projection' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $meta = [ordered]@{
            schema_version = 'lattice.task037.phase-meta.v1'
            acceptance_id = $acceptanceId
            phase = $Phase
            phase_id = $phaseId
            run_mode = $runMode
            tool_name = $toolName
            postgres_run_id = $runId
            process_exit_code = [int]$processContract.ChildExitCode
            tool_is_error = $toolIsError
            tool_code = $toolCode
            response_sha256 = $null
            openclaw = [ordered]@{
                process_id = $openClawPid
                ws_port = $wsPort
                rust_port = $rustPort
                profile_sha256 = $profileSha256
                package_version = $openClawVersion
                runtime_kind = 'Fake'
            }
            hermes = [ordered]@{
                runtime_manifest_sha256 = Get-FileSha256 $runtimeManifest
                broker_helper_sha256 = Get-FileSha256 $brokerExe
                deadline_seconds = 300
            }
            binary = [ordered]@{
                path = $fullChainExe
                sha256 = Get-FileSha256 $fullChainExe
                bytes = (Get-Item -LiteralPath $fullChainExe).Length
            }
        }
        if ($expectError) {
            Enter-Task037PhaseGate `
                -Gate 'memory_empty' `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
            $probe = Invoke-PostgresMemoryProbe -RunId $runId -Password $postgresPassword -HostName $hostName -Port $port -OutputPath $memoryProbePath
            if (
                $probe.analyses -ne 0 -or
                $probe.receipts -ne 0 -or
                $probe.retrieval_audits -ne 0 -or
                $probe.records -ne 0 -or
                $probe.reflections -ne 0 -or
                $probe.openclaw_commands -ne 0
            ) {
                throw 'TASK037_PRE_STATUS_MEMORY_NOT_EMPTY'
            }
            Enter-Task037PhaseGate `
                -Gate 'evidence_commit' `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
            $responseDigest = Write-Task037ValidatedPhaseEvidence `
                -Evidence $structured `
                -IsError $true `
                -OutputPath $outputJsonPath `
                -Meta $meta `
                -MetaPath $metaPath
            Complete-Task037PhaseGate `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
            return
        }
        if ($toolIsError) {
            $toolFailureGate = Get-Task037ToolFailureGate -Code $toolCodeValue
            if ($toolFailureGate -ceq 'success_tool_error_hermes') {
                $toolFailureGate = Get-Task037HermesComponentFailureGate -Code $toolCodeValue -StderrText $stderr
            }
            Enter-Task037PhaseGate `
                -Gate $toolFailureGate `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
            throw ('TASK037_' + $Phase.ToUpperInvariant() + '_TOOL_ERROR')
        }
        Enter-Task037PhaseGate `
            -Gate 'success_tool_result' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        Assert-FullChainSuccess `
            -Evidence $structured `
            -RunId $runId `
            -DeliveryRoot $deliveryRoot `
            -DeliveryLauncher $deliveryLauncher `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        Enter-Task037PhaseGate `
            -Gate 'memory_expected' `
            -ActiveGate ([ref]$activeGate) `
            -LastCompletedGate ([ref]$lastCompletedGate)
        $probe = Invoke-PostgresMemoryProbe -RunId $runId -Password $postgresPassword -HostName $hostName -Port $port -OutputPath $memoryProbePath
        if (
            $probe.analyses -ne 1 -or
            $probe.receipts -ne 1 -or
            $probe.retrieval_audits -ne 1 -or
            $probe.records -le 0 -or
            $probe.reflections -ne 1 -or
            $probe.openclaw_commands -ne 1 -or
            $probe.reflection_status -ne 'INFERENCE_CANDIDATE' -or
            $probe.reflection_receipt_digest -ne [string]$structured.full_chain_receipt_digest -or
            $probe.graph_receipt_digest -ne [string]$structured.graph_receipt_digest
        ) {
            throw 'TASK037_MEMORY_PROBE_REJECTED'
        }
        if ($Phase -eq 'FullChainStatus') {
            Enter-Task037PhaseGate `
                -Gate 'status_replay' `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
            $runEvidence = Read-JsonEvidence -Path (Join-Path $evidenceRoot 'run.json')
            $runProbe = Read-JsonEvidence -Path (Join-Path $evidenceRoot 'memory-run.json')
            Assert-Task037MemoryReplayUnchanged -RunProbe $runProbe -StatusProbe $probe
            if (
                [string]$structured.full_chain_receipt_digest -ne [string]$runEvidence.full_chain_receipt_digest -or
                [string]$structured.hermes_reflection_digest -ne [string]$runEvidence.hermes_reflection_digest -or
                [string]$structured.hermes_identity_digest -ne [string]$runEvidence.hermes_identity_digest -or
                [string]$structured.hermes_input_digest -ne [string]$runEvidence.hermes_input_digest -or
                [string]$structured.graph_receipt_digest -ne [string]$runEvidence.graph_receipt_digest
            ) {
                throw 'TASK037_STATUS_REPLAY_REJECTED'
            }
            Enter-Task037PhaseGate `
                -Gate 'evidence_commit' `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
            $responseDigest = Write-Task037ValidatedPhaseEvidence `
                -Evidence $structured `
                -IsError $false `
                -OutputPath $outputJsonPath `
                -Meta $meta `
                -MetaPath $metaPath
            $preStatus = Read-JsonEvidence -Path (Join-Path $evidenceRoot 'pre-status.json')
            $preMeta = Read-JsonEvidence -Path (Join-Path $evidenceRoot 'pre-status.meta.json')
            if ($preMeta.tool_is_error -isnot [bool] -or -not $preMeta.tool_is_error) {
                throw 'TASK037_PRE_STATUS_FAIL_CLOSED_REJECTED'
            }
            $final = [ordered]@{
                status = 'PASS'
                component = 'task037-full-chain-verification'
                acceptance_id = $acceptanceId
                postgres_run_id = $runId
                repository_path = Get-CanonicalPath -Path ([string]$structured.repository_path)
                codex_mode = 'OFFICIAL_CODEX_APP_SERVER'
                entrypoint = [string]$structured.entrypoint
                entrypoint_runtime_kind = [string]$structured.entrypoint_runtime_kind
                hermes_status = [string]$structured.hermes_status
                hermes_schema_version = [string]$structured.hermes_schema_version
                full_chain_receipt_digest = [string]$structured.full_chain_receipt_digest
                hermes_reflection_digest = [string]$structured.hermes_reflection_digest
                graph_receipt_digest = [string]$structured.graph_receipt_digest
                memory_reflection_rows = $probe.reflections
                memory_receipt_rows = $probe.receipts
                memory_retrieval_audit_rows = $probe.retrieval_audits
                memory_record_rows = $probe.records
                memory_analysis_rows = $probe.analyses
                status_replayed_after_postgres_restart = $true
                memory_counts_unchanged_during_status = $true
                pre_status_fail_closed = ($preMeta.tool_is_error -and [string]$preStatus.status -eq 'ERROR')
                pre_status_error_code = [string]$preStatus.code
                run_response_sha256 = Get-FileSha256 (Join-Path $evidenceRoot 'run.json')
                status_response_sha256 = Get-FileSha256 (Join-Path $evidenceRoot 'status.json')
                runtime_manifest_sha256 = Get-FileSha256 $runtimeManifest
                full_chain_binary_sha256 = Get-FileSha256 $fullChainExe
                broker_binary_sha256 = Get-FileSha256 $brokerExe
            }
            Write-JsonEvidence -Path (Join-Path $evidenceRoot 'final.pending.json') -Value $final
            Complete-Task037PhaseGate `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
        }
        else {
            Enter-Task037PhaseGate `
                -Gate 'evidence_commit' `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
            $responseDigest = Write-Task037ValidatedPhaseEvidence `
                -Evidence $structured `
                -IsError $false `
                -OutputPath $outputJsonPath `
                -Meta $meta `
                -MetaPath $metaPath
            Complete-Task037PhaseGate `
                -ActiveGate ([ref]$activeGate) `
                -LastCompletedGate ([ref]$lastCompletedGate)
        }
    }
    catch {
        $failure = $_.Exception.Message
        $failureEvidence = Get-SafeFailureEvidence -Message $failure
    }
    finally {
        $fullChainProcessStopped = $null -eq $fullChainProcess
        $openClawProcessStopped = $null -eq $openClawProcess
        $fullChainControllerStopped = $null -eq $fullChainControllerProcess
        $openClawControllerStopped = $null -eq $openClawControllerProcess
        $profileRemoved = $false
        $brokerCodexHomeRemoved = $false
        $brokerCredentialSourceUnchanged = $null -eq $brokerCredentialSourceBefore
        $hermesIsolationRemoved = $false
        $brokerIsolationRemoved = $false
        $wsListenerCount = -1
        $rustListenerCount = -1

        if ($phaseJob -eq [IntPtr]::Zero) {
            $jobProcessCountZero = $true
        }
        else {
            try {
                Stop-Task037Job -Job $phaseJob
                $jobProcessCountZero = (Get-Task037JobActiveProcessCount -Job $phaseJob) -eq 0
            }
            catch {
                $phaseCleanupFailure = 'TASK037_PHASE_JOB_CLEANUP_REJECTED'
            }
            finally {
                try {
                    Close-Task037Job -Job $phaseJob
                }
                catch {
                    if ($null -eq $phaseCleanupFailure) {
                        $phaseCleanupFailure = 'TASK037_PHASE_JOB_CLEANUP_REJECTED'
                    }
                }
                $phaseJob = [IntPtr]::Zero
            }
        }

        foreach ($ownedProcess in @(
            $fullChainProcess,
            $openClawProcess,
            $fullChainControllerProcess,
            $openClawControllerProcess
        )) {
            if ($null -eq $ownedProcess) { continue }
            try {
                if (-not $ownedProcess.WaitForExit(5000)) {
                    $phaseCleanupFailure = 'TASK037_PHASE_PROCESS_REAP_REJECTED'
                }
            }
            catch {
                $phaseCleanupFailure = 'TASK037_PHASE_PROCESS_REAP_REJECTED'
            }
        }
        if (-not $fullChainEvidenceCompleted -and $null -ne $fullChainOwned) {
            try { Complete-Task037GatedProcessEvidence -OwnedProcess $fullChainOwned }
            catch { $phaseCleanupFailure = 'TASK037_PHASE_PROCESS_EVIDENCE_REJECTED' }
        }
        if ($null -ne $openClawOwned) {
            try { Complete-Task037GatedProcessEvidence -OwnedProcess $openClawOwned }
            catch { $phaseCleanupFailure = 'TASK037_PHASE_PROCESS_EVIDENCE_REJECTED' }
        }
        foreach ($extraProcess in @($openClawBaselineExtraProcesses)) {
            try {
                if (-not $extraProcess.WaitForExit(5000)) {
                    $phaseCleanupFailure = 'TASK037_PHASE_PROCESS_REAP_REJECTED'
                }
            }
            catch {
                $phaseCleanupFailure = 'TASK037_PHASE_PROCESS_REAP_REJECTED'
            }
            finally {
                try { $extraProcess.Dispose() } catch { }
            }
        }
        $openClawBaselineExtraProcesses = @()
        if ($null -ne $fullChainProcess) {
            $fullChainProcessStopped = $fullChainProcess.HasExited
            $fullChainProcess.Dispose()
        }
        if ($null -ne $openClawProcess) {
            $openClawProcessStopped = $openClawProcess.HasExited
            $openClawProcess.Dispose()
        }
        if ($null -ne $fullChainControllerProcess) {
            $fullChainControllerStopped = $fullChainControllerProcess.HasExited
            $fullChainControllerProcess.Dispose()
        }
        if ($null -ne $openClawControllerProcess) {
            $openClawControllerStopped = $openClawControllerProcess.HasExited
            $openClawControllerProcess.Dispose()
        }

        if ($jobProcessCountZero) {
            try {
                Wait-LoopbackPortClosed -Port $wsPort
                Wait-LoopbackPortClosed -Port $rustPort
                $wsListenerCount = if ($wsPort -eq 0) { 0 } else { @(Get-NetTCPConnection -LocalPort $wsPort -State Listen -ErrorAction SilentlyContinue).Count }
                $rustListenerCount = if ($rustPort -eq 0) { 0 } else { @(Get-NetTCPConnection -LocalPort $rustPort -State Listen -ErrorAction SilentlyContinue).Count }
            }
            catch {
                $phaseCleanupFailure = 'TASK037_PHASE_LISTENER_CLEANUP_REJECTED'
            }
            try {
                if ($null -eq $brokerCredentialOwnership -and (Test-Path -LiteralPath $brokerOwnerRecordPath)) {
                    $brokerCredentialOwnership = Get-Task037CredentialOwnershipFromRecord -OwnerRecordPath $brokerOwnerRecordPath
                }
                if ($null -eq $brokerCredentialOwnership) {
                    if (Test-Path -LiteralPath (Join-Path $brokerCodexHome 'auth.json')) {
                        throw 'TASK037_PHASE_CREDENTIAL_CLEANUP_REJECTED'
                    }
                    $brokerCodexHomeRemoved = Remove-SafeTempRoot `
                        -Path $brokerCodexHome `
                        -AuthorizedTempBase $tempBase
                }
                else {
                    $brokerCodexHomeRemoved = Remove-SafeTempRoot `
                        -Path $brokerCodexHome `
                        -CredentialHome `
                        -Ownership $brokerCredentialOwnership `
                        -AuthorizedTempBase $tempBase
                }
            }
            catch { $phaseCleanupFailure = 'TASK037_PHASE_CREDENTIAL_CLEANUP_REJECTED' }
            try {
                $profileRemoved = Remove-SafeTempRoot `
                    -Path $profileRoot `
                    -AuthorizedTempBase $tempBase
            }
            catch { $phaseCleanupFailure = 'TASK037_PHASE_PROFILE_CLEANUP_REJECTED' }
            try {
                if ($null -ne $brokerCredentialSourceBefore) {
                    Assert-CodexCredentialSourceUnchanged -SourceHome $codexAuthHome -Expected $brokerCredentialSourceBefore
                    $brokerCredentialSourceUnchanged = $true
                }
            }
            catch { $phaseCleanupFailure = 'TASK037_PHASE_CREDENTIAL_SOURCE_REJECTED' }
            try {
                $hermesIsolationRemoved = Remove-SafeTempRoot `
                    -Path $hermesIsolationRoot `
                    -AuthorizedTempBase $tempBase
            }
            catch { $phaseCleanupFailure = 'TASK037_PHASE_ISOLATION_CLEANUP_REJECTED' }
            try {
                $brokerIsolationRemoved = Remove-SafeTempRoot `
                    -Path $brokerIsolationRoot `
                    -AuthorizedTempBase $tempBase
            }
            catch { $phaseCleanupFailure = 'TASK037_PHASE_ISOLATION_CLEANUP_REJECTED' }
        }
        $logEvidence = @()
        if ($jobProcessCountZero) {
            foreach ($log in $logPublicationPlan) {
                try {
                    $logEvidence += @(
                        Get-Task037RawArtifactReceipt `
                            -SourcePath ([string]$log.Source) `
                            -ArtifactId ([string]$log.ArtifactId)
                    )
                }
                catch {
                    $logEvidence += @([ordered]@{
                        artifact_id = [string]$log.ArtifactId
                        exists = $false
                        content_retained = $false
                        receipt_failure = 'TASK037_RAW_ARTIFACT_RECEIPT_REJECTED'
                    })
                    if ($null -eq $phaseCleanupFailure) {
                        $phaseCleanupFailure = 'TASK037_PHASE_LOG_EVIDENCE_REJECTED'
                    }
                }
            }
            try {
                $rawLogRootRemoved = Remove-SafeTempRoot `
                    -Path $rawLogRoot `
                    -AuthorizedTempBase $tempBase
            }
            catch {
                if ($null -eq $phaseCleanupFailure) {
                    $phaseCleanupFailure = 'TASK037_PHASE_RAW_LOG_CLEANUP_REJECTED'
                }
            }
        }
        if (
            -not $jobProcessCountZero -or
            -not $fullChainProcessStopped -or
            -not $openClawProcessStopped -or
            -not $fullChainControllerStopped -or
            -not $openClawControllerStopped -or
            $wsListenerCount -ne 0 -or
            $rustListenerCount -ne 0 -or
            -not $profileRemoved -or
            -not $brokerCodexHomeRemoved -or
            -not $brokerCredentialSourceUnchanged -or
            -not $rawLogRootRemoved -or
            -not $hermesIsolationRemoved -or
            -not $brokerIsolationRemoved
        ) {
            if ($null -eq $phaseCleanupFailure) {
                $phaseCleanupFailure = 'TASK037_PHASE_CLEANUP_REJECTED'
            }
        }
        $failureGate = if ($null -eq $failure) { $null } else { $activeGate }
        if (
            -not (Test-Task037PhaseGateName -Gate $failureGate) -or
            -not (Test-Task037PhaseGateName -Gate $lastCompletedGate)
        ) {
            if ($null -eq $phaseCleanupFailure) {
                $phaseCleanupFailure = 'TASK037_PHASE_GATE_REJECTED'
            }
            $failureGate = $null
            $lastCompletedGate = $null
        }
        $cleanup = [ordered]@{
            schema_version = 'lattice.task037.cleanup.v2'
            acceptance_id = $acceptanceId
            phase = $Phase
            failure_gate = $failureGate
            last_completed_gate = $lastCompletedGate
            job_active_process_count_zero = $jobProcessCountZero
            full_chain_process_stopped = $fullChainProcessStopped
            openclaw_process_stopped = $openClawProcessStopped
            full_chain_controller_stopped = $fullChainControllerStopped
            openclaw_controller_stopped = $openClawControllerStopped
            ws_listener_count = $wsListenerCount
            rust_listener_count = $rustListenerCount
            openclaw_profile_removed = $profileRemoved
            broker_codex_home_removed = $brokerCodexHomeRemoved
            broker_credential_source_unchanged = $brokerCredentialSourceUnchanged
            raw_log_root_removed = $rawLogRootRemoved
            hermes_isolation_removed = $hermesIsolationRemoved
            broker_isolation_removed = $brokerIsolationRemoved
            cleanup_failure = $phaseCleanupFailure
            failure = $failureEvidence
            raw_artifact_receipts = $logEvidence
        }
        Write-JsonEvidence -Path $cleanupPath -Value $cleanup
        if ($null -ne $phaseCleanupFailure) {
            throw $phaseCleanupFailure
        }
    }
    if ($null -ne $failure) {
        Throw-Task037SafeFailure -Message $failure
    }
}

function Invoke-Task037InternalAdmissionProbe {
    if (-not $InternalAdmissionProbe) {
        throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
    }
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
    $acceptanceId = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_SUPERVISOR_ACCEPTANCE_ID',
        'Process'
    )
    $tempRootValue = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_SUPERVISOR_TEMP_ROOT',
        'Process'
    )
    $startedAtUtc = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_ADMISSION_STARTED_AT_UTC',
        'Process'
    )
    if (
        $acceptanceId -notmatch '^[0-9a-f]{32}$' -or
        [string]::IsNullOrWhiteSpace($tempRootValue) -or
        [string]::IsNullOrWhiteSpace($startedAtUtc)
    ) {
        throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
    }
    $tempRoot = Get-CanonicalPath -Path $tempRootValue
    Assert-Task037SupervisorBinding `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -TempRoot $tempRoot
    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $startedAtUtc `
        -Stage 'supervisor_binding'

    $credentialSource = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_ADMISSION_PROBE_CREDENTIAL_SOURCE',
        'Process'
    )
    $credentialDestination = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_ADMISSION_PROBE_CREDENTIAL_DESTINATION',
        'Process'
    )
    $credentialOwnerRecord = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_ADMISSION_PROBE_CREDENTIAL_OWNER_RECORD',
        'Process'
    )
    $tempBase = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_SUPERVISOR_TEMP_BASE',
        'Process'
    )
    if (
        [string]::IsNullOrWhiteSpace($credentialSource) -or
        [string]::IsNullOrWhiteSpace($credentialDestination) -or
        [string]::IsNullOrWhiteSpace($credentialOwnerRecord) -or
        [string]::IsNullOrWhiteSpace($tempBase)
    ) {
        throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
    }
    try {
        $credentialOwnership = Copy-CodexCredentialFiles `
            -SourceHome $credentialSource `
            -DestinationHome $credentialDestination `
            -OwnerToken ($acceptanceId + '-probe') `
            -OwnerRecordPath $credentialOwnerRecord `
            -ForBroker `
            -AuthorizedTempBase $tempBase
        if (-not (Remove-SafeTempRoot `
            -Path $credentialDestination `
            -CredentialHome `
            -Ownership $credentialOwnership `
            -AuthorizedTempBase $tempBase
        )) {
            throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
        }
        if (
            (Test-Path -LiteralPath $credentialDestination) -or
            (Test-Path -LiteralPath $credentialOwnerRecord)
        ) {
            throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
        }
        $sentinelPath = Join-Path $tempRoot 'admission-provisioning-complete.v1'
        $sentinelBytes = [Text.UTF8Encoding]::new($false).GetBytes(
            "lattice.task037.admission-provisioning.v1|$acceptanceId`n"
        )
        $sentinelStream = [IO.FileStream]::new(
            $sentinelPath,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::Write,
            [IO.FileShare]::None,
            4096,
            [IO.FileOptions]::WriteThrough
        )
        try {
            $sentinelStream.Write($sentinelBytes, 0, $sentinelBytes.Length)
            $sentinelStream.Flush($true)
        }
        finally {
            $sentinelStream.Dispose()
        }
    }
    catch {
        throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
    }

    $releaseName = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_ADMISSION_PROBE_RELEASE',
        'Process'
    )
    $release = $null
    try {
        if ($releaseName -notmatch '^LatticeTask037AdmissionRelease-[0-9a-f]{32}$') {
            throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
        }
        $release = [Threading.EventWaitHandle]::OpenExisting($releaseName)
        if (-not $release.WaitOne(30000)) {
            throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
        }
    }
    catch {
        throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
    }
    finally {
        if ($null -ne $release) { $release.Dispose() }
    }
    throw 'TASK037_SYNTHETIC_ADMISSION_REJECTED'
}

function Invoke-DefaultVerification {
    if (-not $InternalController) {
        throw 'TASK037_UNSUPERVISED_CONTROLLER_REJECTED'
    }
    if ([string]::IsNullOrWhiteSpace($CodexAuthHome)) {
        $CodexAuthHome = Join-Path $env:USERPROFILE '.codex'
    }
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
    $acceptanceId = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_SUPERVISOR_ACCEPTANCE_ID', 'Process')
    $tempAcceptanceRootValue = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_SUPERVISOR_TEMP_ROOT', 'Process')
    $admissionStartedAtUtc = [Environment]::GetEnvironmentVariable(
        'LATTICE_TASK037_ADMISSION_STARTED_AT_UTC',
        'Process'
    )
    if ($acceptanceId -notmatch '^[0-9a-f]{32}$') {
        throw 'TASK037_CONTROLLER_ACCEPTANCE_ID_REJECTED'
    }
    if ([string]::IsNullOrWhiteSpace($tempAcceptanceRootValue)) {
        throw 'TASK037_CONTROLLER_TEMP_ROOT_REJECTED'
    }
    if ([string]::IsNullOrWhiteSpace($admissionStartedAtUtc)) {
        throw 'TASK037_ADMISSION_RECEIPT_REJECTED'
    }
    $tempAcceptanceRoot = Get-CanonicalPath -Path $tempAcceptanceRootValue
    $tempBaseValue = [Environment]::GetEnvironmentVariable('LATTICE_TASK037_SUPERVISOR_TEMP_BASE', 'Process')
    if ([string]::IsNullOrWhiteSpace($tempBaseValue)) {
        throw 'TASK037_CONTROLLER_TEMP_ROOT_REJECTED'
    }
    $expectedTempAcceptanceRoot = Get-Task037BoundTempRoot `
        -AcceptanceId $acceptanceId `
        -TempBase $tempBaseValue
    if (
        $acceptanceId -notmatch '^[0-9a-f]{32}$' -or
        -not (Test-ExactPath -Actual $tempAcceptanceRoot -Expected $expectedTempAcceptanceRoot)
    ) {
        throw 'TASK037_CONTROLLER_TEMP_ROOT_REJECTED'
    }
    $acceptanceRoot = Get-CanonicalPath -Path (Join-Path $repositoryTarget ('full-chain-acceptance\' + $acceptanceId))
    $fixtureRoot = Get-CanonicalPath -Path (Join-Path $repositoryTarget ('lattice-delivery\' + $acceptanceId))
    $evidenceRoot = Join-Path $acceptanceRoot 'evidence'
    $runtimeManifest = Join-Path $acceptanceRoot 'offline-runtime-manifest.json'
    $sourceManifest = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'full-chain-acceptance\4fe82cdeaa154c28bbe18f6a\offline-runtime-manifest.json')
    $tempBase = Get-CanonicalPath -Path $tempBaseValue
    $deliveryCodexHome = Join-Path $tempAcceptanceRoot 'delivery-codex-home'
    $schemaDirectory = Join-Path $fixtureRoot 'schema'
    $deliveryRoot = Join-Path $fixtureRoot 'delivery'
    $deliveryLauncher = Get-CanonicalPath -Path (Join-Path $repositoryTarget 'codex-official\0.146.0\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe')
    Assert-Task037SupervisorBinding `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -TempRoot $tempAcceptanceRoot
    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $admissionStartedAtUtc `
        -Stage 'supervisor_binding'
    $openClawCli = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'integrations\openclaw-lattice\node_modules\openclaw\openclaw.mjs')
    $wslExe = Get-CanonicalPath -Path (Join-Path $env:SystemRoot 'System32\wsl.exe')
    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $admissionStartedAtUtc `
        -Stage 'git_identity_preflight'
    $gitExe = Get-TrustedTask037Git -Candidate ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CONTROLLER_GIT_EXE', 'Process'))
    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $admissionStartedAtUtc `
        -Stage 'node_identity_preflight'
    $programDataAuthority = Get-CanonicalPath -Path (
        [Environment]::GetEnvironmentVariable(
            'LATTICE_TASK037_CONTROLLER_PROGRAM_DATA',
            'Process'
        )
    )
    $nodeRuntimeStageWriter = {
        param([string]$Stage)
        $null = Write-Task037AdmissionStageReceipt `
            -RepositoryTarget $repositoryTarget `
            -AcceptanceId $acceptanceId `
            -StartedAtUtc $admissionStartedAtUtc `
            -Stage $Stage
    }.GetNewClosure()
    $nodeExe = Get-TrustedTask037Node `
        -Candidate ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CONTROLLER_NODE_EXE', 'Process')) `
        -ProgramDataAuthority $programDataAuthority `
        -RuntimeStageWriter $nodeRuntimeStageWriter
    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $admissionStartedAtUtc `
        -Stage 'rust_toolchain_preflight'
    $cargoHome = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('CARGO_HOME', 'Process'))
    $rustupHome = Get-CanonicalPath -Path ([Environment]::GetEnvironmentVariable('RUSTUP_HOME', 'Process'))
    $rustToolchain = Get-TrustedTask037RustToolchain `
        -CargoCandidate ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CONTROLLER_CARGO_EXE', 'Process')) `
        -RustcCandidate ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CONTROLLER_RUSTC_EXE', 'Process')) `
        -RustdocCandidate ([Environment]::GetEnvironmentVariable('LATTICE_TASK037_CONTROLLER_RUSTDOC_EXE', 'Process')) `
        -CargoHome $cargoHome
    Assert-Task037CargoConfigurationClosure -RepositoryRoot $repositoryRoot -CargoHome $cargoHome
    $cargoExe = $rustToolchain.Cargo
    if (
        -not (Test-ExactPath -Actual $rustupHome -Expected $rustToolchain.RustupHome) -or
        [Environment]::GetEnvironmentVariable('RUSTUP_TOOLCHAIN', 'Process') -cne $task037RustToolchain -or
        -not (Test-ExactPath -Actual ([Environment]::GetEnvironmentVariable('RUSTC', 'Process')) -Expected $rustToolchain.Rustc) -or
        -not (Test-ExactPath -Actual ([Environment]::GetEnvironmentVariable('RUSTDOC', 'Process')) -Expected $rustToolchain.Rustdoc)
    ) {
        throw 'TASK037_RUST_TOOLCHAIN_SCOPE_REJECTED'
    }
    foreach ($directory in @($cargoHome, $rustupHome)) {
        Assert-NoReparsePath -Path $directory -FailureCode 'TASK037_RUST_TOOLCHAIN_SCOPE_REJECTED'
        $item = Get-Item -LiteralPath $directory -Force -ErrorAction SilentlyContinue
        if ($null -eq $item -or -not $item.PSIsContainer) {
            throw 'TASK037_RUST_TOOLCHAIN_SCOPE_REJECTED'
        }
    }
    Assert-Task037RustToolchainVersion -Toolchain $rustToolchain
    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $admissionStartedAtUtc `
        -Stage 'artifact_identity_preflight'
    foreach ($path in @($acceptanceRoot, $fixtureRoot)) {
        if (Test-Path -LiteralPath $path) {
            throw ('TASK037_FRESH_ROOT_REJECTED|' + $path)
        }
    }
    foreach ($path in @($repositoryTarget, $sourceManifest, $deliveryLauncher, $openClawCli)) {
        Assert-NoReparseAncestor -Path $path -Boundary $repositoryRoot
    }
    Assert-NoReparseAncestor -Path $deliveryCodexHome -Boundary $tempBase
    foreach ($path in @($sourceManifest, $deliveryLauncher, $openClawCli, $wslExe, $gitExe, $nodeExe, $cargoExe)) {
        Assert-RegularFile -Path $path
    }
    if ((Get-FileSha256 $sourceManifest) -ne $hermesRuntimeManifestSha256) {
        throw 'TASK037_SOURCE_MANIFEST_REJECTED'
    }
    if ((Get-FileSha256 $deliveryLauncher) -ne $officialCodexSha256) {
        throw 'TASK037_OFFICIAL_CODEX_REJECTED'
    }
    if ((Get-FileSha256 $openClawCli) -ne $openClawEntrypointSha256) {
        throw 'TASK037_OPENCLAW_REJECTED'
    }

    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $admissionStartedAtUtc `
        -Stage 'worktree_preflight'
    Push-Location $repositoryRoot
    try {
        $branch = Invoke-Task037GitText -GitExe $gitExe -Arguments @('rev-parse', '--abbrev-ref', 'HEAD') -FailureCode 'TASK037_GIT_BRANCH_FAILED'
        $head = Invoke-Task037GitText -GitExe $gitExe -Arguments @('rev-parse', 'HEAD') -FailureCode 'TASK037_GIT_HEAD_FAILED'
        $status = Invoke-Task037GitText -GitExe $gitExe -Arguments @('status', '--short') -FailureCode 'TASK037_GIT_STATUS_FAILED'
    }
    finally {
        Pop-Location
    }
    if (-not [string]::IsNullOrWhiteSpace($status)) {
        throw 'TASK037_WORKTREE_DIRTY_REJECTED'
    }
    if (Test-Path -LiteralPath $deliveryCodexHome) {
        throw ('TASK037_FRESH_TEMP_REJECTED|' + $deliveryCodexHome)
    }

    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $admissionStartedAtUtc `
        -Stage 'inner_pre_acceptance'

    if ($InternalControllerPreflightOnly) {
        Write-Output 'TASK037_CONTROLLER_PREFLIGHT=PASS'
        return
    }

    New-Item -ItemType Directory -Path $acceptanceRoot -Force:$false | Out-Null
    New-Item -ItemType Directory -Path $evidenceRoot -Force:$false | Out-Null
    New-Item -ItemType Directory -Path $fixtureRoot -Force:$false | Out-Null
    Copy-Item -LiteralPath $sourceManifest -Destination $runtimeManifest -Force:$false

    Push-Location $repositoryRoot
    try {
        & $cargoExe 'build' '-p' 'lattice-runtime' '--bin' 'lattice-full-chain' '--locked' | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw 'TASK037_FULL_CHAIN_BUILD_FAILED'
        }
        & $cargoExe 'build' '-p' 'lattice-hermes-adapter' '--bin' 'lattice-hermes-broker' '--locked' | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw 'TASK037_HERMES_BROKER_BUILD_FAILED'
        }
        $metadataText = (& $cargoExe 'metadata' '--no-deps' '--format-version' '1' '--locked') -join "`n"
        if ($LASTEXITCODE -ne 0) {
            throw 'TASK037_CARGO_METADATA_FAILED'
        }
    }
    finally {
        Pop-Location
    }
    $metadata = $metadataText | ConvertFrom-Json
    $targetDirectory = Get-CanonicalPath -Path ([string]$metadata.target_directory)
    $fullChainExe = Get-CanonicalPath -Path (Join-Path $targetDirectory 'debug\lattice-full-chain.exe')
    $brokerExe = Get-CanonicalPath -Path (Join-Path $targetDirectory 'debug\lattice-hermes-broker.exe')
    Assert-RegularFile -Path $fullChainExe
    Assert-RegularFile -Path $brokerExe

    $params = [ordered]@{
        schema_version = 'lattice.task037.params.v1'
        acceptance_id = $acceptanceId
        branch = $branch.Trim()
        head = $head.Trim()
        working_tree_clean_at_start = $true
        repository_root = $repositoryRoot
        acceptance_root = $acceptanceRoot
        evidence_root = $evidenceRoot
        fixture_root = $fixtureRoot
        delivery_root = $deliveryRoot
        delivery_codex_home = $deliveryCodexHome
        delivery_codex_home_scope = 'temp'
        runtime_manifest_sha256 = Get-FileSha256 $runtimeManifest
        full_chain_binary_sha256 = Get-FileSha256 $fullChainExe
        broker_binary_sha256 = Get-FileSha256 $brokerExe
        delivery_launcher_sha256 = Get-FileSha256 $deliveryLauncher
        openclaw_entrypoint_sha256 = Get-FileSha256 $openClawCli
        codex_auth_source = Get-CanonicalPath -Path $CodexAuthHome
        supervised_cleanup = $true
    }
    Write-JsonEvidence -Path (Join-Path $acceptanceRoot 'params.json') -Value $params

    $environmentValues = [ordered]@{
        LATTICE_TASK037_ACCEPTANCE_ID = $acceptanceId
        LATTICE_TASK037_EVIDENCE_ROOT = $evidenceRoot
        LATTICE_TASK037_FIXTURE_ROOT = $fixtureRoot
        LATTICE_TASK037_FULL_CHAIN_EXE = $fullChainExe
        LATTICE_TASK037_HERMES_BROKER_EXE = $brokerExe
        LATTICE_TASK037_RUNTIME_MANIFEST = $runtimeManifest
        LATTICE_TASK037_DELIVERY_LAUNCHER = $deliveryLauncher
        LATTICE_TASK037_DELIVERY_CODEX_HOME = $deliveryCodexHome
        LATTICE_TASK037_SCHEMA_DIR = $schemaDirectory
        LATTICE_TASK037_DELIVERY_ROOT = $deliveryRoot
        LATTICE_TASK037_GIT_EXE = $gitExe
        LATTICE_TASK037_NODE_EXE = $nodeExe
        LATTICE_TASK037_PROGRAM_DATA = $programDataAuthority
        LATTICE_TASK037_OPENCLAW_CLI = $openClawCli
        LATTICE_TASK037_CODEX_AUTH_HOME = Get-CanonicalPath -Path $CodexAuthHome
        LATTICE_TASK037_TEMP_ACCEPTANCE_ROOT = $tempAcceptanceRoot
        LATTICE_TASK037_WSL_EXE = $wslExe
    }
    $originalEnvironment = @{}
    $deliveryOwnerRecordPath = Join-Path $evidenceRoot 'driver.delivery.credential-owner.json'
    $driverFailure = $null
    $driverCleanupFailure = $null
    $deliveryCodexHomeRemoved = $false
    $credentialSourceUnchanged = $false
    $phaseJobsSafeForCredentialCleanup = $false
    $deliveryCredentialOwnership = $null
    $credentialSourceBefore = Get-CodexCredentialSourceEvidence -SourceHome $CodexAuthHome
    $credentialSourceMonitor = Start-Task037CredentialSourceMonitor -SourceHome $CodexAuthHome
    foreach ($entry in $environmentValues.GetEnumerator()) {
        $originalEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, 'Process')
    }
    try {
        $deliveryCredentialOwnership = Copy-CodexCredentialFiles `
            -SourceHome $CodexAuthHome `
            -DestinationHome $deliveryCodexHome `
            -OwnerToken ($acceptanceId + '-delivery') `
            -OwnerRecordPath $deliveryOwnerRecordPath `
            -AuthorizedTempBase $tempBase
        foreach ($entry in $environmentValues.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable($entry.Key, [string]$entry.Value, 'Process')
        }
        $postgresHarness = Get-CanonicalPath -Path (Join-Path $PSScriptRoot 'run-task019-postgres.ps1')
        Assert-RegularFile -Path $postgresHarness
        $null = & $postgresHarness -RunFullChainAcceptanceHook
    }
    catch {
        $driverFailure = $_.Exception.Message
    }
    finally {
        try {
            foreach ($entry in $environmentValues.GetEnumerator()) {
                [Environment]::SetEnvironmentVariable($entry.Key, $originalEnvironment[$entry.Key], 'Process')
            }
        }
        catch {
            $driverCleanupFailure = 'TASK037_DRIVER_ENVIRONMENT_RESTORE_REJECTED'
        }
        try {
            $phaseJobsSafeForCredentialCleanup = Test-Task037StartedPhaseJobsZero `
                -EvidenceRoot $evidenceRoot `
                -AcceptanceId $acceptanceId
        }
        catch { $phaseJobsSafeForCredentialCleanup = $false }
        if ($phaseJobsSafeForCredentialCleanup) {
            try {
                if ($null -eq $deliveryCredentialOwnership -and (Test-Path -LiteralPath $deliveryOwnerRecordPath)) {
                    $deliveryCredentialOwnership = Get-Task037CredentialOwnershipFromRecord -OwnerRecordPath $deliveryOwnerRecordPath
                }
                if ($null -eq $deliveryCredentialOwnership) {
                    if (Test-Path -LiteralPath (Join-Path $deliveryCodexHome 'auth.json')) {
                        throw 'TASK037_DELIVERY_CODEX_HOME_CLEANUP_REJECTED'
                    }
                    $deliveryCodexHomeRemoved = Remove-SafeTempRoot `
                        -Path $deliveryCodexHome `
                        -AuthorizedTempBase $tempBase
                }
                else {
                    $deliveryCodexHomeRemoved = Remove-SafeTempRoot `
                        -Path $deliveryCodexHome `
                        -CredentialHome `
                        -Ownership $deliveryCredentialOwnership `
                        -AuthorizedTempBase $tempBase
                }
            }
            catch {
                $driverCleanupFailure = 'TASK037_DELIVERY_CODEX_HOME_CLEANUP_REJECTED'
            }
        }
        else {
            $driverCleanupFailure = 'TASK037_PHASE_JOB_STATE_UNKNOWN'
        }
        try {
            Stop-Task037CredentialSourceMonitor `
                -Monitor $credentialSourceMonitor `
                -SourceHome $CodexAuthHome `
                -Expected $credentialSourceBefore
            $credentialSourceUnchanged = $true
        }
        catch {
            $driverCleanupFailure = 'TASK037_CODEX_CREDENTIAL_SOURCE_MUTATED'
        }
        Write-JsonEvidence -Path (Join-Path $evidenceRoot 'driver.cleanup.json') -Value ([ordered]@{
            schema_version = 'lattice.task037.driver-cleanup.v1'
            acceptance_id = $acceptanceId
            delivery_codex_home = $deliveryCodexHome
            delivery_codex_home_removed = $deliveryCodexHomeRemoved
            phase_jobs_safe_for_credential_cleanup = $phaseJobsSafeForCredentialCleanup
            credential_source_unchanged = $credentialSourceUnchanged
            cleanup_failure = $driverCleanupFailure
            failure = Get-SafeFailureEvidence -Message $driverFailure
        })
    }
    if ($null -ne $driverCleanupFailure -or -not $deliveryCodexHomeRemoved -or -not $credentialSourceUnchanged) {
        throw 'TASK037_ACCEPTANCE_FINALIZATION_REJECTED'
    }
    if ($null -ne $driverFailure) {
        Throw-Task037SafeFailure -Message $driverFailure
    }

    $finalPath = Join-Path $evidenceRoot 'final.json'
    $pendingFinalPath = Join-Path $evidenceRoot 'final.pending.json'
    if (-not (Test-Path -LiteralPath $pendingFinalPath -PathType Leaf)) {
        throw 'TASK037_FINAL_EVIDENCE_REJECTED'
    }
    Assert-RegularFile -Path $pendingFinalPath
    $final = Read-JsonEvidence -Path $pendingFinalPath
    if (
        [string]$final.status -ne 'PASS' -or
        [string]$final.component -ne 'task037-full-chain-verification' -or
        [string]$final.acceptance_id -ne $acceptanceId -or
        $final.status_replayed_after_postgres_restart -isnot [bool] -or
        $final.memory_counts_unchanged_during_status -isnot [bool] -or
        $final.pre_status_fail_closed -isnot [bool] -or
        -not $final.status_replayed_after_postgres_restart -or
        -not $final.memory_counts_unchanged_during_status -or
        -not $final.pre_status_fail_closed
    ) {
        throw 'TASK037_FINAL_EVIDENCE_REJECTED'
    }
    if (Test-Path -LiteralPath $finalPath) {
        throw 'TASK037_FINAL_EVIDENCE_NOT_FRESH'
    }
    Write-Output 'TASK037_INNER_CONTROLLER=PASS'
}

function Invoke-Task037SupervisedVerification {
    param([switch]$ControllerPreflightOnly)

    if ([string]::IsNullOrWhiteSpace($CodexAuthHome)) {
        $CodexAuthHome = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)) '.codex'
    }
    $repositoryRoot = Get-CanonicalPath -Path (Join-Path $PSScriptRoot '..')
    $repositoryTarget = Get-CanonicalPath -Path (Join-Path $repositoryRoot 'target')
    $supervisorTempBase = Get-Task037TrustedTempBase
    $acceptanceId = [Guid]::NewGuid().ToString('N')
    $admissionStartedAtUtc = [DateTimeOffset]::UtcNow.ToString(
        'O',
        [Globalization.CultureInfo]::InvariantCulture
    )
    $null = Write-Task037AdmissionStageReceipt `
        -RepositoryTarget $repositoryTarget `
        -AcceptanceId $acceptanceId `
        -StartedAtUtc $admissionStartedAtUtc `
        -Stage 'recovery_preflight'
    $null = Invoke-Task037RecoveryOnly -RepositoryTarget $repositoryTarget
    $windows = Get-CanonicalPath -Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Windows))
    $system32 = Get-CanonicalPath -Path (Join-Path $windows 'System32')
    $controllerProgramData = Get-CanonicalPath -Path (
        [Environment]::GetFolderPath([Environment+SpecialFolder]::CommonApplicationData)
    )
    $programFiles = Get-CanonicalPath -Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles))
    $userProfile = Get-CanonicalPath -Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile))
    $controllerGitExe = Get-CanonicalPath -Path (Join-Path $programFiles 'Git\cmd\git.exe')
    $controllerNodeExe = Get-CanonicalPath -Path (Join-Path $programFiles 'nodejs\node.exe')
    $controllerCargoHome = Get-CanonicalPath -Path (Join-Path $userProfile '.cargo')
    $controllerRustupHome = Get-CanonicalPath -Path (Join-Path $userProfile '.rustup')
    $controllerToolchainBin = Get-CanonicalPath -Path (
        Join-Path $controllerRustupHome ('toolchains\' + $task037RustToolchain + '\bin')
    )
    $controllerCargoExe = Get-CanonicalPath -Path (Join-Path $controllerToolchainBin 'cargo.exe')
    $controllerRustcExe = Get-CanonicalPath -Path (Join-Path $controllerToolchainBin 'rustc.exe')
    $controllerRustdocExe = Get-CanonicalPath -Path (Join-Path $controllerToolchainBin 'rustdoc.exe')
    $controllerToolDirectories = @(
        $controllerToolchainBin,
        (Split-Path -Parent $controllerGitExe),
        (Split-Path -Parent $controllerNodeExe),
        $system32,
        $windows,
        (Join-Path $system32 'Wbem'),
        (Get-CanonicalPath -Path $PSHOME)
    ) | Select-Object -Unique
    foreach ($directory in @(
        $controllerToolDirectories + @(
            $controllerCargoHome,
            $controllerRustupHome,
            $controllerProgramData
        )
    )) {
        Assert-NoReparsePath -Path $directory -FailureCode 'TASK037_CONTROLLER_TOOL_SCOPE_REJECTED'
        $item = Get-Item -LiteralPath $directory -Force -ErrorAction SilentlyContinue
        if ($null -eq $item -or -not $item.PSIsContainer) {
            throw 'TASK037_CONTROLLER_TOOL_SCOPE_REJECTED'
        }
    }
    $controllerGitExe = Get-TrustedTask037Git -Candidate $controllerGitExe
    $controllerRustToolchain = Get-TrustedTask037RustToolchain `
        -CargoCandidate $controllerCargoExe `
        -RustcCandidate $controllerRustcExe `
        -RustdocCandidate $controllerRustdocExe `
        -CargoHome $controllerCargoHome
    Assert-Task037CargoConfigurationClosure `
        -RepositoryRoot $repositoryRoot `
        -CargoHome $controllerCargoHome
    $controllerCargoExe = $controllerRustToolchain.Cargo
    $controllerRustcExe = $controllerRustToolchain.Rustc
    $controllerRustdocExe = $controllerRustToolchain.Rustdoc
    Assert-RegularFile -Path $controllerNodeExe
    $preRunGitSnapshot = Get-Task037ClosedGitSnapshot `
        -GitExe $controllerGitExe `
        -RepositoryRoot $repositoryRoot `
        -AuthorizedTempBase $supervisorTempBase `
        -FailureCode 'TASK037_PRE_RUN_WORKTREE_REJECTED'
    if (-not [bool]$preRunGitSnapshot.Clean) {
        throw 'TASK037_WORKTREE_DIRTY_REJECTED'
    }
    $repositoryScheduledJobRoot = Join-Path $repositoryRoot 'Microsoft'
    if (Test-Path -LiteralPath $repositoryScheduledJobRoot) {
        throw 'TASK037_PRE_RUN_DIRECTORY_SHAPE_REJECTED'
    }
    $preRunDirectoryShape = Get-Task037RepositoryDirectoryShape `
        -RepositoryRoot $repositoryRoot `
        -FailureCode 'TASK037_PRE_RUN_DIRECTORY_SHAPE_REJECTED'
    $controllerPath = [string]::Join(';', $controllerToolDirectories)
    $supervision = $null
    $owned = $null
    $supervisorFailure = $null
    try {
        $supervision = New-Task037CleanupSupervision `
            -RepositoryTarget $repositoryTarget `
            -AcceptanceId $acceptanceId `
            -TempBase $supervisorTempBase
        $null = Write-Task037AdmissionStageReceipt `
            -RepositoryTarget $repositoryTarget `
            -AcceptanceId $acceptanceId `
            -StartedAtUtc $admissionStartedAtUtc `
            -Stage 'supervision_create'
        $powerShellExecutable = Get-TrustedTask037PowerShell
        $controllerStdout = Join-Path $supervision.TempRoot 'controller.stdout'
        $controllerStderr = Join-Path $supervision.TempRoot 'controller.stderr'
        $controllerWrapperStdout = Join-Path $supervision.TempRoot 'controller.wrapper.stdout'
        $controllerWrapperStderr = Join-Path $supervision.TempRoot 'controller.wrapper.stderr'
        $controllerPid = Join-Path $supervision.TempRoot 'controller.pid'
        $controllerArguments = @(
            '-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
            '-File', $PSCommandPath, '-InternalController', '-CodexAuthHome', $CodexAuthHome
        )
        if ($ControllerPreflightOnly) {
            $controllerArguments += '-InternalControllerPreflightOnly'
        }
        $beforeControllerGate = {
            $null = Write-Task037AdmissionStageReceipt `
                -RepositoryTarget $repositoryTarget `
                -AcceptanceId $acceptanceId `
                -StartedAtUtc $admissionStartedAtUtc `
                -Stage 'gated_controller_start'
        }.GetNewClosure()
        $owned = Start-Task037GatedProcess `
            -Job $supervision.Job `
            -AttestationJob $supervision.AttestationJob `
            -PowerShellExecutable $powerShellExecutable `
            -Executable $powerShellExecutable `
            -Arguments $controllerArguments `
            -WorkingDirectory $repositoryRoot `
            -EnvironmentValues ([ordered]@{
                CARGO_HOME = $controllerCargoHome
                CARGO_NET_OFFLINE = 'true'
                CARGO_TERM_COLOR = 'never'
                GIT_CONFIG_GLOBAL = 'NUL'
                GIT_CONFIG_NOSYSTEM = '1'
                GIT_CONFIG_SYSTEM = 'NUL'
                GIT_TERMINAL_PROMPT = '0'
                LATTICE_TASK037_SUPERVISOR_ACCEPTANCE_ID = $acceptanceId
                LATTICE_TASK037_SUPERVISOR_MANIFEST_PATH = $supervision.ManifestPath
                LATTICE_TASK037_SUPERVISOR_OWNER_TOKEN = $supervision.OwnerToken
                LATTICE_TASK037_SUPERVISOR_TEMP_BASE = $supervisorTempBase
                LATTICE_TASK037_SUPERVISOR_TEMP_ROOT = $supervision.TempRoot
                LATTICE_TASK037_ADMISSION_STARTED_AT_UTC = $admissionStartedAtUtc
                LATTICE_TASK037_CONTROLLER_CARGO_EXE = $controllerCargoExe
                LATTICE_TASK037_CONTROLLER_GIT_EXE = $controllerGitExe
                LATTICE_TASK037_CONTROLLER_NODE_EXE = $controllerNodeExe
                LATTICE_TASK037_CONTROLLER_PROGRAM_DATA = $controllerProgramData
                LATTICE_TASK037_CONTROLLER_RUSTC_EXE = $controllerRustcExe
                LATTICE_TASK037_CONTROLLER_RUSTDOC_EXE = $controllerRustdocExe
                PATH = $controllerPath
                RUSTC = $controllerRustcExe
                RUSTDOC = $controllerRustdocExe
                RUSTUP_HOME = $controllerRustupHome
                RUSTUP_TOOLCHAIN = $task037RustToolchain
            }) `
            -ChildPidPath $controllerPid `
            -ChildStdoutPath $controllerStdout `
            -ChildStderrPath $controllerStderr `
            -WrapperStdoutPath $controllerWrapperStdout `
            -WrapperStderrPath $controllerWrapperStderr `
            -RuntimeIsolationRoot (Join-Path $supervision.TempRoot 'controller-runtime') `
            -TrustedProgramData $controllerProgramData `
            -BeforeGate $beforeControllerGate
        if (-not $owned.ChildProcess.WaitForExit(2400000)) {
            throw 'TASK037_INNER_CONTROLLER_TIMEOUT'
        }
        $controllerExitCode = [int]$owned.ChildProcess.ExitCode
        if (-not $owned.Process.WaitForExit(10000)) {
            throw 'TASK037_INNER_CONTROLLER_REAP_REJECTED'
        }
        Complete-Task037GatedProcessEvidence -OwnedProcess $owned
        Assert-BoundedTask037EvidenceFile -Path $controllerStdout -Code 'TASK037_INNER_CONTROLLER_OUTPUT_REJECTED'
        Assert-BoundedTask037EvidenceFile -Path $controllerStderr -Code 'TASK037_INNER_CONTROLLER_OUTPUT_REJECTED'
        if ($ControllerPreflightOnly) {
            $preflightStdout = [IO.File]::ReadAllText(
                $controllerStdout,
                [Text.Encoding]::UTF8
            ).TrimEnd([char[]]"`r`n")
            $preflightStderr = [IO.File]::ReadAllText($controllerStderr, [Text.Encoding]::UTF8)
            if (
                $preflightStdout -cne 'TASK037_CONTROLLER_PREFLIGHT=PASS' -or
                -not [string]::IsNullOrEmpty($preflightStderr)
            ) {
                throw 'TASK037_CONTROLLER_PREFLIGHT_OUTPUT_REJECTED'
            }
        }
        if ($controllerExitCode -ne 0) {
            throw 'TASK037_INNER_CONTROLLER_REJECTED'
        }
    }
    catch {
        $supervisorFailure = $_.Exception.Message
    }
    finally {
        if ($null -ne $owned) {
            try { $owned.ChildProcess.Dispose() } catch { }
            try { $owned.Process.Dispose() } catch { }
        }
        if ($null -ne $supervision) {
            try {
                Complete-Task037CleanupSupervision -Supervision $supervision
            }
            catch {
                $supervisorFailure = 'TASK037_SUPERVISOR_CLEANUP_REJECTED'
            }
            finally {
                if ($supervision.Job -ne [IntPtr]::Zero) {
                    try { Stop-Task037Job -Job $supervision.Job } catch { }
                    try { Close-Task037Job -Job $supervision.Job } catch { }
                }
                if ($supervision.AttestationJob -ne [IntPtr]::Zero) {
                    try { Close-Task037Job -Job $supervision.AttestationJob } catch { }
                }
                if ($null -ne $supervision.MarkerLock) { try { $supervision.MarkerLock.Dispose() } catch { } }
                if ($null -ne $supervision.ManifestLock) { try { $supervision.ManifestLock.Dispose() } catch { } }
            }
        }
    }
    if (Test-Path -LiteralPath $repositoryScheduledJobRoot) {
        throw 'TASK037_POST_RUN_DIRECTORY_SHAPE_REJECTED'
    }
    $postRunDirectoryShape = Get-Task037RepositoryDirectoryShape `
        -RepositoryRoot $repositoryRoot `
        -FailureCode 'TASK037_POST_RUN_DIRECTORY_SHAPE_REJECTED'
    Assert-Task037RepositoryDirectoryShapeUnchanged `
        -Before $preRunDirectoryShape `
        -After $postRunDirectoryShape `
        -FailureCode 'TASK037_POST_RUN_DIRECTORY_SHAPE_REJECTED'
    $postRunGitSnapshot = Get-Task037ClosedGitSnapshot `
        -GitExe $controllerGitExe `
        -RepositoryRoot $repositoryRoot `
        -AuthorizedTempBase $supervisorTempBase `
        -FailureCode 'TASK037_POST_RUN_WORKTREE_REJECTED'
    if (
        -not (Test-ExactPath `
            -Actual $postRunGitSnapshot.RepositoryRoot `
            -Expected $preRunGitSnapshot.RepositoryRoot) -or
        [string]$postRunGitSnapshot.Branch -cne [string]$preRunGitSnapshot.Branch -or
        [string]$postRunGitSnapshot.Head -cne [string]$preRunGitSnapshot.Head
    ) {
        throw 'TASK037_POST_RUN_WORKTREE_IDENTITY_REJECTED'
    }
    if (-not [bool]$postRunGitSnapshot.Clean) {
        throw 'TASK037_POST_RUN_WORKTREE_DIRTY_REJECTED'
    }

    if ($null -ne $supervisorFailure) {
        Throw-Task037SafeFailure -Message $supervisorFailure
    }

    if ($ControllerPreflightOnly) {
        $admissionReceiptPath = Get-Task037AdmissionReceiptPath `
            -RepositoryTarget $repositoryTarget `
            -AcceptanceId $acceptanceId
        $admissionReceipt = Read-Task037AdmissionStageReceipt `
            -Path $admissionReceiptPath `
            -AcceptanceId $acceptanceId
        $acceptanceRoot = Join-Path $repositoryTarget ('full-chain-acceptance\' + $acceptanceId)
        $fixtureRoot = Join-Path $repositoryTarget ('lattice-delivery\' + $acceptanceId)
        if (
            [string]$admissionReceipt.stage -cne 'inner_pre_acceptance' -or
            (Test-Path -LiteralPath $acceptanceRoot) -or
            (Test-Path -LiteralPath $fixtureRoot) -or
            (Test-Path -LiteralPath $supervision.TempRoot) -or
            (Test-Path -LiteralPath $supervision.ManifestPath)
        ) {
            throw 'TASK037_CONTROLLER_PREFLIGHT_EVIDENCE_REJECTED'
        }
        [IO.File]::Delete($admissionReceiptPath)
        if (Test-Path -LiteralPath $admissionReceiptPath) {
            throw 'TASK037_CONTROLLER_PREFLIGHT_EVIDENCE_REJECTED'
        }
        Write-Output 'TASK037_CONTROLLER_PREFLIGHT=PASS'
        return
    }

    $evidenceRoot = Join-Path (Join-Path $repositoryTarget ('full-chain-acceptance\' + $acceptanceId)) 'evidence'
    $pendingFinalPath = Join-Path $evidenceRoot 'final.pending.json'
    $finalPath = Join-Path $evidenceRoot 'final.json'
    if (-not (Test-Path -LiteralPath $pendingFinalPath -PathType Leaf)) {
        throw 'TASK037_FINAL_EVIDENCE_REJECTED'
    }
    $final = Read-JsonEvidence -Path $pendingFinalPath
    if (
        [string]$final.status -cne 'PASS' -or
        [string]$final.component -cne 'task037-full-chain-verification' -or
        [string]$final.acceptance_id -cne $acceptanceId -or
        $final.status_replayed_after_postgres_restart -isnot [bool] -or
        $final.memory_counts_unchanged_during_status -isnot [bool] -or
        $final.pre_status_fail_closed -isnot [bool] -or
        -not $final.status_replayed_after_postgres_restart -or
        -not $final.memory_counts_unchanged_during_status -or
        -not $final.pre_status_fail_closed
    ) {
        throw 'TASK037_FINAL_EVIDENCE_REJECTED'
    }
    Promote-Task037FinalEvidence -PendingPath $pendingFinalPath -FinalPath $finalPath
    Write-Output 'TASK037_FULL_CHAIN_VERIFICATION=PASS'
    Write-Output (([ordered]@{
        status = 'PASS'
        component = 'task037-full-chain-verification'
        acceptance_id = $acceptanceId
        evidence_path = $finalPath
        postgres_run_id = [string]$final.postgres_run_id
        full_chain_receipt_digest = [string]$final.full_chain_receipt_digest
        hermes_reflection_digest = [string]$final.hermes_reflection_digest
    }) | ConvertTo-Json -Compress)
}

if ($HarnessSelfTest) {
    Invoke-Task037HarnessSelfTest
    return
}

if ($InternalAdmissionProbe) {
    Invoke-Task037InternalAdmissionProbe
    return
}

if (-not [string]::IsNullOrEmpty($InternalPhase)) {
    Invoke-FullChainInternalPhase -Phase $InternalPhase
    return
}

if ($InternalController) {
    Invoke-DefaultVerification
    return
}

try {
    if ($InternalControllerPreflightOnly) {
        throw 'TASK037_UNSUPERVISED_CONTROLLER_REJECTED'
    }
    if ($HarnessPublicFailureProbe) {
        throw 'task037-public-failure-probe-payload'
    }
    if ($HarnessControllerPreflightProbe) {
        Invoke-Task037SupervisedVerification -ControllerPreflightOnly
    }
    else {
        Invoke-Task037SupervisedVerification
    }
}
catch {
    Throw-Task037SafeFailure -Message $_.Exception.Message
}
