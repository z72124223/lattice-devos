[CmdletBinding()]
param(
    [string]$LatticedExecutable,

    [ValidatePattern('^[0-9a-fA-F]{64}$')]
    [string]$ExpectedBinarySha256,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ExpectedSourceCommit,

    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ExpectedSourceTree,

    [ValidatePattern('^[0-9a-f]{64}$')]
    [string]$ExpectedToolSchemaContractSha256,

    [ValidatePattern('^[0-9a-f]{64}$')]
    [string]$ExpectedToolErrorContractSha256,

    [ValidatePattern('^[0-9a-f]{64}$')]
    [string]$CurrentCandidateReviewCommitment,

    [ValidatePattern('^[0-9a-f]{64}$')]
    [string]$CurrentCandidateAcceptanceCommitment,

    [string]$TunnelLifecycleReceiptPath,

    [string]$HarnessObservedCounterPath,

    [ValidateSet('DISCOVERY_ONLY', 'PROTOCOL_ONLY', 'FULL')]
    [string]$Mode = 'FULL',

    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._:-]{0,51}$')]
    [string]$ClientRequestId = ('task038-accept-' + [Guid]::NewGuid().ToString('N').Substring(0, 16)),

    [string]$SourceRepository = (Split-Path -Parent $PSScriptRoot),

    [string]$EvidenceRoot,

    [ValidateRange(5, 420)]
    [int]$SessionTimeoutSeconds = 360,

    [string]$PsqlExecutable,

    [ValidatePattern('^127\.0\.0\.1$')]
    [string]$PostgresHost = '127.0.0.1',

    [ValidateRange(1, 65535)]
    [int]$PostgresPort,

    [ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })]
    [string]$PostgresRunId,

    [switch]$RequirePostgresRestart,

    [string]$PgCtlExecutable,

    [string]$PostgresDataDirectory,

    [ValidatePattern('^[A-Z][A-Z0-9_]{0,127}$')]
    [string]$PostgresPasswordVariable = 'LATTICE_TASK019_PASSWORD'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:SchemaVersion = 'lattice.task038.four-tool-acceptance.v1'
$script:CleanSeedCommit = '2b424ec9a5401a6fbdc4f37d3d401592331afca0'
$script:CleanSeedTree = '9c4cad5b4b3e3362521643b6dd283d31cde29345'
$script:HistoricalRejectedCommitPrefixes = @(
    '09264024',
    'b4cbe19cace38a2b100150d7faf5d695e6e8b685',
    'dd13770',
    'f9ae267ba3d335aa67bdd9548aadf7218a90c391'
)
$script:ReviewReceiptSourceThread = '019fefd2-171b-7743-93f2-16aea11a8f94'
$script:ReviewTargetCommit = '6ec0f25cd9b89c10e440c7d9e452d61c89d7e527'
$script:ReviewTargetTree = '96f099dc3a2aeee54c60642271031b12de7a36c1'
$script:ReviewTargetParent = '30ab9d7349d8897b9eaa78a918a5ae6d49d2eda4'
$script:P005LifecycleCommit = '392c39cbbc8d416b5d89b872b0be119336946247'
$script:P005LifecycleTree = '4cf03b8177aa3094d6823352986b52422bf86a33'
$script:P005LifecycleParent = $script:ReviewTargetCommit
$script:P007Commit = 'db56d471a1eec2dece06523661e3b571d345cbb2'
$script:P007Tree = '4bd6102f6a83b5984bcc993b74a090a33dcbcea9'
$script:P007Parent = '5087250c17659df7472d46013cc0a199834a2c73'
$script:PostgresVersion = '17.10'
$script:PostgresSha256 = '882a5a073a88817f6c6d4c8827df1e4269ff226d52cf6f47c9883e91088c6345'
$script:PsqlSha256 = 'e43adb9c5032e7efc63eebb44c5d32b142b34e5f4207666fed2dc7a51d43b630'
$script:PgCtlSha256 = 'abe89b0767a8cd0f956059aa5a5a93cd1042efc6194d000c2501da3e23babbd2'
$script:ReviewTargetBlobs = [ordered]@{
    'scripts/run-task038-four-tool-acceptance.ps1' = '6d9fab7d38cead46bbfb08dcdaa144c9302f261b'
    'scripts/test-task038-four-tool-acceptance.ps1' = '385ada4dbc7177e40215d893efebf9597114a6dc'
}
$script:P005LifecycleBlobs = [ordered]@{
    'scripts/start-chatgpt-mcp-tunnel.ps1' = 'b555489e80dcebddd7edf7dd8ab8241b86149cc7'
    'scripts/test-chatgpt-mcp-tunnel-entrypoint.ps1' = '4e6e0d66b297559fe1c2db228c35360c704f396b'
}
$script:P005ProductionCommit = '236b1d6f8362da19d298d65fd652e045cd413a02'
$script:P005ProductionTree = '3d2a01383be7b27871e86fe2df42fe0fc8728be9'
$script:P005ProductionParent = $script:P005LifecycleCommit
$script:P005ProductionBlobs = [ordered]@{
    'apps/lattice-runtime/src/mcp.rs' = 'c889b098c26c8669bd84d36ddd1368ed087fcd3e'
    'apps/lattice-runtime/tests/mcp.rs' = '49035f57ccba7fa38aa664e3bb03fc1dd76bdcd3'
    'scripts/run-task019-postgres.ps1' = '1c4a460744f6471017873675f473671851267846'
    'scripts/run-task038-task-submit.ps1' = '64b10e551911ef5a9ce13f4b4a6ab478fb4ebeb5'
    'scripts/start-chatgpt-mcp-tunnel.ps1' = '4a580765a9f38a965ddb94dbca9ebd6d1bb907ba'
    'scripts/test-chatgpt-mcp-tunnel-entrypoint.ps1' = '0d0a53fd1636a9e4b9e812ea3e360a5117aa28a5'
    'scripts/test-task038-local-acceptance.ps1' = '03c85db4fe857fc218544277279b42c2a897d205'
    'scripts/windows-native-path-identity.ps1' = 'bf872f89f4419b85a5b18e9c5f8f35dfd9772217'
}
$script:P007Blobs = [ordered]@{
    'Cargo.lock' = '337df502cbcb711fb5f64702083873626738622e'
    'apps/lattice-runtime/Cargo.toml' = 'cef328ce466c4624867e8c1e76906834cb9e9175'
    'apps/lattice-runtime/src/composition.rs' = '16cd3fb43b64739d7b1e6254e2a367bdf15f58b8'
    'apps/lattice-runtime/tests/composition.rs' = 'aa64f2c3f96ea80043bb53624e337c028f83e20b'
}
$script:CandidateBindingPaths = @(
    'Cargo.lock',
    'apps/lattice-runtime/Cargo.toml',
    'apps/lattice-runtime/src/composition.rs',
    'apps/lattice-runtime/src/mcp.rs',
    'apps/lattice-runtime/tests/mcp.rs',
    'apps/lattice-runtime/tests/composition.rs',
    'docs/tickets/TASK-038-chatgpt-mcp-gateway.md',
    'scripts/run-task019-postgres.ps1',
    'scripts/run-task038-four-tool-acceptance.ps1',
    'scripts/run-task038-task-submit.ps1',
    'scripts/start-chatgpt-mcp-tunnel.ps1',
    'scripts/test-chatgpt-mcp-tunnel-entrypoint.ps1',
    'scripts/test-task038-four-tool-acceptance.ps1',
    'scripts/test-task038-local-acceptance.ps1',
    'scripts/windows-native-path-identity.ps1'
)
$script:ExpectedTools = @(
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_task_status',
    'lattice_task_submit'
)
$script:SafeToolCodeContractHistoricalSourceCommit = '09264024'
$script:SafeToolCodes = @(
    'LATTICED_CODEX_CONFIGURATION_REJECTED',
    'LATTICED_CONFIGURATION_REJECTED',
    'LATTICED_DATABASE_CONNECT_REJECTED',
    'LATTICED_DATABASE_SECRET_MISSING',
    'LATTICED_LEDGER_CONFIGURATION_REJECTED',
    'LATTICED_STDIO_REJECTED',
    'LATTICED_WORKSPACE_CONFIGURATION_REJECTED',
    'LATTICE_DELIVERY_CONTRACT_REJECTED',
    'LATTICE_DELIVERY_FAILED',
    'LATTICE_DELIVERY_INTENT_REJECTED',
    'LATTICE_DELIVERY_OUTCOME_PERSIST_REJECTED',
    'LATTICE_DELIVERY_RECEIPT_MISMATCH',
    'LATTICE_DELIVERY_RECEIPT_REJECTED',
    'LATTICE_DELIVERY_RECONCILIATION_REQUIRED',
    'LATTICE_DELIVERY_RUN_REQUIRES_CANONICAL_LATTICED',
    'LATTICE_DELIVERY_RUN_STATUS_ONLY',
    'LATTICE_FULL_CHAIN_BINDING_REJECTED',
    'LATTICE_GRAPH_MEMORY_CONFIGURATION_REJECTED',
    'LATTICE_GRAPH_MEMORY_RECEIPT_REJECTED',
    'LATTICE_GRAPH_MEMORY_RUN_REJECTED',
    'LATTICE_HERMES_MEMORY_RECEIPT_REJECTED',
    'LATTICE_HERMES_PRODUCTION_RUNNER_REQUIRED',
    'LATTICE_HERMES_REFLECTION_REJECTED',
    'LATTICE_OFFICIAL_CODEX_IDENTITY_REJECTED',
    'LATTICE_SCRIPTED_FIXTURE_REJECTED',
    'LATTICE_TASK_CONTROL_REJECTED',
    'LATTICE_TASK_PUBLIC_STATUS_REJECTED',
    'LATTICE_TASK_RECONCILIATION_REQUIRED',
    'LATTICE_TASK_REFERENCE_REJECTED',
    'LATTICE_TASK_REQUEST_REJECTED',
    'LATTICE_TASK_REQUEST_SUBSTITUTED',
    'LATTICE_TASK_STATE_MISMATCH',
    'LATTICE_TASK_SUBMIT_STATUS_ONLY',
    'LATTICE_WRITER_LEASE_MISMATCH',
    'LATTICE_WRITER_LEASE_REJECTED'
)
$script:PublicStatusFields = @(
    'ledger_head_digest',
    'result_digest',
    'schema_version',
    'status',
    'task_ref',
    'task_state'
)
$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:CompletedStages = [Collections.Generic.List[string]]::new()
$script:SessionEvidence = [Collections.Generic.List[object]]::new()
$script:FailureCode = $null
$script:FailureExceptionType = $null
$script:FailureLineNumber = $null
$script:FailureMissingProperty = $null
$script:EvidenceDirectory = $null
$script:PublicSubmit = $null
$script:PublicStatus = $null
$script:DatabaseBefore = $null
$script:DatabaseAfterSubmit = $null
$script:DatabaseAfterStatus = $null
$script:PostgresBefore = $null
$script:PostgresAfter = $null
$script:PostgresRestart = $null
$script:ObservedTools = @()
$script:NegativeProtocolCases = [Collections.Generic.List[object]]::new()
$script:DatabaseAfterNegativeProtocol = $null
$script:SafeToolCodeCommitment = $null
$script:ToolSchemaContractCommitment = $null
$script:TunnelLifecycleIntegrationChecked = $false
$script:CandidateBuildEvidence = $null
$script:LatticedNativeIdentity = $null
$script:CandidateLinkage = $null
$script:CandidateReviewCommitment = $null
$script:CandidateAcceptanceCommitment = $null
$script:TunnelLifecycleReceipt = $null
$script:HarnessCounterPath = $null
$script:ProductionDatabasePassword = $null
$script:ProductionCodexHome = $null
$script:ProductionDeliveryRoot = $null
$script:PostgresBinding = $null
$script:ProductionSessionEffects = [Collections.Generic.List[object]]::new()

function Get-CanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return [IO.Path]::GetFullPath($Path).TrimEnd([IO.Path]::DirectorySeparatorChar)
}

function Get-StringSha256 {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash($script:Utf8.GetBytes($Value)))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

$script:SafeToolCodeCommitment = Get-StringSha256 -Value ([string]::Join("`n", $script:SafeToolCodes))

function Get-FileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)

    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Initialize-FileIdentityInterop {
    if ($null -ne ('LatticeTask038NativeFileIdentity' -as [type])) { return }

    Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

public static class LatticeTask038NativeFileIdentity
{
    [StructLayout(LayoutKind.Sequential)]
    private struct FileTime
    {
        public uint Low;
        public uint High;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ByHandleFileInformation
    {
        public uint FileAttributes;
        public FileTime CreationTime;
        public FileTime LastAccessTime;
        public FileTime LastWriteTime;
        public uint VolumeSerialNumber;
        public uint FileSizeHigh;
        public uint FileSizeLow;
        public uint NumberOfLinks;
        public uint FileIndexHigh;
        public uint FileIndexLow;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetFileInformationByHandle(
        SafeFileHandle handle,
        out ByHandleFileInformation information);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFile(
        string fileName,
        uint desiredAccess,
        uint shareMode,
        IntPtr securityAttributes,
        uint creationDisposition,
        uint flagsAndAttributes,
        IntPtr templateFile);

    private static string Token(SafeFileHandle handle)
    {
        ByHandleFileInformation information;
        if (!GetFileInformationByHandle(handle, out information))
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        return information.VolumeSerialNumber.ToString("x8") + ":" +
            information.FileIndexHigh.ToString("x8") +
            information.FileIndexLow.ToString("x8");
    }

    public static string Read(string path)
    {
        using (FileStream stream = new FileStream(
            path,
            FileMode.Open,
            FileAccess.Read,
            FileShare.Read | FileShare.Delete,
            4096,
            FileOptions.SequentialScan))
        {
            return Token(stream.SafeFileHandle);
        }
    }

    public static string ReadDirectory(string path)
    {
        const uint FileReadAttributes = 0x80;
        const uint ShareReadWriteDelete = 0x7;
        const uint OpenExisting = 3;
        const uint BackupSemantics = 0x02000000;
        using (SafeFileHandle handle = CreateFile(
            path, FileReadAttributes, ShareReadWriteDelete, IntPtr.Zero,
            OpenExisting, BackupSemantics, IntPtr.Zero))
        {
            if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error());
            return Token(handle);
        }
    }
}
'@
}

function Get-NativeFileIdentity {
    param([Parameter(Mandatory = $true)][string]$Path)

    Initialize-FileIdentityInterop
    try {
        return [LatticeTask038NativeFileIdentity]::Read($Path)
    }
    catch {
        throw 'TASK038_ACCEPT_BINARY_NATIVE_IDENTITY_REJECTED'
    }
}

function Get-NativeDirectoryIdentity {
    param([Parameter(Mandatory = $true)][string]$Path)

    Initialize-FileIdentityInterop
    try { return [LatticeTask038NativeFileIdentity]::ReadDirectory((Get-CanonicalPath -Path $Path)) }
    catch { throw 'TASK038_ACCEPT_NATIVE_DIRECTORY_IDENTITY_REJECTED' }
}

function Get-AuthoritativeNativeFileIdentity {
    param([Parameter(Mandatory = $true)][string]$Path)

    try {
        if ($null -ne (Get-Command Get-LatticeWindowsNativePathIdentityToken -CommandType Function -ErrorAction SilentlyContinue)) {
            return Get-LatticeWindowsNativePathIdentityToken -Path (Get-CanonicalPath -Path $Path) -Directory $false
        }
        return Get-NativeFileIdentity -Path $Path
    }
    catch { throw 'TASK038_ACCEPT_NATIVE_FILE_IDENTITY_REJECTED' }
}

function Assert-CandidateBinaryUnchanged {
    if (
        [string]::IsNullOrWhiteSpace($script:Latticed) -or
        [string]::IsNullOrWhiteSpace($script:LatticedNativeIdentity) -or
        -not (Test-Path -LiteralPath $script:Latticed -PathType Leaf) -or
        (Get-AuthoritativeNativeFileIdentity -Path $script:Latticed) -cne $script:LatticedNativeIdentity -or
        (Get-FileSha256 -Path $script:Latticed) -cne [string]$script:CandidateBuildEvidence.binary_sha256
    ) {
        throw 'TASK038_ACCEPT_BINARY_IDENTITY_CHANGED'
    }
}

function New-ExactCandidateBuild {
    param(
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string]$Tree
    )

    $archivePath = Join-Path $script:EvidenceDirectory 'candidate-source.tar'
    $materializedRoot = Join-Path $script:EvidenceDirectory 'candidate-source'
    $targetRoot = Join-Path $script:EvidenceDirectory 'candidate-target'
    [IO.Directory]::CreateDirectory($materializedRoot) | Out-Null
    [IO.Directory]::CreateDirectory($targetRoot) | Out-Null

    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $archiveOutput = @(& git -C $script:SourceRoot archive '--format=tar' ('--output=' + $archivePath) $Commit 2>&1)
        $archiveExit = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    if ($archiveExit -ne 0 -or -not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        throw 'TASK038_ACCEPT_CANDIDATE_ARCHIVE_REJECTED'
    }

    $tar = Get-Command tar.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($null -eq $tar -or -not (Test-Path -LiteralPath $tar.Source -PathType Leaf)) {
        throw 'TASK038_ACCEPT_CANDIDATE_MATERIALIZATION_REJECTED'
    }
    try {
        $ErrorActionPreference = 'Continue'
        $extractOutput = @(& $tar.Source '-xf' $archivePath '-C' $materializedRoot 2>&1)
        $extractExit = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    if ($extractExit -ne 0 -or -not (Test-Path -LiteralPath (Join-Path $materializedRoot 'Cargo.lock') -PathType Leaf)) {
        throw 'TASK038_ACCEPT_CANDIDATE_MATERIALIZATION_REJECTED'
    }

    $cargo = Get-Command cargo.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($null -eq $cargo -or -not (Test-Path -LiteralPath $cargo.Source -PathType Leaf)) {
        throw 'TASK038_ACCEPT_CANDIDATE_BUILD_TOOL_REJECTED'
    }
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $cargo.Source
    $startInfo.Arguments = 'build --locked --package lattice-runtime --bin latticed --target-dir "' + $targetRoot.Replace('"', '\"') + '"'
    $startInfo.WorkingDirectory = $materializedRoot
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw 'TASK038_ACCEPT_CANDIDATE_BUILD_REJECTED' }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($SessionTimeoutSeconds * 1000)) {
            $process.Kill()
            $null = $process.WaitForExit(5000)
            throw 'TASK038_ACCEPT_CANDIDATE_BUILD_TIMEOUT'
        }
        $stdout = [string]$stdoutTask.GetAwaiter().GetResult()
        $stderr = [string]$stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0 -or $stdout.Length -gt 1048576 -or $stderr.Length -gt 1048576) {
            throw 'TASK038_ACCEPT_CANDIDATE_BUILD_REJECTED'
        }
        $candidateBinary = Get-CanonicalPath -Path (Join-Path $targetRoot 'debug\latticed.exe')
        $candidateItem = Get-Item -LiteralPath $candidateBinary -Force -ErrorAction SilentlyContinue
        if (
            $null -eq $candidateItem -or
            $candidateItem.PSIsContainer -or
            -not ($candidateItem -is [IO.FileInfo]) -or
            ($candidateItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
        ) {
            throw 'TASK038_ACCEPT_CANDIDATE_BUILD_OUTPUT_REJECTED'
        }
        $binaryIdentity = Get-AuthoritativeNativeFileIdentity -Path $candidateBinary
        return [ordered]@{
            schema_version = 'lattice.task038.candidate-build.v1'
            source_commit = $Commit
            source_tree = $Tree
            source_archive_sha256 = Get-FileSha256 -Path $archivePath
            cargo_sha256 = Get-FileSha256 -Path $cargo.Source
            cargo_native_identity = Get-AuthoritativeNativeFileIdentity -Path $cargo.Source
            build_locked = $true
            build_package = 'lattice-runtime'
            build_binary = 'latticed'
            binary_path = $candidateBinary
            binary_sha256 = Get-FileSha256 -Path $candidateBinary
            binary_length = [long]$candidateItem.Length
            binary_native_identity = $binaryIdentity
            stdout_sha256 = Get-StringSha256 -Value $stdout
            stderr_sha256 = Get-StringSha256 -Value $stderr
            raw_output_retained = $false
        }
    }
    finally {
        $process.Dispose()
    }
}

function Write-SafeJson {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value
    )

    $json = $Value | ConvertTo-Json -Depth 16
    if ($json -match '(?i)(?:https?://|bearer\s|authorization\s*[:=]|api[_-]?key\s*[:=]|token\s*[:=]|password\s*[:=]|"raw_(?:error|prompt)"\s*:)') {
        throw 'TASK038_ACCEPT_EVIDENCE_SECRET_REJECTED'
    }
    [IO.File]::WriteAllText($Path, $json + "`n", $script:Utf8)
}

function Get-SafeFailureCode {
    param([Parameter(Mandatory = $true)]$ErrorRecord)

    $message = [string]$ErrorRecord.Exception.Message
    $match = [regex]::Match($message, '(?<![A-Z0-9_])TASK038_ACCEPT_[A-Z0-9_]{1,95}(?![A-Z0-9_])')
    if ($match.Success) {
        return $match.Value
    }
    return 'TASK038_ACCEPT_OTHER'
}

function Get-SafeToolCode {
    param([Parameter(Mandatory = $true)]$StructuredContent)

    $codeProperty = $StructuredContent.PSObject.Properties['code']
    if ($null -eq $codeProperty -or -not ($codeProperty.Value -is [string])) {
        throw 'TASK038_ACCEPT_TOOL_CODE_REJECTED'
    }
    $code = [string]$codeProperty.Value
    if (-not ($script:SafeToolCodes -ccontains $code)) {
        throw 'TASK038_ACCEPT_TOOL_CODE_REJECTED'
    }
    return $code
}

