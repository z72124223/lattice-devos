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

    [ValidatePattern('^[0-9a-f]{32}$')]
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
$script:ExpectedTools = @(
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_task_status',
    'lattice_task_submit'
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
$script:HermesFailureClassification = $null
$script:PublicSubmit = $null
$script:PublicStatus = $null
$script:DatabaseBefore = $null
$script:DatabaseAfterSubmit = $null
$script:DatabaseAfterStatus = $null
$script:PostgresBefore = $null
$script:PostgresAfter = $null
$script:PostgresRestart = $null
$script:ObservedTools = @()

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

function Get-HermesFailureClassification {
    param([AllowNull()][AllowEmptyString()][string]$Code)

    if ([string]::IsNullOrWhiteSpace($Code) -or $Code -notmatch '(?i)(HERMES|PROVIDER)') {
        return $null
    }
    if ($Code -match '(?i)QUOTA|RATE_LIMIT|CAPACITY') { return 'QUOTA' }
    if ($Code -match '(?i)AUTH|UNAUTHORIZED|FORBIDDEN|CREDENTIAL') { return 'AUTH' }
    if ($Code -match '(?i)TRANSPORT|NETWORK|TIMEOUT|CONNECTION|HTTP|SSE') { return 'TRANSPORT' }
    return 'OTHER'
}

function Get-SafeToolCode {
    param([Parameter(Mandatory = $true)]$StructuredContent)

    $code = [string]$StructuredContent.code
    if ($code -notmatch '^LATTICE(?:D)?_[A-Z0-9_]{1,95}$') {
        throw 'TASK038_ACCEPT_TOOL_CODE_REJECTED'
    }
    $classification = Get-HermesFailureClassification -Code $code
    if ($null -ne $classification) {
        $script:HermesFailureClassification = $classification
        return 'HERMES_FAILURE_REDACTED'
    }
    return $code
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
    if ($null -ne $matches[0].PSObject.Properties['error']) {
        throw 'TASK038_ACCEPT_MCP_PROTOCOL_ERROR'
    }
    return $matches[0]
}

function Invoke-McpSession {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][object[]]$Frames
    )

    $inputText = New-McpInput -Frames $Frames
    $job = [IntPtr]::Zero
    $process = $null
    $startedAt = [DateTime]::UtcNow
    $stdout = [string]::Empty
    $stderr = [string]::Empty
    $timedOut = $false
    $exitCode = $null
    $processId = $null
    $activeAfter = $null
    $processStarted = $false
    $assignedByNativeHandle = $false
    $summaryWritten = $false
    try {
        $job = [LatticeTask038AcceptanceJob]::Create()
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $script:Latticed
        $startInfo.WorkingDirectory = $script:BinaryDirectory
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardInput = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.StandardOutputEncoding = $script:Utf8
        $startInfo.StandardErrorEncoding = $script:Utf8

        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) {
            throw 'TASK038_ACCEPT_PROCESS_START_REJECTED'
        }
        $processStarted = $true
        $processId = [int]$process.Id
        [LatticeTask038AcceptanceJob]::Assign($job, $process.Handle)
        $assignedByNativeHandle = $true

        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.Write($inputText)
        $process.StandardInput.Close()

        if (-not $process.WaitForExit($SessionTimeoutSeconds * 1000)) {
            $timedOut = $true
            [LatticeTask038AcceptanceJob]::Terminate($job)
            $null = $process.WaitForExit(5000)
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

        $cleanupWatch = [Diagnostics.Stopwatch]::StartNew()
        do {
            $activeAfter = [uint32][LatticeTask038AcceptanceJob]::Active($job)
            if ($activeAfter -eq 0) { break }
            Start-Sleep -Milliseconds 25
        } while ($cleanupWatch.ElapsedMilliseconds -lt 5000)
        if ($activeAfter -ne 0) {
            [LatticeTask038AcceptanceJob]::Terminate($job)
            throw 'TASK038_ACCEPT_PROCESS_CLEANUP_REJECTED'
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
                    cleanup_identity = 'WINDOWS_JOB_OBJECT_AND_PROCESS_HANDLE'
                    raw_output_retained = $false
                }
                $script:SessionEvidence.Add($failureSummary)
                Write-SafeJson -Path (Join-Path $script:EvidenceDirectory ($Name + '.process.json')) -Value $failureSummary
            }
            $process.Dispose()
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
    ) {
        throw 'TASK038_ACCEPT_TOOL_DISCOVERY_ENVELOPE_REJECTED'
    }
    $tools = @($Response.result.tools)
    $names = @($tools | ForEach-Object { [string]$_.name } | Sort-Object)
    $script:ObservedTools = $names
    if ($tools.Count -ne 4 -or @(Compare-Object -ReferenceObject $script:ExpectedTools -DifferenceObject $names).Count -ne 0) {
        throw 'TASK038_ACCEPT_TOOL_DISCOVERY_MISMATCH'
    }

    foreach ($name in @('lattice_delivery_run', 'lattice_delivery_status')) {
        $tool = @($tools | Where-Object { [string]$_.name -ceq $name })
        if ($tool.Count -ne 1) { throw 'TASK038_ACCEPT_DELIVERY_SCHEMA_REJECTED' }
        $propertiesMember = $tool[0].inputSchema.PSObject.Properties['properties']
        $propertyCount = if ($null -eq $propertiesMember) { 0 } else { @($propertiesMember.Value.PSObject.Properties).Count }
        if (
            [string]$tool[0].inputSchema.type -cne 'object' -or
            [bool]$tool[0].inputSchema.additionalProperties -ne $false -or
            $propertyCount -ne 0 -or
            $null -ne $tool[0].PSObject.Properties['outputSchema']
        ) {
            throw 'TASK038_ACCEPT_DELIVERY_SCHEMA_REJECTED'
        }
    }

    $submit = @($tools | Where-Object { [string]$_.name -ceq 'lattice_task_submit' })
    $status = @($tools | Where-Object { [string]$_.name -ceq 'lattice_task_status' })
    if ($submit.Count -ne 1 -or $status.Count -ne 1) {
        throw 'TASK038_ACCEPT_TASK_SCHEMA_REJECTED'
    }
    $submitProperties = @($submit[0].inputSchema.properties.PSObject.Properties.Name | Sort-Object)
    $submitRequired = @($submit[0].inputSchema.required | Sort-Object)
    $statusProperties = @($status[0].inputSchema.properties.PSObject.Properties.Name | Sort-Object)
    $statusRequired = @($status[0].inputSchema.required | Sort-Object)
    if (
        [bool]$submit[0].inputSchema.additionalProperties -ne $false -or
        @(Compare-Object @('client_request_id', 'intent') $submitProperties).Count -ne 0 -or
        @(Compare-Object @('client_request_id', 'intent') $submitRequired).Count -ne 0 -or
        [bool]$status[0].inputSchema.additionalProperties -ne $false -or
        @(Compare-Object @('task_ref') $statusProperties).Count -ne 0 -or
        @(Compare-Object @('task_ref') $statusRequired).Count -ne 0
    ) {
        throw 'TASK038_ACCEPT_TASK_SCHEMA_REJECTED'
    }
    foreach ($tool in @($submit[0], $status[0])) {
        $outputProperties = @($tool.outputSchema.properties.PSObject.Properties.Name | Sort-Object)
        $outputRequired = @($tool.outputSchema.required | Sort-Object)
        $expectedStatuses = @('COMPLETED', 'FAILED', 'NOT_SUBMITTED', 'RECONCILIATION_REQUIRED')
        $expectedStates = @(
            'AWAITING_EXECUTION_APPROVAL', 'AWAITING_MERGE_APPROVAL', 'BLOCKED', 'CANCELLED',
            'COMPLETED', 'DRAFT', 'EXECUTING', 'FAILED', 'MERGING', 'NOT_SUBMITTED', 'PREPARING',
            'REJECTED', 'REVIEWING', 'STOPPING', 'VERIFYING'
        )
        if (
            [string]$tool.outputSchema.type -cne 'object' -or
            [bool]$tool.outputSchema.additionalProperties -ne $false -or
            @(Compare-Object $script:PublicStatusFields $outputProperties).Count -ne 0 -or
            @(Compare-Object $script:PublicStatusFields $outputRequired).Count -ne 0 -or
            @(Compare-Object $expectedStatuses @($tool.outputSchema.properties.status.enum | Sort-Object)).Count -ne 0 -or
            @(Compare-Object $expectedStates @($tool.outputSchema.properties.task_state.enum | Sort-Object)).Count -ne 0 -or
            [string]$tool.outputSchema.properties.task_ref.pattern -cne '^[0-9a-f]{64}$' -or
            [string]$tool.outputSchema.properties.ledger_head_digest.pattern -cne '^[0-9a-f]{64}$' -or
            @($tool.outputSchema.properties.result_digest.anyOf).Count -ne 2
        ) {
            throw 'TASK038_ACCEPT_TASK_OUTPUT_SCHEMA_REJECTED'
        }
    }
}

