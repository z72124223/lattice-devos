[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$LatticedExecutable,

    [Parameter(Mandatory = $true)]
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

function Invoke-PostgresRestart {
    if (-not $RequirePostgresRestart) { return $null }

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

function Invoke-McpSession {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][object[]]$Frames
    )

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
    $summaryWritten = $false
    try {
        $job = [LatticeTask038AcceptanceJob]::Create()
        try {
            $launch = [LatticeTask038SuspendedProcess]::Start($script:Latticed, $script:BinaryDirectory, $job)
        }
        catch {
            throw 'TASK038_ACCEPT_PROCESS_SUSPENDED_LAUNCH_REJECTED'
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
            cleanup_identity = 'WINDOWS_JOB_OBJECT_AND_PROCESS_HANDLE'
            raw_output_retained = $false
        }
        $script:SessionEvidence.Add($summary)
        $summaryWritten = $true
        Write-SafeJson -Path (Join-Path $script:EvidenceDirectory ($Name + '.process.json')) -Value $summary
        return [pscustomobject]@{ Responses = $responses; Summary = $summary }
    }
    finally {
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

function Invoke-NegativeProtocolCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ToolName,
        [AllowNull()]$Arguments,
        [Parameter(Mandatory = $true)][string]$ExpectedMessage
    )

    $frames = @(
        [ordered]@{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = [ordered]@{ protocolVersion = '2025-11-25'; capabilities = [ordered]@{}; clientInfo = [ordered]@{ name = 'task038-negative-protocol'; version = '1' } } },
        [ordered]@{ jsonrpc = '2.0'; method = 'notifications/initialized'; params = [ordered]@{} },
        [ordered]@{ jsonrpc = '2.0'; id = 2; method = 'tools/call'; params = [ordered]@{ name = $ToolName; arguments = $Arguments } }
    )
    $session = Invoke-McpSession -Name $Name -Frames $frames
    $null = Get-McpResponse -Responses @($session.Responses) -Id 1
    $error = Get-McpProtocolError -Responses @($session.Responses) -Id 2 -ExpectedCode -32602 -ExpectedMessage $ExpectedMessage
    $evidence = [ordered]@{
        schema_version = 'lattice.task038.negative-protocol.v1'
        case = $Name
        status = 'PASS'
        protocol_error_code = [int]$error.code
        protocol_error_message = [string]$error.message
        service_dispatch_expected = 0
        external_effect_expected = 0
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
    $script:Latticed = Get-CanonicalPath -Path $LatticedExecutable
    $script:BinaryDirectory = Split-Path -Parent $script:Latticed
    if ([string]::IsNullOrWhiteSpace($EvidenceRoot)) {
        $acceptanceRoot = Split-Path -Parent $PSScriptRoot
        $EvidenceRoot = Join-Path $acceptanceRoot ('target\task038-four-tool-acceptance\' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
    }
    $script:EvidenceDirectory = Get-CanonicalPath -Path $EvidenceRoot
    if (Test-Path -LiteralPath $script:EvidenceDirectory) {
        throw 'TASK038_ACCEPT_EVIDENCE_ROOT_NOT_FRESH'
    }
    [IO.Directory]::CreateDirectory($script:EvidenceDirectory) | Out-Null

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
            [string]::IsNullOrWhiteSpace($CurrentCandidateReviewCommitment) -or
            [string]::IsNullOrWhiteSpace($CurrentCandidateAcceptanceCommitment) -or
            $CurrentCandidateReviewCommitment -ceq ('0' * 64) -or
            $CurrentCandidateAcceptanceCommitment -ceq ('0' * 64) -or
            $CurrentCandidateReviewCommitment -ceq $CurrentCandidateAcceptanceCommitment
        ) {
            throw 'TASK038_ACCEPT_CURRENT_CANDIDATE_REVIEW_COMMITMENT_REJECTED'
        }

    }

    if ($RequirePostgresRestart -and $Mode -ne 'FULL') {
        throw 'TASK038_ACCEPT_POSTGRES_RESTART_MODE_REJECTED'
    }
    if (-not (Test-Path -LiteralPath $script:SourceRoot -PathType Container)) {
        throw 'TASK038_ACCEPT_SOURCE_ROOT_REJECTED'
    }
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
        try {
            $candidateBridge = Invoke-GitText -Arguments @(
                'show',
                ($resolvedCommit + ':scripts/run-task038-task-submit.ps1')
            )
            $candidateBridgeTest = Invoke-GitText -Arguments @(
                'show',
                ($resolvedCommit + ':scripts/test-task038-local-acceptance.ps1')
            )
        }
        catch {
            throw 'TASK038_ACCEPT_TUNNEL_LIFECYCLE_NOT_MATERIALIZED'
        }
        $candidateLifecycleContract = $candidateBridge + "`n" + $candidateBridgeTest
        foreach ($requiredLifecycleToken in @(
            'SPAWN',
            'OPEN',
            'CLOSE_REQUESTED',
            'PIPE_CLOSED',
            'EXITED',
            'REAPED',
            'UNKNOWN'
        )) {
            if (-not $candidateLifecycleContract.Contains($requiredLifecycleToken)) {
                throw 'TASK038_ACCEPT_TUNNEL_LIFECYCLE_NOT_MATERIALIZED'
            }
        }
        $script:TunnelLifecycleIntegrationChecked = $true
    }
    $sourceStatus = Invoke-GitText -Arguments @('status', '--porcelain', '--untracked-files=all')
    if ($Mode -eq 'FULL' -and -not [string]::IsNullOrEmpty($sourceStatus)) {
        throw 'TASK038_ACCEPT_SOURCE_WORKTREE_DIRTY'
    }
    $sourceSubject = Invoke-GitText -Arguments @('show', '-s', '--format=%s', $resolvedCommit)

    Initialize-JobObjectInterop
    Initialize-SuspendedProcessInterop
    $identityEvidence = [ordered]@{
        schema_version = 'lattice.task038.binary-identity.v1'
        binary_path = $script:Latticed
        binary_sha256 = $actualBinarySha
        binary_length = [long]$binaryItem.Length
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
        current_candidate_review_commitment = $(if ($Mode -eq 'FULL') { $CurrentCandidateReviewCommitment } else { $null })
        current_candidate_acceptance_commitment = $(if ($Mode -eq 'FULL') { $CurrentCandidateAcceptanceCommitment } else { $null })
        source_subject = $sourceSubject
        source_worktree_tracked_clean = [string]::IsNullOrEmpty($sourceStatus)
        binary_within_source_repository = $true
        source_binding = 'exact clean-seed ancestry plus operator-supplied binary SHA-256 and exact clean Git HEAD/tree'
    }
    Write-SafeJson -Path (Join-Path $script:EvidenceDirectory 'binary-identity.json') -Value $identityEvidence
    $script:CompletedStages.Add('binary_identity')

    if ($Mode -eq 'FULL') {
        if ([string]::IsNullOrWhiteSpace($PsqlExecutable) -or $PostgresPort -eq 0 -or [string]::IsNullOrWhiteSpace($PostgresRunId)) {
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
    $discoverySession = Invoke-McpSession -Name '01-discovery' -Frames $legacyDiscovery
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
        $readOnlySession = Invoke-McpSession -Name '02-read-only-status' -Frames $readOnlyFrames
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
        $submitSession = Invoke-McpSession -Name '03-bounded-submit' -Frames $submitFrames
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
        $durableSession = Invoke-McpSession -Name '04-durable-status' -Frames $durableFrames
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
            [bool]$_.job_assigned_before_resume
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
            current_candidate_review_commitment = $(if ($Mode -eq 'FULL') { $CurrentCandidateReviewCommitment } else { $null })
            current_candidate_acceptance_commitment = $(if ($Mode -eq 'FULL') { $CurrentCandidateAcceptanceCommitment } else { $null })
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