function Get-Task019ProductionDatabaseName {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateScript({ $_ -cmatch '\A[0-9a-f]{32}\z' })]
        [string]$RunId
    )

    return 'lattice_task019_' + $RunId.Substring(0, 8) + '_base'
}

function Invoke-GitText {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)

    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& git -C $script:SourceRoot @Arguments 2>&1)
        $exitCode = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    if ($exitCode -ne 0) {
        throw 'TASK038_ACCEPT_SOURCE_GIT_REJECTED'
    }
    return ([string]::Join("`n", @($output | ForEach-Object { [string]$_ }))).Trim()
}

function Test-GitAncestor {
    param(
        [Parameter(Mandatory = $true)][string]$Ancestor,
        [Parameter(Mandatory = $true)][string]$Descendant
    )

    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $null = & git -C $script:SourceRoot merge-base --is-ancestor $Ancestor $Descendant 2>&1
        $exitCode = [int]$LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    if ($exitCode -eq 0) { return $true }
    if ($exitCode -eq 1) { return $false }
    throw 'TASK038_ACCEPT_SOURCE_GIT_REJECTED'
}

function Get-GitBlobMap {
    param(
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string[]]$Paths
    )

    $map = [ordered]@{}
    foreach ($path in $Paths) {
        $blob = Invoke-GitText -Arguments @('rev-parse', ($Commit + ':' + $path))
        if ($blob -notmatch '^[0-9a-f]{40}$') {
            throw 'TASK038_ACCEPT_CHECKPOINT_BLOB_REJECTED'
        }
        $map[$path] = $blob
    }
    return $map
}

function Get-ExactCheckpointIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Commit,
        [Parameter(Mandatory = $true)][string]$Tree,
        [Parameter(Mandatory = $true)][string]$Parent,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$ExpectedBlobs,
        [Parameter(Mandatory = $true)][string]$CandidateCommit
    )

    $resolvedCommit = Invoke-GitText -Arguments @('rev-parse', ($Commit + '^{commit}'))
    $resolvedTree = Invoke-GitText -Arguments @('rev-parse', ($Commit + '^{tree}'))
    $resolvedParent = Invoke-GitText -Arguments @('show', '-s', '--format=%P', $Commit)
    if (
        $resolvedCommit -cne $Commit -or
        $resolvedTree -cne $Tree -or
        $resolvedParent -cne $Parent -or
        -not (Test-GitAncestor -Ancestor $Commit -Descendant $CandidateCommit)
    ) {
        throw 'TASK038_ACCEPT_CHECKPOINT_LINKAGE_REJECTED'
    }
    $observedBlobs = Get-GitBlobMap -Commit $Commit -Paths @($ExpectedBlobs.Keys)
    foreach ($path in $ExpectedBlobs.Keys) {
        if ([string]$observedBlobs[$path] -cne [string]$ExpectedBlobs[$path]) {
            throw 'TASK038_ACCEPT_CHECKPOINT_BLOB_REJECTED'
        }
    }
    return [ordered]@{
        name = $Name
        commit = $resolvedCommit
        tree = $resolvedTree
        parent = $resolvedParent
        exact_blobs = $observedBlobs
        ancestor_of_candidate = $true
    }
}

function Get-CandidateLinkage {
    param(
        [Parameter(Mandatory = $true)][string]$CandidateCommit,
        [Parameter(Mandatory = $true)][string]$CandidateTree
    )

    $reviewTarget = Get-ExactCheckpointIdentity `
        -Name 'P0-06_REVIEW_TARGET' `
        -Commit $script:ReviewTargetCommit `
        -Tree $script:ReviewTargetTree `
        -Parent $script:ReviewTargetParent `
        -ExpectedBlobs $script:ReviewTargetBlobs `
        -CandidateCommit $CandidateCommit
    $p005Lifecycle = Get-ExactCheckpointIdentity `
        -Name 'P0-05_LIFECYCLE_CHECKPOINT' `
        -Commit $script:P005LifecycleCommit `
        -Tree $script:P005LifecycleTree `
        -Parent $script:P005LifecycleParent `
        -ExpectedBlobs $script:P005LifecycleBlobs `
        -CandidateCommit $CandidateCommit
    $p007 = Get-ExactCheckpointIdentity `
        -Name 'P0-07_OFFICIAL_BUNDLE_CHECKPOINT' `
        -Commit $script:P007Commit `
        -Tree $script:P007Tree `
        -Parent $script:P007Parent `
        -ExpectedBlobs $script:P007Blobs `
        -CandidateCommit $CandidateCommit
    $p005Production = Get-ExactCheckpointIdentity `
        -Name 'P0-05_PRODUCTION_EVIDENCE_CHECKPOINT' `
        -Commit $script:P005ProductionCommit `
        -Tree $script:P005ProductionTree `
        -Parent $script:P005ProductionParent `
        -ExpectedBlobs $script:P005ProductionBlobs `
        -CandidateCommit $CandidateCommit
    return [ordered]@{
        schema_version = 'lattice.task038.candidate-linkage.v1'
        candidate_commit = $CandidateCommit
        candidate_tree = $CandidateTree
        candidate_exact_blobs = Get-GitBlobMap -Commit $CandidateCommit -Paths $script:CandidateBindingPaths
        independent_review_source_thread = $script:ReviewReceiptSourceThread
        independent_review_target = $reviewTarget
        p005_lifecycle_checkpoint = $p005Lifecycle
        p005_production_checkpoint = $p005Production
        p007_checkpoint = $p007
        lifecycle_checkpoint_downstream_of_review_target = (
            (Test-GitAncestor -Ancestor $script:ReviewTargetCommit -Descendant $script:P005LifecycleCommit) -and
            $script:P005LifecycleCommit -cne $script:ReviewTargetCommit
        )
        lifecycle_runtime_receipt_required = $true
        lifecycle_checkpoint_does_not_retroactively_prove_review_target = $true
    }
}

function Get-DomainSeparatedCommitment {
    param(
        [Parameter(Mandatory = $true)][string]$Domain,
        [Parameter(Mandatory = $true)]$Value
    )

    return Get-StringSha256 -Value ($Domain + "`n" + (ConvertTo-CanonicalJson -Value $Value))
}

function Get-LifecycleProcessIdentityParts {
    param(
        [Parameter(Mandatory = $true)]$Identity,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    Assert-ExactJsonKeys -Object $Identity -Expected @('pid', 'creation_time', 'creation_time_source', 'exe_sha256') -FailureCode $FailureCode
    if (
        -not (Test-JsonInteger -Value $Identity.pid) -or [long]$Identity.pid -lt 1 -or
        $Identity.creation_time -isnot [string] -or [string]$Identity.creation_time -cnotmatch '\A[0-9]{1,32}\z' -or
        $Identity.creation_time_source -isnot [string] -or [string]$Identity.creation_time_source -cnotmatch '\A(?:WINDOWS_PROCESS_TIMES|LINUX_PROC_STAT_START_TICKS|DARWIN_KINFO_PROC_START_TIME)\z' -or
        $Identity.exe_sha256 -isnot [string] -or [string]$Identity.exe_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
    ) { throw $FailureCode }
    return @([string][long]$Identity.pid, [string]$Identity.creation_time, [string]$Identity.creation_time_source, [string]$Identity.exe_sha256)
}

function Read-TunnelLifecycleReceipt {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedInnerExeSha256
    )

    $failureCode = 'TASK038_ACCEPT_TUNNEL_LIFECYCLE_RECEIPT_REJECTED'
    $receiptPath = Get-CanonicalPath -Path $Path
    $item = Get-Item -LiteralPath $receiptPath -Force -ErrorAction SilentlyContinue
    if ($null -eq $item -or $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Length -lt 1 -or $item.Length -gt 1048576) {
        throw $failureCode
    }
    $bytes = [IO.File]::ReadAllBytes($receiptPath)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf) { throw $failureCode }
    try { $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes) }
    catch { throw $failureCode }
    if (-not $text.EndsWith("`n", [StringComparison]::Ordinal) -or $text.Contains("`r") -or $text.TrimEnd("`n").Contains("`n")) { throw $failureCode }
    try { $outer = $text | ConvertFrom-Json -ErrorAction Stop }
    catch { throw $failureCode }
    Assert-ExactJsonKeys -Object $outer -Expected @(
        'schema', 'mode', 'process_id', 'tunnel_client_exit_code', 'started_at_utc', 'exited_at_utc',
        'create_suspended', 'job_assigned_before_resume', 'descendant_processes_after_cleanup',
        'profile_raw_sha256', 'profile_byte_count', 'profile_strict_utf8', 'profile_native_identity',
        'latticed_native_identity', 'tunnel_client_native_identity', 'lifecycle_event_path',
        'lifecycle_event_raw_sha256', 'lifecycle_event_byte_count', 'lifecycle_event_strict_utf8',
        'lifecycle_event_native_identity', 'lifecycle_session_id', 'lifecycle_config_generation',
        'lifecycle_safe_config_schema', 'lifecycle_safe_config_sha256', 'lifecycle_safe_config_byte_count',
        'lifecycle_event_count', 'lifecycle_anomaly_count', 'lifecycle_anomaly_codes',
        'lifecycle_chain_complete', 'lifecycle_normal_close_complete', 'lifecycle_final_event_sha256',
        'lifecycle_inner_process_id', 'lifecycle_inner_process_creation_time',
        'lifecycle_inner_process_creation_time_source', 'lifecycle_inner_process_exe_sha256',
        'lifecycle_inner_exit_code', 'lifecycle_threshold_decision', 'lifecycle_threshold_profile',
        'lifecycle_thresholds', 'lifecycle_classification', 'leak_claimed'
    ) -FailureCode $failureCode
    Assert-ExactJsonKeys -Object $outer.lifecycle_thresholds -Expected @(
        'pipe_milliseconds', 'exit_milliseconds', 'reap_milliseconds', 'confirm_milliseconds'
    ) -FailureCode $failureCode
    $outerStartedAt = [DateTimeOffset]::MinValue
    $outerExitedAt = [DateTimeOffset]::MinValue
    if (
        $outer.schema -isnot [string] -or [string]$outer.schema -cne 'lattice.task038.tunnel-outer-lifecycle.v1' -or
        $outer.mode -isnot [string] -or [string]$outer.mode -cnotmatch '\A(?:Run|ManagedRun)\z' -or
        -not (Test-JsonInteger -Value $outer.process_id) -or [int]$outer.process_id -lt 1 -or
        -not (Test-JsonInteger -Value $outer.tunnel_client_exit_code) -or [int]$outer.tunnel_client_exit_code -ne 0 -or
        $outer.started_at_utc -isnot [string] -or
        -not [DateTimeOffset]::TryParse([string]$outer.started_at_utc, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind, [ref]$outerStartedAt) -or
        $outer.exited_at_utc -isnot [string] -or
        -not [DateTimeOffset]::TryParse([string]$outer.exited_at_utc, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind, [ref]$outerExitedAt) -or
        $outerExitedAt -lt $outerStartedAt -or
        $outer.create_suspended -isnot [bool] -or -not [bool]$outer.create_suspended -or
        $outer.job_assigned_before_resume -isnot [bool] -or -not [bool]$outer.job_assigned_before_resume -or
        -not (Test-JsonInteger -Value $outer.descendant_processes_after_cleanup) -or [int]$outer.descendant_processes_after_cleanup -ne 0 -or
        $outer.profile_raw_sha256 -isnot [string] -or [string]$outer.profile_raw_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        -not (Test-JsonInteger -Value $outer.profile_byte_count) -or [long]$outer.profile_byte_count -lt 1 -or
        $outer.profile_strict_utf8 -isnot [bool] -or -not [bool]$outer.profile_strict_utf8 -or
        $outer.profile_native_identity -isnot [string] -or [string]$outer.profile_native_identity -cnotmatch '\Alattice\.win-file-id\.v1:[0-9a-f:]+:f\z' -or
        $outer.latticed_native_identity -isnot [string] -or [string]$outer.latticed_native_identity -cnotmatch '\Alattice\.win-file-id\.v1:[0-9a-f:]+:f\z' -or
        $outer.tunnel_client_native_identity -isnot [string] -or [string]$outer.tunnel_client_native_identity -cnotmatch '\Alattice\.win-file-id\.v1:[0-9a-f:]+:f\z' -or
        $outer.lifecycle_event_path -isnot [string] -or
        $outer.lifecycle_event_raw_sha256 -isnot [string] -or [string]$outer.lifecycle_event_raw_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        -not (Test-JsonInteger -Value $outer.lifecycle_event_byte_count) -or [long]$outer.lifecycle_event_byte_count -lt 1 -or
        $outer.lifecycle_event_strict_utf8 -isnot [bool] -or -not [bool]$outer.lifecycle_event_strict_utf8 -or
        $outer.lifecycle_event_native_identity -isnot [string] -or [string]$outer.lifecycle_event_native_identity -cnotmatch '\Alattice\.win-file-id\.v1:[0-9a-f:]+:f\z' -or
        $outer.lifecycle_session_id -isnot [string] -or [string]$outer.lifecycle_session_id -cnotmatch '\A[0-9a-f]{32}\z' -or
        -not (Test-JsonInteger -Value $outer.lifecycle_config_generation) -or [long]$outer.lifecycle_config_generation -lt 1 -or
        $outer.lifecycle_safe_config_schema -isnot [string] -or [string]$outer.lifecycle_safe_config_schema -cne 'lattice.task038.tunnel-safe-config.v1' -or
        $outer.lifecycle_safe_config_sha256 -isnot [string] -or [string]$outer.lifecycle_safe_config_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        -not (Test-JsonInteger -Value $outer.lifecycle_safe_config_byte_count) -or [long]$outer.lifecycle_safe_config_byte_count -lt 1 -or
        -not (Test-JsonInteger -Value $outer.lifecycle_event_count) -or [long]$outer.lifecycle_event_count -ne 6 -or
        -not (Test-JsonInteger -Value $outer.lifecycle_anomaly_count) -or [long]$outer.lifecycle_anomaly_count -ne 0 -or
        $outer.lifecycle_anomaly_codes -isnot [object[]] -or @($outer.lifecycle_anomaly_codes).Count -ne 0 -or
        $outer.lifecycle_chain_complete -isnot [bool] -or -not [bool]$outer.lifecycle_chain_complete -or
        $outer.lifecycle_normal_close_complete -isnot [bool] -or -not [bool]$outer.lifecycle_normal_close_complete -or
        $outer.lifecycle_final_event_sha256 -isnot [string] -or [string]$outer.lifecycle_final_event_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        -not (Test-JsonInteger -Value $outer.lifecycle_inner_process_id) -or [long]$outer.lifecycle_inner_process_id -lt 1 -or
        $outer.lifecycle_inner_process_creation_time -isnot [string] -or [string]$outer.lifecycle_inner_process_creation_time -cnotmatch '\A[0-9]{1,32}\z' -or
        $outer.lifecycle_inner_process_creation_time_source -isnot [string] -or [string]$outer.lifecycle_inner_process_creation_time_source -cnotmatch '\A(?:WINDOWS_PROCESS_TIMES|LINUX_PROC_STAT_START_TICKS|DARWIN_KINFO_PROC_START_TIME)\z' -or
        $outer.lifecycle_inner_process_exe_sha256 -isnot [string] -or [string]$outer.lifecycle_inner_process_exe_sha256 -cne $ExpectedInnerExeSha256 -or
        -not (Test-JsonInteger -Value $outer.lifecycle_inner_exit_code) -or
        $outer.lifecycle_threshold_decision -isnot [string] -or [string]$outer.lifecycle_threshold_decision -cne 'C_CALIBRATION_FIRST' -or
        $null -ne $outer.lifecycle_threshold_profile -or
        $null -ne $outer.lifecycle_thresholds.pipe_milliseconds -or
        $null -ne $outer.lifecycle_thresholds.exit_milliseconds -or
        $null -ne $outer.lifecycle_thresholds.reap_milliseconds -or
        $null -ne $outer.lifecycle_thresholds.confirm_milliseconds -or
        $outer.lifecycle_classification -isnot [string] -or [string]$outer.lifecycle_classification -cne 'UNKNOWN' -or
        $outer.leak_claimed -isnot [bool] -or [bool]$outer.leak_claimed
    ) { throw $failureCode }
    $eventPath = Get-CanonicalPath -Path ([string]$outer.lifecycle_event_path)
    try { $observedEventIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $eventPath -Directory $false }
    catch { throw $failureCode }
    if (
        -not (Test-Path -LiteralPath $eventPath -PathType Leaf) -or
        $observedEventIdentity -cne [string]$outer.lifecycle_event_native_identity
    ) { throw $failureCode }
    $eventBytes = [IO.File]::ReadAllBytes($eventPath)
    if (
        $eventBytes.Length -ne [long]$outer.lifecycle_event_byte_count -or
        (Get-FileSha256 -Path $eventPath) -cne [string]$outer.lifecycle_event_raw_sha256 -or
        $eventBytes.Length -lt 1 -or $eventBytes.Length -gt 1048576 -or
        ($eventBytes.Length -ge 3 -and $eventBytes[0] -eq 0xef -and $eventBytes[1] -eq 0xbb -and $eventBytes[2] -eq 0xbf)
    ) { throw $failureCode }
    try { $eventText = [Text.UTF8Encoding]::new($false, $true).GetString($eventBytes) }
    catch { throw $failureCode }
    if (-not $eventText.EndsWith("`n", [StringComparison]::Ordinal) -or $eventText.Contains("`r")) { throw $failureCode }
    $lines = @($eventText.Split([string[]]@("`n"), [StringSplitOptions]::None))
    if ($lines[-1] -cne '') { throw $failureCode }
    $lines = @($lines[0..($lines.Count - 2)])
    if ($lines.Count -ne 6) { throw $failureCode }
    $eventTypes = @('SPAWN', 'OPEN', 'CLOSE_REQUESTED', 'PIPE_CLOSED', 'EXITED', 'REAPED')
    $previousEventSha256 = '0' * 64
    $previousObservedAt = [DateTimeOffset]::MinValue
    $stableIdentityParts = $null
    $stableCommandSha256 = $null
    $stableEndpointRef = $null
    $exitCode = $null
    for ($index = 0; $index -lt $lines.Count; $index++) {
        try { $record = $lines[$index] | ConvertFrom-Json -ErrorAction Stop }
        catch { throw $failureCode }
        Assert-ExactJsonKeys -Object $record -Expected @(
            'schema', 'record_type', 'component', 'event_type', 'session_id', 'process_identity',
            'config_generation', 'safe_config_sha256', 'session_command_sha256', 'endpoint_ref',
            'lifecycle_strategy', 'ordinal', 'observed_at_utc', 'exit_code', 'previous_event_sha256',
            'idempotency_key', 'event_sha256', 'lifecycle_classification', 'threshold_profile_version', 'thresholds'
        ) -FailureCode $failureCode
        Assert-ExactJsonKeys -Object $record.lifecycle_strategy -Expected @(
            'transport', 'endpoint_kind', 'spawn_mode', 'create_suspended_owned', 'job_assignment_ownership'
        ) -FailureCode $failureCode
        Assert-ExactJsonKeys -Object $record.thresholds -Expected @(
            'pipe_milliseconds', 'exit_milliseconds', 'reap_milliseconds', 'confirm_milliseconds'
        ) -FailureCode $failureCode
        $identityParts = Get-LifecycleProcessIdentityParts -Identity $record.process_identity -FailureCode $failureCode
        $observedAt = [DateTimeOffset]::MinValue
        if (
            $record.schema -isnot [string] -or [string]$record.schema -cne 'lattice.tunnel-client.lifecycle-event.v1' -or
            $record.record_type -isnot [string] -or [string]$record.record_type -cne 'LIFECYCLE' -or
            $record.component -isnot [string] -or [string]$record.component -cne 'mcpclient' -or
            $record.event_type -isnot [string] -or [string]$record.event_type -cne $eventTypes[$index] -or
            $record.session_id -isnot [string] -or [string]$record.session_id -cne [string]$outer.lifecycle_session_id -or
            -not (Test-JsonInteger -Value $record.config_generation) -or [long]$record.config_generation -ne [long]$outer.lifecycle_config_generation -or
            $record.safe_config_sha256 -isnot [string] -or [string]$record.safe_config_sha256 -cne [string]$outer.lifecycle_safe_config_sha256 -or
            $record.session_command_sha256 -isnot [string] -or [string]$record.session_command_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            $record.endpoint_ref -isnot [string] -or [string]$record.endpoint_ref -cnotmatch '\Ahmac-sha256:[0-9a-f]{64}\z' -or
            $record.lifecycle_strategy.transport -isnot [string] -or [string]$record.lifecycle_strategy.transport -cne 'STDIO' -or
            $record.lifecycle_strategy.endpoint_kind -isnot [string] -or [string]$record.lifecycle_strategy.endpoint_kind -cne 'ANONYMOUS_PIPE' -or
            $record.lifecycle_strategy.spawn_mode -isnot [string] -or [string]$record.lifecycle_strategy.spawn_mode -cne 'DIRECT' -or
            $record.lifecycle_strategy.create_suspended_owned -isnot [bool] -or [bool]$record.lifecycle_strategy.create_suspended_owned -or
            $record.lifecycle_strategy.job_assignment_ownership -isnot [string] -or [string]$record.lifecycle_strategy.job_assignment_ownership -cne 'EXTERNAL_OWNER' -or
            -not (Test-JsonInteger -Value $record.ordinal) -or [long]$record.ordinal -ne ($index + 1) -or
            $record.observed_at_utc -isnot [string] -or [string]$record.observed_at_utc -cnotmatch '\A\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z\z' -or
            -not [DateTimeOffset]::TryParse([string]$record.observed_at_utc, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind, [ref]$observedAt) -or
            $observedAt -lt $previousObservedAt -or
            $record.previous_event_sha256 -isnot [string] -or [string]$record.previous_event_sha256 -cne $previousEventSha256 -or
            $record.idempotency_key -isnot [string] -or [string]$record.idempotency_key -cnotmatch '\A[0-9a-f]{64}\z' -or
            $record.event_sha256 -isnot [string] -or [string]$record.event_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
            $record.lifecycle_classification -isnot [string] -or [string]$record.lifecycle_classification -cne 'UNKNOWN' -or
            $null -ne $record.threshold_profile_version -or
            $null -ne $record.thresholds.pipe_milliseconds -or $null -ne $record.thresholds.exit_milliseconds -or
            $null -ne $record.thresholds.reap_milliseconds -or $null -ne $record.thresholds.confirm_milliseconds
        ) { throw $failureCode }
        if ($index -lt 4) {
            if ($null -ne $record.exit_code) { throw $failureCode }
            $exitCodeText = 'null'
        }
        else {
            if (-not (Test-JsonInteger -Value $record.exit_code)) { throw $failureCode }
            $parsedExitCode = [int]$record.exit_code
            if ($null -eq $exitCode) { $exitCode = $parsedExitCode } elseif ($parsedExitCode -ne $exitCode) { throw $failureCode }
            $exitCodeText = [string]$parsedExitCode
        }
        if ($null -eq $stableIdentityParts) {
            if ($identityParts[3] -cne $ExpectedInnerExeSha256) { throw $failureCode }
            $stableIdentityParts = $identityParts
            $stableCommandSha256 = [string]$record.session_command_sha256
            $stableEndpointRef = [string]$record.endpoint_ref
        }
        elseif (
            ($identityParts -join "`n") -cne ($stableIdentityParts -join "`n") -or
            [string]$record.session_command_sha256 -cne $stableCommandSha256 -or
            [string]$record.endpoint_ref -cne $stableEndpointRef
        ) { throw $failureCode }
        $idempotency = Get-StringSha256 -Value (@(
            'lattice.tunnel-client.lifecycle-idempotency.v1', [string]$outer.lifecycle_session_id,
            [string]$outer.lifecycle_config_generation, [string]$outer.lifecycle_safe_config_sha256,
            $stableCommandSha256, $stableEndpointRef, [string]$record.event_type,
            $identityParts[0], $identityParts[1], $identityParts[2], $identityParts[3], $exitCodeText
        ) -join "`n")
        $eventSha256 = Get-StringSha256 -Value (@(
            'lattice.tunnel-client.lifecycle-event-hash.v1', $previousEventSha256,
            $idempotency, [string]($index + 1), [string]$record.observed_at_utc
        ) -join "`n")
        if ([string]$record.idempotency_key -cne $idempotency -or [string]$record.event_sha256 -cne $eventSha256) { throw $failureCode }
        $previousEventSha256 = $eventSha256
        $previousObservedAt = $observedAt
    }
    if (
        $previousEventSha256 -cne [string]$outer.lifecycle_final_event_sha256 -or
        [long]$stableIdentityParts[0] -ne [long]$outer.lifecycle_inner_process_id -or
        [string]$stableIdentityParts[1] -cne [string]$outer.lifecycle_inner_process_creation_time -or
        [string]$stableIdentityParts[2] -cne [string]$outer.lifecycle_inner_process_creation_time_source -or
        [string]$stableIdentityParts[3] -cne [string]$outer.lifecycle_inner_process_exe_sha256 -or
        [int]$exitCode -ne [int]$outer.lifecycle_inner_exit_code -or [int]$exitCode -ne 0
    ) { throw $failureCode }
    return [ordered]@{
        schema_version = 'lattice.task038.tunnel-lifecycle-validation.v1'
        outer_receipt_raw_sha256 = Get-FileSha256 -Path $receiptPath
        lifecycle_event_raw_sha256 = [string]$outer.lifecycle_event_raw_sha256
        lifecycle_final_event_sha256 = $previousEventSha256
        lifecycle_session_id = [string]$outer.lifecycle_session_id
        lifecycle_safe_config_sha256 = [string]$outer.lifecycle_safe_config_sha256
        lifecycle_inner_process_exe_sha256 = [string]$outer.lifecycle_inner_process_exe_sha256
        exact_schema_hash_order_idempotency_session_config_exe_checked = $true
        downstream_checkpoint = $script:P005LifecycleCommit
        downstream_checkpoint_not_retroactive_review_proof = $true
    }
}