function Get-ToolResult {
    param([Parameter(Mandatory = $true)]$Response)

    if (
        $null -eq $Response.result -or
        $null -eq $Response.result.PSObject.Properties['isError'] -or
        $null -eq $Response.result.PSObject.Properties['structuredContent']
    ) {
        throw 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
    }
    $isError = [bool]$Response.result.isError
    $structured = $Response.result.structuredContent
    $safeCode = $null
    if ($isError) {
        if ([string]$structured.status -cne 'ERROR' -or $null -eq $structured.PSObject.Properties['code']) {
            throw 'TASK038_ACCEPT_TOOL_RESULT_ENVELOPE_REJECTED'
        }
        $safeCode = Get-SafeToolCode -StructuredContent $structured
    }
    return [pscustomobject]@{ IsError = $isError; Structured = $structured; SafeCode = $safeCode }
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

    $databaseName = 'lattice_task019_' + $PostgresRunId
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
  COALESCE(w.current_status, '') AS current_writer_status,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_analyses) AS memory_analyses,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_receipts) AS memory_receipts,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_retrieval_audits) AS memory_retrieval_audits,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_records) AS memory_records,
  (SELECT count(*)::text FROM ONLY memory.codebase_memory_reflections) AS memory_reflections,
  (SELECT count(*)::text FROM ONLY memory.openclaw_gateway_commands) AS openclaw_commands
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
        memory_analyses = [int]$row.memory_analyses
        memory_receipts = [int]$row.memory_receipts
        memory_retrieval_audits = [int]$row.memory_retrieval_audits
        memory_records = [int]$row.memory_records
        memory_reflections = [int]$row.memory_reflections
        openclaw_commands = [int]$row.openclaw_commands
    }
}