function Invoke-PostgresRestart {
    if (-not $RequirePostgresRestart) { return $null }

    Assert-PostgresBindingUnchanged

    $pgCtl = Get-CanonicalPath -Path $PgCtlExecutable
    $dataDirectory = Get-CanonicalPath -Path $PostgresDataDirectory
    if (
        -not (Test-Path -LiteralPath $pgCtl -PathType Leaf) -or
        -not (Test-Path -LiteralPath $dataDirectory -PathType Container)
    ) {
        throw 'TASK038_ACCEPT_POSTGRES_RESTART_CONFIGURATION_REJECTED'
    }

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $pgCtl
    $startInfo.Arguments = 'restart -D "' + $dataDirectory.Replace('"', '\"') + '" -w -s'
    $startInfo.WorkingDirectory = Split-Path -Parent $pgCtl
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw 'TASK038_ACCEPT_POSTGRES_RESTART_REJECTED'
        }
        $processId = [int]$process.Id
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($SessionTimeoutSeconds * 1000)) {
            $process.Kill()
            $process.WaitForExit()
            throw 'TASK038_ACCEPT_POSTGRES_RESTART_TIMEOUT'
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw 'TASK038_ACCEPT_POSTGRES_RESTART_REJECTED'
        }
        Assert-PostgresBindingUnchanged
        return [ordered]@{
            process_id = $processId
            exit_code = [int]$process.ExitCode
            executable_sha256 = Get-FileSha256 -Path $pgCtl
            stdout_sha256 = Get-StringSha256 -Value $stdout
            stderr_sha256 = Get-StringSha256 -Value $stderr
            native_process_handle_identity = ($process.Handle -ne [IntPtr]::Zero)
            raw_output_retained = $false
        }
    }
    finally {
        $process.Dispose()
    }
}

function Initialize-JobObjectInterop {
    if ($null -ne ('LatticeTask038AcceptanceJob' -as [type])) { return }

    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class LatticeTask038AcceptanceJob
{
    private const UInt32 ExtendedInfo = 9;
    private const UInt32 KillOnClose = 0x00002000;

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

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateJobObject(IntPtr attributes, string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool SetInformationJobObject(IntPtr job, UInt32 infoClass, IntPtr info, UInt32 length);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool QueryInformationJobObject(IntPtr job, UInt32 infoClass, out BasicAccountingInformation info, UInt32 length, IntPtr returned);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool TerminateJobObject(IntPtr job, UInt32 exitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    public static IntPtr Create()
    {
        IntPtr job = CreateJobObject(IntPtr.Zero, null);
        if (job == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        var info = new ExtendedLimitInformation();
        info.BasicLimitInformation.LimitFlags = KillOnClose;
        int size = Marshal.SizeOf(typeof(ExtendedLimitInformation));
        IntPtr buffer = Marshal.AllocHGlobal(size);
        try
        {
            Marshal.StructureToPtr(info, buffer, false);
            if (!SetInformationJobObject(job, ExtendedInfo, buffer, (UInt32)size))
            {
                int error = Marshal.GetLastWin32Error();
                CloseHandle(job);
                throw new Win32Exception(error);
            }
        }
        finally { Marshal.FreeHGlobal(buffer); }
        return job;
    }

    public static void Assign(IntPtr job, IntPtr process)
    {
        if (!AssignProcessToJobObject(job, process)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }

    public static UInt32 Active(IntPtr job)
    {
        BasicAccountingInformation info;
        UInt32 size = (UInt32)Marshal.SizeOf(typeof(BasicAccountingInformation));
        if (!QueryInformationJobObject(job, 1, out info, size, IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error());
        return info.ActiveProcesses;
    }

    public static void Terminate(IntPtr job)
    {
        if (!TerminateJobObject(job, 1)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }

    public static void Close(IntPtr job)
    {
        if (job != IntPtr.Zero && !CloseHandle(job)) throw new Win32Exception(Marshal.GetLastWin32Error());
    }
}
'@
}

function Initialize-SuspendedProcessInterop {
    if ('LatticeTask038SuspendedProcess' -as [type]) { return }

    Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public sealed class LatticeTask038SuspendedProcess : IDisposable
{
    private const uint CREATE_SUSPENDED = 0x00000004;
    private const uint CREATE_NO_WINDOW = 0x08000000;
    private const uint STARTF_USESTDHANDLES = 0x00000100;
    private const uint HANDLE_FLAG_INHERIT = 0x00000001;
    private const uint RESUME_FAILED = 0xffffffff;

    [StructLayout(LayoutKind.Sequential)]
    private struct SECURITY_ATTRIBUTES
    {
        public int nLength;
        public IntPtr lpSecurityDescriptor;
        [MarshalAs(UnmanagedType.Bool)] public bool bInheritHandle;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct STARTUPINFO
    {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public uint dwX;
        public uint dwY;
        public uint dwXSize;
        public uint dwYSize;
        public uint dwXCountChars;
        public uint dwYCountChars;
        public uint dwFillAttribute;
        public uint dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput;
        public IntPtr hStdOutput;
        public IntPtr hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct PROCESS_INFORMATION
    {
        public IntPtr hProcess;
        public IntPtr hThread;
        public int dwProcessId;
        public int dwThreadId;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CreatePipe(out IntPtr read, out IntPtr write, ref SECURITY_ATTRIBUTES attributes, int size);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool SetHandleInformation(IntPtr handle, uint mask, uint flags);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CreateProcessW(
        string applicationName,
        StringBuilder commandLine,
        IntPtr processAttributes,
        IntPtr threadAttributes,
        bool inheritHandles,
        uint creationFlags,
        IntPtr environment,
        string currentDirectory,
        ref STARTUPINFO startupInfo,
        out PROCESS_INFORMATION processInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern uint ResumeThread(IntPtr thread);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool TerminateProcess(IntPtr process, uint exitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    public Process Process { get; private set; }
    public StreamWriter StandardInput { get; private set; }
    public StreamReader StandardOutput { get; private set; }
    public StreamReader StandardError { get; private set; }
    public bool CreatedSuspended { get; private set; }
    public bool AssignedBeforeResume { get; private set; }

    private LatticeTask038SuspendedProcess() { }

    public static LatticeTask038SuspendedProcess Start(string executable, string workingDirectory, IntPtr job)
    {
        IntPtr stdinRead = IntPtr.Zero;
        IntPtr stdinWrite = IntPtr.Zero;
        IntPtr stdoutRead = IntPtr.Zero;
        IntPtr stdoutWrite = IntPtr.Zero;
        IntPtr stderrRead = IntPtr.Zero;
        IntPtr stderrWrite = IntPtr.Zero;
        PROCESS_INFORMATION processInfo = new PROCESS_INFORMATION();
        bool processCreated = false;
        bool parentHandlesTransferred = false;
        try
        {
            SECURITY_ATTRIBUTES attributes = new SECURITY_ATTRIBUTES();
            attributes.nLength = Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES));
            attributes.bInheritHandle = true;
            if (!CreatePipe(out stdinRead, out stdinWrite, ref attributes, 0) ||
                !CreatePipe(out stdoutRead, out stdoutWrite, ref attributes, 0) ||
                !CreatePipe(out stderrRead, out stderrWrite, ref attributes, 0))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            if (!SetHandleInformation(stdinWrite, HANDLE_FLAG_INHERIT, 0) ||
                !SetHandleInformation(stdoutRead, HANDLE_FLAG_INHERIT, 0) ||
                !SetHandleInformation(stderrRead, HANDLE_FLAG_INHERIT, 0))
                throw new Win32Exception(Marshal.GetLastWin32Error());

            STARTUPINFO startup = new STARTUPINFO();
            startup.cb = Marshal.SizeOf(typeof(STARTUPINFO));
            startup.dwFlags = STARTF_USESTDHANDLES;
            startup.hStdInput = stdinRead;
            startup.hStdOutput = stdoutWrite;
            startup.hStdError = stderrWrite;
            if (!CreateProcessW(executable, null, IntPtr.Zero, IntPtr.Zero, true,
                CREATE_SUSPENDED | CREATE_NO_WINDOW, IntPtr.Zero, workingDirectory,
                ref startup, out processInfo))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            processCreated = true;

            CloseHandle(stdinRead); stdinRead = IntPtr.Zero;
            CloseHandle(stdoutWrite); stdoutWrite = IntPtr.Zero;
            CloseHandle(stderrWrite); stderrWrite = IntPtr.Zero;

            if (!AssignProcessToJobObject(job, processInfo.hProcess))
                throw new Win32Exception(Marshal.GetLastWin32Error());

            LatticeTask038SuspendedProcess launch = new LatticeTask038SuspendedProcess();
            launch.CreatedSuspended = true;
            launch.AssignedBeforeResume = true;
            launch.Process = System.Diagnostics.Process.GetProcessById(processInfo.dwProcessId);
            launch.StandardInput = new StreamWriter(new FileStream(new SafeFileHandle(stdinWrite, true), FileAccess.Write, 4096, false), new UTF8Encoding(false));
            launch.StandardOutput = new StreamReader(new FileStream(new SafeFileHandle(stdoutRead, true), FileAccess.Read, 4096, false), new UTF8Encoding(false), true);
            launch.StandardError = new StreamReader(new FileStream(new SafeFileHandle(stderrRead, true), FileAccess.Read, 4096, false), new UTF8Encoding(false), true);
            parentHandlesTransferred = true;
            stdinWrite = stdoutRead = stderrRead = IntPtr.Zero;

            uint suspendCount = ResumeThread(processInfo.hThread);
            if (suspendCount != 1)
            {
                TerminateProcess(processInfo.hProcess, 1);
                launch.Dispose();
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
            CloseHandle(processInfo.hThread); processInfo.hThread = IntPtr.Zero;
            CloseHandle(processInfo.hProcess); processInfo.hProcess = IntPtr.Zero;
            return launch;
        }
        catch
        {
            if (processCreated && processInfo.hProcess != IntPtr.Zero)
                TerminateProcess(processInfo.hProcess, 1);
            throw;
        }
        finally
        {
            if (processInfo.hThread != IntPtr.Zero) CloseHandle(processInfo.hThread);
            if (processInfo.hProcess != IntPtr.Zero) CloseHandle(processInfo.hProcess);
            if (!parentHandlesTransferred)
            {
                if (stdinWrite != IntPtr.Zero) CloseHandle(stdinWrite);
                if (stdoutRead != IntPtr.Zero) CloseHandle(stdoutRead);
                if (stderrRead != IntPtr.Zero) CloseHandle(stderrRead);
            }
            if (stdinRead != IntPtr.Zero) CloseHandle(stdinRead);
            if (stdoutWrite != IntPtr.Zero) CloseHandle(stdoutWrite);
            if (stderrWrite != IntPtr.Zero) CloseHandle(stderrWrite);
        }
    }

    public void Dispose()
    {
        if (StandardInput != null) StandardInput.Dispose();
        if (StandardOutput != null) StandardOutput.Dispose();
        if (StandardError != null) StandardError.Dispose();
        if (Process != null) Process.Dispose();
    }
}
'@
}

function New-McpInput {
    param([Parameter(Mandatory = $true)][object[]]$Frames)

    return ((@($Frames | ForEach-Object { $_ | ConvertTo-Json -Compress -Depth 16 }) -join "`n") + "`n")
}

function Get-McpResponses {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Output)

    $responses = [Collections.Generic.List[object]]::new()
    foreach ($line in @($Output -split '\r?\n')) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        try {
            $response = $line | ConvertFrom-Json
        }
        catch {
            throw 'TASK038_ACCEPT_MCP_RESPONSE_JSON_REJECTED'
        }
        if ([string]$response.jsonrpc -cne '2.0') {
            throw 'TASK038_ACCEPT_MCP_RESPONSE_ENVELOPE_REJECTED'
        }
        $responses.Add($response)
    }
    return @($responses)
}

function Get-McpResponse {
    param(
        [Parameter(Mandatory = $true)][object[]]$Responses,
        [Parameter(Mandatory = $true)][int]$Id
    )

    $matches = @($Responses | Where-Object { $null -ne $_.PSObject.Properties['id'] -and [int]$_.id -eq $Id })
    if ($matches.Count -ne 1) {
        throw 'TASK038_ACCEPT_MCP_RESPONSE_ID_REJECTED'
    }
    if (
        @($matches[0].PSObject.Properties.Name | Sort-Object) -join ',' -cne 'id,jsonrpc,result' -or
        $null -ne $matches[0].PSObject.Properties['error']
    ) {
        throw 'TASK038_ACCEPT_MCP_PROTOCOL_ERROR'
    }
    return $matches[0]
}

function Get-McpProtocolError {
    param(
        [Parameter(Mandatory = $true)][object[]]$Responses,
        [Parameter(Mandatory = $true)][int]$Id,
        [Parameter(Mandatory = $true)][int]$ExpectedCode,
        [Parameter(Mandatory = $true)][string]$ExpectedMessage
    )

    $matches = @($Responses | Where-Object { $null -ne $_.PSObject.Properties['id'] -and [int]$_.id -eq $Id })
    if (
        $matches.Count -ne 1 -or
        @($matches[0].PSObject.Properties.Name | Sort-Object) -join ',' -cne 'error,id,jsonrpc' -or
        [string]$matches[0].jsonrpc -cne '2.0' -or
        @($matches[0].error.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'code,message' -or
        -not ($matches[0].error.code -is [int]) -or
        [int]$matches[0].error.code -ne $ExpectedCode -or
        -not ($matches[0].error.message -is [string]) -or
        [string]$matches[0].error.message -cne $ExpectedMessage
    ) {
        throw 'TASK038_ACCEPT_NEGATIVE_PROTOCOL_REJECTION_MISMATCH'
    }
    return $matches[0].error
}

function Assert-ExactJsonKeys {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string[]]$Expected,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if ($null -eq $Object -or $Object -isnot [pscustomobject]) { throw $FailureCode }
    $actual = @($Object.PSObject.Properties.Name | Sort-Object -CaseSensitive)
    $wanted = @($Expected | Sort-Object -CaseSensitive)
    if ($actual.Count -ne $wanted.Count) { throw $FailureCode }
    for ($index = 0; $index -lt $wanted.Count; $index++) {
        if ($actual[$index] -cne $wanted[$index]) { throw $FailureCode }
    }
}

function Test-JsonInteger {
    param($Value)
    return $Value -is [int] -or $Value -is [long]
}

function Set-OwnerOnlyAcl {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][bool]$Directory
    )

    try {
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
        if ($Directory) {
            $security = [Security.AccessControl.DirectorySecurity]::new()
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                $sid,
                [Security.AccessControl.FileSystemRights]::FullControl,
                [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
                [Security.AccessControl.PropagationFlags]::None,
                [Security.AccessControl.AccessControlType]::Allow
            )
            $security.SetOwner($sid)
            $security.SetAccessRuleProtection($true, $false)
            [void]$security.AddAccessRule($rule)
            [IO.Directory]::SetAccessControl($Path, $security)
        }
        else {
            $security = [Security.AccessControl.FileSecurity]::new()
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                $sid,
                [Security.AccessControl.FileSystemRights]::FullControl,
                [Security.AccessControl.AccessControlType]::Allow
            )
            $security.SetOwner($sid)
            $security.SetAccessRuleProtection($true, $false)
            [void]$security.AddAccessRule($rule)
            [IO.File]::SetAccessControl($Path, $security)
        }
    }
    catch { throw 'TASK038_ACCEPT_MCP_EVIDENCE_ACL_REJECTED' }
}

function New-McpAcceptanceEvidenceSink {
    param([Parameter(Mandatory = $true)][string]$SessionId)

    $root = Join-Path $script:EvidenceDirectory 'mcp-dispatch'
    if (-not (Test-Path -LiteralPath $root)) {
        [IO.Directory]::CreateDirectory($root) | Out-Null
        Set-OwnerOnlyAcl -Path $root -Directory $true
    }
    $rootItem = Get-Item -LiteralPath $root -Force -ErrorAction SilentlyContinue
    if ($null -eq $rootItem -or -not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'TASK038_ACCEPT_MCP_EVIDENCE_PATH_REJECTED'
    }
    $path = Join-Path $root ($SessionId + '.jsonl')
    try {
        $stream = [IO.File]::Open($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
        $stream.Flush($true)
        $stream.Dispose()
    }
    catch { throw 'TASK038_ACCEPT_MCP_EVIDENCE_NOT_FRESH' }
    Set-OwnerOnlyAcl -Path $path -Directory $false
    try { $rootNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $root -Directory $true }
    catch { throw 'TASK038_ACCEPT_MCP_EVIDENCE_PATH_REJECTED' }
    return [ordered]@{
        path = $path
        native_identity = Get-AuthoritativeNativeFileIdentity -Path $path
        root_path = $root
        root_native_identity = $rootNativeIdentity
    }
}

function Read-McpAcceptanceEvidence {
    param(
        [Parameter(Mandatory = $true)]$Sink,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][string]$SafeConfigSha256,
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][string[]]$ExpectedDispatchTools
    )

    $failureCode = 'TASK038_ACCEPT_MCP_EVIDENCE_REJECTED'
    try {
        $observedSinkIdentity = Get-AuthoritativeNativeFileIdentity -Path ([string]$Sink.path)
        $observedSinkRootIdentity = Get-LatticeWindowsNativePathIdentityToken -Path ([string]$Sink.root_path) -Directory $true
    }
    catch { throw $failureCode }
    if (
        -not (Test-Path -LiteralPath ([string]$Sink.path) -PathType Leaf) -or
        $observedSinkIdentity -cne [string]$Sink.native_identity -or
        $observedSinkRootIdentity -cne [string]$Sink.root_native_identity
    ) { throw $failureCode }
    $bytes = [IO.File]::ReadAllBytes([string]$Sink.path)
    if (
        $bytes.Length -lt 1 -or $bytes.Length -gt 1048576 -or
        ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf)
    ) { throw $failureCode }
    try { $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes) }
    catch { throw $failureCode }
    if (-not $text.EndsWith("`n", [StringComparison]::Ordinal) -or $text.Contains("`r")) { throw $failureCode }
    $lines = @($text.Split([string[]]@("`n"), [StringSplitOptions]::None))
    if ($lines[-1] -cne '') { throw $failureCode }
    $lines = @($lines[0..($lines.Count - 2)])
    if ($lines.Count -ne ($ExpectedDispatchTools.Count + 2)) { throw $failureCode }

    $previousEventSha256 = '0' * 64
    $dispatchCount = 0
    $previousObservedNanos = [System.Numerics.BigInteger]::Zero
    for ($index = 0; $index -lt $lines.Count; $index++) {
        try { $record = $lines[$index] | ConvertFrom-Json -ErrorAction Stop }
        catch { throw $failureCode }
        Assert-ExactJsonKeys -Object $record -Expected @(
            'schema', 'record_type', 'session_id', 'safe_config_sha256', 'process_id',
            'ordinal', 'tool_name', 'request_id_sha256', 'dispatch_accepted_count',
            'observed_at_unix_nanos', 'previous_event_sha256', 'event_sha256'
        ) -FailureCode $failureCode
        $expectedType = if ($index -eq 0) { 'SESSION_OPEN' } elseif ($index -eq $lines.Count - 1) { 'SESSION_CLOSED' } else { 'DISPATCH_ACCEPTED' }
        if ($expectedType -ceq 'DISPATCH_ACCEPTED') { $dispatchCount++ }
        $observedNanos = [System.Numerics.BigInteger]::Zero
        if (
            $record.schema -isnot [string] -or [string]$record.schema -cne 'lattice.mcp.acceptance-dispatch.v1' -or
            $record.record_type -isnot [string] -or [string]$record.record_type -cne $expectedType -or
            $record.session_id -isnot [string] -or [string]$record.session_id -cne $SessionId -or
            $record.safe_config_sha256 -isnot [string] -or [string]$record.safe_config_sha256 -cne $SafeConfigSha256 -or
            -not (Test-JsonInteger -Value $record.process_id) -or [long]$record.process_id -ne $ProcessId -or
            -not (Test-JsonInteger -Value $record.ordinal) -or [long]$record.ordinal -ne ($index + 1) -or
            -not (Test-JsonInteger -Value $record.dispatch_accepted_count) -or [long]$record.dispatch_accepted_count -ne $dispatchCount -or
            $record.observed_at_unix_nanos -isnot [string] -or [string]$record.observed_at_unix_nanos -cnotmatch '\A[1-9][0-9]*\z' -or
            -not [System.Numerics.BigInteger]::TryParse([string]$record.observed_at_unix_nanos, [ref]$observedNanos) -or
            $observedNanos -lt $previousObservedNanos -or
            $record.previous_event_sha256 -isnot [string] -or [string]$record.previous_event_sha256 -cne $previousEventSha256 -or
            $record.event_sha256 -isnot [string] -or [string]$record.event_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
        ) { throw $failureCode }
        if ($expectedType -ceq 'DISPATCH_ACCEPTED') {
            $expectedTool = [string]$ExpectedDispatchTools[$dispatchCount - 1]
            if (
                $record.tool_name -isnot [string] -or [string]$record.tool_name -cne $expectedTool -or
                -not ($script:ExpectedTools -ccontains [string]$record.tool_name) -or
                $record.request_id_sha256 -isnot [string] -or [string]$record.request_id_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
            ) { throw $failureCode }
            $toolName = [string]$record.tool_name
            $requestIdSha256 = [string]$record.request_id_sha256
        }
        else {
            if ($null -ne $record.tool_name -or $null -ne $record.request_id_sha256) { throw $failureCode }
            $toolName = 'null'
            $requestIdSha256 = 'null'
        }
        $hashInput = @(
            'lattice.mcp.acceptance-dispatch-hash.v1', $previousEventSha256, $SessionId,
            $SafeConfigSha256, $expectedType, [string]($index + 1), [string]$ProcessId,
            $toolName, $requestIdSha256, [string]$dispatchCount,
            [string]$record.observed_at_unix_nanos
        ) -join "`n"
        $eventSha256 = Get-StringSha256 -Value $hashInput
        if ([string]$record.event_sha256 -cne $eventSha256) { throw $failureCode }
        $previousEventSha256 = $eventSha256
        $previousObservedNanos = $observedNanos
    }
    return [ordered]@{
        schema = 'lattice.task038.mcp-acceptance-dispatch-evidence.v1'
        raw_sha256 = Get-FileSha256 -Path ([string]$Sink.path)
        byte_count = [long]$bytes.Length
        strict_utf8 = $true
        session_id = $SessionId
        safe_config_sha256 = $SafeConfigSha256
        process_id = $ProcessId
        record_count = [int]$lines.Count
        dispatch_accepted_count = $dispatchCount
        accepted_tools = @($ExpectedDispatchTools)
        final_event_sha256 = $previousEventSha256
        normal_close_complete = $true
        native_identity = [string]$Sink.native_identity
    }
}

function Invoke-McpSession {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][object[]]$Frames,
        [string[]]$ExpectedDispatchTools = @()
    )

    Assert-CandidateBinaryUnchanged
    $inputText = New-McpInput -Frames $Frames
    $job = [IntPtr]::Zero
    $process = $null
    $launch = $null
    $startedAt = [DateTime]::UtcNow
    $stdout = [string]::Empty
    $stderr = [string]::Empty
    $timedOut = $false
    $exitCode = $null
    $processId = $null
    $activeAfter = $null
    $processStarted = $false
    $assignedByNativeHandle = $false
    $createdSuspended = $false
    $assignedBeforeResume = $false
    $containedDescendantsTerminated = 0
    $processPresentAfterCleanup = $null
    $tcpOwnersAfterCleanup = $null
    $udpOwnersAfterCleanup = $null
    $summaryWritten = $false
    $acceptanceSink = $null
    $acceptanceSessionId = $null
    $acceptanceSafeConfigSha256 = $null
    $acceptanceEvidence = $null
    $acceptanceEnvironment = @(
        'LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH',
        'LATTICE_MCP_ACCEPTANCE_SESSION_ID',
        'LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256'
    )
    $previousAcceptanceEnvironment = [ordered]@{}
    try {
        if ($Mode -eq 'FULL') {
            $acceptanceSessionId = [Guid]::NewGuid().ToString('N')
            $acceptanceSink = New-McpAcceptanceEvidenceSink -SessionId $acceptanceSessionId
            $acceptanceSafeConfigSha256 = Get-DomainSeparatedCommitment `
                -Domain 'LATTICE_TASK038_MCP_ACCEPTANCE_SAFE_CONFIG_V1' `
                -Value ([ordered]@{
                    candidate_commit = [string]$script:CandidateLinkage.candidate_commit
                    candidate_tree = [string]$script:CandidateLinkage.candidate_tree
                    candidate_binary_sha256 = [string]$script:CandidateBuildEvidence.binary_sha256
                    candidate_binary_native_identity = [string]$script:LatticedNativeIdentity
                    phase = $Name
                    input_sha256 = Get-StringSha256 -Value $inputText
                    expected_dispatch_tools = @($ExpectedDispatchTools)
                    review_commitment = [string]$script:CandidateReviewCommitment
                })
            foreach ($environmentName in $acceptanceEnvironment) {
                $previousAcceptanceEnvironment[$environmentName] = [Environment]::GetEnvironmentVariable($environmentName, 'Process')
            }
            [Environment]::SetEnvironmentVariable($acceptanceEnvironment[0], [string]$acceptanceSink.path, 'Process')
            [Environment]::SetEnvironmentVariable($acceptanceEnvironment[1], $acceptanceSessionId, 'Process')
            [Environment]::SetEnvironmentVariable($acceptanceEnvironment[2], $acceptanceSafeConfigSha256, 'Process')
        }
        $job = [LatticeTask038AcceptanceJob]::Create()
        try {
            $launch = [LatticeTask038SuspendedProcess]::Start($script:Latticed, $script:BinaryDirectory, $job)
        }
        catch {
            throw 'TASK038_ACCEPT_PROCESS_SUSPENDED_LAUNCH_REJECTED'
        }
        finally {
            if ($Mode -eq 'FULL') {
                foreach ($environmentName in $acceptanceEnvironment) {
                    [Environment]::SetEnvironmentVariable($environmentName, $previousAcceptanceEnvironment[$environmentName], 'Process')
                }
            }
        }
        $process = $launch.Process
        $processStarted = $true
        $processId = [int]$process.Id
        $createdSuspended = [bool]$launch.CreatedSuspended
        $assignedBeforeResume = [bool]$launch.AssignedBeforeResume
        $assignedByNativeHandle = $createdSuspended -and $assignedBeforeResume
        if (-not $assignedByNativeHandle) {
            throw 'TASK038_ACCEPT_PROCESS_ASSIGNMENT_ORDER_REJECTED'
        }

        $stdoutTask = $launch.StandardOutput.ReadToEndAsync()
        $stderrTask = $launch.StandardError.ReadToEndAsync()
        $launch.StandardInput.Write($inputText)
        $launch.StandardInput.Close()

        if (-not $process.WaitForExit($SessionTimeoutSeconds * 1000)) {
            $timedOut = $true
            [LatticeTask038AcceptanceJob]::Terminate($job)
            $null = $process.WaitForExit(5000)
        }

        $cleanupWatch = [Diagnostics.Stopwatch]::StartNew()
        do {
            $activeAfter = [uint32][LatticeTask038AcceptanceJob]::Active($job)
            if ($activeAfter -eq 0) { break }
            Start-Sleep -Milliseconds 25
        } while ($cleanupWatch.ElapsedMilliseconds -lt 5000)
        if ($activeAfter -ne 0) {
            $containedDescendantsTerminated = [int]$activeAfter
            [LatticeTask038AcceptanceJob]::Terminate($job)
            $cleanupWatch.Restart()
            do {
                $activeAfter = [uint32][LatticeTask038AcceptanceJob]::Active($job)
                if ($activeAfter -eq 0) { break }
                Start-Sleep -Milliseconds 25
            } while ($cleanupWatch.ElapsedMilliseconds -lt 5000)
            if ($activeAfter -ne 0) {
                throw 'TASK038_ACCEPT_PROCESS_CLEANUP_REJECTED'
            }
        }

        if (-not $stdoutTask.Wait(5000) -or -not $stderrTask.Wait(5000)) {
            throw 'TASK038_ACCEPT_PROCESS_CAPTURE_REJECTED'
        }
        $stdout = [string]$stdoutTask.Result
        $stderr = [string]$stderrTask.Result
        if ($stdout.Length -gt 1048576 -or $stderr.Length -gt 1048576) {
            throw 'TASK038_ACCEPT_PROCESS_OUTPUT_LIMIT_REJECTED'
        }
        if ($timedOut) {
            throw 'TASK038_ACCEPT_PROCESS_TIMEOUT'
        }
        $exitCode = [int]$process.ExitCode
        if ($exitCode -ne 0) {
            throw 'TASK038_ACCEPT_PROCESS_EXIT_REJECTED'
        }
        Assert-CandidateBinaryUnchanged

        if ($Mode -eq 'FULL') {
            $acceptanceEvidence = Read-McpAcceptanceEvidence `
                -Sink $acceptanceSink `
                -SessionId $acceptanceSessionId `
                -SafeConfigSha256 $acceptanceSafeConfigSha256 `
                -ProcessId $processId `
                -ExpectedDispatchTools $ExpectedDispatchTools
            $processPresentAfterCleanup = @(
                Get-Process -Id $processId -ErrorAction SilentlyContinue
            ).Count
            try {
                $tcpOwnersAfterCleanup = @(Get-NetTCPConnection -ErrorAction Stop | Where-Object { [int]$_.OwningProcess -eq $processId }).Count
                $udpOwnersAfterCleanup = @(Get-NetUDPEndpoint -ErrorAction Stop | Where-Object { [int]$_.OwningProcess -eq $processId }).Count
            }
            catch { throw 'TASK038_ACCEPT_EFFECT_OBSERVATION_UNKNOWN' }
        }

        $responses = @(Get-McpResponses -Output $stdout)
        $summary = [ordered]@{
            schema_version = 'lattice.task038.mcp-session.v1'
            session_status = 'PASS'
            phase = $Name
            process_id = $processId
            started_at_utc = $startedAt.ToString('o')
            finished_at_utc = [DateTime]::UtcNow.ToString('o')
            exit_code = $exitCode
            timed_out = $false
            response_count = $responses.Count
            stdout_sha256 = Get-StringSha256 -Value $stdout
            stderr_sha256 = Get-StringSha256 -Value $stderr
            stdout_bytes = $script:Utf8.GetByteCount($stdout)
            stderr_bytes = $script:Utf8.GetByteCount($stderr)
            job_active_processes_after_exit = [int]$activeAfter
            job_object_native_handle_assigned = $assignedByNativeHandle
            process_created_suspended = $createdSuspended
            job_assigned_before_resume = $assignedBeforeResume
            contained_descendants_terminated = $containedDescendantsTerminated
            process_session_pid_present_after_cleanup = $(if ($Mode -eq 'FULL') { [int]$processPresentAfterCleanup } else { 0 })
            network_tcp_owner_rows_after_cleanup = $(if ($Mode -eq 'FULL') { [int]$tcpOwnersAfterCleanup } else { 0 })
            network_udp_owner_rows_after_cleanup = $(if ($Mode -eq 'FULL') { [int]$udpOwnersAfterCleanup } else { 0 })
            cleanup_identity = 'WINDOWS_JOB_OBJECT_AND_PROCESS_HANDLE'
            acceptance_dispatch_evidence = $acceptanceEvidence
            raw_output_retained = $false
        }
        $script:SessionEvidence.Add($summary)
        $summaryWritten = $true
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory ($Name + '.process.json')) -Value $summary
        return [pscustomobject]@{ Responses = $responses; Summary = $summary; AcceptanceEvidence = $acceptanceEvidence }
    }
    finally {
        if ($Mode -eq 'FULL') {
            foreach ($environmentName in $acceptanceEnvironment) {
                [Environment]::SetEnvironmentVariable($environmentName, $previousAcceptanceEnvironment[$environmentName], 'Process')
            }
        }
        if ($null -ne $process) {
            if ($processStarted -and -not $process.HasExited) {
                if ($assignedByNativeHandle) {
                    [LatticeTask038AcceptanceJob]::Terminate($job)
                }
                else {
                    $process.Kill()
                }
                if (-not $process.WaitForExit(5000)) {
                    throw 'TASK038_ACCEPT_PROCESS_CLEANUP_REJECTED'
                }
            }
            if ($processStarted -and $job -ne [IntPtr]::Zero) {
                $activeAfter = [uint32][LatticeTask038AcceptanceJob]::Active($job)
            }
            if ($processStarted -and -not $summaryWritten) {
                $failureSummary = [ordered]@{
                    schema_version = 'lattice.task038.mcp-session.v1'
                    session_status = 'FAIL'
                    phase = $Name
                    process_id = $processId
                    started_at_utc = $startedAt.ToString('o')
                    finished_at_utc = [DateTime]::UtcNow.ToString('o')
                    exit_code = $(if ($process.HasExited) { [int]$process.ExitCode } else { $null })
                    timed_out = $timedOut
                    response_count = $null
                    stdout_sha256 = Get-StringSha256 -Value $stdout
                    stderr_sha256 = Get-StringSha256 -Value $stderr
                    stdout_bytes = $script:Utf8.GetByteCount($stdout)
                    stderr_bytes = $script:Utf8.GetByteCount($stderr)
                    job_active_processes_after_exit = $(if ($null -eq $activeAfter) { $null } else { [int]$activeAfter })
                    job_object_native_handle_assigned = $assignedByNativeHandle
                    process_created_suspended = $createdSuspended
                    job_assigned_before_resume = $assignedBeforeResume
                    contained_descendants_terminated = $containedDescendantsTerminated
                    process_session_pid_present_after_cleanup = $processPresentAfterCleanup
                    network_tcp_owner_rows_after_cleanup = $tcpOwnersAfterCleanup
                    network_udp_owner_rows_after_cleanup = $udpOwnersAfterCleanup
                    cleanup_identity = 'WINDOWS_JOB_OBJECT_AND_PROCESS_HANDLE'
                    raw_output_retained = $false
                }
                $script:SessionEvidence.Add($failureSummary)
                Write-SafeJson -Path (Join-Path $script:EvidenceDirectory ($Name + '.process.json')) -Value $failureSummary
            }
            if ($null -ne $launch) {
                $launch.Dispose()
            }
            else {
                $process.Dispose()
            }
        }
        if ($job -ne [IntPtr]::Zero) {
            try {
                [LatticeTask038AcceptanceJob]::Close($job)
            }
            catch {
                throw 'TASK038_ACCEPT_PROCESS_HANDLE_CLOSE_REJECTED'
            }
        }
    }
}

function ConvertTo-CanonicalJson {
    param([AllowNull()]$Value)

    if ($null -eq $Value) { return 'null' }
    if ($Value -is [string]) { return ($Value | ConvertTo-Json -Compress) }
    if ($Value -is [bool]) { return $(if ($Value) { 'true' } else { 'false' }) }
    if ($Value -is [byte] -or $Value -is [sbyte] -or $Value -is [int16] -or $Value -is [uint16] -or
        $Value -is [int32] -or $Value -is [uint32] -or $Value -is [int64] -or $Value -is [uint64] -or
        $Value -is [single] -or $Value -is [double] -or $Value -is [decimal]) {
        return [Convert]::ToString($Value, [Globalization.CultureInfo]::InvariantCulture)
    }
    if ($Value -is [Collections.IDictionary]) {
        $parts = @($Value.Keys | ForEach-Object { [string]$_ } | Sort-Object | ForEach-Object {
            $key = [string]$_
            ($key | ConvertTo-Json -Compress) + ':' + (ConvertTo-CanonicalJson -Value $Value[$key])
        })
        return '{' + [string]::Join(',', $parts) + '}'
    }
    if ($Value -is [Management.Automation.PSCustomObject]) {
        $propertyNames = @($Value.PSObject.Properties | ForEach-Object { [string]$_.Name } | Sort-Object)
        $parts = @($propertyNames | ForEach-Object {
            $propertyName = [string]$_
            ($propertyName | ConvertTo-Json -Compress) + ':' +
                (ConvertTo-CanonicalJson -Value $Value.PSObject.Properties[$propertyName].Value)
        })
        return '{' + [string]::Join(',', $parts) + '}'
    }
    if ($Value -is [Collections.IEnumerable]) {
        $parts = @($Value | ForEach-Object { ConvertTo-CanonicalJson -Value $_ })
        return '[' + [string]::Join(',', $parts) + ']'
    }
    throw 'TASK038_ACCEPT_SCHEMA_CANONICALIZATION_REJECTED'
}

function Assert-CanonicalSchema {
    param(
        [Parameter(Mandatory = $true)]$Actual,
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if ((ConvertTo-CanonicalJson -Value $Actual) -cne (ConvertTo-CanonicalJson -Value $Expected)) {
        throw $FailureCode
    }
}

function Get-ExpectedTaskSubmitSchema {
    return [ordered]@{
        type = 'object'
        properties = [ordered]@{
            client_request_id = [ordered]@{
                type = 'string'; minLength = 1; maxLength = 64; pattern = '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'
            }
            intent = [ordered]@{ type = 'string'; enum = @('CONTROLLED_CODEX_CANARY') }
        }
        required = @('client_request_id', 'intent')
        additionalProperties = $false
    }
}

function Get-ExpectedTaskStatusSchema {
    return [ordered]@{
        type = 'object'
        properties = [ordered]@{
            task_ref = [ordered]@{ type = 'string'; minLength = 64; maxLength = 64; pattern = '^[0-9a-f]{64}$' }
        }
        required = @('task_ref')
        additionalProperties = $false
    }
}

function Get-ExpectedTaskPublicStatusSchema {
    return [ordered]@{
        type = 'object'
        properties = [ordered]@{
            schema_version = [ordered]@{ type = 'string'; enum = @('lattice.task.status.v1') }
            status = [ordered]@{ type = 'string'; enum = @('NOT_SUBMITTED', 'RECONCILIATION_REQUIRED', 'FAILED', 'COMPLETED') }
            task_state = [ordered]@{ type = 'string'; enum = @(
                'NOT_SUBMITTED', 'DRAFT', 'AWAITING_EXECUTION_APPROVAL', 'PREPARING', 'EXECUTING',
                'VERIFYING', 'REVIEWING', 'AWAITING_MERGE_APPROVAL', 'MERGING', 'COMPLETED',
                'REJECTED', 'BLOCKED', 'FAILED', 'STOPPING', 'CANCELLED'
            ) }
            task_ref = [ordered]@{ type = 'string'; minLength = 64; maxLength = 64; pattern = '^[0-9a-f]{64}$' }
            ledger_head_digest = [ordered]@{ type = 'string'; minLength = 64; maxLength = 64; pattern = '^[0-9a-f]{64}$' }
            result_digest = [ordered]@{ anyOf = @(
                [ordered]@{ type = 'string'; minLength = 64; maxLength = 64; pattern = '^[0-9a-f]{64}$' },
                [ordered]@{ type = 'null' }
            ) }
        }
        required = @('schema_version', 'status', 'task_state', 'task_ref', 'ledger_head_digest', 'result_digest')
        additionalProperties = $false
    }
}

function Get-ExpectedToolSchemaContract {
    return [ordered]@{
        delivery_input = [ordered]@{ type = 'object'; additionalProperties = $false }
        submit_input = (Get-ExpectedTaskSubmitSchema)
        status_input = (Get-ExpectedTaskStatusSchema)
        task_output = (Get-ExpectedTaskPublicStatusSchema)
    }
}

function Assert-ToolDiscovery {
    param([Parameter(Mandatory = $true)]$Response)

    $resultNames = @($Response.result.PSObject.Properties.Name | Sort-Object) -join ','
    $stateless = $null -ne $Response.result.PSObject.Properties['resultType']
    if (
        (-not $stateless -and $resultNames -cne 'tools') -or
        ($stateless -and $resultNames -cne '_meta,cacheScope,resultType,tools,ttlMs') -or
        ($stateless -and (
            [string]$Response.result.resultType -cne 'complete' -or
            [string]$Response.result.cacheScope -cne 'private' -or
            [int]$Response.result.ttlMs -ne 0
        ))
    ) { throw 'TASK038_ACCEPT_TOOL_DISCOVERY_ENVELOPE_REJECTED' }

    $tools = @($Response.result.tools)
    $names = @($tools | ForEach-Object { [string]$_.name } | Sort-Object)
    $script:ObservedTools = $names
    if ($tools.Count -ne 4 -or @(Compare-Object -ReferenceObject $script:ExpectedTools -DifferenceObject $names).Count -ne 0) {
        throw 'TASK038_ACCEPT_TOOL_DISCOVERY_MISMATCH'
    }

    $deliverySchema = [ordered]@{ type = 'object'; additionalProperties = $false }
    foreach ($name in @('lattice_delivery_run', 'lattice_delivery_status')) {
        $tool = @($tools | Where-Object { [string]$_.name -ceq $name })
        if ($tool.Count -ne 1 -or $null -ne $tool[0].PSObject.Properties['outputSchema']) {
            throw 'TASK038_ACCEPT_DELIVERY_SCHEMA_REJECTED'
        }
        Assert-CanonicalSchema -Actual $tool[0].inputSchema -Expected $deliverySchema -FailureCode 'TASK038_ACCEPT_DELIVERY_SCHEMA_REJECTED'
    }

    $submit = @($tools | Where-Object { [string]$_.name -ceq 'lattice_task_submit' })
    $status = @($tools | Where-Object { [string]$_.name -ceq 'lattice_task_status' })
    if ($submit.Count -ne 1 -or $status.Count -ne 1) { throw 'TASK038_ACCEPT_TASK_SCHEMA_REJECTED' }

    $submitSchema = Get-ExpectedTaskSubmitSchema
    $statusSchema = Get-ExpectedTaskStatusSchema
    Assert-CanonicalSchema -Actual $submit[0].inputSchema -Expected $submitSchema -FailureCode 'TASK038_ACCEPT_TASK_SCHEMA_REJECTED'
    Assert-CanonicalSchema -Actual $status[0].inputSchema -Expected $statusSchema -FailureCode 'TASK038_ACCEPT_TASK_SCHEMA_REJECTED'

    $outputSchema = Get-ExpectedTaskPublicStatusSchema
    Assert-CanonicalSchema -Actual $submit[0].outputSchema -Expected $outputSchema -FailureCode 'TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'
    Assert-CanonicalSchema -Actual $status[0].outputSchema -Expected $outputSchema -FailureCode 'TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'
}

function Get-ToolResult {
    param(
        [Parameter(Mandatory = $true)]$Response,
        [ValidateSet('LEGACY', 'STATELESS')][string]$Protocol = 'LEGACY'
    )

    $expectedResultNames = if ($Protocol -eq 'STATELESS') {
        '_meta,content,isError,resultType,structuredContent'
    }
    else {
        'content,isError,structuredContent'
    }
    if (
        $null -eq $Response.result -or
        @($Response.result.PSObject.Properties.Name | Sort-Object) -join ',' -cne $expectedResultNames -or
        -not ($Response.result.isError -is [bool]) -or
        $null -eq $Response.result.PSObject.Properties['structuredContent']
    ) {
        throw 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    if ($Protocol -eq 'STATELESS' -and (
        [string]$Response.result.resultType -cne 'complete' -or
        $null -eq $Response.result._meta -or
        @($Response.result._meta.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'io.modelcontextprotocol/serverInfo'
    )) {
        throw 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    $content = @($Response.result.content)
    if (
        $content.Count -ne 1 -or
        @($content[0].PSObject.Properties.Name | Sort-Object) -join ',' -cne 'text,type' -or
        [string]$content[0].type -cne 'text' -or
        -not ($content[0].text -is [string])
    ) {
        throw 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    try {
        $textContent = [string]$content[0].text | ConvertFrom-Json
    }
    catch {
        throw 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    if ((ConvertTo-CanonicalJson -Value $textContent) -cne (ConvertTo-CanonicalJson -Value $Response.result.structuredContent)) {
        throw 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    $isError = [bool]$Response.result.isError
    $structured = $Response.result.structuredContent
    $safeCode = $null
    if ($isError) {
        if (
            @($structured.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'code,status' -or
            -not ($structured.status -is [string]) -or
            [string]$structured.status -cne 'ERROR'
        ) {
            throw 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
        }
        $safeCode = Get-SafeToolCode -StructuredContent $structured
    }
    return [pscustomobject]@{ IsError = $isError; Structured = $structured; SafeCode = $safeCode }
}

$script:ObservedCounterNames = @(
    'dispatch',
    'effect',
    'delivery_run',
    'delivery_status',
    'task_submit',
    'task_status'
)

function Get-HarnessObservedCounters {
    if ([string]::IsNullOrWhiteSpace($script:HarnessCounterPath)) {
        throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REQUIRED'
    }
    $values = [ordered]@{}
    foreach ($name in $script:ObservedCounterNames) { $values[$name] = [long]0 }
    if (-not (Test-Path -LiteralPath $script:HarnessCounterPath -PathType Leaf)) {
        throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REQUIRED'
    }
    $item = Get-Item -LiteralPath $script:HarnessCounterPath -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $item -or
        $item.PSIsContainer -or
        -not ($item -is [IO.FileInfo]) -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED'
    }
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($line in @(Get-Content -LiteralPath $script:HarnessCounterPath -Encoding UTF8)) {
        $parts = [string]$line -split '=', 2
        $parsed = [long]0
        if (
            $parts.Count -ne 2 -or
            -not ($script:ObservedCounterNames -ccontains $parts[0]) -or
            -not $seen.Add([string]$parts[0]) -or
            -not [long]::TryParse([string]$parts[1], [ref]$parsed) -or
            $parsed -lt 0
        ) {
            throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED'
        }
        $values[$parts[0]] = $parsed
    }
    if ($seen.Count -ne $script:ObservedCounterNames.Count) {
        throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED'
    }
    return $values
}

function Get-DirectoryFootprint {
    param([Parameter(Mandatory = $true)][string]$Root)

    $canonicalRoot = Get-CanonicalPath -Path $Root
    $rootItem = Get-Item -LiteralPath $canonicalRoot -Force -ErrorAction SilentlyContinue
    if ($null -eq $rootItem -or -not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'TASK038_ACCEPT_EFFECT_OBSERVATION_ROOT_REJECTED'
    }
    $items = @(Get-ChildItem -LiteralPath $canonicalRoot -Recurse -Force -ErrorAction Stop)
    if ($items.Count -gt 4096) { throw 'TASK038_ACCEPT_EFFECT_OBSERVATION_UNKNOWN' }
    [long]$totalBytes = 0
    $records = [Collections.Generic.List[string]]::new()
    foreach ($item in @($items | Sort-Object FullName)) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'TASK038_ACCEPT_EFFECT_OBSERVATION_UNKNOWN'
        }
        $relative = $item.FullName.Substring($canonicalRoot.Length).TrimStart([char[]]@('\', '/')).Replace('\', '/')
        if ($item.PSIsContainer) {
            $records.Add(('D|' + $relative + '|' + [int64]$item.Attributes + '|' + $item.CreationTimeUtc.Ticks + '|' + $item.LastWriteTimeUtc.Ticks))
        }
        else {
            if ($item.Length -gt 67108864 -or $totalBytes -gt (134217728 - $item.Length)) {
                throw 'TASK038_ACCEPT_EFFECT_OBSERVATION_UNKNOWN'
            }
            $totalBytes += $item.Length
            $records.Add(('F|' + $relative + '|' + [int64]$item.Attributes + '|' + $item.Length + '|' + $item.CreationTimeUtc.Ticks + '|' + $item.LastWriteTimeUtc.Ticks + '|' + (Get-FileSha256 -Path $item.FullName)))
        }
    }
    try {
        $rootNativeIdentity = $(
            if ($null -ne (Get-Command Get-LatticeWindowsNativePathIdentityToken -CommandType Function -ErrorAction SilentlyContinue)) {
                Get-LatticeWindowsNativePathIdentityToken -Path $canonicalRoot -Directory $true
            }
            else { Get-NativeDirectoryIdentity -Path $canonicalRoot }
        )
    }
    catch { throw 'TASK038_ACCEPT_EFFECT_OBSERVATION_UNKNOWN' }
    return [ordered]@{
        root_native_identity = $rootNativeIdentity
        entry_count = [int]$items.Count
        total_bytes = $totalBytes
        digest = Get-StringSha256 -Value ([string]::Join("`n", $records))
    }
}

function Get-SourceGitFootprint {
    $head = Invoke-GitText -Arguments @('rev-parse', 'HEAD')
    $tree = Invoke-GitText -Arguments @('rev-parse', 'HEAD^{tree}')
    $status = Invoke-GitText -Arguments @('status', '--porcelain=v1', '--untracked-files=all')
    return [ordered]@{
        head = $head
        tree = $tree
        status_sha256 = Get-StringSha256 -Value $status
        status_empty = [string]::IsNullOrEmpty($status)
    }
}

function Get-ProductionEffectSnapshot {
    if (
        [string]::IsNullOrWhiteSpace([string]$script:ProductionDatabasePassword) -or
        [string]::IsNullOrWhiteSpace([string]$script:ProductionCodexHome) -or
        [string]::IsNullOrWhiteSpace([string]$script:ProductionDeliveryRoot)
    ) { throw 'TASK038_ACCEPT_EFFECT_OBSERVATION_UNKNOWN' }
    return [ordered]@{
        database = Get-DatabaseFootprint -Password $script:ProductionDatabasePassword -TaskRef ''
        source_git = Get-SourceGitFootprint
        codex_home = Get-DirectoryFootprint -Root $script:ProductionCodexHome
        delivery_root = Get-DirectoryFootprint -Root $script:ProductionDeliveryRoot
    }
}

function New-ZeroProductionSessionEffectReceipt {
    param(
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After,
        [Parameter(Mandatory = $true)]$Session
    )

    $beforeCommitment = Get-DomainSeparatedCommitment -Domain 'LATTICE_TASK038_SESSION_EFFECT_SNAPSHOT_V1' -Value $Before
    $afterCommitment = Get-DomainSeparatedCommitment -Domain 'LATTICE_TASK038_SESSION_EFFECT_SNAPSHOT_V1' -Value $After
    if (
        $beforeCommitment -cne $afterCommitment -or
        $null -eq $Session.AcceptanceEvidence -or
        -not [bool]$Session.AcceptanceEvidence.normal_close_complete -or
        [int]$Session.Summary.job_active_processes_after_exit -ne 0 -or
        [int]$Session.Summary.process_session_pid_present_after_cleanup -ne 0 -or
        [int]$Session.Summary.network_tcp_owner_rows_after_cleanup -ne 0 -or
        [int]$Session.Summary.network_udp_owner_rows_after_cleanup -ne 0
    ) { throw 'TASK038_ACCEPT_PRODUCTION_SESSION_EFFECT_REJECTED' }
    return [ordered]@{
        schema_version = 'lattice.task038.production-session-effect-observation.v1'
        phase = $Phase
        session_id = [string]$Session.AcceptanceEvidence.session_id
        dispatch_evidence_raw_sha256 = [string]$Session.AcceptanceEvidence.raw_sha256
        dispatch_final_event_sha256 = [string]$Session.AcceptanceEvidence.final_event_sha256
        dispatch_accepted_count = [int]$Session.AcceptanceEvidence.dispatch_accepted_count
        before_commitment = $beforeCommitment
        after_commitment = $afterCommitment
        observed_effect_delta = [ordered]@{
            database = 0L; filesystem = 0L; process = 0L; network = 0L; codex = 0L; related = 0L
        }
        exact_zero_effect_observed = $true
    }
}

function Invoke-RepositoryGitText {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& git -C $Repository @Arguments 2>&1)
        $exitCode = [int]$LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previous }
    if ($exitCode -ne 0) { throw 'TASK038_ACCEPT_DELIVERY_GIT_EFFECT_REJECTED' }
    return ([string]::Join("`n", @($output | ForEach-Object { [string]$_ }))).Trim()
}