function Assert-DatabaseCompletion {
    param(
        [Parameter(Mandatory = $true)]$Footprint,
        [Parameter(Mandatory = $true)]$PublicStatus,
        [Parameter(Mandatory = $true)]$Baseline
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
    foreach ($name in @('memory_analyses', 'memory_receipts', 'memory_retrieval_audits', 'memory_records', 'memory_reflections', 'openclaw_commands')) {
        if ([int]$Footprint[$name] -ne [int]$Baseline[$name]) {
            throw 'TASK038_ACCEPT_DOWNSTREAM_EFFECT_REJECTED'
        }
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
    $sourceStatus = Invoke-GitText -Arguments @('status', '--porcelain', '--untracked-files=no')
    if (-not [string]::IsNullOrEmpty($sourceStatus)) {
        throw 'TASK038_ACCEPT_SOURCE_WORKTREE_DIRTY'
    }
    $sourceTree = Invoke-GitText -Arguments @('rev-parse', ($resolvedCommit + '^{tree}'))
    $sourceSubject = Invoke-GitText -Arguments @('show', '-s', '--format=%s', $resolvedCommit)

    Initialize-JobObjectInterop
    $identityEvidence = [ordered]@{
        schema_version = 'lattice.task038.binary-identity.v1'
        binary_path = $script:Latticed
        binary_sha256 = $actualBinarySha
        binary_length = [long]$binaryItem.Length
        source_repository = $script:SourceRoot
        source_commit = $resolvedCommit
        source_head = $sourceHead
        source_tree = $sourceTree
        source_subject = $sourceSubject
        source_worktree_tracked_clean = $true
        binary_within_source_repository = $true
        source_binding = 'operator-supplied expected binary SHA-256 plus exact clean Git HEAD'
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
        $deliveryStatusProbe = Get-ToolResult -Response (Get-McpResponse -Responses $readOnlyResponses -Id 3)
        $taskStatusProbe = Get-ToolResult -Response (Get-McpResponse -Responses $readOnlyResponses -Id 4)
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
            Assert-DatabaseCompletion -Footprint $script:DatabaseAfterSubmit -PublicStatus $script:PublicSubmit -Baseline $script:DatabaseBefore
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
        $durable = Get-ToolResult -Response (Get-McpResponse -Responses $durableResponses -Id 3)
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
            Assert-DatabaseCompletion -Footprint $script:DatabaseAfterStatus -PublicStatus $script:PublicStatus -Baseline $script:DatabaseBefore
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
            [int]$_.job_active_processes_after_exit -eq 0 -and [bool]$_.job_object_native_handle_assigned
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
            status = $(if ($null -eq $script:FailureCode) { 'PASS' } else { 'FAIL' })
            mode = $Mode
            failure_code = $script:FailureCode
            failure_exception_type = $script:FailureExceptionType
            failure_line_number = $script:FailureLineNumber
            failure_missing_property = $script:FailureMissingProperty
            hermes_failure_classification = $script:HermesFailureClassification
            completed_stages = @($script:CompletedStages)
            binary_sha256 = $(if (Get-Variable actualBinarySha -ErrorAction SilentlyContinue) { $actualBinarySha } else { $null })
            source_commit = $(if (Get-Variable resolvedCommit -ErrorAction SilentlyContinue) { $resolvedCommit } else { $null })
            discovered_tools = @($script:ObservedTools)
            expected_tools = $script:ExpectedTools
            delivery_run_invoked = $false
            hermes_run_invoked = $false
            tool_level_is_error_checked = ($script:CompletedStages -contains 'read_only_status')
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