function Get-DeliveryGitEffectReceipt {
    $repository = Get-CanonicalPath -Path (Join-Path $script:ProductionDeliveryRoot 'repo')
    $head = Invoke-RepositoryGitText -Repository $repository -Arguments @('rev-parse', '--verify', 'HEAD')
    $parent = Invoke-RepositoryGitText -Repository $repository -Arguments @('rev-parse', '--verify', 'HEAD^')
    $tree = Invoke-RepositoryGitText -Repository $repository -Arguments @('rev-parse', 'HEAD^{tree}')
    $count = Invoke-RepositoryGitText -Repository $repository -Arguments @('rev-list', '--count', 'HEAD')
    $status = Invoke-RepositoryGitText -Repository $repository -Arguments @('status', '--porcelain=v1', '--untracked-files=all')
    $changed = Invoke-RepositoryGitText -Repository $repository -Arguments @('diff-tree', '--no-commit-id', '--name-only', '-r', '--no-renames', 'HEAD')
    $answerPath = Join-Path $repository 'answer.txt'
    if (-not (Test-Path -LiteralPath $answerPath -PathType Leaf)) { throw 'TASK038_ACCEPT_DELIVERY_GIT_EFFECT_REJECTED' }
    $answer = [IO.File]::ReadAllBytes($answerPath)
    $expected = [Text.Encoding]::ASCII.GetBytes("LATTICE_DELIVERY_OK`n")
    if (
        $head -cnotmatch '\A[0-9a-f]{40}\z' -or $parent -cnotmatch '\A[0-9a-f]{40}\z' -or
        $tree -cnotmatch '\A[0-9a-f]{40}\z' -or [int]$count -ne 2 -or
        -not [string]::IsNullOrEmpty($status) -or $changed -cne 'answer.txt' -or
        [Convert]::ToBase64String($answer) -cne [Convert]::ToBase64String($expected)
    ) { throw 'TASK038_ACCEPT_DELIVERY_GIT_EFFECT_REJECTED' }
    return [ordered]@{
        head = $head; parent = $parent; tree = $tree; commit_count = 2
        clean = $true; changed_path = 'answer.txt'; answer_sha256 = Get-FileSha256 -Path $answerPath
    }
}

function New-SubmitProductionSessionEffectReceipt {
    param(
        [Parameter(Mandatory = $true)]$Before,
        [Parameter(Mandatory = $true)]$After,
        [Parameter(Mandatory = $true)]$Session
    )

    if (
        (ConvertTo-CanonicalJson -Value $Before.source_git) -cne (ConvertTo-CanonicalJson -Value $After.source_git) -or
        [string]$Before.codex_home.root_native_identity -cne [string]$After.codex_home.root_native_identity -or
        [string]$Before.delivery_root.root_native_identity -cne [string]$After.delivery_root.root_native_identity -or
        [string]$Before.delivery_root.digest -ceq [string]$After.delivery_root.digest -or
        [long]$Before.database.codex_intents -ne 0 -or [long]$After.database.codex_intents -ne 1 -or
        $null -eq $Session.AcceptanceEvidence -or [int]$Session.AcceptanceEvidence.dispatch_accepted_count -ne 3 -or
        -not [bool]$Session.AcceptanceEvidence.normal_close_complete -or
        [int]$Session.Summary.job_active_processes_after_exit -ne 0 -or
        [int]$Session.Summary.process_session_pid_present_after_cleanup -ne 0 -or
        [int]$Session.Summary.network_tcp_owner_rows_after_cleanup -ne 0 -or
        [int]$Session.Summary.network_udp_owner_rows_after_cleanup -ne 0
    ) { throw 'TASK038_ACCEPT_PRODUCTION_SESSION_EFFECT_REJECTED' }
    return [ordered]@{
        schema_version = 'lattice.task038.production-session-effect-observation.v1'
        phase = '03-bounded-submit'
        session_id = [string]$Session.AcceptanceEvidence.session_id
        dispatch_evidence_raw_sha256 = [string]$Session.AcceptanceEvidence.raw_sha256
        dispatch_final_event_sha256 = [string]$Session.AcceptanceEvidence.final_event_sha256
        dispatch_accepted_count = 3
        before_commitment = Get-DomainSeparatedCommitment -Domain 'LATTICE_TASK038_SESSION_EFFECT_SNAPSHOT_V1' -Value $Before
        after_commitment = Get-DomainSeparatedCommitment -Domain 'LATTICE_TASK038_SESSION_EFFECT_SNAPSHOT_V1' -Value $After
        delivery_git_effect = Get-DeliveryGitEffectReceipt
        observed_effect = [ordered]@{
            database_codex_intent_count = 1L
            delivery_filesystem_changed = $true
            source_filesystem_changed = $false
            process_active_after_cleanup = 0L
            network_owner_rows_after_cleanup = 0L
        }
        exact_bounded_effect_observed = $true
    }
}

function Invoke-NegativeProtocolCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ToolName,
        [AllowNull()]$Arguments,
        [Parameter(Mandatory = $true)][string]$ExpectedMessage
    )

    if ($Mode -eq 'FULL') {
        $effectBefore = Get-ProductionEffectSnapshot
        $counterBefore = $null
    }
    else {
        $counterBefore = Get-HarnessObservedCounters
        $effectBefore = $null
    }
    $frames = @(
        [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = [ordered]@{ protocolVersion = '2025-11-25'; capabilities = [ordered]@{}; clientInfo = [ordered]@{ name = 'task038-negative-protocol'; version = '1' } } },
        [ordered]@{ jsonrpc = '2.0'; method = 'notifications/initialized'; params = [ordered]@{} },
        [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/call'; params = [ordered]@{ name = $ToolName; arguments = $Arguments } }
    )
    $session = Invoke-McpSession -Name $Name -Frames $frames -ExpectedDispatchTools @()
    $null = Get-McpResponse -Responses @($session.Responses) -Id 1
    $error = Get-McpProtocolError -Responses @($session.Responses) -Id 2 -ExpectedCode -32602 -ExpectedMessage $ExpectedMessage
    if ($Mode -eq 'FULL') {
        if (
            $null -eq $session.AcceptanceEvidence -or
            [string]$session.AcceptanceEvidence.schema -cne 'lattice.task038.mcp-acceptance-dispatch-evidence.v1' -or
            [int]$session.AcceptanceEvidence.dispatch_accepted_count -ne 0 -or
            -not [bool]$session.AcceptanceEvidence.normal_close_complete
        ) { throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED' }
        $effectAfter = Get-ProductionEffectSnapshot
        if ((ConvertTo-CanonicalJson -Value $effectBefore) -cne (ConvertTo-CanonicalJson -Value $effectAfter)) {
            throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED'
        }
        if (
            [int]$session.Summary.job_active_processes_after_exit -ne 0 -or
            [int]$session.Summary.process_session_pid_present_after_cleanup -ne 0 -or
            [int]$session.Summary.network_tcp_owner_rows_after_cleanup -ne 0 -or
            [int]$session.Summary.network_udp_owner_rows_after_cleanup -ne 0
        ) { throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED' }
        $counterDelta = [ordered]@{
            dispatch = 0L
            database = 0L
            filesystem = 0L
            process = 0L
            network = 0L
            codex = 0L
            effect = 0L
        }
        $observationScope = 'PRODUCTION_SERVER_DISPATCH_DB_FS_PROCESS_NETWORK_CODEX'
        $effectReceipt = [ordered]@{
            schema_version = 'lattice.task038.production-negative-effect-observation.v1'
            dispatch_evidence_raw_sha256 = [string]$session.AcceptanceEvidence.raw_sha256
            dispatch_final_event_sha256 = [string]$session.AcceptanceEvidence.final_event_sha256
            before_commitment = Get-DomainSeparatedCommitment -Domain 'LATTICE_TASK038_NEGATIVE_EFFECT_SNAPSHOT_V1' -Value $effectBefore
            after_commitment = Get-DomainSeparatedCommitment -Domain 'LATTICE_TASK038_NEGATIVE_EFFECT_SNAPSHOT_V1' -Value $effectAfter
            exact_match = $true
            observed_counter_delta = $counterDelta
        }
    }
    else {
        $counterAfter = Get-HarnessObservedCounters
        $counterDelta = [ordered]@{}
        foreach ($counterName in $script:ObservedCounterNames) {
            $delta = [long]$counterAfter[$counterName] - [long]$counterBefore[$counterName]
            if ($delta -lt 0) { throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED' }
            $counterDelta[$counterName] = $delta
        }
        if ([long]$counterDelta.dispatch -ne 0 -or [long]$counterDelta.effect -ne 0) {
            throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REJECTED'
        }
        $observationScope = 'HARNESS_FAKE_ONLY_NOT_PRODUCTION'
        $effectReceipt = $null
    }
    $evidence = [ordered]@{
        schema_version = 'lattice.task038.negative-protocol.v1'
        case = $Name
        status = 'PASS'
        protocol_error_code = [int]$error.code
        protocol_error_message = [string]$error.message
        service_dispatch_observed = [long]$counterDelta.dispatch
        external_effect_observed = [long]$counterDelta.effect
        observed_counter_delta = $counterDelta
        authoritative_observation = $true
        observation_scope = $observationScope
        production_effect_receipt = $effectReceipt
        independent_process_id = [int]$session.Summary.process_id
    }
    $script:NegativeProtocolCases.Add($evidence)
    Write-SafeJson -Path (Join-Path $script:EvidenceDirectory ($Name + '.json')) -Value $evidence
}

function Assert-PublicTaskStatus {
    param(
        [Parameter(Mandatory = $true)]$Value,
        [switch]$RequireCompleted
    )

    $names = @($Value.PSObject.Properties.Name | Sort-Object)
    $terminalFailureStates = @('BLOCKED', 'CANCELLED', 'FAILED', 'REJECTED')
    $reconciliationStates = @(
        'AWAITING_EXECUTION_APPROVAL', 'AWAITING_MERGE_APPROVAL', 'DRAFT', 'EXECUTING',
        'MERGING', 'PREPARING', 'REVIEWING', 'STOPPING', 'VERIFYING'
    )
    $statusMappingValid = switch ([string]$Value.status) {
        'COMPLETED' { [string]$Value.task_state -ceq 'COMPLETED' -and $Value.result_digest -is [string]; break }
        'FAILED' { $terminalFailureStates -ccontains [string]$Value.task_state; break }
        'RECONCILIATION_REQUIRED' { $reconciliationStates -ccontains [string]$Value.task_state; break }
        'NOT_SUBMITTED' { [string]$Value.task_state -ceq 'NOT_SUBMITTED' -and $null -eq $Value.result_digest; break }
        default { $false }
    }
    if (
        @(Compare-Object $script:PublicStatusFields $names).Count -ne 0 -or
        [string]$Value.schema_version -cne 'lattice.task.status.v1' -or
        [string]$Value.task_ref -notmatch '^[0-9a-f]{64}$' -or
        [string]$Value.ledger_head_digest -notmatch '^[0-9a-f]{64}$' -or
        ($null -ne $Value.result_digest -and [string]$Value.result_digest -notmatch '^[0-9a-f]{64}$') -or
        -not $statusMappingValid
    ) {
        throw 'TASK038_ACCEPT_PUBLIC_STATUS_REJECTED'
    }
    if ($RequireCompleted -and (
        [string]$Value.status -cne 'COMPLETED' -or
        [string]$Value.task_state -cne 'COMPLETED' -or
        [string]$Value.result_digest -notmatch '^[0-9a-f]{64}$'
    )) {
        throw 'TASK038_ACCEPT_TASK_NOT_COMPLETED'
    }
}

function Assert-SamePublicStatus {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual
    )

    foreach ($name in $script:PublicStatusFields) {
        $left = $Expected.$name
        $right = $Actual.$name
        if (($null -eq $left) -ne ($null -eq $right) -or ($null -ne $left -and [string]$left -cne [string]$right)) {
            throw 'TASK038_ACCEPT_DURABLE_STATUS_MISMATCH'
        }
    }
}

function Invoke-PsqlCsv {
    param(
        [Parameter(Mandatory = $true)][string]$Query,
        [Parameter(Mandatory = $true)][string]$Header,
        [Parameter(Mandatory = $true)][string]$Password
    )

    $databaseName = Get-Task019ProductionDatabaseName -RunId $PostgresRunId
    $original = [Environment]::GetEnvironmentVariable('PGPASSWORD', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $Password, 'Process')
        $previous = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $output = @(& $script:Psql --no-psqlrc --no-password --quiet --csv -h $PostgresHost -p $PostgresPort -U task019_harness -d $databaseName -v ON_ERROR_STOP=1 -c $Query 2>&1)
            $exitCode = [int]$LASTEXITCODE
        }
        finally {
            $ErrorActionPreference = $previous
        }
    }
    finally {
        [Environment]::SetEnvironmentVariable('PGPASSWORD', $original, 'Process')
    }
    $text = [string]::Join("`n", @($output | ForEach-Object { [string]$_ }))
    if ($exitCode -ne 0 -or $text.Length -gt 262144) {
        throw 'TASK038_ACCEPT_POSTGRES_PROBE_REJECTED'
    }
    $lines = @($text -split '\r?\n' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $headerIndex = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        if ([string]$lines[$index] -eq $Header -or [string]$lines[$index] -like ($Header + ',*')) {
            $headerIndex = $index
            break
        }
    }
    if ($headerIndex -lt 0 -or $headerIndex + 1 -ge $lines.Count) {
        throw 'TASK038_ACCEPT_POSTGRES_PROBE_SHAPE_REJECTED'
    }
    try {
        return @(([string]::Join("`n", $lines[$headerIndex..($lines.Count - 1)])) | ConvertFrom-Csv)
    }
    catch {
        throw 'TASK038_ACCEPT_POSTGRES_PROBE_SHAPE_REJECTED'
    }
}

function Get-PostgresProcessEvidence {
    param([Parameter(Mandatory = $true)][string]$Password)

    $rows = @(Invoke-PsqlCsv -Password $Password -Header 'postmaster_started_at' -Query @'
SELECT pg_postmaster_start_time()::text AS postmaster_started_at,
       system_identifier::text
FROM pg_control_system();
'@)
    if ($rows.Count -ne 1 -or [string]$rows[0].system_identifier -notmatch '^[0-9]{1,20}$') {
        throw 'TASK038_ACCEPT_POSTGRES_PROCESS_EVIDENCE_REJECTED'
    }
    return [ordered]@{
        postmaster_started_at = [string]$rows[0].postmaster_started_at
        system_identifier = [string]$rows[0].system_identifier
    }
}

function Get-NativeVersionText {
    param([Parameter(Mandatory = $true)][string]$Executable)

    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $Executable --version 2>&1)
        $exitCode = [int]$LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previous }
    $text = ([string]::Join("`n", @($output | ForEach-Object { [string]$_ }))).Trim()
    if ($exitCode -ne 0 -or $text.Length -gt 4096) { throw 'TASK038_ACCEPT_POSTGRES_NATIVE_IDENTITY_REJECTED' }
    return $text
}

function Assert-PostgresPortPolicy {
    param([Parameter(Mandatory = $true)][int]$Port)

    if ($Port -lt 1 -or $Port -gt 65535 -or $Port -in @(5432, 64272, 55432)) {
        throw 'TASK038_ACCEPT_POSTGRES_PORT_REJECTED'
    }
    $netsh = Join-Path $env:SystemRoot 'System32\netsh.exe'
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = @(& $netsh interface ipv4 show dynamicportrange protocol=tcp 2>&1)
        $exitCode = [int]$LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previous }
    $values = @([regex]::Matches(([string]::Join("`n", @($output))), ':\s*([0-9]{1,5})\s*(?:\r?$|\n)') | ForEach-Object { [int]$_.Groups[1].Value })
    if ($exitCode -ne 0 -or $values.Count -ne 2 -or $values[0] -lt 1 -or $values[1] -lt 1) {
        throw 'TASK038_ACCEPT_POSTGRES_DYNAMIC_PORT_RANGE_REJECTED'
    }
    if ($Port -lt $values[0] -or $Port -ge ($values[0] + $values[1])) {
        throw 'TASK038_ACCEPT_POSTGRES_PORT_REJECTED'
    }
}

function Get-PostgresDatabaseBinding {
    param([Parameter(Mandatory = $true)][string]$Password)

    $failureCode = 'TASK038_ACCEPT_POSTGRES_BINDING_REJECTED'
    Assert-PostgresPortPolicy -Port $PostgresPort
    $canonicalBin = Get-CanonicalPath -Path 'C:\Program Files\PostgreSQL\17\bin'
    $expectedPsql = Join-Path $canonicalBin 'psql.exe'
    $expectedPgCtl = Join-Path $canonicalBin 'pg_ctl.exe'
    $expectedPostgres = Join-Path $canonicalBin 'postgres.exe'
    $expectedData = Get-CanonicalPath -Path (Join-Path $script:SourceRoot ('target\task019-postgres\' + $PostgresRunId + '\data'))
    $clusterRoot = Split-Path -Parent $expectedData
    $markerPath = Join-Path $clusterRoot '.lattice-task019-disposable.json'
    if (
        $script:Psql -cne $expectedPsql -or
        (Get-CanonicalPath -Path $PgCtlExecutable) -cne $expectedPgCtl -or
        (Get-CanonicalPath -Path $PostgresDataDirectory) -cne $expectedData -or
        -not (Test-Path -LiteralPath $expectedPsql -PathType Leaf) -or
        -not (Test-Path -LiteralPath $expectedPgCtl -PathType Leaf) -or
        -not (Test-Path -LiteralPath $expectedPostgres -PathType Leaf) -or
        -not (Test-Path -LiteralPath $expectedData -PathType Container) -or
        -not (Test-Path -LiteralPath $markerPath -PathType Leaf)
    ) { throw $failureCode }
    if (
        (Get-FileSha256 -Path $expectedPsql) -cne $script:PsqlSha256 -or
        (Get-FileSha256 -Path $expectedPgCtl) -cne $script:PgCtlSha256 -or
        (Get-FileSha256 -Path $expectedPostgres) -cne $script:PostgresSha256 -or
        (Get-NativeVersionText -Executable $expectedPsql) -cnotmatch ('\Apsql \(PostgreSQL\) ' + [regex]::Escape($script:PostgresVersion) + '\z') -or
        (Get-NativeVersionText -Executable $expectedPgCtl) -cnotmatch ('\Apg_ctl \(PostgreSQL\) ' + [regex]::Escape($script:PostgresVersion) + '\z') -or
        (Get-NativeVersionText -Executable $expectedPostgres) -cnotmatch ('\Apostgres \(PostgreSQL\) ' + [regex]::Escape($script:PostgresVersion) + '\z')
    ) { throw 'TASK038_ACCEPT_POSTGRES_NATIVE_IDENTITY_REJECTED' }
    $markerBytes = [IO.File]::ReadAllBytes($markerPath)
    if (
        $markerBytes.Length -lt 1 -or $markerBytes.Length -gt 65536 -or
        ($markerBytes.Length -ge 3 -and $markerBytes[0] -eq 0xef -and $markerBytes[1] -eq 0xbb -and $markerBytes[2] -eq 0xbf)
    ) { throw $failureCode }
    try { $markerText = [Text.UTF8Encoding]::new($false, $true).GetString($markerBytes) }
    catch { throw $failureCode }
    if (-not $markerText.EndsWith("`n", [StringComparison]::Ordinal) -or $markerText.Contains("`r")) { throw $failureCode }
    try { $marker = $markerText | ConvertFrom-Json -ErrorAction Stop }
    catch { throw $failureCode }
    Assert-ExactJsonKeys -Object $marker -Expected @(
        'kind', 'run_id', 'created_at_utc', 'root', 'parent', 'repository_target',
        'postgres_version', 'host', 'port', 'excluded_ports', 'identity_materialized',
        'system_identifier', 'initial_postmaster_started_at', 'data_native_identity',
        'postgres_executable_path', 'postgres_executable_raw_sha256', 'postgres_executable_native_identity',
        'psql_executable_path', 'psql_executable_raw_sha256', 'psql_executable_native_identity',
        'pg_ctl_executable_path', 'pg_ctl_executable_raw_sha256', 'pg_ctl_executable_native_identity',
        'restart_postmaster_started_at', 'restart_identity_verified'
    ) -FailureCode $failureCode
    $createdAt = [DateTimeOffset]::MinValue
    $excluded = @($marker.excluded_ports)
    if (
        $marker.kind -isnot [string] -or [string]$marker.kind -cne 'LATTICE_TASK019_DISPOSABLE_POSTGRES_V1' -or
        $marker.run_id -isnot [string] -or [string]$marker.run_id -cne $PostgresRunId -or
        $marker.created_at_utc -isnot [string] -or
        -not [DateTimeOffset]::TryParse([string]$marker.created_at_utc, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind, [ref]$createdAt) -or
        $createdAt -gt [DateTimeOffset]::UtcNow.AddMinutes(5) -or $createdAt -lt [DateTimeOffset]::UtcNow.AddMinutes(-30) -or
        [string]$marker.root -cne $clusterRoot -or
        [string]$marker.parent -cne (Split-Path -Parent $clusterRoot) -or
        [string]$marker.repository_target -cne (Join-Path $script:SourceRoot 'target') -or
        [string]$marker.postgres_version -cne $script:PostgresVersion -or
        [string]$marker.host -cne '127.0.0.1' -or
        -not (Test-JsonInteger -Value $marker.port) -or [int]$marker.port -ne $PostgresPort -or
        $PostgresPort -in @(5432, 64272, 55432) -or
        (ConvertTo-CanonicalJson -Value $excluded) -cne (ConvertTo-CanonicalJson -Value @(5432, 64272, 55432)) -or
        $marker.identity_materialized -isnot [bool] -or -not [bool]$marker.identity_materialized -or
        $marker.restart_identity_verified -isnot [bool] -or -not [bool]$marker.restart_identity_verified -or
        [string]$marker.system_identifier -cnotmatch '\A[1-9][0-9]{0,19}\z' -or
        [string]$marker.initial_postmaster_started_at -ceq [string]$marker.restart_postmaster_started_at -or
        [string]$marker.postgres_executable_path -cne $expectedPostgres -or
        [string]$marker.postgres_executable_raw_sha256 -cne $script:PostgresSha256 -or
        [string]$marker.psql_executable_path -cne $expectedPsql -or
        [string]$marker.psql_executable_raw_sha256 -cne $script:PsqlSha256 -or
        [string]$marker.pg_ctl_executable_path -cne $expectedPgCtl -or
        [string]$marker.pg_ctl_executable_raw_sha256 -cne $script:PgCtlSha256
    ) { throw $failureCode }
    try {
        $dataNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $expectedData -Directory $true
        $postgresNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $expectedPostgres -Directory $false
        $psqlNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $expectedPsql -Directory $false
        $pgCtlNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $expectedPgCtl -Directory $false
    }
    catch { throw 'TASK038_ACCEPT_POSTGRES_NATIVE_IDENTITY_REJECTED' }
    if (
        [string]$marker.data_native_identity -cne $dataNativeIdentity -or
        [string]$marker.postgres_executable_native_identity -cne $postgresNativeIdentity -or
        [string]$marker.psql_executable_native_identity -cne $psqlNativeIdentity -or
        [string]$marker.pg_ctl_executable_native_identity -cne $pgCtlNativeIdentity
    ) { throw 'TASK038_ACCEPT_POSTGRES_NATIVE_IDENTITY_REJECTED' }
    try {
        $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $PostgresPort -ErrorAction Stop | Where-Object {
            [string]$_.LocalAddress -in @('127.0.0.1', '::ffff:127.0.0.1')
        })
    }
    catch { throw $failureCode }
    if ($listeners.Count -ne 1 -or [int]$listeners[0].OwningProcess -lt 1) { throw $failureCode }
    $runtime = Get-PostgresProcessEvidence -Password $Password
    if (
        [string]$runtime.system_identifier -cne [string]$marker.system_identifier -or
        [string]$runtime.postmaster_started_at -cne [string]$marker.restart_postmaster_started_at
    ) { throw 'TASK038_ACCEPT_POSTGRES_RUNTIME_BINDING_REJECTED' }
    try { $markerNativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $markerPath -Directory $false }
    catch { throw 'TASK038_ACCEPT_POSTGRES_NATIVE_IDENTITY_REJECTED' }
    return [ordered]@{
        schema_version = 'lattice.task038.database-binding.v1'
        postgres_version = $script:PostgresVersion
        postgres_run_id = $PostgresRunId
        postgres_port = $PostgresPort
        excluded_ports = @(5432, 64272, 55432)
        cluster_root = $clusterRoot
        cluster_data = $expectedData
        cluster_marker_raw_sha256 = Get-FileSha256 -Path $markerPath
        cluster_marker_native_identity = $markerNativeIdentity
        postgres_system_identifier = [string]$marker.system_identifier
        initial_postmaster_started_at = [string]$marker.initial_postmaster_started_at
        holder_restart_postmaster_started_at = [string]$marker.restart_postmaster_started_at
        psql_executable_raw_sha256 = $script:PsqlSha256
        pg_ctl_executable_raw_sha256 = $script:PgCtlSha256
        postgres_executable_raw_sha256 = $script:PostgresSha256
        listener_owning_process = [int]$listeners[0].OwningProcess
        marker_restart_identity_verified = $true
        fresh_marker_checked = $true
    }
}

function Assert-PostgresBindingUnchanged {
    if ($null -eq $script:PostgresBinding) { throw 'TASK038_ACCEPT_POSTGRES_BINDING_REJECTED' }
    $markerPath = Join-Path ([string]$script:PostgresBinding.cluster_root) '.lattice-task019-disposable.json'
    try {
        if (
            (Get-FileSha256 -Path $markerPath) -cne [string]$script:PostgresBinding.cluster_marker_raw_sha256 -or
            (Get-LatticeWindowsNativePathIdentityToken -Path $markerPath -Directory $false) -cne [string]$script:PostgresBinding.cluster_marker_native_identity -or
            (Get-FileSha256 -Path $script:Psql) -cne $script:PsqlSha256 -or
            (Get-FileSha256 -Path (Get-CanonicalPath -Path $PgCtlExecutable)) -cne $script:PgCtlSha256 -or
            (Get-FileSha256 -Path 'C:\Program Files\PostgreSQL\17\bin\postgres.exe') -cne $script:PostgresSha256
        ) { throw 'TASK038_ACCEPT_POSTGRES_IDENTITY_CHANGED' }
    }
    catch { throw 'TASK038_ACCEPT_POSTGRES_IDENTITY_CHANGED' }
}

function Get-DatabaseFootprint {
    param(
        [Parameter(Mandatory = $true)][string]$Password,
        [AllowEmptyString()][string]$TaskRef
    )

    if (-not [string]::IsNullOrEmpty($TaskRef) -and $TaskRef -notmatch '^[0-9a-f]{64}$') {
        throw 'TASK038_ACCEPT_TASK_REFERENCE_REJECTED'
    }
    $filter = if ([string]::IsNullOrEmpty($TaskRef)) { '' } else { " AND encode(s.task_spec_digest, 'hex')='$TaskRef'" }
    $query = @"
SET ROLE lattice_migrator;
SELECT
  COALESCE(s.event_count, 0)::text AS event_count,
  COALESCE(s.command_count, 0)::text AS command_count,
  COALESCE(encode(s.task_spec_digest, 'hex'), '') AS task_ref,
  COALESCE(encode(s.head_digest, 'hex'), '') AS ledger_head_digest,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EFFECT_INTENT'), 0)::text AS codex_intents,
  COALESCE((SELECT count(*) FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EFFECT_OUTCOME'), 0)::text AS verified_outcomes,
  COALESCE((SELECT encode(e.subject_digest, 'hex') FROM ONLY control.task_ledger_events e WHERE e.stream_id=s.stream_id AND e.event_kind='EVIDENCE_RECORDED' AND e.action_id='TASK_RESULT'), '') AS result_digest,
  COALESCE(w.fencing_high_water, 0)::text AS writer_fencing_high_water,
  COALESCE(w.command_high_water, 0)::text AS writer_command_count,
  COALESCE((SELECT count(*) FROM ONLY writer_lease.writer_lease_transitions t WHERE t.project_id='task038-controlled-canary'), 0)::text AS writer_transition_count,
  COALESCE(w.current_status, '') AS current_writer_status
FROM (SELECT 1) AS anchor
LEFT JOIN ONLY control.task_ledger_streams s
  ON s.project_id='task038-controlled-canary'
 AND s.project_snapshot_id='task038-controlled-canary:snapshot:1'
 AND s.task_id='TASK-038-CANARY'
 AND s.task_revision=1$filter
LEFT JOIN ONLY writer_lease.writer_lease_heads w
  ON w.project_id='task038-controlled-canary';
"@
    $rows = @(Invoke-PsqlCsv -Password $Password -Header 'event_count' -Query $query)
    if ($rows.Count -ne 1) { throw 'TASK038_ACCEPT_DATABASE_FOOTPRINT_REJECTED' }
    $row = $rows[0]
    return [ordered]@{
        event_count = [int]$row.event_count
        command_count = [int]$row.command_count
        task_ref = [string]$row.task_ref
        ledger_head_digest = [string]$row.ledger_head_digest
        codex_intents = [int]$row.codex_intents
        verified_outcomes = [int]$row.verified_outcomes
        result_digest = [string]$row.result_digest
        writer_fencing_high_water = [int]$row.writer_fencing_high_water
        writer_command_count = [int]$row.writer_command_count
        writer_transition_count = [int]$row.writer_transition_count
        current_writer_status = [string]$row.current_writer_status
    }
}

function Assert-DatabaseCompletion {
    param(
        [Parameter(Mandatory = $true)]$Footprint,
        [Parameter(Mandatory = $true)]$PublicStatus
    )

    if (
        [string]$Footprint.task_ref -cne [string]$PublicStatus.task_ref -or
        [string]$Footprint.ledger_head_digest -cne [string]$PublicStatus.ledger_head_digest -or
        [string]$Footprint.result_digest -cne [string]$PublicStatus.result_digest -or
        [int]$Footprint.codex_intents -ne 1 -or
        [int]$Footprint.verified_outcomes -ne 1 -or
        [int]$Footprint.writer_fencing_high_water -ne 1 -or
        [int]$Footprint.writer_command_count -ne 2 -or
        [int]$Footprint.writer_transition_count -ne 2 -or
        -not [string]::IsNullOrEmpty([string]$Footprint.current_writer_status)
    ) {
        throw 'TASK038_ACCEPT_DATABASE_COMPLETION_REJECTED'
    }
}

try {
    $script:SourceRoot = Get-CanonicalPath -Path $SourceRepository
    if ($Mode -ne 'FULL') {
        if ([string]::IsNullOrWhiteSpace($LatticedExecutable) -or [string]::IsNullOrWhiteSpace($ExpectedBinarySha256)) {
            throw 'TASK038_ACCEPT_NON_LIVE_BINARY_REQUIRED'
        }
        $script:Latticed = Get-CanonicalPath -Path $LatticedExecutable
        $script:BinaryDirectory = Split-Path -Parent $script:Latticed
    }
    if ([string]::IsNullOrWhiteSpace($EvidenceRoot)) {
        $acceptanceRoot = if ($Mode -eq 'FULL') { [IO.Path]::GetTempPath() } else { Split-Path -Parent $PSScriptRoot }
        $EvidenceRoot = Join-Path $acceptanceRoot ('target\task038-four-tool-acceptance\' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
    }
    $script:EvidenceDirectory = Get-CanonicalPath -Path $EvidenceRoot
    if (Test-Path -LiteralPath $script:EvidenceDirectory) {
        throw 'TASK038_ACCEPT_EVIDENCE_ROOT_NOT_FRESH'
    }
    [IO.Directory]::CreateDirectory($script:EvidenceDirectory) | Out-Null

    if ($Mode -eq 'PROTOCOL_ONLY') {
        if ([string]::IsNullOrWhiteSpace($HarnessObservedCounterPath)) {
            throw 'TASK038_ACCEPT_OBSERVED_EFFECT_RECEIPT_REQUIRED'
        }
        $script:HarnessCounterPath = Get-CanonicalPath -Path $HarnessObservedCounterPath
    }
    elseif (-not [string]::IsNullOrWhiteSpace($HarnessObservedCounterPath)) {
        throw 'TASK038_ACCEPT_HARNESS_COUNTER_SCOPE_REJECTED'
    }

    if (
        $Mode -eq 'FULL' -and
        (-not [string]::IsNullOrWhiteSpace($LatticedExecutable) -or
         -not [string]::IsNullOrWhiteSpace($ExpectedBinarySha256))
    ) {
        throw 'TASK038_ACCEPT_CALLER_BINARY_TRUST_REJECTED'
    }

    $script:ToolSchemaContractCommitment = Get-StringSha256 -Value (ConvertTo-CanonicalJson -Value (Get-ExpectedToolSchemaContract))
    if ($Mode -eq 'FULL') {
        $normalizedExpectedCommit = $ExpectedSourceCommit.ToLowerInvariant()
        foreach ($prefix in $script:HistoricalRejectedCommitPrefixes) {
            if ($normalizedExpectedCommit.StartsWith($prefix, [StringComparison]::Ordinal)) {
                throw 'TASK038_ACCEPT_CURRENT_CANDIDATE_COMMIT_REJECTED'
            }
        }
        $resolvedCleanSeed = Invoke-GitText -Arguments @('rev-parse', ($script:CleanSeedCommit + '^{commit}'))
        $resolvedCleanSeedTree = Invoke-GitText -Arguments @('rev-parse', ($script:CleanSeedCommit + '^{tree}'))
        if ($resolvedCleanSeed -cne $script:CleanSeedCommit -or $resolvedCleanSeedTree -cne $script:CleanSeedTree) {
            throw 'TASK038_ACCEPT_CLEAN_SEED_IDENTITY_REJECTED'
        }
        if (-not (Test-GitAncestor -Ancestor $script:CleanSeedCommit -Descendant $normalizedExpectedCommit)) {
            throw 'TASK038_ACCEPT_CLEAN_SEED_ANCESTRY_REJECTED'
        }
        if ([string]::IsNullOrWhiteSpace($ExpectedSourceTree)) {
            throw 'TASK038_ACCEPT_CURRENT_CANDIDATE_TREE_REJECTED'
        }
        if ([string]::IsNullOrWhiteSpace($ExpectedToolSchemaContractSha256) -or
            $ExpectedToolSchemaContractSha256 -cne $script:ToolSchemaContractCommitment) {
            throw 'TASK038_ACCEPT_CURRENT_CANDIDATE_SCHEMA_CONTRACT_REJECTED'
        }
        if ([string]::IsNullOrWhiteSpace($ExpectedToolErrorContractSha256) -or
            $ExpectedToolErrorContractSha256 -cne $script:SafeToolCodeCommitment) {
            throw 'TASK038_ACCEPT_CURRENT_CANDIDATE_ERROR_CONTRACT_REJECTED'
        }
        if (
            -not [string]::IsNullOrWhiteSpace($CurrentCandidateReviewCommitment) -or
            -not [string]::IsNullOrWhiteSpace($CurrentCandidateAcceptanceCommitment)
        ) {
            throw 'TASK038_ACCEPT_CALLER_COMMITMENT_REJECTED'
        }

    }

    if ($RequirePostgresRestart -and $Mode -ne 'FULL') {
        throw 'TASK038_ACCEPT_POSTGRES_RESTART_MODE_REJECTED'
    }
    if ($Mode -eq 'FULL' -and -not $RequirePostgresRestart) {
        throw 'TASK038_ACCEPT_POSTGRES_RESTART_REQUIRED'
    }
    if (-not (Test-Path -LiteralPath $script:SourceRoot -PathType Container)) {
        throw 'TASK038_ACCEPT_SOURCE_ROOT_REJECTED'
    }
    $resolvedCommit = Invoke-GitText -Arguments @('rev-parse', ($ExpectedSourceCommit.ToLowerInvariant() + '^{commit}'))
    if ($resolvedCommit -cne $ExpectedSourceCommit.ToLowerInvariant()) {
        throw 'TASK038_ACCEPT_SOURCE_COMMIT_MISMATCH'
    }
    $sourceHead = Invoke-GitText -Arguments @('rev-parse', 'HEAD')
    if ($sourceHead -cne $resolvedCommit) {
        throw 'TASK038_ACCEPT_SOURCE_HEAD_MISMATCH'
    }
    $sourceTree = Invoke-GitText -Arguments @('rev-parse', ($resolvedCommit + '^{tree}'))
    if ($Mode -eq 'FULL' -and $sourceTree -cne $ExpectedSourceTree.ToLowerInvariant()) {
        throw 'TASK038_ACCEPT_CURRENT_CANDIDATE_TREE_REJECTED'
    }
    if ($Mode -eq 'FULL') {
        $script:CandidateLinkage = Get-CandidateLinkage -CandidateCommit $resolvedCommit -CandidateTree $sourceTree
        $reviewCommitmentInput = [ordered]@{
            review_receipt_source = $script:ReviewReceiptSourceThread
            review_target_commit = $script:ReviewTargetCommit
            review_target_tree = $script:ReviewTargetTree
            finding_ids = @('P1-A','P1-B','P1-C','P1-D','P2')
            candidate_linkage = $script:CandidateLinkage
            tool_schema_contract_sha256 = $script:ToolSchemaContractCommitment
            tool_error_contract_sha256 = $script:SafeToolCodeCommitment
        }
        $script:CandidateReviewCommitment = Get-DomainSeparatedCommitment `
            -Domain 'LATTICE_TASK038_P006_REMEDIATION_REVIEW_V1' `
            -Value $reviewCommitmentInput
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'candidate-linkage.json') -Value ([ordered]@{
            linkage = $script:CandidateLinkage
            review_commitment = $script:CandidateReviewCommitment
            commitment_domain = 'LATTICE_TASK038_P006_REMEDIATION_REVIEW_V1'
        })
        $script:CompletedStages.Add('candidate_linkage')
        if ([string]::IsNullOrWhiteSpace($TunnelLifecycleReceiptPath)) {
            throw 'TASK038_ACCEPT_TUNNEL_LIFECYCLE_RECEIPT_REQUIRED'
        }
    }
    $sourceStatus = Invoke-GitText -Arguments @('status', '--porcelain', '--untracked-files=all')
    if ($Mode -eq 'FULL' -and -not [string]::IsNullOrEmpty($sourceStatus)) {
        throw 'TASK038_ACCEPT_SOURCE_WORKTREE_DIRTY'
    }
    $sourceSubject = Invoke-GitText -Arguments @('show', '-s', '--format=%s', $resolvedCommit)

    if ($Mode -eq 'FULL') {
        . (Join-Path $script:SourceRoot 'scripts\windows-native-path-identity.ps1')
        $script:CandidateBuildEvidence = New-ExactCandidateBuild -Commit $resolvedCommit -Tree $sourceTree
        $script:Latticed = [string]$script:CandidateBuildEvidence.binary_path
        $script:BinaryDirectory = Split-Path -Parent $script:Latticed
        $actualBinarySha = [string]$script:CandidateBuildEvidence.binary_sha256
        $binaryItem = Get-Item -LiteralPath $script:Latticed -Force
        $script:LatticedNativeIdentity = [string]$script:CandidateBuildEvidence.binary_native_identity
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'candidate-build.json') -Value $script:CandidateBuildEvidence
        $script:CompletedStages.Add('candidate_build')
        $script:TunnelLifecycleReceipt = Read-TunnelLifecycleReceipt `
            -Path $TunnelLifecycleReceiptPath `
            -ExpectedInnerExeSha256 $actualBinarySha
        $script:TunnelLifecycleIntegrationChecked = $true
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'tunnel-lifecycle-validation.json') -Value $script:TunnelLifecycleReceipt
        $script:CompletedStages.Add('tunnel_lifecycle_exact_validation')
    }
    else {
        if (-not (Test-Path -LiteralPath $script:Latticed -PathType Leaf)) {
            throw 'TASK038_ACCEPT_BINARY_REJECTED'
        }
        $binaryItem = Get-Item -LiteralPath $script:Latticed -Force
        if ($binaryItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'TASK038_ACCEPT_BINARY_REJECTED'
        }
        $sourcePrefix = $script:SourceRoot + [IO.Path]::DirectorySeparatorChar
        if (-not $script:Latticed.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'TASK038_ACCEPT_BINARY_SOURCE_CONTAINMENT_REJECTED'
        }
        $actualBinarySha = Get-FileSha256 -Path $script:Latticed
        if ($actualBinarySha -cne $ExpectedBinarySha256.ToLowerInvariant()) {
            throw 'TASK038_ACCEPT_BINARY_SHA_MISMATCH'
        }
        $script:LatticedNativeIdentity = Get-AuthoritativeNativeFileIdentity -Path $script:Latticed
        $script:CandidateBuildEvidence = [ordered]@{
            schema_version = 'lattice.task038.non-live-binary.v1'
            binary_path = $script:Latticed
            binary_sha256 = $actualBinarySha
            binary_native_identity = $script:LatticedNativeIdentity
            harness_only = $true
            candidate_provenance = $false
        }
    }

    Initialize-JobObjectInterop
    Initialize-SuspendedProcessInterop
    $identityEvidence = [ordered]@{
        schema_version = 'lattice.task038.binary-identity.v1'
        binary_path = $script:Latticed
        binary_sha256 = $actualBinarySha
        binary_length = [long]$binaryItem.Length
        binary_native_identity = $script:LatticedNativeIdentity
        source_repository = $script:SourceRoot
        source_commit = $resolvedCommit
        source_head = $sourceHead
        source_tree = $sourceTree
        clean_seed_commit = $script:CleanSeedCommit
        clean_seed_tree = $script:CleanSeedTree
        canonical_candidate_commit = $resolvedCommit
        canonical_candidate_tree = $sourceTree
        clean_seed_ancestry_checked = ($Mode -eq 'FULL')
        current_candidate_identity_checked = ($Mode -eq 'FULL')
        tunnel_lifecycle_integration_checked = $script:TunnelLifecycleIntegrationChecked
        tool_schema_contract_sha256 = $script:ToolSchemaContractCommitment
        tool_error_contract_sha256 = $script:SafeToolCodeCommitment
        current_candidate_review_commitment = $(if ($Mode -eq 'FULL') { $script:CandidateReviewCommitment } else { $null })
        current_candidate_acceptance_commitment = $(if ($Mode -eq 'FULL') { $script:CandidateAcceptanceCommitment } else { $null })
        source_subject = $sourceSubject
        source_worktree_tracked_clean = [string]::IsNullOrEmpty($sourceStatus)
        binary_within_source_repository = $true
        source_binding = $(if ($Mode -eq 'FULL') { 'fresh git archive of exact candidate commit/tree plus locked cargo build and native file identity' } else { 'HARNESS_FAKE_ONLY_NOT_CANDIDATE_PROVENANCE' })
    }
    Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'binary-identity.json') -Value $identityEvidence
    $script:CompletedStages.Add('binary_identity')

    if ($Mode -eq 'FULL') {
        if (
            [string]::IsNullOrWhiteSpace($PsqlExecutable) -or
            [string]::IsNullOrWhiteSpace($PgCtlExecutable) -or
            [string]::IsNullOrWhiteSpace($PostgresDataDirectory) -or
            $PostgresPort -eq 0 -or
            [string]::IsNullOrWhiteSpace($PostgresRunId)
        ) {
            throw 'TASK038_ACCEPT_POSTGRES_CONFIGURATION_REJECTED'
        }
        $script:Psql = Get-CanonicalPath -Path $PsqlExecutable
        if (-not (Test-Path -LiteralPath $script:Psql -PathType Leaf)) {
            throw 'TASK038_ACCEPT_PSQL_REJECTED'
        }
        $databasePassword = [Environment]::GetEnvironmentVariable($PostgresPasswordVariable, 'Process')
        if ([string]::IsNullOrEmpty($databasePassword) -or $databasePassword.Length -lt 16 -or $databasePassword.Length -gt 16384) {
            throw 'TASK038_ACCEPT_POSTGRES_SECRET_REJECTED'
        }
        $script:ProductionDatabasePassword = $databasePassword
        $script:ProductionCodexHome = [Environment]::GetEnvironmentVariable('LATTICE_DELIVERY_CODEX_HOME', 'Process')
        $script:ProductionDeliveryRoot = [Environment]::GetEnvironmentVariable('LATTICE_DELIVERY_ROOT', 'Process')
        if (
            [string]::IsNullOrWhiteSpace($script:ProductionCodexHome) -or
            [string]::IsNullOrWhiteSpace($script:ProductionDeliveryRoot)
        ) { throw 'TASK038_ACCEPT_EFFECT_OBSERVATION_ROOT_REJECTED' }
        $script:ProductionCodexHome = Get-CanonicalPath -Path $script:ProductionCodexHome
        $script:ProductionDeliveryRoot = Get-CanonicalPath -Path $script:ProductionDeliveryRoot
        $null = Get-DirectoryFootprint -Root $script:ProductionCodexHome
        $null = Get-DirectoryFootprint -Root $script:ProductionDeliveryRoot
        $script:PostgresBinding = Get-PostgresDatabaseBinding -Password $databasePassword
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'database-binding.json') -Value $script:PostgresBinding
        $script:CompletedStages.Add('postgres_native_fresh_binding')
        $script:PostgresBefore = Get-PostgresProcessEvidence -Password $databasePassword
        $script:DatabaseBefore = Get-DatabaseFootprint -Password $databasePassword -TaskRef ''
        if (
            [int]$script:DatabaseBefore.event_count -ne 0 -or
            [int]$script:DatabaseBefore.command_count -ne 0 -or
            [int]$script:DatabaseBefore.writer_command_count -ne 0 -or
            [int]$script:DatabaseBefore.writer_transition_count -ne 0
        ) {
            throw 'TASK038_ACCEPT_DATABASE_NOT_FRESH'
        }
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'postgres-before.json') -Value ([ordered]@{
            phase = 'initial'
            process = $script:PostgresBefore
            footprint = $script:DatabaseBefore
        })
        $script:CompletedStages.Add('postgres_before')
    }

    $legacyDiscovery = @(
        [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = [ordered]@{ protocolVersion = '2025-11-25'; capabilities = [ordered]@{}; clientInfo = [ordered]@{ name = 'task038-independent-acceptance'; version = '1' } } },
        [ordered]@{ jsonrpc = '2.0'; method = 'notifications/initialized'; params = [ordered]@{} },
        [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/list'; params = [ordered]@{} }
    )
    if ($Mode -eq 'FULL') { $discoveryEffectBefore = Get-ProductionEffectSnapshot }
    $discoverySession = Invoke-McpSession -Name '01-discovery' -Frames $legacyDiscovery -ExpectedDispatchTools @()
    if ($Mode -eq 'FULL') {
        $discoveryEffectAfter = Get-ProductionEffectSnapshot
        $discoveryEffectReceipt = New-ZeroProductionSessionEffectReceipt `
            -Phase '01-discovery' -Before $discoveryEffectBefore -After $discoveryEffectAfter -Session $discoverySession
        $script:ProductionSessionEffects.Add($discoveryEffectReceipt)
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory '01-discovery.effects.json') -Value $discoveryEffectReceipt
    }
    $discoveryResponses = @($discoverySession.Responses)
    $null = Get-McpResponse -Responses $discoveryResponses -Id 1
    Assert-ToolDiscovery -Response (Get-McpResponse -Responses $discoveryResponses -Id 2)
    Write-SafeJson -Path (Join-Path $script:EvidenceDirectory '01-discovery.json') -Value ([ordered]@{
        schema_version = 'lattice.task038.discovery.v1'
        status = 'PASS'
        protocol = '2025-11-25'
        tools = $script:ObservedTools
        delivery_run_invoked = $false
    })
    $script:CompletedStages.Add('four_tool_discovery')

    if ($Mode -ne 'DISCOVERY_ONLY') {
        $negativeCases = @(
            [ordered]@{ name = '02n01-unknown-tool'; tool = 'lattice_unknown_tool'; arguments = [ordered]@{}; message = 'Unknown tool' },
            [ordered]@{ name = '02n02-delivery-extra'; tool = 'lattice_delivery_run'; arguments = [ordered]@{ unexpected = $true }; message = 'Tool accepts no arguments' },
            [ordered]@{ name = '02n03-delivery-non-object'; tool = 'lattice_delivery_status'; arguments = 'not-an-object'; message = 'Tool accepts no arguments' },
            [ordered]@{ name = '02n04-submit-unknown-intent'; tool = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = 'negative-unknown-intent'; intent = 'UNKNOWN_INTENT' }; message = 'Invalid task submit arguments' },
            [ordered]@{ name = '02n05-submit-bad-id'; tool = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = '/bad'; intent = 'CONTROLLED_CODEX_CANARY' }; message = 'Invalid task submit arguments' },
            [ordered]@{ name = '02n06-submit-extra-shell'; tool = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = 'negative-extra-shell'; intent = 'CONTROLLED_CODEX_CANARY'; shell = 'TASK038_NON_SECRET_SENTINEL' }; message = 'Invalid task submit arguments' },
            [ordered]@{ name = '02n07-submit-extra-sql'; tool = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = 'negative-extra-sql'; intent = 'CONTROLLED_CODEX_CANARY'; sql = 'TASK038_NON_SECRET_SENTINEL' }; message = 'Invalid task submit arguments' },
            [ordered]@{ name = '02n08-submit-extra-path'; tool = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = 'negative-extra-path'; intent = 'CONTROLLED_CODEX_CANARY'; path = 'TASK038_NON_SECRET_SENTINEL' }; message = 'Invalid task submit arguments' },
            [ordered]@{ name = '02n09-submit-extra-credential'; tool = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = 'negative-extra-credential'; intent = 'CONTROLLED_CODEX_CANARY'; credential = 'TASK038_NON_SECRET_SENTINEL' }; message = 'Invalid task submit arguments' },
            [ordered]@{ name = '02n10-status-bad'; tool = 'lattice_task_status'; arguments = [ordered]@{ task_ref = 'bad' }; message = 'Invalid task status arguments' },
            [ordered]@{ name = '02n11-status-uppercase'; tool = 'lattice_task_status'; arguments = [ordered]@{ task_ref = ('A' * 64) }; message = 'Invalid task status arguments' },
            [ordered]@{ name = '02n12-status-extra-ref'; tool = 'lattice_task_status'; arguments = [ordered]@{ task_ref = ('a' * 64); ref = ('b' * 64) }; message = 'Invalid task status arguments' }
        )
        foreach ($case in $negativeCases) {
            Invoke-NegativeProtocolCase -Name ([string]$case.name) -ToolName ([string]$case.tool) -Arguments $case.arguments -ExpectedMessage ([string]$case.message)
        }
        if ($script:NegativeProtocolCases.Count -ne 12) {
            throw 'TASK038_ACCEPT_NEGATIVE_PROTOCOL_CASE_COUNT_REJECTED'
        }
        if ($Mode -eq 'FULL') {
            $script:DatabaseAfterNegativeProtocol = Get-DatabaseFootprint -Password $databasePassword -TaskRef ''
            if (($script:DatabaseBefore | ConvertTo-Json -Compress -Depth 8) -cne ($script:DatabaseAfterNegativeProtocol | ConvertTo-Json -Compress -Depth 8)) {
                throw 'TASK038_ACCEPT_NEGATIVE_PROTOCOL_EFFECT_REJECTED'
            }
            Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'postgres-after-negative-protocol.json') -Value $script:DatabaseAfterNegativeProtocol
        }
        $script:CompletedStages.Add('negative_protocol_rejections')

        $modernMeta = [ordered]@{
            'io.modelcontextprotocol/protocolVersion' = '2026-07-28'
            'io.modelcontextprotocol/clientCapabilities' = [ordered]@{}
        }
        $unknownTaskRef = '0' * 64
        $readOnlyFrames = @(
            [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'server/discover'; params = [ordered]@{ _meta = $modernMeta } },
            [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/list'; params = [ordered]@{ _meta = $modernMeta } },
            [ordered]@{ jsonrpc = '2.0'; id = 3; method = 'tools/call'; params = [ordered]@{ name = 'lattice_delivery_status'; arguments = [ordered]@{}; _meta = $modernMeta } },
            [ordered]@{ jsonrpc = '2.0'; id = 4; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_status'; arguments = [ordered]@{ task_ref = $unknownTaskRef }; _meta = $modernMeta } }
        )
        if ($Mode -eq 'FULL') { $readOnlyEffectBefore = Get-ProductionEffectSnapshot }
        $readOnlySession = Invoke-McpSession `
            -Name '02-read-only-status' `
            -Frames $readOnlyFrames `
            -ExpectedDispatchTools @('lattice_delivery_status', 'lattice_task_status')
        if ($Mode -eq 'FULL') {
            $readOnlyEffectAfter = Get-ProductionEffectSnapshot
            $readOnlyEffectReceipt = New-ZeroProductionSessionEffectReceipt `
                -Phase '02-read-only-status' -Before $readOnlyEffectBefore -After $readOnlyEffectAfter -Session $readOnlySession
            $script:ProductionSessionEffects.Add($readOnlyEffectReceipt)
            Write-SafeJson -Path (Join-Path $script:EvidenceDirectory '02-read-only-status.effects.json') -Value $readOnlyEffectReceipt
        }
        $readOnlyResponses = @($readOnlySession.Responses)
        $null = Get-McpResponse -Responses $readOnlyResponses -Id 1
        Assert-ToolDiscovery -Response (Get-McpResponse -Responses $readOnlyResponses -Id 2)
        $deliveryStatusProbe = Get-ToolResult -Response (Get-McpResponse -Responses $readOnlyResponses -Id 3) -Protocol 'STATELESS'
        $taskStatusProbe = Get-ToolResult -Response (Get-McpResponse -Responses $readOnlyResponses -Id 4) -Protocol 'STATELESS'
        if (-not $taskStatusProbe.IsError -or [string]$taskStatusProbe.SafeCode -cne 'LATTICE_TASK_REFERENCE_REJECTED') {
            throw 'TASK038_ACCEPT_PRE_STATUS_FAIL_CLOSED_REJECTED'
        }
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory '02-read-only-status.json') -Value ([ordered]@{
            schema_version = 'lattice.task038.read-only-status.v1'
            status = 'PASS'
            delivery_status_is_error = [bool]$deliveryStatusProbe.IsError
            delivery_status_code = $deliveryStatusProbe.SafeCode
            missing_task_status_is_error = [bool]$taskStatusProbe.IsError
            missing_task_status_code = [string]$taskStatusProbe.SafeCode
            no_submit_or_delivery_run_invoked = $true
        })
        $script:CompletedStages.Add('read_only_status')

        $differentClientRequestId = if ($ClientRequestId.Length -le 42) {
            $ClientRequestId + '-different'
        }
        else {
            $ClientRequestId.Substring(0, 42) + '-different'
        }
        $submitFrames = @(
            [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = [ordered]@{ protocolVersion = '2025-11-25'; capabilities = [ordered]@{}; clientInfo = [ordered]@{ name = 'hostile-client-info-not-authority'; version = '999' } } },
            [ordered]@{ jsonrpc = '2.0'; method = 'notifications/initialized'; params = [ordered]@{} },
            [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/list'; params = [ordered]@{} },
            [ordered]@{ jsonrpc = '2.0'; id = 3; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = $ClientRequestId; intent = 'CONTROLLED_CODEX_CANARY' } } },
            [ordered]@{ jsonrpc = '2.0'; id = 4; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = $ClientRequestId; intent = 'CONTROLLED_CODEX_CANARY' } } },
            [ordered]@{ jsonrpc = '2.0'; id = 5; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_submit'; arguments = [ordered]@{ client_request_id = $differentClientRequestId; intent = 'CONTROLLED_CODEX_CANARY' } } }
        )
        if ($Mode -eq 'FULL') { $submitEffectBefore = Get-ProductionEffectSnapshot }
        $submitSession = Invoke-McpSession `
            -Name '03-bounded-submit' `
            -Frames $submitFrames `
            -ExpectedDispatchTools @('lattice_task_submit', 'lattice_task_submit', 'lattice_task_submit')
        if ($Mode -eq 'FULL') { $submitEffectAfter = Get-ProductionEffectSnapshot }
        $submitResponses = @($submitSession.Responses)
        $null = Get-McpResponse -Responses $submitResponses -Id 1
        Assert-ToolDiscovery -Response (Get-McpResponse -Responses $submitResponses -Id 2)
        $submitted = Get-ToolResult -Response (Get-McpResponse -Responses $submitResponses -Id 3)
        $retried = Get-ToolResult -Response (Get-McpResponse -Responses $submitResponses -Id 4)
        $different = Get-ToolResult -Response (Get-McpResponse -Responses $submitResponses -Id 5)
        if ($submitted.IsError -or $retried.IsError -or -not $different.IsError -or [string]$different.SafeCode -cne 'LATTICE_TASK_REQUEST_SUBSTITUTED') {
            throw 'TASK038_ACCEPT_SUBMIT_ISERROR_REJECTED'
        }
        Assert-PublicTaskStatus -Value $submitted.Structured -RequireCompleted
        Assert-PublicTaskStatus -Value $retried.Structured -RequireCompleted
        Assert-SamePublicStatus -Expected $submitted.Structured -Actual $retried.Structured
        $script:PublicSubmit = $submitted.Structured
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory '03-bounded-submit.json') -Value ([ordered]@{
            schema_version = 'lattice.task038.bounded-submit.v1'
            status = 'PASS'
            tool_is_error = [bool]$submitted.IsError
            retry_tool_is_error = [bool]$retried.IsError
            different_key_tool_is_error = [bool]$different.IsError
            different_key_code = [string]$different.SafeCode
            public_status = $script:PublicSubmit
        })
        $script:CompletedStages.Add('bounded_submit')

        if ($Mode -eq 'FULL') {
            $script:DatabaseAfterSubmit = Get-DatabaseFootprint -Password $databasePassword -TaskRef ([string]$script:PublicSubmit.task_ref)
            Assert-DatabaseCompletion -Footprint $script:DatabaseAfterSubmit -PublicStatus $script:PublicSubmit
            $submitEffectReceipt = New-SubmitProductionSessionEffectReceipt `
                -Before $submitEffectBefore -After $submitEffectAfter -Session $submitSession
            $script:ProductionSessionEffects.Add($submitEffectReceipt)
            Write-SafeJson -Path (Join-Path $script:EvidenceDirectory '03-bounded-submit.effects.json') -Value $submitEffectReceipt
            Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'postgres-after-submit.json') -Value $script:DatabaseAfterSubmit
            $script:CompletedStages.Add('postgres_after_submit')
            if ($RequirePostgresRestart) {
                $script:PostgresRestart = Invoke-PostgresRestart
                Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'postgres-restart-controller.json') -Value $script:PostgresRestart
                $script:CompletedStages.Add('postgres_restart_controller')
            }
        }

        $durableFrames = @(
            [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'server/discover'; params = [ordered]@{ _meta = $modernMeta } },
            [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/list'; params = [ordered]@{ _meta = $modernMeta } },
            [ordered]@{ jsonrpc = '2.0'; id = 3; method = 'tools/call'; params = [ordered]@{ name = 'lattice_task_status'; arguments = [ordered]@{ task_ref = [string]$script:PublicSubmit.task_ref }; _meta = $modernMeta } }
        )
        if ($Mode -eq 'FULL') { $durableEffectBefore = Get-ProductionEffectSnapshot }
        $durableSession = Invoke-McpSession `
            -Name '04-durable-status' `
            -Frames $durableFrames `
            -ExpectedDispatchTools @('lattice_task_status')
        if ($Mode -eq 'FULL') {
            $durableEffectAfter = Get-ProductionEffectSnapshot
            $durableEffectReceipt = New-ZeroProductionSessionEffectReceipt `
                -Phase '04-durable-status' -Before $durableEffectBefore -After $durableEffectAfter -Session $durableSession
            $script:ProductionSessionEffects.Add($durableEffectReceipt)
            Write-SafeJson -Path (Join-Path $script:EvidenceDirectory '04-durable-status.effects.json') -Value $durableEffectReceipt
        }
        $durableResponses = @($durableSession.Responses)
        $null = Get-McpResponse -Responses $durableResponses -Id 1
        Assert-ToolDiscovery -Response (Get-McpResponse -Responses $durableResponses -Id 2)
        $durable = Get-ToolResult -Response (Get-McpResponse -Responses $durableResponses -Id 3) -Protocol 'STATELESS'
        if ($durable.IsError) { throw 'TASK038_ACCEPT_DURABLE_STATUS_ISERROR_REJECTED' }
        Assert-PublicTaskStatus -Value $durable.Structured -RequireCompleted
        Assert-SamePublicStatus -Expected $script:PublicSubmit -Actual $durable.Structured
        $script:PublicStatus = $durable.Structured
        if ([int]$submitSession.Summary.process_id -eq [int]$durableSession.Summary.process_id) {
            throw 'TASK038_ACCEPT_CROSS_SESSION_PROCESS_REJECTED'
        }
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory '04-durable-status.json') -Value ([ordered]@{
            schema_version = 'lattice.task038.durable-status.v1'
            status = 'PASS'
            tool_is_error = [bool]$durable.IsError
            fresh_process = $true
            public_status = $script:PublicStatus
        })
        $script:CompletedStages.Add('cross_session_durable_status')

        if ($Mode -eq 'FULL') {
            $script:DatabaseAfterStatus = Get-DatabaseFootprint -Password $databasePassword -TaskRef ([string]$script:PublicStatus.task_ref)
            Assert-DatabaseCompletion -Footprint $script:DatabaseAfterStatus -PublicStatus $script:PublicStatus
            if (($script:DatabaseAfterSubmit | ConvertTo-Json -Compress -Depth 8) -cne ($script:DatabaseAfterStatus | ConvertTo-Json -Compress -Depth 8)) {
                throw 'TASK038_ACCEPT_STATUS_DATABASE_RERUN_REJECTED'
            }
            $script:PostgresAfter = Get-PostgresProcessEvidence -Password $databasePassword
            if ([string]$script:PostgresBefore.system_identifier -cne [string]$script:PostgresAfter.system_identifier) {
                throw 'TASK038_ACCEPT_POSTGRES_IDENTITY_MISMATCH'
            }
            $postgresRestartObserved = (
                [string]$script:PostgresBefore.postmaster_started_at -cne [string]$script:PostgresAfter.postmaster_started_at
            )
            if ($RequirePostgresRestart -and -not $postgresRestartObserved) {
                throw 'TASK038_ACCEPT_POSTGRES_RESTART_NOT_PROVED'
            }
            Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'postgres-after-status.json') -Value ([ordered]@{
                phase = $(if ($RequirePostgresRestart) { 'restart' } else { 'post_protocol' })
                process = $script:PostgresAfter
                footprint = $script:DatabaseAfterStatus
                footprint_unchanged_since_submit = $true
                postgres_restart_required = [bool]$RequirePostgresRestart
                postgres_restarted_during_acceptance = $postgresRestartObserved
            })
            $script:CompletedStages.Add('postgres_durable_replay')
        }
    }

    if ($Mode -eq 'FULL') {
        Assert-CandidateBinaryUnchanged
        Assert-PostgresBindingUnchanged
        $finalSource = Get-SourceGitFootprint
        if (
            [string]$finalSource.head -cne $resolvedCommit -or
            [string]$finalSource.tree -cne $sourceTree -or
            -not [bool]$finalSource.status_empty
        ) { throw 'TASK038_ACCEPT_SOURCE_IDENTITY_CHANGED' }
        $dispatchReceipts = @($script:SessionEvidence | ForEach-Object {
            [ordered]@{
                phase = [string]$_.phase
                process_id = [int]$_.process_id
                raw_sha256 = [string]$_.acceptance_dispatch_evidence.raw_sha256
                final_event_sha256 = [string]$_.acceptance_dispatch_evidence.final_event_sha256
                session_id = [string]$_.acceptance_dispatch_evidence.session_id
                safe_config_sha256 = [string]$_.acceptance_dispatch_evidence.safe_config_sha256
                dispatch_accepted_count = [int]$_.acceptance_dispatch_evidence.dispatch_accepted_count
                normal_close_complete = [bool]$_.acceptance_dispatch_evidence.normal_close_complete
            }
        })
        $acceptanceCommitmentInput = [ordered]@{
            candidate_linkage = $script:CandidateLinkage
            candidate_build = $script:CandidateBuildEvidence
            remediation_review_commitment = $script:CandidateReviewCommitment
            tunnel_lifecycle = $script:TunnelLifecycleReceipt
            postgres_binding = $script:PostgresBinding
            postgres_before = $script:PostgresBefore
            postgres_after = $script:PostgresAfter
            dispatch_receipts = $dispatchReceipts
            production_session_effect_receipts = @($script:ProductionSessionEffects)
            negative_effect_receipts = @($script:NegativeProtocolCases | ForEach-Object { $_.production_effect_receipt })
            database_before = $script:DatabaseBefore
            database_after_submit = $script:DatabaseAfterSubmit
            database_after_status = $script:DatabaseAfterStatus
            public_submit = $script:PublicSubmit
            public_status = $script:PublicStatus
            tool_schema_contract_sha256 = $script:ToolSchemaContractCommitment
            tool_error_contract_sha256 = $script:SafeToolCodeCommitment
        }
        $script:CandidateAcceptanceCommitment = Get-DomainSeparatedCommitment `
            -Domain 'LATTICE_TASK038_P006_CURRENT_CANDIDATE_ACCEPTANCE_V1' `
            -Value $acceptanceCommitmentInput
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'candidate-acceptance-commitment.json') -Value ([ordered]@{
            schema_version = 'lattice.task038.current-candidate-acceptance-commitment.v1'
            commitment_domain = 'LATTICE_TASK038_P006_CURRENT_CANDIDATE_ACCEPTANCE_V1'
            candidate_commit = $resolvedCommit
            candidate_tree = $sourceTree
            review_commitment = $script:CandidateReviewCommitment
            acceptance_commitment = $script:CandidateAcceptanceCommitment
            input = $acceptanceCommitmentInput
        })
        $script:CompletedStages.Add('exact_candidate_acceptance_commitment')
    }

    $script:CompletedStages.Add('process_cleanup')
}
catch {
    $script:FailureCode = Get-SafeFailureCode -ErrorRecord $_
    $script:FailureExceptionType = $_.Exception.GetType().FullName
    $script:FailureLineNumber = [int]$_.InvocationInfo.ScriptLineNumber
    if ($script:FailureExceptionType -ceq 'System.Management.Automation.PropertyNotFoundException') {
        $propertyMatch = [regex]::Match([string]$_.Exception.Message, "'([A-Za-z][A-Za-z0-9]{0,63})'")
        if ($propertyMatch.Success) {
            $script:FailureMissingProperty = $propertyMatch.Groups[1].Value
        }
    }
}
finally {
    if ($null -ne $script:EvidenceDirectory -and (Test-Path -LiteralPath $script:EvidenceDirectory -PathType Container)) {
        $sessionCount = $script:SessionEvidence.Count
        $cleanSessionCount = @($script:SessionEvidence | Where-Object {
            [int]$_.job_active_processes_after_exit -eq 0 -and
            [bool]$_.job_object_native_handle_assigned -and
            [bool]$_.process_created_suspended -and
            [bool]$_.job_assigned_before_resume -and
            ($Mode -ne 'FULL' -or (
                $null -ne $_.acceptance_dispatch_evidence -and
                [bool]$_.acceptance_dispatch_evidence.normal_close_complete -and
                [int]$_.process_session_pid_present_after_cleanup -eq 0 -and
                [int]$_.network_tcp_owner_rows_after_cleanup -eq 0 -and
                [int]$_.network_udp_owner_rows_after_cleanup -eq 0
            ))
        }).Count
        $processCleanupStatus = if ($sessionCount -eq 0) {
            'NOT_RUN'
        }
        elseif ($cleanSessionCount -eq $sessionCount) {
            'PASS'
        }
        else {
            'FAIL'
        }
        $final = [ordered]@{
            schema_version = $script:SchemaVersion
            constitution_version = 'latticed-1.4'
            status = $(if ($null -eq $script:FailureCode) { 'PASS' } else { 'FAIL' })
            mode = $Mode
            evidence_scope = $(if ($Mode -eq 'FULL') { 'CURRENT_CANDIDATE_FULL' } else { 'NON_LIVE_PROTOCOL_TEST' })
            full_current_candidate_accepted = ($Mode -eq 'FULL' -and $null -eq $script:FailureCode)
            failure_code = $script:FailureCode
            failure_exception_type = $script:FailureExceptionType
            failure_line_number = $script:FailureLineNumber
            failure_missing_property = $script:FailureMissingProperty
            completed_stages = @($script:CompletedStages)
            binary_sha256 = $(if (Get-Variable actualBinarySha -ErrorAction SilentlyContinue) { $actualBinarySha } else { $null })
            source_commit = $(if (Get-Variable resolvedCommit -ErrorAction SilentlyContinue) { $resolvedCommit } else { $null })
            source_tree = $(if (Get-Variable sourceTree -ErrorAction SilentlyContinue) { $sourceTree } else { $null })
            clean_seed_commit = $script:CleanSeedCommit
            clean_seed_tree = $script:CleanSeedTree
            canonical_candidate_commit = $(if (Get-Variable resolvedCommit -ErrorAction SilentlyContinue) { $resolvedCommit } else { $null })
            canonical_candidate_tree = $(if (Get-Variable sourceTree -ErrorAction SilentlyContinue) { $sourceTree } else { $null })
            clean_seed_ancestry_checked = ($Mode -eq 'FULL' -and $null -eq $script:FailureCode)
            current_candidate_identity_checked = ($Mode -eq 'FULL' -and $null -eq $script:FailureCode)
            tunnel_lifecycle_integration_checked = ($Mode -eq 'FULL' -and $script:TunnelLifecycleIntegrationChecked -and $null -eq $script:FailureCode)
            current_candidate_review_commitment = $(if ($Mode -eq 'FULL') { $script:CandidateReviewCommitment } else { $null })
            current_candidate_acceptance_commitment = $(if ($Mode -eq 'FULL') { $script:CandidateAcceptanceCommitment } else { $null })
            candidate_linkage = $(if ($Mode -eq 'FULL') { $script:CandidateLinkage } else { $null })
            candidate_build = $(if ($Mode -eq 'FULL') { $script:CandidateBuildEvidence } else { $null })
            tunnel_lifecycle_receipt = $(if ($Mode -eq 'FULL') { $script:TunnelLifecycleReceipt } else { $null })
            postgres_database_binding = $(if ($Mode -eq 'FULL') { $script:PostgresBinding } else { $null })
            tool_schema_contract_sha256 = $script:ToolSchemaContractCommitment
            tool_error_contract_sha256 = $script:SafeToolCodeCommitment
            tool_error_contract_historical_source_commit = $script:SafeToolCodeContractHistoricalSourceCommit
            discovered_tools = @($script:ObservedTools)
            expected_tools = $script:ExpectedTools
            delivery_run_invoked = $false
            tool_level_is_error_checked = ($script:CompletedStages -contains 'read_only_status')
            negative_protocol_cases = @($script:NegativeProtocolCases)
            negative_protocol_case_count = $script:NegativeProtocolCases.Count
            negative_protocol_rejections_checked = ($script:CompletedStages -contains 'negative_protocol_rejections')
            production_negative_effect_observation_checked = ($Mode -eq 'FULL' -and $script:NegativeProtocolCases.Count -eq 12 -and @($script:NegativeProtocolCases | Where-Object { [string]$_.observation_scope -ceq 'PRODUCTION_SERVER_DISPATCH_DB_FS_PROCESS_NETWORK_CODEX' }).Count -eq 12)
            production_session_effect_observations = @($script:ProductionSessionEffects)
            production_session_effect_observation_count = $script:ProductionSessionEffects.Count
            cross_session_status_checked = ($script:CompletedStages -contains 'cross_session_durable_status')
            postgres_evidence_checked = ($script:CompletedStages -contains 'postgres_durable_replay')
            postgres_restart_required = [bool]$RequirePostgresRestart
            postgres_restart_observed = $(
                if ($null -ne $script:PostgresBefore -and $null -ne $script:PostgresAfter) {
                    [string]$script:PostgresBefore.postmaster_started_at -cne [string]$script:PostgresAfter.postmaster_started_at
                }
                else { $false }
            )
            process_cleanup_checked = ($processCleanupStatus -ceq 'PASS')
            process_cleanup_status = $processCleanupStatus
            process_cleanup_basis = 'WINDOWS_JOB_OBJECT_NATIVE_HANDLE'
            process_launch_order = 'CREATE_SUSPENDED_ASSIGN_JOB_RESUME'
            filesystem_cleanup_performed = $false
            exact_pass_marker = 'TASK038_FOUR_TOOL_ACCEPTANCE=PASS'
            exact_pass_marker_count = $(if ($null -eq $script:FailureCode) { 1 } else { 0 })
            skip_marker_count = 0
            not_run = $false
            sessions = @($script:SessionEvidence)
            public_terminal_status = $script:PublicStatus
            raw_error_retained = $false
            raw_prompt_retained = $false
            raw_url_retained = $false
            raw_token_retained = $false
        }
        try {
            Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'final.json') -Value $final
        }
        catch {
            $script:FailureCode = 'TASK038_ACCEPT_EVIDENCE_WRITE_REJECTED'
        }
    }
}

if ($null -ne $script:FailureCode) {
    Write-Error $script:FailureCode
    exit 1
}

Write-Output 'TASK038_FOUR_TOOL_ACCEPTANCE=PASS'
Write-Output ('EVIDENCE_ROOT=' + $script:EvidenceDirectory)
exit 0
